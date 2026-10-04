//! Build the Snapshot from config + registry + evidence (contracts.md section 6). Owner: core-sessions.
//!
//! Evidence order: (1) session files + `kill(pid,0)`, (2) `claude agents --json
//! --all` (cached, every ~10 s, as a cross-check for rows the files missed),
//! (3) events.jsonl (binding and labels), (4) mux panes (the TUI's job, never
//! core's). A refresh every 2 s re-reads only what changed (mtime caches).
//!
//! A row belongs to an identity by its labels (`NIMITZ_ID` from the session's
//! events) or by the registry's lineage — never by its name alone. `Member::
//! issues` carries every session-level code of the member: the per-member
//! rules (`sessions::session_issues`) plus what needs the whole-world view
//! (name.taken, lineage.broken). Folder and config codes stay with `check`.
use crate::env::Env;
use crate::evidence::{self, FileCache, SessionRow};
use crate::types::*;
use std::collections::{BTreeMap, HashMap, HashSet};

const CLI_EVERY_S: f64 = 10.0;
const VERSION_EVERY_S: f64 = 60.0;
/// A sub-agent that wrote within this window of a live session counts as running.
const ACTIVE_WINDOW_S: f64 = 90.0;

/// Caches between refreshes (mtime-keyed JSON, titles, session dirs, the CLI
/// cross-check and the version probe).
#[derive(Default)]
pub struct Cache {
    pub files: FileCache,
    cli: Option<(f64, Vec<SessionRow>)>,
    version: Option<(f64, Option<String>)>,
}

pub fn snapshot(env: &Env, cache: &mut Cache) -> Snapshot {
    let now = evidence::now();
    let (fleets_cfg, cfg_issues) = crate::config::load_all(env);
    let reg = crate::registry::load(env);
    let events = evidence::events_cached(env, &mut cache.files);
    let labels = evidence::labels_by_session(&events);

    // Evidence 1: rows from the files. Evidence 2: the CLI, merged in for rows
    // the files missed (never run more than once per CLI_EVERY_S).
    let mut rows = evidence::rows_from_files(env, &mut cache.files).unwrap_or_default();
    let cli_rows = match &cache.cli {
        Some((t, r)) if now - t < CLI_EVERY_S => r.clone(),
        _ => {
            let r: Vec<SessionRow> =
                crate::claude::agents(env).map(|v| v.into_iter().map(|r| r.to_session_row()).collect()).unwrap_or_default();
            cache.cli = Some((now, r.clone()));
            r
        }
    };
    // A cached CLI row may be up to CLI_EVERY_S old: its pid must still be alive.
    let seen: HashSet<String> = rows.iter().map(|r| r.session_id.clone()).collect();
    rows.extend(cli_rows.into_iter().filter(|r| !seen.contains(&r.session_id)).map(|mut r| {
        if r.pid.map(|p| !evidence::pid_alive(p)).unwrap_or(false) {
            r.pid = None;
            r.status = None;
        }
        r
    }));

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

    // Rows per identity. The registry's `current` wins (so a session whose
    // labels name someone else still belongs to the identity that started it,
    // and shows up as labels.mismatch); then the labels; then the lineage.
    let mut by_identity: HashMap<String, Vec<SessionRow>> = HashMap::new();
    let mut claimed: HashSet<String> = HashSet::new();
    let current_of: HashMap<&str, &str> =
        reg.identities.iter().filter_map(|(id, rec)| Some((rec.current.as_deref()?, id.as_str()))).collect();
    let lineage_of: HashMap<&str, &str> = reg
        .identities
        .iter()
        .flat_map(|(id, rec)| rec.lineage.iter().map(move |l| (l.session_id.as_str(), id.as_str())))
        .collect();
    for r in &rows {
        let owner = current_of
            .get(r.session_id.as_str())
            .copied()
            .or_else(|| labels.get(&r.session_id).and_then(|l| l.get("NIMITZ_ID")).map(String::as_str))
            .or_else(|| lineage_of.get(r.session_id.as_str()).copied());
        if let Some(owner) = owner {
            by_identity.entry(owner.to_string()).or_default().push(r.clone());
            claimed.insert(r.session_id.clone());
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
            admiral: build_member(env, cache, fc, &fid, &reg, &mut by_identity, &labels, &events),
            squadrons: vec![],
            issues: vec![],
        };
        for (_, sq) in &fc.squadrons {
            let sqid = Id::commodore(&fc.config.fleet, &sq.squadron);
            let mut squadron = Squadron {
                id: sqid.clone(),
                commodore: build_member(env, cache, fc, &sqid, &reg, &mut by_identity, &labels, &events),
                ships: vec![],
                tenders: vec![],
                comms: sq.comms.clone(),
                issues: vec![],
            };
            for ship in &sq.ships {
                let sid = Id::ship(&fc.config.fleet, &sq.squadron, &ship.name);
                squadron.ships.push(build_member(env, cache, fc, &sid, &reg, &mut by_identity, &labels, &events));
            }
            // Tenders are not declared: they exist in the registry or as labelled sessions.
            let prefix = format!("{}/~", sqid.0);
            let mut tender_ids: Vec<String> = reg.identities.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
            tender_ids.extend(by_identity.keys().filter(|k| k.starts_with(&prefix)).cloned());
            tender_ids.sort();
            tender_ids.dedup();
            for t in tender_ids {
                squadron.tenders.push(build_member(env, cache, fc, &Id(t), &reg, &mut by_identity, &labels, &events));
            }
            fleet.squadrons.push(squadron);
        }
        snap.fleets.push(fleet);
    }
    snap.machine = snap.fleets.iter().find_map(|f| f.machine.clone());

    // Labelled rows of identities in no config (a removed ship, a foreign fleet).
    let configured: HashSet<String> = {
        let mut s = HashSet::new();
        for f in &snap.fleets {
            s.insert(f.admiral.id.0.clone());
            for sq in &f.squadrons {
                s.insert(sq.commodore.id.0.clone());
                s.extend(sq.ships.iter().map(|m| m.id.0.clone()));
                s.extend(sq.tenders.iter().map(|m| m.id.0.clone()));
            }
        }
        s
    };
    for (owner, rs) in by_identity.iter().filter(|(o, _)| !configured.contains(*o)) {
        for r in rs {
            snap.issues.push(Issue {
                code: "config.invalid".into(),
                severity: Severity::Warn,
                message: format!("session {} is labelled {owner}, which no config declares", r.session_id),
                fix: Some(format!("nimitz rm {}  # or add {owner} back to the config", r.short_id.as_deref().unwrap_or(&r.session_id))),
                id: Some(Id(owner.clone())),
            });
        }
    }

    // Stowaways: unclaimed, unlabelled sessions inside a fleet's scope
    // (NIMITZ_HOME or a member's folder).
    let fleet_dirs: Vec<String> = {
        let mut v: Vec<String> = fleets_cfg.iter().map(|f| f.dir.to_string_lossy().into_owned()).collect();
        v.push(env.home.to_string_lossy().into_owned());
        for f in &snap.fleets {
            let members = std::iter::once(&f.admiral)
                .chain(f.squadrons.iter().flat_map(|s| std::iter::once(&s.commodore).chain(s.ships.iter()).chain(s.tenders.iter())));
            v.extend(members.filter_map(|m| m.cwd.clone()));
        }
        v
    };
    let in_scope = |cwd: &str| {
        let c = std::path::Path::new(cwd);
        fleet_dirs.iter().any(|d| !d.is_empty() && c.starts_with(d))
    };
    for r in rows.iter().filter(|r| !claimed.contains(&r.session_id)) {
        if labels.get(&r.session_id).map(evidence::is_labelled).unwrap_or(false) {
            continue; // labelled but unconfigured: reported above
        }
        if in_scope(&r.cwd) {
            snap.stowaways.push(r.info(Default::default()));
        }
    }

    // name.taken: a member's expected name held by a live session that is not its own.
    let all_rows_info: Vec<SessionInfo> = rows.iter().map(|r| r.info(labels.get(&r.session_id).cloned().unwrap_or_default())).collect();
    for f in &mut snap.fleets {
        let mut fix = |m: &mut Member| {
            let own: HashSet<&str> = m.session.iter().chain(m.extra_rows.iter()).map(|s| s.session_id.as_str()).collect();
            let others: Vec<SessionInfo> = all_rows_info.iter().filter(|r| !own.contains(r.session_id.as_str())).cloned().collect();
            if let Some(i) = crate::sessions::name_taken_issue(m, &others) {
                m.issues.push(i);
            }
        };
        fix(&mut f.admiral);
        for sq in &mut f.squadrons {
            fix(&mut sq.commodore);
            sq.ships.iter_mut().for_each(&mut fix);
            sq.tenders.iter_mut().for_each(&mut fix);
        }
    }
    snap
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
) -> Member {
    let rec = reg.identities.get(&id.0);
    let cwd = crate::config::member_dir(env, fc, id)
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| rec.map(|r| r.cwd.clone()).filter(|c| !c.is_empty()));
    let mut rows = by_identity.remove(&id.0).unwrap_or_default();
    let mut reasons = vec![];

    // Primary: the registry's current session when its row exists; else the
    // newest live row; else the newest row.
    let current = rec.and_then(|r| r.current.clone());
    let primary_at = current
        .as_ref()
        .and_then(|cur| rows.iter().position(|r| &r.session_id == cur))
        .or_else(|| {
            rows.iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| {
                    (a.live(), a.started_at.unwrap_or(0.0))
                        .partial_cmp(&(b.live(), b.started_at.unwrap_or(0.0)))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(i, _)| i)
        });
    let primary = primary_at.map(|i| rows.remove(i));
    let extra_rows: Vec<SessionInfo> = rows.iter().map(|r| r.info(labels.get(&r.session_id).cloned().unwrap_or_default())).collect();
    let session = primary.as_ref().map(|r| r.info(labels.get(&r.session_id).cloned().unwrap_or_default()));

    // Status and the reasons for it, newest evidence first.
    let status = match &primary {
        Some(p) if p.live() => {
            let job = evidence::read_job(env, &mut cache.files, p.short_id.as_deref());
            if let Some(w) = &p.waiting_for {
                reasons.push(format!("session file: status waiting, waitingFor {w}"));
                Status::NeedsYou
            } else if p.status.as_deref() == Some("busy") {
                reasons.push("session file: status busy".into());
                Status::Busy
            } else if let Some(n) = &job.needs {
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
            if let Some(end) = events.iter().rev().find(|e| e.event == "SessionEnd" && e.session_id.as_deref() == Some(p.session_id.as_str())) {
                reasons.push(format!("SessionEnd event, reason {}", end.reason.as_deref().unwrap_or("not given")));
            }
            Status::Stopped
        }
        None => {
            // No row at all. An interactive member may have ended, leaving its
            // transcript (resume with `claude -c`); otherwise it never started.
            let transcript =
                id.role().interactive() && cwd.as_deref().map(|c| !evidence::transcripts_in(env, c).is_empty()).unwrap_or(false);
            if transcript {
                reasons.push("no session row; a transcript in its folder (continue with -c)".into());
                Status::Stopped
            } else if rec.is_some() {
                reasons.push("registry knows it, but no row and no transcript".into());
                Status::Stopped
            } else {
                reasons.push("no session, no lineage".into());
                Status::NotStarted
            }
        }
    };

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
    m
}
