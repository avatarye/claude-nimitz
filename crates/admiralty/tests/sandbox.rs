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
        "history.lost MAIN/ARGOS/BROKEN",
        "copy MAIN/ARGOS/TAKEN",
        "labels.mismatch MAIN/ARGOS/MISLABEL",
        "folder.missing MAIN/ARGOS/GONE",
        "folder.untrusted MAIN/ARGOS/UNTRUSTED",
    ] {
        assert!(codes.iter().any(|c| c == want), "{want} in {codes:?}");
    }
    assert!(!snap.stowaways.is_empty(), "the sandbox has stowaways");
    tab(&mut app, "MAIN");
    let s = screen(&mut app, 200, 60);
    assert!(s.contains("DRIFT"), "the drifted ship, by its own part");
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
    assert_eq!(f.command, "nimitz squadron new MAIN/CIRRUS");
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
    assert!(f.ok(), "{:?}", f.problems);
    // the first field is the brief: a new one goes to the squadron's brief file
    keys(&mut app, "New orders");
    let Some(Modal::Form(f)) = &app.modal else { panic!() };
    assert!(f.ok(), "{:?}", f.problems);
    assert!(f.command.starts_with("nimitz squadron set MAIN/ARGOS --brief"), "{}", f.command);
    assert!(f.plan.steps.iter().any(|s| s.cmd[1].ends_with("SQUADRON.md")), "{:?}", f.plan.steps);
    ctrl(&mut app, 's');
    let Some(Modal::Confirm(p)) = app.modal.take() else { panic!() };
    let o = actions::apply(&app.ctx.env, &p);
    assert!(o.issues.is_empty(), "{:?}", o.issues);
    reload(&w, &mut app);

    let mut f = Form::edit_fleet(&app.ctx.env, "MAIN").unwrap();
    f.set("mode", "auto");
    f.validate(&app.ctx.env, &app.bundle.as_ref().unwrap().snap);
    assert!(f.ok(), "{:?}", f.problems);
    assert_eq!(f.command, "nimitz fleet set MAIN --admiral-mode auto");
    assert!(f.plan.steps.iter().any(|s| s.cmd[1].ends_with("fleet.toml")));
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
    assert!(env.contains("export NIMITZ_MUX=herdr") && (env.contains("bin/herdr") || env.contains("bin\\herdr")), "{env}");
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

/// Admiralty started outside a Herdr pane (no HERDR_ENV, no NIMITZ_MUX: a
/// terminal, a launcher) used to see no pane at all, while a member's
/// `nimitz status` saw every one through the recorded server. Now it takes the
/// Herdr server the user's own commands recorded in active.json, and sees the
/// same panes as inside Herdr; with nothing recorded it stays monitor-only.
#[test]
fn admiralty_outside_herdr_uses_the_recorded_server_and_sees_the_panes() {
    let dir = tempfile::tempdir().unwrap();
    let sb = dir.path().join("sb");
    let bin = env!("CARGO_BIN_EXE_admiralty");
    let ok = std::process::Command::new(bin).args(["sandbox", "create", "--herdr"]).arg(&sb).output().unwrap();
    assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
    let env = String::from_utf8_lossy(&std::process::Command::new(bin).args(["sandbox", "env"]).arg(&sb).output().unwrap().stdout).into_owned();
    let mut inside: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for l in env.lines() {
        let (k, v) = l.trim_start_matches("export ").split_once('=').unwrap();
        inside.insert(k.into(), v.into());
    }
    let seen_inside = CoreSource::detect(&inside).load().panes;
    assert!(!seen_inside.is_empty(), "the sandbox's fake Herdr shows panes");
    // outside Herdr: no NIMITZ_MUX, no HERDR_ENV
    let mut outside = inside.clone();
    outside.remove("NIMITZ_MUX");
    outside.remove("HERDR_ENV");
    assert_eq!(CoreSource::detect(&outside).load().mux_name, "none", "nothing recorded: monitor-only");
    // the user's own command once recorded the server it ran in
    let state = std::path::PathBuf::from(&outside["NIMITZ_STATE"]);
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("active.json"), r#"{"version": 1, "at": 1.0, "herdr": {"session": "sb"}, "members": {}}"#).unwrap();
    let b = CoreSource::detect(&outside).load();
    assert_eq!(b.mux_name, "herdr");
    assert_eq!(b.panes.len(), seen_inside.len(), "the same panes as inside Herdr");
}

/// A lead's attach pane that Herdr never claimed (an attach client runs no
/// Herdr hook: `agent` claude, no `agent_session`) still shows the lead: its
/// `claude attach <job>` foreground names the job, so the row gets its pane
/// (the dot, `j`, `A` focusing it) as `nimitz status` does. So does one a
/// ship's hook claimed (its worker inherited the pane's `HERDR_PANE_ID` from
/// the daemon): the attach does not back that claim, so it is ignored.
#[test]
fn an_unclaimed_attach_pane_still_shows_its_lead() {
    attach_pane_shows_its_lead(false);
}

#[test]
fn an_attach_pane_a_ships_hook_claimed_still_shows_its_lead() {
    attach_pane_shows_its_lead(true);
}

fn attach_pane_shows_its_lead(claimed: bool) {
    let dir = tempfile::tempdir().unwrap();
    let sb = dir.path().join("sb");
    let bin = env!("CARGO_BIN_EXE_admiralty");
    let ok = std::process::Command::new(bin).args(["sandbox", "create", "--herdr"]).arg(&sb).output().unwrap();
    assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
    let env = String::from_utf8_lossy(&std::process::Command::new(bin).args(["sandbox", "env"]).arg(&sb).output().unwrap().stdout).into_owned();
    let mut vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for l in env.lines() {
        let (k, v) = l.trim_start_matches("export ").split_once('=').unwrap();
        vars.insert(k.into(), v.into());
    }
    let b = CoreSource::detect(&vars).load();
    let lead = b.snap.fleets.iter().flat_map(|f| std::iter::once(&f.admiral).chain(f.squadrons.iter().map(|q| &q.commodore))).find_map(|m| m.session.as_ref().and_then(|s| s.short_id.clone().map(|j| (m.id.clone(), j, s.session_id.clone())))).expect("a lead with a job");
    let (id, job, sid) = lead;
    let ship = b.snap.fleets.iter().flat_map(|f| f.squadrons.iter().flat_map(|q| q.ships.iter())).find_map(|m| m.session.as_ref().map(|s| (m.id.clone(), s.session_id.clone()))).expect("a ship with a session");
    let world = sb.join("herdr").join("world.json");
    let mut w: serde_json::Value = serde_json::from_slice(&std::fs::read(&world).unwrap()).unwrap();
    let p = &mut w["panes"][0];
    p.as_object_mut().unwrap().remove("agent_session");
    if claimed {
        p["agent_session"] = serde_json::json!({"kind": "id", "source": "herdr:claude", "value": ship.1});
    }
    p["agent"] = serde_json::json!("claude");
    p["agent_status"] = serde_json::json!("working");
    p["foreground"] = serde_json::json!(["/home/u/.local/bin/claude", "attach", job]);
    let pane = p["pane_id"].as_str().unwrap().to_string();
    std::fs::write(&world, serde_json::to_vec(&w).unwrap()).unwrap();
    let b = CoreSource::detect(&vars).load();
    let shown = b.panes.iter().find(|p| p.pane.pane_id == pane).expect("the pane");
    assert_eq!(shown.session_id.as_deref(), Some(sid.as_str()), "{id}'s attach pane shows its session");
    assert!(shown.inferred, "known by its attach alone");
    assert!(!b.panes.iter().any(|p| p.session_id.as_deref() == Some(ship.1.as_str())), "{}'s claim shows it nowhere", ship.0);
    // `A` finds that pane and only focuses it (nothing is typed, no new tab)
    let ctx = Ctx { env: nimitz_core::env::Env::from_vars(&vars), mux: nimitz_mux::detect(&vars), live: false, exit_after_jump: false };
    match actions::existing_open(&ctx, &b.snap, &Target::Member(id.clone())) {
        Ok(nimitz_mux::TabState::Shown(p)) => assert_eq!(p.pane_id, pane),
        other => panic!("{id}: {other:?}"),
    }
}

/// The user's setup: Admiralty started outside Herdr while three Herdr
/// servers run (the named NIMITZ the user works in, `default`, `nikitz`),
/// each with a pane that looks like the lead's. It speaks only to the NIMITZ
/// server recorded in active.json, the lead's row gets its unclaimed attach
/// pane (the live shape: `agent` claude, no `agent_session`), `J` and `K`
/// land on it and `enter` focuses it there.
#[cfg(unix)]
#[test]
fn with_several_herdr_servers_enter_and_j_k_jump_to_the_attach_pane() {
    let dir = tempfile::tempdir().unwrap();
    let sb = dir.path().join("sb");
    let bin = env!("CARGO_BIN_EXE_admiralty");
    let ok = std::process::Command::new(bin).args(["sandbox", "create", "--herdr"]).arg(&sb).output().unwrap();
    assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
    let env = String::from_utf8_lossy(&std::process::Command::new(bin).args(["sandbox", "env"]).arg(&sb).output().unwrap().stdout).into_owned();
    let mut vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for l in env.lines() {
        let (k, v) = l.trim_start_matches("export ").split_once('=').unwrap();
        vars.insert(k.into(), v.into());
    }
    let b = CoreSource::detect(&vars).load();
    let (id, job, cwd) = b.snap.fleets.iter().flat_map(|f| std::iter::once(&f.admiral).chain(f.squadrons.iter().map(|q| &q.commodore))).find_map(|m| m.session.as_ref().and_then(|s| s.short_id.clone().map(|j| (m.id.clone(), j, s.cwd.clone())))).expect("a lead with a job");
    let claude = "/home/u/.local/share/claude/versions/2.1.293";
    let world = sb.join("herdr").join("world.json");
    let mut w: serde_json::Value = serde_json::from_slice(&std::fs::read(&world).unwrap()).unwrap();
    let p = &mut w["panes"][0];
    p.as_object_mut().unwrap().remove("agent_session");
    p["agent"] = serde_json::json!("claude");
    p["agent_status"] = serde_json::json!("working");
    p["foreground"] = serde_json::json!([claude, "attach", job]);
    let pane = p["pane_id"].as_str().unwrap().to_string();
    std::fs::write(&world, serde_json::to_vec(&w).unwrap()).unwrap();
    for (name, ws) in [("default", "x8"), ("nikitz", "x9")] {
        let d = sb.join(format!("herdr-{name}"));
        std::fs::create_dir_all(&d).unwrap();
        let decoy = serde_json::json!({
            "workspaces": [{"workspace_id": ws, "label": "X", "focused": true, "active_tab_id": format!("{ws}:t1")}],
            "tabs": [{"tab_id": format!("{ws}:t1"), "workspace_id": ws, "label": "X", "focused": true}],
            "panes": [{"pane_id": format!("{ws}:p1"), "tab_id": format!("{ws}:t1"), "workspace_id": ws, "agent": "claude", "agent_status": "idle",
                       "terminal_title_stripped": "x", "cwd": cwd, "foreground_cwd": cwd, "foreground": [claude, "attach", job]}],
            "next_ws": 10
        });
        std::fs::write(d.join("world.json"), serde_json::to_vec(&decoy).unwrap()).unwrap();
    }
    let multi = sb.join("bin").join("herdr-multi");
    std::fs::write(&multi, format!(
        "#!/bin/sh\ncase \"$1\" in --session) s=$2; shift 2;; *) s=default;; esac\ncase \"$s\" in NIMITZ) b='{b}/herdr';; nikitz) b='{b}/herdr-nikitz';; *) b='{b}/herdr-default';; esac\nexec '{bin}' __fake-herdr \"$b\" \"$@\"\n",
        b = sb.display()
    )).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&multi, std::fs::Permissions::from_mode(0o755)).unwrap();
    let state = std::path::PathBuf::from(&vars["NIMITZ_STATE"]);
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("active.json"), r#"{"version": 1, "at": 1.0, "herdr": {"session": "NIMITZ"}, "members": {}}"#).unwrap();
    let mut outside = vars.clone();
    outside.remove("NIMITZ_MUX");
    outside.remove("HERDR_ENV");
    outside.insert("NIMITZ_HERDR_BIN".into(), multi.to_string_lossy().into_owned());
    // as main.rs builds it
    let e = nimitz_core::env::Env::from_vars(&outside);
    let mux = nimitz_mux::detect(&nimitz_core::active::user_mux_vars(&e, &outside));
    let mut app = App::new(Ctx { env: e, mux, live: true, exit_after_jump: false }, None, true);
    app.apply(CoreSource::detect(&outside).load());
    tab(&mut app, id.fleet());
    goto(&mut app, &format!("M:{}", id.0));
    assert_eq!(app.current().unwrap().node.pane_id.as_deref(), Some(pane.as_str()), "{id}'s row has its attach pane");
    press(&mut app, KeyCode::Home);
    press(&mut app, KeyCode::Char('J'));
    assert_eq!(app.current().unwrap().node.pane_id.as_deref(), Some(pane.as_str()), "J lands on it: {}", toast(&app));
    press(&mut app, KeyCode::Char('K'));
    assert_eq!(app.current().unwrap().node.pane_id.as_deref(), Some(pane.as_str()), "K too: {}", toast(&app));
    let from = std::fs::read_to_string(sb.join("herdr").join("calls.jsonl")).unwrap_or_default().lines().count();
    press(&mut app, KeyCode::Enter);
    assert!(toast(&app).contains(&format!("Focused {pane}")), "{}", toast(&app));
    let calls: Vec<String> = std::fs::read_to_string(sb.join("herdr").join("calls.jsonl")).unwrap().lines().skip(from).map(String::from).collect();
    // the wrapper routes `--session NIMITZ` alone to this world
    assert!(calls.iter().any(|c| c.contains("\"agent\",\"focus\"") && c.contains(&pane)), "{calls:?}");
    // `m` types there while the pane runs exactly `claude attach <job>`,
    // read again when the message goes; once the client re-execs as the
    // agent view (`claude agents`, R-24) nothing is typed
    let b = app.bundle.as_ref().unwrap();
    let plan = actions::plan_message(b.mux_caps, &b.snap, &b.panes, &Target::Member(id.clone())).unwrap();
    assert_eq!(plan.attach_job.as_deref(), Some(job.as_str()));
    let prompts = |sb: &std::path::Path| -> Vec<String> {
        let w: serde_json::Value = serde_json::from_slice(&std::fs::read(sb.join("herdr").join("world.json")).unwrap()).unwrap();
        w["panes"][0]["prompts"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
    };
    goto(&mut app, &format!("M:{}", id.0));
    press(&mut app, KeyCode::Char('m'));
    keys(&mut app, "first");
    press(&mut app, KeyCode::Enter);
    assert!(toast(&app).starts_with("Sent to"), "{}", toast(&app));
    assert!(prompts(&sb).iter().any(|p| p.contains("first")), "{:?}", prompts(&sb));
    // #12: this machine names a provider: no record of the member's current
    // session, nothing typed
    let sid = app.current().unwrap().node.session_id.clone().unwrap();
    let rec = app.ctx.env.state.join("provider").join("sessions").join(format!("{sid}.json"));
    let kept = std::fs::read(&rec).unwrap();
    std::fs::remove_file(&rec).unwrap();
    press(&mut app, KeyCode::Char('m'));
    keys(&mut app, "unconfirmed");
    press(&mut app, KeyCode::Enter);
    assert!(toast(&app).contains("provider.unrecorded"), "{}", toast(&app));
    assert!(!prompts(&sb).iter().any(|p| p.contains("unconfirmed")), "nothing typed");
    std::fs::write(&rec, kept).unwrap();
    press(&mut app, KeyCode::Char('m'));
    keys(&mut app, "second");
    let mut w: serde_json::Value = serde_json::from_slice(&std::fs::read(&world).unwrap()).unwrap();
    w["panes"][0]["foreground"] = serde_json::json!([claude, "agents"]);
    std::fs::write(&world, serde_json::to_vec(&w).unwrap()).unwrap();
    press(&mut app, KeyCode::Enter);
    assert!(toast(&app).contains("no longer runs `claude attach"), "{}", toast(&app));
    assert!(!prompts(&sb).iter().any(|p| p.contains("second")), "nothing typed into the agent view");
    // the refusal stays in the action log, as a sent message does (#13)
    let log = std::fs::read_to_string(app.ctx.env.state.join("admiralty-actions.log")).unwrap_or_default();
    let entry = |what: &str, text: &str| log.lines().any(|l| l.contains(&format!("\"what\":\"{what}\"")) && l.contains(text));
    assert!(entry("message", "first") && entry("message refused", "second"), "{log}");
    for name in ["default", "nikitz"] {
        let c = std::fs::read_to_string(sb.join(format!("herdr-{name}/calls.jsonl"))).unwrap_or_default();
        assert!(c.is_empty(), "never a call to the {name} server: {c}");
    }
}

#[cfg(unix)]
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
    let t = base.join("main").join("squadrons").join("argos").join("squadron.toml");
    let s = std::fs::read_to_string(&t).unwrap();
    let at = format!("{}:13", s.lines().count() + 2);
    std::fs::write(&t, s + "\nname = \"oops\n").unwrap();
    let mut app = app_on(&w, TestMux::default());
    tab(&mut app, "MAIN");
    let buf = draw(&mut app, 200, 60);
    let ls = lines(&buf);
    assert!(ls[0].contains("MAIN") && ls.iter().any(|l| l.contains("ADMIRAL ")), "the fleet stays on screen");
    let y = ls.iter().position(|l| l.contains("ARGOS") && is_sq_bar(l)).expect("the squadron stays on screen");
    assert_eq!(buf[(1, y as u16)].bg, admiralty::theme::mono::ERROR, "its bar takes the error hue");
    let all = ls.join("\n");
    assert!(all.contains("squadron.toml") && all.contains(&at), "the file and line, in full:\n{all}");
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
    let t = base.join("main").join("squadrons").join("argos").join("squadron.toml");
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
    std::fs::create_dir_all(base.join("bad").join("admiral")).unwrap();
    std::fs::write(base.join("bad").join("fleet.toml"), "fleet = \"BAD\"\n\n[m").unwrap();
    let mut app = app_on(&w, TestMux::default());
    let snap = app.bundle.as_ref().unwrap().snap.clone();
    assert!(snap.fleets.iter().any(|f| f.id.0 == "MAIN"), "the other fleets stay");
    assert!(admiralty::view::tabs(&snap).contains(&"BAD".to_string()));
    tab(&mut app, "BAD");
    let buf = draw(&mut app, 200, 60);
    let ls = lines(&buf);
    assert!(ls[0].contains("BAD") && ls[0].contains("config does not load"), "the broken fleet's tab says so: {}", ls[0]);
    let y = ls.iter().position(|l| l.starts_with('\u{250f}')).expect("its box is on screen");
    assert_eq!(buf[(0, y as u16)].fg, admiralty::theme::mono::ERROR, "its box takes the error hue");
    let all = ls.join("\n");
    assert!(all.contains("fleet.toml") && all.contains(":3:3") && all.contains("unclosed table"), "the file, line and the parse error's own words:\n{all}");
    assert!(all.contains("fix: fix") && all.contains("line 3"), "{all}");
    let parse: Vec<_> = all_issues(&snap).into_iter().filter(|i| i.message.contains("does not parse")).collect();
    assert_eq!(parse.len(), 1, "{parse:?}");
}

/// `x R` on a fleet band: the plan of `nimitz restore F --plan --json` in
/// the confirm dialog, then `nimitz restore F --yes` through `apply`; a
/// second time there is nothing to bring back. Needs the `nimitz` binary
/// beside this one (a workspace build); skipped when it is not there.
#[test]
fn restore_shows_the_cli_plan_and_runs_it_through_the_cli() {
    let nimitz = actions::nimitz_bin();
    if !std::path::Path::new(&nimitz).is_file() {
        eprintln!("[WARN] no nimitz binary at {nimitz}; the restore action test is skipped (build the workspace first)");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let sb = dir.path().join("sb");
    let bin = env!("CARGO_BIN_EXE_admiralty");
    let ok = std::process::Command::new(bin).args(["sandbox", "create", "--herdr"]).arg(&sb).output().unwrap();
    assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
    let env_text = String::from_utf8_lossy(&std::process::Command::new(bin).args(["sandbox", "env"]).arg(&sb).output().unwrap().stdout).into_owned();
    let mut vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for l in env_text.lines() {
        let (k, v) = l.trim_start_matches("export ").split_once('=').unwrap();
        vars.insert(k.into(), v.trim_matches('\'').into());
    }
    let env = nimitz_core::env::Env::from_vars(&vars);
    let ctx = Ctx { env, mux: nimitz_mux::detect(&vars), live: true, exit_after_jump: false };
    let mut app = App::new(ctx, None, true);
    app.apply(CoreSource::detect(&vars).load());
    goto(&mut app, "F:MAIN");
    keys(&mut app, "x");
    keys(&mut app, "R");
    let Some(Modal::Confirm(p)) = app.modal.take() else { panic!("{}", toast(&app)) };
    assert_eq!((p.act, p.command.as_str()), (Act::Restore, "nimitz muster MAIN/..."));
    assert!(p.plan.ok(), "{:?}", p.plan.refusals);
    assert_eq!(p.plan.steps.iter().filter(|s| s.cmd.first().map(String::as_str) == Some("herdr") && s.cmd.get(2).map(String::as_str) == Some("create")).count(), 3, "a tab per live lead: {:?}", p.plan.steps);
    app.begin(&p).unwrap();
    let o = actions::apply(&app.ctx.env, &p);
    app.finish(&p, &o);
    assert!(o.verified, "{:?}", o.issues);
    // a tab per live lead, and a pin for each (the sandbox's leads run unpinned)
    assert_eq!(o.ran.len(), 6, "{:?}", o.ran);
    assert_eq!(o.ran.iter().filter(|(s, _, _)| s.cmd.first().map(String::as_str) == Some("pin")).count(), 3, "{:?}", o.ran);
    assert!(toast(&app).starts_with("restore: muster MAIN/...: panes missing (Herdr closed, restarted or never laid out) (6 steps"), "{}", toast(&app));
    // again: nothing to bring back
    app.apply(CoreSource::detect(&vars).load());
    goto(&mut app, "F:MAIN");
    keys(&mut app, "x");
    keys(&mut app, "R");
    let Some(Modal::Confirm(p)) = app.modal.take() else { panic!("{}", toast(&app)) };
    assert!(p.plan.steps.is_empty(), "{:?}", p.plan.steps);
    let o = actions::apply(&app.ctx.env, &p);
    app.finish(&p, &o);
    assert!(o.verified && o.ran.is_empty());
    assert_eq!(toast(&app), "restore: muster MAIN/...: nothing to bring back");
    // the detail panel lists it on a fleet band, and no fleet selected means no restore
    assert!(app.available(Act::Restore).is_ok());
    goto(&mut app, "M:MAIN/ARGOS/CLEAN");
    assert!(app.available(Act::Restore).is_err());
}

/// What counts as the exact attach: `claude attach <job>` (by its short id,
/// or the full id it begins) from a Claude Code at or past the version that
/// re-execs as the agent view (its versions/<v> path); never the view, an
/// older or unknown version.
#[test]
fn the_exact_attach_is_only_claude_attach_of_that_job() {
    let mux = TestMux::default();
    let cases: &[(&[&str], bool)] = &[
        (&["/home/u/.local/share/claude/versions/2.1.293", "attach", "f8eee8df"], true),
        (&["/home/u/.local/share/claude/versions/2.1.300", "attach", "f8eee8df-b691-4eac-a8a7-83004ac4653a"], true),
        (&["/home/u/.local/share/claude/versions/2.1.292", "attach", "f8eee8df"], false),
        (&["/home/u/.local/bin/claude", "attach", "f8eee8df"], false),
        (&["/home/u/.local/bin/claude", "agents"], false),
        (&["/v/claude/versions/2.1.293", "agents"], false),
        (&["/v/claude/versions/2.1.293", "attach", "f8eee8dfx"], false),
        (&["/v/claude/versions/2.1.293", "attach", "0000aaaa"], false),
        (&["/v/claude/versions/2.1.293", "attach", "f8eee8df", "--x"], false),
        // a localtools install, with its build component
        (&["/Users/u/.localtools/tools/claude-code/2.1.294.1000/claude", "attach", "f8eee8df"], true),
        (&["/Users/u/.localtools/tools/claude-code/2.1.288.1000/claude", "attach", "f8eee8df"], false),
        (&[], false),
    ];
    for (i, (fg, want)) in cases.iter().enumerate() {
        let pane = format!("p{i}");
        mux.with_tab("W", &format!("T{i}"), &pane, fg);
        let r = nimitz_mux::PaneRef { pane_id: pane, tab_id: None, workspace_id: None };
        assert_eq!(nimitz_mux::runs_exact_attach(&mux, &r, "f8eee8df"), *want, "{fg:?}");
    }
    // and why not, in words
    use nimitz_mux::AttachCheck;
    let n = std::cell::Cell::new(0);
    let check = |fg: &[&str]| {
        n.set(n.get() + 1);
        let pane = format!("pc{}", n.get());
        mux.with_tab("W", &format!("TC{}", n.get()), &pane, fg);
        nimitz_mux::attach_check(&mux, &nimitz_mux::PaneRef { pane_id: pane, tab_id: None, workspace_id: None }, "f8eee8df")
    };
    let old = check(&["/Users/u/.localtools/tools/claude-code/2.1.288.1000/claude", "attach", "f8eee8df"]);
    assert_eq!(old, AttachCheck::Older("2.1.288.1000".into()));
    assert_eq!(
        old.refusal("pc1", "f8eee8df"),
        "its pane pc1 runs `claude attach f8eee8df` on Claude Code 2.1.288.1000, older than 2.1.293 that nimitz types into safely; reattach it (ctrl-z, then nimitz open <id>)"
    );
    // an npm install: the version in its package.json (#13)
    let npm = std::env::temp_dir().join(format!("admiralty-npm-{}", std::process::id()));
    let root = npm.join("node_modules/@anthropic-ai/claude-code");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let exe = root.join("bin").join("claude.exe");
    std::fs::write(&exe, "").unwrap();
    for (v, want) in [("2.1.312", AttachCheck::Exact), ("2.1.292", AttachCheck::Older("2.1.292".into()))] {
        std::fs::write(root.join("package.json"), format!(r#"{{"name": "@anthropic-ai/claude-code", "version": "{v}"}}"#)).unwrap();
        assert_eq!(check(&[exe.to_str().unwrap(), "attach", "f8eee8df"]), want, "{v}");
    }
    let _ = std::fs::remove_dir_all(&npm);
    let unknown = check(&["/opt/somewhere/claude", "attach", "f8eee8df"]);
    assert_eq!(unknown, AttachCheck::Unknown);
    assert!(unknown.refusal("pc", "f8eee8df").contains("whose version nimitz cannot read"));
    let view = check(&["/v/claude/versions/2.1.293", "agents"]);
    assert_eq!(view, AttachCheck::Other);
    assert!(view.refusal("pc", "f8eee8df").contains("no longer runs `claude attach f8eee8df`"));
}

/// A Workflow run's agents sit under one row per run, under the member: the
/// workflow's name, live / total, the newest agent's phase, open while one
/// runs; a run that finished long ago folds into `+N done` as one.
#[test]
fn workflow_agents_show_under_a_row_per_run() {
    let w = world();
    let mut app = app_on(&w, TestMux::default());
    tab(&mut app, "MAIN");
    let ids: Vec<String> = app.rows.iter().map(|r| r.node.id.clone()).collect();
    let run = app.rows.iter().find(|r| r.node.id == "WF:MAIN/ARGOS:wf_sbx0001-aaa").unwrap_or_else(|| panic!("{ids:?}"));
    assert_eq!(run.node.name, "sandbox-fanout");
    assert!(run.expanded, "a run with a live agent is open");
    let detail: String = run.node.detail.iter().map(|s| s.content.to_string()).collect();
    assert!(detail.contains("2 live / 3 total") && detail.contains("Fan-out") && detail.contains("wf_sbx0001-aaa"), "{detail}");
    for a in ["wfa-1", "wfa-2", "wfa-3", "survey-1"] {
        assert!(ids.contains(&format!("C:MAIN/ARGOS:{a}")), "{a} in {ids:?}");
    }
    // the finished run is inside the fold, as one
    assert!(!ids.iter().any(|i| i.contains("wf_sbx0002-bbb")), "{ids:?}");
    let m = app.bundle.as_ref().unwrap().snap.fleets.iter().flat_map(admiralty::view::fleet_members).find(|m| m.id.0 == "MAIN/ARGOS").unwrap().clone();
    assert_eq!(admiralty::view::folded_crew(&m.crew, nimitz_core::evidence::now(), ""), 1);
    let s = screen(&mut app, 200, 80);
    assert!(s.contains("sandbox-fanout") && s.contains("L0:lane zero") && s.contains("+1 done"), "{s}");
}

