//! The self-end helper's state (docs/design/cli-ux.md section 4): a member's
//! command that would end or replace the member itself (its own relaunch, its
//! stop, a retire that takes it) runs that step last, after the command has
//! printed its report, in a detached `nimitz __self-end` that waits for the
//! member's turn to end. Here: the lock (one helper at a time), the log, and
//! the note that tells the member at its next SessionStart what came of it.
use crate::env::Env;
use crate::types::Id;
use std::path::PathBuf;

/// How long the helper waits for the caller's turn to end before it gives up
/// (decision 5: never cut a turn the caller is still in).
pub const WAIT_S: f64 = 600.0;

/// A lock the CLI took for a helper that has not written its pid yet is
/// stale after this long.
const PENDING_S: f64 = 60.0;

pub fn log_path(env: &Env) -> PathBuf {
    env.state.join("relaunch-self.log")
}

pub fn lock_path(env: &Env) -> PathBuf {
    env.state.join("relaunch-self.lock")
}

fn note_path(env: &Env, id: &Id) -> PathBuf {
    env.state.join("self-end").join(format!("{}.txt", id.0.replace('/', ".")))
}

/// Who holds the lock: `Some(what)` while a helper is pending or running (its
/// pid, live; or the CLI's claim, fresh). A stale lock reads as none.
pub fn pending(env: &Env) -> Option<String> {
    let text = std::fs::read_to_string(lock_path(env)).ok()?;
    let mut w = text.split_whitespace();
    match (w.next(), w.next().and_then(|x| x.parse::<f64>().ok())) {
        (Some("pid"), Some(pid)) if crate::os::pid_alive(pid as i64) => Some(format!("helper pid {pid}")),
        (Some("pending"), Some(at)) if crate::evidence::now() - at < PENDING_S => Some("a helper starting".into()),
        _ => None,
    }
}

/// Take the lock for a helper about to start (`Err`: one is pending).
pub fn claim(env: &Env) -> Result<(), String> {
    if let Some(who) = pending(env) {
        return Err(format!("a self-end is pending already ({who}; log {})", log_path(env).display()));
    }
    let _ = std::fs::create_dir_all(&env.state);
    crate::config::atomic_write(&lock_path(env), &format!("pending {}\n", crate::evidence::now())).map_err(|e| e.to_string())
}

/// The helper's own pid in the lock, once it runs.
pub fn hold(env: &Env, pid: u32) {
    let _ = crate::config::atomic_write(&lock_path(env), &format!("pid {pid}\n"));
}

pub fn release(env: &Env) {
    let _ = std::fs::remove_file(lock_path(env));
}

/// Append one line to the log.
pub fn log(env: &Env, line: &str) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(&env.state);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_path(env)) {
        let _ = writeln!(f, "{:.0} {line}", crate::evidence::now());
    }
}

/// The outcome for `id`'s next SessionStart (the last one wins).
pub fn write_note(env: &Env, id: &Id, text: &str) {
    let p = note_path(env, id);
    let _ = std::fs::create_dir_all(p.parent().expect("a folder"));
    let _ = crate::config::atomic_write(&p, text);
}

/// Is an outcome waiting for `id`?
pub fn has_note(env: &Env, id: &Id) -> bool {
    note_path(env, id).is_file()
}

/// The outcome waiting for `id`, taken (read once, then gone).
pub fn take_note(env: &Env, id: &Id) -> Option<String> {
    let p = note_path(env, id);
    let text = std::fs::read_to_string(&p).ok()?;
    let _ = std::fs::remove_file(&p);
    Some(text).filter(|t| !t.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_helper_at_a_time_and_a_note_is_read_once() {
        let (_td, env) = crate::config::tests::test_env();
        assert!(pending(&env).is_none());
        claim(&env).unwrap();
        assert!(claim(&env).unwrap_err().contains("pending already"), "a second claim");
        hold(&env, std::process::id());
        assert!(pending(&env).is_some_and(|w| w.contains("pid")));
        release(&env);
        assert!(pending(&env).is_none());
        // a dead helper's lock, or a claim never taken up, is stale
        hold(&env, 0);
        assert!(pending(&env).is_none());
        std::fs::write(lock_path(&env), "pending 1\n").unwrap();
        assert!(pending(&env).is_none() && claim(&env).is_ok());
        let id = Id::ship("F", "SQ", "S");
        assert!(take_note(&env, &id).is_none());
        write_note(&env, &id, "it ran");
        assert_eq!(take_note(&env, &id).as_deref(), Some("it ran"));
        assert!(take_note(&env, &id).is_none());
        log(&env, "a line");
        assert!(std::fs::read_to_string(log_path(&env)).unwrap().contains("a line"));
    }
}
