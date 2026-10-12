//! The plan and run view every planning command prints (layout B, chosen by
//! the user on 2026-10-10). Reading order: one headline (what happened, then
//! the counts per action), one line per member grouped by fleet and squadron
//! in fixed columns (name, job verb, pane verb, why), then what needs you
//! (skips, refusals and warnings, one fix each), dim notes, then the prompt.
//! The raw command lines show with `-v` (or `NIMITZ_VERBOSE=1`); `--json` is
//! unchanged. Colour only on a terminal, never with `NO_COLOR`; without it
//! the verbs carry the meaning.
use crate::out;
use nimitz_core::{Id, Issue, Outcome, Plan, Role, Severity, Step};
use std::collections::BTreeMap;
use std::io::IsTerminal;

// ------------------------------------------------------------------ colour (theme A)

#[derive(Clone, Copy)]
pub struct Rgb(u8, u8, u8);
const FG: Rgb = Rgb(0xe6, 0xe6, 0xdc);
const STRONG: Rgb = Rgb(0xf8, 0xf8, 0xf2);
const MUTED: Rgb = Rgb(0xaa, 0xa5, 0x8f);
const FAINT: Rgb = Rgb(0x8a, 0x86, 0x72);
const NEEDS: Rgb = Rgb(0xff, 0x5c, 0x93);
const ERROR: Rgb = Rgb(0xfd, 0x97, 0x1f);
const WARN: Rgb = Rgb(0xd8, 0xc7, 0x66);
const BUSY: Rgb = Rgb(0xa6, 0xe2, 0x2e);
const ACCENT: Rgb = Rgb(0x66, 0xd9, 0xef);

/// Colour when the stream the view goes to is a terminal, `NO_COLOR` is
/// unset or empty and the terminal is not `dumb`; `NIMITZ_COLOR=1` forces it
/// (a capture), `NO_COLOR` still wins.
fn colour_on() -> bool {
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    if std::env::var("NIMITZ_COLOR").is_ok_and(|v| v == "1") {
        return !no_color;
    }
    let dumb = std::env::var("TERM").is_ok_and(|t| t == "dumb");
    let tty = if out::json_mode() { std::io::stderr().is_terminal() } else { std::io::stdout().is_terminal() };
    tty && !no_color && !dumb
}

struct Paint(bool);
impl Paint {
    fn c(&self, s: &str, c: Rgb, bold: bool) -> String {
        if !self.0 || s.is_empty() {
            return s.to_string();
        }
        format!("\x1b[{}38;2;{};{};{}m{s}\x1b[0m", if bold { "1;" } else { "" }, c.0, c.1, c.2)
    }
}

/// The colour of a verb (job or pane column).
fn verb_colour(v: &str) -> Rgb {
    match v {
        "wake" | "launch" | "fresh" | "create" | "write" | "bind" | "adopt" => BUSY,
        "relaunch" | "remove" | "archive" | "rename" | "warn" | "trust" => WARN,
        "reattach" | "open pane" | "focus" => ACCENT,
        "skip" => NEEDS,
        "refuse" | "stop" | "taken" => ERROR,
        _ => FAINT,
    }
}

// ------------------------------------------------------------------ the model

/// One member line: its job and pane verbs, why, where its pane goes.
pub struct Row {
    pub id: Option<Id>,
    pub job: String,
    pub pane: String,
    pub why: String,
}

/// What the plan says and does, ready to print.
pub struct View {
    /// `start MAIN`, `stop MAIN/ARGOS/CLEAN`
    pub head: String,
    /// what happened (`Claude Code updated to 2.1.295`), may be empty
    pub what: String,
    pub rows: Vec<Row>,
    /// a pane column (start, restore)
    pub panes: bool,
    /// skips, refusals and warnings, each with its fix
    pub needs: Vec<Issue>,
    /// the rest of the issues (Info)
    pub notes: Vec<Issue>,
}

/// Issues a member is skipped for: shown as `skip`, with their fix.
const SKIPS: [&str; 1] = ["attach.old"];

/// The verb of one raw step: what it does to its member.
pub fn step_verb(s: &Step) -> &'static str {
    let a = |x: &str| s.cmd.iter().any(|c| c == x);
    match s.cmd.first().map(String::as_str) {
        Some("write") => "write",
        Some("mkdir") => "create",
        Some("trust") => "trust",
        Some("archive") => "archive",
        Some("bind") => "bind",
        Some("pin") => "pin",
        Some("herdr") => {
            if a("focus") {
                "focus"
            } else if a("create") || a("split") {
                "open pane"
            } else if a("rename") {
                "rename"
            } else {
                "reattach"
            }
        }
        _ if s.cmd.get(1).is_some_and(|c| c == "stop" || c == "kill") => "stop",
        _ if s.cmd.get(1).is_some_and(|c| c == "rm") => "remove",
        _ if a("--bg") && a("--resume") => "wake",
        _ if a("--bg") => "launch",
        _ => "run",
    }
}

impl View {
    /// The view of a plan: a restore or start (its member lines, `found`), or
    /// any other plan (its steps, by member).
    pub fn of(p: &Plan) -> View {
        let (head, what) = match p.summary.split_once(": ") {
            Some((h, w)) if !p.found.is_empty() => (h.to_string(), w.to_string()),
            _ => (p.summary.clone(), String::new()),
        };
        let mut rows = vec![];
        let panes = !p.found.is_empty();
        if panes {
            for f in &p.found {
                let why = if f.place.is_empty() { f.found.clone() } else { format!("{} · {}", f.found, f.place) };
                rows.push(Row { id: Some(f.id.clone()), job: f.job.clone(), pane: f.pane.clone(), why });
            }
        } else {
            // a plain plan: one line per member its steps act on, in step
            // order; a trust is a line of its own (what it lets run is its why)
            let mut order: Vec<(String, Option<Id>)> = vec![];
            let mut by: BTreeMap<String, Vec<&Step>> = BTreeMap::new();
            for s in &p.steps {
                let mut k = s.id.as_ref().map(|i| i.0.clone()).unwrap_or_default();
                if s.cmd.first().is_some_and(|c| c == "trust") {
                    k = format!("{k}\u{1}trust {}", s.cmd.get(1).map(String::as_str).unwrap_or(""));
                }
                if !order.iter().any(|(o, _)| *o == k) {
                    order.push((k.clone(), s.id.clone()));
                }
                by.entry(k).or_default().push(s);
            }
            for (k, id) in order {
                let steps = &by[&k];
                let mut verbs: Vec<&str> = vec![];
                for s in steps.iter() {
                    let v = step_verb(s);
                    if !verbs.contains(&v) && v != "pin" && v != "bind" {
                        verbs.push(v);
                    }
                }
                // stop + remove + wake is a relaunch; remove + wake (a row gone) too
                let job = if verbs.contains(&"wake") && verbs.contains(&"remove") { "relaunch".to_string() } else { verbs.first().copied().unwrap_or("run").to_string() };
                let why = steps.iter().map(|s| s.why.as_str()).find(|w| !w.is_empty()).unwrap_or("").to_string();
                rows.push(Row { id, job, pane: String::new(), why });
            }
        }
        // the refused and the skipped get their line too, when the plan has none for them
        for i in p.refusals.iter().chain(p.warnings.iter()) {
            let Some(id) = &i.id else { continue };
            if rows.iter().any(|r| r.id.as_ref() == Some(id)) {
                continue;
            }
            let job = if i.severity == Severity::Error { "refuse" } else if SKIPS.contains(&i.code.as_str()) { "skip" } else { continue };
            rows.push(Row { id: Some(id.clone()), job: job.into(), pane: String::new(), why: i.message.clone() });
        }
        let (mut needs, mut notes) = (vec![], vec![]);
        for i in p.refusals.iter().chain(p.warnings.iter()) {
            if i.severity != Severity::Info || SKIPS.contains(&i.code.as_str()) {
                needs.push(i.clone());
            } else {
                notes.push(i.clone());
            }
        }
        View { head, what, rows, panes, needs, notes }
    }

    /// The headline's counts: an action per verb, in a fixed order.
    fn counts(&self) -> Vec<(usize, String, Rgb)> {
        let mut out = vec![];
        let n = |f: &dyn Fn(&Row) -> bool| self.rows.iter().filter(|r| f(r)).count();
        for (verb, words) in [
            ("relaunch", "to relaunch"),
            ("wake", "to wake"),
            ("launch", "to launch"),
            ("fresh", "to start fresh"),
            ("stop", "to stop"),
            ("remove", "to remove"),
            ("write", "to write"),
            ("create", "to create"),
            ("archive", "to archive"),
            ("trust", "to trust"),
        ] {
            let k = n(&|r: &Row| r.job == verb);
            if k > 0 {
                out.push((k, words.to_string(), verb_colour(verb)));
            }
        }
        for (verb, one, many) in [("reattach", "pane to reattach", "panes to reattach"), ("open pane", "pane to open", "panes to open"), ("focus", "pane to focus", "panes to focus")] {
            let k = n(&|r: &Row| r.pane == verb);
            if k > 0 {
                out.push((k, if k == 1 { one } else { many }.to_string(), ACCENT));
            }
        }
        for (verb, words, c) in [("skip", "skipped", NEEDS), ("refuse", "refused", ERROR)] {
            let k = n(&|r: &Row| r.job == verb);
            if k > 0 {
                out.push((k, words.to_string(), c));
            }
        }
        out
    }
}

/// `MAIN`, `MAIN · ARGOS`: the group a member's line is under.
fn group(id: &Id) -> String {
    match id.squadron() {
        Some(sq) => format!("{} · {sq}", id.fleet()),
        None => id.fleet().to_string(),
    }
}

/// The member's name inside its group: `ADMIRAL`, `COMMODORE`, the ship's
/// name.
fn short(id: &Id) -> String {
    match id.role() {
        Role::Admiral => "ADMIRAL".into(),
        Role::Commodore => "COMMODORE".into(),
        Role::Ship => id.parts().last().copied().unwrap_or_default().to_string(),
    }
}

// ------------------------------------------------------------------ printing

/// The plan: headline, member lines, needs you, notes; `-v`: the commands.
pub fn plan(p: &Plan) {
    let v = View::of(p);
    let k = Paint(colour_on());
    let mut head = k.c(&v.head, STRONG, true);
    if !v.what.is_empty() {
        head.push_str(&k.c("  ·  ", FAINT, false));
        head.push_str(&k.c(&v.what, FG, true));
    }
    for (n, words, c) in v.counts() {
        head.push_str(&k.c("  ·  ", FAINT, false));
        head.push_str(&k.c(&format!("{n} {words}"), c, false));
    }
    out::line(&head);
    rows(&k, &v, None);
    needs(&k, &v.needs);
    for i in &v.notes {
        let who = i.id.as_ref().map(|x| format!("{x}: ")).unwrap_or_default();
        let fix = i.fix.as_ref().map(|f| format!("  ({f})")).unwrap_or_default();
        out::line(&k.c(&format!("· {who}{} ({}){fix}", i.message, i.code), FAINT, false));
    }
    if out::verbose() {
        // the detail: the plan's summary, each member's full id, what was
        // found and what it does, then the commands
        out::line("");
        out::line(&k.c(&format!("[PLAN] {}", p.summary), FAINT, false));
        if !p.found.is_empty() {
            out::line("");
            out::line(&k.c("found:", FAINT, true));
            let w = p.found.iter().map(|f| f.id.0.chars().count()).max().unwrap_or(0);
            for f in &p.found {
                out::line(&k.c(&format!("    {:<w$}  {} -> {}", f.id.0, f.found, f.action), FAINT, false));
            }
        }
        if !p.steps.is_empty() {
            out::line("");
            out::line(&k.c("commands:", FAINT, true));
            out::steps(p);
        }
    }
}

/// The member lines, grouped; with `ran`, a mark in front of each (`✓`,
/// `✗`, `–`) and only the members something was done or skipped for.
fn rows(k: &Paint, v: &View, ran: Option<&BTreeMap<String, char>>) {
    let shown: Vec<&Row> = v.rows.iter().filter(|r| ran.is_none() || r.job != "nothing" || !r.pane.is_empty()).collect();
    if shown.is_empty() {
        return;
    }
    let w = shown.iter().map(|r| r.id.as_ref().map(short).unwrap_or_default().chars().count()).max().unwrap_or(0).max(9);
    let lead = if ran.is_some() { 2 } else { 0 };
    out::line("");
    if v.panes {
        out::line(&k.c(&format!("{}  {:<w$}  {:<10}{:<11}why", " ".repeat(lead), "", "job", "pane"), FAINT, false));
    }
    let mut last: Option<String> = None;
    for r in shown {
        let g = r.id.as_ref().map(group);
        if g != last {
            if let Some(g) = &g {
                out::line(&format!("{}{}", " ".repeat(lead), k.c(g, MUTED, true)));
            }
            last = g;
        }
        let quiet = r.job == "nothing" && r.pane.is_empty();
        let mark = match ran.and_then(|m| r.id.as_ref().and_then(|i| m.get(&i.0)).copied()) {
            Some('✓') => format!("{} ", k.c("✓", BUSY, true)),
            Some('✗') => format!("{} ", k.c("✗", ERROR, true)),
            Some(c) => format!("{} ", k.c(&c.to_string(), NEEDS, true)),
            None if ran.is_some() => "  ".to_string(),
            None => String::new(),
        };
        let name = r.id.as_ref().map(short).unwrap_or_else(|| "(config)".into());
        let job = k.c(&format!("{:<10}", r.job), verb_colour(&r.job), !quiet);
        let pane = if v.panes {
            if r.pane.is_empty() { k.c(&format!("{:<11}", "·"), FAINT, false) } else { k.c(&format!("{:<11}", r.pane), verb_colour(&r.pane), true) }
        } else {
            String::new()
        };
        let name = k.c(&format!("{name:<w$}  "), if quiet { FG } else { STRONG }, !quiet);
        out::line(&format!("{mark}  {name}{job}{pane}{}", k.c(&r.why, if quiet { FAINT } else { MUTED }, false)));
    }
}

/// What needs you: each skip, refusal or warning, with its one fix.
fn needs(k: &Paint, needs: &[Issue]) {
    if needs.is_empty() {
        return;
    }
    out::line("");
    out::line(&k.c("needs you", NEEDS, true));
    for i in needs {
        let verb = if SKIPS.contains(&i.code.as_str()) {
            "skip"
        } else if i.severity == Severity::Error {
            "refuse"
        } else {
            "warn"
        };
        let who = i.id.as_ref().map(|x| format!("{x}  ")).unwrap_or_default();
        // (the member is named in front: a message that starts with it drops it)
        let msg = i.id.as_ref().and_then(|x| i.message.strip_prefix(&format!("{x} "))).unwrap_or(&i.message);
        out::line(&format!("  {}{}{} {}", k.c(&format!("{verb:<8}"), verb_colour(verb), true), k.c(&who, STRONG, true), msg, k.c(&format!("({})", i.code), FAINT, false)));
        if let Some(fix) = &i.fix {
            out::line(&format!("          {}{}", k.c("fix: ", FAINT, false), k.c(fix, ACCENT, false)));
        }
    }
}

/// The question before a run.
pub fn prompt(steps: usize) -> String {
    let k = Paint(colour_on());
    format!("{} [y/N] ", k.c(&format!("Run {steps} step{}?", if steps == 1 { "" } else { "s" }), STRONG, true))
}

/// The last line of a `--plan`.
pub fn plan_footer(steps: usize) {
    if steps == 0 {
        return;
    }
    let k = Paint(colour_on());
    out::line("");
    let which = if steps == 1 { "this step".to_string() } else { format!("these {steps} steps") };
    out::line(&k.c(&format!("--yes runs {which}; -v shows the commands"), FAINT, false));
}

/// The run: the plan's member lines with a mark each (✓ done, ✗ failed, –
/// skipped), a fix under a failure, then what needs you.
pub fn run(p: &Plan, o: &Outcome, failed: &[Id]) {
    let v = View::of(p);
    let k = Paint(colour_on());
    let mut marks: BTreeMap<String, char> = BTreeMap::new();
    for r in &v.rows {
        let Some(id) = &r.id else { continue };
        let ran_bad = o.ran.iter().any(|(s, rc, _)| s.id.as_ref() == Some(id) && *rc != 0);
        let mark = if failed.contains(id) || ran_bad || r.job == "refuse" {
            '✗'
        } else if r.job == "skip" {
            '–'
        } else {
            '✓'
        };
        marks.insert(id.0.clone(), mark);
    }
    rows(&k, &v, Some(&marks));
    let issues: Vec<Issue> = o.issues.iter().filter(|i| i.severity != Severity::Info || SKIPS.contains(&i.code.as_str())).cloned().collect();
    needs(&k, &issues);
    out::line("");
    if out::verbose() {
        out::line(&k.c("commands:", FAINT, true));
        out::ran_lines(o);
        if o.verified && failed.is_empty() && !p.found.is_empty() {
            out::ok("verified by re-reading");
        }
    }
}

/// `[OK]` / `[ERROR]` for a result line, coloured.
pub fn verdict(ok: bool, text: &str) {
    let k = Paint(colour_on());
    if ok {
        out::line(&format!("{} {text}", k.c("[OK]", BUSY, true)));
    } else {
        out::line(&format!("{} {text}", k.c("[ERROR]", ERROR, true)));
    }
}
