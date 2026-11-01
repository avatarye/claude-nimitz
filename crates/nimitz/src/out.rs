//! Output lines (`[OK]`, `[INFO]`, `[WARN]`, `[ERROR]`, no emoji), plan printing and the
//! confirm gate every mutating command goes through.
use nimitz_core::{Issue, Outcome, Plan, Severity, Step};
use std::io::{BufRead, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static JSON: AtomicBool = AtomicBool::new(false);
static VERBOSE: AtomicBool = AtomicBool::new(false);

/// `-v` (`--verbose`, `NIMITZ_VERBOSE=1`): plans and runs show their commands.
pub fn set_verbose(on: bool) {
    VERBOSE.store(on, Ordering::Relaxed);
}

pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

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
        Some("mkdir") | Some("archive") | Some("bind") | Some("trust") => s.cmd.join(" "),
        _ => argv(&s.cmd),
    }
}

/// A step in a few words: file steps as in the plan; a command as its program
/// name and its leading words, up to the first flag (`claude stop 1a2b3c4d`,
/// `claude --bg`). The plan above already showed the full command.
fn brief(s: &Step) -> String {
    match s.cmd.first().map(String::as_str) {
        Some("write") | Some("mkdir") | Some("archive") | Some("bind") | Some("trust") | None => step_text(s),
        Some(prog) => {
            let name = std::path::Path::new(prog).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| prog.to_string());
            let mut words = vec![name];
            for a in s.cmd.iter().skip(1) {
                if a.starts_with('-') {
                    if a == "--bg" {
                        words.push(a.clone());
                    }
                    break;
                }
                words.push(quote(a));
            }
            words.join(" ")
        }
    }
}

/// A plan, as the view prints it (`view::plan`).
pub fn plan(p: &Plan) {
    crate::view::plan(p);
}

/// The plan's raw steps, numbered, each with its why (`-v`).
pub fn steps(p: &Plan) {
    for (n, s) in p.steps.iter().enumerate() {
        let cwd = s.cwd.as_ref().map(|c| format!("cd {} && ", quote(c))).unwrap_or_default();
        line(&format!("  {}. {cwd}{}", n + 1, step_text(s)));
        line(&format!("     why: {}", s.why));
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
            if p.ok() {
                crate::view::plan_footer(p.steps.len());
            }
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
        plan(p);
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
    eprint!("{}", crate::view::prompt(p.steps.len()));
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

/// What ran, one short line per step (`-v`), no issues and no verdict.
pub fn ran_lines(o: &Outcome) {
    for (s, rc, text) in &o.ran {
        let text = text.trim();
        let detail = if text.is_empty() || text.starts_with("[OK]") { String::new() } else { format!(" -> {}", first_line(text)) };
        let who = s.id.as_ref().map(|i| format!("{i}: ")).unwrap_or_default();
        if *rc == 0 {
            ok(&format!("{who}{}{detail}", brief(s)));
        } else {
            error(&format!("{who}{} (exit {rc}){detail}", brief(s)));
        }
    }
}

/// The run of plan `p`: its member lines marked (`view::run`), then whether
/// the re-read verified it. Exit code 0 only when verified.
pub fn outcome(p: &Plan, o: &Outcome) -> i32 {
    crate::view::run(p, o, &[]);
    if json_mode() {
        json(o);
    }
    if o.verified {
        crate::view::verdict(true, "verified by re-reading");
        0
    } else {
        crate::view::verdict(false, "not verified; see above");
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

/// The width human lines fit in, on a terminal only: `COLUMNS` when set,
/// else the terminal's own; none for a pipe or `--json` (a line is never cut
/// for a file, a script or an agent reading it). `NIMITZ_COLUMNS` sets it
/// anywhere (tests).
pub fn width() -> Option<usize> {
    let cols = |k: &str| std::env::var(k).ok().and_then(|c| c.trim().parse::<usize>().ok()).filter(|c| *c > 0);
    if let Some(c) = cols("NIMITZ_COLUMNS") {
        return Some(c);
    }
    if json_mode() || !std::io::stdout().is_terminal() {
        return None;
    }
    cols("COLUMNS").or_else(terminal_width)
}

#[cfg(unix)]
fn terminal_width() -> Option<usize> {
    // SAFETY: TIOCGWINSZ fills the winsize it is given and reads nothing else
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let r = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    (r == 0 && ws.ws_col > 0).then_some(ws.ws_col as usize)
}

#[cfg(windows)]
fn terminal_width() -> Option<usize> {
    use windows_sys::Win32::System::Console::{GetConsoleScreenBufferInfo, GetStdHandle, CONSOLE_SCREEN_BUFFER_INFO, STD_OUTPUT_HANDLE};
    // SAFETY: the call fills the info it is given, for the process's own stdout
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) };
    let w = (info.srWindow.Right - info.srWindow.Left + 1) as i32;
    (ok != 0 && w > 0).then_some(w as usize)
}

/// `s` cut to `n` terminal cells (a wide character takes two), `...` marking the cut.
pub fn clip(s: &str, n: usize) -> String {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    if s.width() <= n {
        return s.to_string();
    }
    let room = n.saturating_sub(3);
    let mut used = 0;
    let kept: String = s
        .chars()
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= room
        })
        .collect();
    kept.trim_end().to_string() + "..."
}
