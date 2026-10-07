//! `nimitz restore [<F>] [--all] [--force] [--jobs-only] [--here] [--rename]`
//! (plan-v4 section 13): bring a fleet back from the Herdr shape. Core plans
//! it (`nimitz_core::restore::plan`); this runs the steps: a wake through
//! `sessions::apply`, a claim or an open through the mux, a pin. The same
//! engine, shape only, is `layout herdr` and the tab pass of `up`.
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
                foreground: p.foreground.clone(),
            })
            .collect(),
    };
    Ok((core, sh))
}

/// The panes that show a session, with where they are (labels and ids).
pub fn shown_in(sh: &nimitz_mux::Shape) -> Vec<active::Shown> {
    sh.panes
        .iter()
        .filter_map(|p| {
            let sid = p.session_id.clone()?;
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
        .collect()
}

/// Record the active set from `snap` and the mux (`active.json`). Never
/// fails a command; a set that did not change is not written.
pub fn record_active(ctx: &Ctx, snap: &Snapshot, mux: &dyn Mux, shape: Option<&nimitz_mux::Shape>) {
    let read;
    let shape = match shape {
        Some(s) => Some(s),
        None if mux.caps().open => {
            read = mux.shape(false).ok();
            read.as_ref()
        }
        None => None,
    };
    let shown = shape.map(shown_in);
    let prev = active::load(&ctx.env);
    let set = active::from_snapshot(snap, shown.as_deref(), active::herdr_identity(&ctx.vars), prev.as_ref());
    let _ = active::write(&ctx.env, set);
}

/// `nimitz restore [<F>] [--all] [--force] [--jobs-only] [--here] [--rename]`.
pub fn restore(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--all", "--jobs-only", "--here", "--rename"])?;
    ctx.args.at_most(1)?;
    let fleet = ctx.args.pos.first().map(|s| crate::members::parse_id(s)).transpose()?;
    if fleet.as_ref().is_some_and(|f| f.parts().len() != 1) {
        return Err("restore takes a fleet (F), not a member".into());
    }
    let opts = rs::RestoreOpts { all: ctx.args.flag("--all"), force: ctx.force(), jobs_only: ctx.args.flag("--jobs-only"), here: ctx.args.flag("--here"), rename: ctx.args.flag("--rename"), ..Default::default() };
    Ok(run_shape(ctx, fleet.as_ref().map(|f| f.fleet().to_string()).as_deref(), opts, false))
}

/// Plan, gate and run one restore (or a shape-only pass). `confirmed`: the
/// user said yes already (`up` runs its tab pass after its own gate).
pub fn run_shape(ctx: &Ctx, fleet: Option<&str>, opts: rs::RestoreOpts, confirmed: bool) -> i32 {
    let snap = ctx.snap();
    let mux = ctx.mux();
    let has_mux = mux.caps().open;
    let (shape, raw) = if has_mux && !opts.jobs_only {
        match shape_of(mux.as_ref(), true) {
            Ok((s, r)) => (s, Some(r)),
            Err(e) => {
                let verb = if opts.shape_only { "layout herdr" } else { "restore" };
                let plan = Plan { summary: format!("{verb} {}", fleet.unwrap_or("(every fleet here)")), refusals: vec![refusal("mux.failed", format!("cannot read the Herdr shape ({e}); nothing is typed into panes nimitz cannot see"), None, None)], ..Default::default() };
                return finish(ctx, &plan);
            }
        }
    } else {
        (rs::Shape::default(), None)
    };
    let act = active::load(&ctx.env);
    let herdr = active::herdr_identity(&ctx.vars);
    let mut rp = rs::plan(&ctx.env, &snap, &shape, act.as_ref(), herdr.as_ref(), has_mux, fleet, opts, &ctx.caller);
    for r in rp.plan.refusals.iter_mut().filter(|r| r.code == "mux.none") {
        r.fix = Some(format!("{}; or `nimitz restore --jobs-only` to wake the jobs without tabs", nimitz_mux::missing_mux_fix(&ctx.vars)));
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
            for w in &shown.warnings {
                out::issue(w);
            }
        }
        let code = report(&rp, &[], &[]);
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
    out::ran(&outcome);
    // what the re-read says now: Remote Control reconnected by a resume (R-31)
    let after = ctx.snap();
    let mut late: Vec<Issue> = vec![];
    for f in after.fleets.iter().filter(|f| fleet.map(|x| x == f.id.fleet()).unwrap_or(true)) {
        for i in nimitz_core::check::check(&ctx.env, &after, Some(&f.id)).into_iter().filter(|i| i.code == "rc.exposed") {
            out::issue(&i);
            late.push(i);
        }
    }
    let code = report(&rp, &failed, &outcome.issues);
    if out::json_mode() {
        out::json(&Outcome { ran: outcome.ran.clone(), verified: code == 0, issues: outcome.issues.iter().chain(late.iter()).cloned().collect() });
    }
    let raw_after = raw.as_ref().and_then(|_| shape_of(mux.as_ref(), false).ok()).map(|(_, r)| r);
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
    let errors = rp.plan.warnings.iter().chain(issues.iter()).filter(|i| i.severity == Severity::Error).count();
    for f in &rp.fleets {
        let failed_here = failed.iter().filter(|id| id.fleet() == f.fleet).count() + f.failed;
        let planned: usize = rp.steps.iter().filter(|s| s.id().fleet() == f.fleet).count();
        let parked = if f.parked > 0 { format!("; {} parked", f.parked) } else { String::new() };
        let stopped = if f.stopped > 0 { format!("; {} stopped on purpose", f.stopped) } else { String::new() };
        let not_started = if f.not_started > 0 { format!("; {} not started", f.not_started) } else { String::new() };
        if failed_here > 0 {
            out::error(&format!("{}: {failed_here} member(s) not restored (see the fixes above){parked}{stopped}", f.fleet));
        } else if planned == 0 {
            out::ok(&format!("{}: nothing to restore{}", f.fleet, trim_lead(&format!("{parked}{stopped}{not_started}"))));
        } else {
            out::ok(&format!("{}: restored {} lead(s), {} ship(s){parked}{stopped}{not_started}", f.fleet, f.leads, f.ships));
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
                        out::ok(&format!("{id}: {what}"));
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
                match mux.run_in(&pref, cmd) {
                    Ok(()) => {
                        out.ran.push((last, 0, format!("typed into pane {}", pane.pane_id)));
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
                // a wake that made a new job (its row was gone): the attach names it now
                let cmd: Vec<String> = if cmd.iter().any(|a| a == "<the new job>") {
                    match nimitz_core::sessions::find_row_by_session(env, session).and_then(|r| r.short_id) {
                        Some(j) => nimitz_core::claude::attach(env, &j),
                        None => {
                            fail(&mut out, &mut failed, &id, refusal("session.none", format!("{id}'s job is not there after its wake; no tab"), Some(format!("nimitz status {id}")), Some(id.clone())));
                            prog.done = n + 1;
                            let _ = rs::progress_write(env, &prog);
                            continue;
                        }
                    }
                } else {
                    cmd.clone()
                };
                let display = step.display().remove(0);
                let since = nimitz_core::evidence::now() - 0.001;
                match mux.open(&cmd, Path::new(cwd), Place::Tab { workspace: workspace.clone(), label: label.clone() }) {
                    Ok(p) => {
                        out.ran.push((display, 0, format!("tab {label} in workspace {workspace}, pane {}", p.pane_id)));
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
            rs::RestoreStep::Pin { job, .. } => {
                let display = step.display().remove(0);
                match nimitz_core::pins::pin(env, job) {
                    Ok(true) => out.ran.push((display, 0, "pinned".into())),
                    Ok(false) => out.ran.push((display, 0, "pinned already".into())),
                    Err(e) => {
                        out.ran.push((display, 1, e.to_string()));
                        out.issues.push(Issue { code: "pin.failed".into(), severity: Severity::Warn, message: format!("{id}'s job {job} could not be pinned ({e}); the daemon may retire it when idle"), fix: Some(format!("nimitz relaunch {id}  # pins it again")), id: Some(id.clone()) });
                    }
                }
            }
        }
        prog.done = n + 1;
        let _ = rs::progress_write(env, &prog);
    }
    if failed.is_empty() {
        rs::progress_clear(env);
    }
    out.verified = failed.is_empty();
    (out, failed)
}

/// An attach that wakes a parked job: the SessionStart that follows must be
/// a `resume` of the job's conversation (`bind_started`: a `startup` is
/// `history.lost`, a copy is `lineage.broken`); then a lead is pinned.
fn after_wake(ctx: &Ctx, out: &mut Outcome, failed: &mut Vec<Id>, id: &Id, cwd: &str, since: f64, session: &str) {
    let env = &ctx.env;
    match nimitz_core::sessions::bind_started(env, id, Some(cwd), since, Some(session), "resume", nimitz_core::sessions::BIND_TIMEOUT_S) {
        Ok(sid) => {
            out::ok(&format!("{id}: the attach woke it; SessionStart resume of {} verified", &sid[..sid.len().min(8)]));
            if id.role().lead() {
                if let Some(job) = nimitz_core::sessions::find_row_by_session(env, &sid).and_then(|r| r.short_id) {
                    match nimitz_core::pins::pin(env, &job) {
                        Ok(true) => out::ok(&format!("{id}: pinned {job}")),
                        Ok(false) => {}
                        Err(e) => out.issues.push(Issue { code: "pin.failed".into(), severity: Severity::Warn, message: format!("{id}'s job {job} could not be pinned ({e})"), fix: Some(format!("nimitz relaunch {id}  # pins it again")), id: Some(id.clone()) }),
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
