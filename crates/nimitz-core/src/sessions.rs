//! Session actions along the chain of command: start (top-down), stop, relaunch,
//! rm, sweep, adopt; the relaunch state table (contracts.md section 8). Owner: core-sessions.
//!
//! Every `plan_*` only reads and returns a `Plan`; nothing changes until
//! `apply`. A plan step's command is real argv (`claude ...`) except the
//! internal `["bind", <id>, <session id>, <via>]` step, which `apply` performs
//! itself (a registry write, not a process). A step whose command is not a
//! `--bg` launch and names an interactive member (admiral, commodore) is never
//! run by `apply`: the CLI opens it in a mux pane (plan-v3 section 13); core
//! never depends on the mux.
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

/// The identity a caller gets when its labels are partial or disagree: no
/// authority at all (BUG-06, fail closed). Its role is ship, so every
/// authority rule refuses it.
pub const UNVERIFIED: &str = "?/?/unverified-labels";

/// Who is asking, from the caller's env (labels reach the Bash tool's
/// processes, probe P0.6). No labels at all (`NIMITZ_HOME` alone is none): the
/// user. Labels that are partial (`NIMITZ_ID` or `NIMITZ_ROLE` missing), that
/// disagree, or that name no valid identity: an unverified caller with no
/// authority, never the user.
pub fn caller_from_env(vars: &HashMap<String, String>) -> Caller {
    let get = |k: &str| vars.get(k).filter(|v| !v.is_empty());
    let unverified = Caller::Member { id: Id(UNVERIFIED.into()), role: Role::Ship };
    match (get("NIMITZ_ID"), get("NIMITZ_ROLE")) {
        (None, None) => Caller::User,
        (Some(id), Some(role)) => {
            let id = Id(id.clone());
            let shape_ok = id.parts().iter().all(|p| crate::config::valid_name(p)) && id.name() != id.0;
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

fn caller_name(id: &Id) -> String {
    if id.0 == UNVERIFIED { "a caller with partial or inconsistent NIMITZ_* labels".into() } else { id.name() }
}

// ------------------------------------------------------------------ issues

fn issue(code: &str, severity: Severity, message: String, fix: Option<String>, id: Option<Id>) -> Issue {
    Issue { code: code.into(), severity, message, fix, id }
}

/// The lock file a managed ship folder holds while the ship runs on a machine.
pub fn ship_lock_path(cwd: &str) -> std::path::PathBuf {
    std::path::Path::new(cwd).join(".nimitz-ship.lock")
}

/// A live process with no job that runs a background member's conversation
/// (an SDK host, an interactive or Remote Control `--resume`): the member is
/// `held` (M13), never a copy and never a second live session of it.
fn held_row(m: &Member, r: &SessionInfo) -> bool {
    m.role == Role::Ship && r.pid.is_some() && r.short_id.is_none()
}

/// Session-level issues readable from the member and the filesystem alone:
/// name.drift, labels.missing, labels.mismatch, dup.live, copy.rows, held,
/// ship.elsewhere. (name.taken and lineage.broken need the whole snapshot or
/// the registry; `snapshot` adds those to `Member::issues`.)
pub fn session_issues(env: &Env, m: &Member) -> Vec<Issue> {
    let mut out = vec![];
    let id = Some(m.id.clone());
    let relaunch = Some(format!("nimitz relaunch {}", m.id));
    let rows: Vec<&SessionInfo> = m.session.iter().chain(m.extra_rows.iter()).collect();
    let live: Vec<&&SessionInfo> = rows.iter().filter(|r| r.pid.is_some() && !held_row(m, r)).collect();
    for r in rows.iter().filter(|r| held_row(m, r)) {
        out.push(issue(
            "held",
            Severity::Warn,
            format!(
                "{}'s conversation {} runs in a process with no job (pid {}, {}): nimitz cannot stop or resume it there",
                m.id,
                r.session_id,
                r.pid.unwrap_or(0),
                r.entrypoint.as_deref().unwrap_or(&r.kind)
            ),
            Some(format!("end that process in its host, then nimitz start {}", m.id)),
            id.clone(),
        ));
    }

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
        if p.pid.is_some() && p.name != m.expected_name {
            out.push(issue(
                "name.drift",
                Severity::Error,
                format!("{} is live as {:?}, expected {:?}", m.id, p.name, m.expected_name),
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
                    Some(format!("nimitz relaunch {}  # a labelled copy in the new folder", m.id)),
                    id.clone(),
                ));
            }
        }
    }
    let extra: Vec<&SessionInfo> = m.extra_rows.iter().filter(|r| !held_row(m, r)).collect();
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
/// job row and no transcript). Called by `snapshot` for every member.
pub fn registry_issues(env: &Env, cache: &mut crate::evidence::FileCache, m: &Member, reg: &Registry) -> Vec<Issue> {
    let Some(rec) = reg.identities.get(&m.id.0) else { return vec![] };
    let Some(current) = rec.session.as_deref() else { return vec![] };
    let known = m.session.iter().chain(m.extra_rows.iter()).any(|r| r.session_id == current || r.birth_id.as_deref() == Some(current));
    if known || crate::evidence::transcript(env, cache, m.cwd.as_deref(), current).is_some() {
        return vec![];
    }
    vec![issue(
        "lineage.broken",
        Severity::Error,
        format!("{}'s current session {current} has no job row and no transcript", m.id),
        Some(format!("nimitz start {} --force  # cold launch; lineage starts over", m.id)),
        Some(m.id.clone()),
    )]
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

// ------------------------------------------------------------------ authority

/// May `caller` start (or stop, relaunch, rm, adopt as) `target`? The
/// authority table (plan-v3 section 5.5, widened in 0.3.1): you, anything; an
/// admiral, every member of its own fleet but itself; a commodore, its own
/// ships; ships nothing.
pub fn may_start(caller: &Caller, target: &Id, verb: &str) -> Result<(), String> {
    match caller {
        Caller::User => Ok(()),
        Caller::Member { id, role: Role::Admiral } if target.fleet() == id.fleet() && target != id => Ok(()),
        Caller::Member { id, role: Role::Commodore } if target.parent().as_ref() == Some(id) => Ok(()),
        Caller::Member { id, .. } => Err(format!(
            "{} may not {verb} {target}: you {verb} anything; an admiral, the members of its own fleet; a commodore, its own ships",
            caller_name(id)
        )),
    }
}

/// May `caller` change the config at `scope` (a fleet `F`, a squadron `F/SQ`,
/// a ship `F/SQ/SHIP`)? You, anything; an admiral, its own fleet (the fleet's
/// settings, its squadrons and ships); a commodore, its own squadron's settings
/// and ships. Creating or removing a fleet, machines and migration stay yours
/// (the CLI never asks this for them).
pub fn may_config(caller: &Caller, scope: &Id) -> Result<(), String> {
    match caller {
        Caller::User => Ok(()),
        Caller::Member { id, role: Role::Admiral } if scope.fleet() == id.fleet() => Ok(()),
        Caller::Member { id, role: Role::Commodore } if scope == id || scope.parent().as_ref() == Some(id) => Ok(()),
        Caller::Member { id, .. } => Err(format!(
            "{} may not change the config of {scope}: you change any; an admiral, its own fleet; a commodore, its own squadron and its ships",
            caller_name(id)
        )),
    }
}

/// May `caller` run this fix itself (`nimitz <verb> ...`)? Read-only commands
/// (`status`, `check`, `rules`, `doctor` without `--fix`) always; `signal` by
/// the chain of command (`hook::may_signal`; with no `env` a ship may signal
/// only its commodore and its admiral); a ship's `sweep` of itself; the rest
/// by the authority table for the id the command names. A command that names
/// a session by its short id, or that only you run (init, fleet new / rm,
/// machine, migrate, doctor --fix), is not a member's.
pub fn may_run(caller: &Caller, fix: &str) -> bool {
    may_run_in(None, caller, fix)
}

/// `may_run` with the config at hand (a ship's `signal` to another ship goes
/// by the squadron's comms rule).
pub fn may_run_in(env: Option<&Env>, caller: &Caller, fix: &str) -> bool {
    let Caller::Member { id: me, role } = caller else { return true };
    let w: Vec<&str> = fix.split_whitespace().take_while(|x| *x != "#").collect();
    if w.first() != Some(&"nimitz") {
        return false;
    }
    let id_at = |i: usize| -> Option<Id> {
        let s = w.get(i)?;
        let id = Id(s.to_string());
        let ok = !s.is_empty() && id.parts().iter().all(|p| crate::config::valid_name(p));
        ok.then_some(id)
    };
    let in_fleet = |t: &Id| *role == Role::Admiral && t.fleet() == me.fleet();
    let in_squadron = |t: &Id| *role == Role::Commodore && (t == me || t.parent().as_ref() == Some(me));
    match w.get(1).copied() {
        Some("status") | Some("check") | Some("rules") => true,
        Some("doctor") => !w.contains(&"--fix"),
        Some("signal") => id_at(2).is_some_and(|t| {
            let strict = || Comms::Mode("commodore-only".into());
            match env {
                Some(env) => crate::hook::may_signal(env, me, &t.name()).is_ok(),
                None => crate::hook::signal_table(me, &t, &strict).is_ok(),
            }
        }),
        Some("start") | Some("stop") | Some("relaunch") | Some("rm") => id_at(2).is_some_and(|t| may_start(caller, &t, "start").is_ok()),
        Some("sweep") => match id_at(2) {
            None => w.len() == 2 && matches!(role, Role::Admiral | Role::Commodore),
            Some(t) => in_fleet(&t) || in_squadron(&t) || (*role == Role::Ship && t == *me),
        },
        Some("adopt") => id_at(3).is_some_and(|t| may_start(caller, &t, "adopt as").is_ok()),
        Some("up") => id_at(2).is_some_and(|t| in_fleet(&t)),
        Some("layout") => *role == Role::Admiral && id_at(3).map(|t| t.fleet() == me.fleet()).unwrap_or(true),
        Some("ship") => id_at(3).is_some_and(|t| may_config(caller, &t).is_ok()),
        Some("squadron") => match (w.get(2).copied(), id_at(3)) {
            (Some("new"), Some(t)) => in_fleet(&t),
            (Some("rm"), Some(t)) => in_fleet(&t),
            (Some("set"), Some(t)) => may_config(caller, &t).is_ok(),
            _ => false,
        },
        Some("fleet") => w.get(2) == Some(&"set") && id_at(3).is_some_and(|t| in_fleet(&t)),
        _ => false,
    }
}

/// May `caller` act on one session row (`stop <short>`, `rm <short>`)? A
/// member's row goes by that member (`may_start`); a stowaway in an admiral's
/// own fleet is that admiral's too; anything else is yours.
fn may_touch_row(snap: &Snapshot, caller: &Caller, owner: Option<&Id>, row: &SessionInfo, verb: &str) -> Result<(), String> {
    if let Some(o) = owner {
        return may_start(caller, o, verb);
    }
    match caller {
        Caller::User => Ok(()),
        Caller::Member { id, role: Role::Admiral }
            if snap.stowaways.iter().any(|s| s.session_id == row.session_id)
                && crate::snapshot::stowaway_fleet(snap, row).as_deref() == Some(id.fleet()) =>
        {
            Ok(())
        }
        Caller::Member { id, .. } => Err(format!(
            "{} may not {verb} session {}: it is no member of yours (a stowaway goes to its fleet's admiral or you; the rest to you)",
            caller_name(id),
            row.short_id.as_deref().unwrap_or(&row.session_id)
        )),
    }
}

fn refuse(plan: &mut Plan, code: &str, msg: String, fix: Option<String>, id: Option<Id>) {
    plan.refusals.push(issue(code, Severity::Error, msg, fix, id));
}

// ------------------------------------------------------------------ members in a snapshot

/// Every member of a snapshot, with its fleet.
fn members(snap: &Snapshot) -> Vec<(&Fleet, &Member)> {
    let mut out = vec![];
    for f in &snap.fleets {
        out.push((f, &f.admiral));
        for sq in &f.squadrons {
            out.push((f, &sq.commodore));
            out.extend(sq.ships.iter().map(|m| (f, m)));
        }
    }
    out
}

fn member<'a>(snap: &'a Snapshot, id: &Id) -> Option<(&'a Fleet, &'a Member)> {
    members(snap).into_iter().find(|(_, m)| &m.id == id)
}

/// The chain of command above `id`, top-down, then `id` itself.
fn chain(id: &Id) -> Vec<Id> {
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
fn start_member(env: &Env, plan: &mut Plan, snap: &Snapshot, id: &Id, force: bool) {
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
        refuse(plan, "folder.missing", format!("{id} has no folder on this machine"), Some(format!("nimitz ship set {id} --dir <folder|managed>")), Some(id.clone()));
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
    let needs_relabel = session_errs.iter().any(|i| ROW_FIXES[..2].contains(&i.code.as_str()));
    plan.warnings.extend(session_errs);

    let lineage_broken = m.issues.iter().any(|i| i.code == "lineage.broken");
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
            cold_launch(env, plan, &fod, id, m.role, &mode, &cwd);
        }
        // stopped with its labels and its name: resume = native binary, NO flags,
        // from its folder (row 4). A resume restores the saved name too, so a
        // drifted name must go the relaunch way instead.
        Some(p) if p.labels.get("NIMITZ_ID") == Some(&id.0) && !needs_relabel => {
            if m.role.interactive() {
                plan.steps.push(step(id, crate::claude::interactive(env, id, &mode, true), "continue the interactive session in a pane"));
            } else {
                plan.steps.push(step(id, crate::claude::resume_bg(env, &p.session_id), "resume keeps the id, name, mode and labels (probe 2c1)"));
            }
        }
        // stopped without labels (or with a drifted name): a flag-free resume
        // would keep neither; rm the row, then resume with every flag (row 5)
        Some(p) => plan.steps.extend(relabel_steps(env, id, &mode, p, &cwd)),
        None if lineage_broken && !force => {
            refuse(
                plan,
                "lineage.broken",
                format!("{id}'s recorded session has no job row and no transcript"),
                Some(format!("nimitz start {id} --force  # cold launch; lineage starts over")),
                Some(id.clone()),
            );
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
            // no row: an interactive member continues its transcript with `-c` (row 6)
            if m.role.interactive() && !crate::evidence::transcripts_in(env, &cwd).is_empty() {
                plan.steps.push(step(id, crate::claude::interactive(env, id, &mode, true), "continue the interactive session in a pane"));
            } else if let (false, Some(sid)) = (m.role.interactive(), kept) {
                let interrupted = pending_relaunch(env, id).as_deref() == Some(sid.as_str());
                let why = if interrupted {
                    format!("recovering an interrupted relaunch: resume {} with every flag (its row was removed; Claude Code keeps the id)", &sid[..sid.len().min(8)])
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
                if m.role.interactive() {
                    if let Some(old) = legacy_names_of(&fod, id).first() {
                        if let Some(last) = plan.steps.last_mut() {
                            last.why = format!(
                                "first launch in a pane: the old {old} ran in another folder, and Claude Code resumes a conversation only in its own folder"
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

/// The `why` that marks a step continuing an old fleet's session; `apply`
/// records such a binding with `via: migrate`.
pub const MIGRATE_WHY: &str = "continue the old fleet's session";

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
        Role::Admiral | Role::Commodore => plan.steps.push(Step {
            id: Some(id.clone()),
            cmd: crate::claude::interactive(env, id, mode, false),
            cwd: Some(cwd.to_string()),
            why: "first launch of an interactive member, in a pane".into(),
        }),
    }
}

// ------------------------------------------------------------------ plans

/// Pre-start gate: `nimitz check` scoped to the id. Errors refuse (cc.version
/// and lineage.broken yield to --force); warnings ride along. The relaunch
/// table handles the session-level codes itself, with their row's action.
fn gate(env: &Env, plan: &mut Plan, snap: &Snapshot, id: Option<&Id>, force: bool) {
    const TABLE_HANDLED: [&str; 6] = ["name.drift", "labels.missing", "labels.mismatch", "dup.live", "copy.rows", "lineage.broken"];
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

/// Start `id` and every missing parent, top-down. Runs `check` first.
pub fn plan_start(env: &Env, snap: &Snapshot, id: &Id, caller: &Caller, force: bool) -> Plan {
    let mut plan = Plan { summary: format!("start {id} (top-down)"), ..Default::default() };
    // Authority first (BUG-51): a caller who may not start the target gets that
    // one refusal, and nothing else (no other refusals, warnings or fixes).
    if let Err(why) = may_start(caller, id, "start") {
        refuse(&mut plan, "authority", why, None, Some(id.clone()));
        return plan;
    }
    gate(env, &mut plan, snap, Some(id), force);
    // What is wrong with a member caller's own chain is yours to fix, not a
    // reason to refuse the caller's start of its ship.
    if let Caller::Member { id: me, .. } = caller {
        let above = chain(me);
        let (theirs, keep): (Vec<Issue>, Vec<Issue>) =
            plan.refusals.drain(..).partition(|i| i.id.as_ref().map(|x| x != id && above.contains(x)).unwrap_or(false));
        plan.refusals = keep;
        plan.warnings.extend(theirs.into_iter().map(|i| Issue { severity: Severity::Warn, ..i }));
    }
    for link in chain(id) {
        let running_clean = member(snap, &link)
            .map(|(_, m)| {
                m.session.as_ref().map(|p| p.pid.is_some()).unwrap_or(false)
                    && session_issues(env, m).iter().all(|i| i.severity != Severity::Error)
            })
            .unwrap_or(false);
        if running_clean {
            continue; // row 1 for a parent: nothing, and no authority needed
        }
        // A member caller is running, and what is above it is yours to start: a
        // commodore reviving its ship is not blocked by an admiral that is down.
        if let Caller::Member { id: me, .. } = caller {
            if link != *id && chain(me).contains(&link) {
                plan.warnings.push(issue(
                    "authority",
                    Severity::Info,
                    format!("{link} is not running clean; only you start it"),
                    Some(format!("nimitz start {link}")),
                    Some(link.clone()),
                ));
                continue;
            }
        }
        if let Err(why) = may_start(caller, &link, "start") {
            refuse(&mut plan, "authority", why, None, Some(link.clone()));
            continue;
        }
        if already_refused(&plan, &link) {
            continue; // the gate found something that no step can fix
        }
        start_member(env, &mut plan, snap, &link, force);
    }
    // A `legacy` warning is about starting that member; a running parent left
    // alone does not make the target's start a second agent anywhere (BUG-38).
    let acting: Vec<Id> = plan.steps.iter().filter_map(|s| s.id.clone()).chain(std::iter::once(id.clone())).collect();
    plan.warnings.retain(|i| i.code != "legacy" || i.id.as_ref().map(|x| acting.contains(x)).unwrap_or(true));
    dedup_issues(&mut plan);
    mark_repairs(&mut plan);
    debug_assert!(no_fork(snap, &plan), "a plan would fork a session: {:?}", plan.steps);
    plan
}

/// Has the gate (or an earlier member) already refused for this identity?
fn already_refused(plan: &Plan, id: &Id) -> bool {
    plan.refusals.iter().any(|i| i.id.as_ref() == Some(id) || i.id.is_none())
}

/// `nimitz up <fleet>`: start every member that should run on this machine.
pub fn plan_up(env: &Env, snap: &Snapshot, fleet: &str, caller: &Caller) -> Plan {
    let mut plan = Plan { summary: format!("up {fleet}: reconcile the config with the running sessions"), ..Default::default() };
    let ok = match caller {
        Caller::User => true,
        Caller::Member { id, role: Role::Admiral } => id.fleet() == fleet,
        _ => false,
    };
    if !ok {
        refuse(&mut plan, "authority", format!("`nimitz up {fleet}` is yours or {fleet}'s admiral's; a commodore starts its own ships one by one"), None, None);
        return plan;
    }
    let Some(f) = snap.fleets.iter().find(|f| f.id.fleet() == fleet) else {
        refuse(&mut plan, "member.unknown", format!("no fleet {fleet} in the config"), None, None);
        return plan;
    };
    // a fleet.toml that does not parse: that error and its fix, nothing else (M8)
    let fod = match crate::config::load_fleet(env, fleet) {
        Ok(fod) => Some(fod),
        Err(i) if i.code == "config.invalid" => {
            plan.refusals.push(i);
            return plan;
        }
        Err(_) => None,
    };
    if f.machine.is_none() {
        refuse(&mut plan, "machine.missing", format!("fleet {fleet} does not run on this machine"), Some(format!("nimitz machine add {fleet} <label>")), None);
        return plan;
    }
    gate(env, &mut plan, snap, Some(&f.id), false);
    let mut ids = vec![f.admiral.id.clone()];
    for sq in &f.squadrons {
        ids.push(sq.commodore.id.clone());
        // ships with a folder on this machine, in depends_on order
        let here: Vec<Id> = sq.ships.iter().filter(|s| s.cwd.is_some()).map(|s| s.id.clone()).collect();
        let deps = fod
            .as_ref()
            .and_then(|f| f.squadrons.iter().find(|(_, s)| Some(s.squadron.as_str()) == sq.id.squadron()))
            .map(|(_, s)| s.ships.iter().map(|sh| (sh.name.clone(), sh.depends_on.clone())).collect::<Vec<_>>())
            .unwrap_or_default();
        ids.extend(in_dependency_order(here, &deps));
    }
    for id in ids {
        let clean_live = member(snap, &id)
            .map(|(_, m)| {
                m.session.as_ref().map(|p| p.pid.is_some()).unwrap_or(false)
                    && session_issues(env, m).iter().all(|i| i.severity != Severity::Error)
            })
            .unwrap_or(false);
        if clean_live {
            continue;
        }
        gate(env, &mut plan, snap, Some(&id), false);
        if !already_refused(&plan, &id) {
            start_member(env, &mut plan, snap, &id, false);
        }
    }
    dedup_issues(&mut plan);
    mark_repairs(&mut plan);
    debug_assert!(no_fork(snap, &plan), "a plan would fork a session: {:?}", plan.steps);
    plan
}

/// Ships before the ships that declare them in `depends_on`. A cycle or an
/// unknown name never drops a ship: it keeps its place (`check` reports it).
fn in_dependency_order(ids: Vec<Id>, deps: &[(String, Vec<String>)]) -> Vec<Id> {
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

/// Is this step one the CLI must open in a multiplexer pane (an interactive
/// member) instead of running itself? `apply` never runs these.
pub fn is_pane_step(step: &Step) -> bool {
    classify(step) == Kind::Interactive
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
    let mut seen = std::collections::HashSet::new();
    plan.warnings.retain(|i| seen.insert(key(i)));
}

pub fn plan_stop(env: &Env, snap: &Snapshot, id: &Id, caller: &Caller) -> Plan {
    let mut plan = Plan { summary: format!("stop {id}"), ..Default::default() };
    if let Err(why) = may_start(caller, id, "stop") {
        refuse(&mut plan, "authority", why, None, Some(id.clone()));
        return plan;
    }
    let Some((_, m)) = member(snap, id) else {
        refuse(&mut plan, "member.unknown", format!("{id} is not in any fleet config"), None, Some(id.clone()));
        return plan;
    };
    let Some(p) = m.session.as_ref().filter(|p| p.pid.is_some()) else {
        refuse(&mut plan, "session.none", format!("{id} is not live"), None, Some(id.clone()));
        return plan;
    };
    if m.role.interactive() && p.kind != "background" {
        refuse(
            &mut plan,
            "session.interactive",
            format!("{id} is an interactive session; end it in its own terminal"),
            Some("type /exit in its pane".into()),
            Some(id.clone()),
        );
        return plan;
    }
    let Some(short) = &p.short_id else {
        refuse(&mut plan, "session.none", format!("{id}'s session {} has no job id to stop", p.session_id), None, Some(id.clone()));
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
/// session id. Yours alone. Allowed: a member's row (one of two live copies), a
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
            let labelled_as = labels.get(&r.session_id).and_then(|l| l.get("NIMITZ_ID")).cloned();
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
    let Some((owner, row, why)) = hit else {
        let msg = format!("{target} names no fleet member's session, no stowaway and no holder of a fleet name");
        // a member learns nothing about sessions that are not its business
        let (code, msg) = if *caller == Caller::User { ("session.unknown", msg) } else { ("authority", format!("{msg}; anything else is the user's")) };
        refuse(&mut plan, code, msg, Some("nimitz status".into()), None);
        return plan;
    };
    if let Err(why) = may_touch_row(snap, caller, owner.as_ref(), &row, "stop") {
        refuse(&mut plan, "authority", why, None, owner);
        return plan;
    }
    let Some(short) = row.short_id.clone().filter(|_| row.pid.is_some()) else {
        refuse(&mut plan, "session.none", format!("session {} is not a live background session", row.session_id), None, owner);
        return plan;
    };
    plan.steps.push(Step { id: owner, cmd: crate::claude::stop(env, &short), cwd: None, why });
    plan
}

/// A deliberate relaunch: new id, every flag, old row removed (contracts.md section 8).
pub fn plan_relaunch(env: &Env, snap: &Snapshot, id: &Id, caller: &Caller) -> Plan {
    let mut plan = Plan { summary: format!("relaunch {id} (same session id and history; labels, name and mode restored)"), ..Default::default() };
    if let Err(why) = may_start(caller, id, "relaunch") {
        refuse(&mut plan, "authority", why, None, Some(id.clone()));
        return plan;
    }
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
    if m.role.interactive() {
        if m.session.as_ref().map(|p| p.pid.is_some()).unwrap_or(false) {
            refuse(&mut plan, "session.interactive", format!("{id} is live; end it in its pane first"), Some("type /exit in its pane".into()), Some(id.clone()));
            return plan;
        }
        plan.steps.push(Step {
            id: Some(id.clone()),
            cmd: crate::claude::interactive(env, id, &mode, true),
            cwd: Some(cwd),
            why: "reopen the interactive session with its name and labels, in a pane".into(),
        });
        return plan;
    }
    let Some(p) = &m.session else {
        refuse(&mut plan, "session.none", format!("{id} has no session to relaunch; use nimitz start"), Some(format!("nimitz start {id}")), Some(id.clone()));
        return plan;
    };
    if let Some(r) = m.extra_rows.iter().find(|r| r.pid.is_some()) {
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
        refuse(&mut plan, "session.none", format!("{id} is live with no job id; cannot stop it"), None, Some(id.clone()));
        return plan;
    }
    plan.steps.extend(relabel_steps(env, id, &mode, p, &cwd));
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
    let step = |cmd: Vec<String>, why: String| Step { id: Some(id.clone()), cmd, cwd: Some(cwd.to_string()), why };
    let mut v = vec![];
    if let Some(short) = &p.short_id {
        if p.pid.is_some() {
            v.push(step(crate::claude::stop(env, short), "a live session is never resumed; stop it first".into()));
        }
        v.push(step(crate::claude::rm(env, short), format!("remove row {short} first: a resume with flags while it exists makes an empty copy")));
    }
    v.push(step(
        crate::claude::relaunch_bg(env, id, mode, &p.session_id),
        format!("resume {} with every flag: same id and history, new name and labels", &p.session_id[..p.session_id.len().min(8)]),
    ));
    v
}

/// Remove job rows: a member's stale rows (`target` = identity), or one row of
/// a stowaway / non-fleet session (`target` = short id or session id).
pub fn plan_rm(env: &Env, snap: &Snapshot, target: &str, caller: &Caller) -> Plan {
    let mut plan = Plan { summary: format!("rm {target}"), ..Default::default() };
    // An identity: its stale rows (same as sweep <id>).
    if let Some((_, m)) = member(snap, &Id(target.to_string())) {
        if let Err(why) = may_start(caller, &m.id, "rm") {
            refuse(&mut plan, "authority", why, None, Some(m.id.clone()));
            return plan;
        }
        sweep_member(env, &mut plan, m);
        if plan.steps.is_empty() && plan.refusals.is_empty() {
            plan.warnings.push(issue("copy.rows", Severity::Info, format!("{target} has no stale rows"), None, Some(m.id.clone())));
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
            .filter(|r| labels.get(&r.session_id).and_then(|l| l.get("NIMITZ_ID")).map(|o| binding.iter().any(|(_, m)| &m.id.0 == o)).unwrap_or(false))
            .map(|r| r.info(Default::default()))
    };
    // a job row a 0.3.x tender left: history only, removed by its short id
    let retired = retired_tender_row(env, target);
    let Some(row) = all.iter().copied().find(|r| r.short_id.as_deref() == Some(target) || r.session_id == target).or(misplaced.as_ref()).or(retired.as_ref().map(|(_, r)| r)) else {
        refuse(&mut plan, if *caller == Caller::User { "session.unknown" } else { "authority" }, format!("{target} names no identity and no job row"), None, None);
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
            labels.get(&row.session_id).and_then(|l| l.get("NIMITZ_ID")).map(|o| Id(o.clone())).filter(|_| misplaced.as_ref().is_some_and(|m| m.session_id == row.session_id))
        })
        .or_else(|| retired.as_ref().map(|(id, _)| id.clone()));
    if let Err(why) = may_touch_row(snap, caller, owner.as_ref(), row, "rm") {
        refuse(&mut plan, "authority", why, None, owner);
        return plan;
    }
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
    let current = binding.iter().any(|(_, m)| m.session.as_ref().map(|p| p.session_id == row.session_id).unwrap_or(false));
    if current {
        refuse(
            &mut plan,
            "lineage.current",
            format!("session {} is an identity's current session; removing it breaks the lineage", row.session_id),
            Some("nimitz relaunch <id>  # if you mean to replace it".into()),
            None,
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
/// is the retired identity, so the authority table judges `stop` and `rm` as
/// for a ship of that squadron.
fn retired_tender_row(env: &Env, target: &str) -> Option<(Id, SessionInfo)> {
    let rows = crate::evidence::rows_from_files(env, &mut crate::evidence::FileCache::default()).unwrap_or_default();
    let labels = crate::evidence::session_labels(&crate::evidence::events(env), &rows);
    let r = rows.iter().find(|r| r.short_id.as_deref() == Some(target) || r.session_id == target)?;
    let by_labels = labels.get(&r.session_id).and_then(|l| l.get("NIMITZ_ID")).filter(|o| crate::migrate::is_tender_id(o)).cloned();
    let owner = by_labels.or_else(|| {
        let reg = crate::registry::load(env);
        reg.retired
            .iter()
            .find(|(_, rec)| rec.job.as_deref() == r.short_id.as_deref() && rec.job.is_some() || rec.session_ids().any(|s| r.session_ids().any(|x| x == s)))
            .map(|(id, _)| id.clone())
    })?;
    Some((Id(owner), r.info(labels.get(&r.session_id).cloned().unwrap_or_default())))
}

fn sweep_member(env: &Env, plan: &mut Plan, m: &Member) {
    for r in &m.extra_rows {
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

/// Drop every stale copy row: everything, a fleet, a squadron or one identity
/// (`id` and every member under it). Authority (contracts.md section 8): you,
/// any scope; a commodore, its own squadron only (the stale rows of its own
/// ships); everyone else nothing.
pub fn plan_sweep(env: &Env, snap: &Snapshot, id: Option<&Id>, caller: &Caller) -> Plan {
    let scope = match (id, caller) {
        (Some(i), _) => i.0.clone(),
        (None, Caller::Member { id: me, role: Role::Commodore | Role::Admiral }) => me.0.clone(),
        (None, _) => "everything".into(),
    };
    let mut plan = Plan { summary: format!("sweep {scope}"), ..Default::default() };
    let under = |m: &Id, t: &Id| m == t || m.0.starts_with(&format!("{}/", t.0));
    let mine: Box<dyn Fn(&Id) -> bool> = match caller {
        Caller::User => Box::new(|_| true),
        Caller::Member { id: me, role: Role::Commodore } => {
            let me = me.clone();
            let target_ok = id.map(|t| t == &me || t.parent().as_ref() == Some(&me)).unwrap_or(true);
            if !target_ok {
                let t = id.map(|t| t.0.clone()).unwrap_or_default();
                refuse(&mut plan, "authority", format!("{} may sweep only its own squadron, not {t}", me.name()), None, id.cloned());
                return plan;
            }
            Box::new(move |m: &Id| m.parent().as_ref() == Some(&me))
        }
        Caller::Member { id: me, role: Role::Admiral } => {
            let fleet = me.fleet().to_string();
            if id.is_some_and(|t| t.fleet() != fleet) {
                let t = id.map(|t| t.0.clone()).unwrap_or_default();
                refuse(&mut plan, "authority", format!("{} may sweep only its own fleet, not {t}", me.name()), None, id.cloned());
                return plan;
            }
            Box::new(move |m: &Id| m.fleet() == fleet)
        }
        // a ship: its own stale copy rows, nothing else
        Caller::Member { id: me, role: Role::Ship } if id == Some(me) => {
            let me = me.clone();
            Box::new(move |m: &Id| *m == me)
        }
        Caller::Member { id: me, .. } => {
            refuse(&mut plan, "authority", format!("{} may sweep only itself (nimitz sweep {}); you, its admiral or its commodore sweep the rest", me.name(), me), None, id.cloned());
            return plan;
        }
    };
    if let Some(t) = id {
        if !members(snap).iter().any(|(_, m)| under(&m.id, t)) {
            refuse(&mut plan, "member.unknown", format!("{t} names no fleet member"), Some("nimitz status".into()), Some(t.clone()));
            return plan;
        }
    }
    for (_, m) in members(snap) {
        if id.map(|t| under(&m.id, t)).unwrap_or(true) && mine(&m.id) {
            sweep_member(env, &mut plan, m);
        }
    }
    // Stopped rows of the old fleet (Info `legacy`): offered here, never removed
    // by a start. The transcripts stay.
    let (fleets, _) = crate::config::load_all(env);
    let legacy: HashMap<String, Id> = fleets.iter().flat_map(crate::config::legacy_map).collect();
    for st in snap.stowaways.iter().filter(|s| s.pid.is_none()) {
        let (Some(owner), Some(short)) = (legacy.get(&st.name), st.short_id.as_ref()) else { continue };
        if id.map(|t| under(owner, t)).unwrap_or(true) && mine(owner) && member(snap, owner).is_some() {
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
    if let Err(why) = may_start(caller, id, "adopt as") {
        refuse(&mut plan, "authority", why, None, Some(id.clone()));
        return plan;
    }
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
    // a member adopts only what sits in its own fleet's folders
    if *caller != Caller::User && crate::snapshot::stowaway_fleet(snap, row).as_deref() != Some(id.fleet()) {
        refuse(&mut plan, "authority", format!("session {session_id} does not sit in fleet {}'s folders; adopting it is the user's", id.fleet()), None, Some(id.clone()));
        return plan;
    }
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
        Some(format!("nimitz relaunch {id}")),
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
    // a fleet goes only by you; a squadron by its admiral or you (not by its own
    // commodore); a ship by its admiral, its commodore or you
    let allowed = match (id.role(), caller) {
        (_, Caller::User) => Ok(()),
        (Role::Admiral, Caller::Member { id: me, .. }) => Err(format!("{} may not remove fleet {id}: creating and removing fleets is yours alone", caller_name(me))),
        (Role::Commodore, Caller::Member { id: me, role: Role::Commodore }) if me == id => {
            Err(format!("{} may not remove its own squadron; its admiral or you do", caller_name(me)))
        }
        _ => may_config(caller, id),
    };
    if let Err(why) = allowed {
        refuse(&mut plan, "authority", why, None, None);
        return plan;
    }
    let cfg = crate::config::plan_rm(env, id);
    plan.refusals.extend(cfg.refusals.iter().cloned());
    let under = |m: &Id| m == id || m.0.starts_with(&format!("{id}/"));
    for (_, m) in members(snap).into_iter().filter(|(_, m)| under(&m.id)) {
        for r in m.session.iter().chain(m.extra_rows.iter()) {
            let live = r.pid.is_some();
            if live && (r.kind != "background" || id.role() != Role::Ship) {
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
    Interactive,
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
        ["mkdir", _] | ["write", _, _] | ["archive", _] => Kind::Config,
        [_, "stop", short] => Kind::Stop { short: short.to_string() },
        [_, "rm", short] => Kind::Rm { short: short.to_string() },
        _ if !step.cmd.contains(&"--bg".to_string()) => Kind::Interactive,
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

/// The session id a `--resume <id>` step continues.
fn resumed_id(step: &Step) -> Option<String> {
    let i = step.cmd.iter().position(|a| a == "--resume")?;
    step.cmd.get(i + 1).cloned()
}

/// How long `apply` waits for a background session's SessionStart event.
pub const BIND_TIMEOUT_S: f64 = 15.0;

/// Wait for the SessionStart event that binds `id` to a session, appended at or
/// after `since`. Returns the session id.
fn wait_for_binding(env: &Env, id: &Id, since: f64, timeout_s: f64, cwd: Option<&str>) -> Option<String> {
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
            if e.ts >= since && e.event == "SessionStart" && e.labels.get("NIMITZ_ID") == Some(&id.0) {
                if let Some(sid) = e.session_id.as_ref().filter(|s| !elsewhere(s)) {
                    return Some(sid.clone());
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
/// or after `since`), check a flag-free resume kept `expect`, record the id in
/// the registry (`via` = launch | resume | relaunch), then re-read the files:
/// live, named right, labelled right. `apply` calls this for every background
/// start; the CLI calls it after it opened an interactive member's pane step
/// (`is_pane_step`), with `since` taken just before it opened the pane.
pub fn bind_started(
    env: &Env,
    id: &Id,
    cwd: Option<&str>,
    since: f64,
    expect: Option<&str>,
    via: &str,
    timeout_s: f64,
) -> Result<String, Vec<Issue>> {
    let Some(sid) = wait_for_binding(env, id, since, timeout_s, cwd) else {
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
            Some(format!("nimitz relaunch {id}")),
            Some(id.clone()),
        ));
    }
    if labels.get(sid).map(|l| l.get("NIMITZ_ID") != Some(&id.0)).unwrap_or(true) {
        out.push(issue(
            "labels.missing",
            Severity::Error,
            format!("{id}'s new session {sid} carries no (or foreign) labels"),
            Some(format!("nimitz relaunch {id}")),
            Some(id.clone()),
        ));
    }
    out
}

/// Run a plan's steps, then re-read state and verify (never trust exit codes alone).
///
/// Pane steps (interactive members, `is_pane_step`) are not run: they appear in
/// `ran` with code 0 and a text starting `pane:`. They sit at the top of a
/// top-down plan, so the CLI opens them first, calls `bind_started` for each,
/// and then calls `apply`. `verified` speaks for the steps `apply` ran.
pub fn apply(env: &Env, plan: &Plan) -> Outcome {
    let mut out = Outcome::default();
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
    let mut started: Vec<(Id, String, Option<String>)> = vec![]; // (identity, session id, folder) to verify again
    for (at, step) in plan.steps.iter().enumerate() {
        let kind = classify(step);
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
        if kind == Kind::Interactive {
            out.ran.push((step.clone(), 0, "pane: open this command in a terminal pane, then bind_started".into()));
            continue;
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
                    step.id.as_ref().map(|i| format!("nimitz relaunch {i}")),
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
        let since = crate::evidence::now() - 0.001;
        let (rc, text) = crate::claude::run(&step.cmd, step.cwd.as_deref());
        out.ran.push((step.clone(), rc, text.clone()));
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
                let (expect, via) = match &kind {
                    Kind::Resume { session_id } => (Some(session_id.as_str()), "resume"),
                    Kind::Launch => (None, "launch"),
                    _ if step.why.starts_with(MIGRATE_WHY) => (resumed.as_deref(), "migrate"),
                    _ => (resumed.as_deref(), "relaunch"),
                };
                match bind_started(env, id, step.cwd.as_deref(), since, expect, via, BIND_TIMEOUT_S) {
                    Ok(sid) => {
                        clear_pending(env, id);
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
            }
            Kind::Bind | Kind::Interactive | Kind::Config => unreachable!(),
        }
    }
    // Verify again once every step ran (an rm must not have hit the new row).
    for (id, sid, cwd) in &started {
        out.issues.extend(verify_live(env, id, sid, cwd.as_deref()));
    }
    out.verified = out.issues.iter().all(|i| i.severity != Severity::Error) && out.ran.len() == plan.steps.len();
    out
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
    let _ = std::fs::write(p, serde_json::json!({"id": id.0, "session_id": session_id, "at": crate::evidence::now()}).to_string());
}

fn clear_pending(env: &Env, id: &Id) {
    let _ = std::fs::remove_file(pending_path(env, id));
}

/// The session an interrupted relaunch of `id` was about to resume, if any.
pub fn pending_relaunch(env: &Env, id: &Id) -> Option<String> {
    let v = crate::evidence::first_json(&std::fs::read(pending_path(env, id)).ok()?)?;
    v.get("session_id").and_then(|s| s.as_str()).map(str::to_string)
}

/// An identity's start lock (flock on `NIMITZ_STATE/locks/<id>.lock`); released on drop.
pub struct IdLock(#[allow(dead_code)] std::fs::File);

/// Take `id`'s start lock; the flag says whether another nimitz held it first.
pub fn lock_identity(env: &Env, id: &Id) -> std::io::Result<(IdLock, bool)> {
    use std::os::unix::io::AsRawFd;
    let dir = env.state.join("locks");
    std::fs::create_dir_all(&dir)?;
    let name: String = id.0.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join(format!("{name}.lock")))?;
    let waited = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0;
    if waited && unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((IdLock(f), waited))
}

fn live_row(env: &Env, pick: impl Fn(&crate::evidence::SessionRow) -> bool) -> Option<crate::evidence::SessionRow> {
    let rows = crate::evidence::rows_from_files(env, &mut crate::evidence::FileCache::default()).unwrap_or_default();
    rows.into_iter().find(|r| (r.live() || r.unreadable) && pick(r))
}

/// A live session of `id` right now: its registry job or session, or a row
/// with its labels **in its own folder** (`cwd`). A session carrying the labels in
/// another folder is an impostor (BUG-30) and never counts (BUG-55).
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
                || (labels.get(&r.session_id).and_then(|l| l.get("NIMITZ_ID")) == Some(&id.0) && here(r)))
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
    let line = serde_json::json!({"machine": env.hostname, "id": id.0, "session_id": session_id, "pid": pid, "at": crate::evidence::now()});
    let _ = std::fs::write(ship_lock_path(cwd), line.to_string());
}

/// Release it on a stop, but never another machine's lock.
fn release_folder(env: &Env, id: Option<&Id>, cwd: Option<&str>) {
    let Some(cwd) = cwd.filter(|_| id.map(|i| i.role() == Role::Ship).unwrap_or(false)) else { return };
    if lock_holder(cwd).map(|m| env.matches_hostname(&m)).unwrap_or(false) {
        let _ = std::fs::remove_file(ship_lock_path(cwd));
    }
}

/// The job row running `sid` now (or born as it), if any.
fn find_row_by_session(env: &Env, sid: &str) -> Option<crate::evidence::SessionRow> {
    let mut cache = crate::evidence::FileCache::default();
    crate::evidence::rows_from_files(env, &mut cache).unwrap_or_default().into_iter().find(|r| r.short_id.is_some() && r.session_ids().any(|s| s == sid))
}

fn find_row(env: &Env, short: &str) -> Option<crate::evidence::SessionRow> {
    let mut cache = crate::evidence::FileCache::default();
    crate::evidence::rows_from_files(env, &mut cache).unwrap_or_default().into_iter().find(|r| r.short_id.as_deref() == Some(short))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn caller_needs_matching_labels() {
        let m = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        assert_eq!(caller_from_env(&m(&[])), Caller::User);
        assert_eq!(
            caller_from_env(&m(&[("NIMITZ_ID", "F/SQ"), ("NIMITZ_ROLE", "commodore")])),
            Caller::Member { id: Id::commodore("F", "SQ"), role: Role::Commodore }
        );
        // partial or disagreeing labels: an unverified caller with no authority, never the user
        let unverified = Caller::Member { id: Id(UNVERIFIED.into()), role: Role::Ship };
        assert_eq!(caller_from_env(&m(&[("NIMITZ_ID", "F/SQ/S"), ("NIMITZ_ROLE", "commodore")])), unverified);
        assert_eq!(caller_from_env(&m(&[("NIMITZ_ID", "F/SQ/~fix"), ("NIMITZ_ROLE", "tender")])), unverified, "a 0.3.x tender has no authority");
        assert_eq!(caller_from_env(&m(&[("NIMITZ_ID", "F/SQ/S")])), unverified);
        assert_eq!(caller_from_env(&m(&[("NIMITZ_ROLE", "ship")])), unverified);
        assert_eq!(caller_from_env(&m(&[("NIMITZ_ID", "not an id"), ("NIMITZ_ROLE", "admiral")])), unverified);
        // NIMITZ_HOME alone is no label at all
        assert_eq!(caller_from_env(&m(&[("NIMITZ_HOME", "/h")])), Caller::User);
        assert!(may_start(&unverified, &Id::ship("F", "SQ", "S"), "start").unwrap_err().contains("partial or inconsistent"));
    }

    #[test]
    fn authority_is_the_5_5_table() {
        let ship = Id::ship("F", "SQ", "S");
        let own = Caller::Member { id: Id::commodore("F", "SQ"), role: Role::Commodore };
        let other = Caller::Member { id: Id::commodore("F", "ZZ"), role: Role::Commodore };
        let admiral = Caller::Member { id: Id::admiral("F"), role: Role::Admiral };
        assert!(may_start(&Caller::User, &ship, "start").is_ok());
        assert!(may_start(&own, &ship, "start").is_ok());
        assert!(may_start(&other, &ship, "start").is_err());
        assert!(may_start(&own, &Id::commodore("F", "SQ"), "start").is_err(), "not itself");
        assert!(may_start(&own, &Id::admiral("F"), "start").is_err(), "not its admiral");
        // an admiral: every member of its own fleet but itself
        assert!(may_start(&admiral, &ship, "start").is_ok());
        assert!(may_start(&admiral, &Id::commodore("F", "ZZ"), "relaunch").is_ok());
        assert!(may_start(&admiral, &Id::admiral("F"), "stop").is_err(), "not itself");
        assert!(may_start(&admiral, &Id::ship("G", "SQ", "S"), "start").is_err(), "not another fleet");
        let msg = may_start(&admiral, &Id::ship("G", "SQ", "S"), "start").unwrap_err();
        assert!(msg.contains("an admiral, the members of its own fleet"), "{msg}");
        // ships: nothing
        let a_ship = Caller::Member { id: ship.clone(), role: Role::Ship };
        assert!(may_start(&a_ship, &Id::ship("F", "SQ", "T"), "start").is_err());
    }

    #[test]
    fn config_authority_follows_the_same_scopes() {
        let own = Caller::Member { id: Id::commodore("F", "SQ"), role: Role::Commodore };
        let admiral = Caller::Member { id: Id::admiral("F"), role: Role::Admiral };
        let a_ship = Caller::Member { id: Id::ship("F", "SQ", "S"), role: Role::Ship };
        for scope in [Id::admiral("F"), Id::commodore("F", "ZZ"), Id::ship("F", "ZZ", "X")] {
            assert!(may_config(&admiral, &scope).is_ok(), "{scope}");
            assert!(may_config(&Caller::User, &scope).is_ok());
        }
        assert!(may_config(&admiral, &Id::admiral("G")).is_err(), "another fleet");
        assert!(may_config(&own, &Id::commodore("F", "SQ")).is_ok(), "its squadron's settings");
        assert!(may_config(&own, &Id::ship("F", "SQ", "NEW")).is_ok(), "its ships");
        assert!(may_config(&own, &Id::ship("F", "ZZ", "X")).is_err());
        assert!(may_config(&own, &Id::admiral("F")).is_err(), "not the fleet's settings");
        assert!(may_config(&a_ship, &Id::ship("F", "SQ", "S")).is_err(), "a ship changes no config, not even its own");
    }

    #[test]
    fn a_fix_is_a_members_when_the_table_lets_it_run_it() {
        let own = Caller::Member { id: Id::commodore("F", "SQ"), role: Role::Commodore };
        let admiral = Caller::Member { id: Id::admiral("F"), role: Role::Admiral };
        let a_ship = Caller::Member { id: Id::ship("F", "SQ", "S"), role: Role::Ship };
        for fix in ["nimitz start F/SQ/S", "nimitz relaunch F/SQ/S", "nimitz sweep F/SQ", "nimitz ship set F/SQ/S --mode auto", "nimitz squadron set F/SQ --comms open"] {
            assert!(may_run(&own, fix), "commodore: {fix}");
            assert!(may_run(&admiral, fix), "admiral: {fix}");
        }
        for fix in ["nimitz up F", "nimitz fleet set F --admiral-mode auto", "nimitz squadron new F/NEW", "nimitz relaunch F/SQ", "nimitz layout herdr F"] {
            assert!(may_run(&admiral, fix), "admiral: {fix}");
            assert!(!may_run(&own, fix), "not the commodore's: {fix}");
        }
        for fix in ["nimitz fleet new G", "nimitz fleet rm F", "nimitz machine add F box", "nimitz init", "nimitz stop 1a2b3c4d", "nimitz start G/SQ/S", "claude rm x"] {
            assert!(!may_run(&admiral, fix), "the user's: {fix}");
        }
        assert!(may_run(&own, "nimitz start F/SQ/S  # if it holds work"), "a comment is not an argument");
        // a ship (plan-v4 section 12): every read-only command, signal within
        // its comms, sweep of itself; nothing that changes another member
        for fix in ["nimitz status", "nimitz status --json", "nimitz check F/SQ/S", "nimitz check F", "nimitz rules", "nimitz rules F/SQ", "nimitz doctor", "nimitz sweep F/SQ/S"] {
            assert!(may_run(&a_ship, fix), "a ship's: {fix}");
        }
        for fix in [
            "nimitz start F/SQ/S",
            "nimitz stop F/SQ/S",
            "nimitz relaunch F/SQ/S",
            "nimitz rm F/SQ/S",
            "nimitz sweep F/SQ",
            "nimitz sweep F/SQ/T",
            "nimitz sweep",
            "nimitz ship set F/SQ/S --mode auto",
            "nimitz doctor --fix",
            "nimitz adopt sid F/SQ/S",
            "nimitz up F",
            "nimitz open F/SQ/S",
        ] {
            assert!(!may_run(&a_ship, fix), "not a ship's: {fix}");
        }
        // signal: the chain of command; with no config a ship reaches only its commodore and admiral
        assert!(may_run(&a_ship, "nimitz signal F/SQ hello") && may_run(&a_ship, "nimitz signal F hello"));
        assert!(!may_run(&a_ship, "nimitz signal F/SQ/T hello") && !may_run(&a_ship, "nimitz signal F/ZZ hello"));
        assert!(may_run(&own, "nimitz signal F/SQ/S go") && may_run(&own, "nimitz signal F report"));
        assert!(!may_run(&own, "nimitz signal F/ZZ/X go"));
        assert!(may_run(&admiral, "nimitz signal F/SQ/S go") && may_run(&admiral, "nimitz signal F/ZZ go"));
        assert!(!may_run(&admiral, "nimitz signal G/SQ go") && !may_run(&admiral, "nimitz signal F go"), "not another fleet, not itself");
        // and with the config: the squadron's comms rule decides ship to ship
        let (_td, env) = crate::config::tests::test_env();
        crate::config::tests::make_fleet(&env); // MAIN/ARGOS, comms open
        let argos = Caller::Member { id: Id::ship("MAIN", "ARGOS", "ARGOS"), role: Role::Ship };
        assert!(may_run_in(Some(&env), &argos, "nimitz signal MAIN/ARGOS/OTHER hi"), "open comms");
        assert!(!may_run_in(Some(&env), &argos, "nimitz signal MAIN/ZZ/OTHER hi"), "open within the squadron only");
        assert!(!may_run_in(Some(&env), &argos, "nimitz signal OTHER hi"), "another fleet's admiral");
    }

    #[test]
    fn a_ship_sweeps_only_itself() {
        let (_td, env) = crate::config::tests::test_env();
        crate::config::tests::make_fleet(&env);
        let snap = crate::check::tests::snap_for(&env, None);
        let me = Id::ship("MAIN", "ARGOS", "ARGOS");
        let ship = Caller::Member { id: me.clone(), role: Role::Ship };
        let p = plan_sweep(&env, &snap, Some(&me), &ship);
        assert!(p.ok(), "{:?}", p.refusals);
        assert_eq!(p.summary, "sweep MAIN/ARGOS/ARGOS");
        for target in [None, Some(Id::commodore("MAIN", "ARGOS")), Some(Id::admiral("MAIN"))] {
            let p = plan_sweep(&env, &snap, target.as_ref(), &ship);
            assert_eq!(p.refusals.len(), 1, "{target:?}: {:?}", p.refusals);
            assert_eq!(p.refusals[0].code, "authority");
            assert!(p.refusals[0].message.contains("nimitz sweep MAIN/ARGOS/ARGOS"), "{}", p.refusals[0].message);
        }
    }

    #[test]
    fn classify_reads_argv_back() {
        fn s(cmd: &[&str]) -> Step {
            Step { id: None, cmd: cmd.iter().map(|x| x.to_string()).collect(), cwd: None, why: String::new() }
        }
        assert_eq!(classify(&s(&["claude", "--bg", "--resume", "sid"])), Kind::Resume { session_id: "sid".into() });
        assert_eq!(classify(&s(&["claude", "--bg", "--resume", "sid", "--name", "n", "--settings", "{}"])), Kind::Relaunch);
        assert_eq!(classify(&s(&["claude", "--bg", "--name", "n", "--settings", "{}", "p"])), Kind::Launch);
        assert_eq!(classify(&s(&["claude", "-c", "--name", "n", "--settings", "{}"])), Kind::Interactive);
        assert_eq!(classify(&s(&["/n/claude", "stop", "j1"])), Kind::Stop { short: "j1".into() });
        assert_eq!(classify(&s(&["bind", "F/SQ/S", "sid", "adopt"])), Kind::Bind);
        assert_eq!(classify(&s(&["write", "/x/squadron.toml", "body"])), Kind::Config);
        assert_eq!(classify(&s(&["archive", "/x/fleet.toml"])), Kind::Config);
    }
}
