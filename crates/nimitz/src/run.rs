//! Running a session plan: every step goes to `sessions::apply` (every member
//! is a background job). The multiplexer only attaches: `attach_in_tab` shows
//! a member's job in its tab (`nimitz open`, `signal` to a lead, a restore).
use crate::{out, Ctx};
use nimitz_core::env::Env;
use nimitz_core::{Id, Issue, Member, Outcome, Plan, Severity};
use nimitz_mux::{AttachError, Attached, Mux, PaneRef, Place};
use std::path::Path;

/// Herdr workspace for a member: `F` for the admiral, `F.SQ` for the rest.
pub fn workspace_of(id: &Id) -> String {
    nimitz_core::layout::workspace_of(id)
}

/// Tab label: `F.ADMIRAL`, `F.SQ.COMMODORE` or `F.SQ.SHIP`.
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

/// A member caller sees only fixes it may run itself (`sessions::may_run_in`,
/// the authority table); any other fix becomes "ask the user to run: ..."
/// (BUG-51). The user sees every fix as is.
pub fn fixes_for(env: &nimitz_core::env::Env, caller: &nimitz_core::Caller, plan: &mut Plan) {
    if *caller == nimitz_core::Caller::User {
        return;
    }
    for i in plan.refusals.iter_mut().chain(plan.warnings.iter_mut()) {
        if let Some(f) = i.fix.take() {
            i.fix = Some(if nimitz_core::sessions::may_run_in(Some(env), caller, &f) { f } else { format!("ask the user to run: {f}") });
        }
    }
}

/// Gate, then run. Returns the exit code.
pub fn session_plan(ctx: &Ctx, plan: Plan) -> i32 {
    match ctx.gate(&plan) {
        out::Go::Exit(code) => code,
        out::Go::Run => out::outcome(&execute(ctx, &plan)),
    }
}

pub fn execute(ctx: &Ctx, plan: &Plan) -> Outcome {
    nimitz_core::sessions::apply(&ctx.env, plan)
}

/// Show `m`'s job in its tab (`claude attach <job>` in its folder, tab
/// `F.ADMIRAL` / `F.SQ.COMMODORE` / `F.SQ.SHIP`; a parked job wakes): focus
/// the pane that shows it, claim its tab's shell pane, or open the tab
/// (`nimitz_mux::attach`). A member with no job, or one whose tab runs
/// something else (`tab.taken`: nothing is typed), is an `Err` with the issue
/// that says what to do. The attach-a-lead step a restore uses too.
pub fn attach_in_tab(mux: &dyn Mux, env: &Env, m: &Member) -> Result<Attached, Issue> {
    let tab = nimitz_core::layout::attach_tab(env, m)?;
    nimitz_mux::attach(mux, &tab.cmd, Path::new(&tab.cwd), place_for(&m.id), tab.session_id.as_deref()).map_err(|e| attach_issue(&m.id, e))
}

/// The issue for a failed attach: a taken tab names the pane and what runs
/// there, with the one fix (end it there, or ctrl-z an attach of another job).
pub fn attach_issue(id: &Id, e: AttachError) -> Issue {
    match e {
        AttachError::Taken { pane, running } => refusal(
            "tab.taken",
            format!("{id}'s tab is open but its pane {} runs `{running}`, not a shell; nothing was typed into it", pane.pane_id),
            Some(format!("in pane {}: ctrl-z (an attach) or quit `{running}`, then nimitz open {id}", pane.pane_id)),
            Some(id.clone()),
        ),
        AttachError::Mux(e) => refusal("mux.failed", format!("attaching {id} failed: {e}"), None, Some(id.clone())),
    }
}
