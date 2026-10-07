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
            let r: Vec<SessionRow> =
                crate::claude::agents(env).map(|v| v.into_iter().map(|r| r.to_session_row()).collect()).unwrap_or_default();
            cache.cli = Some((now, r.clone()));
            r
        }
    };
    // A cached CLI row may be up to CLI_EVERY_S old: its pid must still be alive.
    // A row the files show stopped but the CLI shows live (its session file
    // could not be read) is live.
    let cli_rows: Vec<SessionRow> = cli_rows
        .into_iter()
        .map(|mut r| {
            if r.pid.map(|p| !evidence::pid_alive(p)).unwrap_or(false) {
                r.pid = None;
                r.status = None;
            }
            r
        })
        .collect();
    // (joined by job id, never by session id alone: a process with no job on a
    // stopped job's conversation is its own row, section 4)
    for r in rows.iter_mut().filter(|r| r.pid.is_none()) {
        if let Some(c) = cli_rows.iter().find(|c| c.session_id == r.session_id && c.short_id == r.short_id && c.pid.is_some()) {
            r.pid = c.pid;
            r.status = c.status.clone().or(Some("unknown".into()));
        }
    }
    let seen: HashSet<String> = rows.iter().map(|r| r.session_id.clone()).collect();
    rows.extend(cli_rows.into_iter().filter(|r| !seen.contains(&r.session_id)));
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
    // Every configured member's folder on this machine.
    let mut dir_of: HashMap<String, String> = HashMap::new();
    let mut ship_ids: Vec<Id> = vec![];
    for fc in &fleets_cfg {
        let mut put = |id: Id| {
            if let Some(d) = crate::config::member_dir(env, fc, &id) {
                dir_of.insert(id.0.clone(), norm_dir(&d.to_string_lossy()));
            }
            if id.role() == Role::Ship {
                ship_ids.push(id);
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
    for r in &rows {
        let by_labels = labels.get(&r.session_id).and_then(|l| l.get("NIMITZ_ID")).map(String::as_str);
        let job = r.short_id.as_deref();
        let owner = job
            .and_then(|j| job_of.get(j).copied())
            .or_else(|| r.session_ids().find_map(|sid| current_of.get(sid).copied()))
            .or_else(|| by_labels.filter(|o| folder_ok(o, &r.cwd)))
            .or_else(|| r.session_ids().find_map(|sid| lineage_of.get(sid).copied()))
            .or_else(|| r.session_ids().find_map(|sid| retired_of.get(sid).copied()))
            .or_else(|| job.and_then(|j| retired_job.get(j).copied()));
        if let Some(owner) = owner {
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
    // Copies of a member (BUG-11), strictly: an unlabelled, unclaimed row is a
    // copy only when its name is exactly how Claude Code names a copy of the
    // member (`<expected name>-<word>-<word>`, or an old fleet name plus two
    // words), or when it has no name and its job row records it as a fork of a
    // session in the member's lineage. Any other row in a member's folder is
    // someone else's agent: a stowaway (or `legacy` by name), never copy.rows,
    // dup.live or labels.missing of the member.
    let two_words = |rest: &str| {
        let w: Vec<&str> = rest.split('-').collect();
        w.len() == 2 && w.iter().all(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_lowercase()))
    };
    let unclaimed: Vec<&SessionRow> = rows.iter().filter(|r| !claimed.contains(&r.session_id)).collect();
    for r in unclaimed {
        if labels.get(&r.session_id).map(evidence::is_labelled).unwrap_or(false) || legacy_names.contains_key(&r.name) {
            continue;
        }
        let by_name = |id: &Id| {
            let mut names = vec![id.name()];
            names.extend(legacy_names.iter().filter(|(_, v)| *v == id).map(|(k, _)| k.clone()));
            names.iter().any(|n| r.name.strip_prefix(&format!("{n}-")).map(two_words).unwrap_or(false))
        };
        let by_fork = |id: &Id| {
            r.name.is_empty()
                && r.forked_from.as_ref().map(|f| lineage_of.get(f.as_str()) == Some(&id.0.as_str())).unwrap_or(false)
        };
        let copy_of = ship_ids.iter().find(|id| by_name(id) || by_fork(id));
        if let Some(id) = copy_of {
            by_identity.entry(id.0.clone()).or_default().push(r.clone());
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
                comms: sq.comms.clone(),
                issues: vec![],
            };
            for ship in &sq.ships {
                let sid = Id::ship(&fc.config.fleet, &sq.squadron, &ship.name);
                squadron.ships.push(build_member(env, cache, fc, &sid, &reg, &mut by_identity, &labels, &events));
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
            let mut commodore = build_member(env, cache, fc, &sqid, &reg, &mut by_identity, &labels, &events);
            unread(&mut commodore, Some(sd.to_string_lossy().into_owned()));
            let mut squadron = Squadron { id: sqid.clone(), commodore, ships: vec![], comms: Comms::default(), issues: vec![err.clone()] };
            for k in under {
                let id = Id(k);
                let mut m = build_member(env, cache, fc, &id, &reg, &mut by_identity, &labels, &events);
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
        let mut admiral = build_member(env, cache, &fc, &fid, &reg, &mut by_identity, &labels, &events);
        unread(&mut admiral);
        let mut fleet = Fleet { id: fid.clone(), dir: dir.to_string_lossy().into_owned(), machine: None, admiral, squadrons: vec![], issues: vec![err.clone()] };
        for k in under {
            let id = Id(k);
            let Some(sq) = id.squadron() else { continue };
            let sqid = Id::commodore(fid.fleet(), sq);
            if !fleet.squadrons.iter().any(|s| s.id == sqid) {
                let mut commodore = build_member(env, cache, &fc, &sqid, &reg, &mut by_identity, &labels, &events);
                if commodore.cwd.is_none() {
                    commodore.cwd = Some(dir.join("squadrons").join(sq.to_lowercase()).to_string_lossy().into_owned());
                }
                unread(&mut commodore);
                fleet.squadrons.push(Squadron { id: sqid.clone(), commodore, ships: vec![], comms: Comms::default(), issues: vec![] });
            }
            if id == sqid {
                continue;
            }
            let mut m = build_member(env, cache, &fc, &id, &reg, &mut by_identity, &labels, &events);
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
    let configured: HashSet<String> = {
        let mut s = HashSet::new();
        for f in &snap.fleets {
            s.insert(f.admiral.id.0.clone());
            for sq in &f.squadrons {
                s.insert(sq.commodore.id.0.clone());
                s.extend(sq.ships.iter().map(|m| m.id.0.clone()));
            }
        }
        s
    };
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
        for m in all_members(f) {
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
    for r in rows.iter().filter(|r| !claimed.contains(&r.session_id)) {
        if labels.get(&r.session_id).map(evidence::is_labelled).unwrap_or(false) {
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
        for_each_member(f, |m| m.issues.extend(legacy.iter().filter(|i| i.id.as_ref() == Some(&m.id)).cloned()));
    }

    // lead.intruder (plan-v4 section 6): a lead's folder is exclusive. Any
    // live session there that is not the lead's own (its job, a held process
    // on its conversation, a live copy: those carry their own issue) is an
    // intruder, with the one fix per kind. A 0.3.x tender's row stays
    // `tender.retired` only.
    let tender_rows: HashSet<&str> = rows.iter().filter(|r| r.session_ids().any(|sid| retired_of.contains_key(sid)) || r.short_id.as_deref().is_some_and(|j| retired_job.contains_key(j))).map(|r| r.session_id.as_str()).collect();
    for f in &mut snap.fleets {
        let intrude = |m: &mut Member| {
            let Some(dir) = m.cwd.as_deref().map(norm_dir) else { return };
            let own: HashSet<&str> = m.session.iter().chain(m.extra_rows.iter()).map(|s| s.session_id.as_str()).collect();
            for r in rows.iter().filter(|r| r.live() && norm_dir(&r.cwd) == dir && !own.contains(r.session_id.as_str()) && !tender_rows.contains(r.session_id.as_str())) {
                let short = r.short_id.as_deref().unwrap_or(&r.session_id);
                let fix = match &r.short_id {
                    Some(j) => format!("nimitz stop {j}  # then nimitz rm {j}"),
                    None => format!("end it in its terminal{}", r.terminal.as_deref().map(|t| format!(" ({t})")).unwrap_or_default()),
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
        }
    }
    snap
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
        if all_members(f).iter().any(|m| m.cwd.as_deref().is_some_and(|c| norm_dir(c) == cn)) {
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

fn all_members(f: &Fleet) -> Vec<&Member> {
    let mut v = vec![&f.admiral];
    for sq in &f.squadrons {
        v.push(&sq.commodore);
        v.extend(sq.ships.iter());
    }
    v
}

fn for_each_member(f: &mut Fleet, mut each: impl FnMut(&mut Member)) {
    each(&mut f.admiral);
    for sq in &mut f.squadrons {
        each(&mut sq.commodore);
        sq.ships.iter_mut().for_each(&mut each);
    }
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
fn parked_or_stopped(env: &Env, p: &SessionRow, events: &[Event]) -> (Status, String) {
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
    // its lineage, or carrying its labels). A copy found by its name or fork
    // link is never the current session. A process holding the conversation
    // with no job (`held`) never beats the job row.
    let current = rec.and_then(|r| r.session.clone());
    let own = |r: &SessionRow| {
        rec.map(|x| x.job.is_some() && x.job == r.short_id || r.session_ids().any(|sid| x.session_ids().any(|y| y == sid))).unwrap_or(false)
            || labels.get(&r.session_id).and_then(|l| l.get("NIMITZ_ID")) == Some(&id.0)
    };
    let primary_at = rec
        .and_then(|x| x.job.as_deref())
        .and_then(|job| rows.iter().position(|r| r.short_id.as_deref() == Some(job)))
        .or_else(|| current.as_ref().and_then(|cur| rows.iter().position(|r| r.short_id.is_some() && r.session_ids().any(|sid| sid == cur))))
        .or_else(|| current.as_ref().and_then(|cur| rows.iter().position(|r| &r.session_id == cur)))
        .or_else(|| {
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
    let primary = primary_at.map(|i| rows.remove(i));
    let extra_rows: Vec<SessionInfo> = rows.iter().map(|r| r.info(labels.get(&r.session_id).cloned().unwrap_or_default())).collect();
    let session = primary.as_ref().map(|r| r.info(labels.get(&r.session_id).cloned().unwrap_or_default()));

    // Status and the reasons for it, newest evidence first.
    let status = match &primary {
        Some(p) if p.unreadable && !p.live() => {
            reasons.push(format!("session file for job {} cannot be read (mid-write?); the process may be live", p.short_id.as_deref().unwrap_or("?")));
            Status::Unknown
        }
        Some(p) if p.live() => {
            let job = evidence::read_job(env, &mut cache.files, p.short_id.as_deref());
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
                reasons.push("registry knows it, but no row and no transcript".into());
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
