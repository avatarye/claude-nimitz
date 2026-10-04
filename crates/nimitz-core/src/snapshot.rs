//! Build the Snapshot from config + registry + evidence (contracts.md section 6). Owner: core-sessions.
use crate::env::Env;
use crate::types::*;

/// Caches between refreshes (mtime-keyed JSON, titles, session dirs).
#[derive(Default)]
pub struct Cache {}

pub fn snapshot(_env: &Env, _cache: &mut Cache) -> Snapshot {
    todo!("core-sessions")
}
