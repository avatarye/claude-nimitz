//! Commands on running members (docs/design/cli-ux.md section 2): status,
//! check; session control (start, restart, stop, rm, sweep, adopt, signal:
//! jobs, never panes); pane control (open, attach, close, layout). The plans
//! come from core.
use crate::run::{self, refusal};
use crate::{out, Ctx};
use nimitz_core::sessions as ses;
use nimitz_core::{Caller, Id, Issue, Member, Plan, Severity, Snapshot, Status, Step};

/// Parse and shape-check an identity id (contracts.md section 4).
pub fn parse_id(s: &str) -> Result<Id, String> {
    let id = Id(s.to_string());
    if id.well_formed() {
        Ok(id)
    } else {
        Err(format!("{s:?} is not an id (F, F/SQ or F/SQ/SHIP; names are [A-Z0-9_]+)"))
    }
}

/// Parse paths (`nimitz_core::paths`): at least one.
pub fn parse_paths(args: &[String]) -> Result<Vec<nimitz_core::paths::Path>, String> {
    nimitz_core::paths::parse_all(args)
}

pub fn members(snap: &Snapshot) -> Vec<&Member> {
    snap.fleets.iter().flat_map(|f| f.members()).collect()
}

pub fn member<'a>(snap: &'a Snapshot, id: &Id) -> Option<&'a Member> {
    members(snap).into_iter().find(|m| &m.id == id)
}

fn members_mut(snap: &mut Snapshot) -> Vec<&mut Member> {
    snap.fleets.iter_mut().flat_map(|f| f.members_mut()).collect()
}

fn live(m: &Member) -> bool {
    m.session.as_ref().map(|s| s.pid.is_some()).unwrap_or(false)
}

/// The status word (decision 3 of docs/design/cli-ux.md): `down` for a member
/// the active set holds (taken down while active: a `...` start wakes it),
/// `parked` for one the daemon retired or that was not active.
fn status_word(s: Status, in_set: bool) -> &'static str {
    match s {
        Status::NotStarted => "not-started",
        Status::Stopped => "stopped",
        Status::Parked if in_set => "down",
        Status::Parked => "parked",
        Status::Idle => "idle",
        Status::Busy => "busy",
        Status::NeedsYou => "needs-you",
        Status::Unknown => "unknown",
    }
}

// ------------------------------------------------------------------ status, check

/// `nimitz status [<path>...]`: the fleets, their members and issues (the
/// paths: only the members they cover, and their fleets).
pub fn status(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    let all = ctx.args.pos.is_empty();
    let paths = if all { vec![] } else { parse_paths(&ctx.args.pos)? };
    let mut snap = ctx.snap();
    if !all {
        if let Err(i) = nimitz_core::paths::resolve(&snap, &paths) {
            out::issue(&i);
            return Ok(1);
        }
    }
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
    let shown = |id: &Id| all || nimitz_core::paths::covered(&paths, id);
    if !all {
        // (a fleet keeps its admiral, a squadron its commodore: the shape's own;
        // ships and squadrons the paths do not reach go)
        snap.fleets.retain(|f| f.members().any(|m| shown(&m.id)));
        for f in snap.fleets.iter_mut() {
            f.squadrons.retain(|sq| shown(&sq.commodore.id) || sq.ships.iter().any(|m| shown(&m.id)));
            for sq in f.squadrons.iter_mut() {
                sq.ships.retain(|m| shown(&m.id));
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
        .flat_map(|f| f.members())
        .map(|m| if matches!(m.role, nimitz_core::Role::Ship) { 4 } else { 2 } + m.id.0.len())
        .max()
        .unwrap_or(0)
        .max(26);
    let set = nimitz_core::active::load(&ctx.env).map(|a| a.members.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
    let line = |m: &Member, depth: usize| {
        if shown(&m.id) {
            member_line(&ctx.env, m, depth, col, set.contains(&m.id.0));
        }
    };
    for f in &snap.fleets {
        let here = f.machine.as_deref().map(|m| format!("runs here as {m}")).unwrap_or("does not run on this machine".into());
        out::line(&format!("{}  ({here})", f.id));
        for i in &f.issues {
            out::issue(i);
        }
        line(&f.admiral, 1);
        for sq in &f.squadrons {
            line(&sq.commodore, 1);
            if shown(&sq.commodore.id) || sq.ships.iter().any(|m| shown(&m.id)) {
                for i in &sq.issues {
                    out::issue(i);
                }
            }
            for m in &sq.ships {
                line(m, 2);
            }
        }
    }
    if !all {
        return Ok(0);
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

fn member_line(env: &nimitz_core::env::Env, m: &Member, depth: usize, col: usize, in_set: bool) {
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
    out::line(&format!("{pad}{:<width$} {:<11} {:<9} {why}{pane}{pinned}", m.id.0, status_word(m.status, in_set), sess));
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

// ------------------------------------------------------------------ session control: jobs, never panes

/// The engine's options every session-control verb shares: the jobs only
/// (`start`, `restart` make no pane; a relaunch reattaches the pane it cut,
/// continuity, never a new one), `--parked`, `--force`.
fn job_opts(ctx: &Ctx) -> nimitz_core::restore::RestoreOpts {
    nimitz_core::restore::RestoreOpts { parked: ctx.args.flag("--parked"), force: ctx.force(), jobs_only: true, keep_panes: true, ..Default::default() }
}

/// `nimitz start <path>... [--fresh] [--parked] [--force]`: bring members up
/// (`restore::plan_start`), the jobs only: a member that is down is woken by
/// its exact id, top-down; one a path names exactly gets a first launch when
/// it never ran; a worker on an older Claude Code is relaunched. `--fresh`:
/// a named member's new conversation; `--parked`: the parked and stopped
/// ships a `...` covers too; `--force`: a cold launch over a broken lineage
/// or an untested Claude Code. It makes no pane (`open`, `layout`).
pub fn start(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--fresh", "--parked"])?;
    let paths = parse_paths(&ctx.args.pos)?;
    let opts = nimitz_core::restore::RestoreOpts { fresh: ctx.args.flag("--fresh"), ..job_opts(ctx) };
    Ok(crate::restore::run_start(ctx, &paths, opts))
}

/// `nimitz restart <path>... [--parked] [--force]`: relaunch the live
/// members the paths cover (same id and history, every flag again), and
/// wake the ones that are down; a pane whose attach the relaunch ends gets
/// the new job's attach (continuity, never a new pane).
pub fn restart(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--parked"])?;
    let paths = parse_paths(&ctx.args.pos)?;
    let opts = nimitz_core::restore::RestoreOpts { restart: true, ..job_opts(ctx) };
    Ok(crate::restore::run_start(ctx, &paths, opts))
}

/// A target of `stop` or `rm`: the members a path covers, or one session by
/// its job or session id (a word no member path names that a row has); an
/// exact path neither names is `member.unknown`, with what is there.
enum Target {
    Members(nimitz_core::paths::Path, Vec<Id>),
    Session(String),
}

fn targets(ctx: &Ctx, snap: &Snapshot) -> Result<Vec<Target>, String> {
    if ctx.args.pos.is_empty() {
        return Err(format!("missing path or session ({})", nimitz_core::paths::FORMS));
    }
    let is_row = |a: &str| ses::find_row(&ctx.env, a).is_some() || ses::find_row_by_session(&ctx.env, a).is_some();
    Ok(ctx
        .args
        .pos
        .iter()
        .map(|a| match nimitz_core::paths::Path::parse(a) {
            Ok(p) => match nimitz_core::paths::resolve(snap, std::slice::from_ref(&p)) {
                Ok(ids) => Target::Members(p, ids),
                Err(_) if p.member().is_some() && is_row(a) => Target::Session(a.clone()),
                Err(_) => Target::Members(p, vec![]),
            },
            Err(_) => Target::Session(a.clone()),
        })
        .collect())
}

/// One plan of several: steps, refusals and warnings in order, each once.
fn merged(summary: String, plans: Vec<Plan>) -> Plan {
    let mut out = Plan { summary, ..Default::default() };
    for p in plans {
        for st in p.steps {
            if !out.steps.contains(&st) {
                out.steps.push(st);
            }
        }
        for i in p.refusals {
            if !out.refusals.contains(&i) {
                out.refusals.push(i);
            }
        }
        for i in p.warnings {
            if !out.warnings.contains(&i) {
                out.warnings.push(i);
            }
        }
    }
    out
}

/// `nimitz stop <path|session>...`: stop members (a path; under a `...` the
/// live ones), ships first across every path, or one session by its id;
/// start's opposite. The caller's own stop (by its path or its own session's
/// id) runs last, detached, once its turn ends (`self_end`).
pub fn stop(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    let snap = ctx.snap();
    let me = crate::self_end::me(ctx, &snap);
    let mine = me.as_ref().and_then(|id| member(&snap, id)).and_then(|m| m.session.clone());
    let own_session = |s: &str| mine.as_ref().is_some_and(|r| r.session_id == s || r.short_id.as_deref() == Some(s));
    let self_note = |id: &Id| Plan { warnings: vec![Issue { code: "self.end".into(), severity: Severity::Info, message: format!("{id} runs this stop: its own stop runs after it, once its turn ends"), fix: None, id: Some(id.clone()) }], ..Default::default() };
    let (mut plans, mut ids, mut myself) = (vec![], vec![], None);
    for t in targets(ctx, &snap)? {
        match t {
            Target::Members(p, found) if found.is_empty() => plans.push(Plan { refusals: vec![nimitz_core::paths::unknown(&snap, &p)], ..Default::default() }),
            Target::Members(p, found) => {
                for id in found.into_iter().filter(|id| p.member().is_some() || member(&snap, id).is_some_and(live)) {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
            Target::Session(s) if own_session(&s) => myself = Some(s),
            Target::Session(s) => plans.push(ses::plan_stop_session(&ctx.env, &snap, &s, &ctx.caller)),
        }
    }
    bottom_up(&mut ids);
    let mut own = myself.clone();
    for id in ids {
        if me.as_ref() == Some(&id) {
            let p = ses::plan_stop(&ctx.env, &snap, &id, &ctx.caller);
            if p.ok() {
                own = Some(id.0.clone());
                plans.push(self_note(&id));
            } else {
                plans.push(p);
            }
        } else {
            plans.push(ses::plan_stop(&ctx.env, &snap, &id, &ctx.caller));
        }
    }
    if let (Some(_), Some(id)) = (&myself, &me) {
        plans.push(self_note(id));
    }
    let plan = merged(format!("stop {}", ctx.args.pos.join(" ")), plans);
    let code = run::session_plan(ctx, plan.clone());
    // its own stop, last: as a restart does, whatever the other steps came to,
    // once the plan ran (`--yes`); a refused plan runs nothing, its own stop neither
    match (me, own) {
        (Some(id), Some(what)) if plan.ok() && (ctx.plan_only() || ctx.yes()) => Ok(code.max(crate::self_end::defer(ctx, &id, &["stop".to_string(), what]))),
        (Some(id), Some(_)) if plan.ok() => {
            out::line(&format!("[INFO] {id}'s own stop runs with --yes"));
            Ok(code)
        }
        (Some(id), Some(_)) => {
            out::line(&format!("[INFO] {id}'s own stop is not left to a helper: the plan is refused"));
            Ok(code)
        }
        _ => Ok(code),
    }
}

/// Ships first, then commodores, then admirals (a stop takes a fleet down
/// from the bottom).
fn bottom_up(ids: &mut [Id]) {
    ids.sort_by_key(|i| std::cmp::Reverse(i.parts().len()));
}

/// `nimitz rm <path|session>...`: remove job rows (never transcripts): a
/// member's stale rows, or one row of a stowaway or a copy by its id.
pub fn rm(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    let snap = ctx.snap();
    let mut plans = vec![];
    for t in targets(ctx, &snap)? {
        match t {
            Target::Members(p, ids) if ids.is_empty() => plans.push(Plan { refusals: vec![nimitz_core::paths::unknown(&snap, &p)], ..Default::default() }),
            Target::Members(_, ids) => plans.extend(ids.iter().map(|id| ses::plan_rm(&ctx.env, &snap, &id.0, &ctx.caller))),
            Target::Session(s) => plans.push(ses::plan_rm(&ctx.env, &snap, &s, &ctx.caller)),
        }
    }
    Ok(run::session_plan(ctx, merged(format!("rm {}", ctx.args.pos.join(" ")), plans)))
}

/// `nimitz sweep [<path>...]`: drop stale rows and copies of the members the
/// paths cover (none: a member's own scope, else everything).
pub fn sweep(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    let snap = ctx.snap();
    let ids = if ctx.args.pos.is_empty() {
        // a caller with partial labels: its own member by its process, never everything
        match (&ctx.caller, ses::verified(&ctx.caller)) {
            (Caller::Member { .. }, None) => match ses::me_of(&snap, &ctx.caller) {
                Some(me) => Some(nimitz_core::paths::resolve(&snap, &[nimitz_core::paths::Path::Under(Some(me))]).unwrap_or_default()),
                None => return Err("name what to sweep (a path): this caller's labels are partial, and no member runs it".into()),
            },
            _ => None,
        }
    } else {
        let paths = parse_paths(&ctx.args.pos)?;
        match nimitz_core::paths::resolve(&snap, &paths) {
            Ok(ids) => Some(ids),
            Err(i) => return Ok(finish_plan(ctx, &Plan { summary: format!("sweep {}", ctx.args.pos.join(" ")), refusals: vec![i], ..Default::default() })),
        }
    };
    Ok(run::session_plan(ctx, ses::plan_sweep(&ctx.env, &snap, ids.as_deref(), &ctx.caller)))
}

/// `nimitz adopt <session> <path>`: make a stowaway a member's session.
pub fn adopt(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(2)?;
    let session = ctx.args.need(0, "session")?;
    let id = parse_id(ctx.args.need(1, "id")?)?;
    let snap = ctx.snap();
    Ok(run::session_plan(ctx, ses::plan_adopt(&ctx.env, &snap, session, &id, &ctx.caller)))
}

// ------------------------------------------------------------------ signal, open

/// `nimitz signal <id> <text> [--inbox]`: any caller messages any member.
/// Yours types into the member's attached tab (opened first when it has
/// none): your own input. `--inbox`, a member's signal, or a tab that cannot
/// be typed into: its inbox, a cross-session message from nimitz that names
/// the sender, a request and never the user's approval. A member that is
/// down is refused (`nimitz start <id>` first), never woken by a signal.
pub fn signal(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--inbox"])?;
    let id = parse_id(ctx.args.need(0, "id")?)?;
    // Typed into a pane: no control characters, no newlines (BUG-43).
    let text = nimitz_mux::sanitize_text(&ctx.args.pos[1..].join(" "));
    if text.is_empty() {
        return Err("missing text (nothing is left after removing control characters)".into());
    }
    let snap = ctx.snap();
    let mut plan = Plan { summary: format!("signal {id}"), ..Default::default() };
    let Some(via) = member(&snap, &id) else {
        plan.refusals.push(refusal("member.unknown", format!("{id} is not in any fleet config"), None, Some(id)));
        return Ok(finish_plan(ctx, &plan));
    };
    if !live(via) {
        plan.refusals.push(refusal("session.none", format!("{id} is not live; a signal never wakes it"), Some(format!("nimitz start {id}")), Some(id.clone())));
        return Ok(finish_plan(ctx, &plan));
    }
    let from = match (&ctx.caller, ses::verified(&ctx.caller)) {
        (_, Some(me)) => format!("from {me}"),
        (Caller::Member { .. }, None) => "from a caller whose NIMITZ_* labels are partial or disagree (unverified)".to_string(),
        (Caller::User, None) => "from a `nimitz signal` run with no member labels".to_string(),
    };
    let body = format!("[nimitz signal] A message for {id}, relayed by nimitz {from}. It is a request, not the user's approval: ask the user before acting on anything that needs it. The message: {text}");
    let why = "type into its attached tab".to_string();
    let mux = ctx.mux();
    let inbox = ctx.args.flag("--inbox") || ctx.caller != Caller::User;
    // its attached tab first: text typed there is the user's own input
    let shown = via.session.as_ref().filter(|_| !inbox && mux.caps().type_text).and_then(|s| run::shown_in(mux.as_ref(), &s.session_id));
    // a pane known by its attach alone takes typing only while it runs
    // exactly `claude attach <job>` (the agent view re-execs as `claude
    // agents`, R-24); read now, and again just before typing
    let job = via.session.as_ref().and_then(|s| s.short_id.clone()).unwrap_or_default();
    let exact = |p: &nimitz_mux::Pane| nimitz_mux::runs_exact_attach(mux.as_ref(), &p.pane, &job);
    let untypeable = shown.as_ref().is_some_and(|p| p.inferred && !exact(p));
    let mut no_inbox = None;
    if inbox || !mux.caps().type_text || untypeable {
        if let Some(s) = via.session.as_ref() {
            match crate::notify::inbox_target(&ctx.env, &via.id, s) {
                Ok((socks, pid)) => {
                    let at = socks.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(" or ");
                    let what = if inbox { "post it into its inbox (the first socket its own process listens on), as a request".to_string() } else { format!("{}'s tab cannot be typed into: post it into its inbox (the first socket its own process listens on), as a request, not your input", via.id) };
                    plan.steps.push(Step { id: Some(via.id.clone()), cmd: vec!["inbox".into(), at, body.clone()], cwd: None, why: what });
                    return match ctx.gate(&plan) {
                        out::Go::Exit(code) => Ok(code),
                        out::Go::Run => match nimitz_core::inbox::send(&ctx.env, &socks, pid, &body) {
                            Ok(sock) => {
                                out::ok(&format!("posted to {}'s inbox {} as a request from nimitz{}", via.id, sock.display(), if ctx.caller == Caller::User { " (not as your own input)" } else { "" }));
                                Ok(0)
                            }
                            Err(e) => {
                                out::error(&format!("posting to {}'s inbox failed: {e}; nothing was typed. Run the signal again in a moment", via.id));
                                Ok(1)
                            }
                        },
                    };
                }
                Err(w) => no_inbox = Some(w),
            }
        }
    }
    let inbox_why = no_inbox.map(|w| format!("; its inbox cannot be used: {w}")).unwrap_or_default();
    if inbox {
        plan.refusals.push(refusal("inbox.none", format!("{} has no inbox to post to{inbox_why}", via.id), None, Some(via.id.clone())));
        return Ok(finish_plan(ctx, &plan));
    }
    if !mux.caps().type_text {
        plan.refusals.push(refusal(
            "mux.none",
            format!("typing to {} needs a multiplexer that can type ({}){inbox_why}", via.id, mux.name()),
            Some(nimitz_mux::missing_mux_fix(&ctx.vars)),
            Some(via.id.clone()),
        ));
        return Ok(finish_plan(ctx, &plan));
    }
    // with no tab, one is opened first (plan-v4 section 7)
    if let Some(p) = shown.as_ref().filter(|_| untypeable) {
        let mut i = run::untypeable(&via.id, &p.pane);
        i.message.push_str(&inbox_why);
        plan.refusals.push(i);
        return Ok(finish_plan(ctx, &plan));
    }
    let recheck = shown.as_ref().is_some_and(|p| p.inferred);
    let pane = shown.map(|p| p.pane);
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
                            // the new tab is known by its attach alone, and a fresh
                            // attach may not be connected yet (a failed one drops to
                            // the shell, which would run the text): its inbox
                            Some(p) if p.inferred => {
                                let posted = via.session.as_ref().ok_or_else(|| "no session".to_string()).and_then(|s| crate::notify::inbox_target(&ctx.env, &via.id, s)).and_then(|(socks, pid)| nimitz_core::inbox::send(&ctx.env, &socks, pid, &body));
                                return Ok(match posted {
                                    Ok(sock) => {
                                        out::ok(&format!("posted to {}'s inbox {} as a request from nimitz (not as your own input): its new tab cannot be typed into", via.id, sock.display()));
                                        0
                                    }
                                    Err(w) => {
                                        let mut i = run::untypeable(&via.id, &p.pane);
                                        i.message.push_str(&format!("; its inbox cannot be used: {w}"));
                                        out::issue(&i);
                                        1
                                    }
                                });
                            }
                            Some(p) => p.pane,
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
            let check = if recheck { nimitz_mux::attach_check(mux.as_ref(), &pane, &job) } else { nimitz_mux::AttachCheck::Exact };
            if check != nimitz_mux::AttachCheck::Exact {
                out::error(&format!("{}: {}; nothing was typed", via.id, check.refusal(&pane.pane_id, &job)));
                return Ok(1);
            }
            match mux.type_text(&pane, &text) {
                Ok(()) if recheck && nimitz_mux::typed_into_view(mux.as_ref(), &pane) => {
                    out::warn(&format!("typed to {} in pane {}, but the pane runs the agent view now (an /exit raced the typing): check pane {} for a dispatched session", via.id, pane.pane_id, pane.pane_id));
                    Ok(1)
                }
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

// ------------------------------------------------------------------ pane control: open, close, layout

/// `nimitz open <id>... [--tab] [--force]`: a pane for each member, focused
/// (its pane, its tab, a split beside its commodore, a new tab; `--tab`:
/// always its own tab, reused or created; `--force`: a second tab beside a
/// taken one). A member that is down is woken first, by start's own plan.
pub fn open(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--tab"])?;
    let paths = parse_paths(&ctx.args.pos)?;
    let opts = nimitz_core::restore::RestoreOpts { force: ctx.force(), pane: true, tab: ctx.args.flag("--tab"), ..Default::default() };
    Ok(crate::restore::run_start(ctx, &paths, opts))
}

/// `nimitz close <path>...`: close the panes that show those members, each
/// only while its foreground is the member's exact `claude attach <job>`
/// (never a pane that runs anything else); the jobs run on.
pub fn close(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    let paths = parse_paths(&ctx.args.pos)?;
    let snap = ctx.snap();
    let mux = ctx.mux();
    let mut plan = Plan { summary: format!("close {}", ctx.args.pos.join(" ")), ..Default::default() };
    let ids = match nimitz_core::paths::resolve(&snap, &paths) {
        Ok(ids) => ids,
        Err(i) => {
            plan.refusals.push(i);
            return Ok(finish_plan(ctx, &plan));
        }
    };
    if !mux.caps().open {
        plan.refusals.push(refusal("mux.none", format!("closing a pane needs a multiplexer ({})", mux.name()), Some(nimitz_mux::missing_mux_fix(&ctx.vars)), None));
        return Ok(finish_plan(ctx, &plan));
    }
    let panes = mux.panes();
    let mut closing: Vec<nimitz_mux::PaneRef> = vec![];
    for id in &ids {
        let Some(m) = member(&snap, id) else { continue };
        let Some(s) = m.session.as_ref() else { continue };
        let job = s.short_id.clone().unwrap_or_default();
        for p in panes.iter().filter(|p| p.session_id.as_deref() == Some(s.session_id.as_str())) {
            // (its attach on any Claude Code: nothing is typed, the pane only closes)
            if !job.is_empty() && nimitz_mux::attach_check(mux.as_ref(), &p.pane, &job) != nimitz_mux::AttachCheck::Other {
                plan.steps.push(Step { id: Some(id.clone()), cmd: vec!["herdr".into(), "pane".into(), "close".into(), p.pane.pane_id.clone()], cwd: None, why: format!("it shows {id} (`claude attach {job}`); the job runs on") });
                closing.push(p.pane.clone());
            } else {
                plan.warnings.push(Issue { code: "tab.taken".into(), severity: Severity::Warn, message: format!("pane {} shows {id} but does not run its attach; it is left open", p.pane.pane_id), fix: None, id: Some(id.clone()) });
            }
        }
    }
    if plan.steps.is_empty() && plan.warnings.is_empty() {
        plan.warnings.push(Issue { code: "pane.none".into(), severity: Severity::Info, message: "no pane shows them; nothing to close".into(), fix: None, id: None });
    }
    // closing a pane changes no session: no confirmation needed
    match out::gate(&plan, ctx.plan_only(), true) {
        out::Go::Exit(c) => Ok(c),
        out::Go::Run => {
            let mut code = 0;
            for p in &closing {
                match mux.close_pane(p) {
                    Ok(()) => out::ok(&format!("closed pane {}", p.pane_id)),
                    Err(e) => {
                        out::error(&format!("closing pane {} failed: {e}", p.pane_id));
                        code = 1;
                    }
                }
            }
            crate::restore::record_active(ctx, &ctx.snap(), mux.as_ref(), None);
            Ok(code)
        }
    }
}

/// `nimitz layout <path>... [--ships] [--here] [--rename] [--force]`: the
/// Herdr shape only (plan-v4 section 13): a tab attaching each lead the
/// paths cover (a parked lead with a shell tab wakes by the attach; one with
/// no tab needs `nimitz start` first), `--ships` a tab for each live ship;
/// `--here` rebuilds it in this Herdr session, `--rename` gives a tab that
/// shows a member under another name its full name, `--force` opens a
/// second tab beside a taken one.
pub fn layout(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--ships", "--here", "--rename"])?;
    let paths = parse_paths(&ctx.args.pos)?;
    let opts = nimitz_core::restore::RestoreOpts { shape_only: true, ships: ctx.args.flag("--ships"), force: ctx.force(), here: ctx.args.flag("--here"), rename: ctx.args.flag("--rename"), ..Default::default() };
    Ok(crate::restore::run_shape(ctx, &paths, opts))
}

// ------------------------------------------------------------------ attach

/// `nimitz attach <id>`: attach to a member in this terminal. Its exact job
/// from the snapshot (`claude attach` matches a name by part, so `DEV` could
/// be `NIMITZ_DEV` or `MYCC_DEV`: always the job's short id). A member that
/// is down (parked, stopped) is woken first by start's own plan, gated like
/// any other (`--plan`, `--yes`); a live one attaches at once. Then this
/// process becomes `claude attach <job>` (exec; Windows: it waits and exits
/// with its code). Never from inside a Claude Code session (an attach in an
/// attach).
pub fn attach(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(1)?;
    let id = parse_id(ctx.args.need(0, "id")?)?;
    if ctx.force() {
        return Err("--force does nothing for attach (a wake goes by start's plan: nimitz start <id> first)".into());
    }
    let mut plan = Plan { summary: format!("attach {id}"), ..Default::default() };
    // a live Claude Code process above this one (its session file, checked
    // to be that process); else CLAUDECODE alone, which a terminal or a Herdr
    // server started from a Claude Code tool inherits: said so, with the way out
    let nested = match nimitz_core::sessions::session_above_id(&ctx.env, &ctx.vars) {
        Some(sid) => Some((format!("this runs inside a Claude Code session (session {} is above this process)", &sid[..sid.len().min(8)]), format!("nimitz attach {id}  # from a plain terminal; or nimitz open {id}"))),
        None if ctx.vars.get("CLAUDECODE").is_some_and(|v| !v.is_empty()) => {
            let unset = if cfg!(windows) { format!("$env:CLAUDECODE = $null; nimitz attach {id}  # PowerShell") } else { format!("env -u CLAUDECODE nimitz attach {id}") };
            Some(("CLAUDECODE is set, so this may run inside a Claude Code session, though none is above this process (a terminal started from one inherits it)".to_string(), format!("{unset}  # when this terminal is not inside Claude Code")))
        }
        None => None,
    };
    if let Some((why, fix)) = nested {
        plan.refusals.push(refusal("attach.nested", format!("{why}; an attach inside it nests one Claude Code in another"), Some(fix), Some(id)));
        return Ok(finish_plan(ctx, &plan));
    }
    let snap = ctx.snap();
    let Some(m) = member(&snap, &id) else {
        plan.refusals.push(refusal("member.unknown", format!("{id} is not in any fleet config"), None, Some(id)));
        return Ok(finish_plan(ctx, &plan));
    };
    let snap = if live(m) {
        snap
    } else {
        // down: start's own plan wakes it (by its exact id; jobs only, no tab),
        // with its gate; then the attach
        let opts = nimitz_core::restore::RestoreOpts { jobs_only: true, ..Default::default() };
        let code = crate::restore::run_start(ctx, &[nimitz_core::paths::Path::Member(id.clone())], opts);
        if ctx.plan_only() {
            if code == 0 {
                out::line(&format!("then: attach to {id} in this terminal (`claude attach <its job>`)"));
            }
            return Ok(code);
        }
        if code != 0 {
            return Ok(code);
        }
        ctx.snap()
    };
    let m = member(&snap, &id).filter(|m| live(m));
    let Some(job) = m.and_then(|m| m.session.as_ref()).and_then(|s| s.short_id.clone()) else {
        let what = if m.is_some() { "runs in a process with no job; nothing to attach to" } else { "is not live" };
        plan.refusals.push(refusal("session.none", format!("{id} {what}"), Some(format!("nimitz status {id}")), Some(id)));
        return Ok(finish_plan(ctx, &plan));
    };
    let argv = nimitz_core::claude::attach(&ctx.env, &job);
    if ctx.plan_only() {
        plan.steps.push(Step { id: Some(id.clone()), cmd: argv, cwd: None, why: format!("attach to job {job} in this terminal; ctrl-z detaches") });
        return Ok(finish_plan(ctx, &plan));
    }
    out::line(&format!("attaching to {id} (job {job}); ctrl-z detaches, \u{2190} opens the agent view"));
    Ok(exec_attach(ctx, &argv))
}

/// Become `argv` (`claude attach <job>`): exec on Unix; elsewhere spawn, wait
/// and exit with its code. `NIMITZ_ATTACH_DRY=1` prints the argv instead and
/// exits 0: for the tests only (they never hand the terminal to a claude); a
/// release build never reads it.
fn exec_attach(ctx: &Ctx, argv: &[String]) -> i32 {
    use std::io::Write;
    if cfg!(debug_assertions) && ctx.vars.get("NIMITZ_ATTACH_DRY").is_some_and(|v| v == "1") {
        out::line(&format!("exec {}", argv.join(" ")));
        return 0;
    }
    let _ = std::io::stdout().flush();
    let mut c = std::process::Command::new(&argv[0]);
    c.args(&argv[1..]);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let e = c.exec();
        out::error(&format!("cannot run {}: {e}", argv.join(" ")));
        1
    }
    // ctrl-c and ctrl-break reach every process on the console: nimitz
    // swallows them while it waits, so the attach (which handles them) keeps
    // the console. A handler function, set before the spawn: a child never
    // inherits one (only the ignore flag of `SetConsoleCtrlHandler(None, 1)`)
    #[cfg(windows)]
    {
        unsafe extern "system" fn swallow(kind: u32) -> windows_sys::core::BOOL {
            use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT};
            (kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT) as windows_sys::core::BOOL
        }
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(swallow), 1);
        }
    }
    #[cfg(not(unix))]
    {
        let mut child = match c.spawn() {
            Ok(ch) => ch,
            Err(e) => {
                out::error(&format!("cannot run {}: {e}", argv.join(" ")));
                return 1;
            }
        };
        match child.wait() {
            Ok(st) => st.code().unwrap_or(1),
            Err(e) => {
                out::error(&format!("cannot wait for {}: {e}", argv.join(" ")));
                1
            }
        }
    }
}

fn attached_pane(a: &nimitz_mux::Attached) -> nimitz_mux::PaneRef {
    match a {
        nimitz_mux::Attached::Focused(p) | nimitz_mux::Attached::Claimed(p) | nimitz_mux::Attached::Opened(p) => p.clone(),
    }
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
