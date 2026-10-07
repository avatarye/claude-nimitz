//! Opt-in smoke test against the REAL Claude Code (`NIMITZ_E2E=1`), contracts.md
//! section 11.2. One cheap background ship under `~/.cache/nimitz-e2e/`: start,
//! stop, start again (a native resume keeps the session id), retire it, and
//! verify that no row and no session file is left. The admiral and commodore
//! open in a private fake herdr (nothing runs there), so the real Claude Code
//! runs only the ship. Cleanup runs even when an assertion fails.
//!
//! Hooks come from the ship folder's own `.claude/settings.json` (project
//! scope, inside the test folder), so nothing is installed into the user's
//! Claude Code. `NIMITZ_E2E_MODEL` (optional) sets the ship's model. The
//! session name is `[NIMITZ_E2E.SMOKE]-SHIP`: nimitz names sessions by
//! identity, so the `nimitz-e2e-*` rule applies to the folders.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const EXE: &str = env!("CARGO_BIN_EXE_nimitz");
const SHIP: &str = "NIMITZ_E2E/SMOKE/SHIP";

struct World {
    base: PathBuf,
    native: String,
}

impl World {
    fn cmd(&self, args: &[&str]) -> (i32, String) {
        let mut c = Command::new(EXE);
        for (k, _) in std::env::vars() {
            if k.starts_with("NIMITZ_") || k.starts_with("HERDR_") {
                c.env_remove(&k); // never inherit labels or a real mux
            }
        }
        let b = |x: &str| self.base.join(x).to_string_lossy().into_owned();
        c.env("NIMITZ_HOME", b("nimitz"))
            .env("NIMITZ_STATE", b("state"))
            .env("NIMITZ_MUX", "herdr")
            .env("NIMITZ_HERDR_BIN", b("herdr-bin"))
            .args(args)
            .stdin(Stdio::null());
        let o = c.output().expect("run nimitz");
        (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
    }
    fn ok(&self, args: &[&str]) -> String {
        let (code, out) = self.cmd(args);
        assert_eq!(code, 0, "nimitz {args:?}:\n{out}");
        out
    }
    fn ship(&self) -> Option<Value> {
        let (_, out) = self.cmd(&["status", "--json"]);
        let s: Value = serde_json::from_str(out.trim()).ok()?;
        s["fleets"].as_array()?.iter().flat_map(|f| f["squadrons"].as_array().cloned().unwrap_or_default()).flat_map(|q| q["ships"].as_array().cloned().unwrap_or_default()).find(|m| m["id"] == SHIP)
    }
    fn wait<T>(&self, secs: u64, mut f: impl FnMut() -> Option<T>) -> Option<T> {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        loop {
            if let Some(v) = f() {
                return Some(v);
            }
            if std::time::Instant::now() > end {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }
    /// Claude Code rows (job ids) whose folder is inside this test's folder.
    fn our_rows(&self) -> Vec<(String, String, bool)> {
        let home = PathBuf::from(std::env::var("HOME").unwrap()).join(".claude");
        let base = self.base.to_string_lossy().into_owned();
        let mut out = vec![];
        for e in std::fs::read_dir(home.join("jobs")).into_iter().flatten().flatten() {
            let Ok(raw) = std::fs::read(e.path().join("state.json")) else { continue };
            let Some(v) = nimitz_core::evidence::first_json(&raw) else { continue };
            if v["cwd"].as_str().map(|c| c.starts_with(&base)).unwrap_or(false) {
                out.push((e.file_name().to_string_lossy().into_owned(), v["sessionId"].as_str().unwrap_or("").to_string(), false));
            }
        }
        for e in std::fs::read_dir(home.join("sessions")).into_iter().flatten().flatten() {
            let Ok(raw) = std::fs::read(e.path()) else { continue };
            let Some(v) = nimitz_core::evidence::first_json(&raw) else { continue };
            if v["cwd"].as_str().map(|c| c.starts_with(&base)).unwrap_or(false) {
                out.push((v["jobId"].as_str().unwrap_or("").to_string(), v["sessionId"].as_str().unwrap_or("").to_string(), true));
            }
        }
        out
    }
}

impl Drop for World {
    /// Stop and remove every row this test made (only rows inside its folder), then the folder.
    fn drop(&mut self) {
        for (job, _, _) in self.our_rows() {
            if job.is_empty() {
                continue;
            }
            let _ = Command::new(&self.native).args(["stop", &job]).output();
            let _ = Command::new(&self.native).args(["rm", &job]).output();
        }
        let left = self.our_rows();
        if left.is_empty() {
            let _ = std::fs::remove_dir_all(&self.base);
            println!("[OK] e2e cleanup: no rows left; removed {}", self.base.display());
        } else {
            println!("[ERROR] e2e cleanup: rows left: {left:?} (folder kept: {})", self.base.display());
        }
    }
}

fn native() -> String {
    let home = PathBuf::from(std::env::var("HOME").unwrap());
    let mut vs: Vec<PathBuf> = std::fs::read_dir(home.join(".localtools/tools/claude-code"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("claude"))
        .filter(|p| p.is_file())
        .collect();
    vs.sort();
    vs.pop().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "claude".into())
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

#[test]
fn e2e_real_claude_code_ship_lifecycle() {
    if std::env::var("NIMITZ_E2E").ok().as_deref() != Some("1") {
        println!("[OK] skipped: set NIMITZ_E2E=1 to run against the real Claude Code");
        return;
    }
    let home = PathBuf::from(std::env::var("HOME").unwrap());
    let base = home.join(".cache/nimitz-e2e").join(format!("nimitz-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let w = World { base: base.canonicalize().unwrap(), native: native() };
    let b = w.base.clone();

    // A private fake herdr for the admiral and commodore panes.
    nimitz_mux::fake::write_world(&b.join("herdr"), None).unwrap();
    write(&b.join("herdr-bin"), &format!("#!/bin/sh\nexec '{EXE}' __fake-herdr '{}' \"$@\"\n", b.join("herdr").display()));
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(b.join("herdr-bin"), std::fs::Permissions::from_mode(0o755)).unwrap();

    // The ship folder: project hooks (nimitz hook), the test's state dir, a cheap model.
    let ship_dir = b.join("ship");
    let hooks: serde_json::Map<String, Value> = ["SessionStart", "SessionEnd", "UserPromptSubmit", "Stop"]
        .iter()
        .map(|e| (e.to_string(), json!([{"hooks": [{"type": "command", "command": EXE, "args": ["hook", e], "timeout": 5}]}])))
        .collect();
    // NIMITZ_E2E_MODEL picks a cheap model; unset, Claude Code's own default runs
    // (an org policy may refuse a named model).
    let mut settings = json!({"env": {"NIMITZ_STATE": b.join("state")}, "hooks": hooks});
    if let Some(m) = std::env::var("NIMITZ_E2E_MODEL").ok().filter(|m| !m.is_empty()) {
        settings["model"] = Value::from(m);
    }
    write(&ship_dir.join(".claude/settings.json"), &serde_json::to_string_pretty(&settings).unwrap());

    w.ok(&["fleet", "new", "NIMITZ_E2E", "--machine", "e2e", "--yes"]);
    w.ok(&["squadron", "new", "NIMITZ_E2E/SMOKE", "--brief", "nimitz e2e smoke test", "--yes"]);
    w.ok(&["ship", "new", SHIP, "--dir", ship_dir.to_str().unwrap(), "--prompt", "Reply with the single word OK.", "--mode", "default", "--yes"]);

    // 1. cold launch
    let out = w.ok(&["start", SHIP, "--yes"]);
    println!("{out}");
    let first = w.wait(30, || w.ship().filter(|m| !m["session"]["pid"].is_null())).expect("ship live after start");
    let sid = first["session"]["session_id"].as_str().unwrap().to_string();
    assert_eq!(first["session"]["name"], "[NIMITZ_E2E.SMOKE]-SHIP");
    assert_eq!(first["session"]["labels"]["NIMITZ_ID"], SHIP);
    w.wait(120, || w.ship().filter(|m| m["status"] == "Idle")).expect("ship finished its first turn");

    // 2. stop
    w.ok(&["stop", SHIP, "--yes"]);
    w.wait(30, || w.ship().filter(|m| m["session"]["pid"].is_null())).expect("ship stopped");

    // 3. start again: a native resume with no flags keeps the id
    let out = w.ok(&["start", SHIP, "--yes"]);
    println!("{out}");
    let again = w.wait(30, || w.ship().filter(|m| !m["session"]["pid"].is_null())).expect("ship live after resume");
    assert_eq!(again["session"]["session_id"], Value::from(sid.as_str()), "the resume kept the session id");
    assert!(again["extra_rows"].as_array().map(|r| r.is_empty()).unwrap_or(true), "no copy rows: {again}");

    // 4. retire: stop, remove the row, drop it from the config
    let out = w.ok(&["ship", "rm", SHIP, "--yes"]);
    println!("{out}");
    let left = w.wait(30, || Some(w.our_rows()).filter(|r| r.is_empty()));
    assert!(left.is_some(), "rows left after ship rm: {:?}", w.our_rows());
    assert!(w.ship().is_none(), "ship still in the config");
    println!("[OK] e2e: launch, stop, resume kept {sid}, rm left nothing");
}
