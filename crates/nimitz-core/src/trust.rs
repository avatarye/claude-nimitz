//! Claude Code's folder trust, set by nimitz: `nimitz trust <folder>`, and a
//! plan step ("trust <path>") in `ship new`, `ship set --dir`, `start` and
//! `init` when a member's folder is not trusted. It replaces "run `claude`
//! once there and accept the trust prompt".
//!
//! What Claude Code reads (2.1.296; the format is undocumented, so doctor's
//! pinned formats check it): `projects["<key>"].hasTrustDialogAccepted` in
//! its config file (`<claude home>.json`, i.e. `~/.claude.json`). It asks
//! about a folder's git root inside a repository, and a parent's trust does
//! not cover a repository (round 2, M16); outside git, the folder itself. It
//! compares real paths, so both the path as given and the real path are
//! written, each `{...the entry, hasTrustDialogAccepted: true}` as Claude
//! Code's own grant does.
//!
//! The write: under the lock Claude Code takes for that file (proper-lockfile,
//! the directory `<file>.lock`; Claude Code re-reads the file under it and
//! merges `projects`, so a writer under the same lock loses nothing), a backup
//! of the old file in `NIMITZ_STATE/backups/` (the newest five kept), then a
//! temp file with the old file's mode (0600: it holds the login) and a rename.
//! A linked file is written where it points.
//!
//! Any caller's (0.7.3): trust lets the folder's own `.claude` settings,
//! hooks and MCP servers run, which every plan that trusts says
//! (`CONSEQUENCE`); a folder too broad to trust is refused for everyone.
use crate::env::Env;
use crate::types::{Id, Issue, Severity, Step};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A lock directory older than this is a dead holder's (Claude Code's
/// config lock uses proper-lockfile's defaults: stale after 10 s, refreshed
/// every 5 s; twice that, as for pins).
const STALE_S: f64 = 20.0;
/// How long nimitz waits for Claude Code to finish its own write.
const WAIT: Duration = Duration::from_secs(5);
/// Backups of the config file nimitz keeps.
const KEEP_BACKUPS: usize = 5;

/// What trusting a folder lets run: said in every plan line and refusal.
pub const CONSEQUENCE: &str = "Claude Code then runs that folder's own .claude settings, hooks and MCP servers";

/// Claude Code's config file (it holds the trust table): `~/.claude.json`
/// for the default Claude home, the one a new trust goes to.
pub fn file(env: &Env) -> PathBuf {
    crate::check::trust_file(env)
}

/// Every config file nimitz reads trust from: `<claude home>/.claude.json`
/// (a `CLAUDE_CONFIG_DIR` layout) and `<claude home>.json` (`~/.claude.json`).
/// `check` and `evidence` both read every one that exists (a key is accepted
/// when any file accepts it), and a write goes to each that exists (`targets`),
/// so what nimitz writes is what they read.
pub fn files(env: &Env) -> Vec<PathBuf> {
    let mut out = vec![env.claude_home.join(".claude.json"), file(env)];
    out.dedup();
    out
}

/// The files a write goes to: each that exists (a link to nothing too: `edit`
/// refuses it); with none, the one Claude Code itself would make
/// (`~/.claude.json` for the default `~/.claude`, else `<claude
/// home>/.claude.json`, the `CLAUDE_CONFIG_DIR` layout).
fn targets(env: &Env) -> Vec<PathBuf> {
    let have: Vec<PathBuf> = files(env).into_iter().filter(|f| std::fs::symlink_metadata(f).is_ok()).collect();
    if !have.is_empty() {
        return have;
    }
    let default = env.claude_home.file_name().is_some_and(|n| n == ".claude");
    vec![if default { file(env) } else { env.claude_home.join(".claude.json") }]
}

fn abs(dir: &Path) -> PathBuf {
    let d = if dir.is_absolute() { dir.to_path_buf() } else { std::env::current_dir().map(|c| c.join(dir)).unwrap_or_else(|_| dir.to_path_buf()) };
    // a path with `..` is not one Claude Code would see as given: the real one stands
    if d.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return crate::env::canon(&d);
    }
    d
}

/// The forms of a key Claude Code may write for `p`: the trimmed path, and on
/// Windows also `env::trust_key`'s `/` form (Claude Code writes `C:\x` and
/// `C:/x`, and compares with case: the case is the path's, as on disk).
fn key_forms(p: &Path) -> Vec<String> {
    let s = p.to_string_lossy();
    let mut out = vec![];
    if cfg!(windows) {
        let t = s.trim_end_matches(['\\', '/']);
        out.push(if t.len() == 2 && t.ends_with(':') { format!("{t}\\") } else { t.to_string() });
    }
    let k = crate::env::trust_key(&s);
    if !out.contains(&k) {
        out.push(k);
    }
    out
}

#[cfg(test)]
fn key(p: &Path) -> String {
    crate::env::trust_key(&p.to_string_lossy())
}

/// The folder Claude Code asks about for `dir`: the git root that holds its
/// real path (Claude Code resolves links first), in the form as given when
/// that form leads to the same root; outside git, inside the data root, the
/// data root (`nimitz init` trusts it; a member folder under it needs no
/// trust of its own); else the folder itself.
pub fn target(env: &Env, dir: &Path) -> PathBuf {
    let given = abs(dir);
    let real = crate::env::canon(&given);
    if let Some(root) = crate::env::git_root(&real) {
        // the form as given, when one of its parents is that root
        return given.ancestors().find(|a| crate::env::canon(a) == root).map(Path::to_path_buf).unwrap_or(root);
    }
    let data = crate::env::canon(&env.home);
    if real.starts_with(&data) {
        return data;
    }
    given
}

/// The keys written for `dir`: the target as given and its real path.
pub fn keys(env: &Env, dir: &Path) -> Vec<String> {
    let t = target(env, dir);
    let mut out = vec![];
    for k in key_forms(&t).into_iter().chain(key_forms(&crate::env::canon(&t))) {
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

/// The target's real path (what messages and fixes name).
pub fn shown(env: &Env, dir: &Path) -> PathBuf {
    crate::env::canon(&target(env, dir))
}

/// A target nimitz never trusts: `/`, the user's home or a parent of it, a
/// parent of the data root. Claude Code itself never saves trust for the home
/// folder, and its lookup outside git walks up the parents: trusting one of
/// these would let every folder under it run its own hooks and MCP servers.
pub fn too_broad(env: &Env, dir: &Path) -> Option<String> {
    let t = shown(env, dir);
    let user = crate::env::canon(&crate::env::home());
    let data = crate::env::canon(&env.home);
    let why = if t.parent().is_none() {
        "the file system's root"
    } else if user.starts_with(&t) {
        "your home folder or a parent of it"
    } else if data != t && data.starts_with(&t) {
        "a parent of the data root"
    } else {
        return None;
    };
    let repo = crate::env::git_root(&crate::env::canon(&abs(dir))).filter(|r| *r == t).map(|_| " (the git repository that holds it)").unwrap_or("");
    Some(format!("nimitz does not trust {}{repo}: it is {why}, and trusting it lets every folder under it run its own .claude settings, hooks and MCP servers", t.display()))
}

/// A path as a shell word: POSIX quoting, or on Windows double quotes (cmd and
/// PowerShell both take them for a plain argument). None on Windows for a
/// path with `%`, `$` or a backtick: inside double quotes cmd expands `%`
/// and PowerShell `$` and the backtick, and one word does not fit both.
fn quoted(p: &Path) -> Option<String> {
    let s = p.to_string_lossy();
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-~:+@".contains(c) || (cfg!(windows) && c == '\\')) {
        return Some(s.into_owned());
    }
    if cfg!(windows) {
        if s.contains(['%', '$', '`']) {
            return None;
        }
        return Some(format!("\"{}\"", s.replace('"', "\"\"")));
    }
    Some(format!("'{}'", s.replace('\'', r"'\''")))
}

/// The fix for an untrusted folder (`at`: the folder Claude Code asks about).
/// Never a `nimitz trust` that nimitz would refuse (`/`, the home folder or a
/// parent of it): there the trust prompt, which Claude Code keeps for that
/// session only. A path no one shell word can carry on Windows (`quoted`):
/// the trust prompt too, spelled as on disk (`check::case_exact_hint`).
pub fn fix(at: &Path) -> String {
    let at = crate::env::canon(at);
    let home = crate::env::canon(&crate::env::home());
    let by_hand = |why: &str| format!("run `claude` once in {} and accept the trust prompt{why}{}", at.display(), crate::check::case_exact_hint(&at));
    if at.parent().is_none() || home.starts_with(&at) {
        let why = " (nimitz never trusts the home folder or a parent of it)";
        return match quoted(&at) {
            Some(q) => format!("run `claude` in {q} and accept the trust prompt{why}"),
            None => by_hand(why),
        };
    }
    match quoted(&at) {
        Some(q) => format!("nimitz trust {q}"),
        None => by_hand(""),
    }
}

/// Will Claude Code refuse `dir` for want of trust? `Error`: nothing trusts
/// it; `Warn`: only a parent does, which does not cover a repository.
/// (`check`'s verdict; a folder that does not exist yet is judged by its path.)
pub fn untrusted(env: &Env, dir: &Path) -> Option<Severity> {
    crate::check::trust_verdict(env, dir)
}

/// `~/.claude.json` for the plan line.
fn file_shown(env: &Env) -> String {
    let f = file(env);
    // (no home at all reads as `/`: then the path in full)
    let home = Some(crate::env::home()).filter(|h| h.parent().is_some());
    match home.as_deref().and_then(|h| f.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => f.display().to_string(),
    }
}

/// The plan step that trusts `dir`.
pub fn step(env: &Env, id: Option<Id>, dir: &Path) -> Step {
    Step {
        id,
        cmd: vec!["trust".into(), target(env, dir).to_string_lossy().into_owned()],
        cwd: None,
        why: format!("trust {} (writes {}): {CONSEQUENCE}", shown(env, dir).display(), file_shown(env)),
    }
}

/// What a plan gets for a member folder `dir` that Claude Code will not
/// trust: a trust step, for every caller. None when it is trusted. A folder
/// too broad to trust (`too_broad`) is a refusal for everyone.
pub fn for_plan(env: &Env, id: Option<&Id>, dir: &Path) -> Option<Result<Step, Issue>> {
    untrusted(env, dir)?;
    if let Some(why) = too_broad(env, dir) {
        return Some(Err(Issue { code: "trust.broad".into(), severity: Severity::Error, message: why, fix: None, id: id.cloned() }));
    }
    Some(Ok(step(env, id.cloned(), dir)))
}

/// Does every config file trust every key of `dir`?
pub fn is_written(env: &Env, dir: &Path) -> bool {
    let keys = keys(env, dir);
    targets(env).iter().all(|f| {
        let Some(v) = std::fs::read(f).ok().and_then(|b| crate::evidence::first_json(&b)) else { return false };
        keys.iter().all(|k| v["projects"][k]["hasTrustDialogAccepted"].as_bool() == Some(true))
    })
}

/// Trust `dir` (its target, both keys). Returns the keys it set (empty: all
/// were set already).
pub fn write(env: &Env, dir: &Path) -> Result<Vec<String>, String> {
    if let Some(why) = too_broad(env, dir) {
        return Err(why);
    }
    let keys = keys(env, dir);
    let mut set: Vec<String> = vec![];
    for f in targets(env) {
        edit(env, &f, |projects| {
            let mut changed = false;
            for k in &keys {
                let e = projects.entry(k.clone()).or_insert_with(|| json!({}));
                let Some(o) = e.as_object_mut() else { return Err(format!("projects[{k:?}] is not an object")) };
                if o.get("hasTrustDialogAccepted").and_then(Value::as_bool) != Some(true) {
                    o.insert("hasTrustDialogAccepted".into(), Value::Bool(true));
                    changed = true;
                    if !set.contains(k) {
                        set.push(k.clone());
                    }
                }
            }
            Ok(changed)
        })?;
    }
    Ok(set)
}

/// Undo `write` for `dir` (tests and e2e cleanup): the flag goes, and an
/// entry left empty goes too.
pub fn forget(env: &Env, dir: &Path) -> Result<(), String> {
    let keys = keys(env, dir);
    for f in files(env).into_iter().filter(|f| f.exists()) {
        edit(env, &f, |projects| {
            let mut changed = false;
            for k in &keys {
                if let Some(o) = projects.get_mut(k).and_then(Value::as_object_mut) {
                    changed |= o.shift_remove("hasTrustDialogAccepted").is_some();
                    if o.is_empty() {
                        projects.shift_remove(k);
                    }
                }
            }
            Ok(changed)
        })?;
    }
    Ok(())
}

/// doctor's pinned-format check: the file is a JSON object, `projects` an
/// object of objects, `hasTrustDialogAccepted` a bool wherever it is set.
/// Ok(number of trusted keys); Err(what differs). No file: Ok(0).
pub fn format_check(env: &Env) -> Result<usize, String> {
    let mut n = 0;
    for f in files(env) {
        n += format_check_file(&f)?;
    }
    Ok(n)
}

fn format_check_file(f: &Path) -> Result<usize, String> {
    let raw = match std::fs::read(f) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("{}: {e}", f.display())),
    };
    let v: Value = serde_json::from_slice(&raw).map_err(|e| format!("{} is not JSON: {e}", f.display()))?;
    let o = v.as_object().ok_or_else(|| format!("{} is not a JSON object", f.display()))?;
    let Some(p) = o.get("projects") else { return Ok(0) };
    let p = p.as_object().ok_or_else(|| format!("{}: `projects` is not an object", f.display()))?;
    let mut n = 0;
    for (k, e) in p {
        let e = e.as_object().ok_or_else(|| format!("{}: projects[{k:?}] is not an object", f.display()))?;
        match e.get("hasTrustDialogAccepted") {
            None => {}
            Some(Value::Bool(b)) => n += usize::from(*b),
            Some(_) => return Err(format!("{}: projects[{k:?}].hasTrustDialogAccepted is not a bool", f.display())),
        }
    }
    Ok(n)
}

/// Edit `projects` under Claude Code's lock; `f` says whether it changed
/// anything (nothing changed: nothing is written).
fn edit(env: &Env, path: &Path, f: impl FnOnce(&mut Map<String, Value>) -> Result<bool, String>) -> Result<bool, String> {
    let path = path.to_path_buf();
    let lock = PathBuf::from(format!("{}.lock", path.display()));
    let _held = crate::pins::dir_lock(&lock, STALE_S, WAIT).map_err(|e| format!("{e} (Claude Code is writing {}; try again)", path.display()))?;
    // a linked config file is written where it points (Claude Code's own
    // write follows the link too); a link to nothing is refused, never
    // replaced by a file of its own
    let linked = std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink());
    let real = match crate::os::canonical(&path) {
        Ok(r) => r,
        Err(_) if linked => {
            let to = std::fs::read_link(&path).map(|t| t.display().to_string()).unwrap_or_else(|_| "?".into());
            return Err(format!("{} is a link to {to}, which does not exist; nimitz leaves the link alone (point it at Claude Code's config file, or remove it)", path.display()));
        }
        Err(_) => path.clone(),
    };
    let raw = match std::fs::read(&real) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", real.display())),
    };
    let mut v: Value = match &raw {
        Some(b) => serde_json::from_slice(b).map_err(|e| format!("{} is not JSON ({e}); nimitz leaves it alone", real.display()))?,
        None => json!({}),
    };
    let o = v.as_object_mut().ok_or_else(|| format!("{} is not a JSON object; nimitz leaves it alone", real.display()))?;
    let projects = o.entry("projects").or_insert_with(|| json!({})).as_object_mut().ok_or_else(|| format!("{}: `projects` is not an object", real.display()))?;
    if !f(projects)? {
        return Ok(false);
    }
    if let Some(b) = &raw {
        backup(env, &real, b).map_err(|e| format!("backup of {}: {e}", real.display()))?;
    }
    let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    write_like(&real, &text).map_err(|e| format!("{}: {e}", real.display()))?;
    Ok(true)
}

/// `NIMITZ_STATE/backups/<source, / as %>.<ms>`: per source file the newest
/// `KEEP_BACKUPS`, and `<source>.first`, the file as it was before nimitz
/// ever wrote it (never rotated out).
fn backup(env: &Env, source: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = env.state.join("backups");
    std::fs::create_dir_all(&dir)?;
    let prefix = source.to_string_lossy().replace(['/', '\\', ':'], "%");
    let save = |p: &Path| write_like(p, std::str::from_utf8(bytes).unwrap_or_default()).or_else(|_| std::fs::write(p, bytes));
    let first = dir.join(format!("{prefix}.first"));
    if !first.exists() {
        save(&first)?;
    }
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let p = dir.join(format!("{prefix}.{ms}"));
    save(&p)?;
    let stamp = |p: &PathBuf| p.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix(&format!("{prefix}."))).and_then(|t| t.parse::<u128>().ok());
    let mut old: Vec<PathBuf> = std::fs::read_dir(&dir)?.flatten().map(|e| e.path()).filter(|p| stamp(p).is_some()).collect();
    old.sort_by_key(|p| stamp(p).unwrap_or(0));
    while old.len() > KEEP_BACKUPS {
        let _ = std::fs::remove_file(old.remove(0));
    }
    Ok(p)
}

/// Temp file in the same folder, with the old file's mode (0600 when new),
/// synced, then renamed over the target.
fn write_like(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.nimitz-tmp-{}", std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mode = std::fs::metadata(path).map(|m| m.permissions().mode() & 0o777).unwrap_or(0o600);
        o.mode(mode);
    }
    let mut f = o.open(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::test_env;

    fn trust_json(env: &Env) -> Value {
        serde_json::from_slice(&std::fs::read(file(env)).unwrap()).unwrap()
    }

    /// Keys in the one form they are compared in (`env::trust_key`): on
    /// Windows each path is written in two forms.
    fn forms(keys: &[String]) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for k in keys.iter().map(|k| crate::env::trust_key(k)) {
            if !out.contains(&k) {
                out.push(k);
            }
        }
        out
    }

    #[test]
    fn a_folder_outside_git_is_trusted_as_itself_and_by_its_real_path() {
        let (td, env) = test_env();
        let real = crate::env::canon(td.path()).join("work/proj");
        std::fs::create_dir_all(&real).unwrap();
        let link = td.path().join("link");
        crate::os::symlink(real.parent().unwrap(), &link).unwrap();
        let given = link.join("proj");
        assert_eq!(forms(&keys(&env, &given)), vec![key(&given), key(&real)]);
        std::fs::write(file(&env), r#"{"numStartups": 3, "projects": {"/x": {"allowedTools": []}}}"#).unwrap();
        assert!(untrusted(&env, &given).is_some());
        let set = write(&env, &given).unwrap();
        assert_eq!(set, keys(&env, &given));
        let v = trust_json(&env);
        assert_eq!(v["numStartups"], 3, "other fields stay");
        assert_eq!(v["projects"]["/x"], json!({"allowedTools": []}), "other projects stay");
        assert!(is_written(&env, &given) && untrusted(&env, &given).is_none());
        assert!(write(&env, &given).unwrap().is_empty(), "a second write changes nothing");
        forget(&env, &given).unwrap();
        assert!(!is_written(&env, &given));
        assert!(trust_json(&env)["projects"].get(key(&real)).is_none(), "an entry left empty goes");
    }

    #[test]
    fn inside_a_repository_the_root_is_trusted_not_the_folder() {
        let (td, env) = test_env();
        let repo = crate::env::canon(td.path()).join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();
        // a parent's trust does not cover the repository
        std::fs::write(file(&env), json!({"projects": {key(&crate::env::canon(td.path())): {"hasTrustDialogAccepted": true}}}).to_string()).unwrap();
        assert_eq!(untrusted(&env, &repo.join("src/deep")), Some(Severity::Warn));
        assert_eq!(target(&env, &repo.join("src/deep")), repo);
        write(&env, &repo.join("src/deep")).unwrap();
        assert_eq!(trust_json(&env)["projects"][key(&repo)]["hasTrustDialogAccepted"], true);
        assert!(trust_json(&env)["projects"].get(key(&repo.join("src/deep"))).is_none());
        assert!(untrusted(&env, &repo.join("src/deep")).is_none());
        assert_eq!(fix(&target(&env, &repo.join("src"))), format!("nimitz trust {}", repo.display()));
    }

    #[test]
    fn the_write_waits_for_claude_codes_lock_and_takes_a_dead_one() {
        let (td, env) = test_env();
        let dir = crate::env::canon(td.path()).join("p");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(file(&env), "{}").unwrap();
        let lock = PathBuf::from(format!("{}.lock", file(&env).display()));
        std::fs::create_dir(&lock).unwrap();
        let held = lock.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            std::fs::remove_dir(held).unwrap();
        });
        let t0 = std::time::Instant::now();
        write(&env, &dir).unwrap();
        assert!(t0.elapsed() >= Duration::from_millis(250), "waited for the holder");
        t.join().unwrap();
        assert!(!lock.exists(), "released");
        // a lock whose holder died long ago is taken over
        std::fs::create_dir(&lock).unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(60);
        crate::pins::tests::dir_handle(&lock).set_modified(old).unwrap();
        forget(&env, &dir).unwrap();
        assert!(!is_written(&env, &dir));
        assert!(!lock.exists(), "the dead holder's lock was taken over, then released");
    }

    #[test]
    fn a_backup_is_kept_and_the_mode_stays() {
        let (td, env) = test_env();
        let dir = crate::env::canon(td.path()).join("p");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(file(&env), r#"{"oauthAccount": {"x": 1}}"#).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(file(&env), std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        for i in 0..7 {
            if i % 2 == 0 { write(&env, &dir).unwrap(); } else { forget(&env, &dir).unwrap(); }
            std::thread::sleep(Duration::from_millis(3));
        }
        let backups: Vec<_> = std::fs::read_dir(env.state.join("backups")).unwrap().flatten().collect();
        assert_eq!(backups.len(), KEEP_BACKUPS + 1, "the newest five and the first");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(file(&env)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(trust_json(&env)["oauthAccount"]["x"], 1);
    }

    #[test]
    fn a_file_nimitz_cannot_read_as_claude_code_does_is_left_alone() {
        let (td, env) = test_env();
        let dir = crate::env::canon(td.path()).join("p");
        std::fs::create_dir_all(&dir).unwrap();
        for bad in ["not json", "[]", r#"{"projects": []}"#] {
            std::fs::write(file(&env), bad).unwrap();
            assert!(write(&env, &dir).is_err(), "{bad}");
            assert_eq!(std::fs::read_to_string(file(&env)).unwrap(), bad);
        }
        assert!(format_check(&env).is_err());
        std::fs::write(file(&env), r#"{"projects": {"/a": {"hasTrustDialogAccepted": true}, "/b": {}}}"#).unwrap();
        assert_eq!(format_check(&env), Ok(1));
        std::fs::write(file(&env), r#"{"projects": {"/a": {"hasTrustDialogAccepted": "yes"}}}"#).unwrap();
        assert!(format_check(&env).is_err());
    }

    #[test]
    fn the_home_folder_the_root_and_a_parent_of_the_data_root_are_never_trusted() {
        let (td, env) = test_env();
        std::fs::write(file(&env), r#"{"projects": {}}"#).unwrap();
        // (`env::home`: on Windows `USERPROFILE`, as cmd and PowerShell set no `HOME`)
        let home = crate::env::home();
        for d in [Path::new("/"), home.as_path(), home.parent().unwrap()] {
            assert!(too_broad(&env, d).is_some(), "{}", d.display());
            assert!(write(&env, d).is_err(), "{}", d.display());
            let p = for_plan(&env, None, d);
            assert!(matches!(&p, Some(Err(i)) if i.code == "trust.broad") || p.is_none(), "{}: {p:?}", d.display());
            assert!(fix(d).starts_with("run `claude`"), "{}", fix(d));
        }
        // a parent of the data root; the data root itself is fine
        assert!(too_broad(&env, env.home.parent().unwrap()).is_some());
        std::fs::create_dir_all(&env.home).unwrap();
        assert!(too_broad(&env, &env.home).is_none());
        // a folder under a repository rooted at the parent of the data root: the repo is the target, refused
        let top = crate::env::canon(td.path());
        std::fs::create_dir_all(top.join(".git")).unwrap();
        assert!(too_broad(&env, &top.join("elsewhere")).is_some_and(|w| w.contains("git repository")));
        let _ = std::fs::remove_dir_all(top.join(".git"));
    }

    #[test]
    fn a_link_into_another_repository_trusts_the_repository_it_resolves_to() {
        let (td, env) = test_env();
        let top = crate::env::canon(td.path());
        let (mono, other) = (top.join("mono"), top.join("other"));
        std::fs::create_dir_all(mono.join(".git")).unwrap();
        std::fs::create_dir_all(mono.join("vendor")).unwrap();
        std::fs::create_dir_all(other.join(".git")).unwrap();
        std::fs::create_dir_all(other.join("pkg")).unwrap();
        crate::os::symlink(other.join("pkg"), mono.join("vendor/x")).unwrap();
        assert_eq!(target(&env, &mono.join("vendor/x")), other, "Claude Code resolves the link first");
        // a link to the repository itself keeps its form as given too
        crate::os::symlink(&other, top.join("o")).unwrap();
        assert_eq!(target(&env, &top.join("o/pkg")), top.join("o"));
        assert_eq!(forms(&keys(&env, &top.join("o/pkg"))), vec![key(&top.join("o")), key(&other)]);
    }

    #[test]
    fn inside_the_data_root_the_data_root_is_trusted_and_every_config_file_gets_it() {
        let (_td, env) = test_env();
        let lead = env.home.join("main/admiral");
        std::fs::create_dir_all(&lead).unwrap();
        assert_eq!(target(&env, &lead), crate::env::canon(&env.home));
        // both config files exist: both are written
        std::fs::create_dir_all(&env.claude_home).unwrap();
        let both = files(&env);
        for f in &both {
            std::fs::write(f, r#"{"projects": {}}"#).unwrap();
        }
        write(&env, &lead).unwrap();
        for f in &both {
            let v: Value = serde_json::from_slice(&std::fs::read(f).unwrap()).unwrap();
            assert_eq!(v["projects"][key(&crate::env::canon(&env.home))]["hasTrustDialogAccepted"], true, "{}", f.display());
        }
        // a backup per source, and the file as it was before nimitz ever wrote it
        let names: Vec<String> = std::fs::read_dir(env.state.join("backups")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names.iter().filter(|n| n.ends_with(".first")).count(), 2, "{names:?}");
    }

    #[test]
    fn only_the_user_gets_the_step() {
        let (td, env) = test_env();
        let dir = crate::env::canon(td.path()).join("p");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(file(&env), r#"{"projects": {}}"#).unwrap();
        let id = Id::ship("MAIN", "ARGOS", "X");
        let s = for_plan(&env, Some(&id), &dir).unwrap().unwrap();
        assert_eq!(s.cmd, vec!["trust".to_string(), dir.to_string_lossy().into_owned()]);
        assert!(s.why.contains("writes") && s.why.contains(".claude settings, hooks and MCP servers"), "{}", s.why);
    }

    /// `forget` takes the flag out of an entry without reordering the rest
    /// (preserve_order: `remove` would swap the last field into its place).
    #[test]
    fn forget_keeps_the_order_of_what_it_leaves() {
        let (td, env) = test_env();
        let dir = crate::env::canon(td.path()).join("p");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(file(&env), r#"{"projects": {}}"#).unwrap();
        write(&env, &dir).unwrap();
        let k = keys(&env, &dir).remove(0);
        let mut v = trust_json(&env);
        v["projects"][&k] = json!({"a": 1, "hasTrustDialogAccepted": true, "b": 2, "c": 3});
        std::fs::write(file(&env), v.to_string()).unwrap();
        forget(&env, &dir).unwrap();
        let order: Vec<String> = trust_json(&env)["projects"][&k].as_object().unwrap().keys().cloned().collect();
        assert_eq!(order, ["a", "b", "c"]);
    }

    /// A linked config file is written where it points; a link to nothing is
    /// refused and stays a link (never replaced by a file of nimitz's).
    #[cfg(unix)]
    #[test]
    fn a_linked_config_is_written_through_and_a_dangling_one_is_refused() {
        let (td, env) = test_env();
        let dir = crate::env::canon(td.path()).join("p");
        std::fs::create_dir_all(&dir).unwrap();
        let elsewhere = td.path().join("dotfiles/claude.json");
        std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
        std::fs::write(&elsewhere, r#"{"projects": {}}"#).unwrap();
        std::os::unix::fs::symlink(&elsewhere, file(&env)).unwrap();
        write(&env, &dir).unwrap();
        assert!(std::fs::symlink_metadata(file(&env)).unwrap().file_type().is_symlink(), "still a link");
        assert!(is_written(&env, &dir));
        std::fs::remove_file(&elsewhere).unwrap();
        let e = write(&env, &dir).unwrap_err();
        assert!(e.contains("does not exist") && e.contains(&elsewhere.display().to_string()), "{e}");
        assert!(std::fs::symlink_metadata(file(&env)).unwrap().file_type().is_symlink(), "the link stays");
        assert!(!elsewhere.exists());
    }

    /// Windows: each key in its trimmed form and `/` form (case kept), and a
    /// fix line only for a path one double-quoted word carries in cmd and
    /// PowerShell alike; else the trust prompt by hand, spelled as on disk.
    #[cfg(windows)]
    #[test]
    fn windows_keys_come_in_two_forms_and_the_fix_quotes_only_what_both_shells_take() {
        assert_eq!(key_forms(Path::new(r"C:\Users\Me\proj\")), vec![r"C:\Users\Me\proj".to_string(), "C:/Users/Me/proj".into()]);
        assert_eq!(key_forms(Path::new(r"C:\")), vec![r"C:\".to_string(), "C:/".into()]);
        assert_eq!(key_forms(Path::new("D:/x")), vec!["D:/x".to_string()]);
        assert_eq!(quoted(Path::new(r"C:\work\proj")).as_deref(), Some(r"C:\work\proj"));
        assert_eq!(quoted(Path::new(r"C:\my work\proj")).as_deref(), Some(r#""C:\my work\proj""#));
        assert_eq!(quoted(Path::new(r#"C:\a"b"#)).as_deref(), Some(r#""C:\a""b""#));
        for odd in [r"C:\100%\x", r"C:\$x\y", r"C:\a`b"] {
            assert_eq!(quoted(Path::new(odd)), None, "{odd}");
            let f = fix(Path::new(odd));
            assert!(f.starts_with("run `claude` once in ") && f.contains("Set-Location -LiteralPath"), "{f}");
        }
        // the home folder is `USERPROFILE` when there is no `HOME` (cmd, PowerShell)
        let (_td, env) = test_env();
        let profile = PathBuf::from(std::env::var_os("USERPROFILE").expect("USERPROFILE"));
        assert!(too_broad(&env, &profile).is_some());
        assert!(fix(&profile).starts_with("run `claude`"), "{}", fix(&profile));
    }
}
