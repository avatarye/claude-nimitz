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
//! The user's alone: trust lets the folder's own `.claude` settings, hooks
//! and MCP servers run, so a member gets an authority refusal and the fix
//! goes to the user.
use crate::env::Env;
use crate::types::{Caller, Id, Issue, Severity, Step};
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

/// Claude Code's config file (it holds the trust table).
pub fn file(env: &Env) -> PathBuf {
    crate::check::trust_file(env)
}

fn abs(dir: &Path) -> PathBuf {
    let d = if dir.is_absolute() { dir.to_path_buf() } else { std::env::current_dir().map(|c| c.join(dir)).unwrap_or_else(|_| dir.to_path_buf()) };
    // a path with `..` is not one Claude Code would see as given: the real one stands
    if d.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return crate::env::canon(&d);
    }
    d
}

fn key(p: &Path) -> String {
    let s = p.to_string_lossy();
    let t = s.trim_end_matches('/');
    if t.is_empty() { "/".into() } else { t.to_string() }
}

/// The folder Claude Code asks about for `dir`, as given: the git root that
/// holds it (found from the path as given, else from the real path), else
/// the folder itself.
pub fn target(dir: &Path) -> PathBuf {
    let given = abs(dir);
    if let Some(root) = given.ancestors().find(|a| a.join(".git").exists()) {
        return root.to_path_buf();
    }
    crate::env::git_root(&given).unwrap_or(given)
}

/// The keys written for `dir`: the target as given and its real path.
pub fn keys(dir: &Path) -> Vec<String> {
    let t = target(dir);
    let mut out = vec![key(&t)];
    let real = key(&crate::env::canon(&t));
    if !out.contains(&real) {
        out.push(real);
    }
    out
}

/// The target's real path (what messages and fixes name).
pub fn shown(dir: &Path) -> PathBuf {
    crate::env::canon(&target(dir))
}

fn quoted(p: &Path) -> String {
    let s = p.to_string_lossy();
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-~:+@".contains(c)) { s.into_owned() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

/// The fix for an untrusted folder.
pub fn fix(dir: &Path) -> String {
    format!("nimitz trust {}", quoted(&shown(dir)))
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
    let home = std::env::var("HOME").ok().map(PathBuf::from);
    match home.as_deref().and_then(|h| f.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => f.display().to_string(),
    }
}

/// The plan step that trusts `dir`.
pub fn step(env: &Env, id: Option<Id>, dir: &Path) -> Step {
    Step {
        id,
        cmd: vec!["trust".into(), target(dir).to_string_lossy().into_owned()],
        cwd: None,
        why: format!("trust {} (writes {}): {CONSEQUENCE}", shown(dir).display(), file_shown(env)),
    }
}

/// What a plan gets for a member folder `dir` that Claude Code will not
/// trust: the user, a trust step; anyone else, an issue whose fix is the
/// user's (`nimitz trust` is never a member's). None when it is trusted.
pub fn for_plan(env: &Env, caller: &Caller, id: Option<&Id>, dir: &Path) -> Option<Result<Step, Issue>> {
    let severity = untrusted(env, dir)?;
    if *caller == Caller::User {
        return Some(Ok(step(env, id.cloned(), dir)));
    }
    Some(Err(Issue {
        code: "folder.untrusted".into(),
        severity,
        message: format!("{} is not trusted by Claude Code; trusting it is the user's ({CONSEQUENCE})", shown(dir).display()),
        fix: Some(fix(dir)),
        id: id.cloned(),
    }))
}

/// Does the config file trust every key of `dir`?
pub fn is_written(env: &Env, dir: &Path) -> bool {
    let Some(v) = std::fs::read(file(env)).ok().and_then(|b| crate::evidence::first_json(&b)) else { return false };
    keys(dir).iter().all(|k| v["projects"][k]["hasTrustDialogAccepted"].as_bool() == Some(true))
}

/// Trust `dir` (its target, both keys). Returns the keys it set (empty: all
/// were set already).
pub fn write(env: &Env, dir: &Path) -> Result<Vec<String>, String> {
    let keys = keys(dir);
    let mut set = vec![];
    edit(env, |projects| {
        for k in &keys {
            let e = projects.entry(k.clone()).or_insert_with(|| json!({}));
            let Some(o) = e.as_object_mut() else { return Err(format!("projects[{k:?}] is not an object")) };
            if o.get("hasTrustDialogAccepted").and_then(Value::as_bool) != Some(true) {
                o.insert("hasTrustDialogAccepted".into(), Value::Bool(true));
                set.push(k.clone());
            }
        }
        Ok(!set.is_empty())
    })?;
    Ok(set)
}

/// Undo `write` for `dir` (tests and e2e cleanup): the flag goes, and an
/// entry left empty goes too.
pub fn forget(env: &Env, dir: &Path) -> Result<(), String> {
    let keys = keys(dir);
    edit(env, |projects| {
        let mut changed = false;
        for k in &keys {
            if let Some(o) = projects.get_mut(k).and_then(Value::as_object_mut) {
                changed |= o.remove("hasTrustDialogAccepted").is_some();
                if o.is_empty() {
                    projects.remove(k);
                }
            }
        }
        Ok(changed)
    })
    .map(|_| ())
}

/// doctor's pinned-format check: the file is a JSON object, `projects` an
/// object of objects, `hasTrustDialogAccepted` a bool wherever it is set.
/// Ok(number of trusted keys); Err(what differs). No file: Ok(0).
pub fn format_check(env: &Env) -> Result<usize, String> {
    let f = file(env);
    let raw = match std::fs::read(&f) {
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
fn edit(env: &Env, f: impl FnOnce(&mut Map<String, Value>) -> Result<bool, String>) -> Result<bool, String> {
    let path = file(env);
    let lock = PathBuf::from(format!("{}.lock", path.display()));
    let _held = crate::pins::dir_lock(&lock, STALE_S, WAIT).map_err(|e| format!("{e} (Claude Code is writing {}; try again)", path.display()))?;
    // a linked config file is written where it points (Claude Code's own
    // write follows the link too)
    let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
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
        backup(env, b).map_err(|e| format!("backup of {}: {e}", real.display()))?;
    }
    let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    write_like(&real, &text).map_err(|e| format!("{}: {e}", real.display()))?;
    Ok(true)
}

/// `NIMITZ_STATE/backups/claude.json.<ms>`, the newest `KEEP_BACKUPS` kept.
fn backup(env: &Env, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = env.state.join("backups");
    std::fs::create_dir_all(&dir)?;
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let p = dir.join(format!("claude.json.{ms}"));
    write_like(&p, std::str::from_utf8(bytes).unwrap_or_default()).or_else(|_| std::fs::write(&p, bytes))?;
    let mut old: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("claude.json.")))
        .collect();
    old.sort_by_key(|p| p.file_name().and_then(|n| n.to_str()).and_then(|n| n.rsplit('.').next()).and_then(|t| t.parse::<u128>().ok()).unwrap_or(0));
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

    #[test]
    fn a_folder_outside_git_is_trusted_as_itself_and_by_its_real_path() {
        let (td, env) = test_env();
        let real = crate::env::canon(td.path()).join("work/proj");
        std::fs::create_dir_all(&real).unwrap();
        let link = td.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(real.parent().unwrap(), &link).unwrap();
        let given = link.join("proj");
        assert_eq!(keys(&given), vec![key(&given), key(&real)]);
        std::fs::write(file(&env), r#"{"numStartups": 3, "projects": {"/x": {"allowedTools": []}}}"#).unwrap();
        assert!(untrusted(&env, &given).is_some());
        let set = write(&env, &given).unwrap();
        assert_eq!(set.len(), 2);
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
        assert_eq!(target(&repo.join("src/deep")), repo);
        write(&env, &repo.join("src/deep")).unwrap();
        assert_eq!(trust_json(&env)["projects"][key(&repo)]["hasTrustDialogAccepted"], true);
        assert!(trust_json(&env)["projects"].get(key(&repo.join("src/deep"))).is_none());
        assert!(untrusted(&env, &repo.join("src/deep")).is_none());
        assert_eq!(fix(&repo.join("src")), format!("nimitz trust {}", repo.display()));
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
        std::fs::File::open(&lock).unwrap().set_modified(old).unwrap();
        forget(&env, &dir).unwrap();
        assert!(!is_written(&env, &dir));
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
        assert_eq!(backups.len(), KEEP_BACKUPS);
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
    fn only_the_user_gets_the_step() {
        let (td, env) = test_env();
        let dir = crate::env::canon(td.path()).join("p");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(file(&env), r#"{"projects": {}}"#).unwrap();
        let id = Id::ship("MAIN", "ARGOS", "X");
        let s = for_plan(&env, &Caller::User, Some(&id), &dir).unwrap().unwrap();
        assert_eq!(s.cmd, vec!["trust".to_string(), dir.to_string_lossy().into_owned()]);
        assert!(s.why.contains("writes") && s.why.contains(".claude settings, hooks and MCP servers"), "{}", s.why);
        let member = Caller::Member { id: Id("MAIN/ARGOS".into()), role: crate::types::Role::Commodore };
        let i = for_plan(&env, &member, Some(&id), &dir).unwrap().unwrap_err();
        assert_eq!(i.fix.as_deref(), Some(format!("nimitz trust {}", dir.display()).as_str()));
    }
}
