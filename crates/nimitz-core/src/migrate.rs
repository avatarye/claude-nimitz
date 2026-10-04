//! Import fleets made by the old fleet skill (FLEET_CONFIG.json) as squadrons of a nimitz fleet. Owner: core-config.
use crate::env::Env;
use crate::types::*;

/// Read each old fleet dir and plan the new layout under NIMITZ_HOME/<into>/.
/// Never touches the old dirs. Returns a Plan of file writes and a report.
pub fn plan_from_fleet_skill(_env: &Env, _old_dirs: &[std::path::PathBuf], _into: &str) -> (Plan, Vec<String>) {
    todo!("core-config")
}
