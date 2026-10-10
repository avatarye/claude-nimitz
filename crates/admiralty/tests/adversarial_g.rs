//! Adversarial round 1, scenario G (Admiralty): one regression test per BUG the
//! sandbox shows. Each asserts the RIGHT behaviour and fails today;
//! `#[ignore = "BUG-nn"]` keeps the suite green until the fix lands.
//! Sandbox only (fake `claude`, test mux), contracts.md section 11.

mod common;

use admiralty::actions::Ctx;
use admiralty::app::{App, Modal};
use admiralty::data::{CoreSource, DataSource};
use common::*;

struct World {
    _dir: tempfile::TempDir,
    base: std::path::PathBuf,
    vars: std::collections::HashMap<String, String>,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("sb");
    nimitz_core::sandbox::create(&base, std::path::Path::new(env!("CARGO_BIN_EXE_admiralty"))).unwrap();
    let vars = nimitz_core::sandbox::env_for(&base);
    let base = nimitz_core::os::canonical(&base).unwrap();
    World { _dir: dir, base, vars }
}

/// An App on the sandbox; `mux` false = monitor-only (what `admiralty --sandbox` gives).
fn app_on(w: &World, mux: bool) -> App {
    let env = nimitz_core::env::Env::from_vars(&w.vars);
    let ctx = Ctx { env, mux: Box::new(TestMux::default()), live: true, exit_after_jump: false };
    let mut app = App::new(ctx, None, true);
    app.apply(CoreSource::detect(&w.vars).load());
    let caps = if mux { nimitz_mux::Caps { open: true, focus: true, type_text: true, layout: true } } else { nimitz_mux::Caps::default() };
    app.bundle.as_mut().unwrap().mux_caps = caps;
    app
}

#[test]
fn bug32_remove_from_config_of_a_live_ship_stops_it_first() {
    // `nimitz ship rm` plans stop + rm + write; the dialog says it runs that command.
    let w = world();
    let mut app = app_on(&w, true);
    goto(&mut app, "M:MAIN/ARGOS/DRIFT");
    keys(&mut app, "D");
    let Some(Modal::Confirm(p)) = app.modal.take() else { panic!("no confirm: {}", toast(&app)) };
    assert!(
        p.plan.steps.iter().any(|s| s.cmd.iter().any(|a| a == "stop")),
        "`{}` shown, but the plan only rewrites the config; the live ship keeps running and leaves the tree: {:?}",
        p.command,
        p.plan.steps.iter().map(|s| s.cmd.join(" ")).collect::<Vec<_>>()
    );
}

#[test]
fn bug34_monitor_only_still_starts_a_ship() {
    // plan-v3 section 13: a multiplexer is not needed for ships (daemon sessions).
    let w = world();
    let mut app = app_on(&w, false);
    goto(&mut app, "M:MAIN/ARGOS/STOPPED");
    keys(&mut app, "a");
    assert!(matches!(app.modal, Some(Modal::Confirm(_))), "start of a ship refused in monitor-only: {}", toast(&app));
}

#[test]
fn bug35_a_broken_squadron_toml_keeps_the_fleet_on_screen() {
    let w = world();
    let t = w.base.join("nimitz").join("main").join("squadrons").join("argos").join("squadron.toml");
    let s = std::fs::read_to_string(&t).unwrap();
    std::fs::write(&t, s + "\nname = \"oops\n").unwrap();
    let mut app = app_on(&w, false);
    tab(&mut app, "MAIN");
    let sc = screen(&mut app, 120, 40);
    assert!(sc.contains("[MAIN]"), "fleet MAIN (8 live sessions) is gone from the screen:\n{sc}");
}

#[test]
fn bug36_at_80x24_the_issue_code_quit_and_help_are_visible() {
    let w = world();
    let mut app = app_on(&w, false);
    goto(&mut app, "M:MAIN/ARGOS/NOLABEL");
    let sc = screen(&mut app, 80, 24);
    assert!(sc.contains("labels.missing"), "the issue code is cut to `…` at 80 columns:\n{sc}");
    assert!(sc.contains("q quit") || sc.contains("? help"), "the footer loses help and quit at 80 columns:\n{sc}");
}

#[test]
fn bug38_a_ship_start_does_not_warn_about_the_commodore_folder() {
    // The stowaway in ARGOS's commodore folder has nothing to do with starting STOPPED.
    let w = world();
    let mut app = app_on(&w, true);
    goto(&mut app, "M:MAIN/ARGOS/STOPPED");
    keys(&mut app, "a");
    let Some(Modal::Confirm(p)) = app.modal.take() else { panic!("no confirm: {}", toast(&app)) };
    let warns: Vec<String> = p.plan.warnings.iter().map(|i| format!("{} {}", i.code, i.message)).collect();
    assert!(!warns.iter().any(|w| w.contains("would make a second agent")), "unrelated warning in a ship's start dialog: {warns:?}");
}
