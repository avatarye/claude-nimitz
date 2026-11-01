//! `nimitz sandbox create|tick|env <dir>`: a fake world (fake `claude`, and with
//! `--herdr` a fake `herdr`) for tests and demos. Nothing here touches the real
//! Claude Code home, the real NIMITZ_HOME or the user's Herdr.
use crate::{out, Ctx};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn sandbox(ctx: &Ctx) -> Result<i32, String> {
    let verb = ctx.args.need(0, "create|tick|env")?;
    let dir = PathBuf::from(ctx.args.need(1, "sandbox folder")?);
    ctx.args.at_most(2)?;
    match verb {
        "create" => {
            ctx.args.allow(&["--herdr"])?;
            create(&dir, ctx.args.flag("--herdr"))
        }
        "env" => {
            ctx.args.allow(&[])?;
            if !dir.join("world.json").is_file() {
                out::error(&format!("{} is not a sandbox; run `nimitz sandbox create {}`", dir.display(), dir.display()));
                return Ok(1);
            }
            let vars = env_vars(&dir);
            if out::json_mode() {
                out::json(&vars);
            } else {
                for (k, v) in &vars {
                    println!("export {k}={}", out::quote(v));
                }
            }
            Ok(0)
        }
        "tick" => {
            ctx.args.allow(&["--once"])?;
            if !dir.join("world.json").is_file() {
                out::error(&format!("{} is not a sandbox", dir.display()));
                return Ok(1);
            }
            if ctx.args.flag("--once") {
                let n = tick(&dir);
                out::ok(&format!("ticked {n} live session(s)"));
                return Ok(0);
            }
            out::ok(&format!("ticking {} every 2 s; ctrl-c stops", dir.display()));
            loop {
                tick(&dir);
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
        v => Err(format!("sandbox {v}: use create, tick or env")),
    }
}

fn create(dir: &Path, herdr: bool) -> Result<i32, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Err(e) = nimitz_core::sandbox::create(dir, &exe) {
        out::error(&e);
        return Ok(1);
    }
    let base = nimitz_core::os::canonical(&dir).map_err(|e| e.to_string())?;
    if herdr {
        let hb = base.join("herdr");
        std::fs::create_dir_all(&hb).map_err(|e| e.to_string())?;
        // the fake's `pane run` really runs the line (`exec`): a `claude attach
        // <job>` of the sandbox's fake claude wakes a parked job, as Herdr would
        let mut world = nimitz_mux::fake::default_world();
        world["exec"] = serde_json::json!(true);
        nimitz_mux::fake::write_world(&hb, Some(&world)).map_err(|e| e.to_string())?;
        nimitz_core::os::write_shim(&base.join("bin").join("herdr"), &exe, &["__fake-herdr".into(), hb.to_string_lossy().into_owned()]).map_err(|e| e.to_string())?;
    }
    out::ok(&format!("sandbox at {}{}", base.display(), if herdr { " (with a fake herdr)" } else { "" }));
    out::line(&format!("use it: eval \"$(nimitz sandbox env {})\"", base.display()));
    Ok(0)
}

/// The env that points nimitz at the sandbox (and at its fake herdr, if any).
pub fn env_vars(dir: &Path) -> BTreeMap<String, String> {
    let mut v: BTreeMap<String, String> = nimitz_core::sandbox::env_for(dir).into_iter().collect();
    let base = nimitz_core::os::canonical(&dir).unwrap_or_else(|_| dir.to_path_buf());
    let herdr = base.join("bin").join(nimitz_core::os::exe_name("herdr"));
    if herdr.is_file() {
        v.insert("NIMITZ_MUX".into(), "herdr".into());
        v.insert("NIMITZ_HERDR_BIN".into(), herdr.to_string_lossy().into_owned());
        v.insert("HERDR_ENV".into(), "1".into());
    }
    v
}

/// Flip each live session between busy and idle, so a viewer has something to show.
fn tick(dir: &Path) -> usize {
    let sessions = dir.join("claude_home").join("sessions");
    let now_ms = (nimitz_core::evidence::now() * 1000.0) as i64;
    let mut n = 0;
    for e in std::fs::read_dir(&sessions).into_iter().flatten().flatten() {
        let p = e.path();
        let Some(mut v) = std::fs::read(&p).ok().and_then(|b| nimitz_core::evidence::first_json(&b)) else { continue };
        let next = match v.get("status").and_then(Value::as_str) {
            Some("busy") => "idle",
            Some("idle") => "busy",
            _ => continue, // leave waiting sessions waiting
        };
        v["status"] = Value::from(next);
        v["statusUpdatedAt"] = Value::from(now_ms);
        if std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap_or_default()).is_ok() {
            n += 1;
        }
    }
    n
}
