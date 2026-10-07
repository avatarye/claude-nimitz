//! The last-known active set: `NIMITZ_STATE/active.json` (restore-scenarios.md
//! section 3). Which members were live at the last write, their status word,
//! whether a pane showed them and where (workspace, tab, pane: ids for the
//! fast path while the Herdr server lives, labels for after a restart, when
//! ids may change). A restore after a reboot brings that set back.
//!
//! Written on `status`, `layout`, `open`, `restore` (the whole set from the
//! snapshot and the mux) and by the hook's SessionStart (a member is live
//! now) and SessionEnd (it is not). One writer under `registry.lock`; a temp
//! file and a rename; no write when nothing but the clock changed.
use crate::env::Env;
use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

pub const VERSION: u32 = 1;

/// Where a pane showed the member: ids (a fast path while the server lives)
/// and labels (what survives a Herdr restart).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub workspace_id: String,
    pub workspace: String,
    pub tab_id: String,
    pub tab: String,
    pub pane_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActiveMember {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    pub session: String,
    /// the last status word: busy | idle | needs-you | unknown
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_for: Option<String>,
    pub attached: bool,
    #[serde(default)]
    pub place: Option<Place>,
}

/// The Herdr server the set was recorded against (R-29): what the server
/// puts in a pane's env (`HERDR_SESSION`, `HERDR_SOCKET_PATH`), since the
/// only Herdr commands nimitz runs are the shape readers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HerdrIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActiveSet {
    pub version: u32,
    /// when it was written (epoch seconds)
    pub at: f64,
    #[serde(default)]
    pub machine: Option<String>,
    #[serde(default)]
    pub herdr: Option<HerdrIdentity>,
    /// the members live at the last write, by id
    #[serde(default)]
    pub members: BTreeMap<String, ActiveMember>,
}

impl ActiveSet {
    /// The same set, the clock aside.
    fn same_as(&self, other: &ActiveSet) -> bool {
        self.version == other.version && self.machine == other.machine && self.herdr == other.herdr && self.members == other.members
    }
}

pub fn path(env: &Env) -> PathBuf {
    env.state.join("active.json")
}

/// The recorded set, if there is one that parses.
pub fn load(env: &Env) -> Option<ActiveSet> {
    let v = crate::evidence::first_json(&std::fs::read(path(env)).ok()?)?;
    serde_json::from_value(v).ok()
}

/// The Herdr identity of this process's env, when it runs inside Herdr.
pub fn herdr_identity(vars: &HashMap<String, String>) -> Option<HerdrIdentity> {
    if vars.get("HERDR_ENV").map(String::as_str) != Some("1") {
        return None;
    }
    let get = |k: &str| vars.get(k).filter(|v| !v.is_empty()).cloned();
    Some(HerdrIdentity { session: get("HERDR_SESSION"), socket: get("HERDR_SOCKET_PATH") })
}

/// A pane that shows a session, with where it is (from the mux's shape).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    pub session_id: String,
    pub place: Place,
}

/// What the mux shows now: the panes with a session, and the ids of the
/// panes that show none (a recorded place whose pane is still there and
/// shows nothing else is kept: Herdr's agent detection lags an attach).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seen {
    pub shown: Vec<Shown>,
    pub empty_panes: Vec<String>,
}

fn status_word(s: Status) -> &'static str {
    match s {
        Status::Busy => "busy",
        Status::Idle => "idle",
        Status::NeedsYou => "needs-you",
        _ => "unknown",
    }
}

/// The set as the snapshot and the mux see it now: every live member, with
/// the pane that shows its conversation (any id of its job) when one does.
/// With no mux (`shown` None: a `status` over ssh), the places recorded
/// before (`prev`) stay for a member still on the same job, and so does the
/// recorded Herdr identity: a look from outside Herdr forgets no tab.
pub fn from_snapshot(snap: &Snapshot, seen: Option<&Seen>, herdr: Option<HerdrIdentity>, prev: Option<&ActiveSet>) -> ActiveSet {
    let mut members = BTreeMap::new();
    let all = snap.fleets.iter().flat_map(|f| std::iter::once(&f.admiral).chain(f.squadrons.iter().flat_map(|q| std::iter::once(&q.commodore).chain(q.ships.iter()))));
    for m in all {
        let Some(s) = m.session.as_ref() else { continue };
        if s.pid.is_none() {
            // a member the machine or a crash took down (parked, no retire line
            // and no SessionEnd) was live at the loss: it stays in the set, as
            // recorded, until a restore or a stop settles it (R-08). One the
            // daemon retired, or that was stopped, goes.
            let taken_down = m.status == Status::Parked && !m.reasons.iter().any(|r| r.contains("the daemon retired it"));
            if let Some(p) = prev.and_then(|p| p.members.get(&m.id.0)).filter(|p| taken_down && s.short_id.is_some() && p.job == s.short_id) {
                members.insert(m.id.0.clone(), p.clone());
            }
            continue;
        }
        let ids: Vec<&str> = std::iter::once(s.session_id.as_str()).chain(s.birth_id.as_deref()).collect();
        let before = prev.and_then(|p| p.members.get(&m.id.0)).filter(|p| s.short_id.is_some() && p.job == s.short_id);
        let (attached, place) = match seen {
            Some(seen) => {
                let place = seen
                    .shown
                    .iter()
                    .find(|p| ids.contains(&p.session_id.as_str()))
                    .map(|p| p.place.clone())
                    .or_else(|| before.and_then(|p| p.place.clone()).filter(|pl| seen.empty_panes.contains(&pl.pane_id)));
                (place.is_some(), place)
            }
            None => match before {
                Some(p) => (p.attached, p.place.clone()),
                None => (false, None),
            },
        };
        members.insert(m.id.0.clone(), ActiveMember { job: s.short_id.clone(), session: s.session_id.clone(), status: status_word(m.status).into(), waiting_for: s.waiting_for.clone(), attached, place });
    }
    let herdr = if seen.is_some() { herdr } else { herdr.or_else(|| prev.and_then(|p| p.herdr.clone())) };
    ActiveSet { version: VERSION, at: crate::evidence::now(), machine: snap.machine.clone(), herdr, members }
}

/// Write `set` under the registry lock (temp file, rename). Ok(false) when
/// the file already says the same (only the clock would move).
pub fn write(env: &Env, mut set: ActiveSet) -> std::io::Result<bool> {
    crate::registry::with_lock(env, None, || write_locked(env, &mut set))?
}

fn write_locked(env: &Env, set: &mut ActiveSet) -> std::io::Result<bool> {
    if let Some(old) = load(env) {
        if old.same_as(set) {
            return Ok(false);
        }
    }
    set.version = VERSION;
    set.at = crate::evidence::now();
    let text = serde_json::to_string_pretty(set).map_err(std::io::Error::other)?;
    crate::config::atomic_write(&path(env), &text)?;
    Ok(true)
}

/// How long a hook waits for the lock: a hook never hangs a session; a lost
/// note is made good by the next `status`.
const HOOK_BUDGET: std::time::Duration = std::time::Duration::from_secs(1);

/// The hook's SessionStart of a member: it is live now. A member already in
/// the set keeps its place and `attached` when the job is the same (an
/// attach client follows the job across `/clear`); otherwise it starts
/// unattached, status `idle`, until the next full write.
pub fn note_start(env: &Env, id: &Id, job: Option<&str>, session_id: &str) -> std::io::Result<bool> {
    crate::registry::with_lock(env, Some(HOOK_BUDGET), || {
        let mut set = load(env).unwrap_or_default();
        let prev = set.members.remove(&id.0);
        let same_job = prev.as_ref().map(|p| job.is_some() && p.job.as_deref() == job).unwrap_or(false);
        let keep = prev.filter(|_| same_job);
        set.members.insert(
            id.0.clone(),
            ActiveMember {
                job: job.map(String::from),
                session: session_id.to_string(),
                status: "idle".into(),
                waiting_for: None,
                attached: keep.as_ref().map(|p| p.attached).unwrap_or(false),
                place: keep.and_then(|p| p.place),
            },
        );
        if set.machine.is_none() {
            set.machine = crate::registry::machine_label(env);
        }
        write_locked(env, &mut set)
    })?
}

/// The hook's SessionEnd of a member: it is not live any more (a `claude
/// stop`, `/stop`, an exit; the daemon's retire fires none, and `status`
/// catches that). Only the entry for that session (or job) goes.
pub fn note_end(env: &Env, id: &Id, session_id: &str, job: Option<&str>) -> std::io::Result<bool> {
    crate::registry::with_lock(env, Some(HOOK_BUDGET), || {
        let Some(mut set) = load(env) else { return Ok(false) };
        let gone = match set.members.get(&id.0) {
            Some(m) => m.session == session_id || (job.is_some() && m.job.as_deref() == job),
            None => false,
        };
        if !gone {
            return Ok(false);
        }
        set.members.remove(&id.0);
        write_locked(env, &mut set)
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::test_env;

    fn member(job: &str, sid: &str) -> ActiveMember {
        ActiveMember { job: Some(job.into()), session: sid.into(), status: "idle".into(), waiting_for: None, attached: false, place: None }
    }

    #[test]
    fn write_is_atomic_and_skips_an_unchanged_set() {
        let (_td, env) = test_env();
        assert!(load(&env).is_none());
        let mut set = ActiveSet { version: VERSION, at: 1.0, machine: Some("m1".into()), herdr: None, members: BTreeMap::new() };
        set.members.insert("F".into(), member("j1", "s1"));
        assert!(write(&env, set.clone()).unwrap());
        let back = load(&env).expect("written");
        assert_eq!(back.members, set.members);
        assert!(back.at > 1.0, "the clock is nimitz's, not the caller's");
        assert!(!path(&env).with_extension("tmp").exists());
        // the same set again: no write
        let before = std::fs::read(path(&env)).unwrap();
        assert!(!write(&env, set.clone()).unwrap());
        assert_eq!(std::fs::read(path(&env)).unwrap(), before);
        // a change: written
        set.members.get_mut("F").unwrap().attached = true;
        assert!(write(&env, set).unwrap());
        assert!(load(&env).unwrap().members["F"].attached);
    }

    #[test]
    fn the_hook_notes_a_start_and_an_end_and_keeps_the_place_across_a_wake() {
        let (_td, env) = test_env();
        let id = Id::commodore("F", "SQ");
        let place = Place { workspace_id: "w3".into(), workspace: "F.SQ".into(), tab_id: "w3:t1".into(), tab: "F.SQ.COMMODORE".into(), pane_id: "w3:p1".into() };
        let mut set = ActiveSet::default();
        set.members.insert(id.0.clone(), ActiveMember { attached: true, place: Some(place.clone()), ..member("j1", "s1") });
        assert!(write(&env, set).unwrap());
        // a stop: gone; a SessionEnd of another session of the id: untouched
        assert!(!note_end(&env, &id, "other", None).unwrap());
        assert!(load(&env).unwrap().members.contains_key(&id.0));
        assert!(note_end(&env, &id, "s1", None).unwrap());
        assert!(!load(&env).unwrap().members.contains_key(&id.0));
        assert!(!note_end(&env, &id, "s1", None).unwrap(), "already gone");
        // a wake of the same job: live again; the place is forgotten (nobody
        // records a tab for it until the next full write)
        assert!(note_start(&env, &id, Some("j1"), "s1").unwrap());
        let m = load(&env).unwrap().members[&id.0].clone();
        assert_eq!((m.job.as_deref(), m.status.as_str(), m.attached, m.place.is_none()), (Some("j1"), "idle", false, true));
        // a /clear under the same job keeps the attached state and the place
        let mut set = load(&env).unwrap();
        set.members.get_mut(&id.0).unwrap().attached = true;
        set.members.get_mut(&id.0).unwrap().place = Some(place.clone());
        write(&env, set).unwrap();
        assert!(note_start(&env, &id, Some("j1"), "s2").unwrap());
        let m = load(&env).unwrap().members[&id.0].clone();
        assert_eq!((m.session.as_str(), m.attached, m.place.as_ref()), ("s2", true, Some(&place)));
        // another job for the id: a fresh entry
        assert!(note_start(&env, &id, Some("j2"), "s3").unwrap());
        let m = load(&env).unwrap().members[&id.0].clone();
        assert_eq!((m.job.as_deref(), m.attached, m.place.is_none()), (Some("j2"), false, true));
        // the same again: nothing to write
        assert!(!note_start(&env, &id, Some("j2"), "s3").unwrap());
    }

    #[test]
    fn herdr_identity_comes_from_the_pane_env_only_inside_herdr() {
        let v = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        assert_eq!(herdr_identity(&v(&[("HERDR_SESSION", "NIMITZ")])), None);
        let h = herdr_identity(&v(&[("HERDR_ENV", "1"), ("HERDR_SESSION", "NIMITZ"), ("HERDR_SOCKET_PATH", "/s/herdr.sock")])).unwrap();
        assert_eq!((h.session.as_deref(), h.socket.as_deref()), (Some("NIMITZ"), Some("/s/herdr.sock")));
        assert_eq!(herdr_identity(&v(&[("HERDR_ENV", "1")])), Some(HerdrIdentity::default()));
    }
}
