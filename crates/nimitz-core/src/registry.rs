//! Registry of identities and lineage (contracts.md section 5). Owner: core-config.
use crate::env::Env;
use crate::types::*;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

/// Current contents. A missing file is a fresh registry for this machine. A
/// file that does not parse is never replaced in silence: it is kept as
/// `registry.json.corrupt-<ts>`, the registry is rebuilt from `registry.json.bak`
/// (the last good copy) and the SessionStart labels in events.jsonl, and `health`
/// reports `registry.corrupt` until the kept file is removed. Never panics; a
/// read-only state folder leaves the bad file where it is (also reported).
pub fn load(env: &Env) -> Registry {
    load_with(env, false)
}

/// `locked`: the caller holds `registry.lock` already (flock does not nest).
fn load_with(env: &Env, locked: bool) -> Registry {
    match std::fs::read_to_string(env.registry_path()) {
        Err(_) => fresh(env),
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(r) => r,
            Err(_) if locked => recover_locked(env),
            Err(_) => match lock(env) {
                Ok(_lock) => recover_locked(env),
                Err(_) => rebuild(env).0, // read-only: rebuild in memory only
            },
        },
    }
}

fn fresh(env: &Env) -> Registry {
    Registry { version: 1, machine: env.hostname.clone(), identities: Default::default() }
}

fn parse_file(p: &Path) -> Option<Registry> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn bak_path(env: &Env) -> PathBuf {
    env.state.join("registry.json.bak")
}

/// What `recover` could rebuild from: the last good copy, then the events.
fn rebuild(env: &Env) -> (Registry, String) {
    let (mut r, mut from) = match parse_file(&bak_path(env)) {
        Some(r) => (r, vec!["registry.json.bak".to_string()]),
        None => (fresh(env), vec![]),
    };
    let mut added = 0;
    let mut events = crate::evidence::events(env);
    events.sort_by(|a, b| a.ts.partial_cmp(&b.ts).unwrap_or(std::cmp::Ordering::Equal));
    for e in events.iter().filter(|e| e.event == "SessionStart") {
        let (Some(sid), Some(nid)) = (e.session_id.as_deref(), e.labels.get("NIMITZ_ID")) else { continue };
        let id = Id(nid.clone());
        if id.fleet().is_empty() {
            continue;
        }
        let known = r.identities.get(nid).is_some_and(|rec| rec.lineage.iter().any(|l| l.session_id == sid));
        if !known {
            let via = match e.source.as_deref() {
                Some("resume") => "resume",
                Some("startup") | None => "launch",
                Some(other) => other,
            }
            .to_string();
            let (role, name) = (id.role(), id.name());
            bind(&mut r, &id, role, &name, "", sid, &via, e.ts);
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
    let (r, what) = rebuild(env);
    let ts = crate::evidence::now() as u64;
    let kept = env.state.join(format!("registry.json.corrupt-{ts}"));
    if std::fs::rename(env.registry_path(), &kept).is_err() {
        return r;
    }
    let _ = std::fs::write(note_path(&kept), &what);
    let _ = write_unlocked(env, &r);
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

/// Atomic save under an exclusive lock.
pub fn save(env: &Env, r: &Registry) -> std::io::Result<()> {
    let _lock = lock(env)?;
    write_unlocked(env, r)
}

/// Lock, load, apply `f`, save. The only way to change the registry.
pub fn update<T>(env: &Env, f: impl FnOnce(&mut Registry) -> T) -> std::io::Result<T> {
    let _lock = lock(env)?;
    let mut r = load_with(env, true);
    let out = f(&mut r);
    write_unlocked(env, &r)?;
    Ok(out)
}

/// Record a new current session for an identity (appends to lineage).
#[allow(clippy::too_many_arguments)] // one flat row of session facts
pub fn bind(r: &mut Registry, id: &Id, role: Role, name: &str, cwd: &str, session_id: &str, via: &str, now: f64) {
    let rec = r.identities.entry(id.0.clone()).or_insert_with(|| IdentityRecord {
        role,
        name: name.to_string(),
        cwd: cwd.to_string(),
        current: None,
        lineage: vec![],
    });
    rec.role = role;
    rec.name = name.to_string();
    rec.cwd = cwd.to_string();
    rec.current = Some(session_id.to_string());
    rec.lineage.push(LineageEntry { session_id: session_id.to_string(), bound_at: now, via: via.to_string() });
}

/// `registry.lock`, held exclusively until the returned File drops.
fn lock(env: &Env) -> std::io::Result<File> {
    let ctx = |e: std::io::Error| std::io::Error::new(e.kind(), format!("registry lock {}: {e}", env.registry_lock_path().display()));
    std::fs::create_dir_all(&env.state).map_err(ctx)?;
    let f = File::create(env.registry_lock_path()).map_err(ctx)?;
    let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(f) // dropping the fd releases the flock
}

/// Temp file next to registry.json, then rename (same device, atomic).
/// Every save also writes `registry.json.bak` (the last good copy, for `recover`).
fn write_unlocked(env: &Env, r: &Registry) -> std::io::Result<()> {
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
        assert_eq!(r.version, 1);
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
        assert_eq!(rec.current.as_deref(), Some("sid-2"));
        assert!(r.identities.contains_key("MAIN"));
        // the bad file is kept, the registry is good again, and check says so
        let kept: Vec<_> = std::fs::read_dir(&env.state).unwrap().flatten().filter(|e| std::fs::read(e.path()).unwrap_or_default() == b"{garbage").collect();
        assert_eq!(kept.len(), 1);
        assert!(parse_file(&env.registry_path()).is_some());
        let h = health(&env);
        assert!(h.len() == 1 && h[0].code == "registry.corrupt" && h[0].severity == Severity::Error, "{h:?}");
        assert!(h[0].message.contains("registry.json.bak") && h[0].message.contains("1 SessionStart"), "{}", h[0].message);
        assert_eq!(load(&env), r, "a second load reads the rebuilt file");
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
        assert_eq!(got, r);
        let rec = &got.identities["MAIN/ARGOS/ARGOS"];
        assert_eq!(rec.current.as_deref(), Some("sid-1"));
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
        assert_eq!(rec.current.as_deref(), Some("sid-2"));
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
