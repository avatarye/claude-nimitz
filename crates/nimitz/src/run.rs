//! Running a session plan: background steps go to `sessions::apply`; steps for
//! interactive members (admiral, commodore) open in a multiplexer pane, in plan
//! order, so the chain still comes up top-down.
use crate::{out, Ctx};
use nimitz_core::sessions::is_pane_step;
use nimitz_core::{Id, Issue, Outcome, Plan, Role, Severity, Step};
use nimitz_mux::{Mux, PaneRef, Place};
use std::path::Path;

/// Herdr workspace for a member: `F` for the admiral, `F.SQ` for the rest.
pub fn workspace_of(id: &Id) -> String {
    nimitz_core::layout::workspace_of(id)
}

/// Tab label: ADMIRAL, COMMODORE, the ship's name, or `~task`.
pub fn tab_of(id: &Id) -> String {
    nimitz_core::layout::tab_of(id)
}

pub fn place_for(id: &Id) -> Place {
    Place::Tab { workspace: workspace_of(id), label: tab_of(id) }
}

/// The pane that shows `session_id`, if the mux knows one.
pub fn pane_of(mux: &dyn Mux, session_id: &str) -> Option<PaneRef> {
    mux.panes().into_iter().find(|p| p.session_id.as_deref() == Some(session_id)).map(|p| p.pane)
}

pub fn refusal(code: &str, message: String, fix: Option<String>, id: Option<Id>) -> Issue {
    Issue { code: code.into(), severity: Severity::Error, message, fix, id }
}

/// A member caller sees only fixes it may run itself; any other fix becomes
/// "ask the user to run: ..." (BUG-51). The user sees every fix as is.
pub fn fixes_for(caller: &nimitz_core::Caller, plan: &mut Plan) {
    let nimitz_core::Caller::Member { role, .. } = caller else { return };
    let mine = |f: &str| {
        let read_only = f.starts_with("nimitz status") || f.starts_with("nimitz check");
        let commodore = *role == Role::Commodore && ["nimitz start ", "nimitz tender ", "nimitz sweep "].iter().any(|p| f.starts_with(p));
        read_only || commodore
    };
    for i in plan.refusals.iter_mut().chain(plan.warnings.iter_mut()) {
        if let Some(f) = i.fix.take() {
            i.fix = Some(if mine(&f) { f } else { format!("ask the user to run: {f}") });
        }
    }
}

/// A plan with pane steps needs a multiplexer that can open panes.
pub fn require_panes(plan: &mut Plan, mux: &dyn Mux, vars: &std::collections::HashMap<String, String>) {
    if plan.ok() && plan.steps.iter().any(is_pane_step) && !mux.caps().open {
        let who: Vec<String> = plan.steps.iter().filter(|s| is_pane_step(s)).filter_map(|s| s.id.as_ref().map(|i| i.0.clone())).collect();
        plan.refusals.push(refusal(
            "mux.none",
            format!("{} must open in a terminal pane, and no multiplexer is here ({})", who.join(", "), mux.name()),
            Some(nimitz_mux::missing_mux_fix(vars)),
            None,
        ));
    }
}

/// Gate, then run. Returns the exit code.
pub fn session_plan(ctx: &Ctx, mut plan: Plan) -> i32 {
    let mux = ctx.mux();
    require_panes(&mut plan, mux.as_ref(), &ctx.vars);
    match ctx.gate(&plan) {
        out::Go::Exit(code) => code,
        out::Go::Run => out::outcome(&execute(ctx, mux.as_ref(), &plan)),
    }
}

pub fn execute(ctx: &Ctx, mux: &dyn Mux, plan: &Plan) -> Outcome {
    let mut all = Outcome { verified: true, ..Default::default() };
    let mut seg: Vec<Step> = vec![];
    let flush = |seg: &mut Vec<Step>, all: &mut Outcome| -> bool {
        if seg.is_empty() {
            return true;
        }
        let sub = Plan { summary: plan.summary.clone(), steps: std::mem::take(seg), ..Default::default() };
        let o = nimitz_core::sessions::apply(&ctx.env, &sub);
        let done = o.ran.len() == sub.steps.len() && o.issues.iter().all(|i| i.severity != Severity::Error);
        all.verified &= o.verified;
        all.ran.extend(o.ran);
        all.issues.extend(o.issues);
        done
    };
    for step in &plan.steps {
        if !is_pane_step(step) {
            seg.push(step.clone());
            continue;
        }
        if !flush(&mut seg, &mut all) {
            all.verified = false;
            return all;
        }
        if !open_step(ctx, mux, step, &mut all) {
            all.verified = false;
            return all;
        }
    }
    if !flush(&mut seg, &mut all) {
        all.verified = false;
    }
    all
}

/// Open one interactive member in its tab, then bind its session when the
/// SessionStart hook reports it.
fn open_step(ctx: &Ctx, mux: &dyn Mux, step: &Step, all: &mut Outcome) -> bool {
    let Some(id) = step.id.clone() else {
        all.issues.push(refusal("config.invalid", "a pane step without an identity".into(), None, None));
        return false;
    };
    let cwd = step.cwd.clone().unwrap_or_else(|| ".".into());
    // A pane opened earlier that has not bound yet (a trust prompt, a slow start):
    // show it again instead of opening a second one (BUG-47). --force opens anew.
    if !ctx.force() {
        if let Some(p) = pending_pane(ctx, &id) {
            let _ = mux.focus(&p);
            all.ran.push((step.clone(), 0, format!("already opening in pane {}; it has not bound yet (trust prompt?)", p.pane_id)));
            all.issues.push(Issue {
                code: "lineage.pending".into(),
                severity: Severity::Warn,
                message: format!("already opening in pane {}; it has not bound yet (trust prompt?); focused it", p.pane_id),
                fix: Some(format!("answer it in pane {}, or nimitz start {id} --force for a new pane", p.pane_id)),
                id: Some(id),
            });
            return true;
        }
    }
    let since = nimitz_core::evidence::now() - 0.001;
    match mux.open(&step.cmd, Path::new(&cwd), place_for(&id)) {
        Ok(p) => {
            mark_pending(ctx, &id, &p);
            all.ran.push((step.clone(), 0, format!("opened in {} pane {}", mux.name(), p.pane_id)))
        }
        Err(e) => {
            all.ran.push((step.clone(), 1, e.to_string()));
            all.issues.push(refusal("mux.failed", format!("could not open {id}: {e}"), None, Some(id)));
            return false;
        }
    }
    // The pane runs on its own; once its SessionStart arrives, core binds and verifies it.
    if wait_binding(ctx, &id, since).is_none() {
        all.issues.push(Issue {
            code: "lineage.pending".into(),
            severity: Severity::Warn,
            message: "opened; its SessionStart has not arrived yet, so the registry binds it later (status finds it by its labels)".into(),
            fix: Some("nimitz status".into()),
            id: Some(id),
        });
        return true;
    }
    match nimitz_core::sessions::bind_started(&ctx.env, &id, Some(&cwd), since, None, "launch", 2.0) {
        Ok(sid) => {
            clear_pending(ctx, &id);
            if let Some(last) = all.ran.last_mut() {
                last.2 = format!("{}; bound {sid}", last.2); // one line per step, not two
            }
            true
        }
        Err(issues) => {
            all.issues.extend(issues);
            false
        }
    }
}

/// How long a pane that has not bound counts as "still opening".
const PENDING_TTL_S: f64 = 600.0;

fn pending_path(ctx: &Ctx, id: &Id) -> std::path::PathBuf {
    ctx.env.state.join("pending").join(format!("{}.json", id.0.replace('/', "%2F")))
}

/// Record the pane an interactive member is opening in, until it binds.
fn mark_pending(ctx: &Ctx, id: &Id, p: &PaneRef) {
    let path = pending_path(ctx, id);
    let v = serde_json::json!({"pane_id": p.pane_id, "tab_id": p.tab_id, "workspace_id": p.workspace_id, "at": nimitz_core::evidence::now()});
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(path, v.to_string());
}

fn clear_pending(ctx: &Ctx, id: &Id) {
    let _ = std::fs::remove_file(pending_path(ctx, id));
}

/// The pane of an earlier open for `id`, if it is younger than the TTL.
fn pending_pane(ctx: &Ctx, id: &Id) -> Option<PaneRef> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(pending_path(ctx, id)).ok()?).ok()?;
    let at = v.get("at")?.as_f64()?;
    if nimitz_core::evidence::now() - at > PENDING_TTL_S {
        clear_pending(ctx, id);
        return None;
    }
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    Some(PaneRef { pane_id: s("pane_id")?, tab_id: s("tab_id"), workspace_id: s("workspace_id") })
}

/// Seconds to wait for an interactive member's SessionStart (`NIMITZ_BIND_TIMEOUT`, default 10).
fn wait_binding(ctx: &Ctx, id: &Id, since: f64) -> Option<String> {
    let secs: f64 = ctx.vars.get("NIMITZ_BIND_TIMEOUT").and_then(|v| v.parse().ok()).unwrap_or(10.0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(secs.max(0.0));
    loop {
        let found = nimitz_core::evidence::events(&ctx.env).into_iter().rev().find(|e| {
            e.ts >= since && e.event == "SessionStart" && e.labels.get("NIMITZ_ID") == Some(&id.0)
        });
        if let Some(sid) = found.and_then(|e| e.session_id) {
            return Some(sid);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
