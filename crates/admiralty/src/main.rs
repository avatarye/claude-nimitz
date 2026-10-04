//! `admiralty-watch` (TUI), `admiralty-watch dump`, `admiralty-watch sandbox ...`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use admiralty::{env::Env, model, present, sandbox, sources::Cache, ui};

const USAGE: &str = "usage:
  admiralty-watch [--sandbox DIR] [--interval SECS] [--exit-after-jump]   live TUI
  admiralty-watch dump [--others] [--finished] [--sandbox DIR]   the same tree as text
  admiralty-watch sandbox create|tick|env DIR          fictional world for testing

env: ADMIRALTY_WATCH_ROOT, ADMIRALTY_WATCH_CLAUDE_HOME, ADMIRALTY_WATCH_CLAUDE_BIN,
     ADMIRALTY_WATCH_HOSTNAME, ADMIRALTY_WATCH_ACTION_LOG, ADMIRALTY_WATCH_ICONS=ascii,
     ADMIRALTY_WATCH_SOURCE=cli (read `claude agents --json` instead of the session files),
     ADMIRALTY_WATCH_EXIT_AFTER_JUMP=1 (same as --exit-after-jump: quit once a Herdr jump lands)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("__fake-claude") if args.len() >= 2 => return code(sandbox::fake_claude(&PathBuf::from(&args[1]), &args[2..])),
        Some("__fake-herdr") if args.len() >= 2 => return code(sandbox::fake_herdr(&PathBuf::from(&args[1]), &args[2..])),
        Some("sandbox") => return sandbox_cmd(&args[1..]),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let mut cmd = "tui";
    let (mut others, mut finished) = (false, false);
    let mut interval = 2.0f64;
    let mut vars: std::collections::HashMap<String, String> = std::env::vars().collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "dump" => cmd = "dump",
            "tui" => cmd = "tui",
            "--others" => others = true,
            "--finished" => finished = true,
            "--exit-after-jump" => {
                vars.insert("ADMIRALTY_WATCH_EXIT_AFTER_JUMP".into(), "1".into());
            }
            "--interval" => {
                i += 1;
                interval = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(2.0);
            }
            "--sandbox" => {
                i += 1;
                let Some(dir) = args.get(i).map(PathBuf::from) else { return fail("--sandbox needs a DIR") };
                if !dir.join("world.json").exists() {
                    return fail(&format!("{} is not a sandbox; create it with `admiralty-watch sandbox create {}`", dir.display(), dir.display()));
                }
                vars.extend(sandbox::env_for(&dir));
                // so `a`/`m` children (fake claude, attach) see the sandbox too
                for (k, v) in sandbox::env_for(&dir) {
                    std::env::set_var(k, v);
                }
            }
            other => return fail(&format!("unknown argument {other}\n{USAGE}")),
        }
        i += 1;
    }
    let env = Env::from_vars(&vars);
    if cmd == "dump" {
        let snap = model::snapshot(&env, &mut Cache::default());
        println!("{}", present::dump(&snap, others, finished));
        return ExitCode::SUCCESS;
    }
    match ui::run(env, Duration::from_secs_f64(interval.max(0.5))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e.to_string()),
    }
}

fn sandbox_cmd(args: &[String]) -> ExitCode {
    let (Some(sub), Some(dir)) = (args.first(), args.get(1)) else { return fail("usage: admiralty-watch sandbox create|tick|env DIR") };
    let dir = PathBuf::from(dir);
    match sub.as_str() {
        "create" => {
            let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("admiralty-watch"));
            match sandbox::create(&dir, &exe) {
                Ok(n) => {
                    let d = dir.canonicalize().unwrap_or(dir);
                    println!("[OK] sandbox at {} ({n} sessions)", d.display());
                    println!("     watch it:   admiralty-watch --sandbox {}", d.display());
                    println!("     animate it: admiralty-watch sandbox tick {}", d.display());
                    ExitCode::SUCCESS
                }
                Err(e) => fail(&e),
            }
        }
        "tick" => sandbox::tick(&dir),
        "env" => {
            let mut kv: Vec<_> = sandbox::env_for(&dir).into_iter().collect();
            kv.sort();
            for (k, v) in kv {
                println!("export {k}={v}");
            }
            ExitCode::SUCCESS
        }
        _ => fail("usage: admiralty-watch sandbox create|tick|env DIR"),
    }
}

fn code(rc: i32) -> ExitCode {
    ExitCode::from(rc.clamp(0, 255) as u8)
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("[ERROR] {msg}");
    ExitCode::from(2)
}
