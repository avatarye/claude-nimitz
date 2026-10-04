//! Output lines (`[OK]`, `[INFO]`, `[WARN]`, `[ERROR]`, no emoji), plan printing and the
//! confirm gate every mutating command goes through.
use nimitz_core::{Issue, Outcome, Plan, Severity, Step};
use std::io::{BufRead, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static JSON: AtomicBool = AtomicBool::new(false);

/// In `--json` mode stdout carries only JSON; the human lines go to stderr.
pub fn set_json(on: bool) {
    JSON.store(on, Ordering::Relaxed);
}

pub fn json_mode() -> bool {
    JSON.load(Ordering::Relaxed)
}

pub fn line(s: &str) {
    if json_mode() {
        eprintln!("{s}");
    } else {
        println!("{s}");
    }
}

pub fn ok(s: &str) {
    line(&format!("[OK] {s}"));
}

pub fn warn(s: &str) {
    line(&format!("[WARN] {s}"));
}

pub fn error(s: &str) {
    line(&format!("[ERROR] {s}"));
}

pub fn json<T: serde::Serialize>(v: &T) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_else(|_| "null".into()));
}

/// Quote one argument for a POSIX shell, only when it needs it.
pub fn quote(a: &str) -> String {
    let plain = !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,~[]".contains(c));
    if plain {
        a.to_string()
    } else {
        format!("'{}'", a.replace('\'', "'\\''"))
    }
}

pub fn argv(cmd: &[String]) -> String {
    cmd.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
}

fn tag(sev: Severity) -> &'static str {
    match sev {
        Severity::Info => "[INFO]",
        Severity::Warn => "[WARN]",
        Severity::Error => "[ERROR]",
    }
}

pub fn issue(i: &Issue) {
    let who = i.id.as_ref().map(|x| format!("{x}: ")).unwrap_or_default();
    line(&format!("{} {}{} ({})", tag(i.severity), who, i.message, i.code));
    if let Some(fix) = &i.fix {
        line(&format!("       fix: {fix}"));
    }
}

/// How a step reads in a plan: internal file steps short, commands as argv.
fn step_text(s: &Step) -> String {
    match s.cmd.first().map(String::as_str) {
        Some("write") => format!("write {}", s.cmd.get(1).map(String::as_str).unwrap_or("")),
        Some("mkdir") | Some("archive") | Some("bind") => s.cmd.join(" "),
        _ => argv(&s.cmd),
    }
}

pub fn plan(p: &Plan) {
    line(&format!("[PLAN] {}", p.summary));
    for (n, s) in p.steps.iter().enumerate() {
        let cwd = s.cwd.as_ref().map(|c| format!("cd {} && ", quote(c))).unwrap_or_default();
        line(&format!("  {}. {cwd}{}", n + 1, step_text(s)));
        line(&format!("     why: {}", s.why));
    }
    for w in &p.warnings {
        issue(w);
    }
    for r in &p.refusals {
        issue(r);
    }
}

pub enum Go {
    Run,
    Exit(i32),
}

/// The gate before any change: `--plan` prints and stops; a refusal stops;
/// `--yes` runs; a terminal asks y/N; anything else needs `--yes`.
pub fn gate(p: &Plan, plan_only: bool, yes: bool) -> Go {
    if plan_only {
        if json_mode() {
            json(p);
        } else {
            plan(p);
        }
        return Go::Exit(if p.ok() { 0 } else { 1 });
    }
    if !p.ok() {
        plan(p);
        if json_mode() {
            json(p);
        }
        return Go::Exit(1);
    }
    if p.steps.is_empty() {
        for w in &p.warnings {
            issue(w);
        }
        ok(&format!("{}: nothing to do", p.summary));
        return Go::Exit(0);
    }
    plan(p);
    if yes {
        return Go::Run;
    }
    if !std::io::stdin().is_terminal() {
        error("not a terminal: pass --yes to run this plan, or --plan to only print it");
        return Go::Exit(1);
    }
    eprint!("Run this plan? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().lock().read_line(&mut answer);
    if matches!(answer.trim(), "y" | "Y" | "yes") {
        Go::Run
    } else {
        warn("cancelled; nothing changed");
        Go::Exit(1)
    }
}

/// Print what ran and what the re-read found. Exit code 0 only when verified.
pub fn outcome(o: &Outcome) -> i32 {
    for (s, rc, text) in &o.ran {
        let text = text.trim();
        let detail = if text.is_empty() || text.starts_with("[OK]") { String::new() } else { format!(" -> {}", first_line(text)) };
        if *rc == 0 {
            ok(&format!("{}{detail}", step_text(s)));
        } else {
            error(&format!("{} (exit {rc}){detail}", step_text(s)));
        }
    }
    for i in &o.issues {
        issue(i);
    }
    if json_mode() {
        json(o);
    }
    if o.verified {
        ok("verified by re-reading");
        0
    } else {
        error("not verified; see above");
        1
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_is_minimal_and_safe() {
        assert_eq!(quote("--bg"), "--bg");
        assert_eq!(quote("[MAIN.ARGOS]-SHIP"), "[MAIN.ARGOS]-SHIP");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
    }
}
