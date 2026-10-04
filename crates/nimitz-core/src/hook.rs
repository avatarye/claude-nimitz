//! `nimitz hook <Event>` logic: append an Event; for PreToolUse SendMessage, decide
//! allow/deny from the comms rule (contracts.md sections 5, plan-v3 section 7). Owner: core-sessions.
use crate::env::Env;

/// Handle one hook call. `stdin` is Claude Code's hook JSON; `vars` the hook's env.
/// Returns the text to print on stdout (empty, or a decision JSON). Never panics.
pub fn handle(_env: &Env, _event: &str, _stdin: &str, _vars: &std::collections::HashMap<String, String>) -> String {
    todo!("core-sessions")
}
