//! The real path: nimitz-core's snapshot of a sandbox world (fake `claude`,
//! no real sessions), core plans and `apply`, through the App.

mod common;

use admiralty::actions::{self, Act, Ctx, Target};
use admiralty::app::{App, Modal};
use admiralty::data::{CoreSource, DataSource};
use admiralty::forms::Form;
use admiralty::view::all_issues;
use common::*;
use nimitz_core::Id;
use ratatui::crossterm::event::KeyCode;

struct World {
    _dir: tempfile::TempDir,
    vars: std::collections::HashMap<String, String>,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("sb");
    nimitz_core::sandbox::create(&base, std::path::Path::new(env!("CARGO_BIN_EXE_admiralty"))).unwrap();
    let vars = nimitz_core::sandbox::env_for(&base);
    World { _dir: dir, vars }
}

fn app_on(w: &World, mux: TestMux) -> App {
    let env = nimitz_core::env::Env::from_vars(&w.vars);
    let ctx = Ctx { env, mux: Box::new(mux), live: true, exit_after_jump: false };
    let mut app = App::new(ctx, None, true);
    app.apply(CoreSource::detect(&w.vars).load());
    app
}

fn reload(w: &World, app: &mut App) {
    app.apply(CoreSource::detect(&w.vars).load());
}

#[test]
fn every_broken_shape_of_the_sandbox_shows_up() {
    let w = world();
    let mut app = app_on(&w, TestMux::default());
    let snap = &app.bundle.as_ref().unwrap().snap;
    let codes: Vec<String> = all_issues(snap).iter().map(|i| format!("{} {}", i.code, i.id.as_ref().map(|x| x.0.as_str()).unwrap_or("-"))).collect();
    for want in [
        "name.drift MAIN/ARGOS/DRIFT",
        "labels.missing MAIN/ARGOS/NOLABEL",
        "dup.live MAIN/ARGOS/TWIN",
        "copy.rows MAIN/ARGOS/COPIES",
        "lineage.broken MAIN/ARGOS/BROKEN",
        "name.taken MAIN/ARGOS/TAKEN",
        "labels.mismatch MAIN/ARGOS/MISLABEL",
        "folder.missing MAIN/ARGOS/GONE",
        "folder.untrusted MAIN/ARGOS/UNTRUSTED",
    ] {
        assert!(codes.iter().any(|c| c == want), "{want} in {codes:?}");
    }
    assert!(!snap.stowaways.is_empty(), "the sandbox has stowaways");
    let s = screen(&mut app, 200, 60);
    assert!(s.contains("[MAIN.ARGOS]-DRIFT"));
    assert!(s.contains("STOWAWAYS"));
    // no multiplexer in the sandbox: monitor-only
    assert!(s.contains("MONITOR-ONLY"));
}

#[test]
fn sweep_runs_through_core_and_the_rows_are_gone_after() {
    let w = world();
    let mut app = app_on(&w, TestMux::default());
    goto(&mut app, "M:MAIN/ARGOS/COPIES");
    keys(&mut app, "x");
    keys(&mut app, "w");
    let Some(Modal::Confirm(p)) = app.modal.take() else { panic!("{}", toast(&app)) };
    assert_eq!(p.command, "nimitz sweep MAIN/ARGOS/COPIES");
    assert!(p.plan.ok(), "{:?}", p.plan.refusals);
    assert!(!p.plan.steps.is_empty());
    app.begin(&p).unwrap();
    let o = actions::apply(&app.ctx.env, &p);
    app.finish(&p, &o);
    assert!(toast(&app).starts_with("sweep:"), "{}", toast(&app));
    assert!(o.issues.is_empty(), "{:?}", o.issues);
    reload(&w, &mut app);
    let m = actions::find_member(&app.bundle.as_ref().unwrap().snap, &Id("MAIN/ARGOS/COPIES".into())).unwrap();
    assert!(m.extra_rows.is_empty(), "stale rows swept: {:?}", m.extra_rows);
}

#[test]
fn a_new_squadron_form_writes_the_config_through_core() {
    let w = world();
    let mut app = app_on(&w, TestMux::default());
    goto(&mut app, "F:MAIN");
    keys(&mut app, "c");
    keys(&mut app, "s");
    let Some(Modal::Form(_)) = &app.modal else { panic!("{}", toast(&app)) };
    keys(&mut app, "ARGOS");
    let Some(Modal::Form(f)) = &app.modal else { panic!() };
    assert!(!f.ok(), "ARGOS exists already: core refuses");
    assert!(f.problems.iter().any(|p| p.contains("ARGOS")), "{:?}", f.problems);
    ctrl(&mut app, 'u');
    keys(&mut app, "CIRRUS");
    let Some(Modal::Form(f)) = &app.modal else { panic!() };
    assert!(f.ok(), "{:?} {:?}", f.problems, f.fields.iter().map(|x| &x.error).collect::<Vec<_>>());
    assert_eq!(f.command, "nimitz squadron new MAIN/CIRRUS --comms open");
    ctrl(&mut app, 's');
    let Some(Modal::Confirm(p)) = app.modal.take() else { panic!() };
    app.begin(&p).unwrap();
    let o = actions::apply(&app.ctx.env, &p);
    app.finish(&p, &o);
    assert!(o.issues.is_empty(), "{:?}", o.issues);
    reload(&w, &mut app);
    assert!(app.rows.iter().any(|r| r.node.id == "SQ:MAIN/CIRRUS"), "new squadron is listed");
}

#[test]
fn edit_ship_form_loads_the_ship_from_its_squadron_toml() {
    let w = world();
    let app = app_on(&w, TestMux::default());
    let f = Form::edit_ship(&app.ctx.env, &Id("MAIN/ARGOS/CLEAN".into()), "sandbox").unwrap();
    assert_eq!(f.get("ship"), "CLEAN");
    assert!(!f.get("dir").is_empty());
    let mut f = f;
    f.validate(&app.ctx.env, &app.bundle.as_ref().unwrap().snap);
    assert!(f.ok(), "{:?}", f.problems);
    assert!(f.command.starts_with("nimitz ship set MAIN/ARGOS/CLEAN"));
}

#[test]
fn starting_a_stopped_ship_with_a_multiplexer_resumes_it_through_core() {
    let w = world();
    let mux = TestMux::default();
    let mut app = app_on(&w, mux.clone());
    // the sandbox has no mux; say this one has (as Herdr would)
    app.bundle.as_mut().unwrap().mux_caps = nimitz_mux::Caps { open: true, focus: true, type_text: true, layout: true };
    goto(&mut app, "M:MAIN/ARGOS/STOPPED");
    keys(&mut app, "a");
    let Some(Modal::Confirm(p)) = app.modal.take() else { panic!("{}", toast(&app)) };
    assert!(p.plan.ok(), "{:?}", p.plan.refusals);
    assert!(p.plan.steps.iter().any(|s| s.cmd.contains(&"--resume".to_string())), "{:?}", p.plan.steps);
    app.begin(&p).unwrap();
    let o = actions::apply(&app.ctx.env, &p);
    app.finish(&p, &o);
    assert!(o.issues.is_empty(), "{:?}", o.issues);
    reload(&w, &mut app);
    let m = actions::find_member(&app.bundle.as_ref().unwrap().snap, &Id("MAIN/ARGOS/STOPPED".into())).unwrap();
    assert!(m.session.as_ref().and_then(|s| s.pid).is_some(), "running again: {:?}", m.reasons);
}

#[test]
fn rm_a_stopped_stowaway_is_refused_while_live_and_planned_when_stopped() {
    let w = world();
    let mut app = app_on(&w, TestMux::default());
    let snap = app.bundle.as_ref().unwrap().snap.clone();
    let s = &snap.stowaways[0];
    let t = Target::Stowaway(s.session_id.clone());
    let r = actions::available(Default::default(), &snap, &t, Act::Rm);
    if s.pid.is_some() {
        assert!(r.unwrap_err().contains("live"));
    } else {
        assert!(r.is_ok());
    }
    goto(&mut app, &format!("SW:{}", s.session_id));
    press(&mut app, KeyCode::Char('x'));
    assert!(matches!(app.modal, Some(Modal::Menu { .. })));
}

#[test]
fn edit_squadron_and_fleet_forms_plan_through_core() {
    let w = world();
    let mut app = app_on(&w, TestMux::default());
    goto(&mut app, "SQ:MAIN/ARGOS");
    keys(&mut app, "e");
    let Some(Modal::Form(f)) = &app.modal else { panic!("{}", toast(&app)) };
    assert_eq!(f.get("comms"), "open");
    assert!(f.ok(), "{:?}", f.problems);
    // a bad comms rule is caught in the field
    ctrl(&mut app, 'u');
    keys(&mut app, "everyone");
    let Some(Modal::Form(f)) = &app.modal else { panic!() };
    assert!(f.fields.iter().find(|x| x.key == "comms").unwrap().error.is_some());
    ctrl(&mut app, 'u');
    keys(&mut app, "allow:MAIN/ARGOS/CLEAN");
    let Some(Modal::Form(f)) = &app.modal else { panic!() };
    assert!(f.ok(), "{:?}", f.problems);
    assert_eq!(f.command, "nimitz squadron set MAIN/ARGOS --comms allow:MAIN/ARGOS/CLEAN");
    assert!(f.plan.steps.iter().any(|s| s.cmd[1].ends_with("squadron.toml")));
    ctrl(&mut app, 's');
    let Some(Modal::Confirm(p)) = app.modal.take() else { panic!() };
    let o = actions::apply(&app.ctx.env, &p);
    assert!(o.issues.is_empty(), "{:?}", o.issues);
    reload(&w, &mut app);
    let sq = app.bundle.as_ref().unwrap().snap.fleets.iter().flat_map(|f| f.squadrons.iter()).find(|s| s.id.0 == "MAIN/ARGOS").unwrap().clone();
    assert_eq!(sq.comms, nimitz_core::Comms::Allow { allow: vec!["MAIN/ARGOS/CLEAN".into()] });

    let mut f = Form::edit_fleet(&app.ctx.env, "MAIN").unwrap();
    f.set("mode", "auto");
    f.validate(&app.ctx.env, &app.bundle.as_ref().unwrap().snap);
    assert!(f.ok(), "{:?}", f.problems);
    assert_eq!(f.command, "nimitz fleet set MAIN --admiral-mode auto");
    assert!(f.plan.steps.iter().any(|s| s.cmd[1].ends_with("fleet.toml")));
}

#[test]
fn a_tender_form_starts_a_tender_through_core() {
    let w = world();
    let mut app = app_on(&w, TestMux::default());
    goto(&mut app, "SQ:MAIN/ARGOS");
    keys(&mut app, "c");
    keys(&mut app, "t");
    let Some(Modal::Form(_)) = &app.modal else { panic!("{}", toast(&app)) };
    keys(&mut app, "fix ci");
    let Some(Modal::Form(f)) = &app.modal else { panic!() };
    assert!(f.fields.iter().find(|x| x.key == "task").unwrap().error.is_some(), "one word");
    ctrl(&mut app, 'u');
    keys(&mut app, "lint");
    press(&mut app, KeyCode::Tab);
    keys(&mut app, "Run the linter and report.");
    let Some(Modal::Form(f)) = &app.modal else { panic!() };
    assert!(f.ok(), "{:?}", f.problems);
    assert_eq!(f.command, "nimitz tender MAIN/ARGOS lint 'Run the linter and report.'");
    ctrl(&mut app, 's');
    let Some(Modal::Confirm(p)) = app.modal.take() else { panic!() };
    assert_eq!(p.act, Act::Tender);
    let o = actions::apply(&app.ctx.env, &p);
    assert!(o.issues.is_empty(), "{:?}", o.issues);
    reload(&w, &mut app);
    assert!(app.rows.iter().any(|r| r.node.id == "M:MAIN/ARGOS/~lint"), "the tender is listed");
}

#[test]
fn machine_rm_form_plans_through_core() {
    let w = world();
    let app = app_on(&w, TestMux::default());
    let mut f = Form::machine_rm("MAIN", "sandbox");
    f.validate(&app.ctx.env, &app.bundle.as_ref().unwrap().snap);
    assert!(f.ok(), "{:?}", f.problems);
    assert_eq!(f.command, "nimitz machine rm MAIN sandbox");
    assert!(f.plan.steps.iter().any(|s| s.cmd[1].ends_with("fleet.toml")));
}
