//! Every `nimitz ...` command Admiralty shows must be one the real CLI takes,
//! flag for flag, and plan the same thing. Each command is run with
//! `--plan --json` against the same sandbox and its plan compared with ours.

mod common;

use std::path::PathBuf;
use std::sync::Once;

use admiralty::actions::{self, Act, Ctx, Target};
use admiralty::app::App;
use admiralty::data::{CoreSource, DataSource};
use admiralty::forms::Form;
use common::*;
use nimitz_core::{Id, Plan};

/// Build the CLI once and point Admiralty's lookup at it.
fn nimitz() -> PathBuf {
    static BUILD: Once = Once::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    // the build's own target dir (CARGO_TARGET_DIR, relative to the root or absolute)
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).map(|t| root.join(t)).unwrap_or_else(|| root.join("target"));
    let bin = target.join("debug").join("nimitz");
    BUILD.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let st = std::process::Command::new(cargo).args(["build", "-q", "-p", "nimitz"]).current_dir(&root).status().unwrap();
        assert!(st.success(), "cargo build -p nimitz");
        std::env::set_var("NIMITZ_BIN", &bin);
    });
    bin
}

/// Undo `actions::quote`: words, with '...' quoting and '\'' for a quote.
fn split(cmd: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted, mut any) = (vec![], String::new(), false, false);
    let mut it = cmd.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\'' if quoted => quoted = false,
            '\'' => {
                quoted = true;
                any = true;
            }
            '\\' if !quoted => {
                if let Some(n) = it.next() {
                    cur.push(n);
                }
            }
            ' ' if !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

struct World {
    _dir: tempfile::TempDir,
    vars: std::collections::HashMap<String, String>,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("sb");
    nimitz_core::sandbox::create(&base, std::path::Path::new(env!("CARGO_BIN_EXE_admiralty"))).unwrap();
    World { vars: nimitz_core::sandbox::env_for(&base), _dir: dir }
}

fn app_on(w: &World) -> App {
    let ctx = Ctx { env: nimitz_core::env::Env::from_vars(&w.vars), mux: Box::new(TestMux::default()), live: true, exit_after_jump: false };
    let mut app = App::new(ctx, None, true);
    app.apply(CoreSource::detect(&w.vars).load());
    app
}

/// The CLI's plan for a command Admiralty showed.
fn cli(app: &App, command: &str) -> Plan {
    let bin = nimitz();
    let argv = split(command);
    assert_eq!(argv[0], "nimitz", "{command}");
    let out = actions::cli_command(&app.ctx.env, &bin.to_string_lossy()).args(&argv[1..]).args(["--plan", "--json"]).output().unwrap();
    let code = out.status.code().unwrap_or(-1);
    assert_ne!(code, 2, "the CLI rejected `{command}` as a usage error: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("`{command}`: no JSON plan ({e}): {}", String::from_utf8_lossy(&out.stderr)))
}

/// Same steps (what runs, where); our summary may be worded for the screen.
fn same(ours: &Plan, theirs: &Plan, command: &str) {
    assert_eq!(ours.ok(), theirs.ok(), "`{command}`: refusals differ: ours {:?} cli {:?}", ours.refusals, theirs.refusals);
    let cmds = |p: &Plan| p.steps.iter().map(|s| (s.cmd.clone(), s.cwd.clone())).collect::<Vec<_>>();
    assert_eq!(cmds(ours), cmds(theirs), "`{command}`: steps differ");
}

#[test]
fn session_commands_match_the_cli() {
    let w = world();
    let mut app = app_on(&w);
    let caps = nimitz_mux::Caps::default();
    let snap = app.bundle.as_ref().unwrap().snap.clone();
    let cases: Vec<(Target, Act)> = vec![
        (Target::Member(Id("MAIN/ARGOS/CLEAN".into())), Act::Stop),
        (Target::Member(Id("MAIN/ARGOS/COPIES".into())), Act::Sweep),
        (Target::Squadron(Id("MAIN/ARGOS".into())), Act::Sweep),
        (Target::Fleet(Id("MAIN".into())), Act::Sweep),
        (Target::Member(Id("MAIN/ARGOS/COPIES".into())), Act::Rm),
        (Target::Member(Id("MAIN/ARGOS/BROKEN".into())), Act::ConfigRm),
        (Target::Squadron(Id("MAIN/BOREAL".into())), Act::ConfigRm),
    ];
    for (t, a) in cases {
        let p = actions::prepare(&app.ctx, caps, &snap, &t, a, None).unwrap_or_else(|e| panic!("{a:?} {t:?}: {e}"));
        same(&p.plan, &cli(&app, &p.command), &p.command);
    }
    // start and relaunch need a multiplexer in Admiralty; the CLI takes the same command
    let open = nimitz_mux::Caps { open: true, focus: true, type_text: true, layout: true };
    for (id, a) in [("MAIN/ARGOS/STOPPED", Act::Start), ("MAIN/ARGOS/NOLABEL", Act::Relaunch)] {
        let p = actions::prepare(&app.ctx, open, &snap, &Target::Member(Id(id.into())), a, None).unwrap();
        same(&p.plan, &cli(&app, &p.command), &p.command);
    }
    // these the CLI refuses here (no multiplexer); the command must still parse
    for (t, a) in [(Target::Member(Id("MAIN/ARGOS/TWIN".into())), Act::Open), (Target::Fleet(Id("MAIN".into())), Act::Up)] {
        let p = actions::prepare(&app.ctx, open, &snap, &t, a, None).unwrap();
        cli(&app, &p.command);
    }
    // message: the command the message box shows
    app.bundle.as_mut().unwrap().mux_caps = open;
    goto(&mut app, "M:MAIN/ARGOS/CLEAN");
    if let Ok(m) = app.message_plan() {
        cli(&app, &actions::signal_command(&m, "hello there"));
    }
    cli(&app, &actions::signal_command(&actions::MessagePlan { id: Id("MAIN".into()), to: "[MAIN]-ADMIRAL".into(), via: "[MAIN]-ADMIRAL".into(), pane: nimitz_mux::PaneRef { pane_id: "p".into(), tab_id: None, workspace_id: None }, attach_job: None }, "it's me"));
}

#[test]
fn form_commands_match_the_cli() {
    let w = world();
    let app = app_on(&w);
    let snap = app.bundle.as_ref().unwrap().snap.clone();
    let env = app.ctx.env.clone();
    let mut forms: Vec<Form> = vec![];
    let mut f = Form::new_fleet(&env);
    f.set("fleet", "NEWF");
    f.set("machine", "sandbox");
    forms.push(f);
    let mut f = Form::new_squadron("MAIN");
    f.set("squadron", "CIRRUS");
    f.set("brief", "Clouds, with 'quotes'.");
    forms.push(f);
    let mut f = Form::new_ship("MAIN", "ARGOS", "sandbox");
    f.set("ship", "NEWSHIP");
    f.set("prompt", "You build the importer.\nTwo lines.");
    f.set("mode", "auto");
    f.set("depends", "CLEAN");
    forms.push(f);
    let mut f = Form::edit_ship(&env, &Id("MAIN/ARGOS/CLEAN".into()), "sandbox").unwrap();
    f.set("mode", "plan");
    forms.push(f);
    let mut f = Form::edit_squadron(&env, &Id("MAIN/ARGOS".into())).unwrap();
    f.set("brief", "New brief.");
    f.set("mode", "auto");
    forms.push(f);
    let mut f = Form::edit_fleet(&env, "MAIN").unwrap();
    f.set("mode", "acceptEdits");
    forms.push(f);
    forms.push(Form::machine_rm("MAIN", "sandbox"));
    for mut f in forms {
        f.validate(&env, &snap);
        assert!(f.ok(), "{}: {:?}", f.title, f.problems);
        let theirs = cli(&app, &f.command);
        same(&f.plan, &theirs, &f.command);
    }
    // adopt: whatever the identity, the CLI takes the command
    let s = &snap.stowaways[0];
    let mut f = Form::adopt(&s.session_id, s.short_id.as_deref().unwrap_or(""), "MAIN/ARGOS/FRESH");
    f.validate(&env, &snap);
    let theirs = cli(&app, &f.command);
    same(&f.plan, &theirs, &f.command);
}

#[test]
fn layout_is_cores_plan_and_the_cli_takes_its_command() {
    let w = world();
    let mut app = app_on(&w);
    app.bundle.as_mut().unwrap().mux_caps = nimitz_mux::Caps { open: true, focus: true, type_text: true, layout: true };
    goto(&mut app, "F:MAIN");
    keys(&mut app, "x");
    keys(&mut app, "S");
    let Some(admiralty::app::Modal::Confirm(p)) = &app.modal else { panic!("{}", toast(&app)) };
    assert_eq!(p.command, "nimitz layout MAIN/... --ships");
    let lp = p.layout.as_ref().expect("a layout for the mux");
    // --ships: a tab attaching each live ship, in its squadron's workspace
    let argos = lp.workspaces.iter().find(|(w, _)| w == "MAIN.ARGOS").expect("workspace MAIN.ARGOS");
    assert!(argos.1.iter().any(|(tab, cmd, _)| tab == "MAIN.ARGOS.CLEAN" && cmd.contains(&"attach".to_string())), "{argos:?}");
    // the plan's steps are the tabs' commands, nothing else
    let tabs: Vec<&Vec<String>> = lp.workspaces.iter().flat_map(|(_, t)| t.iter().map(|(_, c, _)| c)).collect();
    assert_eq!(p.plan.steps.iter().map(|s| &s.cmd).collect::<Vec<_>>(), tabs);
    // the CLI takes the same command (here it refuses: the sandbox has no Herdr)
    let theirs = cli(&app, &p.command);
    assert!(theirs.refusals.iter().any(|r| r.message.contains("Herdr")), "{:?}", theirs.refusals);
}

#[test]
fn split_undoes_quote() {
    for s in ["plain", "with space", "it's", "a'b'c", "", "--x=y", "line\nbreak"] {
        assert_eq!(split(&format!("nimitz {}", actions::quote(s))), vec!["nimitz".to_string(), s.to_string()], "{s:?}");
    }
}

/// The config under NIMITZ_HOME (folders, .toml, .md), path relative, the
/// sandbox's own path removed.
fn config_tree(w: &World) -> std::collections::BTreeMap<String, String> {
    let home = PathBuf::from(&w.vars["NIMITZ_HOME"]);
    let base = home.parent().unwrap().to_string_lossy().into_owned();
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![home.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                out.insert(format!("{}/", p.strip_prefix(&home).unwrap().display()), String::new());
                stack.push(p);
            } else if p.extension().map(|x| x == "toml" || x == "md").unwrap_or(false) {
                let text = std::fs::read_to_string(&p).unwrap_or_default().replace(&base, "<sandbox>");
                out.insert(p.strip_prefix(&home).unwrap().display().to_string(), text);
            }
        }
    }
    out
}

/// BUG-33: what the CLI stores for the command a form shows is what the form
/// itself stores. Each form runs in one fresh sandbox, its command in another.
#[test]
fn each_form_command_stores_what_the_form_stores() {
    let bin = nimitz();
    type Fill = fn(&nimitz_core::env::Env) -> Form;
    let forms: Vec<(&str, Fill)> = vec![
        ("new fleet", |env| {
            let mut f = Form::new_fleet(env);
            f.set("fleet", "NEWF");
            f.set("machine", "sandbox");
            f
        }),
        ("new squadron", |_| {
            let mut f = Form::new_squadron("MAIN");
            f.set("squadron", "CIRRUS");
            f.set("brief", "Clouds, with 'quotes'.");
            f
        }),
        ("new ship, managed", |_| {
            let mut f = Form::new_ship("MAIN", "ARGOS", "sandbox");
            f.set("ship", "NEWSHIP");
            f.set("prompt", "You build the importer.\nTwo lines.");
            f.set("mode", "auto");
            f.set("depends", "CLEAN");
            f
        }),
        ("new ship, own folder", |_| {
            let mut f = Form::new_ship("MAIN", "ARGOS", "sandbox");
            f.set("ship", "OWNDIR");
            f.set("dir", "/work/owndir");
            f.set("prompt", "Own folder.");
            f
        }),
        ("edit ship", |env| {
            let mut f = Form::edit_ship(env, &Id("MAIN/ARGOS/CLEAN".into()), "sandbox").unwrap();
            f.set("mode", "plan");
            f.set("prompt", "A new prompt.");
            f
        }),
        ("edit squadron", |env| {
            let mut f = Form::edit_squadron(env, &Id("MAIN/ARGOS".into())).unwrap();
            f.set("brief", "New brief.");
            f.set("mode", "auto");
            f
        }),
        ("edit fleet", |env| {
            let mut f = Form::edit_fleet(env, "MAIN").unwrap();
            f.set("mode", "acceptEdits");
            f
        }),
        ("remove machine", |_| Form::machine_rm("MAIN", "sandbox")),
    ];
    for (what, fill) in forms {
        let ours = world();
        let app = app_on(&ours);
        let mut f = fill(&app.ctx.env);
        f.validate(&app.ctx.env, &app.bundle.as_ref().unwrap().snap);
        assert!(f.ok(), "{what}: {:?}", f.problems);
        let o = actions::apply(&app.ctx.env, &f.pending());
        assert!(o.issues.is_empty(), "{what}: {:?}", o.issues);

        let theirs = world();
        let other = app_on(&theirs);
        // the command names sandbox A's folders; in B they are B's
        let base = |w: &World| PathBuf::from(&w.vars["NIMITZ_HOME"]).parent().unwrap().to_string_lossy().into_owned();
        let argv: Vec<String> = split(&f.command).into_iter().map(|a| a.replace(&base(&ours), &base(&theirs))).collect();
        let out = actions::cli_command(&other.ctx.env, &bin.to_string_lossy()).args(&argv[1..]).arg("--yes").output().unwrap();
        assert!(out.status.success(), "{what}: `{}` failed: {}", f.command, String::from_utf8_lossy(&out.stderr));
        let (a, b) = (config_tree(&ours), config_tree(&theirs));
        for k in a.keys().chain(b.keys()).collect::<std::collections::BTreeSet<_>>() {
            assert_eq!(a.get(k), b.get(k), "{what}: `{}` stores {k} differently from the form", f.command);
        }
    }
}
