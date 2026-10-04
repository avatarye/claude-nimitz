//! `nimitz check`: integrity before every start (contracts.md section 7). Owner: core-config.
//! Session-level issues come from `crate::sessions::session_issues`.
use crate::env::Env;
use crate::types::*;
use std::path::Path;

/// Is `member` the target or one of the target's parents?
fn in_scope(member: &Id, target: Option<&Id>) -> bool {
    match target {
        None => true,
        Some(t) => t == member || t.0.starts_with(&format!("{}/", member.0)),
    }
}

/// The trust file next to Claude Code's home (`~/.claude` -> `~/.claude.json`).
fn trust_file(env: &Env) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.json", env.claude_home.display()))
}

/// Trusted = inside NIMITZ_HOME (nimitz init trusts it), or the folder (or a
/// parent) has `hasTrustDialogAccepted` in the trust file. An unreadable trust
/// file is unknown, not untrusted: `None`.
fn trusted(env: &Env, dir: &Path) -> Option<bool> {
    if dir.starts_with(&env.home) {
        return Some(true);
    }
    let raw = std::fs::read_to_string(trust_file(env)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let projects = v.get("projects")?.as_object()?;
    let accepted = |p: &Path| {
        projects
            .get(&p.to_string_lossy().into_owned())
            .and_then(|e| e.get("hasTrustDialogAccepted"))
            .and_then(|b| b.as_bool())
            .unwrap_or(false)
    };
    Some(dir.ancestors().any(|p| accepted(p)))
}

/// All issues for one identity (and its parents), or for everything when `id` is None.
pub fn check(env: &Env, snap: &Snapshot, id: Option<&Id>) -> Vec<Issue> {
    let mut out: Vec<Issue> = vec![];
    let scoped = |i: &Issue| match (&i.id, id) {
        (_, None) => true,
        (Some(iid), t) => in_scope(iid, t),
        (None, Some(t)) => snap.fleets.iter().any(|f| f.id.fleet() == t.fleet()) && i.code.starts_with("config."),
    };

    // Config and layout issues, from the files themselves.
    let (fleets, load_issues) = crate::config::load_all(env);
    out.extend(load_issues.into_iter().filter(|i| scoped(i)));
    for f in &fleets {
        if id.map(|t| t.fleet() == f.config.fleet).unwrap_or(true) {
            out.extend(crate::config::validate(env, f).into_iter().filter(|i| scoped(i)));
        }
    }

    // Member issues: folders, then the session-level rules.
    for fleet in &snap.fleets {
        let mut members: Vec<&Member> = vec![&fleet.admiral];
        for sq in &fleet.squadrons {
            members.push(&sq.commodore);
            members.extend(sq.ships.iter());
            members.extend(sq.tenders.iter());
        }
        for m in members {
            if !in_scope(&m.id, id) {
                continue;
            }
            if let Some(cwd) = &m.cwd {
                let dir = Path::new(cwd);
                if !dir.is_dir() {
                    out.push(Issue {
                        code: "folder.missing".into(),
                        severity: Severity::Error,
                        message: format!("{}'s folder {} does not exist", m.id, cwd),
                        fix: Some(format!("mkdir -p {cwd}  # or point {} elsewhere with nimitz ship set", m.id)),
                        id: Some(m.id.clone()),
                    });
                } else if trusted(env, dir) == Some(false) {
                    out.push(Issue {
                        code: "folder.untrusted".into(),
                        severity: Severity::Error,
                        message: format!("{}'s folder {} is not trusted by Claude Code", m.id, cwd),
                        fix: Some(format!("run `claude` once in {cwd} and accept the trust prompt")),
                        id: Some(m.id.clone()),
                    });
                }
            }
            out.extend(m.issues.iter().filter(|i| scoped(i)).cloned());
            // Session rules (name.*, labels.*, dup.live, copy.rows, lineage.broken,
            // role.background, ship.elsewhere) need a session or job rows to judge.
            if m.session.is_some() || !m.extra_rows.is_empty() {
                out.extend(crate::sessions::session_issues(env, m));
            }
        }
        out.extend(fleet.issues.iter().filter(|i| scoped(i)).cloned());
        for sq in &fleet.squadrons {
            out.extend(sq.issues.iter().filter(|i| scoped(i)).cloned());
        }
    }

    // Claude Code version against the compiled range (launches refuse outside it).
    if !snap.compat_ok {
        let c = crate::compat::supported();
        let (sev, what) = match snap.cc_version.as_deref().map(|v| crate::compat::position(&c, v)) {
            Some(1) => (Severity::Warn, "above".to_string()),
            Some(-1) => (Severity::Error, "below".to_string()),
            _ => (Severity::Error, "unknown against".to_string()),
        };
        out.push(Issue {
            code: "cc.version".into(),
            severity: sev,
            message: format!(
                "Claude Code {} is {what} the supported range {} - {}",
                snap.cc_version.as_deref().unwrap_or("(not found)"),
                c.min,
                c.max
            ),
            fix: Some("read-only commands still work; `--force` overrides launches".into()),
            id: None,
        });
    }

    // Stowaways only matter for the whole-world check.
    if id.is_none() {
        for s in &snap.stowaways {
            out.push(Issue {
                code: "stowaway".into(),
                severity: Severity::Warn,
                message: format!("session {} ({}) sits in fleet scope with no labels", s.session_id, s.name),
                fix: Some(format!("nimitz adopt {} <id>  # or nimitz rm {}", s.session_id, s.session_id)),
                id: None,
            });
        }
        out.extend(snap.issues.iter().cloned());
    }

    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::{make_fleet, test_env};

    fn member(id: Id, cwd: Option<&str>) -> Member {
        Member {
            role: id.role(),
            expected_name: id.name(),
            id,
            cwd: cwd.map(|s| s.to_string()),
            status: Status::NotStarted,
            reasons: vec![],
            session: None, // keep session_issues (a core-sessions stub) out of these tests
            extra_rows: vec![],
            crew: vec![],
            tasks: vec![],
            issues: vec![],
            pane: None,
        }
    }

    fn snap_for(env: &Env, ship_cwd: Option<&str>) -> Snapshot {
        let f = crate::config::load_fleet(env, "MAIN").unwrap();
        let admiral_dir = f.dir.join("admiral");
        let sq_dir = f.squadrons[0].0.clone();
        Snapshot {
            taken_at: 0.0,
            machine: Some("test-mac".into()),
            fleets: vec![Fleet {
                id: Id::admiral("MAIN"),
                dir: f.dir.to_string_lossy().into_owned(),
                machine: Some("test-mac".into()),
                admiral: member(Id::admiral("MAIN"), admiral_dir.to_str()),
                squadrons: vec![Squadron {
                    id: Id::commodore("MAIN", "ARGOS"),
                    commodore: member(Id::commodore("MAIN", "ARGOS"), sq_dir.to_str()),
                    ships: vec![member(Id::ship("MAIN", "ARGOS", "ARGOS"), ship_cwd)],
                    tenders: vec![],
                    comms: Comms::default(),
                    issues: vec![],
                }],
                issues: vec![],
            }],
            stowaways: vec![],
            issues: vec![],
            cc_version: Some("2.1.288".into()),
            compat_ok: true,
        }
    }

    #[test]
    fn clean_world_has_no_issues() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let f = crate::config::load_fleet(&env, "MAIN").unwrap();
        let ship_dir = f.dir.join("ships/argos/argos");
        let snap = snap_for(&env, ship_dir.to_str());
        let issues = check(&env, &snap, None);
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn missing_folder_is_flagged_in_scope_only() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let snap = snap_for(&env, Some("/no/such/dir"));
        let ship = Id::ship("MAIN", "ARGOS", "ARGOS");
        let issues = check(&env, &snap, Some(&ship));
        assert!(issues.iter().any(|i| i.code == "folder.missing" && i.id.as_ref() == Some(&ship)), "{issues:?}");
        // The commodore's own check does not carry the ship's folder issue.
        let issues = check(&env, &snap, Some(&Id::commodore("MAIN", "ARGOS")));
        assert!(!issues.iter().any(|i| i.code == "folder.missing"), "{issues:?}");
        // But the ship's check walks its parents (clean here).
        let issues = check(&env, &snap, Some(&ship));
        assert_eq!(issues.len(), 1, "{issues:?}");
    }

    #[test]
    fn untrusted_folder_is_flagged_when_the_trust_file_says_so() {
        let (td, env) = test_env();
        make_fleet(&env);
        let outside = td.path().join("project");
        std::fs::create_dir_all(&outside).unwrap();
        let trust = format!("{}.json", env.claude_home.display());
        // No trust file: unknown, no issue.
        let snap = snap_for(&env, outside.to_str());
        assert!(check(&env, &snap, None).is_empty());
        // Trust file without the folder: untrusted.
        std::fs::write(&trust, r#"{"projects": {}}"#).unwrap();
        let issues = check(&env, &snap, None);
        assert!(issues.iter().any(|i| i.code == "folder.untrusted"), "{issues:?}");
        // Folder (or a parent) accepted: trusted. Managed folders are always trusted.
        std::fs::write(
            &trust,
            format!(r#"{{"projects": {{"{}": {{"hasTrustDialogAccepted": true}}}}}}"#, outside.display()),
        )
        .unwrap();
        assert!(check(&env, &snap, None).is_empty());
        let f = crate::config::load_fleet(&env, "MAIN").unwrap();
        let managed = f.dir.join("ships/argos/argos");
        std::fs::write(&trust, r#"{"projects": {}}"#).unwrap();
        assert!(check(&env, &snap_for(&env, managed.to_str()), None).is_empty());
    }

    #[test]
    fn config_issues_flow_through() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let f = crate::config::load_fleet(&env, "MAIN").unwrap();
        std::fs::write(f.dir.join("CLAUDE.md"), "x").unwrap();
        let ship_dir = f.dir.join("ships/argos/argos");
        let snap = snap_for(&env, ship_dir.to_str());
        let issues = check(&env, &snap, None);
        assert!(issues.iter().any(|i| i.code == "config.layout"), "{issues:?}");
        // In scope for members of that fleet too.
        let issues = check(&env, &snap, Some(&Id::admiral("MAIN")));
        assert!(issues.iter().any(|i| i.code == "config.layout"), "{issues:?}");
    }

    #[test]
    fn version_and_stowaways() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let f = crate::config::load_fleet(&env, "MAIN").unwrap();
        let ship_dir = f.dir.join("ships/argos/argos");
        let mut snap = snap_for(&env, ship_dir.to_str());
        snap.cc_version = Some("2.1.290".into());
        snap.compat_ok = false;
        snap.stowaways.push(SessionInfo {
            session_id: "deadbeef".into(),
            short_id: None,
            pid: None,
            kind: "background".into(),
            state: None,
            status: None,
            waiting_for: None,
            started_at: None,
            status_since: None,
            name: "mystery".into(),
            cwd: "/x".into(),
            labels: Default::default(),
        });
        let issues = check(&env, &snap, None);
        let v = issues.iter().find(|i| i.code == "cc.version").expect("cc.version");
        assert_eq!(v.severity, Severity::Warn); // above the range warns
        assert!(issues.iter().any(|i| i.code == "stowaway"));
        // Scoped checks still report the version, but not stowaways.
        snap.cc_version = Some("2.1.100".into());
        let issues = check(&env, &snap, Some(&Id::admiral("MAIN")));
        let v = issues.iter().find(|i| i.code == "cc.version").expect("cc.version");
        assert_eq!(v.severity, Severity::Error); // below the range refuses
        assert!(!issues.iter().any(|i| i.code == "stowaway"));
    }
}
