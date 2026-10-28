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
use nimitz_mux::herdr::{self, Herdr};
use nimitz_mux::{AttachError, Attached, LayoutPlan, Mux, MuxError, PaneRef, Place, TabState};
use serde_json::{json, Value};

fn main() {
    // (on Windows the fake herdr is this binary under another name: its prefix comes from the shim)
    let mut args: Vec<String> = std::env::args().take(1).collect();
    args.extend(nimitz_core::os::shim_args());
    args.extend(std::env::args().skip(1));
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
        ("focus: a new pane Herdr names an agent a moment later is focused (#11)", focus_waits_for_the_agent),
        ("focus: a pane never named an agent, its tab in front: Unfocused (#11)", focus_never_an_agent),
        ("focus: a pane closed since it was read is Failed, never Unfocused", focus_a_closed_pane_is_failed),
        ("focus: another agent focus error stays Failed, not asked again", focus_other_error),
        ("attach: a claimed pane not yet an agent is attached all the same (#11)", attach_tolerates_unfocused),
        ("type_text: prompts the pane", type_text_ok),
        ("type_text: spaces as typed, controls gone", type_text_keeps_spaces),
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
        ("attach: a pane showing the session is focused, nothing opens", reuse_by_session),
        ("attach: the tab named for it with a shell pane is claimed; then shown", reuse_by_tab_name),
        ("attach: a taken tab (yazi, another agent, the agent view) is never typed into", taken_tab_is_never_typed_into),
        ("attach: a Yazi split is not the tab's pane; the shell pane is", a_split_running_another_program_is_not_the_attach_pane),
        ("attach: nothing there yet opens a new tab", reuse_none_opens),
        ("layout: reruns a command only into a pane that is a shell again", layout_reruns_only_into_a_shell_pane),
        ("fixtures: process-info (herdr 0.9.3) nests the list under process_info; no list is an error", fixture_process_info),
        ("fixtures: the snapshot's workspaces, tabs, panes and agents parse like the list replies", fixture_snapshot_lists),
        ("fixtures: create replies without root_pane resolve the pane through pane list", fixture_create_replies),
        ("shape: the snapshot fixture and the fake's api snapshot parse alike, field for field", fixture_snapshot_shape),
        ("shape: the adapter reads every pane's foreground; rename_tab renames; exec runs the line", shape_through_the_adapter),
        ("panes: an unbacked claim is ignored; the pane shows what runs in it (the attach, inferred)", an_unbacked_claim_shows_what_runs_in_the_pane),
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

/// `focus`'s waits in tests: as many asks as `herdr::FOCUS_RETRY_MS`, none of its 2.5 s.
const QUICK_RETRY: [u64; herdr::FOCUS_RETRY_MS.len()] = [1; herdr::FOCUS_RETRY_MS.len()];

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
        nimitz_core::os::write_shim(&dir.join("herdr"), &exe, &["__fake-herdr".into(), dir.to_string_lossy().into_owned()]).expect("write wrapper");
        World { dir }
    }

    fn mux(&self) -> Herdr {
        Herdr { bin: self.dir.join(nimitz_core::os::exe_name("herdr")).to_string_lossy().into_owned(), session: None, socket: None, claude_home: "/nonexistent/nimitz-claude-home".into(), focus_retry_ms: QUICK_RETRY.to_vec() }
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

#[cfg_attr(not(unix), allow(unused_variables))]
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
    let out = Command::new(w.dir.join(nimitz_core::os::exe_name("herdr"))).args(["bogus", "thing"]).output().expect("run fake");
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
    let m = Herdr { bin: "/nonexistent/nimitz-no-such-herdr".into(), session: None, socket: None, claude_home: "/nonexistent/nimitz-claude-home".into(), focus_retry_ms: herdr::FOCUS_RETRY_MS.to_vec() };
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

/// A world whose `pane run` leaves the pane no agent for `n` `agent focus` asks.
fn lagging_world(n: u64) -> World {
    let mut world = default_world();
    world["agent_after"] = json!(n);
    World::new(Some(world))
}

fn agent_focus_calls(w: &World) -> usize {
    w.calls().iter().filter(|c| c.starts_with(&cmdv(&["agent", "focus"]))).count()
}

fn focus_waits_for_the_agent() {
    let w = lagging_world(2);
    let p = w.mux().open(&cmdv(&["claude", "attach", "cca8f78b"]), Path::new("/x"), Place::Tab { workspace: "MAIN".into(), label: "MAIN.GX.BROW".into() }).expect("open");
    w.mux().focus(&p).expect("focused on the third ask");
    assert_eq!(agent_focus_calls(&w), 3);
}

fn focus_never_an_agent() {
    let w = lagging_world(1000);
    let p = w.mux().open(&cmdv(&["claude", "attach", "cca8f78b"]), Path::new("/x"), Place::Tab { workspace: "MAIN".into(), label: "MAIN.GX.BROW".into() }).expect("open");
    let e = w.mux().focus(&p).unwrap_err();
    assert!(matches!(&e, MuxError::Unfocused(m) if m.contains("agent_not_found")), "{e:?}");
    assert_eq!(agent_focus_calls(&w), 1 + herdr::FOCUS_RETRY_MS.len());
    // focus_now asks once
    let e = w.mux().focus_now(&p).unwrap_err();
    assert!(matches!(&e, MuxError::Unfocused(_)), "{e:?}");
    assert_eq!(agent_focus_calls(&w), 2 + herdr::FOCUS_RETRY_MS.len());
    // the workspace and tab were focused first: the pane is in front
    let c = w.calls();
    assert!(c.contains(&cmdv(&["tab", "focus", p.tab_id.as_deref().unwrap()])), "{c:?}");
}

fn focus_a_closed_pane_is_failed() {
    // a split closed since it was read: its tab is there, the pane is not
    let w = World::new(None);
    let e = w.mux().focus_now(&pane_ref("w1:p9", "w1:t1", "w1")).unwrap_err();
    assert!(matches!(&e, MuxError::Failed(m) if m.contains("is gone") && m.contains("pane_not_found")), "{e:?}");
}

fn focus_other_error() {
    let mut world = default_world();
    world["fail"] = json!("agent focus");
    let w = World::new(Some(world));
    let e = w.mux().focus(&pane_ref("w1:p1", "w1:t1", "w1")).unwrap_err();
    assert!(matches!(&e, MuxError::Failed(m) if m.contains("injected_failure")), "{e:?}");
    assert_eq!(agent_focus_calls(&w), 1, "only agent_not_found is asked again");
}

fn attach_tolerates_unfocused() {
    let w = tab_world(None);
    let mut world = w.world();
    world["agent_after"] = json!(1000);
    fs::write(w.dir.join("world.json"), serde_json::to_vec_pretty(&world).unwrap()).unwrap();
    let place = Place::Tab { workspace: "HOME.GITHUB".into(), label: "HOME.GITHUB.NIMITZ_DEV".into() };
    let a = nimitz_mux::attach(&w.mux(), &cmdv(&["claude", "attach", "9aacf927"]), Path::new("/dev"), place, None).expect("attached, if not focused");
    assert_eq!(a, Attached::Claimed(pane_ref("w1:p2", "w1:t2", "w1")));
}

// ---------- type_text ----------

fn type_text_ok() {
    let w = World::new(None);
    w.mux().type_text(&pane_ref("w1:p1", "w1:t1", "w1"), "status please").expect("prompt");
    assert_eq!(w.calls(), vec![cmdv(&["agent", "prompt", "w1:p1", "status please"])]);
    assert_eq!(w.pane("w1:p1")["prompts"], json!(["status please"]));
}

fn type_text_keeps_spaces() {
    let w = World::new(None);
    w.mux().type_text(&pane_ref("w1:p1", "w1:t1", "w1"), "a  b\u{1b}[31m ").expect("prompt");
    assert_eq!(w.calls(), vec![cmdv(&["agent", "prompt", "w1:p1", "a  b "])]);
    assert!(w.mux().type_text(&pane_ref("w1:p1", "w1:t1", "w1"), "   ").is_err(), "only spaces: nothing to type");
    // a text that starts with `-` is never read as an option of `agent prompt`
    let w = World::new(None);
    w.mux().type_text(&pane_ref("w1:p1", "w1:t1", "w1"), "--wait now").expect("prompt");
    assert_eq!(w.calls(), vec![cmdv(&["agent", "prompt", "w1:p1", " --wait now"])]);
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
            {"pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1", "cwd": "/fleet/argos", "foreground": ["ccl"]},
            {"pane_id": "w1:p2", "tab_id": "w1:t2", "workspace_id": "w1", "cwd": "/fleet/argos", "foreground": ["yazi"]}
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
        "panes": [{"pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1", "cwd": "/fleet/github", "foreground": ["claude", "attach", "9aacf927"]}]
    })));
    let p = LayoutPlan {
        workspaces: vec![("HOME.GITHUB".into(), vec![("HOME.GITHUB.COMMODORE".into(), cmdv(&["claude", "attach", "9aacf927"]), PathBuf::from("/fleet/github"))])],
    };
    let refs = w.mux().layout(&p).expect("layout");
    assert_eq!(refs, vec![pane_ref("w1:p1", "w1:t1", "w1")]);
    let tabs = w.world()["tabs"].as_array().expect("tabs").clone();
    assert_eq!(tabs.len(), 1, "no second tab: {tabs:?}");
    assert_eq!(tabs[0]["label"], "HOME.GITHUB.COMMODORE", "renamed to its full name");
    assert!(w.pane("w1:p1").get("ran").is_none(), "a reused pane keeps its process");
}

fn opened_tabs(w: &World) -> usize {
    w.world()["tabs"].as_array().expect("tabs").len()
}

fn reuse_by_session() {
    let w = World::new(None);
    let place = Place::Tab { workspace: "MAIN.ARGOS".into(), label: "MAIN.ARGOS.LEAD".into() };
    let before = opened_tabs(&w);
    // pane w1:p2 shows session 6666...: that is the one
    let a = nimitz_mux::attach(&w.mux(), &cmdv(&["claude", "attach", "66666666"]), Path::new("/w"), place, Some("66666666-7777-8888-9999-000000000000")).expect("attach");
    assert_eq!(a, Attached::Focused(pane_ref("w1:p2", "w1:t1", "w1")));
    assert_eq!(opened_tabs(&w), before, "no new tab");
    assert!(w.calls().iter().any(|c| c[..2] == ["agent".to_string(), "focus".to_string()] && c[2] == "w1:p2"), "{:?}", w.calls());
    assert!(!w.calls().iter().any(|c| c[..2] == ["pane".to_string(), "run".to_string()]), "nothing typed anywhere: {:?}", w.calls());
}

/// A world with workspace HOME.GITHUB and the tab HOME.GITHUB.NIMITZ_DEV an
/// earlier `nimitz open` left; its pane runs `foreground` (none: a shell).
fn tab_world(foreground: Option<Vec<&str>>) -> World {
    let mut pane = json!({"pane_id": "w1:p2", "tab_id": "w1:t2", "workspace_id": "w1", "cwd": "/dev"});
    if let Some(fg) = foreground {
        pane["foreground"] = json!(fg);
    }
    World::new(Some(json!({
        "next_ws": 2,
        "workspaces": [{"workspace_id": "w1", "label": "HOME.GITHUB", "focused": false, "active_tab_id": "w1:t1"}],
        "tabs": [{"tab_id": "w1:t1", "workspace_id": "w1", "label": "HOME.GITHUB.COMMODORE", "focused": true},
                 {"tab_id": "w1:t2", "workspace_id": "w1", "label": "HOME.GITHUB.NIMITZ_DEV", "focused": false}],
        "panes": [{"pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1", "cwd": "/sq"}, pane]
    })))
}

fn reuse_by_tab_name() {
    // the tab is there and its pane is a plain shell (Herdr restored it, or the
    // attach client ended): the attach runs there, nothing opens
    let w = tab_world(None);
    let place = Place::Tab { workspace: "HOME.GITHUB".into(), label: "HOME.GITHUB.NIMITZ_DEV".into() };
    let cmd = cmdv(&["claude", "attach", "9aacf927"]);
    assert_eq!(nimitz_mux::tab_state(&w.mux(), &cmd, &place, Some("not-in-any-pane")).expect("state"), TabState::Shell(pane_ref("w1:p2", "w1:t2", "w1")));
    let a = nimitz_mux::attach(&w.mux(), &cmd, Path::new("/dev"), place.clone(), Some("not-in-any-pane")).expect("attach");
    assert_eq!(a, Attached::Claimed(pane_ref("w1:p2", "w1:t2", "w1")));
    assert_eq!(opened_tabs(&w), 2, "no new tab");
    assert_eq!(w.pane("w1:p2")["ran"], json!(["claude attach 9aacf927"]));
    // again: the pane runs that very attach now: shown, nothing typed again
    assert_eq!(nimitz_mux::tab_state(&w.mux(), &cmd, &place, Some("not-in-any-pane")).expect("state"), TabState::Shown(pane_ref("w1:p2", "w1:t2", "w1")));
    let a = nimitz_mux::attach(&w.mux(), &cmd, Path::new("/dev"), place, Some("not-in-any-pane")).expect("attach");
    assert_eq!(a, Attached::Focused(pane_ref("w1:p2", "w1:t2", "w1")));
    assert_eq!(w.pane("w1:p2")["ran"], json!(["claude attach 9aacf927"]), "ran once");
}

fn taken_tab_is_never_typed_into() {
    // R-14 / R-24 / R-33: the tab's pane runs something else (yazi, another
    // agent, the agent view): tab.taken, no pane run, no new tab
    for fg in [vec!["yazi"], vec!["claude", "attach", "deadbeef"], vec!["claude"], vec!["/usr/bin/python3", "x.py"]] {
        let w = tab_world(Some(fg.clone()));
        let place = Place::Tab { workspace: "HOME.GITHUB".into(), label: "HOME.GITHUB.NIMITZ_DEV".into() };
        let cmd = cmdv(&["claude", "attach", "9aacf927"]);
        let state = nimitz_mux::tab_state(&w.mux(), &cmd, &place, Some("not-in-any-pane")).expect("state");
        assert_eq!(state, TabState::Taken { pane: pane_ref("w1:p2", "w1:t2", "w1"), running: fg.join(" ") }, "{fg:?}");
        let e = nimitz_mux::attach(&w.mux(), &cmd, Path::new("/dev"), place, Some("not-in-any-pane")).expect_err("taken");
        assert!(matches!(e, AttachError::Taken { .. }), "{e:?}");
        assert_eq!(opened_tabs(&w), 2, "no new tab for {fg:?}");
        assert!(w.pane("w1:p2").get("ran").is_none(), "nothing typed for {fg:?}");
    }
    // a shell by any of its names is a shell
    for sh in ["zsh", "/bin/bash", "-zsh", "fish"] {
        assert!(nimitz_mux::is_shell(&cmdv(&[sh])), "{sh}");
    }
    assert!(nimitz_mux::is_shell(&[]));
}

fn a_split_running_another_program_is_not_the_attach_pane() {
    // the tab holds a Yazi split (first by id) and a shell pane: the shell one is the tab's
    let w = tab_world(Some(vec!["yazi"]));
    let mut world = w.world();
    world["panes"].as_array_mut().unwrap().push(json!({"pane_id": "w1:p3", "tab_id": "w1:t2", "workspace_id": "w1", "cwd": "/dev"}));
    fs::write(w.dir.join("world.json"), serde_json::to_vec_pretty(&world).unwrap()).unwrap();
    let place = Place::Tab { workspace: "HOME.GITHUB".into(), label: "HOME.GITHUB.NIMITZ_DEV".into() };
    let cmd = cmdv(&["claude", "attach", "9aacf927"]);
    assert_eq!(w.mux().tab(&place), Some(pane_ref("w1:p3", "w1:t2", "w1")), "the shell pane, never the yazi split");
    let a = nimitz_mux::attach(&w.mux(), &cmd, Path::new("/dev"), place, None).expect("attach");
    assert_eq!(a, Attached::Claimed(pane_ref("w1:p3", "w1:t2", "w1")));
    assert!(w.pane("w1:p2").get("ran").is_none(), "yazi's pane untouched");
}

fn reuse_none_opens() {
    let w = World::new(None);
    let place = Place::Tab { workspace: "ADMIRALTY".into(), label: "MAIN.ARGOS.CLEAN".into() };
    let before = opened_tabs(&w);
    let cmd = cmdv(&["claude", "attach", "2287e6e4"]);
    assert_eq!(nimitz_mux::tab_state(&w.mux(), &cmd, &place, Some("no-such-session")).expect("state"), TabState::Missing);
    let a = nimitz_mux::attach(&w.mux(), &cmd, Path::new("/clean"), place.clone(), Some("no-such-session")).expect("open");
    let Attached::Opened(p) = a else { panic!("{a:?}") };
    assert_eq!(opened_tabs(&w), before + 1, "a new tab");
    assert_eq!(w.pane(&p.pane_id)["ran"], json!(["claude attach 2287e6e4"]));
    // and a second call finds that tab, running that attach: focused, nothing more
    let again = nimitz_mux::attach(&w.mux(), &cmd, Path::new("/clean"), place, Some("no-such-session")).expect("again");
    assert_eq!(again, Attached::Focused(p));
    assert_eq!(opened_tabs(&w), before + 1);
}

fn layout_reruns_only_into_a_shell_pane() {
    // a restored Herdr: the tabs are there, their panes plain shells; the
    // layout runs the commands again there (R-03), but never into a pane
    // that runs something (the first run's `ccl`)
    let w = World::new(Some(json!({"next_ws": 2})));
    w.mux().layout(&plan()).expect("first layout");
    assert_eq!(w.pane("w2:p1")["ran"], json!(["ccl"]));
    let tabs_before = opened_tabs(&w);
    let mut world = w.world();
    for p in world["panes"].as_array_mut().unwrap().iter_mut().filter(|p| p["pane_id"] == "w2:p1") {
        p["foreground"] = json!(["zsh"]); // the attach client ended: a shell again
    }
    fs::write(w.dir.join("world.json"), serde_json::to_vec_pretty(&world).unwrap()).unwrap();
    w.mux().layout(&plan()).expect("second layout");
    assert_eq!(w.pane("w2:p1")["ran"], json!(["ccl", "ccl"]), "ran again in the shell pane");
    assert_eq!(w.pane("w2:p2")["ran"], json!(["yazi"]), "yazi still runs there: left alone");
    assert_eq!(opened_tabs(&w), tabs_before, "no duplicate tabs");
}

// ---------- real replies (tests/fixtures/herdr-0.9.3, anonymised captures) ----------

fn fixture(name: &str) -> Value {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("herdr-0.9.3").join(name);
    serde_json::from_slice(&fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).expect("fixture json")
}

fn fixture_process_info() {
    use nimitz_mux::herdr::{foreground_of, parse_process_info};
    let v = fixture("process-info.json");
    let procs = parse_process_info(&v).expect("the nested list");
    assert_eq!(procs, vec![cmdv(&["herdr", "pane", "process-info", "--pane", "w4:pT"])]);
    assert_eq!(foreground_of(&procs), cmdv(&["herdr", "pane", "process-info", "--pane", "w4:pT"]), "not a shell: taken");
    // the flat shape is read too; no list at all is an error, never an idle shell
    let flat = json!({"result": {"type": "pane_process_info", "foreground_processes": [{"pid": 1, "name": "zsh", "argv": ["-zsh"]}]}});
    assert_eq!(foreground_of(&parse_process_info(&flat).unwrap()), Vec::<String>::new(), "only the shell: idle");
    for bad in [json!({"result": {"type": "pane_process_info", "process_info": {"pane_id": "w1:p1"}}}), json!({"result": {"type": "ok"}}), json!({})] {
        assert!(parse_process_info(&bad).is_err(), "{bad}");
    }
    assert_eq!(nimitz_mux::herdr::process_pids(&v), vec![Some(4242)]);
    assert!(nimitz_mux::herdr::process_pids(&json!({"result": {"type": "ok"}})).is_empty());
    // argv missing: cmdline, then name
    let v = json!({"result": {"process_info": {"foreground_processes": [{"pid": 2, "name": "claude", "cmdline": "claude attach 9aacf927"}, {"pid": 3, "name": "yazi"}]}}});
    assert_eq!(parse_process_info(&v).unwrap(), vec![cmdv(&["claude", "attach", "9aacf927"]), cmdv(&["yazi"])]);
    // through the adapter: the fake emits the same nesting
    let w = tab_world(Some(vec!["yazi"]));
    assert_eq!(w.mux().foreground(&pane_ref("w1:p2", "w1:t2", "w1")).unwrap(), cmdv(&["yazi"]));
    assert_eq!(w.mux().foreground(&pane_ref("w1:p1", "w1:t1", "w1")).unwrap(), Vec::<String>::new());
    assert!(w.mux().foreground(&pane_ref("w1:p9", "w1:t1", "w1")).is_err(), "no such pane: an error, never a shell");
}

fn fixture_snapshot_lists() {
    use nimitz_mux::herdr::{agents_in, pane_rows_in, parse_panes, tabs_in, workspaces_in};
    let v = fixture("snapshot.json");
    let snap = v.pointer("/result/snapshot").expect("result.snapshot");
    let ws = workspaces_in(&snap["workspaces"]);
    assert_eq!(ws.len(), 12);
    assert!(ws.contains(&("w3".into(), "ALPHA".into(), "w3:t1".into())) && ws.contains(&("w5".into(), "BRAVO".into(), "w5:t2".into())), "{ws:?}");
    let tabs = tabs_in(&snap["tabs"]);
    assert_eq!(tabs.len(), 12);
    assert!(tabs.contains(&("w3:t1".into(), "ADMIRAL".into())) && tabs.contains(&("w6:t1".into(), "BRAVO.SQA.COMMODORE".into())), "{tabs:?}");
    let panes = pane_rows_in(&snap["panes"]);
    assert_eq!(panes.len(), 14);
    let in_w4t1: Vec<&str> = panes.iter().filter(|p| p.tab_id.as_deref() == Some("w4:t1")).map(|p| p.pane_id.as_str()).collect();
    assert_eq!(in_w4t1, ["w4:p1", "w4:pF", "w4:pT"], "a tab with splits, in id order");
    assert_eq!(panes[0].workspace_id.as_deref(), Some("w1"));
    let agents = agents_in(&snap["agents"]);
    assert_eq!(agents.len(), 11);
    let adm = agents.iter().find(|a| a.pane.pane_id == "w3:p1").unwrap();
    assert_eq!((adm.title.as_str(), adm.state.as_str(), adm.session_id.as_deref().map(|s| &s[..8]), adm.pane.tab_id.as_deref(), adm.pane.workspace_id.as_deref()), ("[ALPHA]-ADMIRAL", "idle", Some("41a2ca48"), Some("w3:t1"), Some("w3")));
    assert_eq!(adm.cwd, "/home/user/nimitz/353517");
    // the same rows under the list replies' paths (`result.agents`, `result.panes`, ...)
    let as_list = json!({"id": "cli:agent:list", "result": {"type": "agent_list", "agents": snap["agents"]}});
    assert_eq!(parse_panes(&as_list).len(), 11);
    assert_eq!(parse_panes(&json!({"result": {"type": "agent_list"}})).len(), 0);
}

fn fixture_create_replies() {
    use nimitz_mux::herdr::{created_pane, created_tab, created_workspace};
    // the schema's ResponseResult shapes: workspace_created carries workspace, tab
    // and root_pane; tab_created carries tab and root_pane; a split replies pane_info
    let ws = json!({"id": "cli:workspace:create", "result": {"type": "workspace_created",
        "workspace": {"workspace_id": "w9", "label": "F", "number": 9, "active_tab_id": "w9:t1", "tab_count": 1, "pane_count": 1, "focused": false, "agent_status": "unknown"},
        "tab": {"tab_id": "w9:t1", "workspace_id": "w9", "label": "1", "number": 1, "pane_count": 1, "focused": false, "agent_status": "unknown"},
        "root_pane": {"pane_id": "w9:p1", "tab_id": "w9:t1", "workspace_id": "w9", "terminal_id": "term_1", "focused": false, "agent_status": "unknown", "revision": 1}}});
    assert_eq!(created_workspace(&ws).unwrap(), ("w9".into(), Some("w9:t1".into()), Some("w9:p1".into())));
    let tab = json!({"id": "cli:tab:create", "result": {"type": "tab_created",
        "tab": {"tab_id": "w9:t2", "workspace_id": "w9", "label": "F.SQ.S", "number": 2, "pane_count": 1, "focused": false, "agent_status": "unknown"},
        "root_pane": {"pane_id": "w9:p2", "tab_id": "w9:t2", "workspace_id": "w9", "terminal_id": "term_2", "focused": false, "agent_status": "unknown", "revision": 1}}});
    assert_eq!(created_tab(&tab).unwrap(), ("w9:t2".into(), Some("w9:p2".into())));
    let split = json!({"result": {"type": "pane_info", "pane": {"pane_id": "w9:p8", "tab_id": "w9:t3", "workspace_id": "w9"}}});
    assert_eq!(created_pane(&split).unwrap(), ("w9:p8".into(), "w9:t3".into()));
    assert!(created_tab(&json!({"result": {"type": "ok"}})).is_err());
    assert!(created_workspace(&json!({"result": {"type": "workspace_created"}})).is_err());
    // through the adapter (the fake answers with those shapes): the direct path, no pane list needed
    let w = World::new(None);
    let r = w.mux().open(&cmdv(&["claude", "attach", "9aacf927"]), Path::new("/x"), Place::Tab { workspace: "NEW".into(), label: "NEW.ADMIRAL".into() }).expect("open");
    assert_eq!(r, pane_ref("w2:p1", "w2:t1", "w2"));
    assert_eq!(w.pane("w2:p1")["ran"], json!(["claude attach 9aacf927"]));
    assert!(!w.calls().iter().any(|c| c[..2] == ["pane".to_string(), "list".to_string()]), "the reply names the pane: {:?}", w.calls());
    // the fallback, apart: a reply with no tab or pane gives None, which the adapter resolves
    let bare_ws = json!({"result": {"type": "workspace_created", "workspace": {"workspace_id": "w9", "active_tab_id": "w9:t1"}}});
    assert_eq!(created_workspace(&bare_ws).unwrap(), ("w9".into(), Some("w9:t1".into()), None));
    let bare_tab = json!({"result": {"type": "tab_created", "tab": {"tab_id": "w9:t2"}}});
    assert_eq!(created_tab(&bare_tab).unwrap(), ("w9:t2".into(), None));
}

/// The key set of a JSON object, sorted.
fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    k.sort();
    k
}

fn fixture_snapshot_shape() {
    use nimitz_mux::herdr::parse_snapshot;
    let v = fixture("snapshot.json");
    let shape = parse_snapshot(&v).expect("the snapshot");
    assert_eq!((shape.version.as_str(), shape.protocol), ("0.9.3", 22));
    assert_eq!((shape.workspaces.len(), shape.tabs.len(), shape.panes.len()), (12, 12, 14));
    // panes[] carry no agent; agents[] are joined in by pane id
    let adm = shape.panes.iter().find(|p| p.pane.pane_id == "w3:p1").expect("w3:p1");
    assert_eq!((adm.agent.as_deref(), adm.session_id.as_deref().map(|s| &s[..8]), adm.state.as_str(), adm.title.as_str()), (Some("claude"), Some("41a2ca48"), "idle", "[ALPHA]-ADMIRAL"));
    assert_eq!(adm.cwd, "/home/user/nimitz/353517");
    assert_eq!(adm.foreground, None, "unread until process-info");
    let shell = shape.panes.iter().find(|p| p.pane.pane_id == "w1:p1").expect("w1:p1");
    assert_eq!((shell.agent.as_deref(), shell.session_id.as_deref(), shell.state.as_str()), (None, None, "unknown"));
    assert_eq!(shape.tab("w6:t1").map(|t| (t.workspace_id.as_str(), t.label.as_str())), Some(("w6", "BRAVO.SQA.COMMODORE")));
    assert_eq!(shape.workspace("w5").map(|w| (w.label.as_str(), w.active_tab_id.as_str())), Some(("BRAVO", "w5:t2")));
    assert_eq!(shape.panes_of("w4:t1").iter().map(|p| p.pane.pane_id.as_str()).collect::<Vec<_>>(), ["w4:p1", "w4:pF", "w4:pT"]);
    assert!(parse_snapshot(&json!({"result": {"type": "ok"}})).is_err(), "no snapshot: an error");
    // the fake answers with the same field set, row for row
    let w = World::new(None);
    let out = Command::new(w.dir.join(nimitz_core::os::exe_name("herdr"))).args(["api", "snapshot"]).output().expect("fake");
    let fv: Value = serde_json::from_slice(&out.stdout).expect("json");
    let real = v.pointer("/result/snapshot").unwrap();
    let fake = fv.pointer("/result/snapshot").expect("result.snapshot");
    assert_eq!(keys(real), keys(fake), "snapshot keys");
    for list in ["workspaces", "tabs", "panes", "layouts"] {
        assert_eq!(keys(&real[list][0]), keys(&fake[list][0]), "{list}[0] keys");
    }
    // an agent row: the real sample's pane and the fake's agent pane
    assert_eq!(keys(&real["agents"][0]), keys(&fake["agents"][0]), "agents[0] keys");
    assert_eq!(keys(&real["agents"][0]["agent_session"]), keys(&fake["agents"][0]["agent_session"]), "agent_session keys");
    assert_eq!(keys(&real["layouts"][0]["panes"][0]), keys(&fake["layouts"][0]["panes"][0]));
    let fs = parse_snapshot(&fv).expect("the fake's snapshot parses through the same reader");
    assert_eq!(fs.panes.len(), 2);
    assert_eq!(fs.panes[0].session_id.as_deref(), Some("11111111-2222-3333-4444-555555555555"));
    assert_eq!(fs.panes[1].state, "blocked");
}

fn shape_through_the_adapter() {
    let w = tab_world(Some(vec!["yazi"]));
    let shape = w.mux().shape(true).expect("shape");
    assert_eq!(shape.workspaces.len(), 1);
    assert_eq!(shape.tabs.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(), ["HOME.GITHUB.COMMODORE", "HOME.GITHUB.NIMITZ_DEV"]);
    let p1 = shape.panes.iter().find(|p| p.pane.pane_id == "w1:p1").unwrap();
    let p2 = shape.panes.iter().find(|p| p.pane.pane_id == "w1:p2").unwrap();
    assert_eq!(p1.foreground, Some(vec![]), "an idle shell");
    assert_eq!((p1.foreground_pid, p2.foreground_pid), (None, Some(4242)), "the pid of what runs, none for an idle shell");
    assert_eq!(p2.foreground, Some(cmdv(&["yazi"])));
    assert_eq!(p2.cwd, "/dev");
    let n = w.calls().iter().filter(|c| c[..2] == ["pane".to_string(), "process-info".to_string()]).count();
    assert_eq!(n, 2, "one process-info per pane: {:?}", w.calls());
    assert!(w.calls().iter().any(|c| c[..2] == ["api".to_string(), "snapshot".to_string()]));
    // rename a tab
    w.mux().rename_tab("w1:t2", "HOME.GITHUB.S").expect("rename");
    assert_eq!(w.mux().shape(false).unwrap().tab("w1:t2").map(|t| t.label.clone()), Some("HOME.GITHUB.S".into()));
    assert!(w.mux().rename_tab("w9:t9", "X").is_err());
    // a named session: `--session <name>` before every call, which the fake logs and strips
    let named = Herdr { bin: w.dir.join(nimitz_core::os::exe_name("herdr")).to_string_lossy().into_owned(), session: Some("nimitz-e2e".into()), socket: None, claude_home: "/nonexistent/nimitz-claude-home".into(), focus_retry_ms: herdr::FOCUS_RETRY_MS.to_vec() };
    assert_eq!(named.shape(false).unwrap().workspaces.len(), 1);
    assert!(w.calls().last().unwrap().starts_with(&["--session".to_string(), "nimitz-e2e".to_string(), "api".to_string()]), "{:?}", w.calls().last());
    // the exec knob: `pane run` really runs the line in the pane's folder
    let mut world = w.world();
    world["exec"] = json!(true);
    let mark = w.dir.join("mark.txt");
    for p in world["panes"].as_array_mut().unwrap().iter_mut().filter(|p| p["pane_id"] == "w1:p1") {
        p["cwd"] = json!(w.dir.to_string_lossy());
    }
    fs::write(w.dir.join("world.json"), serde_json::to_vec_pretty(&world).unwrap()).unwrap();
    w.mux().run_in(&pane_ref("w1:p1", "w1:t1", "w1"), &cmdv(&["sh", "-c", "pwd > mark.txt"])).expect("run");
    let text = fs::read_to_string(&mark).expect("the line ran in the pane's cwd");
    // (Windows: Git Bash's `pwd` prints its own form of the folder; the line ran there)
    if cfg!(unix) {
        assert_eq!(text.trim(), nimitz_core::os::canonical(&w.dir).unwrap().to_string_lossy());
    }
}

/// Herdr's hook in a background job names the pane its daemon was started
/// from (an inherited `HERDR_PANE_ID`): the pane claims member A's session
/// while its foreground attaches to lead B's job. The claim is not backed,
/// so it is ignored: the pane shows B, read from the attach (inferred); a
/// held `--resume` or the foreground's session file the same way, never
/// inferred; a backed claim stands.
fn an_unbacked_claim_shows_what_runs_in_the_pane() {
    let (a_sid, b_sid) = ("9aacf927-0000-4000-8000-00000000000a", "f29c13aa-0000-4000-8000-00000000000b");
    let claim = json!({"kind": "id", "source": "herdr:claude", "value": a_sid});
    let on = |id: &str, fg: Value, pid: i64, claim: &Value| json!({"pane_id": id, "tab_id": "w1:t1", "workspace_id": "w1", "agent": "claude", "agent_status": "working", "cwd": "/home", "agent_session": claim, "foreground": fg, "pid": pid});
    let pane = |id: &str, fg: Value| on(id, fg, 777, &claim);
    let w = World::new(Some(json!({
        "next_ws": 2,
        "workspaces": [{"workspace_id": "w1", "label": "HOME", "focused": true, "active_tab_id": "w1:t1"}],
        "tabs": [{"tab_id": "w1:t1", "workspace_id": "w1", "label": "HOME.ADMIRAL", "focused": true}],
        "panes": [
            pane("w1:p1", json!(["/home/u/.local/bin/claude", "attach", "f29c13aa"])),
            on("w1:p2", json!(["claude", "--resume", "s-held"]), 778, &claim),
            pane("w1:p3", json!(["claude"])),
            pane("w1:p4", json!(["claude", "--resume", a_sid])),
            pane("w1:p5", json!(["yazi"])),
            // an interactive resume a `/clear` moved on: its session file wins over the argv
            pane("w1:p6", json!(["claude", "--resume", "s-held"])),
            // B's own claim, backed by the attach alone: shown, inferred all the same
            on("w1:p7", json!(["claude", "attach", "f29c13aa"]), 779, &json!({"kind": "id", "source": "herdr:claude", "value": b_sid})),
        ]
    })));
    let home = w.dir.join("claude_home");
    fs::create_dir_all(home.join("jobs").join("f29c13aa")).unwrap();
    fs::create_dir_all(home.join("sessions")).unwrap();
    fs::write(home.join("jobs/f29c13aa/state.json"), json!({"sessionId": b_sid}).to_string()).unwrap();
    fs::write(home.join("sessions/777.json"), json!({"pid": 777, "sessionId": "s-own"}).to_string()).unwrap();
    let m = Herdr { claude_home: home.clone(), ..w.mux() };
    let want = [("w1:p1", Some(b_sid), true), ("w1:p2", Some("s-held"), false), ("w1:p3", Some("s-own"), false), ("w1:p4", Some(a_sid), false), ("w1:p5", None, false), ("w1:p6", Some("s-own"), false), ("w1:p7", Some(b_sid), true)];
    let panes = m.panes();
    for (id, sid, inferred) in want {
        let p = panes.iter().find(|p| p.pane.pane_id == id).unwrap_or_else(|| panic!("no pane {id}"));
        assert_eq!((p.session_id.as_deref(), p.inferred), (sid, inferred), "panes(): {id}");
    }
    for foregrounds in [false, true] {
        let shape = m.shape(foregrounds).expect("shape");
        for (id, sid, inferred) in want {
            let p = shape.panes.iter().find(|p| p.pane.pane_id == id).unwrap();
            assert_eq!((p.session_id.as_deref(), p.inferred), (sid, inferred), "shape({foregrounds}): {id}");
            assert!(p.claimed.is_some(), "the claim is kept for the agent-view test: {id}");
        }
    }
    // the job's conversation now: a `/clear` moves it
    fs::write(home.join("jobs/f29c13aa/state.json"), json!({"sessionId": b_sid, "resumeSessionId": "s-after-clear"}).to_string()).unwrap();
    let p1 = m.panes().into_iter().find(|p| p.pane.pane_id == "w1:p1").unwrap();
    assert_eq!((p1.session_id.as_deref(), p1.inferred), (Some("s-after-clear"), true));
    // a foreground that cannot be read backs no claim and shows nothing
    let mut world = w.world();
    world["fail"] = json!("pane process-info");
    fs::write(w.dir.join("world.json"), serde_json::to_vec_pretty(&world).unwrap()).unwrap();
    let panes = m.panes();
    assert!(!panes.is_empty() && panes.iter().all(|p| p.session_id.is_none() && !p.inferred), "{panes:?}");
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
    fs::create_dir_all(tmp.join("config").join("herdr")).expect("mkdir");
    fs::write(tmp.join("config").join("herdr").join("config.toml"), "onboarding = false\n").expect("config");
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
    let sock = tmp.join("config").join("herdr").join("herdr.sock");

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
    let m = Herdr { bin: bin.to_string(), session: None, socket: None, claude_home: "/nonexistent/nimitz-claude-home".into(), focus_retry_ms: herdr::FOCUS_RETRY_MS.to_vec() };
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
