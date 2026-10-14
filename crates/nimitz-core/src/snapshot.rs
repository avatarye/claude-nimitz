//! Build the Snapshot from config + registry + evidence (contracts.md section 6). Owner: core-sessions.
//!
//! Evidence order: (1) session files + `kill(pid,0)`, (2) `claude agents --json
//! --all` (cached, every ~10 s, as a cross-check for rows the files missed),
//! (3) events.jsonl (binding and labels), (4) mux panes (the TUI's job, never
//! core's). A refresh every 2 s re-reads only what changed (mtime caches).
//!
//! A row belongs to an identity by its job (the registry's `job`, plan-v4
//! section 4), by its conversation (the registry's `session` and lineage) or
//! by its labels (`NIMITZ_ID` from the session's events) — never by its name
//! alone. `Member::issues` carries every session-level code of the member: the
//! per-member rules (`sessions::session_issues`) plus what needs the whole-world
//! view (name.taken, lineage.broken). Folder and config codes stay with `check`.
use crate::env::Env;
use crate::evidence::{self, FileCache, SessionRow};
use crate::types::*;
use std::collections::{BTreeMap, HashMap, HashSet};

const CLI_EVERY_S: f64 = 10.0;
const VERSION_EVERY_S: f64 = 60.0;
/// A sub-agent that wrote within this window of a live session counts as running.
const ACTIVE_WINDOW_S: f64 = 90.0;
/// How long a transient process (a `spare: true` session file, a picker with
/// no transcript and no SessionStart) is ignored after its session file
/// appeared; after that it is classified like any other row (plan-v4 section 6).
pub const TRANSIENT_GRACE_S: f64 = 120.0;

/// Caches between refreshes (mtime-keyed JSON, titles, session dirs, the CLI
/// cross-check and the version probe).
#[derive(Default)]
pub struct Cache {
    pub files: FileCache,
    /// the CLI's rows, each with its process's creation time when read
    /// (`os::process_started_ms`: Windows only)
    cli: Option<(f64, Vec<(SessionRow, Option<i64>)>)>,
    version: Option<(f64, Option<String>)>,
}

pub fn snapshot(env: &Env, cache: &mut Cache) -> Snapshot {
    let now = evidence::now();
    let (fleets_cfg, cfg_issues) = crate::config::load_all(env);
    let events = evidence::events_cached(env, &mut cache.files);
    // a SessionStart whose registry write was lost binds now (section 3)
    crate::registry::replay(env, &events);
    let reg = crate::registry::load(env);

    // Evidence 1: rows from the files. Evidence 2: the CLI, merged in for rows
    // the files missed (never run more than once per CLI_EVERY_S).
    let mut rows = evidence::rows_from_files(env, &mut cache.files).unwrap_or_default();
    let cli_rows = match &cache.cli {
        Some((t, r)) if now - t < CLI_EVERY_S => r.clone(),
        _ => {
            let r: Vec<(SessionRow, Option<i64>)> = crate::claude::agents(env)
                .map(|v| {
                    v.into_iter()
                        .map(|r| {
                            let r = r.to_session_row();
                            let created = r.pid.and_then(crate::os::process_started_ms);
                            (r, created)
                        })
                        .collect()
                })
                .unwrap_or_default();
            cache.cli = Some((now, r.clone()));
            r
        }
    };
    // A cached CLI row may be up to CLI_EVERY_S old: its pid must still be
    // alive, and (Windows) the very process the CLI saw, not a reuse of its pid.
    // A row the files show stopped but the CLI shows live (its session file
    // could not be read) is live.
    let cli_rows: Vec<SessionRow> = cli_rows
        .into_iter()
        .map(|(mut r, created)| {
            if r.pid.map(|p| !crate::os::is_same_process(p, created)).unwrap_or(false) {
                r.pid = None;
                r.status = None;
            }
            r
        })
        .collect();
    // (joined by job id, never by session id alone: a process with no job on a
    // stopped job's conversation is its own row, section 4)
    // A job row matches its CLI row by job id (the CLI names a stopped job by
    // its birth id, the file row the conversation it runs now; acceptance
    // F1), a row with no job by session id.
    let same_row = |c: &SessionRow, r: &SessionRow| match (&c.short_id, &r.short_id) {
        (Some(a), Some(b)) => a == b,
        (None, None) => c.session_id == r.session_id,
        _ => false,
    };
    for r in rows.iter_mut().filter(|r| r.pid.is_none()) {
        if let Some(c) = cli_rows.iter().find(|c| same_row(c, r) && c.pid.is_some()) {
            r.pid = c.pid;
            r.status = c.status.clone().or(Some("unknown".into()));
        }
    }
    let seen: HashSet<String> = rows.iter().map(|r| r.session_id.clone()).collect();
    let seen_jobs: HashSet<String> = rows.iter().filter_map(|r| r.short_id.clone()).collect();
    rows.extend(cli_rows.into_iter().filter(|r| !seen.contains(&r.session_id) && !r.short_id.as_ref().is_some_and(|j| seen_jobs.contains(j))));
    // Not rows at all (plan-v4 section 6): sessions labelled for another
    // nimitz home (the hook writes no line for them either), and, for a grace
    // time after their session file appeared, Claude Code's transient
    // processes: a `spare: true` file (the agent view's "new session" or
    // draft) and a picker (no transcript, no SessionStart yet).
    let labels = evidence::session_labels(&events, &rows);
    let started: HashSet<&str> = events.iter().filter(|e| e.event == "SessionStart").filter_map(|e| e.session_id.as_deref()).collect();
    let mut transient: Vec<String> = vec![];
    rows.retain(|r| {
        if labels.get(&evidence::row_key(r)).is_some_and(|l| crate::registry::other_home(env, l)) {
            return false;
        }
        let young = r.appeared_at.is_some_and(|t| now - t < TRANSIENT_GRACE_S);
        // the picker rule: a live process with no job, no transcript, and no
        // SessionStart yet, or a spare (the agent view's draft fires a
        // SessionStart `startup` the moment it is claimed, byhand 2c-live;
        // its age is its last touch, and a long-lived one is touched again and
        // again): hidden only while it has no conversation. A dispatched job
        // that was a pre-warmed spare keeps `spare` and is never hidden.
        // (a spare may carry a `jobId`: the view claims a pre-warmed
        // `bg-spare` job, cli-L3b; a dispatched one grows a transcript). A
        // labelled row is never a picker, whatever Claude Code does with `spare`.
        let labelled = labels.get(&evidence::row_key(r)).is_some_and(evidence::is_labelled);
        let mut picker = || r.live() && !labelled && (r.spare || (r.short_id.is_none() && !started.contains(r.session_id.as_str()))) && evidence::transcript(env, &mut cache.files, Some(&r.cwd), &r.session_id).is_none();
        if young && picker() {
            transient.push(r.session_id.clone());
            return false;
        }
        true
    });
    rows.retain(|r| !transient.contains(&r.session_id));
    let labels = evidence::session_labels(&events, &rows);

    let cc_version = match &cache.version {
        Some((t, v)) if now - t < VERSION_EVERY_S => v.clone(),
        _ => {
            let v = crate::claude::version(env);
            cache.version = Some((now, v.clone()));
            v
        }
    };
    let compat = crate::compat::supported();
    let compat_ok = cc_version.as_deref().map(|v| crate::compat::position(&compat, v) == 0).unwrap_or(false);

    // Rows per identity. The registry's job wins, then its session (so a
    // session whose labels name someone else still belongs to the identity
    // that started it, and shows up as labels.mismatch); then the labels; then
    // the lineage.
    let mut by_identity: HashMap<String, Vec<SessionRow>> = HashMap::new();
    let mut claimed: HashSet<String> = HashSet::new();
    let job_of: HashMap<&str, &str> = reg.identities.iter().filter_map(|(id, rec)| Some((rec.job.as_deref()?, id.as_str()))).collect();
    let current_of: HashMap<&str, &str> =
        reg.identities.iter().filter_map(|(id, rec)| Some((rec.session.as_deref()?, id.as_str()))).collect();
    let lineage_of: HashMap<&str, &str> = reg
        .identities
        .iter()
        .flat_map(|(id, rec)| rec.lineage.iter().map(move |l| (l.session_id.as_str(), id.as_str())))
        .collect();
    // Every configured member's folder on this machine; ships work
    // in folders the user may share (the shared-folder rule below), a lead's
    // folder is exclusive (`lead.intruder`).
    let mut dir_of: HashMap<String, String> = HashMap::new();
    let mut shared_ids: Vec<Id> = vec![];
    for fc in &fleets_cfg {
        let mut put = |id: Id| {
            if let Some(d) = crate::config::member_dir(env, fc, &id) {
                dir_of.insert(id.0.clone(), norm_dir(&d.to_string_lossy()));
            }
            if id.role() == Role::Ship {
                shared_ids.push(id);
            }
        };
        put(Id::admiral(&fc.config.fleet));
        for (_, sq) in &fc.squadrons {
            put(Id::commodore(&fc.config.fleet, &sq.squadron));
            for sh in &sq.ships {
                put(Id::ship(&fc.config.fleet, &sq.squadron, &sh.name));
            }
        }
    }
    let legacy_names: HashMap<String, Id> = fleets_cfg.iter().flat_map(crate::config::legacy_map).collect();
    let folder_ok = |owner: &str, cwd: &str| -> bool {
        match dir_of.get(owner) {
            Some(d) => same_dir(d, cwd),
            None => true, // not deployed here, or not configured: nothing to compare
        }
    };
    // sessions of 0.3.x tenders (retired identities, history only): claimed by
    // their old id, so they are no stowaways, and reported as `tender.retired`
    let retired_of: HashMap<&str, &str> = reg
        .retired
        .iter()
        .flat_map(|(id, rec)| rec.session_ids().map(move |s| (s, id.as_str())))
        .collect();
    // a job row of a 0.3.x tender is history too
    let retired_job: HashMap<&str, &str> = reg.retired.iter().filter_map(|(id, rec)| Some((rec.job.as_deref()?, id.as_str()))).collect();
    // labelled sessions in another folder than their identity's (BUG-30)
    let mut misplaced: Vec<(String, SessionRow)> = vec![];
    // copies of a ship's conversation (plan-v4 section 6), by row key (job,
    // else session id): how the copy was told (`SessionInfo::copy`)
    let mut copies: HashMap<String, String> = HashMap::new();
    let unlabelled = |r: &SessionRow| !labels.get(&evidence::row_key(r)).map(evidence::is_labelled).unwrap_or(false);
    let job_rows: HashSet<&str> = rows.iter().filter_map(|r| r.short_id.as_deref()).collect();
    for r in &rows {
        let by_labels = labels.get(&evidence::row_key(r)).and_then(|l| l.get("NIMITZ_ID")).map(String::as_str);
        let job = r.short_id.as_deref();
        let by_job = job.and_then(|j| job_of.get(j).copied());
        let by_current = r.session_ids().find_map(|sid| current_of.get(sid).copied());
        let by_folder_labels = by_labels.filter(|o| folder_ok(o, &r.cwd));
        let by_lineage = r.session_ids().find_map(|sid| lineage_of.get(sid).copied());
        let owner = by_job
            .or(by_current)
            .or(by_folder_labels)
            .or(by_lineage)
            .or_else(|| r.session_ids().find_map(|sid| retired_of.get(sid).copied()))
            .or_else(|| job.and_then(|j| retired_job.get(j).copied()));
        if let Some(owner) = owner {
            // an unlabelled process on an older conversation of a ship (one in
            // its lineage, not the one it runs now): a `--resume` of that id
            // by hand, which is a copy of the ship (section 6, rule b)
            if by_job.is_none() && by_current.is_none() && by_folder_labels.is_none() && by_lineage.is_some() && Id(owner.to_string()).role() == Role::Ship && unlabelled(r) {
                copies.insert(evidence::row_key(r), "resume".into());
            }
            // an unlabelled JOB on the conversation the ship runs now while the
            // ship's own job row is there: a `--bg --resume` by a non-birth id
            // (Claude Code 2.1.292 starts a new job named after that id,
            // acceptance F1); a copy by its job, never the member's session
            let own_row_there = reg.identities.get(owner).and_then(|x| x.job.as_deref()).is_some_and(|j| job_rows.contains(j));
            // (every role: for a lead the copy is told as such, never as its session)
            if by_job.is_none() && by_current.is_some() && job.is_some() && own_row_there && unlabelled(r) {
                copies.insert(evidence::row_key(r), "job".into());
            }
            by_identity.entry(owner.to_string()).or_default().push(r.clone());
            claimed.insert(r.session_id.clone());
            // another identity's session carrying these labels: an impostor of
            // the labelled one, too (BUG-55)
            if let Some(o) = by_labels.filter(|o| *o != owner && r.live()) {
                misplaced.push((o.to_string(), r.clone()));
            }
        } else if let Some(o) = by_labels {
            misplaced.push((o.to_string(), r.clone()));
            claimed.insert(r.session_id.clone());
        }
    }
    // The stowaway rule in ship folders (plan-v4 section 6). A ship
    // folder may be shared by several ships and by the user's own work: an unlabelled,
    // unclaimed row there is the user's own and is not shown, unless it is a
    // copy of a ship in that folder: (a) its transcript shares the root of a
    // lineage conversation of the ship (a `/branch`, `/fork`, `--fork-session`
    // or `--bg -c` copies the transcript), (b) it resumes a lineage id (found
    // above), (c) it carries the ship's exact name (or an old fleet name).
    // Claude Code's own copy name (`<name>-<word>-<word>`, BUG-11) and a job
    // row that records a fork of a lineage session count for any ship. A
    // copy is the ship's extra row with `copy` set; `sweep` removes it.
    let two_words = |rest: &str| {
        let w: Vec<&str> = rest.split('-').collect();
        w.len() == 2 && w.iter().all(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_lowercase()))
    };
    let names_of = |id: &Id| {
        let mut names = vec![id.name()];
        names.extend(legacy_names.iter().filter(|(_, v)| *v == id).map(|(k, _)| k.clone()));
        names
    };
    // the roots of a ship's lineage conversations, read once per ship
    let mut ship_roots: HashMap<String, Vec<String>> = HashMap::new();
    let mut root_of = |id: &Id, cwd: &str, cache: &mut Cache| -> Vec<String> {
        if let Some(v) = ship_roots.get(&id.0) {
            return v.clone();
        }
        let mut roots: Vec<String> = vec![];
        if let Some(rec) = reg.identities.get(&id.0) {
            for sid in rec.session_ids() {
                if let Some(p) = evidence::transcript_missing_cached(env, &mut cache.files, Some(cwd), sid) {
                    roots.extend(evidence::transcript_root(&mut cache.files, &p));
                }
            }
        }
        roots.sort();
        roots.dedup();
        ship_roots.insert(id.0.clone(), roots.clone());
        roots
    };
    // the user's own sessions in ship folders: not shown, not stowaways
    let mut users_own: HashSet<String> = HashSet::new();
    let unclaimed: Vec<&SessionRow> = rows.iter().filter(|r| !claimed.contains(&r.session_id)).collect();
    for r in unclaimed {
        if !unlabelled(r) || legacy_names.contains_key(&r.name) {
            continue;
        }
        let here: Vec<&Id> = shared_ids.iter().filter(|id| dir_of.get(&id.0).is_some_and(|d| same_dir(d, &r.cwd))).collect();
        let by_exact_name = |id: &Id| names_of(id).contains(&r.name);
        let by_copy_name = |id: &Id| names_of(id).iter().any(|n| r.name.strip_prefix(&format!("{n}-")).map(two_words).unwrap_or(false));
        let by_fork = |id: &Id| r.name.is_empty() && r.forked_from.as_ref().map(|f| lineage_of.get(f.as_str()) == Some(&id.0.as_str())).unwrap_or(false);
        let mut copy_of: Option<(&Id, &str)> = None;
        if !here.is_empty() {
            let own = evidence::transcript(env, &mut cache.files, Some(&r.cwd), &r.session_id).map(|p| evidence::transcript_root(&mut cache.files, &p)).unwrap_or_default();
            if !own.is_empty() {
                copy_of = here.iter().find(|id| root_of(id, &r.cwd, cache).iter().any(|u| own.contains(u))).map(|id| (*id, "transcript"));
            }
        }
        let copy_of = copy_of
            .or_else(|| here.iter().find(|id| by_exact_name(id)).map(|id| (*id, "name")))
            .or_else(|| shared_ids.iter().find(|id| by_copy_name(id)).map(|id| (id, "name")))
            .or_else(|| shared_ids.iter().find(|id| by_fork(id)).map(|id| (id, "fork")));
        match copy_of {
            Some((id, how)) => {
                by_identity.entry(id.0.clone()).or_default().push(r.clone());
                claimed.insert(r.session_id.clone());
                copies.insert(evidence::row_key(r), how.into());
            }
            None if !here.is_empty() => {
                users_own.insert(r.session_id.clone());
            }
            None => {}
        }
    }

    let mut snap = Snapshot {
        taken_at: now,
        machine: None,
        fleets: vec![],
        stowaways: vec![],
        issues: cfg_issues,
        cc_version,
        compat_ok,
    };

    for fc in &fleets_cfg {
        let fid = Id::admiral(&fc.config.fleet);
        let mut fleet = Fleet {
            id: fid.clone(),
            dir: fc.dir.to_string_lossy().into_owned(),
            machine: fc.machine.clone(),
            admiral: build_member(env, cache, fc, &fid, &reg, &mut by_identity, &labels, &events, &copies),
            squadrons: vec![],
            issues: vec![],
        };
        for (_, sq) in &fc.squadrons {
            let sqid = Id::commodore(&fc.config.fleet, &sq.squadron);
            let mut squadron = Squadron {
                id: sqid.clone(),
                commodore: build_member(env, cache, fc, &sqid, &reg, &mut by_identity, &labels, &events, &copies),
                ships: vec![],
                issues: vec![],
            };
            for ship in &sq.ships {
                let sid = Id::ship(&fc.config.fleet, &sq.squadron, &ship.name);
                squadron.ships.push(build_member(env, cache, fc, &sid, &reg, &mut by_identity, &labels, &events, &copies));
            }
            fleet.squadrons.push(squadron);
        }
        // A squadron whose squadron.toml does not parse (BUG-15): the fleet still
        // shows, and so does the squadron, from its folder, the registry and the
        // labels, with the parse error on every member (status reads as such),
        // so its live sessions get no `rm` fixes.
        for (sqid, sd, err) in crate::config::unreadable_squadrons(fc) {
            let prefix = format!("{}/", sqid.0);
            let mut under: Vec<String> = reg.identities.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
            under.extend(by_identity.keys().filter(|k| k.starts_with(&prefix) && !crate::migrate::is_tender_id(k)).cloned());
            under.sort();
            under.dedup();
            let unread = |m: &mut Member, dir: Option<String>| {
                if m.cwd.is_none() {
                    m.cwd = dir;
                }
                m.reasons.insert(0, format!("config unreadable: {}", err.message));
                m.issues.insert(0, err.clone());
            };
            let mut commodore = build_member(env, cache, fc, &sqid, &reg, &mut by_identity, &labels, &events, &copies);
            unread(&mut commodore, Some(sd.to_string_lossy().into_owned()));
            let mut squadron = Squadron { id: sqid.clone(), commodore, ships: vec![], issues: vec![err.clone()] };
            for k in under {
                let id = Id(k);
                let mut m = build_member(env, cache, fc, &id, &reg, &mut by_identity, &labels, &events, &copies);
                unread(&mut m, None);
                squadron.ships.push(m);
            }
            fleet.squadrons.push(squadron);
        }
        snap.fleets.push(fleet);
    }
    // A fleet whose fleet.toml does not parse (M8): it still shows, from its
    // folder, the registry and the labels, with the parse error on the fleet
    // and in every member's reasons (so its live sessions get no `rm` fixes,
    // and the other fleets are not blocked).
    for (fid, dir, err) in crate::config::unreadable_fleets(env) {
        let fc = FleetOnDisk { dir: dir.clone(), config: FleetConfig { fleet: fid.fleet().to_string(), ..Default::default() }, squadrons: vec![], machine: None };
        let prefix = format!("{}/", fid.0);
        let mut under: Vec<String> = reg.identities.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
        under.extend(by_identity.keys().filter(|k| k.starts_with(&prefix)).cloned());
        under.sort();
        under.dedup();
        // the fleet carries the error once; its members say so in their reasons
        let unread = |m: &mut Member| m.reasons.insert(0, format!("config unreadable: {}", err.message));
        let mut admiral = build_member(env, cache, &fc, &fid, &reg, &mut by_identity, &labels, &events, &copies);
        unread(&mut admiral);
        let mut fleet = Fleet { id: fid.clone(), dir: dir.to_string_lossy().into_owned(), machine: None, admiral, squadrons: vec![], issues: vec![err.clone()] };
        for k in under {
            let id = Id(k);
            let Some(sq) = id.squadron() else { continue };
            let sqid = Id::commodore(fid.fleet(), sq);
            if !fleet.squadrons.iter().any(|s| s.id == sqid) {
                let mut commodore = build_member(env, cache, &fc, &sqid, &reg, &mut by_identity, &labels, &events, &copies);
                if commodore.cwd.is_none() {
                    commodore.cwd = Some(dir.join("squadrons").join(sq.to_lowercase()).to_string_lossy().into_owned());
                }
                unread(&mut commodore);
                fleet.squadrons.push(Squadron { id: sqid.clone(), commodore, ships: vec![], issues: vec![] });
            }
            if id == sqid {
                continue;
            }
            let mut m = build_member(env, cache, &fc, &id, &reg, &mut by_identity, &labels, &events, &copies);
            unread(&mut m);
            if let Some(squadron) = fleet.squadrons.iter_mut().find(|s| s.id == sqid) {
                squadron.ships.push(m);
            }
        }
        // the fleet carries the error; not once more at the bottom of status
        snap.issues.retain(|i| *i != err);
        snap.fleets.push(fleet);
    }
    // this machine's label: from a fleet that runs here, else the one `init` saved
    snap.machine = snap.fleets.iter().find_map(|f| f.machine.clone()).or_else(|| crate::registry::machine_label(env));

    // Labelled rows of identities in no config (a removed ship, a foreign fleet).
    let configured: HashSet<String> = snap.fleets.iter().flat_map(|f| f.members()).map(|m| m.id.0.clone()).collect();
    for (owner, rs) in by_identity.iter().filter(|(o, _)| !configured.contains(*o)) {
        for r in rs {
            let short = r.short_id.as_deref().unwrap_or(&r.session_id);
            // a job row a 0.3.x tender left: the role is gone (plan-v4 section 8)
            snap.issues.push(if crate::migrate::is_tender_id(owner) {
                Issue {
                    code: "tender.retired".into(),
                    severity: Severity::Info,
                    message: format!("job row {short} is the 0.3.x tender {owner}; the tender role was removed in 0.4.0, the row is history"),
                    fix: Some(if r.live() { format!("nimitz stop {short}  # then nimitz rm {short}") } else { format!("nimitz rm {short}") }),
                    id: Some(Id(owner.clone())),
                }
            } else if owner.contains("/@") {
                // a job of a 0.5/0.6 aide: the role is gone (0.7.0); pinned, so
                // the daemon never retires it by itself
                Issue {
                    code: "aide.retired".into(),
                    severity: Severity::Warn,
                    message: format!("job row {short} is the 0.6 aide {owner}; aides were removed in 0.7.0, the row is history{}", if r.live() { " (still running, pinned)" } else { "" }),
                    fix: Some(if r.live() { format!("nimitz stop {short}  # then nimitz rm {short}") } else { format!("nimitz rm {short}") }),
                    id: Some(Id(owner.clone())),
                }
            } else {
                Issue {
                    code: "config.invalid".into(),
                    severity: Severity::Warn,
                    message: format!("session {} is labelled {owner}, which no config declares", r.session_id),
                    fix: Some(format!("nimitz rm {short}  # or add {owner} back to the config")),
                    id: Some(Id(owner.clone())),
                }
            });
        }
    }

    // Stowaways: unclaimed, unlabelled sessions in a fleet's scope: anywhere
    // under NIMITZ_HOME (the fleets' own folders), in a member's folder itself
    // (a user project folder is not a scope: a ship at `~` must not claim every
    // session on the machine), or named by a fleet's legacy map.
    // Claude Code records real paths (`~/.claude-nimitz` may be a symlink), so
    // every comparison is on real paths; what the user sees stays as written.
    let mut scope_trees: Vec<std::path::PathBuf> = fleets_cfg.iter().map(|f| crate::env::canon(&f.dir)).collect();
    scope_trees.push(crate::env::canon(&env.home));
    let mut folder_of: Vec<(String, Id)> = vec![];
    for f in &snap.fleets {
        for m in f.members() {
            if let Some(c) = &m.cwd {
                folder_of.push((norm_dir(c), m.id.clone()));
            }
        }
    }
    let in_scope = |r: &SessionRow| {
        let c = crate::env::canon(std::path::Path::new(&r.cwd));
        let cn = norm_dir(&r.cwd);
        scope_trees.iter().any(|d| !d.as_os_str().is_empty() && c.starts_with(d))
            || folder_of.iter().any(|(d, _)| *d == cn)
            || legacy_names.contains_key(&r.name)
    };
    for r in rows.iter().filter(|r| !claimed.contains(&r.session_id) && !users_own.contains(&r.session_id)) {
        if labels.get(&evidence::row_key(r)).map(evidence::is_labelled).unwrap_or(false) {
            continue; // labelled but unconfigured: reported above
        }
        if in_scope(r) {
            snap.stowaways.push(r.info(Default::default()));
        }
    }

    // legacy: a stowaway whose name is in a legacy map belongs to an old fleet.
    // Only the name decides: another agent in a member's folder is a stowaway,
    // not the member's business (contracts.md section 7).
    let mut legacy: Vec<Issue> = vec![];
    for st in &snap.stowaways {
        let Some(id) = legacy_names.get(&st.name).cloned() else { continue };
        let short = st.short_id.as_deref().unwrap_or(&st.session_id);
        let whose = "belongs to the old fleet";
        let (severity, message, fix) = if st.pid.is_some() {
            (
                Severity::Warn,
                format!("live session {short} ({}) {whose}; starting {id} would make a second agent there", st.name),
                format!("stop it in the old fleet first, or nimitz adopt {} {id}", st.session_id),
            )
        } else {
            (
                Severity::Info,
                format!("stopped session {short} ({}) {whose}", st.name),
                format!("nimitz adopt {} {id}  # or nimitz rm {short}", st.session_id),
            )
        };
        legacy.push(Issue { code: "legacy".into(), severity, message, fix: Some(fix), id: Some(id) });
    }
    // A session with an identity's labels in another folder is not that
    // identity (BUG-30): labels.mismatch on the identity, the row not its own.
    for (owner, r) in &misplaced {
        let short = r.short_id.as_deref().unwrap_or(&r.session_id);
        let want = dir_of.get(owner).cloned().unwrap_or_default();
        legacy.push(Issue {
            code: "labels.mismatch".into(),
            // live, it answers to the identity's labels: an error; stopped, a row
            // to remove, which must not block every start in the fleet (BUG-54)
            severity: if r.pid.is_some() { Severity::Error } else { Severity::Warn },
            message: format!("session {short} ({}) carries {owner}'s labels but runs in folder {}, not {want}", r.name, r.cwd),
            fix: Some(if r.pid.is_some() { format!("nimitz stop {short}") } else { format!("nimitz rm {short}") }),
            id: Some(Id(owner.clone())),
        });
    }
    // A job row that does not parse (BUG-14): its session is unknown, so the
    // identity whose session it may be gets an error and no guessed start.
    for jid in evidence::corrupt_jobs(env) {
        let owner = reg.identities.iter().find(|(_, rec)| rec.job.as_deref() == Some(jid.as_str()) || rec.session_ids().any(|sid| sid.starts_with(&jid)));
        let path = env.claude_home.join("jobs").join(&jid).join("state.json");
        let i = Issue {
            code: "job.unreadable".into(),
            severity: Severity::Error,
            message: format!("job row {} does not parse; its session's state is unknown", path.display()),
            fix: Some(format!("nimitz status  # wait for Claude Code to rewrite it; if it stays broken: nimitz rm {jid}")),
            id: owner.map(|(id, _)| Id(id.clone())),
        };
        if i.id.is_some() { legacy.push(i) } else { snap.issues.push(i) }
    }
    for f in &mut snap.fleets {
        f.members_mut().for_each(|m| m.issues.extend(legacy.iter().filter(|i| i.id.as_ref() == Some(&m.id)).cloned()));
    }

    // lead.intruder (plan-v4 section 6): a lead's folder is exclusive. Any
    // live session there that is not the lead's own (its job, a held process
    // on its conversation, a live copy: those carry their own issue) is an
    // intruder, with the one fix per kind. A 0.3.x tender's row stays
    // `tender.retired` only.
    let tender_rows: HashSet<&str> = rows.iter().filter(|r| r.session_ids().any(|sid| retired_of.contains_key(sid)) || r.short_id.as_deref().is_some_and(|j| retired_job.contains_key(j))).map(|r| r.session_id.as_str()).collect();
    // the rows named as each lead's intruder (by row key), so that the
    // name.taken pass below gives a row one issue only
    let mut intruder_rows: std::collections::HashMap<String, HashSet<String>> = Default::default();
    for f in &mut snap.fleets {
        let mut intrude = |m: &mut Member| {
            let Some(dir) = m.cwd.as_deref().map(norm_dir) else { return };
            // the lead's own: its session, a held process on its conversation,
            // a copy (those carry their own issue); any other live row in the
            // folder, a /fork of the lead included, is an intruder
            let own: HashSet<&str> = m.session.iter().chain(m.extra_rows.iter().filter(|r| r.copy.is_some() || crate::sessions::held_row(m, r))).map(|s| s.session_id.as_str()).collect();
            for r in rows.iter().filter(|r| r.live() && norm_dir(&r.cwd) == dir && !own.contains(r.session_id.as_str()) && !tender_rows.contains(r.session_id.as_str())) {
                let short = r.short_id.as_deref().unwrap_or(&r.session_id);
                intruder_rows.entry(m.id.0.clone()).or_default().insert(short.to_string());
                let fix = match &r.short_id {
                    Some(j) => format!("nimitz stop {j}  # then nimitz rm {j}"),
                    None => format!("end it in its terminal{}: /exit, or close the host", r.terminal.as_deref().map(|t| format!(" ({t})")).unwrap_or_default()),
                };
                m.issues.push(Issue {
                    code: "lead.intruder".into(),
                    severity: Severity::Error,
                    message: format!(
                        "{}'s folder is exclusive, but live session {short} ({}, {}, pid {}) runs there and is not its job",
                        m.id,
                        if r.name.is_empty() { "unnamed" } else { &r.name },
                        if r.short_id.is_some() { "a background job" } else { r.entrypoint.as_deref().unwrap_or(&r.kind) },
                        r.pid.unwrap_or(0)
                    ),
                    fix: Some(fix),
                    id: Some(m.id.clone()),
                });
            }
        };
        intrude(&mut f.admiral);
        for sq in &mut f.squadrons {
            intrude(&mut sq.commodore);
        }
    }

    // name.taken: a member's expected name held by a live session that is not its own.
    let all_rows_info: Vec<SessionInfo> = rows.iter().map(|r| r.info(labels.get(&evidence::row_key(r)).cloned().unwrap_or_default())).collect();
    for f in &mut snap.fleets {
        let mut fix = |m: &mut Member| {
            let own: HashSet<&str> = m.session.iter().chain(m.extra_rows.iter()).map(|s| s.session_id.as_str()).collect();
            // one issue per row (plan-v4 section 6): a row that is this lead's
            // intruder (named so above) is not its name.taken as well
            let none = HashSet::new();
            let intruders = intruder_rows.get(&m.id.0).unwrap_or(&none);
            let others: Vec<SessionInfo> = all_rows_info
                .iter()
                .filter(|r| !own.contains(r.session_id.as_str()) && !intruders.contains(r.short_id.as_deref().unwrap_or(r.session_id.as_str())))
                .cloned()
                .collect();
            if let Some(i) = crate::sessions::name_taken_issue(m, &others) {
                m.issues.push(i);
            }
        };
        f.members_mut().for_each(&mut fix);
    }
    snap
}

/// A live job's one-line topic: its `detail`, never the placeholder of a job
/// not yet prompted ("(idle — send a prompt to start)") nor the question it
/// left (a blocked job's detail is its `needs`, said once already); a
/// finished job's result when the detail is empty. Model-written text goes to
/// a terminal: one line, no control characters.
pub(crate) fn job_topic(job: &evidence::JobState) -> Option<String> {
    // (control characters first: a line break or a BEL between two words leaves one space)
    let line = |t: &str| t.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    let needs = job.needs.as_deref().map(line);
    let detail = job.detail.as_deref().map(line).filter(|d| !d.is_empty() && !d.to_lowercase().starts_with("(idle") && Some(d) != needs.as_ref());
    let result = || job.result.as_deref().filter(|_| job.state.as_deref() == Some("done")).map(line).filter(|r| !r.is_empty());
    detail.or_else(result)
}

/// Claude Code's own idle hint in a job row ("send a prompt to start") is not
/// a question for you (BUG-07).
fn idle_hint(needs: &str) -> bool {
    let n = needs.trim().to_lowercase();
    n.starts_with("send a prompt") || n.starts_with("waiting for a prompt") || n.is_empty()
}

/// A folder as a comparable string: the real path (of its longest existing
/// prefix), no trailing slash.
/// The fleet a stowaway belongs to: the fleet of the member whose folder it
/// sits in; else the fleet whose own folder holds it (the deepest); else the
/// fleet a `legacy` issue names for it. None: in fleet scope (under
/// NIMITZ_HOME) but in no one fleet's folders.
pub fn stowaway_fleet(snap: &Snapshot, s: &SessionInfo) -> Option<String> {
    let cn = norm_dir(&s.cwd);
    for f in &snap.fleets {
        if f.members().any(|m| m.cwd.as_deref().is_some_and(|c| norm_dir(c) == cn)) {
            return Some(f.id.fleet().to_string());
        }
    }
    let c = crate::env::canon(std::path::Path::new(&s.cwd));
    let under = snap
        .fleets
        .iter()
        .map(|f| (f, crate::env::canon(std::path::Path::new(&f.dir))))
        .filter(|(_, d)| !d.as_os_str().is_empty() && c.starts_with(d))
        .max_by_key(|(_, d)| d.components().count());
    if let Some((f, _)) = under {
        return Some(f.id.fleet().to_string());
    }
    let short = s.short_id.clone().unwrap_or_else(|| s.session_id.chars().take(8).collect());
    snap.issues
        .iter()
        .filter(|i| i.code == "legacy" && (i.message.contains(&short) || i.fix.as_deref().is_some_and(|f| f.contains(&s.session_id))))
        .find_map(|i| i.id.as_ref().map(|id| id.fleet().to_string()))
}

pub(crate) fn norm_dir(d: &str) -> String {
    let t = d.trim_end_matches('/');
    crate::env::canon(std::path::Path::new(t)).to_string_lossy().trim_end_matches('/').to_string()
}

fn same_dir(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/') || norm_dir(a) == norm_dir(b)
}

/// A job with no live process: stopped when its last event is a SessionEnd
/// (`claude stop`, `/stop`, an exit), else parked (the daemon retired it, it
/// crashed, or the machine went down: no SessionEnd fires for those; plan-v4
/// section 4). Only the job's own processes count: those whose SessionStart
/// names the job (or a line an older nimitz wrote). A SessionEnd from a pid
/// that never sent a SessionStart (a refused `-p --resume`) does not count,
/// nor does a process with no job on the conversation (M21, `held`). A
/// `failed` row is stopped: it never ran, or ended while the daemon was off
/// (relaunch table row 8).
pub(crate) fn parked_or_stopped(env: &Env, p: &SessionRow, events: &[Event]) -> (Status, String) {
    if p.state.as_deref() == Some("failed") {
        return (Status::Stopped, "state failed: it never ran, or ended while the daemon was off".into());
    }
    // the job's own starts: lines that name the job; a line an older nimitz
    // wrote (no job, no kind) by the conversation (a /branch moves it on, so
    // the session id alone never decides for a 0.4.0 line)
    let mine = |e: &Event| e.session_id.as_deref().is_some_and(|sid| p.session_ids().any(|s| s == sid));
    let own_start = |e: &Event| e.event == "SessionStart" && (e.job.is_some() && e.job == p.short_id || (e.job.is_none() && e.kind.is_none() && mine(e)));
    let started: HashSet<i64> = events.iter().filter(|e| own_start(e)).filter_map(|e| e.pid).collect();
    let last = events.iter().rev().find(|e| own_start(e) || (e.event == "SessionEnd" && e.pid.is_some_and(|pid| started.contains(&pid))));
    if let Some(end) = last.filter(|e| e.event == "SessionEnd") {
        return (Status::Stopped, format!("SessionEnd event, reason {}", end.reason.as_deref().unwrap_or("not given")));
    }
    let since = last.map(|e| e.ts);
    let why = match (p.short_id.as_deref().and_then(|j| evidence::daemon_retired(env, j, since)), p.state.as_deref()) {
        (Some(r), _) => format!("parked: the daemon retired it ({r}); a start wakes it"),
        (None, Some("crashed")) => "parked: state crashed, no SessionEnd; the daemon restarts it, else a start wakes it".into(),
        _ => "parked: no SessionEnd (retired, crashed or stopped with the machine); a start wakes it".into(),
    };
    (Status::Parked, why)
}

/// One member: its rows, status, reasons, crew, tasks and whole-world issues.
#[allow(clippy::too_many_arguments)]
fn build_member(
    env: &Env,
    cache: &mut Cache,
    fc: &FleetOnDisk,
    id: &Id,
    reg: &Registry,
    by_identity: &mut HashMap<String, Vec<SessionRow>>,
    labels: &HashMap<String, BTreeMap<String, String>>,
    events: &[Event],
    copies: &HashMap<String, String>,
) -> Member {
    let rec = reg.identities.get(&id.0);
    let cwd = crate::config::member_dir(env, fc, id)
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| rec.map(|r| r.cwd.clone()).filter(|c| !c.is_empty()));
    let mut rows = by_identity.remove(&id.0).unwrap_or_default();
    let mut reasons = vec![];

    // Primary: the registry's job when its row exists (plan-v4 section 4);
    // else the row running the registry's session; else the newest live row;
    // else the newest row; but only among rows that are the identity's own (in
    // its lineage, or carrying its labels). A copy (by its transcript, its
    // name, a fork link or a resume of a lineage id) is never the current
    // session. A process holding the conversation with no job (`held`) never
    // beats the job row.
    let current = rec.and_then(|r| r.session.clone());
    let own = |r: &SessionRow| {
        !copies.contains_key(&evidence::row_key(r))
            && (rec.map(|x| x.job.is_some() && x.job == r.short_id || r.session_ids().any(|sid| x.session_ids().any(|y| y == sid))).unwrap_or(false)
                || labels.get(&evidence::row_key(r)).and_then(|l| l.get("NIMITZ_ID")) == Some(&id.0))
    };
    let info = |r: &SessionRow| {
        let mut i = r.info(labels.get(&evidence::row_key(r)).cloned().unwrap_or_default());
        i.copy = copies.get(&evidence::row_key(r)).cloned();
        i
    };
    let primary_at = rec
        .and_then(|x| x.job.as_deref())
        .and_then(|job| rows.iter().position(|r| r.short_id.as_deref() == Some(job)))
        .or_else(|| current.as_ref().and_then(|cur| rows.iter().position(|r| r.short_id.is_some() && r.session_ids().any(|sid| sid == cur))))
        .or_else(|| current.as_ref().and_then(|cur| rows.iter().position(|r| &r.session_id == cur)))
        .or_else(|| {
            // with no registry job: live before stopped, a job before a
            // process, the newest among those (a lead's two live jobs: below)
            rows.iter()
                .enumerate()
                .filter(|(_, r)| own(r))
                .max_by(|(_, a), (_, b)| {
                    (a.live(), a.short_id.is_some(), a.started_at.unwrap_or(0.0))
                        .partial_cmp(&(b.live(), b.short_id.is_some(), b.started_at.unwrap_or(0.0)))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(i, _)| i)
        });
    // A lead's row is its registry job (a hand rm + resume binds the new job
    // beside an older labelled stray: the registry job stays, the stray is the
    // intruder; a /branch or /rename of the real job changes nothing). Two
    // cases hand the lead to another own live job: (i) the registry has no
    // job; (ii) the registry job bound itself as a fork (the FIRST lineage
    // entry of that job says `via branch`: a /fork copy's hook binds when the
    // lead had no job). Then the earliest-started own live job carrying the
    // expected name, else the earliest with a started_at; never on a None or
    // an equal started_at.
    let mut primary_at = primary_at;
    if id.role().always_on() {
        let reg_job_at = rec.and_then(|x| x.job.as_deref()).and_then(|job| rows.iter().position(|r| r.short_id.as_deref() == Some(job)));
        let expected = id.name();
        let cands: Vec<usize> = (0..rows.len()).filter(|&i| own(&rows[i]) && rows[i].live() && rows[i].short_id.is_some()).collect();
        let named: Vec<usize> = cands.iter().copied().filter(|&i| rows[i].name == expected).collect();
        let earliest = |c: &[usize]| {
            c.iter().copied().filter(|&i| rows[i].started_at.is_some()).min_by(|&a, &b| rows[a].started_at.partial_cmp(&rows[b].started_at).unwrap_or(std::cmp::Ordering::Equal))
        };
        let handover = match reg_job_at {
            None => true,
            Some(_) => rec
                .and_then(|x| x.job.as_deref().and_then(|job| x.lineage.iter().find(|l| l.job.as_deref() == Some(job))))
                .is_some_and(|first| first.via == "branch"),
        };
        if handover {
            if let Some(c) = earliest(&named).or_else(|| earliest(&cands)) {
                match primary_at {
                    None => primary_at = Some(c),
                    Some(i) if c != i => {
                        if let (Some(ca), Some(ia)) = (rows[c].started_at, rows[i].started_at) {
                            if ca < ia {
                                primary_at = Some(c);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let primary = primary_at.map(|i| rows.remove(i));
    let extra_rows: Vec<SessionInfo> = rows.iter().map(info).collect();
    let session = primary.as_ref().map(info);

    // Status and the reasons for it, newest evidence first.
    let mut topic = None;
    let status = match &primary {
        Some(p) if p.unreadable && !p.live() => {
            reasons.push(format!("session file for job {} cannot be read (mid-write?); the process may be live", p.short_id.as_deref().unwrap_or("?")));
            Status::Unknown
        }
        Some(p) if p.live() => {
            let job = evidence::read_job(env, &mut cache.files, p.short_id.as_deref());
            topic = job_topic(&job);
            if let Some(w) = &p.waiting_for {
                reasons.push(format!("session file: status waiting, waitingFor {w}"));
                Status::NeedsYou
            } else if p.status.as_deref() == Some("busy") {
                reasons.push("session file: status busy".into());
                Status::Busy
            } else if p.status.as_deref() == Some("shell") {
                // an idle turn with background work (a shell command) still running
                reasons.push("session file: status shell (background work running)".into());
                Status::Busy
            } else if let Some(n) = job.needs.as_ref().filter(|n| !idle_hint(n)) {
                reasons.push(format!("job row left a question: {n}"));
                Status::NeedsYou
            } else if p.status.as_deref() == Some("idle") {
                reasons.push("session file: status idle".into());
                Status::Idle
            } else {
                // live, but no status we know: say so rather than guess
                reasons.push(format!("live pid {}, session status {:?} not understood", p.pid.unwrap_or(0), p.status.as_deref().unwrap_or("missing")));
                Status::Unknown
            }
        }
        Some(p) => {
            reasons.push(format!(
                "job row {} with no live pid (state {})",
                p.short_id.as_deref().unwrap_or("?"),
                p.state.as_deref().unwrap_or("?")
            ));
            let (status, why) = parked_or_stopped(env, p, events);
            reasons.push(why);
            status
        }
        None => {
            // No row at all: the registry's conversation may still have its
            // transcript (a resume with every flag continues it); otherwise it
            // never started, or its history is gone.
            let current_transcript = current.as_deref().and_then(|sid| evidence::transcript(env, &mut cache.files, cwd.as_deref(), sid));
            if let Some(t) = current_transcript {
                if crate::sessions::pending_relaunch(env, id).is_some() {
                    reasons.push("an interrupted relaunch: its row was removed before the resume ran (nimitz start recovers it)".into());
                }
                reasons.push(format!("no job row; its session's transcript is there ({})", t.display()));
                Status::Stopped
            } else if rec.is_some() {
                // Claude Code writes a transcript only at the first prompt: a
                // job started and never prompted leaves none, and nothing is lost
                let start = if id.role() == Role::Ship { format!("nimitz start {id} --fresh") } else { format!("nimitz start {id}") };
                reasons.push(format!("registry knows it, but no row and no transcript: most likely started and never prompted (a transcript is written at the first prompt), so nothing to resume and nothing lost; `{start}` launches it fresh"));
                Status::Stopped
            } else {
                reasons.push("no session, no lineage".into());
                Status::NotStarted
            }
        }
    };

    // A ship lock this machine left, for a session that is no longer live (it
    // was stopped outside nimitz, or crashed): clear it, or every other machine
    // stays blocked (BUG-21).
    if id.role() == Role::Ship {
        if let Some(c) = cwd.as_deref() {
            if let Some(l) = crate::sessions::read_lock(c).filter(|l| env.matches_hostname(&l.machine)) {
                let live_sid = primary.as_ref().filter(|p| p.live() || p.unreadable).map(|p| p.session_id.clone());
                let stale = match &l.session_id {
                    Some(sid) => Some(sid) != live_sid.as_ref(),
                    None => live_sid.is_none(), // an older lock names no session
                };
                if stale {
                    let _ = std::fs::remove_file(crate::sessions::ship_lock_path(c));
                    reasons.push("cleared this machine's stale ship lock (its session is not live)".into());
                }
            }
        }
    }

    if primary.as_ref().map(|p| p.pinned).unwrap_or(false) {
        reasons.push("pinned (jobs/pins.json): the daemon never idle-retires it".into());
    }

    // Crew and tasks for the live primary.
    let (mut crew, mut tasks) = (vec![], vec![]);
    if let Some(p) = primary.as_ref().filter(|p| p.live()) {
        let recs = evidence::read_subagents(env, &mut cache.files, &p.session_id);
        let now = evidence::now();
        let mut teams: BTreeMap<&str, usize> = BTreeMap::new();
        for r in &recs {
            if let Some(t) = &r.team {
                *teams.entry(t).or_default() += 1;
            }
        }
        crew = recs
            .iter()
            .map(|r| Crew {
                agent_id: r.agent_id.clone(),
                parent_id: r.parent_id.clone(),
                label: r.label.clone(),
                agent_type: r.agent_type.clone(),
                teammate: r.teammate,
                running: now - r.last_write < ACTIVE_WINDOW_S,
                last_write: r.last_write,
                workflow: r.workflow.clone(),
                workflow_name: r.workflow_name.clone(),
                phase: r.phase.clone(),
            })
            .collect();
        let team = teams
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(t, _)| t.to_string())
            .unwrap_or_else(|| format!("session-{}", &p.session_id[..p.session_id.len().min(8)]));
        tasks = evidence::read_tasks(env, &mut cache.files, &team);
    }

    let mut m = Member {
        id: id.clone(),
        role: id.role(),
        expected_name: id.name(),
        cwd,
        status,
        reasons,
        topic,
        session,
        extra_rows,
        crew,
        tasks,
        issues: vec![],
        pane: None,
    };
    // Every member carries its own issues: the session rules, then what needs
    // the registry (name.taken is added by the caller once every member's rows
    // are known).
    m.issues = crate::sessions::session_issues(env, &m);
    m.issues.extend(crate::sessions::registry_issues(env, &mut cache.files, &m, reg));
    // Remote Control on a live row: anyone with the claude.ai link drives the
    // member, and a resume reconnects it (M9).
    for r in primary.iter().chain(rows.iter()).filter(|r| r.live()) {
        if let Some(bridge) = &r.bridge_session_id {
            let short = r.short_id.as_deref().unwrap_or(&r.session_id);
            m.issues.push(Issue {
                code: "rc.exposed".into(),
                severity: Severity::Warn,
                message: format!("{id}'s session {short} is connected to Remote Control (bridge {bridge}); a resume reconnects it"),
                fix: Some(format!("run /remote-control in that session to turn it off (nimitz open {id}, then /remote-control), or archive it on claude.ai/code")),
                id: Some(id.clone()),
            });
        }
    }
    m
}
