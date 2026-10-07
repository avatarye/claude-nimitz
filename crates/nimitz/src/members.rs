//! Commands on running members: status, check, up, start, stop, relaunch, rm,
//! sweep, adopt, signal, open, layout. The plans come from core.
use crate::run::{self, refusal};
use crate::{out, Ctx};
use nimitz_core::sessions as ses;
use nimitz_core::{Caller, Id, Issue, Member, Plan, Severity, Snapshot, Status, Step};

/// Parse and shape-check an identity id (contracts.md section 4).
pub fn parse_id(s: &str) -> Result<Id, String> {
    let parts: Vec<&str> = s.split('/').collect();
    let ok = (1..=3).contains(&parts.len()) && parts.iter().all(|p| nimitz_core::config::valid_name(p));
    if ok {
        Ok(Id(s.to_string()))
    } else {
        Err(format!("{s:?} is not an id (F, F/SQ or F/SQ/SHIP; names are [A-Z0-9_]+)"))
    }
}

pub fn members(snap: &Snapshot) -> Vec<&Member> {
    let mut v = vec![];
    for f in &snap.fleets {
        v.push(&f.admiral);
        for sq in &f.squadrons {
            v.push(&sq.commodore);
            v.extend(sq.ships.iter());
        }
    }
    v
}

pub fn member<'a>(snap: &'a Snapshot, id: &Id) -> Option<&'a Member> {
    members(snap).into_iter().find(|m| &m.id == id)
}

fn members_mut(snap: &mut Snapshot) -> Vec<&mut Member> {
    let mut v = vec![];
    for f in &mut snap.fleets {
        v.push(&mut f.admiral);
        for sq in &mut f.squadrons {
            v.push(&mut sq.commodore);
            v.extend(sq.ships.iter_mut());
        }
    }
    v
}

fn live(m: &Member) -> bool {
    m.session.as_ref().map(|s| s.pid.is_some()).unwrap_or(false)
}

fn status_word(s: Status) -> &'static str {
    match s {
        Status::NotStarted => "not-started",
        Status::Stopped => "stopped",
        Status::Parked => "parked",
        Status::Idle => "idle",
        Status::Busy => "busy",
        Status::NeedsYou => "needs-you",
        Status::Unknown => "unknown",
    }
}

// ------------------------------------------------------------------ status, check

pub fn status(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(0)?;
    let mut snap = ctx.snap();
    let mux = ctx.mux();
    // the shape once: the panes that show members, and the active set
    let shape = if mux.caps().open { mux.shape(false).ok() } else { None };
    if let Some(sh) = &shape {
        for m in members_mut(&mut snap) {
            if let Some(s) = &m.session {
                m.pane = sh.panes.iter().find(|p| p.session_id.as_deref() == Some(s.session_id.as_str())).map(|p| p.pane.pane_id.clone());
            }
        }
    }
    crate::restore::record_active(ctx, &snap, mux.as_ref(), shape.as_ref());
    if out::json_mode() {
        out::json(&snap);
        return Ok(0);
    }
    let cc = snap.cc_version.clone().unwrap_or_else(|| format!("unknown (`{} --version` gave no version)", ctx.env.native_claude_bin));
    let machine = snap.machine.clone().unwrap_or("(no label)".into());
    let c = nimitz_core::compat::supported();
    if snap.compat_ok {
        out::ok(&format!("machine {machine}; Claude Code {cc} (supported)"));
    } else if snap.cc_version.as_deref().is_some_and(|v| !v.contains('-') && nimitz_core::compat::position(&c, v) == 1) {
        out::warn(&format!("machine {machine}; Claude Code {cc} is newer than nimitz is tested with (tested up to {}); launches go ahead", c.max));
    } else {
        out::warn(&format!("machine {machine}; Claude Code {cc} is outside the supported range: launches refuse"));
    }
    if snap.fleets.is_empty() {
        out::line("no fleets; create one with `nimitz fleet new <F>`");
    }
    // One column for every id: indent + the longest id, so the status lines up.
    let col = snap
        .fleets
        .iter()
        .flat_map(|f| {
            std::iter::once(2 + f.admiral.id.0.len()).chain(f.squadrons.iter().flat_map(|q| {
                std::iter::once(2 + q.commodore.id.0.len()).chain(q.ships.iter().map(|m| 4 + m.id.0.len()))
            }))
        })
        .max()
        .unwrap_or(0)
        .max(26);
    for f in &snap.fleets {
        let here = f.machine.as_deref().map(|m| format!("runs here as {m}")).unwrap_or("does not run on this machine".into());
        out::line(&format!("{}  ({here})", f.id));
        for i in &f.issues {
            out::issue(i);
        }
        member_line(&ctx.env, &f.admiral, 1, col);
        for sq in &f.squadrons {
            member_line(&ctx.env, &sq.commodore, 1, col);
            for i in &sq.issues {
                out::issue(i);
            }
            for m in &sq.ships {
                member_line(&ctx.env, m, 2, col);
            }
        }
    }
    if !snap.stowaways.is_empty() {
        out::line("stowaways (no labels; `nimitz adopt <session> <id>` or `nimitz rm <session>`):");
        for s in &snap.stowaways {
            let short = s.short_id.clone().unwrap_or_else(|| s.session_id.chars().take(8).collect());
            out::line(&format!("  {short:<10} {:<10} {}  {}", s.status.as_deref().unwrap_or("-"), s.name, s.cwd));
        }
    }
    for i in &snap.issues {
        out::issue(i);
    }
    Ok(0)
}

fn member_line(env: &nimitz_core::env::Env, m: &Member, depth: usize, col: usize) {
    let pad = "  ".repeat(depth);
    let width = col.saturating_sub(pad.len());
    let sess = m
        .session
        .as_ref()
        .map(|s| s.short_id.clone().unwrap_or_else(|| s.session_id.chars().take(8).collect()))
        .unwrap_or("-".into());
    let why = m.reasons.first().cloned().unwrap_or_default();
    let pane = m.pane.as_ref().map(|p| format!("  pane {p}")).unwrap_or_default();
    let pinned = if m.session.as_ref().is_some_and(|s| s.pinned) { "  pinned" } else { "" };
    out::line(&format!("{pad}{:<width$} {:<11} {:<9} {why}{pane}{pinned}", m.id.0, status_word(m.status), sess));
    let mut issues = m.issues.clone();
    for i in ses::session_issues(env, m) {
        if !issues.iter().any(|x| x.code == i.code && x.message == i.message) {
            issues.push(i);
        }
    }
    for i in &issues {
        let fix = i.fix.as_ref().map(|f| format!("  fix: {f}")).unwrap_or_default();
        let tag = match i.severity {
            Severity::Error => "[ERROR]",
            Severity::Warn => "[WARN]",
            Severity::Info => "[INFO]",
        };
        out::line(&format!("{pad}    {tag} {} ({}){fix}", i.message, i.code));
    }
}

pub fn check(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(1)?;
    let id = ctx.args.pos.first().map(|s| parse_id(s)).transpose()?;
    let snap = ctx.snap();
    let mut issues = nimitz_core::check::check_tree(&ctx.env, &snap, id.as_ref());
    issues.sort_by(|a, b| b.severity.cmp(&a.severity));
    let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
    if out::json_mode() {
        out::json(&issues);
    } else {
        for i in &issues {
            out::issue(i);
        }
        let scope = id.as_ref().map(|i| i.0.clone()).unwrap_or("everything".into());
        if errors == 0 {
            out::ok(&format!("check {scope}: no errors ({} other finding(s))", issues.len()));
        } else {
            out::error(&format!("check {scope}: {errors} error(s)"));
        }
    }
    Ok(if errors == 0 { 0 } else { 1 })
}

// ------------------------------------------------------------------ starting and stopping

pub fn up(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(1)?;
    let fleet = ctx.args.need(0, "fleet")?;
    parse_id(fleet)?;
    let snap = ctx.snap();
    // the jobs first (every member with a row wakes, the rest cold-launch:
    // `restore --all` plus the first launches), then the shape: the leads'
    // tabs, like `restore`, with the yes already given (plan-v4 section 13)
    let code = run::session_plan(ctx, ses::plan_up(&ctx.env, &snap, fleet, &ctx.caller));
    let mux = ctx.mux();
    if ctx.plan_only() {
        if mux.caps().open {
            out::line(&format!("then: the leads' Herdr tabs (`nimitz restore {fleet} --plan` shows them once the jobs run)"));
        }
        return Ok(code);
    }
    if code != 0 || !mux.caps().open {
        return Ok(code);
    }
    let opts = nimitz_core::restore::RestoreOpts { all: true, shape_only: true, ..Default::default() };
    Ok(crate::restore::run_shape(ctx, Some(fleet), opts, true))
}

/// `nimitz start <id> [--force] [--fresh]`: `--fresh` is a cold launch that
/// starts a new lineage entry (the fix for `history.lost`).
pub fn start(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--fresh"])?;
    ctx.args.at_most(1)?;
    let id = parse_id(ctx.args.need(0, "id")?)?;
    let snap = ctx.snap();
    let opts = ses::StartOpts { force: ctx.force(), fresh: ctx.args.flag("--fresh") };
    Ok(run::session_plan(ctx, ses::plan_start_with(&ctx.env, &snap, &id, &ctx.caller, opts)))
}

pub fn stop(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(1)?;
    let target = ctx.args.need(0, "id or session")?;
    let snap = ctx.snap();
    let plan = match parse_id(target).ok().filter(|id| member(&snap, id).is_some()) {
        Some(id) => ses::plan_stop(&ctx.env, &snap, &id, &ctx.caller),
        None => plan_stop_row(ctx, &snap, target),
    };
    Ok(run::session_plan(ctx, plan))
}

/// `nimitz stop <short>`: one live session by its id (core's plan).
fn plan_stop_row(ctx: &Ctx, snap: &Snapshot, target: &str) -> Plan {
    ses::plan_stop_session(&ctx.env, snap, target, &ctx.caller)
}

pub fn relaunch(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(1)?;
    let id = parse_id(ctx.args.need(0, "id")?)?;
    let snap = ctx.snap();
    Ok(run::session_plan(ctx, ses::plan_relaunch(&ctx.env, &snap, &id, &ctx.caller)))
}

pub fn rm(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(1)?;
    let target = ctx.args.need(0, "id or session")?;
    let snap = ctx.snap();
    Ok(run::session_plan(ctx, ses::plan_rm(&ctx.env, &snap, target, &ctx.caller)))
}

pub fn sweep(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(1)?;
    let id = ctx.args.pos.first().map(|s| parse_id(s)).transpose()?;
    let snap = ctx.snap();
    Ok(run::session_plan(ctx, ses::plan_sweep(&ctx.env, &snap, id.as_ref(), &ctx.caller)))
}

pub fn adopt(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(2)?;
    let session = ctx.args.need(0, "session")?;
    let id = parse_id(ctx.args.need(1, "id")?)?;
    let snap = ctx.snap();
    Ok(run::session_plan(ctx, ses::plan_adopt(&ctx.env, &snap, session, &id, &ctx.caller)))
}

// ------------------------------------------------------------------ signal, open

/// `nimitz signal <id> <text>`: you type to an admiral or commodore in its
/// attached tab (opened first when it has none); a ship gets it relayed
/// through its commodore (plan-v3 13). Sessions signal each other with
/// SendMessage; the hook rules those.
pub fn signal(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    let id = parse_id(ctx.args.need(0, "id")?)?;
    // Typed into a pane: no control characters, no newlines (BUG-43).
    let text = nimitz_mux::sanitize_text(&ctx.args.pos[1..].join(" "));
    if text.is_empty() {
        return Err("missing text (nothing is left after removing control characters)".into());
    }
    let snap = ctx.snap();
    let mut plan = Plan { summary: format!("signal {id}"), ..Default::default() };
    if let Caller::Member { id: from, .. } = &ctx.caller {
        let verdict = nimitz_core::hook::may_signal(&ctx.env, from, &id.name());
        let msg = match verdict {
            Ok(()) => format!("sessions signal with SendMessage(to: \"{}\"); the chain allows it", id.name()),
            Err(why) => why,
        };
        plan.refusals.push(refusal("comms.denied", msg, None, Some(id.clone())));
        return Ok(finish_plan(ctx, &plan));
    }
    let Some(m) = member(&snap, &id) else {
        plan.refusals.push(refusal("config.invalid", format!("{id} is not in any fleet config"), None, Some(id)));
        return Ok(finish_plan(ctx, &plan));
    };
    // Who gets the typing: the member itself, or its commodore for a ship.
    let (via, text) = if m.role.lead() {
        (m, text)
    } else {
        let cid = Id::commodore(id.fleet(), id.squadron().unwrap_or_default());
        match member(&snap, &cid) {
            Some(c) => (c, format!("Relay to {} from the user: {text}", id.name())),
            None => {
                plan.refusals.push(refusal("config.invalid", format!("{id} has no commodore to relay through"), None, Some(id)));
                return Ok(finish_plan(ctx, &plan));
            }
        }
    };
    let mux = ctx.mux();
    if !mux.caps().type_text {
        plan.refusals.push(refusal(
            "mux.none",
            format!("typing to {} needs a multiplexer that can type ({})", via.id, mux.name()),
            Some(nimitz_mux::missing_mux_fix(&ctx.vars)),
            Some(via.id.clone()),
        ));
        return Ok(finish_plan(ctx, &plan));
    }
    if !live(via) {
        plan.refusals.push(refusal("session.none", format!("{} is not live", via.id), Some(format!("nimitz start {}", via.id)), Some(via.id.clone())));
        return Ok(finish_plan(ctx, &plan));
    }
    // its attached tab; with no tab, one is opened first (plan-v4 section 7)
    let pane = via.session.as_ref().and_then(|s| run::pane_of(mux.as_ref(), &s.session_id));
    let why = if via.id == id { "type into its attached tab".to_string() } else { format!("you reach {id} through its commodore") };
    match &pane {
        Some(p) => plan.steps.push(Step { id: Some(via.id.clone()), cmd: vec!["type".into(), p.pane_id.clone(), text.clone()], cwd: None, why }),
        None => {
            let tab = match nimitz_core::layout::attach_tab(&ctx.env, via) {
                Ok(t) => t,
                Err(i) => {
                    plan.refusals.push(Issue { severity: Severity::Error, ..i });
                    return Ok(finish_plan(ctx, &plan));
                }
            };
            plan.steps.push(Step { id: Some(via.id.clone()), cmd: tab.cmd, cwd: Some(tab.cwd), why: format!("{} has no tab yet: attach it in tab {} first", via.id, tab.label) });
            plan.steps.push(Step { id: Some(via.id.clone()), cmd: vec!["type".into(), "<the new pane>".into(), text.clone()], cwd: None, why });
        }
    }
    match ctx.gate(&plan) {
        out::Go::Exit(code) => Ok(code),
        out::Go::Run => {
            let pane = match pane {
                Some(p) => p,
                None => match run::attach_in_tab(mux.as_ref(), &ctx.env, via) {
                    Ok(a) => {
                        let p = attached_pane(&a);
                        out::ok(&format!("{} attached in pane {}", via.id, p.pane_id));
                        // the attach takes a moment: type only once the pane shows the session
                        let sid = via.session.as_ref().map(|s| s.session_id.clone()).unwrap_or_default();
                        let wait: f64 = ctx.vars.get("NIMITZ_SIGNAL_WAIT").and_then(|v| v.parse().ok()).unwrap_or(SIGNAL_ATTACH_WAIT_S);
                        match run::wait_for_session(mux.as_ref(), &sid, wait) {
                            Some(p) => p,
                            None => {
                                out::error(&format!("pane {} does not show {}'s session after {wait} s; nothing was typed. Run the signal again once it is attached", p.pane_id, via.id));
                                return Ok(1);
                            }
                        }
                    }
                    Err(i) => {
                        out::issue(&i);
                        return Ok(1);
                    }
                },
            };
            match mux.type_text(&pane, &text) {
                Ok(()) => {
                    out::ok(&format!("typed to {} in pane {}", via.id, pane.pane_id));
                    Ok(0)
                }
                Err(e) => {
                    out::error(&format!("typing to {} failed: {e}  # the tab may still be attaching; run the signal again in a moment", via.id));
                    Ok(1)
                }
            }
        }
    }
}

/// How long `signal` waits for a tab it just attached to show the session
/// (`NIMITZ_SIGNAL_WAIT` overrides it; tests set 0).
const SIGNAL_ATTACH_WAIT_S: f64 = 10.0;

/// Print a refused (or empty) plan and return its exit code.
fn finish_plan(ctx: &Ctx, plan: &Plan) -> i32 {
    match ctx.gate(plan) {
        out::Go::Exit(code) => code,
        out::Go::Run => 0,
    }
}

/// `nimitz open <id>`: focus the pane that shows the member, or attach to its
/// job in its tab (`claude attach <job>`; detach with ctrl-z).
pub fn open(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(1)?;
    let id = parse_id(ctx.args.need(0, "id")?)?;
    let mut plan = Plan { summary: format!("open {id}"), ..Default::default() };
    // Panes and tabs are the user's terminal: no member opens one.
    if ctx.caller != Caller::User {
        plan.refusals.push(refusal("authority", "`nimitz open` is yours alone (it opens a tab in your terminal)".into(), None, Some(id)));
        return Ok(finish_plan(ctx, &plan));
    }
    let snap = ctx.snap();
    let mux = ctx.mux();
    let Some(m) = member(&snap, &id) else {
        plan.refusals.push(refusal("config.invalid", format!("{id} is not in any fleet config"), None, Some(id)));
        return Ok(finish_plan(ctx, &plan));
    };
    if !mux.caps().open {
        plan.refusals.push(refusal("mux.none", format!("opening {id} needs a multiplexer ({})", mux.name()), Some(nimitz_mux::missing_mux_fix(&ctx.vars)), Some(id)));
        return Ok(finish_plan(ctx, &plan));
    }
    if let Some(pane) = m.session.as_ref().and_then(|s| run::pane_of(mux.as_ref(), &s.session_id)) {
        plan.steps.push(Step { id: Some(id.clone()), cmd: vec!["focus".into(), pane.pane_id.clone()], cwd: None, why: "it already has a pane".into() });
        return match out::gate(&plan, ctx.plan_only(), true) {
            out::Go::Exit(c) => Ok(c),
            out::Go::Run => match mux.focus(&pane) {
                Ok(()) => {
                    out::ok(&format!("focused {id} in pane {}", pane.pane_id));
                    Ok(0)
                }
                Err(e) => {
                    out::error(&format!("focus failed: {e}"));
                    Ok(1)
                }
            },
        };
    }
    let tab = match nimitz_core::layout::attach_tab(&ctx.env, m) {
        Ok(t) => t,
        Err(i) if i.severity == Severity::Error => {
            // history.lost, name.drift, labels.missing: the start's business first
            plan.refusals.push(i);
            return Ok(finish_plan(ctx, &plan));
        }
        Err(i) => {
            // no job, or a process with no job: say so, with what to do
            let fix = if live(m) { None } else { Some(format!("nimitz start {id}")) };
            plan.refusals.push(refusal("session.none", i.message, fix, Some(id)));
            return Ok(finish_plan(ctx, &plan));
        }
    };
    let wake = if tab.wakes { "; it wakes the parked job (cli-07)" } else { "" };
    // what its tab shows now: a pane with it (focus), a shell (claim it), something
    // else (never typed into), or no tab (open one)
    match nimitz_mux::tab_state(mux.as_ref(), &tab.cmd, &run::place_for(&id), tab.session_id.as_deref()) {
        Ok(nimitz_mux::TabState::Shown(p)) => {
            plan.steps.push(Step { id: Some(id.clone()), cmd: vec!["focus".into(), p.pane_id.clone()], cwd: None, why: format!("its tab {} shows it already", tab.label) })
        }
        Ok(nimitz_mux::TabState::Shell(p)) => plan.steps.push(Step {
            id: Some(id.clone()),
            cmd: tab.cmd.clone(),
            cwd: Some(tab.cwd.clone()),
            why: format!("its tab {} is open and pane {} is a plain shell: attach there{wake}", tab.label, p.pane_id),
        }),
        Ok(nimitz_mux::TabState::Taken { pane, running }) => {
            plan.refusals.push(run::attach_issue(&id, nimitz_mux::AttachError::Taken { pane, running }));
            return Ok(finish_plan(ctx, &plan));
        }
        Ok(nimitz_mux::TabState::Missing) => {
            plan.steps.push(Step { id: Some(id.clone()), cmd: tab.cmd.clone(), cwd: Some(tab.cwd.clone()), why: format!("attach in a new tab {}{wake}; detaching (ctrl-z) leaves it running", tab.label) })
        }
        Err(e) => {
            plan.refusals.push(refusal("mux.failed", format!("cannot see what {id}'s tab runs ({e}); nothing is typed into a pane nimitz cannot see into"), None, Some(id)));
            return Ok(finish_plan(ctx, &plan));
        }
    }
    // Attaching changes no session: no confirmation needed.
    let code = match out::gate(&plan, ctx.plan_only(), true) {
        out::Go::Exit(c) => Ok(c),
        out::Go::Run => match run::attach_in_tab(mux.as_ref(), &ctx.env, m) {
            Ok(nimitz_mux::Attached::Focused(p)) => {
                out::ok(&format!("{id} is open already; focused pane {}", p.pane_id));
                Ok(0)
            }
            Ok(nimitz_mux::Attached::Claimed(p)) if tab.wakes => {
                out::ok(&format!("{id}: the attach in its tab's pane {} wakes the parked job; check with nimitz status {id} in a moment", p.pane_id));
                Ok(0)
            }
            Ok(nimitz_mux::Attached::Opened(p)) if tab.wakes => {
                out::ok(&format!("{id}: the attach in pane {} wakes the parked job; check with nimitz status {id} in a moment", p.pane_id));
                Ok(0)
            }
            Ok(nimitz_mux::Attached::Claimed(p)) => {
                out::ok(&format!("{id} attached in its tab's pane {}", p.pane_id));
                Ok(0)
            }
            Ok(nimitz_mux::Attached::Opened(p)) => {
                out::ok(&format!("{id} attached in pane {}", p.pane_id));
                Ok(0)
            }
            Err(i) => {
                out::issue(&i);
                Ok(1)
            }
        },
    };
    if !ctx.plan_only() {
        crate::restore::record_active(ctx, &snap, mux.as_ref(), None);
    }
    code
}

fn attached_pane(a: &nimitz_mux::Attached) -> nimitz_mux::PaneRef {
    match a {
        nimitz_mux::Attached::Focused(p) | nimitz_mux::Attached::Claimed(p) | nimitz_mux::Attached::Opened(p) => p.clone(),
    }
}

// ------------------------------------------------------------------ layout

/// `nimitz layout herdr [<F>] [--ships]`: the shape only, as `nimitz restore`
/// makes it (plan-v4 section 13): a tab attaching each lead (a parked lead
/// with a shell tab wakes by the attach; one with no tab needs `nimitz
/// start` or `nimitz restore` first), `--ships` a tab for each live ship.
/// Kept as an alias for one release.
pub fn layout(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--ships"])?;
    ctx.args.at_most(2)?;
    let which = ctx.args.need(0, "multiplexer (herdr)")?;
    if which != "herdr" {
        return Err(format!("no layout for {which:?}; only `herdr` is built"));
    }
    let only = ctx.args.pos.get(1).map(|s| parse_id(s)).transpose()?;
    out::line("[INFO] `layout herdr` is an alias of `nimitz restore` (the shape only: tabs, no resume); it goes away in a later release");
    let opts = nimitz_core::restore::RestoreOpts { shape_only: true, ships: ctx.args.flag("--ships"), force: ctx.force(), ..Default::default() };
    Ok(crate::restore::run_shape(ctx, only.as_ref().map(|i| i.fleet().to_string()).as_deref(), opts, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_shape_checked() {
        for ok in ["MAIN", "MAIN/ARGOS", "MAIN/ARGOS/CLEAN"] {
            assert!(parse_id(ok).is_ok(), "{ok}");
        }
        // `MAIN/ARGOS/~fix-ci` was a 0.3.x tender: no id any more
        for bad in ["", "main", "MAIN/", "MAIN/ARGOS/CLEAN/X", "MAIN/ARGOS/~", "MAIN/ARGOS/~fix-ci", "MAIN/as tra"] {
            assert!(parse_id(bad).is_err(), "{bad}");
        }
    }
}
