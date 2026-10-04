//! Registry of identities and lineage (contracts.md section 5). Owner: core-config.
use crate::env::Env;
use crate::types::*;
use std::fs::File;
use std::os::fd::AsRawFd;

/// Current contents, tolerant: missing or unreadable file = a fresh registry
/// for this machine. Never panics on bad input.
pub fn load(env: &Env) -> Registry {
    let fresh = || Registry { version: 1, machine: env.hostname.clone(), identities: Default::default() };
    let raw = match std::fs::read_to_string(env.registry_path()) {
        Ok(r) => r,
        Err(_) => return fresh(),
    };
    serde_json::from_str(&raw).unwrap_or_else(|_| fresh())
}

/// Atomic save under an exclusive lock.
pub fn save(env: &Env, r: &Registry) -> std::io::Result<()> {
    let _lock = lock(env)?;
    write_unlocked(env, r)
}

/// Lock, load, apply `f`, save. The only way to change the registry.
pub fn update<T>(env: &Env, f: impl FnOnce(&mut Registry) -> T) -> std::io::Result<T> {
    let _lock = lock(env)?;
    let mut r = load(env);
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
    std::fs::create_dir_all(&env.state)?;
    let f = File::create(env.registry_lock_path())?;
    let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(f) // dropping the fd releases the flock
}

/// Temp file next to registry.json, then rename (same device, atomic).
fn write_unlocked(env: &Env, r: &Registry) -> std::io::Result<()> {
    let contents = serde_json::to_string_pretty(r).map_err(std::io::Error::other)?;
    crate::config::atomic_write(&env.registry_path(), &contents)
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
