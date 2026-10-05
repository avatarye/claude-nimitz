//! nimitz-mux tests, custom harness (Cargo.toml: harness = false).
//!
//! The test binary doubles as the fake herdr: re-invoked as
//! `<exe> __fake-herdr <world-dir> <args...>` it dispatches to
//! `nimitz_mux::fake::fake_herdr`, so the Herdr adapter is exercised through a
//! real subprocess (a `herdr` wrapper script in the world dir).
//!
//! `NIMITZ_HERDR_E2E=1` adds an end-to-end run against a PRIVATE Herdr server
//! (temp XDG dirs, every `HERDR_*` unset); it never touches the user's server.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nimitz_mux::fake::{calls, default_world, write_world};
use nimitz_mux::herdr::Herdr;
use nimitz_mux::{LayoutPlan, Mux, MuxError, PaneRef, Place};
use serde_json::{json, Value};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("__fake-herdr") {
        let base = PathBuf::from(args.get(2).expect("__fake-herdr needs a world dir"));
        std::process::exit(nimitz_mux::fake::fake_herdr(&base, &args[3..]));
    }

    let tests: &[(&str, fn())] = &[
        ("fake: unknown command exits 2", fake_unknown_command),
        ("panes: parses agent list (session id, title, cwd, state)", panes_parse),
        ("panes: empty on a broken binary", panes_broken_bin),
        ("focus: workspace, tab, agent focus in order", focus_order),
        ("focus: partial ref skips missing levels", focus_partial),
        ("focus: unknown pane surfaces herdr's error", focus_unknown),
        ("type_text: prompts the pane", type_text_ok),
        ("type_text: blocked pane is rejected with herdr's error", type_text_blocked),
        ("type_text: unknown pane errors", type_text_unknown),
        ("open: new workspace runs the command in its root pane", open_workspace),
        ("open: tab in an existing workspace (loose label match)", open_tab_existing_ws),
        ("open: tab creates the workspace when missing, claims its root tab", open_tab_missing_ws),
        ("open: split next to a pane", open_split),
        ("open: --no-focus unless asked", open_no_focus),
        ("layout: builds workspaces and tabs, labelled, commands run", layout_fresh),
        ("layout: rerun is idempotent (reuses by label, reruns nothing)", layout_idempotent),
        ("layout: fills only the missing tab", layout_partial),
        ("layout: loose label variants match (skill normalisation)", layout_loose_labels),
        ("layout: a short tab name from before full names is reused and renamed", layout_short_tab_renamed),
        ("errors: workspace create failure propagates", error_workspace_create),
        ("errors: pane run failure propagates", error_pane_run),
    ];
    let mut fails = 0;
    for (name, f) in tests {
        match std::panic::catch_unwind(f) {
            Ok(()) => println!("[OK] {name}"),
            Err(_) => {
                println!("[ERROR] {name}");
                fails += 1;
            }
        }
    }
    if std::env::var("NIMITZ_HERDR_E2E").ok().as_deref() == Some("1") {
        match std::panic::catch_unwind(e2e_private_server) {
            Ok(()) => println!("[OK] e2e: open/focus/layout/type against a private Herdr server"),
            Err(_) => {
                println!("[ERROR] e2e: private Herdr server");
                fails += 1;
            }
        }
    } else {
        println!("[WARN] e2e skipped (set NIMITZ_HERDR_E2E=1 to run it against a private Herdr server)");
    }
    if fails > 0 {
        println!("[ERROR] {fails} test(s) failed");
        std::process::exit(1);
    }
    println!("[OK] all nimitz-mux tests passed");
}

// ---------- world + wrapper helpers ----------

struct World {
    dir: PathBuf,
}

impl World {
    /// A world dir with a `herdr` wrapper script that re-invokes this test
    /// binary as the fake herdr.
    fn new(world: Option<Value>) -> World {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nimitz-mux-world-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        write_world(&dir, world.as_ref()).expect("write world");
        let exe = std::env::current_exe().expect("current_exe");
        let script = dir.join("herdr");
        fs::write(
            &script,
            format!("#!/bin/sh\nexec \"{}\" __fake-herdr \"{}\" \"$@\"\n", exe.display(), dir.display()),
        )
        .expect("write wrapper");
        set_exec(&script);
        World { dir }
    }

    fn mux(&self) -> Herdr {
        Herdr { bin: self.dir.join("herdr").to_string_lossy().into_owned() }
    }

    fn world(&self) -> Value {
        serde_json::from_slice(&fs::read(self.dir.join("world.json")).expect("world.json")).expect("world json")
    }

    fn calls(&self) -> Vec<Vec<String>> {
        calls(&self.dir)
    }

    fn pane(&self, pane_id: &str) -> Value {
        self.world()["panes"]
            .as_array()
            .expect("panes")
            .iter()
            .find(|p| p["pane_id"] == pane_id)
            .unwrap_or_else(|| panic!("no pane {pane_id}"))
            .clone()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn set_exec(p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(p, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

fn pane_ref(pane: &str, tab: &str, ws: &str) -> PaneRef {
    PaneRef {
        pane_id: pane.into(),
        tab_id: Some(tab.to_string()).filter(|x| !x.is_empty()),
        workspace_id: Some(ws.to_string()).filter(|x| !x.is_empty()),
    }
}

fn cmdv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

// ---------- fake sanity ----------

fn fake_unknown_command() {
    let w = World::new(None);
    let out = Command::new(w.dir.join("herdr")).args(["bogus", "thing"]).output().expect("run fake");
    assert_eq!(out.status.code(), Some(2), "unknown commands must not look like herdr errors");
}

// ---------- panes ----------

fn panes_parse() {
    let w = World::new(None);
    let panes = w.mux().panes();
    assert_eq!(panes.len(), 2);
    let p = &panes[0];
    assert_eq!(p.pane, pane_ref("w1:p1", "w1:t1", "w1"));
    assert_eq!(p.session_id.as_deref(), Some("11111111-2222-3333-4444-555555555555"));
    assert_eq!(p.title, "[ADMIRALTY]");
    assert_eq!(p.cwd, "/fleet/admiralty");
    assert_eq!(p.state, "idle");
    assert_eq!(panes[1].state, "blocked");
}

fn panes_broken_bin() {
    let m = Herdr { bin: "/nonexistent/nimitz-no-such-herdr".into() };
    assert!(m.panes().is_empty());
}

// ---------- focus ----------

fn focus_order() {
    let w = World::new(None);
    w.mux().focus(&pane_ref("w1:p1", "w1:t1", "w1")).expect("focus");
    assert_eq!(
        w.calls(),
        vec![
            cmdv(&["workspace", "focus", "w1"]),
            cmdv(&["tab", "focus", "w1:t1"]),
            cmdv(&["agent", "focus", "w1:p1"]),
        ]
    );
}

fn focus_partial() {
    let w = World::new(None);
    w.mux().focus(&pane_ref("w1:p1", "", "")).expect("focus");
    assert_eq!(w.calls(), vec![cmdv(&["agent", "focus", "w1:p1"])]);
}

fn focus_unknown() {
    let w = World::new(None);
    let e = w.mux().focus(&pane_ref("w9:p9", "", "")).unwrap_err();
    assert!(e.to_string().contains("agent_not_found"), "{e}");
}

// ---------- type_text ----------

fn type_text_ok() {
    let w = World::new(None);
    w.mux().type_text(&pane_ref("w1:p1", "w1:t1", "w1"), "status please").expect("prompt");
    assert_eq!(w.calls(), vec![cmdv(&["agent", "prompt", "w1:p1", "status please"])]);
    assert_eq!(w.pane("w1:p1")["prompts"], json!(["status please"]));
}

fn type_text_blocked() {
    let w = World::new(None);
    let e = w.mux().type_text(&pane_ref("w1:p2", "w1:t1", "w1"), "hi").unwrap_err();
    let msg = e.to_string();
    assert!(msg.contains("agent_blocked") && msg.contains("blocked"), "{msg}");
    assert!(w.pane("w1:p2").get("prompts").is_none(), "nothing may reach a blocked pane");
}

fn type_text_unknown() {
    let w = World::new(None);
    let e = w.mux().type_text(&pane_ref("w9:p9", "", ""), "hi").unwrap_err();
    assert!(e.to_string().contains("agent_not_found"), "{e}");
}

// ---------- open ----------

fn open_workspace() {
    let w = World::new(None);
    let r = w
        .mux()
        .open(&cmdv(&["sh", "-c", "sleep 30"]), Path::new("/work/x"), Place::Workspace { label: "[NEW]-FLEET".into() })
        .expect("open");
    assert_eq!(r, pane_ref("w2:p1", "w2:t1", "w2"));
    assert_eq!(w.pane("w2:p1")["ran"], json!(["sh -c 'sleep 30'"]));
    assert_eq!(w.pane("w2:p1")["cwd"], "/work/x");
    let ws = w.world()["workspaces"].as_array().expect("ws").last().cloned().expect("new ws");
    assert_eq!(ws["label"], "[NEW]-FLEET");
}

fn open_tab_existing_ws() {
    let w = World::new(None); // world has workspace w1 labelled ADMIRALTY
    let r = w
        .mux()
        .open(
            &cmdv(&["yazi"]),
            Path::new("/work/x"),
            Place::Tab { workspace: "[ADMIRALTY]".into(), label: "Yazi".into() },
        )
        .expect("open");
    assert_eq!(r, pane_ref("w1:p3", "w1:t2", "w1")); // w1 already has 2 panes, 1 tab
    assert!(!w.calls().iter().any(|c| c[..2] == ["workspace".to_string(), "create".to_string()]),
        "must reuse the ADMIRALTY workspace: {:?}", w.calls());
    assert_eq!(w.pane("w1:p3")["ran"], json!(["yazi"]));
    let tab = w.world()["tabs"].as_array().expect("tabs").iter().find(|t| t["tab_id"] == "w1:t2").cloned().expect("tab");
    assert_eq!(tab["label"], "Yazi");
}

fn open_tab_missing_ws() {
    let w = World::new(None);
    let r = w
        .mux()
        .open(
            &cmdv(&["ccl"]),
            Path::new("/fleet/bolt"),
            Place::Tab { workspace: "[BOLT]-FLEET".into(), label: "[BOLT]-LEAD".into() },
        )
        .expect("open");
    assert_eq!(r, pane_ref("w2:p1", "w2:t1", "w2"));
    // the new workspace's root tab is claimed (renamed), not duplicated
    let tabs: Vec<Value> =
        w.world()["tabs"].as_array().expect("tabs").iter().filter(|t| t["workspace_id"] == "w2").cloned().collect();
    assert_eq!(tabs.len(), 1, "{tabs:?}");
    assert_eq!(tabs[0]["label"], "[BOLT]-LEAD");
    assert_eq!(w.pane("w2:p1")["ran"], json!(["ccl"]));
}

fn open_split() {
    let w = World::new(None);
    let next_to = pane_ref("w1:p1", "w1:t1", "w1");
    let r = w
        .mux()
        .open(&cmdv(&["htop"]), Path::new("/work/x"), Place::Split { next_to })
        .expect("open");
    assert_eq!(r, pane_ref("w1:p3", "w1:t1", "w1"));
    assert_eq!(w.pane("w1:p3")["ran"], json!(["htop"]));
}

fn open_no_focus() {
    let w = World::new(None);
    w.mux()
        .open(&cmdv(&["x"]), Path::new("/w"), Place::Workspace { label: "A".into() })
        .expect("open ws");
    w.mux()
        .open(&cmdv(&["x"]), Path::new("/w"), Place::Tab { workspace: "A".into(), label: "T2".into() })
        .expect("open tab");
    for c in w.calls() {
        if c.len() >= 2 && c[1] == "create" {
            assert!(c.contains(&"--no-focus".to_string()), "create without --no-focus: {c:?}");
        }
    }
}

// ---------- layout ----------

fn plan() -> LayoutPlan {
    LayoutPlan {
        workspaces: vec![
            (
                "[ARGOS]-FLEET".into(),
                vec![
                    ("[ARGOS]-LEAD".into(), cmdv(&["ccl"]), PathBuf::from("/fleet/argos")),
                    ("Yazi".into(), cmdv(&["yazi"]), PathBuf::from("/fleet/argos")),
                ],
            ),
            ("[BOLT]-FLEET".into(), vec![("[BOLT]-LEAD".into(), cmdv(&["ccl"]), PathBuf::from("/fleet/bolt"))]),
        ],
    }
}

fn layout_fresh() {
    let w = World::new(Some(json!({"next_ws": 2})));
    let refs = w.mux().layout(&plan()).expect("layout");
    assert_eq!(
        refs,
        vec![pane_ref("w2:p1", "w2:t1", "w2"), pane_ref("w2:p2", "w2:t2", "w2"), pane_ref("w3:p1", "w3:t1", "w3")]
    );
    let world = w.world();
    let labels: Vec<(String, String)> = world["workspaces"]
        .as_array()
        .expect("ws")
        .iter()
        .map(|x| (x["workspace_id"].as_str().unwrap_or("").into(), x["label"].as_str().unwrap_or("").into()))
        .collect();
    assert_eq!(labels, vec![("w2".into(), "[ARGOS]-FLEET".into()), ("w3".into(), "[BOLT]-FLEET".into())]);
    let tabs: Vec<String> =
        world["tabs"].as_array().expect("tabs").iter().map(|t| t["label"].as_str().unwrap_or("").into()).collect();
    assert_eq!(tabs, vec!["[ARGOS]-LEAD", "Yazi", "[BOLT]-LEAD"]);
    assert_eq!(w.pane("w2:p1")["ran"], json!(["ccl"]));
    assert_eq!(w.pane("w2:p2")["ran"], json!(["yazi"]));
    assert_eq!(w.pane("w3:p1")["ran"], json!(["ccl"]));
}

fn layout_idempotent() {
    let w = World::new(Some(json!({"next_ws": 2})));
    w.mux().layout(&plan()).expect("first layout");
    let before = w.world();
    let refs = w.mux().layout(&plan()).expect("second layout");
    assert_eq!(refs.len(), 3);
    let after = w.world();
    assert_eq!(before["workspaces"], after["workspaces"], "no duplicate workspaces");
    assert_eq!(before["tabs"], after["tabs"], "no duplicate tabs");
    assert_eq!(w.pane("w2:p1")["ran"], json!(["ccl"]), "a reused tab's command must not run again");
}

fn layout_partial() {
    let w = World::new(Some(json!({"next_ws": 2})));
    w.mux().layout(&plan()).expect("first layout");
    let mut p = plan();
    p.workspaces[0].1.push(("Logs".into(), cmdv(&["tail", "-f", "x.log"]), PathBuf::from("/fleet/argos")));
    w.mux().layout(&p).expect("second layout");
    let tabs: Vec<String> = w.world()["tabs"]
        .as_array()
        .expect("tabs")
        .iter()
        .filter(|t| t["workspace_id"] == "w2")
        .map(|t| t["label"].as_str().unwrap_or("").into())
        .collect();
    assert_eq!(tabs, vec!["[ARGOS]-LEAD", "Yazi", "Logs"]);
    assert_eq!(w.pane("w2:p3")["ran"], json!(["tail -f x.log"]));
}

fn layout_loose_labels() {
    // an old hand-made layout: lowercase workspace, no brackets, upcased tab
    let w = World::new(Some(json!({
        "next_ws": 2,
        "workspaces": [{"workspace_id": "w1", "label": "argos", "focused": false, "active_tab_id": "w1:t1"}],
        "tabs": [
            {"tab_id": "w1:t1", "workspace_id": "w1", "label": "ARGOS-LEAD", "focused": true},
            {"tab_id": "w1:t2", "workspace_id": "w1", "label": "YAZI", "focused": false}
        ],
        "panes": [
            {"pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1", "cwd": "/fleet/argos"},
            {"pane_id": "w1:p2", "tab_id": "w1:t2", "workspace_id": "w1", "cwd": "/fleet/argos"}
        ]
    })));
    let mut p = plan();
    p.workspaces.truncate(1); // just [ARGOS]-FLEET with [ARGOS]-LEAD + Yazi
    let refs = w.mux().layout(&p).expect("layout");
    assert_eq!(refs, vec![pane_ref("w1:p1", "w1:t1", "w1"), pane_ref("w1:p2", "w1:t2", "w1")]);
    assert_eq!(w.world()["workspaces"].as_array().expect("ws").len(), 1, "argos == [ARGOS]-FLEET");
    assert!(w.pane("w1:p1").get("ran").is_none(), "reused panes keep their processes");
}

fn layout_short_tab_renamed() {
    // a squadron workspace laid out by nimitz 0.3.0: its tab is plain COMMODORE
    let w = World::new(Some(json!({
        "next_ws": 2,
        "workspaces": [{"workspace_id": "w1", "label": "HOME.GITHUB", "focused": false, "active_tab_id": "w1:t1"}],
        "tabs": [{"tab_id": "w1:t1", "workspace_id": "w1", "label": "COMMODORE", "focused": true}],
        "panes": [{"pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1", "cwd": "/fleet/github"}]
    })));
    let p = LayoutPlan {
        workspaces: vec![("HOME.GITHUB".into(), vec![("HOME.GITHUB.COMMODORE".into(), cmdv(&["claude", "-c"]), PathBuf::from("/fleet/github"))])],
    };
    let refs = w.mux().layout(&p).expect("layout");
    assert_eq!(refs, vec![pane_ref("w1:p1", "w1:t1", "w1")]);
    let tabs = w.world()["tabs"].as_array().expect("tabs").clone();
    assert_eq!(tabs.len(), 1, "no second tab: {tabs:?}");
    assert_eq!(tabs[0]["label"], "HOME.GITHUB.COMMODORE", "renamed to its full name");
    assert!(w.pane("w1:p1").get("ran").is_none(), "a reused pane keeps its process");
}

// ---------- error paths ----------

fn error_workspace_create() {
    let mut world = default_world();
    world["fail"] = json!("workspace create");
    let w = World::new(Some(world));
    let e = w
        .mux()
        .open(&cmdv(&["x"]), Path::new("/w"), Place::Workspace { label: "NEW".into() })
        .unwrap_err();
    assert!(matches!(e, MuxError::Failed(_)));
    assert!(e.to_string().contains("injected_failure"), "{e}");
}

fn error_pane_run() {
    let mut world = default_world();
    world["fail"] = json!("pane run");
    let w = World::new(Some(world));
    let e = w
        .mux()
        .open(&cmdv(&["x"]), Path::new("/w"), Place::Workspace { label: "NEW".into() })
        .unwrap_err();
    assert!(e.to_string().contains("injected_failure"), "{e}");
}

// ---------- opt-in e2e against a private Herdr server ----------

/// Never touches the user's Herdr: a wrapper script unsets every `HERDR_*`
/// and points XDG_CONFIG_HOME / XDG_STATE_HOME at a temp dir, so client,
/// server and socket all live there. Panes only run `sh -c 'sleep 30'`.
fn e2e_private_server() {
    let tmp = std::env::temp_dir().join(format!("nimitz-mux-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("config/herdr")).expect("mkdir");
    fs::write(tmp.join("config/herdr/config.toml"), "onboarding = false\n").expect("config");
    let wrapper = tmp.join("herdr-iso");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nfor v in $(env | sed -n 's/^\\(HERDR_[A-Za-z_0-9]*\\)=.*/\\1/p'); do unset \"$v\"; done\n\
             export XDG_CONFIG_HOME=\"{t}/config\" XDG_STATE_HOME=\"{t}/state\"\nexec herdr \"$@\"\n",
            t = tmp.display()
        ),
    )
    .expect("wrapper");
    set_exec(&wrapper);
    let bin = wrapper.to_string_lossy().into_owned();
    let sock = tmp.join("config/herdr/herdr.sock");

    let mut server = Command::new(&wrapper).arg("server").spawn().expect("start private herdr server");
    let cleanup = |server: &mut std::process::Child| {
        // stop only a server whose socket is in OUR temp dir
        if let Ok(out) = Command::new(&wrapper).arg("status").output() {
            if String::from_utf8_lossy(&out.stdout).contains(&*sock.to_string_lossy()) {
                let _ = Command::new(&wrapper).args(["server", "stop"]).output();
            }
        }
        let _ = server.kill();
        let _ = server.wait();
        let _ = fs::remove_dir_all(&tmp);
    };
    let r = std::panic::catch_unwind(|| e2e_body(&bin, &sock, &tmp));
    cleanup(&mut server);
    if let Err(p) = r {
        std::panic::resume_unwind(p);
    }
}

fn e2e_body(bin: &str, sock: &Path, tmp: &Path) {
    // wait for the private server
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let out = Command::new(bin).arg("status").output().expect("status");
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if text.contains(&*sock.to_string_lossy()) && text.contains("running") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "private herdr server did not come up: {text}");
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    let m = Herdr { bin: bin.to_string() };
    let dummy = cmdv(&["sh", "-c", "sleep 30"]);
    let api = |args: &[&str]| -> Value {
        let out = Command::new(bin).args(args).output().expect("herdr");
        serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
    };

    // open: a new workspace, command running in its root pane
    let cwd = tmp.to_path_buf();
    let r1 = m.open(&dummy, &cwd, Place::Workspace { label: "[NIMITZ-E2E]-FLEET".into() }).expect("open workspace");
    let ws_list = api(&["workspace", "list"]);
    let labels: Vec<String> = ws_list
        .pointer("/result/workspaces")
        .and_then(Value::as_array)
        .expect("workspaces")
        .iter()
        .map(|x| x["label"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(labels.contains(&"[NIMITZ-E2E]-FLEET".to_string()), "{labels:?}");

    // the command really ran with its arguments intact (herdr types one shell line)
    let marked = cmdv(&["sh", "-c", "echo NIMITZ E2E MARK; sleep 30"]);
    let r0 = m.open(&marked, &cwd, Place::Split { next_to: r1.clone() }).expect("open split");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let out = Command::new(bin)
            .args(["pane", "read", &r0.pane_id, "--source", "recent-unwrapped", "--lines", "20"])
            .output()
            .expect("pane read");
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if text.lines().any(|l| l.trim() == "NIMITZ E2E MARK") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "marker never printed; pane shows: {text}");
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    // open: a tab into that workspace by loose label
    let r2 = m
        .open(&dummy, &cwd, Place::Tab { workspace: "NIMITZ-E2E".into(), label: "EXTRA".into() })
        .expect("open tab");
    assert_eq!(r2.workspace_id, r1.workspace_id, "loose match must reuse the workspace");

    // layout, twice: second run must not duplicate
    let plan = LayoutPlan {
        workspaces: vec![(
            "[NIMITZ-E2E2]-FLEET".into(),
            vec![("LEAD".into(), dummy.clone(), cwd.clone()), ("SIDE".into(), dummy.clone(), cwd.clone())],
        )],
    };
    let refs = m.layout(&plan).expect("layout");
    assert_eq!(refs.len(), 2);
    let count = |v: &Value| v.pointer("/result/workspaces").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
    let before = count(&api(&["workspace", "list"]));
    let refs2 = m.layout(&plan).expect("layout rerun");
    assert_eq!(refs2.len(), 2);
    assert_eq!(count(&api(&["workspace", "list"])), before, "layout rerun must not add workspaces");

    // focus: workspace and tab focus land; the agent-focus step may fail on a
    // dummy pane (sleep is not an agent) — then the error must say so
    match m.focus(&refs[0]) {
        Ok(()) => {}
        Err(e) => assert!(e.to_string().contains(&refs[0].pane_id) || e.to_string().contains("agent"), "{e}"),
    }
    let ws_list = api(&["workspace", "list"]);
    let focused: Vec<&str> = ws_list
        .pointer("/result/workspaces")
        .and_then(Value::as_array)
        .expect("workspaces")
        .iter()
        .filter(|x| x["focused"] == true)
        .map(|x| x["label"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(focused, vec!["[NIMITZ-E2E2]-FLEET"], "focus must land on the layout workspace");

    // type_text: a sleep pane hosts no agent; herdr must reject it, and the
    // adapter must surface that error (never guess)
    let e = m.type_text(&refs[1], "hello").unwrap_err();
    assert!(matches!(e, MuxError::Failed(_)), "{e}");
}
