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
/// The multiplexer settings a member's own command (or a hook's delivery)
/// may use (plan-v4 section 13; acc-D F6). A member runs in whatever env its
/// job inherited (a daemon-restarted worker has the daemon's, the user's live
/// `HERDR_*` included), so that env alone never names a Herdr server: an
/// explicit `NIMITZ_MUX` or `NIMITZ_HERDR_SESSION` is taken as given; else the
/// server the user's own nimitz command recorded here, by its session name;
/// else the recorded default server's socket (`NIMITZ_HERDR_SOCKET`, set as
/// `HERDR_SOCKET_PATH` on every call); else no multiplexer.
pub fn member_mux_vars(env: &Env, vars: &HashMap<String, String>) -> HashMap<String, String> {
    let set = |k: &str| vars.get(k).is_some_and(|v| !v.is_empty());
    let explicit = vars.get("NIMITZ_MUX").is_some_and(|m| !m.is_empty() && m != "auto") || set("NIMITZ_HERDR_SESSION");
    let mut v = vars.clone();
    if explicit {
        return v;
    }
    // the inherited socket never counts; the recorded one names the server
    v.remove("HERDR_SOCKET_PATH");
    match load(env).and_then(|a| a.herdr) {
        Some(HerdrIdentity { session: Some(s), socket }) => {
            v.insert("NIMITZ_MUX".into(), "herdr".into());
            v.insert("NIMITZ_HERDR_SESSION".into(), s);
            // the child herdr inherits the process env (the inherited
            // HERDR_SOCKET_PATH included): name the recorded socket too, so
            // the two never disagree
            if let Some(sock) = socket {
                v.insert("NIMITZ_HERDR_SOCKET".into(), sock);
            }
        }
        // the default server (no session name): its recorded socket
        Some(HerdrIdentity { session: None, socket: Some(sock) }) => {
            v.insert("NIMITZ_MUX".into(), "herdr".into());
            v.insert("NIMITZ_HERDR_SOCKET".into(), sock);
        }
        _ => {
            v.insert("NIMITZ_MUX".into(), "none".into());
        }
    }
    v
}

pub fn load(env: &Env) -> Option<ActiveSet> {
    let v = crate::evidence::first_json(&std::fs::read(path(env)).ok()?)?;
    serde_json::from_value(v).ok()
}

/// The Herdr identity of this process's env, when it runs inside Herdr.
/// None is "unknown": the set keeps what was recorded before.
pub fn herdr_identity(vars: &HashMap<String, String>) -> Option<HerdrIdentity> {
    let get = |k: &str| vars.get(k).filter(|v| !v.is_empty()).cloned();
    // a named session (`NIMITZ_HERDR_SESSION`, passed as `--session`) is the
    // identity whatever the pane env says; else the pane env, inside Herdr
    if let Some(s) = get("NIMITZ_HERDR_SESSION") {
        return Some(HerdrIdentity { session: Some(s), socket: None });
    }
    // a member's env is its launch env, or the daemon's after a respawn:
    // the user's live `HERDR_*`, whatever world its `NIMITZ_HOME` names
    // (acc-D F6). Only the user's own command records a pane env.
    if !matches!(crate::sessions::caller_from_env(vars), Caller::User) {
        return None;
    }
    if vars.get("HERDR_ENV").map(String::as_str) != Some("1") {
        return None;
    }
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
/// before (`prev`) stay for a member still on the same job. The recorded
/// Herdr identity stays whenever this call has none (`herdr` None: outside
/// Herdr, or a member's command): a look from outside Herdr forgets no tab.
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
    let herdr = herdr.or_else(|| prev.and_then(|p| p.herdr.clone()));
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

    fn snap_member(id: &Id, status: Status, reasons: &[&str], job: &str, sid: &str, live: bool) -> Member {
        let s = SessionInfo {
            session_id: sid.into(),
            short_id: Some(job.into()),
            pid: live.then_some(1),
            kind: "background".into(),
            state: None,
            status: live.then(|| "idle".into()),
            waiting_for: None,
            started_at: None,
            status_since: None,
            name: id.name(),
            cwd: "/f".into(),
            labels: BTreeMap::new(),
            birth_id: None,
            entrypoint: None,
            pinned: false,
            copy: None,
            terminal: None,
        };
        Member { id: id.clone(), role: id.role(), expected_name: id.name(), cwd: Some("/f".into()), status, reasons: reasons.iter().map(|r| r.to_string()).collect(), session: Some(s), extra_rows: vec![], crew: vec![], tasks: vec![], issues: vec![], pane: None }
    }

    #[test]
    fn a_member_taken_down_by_the_machine_stays_in_the_set_a_retired_one_goes() {
        let adm = Id::admiral("F");
        let (s1, s2) = (Id::ship("F", "SQ", "S1"), Id::ship("F", "SQ", "S2"));
        let place = Place { workspace_id: "w3".into(), workspace: "F.SQ".into(), tab_id: "w3:t2".into(), tab: "F.SQ.S1".into(), pane_id: "w3:p2".into() };
        let mut prev = ActiveSet::default();
        prev.members.insert(s1.0.clone(), ActiveMember { attached: true, place: Some(place.clone()), status: "busy".into(), ..member("j1", "x1") });
        prev.members.insert(s2.0.clone(), member("j2", "x2"));
        prev.members.insert(adm.0.clone(), member("j0", "x0"));
        // after the loss: S1 parked with no retire line (the machine went down), S2 retired by the daemon, the admiral live
        let fleet = Fleet {
            id: adm.clone(),
            dir: "/f".into(),
            machine: Some("m1".into()),
            admiral: snap_member(&adm, Status::Idle, &["session file: status idle"], "j0", "x0", true),
            squadrons: vec![Squadron {
                id: Id::commodore("F", "SQ"),
                commodore: snap_member(&Id::commodore("F", "SQ"), Status::NotStarted, &[], "j9", "x9", false),
                ships: vec![
                    snap_member(&s1, Status::Parked, &["job row j1 with no live pid (state working)", "parked: no SessionEnd (retired, crashed or stopped with the machine); a start wakes it"], "j1", "x1", false),
                    snap_member(&s2, Status::Parked, &["job row j2 with no live pid (state done)", "parked: the daemon retired it (bg retire j2: settled, idle 61m); a start wakes it"], "j2", "x2", false),
                ],
                comms: Comms::default(),
                issues: vec![],
            }],
            issues: vec![],
        };
        let snap = Snapshot { taken_at: 0.0, machine: Some("m1".into()), fleets: vec![fleet], stowaways: vec![], issues: vec![], cc_version: None, compat_ok: true };
        let seen = Seen { shown: vec![], empty_panes: vec!["w3:p2".into()] };
        let set = from_snapshot(&snap, Some(&seen), None, Some(&prev));
        assert_eq!(set.members.keys().cloned().collect::<Vec<_>>(), vec![adm.0.clone(), s1.0.clone()], "S2 (retired) is out; S1 (taken down) stays");
        assert_eq!(set.members[&s1.0].place.as_ref(), Some(&place), "with its recorded place and status");
        assert_eq!(set.members[&s1.0].status, "busy");
        assert!(!set.members[&adm.0].attached, "the live admiral shows in no pane now");
        // the same with no previous set: only the live member
        let fresh = from_snapshot(&snap, Some(&seen), None, None);
        assert_eq!(fresh.members.keys().cloned().collect::<Vec<_>>(), vec![adm.0.clone()]);
    }

    #[test]
    fn herdr_identity_comes_from_the_pane_env_only_inside_herdr() {
        let v = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        assert_eq!(herdr_identity(&v(&[("HERDR_SESSION", "NIMITZ")])), None);
        let h = herdr_identity(&v(&[("HERDR_ENV", "1"), ("HERDR_SESSION", "NIMITZ"), ("HERDR_SOCKET_PATH", "/s/herdr.sock")])).unwrap();
        assert_eq!((h.session.as_deref(), h.socket.as_deref()), (Some("NIMITZ"), Some("/s/herdr.sock")));
        assert_eq!(herdr_identity(&v(&[("HERDR_ENV", "1")])), Some(HerdrIdentity::default()));
        let h = herdr_identity(&v(&[("HERDR_ENV", "1"), ("HERDR_SESSION", "NIMITZ"), ("NIMITZ_HERDR_SESSION", "nimitz-e2e")])).unwrap();
        assert_eq!((h.session.as_deref(), h.socket), (Some("nimitz-e2e"), None), "the named session wins over the pane env");
    }

    /// Acc-D F6: a daemon-respawned member inherits the daemon's env, the
    /// user's live `HERDR_*`, in whatever world its labels name. A member's
    /// command never records that as the set's Herdr identity: unknown,
    /// and the set keeps what the user's own command recorded.
    #[test]
    fn a_members_inherited_herdr_env_is_no_identity_and_the_recorded_one_stays() {
        let v = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        let pane = [("HERDR_ENV", "1"), ("HERDR_SESSION", "NIMITZ"), ("HERDR_SOCKET_PATH", "/s/herdr.sock")];
        let member = [("NIMITZ_ID", "F/SQ/S1"), ("NIMITZ_ROLE", "ship")];
        assert_eq!(herdr_identity(&v(&[pane.as_slice(), member.as_slice()].concat())), None);
        assert_eq!(herdr_identity(&v(&[pane.as_slice(), &[("NIMITZ_ID", "F"), ("NIMITZ_ROLE", "admiral")]].concat())), None, "a lead too");
        assert_eq!(herdr_identity(&v(&[pane.as_slice(), &[("NIMITZ_ID", "F/SQ/S1")]].concat())), None, "partial labels: not the user either");
        let named = herdr_identity(&v(&[pane.as_slice(), member.as_slice(), &[("NIMITZ_HERDR_SESSION", "nimitz-e2e")]].concat())).unwrap();
        assert_eq!(named.session.as_deref(), Some("nimitz-e2e"), "a named session is explicit, whoever runs");
        // the set: a member's write (herdr None), with a mux seen or not, keeps the recorded identity
        let recorded = HerdrIdentity { session: Some("NIMITZ".into()), socket: Some("/s/herdr.sock".into()) };
        let prev = ActiveSet { herdr: Some(recorded.clone()), ..Default::default() };
        let snap = Snapshot { taken_at: 0.0, machine: Some("m1".into()), fleets: vec![], stowaways: vec![], issues: vec![], cc_version: None, compat_ok: true };
        let seen = Seen::default();
        assert_eq!(from_snapshot(&snap, Some(&seen), None, Some(&prev)).herdr.as_ref(), Some(&recorded));
        assert_eq!(from_snapshot(&snap, None, None, Some(&prev)).herdr.as_ref(), Some(&recorded));
        assert_eq!(from_snapshot(&snap, Some(&seen), None, None).herdr, None, "nothing recorded, nothing known");
        let other = HerdrIdentity { session: Some("nimitz-e2e".into()), socket: None };
        assert_eq!(from_snapshot(&snap, Some(&seen), Some(other.clone()), Some(&prev)).herdr.as_ref(), Some(&other), "a known identity replaces the recorded one");
    }
}
