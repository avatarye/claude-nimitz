//! Registry of identities and lineage (contracts.md section 5). Owner: core-config.
use crate::env::Env;
use crate::types::*;

pub fn load(_env: &Env) -> Registry {
    todo!("core-config")
}
/// Atomic save under an exclusive lock.
pub fn save(_env: &Env, _r: &Registry) -> std::io::Result<()> {
    todo!("core-config")
}
/// Lock, load, apply `f`, save. The only way to change the registry.
pub fn update<T>(_env: &Env, _f: impl FnOnce(&mut Registry) -> T) -> std::io::Result<T> {
    todo!("core-config")
}
/// Record a new current session for an identity (appends to lineage).
pub fn bind(_r: &mut Registry, _id: &Id, _role: Role, _name: &str, _cwd: &str, _session_id: &str, _via: &str, _now: f64) {
    todo!("core-config")
}
