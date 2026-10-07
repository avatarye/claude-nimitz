//! Registry of identities and lineage (contracts.md section 5). Owner: core-config.
use crate::env::Env;
use crate::types::*;
use std::collections::BTreeMap;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

/// Current contents. A missing file is a fresh registry for this machine. A
/// file that does not parse is never replaced in silence: it is kept as
/// `registry.json.corrupt-<ts>`, the registry is rebuilt from `registry.json.bak`
/// (the last good copy) and the SessionStart lines in events.jsonl, and `health`
/// reports `registry.corrupt` until the kept file is removed. Never panics; a
/// read-only state folder leaves the bad file where it is (also reported).
pub fn load(env: &Env) -> Registry {
    load_with(env, false).0
}

/// `locked`: the caller holds `registry.lock` already (flock does not nest).
/// The flag says the file on disk is still a 0.3.x (v1) registry. A file
/// that cannot be read (EACCES, EIO: anything but "not there") reads as a
/// fresh registry for the read-only callers; `update` refuses to write over
/// it (`read_registry`).
fn load_with(env: &Env, locked: bool) -> (Registry, bool) {
    load_checked(env, locked).unwrap_or_else(|_| (fresh(env), false))
}

/// `load_with`, with the read error when the file is there but unreadable:
/// a fresh registry must never be written over it.
fn load_checked(env: &Env, locked: bool) -> std::io::Result<(Registry, bool)> {
    let mut r = match std::fs::read_to_string(env.registry_path()) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => fresh(env),
        Err(e) => return Err(std::io::Error::new(e.kind(), format!("registry {}: {e}", env.registry_path().display()))),
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(r) => r,
            Err(_) if locked => recover_locked(env),
            Err(_) => match lock(env) {
                Ok(_lock) => recover_locked(env),
                Err(_) => rebuild(env).0, // read-only: rebuild in memory only
            },
        },
    };
    let v1 = r.version < REGISTRY_VERSION;
    // A 0.3.x registry: its tender identities become history (plan-v4 section
    // 8) and `current` becomes `session` with the job filled from the job rows
    // (section 2). A read never writes; the next `update` saves the v2 shape
    // and keeps the old file as `registry.json.v1`.
    crate::migrate::retire_tenders(&mut r);
    crate::migrate::registry_v2(env, &mut r);
    drop_other_homes(env, &mut r);
    Ok((r, v1))
}

/// Strip every binding to a job of another `NIMITZ_HOME` (the job's saved
/// labels, `respawnFlags`, name it). Such a session never appears (plan-v4
/// section 6), and its binding is not this home's either: its hook ran with
/// that home in its own env (the label is the env), so when two homes share
/// one `NIMITZ_STATE` it bound the identity here. The record keeps this
/// home's own history: the foreign lineage entries go, `job` and `session`
/// fall back to the newest entry left, and an identity with nothing of its
/// own left goes. A read strips in memory; the next write saves the result.
fn drop_other_homes(env: &Env, r: &mut Registry) {
    let jobs = env.claude_home.join("jobs");
    let mut named: Vec<String> = r.identities.values().flat_map(|rec| rec.job.iter().chain(rec.lineage.iter().filter_map(|l| l.job.as_ref())).cloned()).collect();
    named.sort();
    named.dedup();
    let foreign: std::collections::BTreeSet<String> = named
        .into_iter()
        .filter(|job| {
            std::fs::read(jobs.join(job).join("state.json"))
                .ok()
                .and_then(|b| crate::evidence::first_json(&b))
                .map(|d| other_home(env, &crate::evidence::job_labels(&d)))
                .unwrap_or(false)
        })
        .collect();
    if foreign.is_empty() {
        return;
    }
    r.identities.retain(|_, rec| {
        rec.lineage.retain(|l| !l.job.as_ref().is_some_and(|j| foreign.contains(j)));
        if rec.job.as_ref().is_some_and(|j| foreign.contains(j)) {
            rec.job = rec.lineage.last().and_then(|l| l.job.clone());
        }
        if !rec.session.as_ref().is_some_and(|s| rec.lineage.iter().any(|l| &l.session_id == s)) {
            rec.session = rec.lineage.last().map(|l| l.session_id.clone());
        }
        !rec.lineage.is_empty()
    });
}

fn fresh(env: &Env) -> Registry {
    Registry { version: REGISTRY_VERSION, machine: env.hostname.clone(), ..Default::default() }
}

fn parse_file(p: &Path) -> Option<Registry> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn bak_path(env: &Env) -> PathBuf {
    env.state.join("registry.json.bak")
}

/// Where the last v1 file is kept once the v2 shape is written.
pub fn v1_path(env: &Env) -> PathBuf {
    env.state.join("registry.json.v1")
}

/// What `recover` could rebuild from: the last good copy, then the events
/// (the same binding rules as the hook, `apply_start`).
fn rebuild(env: &Env) -> (Registry, String) {
    let (mut r, mut from) = match parse_file(&bak_path(env)) {
        Some(r) => (r, vec!["registry.json.bak".to_string()]),
        None => (fresh(env), vec![]),
    };
    let mut added = 0;
    let mut events = crate::evidence::events(env);
    events.sort_by(|a, b| a.ts.partial_cmp(&b.ts).unwrap_or(std::cmp::Ordering::Equal));
    for e in events.iter().filter(|e| e.event == "SessionStart") {
        // a 0.3.x tender's events rebuild its history in `retired`, never a member
        if let (Some(sid), Some(nid)) = (e.session_id.as_deref(), e.labels.get("NIMITZ_ID").filter(|n| crate::migrate::is_tender_id(n))) {
            let id = Id(nid.clone());
            bind_into(&mut r.retired, &id, id.role(), &id.name(), "", e.job.as_deref(), sid, &via_of(e.source.as_deref(), true), e.ts);
            added += 1;
        } else if matches!(apply_event(env, &mut r, e), Binding::Bound(_)) {
            added += 1;
        }
    }
    if added > 0 {
        from.push(format!("{added} SessionStart event(s)"));
    }
    let what = if from.is_empty() { "nothing to rebuild from (no registry.json.bak, no labelled events)".to_string() } else { from.join(" + ") };
    let what = format!("{} identities rebuilt from {what}", r.identities.len());
    (r, what)
}

/// The registry did not parse: keep the bad file aside, rebuild, save.
fn recover_locked(env: &Env) -> Registry {
    // Another process may have recovered while we waited for the lock.
    if let Some(r) = parse_file(&env.registry_path()) {
        return r;
    }
    let (mut r, what) = rebuild(env);
    let ts = crate::evidence::now() as u64;
    let kept = env.state.join(format!("registry.json.corrupt-{ts}"));
    if std::fs::rename(env.registry_path(), &kept).is_err() {
        return r;
    }
    let _ = std::fs::write(note_path(&kept), &what);
    let _ = write_unlocked(env, &mut r);
    r
}

fn note_path(kept: &Path) -> PathBuf {
    PathBuf::from(format!("{}.txt", kept.display()))
}

/// Issues about the registry itself (`nimitz check`): a kept corrupt copy (the
/// lineages may be incomplete), a registry that does not parse and could not be
/// kept aside, a state folder nimitz cannot write.
pub fn health(env: &Env) -> Vec<Issue> {
    let mut out = vec![];
    let reg = env.registry_path();
    let mut kept: Vec<PathBuf> = std::fs::read_dir(&env.state)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    n.starts_with("registry.json.corrupt-") && !n.ends_with(".txt")
                })
                .collect()
        })
        .unwrap_or_default();
    kept.sort();
    for k in &kept {
        let what = std::fs::read_to_string(note_path(k)).unwrap_or_else(|_| "rebuilt from what was left".into());
        out.push(Issue {
            code: "registry.corrupt".into(),
            severity: Severity::Error,
            message: format!(
                "{} did not parse; the bad file is kept as {}; {what}. Lineages from before may be incomplete, so a start could lose a conversation",
                reg.display(),
                k.display()
            ),
            fix: Some(format!("check the members with `nimitz status`, then rm {} {}", k.display(), note_path(k).display())),
            id: None,
        });
    }
    if std::fs::read_to_string(&reg).is_ok_and(|raw| serde_json::from_str::<Registry>(&raw).is_err()) {
        out.push(Issue {
            code: "registry.corrupt".into(),
            severity: Severity::Error,
            message: format!("{} does not parse and could not be kept aside (is {} writable?)", reg.display(), env.state.display()),
            fix: Some(format!("make {} writable; the next nimitz command keeps the bad file and rebuilds", env.state.display())),
            id: None,
        });
    } else if let Some(e) = unwritable(env) {
        out.push(Issue {
            code: "registry.readonly".into(),
            severity: Severity::Error,
            message: format!("nimitz cannot write its registry in {}: {e}; launched sessions would not be recorded", env.state.display()),
            fix: Some(format!("make {} writable (or point NIMITZ_STATE elsewhere)", env.state.display())),
            id: None,
        });
    }
    out
}

/// Why the registry cannot be written, when it cannot (the folder exists but is read-only).
fn unwritable(env: &Env) -> Option<String> {
    let probe = |p: &Path| {
        let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).ok()?;
        (unsafe { libc::access(c.as_ptr(), libc::W_OK) } != 0).then(|| format!("{} is read-only", p.display()))
    };
    if !env.state.is_dir() {
        return None; // created on first write
    }
    probe(&env.state).or_else(|| env.registry_path().is_file().then(|| probe(&env.registry_path())).flatten())
}

/// This machine's label as `nimitz init --label` saved it in NIMITZ_STATE (per
/// machine, so it outlives every fleet). None before `init`: a registry that
/// is not on disk has only the hostname as a placeholder.
pub fn machine_label(env: &Env) -> Option<String> {
    if !env.registry_path().is_file() {
        return None;
    }
    Some(load(env).machine).filter(|m| !m.is_empty())
}

/// Atomic save under an exclusive lock (the file gets a fresh `updated_at`;
/// `r` keeps its own).
pub fn save(env: &Env, r: &Registry) -> std::io::Result<()> {
    let _lock = lock(env)?;
    write_unlocked(env, &mut r.clone())
}

/// Lock, load, apply `f`, save. The only way to change the registry. The
/// first write over a v1 file keeps that file as `registry.json.v1`. A file
/// that is there but cannot be read is an error: nothing is written over it.
pub fn update<T>(env: &Env, f: impl FnOnce(&mut Registry) -> T) -> std::io::Result<T> {
    let lock = lock(env)?;
    update_locked(env, lock, f)
}

/// `update` that waits at most `budget` for the lock (the hook path: a hook
/// must not hang a session; a lost write is replayed by `status`).
pub fn try_update<T>(env: &Env, budget: std::time::Duration, f: impl FnOnce(&mut Registry) -> T) -> std::io::Result<T> {
    let lock = lock_within(env, budget)?;
    update_locked(env, lock, f)
}

fn update_locked<T>(env: &Env, _lock: File, f: impl FnOnce(&mut Registry) -> T) -> std::io::Result<T> {
    let (mut r, v1) = load_checked(env, true)?;
    if v1 && !v1_path(env).exists() {
        let _ = std::fs::copy(env.registry_path(), v1_path(env));
    }
    let out = f(&mut r);
    write_unlocked(env, &mut r)?;
    Ok(out)
}

/// Record a new current session for an identity (appends to lineage, keeps the
/// job the hook bound). A second bind of the session the identity runs now
/// (the hook bound it first) only refines the `via`.
#[allow(clippy::too_many_arguments)] // one flat row of session facts
pub fn bind(r: &mut Registry, id: &Id, role: Role, name: &str, cwd: &str, session_id: &str, via: &str, now: f64) {
    bind_into(&mut r.identities, id, role, name, cwd, None, session_id, via, now)
}

/// `bind` with the job (short id) the session runs in: what the sandbox and a
/// start that knows the job record.
#[allow(clippy::too_many_arguments)]
pub fn bind_job(r: &mut Registry, id: &Id, role: Role, name: &str, cwd: &str, job: Option<&str>, session_id: &str, via: &str, now: f64) {
    bind_into(&mut r.identities, id, role, name, cwd, job, session_id, via, now)
}

#[allow(clippy::too_many_arguments)]
fn bind_into(map: &mut BTreeMap<String, IdentityRecord>, id: &Id, role: Role, name: &str, cwd: &str, job: Option<&str>, session_id: &str, via: &str, now: f64) {
    let rec = map.entry(id.0.clone()).or_insert_with(|| IdentityRecord {
        role,
        name: name.to_string(),
        cwd: cwd.to_string(),
        job: None,
        session: None,
        lineage: vec![],
    });
    rec.role = role;
    rec.name = name.to_string();
    if !cwd.is_empty() {
        rec.cwd = cwd.to_string();
    }
    if let Some(j) = job {
        rec.job = Some(j.to_string());
    }
    rec.session = Some(session_id.to_string());
    match rec.lineage.last_mut() {
        // the conversation it runs already: the same entry, unless another job
        // took it over (a resume after `claude rm`). How the conversation was
        // born (launch, clear, branch, ...) is never overwritten by a wake
        // (resume, relaunch): the clear entry survives a stop and a start.
        Some(last) if last.session_id == session_id && (last.job.is_none() || last.job == rec.job) => {
            if !(is_wake(via) && !is_wake(&last.via)) {
                last.via = via.to_string();
            }
            if last.job.is_none() {
                last.job = rec.job.clone();
            }
        }
        _ => rec.lineage.push(LineageEntry { session_id: session_id.to_string(), job: rec.job.clone(), via: via.to_string(), at: now }),
    }
}

/// A `via` that wakes a conversation it already had, as against one that
/// started it (launch, clear, branch, fork, adopt, migrate, compact).
fn is_wake(via: &str) -> bool {
    matches!(via, "resume" | "relaunch")
}

// ------------------------------------------------------------------ the hook's binding table

/// What a SessionStart did to the registry (plan-v4 section 3).
#[derive(Clone, Debug, PartialEq)]
pub enum Binding {
    /// job and session bound (or rebound); the lineage `via`
    Bound(String),
    /// the identity already runs as another job that still has its row: not
    /// rebound (two jobs claim one identity)
    Dup(String),
    /// no job (an interactive or SDK process runs the conversation): not bound
    Held,
    /// the labels in another folder than the identity's: an impostor, not
    /// bound (BUG-30, BUG-55; the snapshot reports `labels.mismatch`)
    Elsewhere,
    /// the conversation the identity runs already (a compact, a replayed line)
    Same,
    /// not this home's identity, or no valid labels: nothing to do
    None,
}

impl Binding {
    pub fn as_str(&self) -> &'static str {
        match self {
            Binding::Bound(_) => "bound",
            Binding::Dup(_) => "dup.job",
            Binding::Held => "held",
            Binding::Elsewhere => "elsewhere",
            Binding::Same => "same",
            Binding::None => "none",
        }
    }
}

/// Do these labels name a valid identity of THIS home? `NIMITZ_HOME` must be
/// this env's home (as written or as a real path) when present; the role must
/// agree with the id's shape; a 0.3.x tender is no identity.
pub fn identity_of(env: &Env, labels: &BTreeMap<String, String>) -> Option<Id> {
    if let Some(h) = labels.get("NIMITZ_HOME").filter(|h| !h.is_empty()) {
        let same = Path::new(h) == env.home || crate::env::canon(Path::new(h)) == crate::env::canon(&env.home);
        if !same {
            return None;
        }
    }
    let id = Id(labels.get("NIMITZ_ID")?.clone());
    let shape_ok = !id.fleet().is_empty() && id.parts().iter().all(|p| crate::config::valid_name(p));
    (shape_ok && !crate::migrate::is_tender_id(&id.0) && labels.get("NIMITZ_ROLE").map(String::as_str) == Some(id.role().as_str())).then_some(id)
}

/// Labels of another nimitz home: ignored entirely (no event line, section 3).
pub fn other_home(env: &Env, labels: &BTreeMap<String, String>) -> bool {
    crate::evidence::is_labelled(labels) && identity_of(env, labels).is_none() && labels.get("NIMITZ_HOME").is_some_and(|h| {
        !h.is_empty() && Path::new(h) != env.home && crate::env::canon(Path::new(h)) != crate::env::canon(&env.home)
    })
}

/// The lineage `via` for a SessionStart `source`.
fn via_of(source: Option<&str>, same_job: bool) -> String {
    match source {
        Some("startup") | None => "launch",
        Some("fork") if same_job => "branch",
        Some(other) => other,
    }
    .to_string()
}

/// The binding table (plan-v4 section 3), on an identity of this home:
/// - `job` is the identity's job, or none is set yet: bind job and session,
///   append the lineage with `via` from `source` (no new entry for the
///   conversation it runs already: a compact);
/// - another job whose row still exists: `Dup`, nothing rebound;
/// - no job (interactive or SDK): `Held`, nothing bound;
/// - a `cwd` that is not the identity's `folder` (when known): `Elsewhere`.
#[allow(clippy::too_many_arguments)]
pub fn apply_start(r: &mut Registry, id: &Id, cwd: &str, folder: Option<&Path>, job: Option<&str>, session_id: &str, source: Option<&str>, now: f64, job_row_exists: impl Fn(&str) -> bool) -> Binding {
    if let Some(f) = folder.filter(|_| !cwd.is_empty()) {
        if crate::env::canon(Path::new(cwd)) != crate::env::canon(f) {
            return Binding::Elsewhere;
        }
    }
    let Some(job) = job.filter(|j| !j.is_empty()) else { return Binding::Held };
    let rec = r.identities.get(&id.0);
    let same_job = rec.and_then(|x| x.job.as_deref()) == Some(job);
    if let Some(other) = rec.and_then(|x| x.job.as_deref()).filter(|o| *o != job && job_row_exists(o)) {
        return Binding::Dup(other.to_string());
    }
    if same_job && rec.and_then(|x| x.session.as_deref()) == Some(session_id) {
        return Binding::Same;
    }
    let via = via_of(source, same_job || rec.and_then(|x| x.job.as_deref()).is_none());
    bind_into(&mut r.identities, id, id.role(), &id.name(), cwd, Some(job), session_id, &via, now);
    Binding::Bound(via)
}

/// `apply_start` for one SessionStart event line of this home (the hook wrote
/// the job and the cwd in it; a line with no session id or no valid labels
/// binds nothing). A line an older nimitz wrote (no `kind`, no `job`) binds
/// the session only. A line whose session file the hook could not read (`kind`
/// `unknown`) is judged here from the job rows: the job whose `state.json`
/// runs that session, if one does.
pub fn apply_event(env: &Env, r: &mut Registry, e: &Event) -> Binding {
    let (Some(sid), Some(id)) = (e.session_id.as_deref(), identity_of(env, &e.labels)) else { return Binding::None };
    if e.kind.is_none() && e.job.is_none() {
        let rec = r.identities.get(&id.0);
        if rec.and_then(|x| x.session.as_deref()) == Some(sid) {
            return Binding::Same;
        }
        let via = via_of(e.source.as_deref(), true);
        bind_into(&mut r.identities, &id, id.role(), &id.name(), "", None, sid, &via, e.ts);
        return Binding::Bound(via);
    }
    let jobs = env.claude_home.join("jobs");
    let job = match (e.kind.as_deref(), &e.job) {
        (Some("unknown"), None) => job_running(env, sid),
        (_, j) => j.clone(),
    };
    let cwd = e.cwd.clone().unwrap_or_default();
    let folder = identity_folder(env, r, &id);
    apply_start(r, &id, &cwd, folder.as_deref(), job.as_deref(), sid, e.source.as_deref(), e.ts, |j| jobs.join(j).join("state.json").is_file())
}

/// The job whose `state.json` runs `sid` now (`resumeSessionId`, else `sessionId`).
fn job_running(env: &Env, sid: &str) -> Option<String> {
    let rd = std::fs::read_dir(env.claude_home.join("jobs")).ok()?;
    rd.filter_map(|e| e.ok()).find_map(|e| {
        let d = crate::evidence::first_json(&std::fs::read(e.path().join("state.json")).ok()?)?;
        let now = d.get("resumeSessionId").and_then(|v| v.as_str()).or_else(|| d.get("sessionId").and_then(|v| v.as_str()))?;
        (now == sid).then(|| e.file_name().to_string_lossy().into_owned())
    })
}

/// The identity's folder: what the registry recorded, else the config (one
/// TOML read, only for an identity that never bound).
pub fn identity_folder(env: &Env, r: &Registry, id: &Id) -> Option<PathBuf> {
    match r.identities.get(&id.0).map(|x| x.cwd.clone()).filter(|c| !c.is_empty()) {
        Some(c) => Some(PathBuf::from(c)),
        None => crate::config::load_fleet(env, id.fleet()).ok().and_then(|f| crate::config::member_dir(env, &f, id)),
    }
}

/// Replay SessionStart lines newer than the registry (a hook whose registry
/// write was lost), under the lock, with the hook's rules. Writes only when a
/// line binds something; returns how many did.
pub fn replay(env: &Env, events: &[Event]) -> usize {
    let reg = load(env);
    // a line the hook judged (dup.job, held, elsewhere) bound nothing by design;
    // a line replayed before and judged then is behind the watermark
    let judged = |e: &Event| e.binding.as_deref().is_some_and(|b| !matches!(b, "bound" | "none"));
    let floor = reg.updated_at.max(watermark(env));
    let mut newer: Vec<&Event> = events.iter().filter(|e| e.event == "SessionStart" && e.ts > floor && e.session_id.is_some() && !judged(e)).collect();
    if newer.is_empty() {
        return 0;
    }
    newer.sort_by(|a, b| a.ts.partial_cmp(&b.ts).unwrap_or(std::cmp::Ordering::Equal));
    let mut probe = reg.clone();
    let bound = newer.iter().any(|e| matches!(apply_event(env, &mut probe, e), Binding::Bound(_)));
    let n = if bound { update(env, |r| newer.iter().filter(|e| matches!(apply_event(env, r, e), Binding::Bound(_))).count()).unwrap_or(0) } else { 0 };
    // every line up to here is judged: remember the newest one that is old
    // enough (a hook takes its `ts` before waiting up to 2 s for the lock, so
    // a late line never hides behind the mark)
    let settled = crate::evidence::now() - REPLAY_SETTLE_S;
    if let Some(mark) = newer.iter().map(|e| e.ts).filter(|t| *t <= settled).fold(None, |m: Option<f64>, t| Some(m.map_or(t, |m| m.max(t)))) {
        if mark > watermark(env) {
            let _ = crate::config::atomic_write(&watermark_path(env), &serde_json::json!({"watermark": mark}).to_string());
        }
    }
    n
}

/// How old a replayed line must be before the watermark may cover it.
const REPLAY_SETTLE_S: f64 = 5.0;

fn watermark_path(env: &Env) -> PathBuf {
    env.state.join("replay.json")
}

/// The `ts` of the newest replayed line judged so far (`NIMITZ_STATE/replay.json`).
fn watermark(env: &Env) -> f64 {
    std::fs::read(watermark_path(env))
        .ok()
        .and_then(|b| crate::evidence::first_json(&b))
        .and_then(|v| v.get("watermark").and_then(|w| w.as_f64()))
        .unwrap_or(0.0)
}

/// `registry.lock`, held exclusively until the returned File drops.
fn lock(env: &Env) -> std::io::Result<File> {
    let f = lock_file(env)?;
    let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(f) // dropping the fd releases the flock
}

/// `lock`, but give up after `budget` (polling with `LOCK_NB`): a hook never
/// waits on a stuck lock holder.
fn lock_within(env: &Env, budget: std::time::Duration) -> std::io::Result<File> {
    let f = lock_file(env)?;
    let deadline = std::time::Instant::now() + budget;
    loop {
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(f);
        }
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::WouldBlock || std::time::Instant::now() >= deadline {
            return Err(std::io::Error::new(e.kind(), format!("registry lock {}: {e}", env.registry_lock_path().display())));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn lock_file(env: &Env) -> std::io::Result<File> {
    let ctx = |e: std::io::Error| std::io::Error::new(e.kind(), format!("registry lock {}: {e}", env.registry_lock_path().display()));
    std::fs::create_dir_all(&env.state).map_err(ctx)?;
    File::create(env.registry_lock_path()).map_err(ctx)
}

/// Temp file next to registry.json, then rename (same device, atomic).
/// Every save also writes `registry.json.bak` (the last good copy, for `recover`).
fn write_unlocked(env: &Env, r: &mut Registry) -> std::io::Result<()> {
    r.updated_at = crate::evidence::now();
    let contents = serde_json::to_string_pretty(r).map_err(std::io::Error::other)?;
    let path = env.registry_path();
    crate::config::atomic_write(&path, &contents)
        .map_err(|e| std::io::Error::new(e.kind(), format!("registry {}: {e}", path.display())))?;
    let _ = crate::config::atomic_write(&bak_path(env), &contents);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::test_env;

    #[test]
    fn load_missing_or_corrupt_is_fresh() {
        let (_td, env) = test_env();
        let r = load(&env);
        assert_eq!(r.version, REGISTRY_VERSION);
        assert_eq!(r.machine, "test-mac");
        assert!(r.identities.is_empty());
        std::fs::create_dir_all(&env.state).unwrap();
        std::fs::write(env.registry_path(), "{not json").unwrap();
        assert!(load(&env).identities.is_empty());
    }

    #[test]
    fn a_corrupt_registry_is_kept_rebuilt_and_reported() {
        let (_td, env) = test_env();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        update(&env, |r| bind(r, &id, Role::Ship, &id.name(), "/w", "sid-1", "launch", 1.0)).unwrap();
        update(&env, |r| bind(r, &Id::admiral("MAIN"), Role::Admiral, "[MAIN]-ADMIRAL", "/a", "a-1", "launch", 2.0)).unwrap();
        assert!(bak_path(&env).is_file(), "every save keeps the last good copy");
        assert!(health(&env).is_empty());
        // a later session only the events know about
        let ev = r#"{"ts": 5.0, "event": "SessionStart", "session_id": "sid-2", "source": "resume", "labels": {"NIMITZ_ID": "MAIN/ARGOS/ARGOS", "NIMITZ_ROLE": "ship"}}"#;
        std::fs::write(env.events_path(), format!("{ev}\n")).unwrap();
        std::fs::write(env.registry_path(), "{garbage").unwrap();
        let r = load(&env);
        let rec = &r.identities[&id.0];
        assert_eq!(rec.lineage.iter().map(|l| l.session_id.as_str()).collect::<Vec<_>>(), ["sid-1", "sid-2"]);
        assert_eq!(rec.session.as_deref(), Some("sid-2"));
        assert!(r.identities.contains_key("MAIN"));
        // the bad file is kept, the registry is good again, and check says so
        let kept: Vec<_> = std::fs::read_dir(&env.state).unwrap().flatten().filter(|e| std::fs::read(e.path()).unwrap_or_default() == b"{garbage").collect();
        assert_eq!(kept.len(), 1);
        assert!(parse_file(&env.registry_path()).is_some());
        let h = health(&env);
        assert!(h.len() == 1 && h[0].code == "registry.corrupt" && h[0].severity == Severity::Error, "{h:?}");
        assert!(h[0].message.contains("registry.json.bak") && h[0].message.contains("1 SessionStart"), "{}", h[0].message);
        assert_eq!(load(&env), r, "a second load reads the rebuilt file (stamps round-trip exactly: serde_json float_roundtrip)");
    }

    #[test]
    fn a_read_only_registry_is_a_clear_error() {
        use std::os::unix::fs::PermissionsExt;
        let (_td, env) = test_env();
        save(&env, &load(&env)).unwrap();
        std::fs::write(env.registry_path(), "{garbage").unwrap();
        std::fs::set_permissions(&env.state, std::fs::Permissions::from_mode(0o555)).unwrap();
        let r = load(&env); // no panic, nothing written
        assert!(r.identities.is_empty());
        let err = update(&env, |_| ()).unwrap_err().to_string();
        let h = health(&env);
        std::fs::set_permissions(&env.state, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(err.contains("registry"), "{err}");
        assert!(h.iter().any(|i| i.code == "registry.corrupt" && i.message.contains("could not be kept")), "{h:?}");
        std::fs::write(env.registry_path(), serde_json::to_string(&fresh(&env)).unwrap()).unwrap();
        std::fs::set_permissions(env.registry_path(), std::fs::Permissions::from_mode(0o444)).unwrap();
        std::fs::set_permissions(&env.state, std::fs::Permissions::from_mode(0o555)).unwrap();
        let h = health(&env);
        std::fs::set_permissions(&env.state, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(h.iter().any(|i| i.code == "registry.readonly"), "{h:?}");
    }

    #[test]
    fn the_machine_label_lives_in_state_not_in_a_fleet() {
        let (_td, env) = test_env();
        assert_eq!(machine_label(&env), None, "no label before init");
        update(&env, |r| r.machine = "west-mac".into()).unwrap();
        crate::config::tests::make_fleet(&env);
        std::fs::remove_dir_all(env.fleet_dir("MAIN")).unwrap(); // every fleet gone
        assert_eq!(machine_label(&env).as_deref(), Some("west-mac"));
    }

    #[test]
    fn save_then_load_round_trips() {
        let (_td, env) = test_env();
        let mut r = load(&env);
        bind(&mut r, &Id::ship("MAIN", "ARGOS", "ARGOS"), Role::Ship, "[MAIN.ARGOS]-ARGOS", "/w", "sid-1", "launch", 1.0);
        save(&env, &r).unwrap();
        let got = load(&env);
        assert!(got.updated_at > 0.0, "every save stamps the registry");
        r.updated_at = got.updated_at;
        assert_eq!(got, r);
        let rec = &got.identities["MAIN/ARGOS/ARGOS"];
        assert_eq!(rec.session.as_deref(), Some("sid-1"));
        assert_eq!(rec.lineage.len(), 1);
        assert_eq!(rec.lineage[0].via, "launch");
    }

    #[test]
    fn bind_appends_lineage_and_moves_current() {
        let (_td, env) = test_env();
        let id = Id::commodore("MAIN", "ARGOS");
        update(&env, |r| {
            bind(r, &id, Role::Commodore, "[MAIN.ARGOS]-COMMODORE", "/sq", "sid-1", "launch", 1.0);
            bind(r, &id, Role::Commodore, "[MAIN.ARGOS]-COMMODORE", "/sq", "sid-2", "relaunch", 2.0);
        })
        .unwrap();
        let r = load(&env);
        let rec = &r.identities[&id.0];
        assert_eq!(rec.session.as_deref(), Some("sid-2"));
        assert_eq!(rec.lineage.iter().map(|l| l.session_id.as_str()).collect::<Vec<_>>(), ["sid-1", "sid-2"]);
    }

    #[test]
    fn update_is_read_modify_write() {
        let (_td, env) = test_env();
        update(&env, |r| bind(r, &Id::admiral("MAIN"), Role::Admiral, "[MAIN]-ADMIRAL", "/a", "s1", "launch", 1.0)).unwrap();
        update(&env, |r| bind(r, &Id::admiral("ALT"), Role::Admiral, "[ALT]-ADMIRAL", "/b", "s2", "launch", 2.0)).unwrap();
        let r = load(&env);
        assert_eq!(r.identities.len(), 2); // the second update kept the first identity
    }

    #[test]
    fn update_returns_the_closure_value_and_no_temp_files_remain() {
        let (_td, env) = test_env();
        let n = update(&env, |r| {
            bind(r, &Id::admiral("MAIN"), Role::Admiral, "[MAIN]-ADMIRAL", "/a", "s1", "launch", 1.0);
            r.identities.len()
        })
        .unwrap();
        assert_eq!(n, 1);
        let stray: Vec<_> = std::fs::read_dir(&env.state)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp-"))
            .collect();
        assert!(stray.is_empty(), "{stray:?}");
    }

    #[test]
    fn replay_binds_only_newer_lines_and_writes_only_when_one_binds() {
        let (_td, env) = test_env();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        update(&env, |r| bind_job(r, &id, Role::Ship, &id.name(), "/w", Some("j1"), "s1", "launch", 1.0)).unwrap();
        let stamp = load(&env).updated_at;
        let line = |ts: f64, sid: &str, job: &str, binding: Option<&str>| Event {
            ts,
            event: "SessionStart".into(),
            session_id: Some(sid.into()),
            source: Some("clear".into()),
            pid: Some(5),
            labels: [("NIMITZ_ID".to_string(), id.0.clone()), ("NIMITZ_ROLE".to_string(), "ship".into())].into(),
            job: Some(job.into()),
            kind: Some("bg".into()),
            binding: binding.map(str::to_string),
            ..Default::default()
        };
        // older than the registry, or judged by the hook: nothing
        assert_eq!(replay(&env, &[line(stamp - 1.0, "s2", "j1", None), line(stamp + 1.0, "s3", "j9", Some("dup.job"))]), 0);
        assert_eq!(load(&env).updated_at, stamp, "no write");
        // newer and lost: bound under the lock, once
        assert_eq!(replay(&env, &[line(stamp + 1.0, "s2", "j1", Some("none"))]), 1);
        let rec = load(&env).identities[&id.0].clone();
        assert_eq!(rec.session.as_deref(), Some("s2"));
        assert_eq!(rec.lineage.last().map(|l| l.via.as_str()), Some("clear"));
        let stamp2 = load(&env).updated_at;
        assert_eq!(replay(&env, &[line(stamp2 + 1.0, "s2", "j1", Some("bound"))]), 0, "already bound: nothing");
        assert_eq!(load(&env).updated_at, stamp2);
    }

    #[test]
    fn a_wake_never_overwrites_how_the_conversation_was_born() {
        let (_td, env) = test_env();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let mut r = load(&env);
        bind_job(&mut r, &id, Role::Ship, &id.name(), "/w", Some("j1"), "s1", "launch", 1.0);
        bind_job(&mut r, &id, Role::Ship, &id.name(), "/w", Some("j1"), "s2", "clear", 2.0);
        bind_job(&mut r, &id, Role::Ship, &id.name(), "/w", Some("j1"), "s2", "resume", 3.0);
        bind_job(&mut r, &id, Role::Ship, &id.name(), "/w", Some("j1"), "s2", "relaunch", 4.0);
        let vias: Vec<&str> = r.identities[&id.0].lineage.iter().map(|l| l.via.as_str()).collect();
        assert_eq!(vias, ["launch", "clear"]);
        // a birth via refines a wake (nimitz adopts or migrates what the hook saw resumed)
        bind_job(&mut r, &id, Role::Ship, &id.name(), "/w", Some("j1"), "s3", "resume", 5.0);
        bind_job(&mut r, &id, Role::Ship, &id.name(), "/w", Some("j1"), "s3", "migrate", 6.0);
        assert_eq!(r.identities[&id.0].lineage.last().unwrap().via, "migrate");
    }

    #[test]
    fn replay_applies_lines_in_time_order_and_the_folder_rule() {
        let (_td, env) = test_env();
        crate::config::tests::make_fleet(&env);
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let folder = crate::config::load_fleet(&env, "MAIN").ok().and_then(|f| crate::config::member_dir(&env, &f, &id)).unwrap().to_string_lossy().into_owned();
        update(&env, |r| bind_job(r, &id, Role::Ship, &id.name(), &folder, Some("j1"), "s1", "launch", 1.0)).unwrap();
        let stamp = load(&env).updated_at;
        let line = |ts: f64, sid: &str, source: &str, cwd: &str| Event {
            ts,
            event: "SessionStart".into(),
            session_id: Some(sid.into()),
            source: Some(source.into()),
            pid: Some(5),
            labels: [("NIMITZ_ID".to_string(), id.0.clone()), ("NIMITZ_ROLE".to_string(), "ship".into())].into(),
            job: Some("j1".into()),
            kind: Some("bg".into()),
            binding: Some("none".into()),
            cwd: Some(cwd.into()),
            ..Default::default()
        };
        // out of order in the file: a clear (t+2) before a resume of it (t+1)? no: the
        // later line by ts wins, whatever the file order
        let events = vec![line(stamp + 2.0, "s3", "clear", &folder), line(stamp + 1.0, "s2", "clear", &folder)];
        assert_eq!(replay(&env, &events), 2);
        let rec = load(&env).identities[&id.0].clone();
        assert_eq!(rec.session.as_deref(), Some("s3"));
        assert_eq!(rec.lineage.iter().map(|l| l.session_id.as_str()).collect::<Vec<_>>(), ["s1", "s2", "s3"]);
        // a line from another folder: elsewhere, not bound
        let stamp = load(&env).updated_at;
        assert_eq!(replay(&env, &[line(stamp + 1.0, "s9", "startup", "/somewhere/else")]), 0);
        assert_eq!(load(&env).identities[&id.0].session.as_deref(), Some("s3"));
    }

    #[test]
    fn a_registry_that_cannot_be_read_is_an_error_for_update_and_fresh_for_a_read() {
        use std::os::unix::fs::PermissionsExt;
        let (_td, env) = test_env();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        update(&env, |r| bind(r, &id, Role::Ship, &id.name(), "/w", "s1", "launch", 1.0)).unwrap();
        let before = std::fs::read_to_string(env.registry_path()).unwrap();
        std::fs::set_permissions(env.registry_path(), std::fs::Permissions::from_mode(0o000)).unwrap();
        assert!(load(&env).identities.is_empty(), "a read-only caller sees nothing, and nothing is written");
        let err = update(&env, |r| r.identities.clear()).unwrap_err();
        assert!(err.to_string().contains("registry"), "{err}");
        assert!(replay(&env, &[]) == 0);
        std::fs::set_permissions(env.registry_path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(std::fs::read_to_string(env.registry_path()).unwrap(), before, "never written over");
        assert_eq!(load(&env).identities.len(), 1);
    }

    #[test]
    fn a_replayed_line_that_bound_nothing_is_not_probed_again_once_settled() {
        let (_td, env) = test_env();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        update(&env, |r| bind_job(r, &id, Role::Ship, &id.name(), "/w", Some("j1"), "s1", "launch", 1.0)).unwrap();
        // a registry written 20 s ago
        let mut r = load(&env);
        r.updated_at = crate::evidence::now() - 20.0;
        std::fs::write(env.registry_path(), serde_json::to_string(&r).unwrap()).unwrap();
        let line = |ts: f64, sid: &str| Event {
            ts,
            event: "SessionStart".into(),
            session_id: Some(sid.into()),
            source: Some("resume".into()),
            pid: Some(5),
            labels: [("NIMITZ_ID".to_string(), id.0.clone()), ("NIMITZ_ROLE".to_string(), "ship".into())].into(),
            kind: Some("unknown".into()),
            binding: Some("none".into()),
            ..Default::default()
        };
        // a lost line whose session no job runs: judged held, nothing bound; it
        // is 10 s old, so the mark covers it
        let old = line(crate::evidence::now() - 10.0, "s2");
        assert_eq!(replay(&env, &[old.clone()]), 0);
        assert!((watermark(&env) - old.ts).abs() < 1e-6, "{}", watermark(&env));
        // a job now runs s2: the line stays skipped (judged once, never again)
        let p = env.claude_home.join("jobs").join("j1").join("state.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, serde_json::json!({"sessionId": "s1", "resumeSessionId": "s2"}).to_string()).unwrap();
        assert_eq!(replay(&env, &[old.clone()]), 0);
        assert_eq!(load(&env).identities[&id.0].session.as_deref(), Some("s1"));
        // a fresh line (newer than the settle window) is probed every time until it binds
        let fresh = line(crate::evidence::now(), "s2");
        assert_eq!(replay(&env, &[old.clone(), fresh.clone()]), 1);
        assert_eq!(load(&env).identities[&id.0].session.as_deref(), Some("s2"));
        assert!(watermark(&env) < fresh.ts, "a line inside the settle window never marks");
    }

    #[test]
    fn identity_of_wants_this_home_and_a_role_that_fits() {
        let (_td, env) = test_env();
        let home = env.home.to_string_lossy().into_owned();
        let l = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
        assert_eq!(identity_of(&env, &l(&[("NIMITZ_ID", "F/SQ/S"), ("NIMITZ_ROLE", "ship"), ("NIMITZ_HOME", &home)])), Some(Id::ship("F", "SQ", "S")));
        assert_eq!(identity_of(&env, &l(&[("NIMITZ_ID", "F/SQ"), ("NIMITZ_ROLE", "commodore")])), Some(Id::commodore("F", "SQ")), "no home label: this home");
        assert_eq!(identity_of(&env, &l(&[("NIMITZ_ID", "F/SQ/S"), ("NIMITZ_ROLE", "commodore")])), None, "the role must fit");
        assert_eq!(identity_of(&env, &l(&[("NIMITZ_ID", "F/SQ/~t"), ("NIMITZ_ROLE", "tender")])), None, "a 0.3.x tender");
        let other = l(&[("NIMITZ_ID", "F/SQ/S"), ("NIMITZ_ROLE", "ship"), ("NIMITZ_HOME", "/elsewhere")]);
        assert_eq!(identity_of(&env, &other), None);
        assert!(other_home(&env, &other));
        assert!(!other_home(&env, &l(&[("NIMITZ_HOME", "/elsewhere")])), "a home alone is no label");
    }

    #[test]
    fn concurrent_updates_do_not_lose_writes() {
        let (_td, env) = test_env();
        let mut handles = vec![];
        for t in 0..8 {
            let env = env.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..5 {
                    let id = Id::ship("MAIN", "SQ", &format!("S{t}_{i}"));
                    update(&env, |r| {
                        bind(r, &id, Role::Ship, &id.name(), "/w", &format!("sid-{t}-{i}"), "launch", 0.0)
                    })
                    .unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(load(&env).identities.len(), 40);
    }
}
