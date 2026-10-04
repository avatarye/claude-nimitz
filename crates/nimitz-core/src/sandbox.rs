//! Sandbox world (fake NIMITZ_HOME, fake ~/.claude, fake `claude`) for tests and demos.
//! The fake `claude` is `nimitz __fake-claude <sandbox> ...` (contracts.md section 10). Owner: core-sessions.
use std::path::Path;

/// Build a world with every relaunch state of contracts.md section 8. `exe` is the nimitz binary.
pub fn create(_base: &Path, _exe: &Path) -> Result<(), String> {
    todo!("core-sessions")
}
/// Env vars that point nimitz/admiralty at the sandbox.
pub fn env_for(_base: &Path) -> std::collections::HashMap<String, String> {
    todo!("core-sessions")
}
/// The fake `claude`: agents --json, --bg (cold/resume with copy semantics as probed), stop, rm, attach, --version.
pub fn fake_claude(_base: &Path, _args: &[String]) -> i32 {
    todo!("core-sessions")
}
