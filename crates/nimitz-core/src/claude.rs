//! Running the `claude` CLI: launch, resume, stop, rm, agents (contracts.md sections 2, 8). Owner: core-sessions.
use crate::env::Env;

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: Option<String>,
    pub session_id: String,
    pub name: String,
    pub cwd: String,
    pub kind: String,
    pub pid: Option<i64>,
    pub state: Option<String>,
    pub status: Option<String>,
    pub started_at: Option<f64>,
}

/// `claude agents --json --all` through the native binary.
pub fn agents(_env: &Env) -> Result<Vec<Row>, String> {
    todo!("core-sessions")
}
pub fn version(_env: &Env) -> Option<String> {
    todo!("core-sessions")
}
