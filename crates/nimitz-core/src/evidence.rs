//! Readers: session files, job rows, transcripts, sub-agents, task lists, events.jsonl (contracts.md section 6). Owner: core-sessions.
use crate::env::Env;
use crate::types::*;

/// Read events.jsonl (tolerant).
pub fn events(_env: &Env) -> Vec<Event> {
    todo!("core-sessions")
}
