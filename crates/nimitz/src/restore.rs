//! `nimitz restore [<F>] [--all] [--restart] [--force] [--jobs-only] [--here]
//! [--rename]` (plan-v4 section 13): the one command after a Claude Code
//! update, a Herdr closure or crash, or a reboot. Core plans it
//! (`nimitz_core::restore::plan`, which tells the three apart per member);
//! this runs the steps: a wake or a relaunch through `sessions::apply`, a
//! claim, an open or a reattach through the mux, a pin. `up` and `layout
//! herdr` are deprecated for one release and point here.
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

/// `nimitz restore [<F>] [--all] [--restart] [--force] [--jobs-only] [--here] [--rename]`.
pub fn restore(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--all", "--restart", "--jobs-only", "--here", "--rename"])?;
    ctx.args.at_most(1)?;
    let f = ctx.args.pos.first().cloned().unwrap_or_default();
    crate::members::deprecated(&format!("restore{}", if f.is_empty() { String::new() } else { format!(" {f}") }), &format!("start{}{}", if f.is_empty() { String::new() } else { format!(" {f}") }, if ctx.args.flag("--restart") { " -f" } else { "" }));
    let fleet = ctx.args.pos.first().map(|s| crate::members::parse_id(s)).transpose()?;
    if fleet.as_ref().is_some_and(|f| f.parts().len() != 1) {
        return Err("restore takes a fleet (F), not a member".into());
    }
    let opts = rs::RestoreOpts { all: ctx.args.flag("--all"), force: ctx.force(), jobs_only: ctx.args.flag("--jobs-only"), here: ctx.args.flag("--here"), rename: ctx.args.flag("--rename"), restart: ctx.args.flag("--restart"), ..Default::default() };
    Ok(run_shape(ctx, fleet.as_ref().map(|f| f.fleet().to_string()).as_deref(), opts, false))
}

/// Plan, gate and run one restore (or a shape-only pass). `confirmed`: the
/// user said yes already (`up` runs its tab pass after its own gate).
pub fn run_shape(ctx: &Ctx, fleet: Option<&str>, opts: rs::RestoreOpts, confirmed: bool) -> i32 {
    run_engine(ctx, fleet, None, opts, confirmed)
}

/// `nimitz start [<target>]`: the same engine, planned by `restore::plan_start`.
pub fn run_start(ctx: &Ctx, target: Option<&Id>, opts: rs::RestoreOpts) -> i32 {
    run_engine(ctx, target.map(|t| t.fleet()), Some(target), opts, false)
}

/// Plan (`restore::plan`, or `plan_start` when `start` names its target),
/// gate and run.
fn run_engine(ctx: &Ctx, fleet: Option<&str>, start: Option<Option<&Id>>, opts: rs::RestoreOpts, confirmed: bool) -> i32 {
    let snap = ctx.snap();
    let mux = ctx.mux();
    let has_mux = mux.caps().open;
    let (shape, raw) = if has_mux && !opts.jobs_only {
        match shape_of(mux.as_ref(), true) {
            Ok((s, r)) => (s, Some(r)),
            Err(e) => {
                let verb = if start.is_some() { "start" } else if opts.shape_only { "layout herdr" } else { "restore" };
                // a server taken from the record (outside Herdr) says which, and what works
                let fix = active::recorded_server_fix(&ctx.env, &ctx.vars).map(|f| format!("{f}; or nimitz start{} --jobs-only", fleet.map(|f| format!(" {f}")).unwrap_or_default()));
                let plan = Plan { summary: format!("{verb} {}", fleet.unwrap_or("(every fleet here)")), refusals: vec![refusal("mux.failed", format!("cannot read the Herdr shape ({e}); nothing is typed into panes nimitz cannot see"), fix, None)], ..Default::default() };
                return finish(ctx, &plan);
            }
        }
    } else {
        (rs::Shape::default(), None)
    };
    let act = active::load(&ctx.env);
    let herdr = active::herdr_identity(&ctx.vars);
    let mut rp = match start {
        Some(target) => rs::plan_start(&ctx.env, &snap, &shape, act.as_ref(), herdr.as_ref(), has_mux, target, opts, &ctx.caller),
        None => rs::plan(&ctx.env, &snap, &shape, act.as_ref(), herdr.as_ref(), has_mux, fleet, opts, &ctx.caller),
    };
    for r in rp.plan.refusals.iter_mut().filter(|r| r.code == "mux.none" && !opts.pane) {
        r.fix = Some(format!("{}; or `nimitz start --jobs-only` to wake the jobs without tabs", nimitz_mux::missing_mux_fix(&ctx.vars)));
    }
    if !rp.plan.ok() {
        return finish(ctx, &rp.plan);
    }
    if rp.steps.is_empty() {
        if ctx.plan_only() && out::json_mode() {
            out::json(&rp.plan);
        } else {
            let mut shown = rp.plan.clone();
            crate::run::fixes_for(&ctx.env, &ctx.caller, &mut shown);
            out::plan(&shown);
        }
        for f in snap.fleets.iter().filter(|f| fleet.map(|x| x == f.id.fleet()).unwrap_or(true)) {
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
        return code;
    }
    if !confirmed {
        match ctx.gate(&rp.plan) {
            out::Go::Exit(code) => return code,
            out::Go::Run => {}
        }
    } else {
        out::plan(&rp.plan);
    }
    let (outcome, failed) = execute(ctx, mux.as_ref(), &rp, fleet);
    let mut shown = outcome.clone();
    let mut fixed = Plan { warnings: std::mem::take(&mut shown.issues), ..Default::default() };
    crate::run::fixes_for(&ctx.env, &ctx.caller, &mut fixed);
    shown.issues = fixed.warnings;
    crate::view::run(&rp.plan, &shown, &failed);
    // what the re-read says now: Remote Control reconnected by a resume (R-31)
    let after = ctx.snap();
    let mut late: Vec<Issue> = vec![];
    for f in after.fleets.iter().filter(|f| fleet.map(|x| x == f.id.fleet()).unwrap_or(true)) {
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
    code
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
    let start = rp.plan.summary.starts_with("start ");
    let (none, done) = if start { ("nothing to start", "up") } else { ("nothing to restore", "restored") };
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
            crate::view::verdict(true, &format!("{}: restored {} lead(s), {} ship(s){parked}{stopped}{not_started}{held}", f.fleet, f.leads, f.ships));
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
fn execute(ctx: &Ctx, mux: &dyn Mux, rp: &rs::RestorePlan, fleet: Option<&str>) -> (Outcome, Vec<Id>) {
    let env = &ctx.env;
    let mut out = Outcome { verified: true, ..Default::default() };
    let mut failed: Vec<Id> = vec![];
    let mut prog = rs::Progress {
        run: rs::run_id(),
        started: nimitz_core::evidence::now(),
        fleets: fleet.map(|f| vec![f.to_string()]).unwrap_or_else(|| rp.fleets.iter().map(|f| f.fleet.clone()).collect()),
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
    // members whose relaunch was skipped at run time (busy by then): their
    // later steps are skipped too, with no failure
    let mut held_back: Vec<Id> = vec![];
    // the pane each member's last step made or typed into (`Focus { pane: None }`)
    let mut made: std::collections::HashMap<Id, PaneRef> = std::collections::HashMap::new();
    let mut installed: Option<Option<String>> = None;
    for (n, step) in rp.steps.iter().enumerate() {
        let id = step.id().clone();
        if failed.contains(&id) || held_back.contains(&id) {
            let why = if held_back.contains(&id) { "skipped: its relaunch was held back (busy)" } else { "skipped: an earlier step of this member failed" };
            for d in step.display() {
                out.ran.push((d, 0, why.into()));
            }
            prog.done = n + 1;
            let _ = rs::progress_write(env, &prog);
            continue;
        }
        match step {
            rs::RestoreStep::Relaunch { steps, job, .. } => {
                // the plan may be minutes old: a member that turned busy or
                // waits on you is not cut off now
                let status = nimitz_core::sessions::find_row(env, job).filter(|r| r.live()).and_then(|r| r.status);
                if matches!(status.as_deref(), Some("busy" | "waiting" | "shell")) {
                    held_back.push(id.clone());
                    for d in step.display() {
                        out.ran.push((d, 0, format!("skipped: {id} is {} now", status.as_deref().unwrap_or_default())));
                    }
                    out.issues.push(Issue { code: "relaunch.busy".into(), severity: Severity::Info, message: format!("{id} turned busy after the plan: not relaunched now"), fix: Some(format!("{}  # once it is idle", nimitz_core::sessions::start_cmd(&id, "-f"))), id: Some(id.clone()) });
                    prog.done = n + 1;
                    let _ = rs::progress_write(env, &prog);
                    continue;
                }
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
                    fail(&mut out, &mut failed, &id, refusal("tab.taken", format!("{id} is relaunched, but pane {} still runs something other than a shell; nothing is typed into it", pane.pane_id), Some(format!("ctrl-z in pane {}, then nimitz start {}", pane.pane_id, id.fleet())), Some(id.clone())));
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
                        out.issues.push(Issue { code: "pin.failed".into(), severity: Severity::Warn, message: format!("{id}'s job {job} could not be pinned ({e}); the daemon may retire it when idle"), fix: Some(format!("{}  # pins it again", nimitz_core::sessions::start_cmd(&id, "-f"))), id: Some(id.clone()) });
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
/// `history.lost`, a copy is `lineage.broken`); then a lead or an aide is pinned.
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
                        Err(e) => out.issues.push(Issue { code: "pin.failed".into(), severity: Severity::Warn, message: format!("{id}'s job {job} could not be pinned ({e})"), fix: Some(format!("{}  # pins it again", nimitz_core::sessions::start_cmd(&id, "-f"))), id: Some(id.clone()) }),
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
