//! A fleet's knowledge base and its git, run by nimitz, never by the agents
//! that read or write it (`nimitz kb`, docs/design/kb.md): reads at one
//! commit (`head`, `show`, `grep`), the hash of a capture's body, a commit
//! of exactly the named paths with its `note:` and `from:` trailers (once
//! per note, under a lock), a restore that keeps a copy, and `sync` (pull
//! --rebase, push). nimitz runs git with every option that could run a
//! program turned off, but for the filters the user's own committed
//! attributes name (git-lfs, say); an attributes file not committed stops
//! nimitz's git (`stray_attributes`). Fetch and push use the user's own
//! transport (ssh, a credential helper), never a prompt.
use crate::env::Env;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// What an agent never writes in the KB: its own rules (top level),
/// and, at any depth, what makes git or Claude Code act on its own (`.git`,
/// attributes that name a filter program, modules, a CLAUDE.md that becomes
/// instructions at the next session start). Compared without case: on macOS
/// and Windows `claude.md` is `CLAUDE.md`.
pub const OFF_LIMITS: [&str; 7] = ["agent/", "CLAUDE.md", "START_HERE.md", "SCOPE.md", "DESIGN-NOTES.md", "README.md", "CLAUDE.local.md"];
/// Never written at any depth.
pub const OFF_LIMITS_ANYWHERE: [&str; 7] = [".git", ".gitattributes", ".gitmodules", ".gitignore", "CLAUDE.md", ".claude", ".mcp.json"];

/// The most lines `grep` prints, each at most `LINE_MAX` characters: an
/// agent's context is not a dump.
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
    // none of the caller's `GIT_*` steers it (the ssh pair is the user's transport)
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy();
        if k.starts_with("GIT_") && k != "GIT_SSH_COMMAND" && k != "GIT_SSH" {
            c.env_remove(k.as_ref());
        }
    }
    c.env("GIT_PAGER", "cat").env("GIT_TERMINAL_PROMPT", "0");
    c
}

/// A fetch, push or clone: the user's own transport, but never a passphrase
/// prompt nobody will answer nor a connection that hangs on (it holds the
/// KB's lock): `ssh -o BatchMode=yes -o ConnectTimeout=20`, unless the user
/// set an ssh command of their own (`GIT_SSH*`, `core.sshCommand`).
fn transport(c: &mut Command, dir: &Path) {
    let own_env = std::env::var_os("GIT_SSH_COMMAND").is_some() || std::env::var_os("GIT_SSH").is_some();
    let own_config = || {
        let mut g = git(dir);
        g.args(["config", "--get", "core.sshCommand"]);
        run(g).is_ok_and(|s| !s.trim().is_empty())
    };
    if !own_env && !own_config() {
        c.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o ConnectTimeout=20");
    }
}

fn run(mut c: Command) -> Result<String, String> {
    let o = c.output().map_err(|e| format!("git: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

/// A path inside the repo, as an agent names it: relative, no `..`, no
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
    // a name Windows (or an old Mac) reads as another: a trailing dot or space,
    // a stream (`:`), an 8.3 alias (`~1`), an invisible character
    if rel.split('/').any(|p| p.ends_with('.') || p.ends_with(' ') || p.contains(':') || p.contains('~') || p.chars().any(|c| c.is_control() || matches!(c, '\u{200b}'..='\u{200f}' | '\u{feff}' | '\u{2060}'))) {
        return Some("a name that another file system reads as another (a trailing dot or space, `:`, `~`, an invisible character)".into());
    }
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

/// A commit to read at: `HEAD`, or a commit id (4 to 64 hex digits; a
/// `--at` from `nimitz kb head`), resolved among object ids only (a branch
/// or tag of that name is never it) to its one full id.
fn rev(dir: &Path, at: Option<&str>) -> Result<String, String> {
    let s = match at {
        None | Some("HEAD") => return Ok("HEAD".into()),
        Some(s) if (4..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit()) => s.to_lowercase(),
        Some(s) => return Err(format!("{s:?} is no commit id (HEAD, or the hex sha `nimitz kb head` printed)")),
    };
    let mut c = git(dir);
    c.arg("rev-parse").arg(format!("--disambiguate={s}"));
    let ids: Vec<String> = run(c).unwrap_or_default().lines().map(str::to_string).collect();
    let commits: Vec<&String> = ids
        .iter()
        .filter(|id| {
            let mut t = git(dir);
            t.args(["cat-file", "-t", id]);
            run(t).is_ok_and(|k| k.trim() == "commit")
        })
        .collect();
    match commits.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!("{s}: no commit with that id in the clone")),
        _ => Err(format!("{s}: more than one commit starts so: give more of the id")),
    }
}

/// `nimitz kb head`: the commit the KB is at.
pub fn head(dir: &Path) -> Result<String, String> {
    let mut c = git(dir);
    c.args(["rev-parse", "HEAD"]);
    Ok(run(c)?.trim().to_string())
}

/// `nimitz kb grep <term> [<prefix>] [--at <sha>]`: `git grep -n -i -F` in
/// the files of one commit (never the working tree: a write in progress is
/// not seen half done), under `prefix` (a folder or a file) when given, as
/// `path:line:text` (at most `GREP_MAX_LINES` lines).
pub fn grep(dir: &Path, at: Option<&str>, term: &str, prefix: Option<&str>) -> Result<String, String> {
    if term.trim().is_empty() {
        return Err("grep needs a term".into());
    }
    if term.chars().any(char::is_control) {
        return Err("a term is one line, no control characters".into());
    }
    let r = rev(dir, at)?;
    let mut c = git(dir);
    c.args(["grep", "-n", "-i", "-I", "-F", "--no-color", "-e", term, &r, "--"]);
    if let Some(p) = prefix {
        c.arg(clean_rel(p)?);
    }
    let o = c.output().map_err(|e| format!("git: {e}"))?;
    // exit 1: no match
    match o.status.code() {
        Some(0) | Some(1) => {}
        _ => return Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
    }
    let lead = format!("{r}:");
    let text = String::from_utf8_lossy(&o.stdout);
    let lines: Vec<&str> = text.lines().map(|l| l.strip_prefix(&lead).unwrap_or(l)).collect();
    Ok(clip_lines(&lines.join("\n"), GREP_MAX_LINES))
}

/// The most bytes `show` prints.
pub const SHOW_MAX: usize = 512 * 1024;

/// `nimitz kb show <path> [--at <sha>]`: a file as one commit holds it.
pub fn show(dir: &Path, at: Option<&str>, path: &str) -> Result<String, String> {
    let r = rev(dir, at)?;
    let rel = clean_rel(path)?;
    let mut c = git(dir);
    c.args(["show", "--no-textconv", &format!("{r}:{rel}")]);
    let text = run(c).map_err(|e| format!("{rel} at {r}: {e}"))?;
    if text.len() > SHOW_MAX {
        let mut cut = SHOW_MAX;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        return Ok(format!("{}\n... cut at {SHOW_MAX} bytes", &text[..cut]));
    }
    Ok(text)
}

/// The bytes of a page after its frontmatter: after the line `---` that
/// closes a first line `---`; the whole file when it has none (or never
/// closes it).
pub fn body(bytes: &[u8]) -> &[u8] {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let fence = |l: &[u8]| {
        let l = l.strip_suffix(b"\n").unwrap_or(l);
        l.strip_suffix(b"\r").unwrap_or(l).trim_ascii_end() == b"---"
    };
    let mut at = 0;
    for (n, l) in bytes.split_inclusive(|b| *b == b'\n').enumerate() {
        at += l.len();
        match (n, fence(l)) {
            (0, false) => return bytes,
            (0, true) => {}
            (_, true) => return &bytes[at..],
            _ => {}
        }
    }
    bytes
}

/// `nimitz kb hash <path>`: the sha256 (hex) of the file's body as written
/// in the working tree (`body`): what a note's duplicate check and its
/// commit's `note:` trailer carry.
pub fn hash(dir: &Path, path: &str) -> Result<String, String> {
    let rel = clean_rel(path)?;
    inside(dir, &rel)?;
    let bytes = std::fs::read(dir.join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
    // line ends as LF: the same capture written on Windows is the same note
    let b = String::from_utf8_lossy(body(&bytes)).replace("\r\n", "\n");
    if b.trim().is_empty() {
        return Err(format!("{rel}: its body (after the frontmatter) is empty: no note"));
    }
    Ok(crate::sha256::hex(b.as_bytes()))
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

/// `nimitz kb status`: `git status --porcelain` of the folder (the user's
/// uncommitted edits, a sync's conflicted copies).
pub fn status(dir: &Path) -> Result<String, String> {
    if let Some(stray) = stray_attributes(dir) {
        return Err(format!("{stray} is changed and not committed: git is not run until the user looks at it"));
    }
    let mut c = git(dir);
    c.args(["--no-optional-locks", "status", "--porcelain", "--untracked-files=all"]);
    Ok(clip_lines(&run(c)?, GREP_MAX_LINES))
}

/// The commit that already carries this note (`note: <sha256>`), if any:
/// the note was ingested before.
pub fn note_commit(dir: &Path, note: &str) -> Result<Option<String>, String> {
    if note.len() != 64 || !note.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("{note:?} is no sha256 (64 hex digits, from `nimitz kb hash`)"));
    }
    let mut c = git(dir);
    c.args(["log", "-E", "--format=%H", "-1", &format!("--grep=^note: {}$", note.to_lowercase())]);
    Ok(run(c)?.lines().next().map(str::to_string).filter(|s| !s.is_empty()))
}

fn tracked(dir: &Path, rel: &str) -> bool {
    let mut c = git(dir);
    c.args(["ls-files", "--error-unmatch", "--", rel]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    c.status().is_ok_and(|s| s.success())
}

/// Check the paths an agent wants committed: files inside the repo, named
/// one by one (a file, or a tracked file it deleted; never a folder), none
/// off limits, none an existing file under `raw/` (a capture is always new).
pub fn check_paths(dir: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    if paths.is_empty() {
        return Err("--paths names nothing".into());
    }
    let mut out = vec![];
    for p in paths {
        let rel = clean_rel(p)?;
        if let Some(off) = off_limits(&rel) {
            return Err(format!("{rel}: off limits ({off})"));
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

/// What `commit` did.
#[derive(Clone, Debug, PartialEq)]
pub enum Committed {
    New(String),
    /// the note was committed before, in this commit: nothing new
    Already(String),
}

/// One line, no control characters, at most `n` characters: a title or a
/// sender never adds a line (a forged `note:` would make another note look
/// ingested).
fn one_line(t: &str, n: usize) -> String {
    t.lines().next().unwrap_or("").chars().filter(|c| !c.is_control()).take(n).collect::<String>().trim().to_string()
}

/// `nimitz kb commit`: under the KB's lock, a note committed before is that
/// commit again (`Already`); else stage exactly `paths` and commit only them
/// (`--only`: whatever else the user staged stays staged, uncommitted), as
/// `ingest(auto): <title>` with the pages written and the trailers `note:
/// <sha256>` and `from: <sender>`. The author is the repo's own; with none
/// set, `nimitz kb`. A failed commit unstages what it staged.
pub fn commit(env: &Env, dir: &Path, title: &str, paths: &[String], pages: &[String], note: &str, from: &str) -> Result<Committed, String> {
    let note = note.trim().to_lowercase();
    let from = one_line(from, 160);
    if from.is_empty() {
        return Err("no sender: --from names who gave the note".into());
    }
    let _lock = lock(env, dir)?;
    busy(dir)?;
    if let Some(sha) = note_commit(dir, &note)? {
        return Ok(Committed::Already(sha));
    }
    let paths = check_paths(dir, paths)?;
    let title = one_line(title, 120);
    let title = if title.is_empty() { "a note".to_string() } else { title };
    let pages: Vec<String> = pages.iter().map(|p| one_line(p, 200)).filter(|p| !p.is_empty()).collect();
    if let Some(stray) = stray_attributes(dir) {
        return Err(format!("{stray} is changed and not committed: nothing is committed until the user looks at it"));
    }
    let mut add = git(dir);
    add.args(["add", "--"]).args(&paths);
    let unstage = || {
        let mut undo = git(dir);
        undo.args(["reset", "-q", "--"]).args(&paths);
        let _ = run(undo);
    };
    // (a path git ignores fails the add after the others were staged)
    if let Err(e) = run(add) {
        unstage();
        return Err(e);
    }
    let mut c = git(dir);
    let unset = |k: &str| {
        let mut who = git(dir);
        who.args(["config", "--get", k]);
        run(who).map(|s| s.trim().is_empty()).unwrap_or(true)
    };
    if unset("user.email") || unset("user.name") {
        c.args(["-c", "user.name=nimitz kb", "-c", "user.email=kb@nimitz.local"]);
    }
    c.args(["commit", "--only", "--no-verify", "-m", &format!("ingest(auto): {title}")]);
    if !pages.is_empty() {
        c.args(["-m", &format!("pages: {}", pages.join(", "))]);
    }
    c.args(["-m", &format!("note: {note}\nfrom: {from}"), "--"]).args(&paths);
    if let Err(e) = run(c) {
        unstage();
        return Err(e);
    }
    head(dir).map(Committed::New)
}

/// The clone's git folder.
fn git_dir(dir: &Path) -> Option<PathBuf> {
    let mut g = git(dir);
    g.args(["rev-parse", "--absolute-git-dir"]);
    run(g).map(|s| PathBuf::from(s.trim())).ok()
}

/// A rebase, a merge or a cherry-pick under way in the clone (the user's, or
/// a sync's that died), or no branch checked out: nothing is committed or
/// synced until the user looks at it.
fn busy(dir: &Path) -> Result<(), String> {
    if let Some(g) = git_dir(dir) {
        if let Some(op) = ["rebase-merge", "rebase-apply", "MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"].iter().find(|f| g.join(f).exists()) {
            return Err(format!("a git operation is under way in the clone ({op}): nothing is committed or synced until the user finishes it"));
        }
    }
    let mut b = git(dir);
    b.args(["symbolic-ref", "-q", "HEAD"]);
    run(b).map(|_| ()).map_err(|_| "the clone has no branch checked out (a detached HEAD): nothing is committed or synced until the user checks one out".to_string())
}

/// The KB's lock on this machine (`S/locks/kb-<hash of its folder>.lock`):
/// one commit or sync at a time, whichever fleet or session asks.
pub fn lock(env: &Env, dir: &Path) -> Result<std::fs::File, String> {
    let canon = crate::os::canonical(dir).unwrap_or_else(|_| dir.to_path_buf());
    let key = crate::sha256::hex(canon.to_string_lossy().as_bytes());
    let p = env.state.join("locks").join(format!("kb-{}.lock", &key[..16]));
    std::fs::create_dir_all(p.parent().unwrap_or(&env.state)).map_err(|e| format!("{}: {e}", p.display()))?;
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    f.lock().map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(f)
}

/// `nimitz kb sync`: under the lock, `pull --rebase` from the branch's
/// upstream (no autostash: the user's uncommitted edits are never moved;
/// a rebase that stops is undone), then `push`. A branch with no upstream
/// has nothing to sync.
pub fn sync(env: &Env, dir: &Path) -> Result<String, String> {
    let _lock = lock(env, dir)?;
    if let Some(stray) = stray_attributes(dir) {
        return Err(format!("{stray} is changed and not committed: git is not run until the user looks at it"));
    }
    busy(dir)?;
    let mut up = git(dir);
    up.args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]);
    let Ok(upstream) = run(up) else { return Err("the branch has no upstream: the commits stay in this clone (the user sets one: git push -u)".into()) };
    let upstream = upstream.trim().to_string();
    let mut pull = git(dir);
    pull.args(["pull", "--rebase", "--no-autostash", "--quiet"]);
    transport(&mut pull, dir);
    if let Err(e) = run(pull) {
        let mut note = "the local commits are kept".to_string();
        if git_dir(dir).is_some_and(|g| g.join("rebase-merge").exists() || g.join("rebase-apply").exists()) {
            let mut abort = git(dir);
            abort.args(["rebase", "--abort"]);
            if let Err(a) = run(abort) {
                note = format!("the rebase could not be undone ({a}): the user finishes it by hand");
            }
        }
        return Err(format!("pull --rebase from {upstream} failed (nothing pushed; {note}; a sync needs no uncommitted edits to tracked files): {e}"));
    }
    let mut push = git(dir);
    push.args(["push", "--quiet"]);
    transport(&mut push, dir);
    run(push).map_err(|e| format!("push to {upstream} failed (the local commits are kept): {e}"))?;
    Ok(format!("in step with {upstream} at {}", head(dir)?))
}

/// The clone's `origin` URL, if it has one.
pub fn origin(dir: &Path) -> Option<String> {
    let mut c = git(dir);
    c.args(["remote", "get-url", "origin"]);
    run(c).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// `nimitz kb clone`: the KB's remote into `dir` (not there yet, or an
/// empty folder).
pub fn clone(remote: &str, dir: &Path) -> Result<(), String> {
    if dir.exists() && std::fs::read_dir(dir).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(format!("{} is there and not empty", dir.display()));
    }
    if let Some(p) = dir.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    if !dir.is_absolute() {
        return Err(format!("{}: a clone goes to an absolute folder", dir.display()));
    }
    let parent = dir.parent().unwrap_or(Path::new("."));
    let mut c = git(parent);
    c.args(["clone", "--quiet", "--", remote]).arg(dir);
    transport(&mut c, parent);
    run(c).map(|_| ())
}

/// `nimitz kb restore --paths`: undo what a failed ingest wrote: a tracked
/// file back to its last commit (index and tree), a new one removed. Every
/// path is checked first, as a commit's paths are; whatever is there before
/// it goes (a tracked file that differs from its commit, an untracked file:
/// the user may have edited or made it) is copied to `backup` first, never
/// over a copy kept before, and the line says where.
pub fn restore(env: &Env, dir: &Path, paths: &[String], backup: &Path) -> Result<Vec<String>, String> {
    let _lock = lock(env, dir)?;
    if let Some(stray) = stray_attributes(dir) {
        return Err(format!("{stray} is changed and not committed: nothing is restored until the user looks at it"));
    }
    let mut checked = vec![];
    for p in paths {
        let rel = clean_rel(p)?;
        if let Some(off) = off_limits(&rel) {
            return Err(format!("{rel}: off limits ({off})"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::test_env;

    fn g(d: &Path, args: &[&str]) -> String {
        let o = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
        assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    }

    fn repo() -> tempfile::TempDir {
        let td = tempfile::tempdir().unwrap();
        let d = td.path();
        g(d, &["init", "-q", "-b", "main"]);
        g(d, &["config", "user.email", "t@t"]);
        g(d, &["config", "user.name", "t"]);
        std::fs::create_dir_all(d.join("wiki")).unwrap();
        std::fs::create_dir_all(d.join("raw")).unwrap();
        std::fs::write(d.join("CLAUDE.md"), "rules\n").unwrap();
        std::fs::write(d.join("raw/old.md"), "old capture\n").unwrap();
        std::fs::write(d.join("wiki/page.md"), "Alpha beta\n").unwrap();
        g(d, &["add", "-A"]);
        g(d, &["commit", "-qm", "init"]);
        td
    }

    const NOTE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn reads_go_by_one_commit_never_the_working_tree() {
        let td = repo();
        let d = td.path();
        let h = head(d).unwrap();
        assert_eq!(h.len(), 40);
        assert_eq!(grep(d, None, "alpha", Some("wiki")).unwrap(), "wiki/page.md:1:Alpha beta");
        assert_eq!(grep(d, None, "alpha", Some("wiki/page.md")).unwrap(), "wiki/page.md:1:Alpha beta", "a file as the prefix");
        assert_eq!(grep(d, None, "nothing-here", None).unwrap(), "");
        assert!(grep(d, None, "x", Some("../etc")).is_err());
        // an edit not committed is not seen; a later commit is, and `--at` still reads the old one
        std::fs::write(d.join("wiki/page.md"), "Alpha gamma\n").unwrap();
        assert_eq!(show(d, None, "wiki/page.md").unwrap(), "Alpha beta\n");
        g(d, &["commit", "-qam", "two"]);
        assert_eq!(show(d, None, "wiki/page.md").unwrap(), "Alpha gamma\n");
        assert_eq!(show(d, Some(&h), "wiki/page.md").unwrap(), "Alpha beta\n");
        assert_eq!(grep(d, Some(&h[..8]), "beta", None).unwrap(), "wiki/page.md:1:Alpha beta");
        for bad in ["HEAD~1", "main", "--output=/tmp/x", "zz"] {
            assert!(show(d, Some(bad), "wiki/page.md").is_err(), "{bad}");
        }
        assert!(show(d, None, "wiki/missing.md").is_err());
        std::fs::write(d.join("wiki/page.md"), "edited\n").unwrap();
        assert!(status(d).unwrap().contains(" M wiki/page.md"));
    }

    #[test]
    fn the_hash_is_of_the_body_after_the_frontmatter() {
        assert_eq!(body(b"---\ntitle: x\n---\nthe body\n"), b"the body\n");
        assert_eq!(body(b"---\r\ntitle: x\r\n---\r\nbody"), b"body");
        assert_eq!(body(b"no frontmatter\n---\n"), b"no frontmatter\n---\n");
        assert_eq!(body(b"---\nnever closed\n"), b"---\nnever closed\n");
        assert_eq!(body(b"---\n---\n"), b"");
        let td = repo();
        let d = td.path();
        std::fs::write(d.join("raw/cap.md"), "---\nsource: a\n---\nthe body\n").unwrap();
        std::fs::write(d.join("raw/cap2.md"), "---\nsource: b, another day\n---\nthe body\n").unwrap();
        let h = hash(d, "raw/cap.md").unwrap();
        assert_eq!(h, crate::sha256::hex(b"the body\n"));
        assert_eq!(hash(d, "raw/cap2.md").unwrap(), h, "the same body under other frontmatter is the same note");
        assert!(hash(d, "../x").is_err() && hash(d, "raw/none.md").is_err());
    }

    #[test]
    fn a_commit_takes_only_the_named_paths_once_per_note_with_its_trailers() {
        let (_e, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let td = repo();
        let d = td.path();
        // the user's own edit, staged: it stays out of the commit
        std::fs::write(d.join("wiki/page.md"), "the user's edit\n").unwrap();
        g(d, &["add", "wiki/page.md"]);
        std::fs::write(d.join("raw/cap-1.md"), "capture\n").unwrap();
        std::fs::write(d.join("wiki/new.md"), "new page\n").unwrap();
        let paths = vec!["raw/cap-1.md".to_string(), "./wiki/new.md".to_string()];
        let forged = format!("member HOME/GITHUB/X\nnote: {}", "b".repeat(64));
        let Committed::New(sha) = commit(&env, d, "A title\nsecond line", &paths, &["wiki/new.md".into()], NOTE, &forged).unwrap() else { panic!("a new commit") };
        assert_eq!(note_commit(d, &"b".repeat(64)).unwrap(), None, "no forged trailer");
        assert_eq!(g(d, &["show", "--name-only", "--format=", &sha]).trim(), "raw/cap-1.md\nwiki/new.md");
        let msg = g(d, &["log", "-1", "--format=%B"]);
        assert!(msg.starts_with(&format!("ingest(auto): A title\n\npages: wiki/new.md\n\nnote: {NOTE}\nfrom: member HOME/GITHUB/X\n")), "{msg}");
        let trailers = g(d, &["log", "-1", "--format=%(trailers:only)"]);
        assert!(trailers.contains(&format!("note: {NOTE}")) && trailers.contains("from: member HOME/GITHUB/X"), "{trailers}");
        assert!(g(d, &["diff", "--cached", "--name-only"]).contains("wiki/page.md"), "the user's staged edit is still staged");
        // the same note again: that commit, nothing new
        std::fs::write(d.join("raw/cap-2.md"), "again\n").unwrap();
        assert_eq!(commit(&env, d, "t", &["raw/cap-2.md".into()], &[], NOTE, "x").unwrap(), Committed::Already(sha.clone()));
        assert_eq!(head(d).unwrap(), sha);
        // no sender, a note that is no sha256: refused
        assert!(commit(&env, d, "t", &["raw/cap-2.md".into()], &[], &"c".repeat(64), " ").unwrap_err().contains("no sender"));
        assert!(commit(&env, d, "t", &["raw/cap-2.md".into()], &[], "abc", "x").is_err());
        // no title: the note has one anyway
        let Committed::New(s2) = commit(&env, d, "", &["raw/cap-2.md".into()], &[], &"c".repeat(64), "user").unwrap() else { panic!() };
        assert_eq!(g(d, &["log", "-1", "--format=%s", &s2]).trim(), "ingest(auto): a note");
    }

    /// The review's edges: a capture's id is the same with a BOM or CRLF; an
    /// empty body is no note; a path git ignores fails the commit with nothing
    /// left staged; a commit waits while a rebase is under way; `--at` names a
    /// commit, never a branch of that name; names another system reads as
    /// another are off limits.
    #[test]
    fn notes_commits_and_reads_hold_at_their_edges() {
        let (_e, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let td = repo();
        let d = td.path();
        std::fs::write(d.join("raw/a.md"), "---\nx: 1\n---\nline one\nline two\n").unwrap();
        std::fs::write(d.join("raw/b.md"), "\u{feff}--- \r\nx: 2\r\n---\r\nline one\r\nline two\r\n").unwrap();
        assert_eq!(hash(d, "raw/a.md").unwrap(), hash(d, "raw/b.md").unwrap());
        std::fs::write(d.join("raw/empty.md"), "---\nx: 1\n---\n\n").unwrap();
        assert!(hash(d, "raw/empty.md").unwrap_err().contains("empty"));
        std::fs::write(d.join(".git/info/exclude"), "ignored.md\n").unwrap();
        std::fs::write(d.join("wiki/ignored.md"), "i").unwrap();
        assert!(commit(&env, d, "t", &["raw/a.md".into(), "wiki/ignored.md".into()], &[], NOTE, "x").is_err());
        assert!(g(d, &["diff", "--cached", "--name-only"]).is_empty(), "nothing left staged");
        std::fs::create_dir_all(d.join(".git/rebase-merge")).unwrap();
        assert!(commit(&env, d, "t", &["raw/a.md".into()], &[], NOTE, "x").unwrap_err().contains("under way"));
        std::fs::remove_dir_all(d.join(".git/rebase-merge")).unwrap();
        let h = head(d).unwrap();
        g(d, &["branch", "beef"]);
        assert!(show(d, Some("beef"), "wiki/page.md").is_err(), "a branch is no commit id");
        assert_eq!(show(d, Some(&h[..8]), "wiki/page.md").unwrap(), "Alpha beta\n");
        assert!(grep(d, None, "a\nb", None).is_err());
        for p in ["CLAUDE.md.", "wiki/CLAUDE~1.MD", "wiki/x.md:stream", "wiki/C\u{200c}LAUDE.md", "wiki/x.md "] {
            assert!(off_limits(p).is_some(), "{p}");
        }
        assert!(off_limits("wiki/notes-2026.md").is_none());
    }

    #[test]
    fn off_limits_and_existing_raw_paths_are_refused() {
        let td = repo();
        let d = td.path();
        std::fs::write(d.join("wiki/a.md"), "a\n").unwrap();
        std::fs::write(d.join("raw/new.md"), "n\n").unwrap();
        for p in ["CLAUDE.md", "claude.md", "agent/skills/x.md", "Agent/x.md", ".git/config", "../outside.md", "/etc/passwd", "C:/x.md", "raw/old.md", ".gitignore", "wiki/.gitattributes", "wiki/sub/CLAUDE.md", "CLAUDE.local.md", ".", "./", "wiki", "raw/", "wiki/missing.md", "README.md", "wiki/.claude/settings.json"] {
            assert!(check_paths(d, &[p.to_string()]).is_err(), "{p}");
        }
        assert!(check_paths(d, &[]).is_err());
        assert_eq!(check_paths(d, &["wiki/a.md".into(), "raw/new.md".into()]).unwrap(), vec!["wiki/a.md".to_string(), "raw/new.md".into()]);
    }

    /// Paths are literal: a glob or a pathspec magic never sweeps in more.
    #[test]
    fn a_commit_path_is_literal_never_a_glob() {
        let (_e, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let td = repo();
        let d = td.path();
        std::fs::write(d.join("CLAUDE.md"), "the user's edit\n").unwrap();
        std::fs::write(d.join("notes.txt"), "untracked\n").unwrap();
        std::fs::write(d.join("wiki/a.md"), "a\n").unwrap();
        for p in ["CLAUDE.m?", "*", ":(icase)claude.md", "r?w/old.md", "wiki/*"] {
            assert!(commit(&env, d, "t", &[p.to_string()], &[], NOTE, "x").is_err(), "{p}");
        }
        assert!(g(d, &["diff", "--cached", "--name-only"]).is_empty(), "nothing left staged");
    }

    /// A filter named in an attributes file never runs: `.gitattributes` is
    /// never committed, and git reads no global attributes.
    #[cfg(unix)]
    #[test]
    fn an_attributes_filter_never_runs_through_nimitz() {
        let (_e, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let td = repo();
        let d = td.path();
        let flag = d.join("ran");
        g(d, &["config", "filter.x.clean", &format!("touch {}", flag.display())]);
        std::fs::write(d.join("wiki/.gitattributes"), "*.md filter=x\n").unwrap();
        std::fs::write(d.join("wiki/b.md"), "b\n").unwrap();
        assert!(commit(&env, d, "t", &["wiki/.gitattributes".into()], &[], NOTE, "x").is_err());
        let e = commit(&env, d, "t", &["wiki/b.md".into()], &[], NOTE, "x").unwrap_err();
        assert!(e.contains("wiki/.gitattributes is changed"), "{e}");
        assert!(sync(&env, d).unwrap_err().contains("wiki/.gitattributes is changed"));
        assert!(!flag.exists(), "the filter never ran");
    }

    /// A failed ingest's files go back: a tracked one to its commit, a new one away.
    #[test]
    fn restore_undoes_what_a_failed_ingest_wrote() {
        let (_e, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let td = repo();
        let d = td.path();
        std::fs::write(d.join("wiki/page.md"), "half written\n").unwrap();
        std::fs::write(d.join("raw/cap-9.md"), "capture\n").unwrap();
        let keep = tempfile::tempdir().unwrap();
        let done = restore(&env, d, &["wiki/page.md".into(), "raw/cap-9.md".into()], keep.path()).unwrap();
        assert_eq!(done.len(), 2, "{done:?}");
        assert_eq!(std::fs::read_to_string(d.join("wiki/page.md")).unwrap(), "Alpha beta\n");
        assert_eq!(std::fs::read_to_string(keep.path().join("wiki/page.md")).unwrap(), "half written\n", "what was there is kept");
        assert!(!d.join("raw/cap-9.md").exists());
        assert_eq!(std::fs::read_to_string(keep.path().join("raw/cap-9.md")).unwrap(), "capture\n", "a new file is kept too");
        // a second restore of the same page never writes over the first copy
        std::fs::write(d.join("wiki/page.md"), "second pass\n").unwrap();
        restore(&env, d, &["wiki/page.md".into()], keep.path()).unwrap();
        assert_eq!(std::fs::read_to_string(keep.path().join("wiki/page.md")).unwrap(), "half written\n");
        assert_eq!(std::fs::read_to_string(keep.path().join("wiki/page.md.2")).unwrap(), "second pass\n");
        assert!(restore(&env, d, &["CLAUDE.md".into()], keep.path()).is_err());
        assert!(restore(&env, d, &["wiki".into()], keep.path()).is_err());
        // a refused path anywhere in the list: nothing is restored
        std::fs::write(d.join("wiki/page.md"), "again\n").unwrap();
        assert!(restore(&env, d, &["wiki/page.md".into(), "CLAUDE.md".into()], keep.path()).is_err());
        assert_eq!(std::fs::read_to_string(d.join("wiki/page.md")).unwrap(), "again\n");
        // a symlinked folder that leads out: refused, nothing outside removed
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("victim.md"), "v").unwrap();
            std::os::unix::fs::symlink(outside.path(), d.join("wiki/link")).unwrap();
            assert!(restore(&env, d, &["wiki/link/victim.md".into()], keep.path()).is_err());
            assert!(check_paths(d, &["wiki/link/victim.md".into()]).is_err());
            assert!(outside.path().join("victim.md").exists());
        }
    }

    /// clone, then sync both ways through a bare remote: a commit here is
    /// pushed; another clone's commit is pulled (rebased under ours); a
    /// rebase that conflicts is undone and nothing is pushed.
    #[test]
    fn clone_and_sync_through_a_remote() {
        let (_e, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        let src = repo();
        let bare = tempfile::tempdir().unwrap();
        let remote = bare.path().join("kb.git");
        g(bare.path(), &["clone", "-q", "--bare", src.path().to_str().unwrap(), remote.to_str().unwrap()]);
        let here = tempfile::tempdir().unwrap();
        let a = here.path().join("a");
        let b = here.path().join("b");
        clone(remote.to_str().unwrap(), &a).unwrap();
        clone(remote.to_str().unwrap(), &b).unwrap();
        assert!(clone(remote.to_str().unwrap(), &a).unwrap_err().contains("not empty"));
        for d in [&a, &b] {
            g(d, &["config", "user.email", "t@t"]);
            g(d, &["config", "user.name", "t"]);
        }
        std::fs::write(a.join("wiki/a.md"), "a\n").unwrap();
        let Committed::New(sa) = commit(&env, &a, "a", &["wiki/a.md".into()], &[], NOTE, "x").unwrap() else { panic!() };
        assert!(sync(&env, &a).unwrap().contains(&sa));
        std::fs::write(b.join("wiki/b.md"), "b\n").unwrap();
        commit(&env, &b, "b", &["wiki/b.md".into()], &[], &"c".repeat(64), "x").unwrap();
        sync(&env, &b).unwrap();
        assert!(b.join("wiki/a.md").is_file(), "a's commit pulled under b's");
        sync(&env, &a).unwrap();
        assert!(a.join("wiki/b.md").is_file());
        assert_eq!(head(&a).unwrap(), head(&b).unwrap());
        // both change one line: b's rebase stops, is undone, nothing pushed
        std::fs::write(a.join("wiki/page.md"), "from a\n").unwrap();
        commit(&env, &a, "a2", &["wiki/page.md".into()], &[], &"d".repeat(64), "x").unwrap();
        sync(&env, &a).unwrap();
        std::fs::write(b.join("wiki/page.md"), "from b\n").unwrap();
        let Committed::New(sb) = commit(&env, &b, "b2", &["wiki/page.md".into()], &[], &"e".repeat(64), "x").unwrap() else { panic!() };
        let e = sync(&env, &b).unwrap_err();
        assert!(e.contains("pull --rebase") && e.contains("local commits are kept"), "{e}");
        assert_eq!(head(&b).unwrap(), sb, "b's commit is kept, no rebase left half done");
        assert!(status(&b).unwrap().is_empty());
        // no upstream: said, never an OK
        let td = repo();
        assert!(sync(&env, td.path()).unwrap_err().contains("no upstream"));
    }
}
