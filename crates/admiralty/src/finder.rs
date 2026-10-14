//! The finder (`space`): every member and stowaway of every fleet, one line
//! each in tree order, matched fzf-style (`nucleo-matcher`) against what the
//! user types. Several space-separated terms must all match; a letter at the
//! start of a word (after `/`, a space, `.` or `-`) scores higher. fzf alone
//! scores `mas` the same on every `MAIN/ARGOS/...` (`M` + `AS` of ARGOS), so a
//! term that spells the path's word initials in order ranks above the rest:
//! `mas` puts `MAIN/ARGOS/SIGNAL` first, `hgn` `HOME/GITHUB/NIMITZ_DEV`.

use nimitz_core::{Id, Snapshot};
use nucleo_matcher::pattern::{Atom, CaseMatching, Normalization};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::style::Style;

use crate::theme::{bold, mono, st, status_style, status_word};
use crate::view::{fleet_members, stowaway_tab, tabs};

/// One line of the finder: what it lands on and the text it is matched by.
#[derive(Clone, Debug)]
pub struct Cand {
    /// the list row it lands on (`M:<id>`, `SW:<session>`)
    pub row_id: String,
    pub member: Option<Id>,
    /// path, expected name, status word, topic; two spaces between them
    pub text: String,
    /// each field's char range in `text`, with its style
    pub fields: Vec<(usize, usize, Style)>,
    /// the first field's word initials (words split at `/` and `_`),
    /// lowercase, with their char index in `text`
    initials: Vec<(char, u32)>,
}

impl Cand {
    fn new(row_id: String, member: Option<Id>, fields: Vec<(String, Style)>) -> Cand {
        let (mut text, mut ranges) = (String::new(), vec![]);
        for (s, style) in fields.into_iter().filter(|(s, _)| !s.is_empty()) {
            if !text.is_empty() {
                text.push_str("  ");
            }
            let from = text.chars().count();
            // one line: a topic with a line break would break the list
            text.push_str(&crate::app::one_line(&s));
            ranges.push((from, text.chars().count(), style));
        }
        let path_end = ranges.first().map(|f| f.1).unwrap_or(0);
        let mut initials = vec![];
        let mut prev = '/';
        for (i, ch) in text.chars().take(path_end).enumerate() {
            if matches!(prev, '/' | '_') && ch.is_alphanumeric() {
                initials.extend(ch.to_lowercase().map(|c| (c, i as u32)));
            }
            prev = ch;
        }
        Cand { row_id, member, text, fields: ranges, initials }
    }

    /// Where `term` (lowercase) spells this line's word initials in order:
    /// their indices, and whether it spells all of them.
    fn acronym(&self, term: &str) -> Option<(Vec<u32>, bool)> {
        let mut at = self.initials.iter();
        let mut idx = vec![];
        for ch in term.chars() {
            idx.push(at.find(|(c, _)| *c == ch)?.1);
        }
        Some((idx, term.chars().count() == self.initials.len()))
    }

    /// The style of the char at `i`.
    pub fn style_at(&self, i: usize) -> Style {
        self.fields.iter().find(|(a, b, _)| (*a..*b).contains(&i)).map(|f| f.2).unwrap_or_default()
    }
}

/// Every member of every fleet, then the stowaways of its folders, tab by
/// tab (the unassigned stowaways last): the order the list shows them in.
pub fn candidates(snap: &Snapshot) -> Vec<Cand> {
    let mut out = vec![];
    for tab in tabs(snap) {
        for f in snap.fleets.iter().filter(|f| f.id.0 == tab) {
            for m in fleet_members(f) {
                out.push(Cand::new(
                    format!("M:{}", m.id.0),
                    Some(m.id.clone()),
                    vec![
                        (m.id.0.clone(), bold(mono::FG)),
                        (m.expected_name.clone(), st(mono::SOFT)),
                        (status_word(m.status).to_string(), status_style(m.status)),
                        (m.topic.clone().unwrap_or_default(), st(mono::COMMENT)),
                    ],
                ));
            }
        }
        for s in snap.stowaways.iter().filter(|s| stowaway_tab(snap, s) == tab) {
            let short = s.short_id.clone().unwrap_or_else(|| s.session_id.chars().take(8).collect());
            let live = s.pid.is_some();
            out.push(Cand::new(
                format!("SW:{}", s.session_id),
                None,
                vec![
                    (short, bold(mono::WARN)),
                    (s.name.clone(), st(mono::SOFT)),
                    (format!("stowaway {}", if live { "live" } else { "stopped" }), if live { st(mono::WARN) } else { st(mono::COMMENT) }),
                ],
            ));
        }
    }
    out
}

/// A candidate that matches: its index, its score, the matched chars.
#[derive(Clone, Debug)]
pub struct Hit {
    pub cand: usize,
    pub score: u32,
    pub idx: Vec<u32>,
}

/// What a term that spells a path's initials adds to its score (an fzf
/// score of a short term is far below it).
const ACRONYM: u32 = 1000;

pub struct Finder {
    pub query: String,
    /// the selected line of `hits`
    pub sel: usize,
    pub cands: Vec<Cand>,
    /// best first; ties keep the tree order
    pub hits: Vec<Hit>,
    matcher: Matcher,
}

impl Finder {
    pub fn new(snap: &Snapshot) -> Finder {
        let mut f = Finder { query: String::new(), sel: 0, cands: candidates(snap), hits: vec![], matcher: Matcher::new(Config::DEFAULT) };
        f.refilter();
        f
    }

    /// A refresh: the new candidates, the selection kept on its row.
    pub fn set_snapshot(&mut self, snap: &Snapshot) {
        let keep = self.selected().map(|c| c.row_id.clone());
        self.cands = candidates(snap);
        self.refilter();
        if let Some(i) = keep.and_then(|id| self.hits.iter().position(|h| self.cands[h.cand].row_id == id)) {
            self.sel = i;
        }
    }

    /// Match the query again; the selection goes to the best.
    pub fn refilter(&mut self) {
        // each term on its own (fzf's syntax: 'exact, ^prefix, suffix$, !not)
        let terms: Vec<(String, Atom)> = self.query.split_whitespace().map(|t| (t.to_lowercase(), Atom::parse(t, CaseMatching::Ignore, Normalization::Smart))).collect();
        let mut buf = vec![];
        let mut hits = vec![];
        'cand: for (i, c) in self.cands.iter().enumerate() {
            let (mut score, mut idx) = (0u32, vec![]);
            for (raw, atom) in &terms {
                let mut own = vec![];
                let Some(s) = atom.indices(Utf32Str::new(&c.text, &mut buf), &mut self.matcher, &mut own) else { continue 'cand };
                score += s as u32;
                // the path's initials, in order: above any fzf score, all of them above some
                if let Some((at, all)) = raw.chars().all(char::is_alphanumeric).then(|| c.acronym(raw)).flatten() {
                    score += if all { 2 * ACRONYM } else { ACRONYM };
                    own = at;
                }
                idx.extend(own);
            }
            idx.sort_unstable();
            idx.dedup();
            hits.push(Hit { cand: i, score, idx });
        }
        // a stable sort: equal scores stay in tree order
        hits.sort_by(|a, b| b.score.cmp(&a.score));
        self.hits = hits;
        self.sel = 0;
    }

    pub fn selected(&self) -> Option<&Cand> {
        self.hits.get(self.sel).map(|h| &self.cands[h.cand])
    }

    pub fn step(&mut self, by: i64) {
        if !self.hits.is_empty() {
            self.sel = (self.sel as i64 + by).clamp(0, self.hits.len() as i64 - 1) as usize;
        }
    }

    pub fn push(&mut self, text: &str) {
        self.query.push_str(text);
        self.refilter();
    }

    pub fn pop(&mut self) {
        self.query.pop();
        self.refilter();
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.refilter();
    }
}
