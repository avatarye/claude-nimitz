//! Paths and overrides (contracts.md section 2). Owner: core-config.
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Env {
    pub home: PathBuf,
    pub state: PathBuf,
    pub claude_home: PathBuf,
    pub claude_bin: String,
    pub native_claude_bin: String,
    pub herdr_bin: Option<String>,
    pub hostname: String,
    pub mux: Option<String>,
}

impl Env {
    /// Build from variables (the process env in normal use; a map in tests).
    pub fn from_vars(_v: &HashMap<String, String>) -> Env {
        todo!("core-config")
    }
    pub fn from_process() -> Env {
        Env::from_vars(&std::env::vars().collect())
    }
}
