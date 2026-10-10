//! Registry of identities and lineage (contracts.md section 5). Owner: core-config.
use crate::env::Env;
use crate::types::*;
use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

/// Current contents. A missing file is a fresh registry for this machine
/// (unless `registry.json.bak` holds identities: then the file was lost, and
/// the copy and the events rebuild it). A
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
/// it (`read_registry`). What a reader gets is this home's view: the
/// bindings of another home are stripped in memory (`drop_other_homes`).
fn load_with(env: &Env, locked: bool) -> (Registry, bool) {
    let Loaded { mut r, v1, .. } = load_checked(env, locked).unwrap_or_else(|_| Loaded { r: fresh(env), v1: false, dirty: false });
    drop_other_homes(env, &mut r);
    (r, v1)
}

/// What a load found. `v1`: the file is a 0.3.x one. `dirty`: the shape in
/// memory differs from the file (a v1 file, tenders moved to `retired`, a
/// lost file rebuilt from the last good copy), so a writer saves it even
/// when it changes nothing itself.
struct Loaded {
    r: Registry,
    v1: bool,
    dirty: bool,
}

/// `load_with` before the strip, with the read error when the file is there
/// but unreadable: a fresh registry must never be written over it. What is
/// on disk, every home's bindings included: the shape a writer works on.
fn load_checked(env: &Env, locked: bool) -> std::io::Result<Loaded> {
    let mut rebuilt = false;
    let mut r = match std::fs::read_to_string(env.registry_path()) {
        // not there: fresh, unless the last good copy says the file was lost
        // (an rm by hand): then the copy and the events rebuild it, and a
        // locked writer saves the result
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match parse_file(&bak_path(env)).filter(|b| !b.identities.is_empty()) {
            Some(_) => {
                rebuilt = true;
                rebuild(env).0
            }
            None => fresh(env),
        },
        Err(e) => return Err(std::io::Error::new(e.kind(), format!("registry {}: {e}", env.registry_path().display()))),
        // a registry a newer nimitz wrote is refused, never "recovered" as
        // corrupt: it is not read further and never written over
        Ok(raw) if too_new(env, &raw).is_some() => return Err(std::io::Error::other(too_new(env, &raw).map(|t| t.to_string()).unwrap_or_default())),
        Ok(raw) => match parse(&raw) {
            Ok(r) => r,
            Err(_) => {
                rebuilt = true; // the file does not parse: memory holds the rebuild
                if locked {
                    recover_locked(env)
                } else {
                    match lock(env) {
                        Ok(_lock) => recover_locked(env),
                        Err(_) => rebuild(env).0, // read-only: rebuild in memory only
                    }
                }
            }
        },
    };
    let v1 = r.version < 2;
    // A 0.3.x registry: its tender identities become history (plan-v4 section
    // 8) and `current` becomes `session` with the job filled from the job rows
    // (section 2). A read never writes; the next `update` saves the v2 shape
    // and keeps the old file as `registry.json.v1`.
    let moved = crate::migrate::retire_tenders(&mut r);
    crate::migrate::registry_v2(env, &mut r);
    Ok(Loaded { r, v1, dirty: v1 || moved > 0 || rebuilt })
}

/// Is `h` (a `NIMITZ_HOME`, as written) this env's home, as written or as
/// a real path?
fn same_home(env: &Env, h: &str) -> bool {
    Path::new(h) == env.home || crate::env::canon(Path::new(h)) == crate::env::canon(&env.home)
}

/// Is job `j` another home's, as the registry knows it? The lineage entries
/// of the job say which home bound them (`home`, stamped at the write); an
/// entry an older nimitz wrote says nothing, and then the job's row decides
/// while it exists (`foreign_row`). So a binding the other home left keeps
/// its home once its row is gone: it never comes back into this home's view.
fn foreign_job(env: &Env, r: &Registry, j: &str) -> bool {
    let stamped = r.identities.values().chain(r.retired.values()).flat_map(|rec| rec.lineage.iter()).find(|l| l.job.as_deref() == Some(j) && l.home.is_some());
    match stamped.and_then(|l| l.home.as_deref()) {
        Some(h) => !same_home(env, h),
        None => foreign_row(env, j),
    }
}

/// Does job `j` have a row (`jobs/<j>/state.json`) whose saved labels name
/// another `NIMITZ_HOME`? A row that is not there, or not readable, is not.
fn foreign_row(env: &Env, j: &str) -> bool {
    std::fs::read(env.claude_home.join("jobs").join(j).join("state.json"))
        .ok()
        .and_then(|b| crate::evidence::first_json(&b))
        .map(|d| other_home(env, &crate::evidence::job_labels(&d)))
        .unwrap_or(false)
}

/// Does job `j` have a row of this home? The binding table's "another job
/// whose row still exists" (`dup.job`): a row of another home does not
/// count, so a binding the other home's hook left in a shared `NIMITZ_STATE`
/// never keeps this home's own job from binding.
pub fn job_row_of_this_home(env: &Env, j: &str) -> bool {
    env.claude_home.join("jobs").join(j).join("state.json").is_file() && !foreign_row(env, j)
}

/// Strip every binding to a job of another `NIMITZ_HOME` (`foreign_job`: the
/// entry's `home`, else the job's saved labels, `respawnFlags`). Such a session never appears (plan-v4
/// section 6), and its binding is not this home's either: its hook ran with
/// that home in its own env (the label is the env), so when two homes share
/// one `NIMITZ_STATE` it bound the identity here. The record keeps this
/// home's own history: the foreign lineage entries go, `job` and `session`
/// fall back to the newest entry left, and an identity with nothing of its
/// own left goes. Readers strip in memory only; a write never saves the
/// stripped shape: which home is "other" depends on the env of the process,
/// and the hook of the other home (its env is that home) would strip and
/// save away every record of this one (acc-D F1: a registry left with one
/// identity). On disk every home's bindings stay side by side.
fn drop_other_homes(env: &Env, r: &mut Registry) {
    // an entry is judged by its own stamp, else by its job's row
    let foreign_entry = |l: &LineageEntry| match l.home.as_deref() {
        Some(h) => !same_home(env, h),
        None => l.job.as_deref().is_some_and(|j| foreign_row(env, j)),
    };
    // only a record the foreign job touched changes; every other record stays
    // as it is (an empty lineage, or a session outside the lineage such as a
    // v1-migrated current, are not this rule's to judge)
    r.identities.retain(|_, rec| {
        let before = rec.lineage.len();
        let stripped: Vec<String> = rec.lineage.iter().filter(|l| foreign_entry(l)).filter_map(|l| l.job.clone()).collect();
        rec.lineage.retain(|l| !foreign_entry(l));
        // the record's job: the job of a stripped entry, or (no entry of its
        // own says) a row of another home
        let job_foreign = rec.job.as_ref().is_some_and(|j| !rec.lineage.iter().any(|l| l.job.as_ref() == Some(j)) && (stripped.contains(j) || foreign_row(env, j)));
        let touched = job_foreign || rec.lineage.len() != before;
        if !touched {
            return true;
        }
        if job_foreign {
            rec.job = rec.lineage.last().and_then(|l| l.job.clone());
        }
        if !rec.session.as_ref().is_some_and(|s| rec.lineage.iter().any(|l| &l.session_id == s)) {
            rec.session = rec.lineage.last().map(|l| l.session_id.clone());
        }
        !rec.lineage.is_empty()
    });
}

fn fresh(env: &Env) -> Registry {
    Registry { version: REGISTRY_VERSION, min_nimitz: Some(REGISTRY_MIN_NIMITZ.into()), machine: env.hostname.clone(), ..Default::default() }
}

/// The registry text is too new for this nimitz (`formats::guard`: a
/// `version` above `REGISTRY_VERSION`, or a `min_nimitz` above this nimitz).
pub fn too_new(env: &Env, raw: &str) -> Option<crate::formats::TooNew> {
    crate::formats::guard_json_bytes("registry", &env.registry_path(), raw.as_bytes(), "version").err()
}

/// The `registry` artifact's lift (`formats`): a 0.3.x (v1) file read and
/// saved as version 2 by the load that has always read it (`registry.json.v1`
/// keeps the old file).
pub fn lift_v1(env: &Env) -> Result<(), String> {
    update(env, |_| ()).map_err(|e| e.to_string())
}

fn parse_file(p: &Path) -> Option<Registry> {
    parse(&std::fs::read_to_string(p).ok()?).ok()
}

/// A registry from its text, tolerant of a role this nimitz does not know
/// (a newer nimitz wrote it): such a record takes the role its id spells
/// when the id is one nimitz can place (`Id::well_formed`), its own role
/// kept aside; else the whole record is kept aside (`registry.unknown` in
/// check), never a member. `render` writes both back where they were. One
/// record never fails the whole load.
pub fn parse(raw: &str) -> serde_json::Result<Registry> {
    use serde_json::Value;
    let mut v: Value = serde_json::from_str(raw)?;
    let mut unknown = Unknown::default();
    for map in ["identities", "retired"] {
        let Some(ids) = v.get_mut(map).and_then(Value::as_object_mut) else { continue };
        let keys: Vec<String> = ids.keys().cloned().collect();
        for k in keys {
            let known = ids[&k].get("role").is_some_and(|r| serde_json::from_value::<Role>(r.clone()).is_ok());
            if known {
                continue;
            }
            let id = Id(k.clone());
            let at = (map.to_string(), k.clone());
            match ids[&k].as_object_mut() {
                Some(rec) if id.well_formed() => {
                    if let Some(old) = rec.insert("role".into(), Value::from(id.role().as_str())) {
                        unknown.roles.insert(at, old);
                    }
                }
                _ => {
                    if let Some(rec) = ids.remove(&k) {
                        unknown.records.insert(at, rec);
                    }
                }
            }
        }
    }
    let mut r: Registry = serde_json::from_value(v)?;
    r.unknown = unknown;
    Ok(r)
}

/// The registry as its file holds it: what `parse` set aside goes back
/// where it was (a record nimitz now has under the same key keeps nimitz's).
pub fn render(r: &Registry) -> serde_json::Result<String> {
    use serde_json::Value;
    let mut v = serde_json::to_value(r)?;
    for ((map, key), role) in &r.unknown.roles {
        if let Some(rec) = v.get_mut(map.as_str()).and_then(|m| m.get_mut(key.as_str())).and_then(Value::as_object_mut) {
            rec.insert("role".into(), role.clone());
        }
    }
    for ((map, key), rec) in &r.unknown.records {
        if v.get(map.as_str()).and_then(Value::as_object).is_none() {
            v[map.as_str()] = Value::Object(Default::default());
        }
        if let Some(m) = v[map.as_str()].as_object_mut() {
            m.entry(key.clone()).or_insert_with(|| rec.clone());
        }
    }
    serde_json::to_string_pretty(&v)
}

fn bak_path(env: &Env) -> PathBuf {
    env.state.join("registry.json.bak")
}

/// Where the last v1 file is kept once the v2 shape is written.
pub fn v1_path(env: &Env) -> PathBuf {
    env.state.join("registry.json.v1")
}

/// What `recover` could rebuild from: the last good copy, then the
/// SessionStart lines newer than it (the same binding rules as the hook,
/// `apply_start`; a line the copy already holds is not replayed over it).
fn rebuild(env: &Env) -> (Registry, String) {
    let (mut r, mut from) = match parse_file(&bak_path(env)) {
        Some(r) => (r, vec!["registry.json.bak".to_string()]),
        None => (fresh(env), vec![]),
    };
    let since = r.updated_at;
    let copy = r.clone();
    let mut added = 0;
    let mut events = crate::evidence::events(env);
    events.sort_by(|a, b| a.ts.partial_cmp(&b.ts).unwrap_or(std::cmp::Ordering::Equal));
    for e in events.iter().filter(|e| e.event == "SessionStart" && e.ts > since) {
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
        stamp_home(env, &copy, &mut r); // this home's lines (`identity_of`): its bindings
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
    // a file a newer nimitz wrote (meanwhile) is not corrupt: never moved aside
    if std::fs::read_to_string(env.registry_path()).ok().is_some_and(|raw| too_new(env, &raw).is_some()) {
        return r;
    }
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
    // the file is gone but its last good copy is not: nimitz works from the
    // copy until the next write saves it (a reset removes both files)
    if !reg.exists() {
        if let Some(b) = parse_file(&bak_path(env)).filter(|b| !b.identities.is_empty()) {
            out.push(Issue {
                code: "registry.rebuilt".into(),
                severity: Severity::Info,
                message: format!(
                    "{} is missing; nimitz works from {} ({} identities) plus the SessionStart lines newer than it, and the next registry write saves the rebuilt file. To reset the registry, remove both files",
                    reg.display(),
                    bak_path(env).display(),
                    b.identities.len()
                ),
                fix: None,
                id: None,
            });
        }
    }
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
    if let Some(r) = std::fs::read_to_string(&reg).ok().filter(|raw| too_new(env, raw).is_none()).and_then(|raw| parse(&raw).ok()).filter(|r| !r.unknown.records.is_empty()) {
        out.push(Issue {
            code: "registry.unknown".into(),
            severity: Severity::Warn,
            message: format!(
                "{} holds {} record(s) this nimitz cannot read (a role it does not know, on an id it cannot place: a newer nimitz wrote them?): {}; kept as they are, never members",
                reg.display(),
                r.unknown.records.len(),
                r.unknown.records.keys().map(|(_, k)| k.clone()).collect::<Vec<_>>().join(", ")
            ),
            fix: Some("update nimitz to the version that wrote them".into()),
            id: None,
        });
    }
    if std::fs::read_to_string(&reg).is_ok_and(|raw| too_new(env, &raw).is_some()) {
        // `formats::issues` says it (config.too_new): not corrupt, not this nimitz's to read
    } else if std::fs::read_to_string(&reg).is_ok_and(|raw| parse(&raw).is_err()) {
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
        (!crate::os::writable(p)).then(|| format!("{} is read-only", p.display()))
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
/// `r` keeps its own). The same guard as `update`: a save that drops an
/// identity the file has is refused.
pub fn save(env: &Env, r: &Registry) -> std::io::Result<()> {
    let _lock = lock(env)?;
    if let Some(t) = std::fs::read_to_string(env.registry_path()).ok().and_then(|raw| too_new(env, &raw)) {
        return Err(std::io::Error::other(t.to_string()));
    }
    let on_disk = parse_file(&env.registry_path()).map(|cur| cur.identities.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
    refuse_shrink(env, &on_disk, r)?;
    write_unlocked(env, &mut r.clone())
}

/// The invariant of every write: the registry never shrinks except by an
/// explicit removal, and no writer removes an identity today. `had` names
/// the identities the file held when it was read; one that `r` has in
/// neither `identities` nor `retired` (a tender moved there) is lost, and
/// the write is refused: the file and `registry.json.bak` stay as they are.
/// This is the stop for whatever a load or a caller gets wrong (a strip, a
/// fresh registry over a file read wrong), seen once in a real run as a
/// registry left with one identity (acc-D F1).
fn refuse_shrink(env: &Env, had: &[String], r: &Registry) -> std::io::Result<()> {
    let lost: Vec<&str> = had.iter().filter(|k| !r.identities.contains_key(*k) && !r.retired.contains_key(*k)).map(String::as_str).collect();
    if lost.is_empty() {
        return Ok(());
    }
    Err(std::io::Error::other(format!(
        "registry {}: the write would drop {} of {} identities ({}); a registry never shrinks except by an explicit removal, so nothing was written",
        env.registry_path().display(),
        lost.len(),
        had.len(),
        lost.join(", ")
    )))
}

/// Every lineage entry `after` has that `before` had not (by session and
/// job) was bound by this process: it carries this env's home (as written),
/// so a reader judges it without the job's row (`foreign_job`).
fn stamp_home(env: &Env, before: &Registry, after: &mut Registry) {
    let home = env.home.to_string_lossy().into_owned();
    for (map, was) in [(&mut after.identities, &before.identities), (&mut after.retired, &before.retired)] {
        for (k, rec) in map.iter_mut() {
            let old = was.get(k);
            for l in rec.lineage.iter_mut().filter(|l| l.home.is_none()) {
                if !old.is_some_and(|o| o.lineage.iter().any(|x| x.session_id == l.session_id && x.job == l.job)) {
                    l.home = Some(home.clone());
                }
            }
        }
    }
}

/// Lock, load, apply `f`, save. The only way to change the registry. The
/// first write over a v1 file keeps that file as `registry.json.v1`. A file
/// that is there but cannot be read is an error: nothing is written over it.
/// `f` works on what is on disk, every home's bindings included (the view a
/// reader gets strips the other homes', `drop_other_homes`), and a result
/// with fewer identities than the file is refused (`refuse_shrink`).
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
    let Loaded { mut r, v1, dirty } = load_checked(env, true)?;
    if v1 && !v1_path(env).exists() {
        let _ = std::fs::copy(env.registry_path(), v1_path(env));
    }
    let had: Vec<String> = r.identities.keys().cloned().collect();
    let before = r.clone();
    let out = f(&mut r);
    refuse_shrink(env, &had, &r)?;
    stamp_home(env, &before, &mut r);
    // a closure that changed nothing (the hook's held, dup.job, elsewhere and
    // same judgements) writes nothing: the file keeps its stamp. A file that
    // is not there yet is written (`init` saves the fresh registry as it is)
    if dirty || r != before || !env.registry_path().exists() {
        write_unlocked(env, &mut r)?;
    }
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
        extra: Default::default(),
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
        _ => rec.lineage.push(LineageEntry { session_id: session_id.to_string(), job: rec.job.clone(), via: via.to_string(), at: now, home: None }),
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
        if !same_home(env, h) {
            return None;
        }
    }
    let id = Id(labels.get("NIMITZ_ID")?.clone());
    let shape_ok = id.well_formed();
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
    // a job's labels count only when its own launch saved them (the daemon's
    // inherited labels do not; `evidence::job_backs_labels`)
    if e.job.is_some() && crate::evidence::job_backs_labels(&env.claude_home, e.job.as_deref(), &e.labels) != Some(true) {
        return Binding::None;
    }
    if e.kind.is_none() && e.job.is_none() {
        let rec = r.identities.get(&id.0);
        if rec.and_then(|x| x.session.as_deref()) == Some(sid) {
            return Binding::Same;
        }
        let via = via_of(e.source.as_deref(), true);
        bind_into(&mut r.identities, &id, id.role(), &id.name(), "", None, sid, &via, e.ts);
        return Binding::Bound(via);
    }
    let job = match (e.kind.as_deref(), &e.job) {
        (Some("unknown"), None) => job_running(env, sid),
        (_, j) => j.clone(),
    };
    let cwd = e.cwd.clone().unwrap_or_default();
    let folder = identity_folder(env, r, &id);
    apply_start(r, &id, &cwd, folder.as_deref(), job.as_deref(), sid, e.source.as_deref(), e.ts, |j| job_row_of_this_home(env, j))
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
/// TOML read, only for an identity that never bound). A record whose job is
/// a row of another home has that home's folder in `cwd` (its hook wrote
/// it): not this home's, so the config decides.
pub fn identity_folder(env: &Env, r: &Registry, id: &Id) -> Option<PathBuf> {
    let own = r.identities.get(&id.0).filter(|x| !x.job.as_deref().is_some_and(|j| foreign_job(env, r, j)));
    match own.map(|x| x.cwd.clone()).filter(|c| !c.is_empty()) {
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

/// Run `f` under `registry.lock` (the one writer lock of `NIMITZ_STATE`:
/// the registry, `active.json`); with a `budget`, give up after it (a hook).
pub fn with_lock<T>(env: &Env, budget: Option<std::time::Duration>, f: impl FnOnce() -> T) -> std::io::Result<T> {
    let _lock = match budget {
        Some(b) => lock_within(env, b)?,
        None => lock(env)?,
    };
    Ok(f())
}

/// `registry.lock`, held exclusively until the returned File drops.
fn lock(env: &Env) -> std::io::Result<File> {
    let f = lock_file(env)?;
    f.lock()?;
    Ok(f) // dropping the file releases the lock
}

/// `lock`, but give up after `budget` (polling with `LOCK_NB`): a hook never
/// waits on a stuck lock holder.
fn lock_within(env: &Env, budget: std::time::Duration) -> std::io::Result<File> {
    let f = lock_file(env)?;
    let deadline = std::time::Instant::now() + budget;
    loop {
        let e = match f.try_lock() {
            Ok(()) => return Ok(f),
            Err(std::fs::TryLockError::WouldBlock) => std::io::Error::from(std::io::ErrorKind::WouldBlock),
            Err(std::fs::TryLockError::Error(e)) => e,
        };
        if e.kind() != std::io::ErrorKind::WouldBlock || std::time::Instant::now() >= deadline {
            return Err(std::io::Error::new(e.kind(), format!("registry lock {}: {e}", env.registry_lock_path().display())));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn lock_file(env: &Env) -> std::io::Result<File> {
    let ctx = |e: std::io::Error| std::io::Error::new(e.kind(), format!("registry lock {}: {e}", env.registry_lock_path().display()));
    std::fs::create_dir_all(&env.state).map_err(ctx)?;
    // never truncate: another process may hold the lock on this very file
    std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(env.registry_lock_path()).map_err(ctx)
}

/// Temp file next to registry.json, then rename (same device, atomic).
/// Every save also writes `registry.json.bak` (the last good copy, for `recover`).
fn write_unlocked(env: &Env, r: &mut Registry) -> std::io::Result<()> {
    r.updated_at = crate::evidence::now();
    let contents = render(r).map_err(std::io::Error::other)?;
    let path = env.registry_path();
    crate::config::atomic_write(&path, &contents)
        .map_err(|e| std::io::Error::new(e.kind(), format!("registry {}: {e}", path.display())))?;
    let _ = crate::config::atomic_write(&bak_path(env), &contents);
    Ok(())
}

#[cfg(test)]
mod tests {
    /// A job row as nimitz launches it: the identity's labels saved in
    /// `respawnFlags` (a job's labels count only when its launch saved them).
    fn labelled_row(env: &Env, short: &str, sid: &str, id: &Id) {
        let settings = serde_json::json!({"env": {"NIMITZ_ID": id.0, "NIMITZ_ROLE": id.role().as_str()}}).to_string();
        let p = env.claude_home.join("jobs").join(short).join("state.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::json!({"sessionId": sid, "resumeSessionId": sid, "state": "running", "respawnFlags": ["--settings", settings]}).to_string()).unwrap();
    }

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
    fn a_too_new_registry_is_refused_and_never_moved_aside() {
        let (_td, env) = test_env();
        std::fs::create_dir_all(&env.state).unwrap();
        // a newer nimitz's registry, in a shape this one could not parse either
        let raw = serde_json::json!({"version": REGISTRY_VERSION + 1, "min_nimitz": "99.0.0", "machine": "m", "identities": "a shape of the future"}).to_string();
        std::fs::write(env.registry_path(), &raw).unwrap();
        assert!(load(&env).identities.is_empty(), "a reader gets nothing of it");
        let e = update(&env, |r| r.machine = "x".into()).unwrap_err();
        assert!(e.to_string().contains("needs nimitz >= 99.0.0 (this machine runs"), "{e}");
        assert!(save(&env, &fresh(&env)).is_err());
        assert_eq!(std::fs::read_to_string(env.registry_path()).unwrap(), raw, "never written over");
        let names: Vec<String> = std::fs::read_dir(&env.state).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(!names.iter().any(|n| n.contains("corrupt")), "never moved aside as corrupt: {names:?}");
        assert!(!health(&env).iter().any(|i| i.code.starts_with("registry.")), "{:?}", health(&env));
        assert!(crate::formats::issues(&env).iter().any(|i| i.code == "config.too_new" && i.message.contains("needs nimitz >= 99.0.0")));
        // a version above this nimitz's alone is refused the same way
        std::fs::write(env.registry_path(), serde_json::json!({"version": REGISTRY_VERSION + 1, "machine": "m"}).to_string()).unwrap();
        assert!(update(&env, |_| ()).unwrap_err().to_string().contains("is format 4"));
        // equal, or absent: read and written as before
        for min in [Some(crate::formats::NIMITZ), None] {
            let mut v = serde_json::json!({"version": REGISTRY_VERSION, "machine": "m", "identities": {}});
            if let Some(m) = min {
                v["min_nimitz"] = m.into();
            }
            std::fs::write(env.registry_path(), v.to_string()).unwrap();
            update(&env, |r| r.machine = "y".into()).unwrap();
            assert_eq!(load(&env).machine, "y");
        }
    }

    #[test]
    fn a_corrupt_registry_is_kept_rebuilt_and_reported() {
        let (_td, env) = test_env();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        update(&env, |r| bind(r, &id, Role::Ship, &id.name(), "/w", "sid-1", "launch", 1.0)).unwrap();
        update(&env, |r| bind(r, &Id::admiral("MAIN"), Role::Admiral, "[MAIN]-ADMIRAL", "/a", "a-1", "launch", 2.0)).unwrap();
        assert!(bak_path(&env).is_file(), "every save keeps the last good copy");
        assert!(health(&env).is_empty());
        // a later session only the events know about (newer than the copy:
        // an older line is one the copy already holds, never replayed over it)
        let ts = parse_file(&bak_path(&env)).unwrap().updated_at + 1.0;
        let ev = format!(r#"{{"ts": {ts}, "event": "SessionStart", "session_id": "sid-2", "source": "resume", "labels": {{"NIMITZ_ID": "MAIN/ARGOS/ARGOS", "NIMITZ_ROLE": "ship"}}}}"#);
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

    #[cfg(unix)]
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

    /// The write guard: a registry never shrinks except by an explicit
    /// removal (none exists today). A closure that drops an identity, or a
    /// save of a smaller registry, is refused; the file and the last good
    /// copy stay as they were. A tender moved to `retired` is not a loss.
    #[test]
    fn a_write_that_drops_an_identity_is_refused_and_the_file_stays() {
        let (_td, env) = test_env();
        let ship = Id::ship("MAIN", "ARGOS", "ARGOS");
        update(&env, |r| bind(r, &ship, Role::Ship, &ship.name(), "/w", "s1", "launch", 1.0)).unwrap();
        update(&env, |r| bind(r, &Id::admiral("MAIN"), Role::Admiral, "[MAIN]-ADMIRAL", "/a", "a1", "launch", 2.0)).unwrap();
        let (file, bak) = (std::fs::read_to_string(env.registry_path()).unwrap(), std::fs::read_to_string(bak_path(&env)).unwrap());
        for (what, res) in [
            ("clear", update(&env, |r| r.identities.clear()).map(|_| ())),
            ("remove one", update(&env, |r| drop(r.identities.remove("MAIN"))).map(|_| ())),
            ("replace with a fresh one", update(&env, |r| *r = fresh(&env)).map(|_| ())),
            ("save a smaller one", save(&env, &fresh(&env))),
        ] {
            let err = res.unwrap_err().to_string();
            assert!(err.contains("would drop") && err.contains("MAIN") && err.contains("nothing was written"), "{what}: {err}");
            assert_eq!(std::fs::read_to_string(env.registry_path()).unwrap(), file, "{what}: the file changed");
            assert_eq!(std::fs::read_to_string(bak_path(&env)).unwrap(), bak, "{what}: the last good copy changed");
        }
        // a write that keeps every identity goes through, and so does a tender's move to retired
        update(&env, |r| r.machine = "west-mac".into()).unwrap();
        assert_eq!(load(&env).machine, "west-mac");
        let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(env.registry_path()).unwrap()).unwrap();
        v["identities"]["MAIN/ARGOS/~fix"] = serde_json::json!({"role": "tender", "name": "[MAIN.ARGOS]-COMMODORE/fix", "cwd": "/sq", "session": "t1", "lineage": []});
        std::fs::write(env.registry_path(), v.to_string()).unwrap();
        update(&env, |_| ()).unwrap();
        let r = load(&env);
        assert_eq!(r.identities.len(), 2);
        assert!(r.retired.contains_key("MAIN/ARGOS/~fix"));
    }

    /// A registry.json that is gone while its last good copy holds
    /// identities was lost, not reset: the copy and the events rebuild it,
    /// and the next write saves it. A copy with nothing in it is no copy.
    #[test]
    fn a_lost_registry_file_is_rebuilt_from_the_last_good_copy_not_replaced_by_a_fresh_one() {
        let (_td, env) = test_env();
        let ship = Id::ship("MAIN", "ARGOS", "ARGOS");
        update(&env, |r| bind_job(r, &ship, Role::Ship, &ship.name(), "/w", Some("j1"), "s1", "launch", 1.0)).unwrap();
        update(&env, |r| bind(r, &Id::admiral("MAIN"), Role::Admiral, "[MAIN]-ADMIRAL", "/a", "a1", "launch", 2.0)).unwrap();
        labelled_row(&env, "j1", "s-new", &ship);
        std::fs::remove_file(env.registry_path()).unwrap();
        // only the lines newer than the copy are replayed over it: an older
        // line (one the copy already holds, or a stale one) changes nothing
        let bak_at = parse_file(&bak_path(&env)).unwrap().updated_at;
        let line = |ts: f64, sid: &str| serde_json::json!({"ts": ts, "event": "SessionStart", "session_id": sid, "source": "clear", "job": "j1", "kind": "bg",
            "labels": {"NIMITZ_ID": ship.0, "NIMITZ_ROLE": "ship"}}).to_string();
        std::fs::write(env.events_path(), format!("{}\n{}\n", line(bak_at - 1.0, "s-old"), line(bak_at + 1.0, "s-new"))).unwrap();
        let r = load(&env);
        assert_eq!(r.identities.len(), 2, "a read sees the copy");
        assert_eq!(r.identities[&ship.0].lineage.iter().map(|l| l.session_id.as_str()).collect::<Vec<_>>(), ["s1", "s-new"]);
        assert!(!env.registry_path().exists(), "a read writes nothing");
        let h = health(&env);
        assert!(h.len() == 1 && h[0].code == "registry.rebuilt" && h[0].severity == Severity::Info, "{h:?}");
        assert!(h[0].message.contains("registry.json.bak") && h[0].message.contains("2 identities") && h[0].message.contains("remove both files"), "{}", h[0].message);
        std::fs::write(env.events_path(), "").unwrap();
        // the hook's write: the identity it binds joins the two, nothing is lost
        let ev = Event {
            ts: 5.0,
            event: "SessionStart".into(),
            session_id: Some("s2".into()),
            source: Some("clear".into()),
            labels: [("NIMITZ_ID".to_string(), ship.0.clone()), ("NIMITZ_ROLE".to_string(), "ship".into())].into(),
            job: Some("j1".into()),
            kind: Some("bg".into()),
            ..Default::default()
        };
        let b = try_update(&env, std::time::Duration::from_secs(1), |r| apply_event(&env, r, &ev)).unwrap();
        assert_eq!(b, Binding::Bound("clear".into()));
        let r = load(&env);
        assert_eq!(r.identities.len(), 2);
        assert_eq!(r.identities[&ship.0].lineage.iter().map(|l| l.session_id.as_str()).collect::<Vec<_>>(), ["s1", "s2"]);
        assert!(r.identities.contains_key("MAIN"));
        assert!(env.registry_path().is_file() && health(&env).is_empty(), "saved: nothing to report");
        // no copy, or an empty one: a missing file is a fresh registry
        std::fs::remove_file(env.registry_path()).unwrap();
        std::fs::write(bak_path(&env), serde_json::to_string(&fresh(&env)).unwrap()).unwrap();
        assert!(load(&env).identities.is_empty());
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
        labelled_row(&env, "j1", "s2", &id);
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
        labelled_row(&env, "j1", "s1", &id);
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

    #[cfg(unix)]
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

    /// A role this nimitz does not know (a newer one wrote it) never fails
    /// the load: the record takes the role its id spells, or (an id nimitz
    /// cannot place) is set aside, never a member, with a `registry.unknown`
    /// warning; a write puts both back as they were, in their own maps.
    #[test]
    fn a_record_with_an_unknown_role_does_not_fail_the_registry_and_survives_a_write() {
        let (_td, env) = test_env();
        let rec = |role: &str, sid: &str| serde_json::json!({"role": role, "name": "n", "cwd": "/w", "session": sid, "lineage": [], "future_field": {"x": 1}});
        let raw = serde_json::json!({"version": REGISTRY_VERSION, "machine": "m", "updated_at": 1.0,
            "identities": {
                "MAIN": rec("admiral", "s-adm"),
                "MAIN/ARGOS/X": rec("future", "s-x"),
                "MAIN/@KB": rec("aide", "s-kb"),
                "MAIN/ARGOS/X/Y/@Z": rec("future", "s-z"),
            },
            "retired": {
                "MAIN/ARGOS/OLD": rec("future", "s-old"),
                "MAIN/ARGOS/X/Y/@Z": rec("future", "s-z2"),
            }});
        std::fs::create_dir_all(&env.state).unwrap();
        std::fs::write(env.registry_path(), raw.to_string()).unwrap();
        let r = load(&env);
        assert_eq!(r.identities["MAIN"].role, Role::Admiral, "the rest loads");
        assert_eq!(r.identities["MAIN/ARGOS/X"].role, Role::Ship, "the role its id spells");
        assert_eq!(r.retired["MAIN/ARGOS/OLD"].role, Role::Ship);
        assert_eq!(r.identities["MAIN/@KB"].role, Role::Aide);
        assert!(!r.identities.contains_key("MAIN/ARGOS/X/Y/@Z") && !r.retired.contains_key("MAIN/ARGOS/X/Y/@Z"));
        assert_eq!(r.unknown.records.len(), 2, "one per map, the same key in both: {:?}", r.unknown.records);
        let h = health(&env);
        let w = h.iter().find(|i| i.code == "registry.unknown").expect("a warning");
        assert!(w.severity == Severity::Warn && w.message.contains("MAIN/ARGOS/X/Y/@Z"), "{w:?}");
        assert!(!h.iter().any(|i| i.code == "registry.corrupt"), "{h:?}");
        // a write (of another identity) puts every record back as it was, in its own map
        update(&env, |r| bind(r, &Id::ship("MAIN", "ARGOS", "S"), Role::Ship, "[MAIN.ARGOS]-S", "/w", "s-s", "launch", 2.0)).unwrap();
        let back: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(env.registry_path()).unwrap()).unwrap();
        // (the role and every field this nimitz does not know, as they were)
        for (map, key) in [("identities", "MAIN/ARGOS/X"), ("identities", "MAIN/ARGOS/X/Y/@Z"), ("retired", "MAIN/ARGOS/OLD"), ("identities", "MAIN/@KB")] {
            assert_eq!(back[map][key], raw[map][key], "{map} {key}");
        }
        assert_eq!(back["retired"]["MAIN/ARGOS/X/Y/@Z"], raw["retired"]["MAIN/ARGOS/X/Y/@Z"], "an unplaceable record comes back whole");
        assert_eq!(back["identities"]["MAIN/ARGOS/X/Y/@Z"], raw["identities"]["MAIN/ARGOS/X/Y/@Z"]);
        assert!(back.get("unparsed").is_none(), "no map of its own");
        // and the next load reads the same
        let again = load(&env);
        assert_eq!((again.identities["MAIN/ARGOS/X"].role, again.unknown.records.len(), again.unknown.roles.len()), (Role::Ship, 2, 2));
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

    /// A write stamps every entry it adds with this home; a reader judges a
    /// job's home by that stamp, so a binding the other home left stays out
    /// of this home's view (and gives a start neither its cwd nor its
    /// session) once that home's job row is gone. An entry with no stamp (an
    /// older nimitz's) is judged by its row while the row exists, and is
    /// this home's once the row is gone.
    #[test]
    fn a_binding_carries_its_home_and_is_judged_by_it_without_the_row() {
        let (_td, env) = test_env();
        let id = Id::ship("MAIN", "SQ", "A");
        let home = env.home.to_string_lossy().into_owned();
        update(&env, |r| bind_job(r, &id, Role::Ship, &id.name(), "/w", Some("a1a1a1a1"), "s-mine", "launch", 1.0)).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(env.registry_path()).unwrap()).unwrap();
        assert_eq!(raw["identities"][&id.0]["lineage"][0]["home"], home.as_str(), "stamped at the write, as written");
        // the other home's hook bound the identity to its job, whose row is gone already
        let other = Env { home: PathBuf::from("/elsewhere/nimitz"), ..env.clone() };
        update(&other, |r| bind_job(r, &id, Role::Ship, &id.name(), "/elsewhere/w", Some("f0f0f0f0"), "s-theirs", "launch", 2.0)).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(env.registry_path()).unwrap()).unwrap();
        assert_eq!(raw["identities"][&id.0]["lineage"][1]["home"], "/elsewhere/nimitz");
        assert_eq!(raw["identities"][&id.0]["job"], "f0f0f0f0");
        // this home's view: its own binding, folder and session; the other's never resurfaces
        let mine = load(&env);
        let rec = &mine.identities[&id.0];
        assert_eq!((rec.job.as_deref(), rec.session.as_deref(), rec.lineage.len()), (Some("a1a1a1a1"), Some("s-mine"), 1), "{rec:?}");
        assert_eq!(identity_folder(&env, &load_checked(&env, false).unwrap().r, &id).as_deref(), None, "a foreign-bound record gives no folder of its own (the config decides)");
        // and the other home's view is its own
        let theirs = load(&other);
        assert_eq!((theirs.identities[&id.0].job.as_deref(), theirs.identities[&id.0].session.as_deref()), (Some("f0f0f0f0"), Some("s-theirs")));
        // an unstamped entry (an older nimitz's): by its row while it exists, this home's once gone
        let mut r = load_checked(&env, false).unwrap().r;
        r.identities.get_mut(&id.0).unwrap().lineage.push(LineageEntry { session_id: "s-old".into(), job: Some("0ld0ld0l".into()), via: "resume".into(), at: 3.0, home: None });
        save(&env, &r).unwrap();
        let row = env.claude_home.join("jobs").join("0ld0ld0l").join("state.json");
        std::fs::create_dir_all(row.parent().unwrap()).unwrap();
        let settings = serde_json::json!({"env": {"NIMITZ_ID": id.0, "NIMITZ_ROLE": "ship", "NIMITZ_HOME": "/elsewhere/nimitz"}}).to_string();
        std::fs::write(&row, serde_json::json!({"sessionId": "s-old", "respawnFlags": ["--settings", settings]}).to_string()).unwrap();
        assert_eq!(load(&env).identities[&id.0].lineage.len(), 1, "the row says another home");
        std::fs::remove_file(&row).unwrap();
        assert_eq!(load(&env).identities[&id.0].lineage.iter().map(|l| l.session_id.as_str()).collect::<Vec<_>>(), ["s-mine", "s-old"]);
        assert!(std::fs::read_to_string(env.registry_path()).unwrap().contains("\"s-old\""), "a read never writes");
    }

    /// A foreign job's binding is stripped from the record it touched; every
    /// other record stays byte-identical, whatever its shape (an empty
    /// lineage, a session outside the lineage as a v1-migrated current has).
    #[test]
    fn a_foreign_binding_touches_only_its_own_record() {
        let (_td, env) = test_env();
        // a job row of another home, and one of this home
        for (job, home) in [("f0f0f0f0", "/elsewhere/nimitz"), ("a1a1a1a1", env.home.to_string_lossy().as_ref())] {
            let settings = serde_json::json!({"env": {"NIMITZ_FLEET": "MAIN", "NIMITZ_SQUADRON": "SQ", "NIMITZ_ROLE": "ship", "NIMITZ_ID": "MAIN/SQ/A", "NIMITZ_HOME": home}}).to_string();
            let row = serde_json::json!({"sessionId": format!("{job}-sid"), "state": "done", "name": "[MAIN.SQ]-A", "cwd": "/w", "respawnFlags": ["--settings", settings]});
            let dir = env.claude_home.join("jobs").join(job);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("state.json"), row.to_string()).unwrap();
        }
        let entry = |sid: &str, job: Option<&str>, via: &str, at: f64| LineageEntry { session_id: sid.into(), job: job.map(str::to_string), via: via.into(), at, home: None };
        let rec = |job: Option<&str>, session: Option<&str>, lineage: Vec<LineageEntry>| IdentityRecord { role: Role::Ship, name: "[MAIN.SQ]-X".into(), cwd: "/w".into(), job: job.map(str::to_string), session: session.map(str::to_string), lineage, extra: Default::default() };
        let mut r = Registry { version: REGISTRY_VERSION, machine: "m".into(), ..Default::default() };
        // touched: this home's history, then the foreign hook's binding on top
        r.identities.insert("MAIN/SQ/A".into(), rec(Some("f0f0f0f0"), Some("f0f0f0f0-sid"), vec![entry("a1a1a1a1-sid", Some("a1a1a1a1"), "launch", 1.0), entry("f0f0f0f0-sid", Some("f0f0f0f0"), "launch", 2.0)]));
        // untouched, each shape: an empty lineage; a v1-migrated current outside
        // its lineage; a plain record; one whose job row does not exist
        r.identities.insert("MAIN/SQ/EMPTY".into(), rec(None, None, vec![]));
        r.identities.insert("MAIN/SQ/V1".into(), rec(Some("a1a1a1a1"), Some("current-not-in-lineage"), vec![entry("old-sid", None, "launch", 1.0)]));
        r.identities.insert("MAIN/SQ/PLAIN".into(), rec(Some("a1a1a1a1"), Some("a1a1a1a1-sid"), vec![entry("a1a1a1a1-sid", Some("a1a1a1a1"), "launch", 1.0)]));
        r.identities.insert("MAIN/SQ/GONE".into(), rec(Some("deadbeef"), Some("x"), vec![entry("x", Some("deadbeef"), "resume", 1.0)]));
        save(&env, &r).unwrap();
        let before: std::collections::BTreeMap<String, String> = r.identities.iter().map(|(k, v)| (k.clone(), serde_json::to_string(v).unwrap())).collect();
        let after = load(&env);
        for k in ["MAIN/SQ/EMPTY", "MAIN/SQ/V1", "MAIN/SQ/PLAIN", "MAIN/SQ/GONE"] {
            assert_eq!(serde_json::to_string(&after.identities[k]).unwrap(), before[k], "{k} changed");
        }
        let a = &after.identities["MAIN/SQ/A"];
        assert_eq!((a.job.as_deref(), a.session.as_deref(), a.lineage.len()), (Some("a1a1a1a1"), Some("a1a1a1a1-sid"), 1), "{a:?}");
        // only the foreign binding, nothing of its own left: the record goes
        let mut r2 = r.clone();
        r2.identities.insert("MAIN/SQ/A".into(), rec(Some("f0f0f0f0"), Some("f0f0f0f0-sid"), vec![entry("f0f0f0f0-sid", Some("f0f0f0f0"), "launch", 2.0)]));
        save(&env, &r2).unwrap();
        let after = load(&env);
        assert!(!after.identities.contains_key("MAIN/SQ/A"));
        assert_eq!(after.identities.len(), 4);
    }
}
