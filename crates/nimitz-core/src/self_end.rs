//! The self-end helper's state (docs/design/cli-ux.md section 4): a member's
//! command that would end or replace the member itself (its own relaunch, its
//! stop, a retire that takes it) runs that step last, after the command has
//! printed its report, in a detached `nimitz __self-end` that waits for the
//! member's turn to end. Here: the lock (one helper per member), the log, and
//! the note that tells the member at its next SessionStart what came of it.
use crate::env::Env;
use crate::types::Id;
use std::path::PathBuf;

/// How long the helper waits for the caller's turn to end before it gives up
/// (decision 5: never cut a turn the caller is still in).
pub const WAIT_S: f64 = 600.0;

/// A lock older than the longest wait and this margin is stale, whoever
/// wrote it (a pid the system reused never blocks a member for good).
const STALE_AFTER_S: f64 = WAIT_S + 300.0;

/// A claim the CLI took for a helper that never wrote its pid is stale
/// after this long.
const PENDING_S: f64 = 60.0;

pub fn log_path(env: &Env) -> PathBuf {
    env.state.join("relaunch-self.log")
}

fn dir(env: &Env) -> PathBuf {
    env.state.join("self-end")
}

/// One file name per member: its id with `.` for `/`.
fn file_of(id: &Id) -> String {
    id.0.replace('/', ".")
}

pub fn lock_path(env: &Env, id: &Id) -> PathBuf {
    dir(env).join(format!("{}.lock", file_of(id)))
}

fn note_path(env: &Env, id: &Id) -> PathBuf {
    dir(env).join(format!("{}.txt", file_of(id)))
}

/// Who holds `id`'s lock: `Some(what)` while its helper is pending or
/// running (`pid N T`: that pid, live, since T; `pending T`: the CLI's
/// claim, fresh). A stale lock reads as none.
pub fn pending(env: &Env, id: &Id) -> Option<String> {
    let text = std::fs::read_to_string(lock_path(env, id)).ok()?;
    let w: Vec<&str> = text.split_whitespace().collect();
    let num = |i: usize| w.get(i).and_then(|x| x.parse::<f64>().ok());
    let age = |at: f64| crate::evidence::now() - at;
    match (w.first().copied(), num(1), num(2)) {
        (Some("pid"), Some(pid), Some(at)) if age(at) < STALE_AFTER_S && crate::os::pid_alive(pid as i64) => Some(format!("helper pid {pid}")),
        (Some("pending"), Some(at), _) if age(at) < PENDING_S => Some("a helper starting".into()),
        _ => None,
    }
}

/// Take `id`'s lock for a helper about to start (`Err`: one is pending for
/// it). Created exclusively: of two at once, one wins; a stale one is
/// replaced.
pub fn claim(env: &Env, id: &Id) -> Result<(), String> {
    use std::io::Write;
    let _ = std::fs::create_dir_all(dir(env));
    let p = lock_path(env, id);
    for _ in 0..2 {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(mut f) => return f.write_all(format!("pending {}\n", crate::evidence::now()).as_bytes()).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => match pending(env, id) {
                Some(who) => return Err(format!("a self-end of {id} is pending already ({who}; log {})", log_path(env).display())),
                None => {
                    let _ = std::fs::remove_file(&p);
                }
            },
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(format!("could not take {}", p.display()))
}

/// The helper's own pid in `id`'s lock, once it runs.
pub fn hold(env: &Env, id: &Id, pid: u32) {
    let _ = crate::config::atomic_write(&lock_path(env, id), &format!("pid {pid} {}\n", crate::evidence::now()));
}

pub fn release(env: &Env, id: &Id) {
    let _ = std::fs::remove_file(lock_path(env, id));
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
    let _ = std::fs::create_dir_all(dir(env));
    let _ = crate::config::atomic_write(&note_path(env, id), text);
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

/// Drop the notes of `id` and every member under it (a retire): a later
/// member with the same id never reads them.
pub fn forget_under(env: &Env, id: &Id) {
    let me = file_of(id);
    for e in std::fs::read_dir(dir(env)).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if let Some(stem) = n.strip_suffix(".txt") {
            if stem == me || stem.starts_with(&format!("{me}.")) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_helper_per_member_and_a_note_is_read_once() {
        let (_td, env) = crate::config::tests::test_env();
        let (id, other) = (Id::ship("F", "SQ", "S"), Id::ship("F", "SQ", "T"));
        assert!(pending(&env, &id).is_none());
        claim(&env, &id).unwrap();
        assert!(claim(&env, &id).unwrap_err().contains("pending already"), "a second claim for it");
        assert!(claim(&env, &other).is_ok(), "another member's own lock");
        hold(&env, &id, std::process::id());
        assert!(pending(&env, &id).is_some_and(|w| w.contains("pid")));
        release(&env, &id);
        assert!(pending(&env, &id).is_none());
        // a dead helper's lock, an old one (a reused pid), a claim never taken up: stale
        for stale in ["pid 0 99999999999\n".to_string(), format!("pid {} 1\n", std::process::id()), "pending 1\n".into()] {
            std::fs::write(lock_path(&env, &id), stale).unwrap();
            assert!(pending(&env, &id).is_none() && claim(&env, &id).is_ok());
            release(&env, &id);
        }
        assert!(take_note(&env, &id).is_none());
        write_note(&env, &id, "it ran");
        assert_eq!(take_note(&env, &id).as_deref(), Some("it ran"));
        assert!(take_note(&env, &id).is_none());
        write_note(&env, &id, "x");
        write_note(&env, &Id::ship("F", "SQ2", "S"), "y");
        forget_under(&env, &Id::commodore("F", "SQ"));
        assert!(!has_note(&env, &id) && has_note(&env, &Id::ship("F", "SQ2", "S")), "the squadron's notes go, not SQ2's");
        log(&env, "a line");
        assert!(std::fs::read_to_string(log_path(&env)).unwrap().contains("a line"));
    }
}
