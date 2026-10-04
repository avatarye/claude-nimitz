//! Fleet and squadron config: load, validate, save, CRUD (contracts.md section 3). Owner: core-config.
use crate::env::Env;
use crate::types::*;

/// Every fleet under NIMITZ_HOME, with config errors as issues (never panics).
pub fn load_all(_env: &Env) -> (Vec<FleetOnDisk>, Vec<Issue>) {
    todo!("core-config")
}
pub fn load_fleet(_env: &Env, _fleet: &str) -> Result<FleetOnDisk, Issue> {
    todo!("core-config")
}
/// Schema and layout rules; returns issues (config.invalid, config.layout, ...).
pub fn validate(_env: &Env, _f: &FleetOnDisk) -> Vec<Issue> {
    todo!("core-config")
}
/// The folder of a member on this machine (resolves "managed"); None if not deployed here.
pub fn member_dir(_env: &Env, _f: &FleetOnDisk, _id: &Id) -> Option<std::path::PathBuf> {
    todo!("core-config")
}
/// The launch prompt for a ship (prompt or prompt_file).
pub fn ship_prompt(_f: &FleetOnDisk, _id: &Id) -> Option<String> {
    todo!("core-config")
}
/// Permission mode for a member on this machine.
pub fn permission_mode(_f: &FleetOnDisk, _id: &Id) -> String {
    todo!("core-config")
}

// CRUD: each returns a Plan (file writes as steps with cmd ["write", path]) and
// applies only through `apply_config_plan`.
pub fn plan_fleet_new(_env: &Env, _fleet: &str, _machine_label: &str) -> Plan {
    todo!("core-config")
}
pub fn plan_squadron_new(_env: &Env, _fleet: &str, _sq: &str, _brief: &str, _comms: Comms) -> Plan {
    todo!("core-config")
}
pub fn plan_ship_new(_env: &Env, _fleet: &str, _sq: &str, _ship: ShipDef) -> Plan {
    todo!("core-config")
}
pub fn plan_ship_set(_env: &Env, _id: &Id, _ship: ShipDef) -> Plan {
    todo!("core-config")
}
pub fn plan_rm(_env: &Env, _id: &Id) -> Plan {
    todo!("core-config")
}
pub fn plan_machine_add(_env: &Env, _fleet: &str, _label: &str, _dirs: Vec<(Id, String)>) -> Plan {
    todo!("core-config")
}
pub fn apply_config_plan(_env: &Env, _plan: &Plan) -> Outcome {
    todo!("core-config")
}
