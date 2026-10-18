//! The Herdr plugin's two commands (`herdr-plugin/herdr-plugin.toml`), in the
//! binary so one manifest runs on macOS, Linux and Windows (Herdr runs a
//! command as an argv, no shell, and wants one command per action id):
//!
//! - `admiralty herdr toggle` (the `toggle` action): the overlay opens in this
//!   tab; pressed again it closes; open in another tab, it moves here.
//! - `admiralty herdr watch` (the `watch` overlay pane): the TUI with the
//!   overlay's defaults (`watch_setup`), sized to the overlay's box
//!   (`fill_overlay`).
//! - `admiralty herdr rezoom <overlay>` (the watch starts it as it quits):
//!   once the overlay is gone, the user's zoom back (`restore_zoom`).
//!
//! Herdr 0.9.3 starts an overlay's terminal at the size of the pane that had
//! the focus (half the tab when it is one of two, the whole tab when it was
//! zoomed), and draws the box over the whole tab: the TUI took the pane's
//! width or the tab's by chance. Zoom off and on again resizes it to the box.
//! Closing the overlay also drops the zoom of the pane under it; the toggle
//! notes that zoom (the `zoom` file) and it comes back once the overlay is
//! gone (docs/design/herdr-overlay-size.md).
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

/// The first `"key": true|false` field anywhere in a JSON reply.
pub fn flag(key: &str, json: &str) -> Option<bool> {
    let pat = format!("\"{key}\"");
    let mut from = 0;
    while let Some(at) = json[from..].find(&pat) {
        let rest = json[from + at + pat.len()..].trim_start();
        if let Some(v) = rest.strip_prefix(':').map(str::trim_start) {
            if v.starts_with("true") {
                return Some(true);
            }
            if v.starts_with("false") {
                return Some(false);
            }
        }
        from += at + pat.len();
    }
    None
}

/// The file beside the toggle's `pane_id` that names the pane whose zoom the
/// overlay took (absent: it was not zoomed).
pub fn zoom_file(state: &std::path::Path) -> PathBuf {
    state.with_file_name("zoom")
}

/// Note the zoom of the pane the overlay opens over (`pane`): its id in
/// `zoom_file`, or no file when it is not zoomed.
fn note_zoom(state: &std::path::Path, pane: Option<&str>, herdr: &dyn Fn(&[&str]) -> Option<String>) {
    let zoomed = pane.filter(|p| herdr(&["pane", "layout", "--pane", p]).and_then(|l| flag("zoomed", &l)) == Some(true));
    let _ = match zoomed {
        Some(p) => std::fs::write(zoom_file(state), format!("{p}\n")),
        None => std::fs::remove_file(zoom_file(state)),
    };
}

/// Once the overlay is gone, zoom the pane it covered back in, if it was
/// zoomed and still has its tab's focus (a jump to another pane of the tab
/// leaves the zoom off). The `zoom` file is used once.
pub fn restore_zoom(state: &std::path::Path, herdr: &dyn Fn(&[&str]) -> Option<String>) -> bool {
    let file = zoom_file(state);
    let Some(pane) = std::fs::read_to_string(&file).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else { return false };
    let _ = std::fs::remove_file(&file);
    let focused = herdr(&["pane", "layout", "--pane", &pane]).and_then(|l| field("focused_pane_id", &l));
    focused.as_deref() == Some(pane.as_str()) && herdr(&["pane", "zoom", &pane, "--on"]).is_some()
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
            restore_zoom(state, herdr);
            let here = get("HERDR_PLUGIN_CONTEXT_JSON").and_then(|c| field("tab_id", &c)).or_else(|| get("HERDR_TAB_ID"));
            if here.is_none() || field("tab_id", &info) == here {
                return Toggled::Closed;
            }
        }
    }
    // the pane the overlay opens over, and its zoom, before the overlay takes it
    let under = get("HERDR_PLUGIN_CONTEXT_JSON").and_then(|c| field("focused_pane_id", &c)).or_else(|| get("HERDR_PANE_ID"));
    note_zoom(state, under.as_deref(), herdr);
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

/// The Herdr CLI a plugin command runs (`HERDR_BIN_PATH`, which Herdr gives
/// every plugin command): its stdout, or None when it failed.
fn herdr_cli(vars: &HashMap<String, String>) -> impl Fn(&[&str]) -> Option<String> {
    let bin = vars.get("HERDR_BIN_PATH").filter(|v| !v.is_empty()).cloned().unwrap_or_else(|| "herdr".into());
    move |args: &[&str]| {
        let o = std::process::Command::new(&bin).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    }
}

/// The toggle's `pane_id` file, in the plugin's state dir.
fn state_path(vars: &HashMap<String, String>) -> PathBuf {
    vars.get("HERDR_PLUGIN_STATE_DIR").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("herdr-admiralty")).join("pane_id")
}

/// `admiralty herdr watch`, before the TUI starts: zoom its own pane off and
/// on again, so Herdr sizes its terminal to the overlay's box, whatever pane
/// had the focus (`HERDR_PANE_ID`, which Herdr gives a plugin pane).
pub fn fill_overlay_with(vars: &HashMap<String, String>, herdr: &dyn Fn(&[&str]) -> Option<String>) -> bool {
    let Some(me) = vars.get("HERDR_PANE_ID").filter(|v| !v.is_empty()) else { return false };
    herdr(&["pane", "zoom", me, "--off"]).is_some() && herdr(&["pane", "zoom", me, "--on"]).is_some()
}

pub fn fill_overlay(vars: &HashMap<String, String>) -> bool {
    fill_overlay_with(vars, &herdr_cli(vars))
}

/// `admiralty herdr watch`, as it quits: start `admiralty herdr rezoom` on
/// its own (its own process group: Herdr's close of the overlay does not
/// reach it), to give the zoom back once the overlay is gone.
pub fn rezoom_later(vars: &HashMap<String, String>) {
    let Some(me) = vars.get("HERDR_PANE_ID").filter(|v| !v.is_empty()) else { return };
    if !zoom_file(&state_path(vars)).is_file() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["herdr", "rezoom", me]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let _ = cmd.spawn();
}

/// `admiralty herdr rezoom <overlay>`: wait (up to 5 s) for the overlay to
/// go, then `restore_zoom`.
pub fn rezoom(vars: &HashMap<String, String>, overlay: &str) -> i32 {
    let herdr = herdr_cli(vars);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while herdr(&["pane", "get", overlay]).is_some() {
        if std::time::Instant::now() > end {
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    restore_zoom(&state_path(vars), &herdr);
    0
}

/// `admiralty herdr toggle`: the toggle against the running Herdr.
pub fn toggle(vars: &HashMap<String, String>) -> i32 {
    match toggle_with(vars, &state_path(vars), &herdr_cli(vars)) {
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
/// - `ADMIRALTY_INSET` 0,0: rows and columns at the bottom and right left
///   blank. `fill_overlay` sizes the terminal to the overlay's box, so none
///   hide under its border (before 0.7.9 it was 2,2: a terminal the whole
///   tab's size lost its last 2 rows and columns under the border);
/// - arguments from `ADMIRALTY_ARGS`, else `$HERDR_PLUGIN_CONFIG_DIR/args`,
///   split on white space (e.g. `--sandbox /tmp/aw-sb`).
///
/// The `ADMIRALTY_WATCH_*` names of nimitz 0.3.0 count when the new ones are unset.
pub fn watch_setup(vars: &HashMap<String, String>) -> (Vec<(String, String)>, Vec<String>) {
    let get = |k: &str| vars.get(k).cloned();
    let jump = get("ADMIRALTY_EXIT_AFTER_JUMP").or_else(|| get("ADMIRALTY_WATCH_EXIT_AFTER_JUMP")).unwrap_or_else(|| "1".into());
    let inset = get("ADMIRALTY_INSET").unwrap_or_else(|| "0,0".into());
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
        /// the zoomed pane, the focused one, and every zoom call
        zoomed: RefCell<Option<String>>,
        focused: RefCell<String>,
        zooms: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new() -> Fake {
            Fake { panes: RefCell::new(vec![]), next: RefCell::new(2), open_fails: false, tab: String::new(), zoomed: RefCell::new(None), focused: RefCell::new("w1:p1".into()), zooms: RefCell::new(vec![]) }
        }
        fn run(&self, a: &[&str]) -> Option<String> {
            match a {
                ["pane", "layout", "--pane", _] => Some(format!(r#"{{"result":{{"layout":{{"focused_pane_id":"{}","zoomed":{}}}}}}}"#, self.focused.borrow(), self.zoomed.borrow().is_some())),
                ["pane", "zoom", id, how] => {
                    self.zooms.borrow_mut().push(format!("{id} {how}"));
                    *self.zoomed.borrow_mut() = (*how == "--on").then(|| id.to_string());
                    Some("{}".into())
                }
                ["pane", "get", id] => self.panes.borrow().iter().find(|p| p.0 == *id).map(|p| format!(r#"{{"result":{{"pane":{{"pane_id":"{}","tab_id":"{}","label":"{}"}}}}}}"#, p.0, p.1, p.2)),
                ["plugin", "pane", "close", id] => {
                    self.panes.borrow_mut().retain(|p| p.0 != *id);
                    // Herdr drops the zoom with the overlay
                    *self.zoomed.borrow_mut() = None;
                    Some("{}".into())
                }
                ["plugin", "pane", "open", ..] if !self.open_fails => {
                    let id = format!("w1:p{}", self.next.borrow());
                    *self.next.borrow_mut() += 2;
                    self.panes.borrow_mut().push((id.clone(), self.tab.clone(), LABEL.into()));
                    *self.zoomed.borrow_mut() = Some(id.clone());
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
        let vars: HashMap<String, String> = [("HERDR_TAB_ID".to_string(), tab.to_string()), ("HERDR_PLUGIN_CONTEXT_JSON".to_string(), format!(r#"{{"tab_id":"{tab}","focused_pane_id":"w1:p1"}}"#))].into();
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

    /// The toggle notes the zoom of the pane it covers and gives it back when
    /// it closes the overlay; no zoom, nothing to give back; a pane that lost
    /// the focus (a jump) stays unzoomed.
    #[test]
    fn the_toggle_gives_back_the_zoom_the_overlay_took() {
        let td = tempfile::tempdir().unwrap();
        let state = td.path().join("pane_id");
        let mut f = Fake::new();
        *f.zoomed.borrow_mut() = Some("w1:p1".into());
        press(&mut f, &state, "w1:t1");
        assert!(zoom_file(&state).is_file(), "the zoom is noted");
        assert_eq!(press(&mut f, &state, "w1:t1"), Toggled::Closed);
        assert_eq!(f.zoomed.borrow().as_deref(), Some("w1:p1"), "and given back");
        assert!(!zoom_file(&state).exists(), "once");
        // not zoomed: nothing noted, nothing zoomed after
        *f.zoomed.borrow_mut() = None;
        press(&mut f, &state, "w1:t1");
        assert!(!zoom_file(&state).exists());
        press(&mut f, &state, "w1:t1");
        assert_eq!(*f.zoomed.borrow(), None);
        // zoomed, then a jump moved the focus: the zoom stays off
        *f.zoomed.borrow_mut() = Some("w1:p1".into());
        press(&mut f, &state, "w1:t1");
        *f.focused.borrow_mut() = "w1:p9".into();
        press(&mut f, &state, "w1:t1");
        assert_eq!(*f.zoomed.borrow(), None);
    }

    /// The watch zooms its own pane off and on (Herdr then sizes it to the
    /// overlay's box); outside a Herdr pane it does nothing.
    #[test]
    fn the_watch_fills_its_box_by_zooming_off_and_on() {
        let f = Fake::new();
        assert!(fill_overlay_with(&[("HERDR_PANE_ID".to_string(), "w1:p4".to_string())].into(), &|a| f.run(a)));
        assert_eq!(*f.zooms.borrow(), ["w1:p4 --off", "w1:p4 --on"]);
        assert!(!fill_overlay_with(&HashMap::new(), &|a| f.run(a)));
        assert_eq!(f.zooms.borrow().len(), 2);
    }

    #[test]
    fn a_flag_is_the_first_bool_of_its_key() {
        assert_eq!(flag("zoomed", r#"{"layout": {"zoomed" : true}}"#), Some(true));
        assert_eq!(flag("zoomed", r#"{"zoomed": "x", "a": {"zoomed":false}}"#), Some(false));
        assert_eq!(flag("zoomed", "{}"), None);
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
        assert_eq!(env, vec![("ADMIRALTY_EXIT_AFTER_JUMP".to_string(), "1".to_string()), ("ADMIRALTY_INSET".to_string(), "0,0".to_string())]);
        assert_eq!(args, ["--sandbox", "/tmp/aw-sb"]);
        // the env wins over the file; the 0.3.0 names still count
        let (env, args) = watch_setup(&[("HERDR_PLUGIN_CONFIG_DIR".to_string(), cfg), ("ADMIRALTY_ARGS".to_string(), String::new()), ("ADMIRALTY_WATCH_EXIT_AFTER_JUMP".to_string(), "0".to_string())].into());
        assert!(args.is_empty());
        assert_eq!(env[0].1, "0");
    }
}
