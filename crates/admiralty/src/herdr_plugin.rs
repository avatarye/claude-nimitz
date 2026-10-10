//! The Herdr plugin's two commands (`herdr-plugin/herdr-plugin.toml`), in the
//! binary so one manifest runs on macOS, Linux and Windows (Herdr runs a
//! command as an argv, no shell, and wants one command per action id):
//!
//! - `admiralty herdr toggle` (the `toggle` action): the overlay opens in this
//!   tab; pressed again it closes; open in another tab, it moves here.
//! - `admiralty herdr watch` (the `watch` overlay pane): the TUI with the
//!   overlay's defaults (`watch_setup`).
//!
//! The id of the overlay the toggle opened is kept in the plugin state dir. A
//! stale id (the watch quit on its own) fails `pane get`; one that now names
//! another pane (ids restart with the server) fails the label check and is
//! left alone: the toggle never closes a pane it did not open.
use std::collections::HashMap;
use std::path::PathBuf;

/// The `[[panes]]` title, which Herdr reports as the pane label.
pub const LABEL: &str = "Admiralty";

/// The first `"key": "value"` string field anywhere in a JSON reply.
pub fn field(key: &str, json: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let mut from = 0;
    while let Some(at) = json[from..].find(&pat) {
        let rest = json[from + at + pat.len()..].trim_start();
        if let Some(v) = rest.strip_prefix(':').map(str::trim_start).and_then(|r| r.strip_prefix('"')) {
            if let Some(end) = v.find('"') {
                return Some(v[..end].to_string());
            }
        }
        from += at + pat.len();
    }
    None
}

/// What the toggle did (the exit code follows from it).
#[derive(Debug, PartialEq)]
pub enum Toggled {
    Opened(String),
    Closed,
    OpenedNoId(String),
    Failed(String),
}

/// The toggle, with `herdr` the Herdr CLI (its stdout, or None when it
/// failed) and `state` the file that keeps the overlay's pane id.
pub fn toggle_with(vars: &HashMap<String, String>, state: &std::path::Path, herdr: &dyn Fn(&[&str]) -> Option<String>) -> Toggled {
    let get = |k: &str| vars.get(k).filter(|v| !v.is_empty()).cloned();
    let plugin = get("HERDR_PLUGIN_ID").unwrap_or_else(|| "admiralty".into());
    if let Some(dir) = state.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            return Toggled::Failed(format!("cannot make {}: {e}", dir.display()));
        }
    }
    let id = std::fs::read_to_string(state).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(id) = id {
        let _ = std::fs::remove_file(state);
        if let Some(info) = herdr(&["pane", "get", &id]).filter(|i| field("label", i).as_deref() == Some(LABEL)) {
            if herdr(&["plugin", "pane", "close", &id]).is_none() {
                return Toggled::Failed(format!("herdr plugin pane close {id} failed"));
            }
            let here = get("HERDR_PLUGIN_CONTEXT_JSON").and_then(|c| field("tab_id", &c)).or_else(|| get("HERDR_TAB_ID"));
            if here.is_none() || field("tab_id", &info) == here {
                return Toggled::Closed;
            }
        }
    }
    let Some(out) = herdr(&["plugin", "pane", "open", "--plugin", &plugin, "--entrypoint", "watch", "--focus"]) else {
        return Toggled::Failed("herdr plugin pane open failed".into());
    };
    match field("pane_id", &out) {
        Some(id) => match std::fs::write(state, format!("{id}\n")) {
            Ok(()) => Toggled::Opened(id),
            Err(e) => Toggled::Failed(format!("opened {id} but cannot keep its id in {}: {e}", state.display())),
        },
        None => Toggled::OpenedNoId(out),
    }
}

/// `admiralty herdr toggle`: the toggle against the running Herdr
/// (`HERDR_BIN_PATH`, which Herdr gives every plugin command).
pub fn toggle(vars: &HashMap<String, String>) -> i32 {
    let get = |k: &str| vars.get(k).filter(|v| !v.is_empty()).cloned();
    let bin = get("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let state_dir = get("HERDR_PLUGIN_STATE_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("herdr-admiralty"));
    let run = |args: &[&str]| -> Option<String> {
        let o = std::process::Command::new(&bin).args(args).stdin(std::process::Stdio::null()).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    match toggle_with(vars, &state_dir.join("pane_id"), &run) {
        Toggled::Opened(_) | Toggled::Closed => 0,
        Toggled::OpenedNoId(out) => {
            eprintln!("[WARN] opened the watch but found no pane_id in: {}", out.trim());
            0
        }
        Toggled::Failed(why) => {
            eprintln!("[ERROR] {why}");
            1
        }
    }
}

/// `admiralty herdr watch`: the overlay's defaults in the process env, and
/// the extra arguments the TUI runs with:
/// - `ADMIRALTY_EXIT_AFTER_JUMP` 1: the overlay closes once enter lands on a pane;
/// - `ADMIRALTY_INSET` 2,2: Herdr 0.9.3 with its sidebar shown sizes an
///   overlay's terminal to the whole tab and draws the border inside it, so
///   the last 2 rows and columns never show (0,0 turns it off);
/// - arguments from `ADMIRALTY_ARGS`, else `$HERDR_PLUGIN_CONFIG_DIR/args`,
///   split on white space (e.g. `--sandbox /tmp/aw-sb`).
///
/// The `ADMIRALTY_WATCH_*` names of nimitz 0.3.0 count when the new ones are unset.
pub fn watch_setup(vars: &HashMap<String, String>) -> (Vec<(String, String)>, Vec<String>) {
    let get = |k: &str| vars.get(k).cloned();
    let jump = get("ADMIRALTY_EXIT_AFTER_JUMP").or_else(|| get("ADMIRALTY_WATCH_EXIT_AFTER_JUMP")).unwrap_or_else(|| "1".into());
    let inset = get("ADMIRALTY_INSET").unwrap_or_else(|| "2,2".into());
    let args = get("ADMIRALTY_ARGS")
        .or_else(|| get("ADMIRALTY_WATCH_ARGS"))
        .or_else(|| get("HERDR_PLUGIN_CONFIG_DIR").filter(|d| !d.is_empty()).and_then(|d| std::fs::read_to_string(PathBuf::from(d).join("args")).ok()))
        .unwrap_or_default();
    (vec![("ADMIRALTY_EXIT_AFTER_JUMP".into(), jump), ("ADMIRALTY_INSET".into(), inset)], args.split_whitespace().map(String::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A fake Herdr: panes as (id, tab, label); `open` adds the watch.
    struct Fake {
        panes: RefCell<Vec<(String, String, String)>>,
        next: RefCell<u32>,
        open_fails: bool,
        tab: String,
    }

    impl Fake {
        fn new() -> Fake {
            Fake { panes: RefCell::new(vec![]), next: RefCell::new(2), open_fails: false, tab: String::new() }
        }
        fn run(&self, a: &[&str]) -> Option<String> {
            match a {
                ["pane", "get", id] => self.panes.borrow().iter().find(|p| p.0 == *id).map(|p| format!(r#"{{"result":{{"pane":{{"pane_id":"{}","tab_id":"{}","label":"{}"}}}}}}"#, p.0, p.1, p.2)),
                ["plugin", "pane", "close", id] => {
                    self.panes.borrow_mut().retain(|p| p.0 != *id);
                    Some("{}".into())
                }
                ["plugin", "pane", "open", ..] if !self.open_fails => {
                    let id = format!("w1:p{}", self.next.borrow());
                    *self.next.borrow_mut() += 2;
                    self.panes.borrow_mut().push((id.clone(), self.tab.clone(), LABEL.into()));
                    Some(format!(r#"{{"result":{{"pane":{{"pane_id":"{id}"}}}}}}"#))
                }
                _ => None,
            }
        }
        fn ids(&self) -> Vec<String> {
            self.panes.borrow().iter().map(|p| format!("{} {}", p.0, p.1)).collect()
        }
    }

    fn press(f: &mut Fake, state: &std::path::Path, tab: &str) -> Toggled {
        f.tab = tab.into();
        let vars: HashMap<String, String> = [("HERDR_TAB_ID".to_string(), tab.to_string()), ("HERDR_PLUGIN_CONTEXT_JSON".to_string(), format!(r#"{{"tab_id":"{tab}"}}"#))].into();
        let f2: &Fake = f;
        toggle_with(&vars, state, &|a| f2.run(a))
    }

    #[test]
    fn the_toggle_opens_closes_moves_and_never_closes_another_pane() {
        let td = tempfile::tempdir().unwrap();
        let state = td.path().join("state").join("pane_id");
        let kept = || std::fs::read_to_string(&state).map(|s| s.trim().to_string()).unwrap_or_default();
        let mut f = Fake::new();
        assert_eq!(press(&mut f, &state, "w1:t1"), Toggled::Opened("w1:p2".into()), "opens when closed");
        assert_eq!((f.ids(), kept()), (vec!["w1:p2 w1:t1".to_string()], "w1:p2".to_string()));
        assert_eq!(press(&mut f, &state, "w1:t1"), Toggled::Closed, "closes when open in this tab");
        assert!(f.ids().is_empty() && kept().is_empty());
        press(&mut f, &state, "w1:t1");
        assert_eq!(press(&mut f, &state, "w1:t2"), Toggled::Opened("w1:p6".into()), "moves to the calling tab");
        assert_eq!(f.ids(), vec!["w1:p6 w1:t2".to_string()]);
        f.panes.borrow_mut().clear(); // the watch quit on its own (q, or a jump)
        assert_eq!(press(&mut f, &state, "w1:t2"), Toggled::Opened("w1:p8".into()), "a stale id opens a new one");
        // the id now names someone else's pane
        *f.panes.borrow_mut() = vec![("w1:p8".into(), "w1:t2".into(), "zsh".into())];
        assert_eq!(press(&mut f, &state, "w1:t2"), Toggled::Opened("w1:p10".into()), "never closes a pane that is not the watch");
        assert_eq!(f.ids(), vec!["w1:p8 w1:t2".to_string(), "w1:p10 w1:t2".to_string()]);
    }

    #[test]
    fn a_failed_open_leaves_no_state() {
        let td = tempfile::tempdir().unwrap();
        let state = td.path().join("pane_id");
        let mut f = Fake { open_fails: true, ..Fake::new() };
        assert!(matches!(press(&mut f, &state, "w1:t1"), Toggled::Failed(_)));
        assert!(!state.exists());
    }

    #[test]
    fn a_field_is_the_first_string_of_its_key() {
        assert_eq!(field("pane_id", r#"{"result": {"pane": {"pane_id" : "w1:p3", "tab_id":"w1:t1"}}}"#).as_deref(), Some("w1:p3"));
        assert_eq!(field("label", r#"{"label": 3, "x": {"label":"Admiralty"}}"#).as_deref(), Some("Admiralty"));
        assert_eq!(field("tab_id", "not json"), None);
    }

    #[test]
    fn the_watch_defaults_and_its_arguments() {
        let td = tempfile::tempdir().unwrap();
        std::fs::write(td.path().join("args"), "--sandbox /tmp/aw-sb\n").unwrap();
        let cfg = td.path().to_string_lossy().into_owned();
        let (env, args) = watch_setup(&[("HERDR_PLUGIN_CONFIG_DIR".to_string(), cfg.clone())].into());
        assert_eq!(env, vec![("ADMIRALTY_EXIT_AFTER_JUMP".to_string(), "1".to_string()), ("ADMIRALTY_INSET".to_string(), "2,2".to_string())]);
        assert_eq!(args, ["--sandbox", "/tmp/aw-sb"]);
        // the env wins over the file; the 0.3.0 names still count
        let (env, args) = watch_setup(&[("HERDR_PLUGIN_CONFIG_DIR".to_string(), cfg), ("ADMIRALTY_ARGS".to_string(), String::new()), ("ADMIRALTY_WATCH_EXIT_AFTER_JUMP".to_string(), "0".to_string())].into());
        assert!(args.is_empty());
        assert_eq!(env[0].1, "0");
    }
}
