//! Supported Claude Code range (compat.toml). Owner: core-config.
use crate::env::Env;

#[derive(Clone, Debug, PartialEq)]
pub struct Compat {
    pub min: String,
    pub max: String,
}

/// The range compiled into this build (from docs/compat.toml at build time or a const).
pub fn supported() -> Compat {
    todo!("core-config")
}
/// Installed version (native binary `--version`), e.g. "2.1.288".
pub fn installed(_env: &Env) -> Option<String> {
    todo!("core-config")
}
/// -1 below, 0 inside, 1 above the range.
pub fn position(_c: &Compat, _version: &str) -> i32 {
    todo!("core-config")
}
