//! An aide's git, run by nimitz, never by the aide (aide.md 6.3, the S4
//! decision with the KB's skills): `nimitz aide grep` and `nimitz aide
//! status` read its folder; `nimitz inbox done --commit` stages the paths the
//! aide names and commits them with the item's trailer. The aide's own
//! settings deny every git command, so a payload cannot talk it into one, and
//! nimitz runs git with every option that could run a program turned off,
//! but for the filters the user's own committed attributes name (git-lfs, say:
//! a checkout of a reader's snapshot runs them, as any checkout would; an
//! attributes file not committed stops nimitz's git, `stray_attributes`).
use std::path::{Component, Path};
use std::process::Command;

/// What an aide never writes in its repo: the KB's own rules (top level),
/// and, at any depth, what makes git or Claude Code act on its own (`.git`,
/// attributes that name a filter program, modules, a CLAUDE.md that becomes
/// instructions at the next session start). Compared without case: on macOS
/// and Windows `claude.md` is `CLAUDE.md`.
pub const OFF_LIMITS: [&str; 7] = ["agent/", "CLAUDE.md", "START_HERE.md", "SCOPE.md", "DESIGN-NOTES.md", "README.md", "CLAUDE.local.md"];
/// Never written at any depth.
pub const OFF_LIMITS_ANYWHERE: [&str; 7] = [".git", ".gitattributes", ".gitmodules", ".gitignore", "CLAUDE.md", ".claude", ".mcp.json"];

/// The most lines `grep` prints, each at most `LINE_MAX` characters: an
/// aide's context is not a dump.
pub const GREP_MAX_LINES: usize = 400;
const LINE_MAX: usize = 300;

/// git in `dir` with nothing that runs a program: no fsmonitor, no pager, no
/// hooks, no external diff or textconv, no signing, no editor, no global
/// attributes; every path literal (`--literal-pathspecs`: no glob, no `:(magic)`);
/// none of the caller's `GIT_*` steering it elsewhere.
fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("--literal-pathspecs").arg("-C").arg(dir);
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let hooks = format!("core.hooksPath={null}");
    let attrs = format!("core.attributesFile={null}");
    for kv in ["core.fsmonitor=false", "core.pager=cat", hooks.as_str(), attrs.as_str(), "diff.external=", "commit.gpgSign=false", "log.showSignature=false", "core.editor=true", "color.ui=false", "core.untrackedCache=false"] {
        c.arg("-c").arg(kv);
    }
    c.env("GIT_PAGER", "cat").env("GIT_TERMINAL_PROMPT", "0");
    for v in ["GIT_EXTERNAL_DIFF", "GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_CONFIG", "GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM", "GIT_CONFIG_COUNT", "GIT_CONFIG_PARAMETERS", "GIT_ATTR_SOURCE", "GIT_OBJECT_DIRECTORY", "GIT_NAMESPACE"] {
        c.env_remove(v);
    }
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
    if path.is_absolute() || p.starts_with('/') || p.as_bytes().get(1) == Some(&b':') {
        return Err(format!("{p:?} is not relative to the repo"));
    }
    if path.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(format!("{p:?} leaves the repo"));
    }
    let rel = p.trim_start_matches("./").trim_end_matches('/').to_string();
    if rel.is_empty() || rel == "." {
        return Err("the repo itself is no path to commit: name each file".into());
    }
    Ok(rel)
}

/// Does `rel`'s folder lie inside the repo (no symlinked folder leads out)?
fn inside(dir: &Path, rel: &str) -> Result<(), String> {
    let root = crate::os::canonical(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let parent = dir.join(rel).parent().map(Path::to_path_buf).unwrap_or_else(|| dir.to_path_buf());
    // a folder not made yet is inside when its nearest existing one is
    let mut p = parent.as_path();
    loop {
        if let Ok(c) = crate::os::canonical(p) {
            return if c.starts_with(&root) { Ok(()) } else { Err(format!("{rel}: its folder leads out of the repo")) };
        }
        match p.parent() {
            Some(up) => p = up,
            None => return Err(format!("{rel}: no folder")),
        }
    }
}

/// Is `rel` off limits (any case)?
pub fn off_limits(rel: &str) -> Option<String> {
    let low = rel.to_lowercase();
    if let Some(o) = OFF_LIMITS.iter().find(|o| {
        let o = o.to_lowercase();
        if o.ends_with('/') { low.starts_with(&o) || low == o.trim_end_matches('/') } else { low == o }
    }) {
        return Some(o.to_string());
    }
    low.split('/').find_map(|part| OFF_LIMITS_ANYWHERE.iter().find(|o| o.to_lowercase() == part).map(|o| o.to_string()))
}

fn clip_lines(text: &str, max: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = lines.iter().take(max).map(|l| if l.chars().count() > LINE_MAX { format!("{}...", l.chars().take(LINE_MAX).collect::<String>()) } else { l.to_string() }).collect::<Vec<_>>().join("\n");
    if lines.len() > max {
        out.push_str(&format!("\n... {} more lines: narrow it down", lines.len() - max));
    }
    out
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
    Ok(clip_lines(&String::from_utf8_lossy(&o.stdout), GREP_MAX_LINES))
}

/// An attributes or modules file (any depth) that differs from its last
/// commit, or that no commit has: a filter it names would run inside git
/// (`status` and `add` clean the files they compare). Found without any git
/// that reads attributes: a walk of the tree, then each file's blob as
/// written (`hash-object --no-filters`) against the index's (`ls-files -s`).
pub fn stray_attributes(dir: &Path) -> Option<String> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if name != ".git" {
                    stack.push(e.path());
                }
                continue;
            }
            if name != ".gitattributes" && name != ".gitmodules" {
                continue;
            }
            let rel = e.path().strip_prefix(dir).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let mut idx = git(dir);
            idx.args(["ls-files", "-s", "--"]).arg(&rel);
            let committed = run(idx).ok().and_then(|o| o.split_whitespace().nth(1).map(str::to_string));
            let mut h = git(dir);
            h.args(["hash-object", "--no-filters", "--"]).arg(&rel);
            let written = run(h).ok().map(|o| o.trim().to_string());
            if committed.is_none() || committed != written {
                return Some(rel);
            }
        }
    }
    None
}

/// `nimitz aide status`: `git status --porcelain` of the folder (the user's
/// uncommitted edits, Dropbox's conflicted copies).
pub fn status(dir: &Path) -> Result<String, String> {
    if let Some(stray) = stray_attributes(dir) {
        return Err(format!("{stray} is changed and not committed: git is not run until the user looks at it"));
    }
    let mut c = git(dir);
    c.args(["--no-optional-locks", "status", "--porcelain", "--untracked-files=all"]);
    Ok(clip_lines(&run(c)?, GREP_MAX_LINES))
}

/// The commit that already carries this item (`inbox-item: <ref>`), if any:
/// an earlier pass committed and died before `done`.
pub fn committed(dir: &Path, reference: &str) -> Result<Option<String>, String> {
    let pat: String = reference.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == ':' { ch.to_string() } else { format!("\\{ch}") }).collect();
    let mut c = git(dir);
    c.args(["log", "-E", "--format=%H", "-1", &format!("--grep=^inbox-item: {pat}$")]);
    Ok(run(c)?.lines().next().map(str::to_string).filter(|s| !s.is_empty()))
}

fn tracked(dir: &Path, rel: &str) -> bool {
    let mut c = git(dir);
    c.args(["ls-files", "--error-unmatch", "--", rel]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    c.status().is_ok_and(|s| s.success())
}

/// Check the paths an aide wants committed (or restored): files inside the
/// repo, named one by one (a file, or a tracked file it deleted; never a
/// folder), none off limits, none an existing file under `raw/` (a capture
/// is always new), none `extra` (its brief).
pub fn check_paths(dir: &Path, paths: &[String], extra: &[String]) -> Result<Vec<String>, String> {
    if paths.is_empty() {
        return Err("--paths names nothing".into());
    }
    let mut out = vec![];
    for p in paths {
        let rel = clean_rel(p)?;
        if let Some(off) = off_limits(&rel) {
            return Err(format!("{rel}: off limits to an aide ({off})"));
        }
        if extra.iter().any(|e| e.to_lowercase() == rel.to_lowercase()) {
            return Err(format!("{rel}: the aide's brief is never its own to write"));
        }
        inside(dir, &rel)?;
        let full = dir.join(&rel);
        let meta = std::fs::symlink_metadata(&full);
        match meta {
            Ok(m) if m.is_dir() => return Err(format!("{rel} is a folder: name each file")),
            Ok(_) => {}
            Err(_) if tracked(dir, &rel) => {} // a deletion
            Err(_) => return Err(format!("{rel}: no such file")),
        }
        if rel.to_lowercase().starts_with("raw/") && tracked(dir, &rel) {
            return Err(format!("{rel}: an existing raw/ file is never changed (a capture is always a new file)"));
        }
        out.push(rel);
    }
    Ok(out)
}

/// Stage exactly `paths` and commit only them (`--only`: whatever else the
/// user staged stays staged, uncommitted), with the item's trailer. The
/// commit's author is the repo's own; with none set, the aide's id. A failed
/// commit unstages what it staged.
pub fn commit(dir: &Path, aide: &str, reference: &str, title: &str, from: &str, paths: &[String], brief: &[String]) -> Result<String, String> {
    let paths = check_paths(dir, paths, brief)?;
    // one line each, no control characters: a title or a sender label never
    // adds a line (a forged `inbox-item:` would make another item look done)
    let line = |t: &str, n: usize| -> String { t.lines().next().unwrap_or("").chars().filter(|c| !c.is_control()).take(n).collect() };
    let title = line(title, 120);
    let title = if title.trim().is_empty() { "a note".to_string() } else { title };
    let from = line(from, 120);
    if let Some(stray) = stray_attributes(dir) {
        return Err(format!("{stray} is changed and not committed: nothing is committed until the user looks at it"));
    }
    let mut add = git(dir);
    add.args(["add", "--"]).args(&paths);
    run(add)?;
    let mut c = git(dir);
    let unset = |k: &str| {
        let mut who = git(dir);
        who.args(["config", "--get", k]);
        run(who).map(|s| s.trim().is_empty()).unwrap_or(true)
    };
    if unset("user.email") || unset("user.name") {
        let me = aide.replace('/', ".").replace('@', "");
        c.args(["-c", &format!("user.name={aide}"), "-c", &format!("user.email={me}@nimitz.local")]);
    }
    c.args(["commit", "--only", "--no-verify", "-m", &format!("ingest(auto): {title}"), "-m", &format!("inbox-item: {reference}"), "-m", &format!("from: {from}"), "--"]).args(&paths);
    if let Err(e) = run(c) {
        let mut undo = git(dir);
        undo.args(["reset", "-q", "--"]).args(&paths);
        let _ = run(undo);
        return Err(e);
    }
    let mut head = git(dir);
    head.args(["rev-parse", "HEAD"]);
    Ok(run(head)?.trim().to_string())
}

/// `inbox fail --restore <paths>`: undo what a failed pass wrote: a tracked
/// file back to its last commit (index and tree), a new one removed. Every
/// path is checked first, as a commit's paths are; whatever is there before
/// it goes (a tracked file that differs from its commit, an untracked file:
/// the user may have edited or made it) is copied to `backup` first, never
/// over a copy kept before, and the note says where.
pub fn restore(dir: &Path, paths: &[String], brief: &[String], backup: &Path) -> Result<Vec<String>, String> {
    if let Some(stray) = stray_attributes(dir) {
        return Err(format!("{stray} is changed and not committed: nothing is restored until the user looks at it"));
    }
    let mut checked = vec![];
    for p in paths {
        let rel = clean_rel(p)?;
        if let Some(off) = off_limits(&rel) {
            return Err(format!("{rel}: off limits to an aide ({off})"));
        }
        if brief.iter().any(|e| e.to_lowercase() == rel.to_lowercase()) {
            return Err(format!("{rel}: the aide's brief is never its own to write"));
        }
        inside(dir, &rel)?;
        if std::fs::symlink_metadata(dir.join(&rel)).is_ok_and(|m| m.is_dir()) {
            return Err(format!("{rel} is a folder: name each file"));
        }
        checked.push(rel);
    }
    let keep = |rel: &str, full: &Path| -> Result<std::path::PathBuf, String> {
        let mut to = backup.join(rel);
        let mut n = 1;
        while to.exists() {
            n += 1;
            to = backup.join(format!("{rel}.{n}"));
        }
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
        }
        std::fs::copy(full, &to).map_err(|e| format!("{rel}: keeping a copy: {e}"))?;
        Ok(to)
    };
    let mut done = vec![];
    for rel in checked {
        let full = dir.join(&rel);
        if tracked(dir, &rel) {
            // its content as it is now, if it is not its commit's: kept
            let mut diff = git(dir);
            diff.args(["diff", "--quiet", "HEAD", "--"]).arg(&rel).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
            let changed = !diff.status().is_ok_and(|s| s.success());
            let kept = if changed && full.is_file() { Some(keep(&rel, &full)?) } else { None };
            let mut c = git(dir);
            c.args(["checkout", "HEAD", "--"]).arg(&rel);
            run(c)?;
            done.push(match kept {
                Some(k) => format!("{rel} restored (as it was: {})", k.display()),
                None => format!("{rel} restored"),
            });
        } else if full.is_file() {
            let k = keep(&rel, &full)?;
            let mut c = git(dir);
            c.args(["reset", "-q", "--"]).arg(&rel);
            let _ = run(c);
            std::fs::remove_file(&full).map_err(|e| format!("{rel}: {e}"))?;
            done.push(format!("{rel} removed (kept: {})", k.display()));
        }
    }
    Ok(done)
}

/// Is `snap` a worktree of `repo`, and its own top (not a folder inside
/// another repo, not a link to one)? Its `--git-common-dir` is the repo's.
pub fn snapshot_healthy(repo: &Path, snap: &Path) -> bool {
    // a link is never the snapshot (it could lead into the user's own worktree,
    // which a refresh would check out and clean)
    if std::fs::symlink_metadata(snap).map(|m| m.file_type().is_symlink()).unwrap_or(true) || !snap.join(".git").is_file() {
        return false;
    }
    let ask = |dir: &Path, what: &str| -> Option<String> {
        let mut c = git(dir);
        c.args(["rev-parse", "--path-format=absolute", what]);
        run(c).ok().map(|s| s.trim().to_string())
    };
    let canon = |s: &str| crate::os::canonical(Path::new(s)).ok();
    let same_repo = ask(repo, "--git-common-dir").and_then(|a| canon(&a)).is_some_and(|a| ask(snap, "--git-common-dir").and_then(|b| canon(&b)) == Some(a));
    let own_top = ask(snap, "--show-toplevel").and_then(|t| canon(&t)) == crate::os::canonical(snap).ok();
    same_repo && own_top
}

/// A snapshot reader's worktree (aide.md 6.4): made when missing, broken or
/// another repo's (`worktree add --detach --force`, which takes back a path
/// still registered; never `worktree prune`, which would drop every worktree
/// registered from another machine of a synced repo), then moved to the
/// repo's current `HEAD`. A commit is atomic for a reader (HEAD moves after
/// its objects are written), so every query reads one whole commit. Returns
/// the commit the snapshot is at.
pub fn ensure_snapshot(repo: &Path, snap: &Path) -> Result<String, String> {
    let mut head = git(repo);
    head.args(["rev-parse", "HEAD"]);
    let sha = run(head)?.trim().to_string();
    if !snapshot_healthy(repo, snap) {
        // the folder is nimitz's own (under NIMITZ_STATE): whatever is there goes
        if std::fs::symlink_metadata(snap).is_ok_and(|m| m.file_type().is_symlink()) {
            std::fs::remove_file(snap).map_err(|e| format!("{}: {e}", snap.display()))?;
        } else if snap.exists() {
            std::fs::remove_dir_all(snap).map_err(|e| format!("{}: {e}", snap.display()))?;
        }
        if let Some(p) = snap.parent() {
            std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
        }
        let mut add = git(repo);
        add.args(["worktree", "add", "--detach", "--force"]).arg(snap).arg(&sha);
        run(add)?;
        return Ok(sha);
    }
    // a reader writes nothing, but whatever is there goes: the snapshot is the commit
    let mut co = git(snap);
    co.args(["checkout", "--detach", "--force", &sha]);
    run(co)?;
    let mut clean = git(snap);
    clean.args(["clean", "-ffdxq"]);
    run(clean)?;
    Ok(sha)
}

/// An aide's snapshot unregistered from its repo and gone (`aide rm`).
pub fn drop_snapshot(repo: &Path, snap: &Path) -> Result<(), String> {
    if snapshot_healthy(repo, snap) {
        let mut c = git(repo);
        c.args(["worktree", "remove", "--force"]).arg(snap);
        run(c)?;
    }
    if snap.exists() {
        std::fs::remove_dir_all(snap).map_err(|e| format!("{}: {e}", snap.display()))?;
    }
    Ok(())
}

/// The commit a snapshot is at.
pub fn snapshot_at(snap: &Path) -> Option<String> {
    let mut c = git(snap);
    c.args(["rev-parse", "HEAD"]);
    run(c).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
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
        // a sender label with a forged trailer stays on its own line
        assert_eq!(committed(d, "u9:000000000009").unwrap(), None);
        assert_eq!(committed(d, "u1:000000000001").unwrap(), None);
        let sha = commit(d, "HOME/@KB", "u1:000000000001", "A title\nsecond line", "HOME/GITHUB/X\ninbox-item: u9:000000000009", &paths, &[]).unwrap();
        assert_eq!(committed(d, "u9:000000000009").unwrap(), None, "no forged trailer");
        let show = |args: &[&str]| String::from_utf8(Command::new("git").arg("-C").arg(d).args(args).output().unwrap().stdout).unwrap();
        assert_eq!(show(&["show", "--name-only", "--format=", &sha]).trim(), "raw/cap-1.md\nwiki/new.md");
        assert!(show(&["log", "-1", "--format=%B"]).contains("ingest(auto): A title\n\ninbox-item: u1:000000000001\n\nfrom: HOME/GITHUB/X"));
        assert!(show(&["diff", "--cached", "--name-only"]).contains("wiki/page.md"), "the user's staged edit is still staged");
        assert_eq!(committed(d, "u1:000000000001").unwrap(), Some(sha));
        assert_eq!(committed(d, "u1:00000000000"), Ok(None), "anchored: no prefix match");
    }

    #[test]
    fn a_snapshot_is_made_then_follows_the_repos_head() {
        let td = repo();
        let d = td.path();
        let state = tempfile::tempdir().unwrap();
        let snap = state.path().join("aides").join("HOME%2F@REF").join("snapshot");
        let first = ensure_snapshot(d, &snap).unwrap();
        assert!(snap.join("wiki/page.md").is_file() && snap.join(".git").is_file());
        // a new commit in the repo, a stray file in the snapshot: the next refresh is that commit, clean
        std::fs::write(d.join("wiki/new.md"), "new\n").unwrap();
        Command::new("git").arg("-C").arg(d).args(["add", "wiki/new.md"]).output().unwrap();
        Command::new("git").arg("-C").arg(d).args(["commit", "-qm", "two"]).output().unwrap();
        std::fs::write(snap.join("stray.md"), "x").unwrap();
        let second = ensure_snapshot(d, &snap).unwrap();
        assert_ne!(first, second);
        assert_eq!(snapshot_at(&snap).as_deref(), Some(second.as_str()));
        assert!(snap.join("wiki/new.md").is_file() && !snap.join("stray.md").exists());
        // the user's uncommitted edit in the repo never shows in the snapshot
        std::fs::write(d.join("wiki/page.md"), "half written\n").unwrap();
        ensure_snapshot(d, &snap).unwrap();
        assert_eq!(std::fs::read_to_string(snap.join("wiki/page.md")).unwrap(), "Alpha beta\n");
        // broken (its folder gone): made again; a sibling worktree whose folder is
        // away (another machine's, a drive unmounted) is never pruned
        let away = state.path().join("away");
        Command::new("git").arg("-C").arg(d).args(["worktree", "add", "--detach", away.to_str().unwrap()]).output().unwrap();
        std::fs::rename(&away, state.path().join("moved")).unwrap();
        std::fs::remove_dir_all(&snap).unwrap();
        assert_eq!(ensure_snapshot(d, &snap).unwrap(), second);
        assert!(snap.join("wiki/new.md").is_file());
        let list = String::from_utf8(Command::new("git").arg("-C").arg(d).args(["worktree", "list"]).output().unwrap().stdout).unwrap();
        assert!(list.contains("away"), "the away worktree is still registered: {list}");
        // a snapshot of another repo is not this repo's: made again from this one
        let other = repo();
        let other_snap = state.path().join("o");
        ensure_snapshot(other.path(), &other_snap).unwrap();
        assert!(!snapshot_healthy(d, &other_snap));
        ensure_snapshot(d, &other_snap).unwrap();
        assert!(snapshot_healthy(d, &other_snap) && other_snap.join("wiki/new.md").is_file());
        // a link to a worktree of this repo (the user's own) is no snapshot
        #[cfg(unix)]
        {
            let users = state.path().join("users");
            Command::new("git").arg("-C").arg(d).args(["worktree", "add", "--detach", users.to_str().unwrap()]).output().unwrap();
            std::fs::write(users.join("mine.txt"), "the user's\n").unwrap();
            let linked = state.path().join("linked");
            std::os::unix::fs::symlink(&users, &linked).unwrap();
            assert!(!snapshot_healthy(d, &linked));
            ensure_snapshot(d, &linked).unwrap();
            assert!(users.join("mine.txt").exists(), "the user's worktree is untouched");
            assert!(!std::fs::symlink_metadata(&linked).unwrap().file_type().is_symlink(), "the link is replaced by a snapshot of nimitz's own");
        }
        // ignored files go too
        std::fs::write(other_snap.join("junk.tmp"), "x").unwrap();
        std::fs::write(other_snap.join(".git").parent().unwrap().join(".gitignore-me"), "x").unwrap();
        drop_snapshot(d, &other_snap).unwrap();
        assert!(!other_snap.exists());
    }

    #[test]
    fn off_limits_and_existing_raw_paths_are_refused() {
        let td = repo();
        let d = td.path();
        std::fs::write(d.join("wiki/a.md"), "a\n").unwrap();
        std::fs::write(d.join("raw/new.md"), "n\n").unwrap();
        for p in ["CLAUDE.md", "claude.md", "agent/skills/x.md", "Agent/x.md", ".git/config", "../outside.md", "/etc/passwd", "C:/x.md", "raw/old.md", ".gitignore", "wiki/.gitattributes", "wiki/sub/CLAUDE.md", "CLAUDE.local.md", ".", "./", "wiki", "raw/", "wiki/missing.md", "AIDE.md"] {
            assert!(check_paths(d, &[p.to_string()], &["AIDE.md".into()]).is_err(), "{p}");
        }
        assert!(check_paths(d, &[], &[]).is_err());
        assert_eq!(check_paths(d, &["wiki/a.md".into(), "raw/new.md".into()], &[]).unwrap(), vec!["wiki/a.md".to_string(), "raw/new.md".into()]);
    }

    /// Paths are literal: a glob or a pathspec magic never sweeps in more.
    #[test]
    fn a_commit_path_is_literal_never_a_glob() {
        let td = repo();
        let d = td.path();
        std::fs::write(d.join("CLAUDE.md"), "the user's edit\n").unwrap();
        std::fs::write(d.join("notes.txt"), "untracked\n").unwrap();
        std::fs::write(d.join("wiki/a.md"), "a\n").unwrap();
        for p in ["CLAUDE.m?", "*", ":(icase)claude.md", "r?w/old.md", "wiki/*"] {
            assert!(commit(d, "HOME/@KB", "u1:000000000005", "t", "x", &[p.to_string()], &[]).is_err(), "{p}");
        }
        let out = String::from_utf8(Command::new("git").arg("-C").arg(d).args(["diff", "--cached", "--name-only"]).output().unwrap().stdout).unwrap();
        assert!(out.is_empty(), "nothing left staged: {out}");
    }

    /// A filter named in an attributes file never runs: `.gitattributes` is
    /// never committed, and git reads no global attributes.
    #[cfg(unix)]
    #[test]
    fn an_attributes_filter_never_runs_through_nimitz() {
        let td = repo();
        let d = td.path();
        let flag = d.join("ran");
        Command::new("git").arg("-C").arg(d).args(["config", "filter.x.clean", &format!("touch {}", flag.display())]).output().unwrap();
        std::fs::write(d.join("wiki/.gitattributes"), "*.md filter=x\n").unwrap();
        std::fs::write(d.join("wiki/b.md"), "b\n").unwrap();
        assert!(commit(d, "HOME/@KB", "u1:000000000006", "t", "x", &["wiki/.gitattributes".into()], &[]).is_err());
        let e = commit(d, "HOME/@KB", "u1:000000000006", "t", "x", &["wiki/b.md".into()], &[]).unwrap_err();
        assert!(e.contains("wiki/.gitattributes is changed"), "{e}");
        assert!(!flag.exists(), "the filter never ran");
    }

    /// A failed pass's files go back: a tracked one to its commit, a new one away.
    #[test]
    fn restore_undoes_what_a_failed_pass_wrote() {
        let td = repo();
        let d = td.path();
        std::fs::write(d.join("wiki/page.md"), "half written\n").unwrap();
        std::fs::write(d.join("raw/cap-9.md"), "capture\n").unwrap();
        let keep = tempfile::tempdir().unwrap();
        let done = restore(d, &["wiki/page.md".into(), "raw/cap-9.md".into()], &[], keep.path()).unwrap();
        assert_eq!(done.len(), 2, "{done:?}");
        assert_eq!(std::fs::read_to_string(d.join("wiki/page.md")).unwrap(), "Alpha beta\n");
        assert_eq!(std::fs::read_to_string(keep.path().join("wiki/page.md")).unwrap(), "half written\n", "what was there is kept");
        assert!(!d.join("raw/cap-9.md").exists());
        assert_eq!(std::fs::read_to_string(keep.path().join("raw/cap-9.md")).unwrap(), "capture\n", "a new file is kept too");
        // a second restore of the same page never writes over the first copy
        std::fs::write(d.join("wiki/page.md"), "second pass\n").unwrap();
        restore(d, &["wiki/page.md".into()], &[], keep.path()).unwrap();
        assert_eq!(std::fs::read_to_string(keep.path().join("wiki/page.md")).unwrap(), "half written\n");
        assert_eq!(std::fs::read_to_string(keep.path().join("wiki/page.md.2")).unwrap(), "second pass\n");
        assert!(restore(d, &["CLAUDE.md".into()], &[], keep.path()).is_err());
        assert!(restore(d, &["wiki".into()], &[], keep.path()).is_err());
        // a refused path anywhere in the list: nothing is restored
        std::fs::write(d.join("wiki/page.md"), "again\n").unwrap();
        assert!(restore(d, &["wiki/page.md".into(), "CLAUDE.md".into()], &[], keep.path()).is_err());
        assert_eq!(std::fs::read_to_string(d.join("wiki/page.md")).unwrap(), "again\n");
        // a symlinked folder that leads out: refused, nothing outside removed
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("victim.md"), "v").unwrap();
            std::os::unix::fs::symlink(outside.path(), d.join("wiki/link")).unwrap();
            assert!(restore(d, &["wiki/link/victim.md".into()], &[], keep.path()).is_err());
            assert!(check_paths(d, &["wiki/link/victim.md".into()], &[]).is_err());
            assert!(outside.path().join("victim.md").exists());
        }
    }
}
