//! Pins: `<claude home>/jobs/pins.json`, the jobs Claude Code's daemon never
//! idle-retires (plan-v4 section 7, M24). Owner: core-sessions.
//!
//! nimitz pins each lead's job after it starts (a lead must stay up; a ship
//! parks and wakes on demand) and removes the pin when the lead's row goes
//! (`claude rm` leaves the id behind, env-G9). It never touches an id it did
//! not pin itself: `NIMITZ_STATE/pins.json` lists nimitz's own.
//!
//! The file is Claude Code's, edited under the lock Claude Code takes for it
//! (proper-lockfile, as the binary's "pins.json lock compromised" path shows:
//! `stale: 5000` ms, the holder refreshing the mtime every 2.5 s): the
//! directory `pins.json.lock`, made with `mkdir`; a lock whose mtime is older
//! than the stale time is a dead holder's and is removed. nimitz waits for a
//! lock to be 10 s old (`STALE_S`, twice Claude Code's 5 s): a live holder
//! refreshes it well inside that, and taking it one refresh too late would
//! cost nothing but a wait, while taking a live one could lose a pin. nimitz
//! holds it for milliseconds: read, edit, write a temp file, rename, `rmdir`.
use crate::env::Env;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A lock directory older than this is a dead holder's. Claude Code's own
/// rule is 5 s (`stale: 5000`), its holder refreshing the mtime every 2.5 s;
/// nimitz waits twice that before it takes a lock over.
pub const STALE_S: f64 = 10.0;
/// How long nimitz waits for the lock before it gives up.
const WAIT: Duration = Duration::from_secs(3);

pub fn pins_path(env: &Env) -> PathBuf {
    env.claude_home.join("jobs").join("pins.json")
}

/// The ids nimitz pinned itself (the only ones it ever removes).
pub fn own_path(env: &Env) -> PathBuf {
    env.state.join("pins.json")
}

fn lock_path(env: &Env) -> PathBuf {
    env.claude_home.join("jobs").join("pins.json.lock")
}

/// A proper-lockfile lock directory (`mkdir`), held until dropped. Claude
/// Code takes its files' locks this way (`pins.json`, `.claude.json`).
pub(crate) struct DirLock(PathBuf);

impl Drop for DirLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn lock(env: &Env) -> std::io::Result<DirLock> {
    dir_lock(&lock_path(env), STALE_S, WAIT)
}

/// Take the lock directory `p`: a lock older than `stale_s` is a dead
/// holder's and is taken over; otherwise wait up to `wait`.
pub(crate) fn dir_lock(p: &std::path::Path, stale_s: f64, wait: Duration) -> std::io::Result<DirLock> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let deadline = Instant::now() + wait;
    loop {
        match std::fs::create_dir(p) {
            Ok(()) => return Ok(DirLock(p.to_path_buf())),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let age = crate::evidence::mtime_secs(p).map(|t| crate::evidence::now() - t).unwrap_or(0.0);
                if age > stale_s {
                    let _ = std::fs::remove_dir(p); // a dead holder's; the next mkdir takes it
                    continue;
                }
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, format!("{} is held by another process", p.display())));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    }
}

fn read_list(p: &std::path::Path) -> Vec<String> {
    std::fs::read(p)
        .ok()
        .and_then(|b| crate::evidence::first_json(&b))
        .and_then(|v| v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()))
        .unwrap_or_default()
}

fn write_list(p: &std::path::Path, list: &[String]) -> std::io::Result<()> {
    crate::config::atomic_write(p, &serde_json::to_string_pretty(list).unwrap_or_else(|_| "[]".into()))
}

/// Edit the pin list under the lock; `f` says whether it changed anything.
fn edit(env: &Env, f: impl FnOnce(&mut Vec<String>) -> bool) -> std::io::Result<bool> {
    let _lock = lock(env)?;
    let p = pins_path(env);
    let mut list = read_list(&p);
    if !f(&mut list) {
        return Ok(false);
    }
    write_list(&p, &list)?;
    Ok(true)
}

/// Is `short` pinned (by anyone)?
pub fn is_pinned(env: &Env, short: &str) -> bool {
    read_list(&pins_path(env)).iter().any(|p| p == short)
}

/// The ids nimitz pinned itself.
pub fn own(env: &Env) -> Vec<String> {
    read_list(&own_path(env))
}

fn record_own(env: &Env, short: &str, add: bool) -> std::io::Result<()> {
    let mut list = own(env);
    let had = list.iter().any(|p| p == short);
    if add == had {
        return Ok(());
    }
    if add {
        list.push(short.to_string());
    } else {
        list.retain(|p| p != short);
    }
    write_list(&own_path(env), &list)
}

/// Pin `short` (a lead's job) and remember it as nimitz's own. Ok(true) when
/// the pin was added now.
pub fn pin(env: &Env, short: &str) -> std::io::Result<bool> {
    let added = edit(env, |list| {
        if list.iter().any(|p| p == short) {
            return false;
        }
        list.push(short.to_string());
        true
    })?;
    record_own(env, short, true)?;
    Ok(added)
}

/// Remove nimitz's own pin of `short` (a row that is gone). A pin nimitz did
/// not make stays: Ok(false). The job's own record is dropped either way.
pub fn unpin(env: &Env, short: &str) -> std::io::Result<bool> {
    if !own(env).iter().any(|p| p == short) {
        return Ok(false);
    }
    let removed = edit(env, |list| {
        let n = list.len();
        list.retain(|p| p != short);
        list.len() != n
    })?;
    record_own(env, short, false)?;
    Ok(removed)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::tests::test_env;

    #[test]
    fn pin_and_unpin_touch_only_nimitzs_own_ids_under_the_lock() {
        let (_td, env) = test_env();
        // someone else's pin is already there (the agent view's Ctrl+T)
        std::fs::create_dir_all(pins_path(&env).parent().unwrap()).unwrap();
        std::fs::write(pins_path(&env), r#"["theirs1"]"#).unwrap();
        assert!(pin(&env, "lead1").unwrap());
        assert!(!pin(&env, "lead1").unwrap(), "pinned already");
        assert_eq!(read_list(&pins_path(&env)), ["theirs1", "lead1"]);
        assert_eq!(own(&env), ["lead1"]);
        assert!(is_pinned(&env, "theirs1") && is_pinned(&env, "lead1"));
        // not nimitz's: left alone
        assert!(!unpin(&env, "theirs1").unwrap());
        assert!(is_pinned(&env, "theirs1"));
        assert!(unpin(&env, "lead1").unwrap());
        assert_eq!(read_list(&pins_path(&env)), ["theirs1"]);
        assert!(own(&env).is_empty());
        assert!(!lock_path(&env).exists(), "the lock is released");
        // a pin nimitz made that someone else removed meanwhile: the record goes too
        assert!(pin(&env, "lead2").unwrap());
        std::fs::write(pins_path(&env), r#"["theirs1"]"#).unwrap();
        assert!(!unpin(&env, "lead2").unwrap());
        assert!(own(&env).is_empty());
    }

    /// A folder opened to set its times (Windows opens a folder only with
    /// `FILE_FLAG_BACKUP_SEMANTICS`, and needs write access for the times).
    pub(crate) fn dir_handle(p: &std::path::Path) -> std::fs::File {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().write(true).custom_flags(0x0200_0000).open(p).unwrap()
        }
        #[cfg(not(windows))]
        {
            std::fs::File::open(p).unwrap()
        }
    }

    #[test]
    fn a_stale_lock_is_taken_over_and_a_live_one_is_waited_for() {
        let (_td, env) = test_env();
        let l = lock_path(&env);
        std::fs::create_dir_all(&l).unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs_f64(STALE_S + 5.0);
        dir_handle(&l).set_modified(old).unwrap();
        assert!(pin(&env, "lead1").unwrap(), "a lock older than {STALE_S} s is a dead holder's");
        assert!(!l.exists());
        // a fresh lock: held by a live Claude Code; nimitz waits, then gives up
        std::fs::create_dir_all(&l).unwrap();
        let t = Instant::now();
        let err = pin(&env, "lead2").unwrap_err();
        assert!(t.elapsed() >= WAIT && err.kind() == std::io::ErrorKind::WouldBlock, "{err}");
        assert!(!is_pinned(&env, "lead2"));
        std::fs::remove_dir(&l).unwrap();
    }
}
