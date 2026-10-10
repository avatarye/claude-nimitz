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

/// Is `member` under the target (a ship of a squadron, anything of a fleet)?
fn below(member: &Id, target: Option<&Id>) -> bool {
    target.is_some_and(|t| member.0.starts_with(&format!("{}/", t.0)))
}

/// The trust file next to Claude Code's home (`~/.claude` -> `~/.claude.json`).
pub(crate) fn trust_file(env: &Env) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.json", env.claude_home.display()))
}

/// What one check run reads once for every member's folder: the trust
/// files' `projects` tables and the trust roots (`evidence::folder_trusted`).
/// A run sees the files as they were when it first read them.
#[derive(Default)]
pub struct TrustCache {
    files: crate::evidence::FileCache,
    projects: Option<Option<serde_json::Map<String, serde_json::Value>>>,
}

/// The `projects` tables of Claude Code's config files (`trust::files`, the
/// set `evidence::folder_trusted` reads), merged: a key is accepted when any
/// file accepts it. None when no file can be read.
fn trust_projects(env: &Env) -> Option<serde_json::Map<String, serde_json::Value>> {
    let accepted = |e: &serde_json::Value| e.get("hasTrustDialogAccepted").and_then(|b| b.as_bool()) == Some(true);
    let mut out: Option<serde_json::Map<String, serde_json::Value>> = None;
    for f in crate::trust::files(env) {
        let Some(projects) = std::fs::read_to_string(f).ok().and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok()).and_then(|v| v.get("projects")?.as_object().cloned()) else {
            continue;
        };
        let all = out.get_or_insert_with(Default::default);
        for (k, e) in projects {
            if !all.get(&k).is_some_and(accepted) {
                all.insert(k, e);
            }
        }
    }
    out
}

/// Has Claude Code already started a session in exactly this folder (a
/// transcript under its `projects/` dir, or a job row or session file with
/// that cwd)? Then it accepted the folder, whatever the trust file says. The
/// time is the newest start (transcript birth, job row `createdAt`, live
/// session file `startedAt`): a start is the moment Claude Code accepts a
/// folder, so later writes do not count. `Some(None)`: it ran here, but one
/// piece of evidence has no stamp, so when is not known.
pub fn last_ran_here(env: &Env, dir: &Path) -> Option<Option<f64>> {
    let cwd = dir.to_string_lossy();
    let mut starts: Vec<Option<f64>> = crate::evidence::transcript_files_in(env, &cwd).iter().map(|p| crate::evidence::birth_secs(p)).collect();
    let want = [dir.to_path_buf(), crate::env::canon(dir)];
    let mut rows = |sub: &str, file: Option<&str>, stamp: &str| {
        for e in std::fs::read_dir(env.claude_home.join(sub)).into_iter().flatten().flatten() {
            let p = match file {
                Some(f) => e.path().join(f),
                None => e.path(),
            };
            let Some(v) = std::fs::read(p).ok().and_then(|b| crate::evidence::first_json(&b)) else { continue };
            let Some(c) = v.get("cwd").and_then(|c| c.as_str()).map(|c| PathBuf::from(c.trim_end_matches('/'))) else { continue };
            if want.contains(&c) {
                starts.push(crate::evidence::stamp(&v, stamp));
            }
        }
    };
    rows("jobs", Some("state.json"), "createdAt");
    rows("sessions", None, "startedAt");
    if starts.is_empty() {
        return None;
    }
    Some(starts.iter().try_fold(f64::MIN, |a, t| Some(a.max((*t)?))))
}

/// Did Claude Code start a session in `dir` that proves it accepts the folder
/// as it is now? Outside git, any run. Inside a repository, a start since the
/// repository appeared: a folder that became a git root after its sessions ran
/// loses the parent's trust, and Claude Code asks again (M16, cli-G6). When a
/// time is not known (a file system with no birth times), any run counts, as
/// before.
fn ran_since_repo(env: &Env, dir: &Path) -> bool {
    let Some(last) = last_ran_here(env, dir) else { return false };
    match (crate::env::git_root(dir).and_then(|r| crate::env::git_birth(&r)), last) {
        (Some(birth), Some(t)) => t >= birth,
        _ => true,
    }
}

/// How Claude Code will treat `dir` (BUG-50; evidence over prediction):
/// - trusted: inside NIMITZ_HOME when the data root is trusted (nimitz init
///   asks for it; the real path only, and outside any repository other than
///   the data root's own: Claude Code wants a repository's own trust, and
///   resolves a link out of the data root first); a trust entry on the
///   folder's repository itself (git root or below); Claude Code ran a session
///   in this exact folder (inside a repository: since the repository appeared,
///   M16); or, outside git, the folder or a parent accepted.
/// - a git repository or worktree covered only by a parent's trust: Warn, it
///   may not be trusted (real Claude Code refused both, with no session there).
/// - nothing accepts it: Error.
///
/// Keys are compared as given and as real paths (/tmp is /private/tmp). With
/// no config file readable, trust is unknown, not untrusted: no issue.
fn trust_issue(env: &Env, cache: &mut TrustCache, id: &Id, dir: &Path) -> Option<Issue> {
    let real = crate::env::canon(dir);
    let home = crate::env::canon(&env.home);
    // inside NIMITZ_HOME: trusted when the data root is (nimitz init asks for
    // it); when it is not, the fix names the data root, which covers them all.
    // The real path (a link out of the data root is judged where it leads),
    // and not inside a repository of its own (`trust::target` agrees).
    let in_home = real.starts_with(&home) && crate::env::git_root(&real).is_none_or(|r| r == home);
    if in_home && crate::evidence::folder_trusted(env, &mut cache.files, &home.to_string_lossy()) {
        return None;
    }
    let projects = cache.projects.get_or_insert_with(|| trust_projects(env)).as_ref()?;
    // (keys compared in one form: Claude Code on Windows writes `C:\x` and `C:/x`)
    // (one key accepted is enough when two forms name the same folder)
    let mut by_key: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    for (k, e) in projects {
        *by_key.entry(crate::env::trust_key(k)).or_default() |= e.get("hasTrustDialogAccepted").and_then(|b| b.as_bool()).unwrap_or(false);
    }
    let accepted = |p: &PathBuf| by_key.get(&crate::env::trust_key(&p.to_string_lossy())).copied().unwrap_or(false);
    if crate::env::trust_keys(dir).iter().any(accepted) || ran_since_repo(env, dir) {
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
            let at = if in_home && root.is_none() { home.clone() } else { root.clone().unwrap_or_else(|| real.clone()) };
            let why = root.map(|r| format!(" (git repository {})", r.display())).unwrap_or_default();
            let twin = case_twin(&projects, &crate::env::trust_ancestors(dir));
            let note = twin.map(|k| format!("; it is trusted as `{k}`, but Claude Code compares the folder's case and looks it up as it is on disk")).unwrap_or_default();
            Some(mk(Severity::Error, format!("{id}'s folder {} is not trusted by Claude Code{why}{note}", real.display()), &at))
        }
    }
}

/// An accepted trust key that names one of `keys` but for case (Windows:
/// `c:/x` for `C:\x`), which Claude Code does not count.
fn case_twin(projects: &serde_json::Map<String, serde_json::Value>, keys: &[PathBuf]) -> Option<String> {
    let want: Vec<String> = keys.iter().map(|k| crate::env::trust_key(&k.to_string_lossy())).collect();
    projects
        .iter()
        .filter(|(_, e)| e.get("hasTrustDialogAccepted").and_then(|b| b.as_bool()) == Some(true))
        .map(|(k, _)| k)
        .find(|k| {
            let t = crate::env::trust_key(k);
            want.iter().any(|w| *w != t && w.eq_ignore_ascii_case(&t))
        })
        .cloned()
}

/// The fix for an untrusted folder: Claude Code records the real path, and
/// inside a git repository asks about its root. Also used when a launch fails
/// with "Workspace not trusted".
pub fn untrusted_fix(dir: &Path) -> String {
    let at = crate::env::git_root(dir).unwrap_or_else(|| crate::env::canon(dir));
    crate::trust::fix(&at)
}

/// `trust_issue`'s verdict for a folder alone (no member): Error, Warn or
/// trusted (None). For `trust::untrusted`.
pub(crate) fn trust_verdict(env: &Env, dir: &Path) -> Option<Severity> {
    // a fresh read: a trust written a moment ago counts
    trust_issue(env, &mut TrustCache::default(), &Id(String::new()), dir).map(|i| i.severity)
}

/// Windows: how to start `claude` in `dir` spelled exactly as on disk.
/// Claude Code stores the trust under the folder as the shell spelled it, and
/// a cmd.exe with clink/zoxide hands it `c:\...`; a background launch looks
/// it up as `C:\...`, with case. PowerShell's Set-Location keeps the case.
/// (`nimitz trust` writes the on-disk form itself; this is for a fix that
/// still runs `claude` by hand.)
pub fn case_exact_hint(dir: &Path) -> String {
    if cfg!(windows) {
        format!(" (spelled exactly, from PowerShell: Set-Location -LiteralPath '{}'; claude)", dir.display())
    } else {
        String::new()
    }
}

/// `folder.missing` / `folder.untrusted` for a member's folder, if either applies.
pub fn folder_issue(env: &Env, id: &Id, cwd: &str) -> Option<Issue> {
    folder_issue_in(env, &mut TrustCache::default(), id, cwd)
}

/// `folder_issue` with the trust files read once per run (`TrustCache`).
pub fn folder_issue_in(env: &Env, cache: &mut TrustCache, id: &Id, cwd: &str) -> Option<Issue> {
    let dir = Path::new(cwd);
    if !dir.is_dir() {
        return Some(Issue {
            code: "folder.missing".into(),
            severity: Severity::Error,
            message: format!("{id}'s folder {cwd} does not exist"),
            fix: Some(match id.role() {
                Role::Aide => format!("mkdir -p {cwd}  # or point {id} elsewhere in its [[aide]] dirs"),
                _ => format!("mkdir -p {cwd}  # or point {id} elsewhere with nimitz ship set"),
            }),
            id: Some(id.clone()),
        });
    }
    trust_issue(env, cache, id, dir)
}

/// An aide is always on (docs/design/aide.md section 9): `aide.down` when
/// the machine that runs it (its `dirs` names this one) has no live job of
/// it (an error while its parent runs; Info when the parent is down too: a
/// `stop F` stops both on purpose), `aide.unpinned` when its live job is not pinned (the daemon would
/// idle-retire it), `aide.model` when the account's `availableModels` leave
/// its model out (Claude Code would silently run the default instead).
pub fn aide_issues(env: &Env, m: &Member, here: bool, parent_live: bool) -> Vec<Issue> {
    let mut out = vec![];
    let id = &m.id;
    let live = m.session.as_ref().filter(|s| s.pid.is_some());
    if here && live.is_none() {
        let why = m.reasons.first().map(String::as_str).unwrap_or("no live job");
        out.push(Issue {
            code: "aide.down".into(),
            severity: if parent_live { Severity::Error } else { Severity::Info },
            message: if parent_live {
                format!("{id} is always on but not running ({why})")
            } else {
                format!("{id} is down with its parent ({why}); it comes back with the parent's start")
            },
            fix: Some(format!("nimitz start {id}")),
            id: Some(id.clone()),
        });
    }
    if let Some(s) = live.filter(|s| s.short_id.is_some() && !s.pinned) {
        out.push(Issue {
            code: "aide.unpinned".into(),
            severity: Severity::Warn,
            message: format!("{id}'s job {} is not pinned; the daemon may retire it when idle", s.short_id.as_deref().unwrap_or_default()),
            fix: Some(format!("nimitz start {id}")),
            id: Some(id.clone()),
        });
    }
    if let (Some(model), Some(allowed)) = (crate::claude::model_for(env, id), crate::claude::available_models(env)) {
        if !crate::claude::model_allowed(&model, &allowed) {
            out.push(Issue {
                code: "aide.model".into(),
                severity: Severity::Error,
                message: format!("{id} runs on {model}, which availableModels ({}) leaves out; Claude Code would run the default model instead", allowed.join(", ")),
                fix: Some(format!("add {model} to availableModels, or set model in [[aide]]")),
                id: Some(id.clone()),
            });
        }
    }
    out
}

/// `nimitz check [<id>]`: the target, every member under it (a fleet's
/// squadrons and ships, a squadron's ships), and its parents.
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
    if id.is_none() || snap.fleets.iter().any(|f| id.is_some_and(|t| t.fleet() == f.id.fleet())) {
        out.extend(crate::config::home_layout(env));
    }
    out.extend(crate::registry::health(env));
    // needs-you notices that never reached the commodore
    out.extend(crate::notify::issues(env).into_iter().filter(|i| scoped(i)));
    // briefs an older nimitz wrote; live sessions on older rules
    out.extend(crate::rules::issues(env, snap).into_iter().filter(|i| scoped(i)));
    for f in &fleets {
        if id.map(|t| t.fleet() == f.config.fleet).unwrap_or(true) {
            out.extend(crate::config::validate(env, f).into_iter().filter(|i| scoped(i)));
        }
    }

    // Member issues: folders, then the session-level rules.
    let mut trust = TrustCache::default();
    for fleet in &snap.fleets {
        for m in fleet.members() {
            if !in_scope(&m.id, id) {
                continue;
            }
            if let Some(i) = m.cwd.as_deref().and_then(|cwd| folder_issue_in(env, &mut trust, &m.id, cwd)) {
                out.push(i);
            }
            if m.role == Role::Aide {
                let here = fleets.iter().find(|f| f.config.fleet == m.id.fleet()).is_some_and(|f| crate::config::member_dir(env, f, &m.id).is_some());
                let parent_live = m.id.parent().and_then(|p| fleet.members().find(|x| x.id == p)).is_some_and(|p| p.session.as_ref().is_some_and(|s| s.pid.is_some()));
                out.extend(aide_issues(env, m, here, parent_live));
            }
            // Session rules (name.*, labels.*, dup.live, copy.rows, lineage.broken,
            // held, ship.elsewhere) arrive here too: snapshot puts
            // sessions::session_issues into Member::issues.
            out.extend(m.issues.iter().filter(|i| scoped(i)).cloned());
        }
        out.extend(fleet.issues.iter().filter(|i| scoped(i)).cloned());
        for sq in &fleet.squadrons {
            out.extend(sq.issues.iter().filter(|i| scoped(i)).cloned());
        }
    }

    // Claude Code version against the compiled range: below the floor, unknown or
    // an untested pre-release is an error (launches refuse; --force overrides);
    // above the highest tested version only a warning (launches go ahead).
    if !snap.compat_ok {
        let c = crate::compat::supported();
        let range = format!("the supported range {} - {}", c.min, c.max);
        let refuse = Some("read-only commands still work; `--force` overrides launches".to_string());
        let (sev, message, fix) = match snap.cc_version.as_deref() {
            None => (Severity::Error, format!("Claude Code's version could not be read (`{} --version` gave none); {range}", env.native_claude_bin), refuse),
            Some(v) if v.contains('-') => (Severity::Error, format!("Claude Code {v} is a pre-release that compat.toml does not list; {range}"), refuse),
            Some(v) => match crate::compat::position(&c, v) {
                1 => (
                    Severity::Warn,
                    format!("Claude Code {v} is newer than nimitz is tested with (tested up to {}); launches go ahead", c.max),
                    Some("nothing to do; report anything odd, and the tested range widens with the next nimitz".to_string()),
                ),
                -1 => (Severity::Error, format!("Claude Code {v} is below {range}"), Some("update Claude Code (`claude update`)".to_string())),
                _ => (Severity::Error, format!("Claude Code version {v:?} is not a version nimitz can compare with {range}"), refuse),
            },
        };
        out.push(Issue { code: "cc.version".into(), severity: sev, message, fix, id: None });
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
                exact.extend(fl.members().filter(|m| m.id == *t || below(&m.id, Some(t))).filter_map(|m| m.cwd.as_deref()).map(|c| crate::env::canon(Path::new(c))));
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
        // a live one in a lead's folder is that lead's `lead.intruder` (one issue per row)
        let lead_dirs: Vec<PathBuf> = snap
            .fleets
            .iter()
            .flat_map(|f| std::iter::once(&f.admiral).chain(f.squadrons.iter().map(|sq| &sq.commodore)))
            .filter_map(|m| m.cwd.as_deref().map(|c| crate::env::canon(Path::new(c))))
            .collect();
        let in_lead_folder = |s: &SessionInfo| s.pid.is_some() && lead_dirs.contains(&crate::env::canon(Path::new(&s.cwd)));
        for s in snap.stowaways.iter().filter(|s| !legacy.contains(&s.name) && stowaway_in_scope(&s.cwd) && !in_lead_folder(s)) {
            let short = s.short_id.as_deref().unwrap_or(&s.session_id);
            // one fix per kind: a live background job is stopped, then its row
            // removed; a process with no job (interactive, SDK) has nothing
            // nimitz can stop, so it is ended in its terminal; never rm for a
            // live session (contracts.md section 7)
            let adopt = format!("# or, if it holds work to keep: nimitz adopt {} <id>", s.session_id);
            let fix = match (s.pid, &s.short_id) {
                (Some(_), Some(_)) => format!("nimitz stop {short}, then nimitz rm {short}  {adopt}"),
                (Some(_), None) => match &s.terminal {
                    Some(t) => format!("end it in its terminal ({t}): /exit, or close the host  {adopt}"),
                    None => format!("end it in its terminal: /exit, or close the host  {adopt}"),
                },
                (None, _) => format!("nimitz rm {short}  {adopt}"),
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

    // one line per finding: a config error rides on the fleet, its members and
    // the snapshot alike
    let mut seen: Vec<Issue> = vec![];
    out.retain(|i| {
        let new = !seen.contains(i);
        if new {
            seen.push(i.clone());
        }
        new
    });
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
            birth_id: None,
            entrypoint: None,
            pinned: false,
            copy: None,
            terminal: None,
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
            parent: None,
            inbox: None,
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
                aides: vec![],
                squadrons: vec![Squadron {
                    id: Id::commodore("MAIN", "ARGOS"),
                    commodore: member(Id::commodore("MAIN", "ARGOS"), sq_dir.to_str()),
                    ships: vec![member(Id::ship("MAIN", "ARGOS", "ARGOS"), ship_cwd)],
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
    fn a_stowaway_belongs_to_the_fleet_whose_folders_hold_it() {
        let (td, env) = test_env();
        make_fleet(&env);
        let ship_dir = td.path().join("project");
        std::fs::create_dir_all(&ship_dir).unwrap();
        let mut snap = snap_for(&env, ship_dir.to_str());
        let at = |cwd: &str| SessionInfo { cwd: cwd.into(), ..live_session("stray") };
        let fleet_dir = snap.fleets[0].dir.clone();
        // in a member's own folder (a user project), and anywhere under the fleet's folder
        assert_eq!(crate::snapshot::stowaway_fleet(&snap, &at(ship_dir.to_str().unwrap())).as_deref(), Some("MAIN"));
        assert_eq!(crate::snapshot::stowaway_fleet(&snap, &at(&format!("{fleet_dir}/squadrons/x"))).as_deref(), Some("MAIN"));
        // under NIMITZ_HOME but in no fleet's folder: unassigned
        assert_eq!(crate::snapshot::stowaway_fleet(&snap, &at(env.home.join("loose").to_str().unwrap())), None);
        // a legacy issue names its fleet
        let s = SessionInfo { session_id: "99999999-bbbb".into(), short_id: Some("99999999".into()), ..at("/elsewhere") };
        snap.issues.push(Issue {
            code: "legacy".into(),
            severity: Severity::Info,
            message: "stopped session 99999999 (old) belongs to the old fleet".into(),
            fix: None,
            id: Some(Id::ship("MAIN", "ARGOS", "ARGOS")),
        });
        assert_eq!(crate::snapshot::stowaway_fleet(&snap, &s).as_deref(), Some("MAIN"));
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
        let home = crate::os::canonical(&td.path()).unwrap().join("home");
        let wt = home.join(".cache").join("wt");
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
        assert_eq!(i.fix.as_deref(), Some(format!("nimitz trust {}", wt.display()).as_str()));
        set(&[&wt]);
        assert!(folder_issue(&env, &id, wt.join("src").to_str().unwrap()).is_none());
        // outside git, the home's trust still covers it
        std::fs::create_dir_all(home.join("plain")).unwrap();
        set(&[&home]);
        assert!(folder_issue(&env, &id, home.join("plain").to_str().unwrap()).is_none());
    }

    /// One check run reads the trust file once (`TrustCache`), however many
    /// members it checks: a second member's check sees the file as the
    /// first read it; a new run reads it again.
    #[test]
    fn a_check_run_reads_the_trust_file_once() {
        let (td, env) = test_env();
        let home = crate::os::canonical(&td.path()).unwrap().join("home");
        let (a, b) = (home.join("a"), home.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let trust = format!("{}.json", env.claude_home.display());
        let set = |dirs: &[&Path]| {
            let p: serde_json::Map<String, serde_json::Value> = dirs.iter().map(|d| (d.display().to_string(), serde_json::json!({"hasTrustDialogAccepted": true}))).collect();
            std::fs::write(&trust, serde_json::json!({ "projects": p }).to_string()).unwrap();
        };
        let id = Id::ship("MAIN", "ARGOS", "S4");
        set(&[&a, &b]);
        let mut run = TrustCache::default();
        assert!(folder_issue_in(&env, &mut run, &id, a.to_str().unwrap()).is_none());
        set(&[]);
        assert!(folder_issue_in(&env, &mut run, &id, b.to_str().unwrap()).is_none(), "the run's one read");
        assert!(folder_issue_in(&env, &mut TrustCache::default(), &id, b.to_str().unwrap()).is_some(), "a new run reads again");
    }

    /// M16 (cli-G6): a ship folder under a trusted home ran sessions as a plain
    /// folder, then became its own git root. Claude Code keys trust on the git
    /// root, so the old runs prove nothing: nimitz says so before a launch.
    #[test]
    fn a_folder_that_became_a_git_root_after_its_sessions_is_asked_again() {
        let (td, env) = test_env();
        make_fleet(&env);
        let home = crate::os::canonical(&td.path()).unwrap().join("home");
        let ship = home.join("ship3");
        std::fs::create_dir_all(&ship).unwrap();
        let trust = format!("{}.json", env.claude_home.display());
        let set = |dirs: &[&Path]| {
            let p: serde_json::Map<String, serde_json::Value> = dirs.iter().map(|d| (d.display().to_string(), serde_json::json!({"hasTrustDialogAccepted": true}))).collect();
            std::fs::write(&trust, serde_json::json!({ "projects": p }).to_string()).unwrap();
        };
        set(&[&home]);
        let id = Id::ship("MAIN", "ARGOS", "S3");
        // sessions ran there (a transcript and a job row with that cwd): trusted through the home
        let proj = env.claude_home.join("projects").join(crate::evidence::project_dir_name(ship.to_str().unwrap()));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("0b1d2c3e-0000-4000-8000-000000000001.jsonl"), "{}\n").unwrap();
        let job = env.claude_home.join("jobs/0b1d2c3e/state.json");
        std::fs::create_dir_all(job.parent().unwrap()).unwrap();
        let created_ms = (crate::evidence::now() * 1000.0) as i64;
        std::fs::write(&job, serde_json::json!({"sessionId": "0b1d2c3e-0000-4000-8000-000000000001", "cwd": ship, "state": "done", "createdAt": created_ms}).to_string()).unwrap();
        assert!(folder_issue(&env, &id, ship.to_str().unwrap()).is_none(), "a plain folder under the home");
        // `git init` there: a repository under a trusted home, with no entry of its own
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::create_dir_all(ship.join(".git")).unwrap();
        std::fs::write(ship.join(".git/config"), "[core]\n").unwrap();
        let i = folder_issue(&env, &id, ship.to_str().unwrap()).expect("the old runs do not cover the new repository");
        assert_eq!((i.code.as_str(), i.severity), ("folder.untrusted", Severity::Warn));
        assert!(i.message.contains("git repository"), "{}", i.message);
        assert_eq!(i.fix.as_deref(), Some(format!("nimitz trust {}", ship.display()).as_str()));
        // (this needs birth times; without them the old answer, "ran here", stands)
        assert!(crate::env::git_birth(&ship).is_some(), "the test file system keeps no birth time");
        // a later write to the old transcript is no proof either: only a start is
        std::fs::write(proj.join("0b1d2c3e-0000-4000-8000-000000000001.jsonl"), "{}\n{}\n").unwrap();
        assert!(folder_issue(&env, &id, ship.to_str().unwrap()).is_some());
        // the git root's own entry is what Claude Code looks at (M16)
        set(&[&home, &ship]);
        assert!(folder_issue(&env, &id, ship.to_str().unwrap()).is_none());
        // ... or a session started there since the repository appeared
        set(&[&home]);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(proj.join("0b1d2c3e-0000-4000-8000-000000000002.jsonl"), "{}\n").unwrap();
        assert!(folder_issue(&env, &id, ship.to_str().unwrap()).is_none());
        // the cheap predicate agrees: inside a repository, a parent above the root
        // does not count (a fresh cache per read: two writes within one clock
        // tick share an mtime)
        let cfg = env.claude_home.join(".claude.json");
        let trusted = |dir: &Path| crate::evidence::folder_trusted(&env, &mut Default::default(), dir.to_str().unwrap());
        std::fs::write(&cfg, format!(r#"{{"projects": {{"{}": {{"hasTrustDialogAccepted": true}}}}}}"#, home.display().to_string().replace('\\', "\\\\"))).unwrap();
        assert!(trusted(&home.join("plain")));
        assert!(!trusted(&ship));
        std::fs::write(&cfg, format!(r#"{{"projects": {{"{}": {{"hasTrustDialogAccepted": true}}}}}}"#, ship.display().to_string().replace('\\', "\\\\"))).unwrap();
        assert!(trusted(&ship.join("src")));
    }

    #[test]
    fn a_symlink_to_a_trusted_folder_is_trusted() {
        let (td, env) = test_env();
        make_fleet(&env);
        let real = crate::os::canonical(&td.path()).unwrap().join("project");
        std::fs::create_dir_all(&real).unwrap();
        let link = td.path().join("link");
        crate::os::symlink(&real, &link).unwrap();
        let trust = format!("{}.json", env.claude_home.display());
        std::fs::write(&trust, format!(r#"{{"projects": {{"{}": {{"hasTrustDialogAccepted": true}}}}}}"#, real.display().to_string().replace('\\', "\\\\"))).unwrap();
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
        // Folder (or a parent) accepted: trusted. Managed folders are trusted with the data root.
        // (and the data root, as `nimitz init` has it: the leads' folders are under it)
        let root = crate::env::canon(&env.home).display().to_string();
        std::fs::write(&trust, serde_json::json!({"projects": {outside.display().to_string(): {"hasTrustDialogAccepted": true}, root: {"hasTrustDialogAccepted": true}}}).to_string()).unwrap();
        assert!(check(&env, &snap, None).is_empty());
        let f = crate::config::load_fleet(&env, "MAIN").unwrap();
        let managed = f.dir.join("ships/argos/argos");
        std::fs::write(&trust, r#"{"projects": {}}"#).unwrap();
        // the data root untrusted: so is a managed folder, and the fix names the data root
        let issues = check(&env, &snap_for(&env, managed.to_str()), None);
        let i = issues.iter().find(|i| i.code == "folder.untrusted").expect("untrusted with the data root");
        assert!(i.fix.as_deref().unwrap().contains(&crate::env::canon(&env.home).display().to_string()), "{i:?}");
        let root = crate::env::canon(&env.home).display().to_string().replace('\\', "/");
        std::fs::write(&trust, serde_json::json!({"projects": {root: {"hasTrustDialogAccepted": true}}}).to_string()).unwrap();
        assert!(check(&env, &snap_for(&env, managed.to_str()), None).is_empty());
    }

    /// The data root's trust covers what really is in it, outside git: a
    /// managed folder that became a repository wants its own trust, and a
    /// link in the data root is judged where it leads. `trust::target` names
    /// the same folder the fix does.
    #[test]
    fn the_data_roots_trust_covers_neither_a_repository_in_it_nor_a_link_out_of_it() {
        let (td, env) = test_env();
        make_fleet(&env);
        let root = crate::env::canon(&env.home);
        std::fs::write(trust_file(&env), serde_json::json!({"projects": {root.display().to_string(): {"hasTrustDialogAccepted": true}}}).to_string()).unwrap();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let managed = crate::config::load_fleet(&env, "MAIN").unwrap().dir.join("ships/argos/argos");
        std::fs::create_dir_all(&managed).unwrap();
        assert!(folder_issue(&env, &id, managed.to_str().unwrap()).is_none(), "under the trusted data root");
        // `git init` there: Claude Code asks about the repository
        std::fs::create_dir_all(managed.join(".git")).unwrap();
        let i = folder_issue(&env, &id, managed.to_str().unwrap()).expect("the data root's trust does not cover a repository in it");
        assert_eq!((i.code.as_str(), i.severity), ("folder.untrusted", Severity::Warn));
        assert_eq!(crate::trust::shown(&env, &managed), crate::env::canon(&managed));
        assert_eq!(i.fix, Some(crate::trust::fix(&crate::trust::target(&env, &managed))));
        assert_eq!(crate::trust::untrusted(&env, &managed), Some(Severity::Warn));
        // a link in the data root to a folder outside it
        let outside = crate::env::canon(td.path()).join("outside/proj");
        std::fs::create_dir_all(&outside).unwrap();
        let link = env.home.join("linked");
        crate::os::symlink(&outside, &link).unwrap();
        let i = folder_issue(&env, &id, link.to_str().unwrap()).expect("a link out of the data root is not covered by it");
        assert_eq!((i.code.as_str(), i.severity), ("folder.untrusted", Severity::Error));
        assert_eq!(crate::trust::shown(&env, &link), outside);
        assert_eq!(i.fix, Some(crate::trust::fix(&crate::trust::target(&env, &link))));
        assert!(i.fix.as_deref().unwrap().contains(outside.to_str().unwrap()), "{i:?}");
        // the data root itself as a repository is still the data root's trust
        std::fs::remove_dir_all(managed.join(".git")).unwrap();
        std::fs::create_dir_all(env.home.join(".git")).unwrap();
        assert!(folder_issue(&env, &id, managed.to_str().unwrap()).is_none());
        assert_eq!(crate::trust::shown(&env, &managed), root);
    }

    /// A `CLAUDE_CONFIG_DIR` layout (the Claude home not named `.claude`)
    /// keeps its trust in `<claude home>/.claude.json`: check reads it, as
    /// `evidence::folder_trusted` and `nimitz trust` do, and a key is accepted
    /// when any of the files accepts it.
    #[test]
    fn check_reads_every_config_file_trust_writes() {
        let (td, env) = test_env();
        make_fleet(&env);
        assert_ne!(env.claude_home.file_name().unwrap(), ".claude");
        let dir = crate::env::canon(td.path()).join("project");
        std::fs::create_dir_all(&dir).unwrap();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let (inner, outer) = (env.claude_home.join(".claude.json"), trust_file(&env));
        assert_eq!(crate::trust::files(&env), vec![inner.clone(), outer.clone()]);
        std::fs::create_dir_all(&env.claude_home).unwrap();
        let accept = |yes: bool| serde_json::json!({"projects": {dir.display().to_string(): {"hasTrustDialogAccepted": yes}}}).to_string();
        let trusted = || crate::evidence::folder_trusted(&env, &mut Default::default(), dir.to_str().unwrap());
        // no file readable: unknown, no issue
        std::fs::write(&outer, "not json").unwrap();
        assert!(folder_issue(&env, &id, dir.to_str().unwrap()).is_none());
        // only the Claude home's own file, and it does not trust the folder: flagged
        std::fs::remove_file(&outer).unwrap();
        std::fs::write(&inner, r#"{"projects": {}}"#).unwrap();
        let i = folder_issue(&env, &id, dir.to_str().unwrap()).expect("read from <claude home>/.claude.json");
        assert_eq!(i.code, "folder.untrusted");
        // `nimitz trust` writes it there, and both readers see it
        crate::trust::write(&env, &dir).unwrap();
        assert!(folder_issue(&env, &id, dir.to_str().unwrap()).is_none());
        assert!(trusted());
        // one file accepts, the other names the folder without: accepted
        std::fs::write(&outer, accept(false)).unwrap();
        assert!(folder_issue(&env, &id, dir.to_str().unwrap()).is_none());
        std::fs::write(&inner, accept(false)).unwrap();
        std::fs::write(&outer, accept(true)).unwrap();
        assert!(folder_issue(&env, &id, dir.to_str().unwrap()).is_none());
        assert!(trusted());
        std::fs::write(&outer, accept(false)).unwrap();
        assert!(folder_issue(&env, &id, dir.to_str().unwrap()).is_some());
        assert!(!trusted());
    }

    /// Claude Code (Windows) compares trust keys with case and looks a folder
    /// up as it is on disk: `c:/x` does not cover `C:\x`, and check says why.
    #[cfg(windows)]
    #[test]
    fn a_trust_key_in_another_case_does_not_count_and_is_named() {
        let (td, env) = test_env();
        make_fleet(&env);
        let outside = crate::env::canon(td.path()).join("project");
        std::fs::create_dir_all(&outside).unwrap();
        let s = outside.display().to_string().replace('\\', "/");
        let lower = format!("{}{}", s[..1].to_lowercase(), &s[1..]);
        let root = crate::env::canon(&env.home).display().to_string();
        std::fs::write(format!("{}.json", env.claude_home.display()), serde_json::json!({"projects": {lower.clone(): {"hasTrustDialogAccepted": true}, root: {"hasTrustDialogAccepted": true}}}).to_string()).unwrap();
        let issues = check(&env, &snap_for(&env, outside.to_str()), None);
        let i = issues.iter().find(|i| i.code == "folder.untrusted" && i.id == Some(Id::ship("MAIN", "ARGOS", "ARGOS"))).expect("another case is no trust");
        assert!(i.message.contains(&lower), "{}", i.message);
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
        snap.cc_version = Some("2.1.297".into());
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
            birth_id: None,
            entrypoint: None,
            pinned: false,
            copy: None,
            terminal: None,
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
