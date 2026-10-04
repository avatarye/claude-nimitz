//! Session actions along the chain of command: start (top-down), stop, relaunch,
//! rm, sweep, tender, adopt; the relaunch state table (contracts.md section 8). Owner: core-sessions.
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
/// `[F]-ADMIRAL`, `[F.SQ]-COMMODORE`, `[F.SQ]-COMMODORE/task`, `[F.SQ]-SHIP`.
/// None for names that are not fleet addresses (teammates, ad-hoc sessions).
pub fn id_from_name(name: &str) -> Option<Id> {
    let rest = name.trim().strip_prefix('[')?;
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
        (Some(sq), r) => match r.strip_prefix("COMMODORE/") {
            Some(task) if !task.is_empty() => Some(Id::tender(fleet, sq, task)),
            Some(_) => None,
            None if crate::config::valid_name(r) => Some(Id::ship(fleet, sq, r)),
            None => None,
        },
        _ => None,
    }
}

/// Who is asking, from the caller's env (labels reach the Bash tool's
/// processes, probe P0.6). No labels, or half of them: the user.
pub fn caller_from_env(vars: &HashMap<String, String>) -> Caller {
    let get = |k: &str| vars.get(k).filter(|v| !v.is_empty());
    match (get("NIMITZ_ID"), get("NIMITZ_ROLE")) {
        (Some(id), Some(role)) => {
            let id = Id(id.clone());
            // the role label must agree with the id's shape; otherwise distrust both
            if id.role().as_str() == role { Caller::Member { role: id.role(), id } } else { Caller::User }
        }
        _ => Caller::User,
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

/// Session-level issues readable from the member and the filesystem alone:
/// name.drift, labels.missing, labels.mismatch, dup.live, copy.rows,
/// role.background, ship.elsewhere. (name.taken and lineage.broken need the
/// whole snapshot or the registry; `snapshot` adds those to `Member::issues`.)
pub fn session_issues(env: &Env, m: &Member) -> Vec<Issue> {
    let mut out = vec![];
    let id = Some(m.id.clone());
    let relaunch = Some(format!("nimitz relaunch {}", m.id));
    let rows: Vec<&SessionInfo> = m.session.iter().chain(m.extra_rows.iter()).collect();
    let live: Vec<&&SessionInfo> = rows.iter().filter(|r| r.pid.is_some()).collect();

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
        if m.role.interactive() && p.kind == "background" {
            out.push(issue(
                "role.background",
                Severity::Error,
                format!("{} (an {}) is a background session", m.id, m.role.as_str()),
                Some(format!(
                    "{} attach {}  # and keep it in a terminal",
                    env.native_claude_bin,
                    p.short_id.as_deref().unwrap_or(&p.session_id)
                )),
                id.clone(),
            ));
        }
    }
    if !m.extra_rows.is_empty() {
        let shorts: Vec<&str> = m.extra_rows.iter().map(|r| r.short_id.as_deref().unwrap_or(r.session_id.as_str())).collect();
        out.push(issue(
            "copy.rows",
            Severity::Warn,
            format!("{} has {} extra job rows (copies or stale rows): {}", m.id, m.extra_rows.len(), shorts.join(", ")),
            Some(format!("nimitz sweep {}", m.id)),
            id.clone(),
        ));
    }
    // A managed ship folder in use on another machine leaves a lock file.
    if m.role == Role::Ship {
        if let Some(l) = m.cwd.as_deref().and_then(lock_holder) {
            if !env.matches_hostname(&l) {
                out.push(issue(
                    "ship.elsewhere",
                    Severity::Error,
                    format!("{}'s folder is in use on {l:?} (lock file)", m.id),
                    Some(format!("stop the ship there first, or remove {}", ship_lock_path(m.cwd.as_deref().unwrap_or("")).display())),
                    id.clone(),
                ));
            }
        }
    }
    out
}

/// The machine named in a folder's ship lock, if the lock exists and parses.
pub fn lock_holder(cwd: &str) -> Option<String> {
    let raw = std::fs::read(ship_lock_path(cwd)).ok()?;
    let v = crate::evidence::first_json(&raw)?;
    v.get("machine").and_then(|m| m.as_str()).map(str::to_string)
}

/// Issues that need the registry: lineage.broken (the current session id has no
/// job row and no transcript). Called by `snapshot` for every member.
pub fn registry_issues(env: &Env, cache: &mut crate::evidence::FileCache, m: &Member, reg: &Registry) -> Vec<Issue> {
    let Some(rec) = reg.identities.get(&m.id.0) else { return vec![] };
    let Some(current) = rec.current.as_deref() else { return vec![] };
    let known = m.session.iter().chain(m.extra_rows.iter()).any(|r| r.session_id == current);
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
        Some(format!("nimitz stop {}  # or adopt it", holder.short_id.as_deref().unwrap_or(&holder.session_id))),
        Some(m.id.clone()),
    ))
}

// ------------------------------------------------------------------ authority

/// May `caller` start (or stop, relaunch) `target`? Plan-v3 section 5.5: you
/// start anything; a commodore its own ships and tenders; everyone else nothing.
fn may_start(caller: &Caller, target: &Id) -> Result<(), String> {
    match caller {
        Caller::User => Ok(()),
        Caller::Member { id, role: Role::Commodore } if target.parent().as_ref() == Some(id) => Ok(()),
        Caller::Member { id, .. } => Err(format!(
            "{} may not start {target}: only you start admirals and commodores; a commodore starts its own ships and tenders",
            Id(id.0.clone()).name()
        )),
    }
}

fn refuse(plan: &mut Plan, code: &str, msg: String, fix: Option<String>, id: Option<Id>) {
    plan.refusals.push(issue(code, Severity::Error, msg, fix, id));
}

// ------------------------------------------------------------------ members in a snapshot

/// Every member of a snapshot, with its fleet.
fn members<'a>(snap: &'a Snapshot) -> Vec<(&'a Fleet, &'a Member)> {
    let mut out = vec![];
    for f in &snap.fleets {
        out.push((f, &f.admiral));
        for sq in &f.squadrons {
            out.push((f, &sq.commodore));
            out.extend(sq.ships.iter().map(|m| (f, m)));
            out.extend(sq.tenders.iter().map(|m| (f, m)));
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

/// One member's action per contracts.md section 8. Appends steps or refusals.
fn start_member(env: &Env, plan: &mut Plan, snap: &Snapshot, id: &Id, force: bool) {
    let Some((fleet, m)) = member(snap, id) else {
        refuse(plan, "config.invalid", format!("{id} is not in any fleet config"), None, Some(id.clone()));
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
        refuse(plan, "config.invalid", format!("{id} has no folder on this machine"), None, Some(id.clone()));
        return;
    };
    let mode = crate::config::permission_mode(&fod, id);
    let session_errs = session_issues(env, m);
    let step = |id: &Id, cmd: Vec<String>, why: &str| Step { id: Some(id.clone()), cmd, cwd: Some(cwd.clone()), why: why.into() };

    if m.session.as_ref().map(|p| p.pid.is_some()).unwrap_or(false) {
        // live with drift / missing labels / a twin / in the background: refuse
        // with the fix (rows 2, 3, 11); live and clean: nothing (row 1)
        if let Some(e) = session_errs.iter().find(|i| i.severity == Severity::Error) {
            plan.refusals.push(e.clone());
        }
        return;
    }
    if let Some(e) = session_errs.iter().find(|i| i.severity == Severity::Error) {
        // e.g. ship.elsewhere on a stopped ship
        plan.refusals.push(e.clone());
        return;
    }
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
        // stopped with its labels: resume = native binary, NO flags, from its folder (row 4)
        Some(p) if p.labels.get("NIMITZ_ID") == Some(&id.0) => {
            if m.role.interactive() {
                plan.steps.push(step(id, crate::claude::interactive(env, id, &mode, true), "continue the interactive session in a pane"));
            } else {
                plan.steps.push(step(id, crate::claude::resume_bg(env, &p.session_id), "resume keeps the id, name, mode and labels (probe 2c1)"));
            }
        }
        // stopped without labels: a flag-free resume would keep none; full relaunch (row 5)
        Some(p) => {
            plan.steps.push(step(
                id,
                crate::claude::relaunch_bg(env, id, &mode, &p.session_id),
                "relaunch with every flag: a copy (source fork) that carries the labels",
            ));
            if let Some(short) = &p.short_id {
                plan.steps.push(step(id, crate::claude::rm(env, short), "drop the old, unlabelled row"));
            }
        }
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
            // no row: an interactive member continues its transcript with `-c` (row 6)
            if m.role.interactive() && !crate::evidence::transcripts_in(env, &cwd).is_empty() {
                plan.steps.push(step(id, crate::claude::interactive(env, id, &mode, true), "continue the interactive session in a pane"));
            } else {
                cold_launch(env, plan, &fod, id, m.role, &mode, &cwd); // row 7
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

fn cold_launch(env: &Env, plan: &mut Plan, fod: &FleetOnDisk, id: &Id, role: Role, mode: &str, cwd: &str) {
    match role {
        Role::Tender => refuse(
            plan,
            "lineage.broken",
            format!("{id} is gone; a tender is one job, it is not restarted cold"),
            Some(format!("nimitz tender {} <task>  # a new tender", id.parent().map(|p| p.0).unwrap_or_default())),
            Some(id.clone()),
        ),
        Role::Ship => {
            let Some(prompt) = crate::config::ship_prompt(fod, id) else {
                refuse(plan, "config.invalid", format!("{id} has no prompt (prompt or prompt_file) in squadron.toml"), None, Some(id.clone()));
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
    const TABLE_HANDLED: [&str; 7] =
        ["name.drift", "labels.missing", "labels.mismatch", "dup.live", "copy.rows", "lineage.broken", "role.background"];
    for i in crate::check::check(env, snap, id) {
        let forceable = i.code == "cc.version";
        match i.severity {
            Severity::Error if !TABLE_HANDLED.contains(&i.code.as_str()) && !(force && forceable) => plan.refusals.push(i),
            _ => plan.warnings.push(i),
        }
    }
}

/// Start `id` and every missing parent, top-down. Runs `check` first.
pub fn plan_start(env: &Env, snap: &Snapshot, id: &Id, caller: &Caller, force: bool) -> Plan {
    let mut plan = Plan { summary: format!("start {id} (top-down)"), ..Default::default() };
    gate(env, &mut plan, snap, Some(id), force);
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
        if let Err(why) = may_start(caller, &link) {
            refuse(&mut plan, "config.invalid", why, None, Some(link.clone()));
            continue;
        }
        start_member(env, &mut plan, snap, &link, force);
    }
    dedup_issues(&mut plan);
    plan
}

/// `nimitz up <fleet>`: start every member that should run on this machine.
pub fn plan_up(env: &Env, snap: &Snapshot, fleet: &str, caller: &Caller) -> Plan {
    let mut plan = Plan { summary: format!("up {fleet}: reconcile the config with the running sessions"), ..Default::default() };
    if *caller != Caller::User {
        refuse(&mut plan, "config.invalid", "`nimitz up` is yours alone; members start nothing fleet-wide".into(), None, None);
        return plan;
    }
    let Some(f) = snap.fleets.iter().find(|f| f.id.fleet() == fleet) else {
        refuse(&mut plan, "config.invalid", format!("no fleet {fleet} in the config"), None, None);
        return plan;
    };
    if f.machine.is_none() {
        refuse(&mut plan, "config.invalid", format!("fleet {fleet} does not run on this machine"), None, None);
        return plan;
    }
    gate(env, &mut plan, snap, Some(&f.id), false);
    let mut ids = vec![f.admiral.id.clone()];
    for sq in &f.squadrons {
        ids.push(sq.commodore.id.clone());
        // ships with a folder on this machine; tenders are one job, `up` leaves them
        ids.extend(sq.ships.iter().filter(|s| s.cwd.is_some()).map(|s| s.id.clone()));
    }
    for id in ids {
        let clean_live = member(snap, &id)
            .map(|(_, m)| {
                m.session.as_ref().map(|p| p.pid.is_some()).unwrap_or(false)
                    && session_issues(env, m).iter().all(|i| i.severity != Severity::Error)
            })
            .unwrap_or(false);
        if !clean_live {
            start_member(env, &mut plan, snap, &id, false);
        }
    }
    dedup_issues(&mut plan);
    plan
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
    if let Err(why) = may_start(caller, id) {
        refuse(&mut plan, "config.invalid", why, None, Some(id.clone()));
        return plan;
    }
    let Some((_, m)) = member(snap, id) else {
        refuse(&mut plan, "config.invalid", format!("{id} is not in any fleet config"), None, Some(id.clone()));
        return plan;
    };
    let Some(p) = m.session.as_ref().filter(|p| p.pid.is_some()) else {
        refuse(&mut plan, "config.invalid", format!("{id} is not live"), None, Some(id.clone()));
        return plan;
    };
    if m.role.interactive() && p.kind != "background" {
        refuse(
            &mut plan,
            "config.invalid",
            format!("{id} is an interactive session; end it in its own terminal"),
            Some("type /exit in its pane".into()),
            Some(id.clone()),
        );
        return plan;
    }
    let Some(short) = &p.short_id else {
        refuse(&mut plan, "config.invalid", format!("{id}'s session {} has no job id to stop", p.session_id), None, Some(id.clone()));
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

/// A deliberate relaunch: new id, every flag, old row removed (contracts.md section 8).
pub fn plan_relaunch(env: &Env, snap: &Snapshot, id: &Id, caller: &Caller) -> Plan {
    let mut plan = Plan { summary: format!("relaunch {id} (new session id, labels restored)"), ..Default::default() };
    if let Err(why) = may_start(caller, id) {
        refuse(&mut plan, "config.invalid", why, None, Some(id.clone()));
        return plan;
    }
    let Some((fleet, m)) = member(snap, id) else {
        refuse(&mut plan, "config.invalid", format!("{id} is not in any fleet config"), None, Some(id.clone()));
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
    if m.role.interactive() {
        if m.session.as_ref().map(|p| p.pid.is_some()).unwrap_or(false) {
            refuse(&mut plan, "config.invalid", format!("{id} is live; end it in its pane first"), Some("type /exit in its pane".into()), Some(id.clone()));
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
        refuse(&mut plan, "config.invalid", format!("{id} has no session to relaunch; use nimitz start"), Some(format!("nimitz start {id}")), Some(id.clone()));
        return plan;
    };
    let step = |cmd: Vec<String>, why: &str| Step { id: Some(id.clone()), cmd, cwd: Some(cwd.clone()), why: why.into() };
    if p.pid.is_some() {
        match &p.short_id {
            Some(short) => plan.steps.push(step(crate::claude::stop(env, short), "a live session is never resumed; stop it first")),
            None => {
                refuse(&mut plan, "config.invalid", format!("{id} is live with no job id; cannot stop it"), None, Some(id.clone()));
                return plan;
            }
        }
    }
    plan.steps.push(step(
        crate::claude::relaunch_bg(env, id, &mode, &p.session_id),
        "relaunch with every flag: a copy (source fork) that carries the labels",
    ));
    if let Some(short) = &p.short_id {
        plan.steps.push(step(crate::claude::rm(env, short), "drop the old row; one row per identity"));
    }
    plan
}

/// Remove job rows: a member's stale rows (`target` = identity), or one row of
/// a stowaway / non-fleet session (`target` = short id or session id).
pub fn plan_rm(env: &Env, snap: &Snapshot, target: &str, caller: &Caller) -> Plan {
    let mut plan = Plan { summary: format!("rm {target}"), ..Default::default() };
    if *caller != Caller::User {
        refuse(&mut plan, "config.invalid", "`nimitz rm` is yours alone".into(), None, None);
        return plan;
    }
    // An identity: its stale rows (same as sweep <id>).
    if let Some((_, m)) = member(snap, &Id(target.to_string())) {
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
    let Some(row) = all.iter().find(|r| r.short_id.as_deref() == Some(target) || r.session_id == target) else {
        refuse(&mut plan, "config.invalid", format!("{target} names no identity and no job row"), None, None);
        return plan;
    };
    if row.pid.is_some() {
        refuse(
            &mut plan,
            "config.invalid",
            format!("session {} is live; stop it first", row.session_id),
            Some(format!("{} stop {}", env.native_claude_bin, row.short_id.as_deref().unwrap_or(&row.session_id))),
            None,
        );
        return plan;
    }
    let current = binding.iter().any(|(_, m)| m.session.as_ref().map(|p| p.session_id == row.session_id).unwrap_or(false));
    if current {
        refuse(
            &mut plan,
            "config.invalid",
            format!("session {} is an identity's current session; removing it breaks the lineage", row.session_id),
            Some("nimitz relaunch <id>  # if you mean to replace it".into()),
            None,
        );
        return plan;
    }
    let Some(short) = &row.short_id else {
        refuse(&mut plan, "config.invalid", format!("session {} has no job row to remove", row.session_id), None, None);
        return plan;
    };
    plan.steps.push(Step { id: None, cmd: crate::claude::rm(env, short), cwd: None, why: "remove the job row; the transcript stays".into() });
    plan
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

/// Drop every stale copy row, fleet-wide or for one identity.
pub fn plan_sweep(env: &Env, snap: &Snapshot, id: Option<&Id>, caller: &Caller) -> Plan {
    let mut plan = Plan { summary: format!("sweep {}", id.map(|i| i.0.clone()).unwrap_or("everything".into())), ..Default::default() };
    if *caller != Caller::User {
        refuse(&mut plan, "config.invalid", "`nimitz sweep` is yours alone".into(), None, None);
        return plan;
    }
    for (_, m) in members(snap) {
        if id.map(|t| t == &m.id).unwrap_or(true) {
            sweep_member(env, &mut plan, m);
        }
    }
    plan
}

/// A tender: one background job for a commodore, in the squadron's folder.
pub fn plan_tender(env: &Env, snap: &Snapshot, commodore: &Id, task: &str, prompt: &str, caller: &Caller) -> Plan {
    let mut plan = Plan { summary: format!("tender {commodore}/{task}"), ..Default::default() };
    if commodore.role() != Role::Commodore {
        refuse(&mut plan, "config.invalid", format!("{commodore} is not a commodore"), None, Some(commodore.clone()));
        return plan;
    }
    let ok = match caller {
        Caller::User => true,
        Caller::Member { id, role: Role::Commodore } => id == commodore,
        _ => false,
    };
    if !ok {
        refuse(&mut plan, "config.invalid", format!("only you or {commodore} itself may start its tenders"), None, Some(commodore.clone()));
        return plan;
    }
    if task.is_empty() || !task.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        refuse(&mut plan, "config.invalid", format!("task {task:?} must be [A-Za-z0-9_-]+"), None, Some(commodore.clone()));
        return plan;
    }
    if prompt.trim().is_empty() {
        refuse(&mut plan, "config.invalid", "a tender needs a prompt (its one job)".into(), None, Some(commodore.clone()));
        return plan;
    }
    let id = Id::tender(commodore.fleet(), commodore.squadron().unwrap_or_default(), task);
    if let Some((_, t)) = member(snap, &id) {
        if t.session.as_ref().map(|p| p.pid.is_some()).unwrap_or(false) {
            refuse(&mut plan, "dup.live", format!("{id} is already live"), None, Some(id.clone()));
            return plan;
        }
    }
    let Some((_, cm)) = member(snap, commodore) else {
        refuse(&mut plan, "config.invalid", format!("{commodore} is not in any fleet config"), None, Some(commodore.clone()));
        return plan;
    };
    let Some(cwd) = cm.cwd.clone() else {
        refuse(&mut plan, "config.invalid", format!("{commodore} has no folder on this machine"), None, Some(commodore.clone()));
        return plan;
    };
    let mode = crate::config::load_fleet(env, commodore.fleet()).map(|f| crate::config::permission_mode(&f, &id)).unwrap_or("default".into());
    plan.steps.push(Step {
        id: Some(id.clone()),
        cmd: crate::claude::launch_bg(env, &id, &mode, Some(prompt)),
        cwd: Some(cwd),
        why: "one background job with a parent label; it stops when the job is done".into(),
    });
    plan
}

/// Adopt a stowaway: record it as `id`'s session. Labels can only be given at a
/// start, so the adoption binds the registry now and warns that the labels
/// arrive at the next relaunch.
pub fn plan_adopt(env: &Env, snap: &Snapshot, session_id: &str, id: &Id, caller: &Caller) -> Plan {
    let _ = env;
    let mut plan = Plan { summary: format!("adopt {session_id} as {id}"), ..Default::default() };
    if *caller != Caller::User {
        refuse(&mut plan, "config.invalid", "`nimitz adopt` is yours alone".into(), None, Some(id.clone()));
        return plan;
    }
    let Some(row) = snap.stowaways.iter().find(|r| r.session_id == session_id || r.short_id.as_deref() == Some(session_id)) else {
        refuse(
            &mut plan,
            "config.invalid",
            format!("{session_id} is not a stowaway (only unlabelled sessions in fleet scope are adopted)"),
            None,
            Some(id.clone()),
        );
        return plan;
    };
    let Some((_, m)) = member(snap, id) else {
        refuse(&mut plan, "config.invalid", format!("{id} is not in any fleet config; add it first"), Some("nimitz ship new ...".into()), Some(id.clone()));
        return plan;
    };
    if m.session.as_ref().map(|p| p.pid.is_some()).unwrap_or(false) {
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

// ------------------------------------------------------------------ apply

/// What a step is, read back from its argv.
#[derive(Debug, PartialEq)]
enum Kind {
    Bind,
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

const BIND_TIMEOUT_S: f64 = 15.0;

/// Wait for the SessionStart event that binds `id` to a session, appended at or
/// after `since`. Returns the session id.
fn wait_for_binding(env: &Env, id: &Id, since: f64, timeout_s: f64) -> Option<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(timeout_s);
    loop {
        for e in crate::evidence::events(env).iter().rev() {
            if e.ts >= since && e.event == "SessionStart" && e.labels.get("NIMITZ_ID") == Some(&id.0) {
                if let Some(sid) = &e.session_id {
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

/// Run a plan's steps, then re-read state and verify (never trust exit codes alone).
pub fn apply(env: &Env, plan: &Plan) -> Outcome {
    let mut out = Outcome::default();
    if !plan.ok() {
        out.issues = plan.refusals.clone();
        return out;
    }
    let mut started: Vec<(Id, String)> = vec![]; // (identity, session id) to verify live
    for step in &plan.steps {
        let kind = classify(step);
        // Interactive members open in a mux pane; core runs nothing for them.
        if kind == Kind::Interactive {
            out.ran.push((step.clone(), 0, "left for the CLI: open this command in a pane".into()));
            continue;
        }
        if kind == Kind::Bind {
            let (id, sid, via) = (Id(step.cmd[1].clone()), step.cmd[2].clone(), step.cmd[3].clone());
            let (role, name) = (id.role(), id.name());
            let cwd = step.cwd.clone().unwrap_or_default();
            match crate::registry::update(env, |r| crate::registry::bind(r, &id, role, &name, &cwd, &sid, &via, crate::evidence::now())) {
                Ok(()) => out.ran.push((step.clone(), 0, format!("bound {sid} to {id} (via {via})"))),
                Err(e) => {
                    out.ran.push((step.clone(), -1, e.to_string()));
                    out.issues.push(issue("lineage.broken", Severity::Error, format!("registry write failed: {e}"), None, Some(id)));
                    return out;
                }
            }
            continue;
        }
        let since = crate::evidence::now() - 0.001;
        let (rc, text) = crate::claude::run(&step.cmd, step.cwd.as_deref());
        out.ran.push((step.clone(), rc, text.clone()));
        if rc != 0 {
            out.issues.push(issue(
                "config.invalid",
                Severity::Error,
                format!("step failed ({rc}): {} -> {text}", step.cmd.join(" ")),
                None,
                step.id.clone(),
            ));
            return out; // a failed step stops the plan; nothing after it is safe
        }
        match kind {
            Kind::Launch | Kind::Relaunch | Kind::Resume { .. } => {
                let Some(id) = &step.id else { continue };
                let Some(sid) = wait_for_binding(env, id, since, BIND_TIMEOUT_S) else {
                    out.issues.push(issue(
                        "lineage.broken",
                        Severity::Error,
                        format!("{id} started but no SessionStart event bound it within {BIND_TIMEOUT_S}s"),
                        Some("check the hooks: nimitz doctor".into()),
                        Some(id.clone()),
                    ));
                    return out;
                };
                let via = match &kind {
                    Kind::Resume { session_id } => {
                        if &sid != session_id {
                            out.issues.push(issue(
                                "lineage.broken",
                                Severity::Error,
                                format!("{id}'s resume was expected to keep {session_id} but bound {sid} (a copy; flags on the resume?)"),
                                None,
                                Some(id.clone()),
                            ));
                            return out;
                        }
                        "resume"
                    }
                    Kind::Launch => "launch",
                    _ => "relaunch",
                };
                let (role, name) = (id.role(), id.name());
                let cwd = step.cwd.clone().unwrap_or_default();
                if let Err(e) =
                    crate::registry::update(env, |r| crate::registry::bind(r, id, role, &name, &cwd, &sid, via, crate::evidence::now()))
                {
                    out.issues.push(issue("lineage.broken", Severity::Error, format!("registry write failed: {e}"), None, Some(id.clone())));
                    return out;
                }
                started.push((id.clone(), sid));
            }
            Kind::Stop { short } => {
                if find_row(env, &short).map(|r| r.live()).unwrap_or(false) {
                    out.issues.push(issue("dup.live", Severity::Error, format!("stop {short} ran but the session is still live"), None, step.id.clone()));
                } // no row at all after a stop is fine too
            }
            Kind::Rm { short } => {
                if find_row(env, &short).is_some() {
                    out.issues.push(issue("copy.rows", Severity::Error, format!("rm {short} ran but the row is still there"), None, step.id.clone()));
                }
            }
            Kind::Bind | Kind::Interactive => unreachable!(),
        }
    }
    // Verify by re-reading, never by exit codes: every started id is live with
    // its name and its labels.
    let mut cache = crate::evidence::FileCache::default();
    let rows = crate::evidence::rows_from_files(env, &mut cache).unwrap_or_default();
    let labels = crate::evidence::labels_by_session(&crate::evidence::events(env));
    for (id, sid) in &started {
        let Some(row) = rows.iter().find(|r| &r.session_id == sid) else {
            out.issues.push(issue("lineage.broken", Severity::Error, format!("{id}'s new session {sid} left no row"), None, Some(id.clone())));
            continue;
        };
        if !row.live() {
            out.issues.push(issue("lineage.broken", Severity::Error, format!("{id}'s new session {sid} is not live"), None, Some(id.clone())));
        }
        if row.name != id.name() {
            out.issues.push(issue(
                "name.drift",
                Severity::Error,
                format!("{id} started as {:?}, expected {:?}", row.name, id.name()),
                None,
                Some(id.clone()),
            ));
        }
        if labels.get(sid).map(|l| l.get("NIMITZ_ID") != Some(&id.0)).unwrap_or(true) {
            out.issues.push(issue("labels.missing", Severity::Error, format!("{id}'s new session {sid} carries no (or foreign) labels"), None, Some(id.clone())));
        }
    }
    out.verified = out.issues.iter().all(|i| i.severity != Severity::Error) && out.ran.len() == plan.steps.len();
    out
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
        for id in [Id::admiral("MAIN"), Id::commodore("MAIN", "ARGOS"), Id::ship("MAIN", "ARGOS", "BOLT"), Id::tender("MAIN", "ARGOS", "fix-ci")] {
            assert_eq!(id_from_name(&id.name()), Some(id.clone()), "{}", id.name());
        }
        assert_eq!(id_from_name("[ADMIRALTY]"), None);
        assert_eq!(id_from_name("scratch notes"), None);
        assert_eq!(id_from_name("[main.ARGOS]-SHIP"), None); // lowercase fleet
        assert_eq!(id_from_name("[F.SQ]-COMMODORE/"), None);
    }

    #[test]
    fn caller_needs_matching_labels() {
        let m = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        assert_eq!(caller_from_env(&m(&[])), Caller::User);
        assert_eq!(
            caller_from_env(&m(&[("NIMITZ_ID", "F/SQ"), ("NIMITZ_ROLE", "commodore")])),
            Caller::Member { id: Id::commodore("F", "SQ"), role: Role::Commodore }
        );
        // a role label that disagrees with the id's shape is distrusted
        assert_eq!(caller_from_env(&m(&[("NIMITZ_ID", "F/SQ/S"), ("NIMITZ_ROLE", "commodore")])), Caller::User);
        assert_eq!(caller_from_env(&m(&[("NIMITZ_ID", "F/SQ/S")])), Caller::User);
    }

    #[test]
    fn authority_is_the_5_5_table() {
        let ship = Id::ship("F", "SQ", "S");
        let own = Caller::Member { id: Id::commodore("F", "SQ"), role: Role::Commodore };
        let other = Caller::Member { id: Id::commodore("F", "ZZ"), role: Role::Commodore };
        let admiral = Caller::Member { id: Id::admiral("F"), role: Role::Admiral };
        assert!(may_start(&Caller::User, &ship).is_ok());
        assert!(may_start(&own, &ship).is_ok());
        assert!(may_start(&own, &Id::tender("F", "SQ", "t")).is_ok());
        assert!(may_start(&other, &ship).is_err());
        assert!(may_start(&admiral, &ship).is_err());
        assert!(may_start(&own, &Id::commodore("F", "SQ")).is_err());
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
    }
}
