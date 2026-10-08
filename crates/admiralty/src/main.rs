//! `admiralty` (TUI), `admiralty dump`, `admiralty sandbox ...`, and the hidden
//! helpers a sandbox needs (`__fake-claude`, `__fake-herdr`, `hook`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use admiralty::actions::Ctx;
use admiralty::app::App;
use admiralty::data::{load_fixture_file, Bundle, CoreSource, DataSource, FixtureSource};
use admiralty::{fixtures, frames, render, run};

const USAGE: &str = "usage:
  admiralty [--sandbox DIR | --demo | --demo-monitor | --fixture FILE] [--interval SECS] [--exit-after-jump]
  admiralty dump [--width N] [--sandbox DIR | --demo | --fixture FILE]   the list as plain text
  admiralty sandbox create [--herdr]|env DIR                             a fake world (fake claude, optional fake herdr)

env: NIMITZ_HOME, NIMITZ_STATE, NIMITZ_CLAUDE_HOME, NIMITZ_MUX ... (contracts.md section 2),
     ADMIRALTY_ICONS=ascii (no Nerd Font), ADMIRALTY_STATE (where folds and filters are saved),
     ADMIRALTY_EXIT_AFTER_JUMP=1 (same as --exit-after-jump: quit once a pane jump lands),
     ADMIRALTY_INSET=rows,cols (left blank at the bottom and right: the Herdr overlay hides them),
     NIMITZ_NO_ANIM=1 or NO_COLOR (no status motion)";

enum Source {
    Core,
    Fixture(Bundle),
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("__fake-claude") if args.len() >= 2 => return code(nimitz_core::sandbox::fake_claude(&PathBuf::from(&args[1]), &args[2..])),
        Some("__fake-herdr") if args.len() >= 2 => return code(nimitz_mux::fake::fake_herdr(&PathBuf::from(&args[1]), &args[2..])),
        Some("hook") => return hook(args.get(1).cloned().unwrap_or_default()),
        Some("sandbox") => return sandbox_cmd(&args[1..]),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let mut vars: HashMap<String, String> = std::env::vars().collect();
    let mut dump = false;
    let mut width = 160u16;
    let mut interval = 2.0f64;
    let mut source = Source::Core;
    let mut exit_after_jump = ["ADMIRALTY_WATCH_EXIT_AFTER_JUMP", "ADMIRALTY_EXIT_AFTER_JUMP"].iter().any(|k| vars.get(*k).map(|v| v == "1").unwrap_or(false));
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "dump" => dump = true,
            "--exit-after-jump" => exit_after_jump = true,
            "--demo" => source = Source::Fixture(fixtures::demo()),
            "--demo-monitor" => source = Source::Fixture(fixtures::demo_monitor_only()),
            "--interval" | "--width" | "--fixture" | "--sandbox" => {
                let flag = args[i].clone();
                i += 1;
                let Some(v) = args.get(i) else { return fail(&format!("{flag} needs a value")) };
                match flag.as_str() {
                    "--interval" => interval = v.parse().unwrap_or(2.0),
                    "--width" => width = v.parse().unwrap_or(160),
                    "--fixture" => match load_fixture_file(&PathBuf::from(v)) {
                        Ok(b) => source = Source::Fixture(b),
                        Err(e) => return fail(&e),
                    },
                    _ => {
                        let dir = PathBuf::from(v);
                        if !dir.join("world.json").exists() {
                            return fail(&format!("{} is not a sandbox; create it with `admiralty sandbox create {}`", dir.display(), dir.display()));
                        }
                        // the children (fake claude, hooks) must see the sandbox too; a
                        // multiplexer chosen by the caller (a private or fake Herdr) stays,
                        // else the sandbox's own fake herdr, if it has one
                        for (k, val) in sandbox_env(&dir) {
                            if matches!(k.as_str(), "NIMITZ_MUX" | "NIMITZ_HERDR_BIN") && vars.contains_key(&k) {
                                continue;
                            }
                            std::env::set_var(&k, &val);
                            vars.insert(k, val);
                        }
                    }
                }
            }
            other => return fail(&format!("unknown argument {other}\n{USAGE}")),
        }
        i += 1;
    }
    let ascii = ["ADMIRALTY_ICONS", "ADMIRALTY_WATCH_ICONS"].iter().any(|k| vars.get(*k).map(|v| v.eq_ignore_ascii_case("ascii")).unwrap_or(false));
    let (ctx, state_path, make, fixed_now): (Ctx, Option<PathBuf>, run::SourceMaker, Option<f64>) = match source {
        Source::Core => {
            let env = nimitz_core::env::Env::from_vars(&vars);
            let state = vars.get("ADMIRALTY_STATE").map(PathBuf::from).unwrap_or_else(|| env.state.join("admiralty-ui.json"));
            let v2 = vars.clone();
            // outside a Herdr pane, the Herdr server the user's commands recorded
            let mux_vars = nimitz_core::active::user_mux_vars(&env, &vars);
            let ctx = Ctx { env, mux: nimitz_mux::detect(&mux_vars), live: true, exit_after_jump };
            (ctx, Some(state), Box::new(move || Box::new(CoreSource { first: true, ..CoreSource::detect(&v2) }) as Box<dyn DataSource>), None)
        }
        Source::Fixture(b) => {
            let ctx = Ctx { env: fixtures::demo_env(), mux: Box::new(nimitz_mux::none::NoMux), live: false, exit_after_jump };
            (ctx, None, Box::new(move || Box::new(FixtureSource { bundle: b }) as Box<dyn DataSource>), Some(fixtures::NOW))
        }
    };
    let mut app = App::new(ctx, state_path, ascii);
    // rows,cols the terminal never shows (Herdr's overlay: watch.sh sets 2,2)
    if let Some((r, c)) = vars.get("ADMIRALTY_INSET").and_then(|v| v.split_once(',')) {
        app.inset = (r.trim().parse().unwrap_or(0), c.trim().parse().unwrap_or(0));
    }
    app.fixed_now = fixed_now;
    if dump {
        let mut src = make();
        app.apply(src.load());
        for (l, _) in frames::frame_lines(&app.rows, width, usize::MAX, app.icons) {
            println!("{}", render::plain(&l));
        }
        return ExitCode::SUCCESS;
    }
    match run::run(app, make, Duration::from_secs_f64(interval.max(0.5))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e.to_string()),
    }
}

fn hook(event: String) -> ExitCode {
    use std::io::Read;
    let mut stdin = String::new();
    let _ = std::io::stdin().read_to_string(&mut stdin);
    let vars: HashMap<String, String> = std::env::vars().collect();
    let env = nimitz_core::env::Env::from_vars(&vars);
    let out = nimitz_core::hook::handle(&env, &event, &stdin, &vars);
    if !out.is_empty() {
        println!("{out}");
    }
    ExitCode::SUCCESS // a hook never fails the session
}

fn sandbox_cmd(args: &[String]) -> ExitCode {
    let herdr = args.iter().any(|a| a == "--herdr");
    let args: Vec<&String> = args.iter().filter(|a| *a != "--herdr").collect();
    let (Some(sub), Some(dir)) = (args.first(), args.get(1)) else { return fail("usage: admiralty sandbox create [--herdr]|env DIR") };
    let dir = PathBuf::from(dir);
    match sub.as_str() {
        "create" => {
            let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("admiralty"));
            match nimitz_core::sandbox::create(&dir, &exe).and_then(|()| if herdr { add_fake_herdr(&dir, &exe) } else { Ok(()) }) {
                Ok(()) => {
                    let d = dir.canonicalize().unwrap_or(dir);
                    println!("[OK] sandbox at {}", d.display());
                    println!("     watch it: admiralty --sandbox {}", d.display());
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            }
        }
        "env" => {
            for (k, v) in sandbox_env(&dir) {
                println!("export {k}={v}");
            }
            ExitCode::SUCCESS
        }
        _ => fail("usage: admiralty sandbox create|env DIR"),
    }
}

/// The sandbox's env, with its fake herdr when `sandbox create --herdr` made
/// one (the same rule as `nimitz sandbox env`).
fn sandbox_env(dir: &std::path::Path) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = nimitz_core::sandbox::env_for(dir).into_iter().collect();
    let herdr = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).join("bin").join("herdr");
    if herdr.is_file() {
        v.retain(|(k, _)| k != "NIMITZ_MUX");
        v.push(("NIMITZ_MUX".into(), "herdr".into()));
        v.push(("NIMITZ_HERDR_BIN".into(), herdr.to_string_lossy().into_owned()));
        v.push(("HERDR_ENV".into(), "1".into()));
    }
    v.sort();
    v
}

/// `sandbox create --herdr`: a fake herdr in the sandbox, with a small world.
fn add_fake_herdr(dir: &std::path::Path, exe: &std::path::Path) -> Result<(), String> {
    let base = dir.canonicalize().map_err(|e| e.to_string())?;
    let hb = base.join("herdr");
    nimitz_mux::fake::write_world(&hb, None).map_err(|e| e.to_string())?;
    let script = base.join("bin").join("herdr");
    std::fs::write(&script, format!("#!/bin/sh\nexec '{}' __fake-herdr '{}' \"$@\"\n", exe.display(), hb.display())).map_err(|e| e.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())
}

fn code(rc: i32) -> ExitCode {
    ExitCode::from(rc.clamp(0, 255) as u8)
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("[ERROR] {msg}");
    ExitCode::from(2)
}
