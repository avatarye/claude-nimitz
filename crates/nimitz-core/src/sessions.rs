//! Session actions along the chain of command: start (top-down), stop, relaunch,
//! rm, sweep, adopt; the relaunch state table (contracts.md section 8). Owner: core-sessions.
//!
//! Every `plan_*` only reads and returns a `Plan`; nothing changes until
//! `apply`. A plan step's command is real argv (`claude ...`) except the
//! internal `["bind", <id>, <session id>, <via>]` step, which `apply` performs
//! itself (a registry write, not a process). Every member, leads too, is a
//! Claude Code background job (plan-v4 section 7): `apply` runs every step;
//! a lead's Herdr tab only attaches (`layout::attach_tab`), so core never
//! depends on the mux.
use crate::env::Env;
use crate::types::*;
use std::collections::HashMap;

/// Parse a session name (address) back into an identity (contracts.md section 4):
/// `[F]-ADMIRAL`, `[F.SQ]-COMMODORE`, `[F.SQ]-SHIP`.
/// None for names that are not fleet addresses (teammates, ad-hoc sessions).
/// A trailing ` [ref]` (how `SendMessage` disambiguates two sessions with one
/// name) is not part of the address, so it cannot hide a target from the rules.
pub fn id_from_name(name: &str) -> Option<Id> {
    let mut name = name.trim();
    if let Some((head, tail)) = name.rsplit_once(' ') {
        if tail.starts_with('[') && tail.ends_with(']') {
            name = head.trim_end();
        }
    }
    let rest = name.strip_prefix('[')?;
    let close = rest.find(']')?;
    let tag = &rest[..close];
    let role = rest[close + 1..].strip_prefix('-')?;
    let (fleet, sq) = match tag.split_once('.') {
        Some((f, s)) => (f, Some(s)),
        None => (tag, None),
    };
    if !crate::config::valid_name(fleet) || !sq.map(crate::config::valid_name).unwrap_or(true) {
        return None;
    }
    match (sq, role) {
        (None, "ADMIRAL") => Some(Id::admiral(fleet)),
        (Some(sq), "COMMODORE") => Some(Id::commodore(fleet, sq)),
        (Some(sq), r) if crate::config::valid_name(r) => Some(Id::ship(fleet, sq, r)),
        _ => None,
    }
}

/// The Claude Code session this process runs inside, when one is above it
/// (its session file, found through the process's ancestry): its session
/// id, or `?` when the file names none.
pub fn session_above_id(env: &Env, vars: &HashMap<String, String>) -> Option<String> {
    session_above(env, vars).map(|v| v.get("sessionId").and_then(serde_json::Value::as_str).unwrap_or("?").to_string())
}

/// The session file of the nearest Claude Code process above this one (one
/// whose `procStart` names another process is a stale file: skipped).
fn session_above(env: &Env, vars: &HashMap<String, String>) -> Option<serde_json::Value> {
    // tests only (a debug build): the ancestry is given, a sandbox has no
    // Claude Code above it. A release build never reads it, so no caller
    // can name another session as its own.
    if cfg!(debug_assertions) {
        if let Some(pids) = vars.get("NIMITZ_TEST_ANCESTORS") {
            return pids.split(',').filter_map(|p| p.trim().parse::<i64>().ok()).find_map(|p| read_session_file(env, p));
        }
    }
    crate::evidence::ancestors().into_iter().find_map(|p| read_session_file(env, p))
}

fn read_session_file(env: &Env, pid: i64) -> Option<serde_json::Value> {
    std::fs::read(env.claude_home.join("sessions").join(format!("{pid}.json"))).ok().and_then(|b| crate::evidence::first_json(&b)).filter(|d| crate::evidence::session_is_its_process(pid, d))
}

/// The identity a caller gets when its labels are partial or disagree: one
/// `[WARN]` and it runs as you (kb-guard reads it as no member's).
pub const UNVERIFIED: &str = "?/?/unverified-labels";

/// Who is asking, from the caller's env (labels reach the Bash tool's
/// processes, probe P0.6). No labels at all (`NIMITZ_HOME` alone is none): the
/// user. Labels that are partial (`NIMITZ_ID` or `NIMITZ_ROLE` missing), that
/// disagree, or that name no valid identity: an unverified caller (the CLI
/// warns and runs it as the user).
pub fn caller_from_env(vars: &HashMap<String, String>) -> Caller {
    let get = |k: &str| vars.get(k).filter(|v| !v.is_empty());
    let unverified = Caller::Member { id: Id(UNVERIFIED.into()), role: Role::Ship };
    match (get("NIMITZ_ID"), get("NIMITZ_ROLE")) {
        (None, None) => Caller::User,
        (Some(id), Some(role)) => {
            let id = Id(id.clone());
            let shape_ok = id.well_formed();
            // the role label must agree with the id's shape; otherwise distrust both
            if shape_ok && id.role().as_str() == role {
                Caller::Member { role: id.role(), id }
            } else {
                unverified
            }
        }
        _ => unverified,
    }
}

// ------------------------------------------------------------------ issues

fn issue(code: &str, severity: Severity, message: String, fix: Option<String>, id: Option<Id>) -> Issue {
    Issue { code: code.into(), severity, message, fix, id }
}

/// The lock file a managed ship folder holds while the ship runs on a machine.
pub fn ship_lock_path(cwd: &str) -> std::path::PathBuf {
    std::path::Path::new(cwd).join(".nimitz-ship.lock")
}

/// A live process with no job that runs a member's conversation now (an SDK
/// host, an interactive or Remote Control `--resume`, a 0.3.x interactive
/// lead): the member is `held` (M13), never a copy and never a second live
/// session of it. Every member is a job (0.4.0), leads too; a process with
/// no job on an older lineage conversation is a `copy`, not `held`.
pub(crate) fn held_row(_m: &Member, r: &SessionInfo) -> bool {
    r.pid.is_some() && r.short_id.is_none() && r.copy.is_none()
}

/// The row that holds `m`'s conversation with no job, if one does.
pub(crate) fn held_of(m: &Member) -> Option<&SessionInfo> {
    m.session.iter().chain(m.extra_rows.iter()).find(|r| held_row(m, r))
}

/// The `held` issue for one such row: a 0.3.x interactive lead is ended with
/// `/exit` (then `nimitz start` runs it as a job, plan-v4 section 7); any
/// other holder is ended in its host.
pub(crate) fn held_issue(m: &Member, r: &SessionInfo) -> Issue {
    // (a lead with a job, live or parked, is no 0.3.x lead: the holder is a
    // by-hand resume of a conversation of its lineage)
    let has_job = m.session.as_ref().is_some_and(|p| p.short_id.is_some());
    let job_live = m.session.as_ref().is_some_and(|p| p.pid.is_some() && p.short_id.is_some());
    let interactive_lead = m.role.lead() && !has_job && r.kind == "interactive" && r.entrypoint.as_deref().unwrap_or("cli") == "cli";
    let (what, fix) = if interactive_lead {
        (
            format!("{} runs as a 0.3.x interactive lead (pid {}, no job); nimitz cannot stop or resume it there", m.id, r.pid.unwrap_or(0)),
            format!("end the interactive lead with /exit, then run nimitz start {} again  # it comes back as a background job with the same conversation", m.id),
        )
    } else {
        (
            format!(
                "{}'s conversation {} runs in a process with no job (pid {}, {}): nimitz cannot stop or resume it there",
                m.id,
                r.session_id,
                r.pid.unwrap_or(0),
                r.entrypoint.as_deref().unwrap_or(&r.kind)
            ),
            // a live member is relaunched (start would do nothing), a stopped one started
            // a live member needs nothing more once that process ends (the
            // relaunch refusal adds its own step); a stopped one is started
            if job_live { "end that process in its host".to_string() } else { format!("end that process in its host, then nimitz start {}", m.id) },
        )
    };
    issue("held", Severity::Warn, what, Some(fix), Some(m.id.clone()))
}

/// The one fix that ends a copy of a member (plan-v4 section 6): a
/// background job is swept (`stop`, then `rm`); an interactive or SDK
/// process has no job to stop, so it is ended in its own terminal.
fn copy_fix(m: &Member, r: &SessionInfo) -> String {
    match (&r.short_id, &r.terminal) {
        (Some(_), _) => format!("nimitz sweep {}", m.id),
        (None, Some(t)) => format!("end it in its terminal ({t}): /exit, or close the host"),
        (None, None) => "end it in its terminal: /exit, or close the host".into(),
    }
}

/// A live extra row of a LEAD in its own (exclusive) folder that is not its
/// session: its `lead.intruder` (the snapshot names it, one fix: stop, then
/// rm), never a second live session of the lead, a copy row, or a reason to
/// refuse the lead's relaunch (acceptance inside FAIL-3, re-review 1).
pub(crate) fn lead_intruder_row(m: &Member, r: &SessionInfo) -> bool {
    m.role.lead() && r.pid.is_some() && m.cwd.as_deref().is_some_and(|d| same_folder(&r.cwd, d)) && m.session.as_ref().map(|p| p.session_id != r.session_id).unwrap_or(true)
}

/// How a copy was told, for a message.
fn copy_how(r: &SessionInfo) -> &'static str {
    match r.copy.as_deref() {
        Some("transcript") => "its transcript starts the member's conversation",
        Some("resume") => "it resumes a conversation of the member's lineage",
        Some("name") => "it carries the member's name",
        Some("job") => "it is a second job on the member's conversation (a wake by an id that is not the job's birth id)",
        Some("fork") => "its job row records a fork of the member's conversation",
        _ => "a copy",
    }
}

/// Session-level issues readable from the member and the filesystem alone:
/// name.drift, labels.missing, labels.mismatch, dup.live, copy.rows, held,
/// ship.elsewhere. (name.taken and lineage.broken need the whole snapshot or
/// the registry; `snapshot` adds those to `Member::issues`.)
pub fn session_issues(env: &Env, m: &Member) -> Vec<Issue> {
    let mut out = vec![];
    let id = Some(m.id.clone());
    let relaunch = Some(restart_cmd(&m.id));
    let rows: Vec<&SessionInfo> = m.session.iter().chain(m.extra_rows.iter()).collect();
    // a lead's folder is exclusive: a live extra row there (a /fork, a hand
    // launch with its labels) is its `lead.intruder` (the snapshot names it,
    // one fix), never a second live session or a copy row of the lead
    let intruder = |r: &SessionInfo| lead_intruder_row(m, r);
    let live: Vec<&&SessionInfo> = rows.iter().filter(|r| r.pid.is_some() && !held_row(m, r) && r.copy.is_none() && !intruder(r)).collect();
    // A copy of the member's conversation (plan-v4 section 6): one Warn per
    // copy, with the fix that works for its kind.
    for r in m.extra_rows.iter().filter(|r| r.copy.is_some()) {
        let short = r.short_id.as_deref().unwrap_or(&r.session_id);
        let kind = match (&r.short_id, r.pid) {
            (Some(_), Some(_)) => "a live background job",
            (Some(_), None) => "a stopped job row",
            (None, _) => "a process with no job (interactive or SDK)",
        };
        out.push(issue(
            "copy",
            Severity::Warn,
            format!("{} has a copy {short} ({}), {kind}: {}", m.id, r.name, copy_how(r)),
            Some(copy_fix(m, r)),
            id.clone(),
        ));
    }
    out.extend(rows.iter().filter(|r| held_row(m, r)).map(|r| held_issue(m, r)));

    if live.len() > 1 {
        let names: Vec<String> =
            live.iter().map(|r| format!("{} ({})", r.short_id.as_deref().unwrap_or(&r.session_id), r.name)).collect();
        out.push(issue(
            "dup.live",
            Severity::Error,
            format!("{} has {} live sessions: {}", m.id, live.len(), names.join(", ")),
            Some("nimitz stop <short>  # for the one you drop".into()),
            id.clone(),
        ));
    }
    if let Some(p) = &m.session {
        // the name the job carries (its state.json, or the live session file)
        // against the identity's: a `/rename` or Ctrl+R drifts it, and a wake
        // keeps the drifted name, so a stopped job drifts too (M19)
        if p.name != m.expected_name && !held_row(m, p) {
            out.push(issue(
                "name.drift",
                Severity::Error,
                if p.pid.is_some() {
                    format!("{} is live as {:?}, expected {:?}", m.id, p.name, m.expected_name)
                } else {
                    format!("{}'s job {} is named {:?}, expected {:?}; a wake would keep that name", m.id, p.short_id.as_deref().unwrap_or(&p.session_id), p.name, m.expected_name)
                },
                relaunch.clone(),
                id.clone(),
            ));
        }
        match p.labels.get("NIMITZ_ID") {
            None if p.labels.is_empty() => out.push(issue(
                "labels.missing",
                Severity::Error,
                format!("{}'s session {} has no labels (resumed with flags, or legacy)", m.id, p.session_id),
                relaunch.clone(),
                id.clone(),
            )),
            Some(other) if *other != m.id.0 => out.push(issue(
                "labels.mismatch",
                Severity::Error,
                format!("{}'s session {} carries labels of {other}", m.id, p.session_id),
                relaunch.clone(),
                id.clone(),
            )),
            _ => {}
        }
    }
    // The session's saved folder is not the member's folder any more (moved, or
    // `ship set --dir`): Claude Code resumes in the saved folder, so a resume
    // fails when that folder is gone or untrusted (BUG-31).
    if let (Some(p), Some(want)) = (m.session.as_ref().filter(|p| p.pid.is_none() && !p.cwd.is_empty()), m.cwd.as_deref()) {
        let same = p.cwd.trim_end_matches('/') == want.trim_end_matches('/')
            || std::fs::canonicalize(&p.cwd).ok() == std::fs::canonicalize(want).ok() && std::path::Path::new(&p.cwd).exists();
        if !same {
            let gone = !std::path::Path::new(&p.cwd).is_dir();
            let untrusted = !gone && !crate::evidence::folder_trusted(env, &mut crate::evidence::FileCache::default(), &p.cwd);
            if gone || untrusted {
                out.push(issue(
                    "folder.moved",
                    Severity::Error,
                    format!(
                        "{}'s session {} was saved in {}, which is {}; its folder is now {want}, and a resume would run in the old one",
                        m.id,
                        p.short_id.as_deref().unwrap_or(&p.session_id),
                        p.cwd,
                        if gone { "gone" } else { "not trusted" }
                    ),
                    Some(format!("{}  # a labelled copy in the new folder", restart_cmd(&m.id))),
                    id.clone(),
                ));
            }
        }
    }
    let extra: Vec<&SessionInfo> = m.extra_rows.iter().filter(|r| !held_row(m, r) && r.copy.is_none() && !intruder(r)).collect();
    if !extra.is_empty() {
        let shorts: Vec<&str> = extra.iter().map(|r| r.short_id.as_deref().unwrap_or(r.session_id.as_str())).collect();
        out.push(issue(
            "copy.rows",
            Severity::Warn,
            format!("{} has {} extra job rows (copies or stale rows): {}", m.id, extra.len(), shorts.join(", ")),
            Some(format!("nimitz sweep {}", m.id)),
            id.clone(),
        ));
    }
    // A managed ship folder in use on another machine leaves a lock file.
    if m.role == Role::Ship {
        if let Some(l) = m.cwd.as_deref().and_then(read_lock) {
            if !env.matches_hostname(&l.machine) {
                let age = l.at.map(|t| format!(", locked {:.0} min ago", ((crate::evidence::now() - t) / 60.0).max(0.0))).unwrap_or_default();
                let by = l.session_id.as_deref().map(|s| format!(" by session {}", &s[..s.len().min(8)])).unwrap_or_default();
                out.push(issue(
                    "ship.elsewhere",
                    Severity::Error,
                    format!("{}'s folder is in use on {:?}{by}{age} (lock file)", m.id, l.machine),
                    Some(format!(
                        "on {}: nimitz stop {}  # if that machine is gone for good: rm {}",
                        l.machine,
                        m.id,
                        ship_lock_path(m.cwd.as_deref().unwrap_or("")).display()
                    )),
                    id.clone(),
                ));
            }
        }
    }
    out
}

/// A managed ship folder's lock: which machine runs the ship there, and as
/// which session (BUG-21: enough to tell a stale lock from a live one).
#[derive(Clone, Debug, PartialEq)]
pub struct ShipLock {
    pub machine: String,
    pub session_id: Option<String>,
    pub pid: Option<i64>,
    pub at: Option<f64>,
}

pub fn read_lock(cwd: &str) -> Option<ShipLock> {
    let raw = std::fs::read(ship_lock_path(cwd)).ok()?;
    let v = crate::evidence::first_json(&raw)?;
    Some(ShipLock {
        machine: v.get("machine").and_then(|m| m.as_str())?.to_string(),
        session_id: v.get("session_id").and_then(|m| m.as_str()).map(str::to_string),
        pid: v.get("pid").and_then(|m| m.as_i64()),
        at: v.get("at").or_else(|| v.get("since")).and_then(|m| m.as_f64()),
    })
}

/// The machine named in a folder's ship lock, if the lock exists and parses.
pub fn lock_holder(cwd: &str) -> Option<String> {
    read_lock(cwd).map(|l| l.machine)
}

/// Issues that need the registry: lineage.broken (the current session id has no
/// job row and no transcript). Called by `snapshot` for every member. A lead
/// with no transcript gets a cold launch instead (plan-v4 section 5): an
/// error on an admiral would refuse every start in its fleet.
pub fn registry_issues(env: &Env, cache: &mut crate::evidence::FileCache, m: &Member, reg: &Registry) -> Vec<Issue> {
    let mut out = vec![];
    // the registry names another conversation than the one the row would
    // wake, and not a later one of the lineage (a lost hook line, a hand
    // edit): the row wins, and a relaunch brings the two together
    if let Some(p) = m.session.as_ref().filter(|p| p.pid.is_none() && p.short_id.is_some()) {
        if let Some(stale) = wake_target_in(reg.identities.get(&m.id.0), p).stale {
            out.push(issue(
                "lineage.stale",
                Severity::Warn,
                format!("{}'s registry names {} but its job would wake on {}; the job's conversation wins", m.id, &stale[..stale.len().min(8)], &p.session_id[..p.session_id.len().min(8)]),
                Some(restart_cmd(&m.id)),
                Some(m.id.clone()),
            ));
        }
    }
    if m.role.lead() {
        return out;
    }
    let Some(rec) = reg.identities.get(&m.id.0) else { return out };
    let Some(current) = rec.session.as_deref() else { return out };
    let known = m.session.iter().chain(m.extra_rows.iter()).any(|r| r.session_id == current || r.birth_id.as_deref() == Some(current));
    if known || crate::evidence::transcript(env, cache, m.cwd.as_deref(), current).is_some() {
        return out;
    }
    out.push(issue(
        "history.lost",
        Severity::Error,
        format!("{}'s current session {current} has no job row and no transcript; a resume has nothing to continue", m.id),
        Some(format!("nimitz start {} --fresh  # a cold launch; the lineage starts a new entry", m.id)),
        Some(m.id.clone()),
    ));
    out
}

/// name.taken: the expected name is held by a live session that is not this
/// member's. `others` are live rows claimed by other identities or by nobody.
pub fn name_taken_issue(m: &Member, others: &[SessionInfo]) -> Option<Issue> {
    let own = m.session.as_ref().filter(|p| p.pid.is_some()).map(|p| p.session_id.as_str());
    let holder = others.iter().find(|r| r.pid.is_some() && r.name == m.expected_name && Some(r.session_id.as_str()) != own)?;
    Some(issue(
        "name.taken",
        Severity::Error,
        format!("{}'s name {:?} is held by live session {} in {}", m.id, m.expected_name, holder.session_id, holder.cwd),
        // a hand copy in the member's own folder may hold no history (a fork
        // stopped before its first turn, P0.9 A2): stop it, never adopt it
        Some(match m.cwd.as_deref().filter(|c| same_folder(c, &holder.cwd)) {
            Some(_) => format!("nimitz stop {}", holder.short_id.as_deref().unwrap_or(&holder.session_id)),
            None => format!("nimitz stop {}  # or adopt it", holder.short_id.as_deref().unwrap_or(&holder.session_id)),
        }),
        Some(m.id.clone()),
    ))
}

// ------------------------------------------------------------------ the caller

/// The command that relaunches `id` alone (same id and history).
pub fn restart_cmd(id: &Id) -> String {
    format!("nimitz restart {id}")
}

/// The caller's member id by its labels: none for the user or an
/// unverified caller (partial or disagreeing labels).
pub fn verified(caller: &Caller) -> Option<&Id> {
    match caller {
        Caller::Member { id, .. } if id.0 != UNVERIFIED => Some(id),
        _ => None,
    }
}

/// The member that runs this command: by its labels, else the member whose
/// worker is an ancestor of this process (a session with no labels, or
/// partial ones). Its own end goes to the self-end helper.
pub fn me_of(snap: &Snapshot, caller: &Caller) -> Option<Id> {
    if let Some(id) = verified(caller) {
        return Some(id.clone());
    }
    // tests only (a debug build): the ancestry is given, as for `session_above`
    let given = if cfg!(debug_assertions) { std::env::var("NIMITZ_TEST_ANCESTORS").ok() } else { None };
    let up: Vec<i64> = match given {
        Some(pids) => pids.split(',').filter_map(|p| p.trim().parse().ok()).collect(),
        None => crate::evidence::ancestors(),
    };
    members(snap).into_iter().map(|(_, m)| m).find(|m| m.session.as_ref().and_then(|s| s.pid).is_some_and(|p| p > 1 && up.contains(&p))).map(|m| m.id.clone())
}

/// Is member `me` `target` or under it? A retire of `target` (ship,
/// squadron or fleet rm) would end it mid-command: the CLI runs that retire
/// last, detached, once its turn ends (`self_end`). A stop or a relaunch
/// ends only its own member: those go by `me == target`.
pub fn inside(me: &Id, target: &Id) -> bool {
    me == target || me.0.starts_with(&format!("{}/", target.0))
}

fn refuse(plan: &mut Plan, code: &str, msg: String, fix: Option<String>, id: Option<Id>) {
    plan.refusals.push(issue(code, Severity::Error, msg, fix, id));
}

// ------------------------------------------------------------------ members in a snapshot

/// Every member of a snapshot, with its fleet.
fn members(snap: &Snapshot) -> Vec<(&Fleet, &Member)> {
    snap.fleets.iter().flat_map(|f| f.members().map(move |m| (f, m))).collect()
}

fn member<'a>(snap: &'a Snapshot, id: &Id) -> Option<(&'a Fleet, &'a Member)> {
    members(snap).into_iter().find(|(_, m)| &m.id == id)
}

/// The chain of command above `id`, top-down, then `id` itself.
pub(crate) fn chain(id: &Id) -> Vec<Id> {
    let mut c = vec![id.clone()];
    let mut cur = id.clone();
    while let Some(p) = cur.parent() {
        c.push(p.clone());
        cur = p;
    }
    c.reverse();
    c
}

// ------------------------------------------------------------------ the relaunch state table

/// A live old-fleet or unlabelled session in the member's folder (issue
/// `legacy`, Warn while live): any start would make a second agent there.
fn refuse_legacy(plan: &mut Plan, m: &Member) -> bool {
    let Some(i) = m.issues.iter().find(|i| i.code == "legacy" && i.severity == Severity::Warn) else { return false };
    plan.refusals.push(Issue { severity: Severity::Error, ..i.clone() });
    true
}

/// One member's action per contracts.md section 8. Appends steps or refusals.
fn start_member(env: &Env, plan: &mut Plan, snap: &Snapshot, id: &Id, opts: StartOpts) {
    let force = opts.force;
    let Some((fleet, m)) = member(snap, id) else {
        refuse(plan, "member.unknown", format!("{id} is not in any fleet config"), Some("nimitz status  # the ids that exist".into()), Some(id.clone()));
        return;
    };
    let fod = match crate::config::load_fleet(env, fleet.id.fleet()) {
        Ok(f) => f,
        Err(i) => {
            plan.refusals.push(i);
            return;
        }
    };
    let cwd = m.cwd.clone().or_else(|| crate::config::member_dir(env, &fod, id).map(|p| p.to_string_lossy().into_owned()));
    let Some(cwd) = cwd else {
        refuse(plan, "folder.missing", format!("{id} has no folder on this machine"), Some(dir_fix(id)), Some(id.clone()));
        return;
    };
    let mode = crate::config::permission_mode(&fod, id);
    if refuse_legacy(plan, m) {
        return;
    }
    // What the whole-world view found and no row of the table fixes: a job row
    // that does not parse, this identity's labels on a session in another
    // folder, its name held by another live session.
    if let Some(e) = m.issues.iter().find(|i| i.severity == Severity::Error && ["job.unreadable", "labels.mismatch", "name.taken"].contains(&i.code.as_str())) {
        plan.refusals.push(e.clone());
        return;
    }
    // A session file that cannot be read: the process may be live, and a live
    // session is never resumed (BUG-13).
    if let Some(p) = m.session.as_ref().filter(|p| p.pid.is_none() && p.status.as_deref() == Some("unreadable")) {
        refuse(
            plan,
            "session.unreadable",
            format!("{id}'s session file for {} cannot be read; its process may be live", p.short_id.as_deref().unwrap_or(&p.session_id)),
            Some(format!("nimitz status  # read it again in a moment; if it stays so: nimitz stop {id}")),
            Some(id.clone()),
        );
        return;
    }
    let session_errs = session_issues(env, m);
    let step = |id: &Id, cmd: Vec<String>, why: &str| Step { id: Some(id.clone()), cmd, cwd: Some(cwd.clone()), why: why.into() };

    // A process with no job holds the conversation (a 0.3.x interactive lead,
    // an SDK host): a resume would start a copy (cli-L2), so nothing starts
    // until that process ends (the migration of plan-v4 section 7).
    if let Some(h) = held_of(m) {
        plan.refusals.push(Issue { severity: Severity::Error, ..held_issue(m, h) });
        return;
    }
    if m.session.as_ref().map(|p| p.pid.is_some()).unwrap_or(false) {
        // live with drift / missing labels / a twin: refuse with the fix
        // (rows 2, 3); live and clean: nothing (row 1)
        if let Some(e) = session_errs.iter().find(|i| i.severity == Severity::Error) {
            plan.refusals.push(e.clone());
        }
        return;
    }
    // The current session is stopped but a copy of the identity is live: a start
    // would make two. Never start a second live session (row 3's rule).
    if let Some(r) = m.extra_rows.iter().find(|r| r.pid.is_some()) {
        let short = r.short_id.as_deref().unwrap_or(&r.session_id);
        refuse(
            plan,
            "dup.live",
            format!("{id}'s current session is stopped, but its copy {short} ({}) is live", r.name),
            Some(format!("nimitz stop {short}  # or nimitz adopt {} {id} to keep the copy", r.session_id)),
            Some(id.clone()),
        );
        return;
    }
    // A stopped member: name drift and missing labels are what rows 5 and 9 fix,
    // so they do not refuse. Everything else about the session still does.
    const ROW_FIXES: [&str; 3] = ["name.drift", "labels.missing", "copy.rows"];
    if let Some(e) = session_errs.iter().find(|i| i.severity == Severity::Error && !ROW_FIXES.contains(&i.code.as_str())) {
        plan.refusals.push(e.clone());
        return;
    }
    plan.warnings.extend(session_errs);

    // `--fresh`: a cold launch that starts a new lineage entry; the old rows
    // (their history gone, `history.lost`) go after it
    if opts.fresh {
        let before = plan.steps.len();
        cold_launch(env, plan, &fod, id, m.role, &mode, &cwd);
        if plan.steps.len() > before {
            for r in m.session.iter().chain(m.extra_rows.iter()).filter(|r| r.pid.is_none()) {
                if let Some(short) = &r.short_id {
                    plan.steps.push(step(id, crate::claude::rm(env, short), "drop the old row: its history is not coming back (--fresh)"));
                }
            }
        }
        return;
    }

    let lineage_broken = m.issues.iter().any(|i| i.code == "lineage.broken");
    let history_lost = m.issues.iter().any(|i| i.code == "history.lost");
    match &m.session {
        // row `failed` or broken lineage: cold launch only with --force (row 8)
        Some(p) if p.state.as_deref() == Some("failed") || lineage_broken => {
            if !force {
                refuse(
                    plan,
                    "lineage.broken",
                    format!("{id}'s session {} ended `failed` or points nowhere", p.session_id),
                    Some(format!("nimitz start {id} --force  # cold launch; lineage starts over")),
                    Some(id.clone()),
                );
                return;
            }
            let before = plan.steps.len();
            cold_launch(env, plan, &fod, id, m.role, &mode, &cwd);
            // the failed row goes after the launch, as `--fresh`'s do: left,
            // it would be a copy row (`copy.rows`) straight away (issue #2)
            if plan.steps.len() > before && p.pid.is_none() {
                if let Some(short) = &p.short_id {
                    plan.steps.push(step(id, crate::claude::rm(env, short), "drop the failed row: its lineage starts over (--force)"));
                }
            }
        }
        // stopped or parked with a row: wake it by its exact id (rows 4, 5)
        Some(_) => match wake_steps(env, m, &mode, &cwd) {
            Ok(steps) => plan.steps.extend(steps),
            Err(i) => {
                plan.refusals.push(i);
                return;
            }
        },
        None if (history_lost || lineage_broken) && !force => {
            // the gate kept the member's own history.lost as a warning: that
            // one line is the refusal (one issue, one line)
            match plan.warnings.iter().position(|i| i.code == "history.lost" && i.id.as_ref() == Some(id)) {
                Some(at) => {
                    let w = plan.warnings.remove(at);
                    plan.refusals.push(w);
                }
                None => refuse(
                    plan,
                    "history.lost",
                    format!("{id}'s recorded session has no job row and no transcript; a resume has nothing to continue"),
                    Some(format!("nimitz start {id} --fresh  # a cold launch; the lineage starts a new entry")),
                    Some(id.clone()),
                ),
            }
            return;
        }
        None => {
            let olds = legacy_rows(snap, &fod, id, &cwd);
            // the registry's current session lost its job row but kept its
            // transcript (BUG-12): `--resume <id>` with every flag continues it
            // (or an interrupted relaunch's session: a migration's old row was
            // already removed, and the registry does not know it yet)
            let current = crate::registry::load(env).identities.get(&id.0).and_then(|r| r.session.clone());
            let has_log = |sid: &String| crate::evidence::transcript(env, &mut crate::evidence::FileCache::default(), Some(&cwd), sid).is_some();
            let kept = pending_relaunch(env, id).filter(has_log).or_else(|| current.filter(has_log));
            if let Some(sid) = kept {
                let interrupted = pending_relaunch(env, id).as_deref() == Some(sid.as_str());
                let never_a_job = m.role.lead() && crate::registry::load(env).identities.get(&id.0).and_then(|r| r.job.clone()).is_none();
                let why = if interrupted {
                    format!("recovering an interrupted relaunch: resume {} with every flag (its row was removed; Claude Code keeps the id)", &sid[..sid.len().min(8)])
                } else if never_a_job {
                    format!("{MIGRATE_LEAD_WHY}: resume {} as a background job with every flag (same id and history)", &sid[..sid.len().min(8)])
                } else {
                    format!("resume {} from its transcript with every flag (its job row is gone; Claude Code keeps the id)", &sid[..sid.len().min(8)])
                };
                plan.steps.push(step(id, crate::claude::relaunch_bg(env, id, &mode, &sid), &why));
            } else if let (Role::Ship, Some(newest)) = (m.role, olds.first()) {
                // a migrated ship keeps its history: continue the old fleet's newest
                // stopped session the row-5 way (a labelled copy); the old rows stay
                // (Info `legacy`, for `sweep` to offer)
                let short = newest.short_id.as_deref().unwrap_or(&newest.session_id);
                let mut steps = relabel_steps(env, id, &mode, newest, &cwd);
                if let Some(last) = steps.last_mut() {
                    last.why = format!("{MIGRATE_WHY} {short} (newest of {}): its id becomes the ship's, history kept", olds.len());
                }
                plan.steps.extend(steps);
            } else {
                cold_launch(env, plan, &fod, id, m.role, &mode, &cwd); // row 7
                if m.role.lead() {
                    if let Some(old) = legacy_names_of(&fod, id).first() {
                        if let Some(last) = plan.steps.last_mut() {
                            last.why = format!(
                                "first launch: the old {old} ran in another folder, and Claude Code resumes a conversation only in its own folder"
                            );
                        }
                    }
                }
            }
        }
    }
    // copy rows: after the start, rm every row that is not current (row 9)
    for r in m.extra_rows.iter().filter(|r| r.pid.is_none()) {
        if let Some(short) = &r.short_id {
            plan.steps.push(step(id, crate::claude::rm(env, short), "drop a stale copy row"));
        }
    }
}

/// The steps that wake a stopped or parked member by its exact id (plan-v4
/// section 5; relaunch table rows 4 and 5): the member has a job row and no
/// live process. With its labels and its name: `claude --bg --resume
/// <session>`, native, no flags (the job keeps its short id). Without labels,
/// or under a drifted name: `rm` the row, then `--bg --resume <session>` with
/// every flag (Claude Code keeps the id and the history). Before either, the
/// transcript must exist (any project dir): missing, it is `history.lost`
/// (Error, no launch; fix `nimitz start <id> --fresh`). A live job is never
/// resumed (that starts a copy): `Err(session.live)`. The restore of a fleet
/// (P9) builds on this.
pub fn wake_steps(env: &Env, m: &Member, mode: &str, cwd: &str) -> Result<Vec<Step>, Issue> {
    let id = &m.id;
    let Some(p) = m.session.as_ref() else {
        return Err(issue("session.none", Severity::Error, format!("{id} has no job row to wake"), Some(format!("nimitz start {id}")), Some(id.clone())));
    };
    if p.pid.is_some() {
        return Err(issue(
            "session.live",
            Severity::Error,
            format!("{id} is live as {}; nothing to wake (a resume of a live job starts a copy)", p.short_id.as_deref().unwrap_or(&p.session_id)),
            Some(format!("nimitz status  # it is live; nothing to wake")),
            Some(id.clone()),
        ));
    }
    // A job in the crash window (state `crashed`, or `working` with no live
    // pid) is the daemon's to restart first: `apply` waits for that before
    // any resume. Its transcript may not be on disk yet (killed in its first
    // turn), so it is not judged `history.lost` here; a resume that comes
    // back `startup` reports it then (e2e case 6).
    // The conversation to wake first (`wake_target`), then its transcript:
    // missing, the wake would run with no history (`history.lost`)
    let target = wake_target(env, id, p);
    let recent = p.status_since.is_some_and(|t| crate::evidence::now() - t < CRASH_RECENT_S);
    let crash_window = !target.relabel && (matches!(p.state.as_deref(), Some("crashed") | Some("working")) || (m.status == Status::Parked && recent));
    if !crash_window && crate::evidence::transcript(env, &mut crate::evidence::FileCache::default(), Some(cwd), &target.session).is_none() {
        return Err(issue(
            "history.lost",
            Severity::Error,
            format!("{id}'s conversation {} has no transcript in any project dir; a resume would start with no history", target.session),
            Some(format!("nimitz start {id} --fresh  # a cold launch; the lineage starts a new entry")),
            Some(id.clone()),
        ));
    }
    let step = |cmd: Vec<String>, why: &str| Step { id: Some(id.clone()), cmd, cwd: Some(cwd.to_string()), why: why.into() };
    let needs_relabel = session_issues(env, m).iter().any(|i| matches!(i.code.as_str(), "name.drift" | "labels.missing"));
    let short8 = |s: &str| s[..s.len().min(8)].to_string();
    // The registry's conversation is later than the one a native wake brings
    // back (a /branch or /resume after it): the row goes and that one is
    // resumed by id with every flag, and the plan says so (r3-e2e finding 11).
    if target.relabel {
        let mut v = relabel_to(env, id, mode, p, cwd, &target.session);
        if let Some(s) = v.last_mut() {
            s.why = format!(
                "the job would wake on {} (resumeSessionId follows /clear only), not on {} that it ran last (a /branch or /resume): resume that one with every flag, same history, new row",
                short8(&p.session_id),
                short8(&target.session)
            );
        }
        return Ok(v);
    }
    // with its labels and its name: resume = native binary, NO flags, from its
    // folder (row 4). A resume restores the saved name too, so a drifted name
    // goes the relaunch way instead (row 5).
    if p.labels.get("NIMITZ_ID") == Some(&id.0) && !needs_relabel {
        // by the job's BIRTH id (`state.json` `sessionId`): Claude Code 2.1.292
        // wakes a stopped job only by that id, and it comes back on the
        // conversation it runs now (after a `/clear`, a later one); any other
        // id starts a new unlabelled job named after it (acceptance F1)
        let birth = p.birth_id.as_deref().unwrap_or(&p.session_id);
        let why = if birth == p.session_id {
            "resume keeps the id, name, mode and labels (probe 2c1)".to_string()
        } else {
            format!("resume by the job's birth id {}: it wakes on {} with its name, mode and labels", &birth[..birth.len().min(8)], &p.session_id[..p.session_id.len().min(8)])
        };
        return Ok(vec![step(crate::claude::resume_bg(env, birth), &why)]);
    }
    Ok(relabel_steps(env, id, mode, p, cwd))
}

/// The conversation a wake of a stopped row must bring back, and how
/// (plan-v4 section 5, r3-e2e finding 11). Claude Code wakes a stopped job on
/// its `resumeSessionId` (the row's session: `/clear` moves it, `/branch` and
/// `/resume` do not). The registry's `session` is the conversation the
/// member ran last. When it is LATER in the lineage than the row's (a
/// /branch or /resume after the row's conversation), that one is the target
/// and the wake needs `rm` + every flag (`relabel`); when it is the same, a
/// native wake; when it differs any other way (a lost hook line, a hand
/// edit) the row wins, Claude Code's own truth, and `stale` names the
/// registry's session for a `lineage.stale` warning.
#[derive(Clone, Debug, PartialEq)]
pub struct WakeTarget {
    pub session: String,
    pub relabel: bool,
    pub stale: Option<String>,
}

/// `WakeTarget` of `id`'s row `p` (every wake path uses it: `wake_steps`,
/// `plan_relaunch`, restore's attach-wake).
pub fn wake_target(env: &Env, id: &Id, p: &SessionInfo) -> WakeTarget {
    let rec = crate::registry::load(env).identities.remove(&id.0);
    wake_target_in(rec.as_ref(), p)
}

/// `wake_target` on a registry record already loaded (the snapshot's).
pub fn wake_target_in(rec: Option<&IdentityRecord>, p: &SessionInfo) -> WakeTarget {
    let row_now = p.session_id.clone();
    let Some(reg) = rec.and_then(|r| r.session.clone()).filter(|s| *s != row_now) else {
        return WakeTarget { session: row_now, relabel: false, stale: None };
    };
    let pos = |sid: &str| rec.and_then(|r| r.lineage.iter().rposition(|l| l.session_id == sid));
    match (pos(&reg), pos(&row_now)) {
        (Some(a), Some(b)) if a > b => WakeTarget { session: reg, relabel: true, stale: None },
        _ => WakeTarget { session: row_now, relabel: false, stale: Some(reg) },
    }
}

/// The `why` that marks a step continuing an old fleet's session; `apply`
/// records such a binding with `via: migrate`.
pub const MIGRATE_WHY: &str = "continue the old fleet's session";

/// The `why` that marks a 0.3.x interactive lead's conversation becoming a
/// background job (plan-v4 section 7); `apply` records it `via: migrate`.
pub const MIGRATE_LEAD_WHY: &str = "migrate the 0.3.x interactive lead";

/// Old names (from `<fleet>/legacy.toml`) that map to `id`.
fn legacy_names_of(fod: &FleetOnDisk, id: &Id) -> Vec<String> {
    crate::config::legacy_map(fod).into_iter().filter(|(_, v)| v == id).map(|(k, _)| k).collect()
}

/// Stopped old-fleet sessions of `id` in its own folder (stowaways named in the
/// legacy map), newest first (job `updatedAt`, else `startedAt`).
fn legacy_rows<'a>(snap: &'a Snapshot, fod: &FleetOnDisk, id: &Id, cwd: &str) -> Vec<&'a SessionInfo> {
    let names = legacy_names_of(fod, id);
    let mut v: Vec<&SessionInfo> = snap
        .stowaways
        .iter()
        .filter(|s| s.pid.is_none() && s.short_id.is_some() && s.state.as_deref() != Some("failed"))
        .filter(|s| names.contains(&s.name) && crate::snapshot::norm_dir(&s.cwd) == crate::snapshot::norm_dir(cwd))
        .collect();
    let at = |s: &SessionInfo| s.status_since.or(s.started_at).unwrap_or(0.0);
    v.sort_by(|a, b| at(b).partial_cmp(&at(a)).unwrap_or(std::cmp::Ordering::Equal));
    v
}

fn cold_launch(env: &Env, plan: &mut Plan, fod: &FleetOnDisk, id: &Id, role: Role, mode: &str, cwd: &str) {
    match role {
        Role::Ship => {
            let Some(prompt) = crate::config::ship_prompt(fod, id) else {
                refuse(plan, "prompt.missing", format!("{id} has no prompt (prompt or prompt_file) in squadron.toml"), Some(format!("nimitz ship set {id} --prompt <text>")), Some(id.clone()));
                return;
            };
            plan.steps.push(Step {
                id: Some(id.clone()),
                cmd: crate::claude::launch_bg(env, id, mode, Some(&prompt)),
                cwd: Some(cwd.to_string()),
                why: "cold launch with the prompt from squadron.toml".into(),
            });
        }
        // a lead is a background job too (plan-v4 section 7); its tab attaches
        Role::Admiral | Role::Commodore => plan.steps.push(Step {
            id: Some(id.clone()),
            cmd: crate::claude::launch_bg(env, id, mode, Some(&crate::claude::lead_prompt(id))),
            cwd: Some(cwd.to_string()),
            why: "cold launch of a lead as a background job; `nimitz open` attaches to it".into(),
        }),
    }
}

// ------------------------------------------------------------------ plans

/// Pre-start gate: `nimitz check` scoped to the id. Errors refuse (cc.version
/// and lineage.broken yield to --force); warnings ride along. The relaunch
/// table handles the session-level codes itself, with their row's action.
fn gate(env: &Env, plan: &mut Plan, snap: &Snapshot, id: Option<&Id>, force: bool) {
    // (lead.intruder rides along too: an intruder in a lead's folder is told
    // and fixed, but it does not hold up the lead's own job)
    const TABLE_HANDLED: [&str; 8] = ["name.drift", "labels.missing", "labels.mismatch", "dup.live", "copy.rows", "lineage.broken", "history.lost", "lead.intruder"];
    for i in crate::check::check(env, snap, id) {
        let forced_away = force && i.code == "cc.version";
        match i.severity {
            Severity::Error if !TABLE_HANDLED.contains(&i.code.as_str()) && !forced_away => plan.refusals.push(i),
            _ => plan.warnings.push(i),
        }
    }
    // The version: `check` makes it an error below the floor, when unknown, or
    // for an untested pre-release (those refuse, --force overrides); above the
    // highest tested version it is a warning and the launch goes ahead, so a
    // nightly Claude Code update never stops a fleet (0.3.2).
}

/// The start of a member that never ran: a folder Claude Code will not
/// trust is no refusal, the plan trusts it first (`trust::step`, which says
/// what that lets run). Only the started member's own folder, only on its
/// first launch, never one `trust::too_broad` names; a wake and a parent's
/// folder keep the refusal (or the warning), the fix `nimitz trust`.
fn trust_first(env: &Env, plan: &mut Plan, snap: &Snapshot, id: &Id) {
    let Some((_, m)) = member(snap, id) else { return };
    let never_ran = m.session.is_none() && crate::registry::load(env).identities.get(&id.0).and_then(|r| r.session.clone()).is_none();
    let Some(dir) = m.cwd.clone().filter(|_| never_ran) else { return };
    let dir = std::path::Path::new(&dir);
    if crate::trust::too_broad(env, dir).is_some() {
        return;
    }
    let mine = |i: &Issue| i.code == "folder.untrusted" && i.id.as_ref() == Some(id);
    if !plan.refusals.iter().chain(plan.warnings.iter()).any(mine) {
        return;
    }
    plan.refusals.retain(|i| !mine(i));
    plan.warnings.retain(|i| !mine(i));
    plan.steps.insert(0, crate::trust::step(env, Some(id.clone()), dir));
}

/// The flags of `nimitz start`: `--force` (a cold launch over a broken
/// lineage or an untested Claude Code), `--fresh` (a cold launch that starts
/// a new lineage entry, the fix for `history.lost`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StartOpts {
    pub force: bool,
    pub fresh: bool,
}

/// Start `id` and every missing parent, top-down. Runs `check` first.
pub fn plan_start(env: &Env, snap: &Snapshot, id: &Id, caller: &Caller, force: bool) -> Plan {
    plan_start_with(env, snap, id, caller, StartOpts { force, fresh: false })
}

/// `plan_start` with every flag.
pub fn plan_start_with(env: &Env, snap: &Snapshot, id: &Id, caller: &Caller, opts: StartOpts) -> Plan {
    let force = opts.force;
    let mut plan = Plan { summary: format!("start {id} (top-down)"), ..Default::default() };
    let _ = caller;
    gate(env, &mut plan, snap, Some(id), force);
    trust_first(env, &mut plan, snap, id);
    // `--fresh` starts over: not under a live job (that would make two)
    if opts.fresh && member(snap, id).map(|(_, m)| m.session.as_ref().map(|p| p.pid.is_some()).unwrap_or(false)).unwrap_or(false) {
        refuse(
            &mut plan,
            "session.live",
            format!("{id} is live; `--fresh` starts a new conversation, so stop it first"),
            Some(format!("nimitz stop {id}, then nimitz start {id} --fresh")),
            Some(id.clone()),
        );
        return plan;
    }
    for link in chain(id) {
        let running_clean = member(snap, &link)
            .map(|(_, m)| {
                m.session.as_ref().map(|p| p.pid.is_some() && !held_row(m, p)).unwrap_or(false)
                    && session_issues(env, m).iter().all(|i| i.severity != Severity::Error)
            })
            .unwrap_or(false);
        if running_clean {
            if link == *id {
                // the target itself: the empty plan says why (M25)
                plan.warnings.push(issue("session.live", Severity::Info, format!("nothing to do: {id} is live"), None, Some(id.clone())));
            }
            continue; // row 1 for a parent: nothing
        }
        // a parent held by a process with no job (a 0.3.x interactive admiral
        // still running) is live for the chain: its ships start; the warning
        // rides along, and only a start of the parent itself refuses
        if let Some(h) = member(snap, &link).filter(|_| link != *id).and_then(|(_, m)| held_of(m).map(|h| held_issue(m, h))) {
            plan.warnings.push(h);
            continue;
        }
        if already_refused(&plan, &link) {
            continue; // the gate found something that no step can fix
        }
        // `--fresh` is for the target alone; a parent wakes as usual
        start_member(env, &mut plan, snap, &link, StartOpts { force, fresh: opts.fresh && link == *id });
    }
    // A `legacy` warning is about starting that member; a running parent left
    // alone does not make the target's start a second agent anywhere (BUG-38).
    let acting: Vec<Id> = plan.steps.iter().filter_map(|s| s.id.clone()).chain(std::iter::once(id.clone())).collect();
    plan.warnings.retain(|i| i.code != "legacy" || i.id.as_ref().map(|x| acting.contains(x)).unwrap_or(true));
    // `--fresh` is the answer to a lost history: the plan does not repeat the question
    if opts.fresh {
        plan.warnings.retain(|i| !(i.code == "history.lost" && i.id.as_ref() == Some(id)));
    }
    dedup_issues(&mut plan);
    mark_repairs(&mut plan);
    debug_assert!(no_fork(snap, &plan), "a plan would fork a session: {:?}", plan.steps);
    plan
}

/// Has the gate (or an earlier member) already refused for this identity?
fn already_refused(plan: &Plan, id: &Id) -> bool {
    plan.refusals.iter().any(|i| i.id.as_ref() == Some(id) || i.id.is_none())
}

/// Ships before the ships that declare them in `depends_on`. A cycle or an
/// unknown name never drops a ship: it keeps its place (`check` reports it).
pub(crate) fn in_dependency_order(ids: Vec<Id>, deps: &[(String, Vec<String>)]) -> Vec<Id> {
    let dep_of = |id: &Id| -> Vec<String> {
        let name = id.parts().get(2).map(|s| s.to_string()).unwrap_or_default();
        deps.iter().find(|(n, _)| *n == name).map(|(_, d)| d.clone()).unwrap_or_default()
    };
    let name_of = |id: &Id| id.parts().get(2).map(|s| s.to_string()).unwrap_or_default();
    let mut out: Vec<Id> = vec![];
    let mut left = ids;
    while !left.is_empty() {
        let ready: Vec<Id> = left
            .iter()
            .filter(|id| dep_of(id).iter().all(|d| out.iter().any(|o| &name_of(o) == d) || !left.iter().any(|l| &name_of(l) == d)))
            .cloned()
            .collect();
        // a cycle: take what is left in its config order rather than dropping it
        let take = if ready.is_empty() { left.clone() } else { ready };
        left.retain(|id| !take.iter().any(|t| t == id));
        out.extend(take);
    }
    out
}

/// The fork rule (BUG-53, probe P0.9 case A2): a `--resume <id>` with flags
/// while <id>'s job row exists makes a fork, which keeps the history only after
/// its first turn. Every such step in a plan must come after the `rm` of that
/// row, or <id> must have no row at all. Checked on every plan in debug builds;
/// `apply` refuses the step at run time as well.
pub fn no_fork(snap: &Snapshot, plan: &Plan) -> bool {
    let rows: Vec<&SessionInfo> = members(snap)
        .into_iter()
        .flat_map(|(_, m)| m.session.iter().chain(m.extra_rows.iter()))
        .chain(snap.stowaways.iter())
        .collect();
    plan.steps.iter().enumerate().all(|(at, st)| {
        if classify(st) != Kind::Relaunch {
            return true;
        }
        let Some(sid) = resumed_id(st) else { return true };
        let row = rows.iter().find(|r| r.session_id == sid && r.short_id.is_some());
        match row.and_then(|r| r.short_id.as_deref()) {
            None => true,
            Some(short) => plan.steps[..at].iter().any(|p| classify(p) == Kind::Rm { short: short.to_string() }),
        }
    })
}

/// An issue the plan's own steps repair reads as such: `[INFO] will repair:`
/// rather than an error about the very thing the plan is about to fix.
fn mark_repairs(plan: &mut Plan) {
    let relabels = |id: &Id| plan.steps.iter().any(|s| s.id.as_ref() == Some(id) && classify(s) == Kind::Relaunch);
    let removes = |id: &Id| plan.steps.iter().any(|s| s.id.as_ref() == Some(id) && matches!(classify(s), Kind::Rm { .. }));
    let fixed: Vec<bool> = plan
        .warnings
        .iter()
        .map(|i| match (i.code.as_str(), &i.id) {
            ("labels.missing" | "name.drift" | "folder.moved", Some(id)) => relabels(id),
            ("copy.rows", Some(id)) => removes(id),
            _ => false,
        })
        .collect();
    for (i, f) in plan.warnings.iter_mut().zip(fixed) {
        if f && !i.message.starts_with("will repair: ") {
            i.severity = Severity::Info;
            i.message = format!("will repair: {}", i.message);
            i.fix = None;
        }
    }
}

fn dedup_issues(plan: &mut Plan) {
    let key = |i: &Issue| (i.code.clone(), i.id.clone(), i.message.clone());
    let mut seen = std::collections::HashSet::new();
    plan.refusals.retain(|i| seen.insert(key(i)));
    // an issue the gate kept as a warning and a member's own refusal name
    // once (acceptance F6: name.drift twice on a start of an adopted ship)
    plan.warnings.retain(|i| seen.insert(key(i)));
}

pub fn plan_stop(env: &Env, snap: &Snapshot, id: &Id, caller: &Caller) -> Plan {
    let mut plan = Plan { summary: format!("stop {id}"), ..Default::default() };
    let _ = caller;
    let Some((_, m)) = member(snap, id) else {
        refuse(&mut plan, "member.unknown", format!("{id} is not in any fleet config"), None, Some(id.clone()));
        return plan;
    };
    // its conversation runs in a process with no job (a `claude --resume`
    // by hand, an SDK host) while its own job is not live: nimitz cannot stop
    // it there (cli-18). A live job is stopped as ever, whatever else holds
    // an older conversation of its lineage
    let job_live = m.session.as_ref().is_some_and(|p| p.pid.is_some() && p.short_id.is_some());
    if let Some(h) = held_of(m).filter(|_| !job_live) {
        let i = held_issue(m, h);
        refuse(&mut plan, "session.interactive", i.message, i.fix, Some(id.clone()));
        return plan;
    }
    let Some(p) = m.session.as_ref().filter(|p| p.pid.is_some()) else {
        if plan.steps.is_empty() {
            refuse(&mut plan, "session.none", format!("{id} is not live"), None, Some(id.clone()));
        }
        return plan;
    };
    // a process with no job (an interactive 0.3.x lead, an SDK host): nimitz
    // can neither stop nor resume it there
    let Some(short) = &p.short_id else {
        refuse(
            &mut plan,
            "session.interactive",
            format!("{id} runs in a process with no job ({}, pid {}); end it in its own terminal", p.entrypoint.as_deref().unwrap_or(&p.kind), p.pid.unwrap_or(0)),
            Some("type /exit in its pane".into()),
            Some(id.clone()),
        );
        return plan;
    };
    plan.steps.push(Step {
        id: Some(id.clone()),
        cmd: crate::claude::stop(env, short),
        cwd: m.cwd.clone(),
        why: "stop through the native binary; the job row stays for a later resume".into(),
    });
    plan
}

/// `nimitz stop <short>`: stop one live background session by its job id or
/// session id. Allowed: a member's row (one of two live copies), a
/// stowaway, or a session anywhere that holds a fleet member's name (the
/// `name.taken` fix, BUG-08); never an unrelated session.
pub fn plan_stop_session(env: &Env, snap: &Snapshot, target: &str, caller: &Caller) -> Plan {
    let mut plan = Plan { summary: format!("stop session {target}"), ..Default::default() };
    let is = |r: &&SessionInfo| r.short_id.as_deref() == Some(target) || r.session_id == target;
    let all = members(snap);
    let mut hit: Option<(Option<Id>, SessionInfo, String)> = all
        .iter()
        .find_map(|(_, m)| m.session.iter().chain(m.extra_rows.iter()).find(is).map(|r| (Some(m.id.clone()), r.clone(), format!("stop this one session of {}; the others stay", m.id))))
        .or_else(|| snap.stowaways.iter().find(is).map(|r| (None, r.clone(), format!("stop the unlabelled session {:?}", r.name))));
    if hit.is_none() {
        // a session outside fleet scope that holds a member's address
        let rows = crate::evidence::rows_from_files(env, &mut crate::evidence::FileCache::default()).unwrap_or_default();
        let labels = crate::evidence::session_labels(&crate::evidence::events(env), &rows);
        if let Some(r) = rows.iter().find(|r| r.short_id.as_deref() == Some(target) || r.session_id == target) {
            let labelled_as = labels.get(&crate::evidence::row_key(r)).and_then(|l| l.get("NIMITZ_ID")).cloned();
            if let Some((_, m)) = all.iter().find(|(_, m)| m.expected_name == r.name) {
                hit = Some((None, r.info(Default::default()), format!("stop the session in {} that holds {}'s name {:?}", r.cwd, m.id, r.name)));
            } else if let Some((_, m)) = all.iter().find(|(_, m)| Some(&m.id.0) == labelled_as.as_ref()) {
                // BUG-52: a session carrying a member's labels in another folder
                hit = Some((None, r.info(Default::default()), format!("stop the session in {} that carries {}'s labels in the wrong folder", r.cwd, m.id)));
            }
        }
    }
    if hit.is_none() {
        // a job row a 0.3.x tender left (its identity is retired, section 2)
        if let Some((owner, r)) = retired_tender_row(env, target) {
            hit = Some((Some(owner.clone()), r, format!("stop the row of the 0.3.x tender {owner}; the role was removed in 0.4.0")));
        }
    }
    if hit.is_none() {
        // a job a 0.5/0.6 aide left running (pinned): its labels name an `@`
        // id, a role removed in 0.7.0 (`aide.retired`); the user's to stop
        if let Some((owner, r)) = retired_aide_row(env, target) {
            hit = Some((None, r, format!("stop the row of the 0.6 aide {owner}; aides were removed in 0.7.0")));
        }
    }
    let Some((owner, row, why)) = hit else {
        let msg = format!("{target} names no fleet member's session, no stowaway and no holder of a fleet name");
        refuse(&mut plan, "session.unknown", msg, Some("nimitz status".into()), None);
        return plan;
    };
    let _ = caller;
    let Some(short) = row.short_id.clone().filter(|_| row.pid.is_some()) else {
        refuse(&mut plan, "session.none", format!("session {} is not a live background session", row.session_id), None, owner);
        return plan;
    };
    plan.steps.push(Step { id: owner.clone(), cmd: crate::claude::stop(env, &short), cwd: None, why });
    // a copy goes with its stop (as `sweep` does): once stopped, a copy that
    // was never prompted has no name, no labels and no transcript, so no
    // nimitz command sees its row again (cli-N1)
    if row.copy.is_some() {
        plan.steps.push(Step { id: owner, cmd: crate::claude::rm(env, &short), cwd: None, why: format!("a copy's row goes with its stop ({}); the transcript stays", copy_how(&row)) });
    }
    plan
}

/// A deliberate relaunch: new id, every flag, old row removed (contracts.md section 8).
pub fn plan_relaunch(env: &Env, snap: &Snapshot, id: &Id, caller: &Caller) -> Plan {
    let mut plan = Plan { summary: format!("relaunch {id} (same session id and history; labels, name and mode restored)"), ..Default::default() };
    let _ = caller;
    let Some((fleet, m)) = member(snap, id) else {
        refuse(&mut plan, "member.unknown", format!("{id} is not in any fleet config"), None, Some(id.clone()));
        return plan;
    };
    let fod = match crate::config::load_fleet(env, fleet.id.fleet()) {
        Ok(f) => f,
        Err(i) => {
            plan.refusals.push(i);
            return plan;
        }
    };
    let mode = crate::config::permission_mode(&fod, id);
    let cwd = m.cwd.clone().unwrap_or_default();
    if refuse_legacy(&mut plan, m) {
        return plan;
    }
    // a relaunch makes a new live session: not while the name is held by
    // another live session, or the session file says nothing reliable (BUG-10)
    if let Some(e) = m.issues.iter().find(|i| i.severity == Severity::Error && ["name.taken", "job.unreadable", "labels.mismatch"].contains(&i.code.as_str()) && !(i.code == "labels.mismatch" && m.session.as_ref().map(|p| i.message.contains(&p.session_id)).unwrap_or(false))) {
        plan.refusals.push(e.clone());
        return plan;
    }
    if m.session.as_ref().map(|p| p.pid.is_none() && p.status.as_deref() == Some("unreadable")).unwrap_or(false) {
        refuse(&mut plan, "session.unreadable", format!("{id}'s session file cannot be read; its process may be live"), Some("nimitz status  # read it again in a moment".into()), Some(id.clone()));
        return plan;
    }
    let Some(p) = &m.session else {
        refuse(&mut plan, "session.none", format!("{id} has no session to relaunch; use nimitz start"), Some(format!("nimitz start {id}")), Some(id.clone()));
        return plan;
    };
    // its conversation runs in a process with no job: a relaunch would start
    // a copy; end that process first (cli-18: never `dup.live` with a
    // `nimitz stop <sid>` that cannot work)
    if let Some(h) = held_of(m) {
        let i = held_issue(m, h);
        let live = m.session.as_ref().is_some_and(|p| p.pid.is_some() && p.short_id.is_some());
        let fix = i.fix.map(|f| if live { format!("{f}, then {}", restart_cmd(id)) } else { f });
        refuse(&mut plan, "session.interactive", i.message, fix, Some(id.clone()));
        return plan;
    }
    // (a lead's live intruder is not its second session: it stays the
    // intruder, with its own fix, and the relaunch goes on)
    if let Some(r) = m.extra_rows.iter().find(|r| r.pid.is_some() && !lead_intruder_row(m, r)) {
        let short = r.short_id.as_deref().unwrap_or(&r.session_id);
        refuse(
            &mut plan,
            "dup.live",
            format!("{id} has another live session {short}; a relaunch would leave two"),
            Some(format!("nimitz stop {short}  # for the one you drop")),
            Some(id.clone()),
        );
        return plan;
    }
    if p.pid.is_some() && p.short_id.is_none() {
        refuse(&mut plan, "session.interactive", format!("{id} is live in a process with no job; end it in its own terminal first"), Some("type /exit in its pane".into()), Some(id.clone()));
        return plan;
    }
    // onto the conversation to wake (after a /branch or /resume the registry's later one, finding 11)
    plan.steps.extend(relabel_to(env, id, &mode, p, &cwd, &wake_target(env, id, p).session));
    debug_assert!(no_fork(snap, &plan), "a relaunch would fork a session: {:?}", plan.steps);
    plan
}

/// Give a session new labels, name and mode while keeping its id and its
/// conversation (BUG-53, found on real Claude Code): `--resume <id>` with flags
/// while the job row exists starts a copy with a new id and NO history; with the
/// row removed, the same command continues <id> with its history. So: stop it
/// if live, `rm` its row (native), then `--resume <id>` with every flag. If the
/// rm fails, `apply` stops there and nothing is resumed.
fn relabel_steps(env: &Env, id: &Id, mode: &str, p: &SessionInfo, cwd: &str) -> Vec<Step> {
    relabel_to(env, id, mode, p, cwd, &p.session_id)
}

/// `relabel_steps` onto `sid`: the conversation to resume with every flag
/// once the row is gone (its own, or a later one of the member's lineage).
fn relabel_to(env: &Env, id: &Id, mode: &str, p: &SessionInfo, cwd: &str, sid: &str) -> Vec<Step> {
    let step = |cmd: Vec<String>, why: String| Step { id: Some(id.clone()), cmd, cwd: Some(cwd.to_string()), why };
    let mut v = vec![];
    if let Some(short) = &p.short_id {
        if p.pid.is_some() {
            v.push(step(crate::claude::stop(env, short), "a live session is never resumed; stop it first".into()));
        }
        v.push(step(crate::claude::rm(env, short), format!("remove row {short} first: a resume with flags while it exists makes an empty copy")));
    }
    v.push(step(
        crate::claude::relaunch_bg(env, id, mode, sid),
        format!("resume {} with every flag: same id and history, new name and labels", &sid[..sid.len().min(8)]),
    ));
    v
}

/// How a member gets a folder on this machine: a ship's `ship set --dir`.
pub fn dir_fix(id: &Id) -> String {
    format!("nimitz ship set {id} --dir <folder|managed>")
}

/// What retires a member for good: its config's `rm`.
fn retire_hint(id: &Id) -> String {
    match id.role() {
        Role::Admiral => format!("nimitz fleet rm {id}"),
        Role::Commodore => format!("nimitz squadron rm {id}"),
        Role::Ship => format!("nimitz ship rm {id}"),
    }
}

/// Remove job rows: a member's stale rows (`target` = identity), or one row of
/// a stowaway / non-fleet session (`target` = short id or session id).
pub fn plan_rm(env: &Env, snap: &Snapshot, target: &str, caller: &Caller) -> Plan {
    let mut plan = Plan { summary: format!("rm {target}"), ..Default::default() };
    // An identity: its stale rows (same as sweep <id>).
    if let Some((_, m)) = member(snap, &Id(target.to_string())) {
        sweep_member(env, &mut plan, m);
        if plan.steps.is_empty() && plan.refusals.is_empty() {
            let retire = retire_hint(&m.id);
            let own = m.session.as_ref().map(|p| format!("; its own row {} goes with the member", p.short_id.as_deref().unwrap_or(&p.session_id))).unwrap_or_default();
            plan.warnings.push(issue(
                "copy.rows",
                Severity::Info,
                format!("{target} has no stale rows{own}"),
                m.session.as_ref().map(|_| format!("{}  # replaces the row (same conversation); {retire} retires the member", restart_cmd(&m.id))),
                Some(m.id.clone()),
            ));
        }
        return plan;
    }
    // A row: stowaways and members' rows, by short id or session id.
    let binding = members(snap);
    let all: Vec<&SessionInfo> = binding
        .iter()
        .flat_map(|(_, m)| m.session.iter().chain(m.extra_rows.iter()))
        .chain(snap.stowaways.iter())
        .collect();
    // also a session carrying a member's labels in another folder (BUG-54: the
    // labels.mismatch fix): not that member's row, so not in the lists above
    let misplaced: Option<SessionInfo> = {
        let rows = crate::evidence::rows_from_files(env, &mut crate::evidence::FileCache::default()).unwrap_or_default();
        let labels = crate::evidence::session_labels(&crate::evidence::events(env), &rows);
        rows.iter()
            .find(|r| r.short_id.as_deref() == Some(target) || r.session_id == target)
            .filter(|r| labels.get(&crate::evidence::row_key(r)).and_then(|l| l.get("NIMITZ_ID")).map(|o| binding.iter().any(|(_, m)| &m.id.0 == o)).unwrap_or(false))
            .map(|r| r.info(Default::default()))
    };
    // a job row a 0.3.x tender left: history only, removed by its short id
    let retired = retired_tender_row(env, target);
    // and one a 0.5/0.6 aide left (`aide.retired`): the user's
    let aide = retired_aide_row(env, target);
    let Some(row) = all.iter().copied().find(|r| r.short_id.as_deref() == Some(target) || r.session_id == target).or(misplaced.as_ref()).or(retired.as_ref().map(|(_, r)| r)).or(aide.as_ref().map(|(_, r)| r)) else {
        refuse(&mut plan, "session.unknown", format!("{target} names no identity and no job row"), None, None);
        return plan;
    };
    // whose row: a member's (its session or a copy), else the labels it carries (misplaced)
    let owner: Option<Id> = binding
        .iter()
        .find(|(_, m)| m.session.iter().chain(m.extra_rows.iter()).any(|r| r.session_id == row.session_id))
        .map(|(_, m)| m.id.clone())
        .or_else(|| {
            let rows = crate::evidence::rows_from_files(env, &mut crate::evidence::FileCache::default()).unwrap_or_default();
            let labels = crate::evidence::session_labels(&crate::evidence::events(env), &rows);
            labels.get(&row.short_id.clone().unwrap_or_else(|| row.session_id.clone())).and_then(|l| l.get("NIMITZ_ID")).map(|o| Id(o.clone())).filter(|_| misplaced.as_ref().is_some_and(|m| m.session_id == row.session_id))
        })
        .or_else(|| retired.as_ref().map(|(id, _)| id.clone()));
    let _ = (caller, owner);
    if row.pid.is_some() {
        refuse(
            &mut plan,
            "session.live",
            format!("session {} is live; stop it first", row.session_id),
            Some(format!("nimitz stop {}", row.short_id.as_deref().unwrap_or(&row.session_id))),
            None,
        );
        return plan;
    }
    let current = binding.iter().find(|(_, m)| m.session.as_ref().map(|p| p.session_id == row.session_id).unwrap_or(false)).map(|(_, m)| m.id.clone());
    if let Some(member) = current {
        // a member's own row goes with the member: a relaunch replaces it, a
        // retire removes it (acceptance env F7)
        let retire = retire_hint(&member);
        refuse(
            &mut plan,
            "lineage.current",
            format!("session {} is {member}'s current session; its row goes with the member, never alone", row.session_id),
            Some(format!("{}  # replaces the row (same conversation); {retire} retires the member", restart_cmd(&member))),
            Some(member.clone()),
        );
        return plan;
    }
    let Some(short) = &row.short_id else {
        refuse(&mut plan, "session.none", format!("session {} has no job row to remove", row.session_id), None, None);
        return plan;
    };
    plan.steps.push(Step { id: None, cmd: crate::claude::rm(env, short), cwd: None, why: "remove the job row; the transcript stays".into() });
    plan
}

/// The job row of a 0.3.x tender, by short id or session id: its labels name
/// `F/SQ/~task`, or the registry's `retired` map has the session. The owner
/// is the retired identity.
fn retired_tender_row(env: &Env, target: &str) -> Option<(Id, SessionInfo)> {
    let rows = crate::evidence::rows_from_files(env, &mut crate::evidence::FileCache::default()).unwrap_or_default();
    let labels = crate::evidence::session_labels(&crate::evidence::events(env), &rows);
    let r = rows.iter().find(|r| r.short_id.as_deref() == Some(target) || r.session_id == target)?;
    let by_labels = labels.get(&crate::evidence::row_key(r)).and_then(|l| l.get("NIMITZ_ID")).filter(|o| crate::migrate::is_tender_id(o)).cloned();
    let owner = by_labels.or_else(|| {
        let reg = crate::registry::load(env);
        reg.retired
            .iter()
            .find(|(_, rec)| rec.job.as_deref() == r.short_id.as_deref() && rec.job.is_some() || rec.session_ids().any(|s| r.session_ids().any(|x| x == s)))
            .map(|(id, _)| id.clone())
    })?;
    Some((Id(owner), r.info(labels.get(&crate::evidence::row_key(r)).cloned().unwrap_or_default())))
}

fn sweep_member(env: &Env, plan: &mut Plan, m: &Member) {
    for r in &m.extra_rows {
        // a copy (plan-v4 section 6): a background job is stopped and removed;
        // a process with no job is ended in its terminal, not by nimitz
        if r.copy.is_some() {
            match &r.short_id {
                Some(short) => {
                    let why = format!("copy of {}: {}", m.id, copy_how(r));
                    if r.pid.is_some() {
                        plan.steps.push(Step { id: Some(m.id.clone()), cmd: crate::claude::stop(env, short), cwd: None, why: format!("{why}; stop it first") });
                    }
                    plan.steps.push(Step { id: Some(m.id.clone()), cmd: crate::claude::rm(env, short), cwd: None, why: format!("{why}; the transcript stays") });
                }
                None => plan.warnings.push(issue(
                    "copy",
                    Severity::Warn,
                    format!("{}'s copy {} ({}) runs in a process with no job; sweep cannot end it", m.id, r.session_id, r.name),
                    Some(copy_fix(m, r)),
                    Some(m.id.clone()),
                )),
            }
            continue;
        }
        if r.pid.is_some() {
            plan.warnings.push(issue(
                "dup.live",
                Severity::Warn,
                format!("{}'s extra session {} is live; sweep skips it", m.id, r.session_id),
                Some(format!("nimitz stop {}", r.short_id.as_deref().unwrap_or(&r.session_id))),
                Some(m.id.clone()),
            ));
            continue;
        }
        if let Some(short) = &r.short_id {
            plan.steps.push(Step {
                id: Some(m.id.clone()),
                cmd: crate::claude::rm(env, short),
                cwd: None,
                why: format!("stale row of {}; one row per identity", m.id),
            });
        }
    }
}

/// Drop every stale copy row of the members `ids` names (the CLI resolves
/// its paths into them); none: a member caller's own scope (itself and
/// everything under it), else everything.
pub fn plan_sweep(env: &Env, snap: &Snapshot, ids: Option<&[Id]>, caller: &Caller) -> Plan {
    let under = |m: &Id, t: &Id| m == t || m.0.starts_with(&format!("{}/", t.0));
    let own = match caller {
        Caller::Member { id: me, .. } if me.well_formed() => Some(me.clone()),
        _ => None,
    };
    let scope = match (ids, &own) {
        (Some(v), _) => v.iter().map(|i| i.0.clone()).collect::<Vec<_>>().join(" "),
        (None, Some(me)) => me.0.clone(),
        (None, None) => "everything".into(),
    };
    let mut plan = Plan { summary: format!("sweep {scope}"), ..Default::default() };
    let in_scope = |m: &Id| match (ids, &own) {
        (Some(v), _) => v.contains(m),
        (None, Some(me)) => under(m, me),
        (None, None) => true,
    };
    for (_, m) in members(snap) {
        if in_scope(&m.id) {
            sweep_member(env, &mut plan, m);
        }
    }
    // Stopped rows of the old fleet (Info `legacy`): offered here, never removed
    // by a start. The transcripts stay.
    let (fleets, _) = crate::config::load_all(env);
    let legacy: HashMap<String, Id> = fleets.iter().flat_map(crate::config::legacy_map).collect();
    for st in snap.stowaways.iter().filter(|s| s.pid.is_none()) {
        let (Some(owner), Some(short)) = (legacy.get(&st.name), st.short_id.as_ref()) else { continue };
        if in_scope(owner) && member(snap, owner).is_some() {
            plan.steps.push(Step {
                id: Some(owner.clone()),
                cmd: crate::claude::rm(env, short),
                cwd: None,
                why: format!("the old fleet's stopped row {short} ({}) of {owner}; its transcript stays", st.name),
            });
        }
    }
    plan
}

/// Adopt a stowaway: record it as `id`'s session. Labels can only be given at a
/// start, so the adoption binds the registry now and warns that the labels
/// arrive at the next relaunch.
pub fn plan_adopt(env: &Env, snap: &Snapshot, session_id: &str, id: &Id, caller: &Caller) -> Plan {
    let _ = env;
    let mut plan = Plan { summary: format!("adopt {session_id} as {id}"), ..Default::default() };
    let _ = caller;
    let Some(row) = snap.stowaways.iter().find(|r| r.session_id == session_id || r.short_id.as_deref() == Some(session_id)) else {
        refuse(
            &mut plan,
            "session.unknown",
            format!("{session_id} is not a stowaway (only unlabelled sessions in fleet scope are adopted)"),
            None,
            Some(id.clone()),
        );
        return plan;
    };
    let found = member(snap, id);
    if found.is_none() {
        refuse(&mut plan, "member.unknown", format!("{id} is not in any fleet config; add it first"), Some(format!("nimitz ship new {id} --dir <folder|managed> --prompt <text>")), Some(id.clone()));
        return plan;
    }
    if found.map(|(_, m)| m.session.as_ref().map(|p| p.pid.is_some()).unwrap_or(false)).unwrap_or(false) {
        refuse(&mut plan, "dup.live", format!("{id} already has a live session"), None, Some(id.clone()));
        return plan;
    }
    plan.warnings.push(issue(
        "labels.missing",
        Severity::Warn,
        format!("{} has no labels; they arrive at its next relaunch", row.session_id),
        Some(restart_cmd(id)),
        Some(id.clone()),
    ));
    plan.steps.push(Step {
        id: Some(id.clone()),
        cmd: vec!["bind".into(), id.0.clone(), row.session_id.clone(), "adopt".into()],
        cwd: Some(row.cwd.clone()),
        why: "record the session as this identity's current (registry write, no process)".into(),
    });
    plan
}

/// Retire a ship, a squadron or a fleet (`ship rm`, `squadron rm`, `fleet rm`):
/// stop its live background sessions, remove its job rows (transcripts stay),
/// then the config change (`config::plan_rm`), in one plan that `apply` runs and
/// verifies. A squadron or fleet goes only with nothing live in it and no ships
/// left in the config; a live interactive member is ended in its own pane.
pub fn plan_retire(env: &Env, snap: &Snapshot, caller: &Caller, id: &Id) -> Plan {
    let mut plan = Plan { summary: format!("retire {id}: stop, remove job rows, remove from the config"), ..Default::default() };
    let _ = caller;
    let cfg = crate::config::plan_rm(env, id);
    plan.refusals.extend(cfg.refusals.iter().cloned());
    let under = |m: &Id| m == id || m.0.starts_with(&format!("{id}/"));
    for (_, m) in members(snap).into_iter().filter(|(_, m)| under(&m.id)) {
        for r in m.session.iter().chain(m.extra_rows.iter()) {
            let live = r.pid.is_some();
            if live && (r.kind != "background" || !matches!(id.role(), Role::Ship)) {
                // a fleet or squadron is removed only with nothing live in it
                refuse(
                    &mut plan,
                    "session.live",
                    format!("{} is live; {id} is removed only with nothing live in it", m.id),
                    Some(if r.kind == "background" { format!("nimitz stop {}", m.id) } else { "type /exit in its pane".into() }),
                    Some(m.id.clone()),
                );
                continue;
            }
            let Some(short) = &r.short_id else { continue }; // an interactive session leaves no job row
            if live {
                plan.steps.push(Step { id: Some(m.id.clone()), cmd: crate::claude::stop(env, short), cwd: m.cwd.clone(), why: "stop it first".into() });
            }
            plan.steps.push(Step { id: Some(m.id.clone()), cmd: crate::claude::rm(env, short), cwd: None, why: "remove the job row; the transcript stays".into() });
        }
    }
    plan.steps.extend(cfg.steps);
    dedup_issues(&mut plan);
    plan
}

// ------------------------------------------------------------------ apply

/// What a step is, read back from its argv.
#[derive(Debug, PartialEq)]
enum Kind {
    Bind,
    /// a config step (`mkdir`, `write`, `archive`), run by `config::apply_config_plan`
    Config,
    /// a command `apply` never runs (`claude attach`, a step with no `--bg`):
    /// the multiplexer's, or no plan of core's
    Other,
    Launch,
    Resume { session_id: String },
    Relaunch,
    Stop { short: String },
    Rm { short: String },
}

fn classify(step: &Step) -> Kind {
    let c: Vec<&str> = step.cmd.iter().map(String::as_str).collect();
    match c.as_slice() {
        ["bind", ..] => Kind::Bind,
        ["mkdir", _] | ["write", _, _] | ["archive", _] | ["trust", _] => Kind::Config,
        [_, "stop", short] => Kind::Stop { short: short.to_string() },
        [_, "rm", short] => Kind::Rm { short: short.to_string() },
        _ if !step.cmd.contains(&"--bg".to_string()) => Kind::Other,
        _ => match step.cmd.iter().position(|a| a == "--resume") {
            Some(i) if step.cmd.len() == i + 2 && !step.cmd.iter().any(|a| a == "--settings") => {
                Kind::Resume { session_id: step.cmd[i + 1].clone() }
            }
            Some(_) => Kind::Relaunch,
            None => Kind::Launch,
        },
    }
}

/// The fix for a failed launch, resume or relaunch: the trust prompt when
/// Claude Code said the folder is not trusted (its text: "Workspace not
/// trusted. Run `claude` in <dir> once"); after an `rm` of the row, the start
/// that resumes from the transcript.
fn failure_fix(step: &Step, text: &str, after_rm: bool) -> Option<String> {
    let t = text.to_lowercase();
    let trust = (t.contains("workspace not trusted") || t.contains("not trusted"))
        .then(|| crate::check::untrusted_fix(std::path::Path::new(step.cwd.as_deref().unwrap_or("."))));
    let resume = step.id.as_ref().filter(|_| after_rm).map(|id| format!("nimitz start {id}  # resumes from its transcript"));
    match (trust, resume) {
        (Some(t), Some(r)) => Some(format!("{t}; then {r}")),
        (t, r) => t.or(r),
    }
}

/// Why Claude Code started a copy, when the job was not live: it could not
/// check (issue #6).
struct CopyCause {
    why: String,
    fix: String,
}

/// The cause of a copy that is no race with the daemon (issue #6): a session
/// file Claude Code cannot parse (2.1.296 then cannot check whether any session
/// runs, and every resume is a copy), or its own note that it could not check.
/// None: the copy says the job was live or restarting.
fn copy_cause(env: &Env, text: &str) -> Option<CopyCause> {
    let plain = crate::claude::plain(text);
    let note = plain.lines().find(|l| l.contains("could not check")).map(|l| l.trim().to_string());
    let bad = crate::evidence::unparseable_sessions_settled(env);
    let said = note.as_ref().map(|n| format!(" (Claude Code: {n})")).unwrap_or_default();
    if !bad.is_empty() {
        let files: Vec<String> = bad.iter().map(|b| b.describe()).collect();
        let fix: Vec<String> = bad.iter().map(|b| b.fix()).collect();
        return Some(CopyCause {
            why: format!("it could not check whether the job is running, because it cannot parse the session file {}{said}", files.join(", ")),
            fix: fix.join("; "),
        });
    }
    note.map(|n| CopyCause {
        why: format!("it could not check whether the job is running (Claude Code: {n})"),
        fix: "nimitz doctor  # names a session file Claude Code cannot parse".into(),
    })
}

/// The session id a `--resume <id>` step continues.
fn resumed_id(step: &Step) -> Option<String> {
    let i = step.cmd.iter().position(|a| a == "--resume")?;
    step.cmd.get(i + 1).cloned()
}

/// How long `apply` waits for a background session's SessionStart event.
pub const BIND_TIMEOUT_S: f64 = 15.0;

/// Wait for the SessionStart event that binds `id` to a session, appended at or
/// after `since`. Returns the session id and the event's `source`.
fn wait_for_binding(env: &Env, id: &Id, since: f64, timeout_s: f64, cwd: Option<&str>) -> Option<(String, Option<String>)> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(timeout_s);
    let mut cache = crate::evidence::FileCache::default();
    loop {
        let rows = crate::evidence::rows_from_files(env, &mut crate::evidence::FileCache::default()).unwrap_or_default();
        // an impostor starting elsewhere with the same labels does not bind (BUG-55)
        let elsewhere = |sid: &str| match (cwd, rows.iter().find(|r| r.session_id == sid)) {
            (Some(c), Some(r)) => !same_folder(&r.cwd, c),
            _ => false,
        };
        for e in crate::evidence::events_cached(env, &mut cache).iter().rev() {
            // a job's line counts only when its row backs the labels (the line
            // keeps the env's labels; a polluted daemon's job carries them too)
            // (a line written while the session file was unreadable names no
            // job: its job comes from the rows before its labels are trusted)
            let backed = || {
                let job = e.job.clone().or_else(|| e.session_id.as_deref().and_then(|sid| rows.iter().find(|r| r.session_id == sid)).and_then(|r| r.short_id.clone()));
                job.is_none() || crate::evidence::job_backs_labels(&env.claude_home, job.as_deref(), &e.labels) == Some(true)
            };
            if e.ts >= since && e.event == "SessionStart" && e.labels.get("NIMITZ_ID") == Some(&id.0) && backed() {
                if let Some(sid) = e.session_id.as_ref().filter(|s| !elsewhere(s)) {
                    return Some((sid.clone(), e.source.clone()));
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
}

/// Bind one started member and verify it: wait for its SessionStart event (at
/// or after `since`), check a resume kept `expect` and found its history (a
/// SessionStart `startup` instead of `resume` means Claude Code lost it:
/// `history.lost`, never verified), record the id in the registry (`via` =
/// launch | resume | relaunch | migrate), then re-read the files: live, named
/// right, labelled right. `apply` calls this for every start.
pub fn bind_started(
    env: &Env,
    id: &Id,
    cwd: Option<&str>,
    since: f64,
    expect: Option<&str>,
    via: &str,
    timeout_s: f64,
) -> Result<String, Vec<Issue>> {
    let Some((sid, source)) = wait_for_binding(env, id, since, timeout_s, cwd) else {
        return Err(vec![issue(
            "lineage.broken",
            Severity::Error,
            format!("{id} started but no SessionStart event bound it within {timeout_s}s"),
            Some("check the hooks: nimitz doctor".into()),
            Some(id.clone()),
        )]);
    };
    if let Some(want) = expect.filter(|w| *w != sid) {
        return Err(vec![issue(
            "lineage.broken",
            Severity::Error,
            format!("{id}'s resume was expected to keep {want} but bound {sid} (a copy; flags on the resume?)"),
            Some(format!("nimitz sweep {id}  # drop the copy, then nimitz start {id}")),
            Some(id.clone()),
        )]);
    }
    if expect.is_some() && source.as_deref() == Some("startup") {
        return Err(vec![issue(
            "history.lost",
            Severity::Error,
            format!("{id}'s resume of {sid} started a new conversation (SessionStart `startup`, not `resume`): Claude Code did not find its history"),
            Some(format!("nimitz stop {id}  # it runs with no history; then nimitz start {id} --fresh, or find the transcript first")),
            Some(id.clone()),
        )]);
    }
    let (role, name) = (id.role(), id.name());
    let cwd_s = cwd.unwrap_or_default().to_string();
    // the job that runs it: the hook bound it already unless another job of
    // the identity still has its row (an impostor, a stale copy); nimitz knows
    // better here, it started this one
    let job = find_row_by_session(env, &sid).and_then(|r| r.short_id);
    if let Err(e) = crate::registry::update(env, |r| crate::registry::bind_job(r, id, role, &name, &cwd_s, job.as_deref(), &sid, via, crate::evidence::now())) {
        return Err(vec![issue("lineage.broken", Severity::Error, format!("registry write failed: {e}"), None, Some(id.clone()))]);
    }
    claim_folder(env, id, cwd, &sid);
    let problems = verify_live(env, id, &sid, cwd);
    if problems.is_empty() { Ok(sid) } else { Err(problems) }
}

/// Re-read the files: `sid` is live, carries `id`'s name and `id`'s labels.
fn verify_live(env: &Env, id: &Id, sid: &str, cwd: Option<&str>) -> Vec<Issue> {
    let mut out = vec![];
    let mut cache = crate::evidence::FileCache::default();
    let rows = crate::evidence::rows_from_files(env, &mut cache).unwrap_or_default();
    let labels = crate::evidence::labels_by_session(&crate::evidence::events(env));
    let Some(row) = rows.iter().find(|r| r.session_id == sid) else {
        return vec![issue("lineage.broken", Severity::Error, format!("{id}'s new session {sid} left no row"), None, Some(id.clone()))];
    };
    if !row.live() {
        out.push(issue("lineage.broken", Severity::Error, format!("{id}'s new session {sid} is not live"), None, Some(id.clone())));
    }
    if let Some(c) = cwd.filter(|c| !same_folder(&row.cwd, c)) {
        out.push(issue(
            "labels.mismatch",
            Severity::Error,
            format!("{id}'s session {sid} runs in {}, not its folder {c}", row.cwd),
            Some(format!("nimitz stop {}", row.short_id.as_deref().unwrap_or(sid))),
            Some(id.clone()),
        ));
    }
    if row.name != id.name() {
        out.push(issue(
            "name.drift",
            Severity::Error,
            format!("{id} started as {:?}, expected {:?}", row.name, id.name()),
            Some(restart_cmd(id)),
            Some(id.clone()),
        ));
    }
    // a job's labels are its row's saved ones (the env's on the line may be inherited)
    let have = if row.short_id.is_some() { Some(row.job_labels.clone()) } else { labels.get(sid).cloned() };
    if have.map(|l| l.get("NIMITZ_ID") != Some(&id.0)).unwrap_or(true) {
        out.push(issue(
            "labels.missing",
            Severity::Error,
            format!("{id}'s new session {sid} carries no (or foreign) labels"),
            Some(restart_cmd(id)),
            Some(id.clone()),
        ));
    }
    out
}

/// Run a plan's steps, then re-read state and verify (never trust exit codes alone).
pub fn apply(env: &Env, plan: &Plan) -> Outcome {
    let mut out = Outcome::default();
    // commands run beyond the plan's own steps (a raced wake's copy removal
    // and its one retry): `verified` counts the plan's steps only
    let mut extra_ran = 0usize;
    if !plan.ok() {
        out.issues = plan.refusals.clone();
        return out;
    }
    // One start at a time per identity (BUG-02/03): hold each identity's lock
    // through launch, bind and verify. A second `nimitz start` waits here, then
    // finds the member live and does nothing.
    let mut lock_ids: Vec<Id> = plan.steps.iter().filter_map(|s| s.id.clone()).collect();
    lock_ids.sort();
    lock_ids.dedup();
    let mut waited: Vec<Id> = vec![];
    let _locks: Vec<IdLock> = match lock_ids
        .iter()
        .map(|id| {
            lock_identity(env, id).map(|(l, w)| {
                if w {
                    waited.push(id.clone());
                }
                l
            })
        })
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(l) => l,
        Err(e) => {
            out.issues.push(issue("step.failed", Severity::Error, format!("cannot take the identity lock: {e}"), Some("nimitz doctor".into()), None));
            return out;
        }
    };
    let mut skipped: Vec<Id> = vec![];
    // Re-plan under the lock: when another nimitz held an identity's lock, it
    // may have started or relaunched it meanwhile. If that member is now live,
    // labelled and named right, redoing stop, rm and resume would only undo the
    // other one's work: skip it ("already relaunched").
    for id in &waited {
        let done = live_session_of(env, id, step_folder(plan, id)).map(|r| r.name == id.name()).unwrap_or(false);
        let relaunching = plan.steps.iter().any(|s| s.id.as_ref() == Some(id) && matches!(classify(s), Kind::Relaunch | Kind::Stop { .. } | Kind::Rm { .. }));
        if done && relaunching {
            skipped.push(id.clone());
        }
    }
    // The crash window (plan-v4 section 5): a job in state `crashed`, or
    // `working` with no live pid, is usually restarted by the daemon within
    // seconds. Wait for that before any resume of it (a resume of a job the
    // daemon then restarts makes a copy, cli-31); restarted, its steps are
    // skipped.
    // (identity, the conversation resumed, the job's row): the row that
    // carries the conversation; when none does (a relabel after a /branch
    // resumes a conversation no row carries), the plan's own stop or rm step
    // names the job. Never the other way round: a plan may also rm a stale
    // copy row of the same identity, which is not the job to wait for.
    let short_of = |id: &Id, sid: &str| {
        find_row_by_session(env, sid).and_then(|r| r.short_id).or_else(|| {
            plan.steps.iter().filter(|s| s.id.as_ref() == Some(id)).find_map(|s| match classify(s) {
                Kind::Rm { short } | Kind::Stop { short } => Some(short),
                _ => None,
            })
        })
    };
    let waking: Vec<(Id, String, Option<String>)> = plan
        .steps
        .iter()
        .filter_map(|s| s.id.clone().filter(|i| !skipped.contains(i)).map(|i| (i, s)))
        .filter_map(|(i, s)| match classify(s) {
            Kind::Resume { .. } | Kind::Relaunch => resumed_id(s).map(|sid| (i, sid)),
            _ => None,
        })
        .collect::<std::collections::BTreeMap<_, _>>()
        .into_iter()
        .map(|(i, sid)| {
            let short = short_of(&i, &sid);
            (i, sid, short)
        })
        .collect();
    // Claude Code's daemon settles a stopped job a moment after its process
    // ended: a `--bg --resume` inside that moment is "already running in the
    // background" and starts a copy (r3-e2e finding 12). Before a wake, wait
    // (briefly) until `claude agents` no longer lists the job as running.
    for (_, _, short) in &waking {
        if let Some(job) = short.as_deref().filter(|j| find_row(env, j).is_some_and(|r| !r.live())) {
            wait_settled(env, job, SETTLE_WAIT_S);
        }
    }
    let mut restarted: Vec<Id> = vec![];
    let by_job: Vec<(Id, String)> = waking.iter().filter_map(|(i, _, short)| short.clone().map(|s| (i.clone(), s))).collect();
    for (id, what) in wait_crash_window(env, &by_job) {
        restarted.push(id.clone());
        for step in plan.steps.iter().filter(|s| s.id.as_ref() == Some(&id)) {
            out.ran.push((step.clone(), 0, format!("skipped: {what}")));
        }
    }
    // A job the daemon did not restart in the window: its transcript must be
    // there now (`wake_steps` lets a crashed job through without one), else the
    // resume would run with no history: `history.lost` with the `--fresh` fix
    for (id, sid, short) in waking.iter().filter(|(i, _, _)| !restarted.contains(i)) {
        // a native resume names the birth id and wakes the row's conversation;
        // a relabel names the conversation itself
        let now = match short.as_deref().and_then(|j| find_row(env, j)) {
            Some(r) if r.session_ids().any(|s| s == sid) => r.session_id,
            _ => sid.clone(),
        };
        if crate::evidence::transcript(env, &mut crate::evidence::FileCache::default(), step_folder(plan, id), &now).is_some() {
            continue;
        }
        let sid = &now;
        for step in plan.steps.iter().filter(|s| s.id.as_ref() == Some(id)) {
            out.ran.push((step.clone(), -1, "not run: its conversation has no transcript".into()));
        }
        out.issues.push(issue(
            "history.lost",
            Severity::Error,
            format!("{id}'s conversation {sid} has no transcript in any project dir, and the daemon did not restart its job; a resume would start with no history"),
            Some(format!("nimitz start {id} --fresh  # a cold launch; the lineage starts a new entry")),
            Some(id.clone()),
        ));
        return out;
    }
    let mut started: Vec<(Id, String, Option<String>)> = vec![]; // (identity, session id, folder) to verify again
    for (at, step) in plan.steps.iter().enumerate() {
        let kind = classify(step);
        if step.id.as_ref().map(|i| restarted.contains(i)).unwrap_or(false) {
            continue; // reported above
        }
        if step.id.as_ref().map(|i| skipped.contains(i)).unwrap_or(false) {
            out.ran.push((step.clone(), 0, "skipped: already relaunched (or started) by a concurrent nimitz".into()));
            continue;
        }
        // Re-plan under the lock: a launch for a member that is live by now, or a
        // resume of a session that is live by now, would make a second agent.
        if matches!(kind, Kind::Launch | Kind::Relaunch | Kind::Resume { .. }) {
            if let Some(id) = &step.id {
                let live_now = match &kind {
                    Kind::Resume { session_id } => live_row(env, |r| &r.session_id == session_id).is_some() || live_session_of(env, id, step.cwd.as_deref()).is_some(),
                    _ => live_session_of(env, id, step.cwd.as_deref()).is_some(),
                };
                if live_now {
                    skipped.push(id.clone());
                    out.ran.push((step.clone(), 0, format!("skipped: {id} is live already (a concurrent start); nothing to do")));
                    continue;
                }
            }
        }
        if kind == Kind::Other {
            out.ran.push((step.clone(), -1, "not run".into()));
            out.issues.push(issue("step.failed", Severity::Error, format!("not a step apply runs: {}", step.cmd.join(" ")), None, step.id.clone()));
            return out;
        }
        if kind == Kind::Config {
            let one = Plan { summary: step.why.clone(), steps: vec![step.clone()], ..Default::default() };
            let o = crate::config::apply_config_plan(env, &one);
            let ok = o.verified && o.issues.iter().all(|i| i.severity != Severity::Error);
            out.ran.push((step.clone(), if ok { 0 } else { -1 }, o.ran.first().map(|r| r.2.clone()).unwrap_or_default()));
            if !ok {
                out.issues.extend(o.issues);
                return out;
            }
            continue;
        }
        if kind == Kind::Bind {
            let (id, sid, via) = (Id(step.cmd[1].clone()), step.cmd[2].clone(), step.cmd[3].clone());
            let (role, name) = (id.role(), id.name());
            let cwd = step.cwd.clone().unwrap_or_default();
            let job = find_row_by_session(env, &sid).and_then(|r| r.short_id);
            match crate::registry::update(env, |r| crate::registry::bind_job(r, &id, role, &name, &cwd, job.as_deref(), &sid, &via, crate::evidence::now())) {
                Ok(()) => out.ran.push((step.clone(), 0, format!("bound {sid} to {id} (via {via})"))),
                Err(e) => {
                    out.ran.push((step.clone(), -1, e.to_string()));
                    out.issues.push(issue("lineage.broken", Severity::Error, format!("registry write failed: {e}"), None, Some(id)));
                    return out;
                }
            }
            continue;
        }
        if kind == Kind::Relaunch {
            if let Some(r) = resumed_id(step).and_then(|sid| crate::evidence::rows_from_files(env, &mut crate::evidence::FileCache::default()).unwrap_or_default().into_iter().find(|r| r.session_id == sid && r.short_id.is_some())) {
                out.ran.push((step.clone(), -1, "not run".into()));
                out.issues.push(issue(
                    "step.failed",
                    Severity::Error,
                    format!("the job row {} of {} still exists; a resume with flags would start an empty copy", r.short_id.unwrap_or_default(), r.session_id),
                    step.id.as_ref().map(|i| restart_cmd(i)),
                    step.id.clone(),
                ));
                return out;
            }
        }
        // rm-then-resume (BUG-53) is two steps; if nimitz dies between them the
        // row is gone and only the transcript and the registry remain. A marker
        // says so to the next start ("recovering an interrupted relaunch").
        if let (Kind::Rm { .. }, Some(id), Some(next)) = (&kind, &step.id, plan.steps.get(at + 1)) {
            if classify(next) == Kind::Relaunch && next.id.as_ref() == Some(id) {
                if let Some(sid) = resumed_id(next) {
                    write_pending(env, id, &sid);
                }
            }
        }
        // the worker a stop ends: its process, waited for below (its row
        // goes before the process has exited)
        let stopping = match &kind {
            Kind::Stop { short } => find_row(env, short).and_then(|r| r.pid),
            _ => None,
        };
        let since = crate::evidence::now() - 0.001;
        let (mut rc, mut text) = crate::claude::run(&step.cmd, step.cwd.as_deref());
        let mut copy_removed: Option<bool> = None;
        // a resume of a conversation whose worker is still exiting is a copy
        // (measured on 2.1.295: stop, rm, `--bg --resume <id>` with flags
        // started a copy now and then); the next step waits for the process
        if let (0, Some(pid)) = (rc, stopping) {
            worker_gone_within(pid, STOP_EXIT_S);
        }
        out.ran.push((step.clone(), rc, text.clone()));
        // a native wake that raced the daemon's bookkeeping (a stop a moment
        // before, under load: r3-e2e finding 12) answers with a copy: the copy
        // goes, the job is waited for (its row settled, then a breath), and the
        // wake runs once more; a second copy is the failure below
        // (a resume with every flag after an rm, a relaunch, the same)
        let wake_of = match &kind {
            Kind::Resume { session_id } => Some(session_id.clone()),
            Kind::Relaunch => resumed_id(step),
            _ => None,
        };
        // a copy Claude Code started because it could not check the job at
        // all (issue #6: another session's file it cannot parse) is no race:
        // a second wake is only a second copy
        let mut cause: Option<CopyCause> = None;
        if rc == 0 {
            if let (Some(session_id), Some(copy)) = (wake_of.as_ref(), crate::claude::copy_started(&text)) {
                cause = copy_cause(env, &text);
                let again = if cause.is_some() { "" } else { "; then wake once more" };
                for cmd in [crate::claude::stop(env, &copy), crate::claude::rm(env, &copy)] {
                    let (rc2, t) = crate::claude::run(&cmd, None);
                    out.ran.push((Step { id: step.id.clone(), cmd, cwd: None, why: format!("remove the copy {copy} Claude Code started instead of the wake{again}") }, rc2, t));
                    extra_ran += 1;
                }
                let gone = row_gone_within(env, &copy, 3.0);
                copy_removed = Some(gone);
                // the daemon restarted the job itself meanwhile (cli-31): no
                // second wake (it would be a copy); the failure below says so
                let live = || find_row_by_session(env, session_id).is_some_and(|r| r.live());
                if cause.is_none() && !live() {
                    if let Some(job) = find_row_by_session(env, session_id).and_then(|r| r.short_id) {
                        wait_settled(env, &job, WAKE_RETRY_SETTLE_S);
                    }
                    std::thread::sleep(std::time::Duration::from_secs_f64(WAKE_RETRY_PAUSE_S));
                }
                // only once the copy is gone: a relaunch's copy carries the
                // member's name and labels, and is never left behind unsaid
                if cause.is_none() && gone && !live() {
                    copy_removed = None;
                    (rc, text) = crate::claude::run(&step.cmd, step.cwd.as_deref());
                    out.ran.push((step.clone(), rc, text.clone()));
                    extra_ran += 1;
                }
            }
        }
        let after_rm = at > 0 && plan.steps.get(at - 1).map(|p| matches!(classify(p), Kind::Rm { .. }) && p.id == step.id).unwrap_or(false);
        if rc != 0 {
            out.issues.push(issue(
                "step.failed",
                Severity::Error,
                format!("step failed ({rc}): {} -> {text}", step.cmd.join(" ")),
                failure_fix(step, &text, after_rm),
                step.id.clone(),
            ));
            return out; // a failed step stops the plan; nothing after it is safe
        }
        match kind {
            Kind::Launch | Kind::Relaunch | Kind::Resume { .. } => {
                let Some(id) = &step.id else { continue };
                let resumed = resumed_id(step);
                // Claude Code's output first (plan-v4 section 5): a "started a
                // copy as X" note means the job was live after all (a race with
                // the daemon): the copy goes, in this run, and the step failed
                if let Some(copy) = crate::claude::copy_started(&text) {
                    let gone = match copy_removed {
                        Some(gone) => gone,
                        None => {
                            for cmd in [crate::claude::stop(env, &copy), crate::claude::rm(env, &copy)] {
                                let (rc, t) = crate::claude::run(&cmd, None);
                                out.ran.push((Step { id: Some(id.clone()), cmd, cwd: None, why: format!("remove the copy {copy} Claude Code started instead of waking {id}") }, rc, t));
                            }
                            // the daemon's rm lands a moment after `claude rm` returns
                            row_gone_within(env, &copy, 3.0)
                        }
                    };
                    let instead = resumed.as_deref().map(|r| format!("resuming {r}")).unwrap_or_else(|| "a new job".into());
                    let left = if gone { "stopped and removed" } else { "still there" };
                    let (message, fix) = match cause.take().or_else(|| copy_cause(env, &text)) {
                        // not a live job: the files Claude Code could not read
                        // are the cause, and a retry is no fix (issue #6)
                        Some(c) => (
                            format!("Claude Code started a copy as {copy} instead of {instead}: {}; the copy is {left}", c.why),
                            if gone { format!("{}; then nimitz start {id}", c.fix) } else { format!("nimitz stop {copy}, then nimitz rm {copy}; {}", c.fix) },
                        ),
                        None => (
                            format!("Claude Code started a copy as {copy} instead of {instead}; the copy is {left}"),
                            if gone { format!("nimitz start {id}  # in a moment; the job was live or restarting") } else { format!("nimitz stop {copy}, then nimitz rm {copy}") },
                        ),
                    };
                    out.issues.push(issue("step.failed", Severity::Error, message, Some(fix), Some(id.clone())));
                    return out;
                }
                // the job it names must be the one expected: the resumed job, or
                // the new job of a cold launch (checked again after the bind)
                let want_job = match &kind {
                    Kind::Resume { session_id } => find_row_by_session(env, session_id).and_then(|r| r.short_id),
                    _ => None,
                };
                let said = crate::claude::job_started(&text);
                if let (Some(said), Some(want)) = (said.as_deref(), want_job.as_deref()) {
                    if said != want {
                        out.issues.push(issue("lineage.broken", Severity::Error, format!("{id}'s resume answered with job {said}, not its job {want}"), Some(format!("nimitz status  # then nimitz sweep {id}")), Some(id.clone())));
                        return out;
                    }
                }
                // a resume names the job's birth id; the SessionStart that binds
                // it carries the conversation the row runs now (after a /clear,
                // a later one), so that is what must come back
                let now_of = match &kind {
                    Kind::Resume { session_id } => find_row_by_session(env, session_id).map(|r| r.session_id).unwrap_or_else(|| session_id.clone()),
                    _ => String::new(),
                };
                let (expect, via) = match &kind {
                    Kind::Resume { .. } => (Some(now_of.as_str()), "resume"),
                    Kind::Launch => (None, "launch"),
                    _ if step.why.starts_with(MIGRATE_WHY) || step.why.starts_with(MIGRATE_LEAD_WHY) => (resumed.as_deref(), "migrate"),
                    _ => (resumed.as_deref(), "relaunch"),
                };
                match bind_started(env, id, step.cwd.as_deref(), since, expect, via, BIND_TIMEOUT_S) {
                    Ok(sid) => {
                        clear_pending(env, id);
                        let job = find_row_by_session(env, &sid).and_then(|r| r.short_id);
                        let want = want_job.clone().or(said);
                        if let (Some(job), Some(want)) = (job.as_deref(), want.as_deref()) {
                            if job != want {
                                out.issues.push(issue("lineage.broken", Severity::Error, format!("{id} bound {sid} in job {job}, not the expected {want}"), Some(format!("nimitz status  # then nimitz sweep {id}")), Some(id.clone())));
                                return out;
                            }
                        }
                        // a lead stays up: pin its job (plan-v4 section 7);
                        // a pin that cannot be written is a warning, not a failed start
                        if id.role().always_on() {
                            if let Some(job) = &job {
                                match crate::pins::pin(env, job) {
                                    Ok(true) => note(&mut out, "; pinned"),
                                    Ok(false) => {}
                                    Err(e) => out.issues.push(issue("pin.failed", Severity::Warn, format!("{id}'s job {job} could not be pinned ({e}); the daemon may retire it when idle"), Some(format!("{}  # pins it again", restart_cmd(id))), Some(id.clone()))),
                                }
                            }
                        }
                        started.push((id.clone(), sid, step.cwd.clone()))
                    }
                    Err(mut issues) => {
                        // real Claude Code may print "Workspace not trusted ..." and
                        // exit 0; then only the missing SessionStart shows it
                        if let Some(f) = failure_fix(step, &text, after_rm) {
                            for i in issues.iter_mut() {
                                i.fix = Some(f.clone());
                            }
                        }
                        out.issues.extend(issues);
                        return out; // the next steps assume this one worked
                    }
                }
            }
            Kind::Stop { short } => {
                release_folder(env, step.id.as_ref(), step.cwd.as_deref());
                if find_row(env, &short).map(|r| r.live()).unwrap_or(false) {
                    out.issues.push(issue("dup.live", Severity::Error, format!("stop {short} ran but the session is still live"), None, step.id.clone()));
                    return out;
                } // no row at all after a stop is fine too
            }
            Kind::Rm { short } => {
                if find_row(env, &short).is_some() {
                    out.issues.push(issue("step.failed", Severity::Error, format!("rm {short} ran but the row is still there; nothing after it ran"), None, step.id.clone()));
                    return out; // a resume with flags would now make an empty copy
                }
                // the row is gone: so is nimitz's pin of it (M24); other pins stay
                if crate::pins::unpin(env, &short).unwrap_or(false) {
                    note(&mut out, "; unpinned");
                }
            }
            Kind::Bind | Kind::Other | Kind::Config => unreachable!(),
        }
    }
    // Verify again once every step ran (an rm must not have hit the new row).
    for (id, sid, cwd) in &started {
        out.issues.extend(verify_live(env, id, sid, cwd.as_deref()));
    }
    out.verified = out.issues.iter().all(|i| i.severity != Severity::Error) && out.ran.len() - extra_ran == plan.steps.len();
    out
}

/// How long `apply` waits for the daemon to settle a stopped job before a
/// wake (r3-e2e finding 12: about a second after the SessionEnd).
const SETTLE_WAIT_S: f64 = 5.0;

/// Is the job row `short` gone, now or within `secs` (polled)? The daemon's
/// rm lands a moment after `claude rm` returns (acceptance env F2).
pub fn row_gone_within(env: &Env, short: &str, secs: f64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(secs);
    loop {
        if find_row(env, short).is_none() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Wait until `claude agents` lists `job` with no pid and a state other than
/// `running`, at most `secs`; a CLI that cannot be read ends the wait.
/// How long a wake that answered with a copy waits for the job's row to
/// settle, and the pause after, before its one retry.
pub const WAKE_RETRY_SETTLE_S: f64 = 15.0;
pub const WAKE_RETRY_PAUSE_S: f64 = 2.0;

pub fn wait_settled(env: &Env, job: &str, secs: f64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(secs);
    loop {
        let settled = match crate::claude::agents(env) {
            Ok(rows) => rows.iter().find(|r| r.id.as_deref() == Some(job)).map(|r| r.pid.is_none() && r.state.as_deref() != Some("running")).unwrap_or(true),
            Err(_) => true,
        };
        if settled || std::time::Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

/// A job whose process went away this recently with no SessionEnd (a kill
/// -9 of a `blocked` or `done` job, state untouched) is in the crash window
/// too: the 2.1.292 daemon restarts it within about 10 s (acceptance env F3).
pub const CRASH_RECENT_S: f64 = 30.0;

/// The crash window (plan-v4 section 5): no live pid, and the state is
/// `crashed` or `working`, or the row is parked (no SessionEnd) and changed
/// within `CRASH_RECENT_S`.
pub fn in_crash_window(env: &Env, r: &crate::evidence::SessionRow, events: &[crate::types::Event]) -> bool {
    if r.live() {
        return false;
    }
    if matches!(r.state.as_deref(), Some("crashed") | Some("working")) {
        return true;
    }
    let recent = r.status_since.is_some_and(|t| crate::evidence::now() - t < CRASH_RECENT_S);
    recent && crate::snapshot::parked_or_stopped(env, r, events).0 == Status::Parked
}

/// The crash window: a job that is `crashed`, or `working` with no live pid,
/// may be restarted by the daemon. Of the members about to be woken (`(id,
/// session id)`), wait once, up to `env.crash_window_s` for all of them
/// together, polling, and return those that came back live with why: their
/// steps are skipped (a resume of a restarted job makes a copy, and a
/// relaunch's stop + rm would end what the daemon just brought back). The
/// rest are woken as planned. Members not in the window to begin with are
/// never waited for.
pub fn wait_crash_window(env: &Env, waking: &[(Id, String)]) -> Vec<(Id, String)> {
    // keyed on the job's row (its short id): a crashed job that /branched is
    // waited for too, although its resume names a conversation no row carries
    let row = |short: &str| find_row(env, short);
    let events = crate::evidence::events(env);
    let in_window = |r: &crate::evidence::SessionRow| in_crash_window(env, r, &events);
    let mut pending: Vec<(Id, String, String, String)> = waking
        .iter()
        .filter_map(|(id, job)| row(job).filter(in_window).map(|r| (id.clone(), job.clone(), r.short_id.clone().unwrap_or_default(), r.state.clone().unwrap_or_else(|| "?".into()))))
        .collect();
    let mut back = vec![];
    if pending.is_empty() {
        return back;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(env.crash_window_s.max(0.0));
    loop {
        pending.retain(|(id, sid, short, state)| {
            if row(sid).map(|r| r.live()).unwrap_or(false) {
                back.push((id.clone(), format!("the daemon restarted job {short} (state was {state}); nothing to resume")));
                false
            } else {
                true
            }
        });
        if pending.is_empty() || std::time::Instant::now() >= deadline {
            return back;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Add to the text of the step that ran last, on its first line: the CLI
/// shows one line per step, and Claude Code's launch output has several
/// (`backgrounded · <id> · <name>`, then the `claude attach ...` hints).
fn note(out: &mut Outcome, text: &str) {
    if let Some(last) = out.ran.last_mut() {
        match last.2.find('\n') {
            Some(i) => last.2.insert_str(i, text),
            None => last.2.push_str(text),
        }
    }
}

fn pending_path(env: &Env, id: &Id) -> std::path::PathBuf {
    let name: String = id.0.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    env.state.join("pending").join(format!("{name}.json"))
}

fn write_pending(env: &Env, id: &Id, session_id: &str) {
    let p = pending_path(env, id);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, pending_record(id, session_id, crate::evidence::now()).to_string());
}

/// A `S/pending/<id>.json` record: the session a start is waiting on.
pub fn pending_record(id: &Id, session_id: &str, at: f64) -> serde_json::Value {
    serde_json::json!({"id": id.0, "session_id": session_id, "at": at})
}

fn clear_pending(env: &Env, id: &Id) {
    let _ = std::fs::remove_file(pending_path(env, id));
}

/// The session an interrupted relaunch of `id` was about to resume, if any.
pub fn pending_relaunch(env: &Env, id: &Id) -> Option<String> {
    let v = crate::evidence::first_json(&std::fs::read(pending_path(env, id)).ok()?)?;
    v.get("session_id").and_then(|s| s.as_str()).map(str::to_string)
}

/// An identity's start lock (a file lock on `NIMITZ_STATE/locks/<id>.lock`); released on drop.
pub struct IdLock(#[allow(dead_code)] std::fs::File);

/// Take `id`'s start lock; the flag says whether another nimitz held it first.
pub fn lock_identity(env: &Env, id: &Id) -> std::io::Result<(IdLock, bool)> {
    let dir = env.state.join("locks");
    std::fs::create_dir_all(&dir)?;
    let name: String = id.0.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join(format!("{name}.lock")))?;
    let waited = f.try_lock().is_err();
    if waited {
        f.lock()?;
    }
    Ok((IdLock(f), waited))
}

fn live_row(env: &Env, pick: impl Fn(&crate::evidence::SessionRow) -> bool) -> Option<crate::evidence::SessionRow> {
    let rows = crate::evidence::rows_from_files(env, &mut crate::evidence::FileCache::default()).unwrap_or_default();
    rows.into_iter().find(|r| (r.live() || r.unreadable) && pick(r))
}

/// A live session of `id` right now: its registry job or session, or, while
/// the registry names no job yet (a first launch racing a concurrent start),
/// a row with its labels **in its own folder** (`cwd`). Once the registry
/// names a job, another labelled job in the folder is a lead's intruder or a
/// ship's copy, never the member: counting it skipped a lead's relaunch after
/// its stop + rm and left the stray as the lead (re-review 1). A session
/// carrying the labels in another folder is an impostor (BUG-30) and never
/// counts (BUG-55).
fn live_session_of(env: &Env, id: &Id, cwd: Option<&str>) -> Option<crate::evidence::SessionRow> {
    let rec = crate::registry::load(env).identities.remove(&id.0);
    let (job, current) = (rec.as_ref().and_then(|r| r.job.clone()), rec.and_then(|r| r.session));
    let rows = crate::evidence::rows_from_files(env, &mut crate::evidence::FileCache::default()).unwrap_or_default();
    let labels = crate::evidence::session_labels(&crate::evidence::events(env), &rows);
    let here = |r: &crate::evidence::SessionRow| cwd.map(|c| same_folder(&r.cwd, c)).unwrap_or(true);
    rows.into_iter().find(|r| {
        (r.live() || r.unreadable)
            && (job.is_some() && r.short_id == job
                || Some(&r.session_id) == current.as_ref()
                || (job.is_none() && labels.get(&crate::evidence::row_key(r)).and_then(|l| l.get("NIMITZ_ID")) == Some(&id.0) && here(r)))
    })
}

fn same_folder(a: &str, b: &str) -> bool {
    crate::snapshot::norm_dir(a) == crate::snapshot::norm_dir(b)
}

/// The folder a plan runs `id`'s session steps in (its member folder).
fn step_folder<'a>(plan: &'a Plan, id: &Id) -> Option<&'a str> {
    plan.steps.iter().filter(|s| s.id.as_ref() == Some(id) && !matches!(classify(s), Kind::Stop { .. } | Kind::Rm { .. } | Kind::Config | Kind::Bind)).find_map(|s| s.cwd.as_deref())
}

/// A ship whose folder is shared between machines (one under `NIMITZ_HOME`, so
/// it travels with the data root) leaves a lock naming this machine, so another
/// machine's `check` sees `ship.elsewhere` instead of starting a second copy.
fn claim_folder(env: &Env, id: &Id, cwd: Option<&str>, session_id: &str) {
    let Some(cwd) = cwd.filter(|c| id.role() == Role::Ship && crate::env::canon(std::path::Path::new(c)).starts_with(crate::env::canon(&env.home))) else { return };
    let mut cache = crate::evidence::FileCache::default();
    let pid = crate::evidence::rows_from_files(env, &mut cache).unwrap_or_default().into_iter().find(|r| r.session_id == session_id).and_then(|r| r.pid);
    let line = ship_lock_line(&env.hostname, id, session_id, pid, crate::evidence::now());
    let _ = std::fs::write(ship_lock_path(cwd), line.to_string());
}

/// A ship lock as its file holds it (`.nimitz-ship.lock`, shared on purpose).
pub fn ship_lock_line(machine: &str, id: &Id, session_id: &str, pid: Option<i64>, at: f64) -> serde_json::Value {
    serde_json::json!({"machine": machine, "id": id.0, "session_id": session_id, "pid": pid, "at": at})
}

/// How long a stop waits for the worker process to exit (seconds).
const STOP_EXIT_S: f64 = 10.0;

/// Wait up to `limit` seconds for process `pid` to exit; true once it has.
/// Pid 1 and the sandbox's `LIVE_PID` (4 on Windows) are never a worker (the
/// sandbox's sessions record it as "alive"). The process is the one there
/// when the wait began (`os::is_same_process`): a pid Windows reuses meanwhile
/// is gone.
fn worker_gone_within(pid: i64, limit: f64) -> bool {
    if pid <= 1 || pid == crate::sandbox::LIVE_PID {
        return true;
    }
    let created = crate::os::process_started_ms(pid);
    let until = std::time::Instant::now() + std::time::Duration::from_secs_f64(limit);
    while crate::os::is_same_process(pid, created) {
        if std::time::Instant::now() >= until {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    true
}

/// Release it on a stop, but never another machine's lock.
fn release_folder(env: &Env, id: Option<&Id>, cwd: Option<&str>) {
    let Some(cwd) = cwd.filter(|_| id.map(|i| i.role() == Role::Ship).unwrap_or(false)) else { return };
    if lock_holder(cwd).map(|m| env.matches_hostname(&m)).unwrap_or(false) {
        let _ = std::fs::remove_file(ship_lock_path(cwd));
    }
}

/// The job row `target` (job or session id) when its labels name a 0.5/0.6
/// aide (`F/@NAME`), with that id.
fn retired_aide_row(env: &Env, target: &str) -> Option<(String, SessionInfo)> {
    let rows = crate::evidence::rows_from_files(env, &mut crate::evidence::FileCache::default()).unwrap_or_default();
    let labels = crate::evidence::session_labels(&crate::evidence::events(env), &rows);
    let r = rows.iter().find(|r| r.short_id.as_deref() == Some(target) || r.session_id == target)?;
    let owner = labels.get(&crate::evidence::row_key(r)).and_then(|l| l.get("NIMITZ_ID")).filter(|i| i.contains("/@"))?.clone();
    Some((owner, r.info(Default::default())))
}

/// The job row running `sid` now (or born as it), if any.
pub fn find_row_by_session(env: &Env, sid: &str) -> Option<crate::evidence::SessionRow> {
    let mut cache = crate::evidence::FileCache::default();
    crate::evidence::rows_from_files(env, &mut cache).unwrap_or_default().into_iter().find(|r| r.short_id.is_some() && r.session_ids().any(|s| s == sid))
}

pub fn find_row(env: &Env, short: &str) -> Option<crate::evidence::SessionRow> {
    let mut cache = crate::evidence::FileCache::default();
    crate::evidence::rows_from_files(env, &mut cache).unwrap_or_default().into_iter().find(|r| r.short_id.as_deref() == Some(short))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sandbox's always-alive pid (1, or 4, System, on Windows) is never
    /// waited on; a live process is waited on to the limit; a dead one is gone.
    #[test]
    fn a_stop_waits_only_for_a_real_worker() {
        let t = std::time::Instant::now();
        assert!(worker_gone_within(crate::sandbox::LIVE_PID, 5.0) && worker_gone_within(1, 5.0));
        assert!(t.elapsed().as_secs_f64() < 1.0);
        assert!(!worker_gone_within(std::process::id() as i64, 0.2), "this process is alive");
        let mut c = std::process::Command::new(std::env::current_exe().unwrap()).arg("--list").stdout(std::process::Stdio::null()).spawn().unwrap();
        let pid = c.id() as i64;
        c.wait().unwrap();
        assert!(worker_gone_within(pid, 2.0));
    }

    /// The post-launch wait binds a job's SessionStart only when the job's row
    /// backs the labels on the line: a polluted daemon's job (no saved labels)
    /// that starts during a launch is never taken for the member.
    #[test]
    fn the_launch_wait_skips_a_job_whose_row_does_not_back_its_labels() {
        let (_td, env) = crate::config::tests::test_env();
        let id = Id::ship("F", "SQ", "S");
        std::fs::create_dir_all(&env.state).unwrap();
        let row = |short: &str, sid: &str, labelled: bool| {
            let flags = if labelled { serde_json::json!(["--settings", serde_json::json!({"env": {"NIMITZ_ID": id.0, "NIMITZ_ROLE": "ship"}}).to_string()]) } else { serde_json::json!([]) };
            let p = env.claude_home.join("jobs").join(short).join("state.json");
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, serde_json::json!({"sessionId": sid, "state": "running", "respawnFlags": flags}).to_string()).unwrap();
        };
        let line = |sid: &str, job: &str| serde_json::json!({"ts": 10.0, "event": "SessionStart", "session_id": sid, "source": "startup", "job": job, "kind": "bg",
            "labels": {"NIMITZ_ID": id.0, "NIMITZ_ROLE": "ship"}}).to_string();
        row("p1", "s-polluted", false);
        std::fs::write(env.events_path(), line("s-polluted", "p1") + "\n").unwrap();
        assert_eq!(wait_for_binding(&env, &id, 1.0, 0.0, None), None, "an inherited label is no binding");
        row("m1", "s-member", true);
        std::fs::write(env.events_path(), line("s-polluted", "p1") + "\n" + &line("s-member", "m1") + "\n").unwrap();
        assert_eq!(wait_for_binding(&env, &id, 1.0, 0.0, None).map(|(s, _)| s).as_deref(), Some("s-member"));
        // a job-less line (the session file was unreadable at SessionStart) of a
        // polluted job: the job comes from the rows, and its labels are not backed
        let jobless = serde_json::json!({"ts": 11.0, "event": "SessionStart", "session_id": "s-polluted2", "source": "startup", "kind": "unknown",
            "labels": {"NIMITZ_ID": id.0, "NIMITZ_ROLE": "ship"}}).to_string();
        row("p2", "s-polluted2", false);
        let sf = env.claude_home.join("sessions").join("77.json");
        std::fs::create_dir_all(sf.parent().unwrap()).unwrap();
        std::fs::write(&sf, serde_json::json!({"pid": std::process::id(), "sessionId": "s-polluted2", "jobId": "p2", "kind": "bg", "cwd": "/w"}).to_string()).unwrap();
        std::fs::write(env.events_path(), jobless + "\n").unwrap();
        assert_eq!(wait_for_binding(&env, &id, 1.0, 0.0, None), None, "a job-less line of a polluted job is no binding");
    }

    #[test]
    fn names_round_trip() {
        for id in [Id::admiral("MAIN"), Id::commodore("MAIN", "ARGOS"), Id::ship("MAIN", "ARGOS", "BOLT")] {
            assert_eq!(id_from_name(&id.name()), Some(id.clone()), "{}", id.name());
        }
        assert_eq!(id_from_name("[F.SQ]-COMMODORE/fix"), None, "the 0.3.x tender address names no one");
        assert_eq!(id_from_name("[ADMIRALTY]"), None);
        assert_eq!(id_from_name("scratch notes"), None);
        assert_eq!(id_from_name("[main.ARGOS]-SHIP"), None); // lowercase fleet
        assert_eq!(id_from_name("[F.SQ]-COMMODORE/"), None);
        // a SendMessage ref suffix does not hide the address
        assert_eq!(id_from_name("[F.SQ]-S [3fa9c1]"), Some(Id::ship("F", "SQ", "S")));
        // a 0.6 aide's address is no member's any more
        assert_eq!(id_from_name("[HOME]-@KB"), None);
    }

    #[test]
    fn caller_needs_matching_labels() {
        let m = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        assert_eq!(caller_from_env(&m(&[])), Caller::User);
        assert_eq!(
            caller_from_env(&m(&[("NIMITZ_ID", "F/SQ"), ("NIMITZ_ROLE", "commodore")])),
            Caller::Member { id: Id::commodore("F", "SQ"), role: Role::Commodore }
        );
        // partial or disagreeing labels: an unverified caller, never the user
        let unverified = Caller::Member { id: Id(UNVERIFIED.into()), role: Role::Ship };
        assert_eq!(caller_from_env(&m(&[("NIMITZ_ID", "F/SQ/S"), ("NIMITZ_ROLE", "commodore")])), unverified);
        assert_eq!(caller_from_env(&m(&[("NIMITZ_ID", "F/SQ/~fix"), ("NIMITZ_ROLE", "tender")])), unverified, "a 0.3.x tender is unverified");
        assert_eq!(caller_from_env(&m(&[("NIMITZ_ID", "F/SQ/S")])), unverified);
        assert_eq!(caller_from_env(&m(&[("NIMITZ_ROLE", "ship")])), unverified);
        assert_eq!(caller_from_env(&m(&[("NIMITZ_ID", "not an id"), ("NIMITZ_ROLE", "admiral")])), unverified);
        // NIMITZ_HOME alone is no label at all
        assert_eq!(caller_from_env(&m(&[("NIMITZ_HOME", "/h")])), Caller::User);
        // a 0.6 aide's labels: no member's
        assert_eq!(caller_from_env(&m(&[("NIMITZ_ID", "HOME/@KB"), ("NIMITZ_ROLE", "aide")])), unverified);
    }

    #[test]
    fn a_members_sweep_with_no_id_is_its_own_scope_and_any_scope_runs() {
        let (_td, env) = crate::config::tests::test_env();
        crate::config::tests::make_fleet(&env);
        let snap = crate::check::tests::snap_for(&env, None);
        let me = Id::ship("MAIN", "ARGOS", "ARGOS");
        let ship = Caller::Member { id: me.clone(), role: Role::Ship };
        assert_eq!(plan_sweep(&env, &snap, None, &ship).summary, "sweep MAIN/ARGOS/ARGOS");
        for target in [Id::commodore("MAIN", "ARGOS"), Id::admiral("MAIN")] {
            let p = plan_sweep(&env, &snap, Some(std::slice::from_ref(&target)), &ship);
            assert!(p.ok(), "{target}: {:?}", p.refusals);
        }
        assert_eq!(plan_sweep(&env, &snap, None, &Caller::User).summary, "sweep everything");
    }

    #[test]
    fn a_member_is_inside_itself_and_its_leads() {
        let me = Id::ship("MAIN", "ARGOS", "ARGOS");
        for target in [Id::ship("MAIN", "ARGOS", "ARGOS"), Id::commodore("MAIN", "ARGOS"), Id::admiral("MAIN")] {
            assert!(inside(&me, &target), "{target}");
        }
        assert!(!inside(&me, &Id::ship("MAIN", "ARGOS", "AST")) && !inside(&me, &Id::admiral("BETA")));
        let unverified = Caller::Member { id: Id(UNVERIFIED.into()), role: Role::Ship };
        assert!(verified(&unverified).is_none() && verified(&Caller::User).is_none());
    }

    #[test]
    fn classify_reads_argv_back() {
        fn s(cmd: &[&str]) -> Step {
            Step { id: None, cmd: cmd.iter().map(|x| x.to_string()).collect(), cwd: None, why: String::new() }
        }
        assert_eq!(classify(&s(&["claude", "--bg", "--resume", "sid"])), Kind::Resume { session_id: "sid".into() });
        assert_eq!(classify(&s(&["claude", "--bg", "--resume", "sid", "--name", "n", "--settings", "{}"])), Kind::Relaunch);
        assert_eq!(classify(&s(&["claude", "--bg", "--name", "n", "--settings", "{}", "p"])), Kind::Launch);
        assert_eq!(classify(&s(&["/n/claude", "attach", "j1"])), Kind::Other, "an attach is the multiplexer's, never apply's");
        assert_eq!(classify(&s(&["/n/claude", "stop", "j1"])), Kind::Stop { short: "j1".into() });
        assert_eq!(classify(&s(&["bind", "F/SQ/S", "sid", "adopt"])), Kind::Bind);
        assert_eq!(classify(&s(&["write", "/x/squadron.toml", "body"])), Kind::Config);
        assert_eq!(classify(&s(&["archive", "/x/fleet.toml"])), Kind::Config);
    }
}
