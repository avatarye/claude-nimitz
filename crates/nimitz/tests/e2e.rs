//! Opt-in tests against the REAL Claude Code (`NIMITZ_E2E=1`): the risky
//! 0.4.0 cases of plan-v4 section 10, one test each, under the round-2
//! isolation rules (contracts.md section 11, rule 2):
//!
//! - every child runs scrubbed (`env_clear` + HOME, PATH, TERM, USER, SHELL,
//!   LANG): nothing of the parent session (CLAUDE_CODE_*, NIMITZ_*, HERDR_*)
//!   reaches a test process;
//! - one fleet `NIMITZ_E2E<n>` per test, every folder under
//!   `~/.cache/nimitz-e2e/nimitz-e2e-<n>-<pid>/`, private `NIMITZ_HOME` and
//!   `NIMITZ_STATE` under it; never the user's `~/.claude-nimitz` or
//!   `~/.local/state/nimitz`;
//! - each member folder's `.claude/settings.json` disables the live plugin
//!   and points every hook event at THIS build's `nimitz hook <Event>`;
//! - the model is the cheapest one the user's `availableModels` allows
//!   (`cheap_model`: haiku, else sonnet; `NIMITZ_E2E_MODEL` overrides), as
//!   `--model` on the command line (a wrapper around the
//!   native binary for cold launches; project settings `model` is not
//!   honoured), prompts are one word;
//! - never the Herdr CLI: attaching goes through `claude attach` in a private
//!   tmux server (`tmux -L nz-e2e-<pid>-<n>`), killed at the end;
//! - the test never stops, removes, attaches to, resumes or messages a session
//!   it did not start: it finds its own by the folders under its base;
//! - cleanup is a drop guard (runs when an assertion fails too): `claude stop`
//!   + `claude rm` of every row under the base, the transcript folders under
//!   `~/.claude/projects/*nimitz-e2e-<n>-<pid>*`, the pins the test added;
//!   then the live-fleet guard: every fleet-named session that is not a
//!   `NIMITZ_E2E*` one must still have the same id, pid and kind.
//!
//! Run: `NIMITZ_E2E=1 NIMITZ_E2E_LIVE_FLEETS=HOME,MAIN,ADMIRALTY cargo test -p nimitz --test e2e -- --test-threads=3`
//! (the fleets the guard watches; unset, every fleet that is not `NIMITZ_E2E*`).
//! The update case (#97) also needs `NIMITZ_E2E_OLD_BIN` (a 0.4.0 `nimitz`
//! build): `NIMITZ_E2E=1 NIMITZ_E2E_OLD_BIN=<path> cargo test -p nimitz --test e2e e2e_97`.
//! The slow cases (#6 crash window, #7 lead attach) are marked.
// Unix only for now: the real-Claude wrappers here are `#!/bin/sh` scripts.
#![cfg(unix)]
use serde_json::{json, Value};
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const EXE: &str = env!("CARGO_BIN_EXE_nimitz");
const SCRUB_KEEP: [&str; 4] = ["HOME", "PATH", "TERM", "LANG"];

fn gated() -> bool {
    if std::env::var("NIMITZ_E2E").ok().as_deref() == Some("1") {
        return true;
    }
    println!("[OK] skipped: set NIMITZ_E2E=1 to run against the real Claude Code");
    false
}

/// The cheapest model the settings let a session pick: Claude Code swaps a
/// `--model` outside `availableModels` for the account default (Opus here)
/// without failing, which made every e2e session an Opus one. The list is
/// the managed one when there is one (it replaces the rest), else the
/// user's; `haiku` when it allows that (or there is no list), else
/// `sonnet`, else its first entry (not necessarily cheap). `NIMITZ_E2E_MODEL`
/// overrides (a family name or a model id; `e2e_00` checks the transcript).
fn cheap_model() -> String {
    if let Some(m) = std::env::var("NIMITZ_E2E_MODEL").ok().filter(|m| !m.is_empty()) {
        return m;
    }
    let read = |p: PathBuf| -> Option<Vec<String>> {
        std::fs::read(p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v["availableModels"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_lowercase)).collect()))
    };
    let list = ["/etc/claude-code/managed-settings.json", "/Library/Application Support/ClaudeCode/managed-settings.json"]
        .into_iter()
        .find_map(|p| read(PathBuf::from(p)))
        .or_else(|| read(home().join(".claude").join("settings.json")))
        .unwrap_or_default();
    let allows = |fam: &str| list.is_empty() || list.iter().any(|m| m == fam || m.contains(&format!("-{fam}")) || m.starts_with(fam));
    for fam in ["haiku", "sonnet"] {
        if allows(fam) {
            return fam.into();
        }
    }
    list.first().cloned().unwrap_or_else(|| "sonnet".into())
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME"))
}

/// The native `claude`: newest `~/.localtools/tools/claude-code/*/claude`, else
/// the first on PATH, resolved (a wrapper script on PATH would add flags).
fn native() -> String {
    let mut vs: Vec<PathBuf> = std::fs::read_dir(home().join(".localtools").join("tools").join("claude-code"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("claude"))
        .filter(|p| p.is_file())
        .collect();
    vs.sort();
    if let Some(p) = vs.pop() {
        return p.to_string_lossy().into_owned();
    }
    let on_path = nimitz_core::env::which("claude").expect("claude on PATH");
    std::fs::canonicalize(&on_path).map(|p| p.to_string_lossy().into_owned()).unwrap_or(on_path)
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn executable(p: &Path, s: &str) {
    use std::os::unix::fs::PermissionsExt;
    write(p, s);
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn read_json(p: &Path) -> Value {
    std::fs::read(p).ok().and_then(|b| nimitz_core::evidence::first_json(&b)).unwrap_or(Value::Null)
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// One test's world: a fleet `NIMITZ_E2E<n>`, its folders, its tmux server.
struct E2e {
    n: u32,
    fleet: String,
    base: PathBuf,
    native: String,
    env: nimitz_core::env::Env,
    tmux: String,
    tmux_used: Cell<bool>,
    /// every fleet-named session that is not ours: name|id -> (session, pid, kind)
    live_before: BTreeMap<String, (String, Option<i64>, String)>,
    pins_before: Vec<String>,
    /// set at the end of a passing test: the folder is removed
    done: Cell<bool>,
    /// the nimitz the commands, the hooks and the members' PATH run: this
    /// build, or (the upgrade case) an older one first (`use_bin`)
    bin: std::cell::RefCell<String>,
}

impl E2e {
    fn new(n: u32) -> E2e {
        let base = home().join(".cache").join("nimitz-e2e").join(format!("nimitz-e2e-{n}-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let base = nimitz_core::os::canonical(&base).unwrap();
        for d in ["nimitz", "state", "bin", "ship"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        let native = native();
        // `nimitz` on the members' PATH is THIS build (a wrapper that logs each
        // call to `nimitz-calls.log` first); `claude-cheap` adds the model to a
        // cold launch (the native binary stays flag-free for wakes)
        executable(&base.join("bin").join("nimitz"), &format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec '{EXE}' \"$@\"\n", base.join("nimitz-calls.log").display()));
        executable(&base.join("bin").join("claude-cheap"), &format!("#!/bin/sh\nexec '{native}' --model '{}' \"$@\"\n", cheap_model()));
        let b = |x: &str| base.join(x).to_string_lossy().into_owned();
        let vars: HashMap<String, String> = [
            ("HOME", home().to_string_lossy().into_owned()),
            ("NIMITZ_HOME", b("nimitz")),
            ("NIMITZ_STATE", b("state")),
            ("NIMITZ_CLAUDE_BIN", b("bin/claude-cheap")),
            ("NIMITZ_NATIVE_CLAUDE_BIN", native.clone()),
            ("NIMITZ_MUX", "none".into()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let env = nimitz_core::env::Env::from_vars(&vars);
        let mut w = E2e {
            n,
            fleet: format!("NIMITZ_E2E{n}"),
            base,
            native,
            env,
            tmux: format!("nz-e2e-{}-{n}", std::process::id()),
            tmux_used: Cell::new(false),
            live_before: BTreeMap::new(),
            pins_before: vec![],
            done: Cell::new(false),
            bin: std::cell::RefCell::new(EXE.to_string()),
        };
        w.live_before = w.live_fleet();
        w.pins_before = w.pins();
        println!("[OK] e2e #{n}: base {}", w.base.display());
        w
    }

    // ------------------------------------------------------------ processes

    /// A scrubbed command: nothing of this process's env but HOME, PATH (this
    /// test's `bin` first), TERM, USER, SHELL, LANG.
    fn scrubbed(&self, bin: &str) -> Command {
        let mut c = Command::new(bin);
        c.env_clear();
        for k in SCRUB_KEEP {
            if let Ok(v) = std::env::var(k) {
                c.env(k, v);
            }
        }
        let path = std::env::var("PATH").unwrap_or_default();
        c.env("PATH", format!("{}:{path}", self.base.join("bin").display()));
        c.env("USER", std::env::var("USER").unwrap_or_else(|_| "user".into()));
        c.env("SHELL", "/bin/bash");
        c.env("TERM", std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".into()));
        c.env("LANG", std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into()));
        c.stdin(Stdio::null());
        c
    }

    fn run(mut c: Command) -> (i32, String) {
        let o = c.output().expect("spawn");
        (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
    }

    /// Run `bin` from now on: the commands, the hooks of the member folders
    /// written after this (`settings`), and the members' `nimitz` on PATH.
    fn use_bin(&self, bin: &str) {
        *self.bin.borrow_mut() = bin.to_string();
        executable(&self.base.join("bin").join("nimitz"), &format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec '{bin}' \"$@\"\n", self.base.join("nimitz-calls.log").display()));
    }

    fn nz_cmd(&self) -> Command {
        let bin = self.bin.borrow().clone();
        let mut c = self.scrubbed(&bin);
        let b = |x: &str| self.base.join(x).to_string_lossy().into_owned();
        c.env("NIMITZ_HOME", b("nimitz"))
            .env("NIMITZ_STATE", b("state"))
            .env("NIMITZ_CLAUDE_BIN", b("bin/claude-cheap"))
            .env("NIMITZ_NATIVE_CLAUDE_BIN", &self.native)
            .env("NIMITZ_MUX", "none")
            .env("NIMITZ_VERBOSE", "1");
        c
    }

    /// `nimitz <args>` as the user.
    fn nz(&self, args: &[&str]) -> (i32, String) {
        let mut c = self.nz_cmd();
        c.args(args);
        Self::run(c)
    }

    fn ok(&self, args: &[&str]) -> String {
        let (code, out) = self.nz(args);
        assert_eq!(code, 0, "nimitz {args:?}:\n{out}");
        out
    }

    /// The native `claude <args>` in `cwd`, scrubbed.
    fn claude(&self, cwd: &Path, args: &[&str]) -> (i32, String) {
        let mut c = self.scrubbed(&self.native);
        c.args(args).current_dir(cwd);
        Self::run(c)
    }

    // ------------------------------------------------------------ the fleet

    fn ship_dir(&self) -> PathBuf {
        self.base.join("ship")
    }
    fn ship(&self) -> String {
        format!("{}/SQ/SHIP", self.fleet)
    }
    fn fleet_dir(&self) -> PathBuf {
        self.base.join("nimitz").join(self.fleet.to_lowercase())
    }

    /// Project settings for a member folder: the live plugin off, every hook
    /// event the plugin registers pointed at this build, the test's state
    /// and home in the hook's env.
    fn settings(&self, dir: &Path) {
        // `command` + `args`, as the plugin's hooks.json: Claude Code then runs
        // the hook directly, so its parent pid (the session file's name) is
        // the Claude Code process; a command string runs through `sh -c`
        let exe = self.bin.borrow().clone();
        let one = |e: &str| json!([{"hooks": [{"type": "command", "command": exe, "args": ["hook", e], "timeout": 5}]}]);
        let mut hooks = serde_json::Map::new();
        for e in ["SessionStart", "SessionEnd", "UserPromptSubmit", "Stop", "Notification", "SubagentStart", "SubagentStop", "PermissionRequest"] {
            hooks.insert(e.into(), one(e));
        }
        hooks.insert(
            "PreToolUse".into(),
            json!([
                {"matcher": "SendMessage", "hooks": [{"type": "command", "command": exe, "args": ["hook", "PreToolUse"], "timeout": 5}]},
                {"matcher": "AskUserQuestion", "hooks": [{"type": "command", "command": exe, "args": ["hook", "PreToolUse"], "timeout": 5}]}
            ]),
        );
        // NIMITZ_MUX=none and no HERDR_ENV: a daemon-respawned worker inherits
        // the daemon's env, the user's live HERDR_* included (acceptance env
        // F6), and a member running `nimitz status` must never reach that Herdr
        let s = json!({
            "enabledPlugins": {"nimitz@claude-nimitz": false},
            "env": {"NIMITZ_STATE": self.base.join("state"), "NIMITZ_HOME": self.base.join("nimitz"), "NIMITZ_MUX": "none", "HERDR_ENV": "", "NIMITZ_HERDR_SESSION": ""},
            "hooks": hooks,
        });
        write(&dir.join(".claude").join("settings.json"), &serde_json::to_string_pretty(&s).unwrap());
    }

    /// Fleet, squadron SQ and ship SHIP (folder `ship`, the given prompt and
    /// permission mode); project settings in every member folder.
    fn fleet_with_ship(&self, prompt: &str, mode: &str) {
        self.ok(&["fleet", "new", &self.fleet, "--machine", "e2e", "--yes"]);
        self.ok(&["squadron", "new", &format!("{}/SQ", self.fleet), "--brief", "nimitz e2e", "--yes"]);
        self.ok(&["ship", "new", &self.ship(), "--dir", self.ship_dir().to_str().unwrap(), "--prompt", prompt, "--mode", mode, "--yes"]);
        self.settings(&self.ship_dir());
        self.settings(&self.fleet_dir().join("admiral"));
        self.settings(&self.fleet_dir().join("squadrons").join("sq"));
        self.leads_by_hand();
    }

    /// The admiral and the commodore, launched by hand with the exact labels
    /// and names nimitz gives them but a one-word prompt (a lead's own prompt
    /// runs `nimitz status`, a tool-using turn): a ship's start is top-down,
    /// and a lead that is live and clean is left alone (case 7 tests a lead's
    /// own launch).
    fn leads_by_hand(&self) {
        let leads = [(self.fleet.clone(), self.fleet_dir().join("admiral")), (format!("{}/SQ", self.fleet), self.fleet_dir().join("squadrons").join("sq"))];
        for (id, dir) in &leads {
            let idv = nimitz_core::Id(id.clone());
            let settings = self.no_mux(nimitz_core::claude::settings_json(&self.env, &idv, true));
            let (code, out) = self.claude(dir, &["--bg", "--model", &cheap_model(), "--name", &idv.name(), "--settings", &settings, OK_PROMPT]);
            assert_eq!(code, 0, "lead launch by hand: {out}");
        }
        for (id, _) in &leads {
            let m = self.wait(45, || self.member(id).filter(|m| !m["session"]["pid"].is_null())).expect("lead live");
            assert_eq!(m["session"]["labels"]["NIMITZ_ID"], id.as_str(), "{m}");
        }
    }

    /// A `--settings` JSON the harness writes, with the multiplexer off in its
    /// env (no test member reaches the user's Herdr, whatever its process inherits).
    fn no_mux(&self, settings: String) -> String {
        let mut v: Value = serde_json::from_str(&settings).unwrap();
        v["env"]["NIMITZ_MUX"] = Value::from("none");
        v["env"]["HERDR_ENV"] = Value::from("");
        v["env"]["NIMITZ_HERDR_SESSION"] = Value::from("");
        v.to_string()
    }

    /// Start the ship and wait until it is live and its first turn is over.
    fn start_ship_idle(&self) -> Value {
        let out = self.ok(&["start", &self.ship(), "--yes"]);
        println!("{out}");
        self.wait(45, || self.member(&self.ship()).filter(|m| !m["session"]["pid"].is_null())).expect("ship live after start");
        self.wait(180, || self.member(&self.ship()).filter(|m| m["status"] == "Idle")).expect("ship finished its first turn")
    }

    // ------------------------------------------------------------ reading

    fn status(&self) -> Value {
        let (_, out) = self.nz(&["status", "--json"]);
        serde_json::from_str(out.trim()).unwrap_or(Value::Null)
    }

    /// The member `id` in `status --json` (admiral, commodore or ship).
    fn member(&self, id: &str) -> Option<Value> {
        let s = self.status();
        for f in s["fleets"].as_array()? {
            if f["admiral"]["id"] == id {
                return Some(f["admiral"].clone());
            }
            for q in f["squadrons"].as_array().cloned().unwrap_or_default() {
                if q["commodore"]["id"] == id {
                    return Some(q["commodore"].clone());
                }
                if let Some(m) = q["ships"].as_array().and_then(|s| s.iter().find(|m| m["id"] == id)) {
                    return Some(m.clone());
                }
            }
        }
        None
    }

    /// Every issue code in `status --json` (fleet, squadron, member, top).
    fn issue_codes(&self) -> Vec<(String, String)> {
        fn walk(v: &Value, out: &mut Vec<(String, String)>) {
            match v {
                Value::Object(m) => {
                    if let (Some(c), Some(msg)) = (m.get("code").and_then(|c| c.as_str()), m.get("message").and_then(|c| c.as_str())) {
                        if m.contains_key("severity") {
                            out.push((c.to_string(), msg.to_string()));
                        }
                    }
                    m.values().for_each(|x| walk(x, out));
                }
                Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
                _ => {}
            }
        }
        let mut out = vec![];
        walk(&self.status(), &mut out);
        out
    }

    fn registry(&self) -> Value {
        read_json(&self.base.join("state").join("registry.json"))
    }

    fn events(&self) -> Vec<Value> {
        std::fs::read_to_string(self.base.join("state").join("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn session_starts(&self, sid: &str) -> Vec<Value> {
        self.events().into_iter().filter(|e| e["event"] == "SessionStart" && e["session_id"] == sid).collect()
    }

    fn deliveries(&self) -> Value {
        read_json(&self.base.join("state").join("rules-delivered.json"))
    }

    /// Claude Code's pin list (`~/.claude/jobs/pins.json`).
    fn pins(&self) -> Vec<String> {
        read_json(&nimitz_core::pins::pins_path(&self.env)).as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
    }

    /// Claude Code rows whose folder is under this test's base: (job, session, live, folder).
    fn rows(&self) -> Vec<(String, String, bool, String)> {
        let ch = home().join(".claude");
        let base = self.base.to_string_lossy().into_owned();
        let under = |v: &Value| v["cwd"].as_str().map(|c| c == base || c.starts_with(&format!("{base}/"))).unwrap_or(false);
        let mut out = vec![];
        for e in std::fs::read_dir(ch.join("jobs")).into_iter().flatten().flatten() {
            let v = read_json(&e.path().join("state.json"));
            if under(&v) {
                out.push((e.file_name().to_string_lossy().into_owned(), v["sessionId"].as_str().unwrap_or("").to_string(), false, v["cwd"].as_str().unwrap_or("").to_string()));
            }
        }
        for e in std::fs::read_dir(ch.join("sessions")).into_iter().flatten().flatten() {
            if e.path().extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let v = read_json(&e.path());
            if under(&v) && v["pid"].as_i64().map(nimitz_core::evidence::pid_alive).unwrap_or(false) {
                out.push((v["jobId"].as_str().unwrap_or("").to_string(), v["sessionId"].as_str().unwrap_or("").to_string(), true, v["cwd"].as_str().unwrap_or("").to_string()));
            }
        }
        out
    }

    /// Job rows (sorted short ids) under the base, or in one folder.
    fn job_rows(&self) -> Vec<String> {
        self.jobs_in(&self.base)
    }
    fn ship_rows(&self) -> Vec<String> {
        self.jobs_in(&self.ship_dir())
    }
    fn jobs_in(&self, dir: &Path) -> Vec<String> {
        let d = dir.to_string_lossy().into_owned();
        let mut v: Vec<String> = self.rows().into_iter().filter(|(j, _, live, cwd)| !live && !j.is_empty() && (cwd == &d || dir == self.base)).map(|(j, _, _, _)| j).collect();
        v.sort();
        v.dedup();
        v
    }

    /// `claude agents --json --all` rows (scrubbed, native).
    fn agents(&self) -> Vec<nimitz_core::evidence::SessionRow> {
        let mut c = self.scrubbed(&self.native);
        c.args(["agents", "--json", "--all"]);
        let (_, out) = Self::run(c);
        nimitz_core::evidence::first_json(out.as_bytes()).map(|v| nimitz_core::evidence::parse_cli_rows(&v)).unwrap_or_default()
    }

    /// The live-fleet guard's view: every session named like a fleet member
    /// (`[F]-ADMIRAL`, `[F.SQ]-X`) of the fleets `NIMITZ_E2E_LIVE_FLEETS` names
    /// (comma-separated; the user's live fleets, e.g. `HOME,MAIN,ADMIRALTY`),
    /// else of every fleet that is not a `NIMITZ_E2E*` one (other agents'
    /// test fleets then count too, and their own starts trip the guard).
    fn live_fleet(&self) -> BTreeMap<String, (String, Option<i64>, String)> {
        let only: Vec<String> = std::env::var("NIMITZ_E2E_LIVE_FLEETS").ok().map(|v| v.split(',').map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect()).unwrap_or_default();
        let tracked = |fleet: &str| if only.is_empty() { !fleet.starts_with("NIMITZ_E2E") } else { only.iter().any(|f| f == fleet) };
        self.agents()
            .into_iter()
            .filter(|r| nimitz_core::sessions::id_from_name(&r.name).map(|id| tracked(id.fleet())).unwrap_or(false))
            .map(|r| (format!("{}|{}", r.name, r.short_id.clone().unwrap_or_else(|| r.session_id.chars().take(8).collect())), (r.session_id, r.pid, r.kind)))
            .collect()
    }

    fn transcript(&self, cwd: &Path, sid: &str) -> Option<PathBuf> {
        nimitz_core::evidence::transcript(&self.env, &mut nimitz_core::evidence::FileCache::default(), Some(cwd.to_str().unwrap()), sid)
    }

    fn wait<T>(&self, s: u64, mut f: impl FnMut() -> Option<T>) -> Option<T> {
        let end = std::time::Instant::now() + secs(s);
        loop {
            if let Some(v) = f() {
                return Some(v);
            }
            if std::time::Instant::now() > end {
                return None;
            }
            std::thread::sleep(Duration::from_millis(1500));
        }
    }

    // ------------------------------------------------------------ tmux attach

    fn tx(&self, args: &[&str]) -> (i32, String) {
        self.tmux_used.set(true);
        let mut c = self.scrubbed("tmux");
        c.arg("-L").arg(&self.tmux).args(args);
        Self::run(c)
    }

    fn pane(&self) -> String {
        self.tx(&["capture-pane", "-p", "-t", "e2e"]).1
    }

    /// `claude attach <job>` in a private tmux window started in `cwd`; waits
    /// until the pane shows the session (its name in the status line).
    fn attach(&self, cwd: &Path, job: &str, name: &str) {
        let _ = self.tx(&["kill-session", "-t", "e2e"]); // a second attach in one test
        let (code, out) = self.tx(&["new-session", "-d", "-s", "e2e", "-x", "180", "-y", "50", "-c", cwd.to_str().unwrap()]);
        assert_eq!(code, 0, "tmux new-session: {out}");
        self.tx(&["send-keys", "-t", "e2e", "-l", &format!("'{}' attach {job}", self.native)]);
        self.tx(&["send-keys", "-t", "e2e", "Enter"]);
        let shown = self.wait(45, || Some(self.pane()).filter(|p| p.contains(name) && p.contains('\u{276f}')));
        assert!(shown.is_some(), "claude attach {job} did not show {name} with a prompt:\n{}", self.pane());
        std::thread::sleep(secs(2));
    }

    /// Run `line` in a fresh private tmux window started in `cwd` (an
    /// interactive `claude`, say); waits until the pane shows `want`.
    fn tmux_run(&self, cwd: &Path, line: &str, want: &str) {
        let _ = self.tx(&["kill-session", "-t", "e2e"]);
        let (code, out) = self.tx(&["new-session", "-d", "-s", "e2e", "-x", "180", "-y", "50", "-c", cwd.to_str().unwrap()]);
        assert_eq!(code, 0, "tmux new-session: {out}");
        self.tx(&["send-keys", "-t", "e2e", "-l", line]);
        self.tx(&["send-keys", "-t", "e2e", "Enter"]);
        let shown = self.wait(60, || Some(self.pane()).filter(|p| p.contains(want)));
        assert!(shown.is_some(), "{line} did not show {want}:\n{}", self.pane());
        std::thread::sleep(secs(3));
    }

    /// The issues of member `id` in `status --json`: (code, fix).
    fn member_issues(&self, id: &str) -> Vec<(String, String)> {
        self.member(id).and_then(|m| m["issues"].as_array().cloned()).unwrap_or_default().iter().map(|i| (i["code"].as_str().unwrap_or("").to_string(), i["fix"].as_str().unwrap_or("").to_string())).collect()
    }

    /// Type one line into the attached session.
    fn type_line(&self, text: &str) {
        self.tx(&["send-keys", "-t", "e2e", "-l", text]);
        std::thread::sleep(Duration::from_millis(800));
        self.tx(&["send-keys", "-t", "e2e", "Enter"]);
    }

    /// ctrl-z: the attach ends, the job runs on.
    fn detach(&self) {
        self.tx(&["send-keys", "-t", "e2e", "C-z"]);
        let at_shell = |p: &String| p.lines().rev().find(|l| !l.trim().is_empty()).map(|l| l.trim_end().ends_with('$')).unwrap_or(false);
        let back = self.wait(20, || Some(self.pane()).filter(at_shell));
        assert!(back.is_some(), "ctrl-z did not detach to the shell:\n{}", self.pane());
    }

    fn finish(&self) {
        self.done.set(true);
    }
}

impl Drop for E2e {
    /// Stop and remove every row under the base, the transcripts, the pins
    /// this test added; kill the tmux server; then the live-fleet guard.
    fn drop(&mut self) {
        if self.tmux_used.get() {
            let _ = self.tx(&["kill-server"]);
        }
        let mut jobs: Vec<String> = self.rows().into_iter().map(|(j, _, _, _)| j).filter(|j| !j.is_empty()).collect();
        jobs.sort();
        jobs.dedup();
        for job in &jobs {
            let _ = self.claude(&self.base, &["stop", job]);
            let _ = self.claude(&self.base, &["rm", job]);
        }
        // a row the daemon writes a moment late (a spare it claimed in a test
        // folder, a job settling): a second pass after a short wait
        std::thread::sleep(secs(3));
        let mut late: Vec<String> = self.rows().into_iter().map(|(j, _, _, _)| j).filter(|j| !j.is_empty()).collect();
        late.sort();
        late.dedup();
        for job in &late {
            let _ = self.claude(&self.base, &["stop", job]);
            let _ = self.claude(&self.base, &["rm", job]);
        }
        // the daemon removes a stopped row a moment after `rm` returns: wait for
        // it (up to 20 s); a row still live gets one more stop and rm, once
        let mut retried: Vec<String> = vec![];
        let until = std::time::Instant::now() + secs(20);
        loop {
            let rows = self.rows();
            if rows.is_empty() || std::time::Instant::now() >= until {
                break;
            }
            for (job, _, live, _) in &rows {
                if *live && !job.is_empty() && !retried.contains(job) {
                    let _ = self.claude(&self.base, &["stop", job]);
                    let _ = self.claude(&self.base, &["rm", job]);
                    retried.push(job.clone());
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let jobs: Vec<String> = jobs.into_iter().chain(late).chain(retried).collect();
        // a process with no job (none expected): end it so nothing of ours lives on
        for _ in self.rows().iter().filter(|(j, _, live, _)| *live && j.is_empty()) {
            println!("[WARN] e2e #{}: a live process with no job under {} is left", self.n, self.base.display());
        }
        for own in nimitz_core::pins::own(&self.env) {
            let _ = nimitz_core::pins::unpin(&self.env, &own);
        }
        let tag = nimitz_core::evidence::project_dir_name(&self.base.to_string_lossy());
        for e in std::fs::read_dir(home().join(".claude").join("projects")).into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().contains(&tag) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
        let left = self.rows();
        let pins_now = self.pins();
        let mut errors = vec![];
        if !left.is_empty() {
            errors.push(format!("rows left: {left:?}"));
        }
        // pins: none of ours left (other tests and agents add and remove their
        // own meanwhile; case 7 checks that nobody else's pin goes)
        if let Some(p) = pins_now.iter().find(|p| jobs.contains(p)) {
            errors.push(format!("pin of our job {p} left in pins.json: {:?} -> {pins_now:?}", self.pins_before));
        }
        let live_now = self.live_fleet();
        for k in self.live_before.keys().chain(live_now.keys()).collect::<std::collections::BTreeSet<_>>() {
            match (self.live_before.get(k), live_now.get(k)) {
                (Some(a), Some(b)) if a != b => errors.push(format!("live fleet changed: {k}: {a:?} -> {b:?}")),
                (None, Some(b)) => errors.push(format!("live fleet: NEW {k} {b:?}")),
                (Some(a), None) => errors.push(format!("live fleet: GONE {k} {a:?}")),
                _ => {}
            }
        }
        if errors.is_empty() {
            if self.done.get() {
                let _ = std::fs::remove_dir_all(&self.base);
            }
            println!("[OK] e2e #{}: cleanup done ({} rows stopped and removed); live fleet unchanged", self.n, jobs.len());
        } else {
            println!("[ERROR] e2e #{}: cleanup (folder kept: {}):\n  {}", self.n, self.base.display(), errors.join("\n  "));
            if !std::thread::panicking() {
                panic!("e2e #{} cleanup: {}", self.n, errors.join("; "));
            }
        }
    }
}

const OK_PROMPT: &str = "Reply with the single word OK.";

// ------------------------------------------------------------------ cases

/// 1. A launch binds job and session (via launch), and the rules are delivered.
#[test]
fn e2e_01_launch_binds_job_and_session_and_delivers_rules() {
    if !gated() {
        return;
    }
    let w = E2e::new(1);
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    assert_eq!(m["session"]["name"], format!("[{}.SQ]-SHIP", w.fleet));
    assert!(matches!(m["session"]["kind"].as_str(), Some("background" | "bg")), "{m}");
    assert_eq!(m["session"]["labels"]["NIMITZ_ID"], w.ship());
    // the registry: the job is the anchor, the session the conversation now
    let r = w.registry();
    assert_eq!(r["version"], nimitz_core::REGISTRY_VERSION, "a fresh registry is at the current format");
    let me = &r["identities"][w.ship()];
    assert_eq!(me["job"], job.as_str(), "{me}");
    assert_eq!(me["session"], sid.as_str());
    let lineage = me["lineage"].as_array().unwrap();
    assert_eq!(lineage.len(), 1, "{lineage:?}");
    assert_eq!((lineage[0]["via"].as_str(), lineage[0]["job"].as_str(), lineage[0]["session_id"].as_str()), (Some("launch"), Some(job.as_str()), Some(sid.as_str())));
    // the hook's line: bound, with the job, the rules version
    let starts = w.session_starts(&sid);
    assert_eq!(starts.len(), 1, "{starts:?}");
    assert_eq!((starts[0]["source"].as_str(), starts[0]["binding"].as_str(), starts[0]["job"].as_str()), (Some("startup"), Some("bound"), Some(job.as_str())));
    assert_eq!(starts[0]["rules"], nimitz_core::rules::VERSION);
    // the delivery record, per job; the rules reached the session
    let d = w.deliveries();
    assert_eq!((d[&job]["version"].as_u64(), d[&job]["via"].as_str()), (Some(nimitz_core::rules::VERSION as u64), Some("SessionStart")), "{d}");
    // the transcript records the SessionStart context as an attachment line;
    // it can land a moment after the turn ends
    let t = w.wait(30, || w.transcript(&w.ship_dir(), &sid)).expect("transcript");
    let seen = w.wait(30, || std::fs::read_to_string(&t).ok().filter(|text| text.contains("nimitz fleet rules")));
    assert!(seen.is_some(), "the rules text is not in the transcript {}", t.display());
    assert!(w.issue_codes().iter().all(|(c, _)| c == "cc.version"), "issues: {:?}", w.issue_codes());
    w.finish();
}

/// 2. `/clear` inside the job: the same job, a new session, lineage via clear.
#[test]
fn e2e_02_clear_moves_the_session_inside_the_same_job() {
    if !gated() {
        return;
    }
    let w = E2e::new(2);
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    w.attach(&w.ship_dir(), &job, &format!("[{}.SQ]-SHIP", w.fleet));
    w.type_line("/clear");
    let cleared = w
        .wait(60, || {
            w.events().into_iter().rev().find(|e| e["event"] == "SessionStart" && e["source"] == "clear" && e["labels"]["NIMITZ_ID"] == w.ship())
        })
        .expect("a SessionStart clear after /clear");
    w.detach();
    let new_sid = cleared["session_id"].as_str().unwrap().to_string();
    assert_ne!(new_sid, sid, "/clear starts a new conversation");
    assert_eq!((cleared["job"].as_str(), cleared["binding"].as_str()), (Some(job.as_str()), Some("bound")), "{cleared}");
    let r = w.registry();
    let me = &r["identities"][w.ship()];
    assert_eq!((me["job"].as_str(), me["session"].as_str()), (Some(job.as_str()), Some(new_sid.as_str())), "{me}");
    let lineage = me["lineage"].as_array().unwrap();
    assert_eq!(lineage.len(), 2, "{lineage:?}");
    assert_eq!((lineage[1]["via"].as_str(), lineage[1]["job"].as_str(), lineage[1]["session_id"].as_str()), (Some("clear"), Some(job.as_str()), Some(new_sid.as_str())));
    let now = w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["session_id"] == new_sid.as_str())).expect("status follows the new conversation");
    assert_eq!(now["session"]["short_id"], job.as_str());
    assert!(now["extra_rows"].as_array().unwrap().is_empty(), "no copy rows: {now}");
    assert!(!w.issue_codes().iter().any(|(c, _)| matches!(c.as_str(), "dup.live" | "copy" | "copy.rows" | "lineage.broken")), "{:?}", w.issue_codes());
    w.finish();
}

/// 3. stop, then start: a native `--bg --resume <id>`, the same job, the same
/// session, via resume; never `-c`.
#[test]
fn e2e_03_stop_then_start_resumes_the_exact_id() {
    if !gated() {
        return;
    }
    let w = E2e::new(3);
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    w.ok(&["stop", &w.ship(), "--yes"]);
    w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["pid"].is_null())).expect("ship stopped");
    // the plan: the native binary, `--bg --resume <sid>`, no other flag
    let plan = w.ok(&["start", &w.ship(), "--plan"]);
    println!("{plan}");
    let want = format!("{} --bg --resume {sid}", w.native);
    assert!(plan.contains(&want), "the plan is not a flag-free native resume:\n{plan}");
    for bad in [" -c ", " -c\n", "--continue", "--name", "--settings"] {
        assert!(!plan.contains(bad), "{bad:?} in a resume plan:\n{plan}");
    }
    let out = w.ok(&["start", &w.ship(), "--yes"]);
    println!("{out}");
    assert!(out.contains(&format!("backgrounded \u{b7} {job}")), "Claude Code did not wake job {job}:\n{out}");
    let again = w.wait(45, || w.member(&w.ship()).filter(|m| !m["session"]["pid"].is_null())).expect("ship live after resume");
    assert_eq!((again["session"]["short_id"].as_str(), again["session"]["session_id"].as_str()), (Some(job.as_str()), Some(sid.as_str())), "{again}");
    assert_eq!(w.ship_rows(), vec![job.clone()], "one row");
    let starts = w.session_starts(&sid);
    assert_eq!(starts.len(), 2, "{starts:?}");
    assert_eq!((starts[1]["source"].as_str(), starts[1]["job"].as_str()), (Some("resume"), Some(job.as_str())), "{}", starts[1]);
    // the lineage keeps how the conversation was born: a wake of the same
    // conversation adds no entry (registry::bind_into)
    let me = &w.registry()["identities"][w.ship()];
    let lineage = me["lineage"].as_array().unwrap();
    assert_eq!((me["job"].as_str(), me["session"].as_str()), (Some(job.as_str()), Some(sid.as_str())));
    assert_eq!(lineage.len(), 1, "{lineage:?}");
    assert_eq!((lineage[0]["via"].as_str(), lineage[0]["session_id"].as_str()), (Some("launch"), Some(sid.as_str())));
    w.finish();
}

/// 4. The transcript deleted, then start: `history.lost`, no launch; `--fresh`
/// makes a cold launch with a new lineage entry.
#[test]
fn e2e_04_lost_transcript_refuses_and_fresh_relaunches_cold() {
    if !gated() {
        return;
    }
    let w = E2e::new(4);
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    w.ok(&["stop", &w.ship(), "--yes"]);
    w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["pid"].is_null())).expect("ship stopped");
    let t = w.transcript(&w.ship_dir(), &sid).expect("transcript");
    std::fs::remove_file(&t).unwrap();
    assert!(w.transcript(&w.ship_dir(), &sid).is_none());
    let (code, out) = w.nz(&["start", &w.ship(), "--yes"]);
    println!("{out}");
    assert_ne!(code, 0, "a start with no transcript must refuse:\n{out}");
    assert!(out.contains("history.lost") && out.contains("--fresh"), "{out}");
    assert!(w.member(&w.ship()).unwrap()["session"]["pid"].is_null(), "nothing launched");
    assert_eq!(w.ship_rows(), vec![job.clone()], "the old row stays, no new one");
    // --fresh: a cold launch, a new conversation, a new lineage entry via launch
    let out = w.ok(&["start", &w.ship(), "--fresh", "--yes"]);
    println!("{out}");
    let fresh = w.wait(45, || w.member(&w.ship()).filter(|m| !m["session"]["pid"].is_null())).expect("ship live after --fresh");
    let (job2, sid2) = (fresh["session"]["short_id"].as_str().unwrap().to_string(), fresh["session"]["session_id"].as_str().unwrap().to_string());
    assert_ne!(sid2, sid);
    assert_ne!(job2, job);
    let me = &w.registry()["identities"][w.ship()];
    let lineage = me["lineage"].as_array().unwrap();
    assert_eq!((me["job"].as_str(), me["session"].as_str()), (Some(job2.as_str()), Some(sid2.as_str())), "{me}");
    assert_eq!(lineage.last().unwrap()["via"], "launch");
    assert_eq!(lineage.len(), 2, "{lineage:?}");
    assert_eq!(w.wait(30, || Some(w.ship_rows()).filter(|r| *r == vec![job2.clone()])), Some(vec![job2.clone()]), "the old row is dropped after --fresh: {:?}", w.ship_rows());
    w.wait(180, || w.member(&w.ship()).filter(|m| m["status"] == "Idle")).expect("fresh ship idle");
    w.finish();
}

/// 5. A resume of a live job is never attempted: start of a live member is a
/// no-op. A hand `--bg --resume <sid>` of the live job is a copy; sweep removes it.
#[test]
fn e2e_05_live_member_start_is_a_noop_and_a_hand_resume_is_a_swept_copy() {
    if !gated() {
        return;
    }
    let w = E2e::new(5);
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, sid, pid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string(), m["session"]["pid"].as_i64().unwrap());
    let plan = w.ok(&["start", &w.ship(), "--plan"]);
    assert!(!plan.contains("--resume") && !plan.contains("--bg"), "a live member's start plans a launch:\n{plan}");
    let out = w.ok(&["start", &w.ship(), "--yes"]);
    println!("{out}");
    let still = w.member(&w.ship()).unwrap();
    assert_eq!((still["session"]["short_id"].as_str(), still["session"]["pid"].as_i64()), (Some(job.as_str()), Some(pid)), "the live member was touched: {still}");
    assert_eq!(w.ship_rows(), vec![job.clone()]);
    // by hand: a resume of the live job starts a copy (2.1.292)
    let (code, out) = w.claude(&w.ship_dir(), &["--bg", "--resume", &sid, "--model", &cheap_model()]);
    println!("hand resume ({code}): {out}");
    let copy = nimitz_core::claude::copy_started(&out).or_else(|| nimitz_core::claude::job_started(&out)).expect("the hand resume named a job");
    assert_ne!(copy, job, "Claude Code resumed the live job in place?");
    let seen = w
        .wait(45, || w.member(&w.ship()).filter(|m| m["extra_rows"].as_array().unwrap().iter().any(|r| r["short_id"] == copy.as_str() && !r["copy"].is_null())))
        .expect("status shows the copy");
    let row = seen["extra_rows"].as_array().unwrap().iter().find(|r| r["short_id"] == copy.as_str()).unwrap();
    println!("copy row: {row}");
    assert_eq!((seen["session"]["short_id"].as_str(), seen["session"]["session_id"].as_str()), (Some(job.as_str()), Some(sid.as_str())), "the member keeps its job");
    let codes = w.issue_codes();
    assert!(codes.iter().any(|(c, m)| c == "copy" && m.contains(&copy)), "no copy issue for {copy}: {codes:?}");
    assert!(!codes.iter().any(|(c, _)| matches!(c.as_str(), "dup.live" | "name.taken" | "held")), "{codes:?}");
    let out = w.ok(&["sweep", &w.ship(), "--yes"]);
    println!("{out}");
    assert_eq!(w.wait(30, || Some(w.ship_rows()).filter(|r| *r == vec![job.clone()])), Some(vec![job.clone()]), "the copy is gone: {:?}", w.ship_rows());
    assert!(w.member(&w.ship()).unwrap()["session"]["pid"].is_i64(), "the member is still live");
    w.finish();
}

/// 6. (slow: the crash window) kill -9 of a working member: afterwards the
/// member is live again under the same job, or parked; never a duplicate.
#[cfg(unix)]
#[test]
fn e2e_06_slow_kill_9_of_a_working_member_never_makes_a_duplicate() {
    if !gated() {
        return;
    }
    let w = E2e::new(6);
    w.fleet_with_ship("Count from 1 to 400, one number per line, in your reply. No tools. Then reply DONE.", "default");
    let out = w.ok(&["start", &w.ship(), "--yes"]);
    println!("{out}");
    let m = w.wait(45, || w.member(&w.ship()).filter(|m| !m["session"]["pid"].is_null())).expect("ship live");
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    let busy = w.wait(60, || w.member(&w.ship()).filter(|m| m["status"] == "Busy")).unwrap_or(m);
    let pid = busy["session"]["pid"].as_i64().unwrap();
    println!("[OK] killing pid {pid} (status {})", busy["status"]);
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    std::thread::sleep(secs(1));
    let state = read_json(&home().join(".claude").join("jobs").join(&job).join("state.json"));
    println!("state after kill -9: {} ", state["state"]);
    let t0 = std::time::Instant::now();
    let (code, out) = w.nz(&["start", &w.ship(), "--yes"]);
    println!("start after kill ({code}, {:.1}s):\n{out}", t0.elapsed().as_secs_f64());
    // the crash window, never a lost-history refusal (the transcript may not be
    // on disk yet; the daemon restarts the job, else nimitz resumes it)
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("history.lost") && !out.contains("--fresh"), "{out}");
    // settle, then: one row, the same job; live or parked; never two
    std::thread::sleep(secs(15));
    let now = w.member(&w.ship()).unwrap();
    println!("after: status {} session {}", now["status"], now["session"]);
    assert_eq!(w.ship_rows(), vec![job.clone()], "exactly one row for the ship");
    assert_eq!(now["session"]["short_id"], job.as_str(), "{now}");
    assert_eq!(now["session"]["session_id"], sid.as_str(), "the conversation did not move");
    assert!(now["extra_rows"].as_array().unwrap().is_empty(), "copy rows: {now}");
    let codes = w.issue_codes();
    assert!(!codes.iter().any(|(c, _)| matches!(c.as_str(), "dup.live" | "name.taken" | "copy" | "copy.rows")), "{codes:?}");
    let live = w.rows().iter().filter(|(_, _, live, cwd)| *live && Path::new(cwd) == w.ship_dir()).count();
    assert!(live <= 1, "two live processes: {:?}", w.rows());
    assert!(matches!(now["status"].as_str(), Some("Idle" | "Busy" | "Parked")), "{now}");
    w.finish();
}

/// 7. (slow: a lead's turn and an attach) A lead is a background job: launch,
/// pinned under the lock, attach in tmux, ctrl-z, retire unpins its own id only.
#[test]
fn e2e_07_slow_lead_is_a_pinned_background_job_attach_detach_unpin() {
    if !gated() {
        return;
    }
    let w = E2e::new(7);
    w.ok(&["fleet", "new", &w.fleet, "--machine", "e2e", "--yes"]);
    w.settings(&w.fleet_dir().join("admiral"));
    let pins0 = w.pins();
    let out = w.ok(&["start", &w.fleet, "--yes"]);
    println!("{out}");
    assert!(out.contains("pinned"), "the start did not pin the lead:\n{out}");
    let adm = w.wait(45, || w.member(&w.fleet).filter(|m| !m["session"]["pid"].is_null())).expect("admiral live");
    let job = adm["session"]["short_id"].as_str().unwrap().to_string();
    assert_eq!(adm["session"]["name"], format!("[{}]-ADMIRAL", w.fleet));
    assert!(matches!(adm["session"]["kind"].as_str(), Some("background" | "bg")), "{adm}");
    assert_eq!(adm["session"]["pinned"], true, "{adm}");
    // the pin: added to Claude Code's file, nothing else changed, the lock released
    let pins1 = w.pins();
    assert!(pins1.contains(&job), "{pins1:?}");
    assert!(pins0.iter().all(|p| pins1.contains(p)), "another pin was dropped: {pins0:?} -> {pins1:?}");
    assert!(!home().join(".claude").join("jobs").join("pins.json.lock").exists(), "the pin lock was left behind");
    assert_eq!(read_json(&w.base.join("state").join("pins.json")).as_array().map(|a| a.len()), Some(1));
    // its first turn (CLAUDE.md, `nimitz status` through Bash(nimitz *)) ends idle
    let rested = w.wait(240, || w.member(&w.fleet).filter(|m| matches!(m["status"].as_str(), Some("Idle" | "NeedsYou")))).expect("admiral's first turn over");
    println!("admiral after its first turn: status {} reasons {}", rested["status"], rested["reasons"]);
    assert!(!w.events().iter().any(|e| e["event"] == "PermissionRequest"), "a permission prompt fired: {:?}", w.events());
    let codes = w.issue_codes();
    assert!(!codes.iter().any(|(c, _)| matches!(c.as_str(), "needs" | "notify.failed" | "lead.intruder" | "held")), "{codes:?}");
    w.attach(&w.fleet_dir().join("admiral"), &job, &format!("[{}]-ADMIRAL", w.fleet));
    let pane = w.pane();
    println!("attached pane:\n{pane}");
    w.detach();
    std::thread::sleep(secs(3));
    let after = w.member(&w.fleet).unwrap();
    assert_eq!(after["session"]["short_id"], job.as_str(), "the detach ended the job: {after}");
    assert!(after["session"]["pid"].is_i64());
    // retire: stop (a fleet is removed only with nothing live in it), then
    // rm + config; the pin of its own id goes, the others stay
    w.ok(&["stop", &w.fleet, "--yes"]);
    w.wait(30, || w.member(&w.fleet).filter(|m| m["session"]["pid"].is_null())).expect("admiral stopped");
    let out = w.ok(&["fleet", "rm", &w.fleet, "--yes"]);
    println!("{out}");
    assert!(w.wait(30, || Some(w.job_rows()).filter(|r| r.is_empty())).is_some(), "rows left: {:?}", w.job_rows());
    let pins2 = w.pins();
    assert!(!pins2.contains(&job), "the retired lead's pin stays: {pins2:?}");
    assert!(pins0.iter().all(|p| pins2.contains(p)), "another pin was dropped: {pins0:?} -> {pins2:?}");
    assert_eq!(read_json(&w.base.join("state").join("pins.json")).as_array().map(|a| a.len()).unwrap_or(0), 0);
    w.finish();
}

/// 8. Rules update: a member whose delivered version is older than the
/// binary's gets "nimitz rules updated" at its next prompt, exactly once.
#[test]
fn e2e_08_rules_update_is_delivered_once_at_the_next_prompt() {
    if !gated() {
        return;
    }
    let w = E2e::new(8);
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    // an older binary delivered v1 to this job: the record says so (this binary's
    // VERSION is newer; the same path a newer binary takes against an old record)
    let p = w.base.join("state").join("rules-delivered.json");
    let mut d = read_json(&p);
    assert_eq!(d[&job]["version"], nimitz_core::rules::VERSION, "{d}");
    d[&job]["version"] = json!(1);
    std::fs::write(&p, d.to_string()).unwrap();
    let ups_before = w.events().iter().filter(|e| e["event"] == "UserPromptSubmit").count();
    w.attach(&w.ship_dir(), &job, &format!("[{}.SQ]-SHIP", w.fleet));
    w.type_line("Reply with the single word PONG.");
    w.wait(120, || Some(w.events()).filter(|ev| ev.iter().filter(|e| e["event"] == "UserPromptSubmit").count() > ups_before)).expect("a UserPromptSubmit");
    w.wait(180, || w.member(&w.ship()).filter(|m| m["status"] == "Idle")).expect("turn 2 over");
    w.type_line("Reply with the single word PONG again.");
    w.wait(120, || Some(w.events()).filter(|ev| ev.iter().filter(|e| e["event"] == "UserPromptSubmit").count() > ups_before + 1)).expect("a second UserPromptSubmit");
    w.wait(180, || w.member(&w.ship()).filter(|m| m["status"] == "Idle")).expect("turn 3 over");
    w.detach();
    let ups: Vec<Value> = w.events().into_iter().filter(|e| e["event"] == "UserPromptSubmit" && e["session_id"] == sid.as_str()).collect();
    let delivered: Vec<&Value> = ups.iter().filter(|e| !e["rules"].is_null()).collect();
    assert_eq!(delivered.len(), 1, "the update went out {} times: {ups:?}", delivered.len());
    assert_eq!(delivered[0]["rules"], nimitz_core::rules::VERSION);
    let d = w.deliveries();
    assert_eq!((d[&job]["version"].as_u64(), d[&job]["via"].as_str()), (Some(nimitz_core::rules::VERSION as u64), Some("UserPromptSubmit")), "{d}");
    let text = std::fs::read_to_string(w.wait(30, || w.transcript(&w.ship_dir(), &sid)).expect("transcript")).unwrap();
    let header = format!("nimitz rules updated v1\u{2192}v{}", nimitz_core::rules::VERSION);
    let lines: Vec<String> = text.lines().filter(|l| l.contains(&header)).map(|l| l.chars().take(240).collect()).collect();
    println!("transcript lines with the header ({}):\n{}", lines.len(), lines.join("\n"));
    assert!(!lines.is_empty(), "the update never reached the transcript");
    assert!(!w.issue_codes().iter().any(|(c, _)| c == "rules.old"), "{:?}", w.issue_codes());
    w.finish();
}

/// 9. A ship runs `nimitz status` through its `Bash(nimitz *)` allow with no
/// permission prompt; `nimitz stop <other>` is refused with `authority`.
#[test]
fn e2e_09_a_ship_runs_nimitz_without_a_prompt_and_stop_is_refused() {
    if !gated() {
        return;
    }
    let w = E2e::new(9);
    let other = format!("{}/SQ/OTHER", w.fleet);
    let prompt = format!(
        "Use the Bash tool to run these three commands, one call each, in this order: `nimitz --version`, `nimitz status`, `nimitz stop {other} --yes`. Do not ask; a failing command is expected. Then reply with the single word DONE."
    );
    w.fleet_with_ship(&prompt, "default");
    w.ok(&["ship", "new", &other, "--dir", w.base.join("other").to_str().unwrap(), "--prompt", OK_PROMPT, "--mode", "default", "--yes"]);
    let m = w.start_ship_idle();
    let sid = m["session"]["session_id"].as_str().unwrap().to_string();
    assert_ne!(m["status"], "NeedsYou");
    let ev = w.events();
    assert!(!ev.iter().any(|e| e["event"] == "PermissionRequest"), "a permission prompt fired: {ev:?}");
    assert!(!ev.iter().any(|e| e["reason"].as_str().map(|r| r.starts_with("needs-you")).unwrap_or(false)), "{ev:?}");
    assert!(std::fs::read_dir(w.base.join("state").join("needs")).map(|d| d.count() == 0).unwrap_or(true), "a needs-you notice was left");
    let text = std::fs::read_to_string(w.wait(30, || w.transcript(&w.ship_dir(), &sid)).expect("transcript")).unwrap();
    let calls = std::fs::read_to_string(w.base.join("nimitz-calls.log")).unwrap_or_default();
    for want in ["--version", "status", &format!("stop {other} --yes")] {
        assert!(calls.lines().any(|l| l.trim() == want), "the ship did not run `nimitz {want}` through its PATH; calls:\n{calls}");
    }
    assert!(text.contains(&w.fleet), "the status output is not in the transcript");
    assert!(text.contains("authority"), "the stop was not refused with authority:\n{}", text.lines().filter(|l| l.contains("nimitz stop")).collect::<Vec<_>>().join("\n"));
    assert!(w.member(&other).unwrap()["session"].is_null(), "OTHER was touched");
    w.finish();
}

/// 10. An unlabelled session in a ship folder is not shown; a `--fork-session`
/// copy of the ship is shown as a copy (and swept); the user's own stays.
#[test]
fn e2e_10_unlabelled_session_hidden_and_fork_shown_as_copy() {
    if !gated() {
        return;
    }
    let w = E2e::new(10);
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    // the user's own session in the ship folder: no labels, its own conversation
    let (code, out) = w.claude(&w.ship_dir(), &["--bg", "--model", &cheap_model(), "--name", "e2e10 own work", OK_PROMPT]);
    println!("own ({code}): {out}");
    let own = nimitz_core::claude::job_started(&out).expect("own job");
    // a fork of the ship's conversation (stopped first: a fork of a live job is a copy anyway)
    w.ok(&["stop", &w.ship(), "--yes"]);
    w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["pid"].is_null())).expect("ship stopped");
    let (code, out) = w.claude(&w.ship_dir(), &["--bg", "--resume", &sid, "--fork-session", "--model", &cheap_model()]);
    println!("fork ({code}): {out}");
    let fork = nimitz_core::claude::job_started(&out).expect("fork job");
    assert_ne!(fork, job);
    let seen = w
        .wait(60, || w.member(&w.ship()).filter(|m| m["extra_rows"].as_array().unwrap().iter().any(|r| r["short_id"] == fork.as_str() && !r["copy"].is_null())))
        .expect("the fork shows as a copy of the ship");
    let row = seen["extra_rows"].as_array().unwrap().iter().find(|r| r["short_id"] == fork.as_str()).unwrap();
    println!("fork row: {row}");
    assert_eq!((seen["session"]["short_id"].as_str(), seen["session"]["session_id"].as_str()), (Some(job.as_str()), Some(sid.as_str())), "the member keeps its row");
    assert!(!seen["extra_rows"].as_array().unwrap().iter().any(|r| r["short_id"] == own.as_str()), "the user's own session is a row of the ship: {seen}");
    let s = w.status();
    assert!(!s["stowaways"].as_array().unwrap().iter().any(|x| x["short_id"] == own.as_str() || x["short_id"] == fork.as_str()), "stowaways: {}", s["stowaways"]);
    let codes = w.issue_codes();
    assert!(codes.iter().any(|(c, m)| c == "copy" && m.contains(&fork)), "{codes:?}");
    assert!(!codes.iter().any(|(_, m)| m.contains(&own)), "an issue names the user's own session: {codes:?}");
    let out = w.ok(&["sweep", &w.ship(), "--yes"]);
    println!("{out}");
    let rows = w.wait(30, || Some(w.ship_rows()).filter(|r| !r.contains(&fork))).expect("the fork is gone");
    assert!(rows.contains(&job) && rows.contains(&own), "sweep touched the wrong rows: {rows:?}");
    w.finish();
}

/// 11. Other home: a member labelled for another `NIMITZ_HOME` never appears.
#[test]
fn e2e_11_another_homes_member_never_appears() {
    if !gated() {
        return;
    }
    let w = E2e::new(11);
    w.fleet_with_ship(OK_PROMPT, "default");
    let other_home = w.base.join("other-nimitz");
    std::fs::create_dir_all(&other_home).unwrap();
    let labels = w.no_mux(json!({"env": {
        "NIMITZ_FLEET": w.fleet, "NIMITZ_SQUADRON": "SQ", "NIMITZ_ROLE": "ship", "NIMITZ_ID": w.ship(), "NIMITZ_HOME": other_home},
        "worktree": {"bgIsolation": "none"}}).to_string());
    let name = format!("[{}.SQ]-SHIP", w.fleet);
    let (code, out) = w.claude(&w.ship_dir(), &["--bg", "--model", &cheap_model(), "--name", &name, "--settings", &labels, OK_PROMPT]);
    println!("other home ({code}): {out}");
    let job = nimitz_core::claude::job_started(&out).expect("job");
    let row = w.wait(45, || w.agents().into_iter().find(|r| r.short_id.as_deref() == Some(job.as_str()) && r.pid.is_some())).expect("live");
    let sid = row.session_id.clone();
    w.wait(120, || w.agents().into_iter().find(|r| r.session_id == sid && r.status.as_deref() == Some("idle")));
    let s = w.status();
    let text = s.to_string();
    assert!(!text.contains(&job) && !text.contains(&sid), "the other home's session appears in status: {s}");
    let me = w.member(&w.ship()).unwrap();
    assert!(me["session"].is_null(), "taken for the member: {me}");
    assert_eq!(me["status"], "NotStarted");
    // its hook ran with the other home in its own env (the label is the env):
    // whatever it wrote carries that home, and no reader of this home uses it
    let theirs: Vec<Value> = w.events().into_iter().filter(|e| e["session_id"] == sid.as_str()).collect();
    println!("lines of the other home's session: {}", theirs.len());
    assert!(theirs.iter().all(|e| e["labels"]["NIMITZ_HOME"] == other_home.to_str().unwrap()), "{theirs:?}");
    assert!(w.issue_codes().iter().all(|(c, _)| c == "cc.version"), "{:?}", w.issue_codes());
    let (code, out) = w.nz(&["check", &w.ship()]);
    assert!(code == 0 && !out.contains(&job), "{out}");
    w.finish();
}

/// 12. Registry v1 migration on a copy of a real-shaped v1 file, then a live
/// start binds into v2 (the old file kept as registry.json.v1).
#[test]
fn e2e_12_v1_registry_migrates_and_a_live_start_binds_into_v2() {
    if !gated() {
        return;
    }
    let w = E2e::new(12);
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    w.ok(&["stop", &w.ship(), "--yes"]);
    w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["pid"].is_null())).expect("ship stopped");
    // the 0.3.x file, word for word (round-2 `inside-registry.json` shape):
    // version 1, `current`, `bound_at`, no `job`; a tender identity
    let p = w.base.join("state").join("registry.json");
    let v2 = read_json(&p);
    let mut ids = serde_json::Map::new();
    for (k, rec) in v2["identities"].as_object().unwrap() {
        let lineage: Vec<Value> = rec["lineage"].as_array().unwrap().iter().map(|l| json!({"session_id": l["session_id"], "bound_at": l["at"], "via": l["via"]})).collect();
        ids.insert(k.clone(), json!({"role": rec["role"], "name": rec["name"], "cwd": rec["cwd"], "current": rec["session"], "lineage": lineage}));
    }
    ids.insert(
        format!("{}/SQ/~t1", w.fleet),
        json!({"role": "tender", "name": format!("[{}.SQ]-COMMODORE/t1", w.fleet), "cwd": w.fleet_dir().join("squadrons").join("sq"),
            "current": "00000000-0000-4000-8000-000000000001",
            "lineage": [{"session_id": "00000000-0000-4000-8000-000000000001", "bound_at": 1791334000.0, "via": "launch"}]}),
    );
    let v1 = json!({"version": 1, "machine": v2["machine"], "identities": ids});
    std::fs::write(&p, serde_json::to_string_pretty(&v1).unwrap()).unwrap();
    let _ = std::fs::remove_file(w.base.join("state").join("registry.json.bak"));
    // a read migrates in memory and writes nothing
    let st = w.member(&w.ship()).unwrap();
    assert_eq!((st["status"].as_str(), st["session"]["short_id"].as_str()), (Some("Stopped"), Some(job.as_str())), "{st}");
    assert_eq!(read_json(&p)["version"], 1, "a read rewrote the file");
    assert!(!w.base.join("state").join("registry.json.v1").exists());
    // a live start: the job comes from the row that runs `current`; a native resume; v2 saved
    let out = w.ok(&["start", &w.ship(), "--yes"]);
    println!("{out}");
    assert!(out.contains(&format!("backgrounded \u{b7} {job}")), "{out}");
    let again = w.wait(45, || w.member(&w.ship()).filter(|m| !m["session"]["pid"].is_null())).expect("ship live after resume");
    assert_eq!((again["session"]["short_id"].as_str(), again["session"]["session_id"].as_str()), (Some(job.as_str()), Some(sid.as_str())));
    let r = read_json(&p);
    assert_eq!(r["version"], 2, "{r}");
    let me = &r["identities"][w.ship()];
    assert_eq!((me["job"].as_str(), me["session"].as_str()), (Some(job.as_str()), Some(sid.as_str())), "{me}");
    assert!(me.get("current").is_none() && me["lineage"][0].get("bound_at").is_none(), "v1 keys survived: {me}");
    let lineage = me["lineage"].as_array().unwrap();
    assert_eq!(lineage.len(), 1, "a wake of the same conversation adds no entry: {lineage:?}");
    assert_eq!((lineage[0]["via"].as_str(), lineage[0]["job"].as_str(), lineage[0]["session_id"].as_str()), (Some("launch"), Some(job.as_str()), Some(sid.as_str())), "the old entry got its job");
    assert!(r["identities"].get(format!("{}/SQ/~t1", w.fleet)).is_none(), "the tender is a member: {r}");
    assert_eq!(r["retired"][format!("{}/SQ/~t1", w.fleet)]["session"], "00000000-0000-4000-8000-000000000001");
    let old = read_json(&w.base.join("state").join("registry.json.v1"));
    assert_eq!(old["version"], 1, "registry.json.v1 is not the old file: {old}");
    assert!(!w.issue_codes().iter().any(|(c, _)| matches!(c.as_str(), "registry.corrupt" | "lineage.broken")), "{:?}", w.issue_codes());
    w.finish();
}

/// 13. `/clear`, then stop, then start: the wake names the job's birth id and
/// the same job comes back on the post-clear conversation with its labels
/// (acceptance F1: a wake by the conversation id made an unlabelled copy).
#[test]
fn e2e_13_clear_then_stop_then_start_wakes_the_same_job() {
    if !gated() {
        return;
    }
    let w = E2e::new(13);
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, birth) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    w.attach(&w.ship_dir(), &job, &format!("[{}.SQ]-SHIP", w.fleet));
    w.type_line("/clear");
    let cleared = w
        .wait(60, || w.events().into_iter().rev().find(|e| e["event"] == "SessionStart" && e["source"] == "clear" && e["labels"]["NIMITZ_ID"] == w.ship()))
        .expect("a SessionStart clear after /clear");
    w.detach();
    let now = cleared["session_id"].as_str().unwrap().to_string();
    assert_ne!(now, birth);
    w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["session_id"] == now.as_str())).expect("status follows the new conversation");
    w.ok(&["stop", &w.ship(), "--yes"]);
    w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["pid"].is_null())).expect("ship stopped");
    // the plan names the birth id, never the conversation id; nothing is removed
    let plan = w.ok(&["start", &w.ship(), "--plan"]);
    println!("{plan}");
    assert!(plan.contains(&format!("{} --bg --resume {birth}", w.native)), "{plan}");
    assert!(!plan.contains(&format!("--resume {now}")) && !plan.contains(" rm "), "{plan}");
    let out = w.ok(&["start", &w.ship(), "--yes"]);
    println!("{out}");
    assert!(out.contains(&format!("backgrounded \u{b7} {job}")) && out.contains("verified by re-reading"), "{out}");
    assert!(!out.contains("lineage.broken") && !out.contains("started a copy"), "{out}");
    let again = w.wait(45, || w.member(&w.ship()).filter(|m| !m["session"]["pid"].is_null())).expect("ship live after the wake");
    assert_eq!((again["session"]["short_id"].as_str(), again["session"]["session_id"].as_str()), (Some(job.as_str()), Some(now.as_str())), "{again}");
    assert_eq!(again["session"]["labels"]["NIMITZ_ID"], w.ship(), "labels kept: {again}");
    assert_eq!(again["session"]["name"], format!("[{}.SQ]-SHIP", w.fleet));
    assert!(again["extra_rows"].as_array().unwrap().is_empty(), "{again}");
    assert_eq!(w.ship_rows(), vec![job.clone()], "one row, the same job");
    let starts = w.session_starts(&now);
    assert_eq!(starts.last().map(|e| (e["source"].as_str(), e["job"].as_str())), Some((Some("resume"), Some(job.as_str()))), "{starts:?}");
    let me = &w.registry()["identities"][w.ship()];
    assert_eq!((me["job"].as_str(), me["session"].as_str()), (Some(job.as_str()), Some(now.as_str())), "{me}");
    let codes = w.issue_codes();
    assert!(!codes.iter().any(|(c, _)| matches!(c.as_str(), "copy" | "copy.rows" | "dup.live" | "labels.missing" | "lineage.broken" | "name.drift")), "{codes:?}");
    // /branch, then stop, then start: a native wake would bring the pre-branch
    // conversation back (`resumeSessionId` follows /clear only, case 14), so
    // nimitz removes the row and resumes the branch with every flag, and says so
    w.attach(&w.ship_dir(), &job, &format!("[{}.SQ]-SHIP", w.fleet));
    w.type_line("/branch");
    let branched = w
        .wait(60, || w.events().into_iter().rev().find(|e| e["event"] == "SessionStart" && e["labels"]["NIMITZ_ID"] == w.ship() && e["session_id"] != now.as_str() && e["session_id"] != birth.as_str()))
        .expect("a SessionStart after /branch");
    w.detach();
    let branch = branched["session_id"].as_str().unwrap().to_string();
    println!("/branch: SessionStart source {} session {branch} job {} binding {}", branched["source"], branched["job"], branched["binding"]);
    w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["session_id"] == branch.as_str())).expect("status follows the branch");
    w.ok(&["stop", &w.ship(), "--yes"]);
    w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["pid"].is_null())).expect("ship stopped again");
    let plan = w.ok(&["start", &w.ship(), "--plan"]);
    println!("{plan}");
    assert!(plan.contains(&format!("rm {job}")) && plan.contains(&format!("--resume {branch} --name")), "{plan}");
    assert!(plan.contains("resumeSessionId follows /clear only") && !plan.contains(&format!("--bg --resume {birth}\n")), "{plan}");
    let out = w.ok(&["start", &w.ship(), "--yes"]);
    println!("{out}");
    assert!(out.contains("verified by re-reading") && !out.contains("history.lost"), "{out}");
    let on_branch = w.wait(45, || w.member(&w.ship()).filter(|m| !m["session"]["pid"].is_null())).expect("ship live on the branch");
    assert_eq!(on_branch["session"]["session_id"], branch.as_str(), "the branch is what runs: {on_branch}");
    assert_eq!(on_branch["session"]["labels"]["NIMITZ_ID"], w.ship());
    assert_eq!(on_branch["session"]["name"], format!("[{}.SQ]-SHIP", w.fleet));
    assert_eq!(w.ship_rows().len(), 1, "one row: {:?}", w.ship_rows());
    assert_eq!(w.registry()["identities"][w.ship()]["session"], branch.as_str());
    let codes = w.issue_codes();
    assert!(!codes.iter().any(|(c, _)| matches!(c.as_str(), "copy" | "copy.rows" | "dup.live" | "labels.missing" | "lineage.broken" | "history.lost" | "name.drift")), "{codes:?}");
    w.finish();
}
/// 14. `/branch`, then stop, then a raw wake by the job's birth id: Claude
/// Code brings the pre-branch conversation back (`resumeSessionId` follows
/// `/clear` only; r3-e2e finding 11), and nimitz follows what woke. nimitz's
/// own start after a `/branch` goes the other way (case 13: rm + resume of
/// the branch with every flag).
#[test]
fn e2e_14_raw_birth_id_wake_after_branch_returns_the_pre_branch_conversation() {
    if !gated() {
        return;
    }
    let w = E2e::new(14);
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, birth) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    w.attach(&w.ship_dir(), &job, &format!("[{}.SQ]-SHIP", w.fleet));
    w.type_line("/branch");
    let branched = w
        .wait(60, || w.events().into_iter().rev().find(|e| e["event"] == "SessionStart" && e["labels"]["NIMITZ_ID"] == w.ship() && e["session_id"] != birth.as_str()))
        .expect("a SessionStart after /branch");
    w.detach();
    let branch = branched["session_id"].as_str().unwrap().to_string();
    println!("/branch: SessionStart source {} session {branch} job {}", branched["source"], branched["job"]);
    w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["session_id"] == branch.as_str())).expect("status follows the branch");
    w.ok(&["stop", &w.ship(), "--yes"]);
    w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["pid"].is_null())).expect("ship stopped");
    let state = read_json(&home().join(".claude").join("jobs").join(&job).join("state.json"));
    println!("state.json after /branch + stop: sessionId {} resumeSessionId {}", state["sessionId"], state["resumeSessionId"]);
    // the daemon's own bookkeeping settles a moment after the process ended: a
    // wake inside that moment makes a copy ("already running in the background";
    // r3-e2e finding 12), so wait until `claude agents` no longer lists the job as running
    w.wait(30, || w.agents().into_iter().find(|r| r.short_id.as_deref() == Some(job.as_str()) && r.pid.is_none() && r.state.as_deref() != Some("running"))).expect("the daemon settled the stopped job");
    std::thread::sleep(secs(3));
    // the raw fact: a wake by the birth id, by hand
    let before = w.events().len();
    let (mut code, mut out) = w.claude(&w.ship_dir(), &["--bg", "--resume", &birth]);
    println!("raw wake ({code}): {out}");
    // under load the raw wake can still race the daemon (finding 12): the
    // copy goes, and the wake runs once more (what nimitz's apply does)
    if let Some(copy) = nimitz_core::claude::copy_started(&out) {
        let _ = w.claude(&w.base, &["stop", &copy]);
        let _ = w.claude(&w.base, &["rm", &copy]);
        std::thread::sleep(secs(5));
        (code, out) = w.claude(&w.ship_dir(), &["--bg", "--resume", &birth]);
        println!("raw wake, once more ({code}): {out}");
    }
    let woke = w
        .wait(45, || w.events().into_iter().skip(before).find(|e| e["event"] == "SessionStart" && e["labels"]["NIMITZ_ID"] == w.ship()))
        .expect("a SessionStart after the raw wake");
    let woken = woke["session_id"].as_str().unwrap().to_string();
    println!("the birth-id wake brought back {woken} (birth {birth}, branch {branch}); source {}", woke["source"]);
    assert_eq!(woke["job"], job.as_str(), "the same job: {woke}");
    assert_ne!(woken, branch, "Claude Code 2.1.292 wakes on resumeSessionId, which /branch does not move; if this changed, r3-e2e finding 11 is out of date");
    assert_eq!(woken, state["resumeSessionId"].as_str().unwrap_or(&birth), "{woke}");
    // nimitz follows what woke, and says the branch is still in the lineage
    let now = w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["session_id"] == woken.as_str() && !m["session"]["pid"].is_null())).expect("status follows the woken conversation");
    assert_eq!(now["session"]["short_id"], job.as_str());
    let me = &w.registry()["identities"][w.ship()];
    assert_eq!(me["session"], woken.as_str(), "{me}");
    assert!(me["lineage"].as_array().unwrap().iter().any(|l| l["session_id"] == branch.as_str()), "the branch stays in the lineage: {me}");
    w.finish();
}


// ------------------------------------------------------------------ final acceptance (re-runs of the first pass's FAIL rows)

/// Acceptance cli-18: a stopped ship whose conversation is resumed
/// interactively in its folder is `held` only (one issue), and `stop` and
/// `relaunch` refuse it with the held fix; after `/exit`, start wakes it.
#[test]
fn e2e_15_accept_cli18_a_held_member_gives_held_only() {
    if !gated() {
        return;
    }
    let w = E2e::new(15);
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    w.ok(&["stop", &w.ship(), "--yes"]);
    w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["pid"].is_null())).expect("ship stopped");
    w.tmux_run(&w.ship_dir(), &format!("'{}' --model '{}' --resume {sid}", w.native, cheap_model()), "\u{276f}");
    let held = w.wait(45, || Some(w.member_issues(&w.ship())).filter(|v| v.iter().any(|(c, _)| c == "held")));
    let codes = held.expect("held").into_iter().map(|(c, _)| c).collect::<Vec<_>>();
    println!("cli-18 issues: {codes:?}");
    assert_eq!(codes, vec!["held".to_string()], "held only");
    for cmd in [vec!["stop", &w.ship(), "--plan"], vec!["relaunch", &w.ship(), "--plan"]] {
        let (code, out) = w.nz(&cmd.iter().map(|s| &**s).collect::<Vec<_>>());
        println!("{cmd:?}: {out}");
        assert!(code != 0 && out.contains("session.interactive"), "{out}");
    }
    w.type_line("/exit");
    w.wait(45, || w.member(&w.ship()).filter(|m| !m["issues"].as_array().unwrap().iter().any(|i| i["code"] == "held"))).expect("held gone after /exit");
    let out = w.ok(&["start", &w.ship(), "--yes"]);
    println!("{out}");
    assert!(out.contains(&format!("backgrounded \u{b7} {job}")) && out.contains("verified by re-reading"), "{out}");
    w.finish();
}

/// Acceptance cli-L8 / cli-K1: an interactive process with the admiral's
/// name in its folder is the admiral's `lead.intruder` only (no second
/// `name.taken`), with the terminal fix; a ship under it still starts.
#[test]
fn e2e_16_accept_clil8_an_interactive_namesake_is_the_leads_intruder_only() {
    if !gated() {
        return;
    }
    let w = E2e::new(16);
    w.fleet_with_ship(OK_PROMPT, "default");
    let adm = w.fleet.clone();
    let name = nimitz_core::Id(adm.clone()).name();
    w.tmux_run(&w.fleet_dir().join("admiral"), &format!("'{}' --model '{}' --name '{name}'", w.native, cheap_model()), "\u{276f}");
    let got = w.wait(45, || Some(w.member_issues(&adm)).filter(|v| v.iter().any(|(c, _)| c == "lead.intruder"))).expect("lead.intruder");
    println!("cli-L8 issues: {got:?}");
    assert!(!got.iter().any(|(c, _)| c == "name.taken" || c == "dup.live"), "{got:?}");
    assert!(got.iter().any(|(c, f)| c == "lead.intruder" && f.contains("end it in its terminal")), "{got:?}");
    let (code, out) = w.nz(&["start", &w.ship(), "--plan"]);
    println!("{out}");
    assert_eq!(code, 0, "a ship under it still starts: {out}");
    w.type_line("/exit");
    w.wait(30, || Some(w.member_issues(&adm)).filter(|v| !v.iter().any(|(c, _)| c == "lead.intruder"))).expect("intruder gone after /exit");
    w.finish();
}

/// Acceptance cli-N1: a hand `--bg --resume` of the live ship makes a copy;
/// `nimitz stop <copy>` removes its row too (it would be invisible after).
#[test]
fn e2e_17_accept_clin1_stop_of_a_copy_removes_its_row() {
    if !gated() {
        return;
    }
    let w = E2e::new(17);
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    let (code, out) = w.claude(&w.ship_dir(), &["--bg", "--resume", &sid]);
    println!("hand resume of the live ship: {code} {out}");
    let copy = w.wait(45, || w.ship_rows().into_iter().find(|j| *j != job)).expect("a copy row");
    w.wait(30, || Some(w.member_issues(&w.ship())).filter(|v| v.iter().any(|(c, _)| c == "copy"))).expect("the copy is named");
    let out = w.ok(&["stop", &copy, "--yes"]);
    println!("{out}");
    assert!(out.contains(&format!("rm {copy}")), "the stop removes the copy's row: {out}");
    w.wait(30, || Some(w.ship_rows()).filter(|r| !r.contains(&copy))).expect("the copy row is gone");
    let ship = w.member(&w.ship()).unwrap();
    assert_eq!(ship["session"]["short_id"], job.as_str(), "the ship untouched: {ship}");
    w.finish();
}

/// Acceptance cli-L3b / 3.5: `/exit` inside the attached admiral drops into
/// the agent view, whose draft spare sits in the admiral's folder: no
/// `lead.intruder` and no stowaway while the view is open.
#[test]
fn e2e_18_accept_clil3b_the_agent_views_draft_in_a_lead_folder_is_not_an_intruder() {
    if !gated() {
        return;
    }
    let w = E2e::new(18);
    w.fleet_with_ship(OK_PROMPT, "default");
    let adm = w.fleet.clone();
    let a = w.member(&adm).unwrap();
    let job = a["session"]["short_id"].as_str().unwrap().to_string();
    w.attach(&w.fleet_dir().join("admiral"), &job, &nimitz_core::Id(adm.clone()).name());
    w.type_line("/exit");
    // the agent view claims a draft spare in the admiral's folder
    let dir = w.fleet_dir().join("admiral").to_string_lossy().into_owned();
    let spare = w.wait(30, || {
        std::fs::read_dir(home().join(".claude").join("sessions")).into_iter().flatten().flatten().map(|e| read_json(&e.path())).find(|v| v["spare"] == true && v["cwd"].as_str() == Some(dir.as_str()) && v["pid"].as_i64().map(nimitz_core::evidence::pid_alive).unwrap_or(false))
    });
    let pane = w.pane();
    let spare = spare.unwrap_or_else(|| panic!("no draft spare in the admiral folder after /exit:\n{pane}"));
    let codes = w.issue_codes();
    println!("cli-L3b: draft spare {} (job {}) in the admiral folder; issues {codes:?}", spare["sessionId"], spare["jobId"]);
    assert!(!codes.iter().any(|(c, _)| c == "lead.intruder" || c == "stowaway"), "{codes:?}");
    w.finish();
}

/// Acceptance E9: no job row and no transcript is `history.lost` with the
/// `--fresh` fix (never lineage.broken / --force); `--fresh` plans the cold
/// launch with no error line.
#[test]
fn e2e_19_accept_e9_no_row_and_no_transcript_is_history_lost_with_fresh() {
    if !gated() {
        return;
    }
    let w = E2e::new(19);
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    w.ok(&["stop", &w.ship(), "--yes"]);
    w.wait(30, || w.member(&w.ship()).filter(|m| m["session"]["pid"].is_null())).expect("ship stopped");
    let (code, out) = w.claude(&w.base, &["rm", &job]);
    assert_eq!(code, 0, "{out}");
    let t = w.wait(30, || w.transcript(&w.ship_dir(), &sid)).expect("a transcript");
    std::fs::rename(&t, t.with_extension("aside")).unwrap();
    let (code, out) = w.nz(&["start", &w.ship(), "--plan"]);
    println!("E9 plan: {out}");
    assert!(code != 0 && out.contains("history.lost") && out.contains("--fresh") && !out.contains("lineage.broken") && !out.contains("--force"), "{out}");
    let (code, out) = w.nz(&["start", &w.ship(), "--fresh", "--plan"]);
    println!("E9 --fresh plan: {out}");
    assert!(code == 0 && !out.contains("[ERROR]"), "{out}");
    w.finish();
}

/// Acceptance N2 / N15: `squadron new --mode` is taken; a start of a live
/// member and an `up` of a fleet that is all live say why nothing happens.
#[test]
fn e2e_20_accept_n2_n15_mode_flag_and_empty_plans_say_why() {
    if !gated() {
        return;
    }
    let w = E2e::new(20);
    w.fleet_with_ship(OK_PROMPT, "default");
    let (code, out) = w.nz(&["squadron", "new", &format!("{}/SQ2", w.fleet), "--mode", "default", "--plan"]);
    println!("N2: {out}");
    assert!(code == 0 && !out.contains("unknown flag"), "{out}");
    w.start_ship_idle();
    let (_, out) = w.nz(&["start", &w.ship(), "--plan"]);
    println!("N15 start: {out}");
    assert!(out.contains("nothing to start"), "{out}");
    let (_, out) = w.nz(&["up", &w.fleet, "--plan"]);
    println!("N15 up: {out}");
    assert!(out.contains("nothing to do"), "{out}");
    w.finish();
}

/// Acceptance 3.8: `/fork` in the attached admiral (after its first turn)
/// makes a labelled bg copy "<name> ⑂" in the admiral folder: it is the
/// admiral's `lead.intruder` (stop then rm), never `dup.live`; a ship under
/// it still starts; the admiral keeps its own job.
#[test]
fn e2e_21_accept_3_8_a_fork_of_a_lead_is_its_intruder() {
    if !gated() {
        return;
    }
    let w = E2e::new(21);
    w.fleet_with_ship(OK_PROMPT, "default");
    let adm = w.fleet.clone();
    let job = w.member(&adm).unwrap()["session"]["short_id"].as_str().unwrap().to_string();
    w.attach(&w.fleet_dir().join("admiral"), &job, &nimitz_core::Id(adm.clone()).name());
    w.type_line("/fork");
    let fork = w.wait(60, || w.jobs_in(&w.fleet_dir().join("admiral")).into_iter().find(|j| *j != job));
    w.detach();
    let fork = fork.expect("a fork job in the admiral folder");
    let got = w.wait(45, || Some(w.member_issues(&adm)).filter(|v| v.iter().any(|(c, _)| c == "lead.intruder"))).expect("lead.intruder");
    println!("3.8 issues: {got:?}");
    assert!(!got.iter().any(|(c, _)| c == "dup.live"), "{got:?}");
    assert!(got.iter().any(|(c, f)| c == "lead.intruder" && f.contains(&format!("nimitz stop {fork}"))), "{got:?}");
    assert_eq!(w.member(&adm).unwrap()["session"]["short_id"], job.as_str(), "the admiral keeps its job");
    let (code, out) = w.nz(&["start", &w.ship(), "--plan"]);
    assert_eq!(code, 0, "{out}");
    w.finish();
}

/// The e2e sessions run on the cheap model they ask for: a `--model` the
/// user's `availableModels` does not allow is swapped for the account default (Opus here)
/// without an error, so the suite checks what the transcript says.
#[test]
fn e2e_00_sessions_run_on_the_cheap_model() {
    if !gated() {
        return;
    }
    let w = E2e::new(93);
    w.ok(&["fleet", "new", &w.fleet, "--machine", "e2e", "--yes"]);
    let dir = w.fleet_dir().join("admiral");
    w.settings(&dir);
    let idv = nimitz_core::Id(w.fleet.clone());
    let settings = w.no_mux(nimitz_core::claude::settings_json(&w.env, &idv, true));
    let (code, out) = w.claude(&dir, &["--bg", "--model", &cheap_model(), "--name", &idv.name(), "--settings", &settings, OK_PROMPT]);
    assert_eq!(code, 0, "{out}");
    let m = w.wait(240, || w.member(&w.fleet).filter(|m| !m["session"]["pid"].is_null() && m["status"] == "Idle")).expect("admiral idle");
    let sid = m["session"]["session_id"].as_str().unwrap().to_string();
    // Claude Code 2.1.296 may report the session idle before its transcript
    // is on disk, and before the reply is in it (seen under load: no project
    // folder at the idle, the file there seconds later), so both are waited for
    // (`<synthetic>` lines are Claude Code's own, for an API error or an interrupt)
    let models_in = |t: &str| -> Vec<String> { t.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|v| v["type"] == "assistant").filter_map(|v| v["message"]["model"].as_str().map(str::to_lowercase)).filter(|m| m != "<synthetic>").collect() };
    // the reply lands in the transcript after the session reads idle (2.1.296)
    let models = w
        .wait(30, || w.transcript(&dir, &sid).and_then(|p| std::fs::read_to_string(p).ok()).map(|t| models_in(&t)).filter(|m| !m.is_empty()))
        .unwrap_or_default();
    let want = cheap_model().to_lowercase();
    let want = want.split('[').next().unwrap_or("").to_string();
    println!("[OK] asked for {want}, ran on {models:?}");
    assert!(!models.is_empty() && models.iter().all(|m| m.contains(&want)), "asked for {want}, ran on {models:?} (NIMITZ_E2E_MODEL must be a family name or a model id)");
    w.finish();
}


/// What tells the agent view (`/exit` inside an attach, R-24) from a live
/// `claude attach <job>`: the attach client re-execs in place as `claude
/// agents` (same pid), so its argv says which it is. The typing guard
/// (`nimitz_mux::runs_exact_attach`) rests on this. Also prints the terminal
/// title, the process tree, session files and the job row of both states.
#[test]
fn e2e_agent_view_re_execs_as_claude_agents() {
    if !gated() {
        return;
    }
    let w = E2e::new(90);
    w.ok(&["fleet", "new", &w.fleet, "--machine", "e2e", "--yes"]);
    let dir = w.fleet_dir().join("admiral");
    w.settings(&dir);
    let idv = nimitz_core::Id(w.fleet.clone());
    let settings = w.no_mux(nimitz_core::claude::settings_json(&w.env, &idv, true));
    let (code, out) = w.claude(&dir, &["--bg", "--model", &cheap_model(), "--name", &idv.name(), "--settings", &settings, OK_PROMPT]);
    assert_eq!(code, 0, "{out}");
    let m = w.wait(240, || w.member(&w.fleet).filter(|m| !m["session"]["pid"].is_null() && m["status"] == "Idle")).expect("admiral idle");
    let job = m["session"]["short_id"].as_str().unwrap().to_string();
    let claude_home = home().join(".claude");
    let client = std::cell::RefCell::new((0i64, String::new()));
    let collect = |label: &str| {
        let (_, info) = w.tx(&["display-message", "-p", "-t", "e2e", "title=#{pane_title} | pid=#{pane_pid} | cmd=#{pane_current_command}"]);
        println!("PROBE[{label}] tmux: {}", info.trim());
        let pane_pid: i64 = info.split("pid=").nth(1).and_then(|s| s.split_whitespace().next()).and_then(|s| s.parse().ok()).unwrap_or(0);
        let (_, ps) = E2e::run({
            let mut c = Command::new("ps");
            c.args(["-eo", "pid=,ppid=,stat=,args="]);
            c
        });
        let rows: Vec<(i64, i64, String)> = ps
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                let pid = it.next()?.parse().ok()?;
                let ppid = it.next()?.parse().ok()?;
                Some((pid, ppid, it.collect::<Vec<_>>().join(" ")))
            })
            .collect();
        let mut tree = vec![pane_pid];
        let mut i = 0;
        while i < tree.len() {
            let p = tree[i];
            tree.extend(rows.iter().filter(|r| r.1 == p).map(|r| r.0));
            i += 1;
        }
        *client.borrow_mut() = rows.iter().filter(|r| r.1 == pane_pid).map(|r| (r.0, r.2.clone())).next().unwrap_or_default();
        for (pid, ppid, args) in rows.iter().filter(|r| tree.contains(&r.0)) {
            let sf = std::fs::read_to_string(claude_home.join("sessions").join(format!("{pid}.json"))).unwrap_or_default();
            println!("PROBE[{label}] proc {pid} (ppid {ppid}): {args}{}", if sf.is_empty() { String::new() } else { format!("\n    session file: {sf}") });
        }
        let row = std::fs::read_to_string(claude_home.join("jobs").join(&job).join("state.json")).unwrap_or_default();
        println!("PROBE[{label}] job row {job}: {row}");
        let pane = w.pane();
        let lines: Vec<&str> = pane.lines().filter(|l| !l.trim().is_empty()).collect();
        println!("PROBE[{label}] screen (first 6, last 6):\n{}\n...\n{}", lines.iter().take(6).cloned().collect::<Vec<_>>().join("\n"), lines.iter().rev().take(6).rev().cloned().collect::<Vec<_>>().join("\n"));
    };
    w.attach(&dir, &job, &idv.name());
    collect("attach");
    let (attach_pid, attach) = client.borrow().clone();
    assert!(attach.ends_with(&format!(" attach {job}")), "the attach client: {attach}");
    w.type_line("/exit");
    std::thread::sleep(secs(10));
    collect("after /exit");
    let (view_pid, view) = client.borrow().clone();
    assert!(view.ends_with(" agents") && !view.contains(" attach "), "the agent view's client: {view}");
    assert_eq!(view_pid, attach_pid, "the same process, re-exec'd in place");
    w.finish();
}

/// Probe (`NIMITZ_E2E=1 NIMITZ_E2E_PROBE=1`): a notice delivered without
/// typing, through the session's inbox socket (Claude Code's cross-session
/// messaging; the line format Claude Code logs as its own inject example):
/// does an idle background job take it as a turn, and what does it see?
#[cfg(unix)]
#[test]
fn e2e_probe_inbox_delivery() {
    if !gated() || std::env::var("NIMITZ_E2E_PROBE").ok().as_deref() != Some("1") {
        return;
    }
    use std::io::Write;
    let w = E2e::new(91);
    w.ok(&["fleet", "new", &w.fleet, "--machine", "e2e", "--yes"]);
    let dir = w.fleet_dir().join("admiral");
    w.settings(&dir);
    let idv = nimitz_core::Id(w.fleet.clone());
    let settings = w.no_mux(nimitz_core::claude::settings_json(&w.env, &idv, true));
    let (code, out) = w.claude(&dir, &["--bg", "--model", &cheap_model(), "--name", &idv.name(), "--settings", &settings, OK_PROMPT]);
    assert_eq!(code, 0, "{out}");
    let m = w.wait(240, || w.member(&w.fleet).filter(|m| !m["session"]["pid"].is_null() && m["status"] == "Idle")).expect("admiral idle");
    let pid = m["session"]["pid"].as_i64().unwrap();
    let sid = m["session"]["session_id"].as_str().unwrap().to_string();
    let rt = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| format!("/run/user/{}", unsafe_uid()));
    let sock = PathBuf::from(format!("{rt}/cc-socks/{pid}.sock"));
    println!("PROBE inbox: pid {pid} socket {} exists {}", sock.display(), sock.exists());
    let ups = w.events().iter().filter(|e| e["event"] == "UserPromptSubmit").count();
    // NIMITZ_E2E_PROBE_FROM=1: the content wrapped as Claude Code wraps a peer
    // message, with a sender name (and a reply address nobody listens on)
    let body = "[nimitz notice, not the user] Reply with the single word PROBE.";
    let content = if std::env::var("NIMITZ_E2E_PROBE_FROM").ok().as_deref() == Some("1") {
        format!("<cross-session-message from=\"uds:{rt}/cc-socks/nimitz-none.sock\" from-name=\"nimitz\">\n{body}\n</cross-session-message>")
    } else {
        body.to_string()
    };
    let line = json!({"type": "user", "message": {"role": "user", "content": content}}).to_string();
    match std::os::unix::net::UnixStream::connect(&sock) {
        Ok(mut s) => {
            let r = s.write_all(format!("{line}\n").as_bytes());
            println!("PROBE inbox: wrote {} bytes: {r:?}", line.len() + 1);
            let _ = s.shutdown(std::net::Shutdown::Write);
            let mut back = String::new();
            let _ = std::io::Read::read_to_string(&mut s, &mut back);
            println!("PROBE inbox: reply {back:?}");
        }
        Err(e) => println!("PROBE inbox: connect failed: {e}"),
    }
    let got = w.wait(90, || Some(w.events()).filter(|ev| ev.iter().filter(|e| e["event"] == "UserPromptSubmit").count() > ups));
    println!("PROBE inbox: a new UserPromptSubmit: {}", got.is_some());
    w.wait(120, || w.member(&w.fleet).filter(|m| m["status"] == "Idle"));
    if let Some(t) = w.transcript(&dir, &sid) {
        let text = std::fs::read_to_string(&t).unwrap_or_default();
        for l in text.lines().rev().filter(|l| l.contains("PROBE")).take(4) {
            println!("PROBE inbox: transcript line: {}", &l[..l.len().min(1500)]);
        }
    }
    let row = std::fs::read_to_string(home().join(".claude").join("jobs").join(m["session"]["short_id"].as_str().unwrap()).join("state.json")).unwrap_or_default();
    println!("PROBE inbox: job row detail: {}", serde_json::from_str::<Value>(&row).map(|v| v["detail"].clone()).unwrap_or_default());
    w.finish();
}

fn unsafe_uid() -> String {
    String::from_utf8_lossy(&Command::new("id").arg("-u").output().unwrap().stdout).trim().to_string()
}

/// Socket delivery: a lead's SessionStart hook records its inbox socket
/// (`CLAUDE_CODE_MESSAGING_SOCKET`), and `nimitz signal` posts into it with
/// no multiplexer at all: the idle background job takes a new turn, and sees
/// a cross-session message from nimitz.
#[test]
fn e2e_inbox_signal_reaches_an_idle_lead_without_a_pane() {
    if !gated() {
        return;
    }
    let w = E2e::new(92);
    w.ok(&["fleet", "new", &w.fleet, "--machine", "e2e", "--yes"]);
    let dir = w.fleet_dir().join("admiral");
    w.settings(&dir);
    let idv = nimitz_core::Id(w.fleet.clone());
    let settings = w.no_mux(nimitz_core::claude::settings_json(&w.env, &idv, true));
    let (code, out) = w.claude(&dir, &["--bg", "--model", &cheap_model(), "--name", &idv.name(), "--settings", &settings, OK_PROMPT]);
    assert_eq!(code, 0, "{out}");
    let m = w.wait(240, || w.member(&w.fleet).filter(|m| !m["session"]["pid"].is_null() && m["status"] == "Idle")).expect("admiral idle");
    let sid = m["session"]["session_id"].as_str().unwrap().to_string();
    // The SessionStart hook writes the record; under load the job can show Idle first.
    let path = w.base.join("state").join("inbox").join(format!("{sid}.json"));
    let rec = w.wait(30, || Some(read_json(&path)).filter(|r| r["socket"].is_string())).unwrap_or_else(|| read_json(&path));
    let sock = rec["socket"].as_str().unwrap_or_else(|| panic!("no inbox recorded for {sid}: {rec}")).to_string();
    assert!(Path::new(&sock).exists() && rec["pid"] == m["session"]["pid"], "{rec}");
    let ups = w.events().iter().filter(|e| e["event"] == "UserPromptSubmit").count();
    let out = w.ok(&["signal", &w.fleet, "Reply with the single word PONG.", "--yes"]);
    assert!(out.contains("inbox"), "{out}");
    w.wait(120, || Some(w.events()).filter(|ev| ev.iter().filter(|e| e["event"] == "UserPromptSubmit").count() > ups)).expect("the signal started a turn");
    w.wait(180, || w.member(&w.fleet).filter(|m| m["status"] == "Idle")).expect("turn over");
    let t = std::fs::read_to_string(w.wait(30, || w.transcript(&dir, &sid)).expect("transcript")).unwrap();
    let seen = t.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|v| v["type"] == "user" && v["message"]["content"].as_str().is_some_and(|c| c.contains("[nimitz signal]"))).expect("the message in the transcript");
    println!("[OK] seen as: origin {}", seen["origin"]);
    assert_eq!(seen["origin"]["kind"], "peer");
    assert_eq!(seen["origin"]["name"], "nimitz", "{}", seen["origin"]);
    assert!(t.contains("PONG"), "the lead answered");
    w.finish();
}

/// One restore after a Claude Code update (real Claude Code): the installed
/// version reads newer than every worker's (a wrapper as the native binary
/// answers `--version` with 2.1.999 and runs the real one for the rest), so
/// `restore --jobs-only` relaunches each live member, the ship first, then
/// the commodore, then the admiral: the same session id, a new process on
/// the binary the worker runs, its labels intact. The test's own members
/// only; no multiplexer (`--jobs-only`).
#[cfg(unix)]
#[test]
fn e2e_restore_relaunches_workers_older_than_the_installed_claude() {
    if !gated() {
        return;
    }
    let w = E2e::new(94);
    w.fleet_with_ship(OK_PROMPT, "default");
    w.start_ship_idle();
    let ids = [w.ship(), format!("{}/SQ", w.fleet), w.fleet.clone()];
    for id in &ids {
        w.wait(180, || w.member(id).filter(|m| !m["session"]["pid"].is_null() && m["status"] == "Idle")).unwrap_or_else(|| panic!("{id} idle"));
    }
    let before: Vec<(String, String, i64)> = ids.iter().map(|id| {
        let m = w.member(id).unwrap();
        (id.clone(), m["session"]["session_id"].as_str().unwrap().to_string(), m["session"]["pid"].as_i64().unwrap())
    }).collect();
    let newer = w.base.join("bin").join("claude-newer");
    executable(&newer, &format!("#!/bin/sh\nfor a; do [ \"$a\" = --version ] && {{ echo '2.1.999 (Claude Code)'; exit 0; }}; done\nexec '{}' \"$@\"\n", w.native));
    let restore = |form: &str| {
        let mut c = w.nz_cmd();
        c.env("NIMITZ_NATIVE_CLAUDE_BIN", &newer).args(["restore", &w.fleet, "--jobs-only", form]);
        E2e::run(c)
    };
    let (code, plan) = restore("--plan");
    assert_eq!(code, 0, "{plan}");
    assert!(plan.contains("Claude Code updated to 2.1.999  ·  3 to relaunch"), "{plan}");
    for id in &ids {
        assert!(plan.lines().any(|l| l.trim_start().starts_with(&format!("{id} ")) && l.contains("-> relaunch (same id and history)")), "{id}:\n{plan}");
    }
    let (code, out) = restore("--yes");
    println!("{out}");
    assert_eq!(code, 0, "{out}");
    let order: Vec<&str> = out.lines().filter(|l| l.starts_with("[OK] ") && l.contains(" -> stopped ")).filter_map(|l| l.strip_prefix("[OK] ")?.split(':').next()).collect();
    assert_eq!(order, ids.iter().map(String::as_str).collect::<Vec<_>>(), "ship, then commodore, then admiral:\n{out}");
    for (id, sid, pid) in &before {
        let m = w.wait(60, || w.member(id).filter(|m| !m["session"]["pid"].is_null())).unwrap_or_else(|| panic!("{id} live again:\n{out}"));
        assert_eq!(m["session"]["session_id"].as_str(), Some(sid.as_str()), "{id}: the same conversation");
        assert_ne!(m["session"]["pid"].as_i64(), Some(*pid), "{id}: a new worker process");
        assert_eq!(m["session"]["labels"]["NIMITZ_ID"], id.as_str(), "{id}: its labels: {m}");
    }
    w.finish();
}

/// `nimitz start` on real Claude Code (no multiplexer: the jobs only): `-f`
/// relaunches a live, current ship (the same conversation, a new process,
/// its labels); after a stop, a fleet-wide `start --all` wakes it by its id.
#[test]
fn e2e_start_f_relaunches_and_a_fleet_start_all_wakes_a_stopped_ship() {
    if !gated() {
        return;
    }
    let w = E2e::new(95);
    w.fleet_with_ship(OK_PROMPT, "default");
    w.start_ship_idle();
    let ship = w.ship();
    let m = w.member(&ship).unwrap();
    let (sid, pid) = (m["session"]["session_id"].as_str().unwrap().to_string(), m["session"]["pid"].as_i64().unwrap());
    let plan = w.ok(&["start", &ship, "-f", "--plan"]);
    assert!(plan.lines().any(|l| l.trim_start().starts_with("SHIP") && l.contains("relaunch")), "{plan}");
    let out = w.ok(&["start", &ship, "-f", "--yes"]);
    println!("{out}");
    let m = w.wait(60, || w.member(&ship).filter(|m| !m["session"]["pid"].is_null() && m["session"]["pid"].as_i64() != Some(pid))).unwrap_or_else(|| panic!("relaunched:\n{out}"));
    assert_eq!(m["session"]["session_id"].as_str(), Some(sid.as_str()), "the same conversation");
    assert_eq!(m["session"]["labels"]["NIMITZ_ID"], ship.as_str(), "{m}");
    w.wait(120, || w.member(&ship).filter(|m| m["status"] == "Idle")).expect("idle after the relaunch");
    w.ok(&["stop", &ship, "--yes"]);
    w.wait(30, || w.member(&ship).filter(|m| m["session"]["pid"].is_null())).expect("stopped");
    let out = w.ok(&["start", &w.fleet, "--all", "--yes"]);
    println!("{out}");
    let m = w.wait(60, || w.member(&ship).filter(|m| !m["session"]["pid"].is_null())).unwrap_or_else(|| panic!("woken:\n{out}"));
    assert_eq!(m["session"]["session_id"].as_str(), Some(sid.as_str()), "woken by its id");
    w.finish();
}

/// `nimitz trust` replaces the trust prompt: in a git repo Claude Code does
/// not trust (a fresh `git init` under the test base: a parent's trust never
/// covers a repository), a background launch is refused; `nimitz trust <repo>`
/// writes the trust as the prompt would (the real `~/.claude.json`, under
/// Claude Code's lock, a backup in the test's state), and the same launch
/// starts. The trust is removed again at the end, pass or fail.
#[test]
fn e2e_trust_lets_a_background_job_start_in_an_untrusted_repo() {
    if !gated() {
        return;
    }
    let w = E2e::new(96);
    let repo = w.base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let mut git = w.scrubbed("git");
    git.args(["init", "-q"]).current_dir(&repo);
    assert_eq!(E2e::run(git).0, 0, "git init");
    w.settings(&repo);
    struct Forget<'a>(&'a nimitz_core::env::Env, PathBuf);
    impl Drop for Forget<'_> {
        fn drop(&mut self) {
            if let Err(e) = nimitz_core::trust::forget(self.0, &self.1) {
                println!("[WARN] e2e #96: the trust of {} is left: {e}", self.1.display());
            }
        }
    }
    let _forget = Forget(&w.env, repo.clone());
    assert!(nimitz_core::trust::untrusted(&w.env, &repo).is_some(), "the test repo starts untrusted");
    let name = format!("[{}]-TRUST", w.fleet);
    let launch = || w.claude(&repo, &["--bg", "--name", &name, "--model", &cheap_model(), "Reply with the word ok and nothing else."]);
    let (code, out) = launch();
    println!("before: exit {code}: {out}");
    assert!(out.to_lowercase().contains("trust"), "refused for want of trust:\n{out}");
    assert!(w.jobs_in(&repo).is_empty(), "no job before the trust: {:?}", w.jobs_in(&repo));
    let plan = w.ok(&["trust", repo.to_str().unwrap(), "--plan"]);
    assert!(plan.contains("hooks and MCP servers") && plan.contains(".claude.json"), "{plan}");
    println!("{}", w.ok(&["trust", repo.to_str().unwrap(), "--yes"]));
    assert!(nimitz_core::trust::is_written(&w.env, &repo));
    assert!(nimitz_core::trust::untrusted(&w.env, &repo).is_none(), "check agrees");
    let (code, out) = launch();
    println!("after: exit {code}: {out}");
    assert_eq!(code, 0, "{out}");
    w.wait(60, || (!w.jobs_in(&repo).is_empty()).then_some(())).unwrap_or_else(|| panic!("a job in the repo:\n{out}"));
    w.finish();
}

/// An update from nimitz 0.4 (`NIMITZ_E2E_OLD_BIN`: a 0.4.0 build; gated on
/// it as well as `NIMITZ_E2E=1`): the old binary creates the fleet and the
/// ship, and starts it (its hooks bind it); then this build, as if the user
/// updated the binary on PATH: its `status` migrates (the one line), shows
/// the ship live on the same job and session with the same lineage, and
/// `start <ship> -f` relaunches it into the same conversation.
#[test]
fn e2e_97_an_update_from_0_4_migrates_and_the_ship_lives_on() {
    if !gated() {
        return;
    }
    let Some(old) = std::env::var("NIMITZ_E2E_OLD_BIN").ok().filter(|p| !p.is_empty()) else {
        println!("[OK] skipped: set NIMITZ_E2E_OLD_BIN to a 0.4.0 nimitz build");
        return;
    };
    let w = E2e::new(97);
    w.use_bin(&old);
    let (_, v) = w.nz(&["--version"]);
    println!("old binary: {}", v.trim());
    assert!(v.trim().starts_with("nimitz 0.4."), "NIMITZ_E2E_OLD_BIN is not a 0.4 build: {v}");
    w.fleet_with_ship(OK_PROMPT, "default");
    let m = w.start_ship_idle();
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    let before = w.registry();
    assert_eq!(before["version"], 2, "a 0.4 registry: {before}");
    let lineage = before["identities"][w.ship()]["lineage"].clone();
    assert!(!w.base.join("state").join("formats.json").exists());

    // the update: this build on PATH and in the member folders' hooks
    w.use_bin(EXE);
    for d in [w.ship_dir(), w.fleet_dir().join("admiral"), w.fleet_dir().join("squadrons").join("sq")] {
        w.settings(&d);
    }
    let out = w.ok(&["status"]);
    println!("{out}");
    let v = env!("CARGO_PKG_VERSION");
    assert!(out.contains(&format!("[OK] nimitz {v}: migrated registry 2->3")), "the migration line:\n{out}");
    let state = w.base.join("state");
    assert_eq!(read_json(&state.join("formats.json"))["nimitz"], v);
    assert_eq!(read_json(&state.join(format!("registry.json.pre-{v}")))["version"], 2, "the 0.4 file kept");
    let after = w.registry();
    assert_eq!((after["version"].as_u64(), after["min_nimitz"].as_str()), (Some(3), Some("0.5.0")));
    assert_eq!(after["identities"][w.ship()]["lineage"], lineage, "the lineage as the old binary left it");
    let again = w.ok(&["status"]);
    assert!(!again.contains("migrated"), "a second run migrates nothing:\n{again}");
    let live = w.member(&w.ship()).expect("the ship in status");
    assert!(!live["session"]["pid"].is_null(), "the ship is live: {live}");
    assert_eq!((live["session"]["short_id"].as_str(), live["session"]["session_id"].as_str()), (Some(job.as_str()), Some(sid.as_str())), "{live}");
    let pid = live["session"]["pid"].as_i64();
    assert!(!w.issue_codes().iter().any(|(c, _)| matches!(c.as_str(), "registry.corrupt" | "lineage.broken" | "config.too_new" | "format.failed")), "{:?}", w.issue_codes());

    // the old binary once more on the migrated (v3) registry, no aides: it
    // reads it (version 3 is past its own 2, the stamp a key it ignores),
    // shows the same ship, and moves nothing aside
    w.use_bin(&old);
    let (code, out) = w.nz(&["status", "--json"]);
    assert_eq!(code, 0, "the old binary's status on the v3 registry:\n{out}");
    let old_view: Value = serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{e}:\n{out}"));
    let seen = old_view["fleets"].as_array().into_iter().flatten().flat_map(|f| f["squadrons"].as_array().cloned().unwrap_or_default()).flat_map(|q| q["ships"].as_array().cloned().unwrap_or_default()).find(|m| m["id"] == w.ship().as_str());
    let seen = seen.unwrap_or_else(|| panic!("the old binary lost the ship:\n{out}"));
    assert_eq!(seen["session"]["session_id"].as_str(), Some(sid.as_str()), "{seen}");
    assert!(!std::fs::read_dir(&state).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("corrupt")), "the old binary moved the registry aside");
    assert_eq!(w.registry()["version"], 3, "the old binary kept the version");
    w.use_bin(EXE);
    assert!(!w.ok(&["status"]).contains("migrated"), "the new binary has nothing to migrate after it");

    // the new build relaunches the old build's ship: the same conversation, a new process
    let out = w.ok(&["start", &w.ship(), "-f", "--yes"]);
    println!("{out}");
    let m = w.wait(60, || w.member(&w.ship()).filter(|m| !m["session"]["pid"].is_null() && m["session"]["pid"].as_i64() != pid)).unwrap_or_else(|| panic!("relaunched:\n{out}"));
    assert_eq!(m["session"]["session_id"].as_str(), Some(sid.as_str()), "the same conversation: {m}");
    assert_eq!(m["session"]["labels"]["NIMITZ_ID"], w.ship().as_str(), "{m}");
    w.wait(120, || w.member(&w.ship()).filter(|m| m["status"] == "Idle")).expect("idle after the relaunch");
    let r = w.registry();
    let now = r["identities"][w.ship()]["lineage"].as_array().cloned().unwrap_or_default();
    assert_eq!(now.first(), lineage.as_array().and_then(|l| l.first()), "the lineage keeps its start: {now:?}");
    assert_eq!(r["version"], 3);
    w.finish();
}
