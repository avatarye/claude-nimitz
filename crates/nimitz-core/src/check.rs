//! `nimitz check`: integrity before every start (contracts.md section 7). Owner: core-config.
//! Session-level issues come from `crate::sessions::session_issues`.
use crate::env::Env;
use crate::types::*;

/// All issues for one identity (and its parents), or for everything when `id` is None.
pub fn check(_env: &Env, _snap: &Snapshot, _id: Option<&Id>) -> Vec<Issue> {
    todo!("core-config")
}
