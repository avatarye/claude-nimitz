//! The finder (`space`): every member and stowaway of every fleet, one line
//! each in tree order, matched fzf-style (`nucleo-matcher`) against what the
//! user types. Several space-separated terms must all match; a letter at the
//! start of a word (after `/`, a space, `.` or `-`) scores higher. fzf alone
//! scores `mas` the same on every `MAIN/ARGOS/...` (`M` + `AS` of ARGOS), so a
//! term that spells the path's word initials in order ranks above the rest:
//! `mas` puts `MAIN/ARGOS/SIGNAL` first, `hgn` `HOME/GITHUB/NIMITZ_DEV`.
//!
//! Noise toggles (#14, `Toggles`, kept with the sticky state): only the
//! rows with a live pane (on by default, the header counts what it hides),
//! the topic matched too (off by default: path, name and status word only),
//! no stowaways, this fleet's tab only, ships only.

use nimitz_core::{Id, Role, Snapshot};
use nimitz_mux::Pane;
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
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
    /// it shows in a live pane (one the multiplexer lists)
    pub has_pane: bool,
    pub stowaway: bool,
    pub ship: bool,
    /// the fleet tab its row is on
    pub tab: String,
    /// where its topic starts in `text` (chars), when it has one
    topic_at: Option<usize>,
}

/// What the finder leaves out (#14); each one key in the finder, all kept
/// with the sticky state (`app::Saved.finder`).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct Toggles {
    /// only rows with a live pane (ctrl-a)
    pub pane_only: bool,
    /// match the topic too (ctrl-t)
    pub topic: bool,
    /// no stowaways (ctrl-s)
    pub no_stowaways: bool,
    /// only the fleet tab shown (ctrl-f)
    pub this_fleet: bool,
    /// ships only: no admirals or commodores (ctrl-l)
    pub ships_only: bool,
}

impl Default for Toggles {
    fn default() -> Toggles {
        Toggles { pane_only: true, topic: false, no_stowaways: false, this_fleet: false, ships_only: false }
    }
}

impl Toggles {
    /// Nothing left out, the topic matched (the finder before #14).
    pub fn all() -> Toggles {
        Toggles { pane_only: false, topic: true, ..Toggles::default() }
    }

    /// The toggle a key in the finder flips (ctrl-a, ctrl-t, ctrl-s, ctrl-f, ctrl-l).
    pub fn flip(&mut self, key: char) -> bool {
        let t = match key {
            'a' => &mut self.pane_only,
            't' => &mut self.topic,
            's' => &mut self.no_stowaways,
            'f' => &mut self.this_fleet,
            'l' => &mut self.ships_only,
            _ => return false,
        };
        *t = !*t;
        true
    }

    /// The header's state words: `[pane] [no topic]`, and each other toggle
    /// that is on (`[MAIN only]` while it keeps a fleet's tab: `tab`).
    pub fn words(&self, tab: Option<&str>) -> String {
        let mut w = vec![if self.pane_only { "[pane]" } else { "[any pane]" }.to_string(), if self.topic { "[topic]" } else { "[no topic]" }.to_string()];
        if self.no_stowaways {
            w.push("[no stowaways]".into());
        }
        if let Some(tab) = tab.filter(|_| self.this_fleet) {
            w.push(format!("[{tab} only]"));
        }
        if self.ships_only {
            w.push("[ships only]".into());
        }
        w.join(" ")
    }
}

/// The finder's toggle keys, for its footer and the `?` panel.
pub const TOGGLE_KEYS: &[(&str, &str)] = &[("ctrl-a", "pane only"), ("ctrl-t", "topic"), ("ctrl-s", "stowaways"), ("ctrl-f", "this fleet"), ("ctrl-l", "ships only")];

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
        // the fourth field of a member is its topic
        let topic_at = ranges.get(3).map(|r| r.0);
        let mut initials = vec![];
        let mut prev = '/';
        for (i, ch) in text.chars().take(path_end).enumerate() {
            if matches!(prev, '/' | '_') && ch.is_alphanumeric() {
                initials.extend(ch.to_lowercase().map(|c| (c, i as u32)));
            }
            prev = ch;
        }
        Cand { row_id, member, text, fields: ranges, initials, has_pane: false, stowaway: false, ship: false, tab: String::new(), topic_at }
    }

    /// The text a query matches: all of it, or without the topic.
    fn matched(&self, topic: bool) -> &str {
        match self.topic_at.filter(|_| !topic) {
            // the two spaces before the topic go too
            Some(at) => {
                let end = self.text.char_indices().nth(at.saturating_sub(2)).map(|(b, _)| b).unwrap_or(self.text.len());
                &self.text[..end]
            }
            None => &self.text,
        }
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
    candidates_in(snap, &[])
}

/// `candidates`, each marked with whether it shows in one of `panes`.
pub fn candidates_in(snap: &Snapshot, panes: &[Pane]) -> Vec<Cand> {
    let mut out = vec![];
    for tab in tabs(snap) {
        for f in snap.fleets.iter().filter(|f| f.id.0 == tab) {
            for m in fleet_members(f) {
                let mut c = Cand::new(
                    format!("M:{}", m.id.0),
                    Some(m.id.clone()),
                    vec![
                        (m.id.0.clone(), bold(mono::FG)),
                        (m.expected_name.clone(), st(mono::SOFT)),
                        (status_word(m.status).to_string(), status_style(m.status)),
                        (m.topic.clone().unwrap_or_default(), st(mono::COMMENT)),
                    ],
                );
                c.has_pane = m.pane.as_deref().is_some_and(|id| panes.iter().any(|p| p.pane.pane_id == id));
                c.ship = m.role == Role::Ship;
                c.tab = tab.clone();
                out.push(c);
            }
        }
        for s in snap.stowaways.iter().filter(|s| stowaway_tab(snap, s) == tab) {
            let short = s.short_id.clone().unwrap_or_else(|| s.session_id.chars().take(8).collect());
            let live = s.pid.is_some();
            let mut c = Cand::new(
                format!("SW:{}", s.session_id),
                None,
                vec![
                    (short, bold(mono::WARN)),
                    (s.name.clone(), st(mono::SOFT)),
                    (format!("stowaway {}", if live { "live" } else { "stopped" }), if live { st(mono::WARN) } else { st(mono::COMMENT) }),
                ],
            );
            let name = if s.name.is_empty() { s.session_id.chars().take(8).collect() } else { s.name.clone() };
            c.has_pane = crate::data::pane_for(Some(&s.session_id), &name, Some(&s.cwd), panes).is_some();
            c.stowaway = true;
            c.tab = tab.clone();
            out.push(c);
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
    pub toggles: Toggles,
    /// the fleet tab shown when it opened (`Toggles::this_fleet`)
    pub tab: Option<String>,
    /// what the toggles leave out, each counted once, in this order: other
    /// fleets, leads, stowaways, rows without a pane
    pub hidden: Hidden,
    matcher: Matcher,
}

/// How many candidates each toggle leaves out (`Finder::hidden`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hidden {
    pub other_fleets: usize,
    pub leads: usize,
    pub stowaways: usize,
    pub no_pane: usize,
    /// of `no_pane`, those the query matches (ctrl-a would show them)
    pub no_pane_matching: usize,
}

impl Hidden {
    /// `· 28 without a pane`, a part for each toggle that hides any.
    pub fn text(&self) -> String {
        let parts = [(self.no_pane, "without a pane"), (self.stowaways, "stowaways"), (self.leads, "leads"), (self.other_fleets, "in other fleets")];
        parts.iter().filter(|(n, _)| *n > 0).map(|(n, w)| format!(" \u{b7} {n} {w}")).collect()
    }
}

impl Finder {
    /// Every candidate, the topic matched (no toggle on).
    pub fn new(snap: &Snapshot) -> Finder {
        Finder::with(snap, &[], None, Toggles::all())
    }

    /// The finder Admiralty opens: the candidates marked against `panes`,
    /// `tab` the fleet tab shown, `toggles` as the user left them.
    pub fn with(snap: &Snapshot, panes: &[Pane], tab: Option<&str>, toggles: Toggles) -> Finder {
        let mut f = Finder {
            query: String::new(),
            sel: 0,
            cands: candidates_in(snap, panes),
            hits: vec![],
            toggles,
            tab: tab.map(String::from),
            hidden: Hidden::default(),
            matcher: Matcher::new(Config::DEFAULT),
        };
        f.refilter();
        f
    }

    /// Flip the toggle `key` names (`Toggles::flip`), then match again.
    pub fn flip(&mut self, key: char) -> bool {
        let flipped = self.toggles.flip(key);
        if flipped {
            self.refilter();
        }
        flipped
    }

    /// A refresh: the new candidates, the selection kept on its row.
    pub fn set_snapshot(&mut self, snap: &Snapshot, panes: &[Pane]) {
        let keep = self.selected().map(|c| c.row_id.clone());
        self.cands = candidates_in(snap, panes);
        self.refilter();
        if let Some(i) = keep.and_then(|id| self.hits.iter().position(|h| self.cands[h.cand].row_id == id)) {
            self.sel = i;
        }
    }

    /// Match the query again; the selection goes to the best.
    pub fn refilter(&mut self) {
        // fzf's syntax, term by term ('exact, ^prefix, suffix$, !not, `\ ` a
        // space); a term with nothing after its operator (a `!` half typed) is
        // left out until it has
        let pat = Pattern::parse(&self.query, CaseMatching::Ignore, Normalization::Smart);
        let terms: Vec<(Option<String>, &nucleo_matcher::pattern::Atom)> = pat
            .atoms
            .iter()
            .map(|a| {
                let text: String = a.needle_text().chars().collect();
                let plain = !a.negative && a.kind == AtomKind::Fuzzy && text.chars().all(char::is_alphanumeric);
                (plain.then(|| text.to_lowercase()), a)
            })
            .collect();
        let mut buf = vec![];
        let mut hits = vec![];
        let (t, mut hidden) = (self.toggles, Hidden::default());
        'cand: for (i, c) in self.cands.iter().enumerate() {
            // the toggles first, each hidden row counted once
            let fleet = self.fleet_only();
            // hidden by the pane toggle alone (no other toggle hides it)
            let mut no_pane = false;
            let count = if fleet.is_some_and(|tab| tab != c.tab) {
                Some(&mut hidden.other_fleets)
            } else if t.ships_only && !c.ship && !c.stowaway {
                Some(&mut hidden.leads)
            } else if t.no_stowaways && c.stowaway {
                Some(&mut hidden.stowaways)
            } else if t.pane_only && !c.has_pane {
                no_pane = true;
                Some(&mut hidden.no_pane)
            } else {
                None
            };
            if let Some(n) = count {
                *n += 1;
                // would the query match it with a pane? (the empty list's
                // hint; an empty query matches every row)
                if no_pane {
                    let text = c.matched(t.topic);
                    if terms.iter().all(|(_, atom)| atom.score(Utf32Str::new(text, &mut buf), &mut self.matcher).is_some()) {
                        hidden.no_pane_matching += 1;
                    }
                }
                continue;
            }
            let (mut score, mut idx) = (0u32, vec![]);
            let text = c.matched(t.topic);
            // nucleo counts a non-ASCII line in graphemes (or bytes), the list
            // in chars: its letters are lit on ASCII lines only
            let ascii = text.is_ascii();
            for (plain, atom) in &terms {
                let mut own = vec![];
                let Some(s) = atom.indices(Utf32Str::new(text, &mut buf), &mut self.matcher, &mut own) else { continue 'cand };
                score += s as u32;
                if !ascii {
                    own.clear();
                }
                // the path's initials, in order: above any fzf score; all of
                // them (two or more: one letter is no acronym) above some
                if let Some((at, all)) = plain.as_deref().and_then(|t| c.acronym(t)) {
                    score += if all && c.initials.len() >= 2 { 2 * ACRONYM } else { ACRONYM };
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
        self.hidden = hidden;
        self.sel = 0;
    }

    /// The fleet tab `this_fleet` keeps: only when a fleet's tab is shown
    /// (not every fleet, not the unassigned stowaways).
    pub fn fleet_only(&self) -> Option<&str> {
        self.tab.as_deref().filter(|t| self.toggles.this_fleet && *t != crate::view::UNASSIGNED)
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
