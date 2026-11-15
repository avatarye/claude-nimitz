//! Leads held down on purpose (#4): `nimitz stop` of an admiral or a
//! commodore records it here, and `muster` and `layout` then leave that lead
//! down, with no wake and no tab, until something brings it up on purpose.
//!
//! Only nimitz's own stop records a hold. A job that merely ended holds
//! nothing: a stop by hand (`claude stop`, `/stop`), a crash, the daemon's
//! retire, a reboot. A clean OS shutdown fires SessionEnd (reason `other`)
//! on every live worker (a field observation, measured 2026-10-31 on Linux),
//! the reason most deliberate stops give too, so the event log cannot tell a
//! reboot from a stop.
//!
//! The record goes with anything that brings the lead up: a launch, wake or
//! relaunch `sessions::apply` runs for it (`start`, `open`, `restart`,
//! `muster --parked`, Admiralty), the member's own SessionStart on its job
//! (any path, an attach by hand included), and its retire (`fleet rm`,
//! `squadron rm`, and their `/...` forms). A hold whose lead came up anyway
//! (a SessionStart on its job after it, which the hook could not record) is
//! void (`current`), and a writing command drops it (`prune`).
//!
//! `NIMITZ_STATE/held.json` (artifact `held`, version 1, kept in the
//! manifest), a local file an older nimitz never reads (it wakes the lead, as
//! before): one JSON object, `{"<id>": {"at": <epoch seconds>, "job": "<short
//! id>"}}` (`job` is the job the stop ended, or null). Missing or unreadable:
//! no lead is held.
use crate::env::Env;
use crate::types::{Event, Id, Member, Snapshot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One lead held down: when, and the job its stop ended.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Hold {
    pub at: f64,
    #[serde(default)]
    pub job: Option<String>,
}

pub fn path(env: &Env) -> PathBuf {
    env.state.join("held.json")
}

/// Every hold (an unreadable file holds nothing: the leads come back).
pub fn load(env: &Env) -> BTreeMap<String, Hold> {
    std::fs::read(path(env)).ok().and_then(|b| crate::evidence::first_json(&b)).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

pub fn is_held(env: &Env, id: &Id) -> bool {
    path(env).exists() && load(env).contains_key(&id.0)
}

/// The file's text for `holds`.
pub fn text(holds: &BTreeMap<String, Hold>) -> String {
    serde_json::to_string_pretty(holds).unwrap_or_else(|_| "{}".into())
}

/// How long a command waits for `held.lock`.
const WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// How long the hook waits: a hook never hangs a session, and a hold it
/// could not drop is void anyway once the lead's start is on record (`current`).
pub const HOOK_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// Read, change, write `held.json` under its own lock, waited for up to `wait`.
fn update(env: &Env, wait: std::time::Duration, f: impl FnOnce(&mut BTreeMap<String, Hold>) -> bool) -> std::io::Result<bool> {
    let dir = env.state.join("locks");
    std::fs::create_dir_all(&dir)?;
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("held.lock"))?;
    let until = std::time::Instant::now() + wait;
    while lock.try_lock().is_err() {
        if std::time::Instant::now() >= until {
            return Err(std::io::Error::other(format!("{} is busy (waited {:?})", dir.join("held.lock").display(), wait)));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let mut holds = load(env);
    let changed = f(&mut holds);
    if changed {
        crate::config::atomic_write(&path(env), &(text(&holds) + "\n"))?;
    }
    Ok(changed)
}

/// Hold `id` down (`nimitz stop` of a lead).
pub fn hold(env: &Env, id: &Id, job: Option<&str>) -> std::io::Result<()> {
    let h = Hold { at: crate::evidence::now(), job: job.map(str::to_string) };
    update(env, WAIT, |m| {
        m.insert(id.0.clone(), h);
        true
    })
    .map(|_| ())
}

/// Let `id` come back: whether it was held. A file that is not there is not
/// touched.
pub fn release(env: &Env, id: &Id) -> std::io::Result<bool> {
    release_within(env, id, WAIT)
}

/// `release`, waiting for the lock up to `wait` (the hook: `HOOK_WAIT`).
pub fn release_within(env: &Env, id: &Id, wait: std::time::Duration) -> std::io::Result<bool> {
    if !path(env).exists() {
        return Ok(false);
    }
    update(env, wait, |m| m.remove(&id.0).is_some())
}

/// Did the lead come up after `at`: a SessionStart on its job `job` after
/// it, or its live process (on that job) started after it. Only a start on
/// the job counts: a process with no job that holds the conversation
/// (Herdr's resume after a restart, an SDK host) is no lead up.
pub fn started_since(at: f64, job: Option<&str>, live_started: Option<f64>, events: &[Event]) -> bool {
    let Some(job) = job else { return false };
    live_started.is_some_and(|t| t > at) || events.iter().any(|e| e.event == "SessionStart" && e.ts > at && e.job.as_deref() == Some(job))
}

/// A hold its lead outlived (`started_since`).
fn void(hold: &Hold, m: &Member, events: &[Event]) -> bool {
    let s = m.session.as_ref();
    let job = s.and_then(|s| s.short_id.as_deref());
    let live = s.filter(|s| s.pid.is_some()).and_then(|s| s.started_at);
    started_since(hold.at, job, live, events)
}

/// The holds that stand: every hold but the void ones (read-only). A hold
/// of a member the snapshot does not have stands.
pub fn current(env: &Env, snap: &Snapshot) -> BTreeMap<String, Hold> {
    let holds = if path(env).exists() { load(env) } else { BTreeMap::new() };
    if holds.is_empty() {
        return holds;
    }
    let events = crate::evidence::events(env);
    let members: Vec<&Member> = snap.fleets.iter().flat_map(|f| f.members()).collect();
    holds.into_iter().filter(|(id, h)| !members.iter().find(|m| &m.id.0 == id).is_some_and(|m| void(h, m, &events))).collect()
}

/// Drop the holds `current` finds void: how many (a writing command's).
pub fn prune(env: &Env, snap: &Snapshot) -> std::io::Result<usize> {
    let all = if path(env).exists() { load(env) } else { BTreeMap::new() };
    let keep = current(env, snap);
    let gone: Vec<String> = all.keys().filter(|k| !keep.contains_key(*k)).cloned().collect();
    if gone.is_empty() {
        return Ok(0);
    }
    update(env, WAIT, |m| {
        for g in &gone {
            m.remove(g);
        }
        true
    })?;
    Ok(gone.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_is_kept_until_released_and_an_unreadable_file_holds_nothing() {
        let td = tempfile::tempdir().unwrap();
        let env = Env::from_vars(&[("NIMITZ_STATE".to_string(), td.path().to_string_lossy().into_owned())].into_iter().collect());
        let main = Id::admiral("MAIN");
        assert!(!is_held(&env, &main));
        assert!(!release(&env, &main).unwrap(), "no file: nothing to release, nothing written");
        assert!(!path(&env).exists());
        hold(&env, &main, Some("ab12cd34")).unwrap();
        hold(&env, &Id::commodore("MAIN", "ARGOS"), None).unwrap();
        assert!(is_held(&env, &main) && is_held(&env, &Id::commodore("MAIN", "ARGOS")));
        assert_eq!(load(&env)[&main.0].job.as_deref(), Some("ab12cd34"));
        assert!(release(&env, &main).unwrap());
        assert!(!is_held(&env, &main) && is_held(&env, &Id::commodore("MAIN", "ARGOS")));
        // a busy lock: given up after the wait, nothing written
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(td.path().join("locks").join("held.lock")).unwrap();
        lock.lock().unwrap();
        assert!(release_within(&env, &Id::commodore("MAIN", "ARGOS"), std::time::Duration::from_millis(50)).is_err());
        assert!(is_held(&env, &Id::commodore("MAIN", "ARGOS")));
        drop(lock);
        // a start on the job after the hold voids it; one on another job, or before, does not
        let at = load(&env)[&Id::commodore("MAIN", "ARGOS").0].at;
        let start = |ts: f64, job: Option<&str>| Event { ts, event: "SessionStart".into(), job: job.map(str::to_string), ..serde_json::from_value(serde_json::json!({"ts": 0.0, "event": "x"})).unwrap() };
        assert!(started_since(at, Some("j1"), None, &[start(at + 1.0, Some("j1"))]));
        assert!(!started_since(at, Some("j1"), None, &[start(at + 1.0, Some("j2")), start(at - 1.0, Some("j1")), start(at + 1.0, None)]));
        assert!(started_since(at, Some("j1"), Some(at + 2.0), &[]) && !started_since(at, None, Some(at + 2.0), &[]));
        std::fs::write(path(&env), "not json").unwrap();
        assert!(!is_held(&env, &Id::commodore("MAIN", "ARGOS")), "unreadable: the leads come back");
    }
}
