//! The restore engine's runner (plan-v4 section 13; docs/design/cli-ux.md):
//! `start`, `restart` and `open` (`restore::plan_start`), `layout` (the
//! shape only) and `muster` (`restore::plan`, the one command
//! after a Claude Code update, a Herdr closure or a reboot, which tells the
//! three apart per member). Core plans; this runs the steps: a wake or a
//! relaunch through `sessions::apply`, a claim, an open or a reattach
//! through the mux, a pin.
//!
//! Every run also records the active set (`active.json`), as `status`,
//! `open` and `layout` do (`record_active`).
use crate::run::refusal;
use crate::{out, Ctx};
use nimitz_core::active;
use nimitz_core::restore as rs;
use nimitz_core::{Id, Issue, Outcome, Plan, Severity, Snapshot};
use nimitz_mux::{Mux, PaneRef, Place};
use std::path::Path;

/// The mux's shape as core sees it (`foregrounds`: with what each pane runs).
pub fn shape_of(mux: &dyn Mux, foregrounds: bool) -> Result<(rs::Shape, nimitz_mux::Shape), nimitz_mux::MuxError> {
    let sh = mux.shape(foregrounds)?;
    let core = rs::Shape {
        workspaces: sh.workspaces.iter().map(|w| (w.id.clone(), w.label.clone())).collect(),
        tabs: sh.tabs.iter().map(|t| (t.id.clone(), t.workspace_id.clone(), t.label.clone())).collect(),
        panes: sh
            .panes
            .iter()
            .map(|p| rs::ShapePane {
                pane_id: p.pane.pane_id.clone(),
                tab_id: p.pane.tab_id.clone().unwrap_or_default(),
                workspace_id: p.pane.workspace_id.clone().unwrap_or_default(),
                cwd: p.cwd.clone(),
                session_id: p.session_id.clone(),
                claimed: p.claimed.clone(),
                inferred: p.inferred,
                foreground: p.foreground.clone(),
                foreground_pid: p.foreground_pid,
            })
            .collect(),
    };
    Ok((core, sh))
}

/// The panes that show a session, with where they are (labels and ids), and
/// the panes that show none. A pane whose foreground is `claude attach <job>`
/// (read with the shape) shows that job's conversation even before Herdr's
/// agent detection says so.
pub fn seen_in(env: &nimitz_core::env::Env, sh: &nimitz_mux::Shape) -> active::Seen {
    let attached = |p: &nimitz_mux::ShapePane| -> Option<String> {
        let fg = p.foreground.as_ref()?;
        (fg.len() == 3 && fg[1] == "attach").then(|| nimitz_core::sessions::find_row(env, &fg[2]).map(|r| r.session_id))?
    };
    let shown = sh
        .panes
        .iter()
        .filter_map(|p| {
            let sid = p.session_id.clone().or_else(|| attached(p))?;
            let tab_id = p.pane.tab_id.clone().unwrap_or_default();
            let ws_id = p.pane.workspace_id.clone().unwrap_or_default();
            Some(active::Shown {
                session_id: sid,
                place: active::Place {
                    workspace: sh.workspace(&ws_id).map(|w| w.label.clone()).unwrap_or_default(),
                    workspace_id: ws_id,
                    tab: sh.tab(&tab_id).map(|t| t.label.clone()).unwrap_or_default(),
                    tab_id,
                    pane_id: p.pane.pane_id.clone(),
                },
            })
        })
        .collect::<Vec<_>>();
    let empty_panes = sh.panes.iter().filter(|p| !shown.iter().any(|s| s.place.pane_id == p.pane.pane_id)).map(|p| p.pane.pane_id.clone()).collect();
    active::Seen { shown, empty_panes }
}

/// Record the active set from `snap` and the mux (`active.json`). Never
/// fails a command; a set that did not change is not written.
pub fn record_active(ctx: &Ctx, snap: &Snapshot, mux: &dyn Mux, shape: Option<&nimitz_mux::Shape>) {
    let read;
    let shape = match shape {
        Some(s) => Some(s),
        None if mux.caps().open => {
            // (with the foregrounds: an attach just typed is not yet an agent to Herdr)
            read = mux.shape(true).ok();
            read.as_ref()
        }
        None => None,
    };
    let seen = shape.map(|s| seen_in(&ctx.env, s));
    let prev = active::load(&ctx.env);
    let set = active::from_snapshot(snap, seen.as_ref(), active::herdr_identity(&ctx.vars), prev.as_ref());
    let _ = active::write(&ctx.env, set);
}

/// `nimitz muster [<path>...] [--parked] [--force] [--here] [--rename]`
/// (no path: `//...`): the one command after a reboot, a Claude Code update
/// or a Herdr closure (docs/design/cli-ux.md section 3), one plan: workers
/// on an older Claude Code relaunched (decision 1; `restart //...` relaunches
/// every one), down leads and the active set's down ships woken (`--parked`:
/// the rest too), every member's pane reattached (its own, else its lead
/// tab). With no multiplexer, the job steps alone, and the plan says so.
pub fn muster(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--parked", "--here", "--rename"])?;
    let paths = if ctx.args.pos.is_empty() { vec![nimitz_core::paths::Path::Under(None)] } else { crate::members::parse_paths(&ctx.args.pos)? };
    let jobs_only = !ctx.mux().caps().open;
    let opts = rs::RestoreOpts { parked: ctx.args.flag("--parked"), force: ctx.force(), jobs_only, here: ctx.args.flag("--here"), rename: ctx.args.flag("--rename"), ..Default::default() };
    Ok(run_shape(ctx, &paths, opts))
}

/// Plan, gate and run one restore (or a shape-only pass).
pub fn run_shape(ctx: &Ctx, paths: &[nimitz_core::paths::Path], opts: rs::RestoreOpts) -> i32 {
    run_engine(ctx, paths, false, opts)
}

/// `nimitz start|restart|open <path>...`: the same engine, planned by `restore::plan_start`.
pub fn run_start(ctx: &Ctx, paths: &[nimitz_core::paths::Path], opts: rs::RestoreOpts) -> i32 {
    run_engine(ctx, paths, true, opts)
}

/// Plan (`restore::plan`, or `plan_start` for `start`), gate and run.
fn run_engine(ctx: &Ctx, paths: &[nimitz_core::paths::Path], start: bool, opts: rs::RestoreOpts) -> i32 {
    let snap = ctx.snap();
    // the fleets the paths reach, for the checks after and the run's record
    let fleets: Vec<String> = snap.fleets.iter().filter(|f| f.members().any(|m| nimitz_core::paths::covered(paths, &m.id))).map(|f| f.id.0.clone()).collect();
    let mine = |f: &nimitz_core::Fleet| fleets.contains(&f.id.0);
    let named = paths.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" ");
    let mux = ctx.mux();
    let has_mux = mux.caps().open;
    let (shape, raw) = if has_mux && (!opts.jobs_only || opts.keep_panes) {
        match shape_of(mux.as_ref(), true) {
            Ok((s, r)) => (s, Some(r)),
            Err(e) => {
                let verb = if start { "start" } else if opts.shape_only { "layout" } else { "muster" };
                // a server taken from the record (outside Herdr) says which, and what works
                let fix = active::recorded_server_fix(&ctx.env, &ctx.vars).map(|f| format!("{f}; or nimitz start {named}  # the jobs, no panes"));
                let plan = Plan { summary: format!("{verb} {named}"), refusals: vec![refusal("mux.failed", format!("cannot read the Herdr shape ({e}); nothing is typed into panes nimitz cannot see"), fix, None)], ..Default::default() };
                return finish(ctx, &plan);
            }
        }
    } else {
        (rs::Shape::default(), None)
    };
    let act = active::load(&ctx.env);
    let herdr = active::herdr_identity(&ctx.vars);
    let mut rp = if start {
        rs::plan_start(&ctx.env, &snap, &shape, act.as_ref(), herdr.as_ref(), has_mux, paths, opts, &ctx.caller)
    } else {
        rs::plan(&ctx.env, &snap, &shape, act.as_ref(), herdr.as_ref(), has_mux, paths, opts, &ctx.caller)
    };
    for r in rp.plan.refusals.iter_mut().filter(|r| r.code == "mux.none" && !opts.pane) {
        r.fix = Some(format!("{}; or `nimitz start {named}` to wake the jobs without tabs", nimitz_mux::missing_mux_fix(&ctx.vars)));
    }
    if !rp.plan.ok() {
        return finish(ctx, &rp.plan);
    }
    if rp.steps.is_empty() {
        if ctx.plan_only() && out::json_mode() {
            out::json(&rp.plan);
        } else {
            out::plan(&rp.plan);
        }
        for f in snap.fleets.iter().filter(|f| mine(f)) {
            for i in nimitz_core::check::check_tree(&ctx.env, &snap, Some(&f.id)).into_iter().filter(|i| i.code == "rc.exposed") {
                out::issue(&i);
            }
        }
        let code = report(&rp, &[], &[]);
        if out::json_mode() && !ctx.plan_only() {
            out::json(&Outcome { ran: vec![], verified: code == 0, issues: rp.plan.warnings.iter().filter(|i| i.severity == Severity::Error).cloned().collect() });
        }
        if let Some(r) = raw.as_ref() {
            record_active(ctx, &snap, mux.as_ref(), Some(r));
        }
        return code.max(self_last(ctx, &rp, true));
    }
    // panes alone change no session (an `open` of live members): no confirmation
    let panes_only = opts.pane && rp.steps.iter().all(|s| !matches!(s, rs::RestoreStep::Wake { .. } | rs::RestoreStep::Relaunch { .. }) && !matches!(s, rs::RestoreStep::Claim { wakes: true, .. } | rs::RestoreStep::Open { wakes: true, .. }));
    match out::gate(&rp.plan, ctx.plan_only(), ctx.yes() || panes_only) {
        out::Go::Exit(code) => return code.max(if ctx.plan_only() { self_last(ctx, &rp, false) } else { 0 }),
        out::Go::Run => {}
    }
    let (outcome, failed) = execute(ctx, mux.as_ref(), &rp, &fleets);
    crate::view::run(&rp.plan, &outcome, &failed);
    // what the re-read says now: Remote Control reconnected by a resume (R-31)
    let after = ctx.snap();
    let mut late: Vec<Issue> = vec![];
    for f in after.fleets.iter().filter(|f| mine(f)) {
        for i in nimitz_core::check::check_tree(&ctx.env, &after, Some(&f.id)).into_iter().filter(|i| i.code == "rc.exposed") {
            out::issue(&i);
            late.push(i);
        }
    }
    let code = report(&rp, &failed, &outcome.issues);
    if out::json_mode() {
        out::json(&Outcome { ran: outcome.ran.clone(), verified: code == 0, issues: outcome.issues.iter().chain(late.iter()).cloned().collect() });
    }
    let raw_after = raw.as_ref().and_then(|_| shape_of(mux.as_ref(), true).ok()).map(|(_, r)| r);
    record_active(ctx, &after, mux.as_ref(), raw_after.as_ref());
    code.max(self_last(ctx, &rp, false))
}

/// The caller's own relaunch, last of all: left to the detached helper once
/// the rest ran and the report is printed (`self_end`; said under `--plan`).
/// `ungated`: no gate ran (a plan whose only step is that one), so it needs
/// its own `--yes`.
fn self_last(ctx: &Ctx, rp: &rs::RestorePlan, ungated: bool) -> i32 {
    let Some(id) = &rp.self_end else { return 0 };
    if ungated && !ctx.plan_only() && !ctx.yes() {
        out::line(&format!("[INFO] {id}'s own relaunch runs with --yes"));
        return 0;
    }
    crate::self_end::defer(ctx, id, &["restart".to_string(), id.0.clone()])
}

/// Print a refused (or empty) plan and return its exit code.
fn finish(ctx: &Ctx, plan: &Plan) -> i32 {
    match ctx.gate(plan) {
        out::Go::Exit(code) => code,
        out::Go::Run => 0,
    }
}

/// The verdict per fleet (restore-scenarios.md decision 10) and the exit code.
fn report(rp: &rs::RestorePlan, failed: &[Id], issues: &[Issue]) -> i32 {
    // only what could not be restored is an error: the planner's failures
    // and the executor's (a config finding that rides along is not)
    let errors = rp.fleets.iter().map(|f| f.failed).sum::<usize>() + issues.iter().filter(|i| i.severity == Severity::Error).count();
    let start = ["start ", "restart ", "open "].iter().any(|v| rp.plan.summary.starts_with(v));
    let (none, done) = if start { ("nothing to start", "up") } else { ("nothing to bring back", "back") };
    for f in &rp.fleets {
        let failed_here = failed.iter().filter(|id| id.fleet() == f.fleet).count() + f.failed;
        let planned: usize = rp.steps.iter().filter(|s| s.id().fleet() == f.fleet).count();
        let parked = if f.parked > 0 { format!("; {} parked", f.parked) } else { String::new() };
        let stopped = if f.stopped > 0 { format!("; {} stopped on purpose", f.stopped) } else { String::new() };
        let not_started = if f.not_started > 0 { format!("; {} not started", f.not_started) } else { String::new() };
        let held = if f.held > 0 { format!("; {} held in a pane (see the fix above)", f.held) } else { String::new() };
        if failed_here > 0 {
            crate::view::verdict(false, &format!("{}: {failed_here} member(s) not {done} (see the fixes above){parked}{stopped}{held}", f.fleet));
        } else if planned == 0 {
            crate::view::verdict(true, &format!("{}: {none}{}", f.fleet, trim_lead(&format!("{parked}{stopped}{not_started}{held}"))));
        } else if start {
            crate::view::verdict(true, &format!("{}: {} lead(s), {} ship(s) up{parked}{stopped}{not_started}{held}", f.fleet, f.leads, f.ships));
        } else {
            crate::view::verdict(true, &format!("{}: {} lead(s), {} ship(s) back{parked}{stopped}{not_started}{held}", f.fleet, f.leads, f.ships));
        }
    }
    if errors > 0 || !failed.is_empty() {
        1
    } else {
        0
    }
}

fn trim_lead(s: &str) -> String {
    let t = s.trim_start_matches("; ");
    if t.is_empty() {
        String::new()
    } else {
        format!(" ({t})")
    }
}

/// Run the steps in order, recording progress in `restore.json`. A member
/// whose step failed gets no later step (its tab is not made for a job that
/// did not come back). Returns what ran and the members that failed.
fn execute(ctx: &Ctx, mux: &dyn Mux, rp: &rs::RestorePlan, fleets: &[String]) -> (Outcome, Vec<Id>) {
    let env = &ctx.env;
    let mut out = Outcome { verified: true, ..Default::default() };
    let mut failed: Vec<Id> = vec![];
    let mut prog = rs::Progress {
        run: rs::run_id(),
        started: nimitz_core::evidence::now(),
        fleets: fleets.to_vec(),
        total: rp.steps.len(),
        done: 0,
        steps: rp.steps.iter().map(|s| s.display().iter().map(|d| d.cmd.join(" ")).collect::<Vec<_>>().join(" && ")).collect(),
    };
    let _ = rs::progress_write(env, &prog);
    let fail = |out: &mut Outcome, failed: &mut Vec<Id>, id: &Id, i: Issue| {
        out.issues.push(i);
        if !failed.contains(id) {
            failed.push(id.clone());
        }
    };
    // the pane each member's last step made or typed into (`Focus { pane: None }`)
    let mut made: std::collections::HashMap<Id, PaneRef> = std::collections::HashMap::new();
    let mut installed: Option<Option<String>> = None;
    for (n, step) in rp.steps.iter().enumerate() {
        let id = step.id().clone();
        if failed.contains(&id) {
            for d in step.display() {
                out.ran.push((d, 0, "skipped: an earlier step of this member failed".into()));
            }
            prog.done = n + 1;
            let _ = rs::progress_write(env, &prog);
            continue;
        }
        match step {
            rs::RestoreStep::Relaunch { steps, .. } => {
                let plan = Plan { summary: format!("restore {id}"), steps: steps.clone(), ..Default::default() };
                let o = nimitz_core::sessions::apply(env, &plan);
                out.ran.extend(o.ran);
                if !o.verified || o.issues.iter().any(|i| i.severity == Severity::Error) {
                    let issues = if o.issues.is_empty() { vec![refusal("step.failed", format!("{id}'s relaunch was not verified"), Some(format!("nimitz status {id}")), Some(id.clone()))] } else { o.issues };
                    for i in issues {
                        fail(&mut out, &mut failed, &id, i);
                    }
                } else {
                    out.issues.extend(o.issues);
                    // the new worker should run the installed Claude Code; one
                    // that does not would be relaunched by every restore
                    let now = installed.get_or_insert_with(|| nimitz_core::claude::version(env)).clone();
                    let resumed = steps.last().and_then(|s| s.cmd.windows(2).find(|w| w[0] == "--resume").map(|w| w[1].clone()));
                    let pid = resumed.and_then(|sid| nimitz_core::sessions::find_row_by_session(env, &sid)).and_then(|r| r.pid);
                    if let (Some(v), Some(now)) = (pid.and_then(|p| nimitz_core::compat::process_version("", Some(p))), now) {
                        if nimitz_core::compat::older(&v, &now) {
                            out.issues.push(Issue { code: "relaunch.old".into(), severity: Severity::Warn, message: format!("{id} was relaunched but its worker still runs Claude Code {v}, not the installed {now}"), fix: Some("claude daemon status  # shows the daemon's version; report it (never `claude daemon stop`: it ends every background session)".into()), id: Some(id.clone()) });
                        }
                    }
                }
            }
            rs::RestoreStep::Wake { steps, .. } => {
                let plan = Plan { summary: format!("restore {id}"), steps: steps.clone(), ..Default::default() };
                let o = nimitz_core::sessions::apply(env, &plan);
                out.ran.extend(o.ran);
                if !o.verified || o.issues.iter().any(|i| i.severity == Severity::Error) {
                    let issues = if o.issues.is_empty() { vec![refusal("step.failed", format!("{id}'s wake was not verified"), Some(format!("nimitz status {id}")), Some(id.clone()))] } else { o.issues };
                    for i in issues {
                        fail(&mut out, &mut failed, &id, i);
                    }
                } else {
                    out.issues.extend(o.issues);
                }
            }
            rs::RestoreStep::Claim { pane, rename, cmd, cwd, session, wakes, .. } => {
                let mut wakes = *wakes;
                if wakes {
                    // the crash window (plan-v4 section 5): a job the daemon is
                    // restarting is not woken twice; the attach then only attaches
                    for (_, what) in nimitz_core::sessions::wait_crash_window(env, &[(id.clone(), session.clone())]) {
                        if out::verbose() {
                            out::ok(&format!("{id}: {what}"));
                        }
                        wakes = false;
                    }
                }
                let display = step.display();
                if let Some((tab, label)) = rename {
                    match mux.rename_tab(tab, label) {
                        Ok(()) => out.ran.push((display[0].clone(), 0, String::new())),
                        Err(e) => {
                            out.ran.push((display[0].clone(), 1, e.to_string()));
                            fail(&mut out, &mut failed, &id, refusal("mux.failed", format!("renaming tab {tab} to {label} failed: {e}"), None, Some(id.clone())));
                            prog.done = n + 1;
                            let _ = rs::progress_write(env, &prog);
                            continue;
                        }
                    }
                }
                let pref = PaneRef { pane_id: pane.pane_id.clone(), tab_id: Some(pane.tab_id.clone()).filter(|x| !x.is_empty()), workspace_id: Some(pane.workspace_id.clone()).filter(|x| !x.is_empty()) };
                let since = nimitz_core::evidence::now() - 0.001;
                let last = display.last().cloned().expect("a claim displays its pane run");
                let Some(cmd) = name_the_job(env, &id, cmd, session) else {
                    fail(&mut out, &mut failed, &id, refusal("session.none", format!("{id}'s job is not there after its wake; no attach"), Some(format!("nimitz status {id}")), Some(id.clone())));
                    prog.done = n + 1;
                    let _ = rs::progress_write(env, &prog);
                    continue;
                };
                let last = resolved(last, &cmd);
                match mux.run_in(&pref, &cmd) {
                    Ok(()) => {
                        out.ran.push((last, 0, format!("typed into pane {}", pane.pane_id)));
                        made.insert(id.clone(), pref.clone());
                        if wakes {
                            after_wake(ctx, &mut out, &mut failed, &id, cwd, since, session);
                        }
                    }
                    Err(e) => {
                        out.ran.push((last, 1, e.to_string()));
                        fail(&mut out, &mut failed, &id, refusal("mux.failed", format!("attaching {id} in pane {} failed: {e}", pane.pane_id), None, Some(id.clone())));
                    }
                }
            }
            rs::RestoreStep::Open { workspace, label, cmd, cwd, session, wakes, .. } => {
                let Some(cmd) = name_the_job(env, &id, cmd, session) else {
                    fail(&mut out, &mut failed, &id, refusal("session.none", format!("{id}'s job is not there after its wake; no tab"), Some(format!("nimitz status {id}")), Some(id.clone())));
                    prog.done = n + 1;
                    let _ = rs::progress_write(env, &prog);
                    continue;
                };
                let display = resolved(step.display().remove(0), &cmd);
                let since = nimitz_core::evidence::now() - 0.001;
                match mux.open(&cmd, Path::new(cwd), Place::Tab { workspace: workspace.clone(), label: label.clone() }) {
                    Ok(p) => {
                        out.ran.push((display, 0, format!("tab {label} in workspace {workspace}, pane {}", p.pane_id)));
                        made.insert(id.clone(), p.clone());
                        if *wakes {
                            after_wake(ctx, &mut out, &mut failed, &id, cwd, since, session);
                        }
                    }
                    Err(e) => {
                        out.ran.push((display, 1, e.to_string()));
                        fail(&mut out, &mut failed, &id, refusal("mux.failed", format!("opening tab {label} for {id} failed: {e}"), None, Some(id.clone())));
                    }
                }
            }
            rs::RestoreStep::Reattach { pane, cmd, cwd, session, .. } => {
                let display = step.display().remove(0);
                let pref = PaneRef { pane_id: pane.pane_id.clone(), tab_id: Some(pane.tab_id.clone()).filter(|x| !x.is_empty()), workspace_id: Some(pane.workspace_id.clone()).filter(|x| !x.is_empty()) };
                // the relaunch ended the old attach: its pane drops to the shell
                // within a second or so; nothing is typed until it has
                if !wait_for_shell(mux, &pref) {
                    out.ran.push((display, 1, "the old attach did not end".into()));
                    fail(&mut out, &mut failed, &id, refusal("tab.taken", format!("{id} is relaunched, but pane {} still runs something other than a shell; nothing is typed into it", pane.pane_id), Some(format!("ctrl-z in pane {}, then nimitz start {}/...", pane.pane_id, id.fleet())), Some(id.clone())));
                    prog.done = n + 1;
                    let _ = rs::progress_write(env, &prog);
                    continue;
                }
                let Some(cmd) = name_the_job(env, &id, cmd, session) else {
                    fail(&mut out, &mut failed, &id, refusal("session.none", format!("{id}'s job is not there after its relaunch; no attach"), Some(format!("nimitz status {id}")), Some(id.clone())));
                    prog.done = n + 1;
                    let _ = rs::progress_write(env, &prog);
                    continue;
                };
                let _ = cwd;
                let display = resolved(display, &cmd);
                match mux.run_in(&pref, &cmd) {
                    Ok(()) => {
                        out.ran.push((display, 0, format!("typed into pane {}", pane.pane_id)));
                        made.insert(id.clone(), pref.clone());
                    }
                    Err(e) => {
                        out.ran.push((display, 1, e.to_string()));
                        fail(&mut out, &mut failed, &id, refusal("mux.failed", format!("attaching {id} in pane {} failed: {e}", pane.pane_id), None, Some(id.clone())));
                    }
                }
            }
            rs::RestoreStep::Split { next_to, cmd, cwd, session, .. } => {
                let Some(cmd) = name_the_job(env, &id, cmd, session) else {
                    fail(&mut out, &mut failed, &id, refusal("session.none", format!("{id}'s job is not there after its start; no pane"), Some(format!("nimitz status {id}")), Some(id.clone())));
                    prog.done = n + 1;
                    let _ = rs::progress_write(env, &prog);
                    continue;
                };
                let display = resolved(step.display().remove(0), &cmd);
                let beside = PaneRef { pane_id: next_to.pane_id.clone(), tab_id: Some(next_to.tab_id.clone()).filter(|x| !x.is_empty()), workspace_id: Some(next_to.workspace_id.clone()).filter(|x| !x.is_empty()) };
                match mux.open(&cmd, Path::new(cwd), Place::Split { next_to: beside }) {
                    Ok(p) => {
                        out.ran.push((display, 0, format!("pane {} beside pane {}", p.pane_id, next_to.pane_id)));
                        made.insert(id.clone(), p);
                    }
                    Err(e) => {
                        out.ran.push((display, 1, e.to_string()));
                        fail(&mut out, &mut failed, &id, refusal("mux.failed", format!("splitting pane {} for {id} failed: {e}", next_to.pane_id), None, Some(id.clone())));
                    }
                }
            }
            rs::RestoreStep::Focus { pane, .. } => {
                let display = step.display().remove(0);
                let target = pane
                    .as_ref()
                    .map(|p| PaneRef { pane_id: p.pane_id.clone(), tab_id: Some(p.tab_id.clone()).filter(|x| !x.is_empty()), workspace_id: Some(p.workspace_id.clone()).filter(|x| !x.is_empty()) })
                    .or_else(|| made.get(&id).cloned());
                match target {
                    Some(p) => match mux.focus(&p) {
                        Ok(()) => {
                            let mut d = display;
                            if let Some(last) = d.cmd.last_mut() {
                                *last = p.pane_id.clone();
                            }
                            out.ran.push((d, 0, format!("focused pane {}", p.pane_id)));
                        }
                        Err(e) => {
                            out.ran.push((display, 1, format!("focus failed: {e}")));
                            fail(&mut out, &mut failed, &id, refusal("mux.failed", format!("focusing {id}'s pane failed: {e}"), None, Some(id.clone())));
                        }
                    },
                    None => {
                        out.ran.push((display, 1, "no pane to focus".into()));
                        fail(&mut out, &mut failed, &id, refusal("mux.failed", format!("{id} has no pane to focus"), None, Some(id.clone())));
                    }
                }
            }
            rs::RestoreStep::Rename { tab_id, label, .. } => {
                let display = step.display().remove(0);
                match mux.rename_tab(tab_id, label) {
                    Ok(()) => out.ran.push((display, 0, format!("tab {tab_id} is {label} now"))),
                    Err(e) => {
                        out.ran.push((display, 1, e.to_string()));
                        fail(&mut out, &mut failed, &id, refusal("mux.failed", format!("renaming tab {tab_id} to {label} failed: {e}"), None, Some(id.clone())));
                    }
                }
            }
            rs::RestoreStep::Pin { job, .. } => {
                let display = step.display().remove(0);
                match nimitz_core::pins::pin(env, job) {
                    Ok(true) => out.ran.push((display, 0, "pinned".into())),
                    Ok(false) => out.ran.push((display, 0, "pinned already".into())),
                    Err(e) => {
                        out.ran.push((display, 1, e.to_string()));
                        out.issues.push(Issue { code: "pin.failed".into(), severity: Severity::Warn, message: format!("{id}'s job {job} could not be pinned ({e}); the daemon may retire it when idle"), fix: Some(format!("{}  # pins it again", nimitz_core::sessions::restart_cmd(&id))), id: Some(id.clone()) });
                    }
                }
            }
        }
        prog.done = n + 1;
        let _ = rs::progress_write(env, &prog);
    }
    // the run reached the end of its list: not interrupted, whatever failed
    rs::progress_clear(env);
    if !failed.is_empty() {
        out::line(&format!("[INFO] restore run {}: finished, {} member(s) failed (see the fixes above)", prog.run, failed.len()));
    }
    out.verified = failed.is_empty();
    (out, failed)
}

/// A wake that made a new job (its row was gone, a relaunch, a first or a
/// fresh launch) leaves the attach naming `NEW_JOB`: the row running the
/// conversation now names it (`session`; empty: the one the member's identity
/// records). None when no row runs it (the wake did not land).
fn name_the_job(env: &nimitz_core::env::Env, id: &Id, cmd: &[String], session: &str) -> Option<Vec<String>> {
    if !cmd.iter().any(|a| a == rs::NEW_JOB) {
        return Some(cmd.to_vec());
    }
    // a first launch or a fresh one: the session its identity records now
    let session = if session.is_empty() { nimitz_core::registry::load(env).identities.remove(&id.0)?.session? } else { session.to_string() };
    let job = nimitz_core::sessions::find_row_by_session(env, &session)?.short_id?;
    Some(nimitz_core::claude::attach(env, &job))
}

/// A step as it ran: the attach's `NEW_JOB` replaced by the job it named.
fn resolved(mut step: nimitz_core::Step, cmd: &[String]) -> nimitz_core::Step {
    if let Some(job) = cmd.last() {
        for a in step.cmd.iter_mut().filter(|a| *a == rs::NEW_JOB) {
            *a = job.clone();
        }
    }
    step
}

/// Wait (up to `NIMITZ_REATTACH_WAIT` seconds, 15 by default) for `pane`'s
/// foreground to be a shell at its prompt.
fn wait_for_shell(mux: &dyn Mux, pane: &PaneRef) -> bool {
    let limit = std::env::var("NIMITZ_REATTACH_WAIT").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(15.0);
    let until = std::time::Instant::now() + std::time::Duration::from_secs_f64(limit);
    loop {
        if mux.foreground(pane).is_ok_and(|fg| nimitz_mux::is_shell(&fg)) {
            return true;
        }
        if std::time::Instant::now() >= until {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

/// An attach that wakes a parked job: the SessionStart that follows must be
/// a `resume` of the job's conversation (`bind_started`: a `startup` is
/// `history.lost`, a copy is `lineage.broken`); then a lead is pinned.
fn after_wake(ctx: &Ctx, out: &mut Outcome, failed: &mut Vec<Id>, id: &Id, cwd: &str, since: f64, session: &str) {
    let env = &ctx.env;
    match nimitz_core::sessions::bind_started(env, id, Some(cwd), since, Some(session), "resume", nimitz_core::sessions::BIND_TIMEOUT_S) {
        Ok(sid) => {
            if out::verbose() {
                out::ok(&format!("{id}: the attach woke it; SessionStart resume of {} verified", &sid[..sid.len().min(8)]));
            }
            if id.role().always_on() {
                if let Some(job) = nimitz_core::sessions::find_row_by_session(env, &sid).and_then(|r| r.short_id) {
                    match nimitz_core::pins::pin(env, &job) {
                        Ok(true) if out::verbose() => out::ok(&format!("{id}: pinned {job}")),
                        Ok(true) => {}
                        Ok(false) => {}
                        Err(e) => out.issues.push(Issue { code: "pin.failed".into(), severity: Severity::Warn, message: format!("{id}'s job {job} could not be pinned ({e})"), fix: Some(format!("{}  # pins it again", nimitz_core::sessions::restart_cmd(&id))), id: Some(id.clone()) }),
                    }
                }
            }
        }
        Err(issues) => {
            for i in issues {
                out.issues.push(i);
            }
            if !failed.contains(id) {
                failed.push(id.clone());
            }
        }
    }
}
