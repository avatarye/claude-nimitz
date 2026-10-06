//! Commands on running members: status, check, up, start, stop, relaunch, rm,
//! sweep, tender, adopt, signal, open, layout. The plans come from core.
use crate::run::{self, refusal};
use crate::{out, Ctx};
use nimitz_core::sessions as ses;
use nimitz_core::{Caller, Id, Issue, Member, Plan, Severity, Snapshot, Status, Step};
use nimitz_mux::LayoutPlan;
use std::path::PathBuf;

/// Parse and shape-check an identity id (contracts.md section 4).
pub fn parse_id(s: &str) -> Result<Id, String> {
    let parts: Vec<&str> = s.split('/').collect();
    let ok_tail = |p: &str| match p.strip_prefix('~') {
        Some(t) => !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        None => nimitz_core::config::valid_name(p),
    };
    let ok = match parts.as_slice() {
        [f] => nimitz_core::config::valid_name(f),
        [f, sq] => nimitz_core::config::valid_name(f) && nimitz_core::config::valid_name(sq),
        [f, sq, t] => nimitz_core::config::valid_name(f) && nimitz_core::config::valid_name(sq) && ok_tail(t),
        _ => false,
    };
    if ok {
        Ok(Id(s.to_string()))
    } else {
        Err(format!("{s:?} is not an id (F, F/SQ, F/SQ/SHIP or F/SQ/~task; names are [A-Z0-9_]+)"))
    }
}

pub fn members(snap: &Snapshot) -> Vec<&Member> {
    let mut v = vec![];
    for f in &snap.fleets {
        v.push(&f.admiral);
        for sq in &f.squadrons {
            v.push(&sq.commodore);
            v.extend(sq.ships.iter());
            v.extend(sq.tenders.iter());
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
            v.extend(sq.tenders.iter_mut());
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
    if mux.caps().open {
        let panes = mux.panes();
        for m in members_mut(&mut snap) {
            if let Some(s) = &m.session {
                m.pane = panes.iter().find(|p| p.session_id.as_deref() == Some(s.session_id.as_str())).map(|p| p.pane.pane_id.clone());
            }
        }
    }
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
                std::iter::once(2 + q.commodore.id.0.len()).chain(q.ships.iter().chain(q.tenders.iter()).map(|m| 4 + m.id.0.len()))
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
            for m in sq.ships.iter().chain(sq.tenders.iter()) {
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
    out::line(&format!("{pad}{:<width$} {:<11} {:<9} {why}{pane}", m.id.0, status_word(m.status), sess));
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
    let mut plan = ses::plan_up(&ctx.env, &snap, fleet, &ctx.caller);
    let mux = ctx.mux();
    if !mux.caps().open {
        plan.refusals.push(refusal(
            "mux.none",
            "`nimitz up` runs a whole fleet and needs a multiplexer for its admiral and commodores".into(),
            Some(nimitz_mux::missing_mux_fix(&ctx.vars)),
            None,
        ));
    }
    Ok(run::session_plan(ctx, plan))
}

pub fn start(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(1)?;
    let id = parse_id(ctx.args.need(0, "id")?)?;
    let snap = ctx.snap();
    Ok(run::session_plan(ctx, ses::plan_start(&ctx.env, &snap, &id, &ctx.caller, ctx.force())))
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

pub fn tender(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--prompt"])?;
    let commodore = parse_id(ctx.args.need(0, "commodore id F/SQ")?)?;
    let task = ctx.args.need(1, "task")?;
    let prompt = match ctx.args.value("--prompt") {
        Some(p) => p.to_string(),
        None => ctx.args.pos[2..].join(" "),
    };
    let snap = ctx.snap();
    Ok(run::session_plan(ctx, ses::plan_tender(&ctx.env, &snap, &commodore, task, &prompt, &ctx.caller)))
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
/// pane; a ship or tender gets it relayed through its commodore (plan-v3 13).
/// Sessions signal each other with SendMessage; the hook rules those.
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
    // Who gets the typing: the member itself, or its commodore for a ship/tender.
    let (via, text) = if m.role.interactive() {
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
    let pane = via.session.as_ref().filter(|_| live(via)).and_then(|s| run::pane_of(mux.as_ref(), &s.session_id));
    let Some(pane) = pane else {
        plan.refusals.push(refusal(
            "mux.nopane",
            format!("{} is not live in a {} pane", via.id, mux.name()),
            Some(format!("nimitz start {}", via.id)),
            Some(via.id.clone()),
        ));
        return Ok(finish_plan(ctx, &plan));
    };
    plan.steps.push(Step {
        id: Some(via.id.clone()),
        cmd: vec!["type".into(), pane.pane_id.clone(), text.clone()],
        cwd: None,
        why: if via.id == id { "type into its pane".into() } else { format!("you reach {id} through its commodore") },
    });
    match ctx.gate(&plan) {
        out::Go::Exit(code) => Ok(code),
        out::Go::Run => match mux.type_text(&pane, &text) {
            Ok(()) => {
                out::ok(&format!("typed to {} in pane {}", via.id, pane.pane_id));
                Ok(0)
            }
            Err(e) => {
                out::error(&format!("typing to {} failed: {e}", via.id));
                Ok(1)
            }
        },
    }
}

/// Print a refused (or empty) plan and return its exit code.
fn finish_plan(ctx: &Ctx, plan: &Plan) -> i32 {
    match ctx.gate(plan) {
        out::Go::Exit(code) => code,
        out::Go::Run => 0,
    }
}

/// `nimitz open <id>`: focus its pane, or attach a background member in a new tab.
pub fn open(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(1)?;
    let id = parse_id(ctx.args.need(0, "id")?)?;
    let snap = ctx.snap();
    let mux = ctx.mux();
    let mut plan = Plan { summary: format!("open {id}"), ..Default::default() };
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
    if m.role.interactive() {
        let (msg, fix) = if live(m) {
            (format!("{id} runs in a terminal outside {}; switch to that terminal", mux.name()), None)
        } else {
            (format!("{id} is not running"), Some(format!("nimitz start {id}")))
        };
        plan.refusals.push(refusal("mux.nopane", msg, fix, Some(id)));
        return Ok(finish_plan(ctx, &plan));
    }
    let Some(short) = m.session.as_ref().and_then(|s| s.short_id.clone()) else {
        plan.refusals.push(refusal("config.invalid", format!("{id} has no background session to attach to"), Some(format!("nimitz start {id}")), Some(id)));
        return Ok(finish_plan(ctx, &plan));
    };
    let cwd = m.cwd.clone().unwrap_or_else(|| ".".into());
    let cmd = nimitz_core::claude::attach(&ctx.env, &short);
    plan.steps.push(Step { id: Some(id.clone()), cmd: cmd.clone(), cwd: Some(cwd.clone()), why: "attach in a new tab; detaching leaves it running".into() });
    // Attaching changes no session: no confirmation needed.
    match out::gate(&plan, ctx.plan_only(), true) {
        out::Go::Exit(c) => Ok(c),
        out::Go::Run => match mux.open(&cmd, std::path::Path::new(&cwd), run::place_for(&id)) {
            Ok(p) => {
                out::ok(&format!("{id} attached in pane {}", p.pane_id));
                Ok(0)
            }
            Err(e) => {
                out::error(&format!("open failed: {e}"));
                Ok(1)
            }
        },
    }
}

// ------------------------------------------------------------------ layout

/// `nimitz layout herdr [<F>]`: a workspace per fleet (admiral) and per
/// squadron (commodore). A member that is not running gets a tab that starts it
/// (the step `nimitz start` would run); a running one is left where it is.
pub fn layout(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--ships"])?;
    ctx.args.at_most(2)?;
    let which = ctx.args.need(0, "multiplexer (herdr)")?;
    if which != "herdr" {
        return Err(format!("no layout for {which:?}; only `herdr` is built"));
    }
    let only = ctx.args.pos.get(1).map(|s| parse_id(s)).transpose()?;
    let snap = ctx.snap();
    let mux = ctx.mux();
    let mut plan = Plan { summary: format!("layout herdr {}", only.as_ref().map(|i| i.0.as_str()).unwrap_or("(every fleet here)")), ..Default::default() };
    if mux.name() != "herdr" || !mux.caps().layout {
        plan.refusals.push(refusal("mux.none", format!("`layout herdr` needs Herdr (multiplexer here: {})", mux.name()), Some(nimitz_mux::missing_mux_fix(&ctx.vars)), None));
        return Ok(finish_plan(ctx, &plan));
    }
    if ctx.caller != Caller::User {
        plan.refusals.push(refusal("authority", "`nimitz layout` is yours alone".into(), None, None));
        return Ok(finish_plan(ctx, &plan));
    }
    // The layout itself is core's (pure data); this maps it onto the mux.
    let ships = ctx.args.flag("--ships") && mux.caps().open;
    let mut spec = nimitz_core::layout::plan(&ctx.env, &snap, only.as_ref().map(|i| i.fleet()), ships, &ctx.caller, ctx.force());
    if ctx.args.flag("--ships") && !mux.caps().open {
        spec.plan.warnings.push(Issue { id: None, ..warn_issue(&Id::admiral("-"), format!("{} cannot open panes; no ship tabs", mux.name())) });
    }
    let shown: Vec<String> = mux.panes().into_iter().filter_map(|p| p.session_id).collect();
    nimitz_core::layout::drop_shown(&mut spec, &shown);
    let lp = LayoutPlan {
        workspaces: spec
            .workspaces
            .iter()
            .map(|w| (w.label.clone(), w.tabs.iter().map(|t| (t.label.clone(), t.cmd.clone(), PathBuf::from(&t.cwd))).collect()))
            .collect(),
    };
    plan.steps = spec.plan.steps;
    plan.warnings.extend(spec.plan.warnings);
    plan.refusals.extend(spec.plan.refusals);
    match ctx.gate(&plan) {
        out::Go::Exit(code) => Ok(code),
        out::Go::Run => match mux.layout(&lp) {
            Ok(panes) => {
                out::ok(&format!("layout ready: {} pane(s); members bind at their SessionStart (`nimitz status`)", panes.len()));
                Ok(0)
            }
            Err(e) => {
                out::error(&format!("layout failed: {e}"));
                Ok(1)
            }
        },
    }
}

fn warn_issue(id: &Id, message: String) -> Issue {
    Issue { code: "layout.skip".into(), severity: Severity::Warn, message, fix: None, id: Some(id.clone()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_shape_checked() {
        for ok in ["MAIN", "MAIN/ARGOS", "MAIN/ARGOS/CLEAN", "MAIN/ARGOS/~fix-ci"] {
            assert!(parse_id(ok).is_ok(), "{ok}");
        }
        for bad in ["", "main", "MAIN/", "MAIN/ARGOS/CLEAN/X", "MAIN/ARGOS/~", "MAIN/as tra"] {
            assert!(parse_id(bad).is_err(), "{bad}");
        }
    }
}
