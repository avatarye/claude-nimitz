//! An aide's git, run by nimitz, never by the aide (aide.md 6.3, the S4
//! decision with the KB's skills): `nimitz aide grep` and `nimitz aide
//! status` read its folder; `nimitz inbox done --commit` stages the paths the
//! aide names and commits them with the item's trailer. The aide's own
//! settings deny every git command, so a payload cannot talk it into one, and
//! nimitz runs git with every option that could run a program turned off.
use std::path::{Component, Path};
use std::process::Command;

/// What an aide never writes in its repo (the KB's own rules; `.git` too).
pub const OFF_LIMITS: [&str; 8] = [".git/", "agent/", "CLAUDE.md", "START_HERE.md", "SCOPE.md", "DESIGN-NOTES.md", "README.md", ".gitignore"];

/// The most lines `grep` prints: an aide's context is not a dump.
pub const GREP_MAX_LINES: usize = 400;

/// git in `dir` with nothing that runs a program: no fsmonitor, no pager, no
/// hooks, no external diff or textconv, no signing, no editor.
fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(dir);
    let hooks = if cfg!(windows) { "core.hooksPath=NUL" } else { "core.hooksPath=/dev/null" };
    for kv in ["core.fsmonitor=false", "core.pager=cat", hooks, "diff.external=", "commit.gpgSign=false", "core.editor=true", "color.ui=false", "core.untrackedCache=false"] {
        c.arg("-c").arg(kv);
    }
    c.env("GIT_PAGER", "cat").env("GIT_TERMINAL_PROMPT", "0").env_remove("GIT_EXTERNAL_DIFF");
    c
}

fn run(mut c: Command) -> Result<String, String> {
    let o = c.output().map_err(|e| format!("git: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

/// A path inside the repo, as the aide names it: relative, no `..`, no
/// leading `/`, written with `/`.
fn clean_rel(p: &str) -> Result<String, String> {
    let p = p.trim().replace('\\', "/");
    if p.is_empty() {
        return Err("an empty path".into());
    }
    let path = Path::new(&p);
    if path.is_absolute() || p.starts_with('/') {
        return Err(format!("{p:?} is not relative to the repo"));
    }
    if path.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(format!("{p:?} leaves the repo"));
    }
    Ok(p.trim_start_matches("./").to_string())
}

/// `nimitz aide grep <term> [<prefix>]`: `git grep -n -i -F` in the folder's
/// tracked files, under `prefix` when given (at most `GREP_MAX_LINES` lines).
pub fn grep(dir: &Path, term: &str, prefix: Option<&str>) -> Result<String, String> {
    if term.trim().is_empty() {
        return Err("grep needs a term".into());
    }
    let mut c = git(dir);
    c.args(["grep", "-n", "-i", "-F", "--no-color", "-e", term, "--"]);
    if let Some(p) = prefix {
        c.arg(clean_rel(p)?);
    }
    let o = c.output().map_err(|e| format!("git: {e}"))?;
    // exit 1: no match
    match o.status.code() {
        Some(0) | Some(1) => {}
        _ => return Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
    }
    let text = String::from_utf8_lossy(&o.stdout);
    let lines: Vec<&str> = text.lines().collect();
    let mut out = lines.iter().take(GREP_MAX_LINES).copied().collect::<Vec<_>>().join("\n");
    if lines.len() > GREP_MAX_LINES {
        out.push_str(&format!("\n... {} more lines: narrow the term or the path", lines.len() - GREP_MAX_LINES));
    }
    Ok(out)
}

/// `nimitz aide status`: `git status --porcelain` of the folder (the user's
/// uncommitted edits, Dropbox's conflicted copies).
pub fn status(dir: &Path) -> Result<String, String> {
    let mut c = git(dir);
    c.args(["status", "--porcelain", "--untracked-files=all"]);
    run(c)
}

/// The commit that already carries this item (`inbox-item: <ref>`), if any:
/// an earlier pass committed and died before `done`.
pub fn committed(dir: &Path, reference: &str) -> Result<Option<String>, String> {
    let pat: String = reference.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == ':' { ch.to_string() } else { format!("\\{ch}") }).collect();
    let mut c = git(dir);
    c.args(["log", "-E", "--format=%H", "-1", &format!("--grep=^inbox-item: {pat}$")]);
    Ok(run(c)?.lines().next().map(str::to_string).filter(|s| !s.is_empty()))
}

/// Check the paths an aide wants committed: inside the repo, none off
/// limits, none an existing file under `raw/` (a capture is always new).
pub fn check_paths(dir: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    if paths.is_empty() {
        return Err("--paths names nothing to commit".into());
    }
    let mut out = vec![];
    for p in paths {
        let rel = clean_rel(p)?;
        if let Some(off) = OFF_LIMITS.iter().find(|o| if o.ends_with('/') { rel.starts_with(*o) || rel == o.trim_end_matches('/') } else { rel == **o }) {
            return Err(format!("{rel}: off limits to an aide ({off})"));
        }
        if rel.starts_with("raw/") {
            let mut c = git(dir);
            c.args(["ls-files", "--error-unmatch", "--", &rel]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
            if c.status().is_ok_and(|s| s.success()) {
                return Err(format!("{rel}: an existing raw/ file is never changed (a capture is always a new file)"));
            }
        }
        out.push(rel);
    }
    Ok(out)
}

/// Stage exactly `paths` and commit only them (`--only`: whatever else the
/// user staged stays staged, uncommitted), with the item's trailer. The
/// commit's author is the repo's own; with none set, the aide's id.
pub fn commit(dir: &Path, aide: &str, reference: &str, title: &str, from: &str, paths: &[String]) -> Result<String, String> {
    let paths = check_paths(dir, paths)?;
    let title: String = title.lines().next().unwrap_or("").chars().take(120).collect();
    let title = if title.trim().is_empty() { "a note".to_string() } else { title };
    let mut add = git(dir);
    add.args(["add", "--"]).args(&paths);
    run(add)?;
    let mut c = git(dir);
    let mut who = git(dir);
    who.args(["config", "--get", "user.email"]);
    if run(who).map(|s| s.trim().is_empty()).unwrap_or(true) {
        let me = aide.replace('/', ".").replace('@', "");
        c.args(["-c", &format!("user.name={aide}"), "-c", &format!("user.email={me}@nimitz.local")]);
    }
    c.args(["commit", "--only", "--no-verify", "-m", &format!("ingest(auto): {title}"), "-m", &format!("inbox-item: {reference}"), "-m", &format!("from: {from}"), "--"]).args(&paths);
    run(c)?;
    let mut head = git(dir);
    head.args(["rev-parse", "HEAD"]);
    Ok(run(head)?.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let td = tempfile::tempdir().unwrap();
        let g = |args: &[&str]| assert!(Command::new("git").arg("-C").arg(td.path()).args(args).output().unwrap().status.success(), "{args:?}");
        g(&["init", "-q"]);
        g(&["config", "user.email", "t@t"]);
        g(&["config", "user.name", "t"]);
        std::fs::create_dir_all(td.path().join("wiki")).unwrap();
        std::fs::create_dir_all(td.path().join("raw")).unwrap();
        std::fs::write(td.path().join("CLAUDE.md"), "rules\n").unwrap();
        std::fs::write(td.path().join("raw/old.md"), "old capture\n").unwrap();
        std::fs::write(td.path().join("wiki/page.md"), "Alpha beta\n").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "-qm", "init"]);
        td
    }

    #[test]
    fn grep_and_status_read_the_folder() {
        let td = repo();
        assert_eq!(grep(td.path(), "alpha", Some("wiki")).unwrap(), "wiki/page.md:1:Alpha beta");
        assert_eq!(grep(td.path(), "nothing-here", None).unwrap(), "");
        assert!(grep(td.path(), "x", Some("../etc")).is_err());
        std::fs::write(td.path().join("wiki/page.md"), "edited\n").unwrap();
        assert!(status(td.path()).unwrap().contains(" M wiki/page.md"));
    }

    #[test]
    fn a_commit_takes_only_the_named_paths_once_with_the_trailer() {
        let td = repo();
        let d = td.path();
        // the user's own edit, staged: it stays out of the aide's commit
        std::fs::write(d.join("wiki/page.md"), "the user's edit\n").unwrap();
        Command::new("git").arg("-C").arg(d).args(["add", "wiki/page.md"]).output().unwrap();
        std::fs::write(d.join("raw/cap-1.md"), "capture\n").unwrap();
        std::fs::write(d.join("wiki/new.md"), "new page\n").unwrap();
        let paths = vec!["raw/cap-1.md".to_string(), "./wiki/new.md".to_string()];
        assert_eq!(committed(d, "u1:000000000001").unwrap(), None);
        let sha = commit(d, "HOME/@KB", "u1:000000000001", "A title\nsecond line", "HOME/GITHUB/X", &paths).unwrap();
        let show = |args: &[&str]| String::from_utf8(Command::new("git").arg("-C").arg(d).args(args).output().unwrap().stdout).unwrap();
        assert_eq!(show(&["show", "--name-only", "--format=", &sha]).trim(), "raw/cap-1.md\nwiki/new.md");
        assert!(show(&["log", "-1", "--format=%B"]).contains("ingest(auto): A title\n\ninbox-item: u1:000000000001\n\nfrom: HOME/GITHUB/X"));
        assert!(show(&["diff", "--cached", "--name-only"]).contains("wiki/page.md"), "the user's staged edit is still staged");
        assert_eq!(committed(d, "u1:000000000001").unwrap(), Some(sha));
        assert_eq!(committed(d, "u1:00000000000"), Ok(None), "anchored: no prefix match");
    }

    #[test]
    fn off_limits_and_existing_raw_paths_are_refused() {
        let td = repo();
        for p in ["CLAUDE.md", "agent/skills/x.md", ".git/config", "../outside.md", "/etc/passwd", "raw/old.md", ".gitignore"] {
            assert!(check_paths(td.path(), &[p.to_string()]).is_err(), "{p}");
        }
        assert!(check_paths(td.path(), &[]).is_err());
        assert!(check_paths(td.path(), &["wiki/a.md".into(), "raw/new.md".into()]).is_ok());
    }
}
