//! A fleet's settings: `<NIMITZ_HOME>/<fleet>/settings.json`, beside
//! fleet.toml but apart from it (its own format, so a key one machine does
//! not know never stops another from loading the fleet). Members read it
//! (`nimitz fleet settings get`); the fleet's admiral and the user change it
//! (`nimitz fleet settings set`, planned). Keys are free; the ones nimitz
//! reads are checked on a write:
//!
//! - `kb`: the fleet's knowledge base, `{"remote": "<git url>", "dirs":
//!   {"<machine label>": "<folder>"}}`. `nimitz kb path` is this machine's
//!   folder (by the fleet's `[machines]` label).
use crate::env::Env;
use crate::types::*;
use serde_json::{Map, Value};
use std::path::PathBuf;

/// The format this nimitz writes (`formats`: artifact `fleet-settings`).
pub const FORMAT: u32 = 1;

pub fn path(fleet_dir: &std::path::Path) -> PathBuf {
    fleet_dir.join("settings.json")
}

/// The settings of a fleet on disk: `{}` when it has no file. A file a newer
/// nimitz wrote (its `format` or `min_nimitz`) or one that is no JSON object
/// is an error, never read as empty.
pub fn load(f: &FleetOnDisk) -> Result<Map<String, Value>, String> {
    let p = path(&f.dir);
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(format!("{}: {e}", p.display())),
    };
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| format!("{} does not parse: {e}", p.display()))?;
    crate::formats::guard_json("fleet-settings", &p, &v, "format").map_err(|t| t.to_string())?;
    match v {
        Value::Object(m) => Ok(m),
        _ => Err(format!("{} is not a JSON object", p.display())),
    }
}

/// The text written: `format` first, then the keys as they are.
pub fn render(m: &Map<String, Value>) -> String {
    let mut out = Map::new();
    out.insert("format".into(), Value::from(FORMAT));
    for (k, v) in m.iter().filter(|(k, _)| *k != "format") {
        out.insert(k.clone(), v.clone());
    }
    serde_json::to_string_pretty(&Value::Object(out)).unwrap_or_default() + "\n"
}

/// A dotted key's parts (`kb.dirs.home-server`): each a non-empty name.
fn parts(key: &str) -> Result<Vec<&str>, String> {
    let p: Vec<&str> = key.split('.').collect();
    if key.is_empty() || p.iter().any(|s| s.is_empty()) {
        return Err(format!("{key:?} is no key (names joined by dots, e.g. kb.remote)"));
    }
    if p[0] == "format" || p[0] == "min_nimitz" {
        return Err(format!("{key:?} is nimitz's own stamp, not a setting"));
    }
    Ok(p)
}

/// The value at a dotted key.
pub fn get<'a>(m: &'a Map<String, Value>, key: &str) -> Result<Option<&'a Value>, String> {
    let p = parts(key)?;
    let mut v = m.get(p[0]);
    for k in &p[1..] {
        v = v.and_then(|x| x.get(*k));
    }
    Ok(v)
}

/// Set (or, with `None`, remove) a dotted key; the tables on the way are made.
pub fn set(m: &mut Map<String, Value>, key: &str, value: Option<Value>) -> Result<(), String> {
    let p = parts(key)?;
    let (last, path) = p.split_last().expect("one part at least");
    let mut cur = m;
    for k in path {
        let next = cur.entry(k.to_string()).or_insert_with(|| Value::Object(Map::new()));
        cur = match next {
            Value::Object(o) => o,
            _ => return Err(format!("{key}: {k} holds a value, not a table")),
        };
    }
    match value {
        Some(v) => {
            cur.insert(last.to_string(), v);
        }
        None => {
            cur.remove(*last);
        }
    }
    Ok(())
}

/// A value as typed: JSON when it reads as JSON (`true`, `3`, `{"a":1}`,
/// `"x"`), else the text itself.
pub fn parse_value(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
}

/// What is wrong with the keys nimitz reads (empty: nothing).
pub fn problems(m: &Map<String, Value>, machines: &std::collections::BTreeMap<String, MachineDef>) -> Vec<String> {
    let mut out = vec![];
    if let Some(kb) = m.get("kb") {
        let Value::Object(kb) = kb else {
            return vec!["kb: a table {\"remote\": ..., \"dirs\": {...}}".into()];
        };
        for k in kb.keys().filter(|k| !matches!(k.as_str(), "remote" | "dirs")) {
            out.push(format!("kb.{k}: not a kb key (remote, dirs)"));
        }
        // a URL, a scp-like `host:path`, or an absolute folder: never relative
        // (each clone would read it from its own folder)
        let drive = |s: &str| s.len() > 2 && s.as_bytes()[0].is_ascii_alphabetic() && s.as_bytes()[1] == b':' && matches!(s.as_bytes()[2], b'/' | b'\\');
        // `host:path` as git reads it: a `:` before any `/`, a host of two or more
        let scp = |s: &str| s.split_once(':').is_some_and(|(h, _)| h.len() > 1 && !h.contains('/') && !h.starts_with('.'));
        let url = |s: &str| s.contains("://") || drive(s) || s.starts_with('/') || s.starts_with("\\\\") || scp(s);
        if kb.get("remote").is_some_and(|r| !r.as_str().is_some_and(|s| !s.trim().is_empty() && url(s.trim()))) {
            out.push("kb.remote: a git URL, a host:path, or an absolute folder (never relative)".into());
        }
        match kb.get("dirs") {
            None => {}
            Some(Value::Object(d)) => {
                for (label, dir) in d {
                    if !machines.contains_key(label) {
                        out.push(format!("kb.dirs.{label}: no machine {label} in the fleet's [machines] (nimitz machine add first)"));
                    }
                    let abs = |s: &str| s.starts_with('/') || s.starts_with("~/") || (s.len() > 2 && s.as_bytes()[1] == b':' && matches!(s.as_bytes()[2], b'/' | b'\\'));
                    if !dir.as_str().is_some_and(abs) {
                        out.push(format!("kb.dirs.{label}: an absolute folder (/..., ~/..., or C:\\...)"));
                    }
                }
            }
            Some(_) => out.push("kb.dirs: a table {\"<machine label>\": \"<folder>\"}".into()),
        }
    }
    out
}

/// The plan of `fleet settings set <key> <value>` (`None`: remove the key):
/// one write of the whole file. Refused while the file does not read here,
/// or when the result breaks a key nimitz reads.
pub fn plan_set(env: &Env, fleet: &str, key: &str, value: Option<Value>) -> Plan {
    let fid = Id::admiral(fleet);
    let mut plan = Plan { summary: format!("fleet settings set {key} ({fleet})"), ..Default::default() };
    let refuse = |plan: &mut Plan, code: &str, msg: String| plan.refusals.push(Issue { code: code.into(), severity: Severity::Error, message: msg, fix: None, id: Some(fid.clone()) });
    let f = match crate::config::load_fleet(env, fleet) {
        Ok(f) => f,
        Err(i) => {
            plan.refusals.push(i);
            return plan;
        }
    };
    let mut m = match load(&f) {
        Ok(m) => m,
        Err(e) => {
            refuse(&mut plan, "settings.unreadable", e);
            return plan;
        }
    };
    // (a kb value as typed, without the spaces around it)
    let value = value.map(|v| match v {
        Value::String(t) if key.starts_with("kb") => Value::String(t.trim().to_string()),
        v => v,
    });
    if let Err(e) = set(&mut m, key, value) {
        refuse(&mut plan, "settings.key", e);
        return plan;
    }
    for p in problems(&m, &f.config.machines) {
        refuse(&mut plan, "settings.invalid", p);
    }
    if !plan.ok() {
        return plan;
    }
    let p = path(&f.dir);
    let text = render(&m);
    if std::fs::read_to_string(&p).ok().as_deref() == Some(text.as_str()) {
        return plan; // nothing to change
    }
    plan.steps.push(Step { id: Some(fid), cmd: vec!["write".into(), p.to_string_lossy().into_owned(), text], cwd: None, why: format!("{key} in the fleet's settings (shared: every machine of the fleet reads it)") });
    plan
}

/// The fleet's knowledge base on this machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Kb {
    pub fleet: String,
    /// this machine's label in the fleet
    pub label: String,
    /// its folder here (`~` expanded); it may not exist yet (`nimitz kb clone`)
    pub dir: PathBuf,
    pub remote: Option<String>,
}

/// The fleet's KB here, or why there is none.
pub fn kb(env: &Env, fleet: &str) -> Result<Kb, String> {
    let f = crate::config::load_fleet(env, fleet).map_err(|i| i.message)?;
    let m = load(&f)?;
    let Some(Value::Object(kb)) = m.get("kb") else {
        return Err(format!("fleet {fleet} has no knowledge base (no kb in {}): the user sets one with `nimitz fleet settings set kb.remote <git url> --fleet {fleet}`", path(&f.dir).display()));
    };
    let Some(label) = f.machine.clone() else {
        return Err(format!("this machine is not in fleet {fleet}'s [machines]"));
    };
    let remote = kb.get("remote").and_then(Value::as_str).map(|r| r.trim().to_string());
    let Some(dir) = kb.get("dirs").and_then(|d| d.get(&label)).and_then(Value::as_str) else {
        return Err(format!("fleet {fleet}'s knowledge base has no folder on this machine ({label}): the user sets one with `nimitz fleet settings set kb.dirs '{{\"{label}\": \"<folder>\"}}' --fleet {fleet}` (with the other machines' folders too), then `nimitz kb clone --fleet {fleet}`"));
    };
    Ok(Kb { fleet: f.config.fleet.clone(), label, dir: crate::config::expand_dir(dir.trim()), remote })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::{make_fleet, test_env};
    use serde_json::json;

    #[test]
    fn dotted_keys_get_set_and_remove() {
        let mut m = Map::new();
        set(&mut m, "kb.remote", Some(json!("git@x:kb.git"))).unwrap();
        set(&mut m, "kb.dirs.test-mac", Some(json!("~/kb"))).unwrap();
        assert_eq!(get(&m, "kb.dirs.test-mac").unwrap(), Some(&json!("~/kb")));
        assert_eq!(get(&m, "kb.nothing").unwrap(), None);
        assert!(set(&mut m, "kb.remote.x", Some(json!(1))).unwrap_err().contains("holds a value"));
        for bad in ["", "kb..x", ".kb", "format", "min_nimitz.x"] {
            assert!(set(&mut m, bad, Some(json!(1))).is_err(), "{bad}");
        }
        set(&mut m, "kb.dirs.test-mac", None).unwrap();
        assert_eq!(m["kb"]["dirs"], json!({}));
        assert_eq!(parse_value("true"), json!(true));
        assert_eq!(parse_value("~/kb"), json!("~/kb"));
        assert_eq!(parse_value("{\"a\": 1}"), json!({"a": 1}));
        assert!(render(&m).starts_with("{\n  \"format\": 1,\n  \"kb\""), "{}", render(&m));
    }

    #[test]
    fn a_set_is_planned_checked_and_the_kb_resolves_by_this_machines_label() {
        let (td, env) = test_env();
        make_fleet(&env);
        assert!(kb(&env, "MAIN").unwrap_err().contains("has no knowledge base"));
        let apply = |p: &Plan| {
            assert!(p.ok(), "{p:?}");
            assert!(crate::config::apply_config_plan(&env, p).verified);
        };
        apply(&plan_set(&env, "MAIN", "kb.remote", Some(json!("git@github.com:u/kb.git"))));
        assert!(kb(&env, "MAIN").unwrap_err().contains("no folder on this machine (test-mac)"));
        // a machine the fleet does not list, a remote that is no text: refused
        let p = plan_set(&env, "MAIN", "kb.dirs.elsewhere", Some(json!("/x")));
        assert!(p.refusals.iter().any(|r| r.message.contains("no machine elsewhere")), "{p:?}");
        assert!(!plan_set(&env, "MAIN", "kb.remote", Some(json!(3))).ok());
        for bad in ["../kb.git", "kb", "~/kb.git", "sub/dir:x", "a:repo"] {
            assert!(!plan_set(&env, "MAIN", "kb.remote", Some(json!(bad))).ok(), "{bad}");
        }
        for good in ["https://github.com/u/kb", "git@github.com:u/kb.git", "/srv/kb.git", "C:/kb.git", "C:\\kb.git", "\\\\server\\share\\kb.git", "ssh://h/kb", "file:///srv/kb.git"] {
            assert!(plan_set(&env, "MAIN", "kb.remote", Some(json!(good))).ok(), "{good}");
        }
        assert!(!plan_set(&env, "MAIN", "kb.color", Some(json!("x"))).ok());
        let dir = td.path().join("kb");
        apply(&plan_set(&env, "MAIN", "kb.dirs.test-mac", Some(json!(dir.to_string_lossy()))));
        let k = kb(&env, "MAIN").unwrap();
        assert_eq!((k.label.as_str(), k.dir.clone(), k.remote.as_deref()), ("test-mac", dir, Some("git@github.com:u/kb.git")));
        // kb.* is the user's alone; a folder is absolute
        assert!(plan_set(&env, "MAIN", "notes.x", Some(json!(1))).ok());
        assert!(plan_set(&env, "MAIN", "kb.dirs.test-mac", Some(json!("kb"))).refusals.iter().any(|r| r.message.contains("an absolute folder")));
        // the same value again: nothing to write
        assert!(plan_set(&env, "MAIN", "kb.remote", Some(json!("git@github.com:u/kb.git"))).steps.is_empty());
        // free keys are kept, and read back
        apply(&plan_set(&env, "MAIN", "notes.channel", Some(json!("x"))));
        let f = crate::config::load_fleet(&env, "MAIN").unwrap();
        assert_eq!(get(&load(&f).unwrap(), "notes.channel").unwrap(), Some(&json!("x")));
        // a file a newer nimitz wrote: never read as empty, never written over
        let p = path(&f.dir);
        std::fs::write(&p, "{\"format\": 2, \"kb\": {}}").unwrap();
        assert!(load(&f).unwrap_err().contains("newer than this nimitz reads"));
        assert!(plan_set(&env, "MAIN", "x", Some(json!(1))).refusals.iter().any(|r| r.code == "settings.unreadable"));
        std::fs::write(&p, "[1]").unwrap();
        assert!(load(&f).unwrap_err().contains("not a JSON object"));
    }
}
