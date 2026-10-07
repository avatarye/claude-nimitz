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
    tab(&mut app, "MAIN");
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

#[test]
fn retiring_a_live_ship_stops_it_removes_its_rows_and_the_config() {
    let w = world();
    let mut app = app_on(&w, TestMux::default());
    goto(&mut app, "M:MAIN/ARGOS/CLEAN");
    keys(&mut app, "D");
    let Some(Modal::Confirm(p)) = app.modal.take() else { panic!("{}", toast(&app)) };
    assert_eq!(p.command, "nimitz ship rm MAIN/ARGOS/CLEAN");
    assert!(p.plan.ok(), "{:?}", p.plan.refusals);
    assert!(p.plan.steps.iter().any(|s| s.cmd.contains(&"stop".to_string())), "stop first: {:?}", p.plan.steps);
    assert!(p.plan.steps.iter().any(|s| s.cmd[0] == "write"), "then the config: {:?}", p.plan.steps);
    app.begin(&p).unwrap();
    let o = actions::apply(&app.ctx.env, &p);
    app.finish(&p, &o);
    assert!(o.issues.is_empty(), "{:?}", o.issues);
    reload(&w, &mut app);
    assert!(!app.rows.iter().any(|r| r.node.id == "M:MAIN/ARGOS/CLEAN"), "gone from the list");
}

#[test]
fn layout_goes_to_the_multiplexer_from_cores_plan() {
    let w = world();
    let mux = TestMux::default();
    let mut app = app_on(&w, mux.clone());
    app.bundle.as_mut().unwrap().mux_caps = nimitz_mux::Caps { open: true, focus: true, type_text: true, layout: true };
    goto(&mut app, "F:MAIN");
    keys(&mut app, "x");
    keys(&mut app, "S");
    let Some(Modal::Confirm(p)) = app.modal.take() else { panic!("{}", toast(&app)) };
    app.begin(&p).unwrap();
    let o = actions::apply(&app.ctx.env, &p);
    app.finish(&p, &o);
    let calls = mux.calls.lock().unwrap().clone();
    assert!(calls.contains(&"layout MAIN.ARGOS/MAIN.ARGOS.CLEAN".to_string()), "{calls:?}");
    assert!(toast(&app).starts_with("layout:"), "{}", toast(&app));
}

#[test]
fn bug38_a_legacy_warning_stays_in_its_own_members_dialog() {
    let w = world();
    let mut app = app_on(&w, TestMux::default());
    app.bundle.as_mut().unwrap().mux_caps = nimitz_mux::Caps { open: true, focus: true, type_text: true, layout: true };
    for row in ["M:MAIN/ARGOS/STOPPED", "M:MAIN/ARGOS/NOLABEL"] {
        goto(&mut app, row);
        keys(&mut app, "a");
        if let Some(Modal::Confirm(p)) = app.modal.take() {
            let id = Id(row.trim_start_matches("M:").into());
            assert!(p.plan.warnings.iter().all(|i| i.code != "legacy" || i.id.as_ref() == Some(&id)), "{row}: {:?}", p.plan.warnings);
        }
    }
}

#[test]
fn bug39_sandbox_with_a_fake_herdr_uses_it() {
    let dir = tempfile::tempdir().unwrap();
    let sb = dir.path().join("sb");
    let bin = env!("CARGO_BIN_EXE_admiralty");
    let ok = std::process::Command::new(bin).args(["sandbox", "create", "--herdr"]).arg(&sb).output().unwrap();
    assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
    let env = String::from_utf8_lossy(&std::process::Command::new(bin).args(["sandbox", "env"]).arg(&sb).output().unwrap().stdout).into_owned();
    assert!(env.contains("export NIMITZ_MUX=herdr") && env.contains("bin/herdr"), "{env}");
    // and the TUI's own dump runs against it with the mux, not monitor-only
    let mut vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for l in env.lines() {
        let (k, v) = l.trim_start_matches("export ").split_once('=').unwrap();
        vars.insert(k.into(), v.into());
    }
    let src = CoreSource::detect(&vars).load();
    assert_eq!(src.mux_name, "herdr");
    assert!(src.mux_caps.open);
}

#[test]
fn bug40_the_first_look_is_quick_and_the_full_one_follows() {
    let w = world();
    let mut vars = w.vars.clone();
    // a `claude agents` that takes 3 s, like the real one
    let slow = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(slow.path(), "#!/bin/sh\nsleep 3\necho '[]'\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(slow.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let slow = slow.into_temp_path();
    vars.insert("NIMITZ_NATIVE_CLAUDE_BIN".into(), slow.to_string_lossy().into_owned());
    let mut src = CoreSource { first: true, ..CoreSource::detect(&vars) };
    let t = std::time::Instant::now();
    let b = src.load();
    assert!(b.partial && t.elapsed() < std::time::Duration::from_millis(1500), "first look took {:?}", t.elapsed());
    assert!(b.snap.fleets.iter().any(|f| f.id.0 == "MAIN"), "live sessions already show");
    let b = src.load();
    assert!(!b.partial);
}

#[test]
fn bug35_a_broken_config_is_a_red_frame_with_the_error_in_full() {
    let w = world();
    let base = std::path::PathBuf::from(&w.vars["NIMITZ_HOME"]);
    let t = base.join("main/squadrons/argos/squadron.toml");
    let s = std::fs::read_to_string(&t).unwrap();
    std::fs::write(&t, s + "\nname = \"oops\n").unwrap();
    let mut app = app_on(&w, TestMux::default());
    tab(&mut app, "MAIN");
    let buf = draw(&mut app, 200, 60);
    let ls = lines(&buf);
    assert!(ls.iter().any(|l| l.contains("[MAIN]")), "the fleet stays on screen");
    let y = ls.iter().position(|l| l.contains("ARGOS") && l.contains("comms")).expect("the squadron stays on screen");
    assert_eq!(buf[(1, y as u16)].fg, admiralty::theme::mono::ERROR, "its frame takes the error hue");
    let all = ls.join("\n");
    assert!(all.contains("squadron.toml") && all.contains("164:13"), "the file and line, in full:\n{all}");
    assert!(all.contains("invalid basic string"), "the parse error's own words");
}

#[test]
fn bug33_the_ship_form_refuses_label_equals_folder_and_relative_folders() {
    let w = world();
    let app = app_on(&w, TestMux::default());
    let snap = app.bundle.as_ref().unwrap().snap.clone();
    for bad in ["sandbox=managed", "sandbox=/work/x", "relative/dir", "./x"] {
        let mut f = Form::new_ship("MAIN", "ARGOS", "sandbox");
        f.set("ship", "X33");
        f.set("prompt", "p");
        f.set("dir", bad);
        f.validate(&app.ctx.env, &snap);
        assert!(!f.ok(), "{bad:?} must be refused");
        assert!(f.fields.iter().find(|x| x.key == "dir").unwrap().error.is_some(), "{bad:?}: the dir field says why");
        // and nothing reaches the config: the confirm dialog is never offered
    }
    for good in ["managed", "/work/x33", "~/work/x33"] {
        let mut f = Form::new_ship("MAIN", "ARGOS", "sandbox");
        f.set("ship", "X33");
        f.set("prompt", "p");
        f.set("dir", good);
        f.validate(&app.ctx.env, &snap);
        assert!(f.fields.iter().find(|x| x.key == "dir").unwrap().error.is_none(), "{good:?} is fine");
    }
}

#[test]
fn the_issues_view_lists_a_broken_squadron_once() {
    let w = world();
    let base = std::path::PathBuf::from(&w.vars["NIMITZ_HOME"]);
    let t = base.join("main/squadrons/argos/squadron.toml");
    let s = std::fs::read_to_string(&t).unwrap();
    std::fs::write(&t, s + "\nname = \"oops\n").unwrap();
    let app = app_on(&w, TestMux::default());
    let issues = app.issues();
    let parse: Vec<_> = issues.iter().filter(|i| i.message.contains("does not parse")).collect();
    assert_eq!(parse.len(), 1, "{parse:?}");
    let mut keys: Vec<_> = issues.iter().map(|i| (i.code.clone(), i.id.clone(), i.message.clone())).collect();
    let n = keys.len();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), n, "every issue once");
}

#[test]
fn a_broken_fleet_toml_is_a_red_band_with_the_error_in_full_and_the_other_fleets_stay() {
    let w = world();
    let base = std::path::PathBuf::from(&w.vars["NIMITZ_HOME"]);
    std::fs::create_dir_all(base.join("bad/admiral")).unwrap();
    std::fs::write(base.join("bad/fleet.toml"), "fleet = \"BAD\"\n\n[m").unwrap();
    let mut app = app_on(&w, TestMux::default());
    let snap = app.bundle.as_ref().unwrap().snap.clone();
    assert!(snap.fleets.iter().any(|f| f.id.0 == "MAIN"), "the other fleets stay");
    assert!(admiralty::view::tabs(&snap).contains(&"BAD".to_string()));
    tab(&mut app, "BAD");
    let buf = draw(&mut app, 200, 60);
    let ls = lines(&buf);
    let y = ls.iter().position(|l| l.contains("[BAD]")).expect("the broken fleet is on screen");
    assert_eq!(buf[(1, y as u16)].fg, admiralty::theme::mono::ERROR, "its frame takes the error hue");
    let all = ls.join("\n");
    assert!(all.contains("config does not load"), "{all}");
    assert!(all.contains("fleet.toml") && all.contains(":3:3") && all.contains("unclosed table"), "the file, line and the parse error's own words:\n{all}");
    assert!(all.contains("fix: fix") && all.contains("line 3"), "{all}");
    let parse: Vec<_> = all_issues(&snap).into_iter().filter(|i| i.message.contains("does not parse")).collect();
    assert_eq!(parse.len(), 1, "{parse:?}");
}
