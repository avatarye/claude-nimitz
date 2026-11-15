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
//! `squadron rm`, and their `/...` forms).
//!
//! `NIMITZ_STATE/held.json` (artifact `held`, version 1, kept in the
//! manifest), a local file an older nimitz never reads (it wakes the lead, as
//! before): one JSON object, `{"<id>": {"at": <epoch seconds>, "job": "<short
//! id>"}}` (`job` is the job the stop ended, or null). Missing or unreadable:
//! no lead is held.
use crate::env::Env;
use crate::types::Id;
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

/// Read, change, write `held.json` under its own lock.
fn update(env: &Env, f: impl FnOnce(&mut BTreeMap<String, Hold>) -> bool) -> std::io::Result<bool> {
    let dir = env.state.join("locks");
    std::fs::create_dir_all(&dir)?;
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("held.lock"))?;
    lock.lock()?;
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
    update(env, |m| {
        m.insert(id.0.clone(), h);
        true
    })
    .map(|_| ())
}

/// Let `id` come back: whether it was held. A file that is not there is not
/// touched (the hook calls this at every member's start).
pub fn release(env: &Env, id: &Id) -> std::io::Result<bool> {
    if !path(env).exists() {
        return Ok(false);
    }
    update(env, |m| m.remove(&id.0).is_some())
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
        std::fs::write(path(&env), "not json").unwrap();
        assert!(!is_held(&env, &Id::commodore("MAIN", "ARGOS")), "unreadable: the leads come back");
    }
}
