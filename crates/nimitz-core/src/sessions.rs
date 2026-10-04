//! Session actions along the chain of command: start (top-down), stop, relaunch,
//! rm, sweep, tender, adopt, signal; the relaunch state table (contracts.md section 8). Owner: core-sessions.
use crate::env::Env;
use crate::types::*;

/// Session-level issues for one member (name.drift, labels.missing, dup.live, ...).
pub fn session_issues(_env: &Env, _m: &Member) -> Vec<Issue> {
    todo!("core-sessions")
}
pub fn caller_from_env(_vars: &std::collections::HashMap<String, String>) -> Caller {
    todo!("core-sessions")
}
/// Start `id` and every missing parent, top-down. Runs `check` first.
pub fn plan_start(_env: &Env, _snap: &Snapshot, _id: &Id, _caller: &Caller, _force: bool) -> Plan {
    todo!("core-sessions")
}
/// `nimitz up <fleet>`: start every member that should run on this machine.
pub fn plan_up(_env: &Env, _snap: &Snapshot, _fleet: &str, _caller: &Caller) -> Plan {
    todo!("core-sessions")
}
pub fn plan_stop(_env: &Env, _snap: &Snapshot, _id: &Id, _caller: &Caller) -> Plan {
    todo!("core-sessions")
}
pub fn plan_relaunch(_env: &Env, _snap: &Snapshot, _id: &Id, _caller: &Caller) -> Plan {
    todo!("core-sessions")
}
/// Remove job rows of a stopped non-fleet session or stowaway, or a member's stale rows.
pub fn plan_rm(_env: &Env, _snap: &Snapshot, _target: &str, _caller: &Caller) -> Plan {
    todo!("core-sessions")
}
pub fn plan_sweep(_env: &Env, _snap: &Snapshot, _id: Option<&Id>, _caller: &Caller) -> Plan {
    todo!("core-sessions")
}
pub fn plan_tender(_env: &Env, _snap: &Snapshot, _commodore: &Id, _task: &str, _prompt: &str, _caller: &Caller) -> Plan {
    todo!("core-sessions")
}
pub fn plan_adopt(_env: &Env, _snap: &Snapshot, _session_id: &str, _id: &Id, _caller: &Caller) -> Plan {
    todo!("core-sessions")
}
/// Run a plan's steps, then re-read state and verify (never trust exit codes alone).
pub fn apply(_env: &Env, _plan: &Plan) -> Outcome {
    todo!("core-sessions")
}
