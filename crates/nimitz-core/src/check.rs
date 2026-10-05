//! `nimitz check`: integrity before every start (contracts.md section 7). Owner: core-sessions (was core-config).
//! Session-level issues ride in on `Member::issues` (snapshot fills them).
use crate::env::Env;
use crate::types::*;
use std::path::{Path, PathBuf};

/// Is `member` the target or one of the target's parents?
fn in_scope(member: &Id, target: Option<&Id>) -> bool {
    match target {
        None => true,
        Some(t) => t == member || t.0.starts_with(&format!("{}/", member.0)),
    }
}

/// Is `member` under the target (a ship or tender of a squadron, anything of a fleet)?
fn below(member: &Id, target: Option<&Id>) -> bool {
    target.is_some_and(|t| member.0.starts_with(&format!("{}/", t.0)))
}

/// The trust file next to Claude Code's home (`~/.claude` -> `~/.claude.json`).
fn trust_file(env: &Env) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.json", env.claude_home.display()))
}

/// The `projects` table of Claude Code's trust file, or None when it cannot be read.
fn trust_projects(env: &Env) -> Option<serde_json::Map<String, serde_json::Value>> {
    let raw = std::fs::read_to_string(trust_file(env)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("projects")?.as_object().cloned()
}

/// Has Claude Code already run a session in exactly this folder? A transcript
/// folder for it under `projects/`, or a job row or session file with that cwd.
/// Then it accepted the folder, whatever the trust file says.
pub fn ran_here(env: &Env, dir: &Path) -> bool {
    let cwd = dir.to_string_lossy();
    if !crate::evidence::transcripts_in(env, &cwd).is_empty() {
        return true;
    }
    let want = [dir.to_path_buf(), crate::env::canon(dir)];
    let cwd_of = |p: PathBuf| -> Option<PathBuf> {
        let v = crate::evidence::first_json(&std::fs::read(p).ok()?)?;
        Some(PathBuf::from(v.get("cwd")?.as_str()?.trim_end_matches('/')))
    };
    let rows = |sub: &str, file: Option<&str>| -> bool {
        std::fs::read_dir(env.claude_home.join(sub))
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| match file {
                        Some(f) => e.path().join(f),
                        None => e.path(),
                    })
                    .filter_map(cwd_of)
                    .any(|c| want.contains(&c))
            })
            .unwrap_or(false)
    };
    rows("jobs", Some("state.json")) || rows("sessions", None)
}

/// How Claude Code will treat `dir` (BUG-50; evidence over prediction):
/// - trusted: inside NIMITZ_HOME (nimitz init trusts it); a trust entry on the
///   folder's repository itself (git root or below); Claude Code ran a session
///   in this exact folder; or, outside git, the folder or a parent accepted.
/// - a git repository or worktree covered only by a parent's trust: Warn, it
///   may not be trusted (real Claude Code refused both, with no session there).
/// - nothing accepts it: Error.
///
/// Keys are compared as given and as real paths (/tmp is /private/tmp). An
/// unreadable trust file is unknown, not untrusted: no issue.
fn trust_issue(env: &Env, id: &Id, dir: &Path) -> Option<Issue> {
    let real = crate::env::canon(dir);
    if dir.starts_with(&env.home) || real.starts_with(crate::env::canon(&env.home)) {
        return None;
    }
    let projects = trust_projects(env)?;
    let accepted = |p: &PathBuf| {
        let k = p.to_string_lossy();
        projects
            .get(k.as_ref())
            .or_else(|| projects.get(&format!("{k}/")))
            .and_then(|e| e.get("hasTrustDialogAccepted"))
            .and_then(|b| b.as_bool())
            .unwrap_or(false)
    };
    if crate::env::trust_keys(dir).iter().any(accepted) || ran_here(env, dir) {
        return None;
    }
    let by_parent = crate::env::trust_ancestors(dir).into_iter().find(|a| accepted(a));
    let root = crate::env::git_root(dir);
    let mk = |severity: Severity, message: String, at: &Path| Issue {
        code: "folder.untrusted".into(),
        severity,
        message,
        fix: Some(untrusted_fix(at)),
        id: Some(id.clone()),
    };
    match (root, by_parent) {
        (Some(root), Some(parent)) => {
            let kind = if crate::env::is_git_worktree(&root) { "git worktree" } else { "git repository" };
            Some(mk(
                Severity::Warn,
                format!(
                    "{id}'s folder {} may not be trusted ({kind} {}): only the parent {} is trusted, and Claude Code wants the repository's own trust",
                    real.display(),
                    root.display(),
                    parent.display()
                ),
                &root,
            ))
        }
        (_, Some(_)) => None,
        (root, None) => {
            let at = root.clone().unwrap_or_else(|| real.clone());
            let why = root.map(|r| format!(" (git repository {})", r.display())).unwrap_or_default();
            Some(mk(Severity::Error, format!("{id}'s folder {} is not trusted by Claude Code{why}", real.display()), &at))
        }
    }
}

/// The fix for an untrusted folder: Claude Code records the real path, and
/// inside a git repository asks about its root. Also used when a launch fails
/// with "Workspace not trusted".
pub fn untrusted_fix(dir: &Path) -> String {
    let at = crate::env::git_root(dir).unwrap_or_else(|| crate::env::canon(dir));
    format!("run `claude` once in {} and accept the trust prompt", at.display())
}

/// `folder.missing` / `folder.untrusted` for a member's folder, if either applies.
pub fn folder_issue(env: &Env, id: &Id, cwd: &str) -> Option<Issue> {
    let dir = Path::new(cwd);
    if !dir.is_dir() {
        return Some(Issue {
            code: "folder.missing".into(),
            severity: Severity::Error,
            message: format!("{id}'s folder {cwd} does not exist"),
            fix: Some(format!("mkdir -p {cwd}  # or point {id} elsewhere with nimitz ship set")),
            id: Some(id.clone()),
        });
    }
    trust_issue(env, id, dir)
}

/// `nimitz check [<id>]`: the target, every member under it (a fleet's
/// squadrons and ships, a squadron's ships and tenders), and its parents.
pub fn check_tree(env: &Env, snap: &Snapshot, id: Option<&Id>) -> Vec<Issue> {
    check_scoped(env, snap, id, true)
}

/// All issues for one identity and its parents (the pre-start gate), or for
/// everything when `id` is None.
pub fn check(env: &Env, snap: &Snapshot, id: Option<&Id>) -> Vec<Issue> {
    check_scoped(env, snap, id, false)
}

fn check_scoped(env: &Env, snap: &Snapshot, id: Option<&Id>, tree: bool) -> Vec<Issue> {
    let in_scope = |m: &Id, t: Option<&Id>| in_scope(m, t) || (tree && below(m, t));
    let mut out: Vec<Issue> = vec![];
    let scoped = |i: &Issue| match (&i.id, id) {
        (_, None) => true,
        (Some(iid), t) => in_scope(iid, t),
        (None, Some(t)) => snap.fleets.iter().any(|f| f.id.fleet() == t.fleet()) && i.code.starts_with("config."),
    };

    // Config and layout issues, from the files themselves.
    let (fleets, load_issues) = crate::config::load_all(env);
    out.extend(load_issues.into_iter().filter(|i| scoped(i)));
    if id.is_none() || fleets.iter().any(|f| id.is_some_and(|t| t.fleet() == f.config.fleet)) {
        out.extend(crate::config::home_layout(env));
    }
    out.extend(crate::registry::health(env));
    // needs-you notices that never reached the commodore
    out.extend(crate::notify::issues(env).into_iter().filter(|i| scoped(i)));
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
            if let Some(i) = m.cwd.as_deref().and_then(|cwd| folder_issue(env, &m.id, cwd)) {
                out.push(i);
            }
            // Session rules (name.*, labels.*, dup.live, copy.rows, lineage.broken,
            // role.background, ship.elsewhere) arrive here too: snapshot puts
            // sessions::session_issues into Member::issues.
            out.extend(m.issues.iter().filter(|i| scoped(i)).cloned());
        }
        out.extend(fleet.issues.iter().filter(|i| scoped(i)).cloned());
        for sq in &fleet.squadrons {
            out.extend(sq.issues.iter().filter(|i| scoped(i)).cloned());
        }
    }

    // Claude Code version against the compiled range (launches refuse outside it).
    if !snap.compat_ok {
        let c = crate::compat::supported();
        let range = format!("the supported range {} - {}", c.min, c.max);
        let (sev, message) = match snap.cc_version.as_deref() {
            None => (Severity::Error, format!("Claude Code's version could not be read (`{} --version` gave none); {range}", env.native_claude_bin)),
            Some(v) if v.contains('-') => (Severity::Error, format!("Claude Code {v} is a pre-release that compat.toml does not list; {range}")),
            Some(v) => match crate::compat::position(&c, v) {
                1 => (Severity::Warn, format!("Claude Code {v} is above {range}")),
                -1 => (Severity::Error, format!("Claude Code {v} is below {range}")),
                _ => (Severity::Error, format!("Claude Code version {v:?} is not a version nimitz can compare with {range}")),
            },
        };
        out.push(Issue {
            code: "cc.version".into(),
            severity: sev,
            message,
            fix: Some("read-only commands still work; `--force` overrides launches".into()),
            id: None,
        });
    }

    // Stowaways: the whole-world check lists them all; `check_tree` of a fleet,
    // squadron or ship lists those in its folders (a member's folder itself, or
    // anywhere under the fleet's / squadron's nimitz folders).
    let stowaway_scope: Option<(Vec<PathBuf>, Vec<PathBuf>)> = match id {
        None => None,
        Some(_) if !tree => Some((vec![], vec![])),
        Some(t) => {
            let mut exact: Vec<PathBuf> = vec![];
            let mut trees: Vec<PathBuf> = vec![];
            for f in fleets.iter().filter(|f| f.config.fleet == t.fleet()) {
                match t.role() {
                    Role::Admiral => trees.push(f.dir.clone()),
                    Role::Commodore => {
                        if let Some(sd) = crate::config::member_dir(env, f, t) {
                            trees.push(sd);
                        }
                        trees.push(f.dir.join("ships").join(t.squadron().unwrap_or_default().to_lowercase()));
                    }
                    _ => {}
                }
            }
            for fl in &snap.fleets {
                let mut ms = vec![&fl.admiral];
                for sq in &fl.squadrons {
                    ms.push(&sq.commodore);
                    ms.extend(sq.ships.iter().chain(sq.tenders.iter()));
                }
                exact.extend(ms.into_iter().filter(|m| m.id == *t || below(&m.id, Some(t))).filter_map(|m| m.cwd.as_deref()).map(|c| crate::env::canon(Path::new(c))));
            }
            Some((exact, trees.iter().map(|d| crate::env::canon(d)).collect()))
        }
    };
    let stowaway_in_scope = |cwd: &str| match &stowaway_scope {
        None => true,
        Some((exact, trees)) => {
            let c = crate::env::canon(Path::new(cwd));
            exact.contains(&c) || trees.iter().any(|t| c.starts_with(t))
        }
    };
    {
        // A stowaway named in a legacy map is an old fleet's session: its member
        // carries the `legacy` issue instead.
        let legacy: std::collections::BTreeSet<String> = fleets.iter().flat_map(|f| crate::config::legacy_map(f).into_keys()).collect();
        for s in snap.stowaways.iter().filter(|s| !legacy.contains(&s.name) && stowaway_in_scope(&s.cwd)) {
            let short = s.short_id.as_deref().unwrap_or(&s.session_id);
            // never offer rm for a live session (contracts.md section 7)
            let fix = if s.pid.is_some() {
                format!("nimitz stop {short}, then nimitz rm {short}  # or, if it holds work to keep: nimitz adopt {} <id>", s.session_id)
            } else {
                format!("nimitz rm {short}  # or, if it holds work to keep: nimitz adopt {} <id>", s.session_id)
            };
            out.push(Issue {
                code: "stowaway".into(),
                severity: Severity::Warn,
                message: format!("session {} ({}) sits in fleet scope with no labels{}", s.session_id, s.name, if s.pid.is_some() { " (live)" } else { "" }),
                fix: Some(fix),
                id: None,
            });
        }
    }
    if id.is_none() {
        out.extend(snap.issues.iter().cloned());
    }

    out.dedup();
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::tests::{make_fleet, test_env};

    pub(crate) fn live_session(name: &str) -> SessionInfo {
        SessionInfo {
            session_id: "11111111-aaaa".into(),
            short_id: Some("11111111".into()),
            pid: Some(1),
            kind: "interactive".into(),
            state: None,
            status: None,
            waiting_for: None,
            started_at: None,
            status_since: None,
            name: name.into(),
            cwd: "/x".into(),
            labels: Default::default(),
        }
    }

    pub(crate) fn member(id: Id, cwd: Option<&str>) -> Member {
        Member {
            role: id.role(),
            expected_name: id.name(),
            id,
            cwd: cwd.map(|s| s.to_string()),
            status: Status::NotStarted,
            reasons: vec![],
            session: None,
            extra_rows: vec![],
            crew: vec![],
            tasks: vec![],
            issues: vec![],
            pane: None,
        }
    }

    pub(crate) fn snap_for(env: &Env, ship_cwd: Option<&str>) -> Snapshot {
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
    fn check_tree_covers_members_below_the_target() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let snap = snap_for(&env, Some("/no/such/dir"));
        for t in [Id::admiral("MAIN"), Id::commodore("MAIN", "ARGOS")] {
            let issues = check_tree(&env, &snap, Some(&t));
            assert!(issues.iter().any(|i| i.code == "folder.missing"), "{t}: {issues:?}");
            assert!(!check(&env, &snap, Some(&t)).iter().any(|i| i.code == "folder.missing"), "the gate stays chain-only");
        }
        let other = Id::commodore("MAIN", "OTHER");
        assert!(check_tree(&env, &snap, Some(&other)).is_empty());
    }

    #[test]
    fn a_git_repo_under_a_trusted_home_needs_its_own_trust() {
        let (td, env) = test_env();
        make_fleet(&env);
        let home = td.path().canonicalize().unwrap().join("home");
        let wt = home.join(".cache/wt");
        std::fs::create_dir_all(wt.join("src")).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: /elsewhere/.git/worktrees/wt\n").unwrap();
        let trust = format!("{}.json", env.claude_home.display());
        let set = |dirs: &[&Path]| {
            let p: serde_json::Map<String, serde_json::Value> = dirs.iter().map(|d| (d.display().to_string(), serde_json::json!({"hasTrustDialogAccepted": true}))).collect();
            std::fs::write(&trust, serde_json::json!({ "projects": p }).to_string()).unwrap();
        };
        let id = Id::ship("MAIN", "ARGOS", "S4");
        set(&[&home]);
        let i = folder_issue(&env, &id, wt.join("src").to_str().unwrap()).expect("home trust does not cover a git root");
        assert_eq!(i.code, "folder.untrusted");
        assert_eq!(i.fix.as_deref(), Some(format!("run `claude` once in {} and accept the trust prompt", wt.display()).as_str()));
        set(&[&wt]);
        assert!(folder_issue(&env, &id, wt.join("src").to_str().unwrap()).is_none());
        // outside git, the home's trust still covers it
        std::fs::create_dir_all(home.join("plain")).unwrap();
        set(&[&home]);
        assert!(folder_issue(&env, &id, home.join("plain").to_str().unwrap()).is_none());
    }

    #[test]
    fn a_symlink_to_a_trusted_folder_is_trusted() {
        let (td, env) = test_env();
        make_fleet(&env);
        let real = td.path().canonicalize().unwrap().join("project");
        std::fs::create_dir_all(&real).unwrap();
        let link = td.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let trust = format!("{}.json", env.claude_home.display());
        std::fs::write(&trust, format!(r#"{{"projects": {{"{}": {{"hasTrustDialogAccepted": true}}}}}}"#, real.display())).unwrap();
        assert!(folder_issue(&env, &Id::admiral("MAIN"), link.to_str().unwrap()).is_none());
        // untrusted: the fix names the real path, the one Claude Code records
        std::fs::write(&trust, r#"{"projects": {}}"#).unwrap();
        let i = folder_issue(&env, &Id::admiral("MAIN"), link.to_str().unwrap()).unwrap();
        assert!(i.fix.unwrap().contains(real.to_str().unwrap()));
    }

    #[test]
    fn check_tree_lists_stowaways_in_its_folders_only() {
        let (td, env) = test_env();
        make_fleet(&env);
        let f = crate::config::load_fleet(&env, "MAIN").unwrap();
        let ship_dir = f.dir.join("ships/argos/argos");
        std::fs::create_dir_all(&ship_dir).unwrap();
        let mut snap = snap_for(&env, ship_dir.to_str());
        let mut st = live_session("stray");
        st.pid = None;
        st.cwd = ship_dir.to_string_lossy().into_owned();
        snap.stowaways.push(st.clone());
        let mut far = st.clone();
        far.session_id = "22222222-bbbb".into();
        far.cwd = td.path().join("elsewhere").to_string_lossy().into_owned();
        snap.stowaways.push(far);
        let count = |v: Vec<Issue>| v.iter().filter(|i| i.code == "stowaway").count();
        assert_eq!(count(check_tree(&env, &snap, Some(&Id::admiral("MAIN")))), 1);
        assert_eq!(count(check_tree(&env, &snap, Some(&Id::commodore("MAIN", "ARGOS")))), 1);
        assert_eq!(count(check_tree(&env, &snap, Some(&Id::ship("MAIN", "ARGOS", "ARGOS")))), 1);
        assert_eq!(count(check_tree(&env, &snap, Some(&Id::commodore("MAIN", "OTHER")))), 0);
        assert_eq!(count(check(&env, &snap, Some(&Id::admiral("MAIN")))), 0, "the gate does not list stowaways");
        assert_eq!(count(check(&env, &snap, None)), 2);
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
