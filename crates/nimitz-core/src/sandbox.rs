//! Sandbox world (fake NIMITZ_HOME, fake ~/.claude, fake `claude`) for tests and demos.
//! The fake `claude` is `nimitz __fake-claude <sandbox> ...` (contracts.md section 10). Owner: core-sessions.
//!
//! The world covers every row of the relaunch state table (contracts.md section
//! 8) and every issue code of section 7, so one test per row can drive the real
//! planner and `apply` end to end. Nothing here touches the real `~/.claude`,
//! the real `NIMITZ_HOME` or any real session.
//!
//! The fake `claude` models the job model the round-2 report measured on
//! Claude Code 2.1.292 (`docs/design/session-mapping/notes/r2-*.md`):
//!
//! - a job is a folder `jobs/<short>/state.json` with `sessionId` (the birth
//!   id), `resumeSessionId` (the conversation now), `respawnFlags` (`--name`,
//!   `--permission-mode`, `--settings`: what every wake replays), `name`,
//!   `nameSource` and `state`; a live job also has a session file with
//!   `jobId`, `kind bg`, `status`, `entrypoint` and `bridgeSessionId`;
//! - `stop`/`kill` end the process (SessionEnd `other`) and keep the row;
//!   `rm` removes the row and keeps the transcript; `respawn` restarts it
//!   (SessionStart `resume`); `attach` on a stopped job wakes it;
//! - `--bg --resume <id>` of a stopped job with **no flags** wakes that job on
//!   `<id>` (same short id, `resumeSessionId` moves); with flags it makes a
//!   **copy** (new job, SessionStart `fork`) that carries only the flags
//!   passed; while the job's process is alive (any state, a `done` job stays
//!   alive until `claude stop`) it makes a copy under an auto-generated name
//!   and ignores `--name` (2.1.292); with no row but a transcript it makes a
//!   new job named by the short form of `<id>` (cli-03, check 1);
//! - `--bg` ignores `--session-id` and says so on stderr, and refuses an
//!   untrusted folder;
//! - `-p --resume <id>` (an SDK host) runs the conversation in a process with
//!   no job (`entrypoint sdk-cli`, `jobId null`); on a live job it is refused
//!   and still fires a SessionEnd with no labels (R-13, M21);
//! - `__inside <short> clear|branch <name>|resume <id>|rename <name>|fork`
//!   are what the slash commands do inside an attached job (same job, a new
//!   session id for clear and branch; `/fork` makes a second labelled job);
//! - `__retire <short>` is the daemon's idle retire (no SessionEnd, a
//!   `bg retire` line in `daemon.log`); `__crash <short>` a dead worker (state
//!   `crashed`, no SessionEnd); `__pin`/`__unpin <short>` edit `jobs/pins.json`;
//!   `__end <id>` ends an interactive or SDK process;
//! - two knobs in `world.json` model what a resume cannot see coming:
//!   `copy_on_resume` (a flag-free resume of a stopped job races the daemon's
//!   restart: a copy is started with the "started a copy as X" note and the
//!   job comes back live by itself, cli-31) and `resume_loses_history` (the
//!   wake fires SessionStart `startup`: Claude Code found no transcript, E8).
//!
//! Live sandbox sessions record pid 1 (always alive), so the real `kill(pid, 0)`
//! liveness check works unchanged; a stopped session simply has no session file.
//! The hooks get a stable fake pid per job (`NIMITZ_HOOK_PID`) and the path of
//! the session file (`NIMITZ_HOOK_SESSION_FILE`): what real Claude Code gives
//! them as the parent pid and `sessions/<pid>.json`.
//!
//! Hooks: the fake `claude` runs the hook command in `<base>/hook-cmd.json`
//! (argv; the sandbox writes `[<exe>, "hook"]`) with the session's labels in its
//! env, exactly as Claude Code would. With no such file it calls
//! `crate::hook::handle` in process, so a test can run before the `nimitz` CLI
//! exists. `crates/nimitz-core/src/bin/nimitz-sandbox.rs` is that stand-in
//! binary (`nimitz-sandbox __fake-claude <base> ...`, `nimitz-sandbox hook
//! <Event>`, `nimitz-sandbox create <base>`); integration tests reach it through
//! `env!("CARGO_BIN_EXE_nimitz-sandbox")`.
use crate::types::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const HOST: &str = "sandbox-host";
/// pid 1 is always alive, so `kill(pid, 0)` needs no fake.
pub const LIVE_PID: i64 = 1;
pub const CC_VERSION: &str = "2.1.288";

// ------------------------------------------------------------------ small helpers

fn write_json(p: &Path, v: &Value) {
    if let Some(d) = p.parent() {
        let _ = fs::create_dir_all(d);
    }
    let _ = fs::write(p, serde_json::to_vec_pretty(v).unwrap_or_default());
}

fn read_json(p: &Path) -> Option<Value> {
    crate::evidence::first_json(&fs::read(p).ok()?)
}

fn ms(t: f64) -> i64 {
    (t * 1000.0) as i64
}

/// The pid the hooks of a job (or an interactive session) report: stable per
/// job, never a live process (the session file says pid 1 for liveness).
pub fn fake_pid(key: &str) -> i64 {
    100_000 + key.bytes().fold(0i64, |a, b| (a * 31 + b as i64) % 400_000)
}

/// Env vars that point nimitz/admiralty at the sandbox.
pub fn env_for(base: &Path) -> HashMap<String, String> {
    let b = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let p = |x: &str| b.join(x).to_string_lossy().into_owned();
    HashMap::from([
        ("HOME".into(), p("home")),
        ("NIMITZ_HOME".into(), p("nimitz")),
        ("NIMITZ_STATE".into(), p("state")),
        ("NIMITZ_CLAUDE_HOME".into(), p("claude_home")),
        // like ACME: the PATH `claude` is a wrapper that adds its own --settings
        ("NIMITZ_CLAUDE_BIN".into(), p("bin/claude-wrapper")),
        ("NIMITZ_NATIVE_CLAUDE_BIN".into(), p("bin/claude")),
        ("NIMITZ_HOSTNAME".into(), HOST.into()),
        ("NIMITZ_MUX".into(), "none".into()),
    ])
}

/// The Env of a sandbox.
pub fn env(base: &Path) -> crate::env::Env {
    crate::env::Env::from_vars(&env_for(base))
}

fn claude_home(base: &Path) -> PathBuf {
    base.join("claude_home")
}

fn job_file(base: &Path, short: &str) -> PathBuf {
    claude_home(base).join("jobs").join(short).join("state.json")
}

fn session_path(base: &Path, short: &str) -> PathBuf {
    claude_home(base).join("sessions").join(format!("{short}.json"))
}

/// The session file of an interactive or SDK process on `sid`.
fn interactive_path(base: &Path, sid: &str) -> PathBuf {
    claude_home(base).join("sessions").join(format!("i-{}.json", &sid[..sid.len().min(8)]))
}

/// Trust both layouts: `<claude_home>/.claude.json` (what `evidence` reads) and
/// `<claude_home>.json` (what `check` reads).
fn trust_files(base: &Path) -> Vec<PathBuf> {
    vec![claude_home(base).join(".claude.json"), base.join("claude_home.json")]
}

fn trust(base: &Path, dirs: &[String]) {
    let projects: serde_json::Map<String, Value> =
        dirs.iter().map(|d| (d.clone(), json!({"hasTrustDialogAccepted": true}))).collect();
    for f in trust_files(base) {
        write_json(&f, &json!({ "projects": projects }));
    }
}

fn trusted(base: &Path, cwd: &str) -> bool {
    // NIMITZ_HOME is trusted by `nimitz init`; anything else needs the trust file.
    let home = base.join("nimitz");
    let real_home = home.canonicalize().unwrap_or_else(|_| home.clone());
    if Path::new(cwd).starts_with(&home) || Path::new(cwd).starts_with(&real_home) {
        return true;
    }
    // Like real Claude Code: a parent's trust covers a folder, except inside a
    // git repository (a `.git` folder or file), which needs an entry at its root
    // or below (BUG-50, seen for worktrees and plain repos).
    let real = crate::env::canon(Path::new(cwd));
    // sessions ran here before: Claude Code accepted this folder already; inside
    // a git repo only a run since the repo appeared counts (M16: a folder that
    // became a git root after its sessions ran is asked again)
    let proj = claude_home(base).join("projects");
    let starts: Vec<Option<f64>> = [cwd.to_string(), real.to_string_lossy().into_owned()]
        .iter()
        .flat_map(|c| fs::read_dir(proj.join(crate::evidence::project_dir_name(c))).into_iter().flatten().flatten())
        .map(|e| crate::evidence::birth_secs(&e.path()))
        .collect();
    let root = crate::env::git_root(&real);
    let ran_here = match (starts.is_empty(), root.as_ref().and_then(|r| crate::env::git_birth(r))) {
        (true, _) => false,
        (false, Some(birth)) => starts.iter().any(|t| t.map_or(true, |t| t >= birth)),
        (false, None) => true,
    };
    // inside a git repo, a parent's trust is not enough, unless sessions ran
    // here since (Claude Code accepted the repo then; check.rs ran_since_repo)
    let floor = root.filter(|_| !ran_here);
    trust_files(base).iter().any(|f| {
        read_json(f)
            .and_then(|v| v.get("projects").and_then(|p| p.as_object()).cloned())
            .map(|projects| {
                let accepted = |a: &Path| projects.get(&a.to_string_lossy().into_owned()).and_then(|e| e.get("hasTrustDialogAccepted")).and_then(|b| b.as_bool()) == Some(true);
                let covers = |a: &Path| floor.as_ref().map(|r| a.starts_with(r)).unwrap_or(true);
                Path::new(cwd).ancestors().chain(real.ancestors()).any(|a| covers(a) && accepted(a))
            })
            .unwrap_or(false)
    })
}

/// `respawnFlags` as Claude Code keeps them: `--name`, `--permission-mode`,
/// `--settings`, each only when given.
fn respawn_flags(name: &str, mode: &str, settings: Option<&str>) -> Value {
    let mut f: Vec<String> = vec![];
    if !name.is_empty() {
        f.extend(["--name".into(), name.into()]);
    }
    if !mode.is_empty() {
        f.extend(["--permission-mode".into(), mode.into()]);
    }
    if let Some(s) = settings {
        f.extend(["--settings".into(), s.into()]);
    }
    json!(f)
}

/// What a job's `respawnFlags` say: (name, mode, settings).
fn flags_of(row: &Value) -> (Option<String>, Option<String>, Option<String>) {
    let argv: Vec<String> = row.get("respawnFlags").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let after = |flag: &str| argv.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone());
    (after("--name"), after("--permission-mode"), after("--settings"))
}

// ------------------------------------------------------------------ world building

struct World {
    base: PathBuf,
    now: f64,
    seq: u64,
    /// every session created: (name, session id, short id)
    sessions: Vec<(String, String, String)>,
    trusted: Vec<String>,
}

/// How to make one session in the world.
#[derive(Default, Clone)]
struct S<'a> {
    /// "background" (default) | "interactive"
    kind: &'a str,
    live: bool,
    status: &'a str,
    state: &'a str,
    started_ago: f64,
    /// labels written as a SessionStart event (none = an unlabelled session)
    labels: Option<&'a Id>,
    /// labels of another identity (labels.mismatch)
    labels_of: Option<&'a Id>,
    /// labels as given (a 0.3.x tender's, which no id makes any more)
    labels_raw: Option<std::collections::BTreeMap<String, String>>,
    /// a question left in the job row
    needs: Option<&'a str>,
    name_override: Option<String>,
}

impl World {
    fn next(&mut self) -> (String, String) {
        self.seq += 1;
        let short = format!("{:08x}", (self.seq * 0x9e37_79b9) as u32 ^ 0x5a5a_0000);
        (short.clone(), format!("{short}-5b0c-4d1e-9f2a-{:012x}", self.seq))
    }

    fn dir(&mut self, parts: &[&str], create: bool, trust_it: bool) -> String {
        let mut p = self.base.join("dirs");
        for x in parts {
            p = p.join(x);
        }
        if create {
            let _ = fs::create_dir_all(&p);
        }
        let s = p.to_string_lossy().into_owned();
        if trust_it {
            self.trusted.push(s.clone());
        }
        s
    }

    /// A session: a job row, a session file while live, and its labels as an event.
    fn session(&mut self, name: &str, cwd: &str, o: S) -> (String, String) {
        let (short, sid) = self.next();
        let kind = if o.kind.is_empty() { "background" } else { o.kind };
        let started = self.now - if o.started_ago == 0.0 { 3600.0 } else { o.started_ago };
        let name = o.name_override.clone().unwrap_or_else(|| name.to_string());
        let labels = o.labels_raw.clone().or_else(|| o.labels_of.or(o.labels).map(|id| crate::claude::labels_for(&env(&self.base), id)));
        let settings = labels.as_ref().map(|l| json!({"env": l, "worktree": {"bgIsolation": "none"}}).to_string());
        if kind == "background" {
            let mut row = json!({
                "sessionId": sid, "resumeSessionId": sid, "daemonShort": short, "name": name, "nameSource": "user", "cwd": cwd,
                "state": if o.state.is_empty() { if o.live { "running" } else { "done" } } else { o.state },
                "createdAt": ms(started), "updatedAt": ms(started), "tokens": 1200, "inFlight": {"tasks": 0, "queued": 0},
                // what every wake replays (check 2 of the report)
                "respawnFlags": respawn_flags(&name, "bypassPermissions", settings.as_deref()),
            });
            if let Some(n) = o.needs {
                row["needs"] = json!(n);
            }
            write_json(&job_file(&self.base, &short), &row);
        }
        if o.live {
            let mut d = json!({
                "pid": LIVE_PID, "sessionId": sid, "cwd": cwd, "startedAt": ms(started),
                "kind": if kind == "background" { "bg" } else { "interactive" }, "name": name,
                "status": if o.status.is_empty() { "idle" } else { o.status },
                "statusUpdatedAt": ms(self.now - 60.0), "bridgeSessionId": null,
                "entrypoint": "cli", "version": CC_VERSION,
            });
            if o.status == "waiting" {
                d["waitingFor"] = json!("permission prompt");
            }
            if kind == "background" {
                d["jobId"] = json!(short);
            }
            write_json(&session_path(&self.base, &short), &d);
        }
        if let Some(l) = labels {
            let job = (kind == "background").then(|| short.as_str());
            self.event("SessionStart", &sid, Some("startup"), &l, started, fake_pid(&short), job, kind);
        }
        // every session that ran has a transcript in its folder's project dir
        let proj = claude_home(&self.base).join("projects").join(crate::evidence::project_dir_name(cwd));
        let _ = fs::create_dir_all(&proj);
        let _ = fs::write(proj.join(format!("{sid}.jsonl")), format!("{}\n", json!({"type": "user", "cwd": cwd, "sessionId": sid, "uuid": root_uuid(&sid), "parentUuid": null})));
        self.sessions.push((name, sid.clone(), short.clone()));
        (short, sid)
    }

    #[allow(clippy::too_many_arguments)]
    fn event(&self, event: &str, sid: &str, source: Option<&str>, labels: &std::collections::BTreeMap<String, String>, ts: f64, pid: i64, job: Option<&str>, kind: &str) {
        append_event(&self.base, &json!({
            "ts": ts, "event": event, "session_id": sid, "source": source,
            "pid": pid, "labels": labels, "job": job, "kind": if kind == "background" { "bg" } else { kind }, "entrypoint": "cli",
        }));
    }

    /// A finished interactive session: a transcript, no row, no session file.
    fn transcript(&mut self, cwd: &str, name: &str, ago: f64) -> String {
        let (_, sid) = self.next();
        let proj = claude_home(&self.base).join("projects").join(crate::evidence::project_dir_name(cwd));
        let _ = fs::create_dir_all(&proj);
        let log = proj.join(format!("{sid}.jsonl"));
        let _ = fs::write(
            &log,
            format!("{}\n{}\n", json!({"type": "custom-title", "customTitle": name, "sessionId": sid}), json!({"type": "user", "cwd": cwd, "sessionId": sid, "uuid": root_uuid(&sid), "parentUuid": null})),
        );
        write_json(&proj.join(&sid).join("custom-title.json"), &json!({"customTitle": name}));
        if let Ok(f) = fs::File::options().write(true).open(&log) {
            let _ = f.set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs_f64((self.now - ago).max(0.0)));
        }
        sid
    }

    #[allow(clippy::too_many_arguments)]
    fn subagent(&self, sid: &str, cwd: &str, agent: &str, label: &str, ago: f64, teammate: bool, team: &str) {
        let sd = claude_home(&self.base).join("projects").join(crate::evidence::project_dir_name(cwd)).join(sid).join("subagents");
        let mut meta = json!({"agentType": "general-purpose", "description": label});
        if teammate {
            meta["name"] = json!(label);
            meta["taskKind"] = json!("in_process_teammate");
            meta["teamName"] = json!(team);
        }
        write_json(&sd.join(format!("agent-{agent}.meta.json")), &meta);
        let log = sd.join(format!("agent-{agent}.jsonl"));
        let _ = fs::write(&log, format!("{}\n", json!({"agentId": agent, "type": "user"})));
        if let Ok(f) = fs::File::options().write(true).open(&log) {
            let _ = f.set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs_f64((self.now - ago).max(0.0)));
        }
    }

    fn task(&self, team: &str, id: i64, subject: &str, status: &str) {
        write_json(
            &claude_home(&self.base).join("tasks").join(team).join(format!("{id}.json")),
            &json!({"id": id.to_string(), "subject": subject, "status": status, "owner": "planner"}),
        );
    }

    /// Bind `sid` as `id`'s conversation; its job too when a job row runs it.
    fn bind(&self, id: &Id, cwd: &str, sid: &str, via: &str) {
        let e = env(&self.base);
        let job = self.sessions.iter().find(|(_, s, short)| s == sid && job_file(&self.base, short).is_file()).map(|(_, _, short)| short.clone());
        let _ = crate::registry::update(&e, |r| crate::registry::bind_job(r, id, id.role(), &id.name(), cwd, job.as_deref(), sid, via, self.now));
    }
}

fn append_event(base: &Path, line: &Value) {
    let p = base.join("state").join("events.jsonl");
    if let Some(d) = p.parent() {
        let _ = fs::create_dir_all(d);
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

/// The ships of squadron ARGOS, one per relaunch-table row and issue code.
const SHIPS: [(&str, &str); 16] = [
    ("CLEAN", "live, labels right, name right: nothing to do (row 1)"),
    ("DRIFT", "live under another name: refuse with a relaunch (row 2)"),
    ("NOLABEL", "live without labels: refuse with a relaunch (row 2)"),
    ("TWIN", "two live sessions: refuse and list both (row 3)"),
    ("STOPPED", "stopped with its labels: flag-free native resume (row 4)"),
    ("BARE", "stopped without labels: full relaunch, then rm the old row (row 5)"),
    ("FRESH", "no row, no transcript: cold launch (row 7)"),
    ("FAILED", "row `failed`: lineage broken, cold launch only with --force (row 8)"),
    ("COPIES", "stopped with copy rows: resume, then rm every extra row (row 9)"),
    ("GONE", "folder missing: refuse with the fix (row 10)"),
    ("UNTRUSTED", "folder not trusted by Claude Code: refuse (row 10)"),
    ("ELSEWHERE", "managed folder locked by another machine (ship.elsewhere)"),
    ("NOPROMPT", "no prompt in squadron.toml: cold launch refuses"),
    ("BROKEN", "registry points at a session with no row and no transcript (lineage.broken)"),
    ("TAKEN", "an unlabelled live session in its folder holds its name: a copy (copy)"),
    ("MISLABEL", "live with the labels of another identity (labels.mismatch)"),
];

/// Build a world with every relaunch state of contracts.md section 8. `exe` is the nimitz binary.
pub fn create(base: &Path, exe: &Path) -> Result<(), String> {
    if base.exists() {
        if !base.join("world.json").exists() && fs::read_dir(base).map(|mut d| d.next().is_some()).unwrap_or(false) {
            return Err(format!("{} exists and is not a sandbox; refusing to overwrite.", base.display()));
        }
        fs::remove_dir_all(base).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let base = base.canonicalize().map_err(|e| e.to_string())?;
    for d in ["home", "nimitz", "state", "claude_home/sessions", "claude_home/jobs", "claude_home/projects", "bin", "dirs"] {
        fs::create_dir_all(base.join(d)).map_err(|e| e.to_string())?;
    }
    // world.json first: the fake claude reads its version and its counter there.
    write_json(&base.join("world.json"), &json!({"next": 1000, "cc_version": CC_VERSION}));
    write_json(&base.join("hook-cmd.json"), &json!([exe.to_string_lossy(), "hook"]));
    let bin = base.join("bin").join("claude");
    fs::write(&bin, format!("#!/bin/sh\nexec '{}' __fake-claude '{}' \"$@\"\n", exe.display(), base.display())).map_err(|e| e.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    // The ACME wrapper (findings-acme-2.1.288.md): it puts its own `--settings`
    // before your arguments, so a resume through it is always a copy. Cold
    // launches go through it; resume, stop, rm and agents must not.
    let wrapper = base.join("bin").join("claude-wrapper");
    fs::write(
        &wrapper,
        format!("#!/bin/sh\nexec '{}' --settings '{{\"env\":{{\"CLAUDE_CODE_USE_BEDROCK\":\"1\"}}}}' \"$@\"\n", bin.display()),
    )
    .map_err(|e| e.to_string())?;
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;

    let e = env(&base);
    let mut w = World { base: base.clone(), now: crate::evidence::now(), seq: 0, sessions: vec![], trusted: vec![] };

    // ---------------------------------------------------------------- configs
    // MAIN: the fleet every relaunch-table test drives.
    let plan = crate::config::plan_fleet_new(&e, "MAIN", "sandbox");
    apply_cfg(&e, &plan)?;
    // the machine label must point at this sandbox's hostname
    let fleet_toml = e.fleet_dir("MAIN").join("fleet.toml");
    let raw = fs::read_to_string(&fleet_toml).map_err(|e| e.to_string())?;
    fs::write(&fleet_toml, raw.replace(&format!("hostname = \"{}\"", e.hostname), &format!("hostname = \"{HOST}\""))).map_err(|e| e.to_string())?;
    apply_cfg(&e, &crate::config::plan_squadron_new(&e, "MAIN", "ARGOS", "Everything the relaunch table needs.", Comms::default(), None))?;
    apply_cfg(&e, &crate::config::plan_squadron_new(&e, "MAIN", "BOREAL", "A squadron whose commodore runs as a background job.", Comms::Mode("commodore-only".into()), None))?;

    let mut dirs: HashMap<&str, String> = HashMap::new();
    for (ship, _) in SHIPS {
        let (create_dir, trust_it, managed) = match ship {
            "GONE" => (false, true, false),
            "UNTRUSTED" => (true, false, false),
            "ELSEWHERE" => (false, false, true),
            _ => (true, true, false),
        };
        let dir = if managed {
            "managed".to_string()
        } else {
            w.dir(&["main", &ship.to_lowercase()], create_dir, trust_it)
        };
        let mut def = ShipDef {
            name: ship.to_string(),
            prompt: Some(format!("You are [MAIN.ARGOS]-{ship}.")),
            dirs: [("sandbox".to_string(), dir.clone())].into(),
            ..Default::default()
        };
        if ship == "NOPROMPT" {
            def.prompt = None;
        }
        apply_cfg(&e, &crate::config::plan_ship_new(&e, "MAIN", "ARGOS", def))?;
        dirs.insert(ship, dir);
    }
    // Resolve the managed folders now that the config exists.
    let fod = crate::config::load_fleet(&e, "MAIN").map_err(|i| i.message)?;
    for (ship, dir) in dirs.iter_mut() {
        if dir == "managed" {
            let d = crate::config::member_dir(&e, &fod, &Id::ship("MAIN", "ARGOS", ship)).ok_or("managed dir")?;
            let _ = fs::create_dir_all(&d);
            *dir = d.to_string_lossy().into_owned();
        }
    }
    let admiral_dir = fod.dir.join("admiral").to_string_lossy().into_owned();
    let argos_dir = fod.squadrons.iter().find(|(_, s)| s.squadron == "ARGOS").map(|(d, _)| d.clone()).ok_or("ARGOS dir")?;
    let boreal_dir = fod.squadrons.iter().find(|(_, s)| s.squadron == "BOREAL").map(|(d, _)| d.clone()).ok_or("BOREAL dir")?;

    // BETA: a 0.3.x admiral that ended and left only its transcript, bound in
    // the registry as its conversation (no job): a start resumes it with every
    // flag as a background job (row 6, the lead migration).
    apply_cfg(&e, &crate::config::plan_fleet_new(&e, "BETA", "sandbox"))?;
    let beta_toml = e.fleet_dir("BETA").join("fleet.toml");
    let raw = fs::read_to_string(&beta_toml).map_err(|e| e.to_string())?;
    fs::write(&beta_toml, raw.replace(&format!("hostname = \"{}\"", e.hostname), &format!("hostname = \"{HOST}\""))).map_err(|e| e.to_string())?;
    let beta_admiral = e.fleet_dir("BETA").join("admiral").to_string_lossy().into_owned();

    // LAYOUT: a CLAUDE.md in the fleet folder (config.layout).
    apply_cfg(&e, &crate::config::plan_fleet_new(&e, "LAYOUT", "sandbox"))?;
    fs::write(e.fleet_dir("LAYOUT").join("CLAUDE.md"), "every child session would inherit this\n").map_err(|e| e.to_string())?;
    // A fleet whose fleet.toml does not parse (config.invalid) is NOT part of the
    // base world: such an issue carries no identity, so `check` treats it as in
    // scope for every start. `break_a_config` adds one when a test wants it.

    // ---------------------------------------------------------------- sessions
    fn bg<'a>(live: bool, labels: Option<&'a Id>) -> S<'a> {
        S { live, labels, ..Default::default() }
    }

    // The admiral: a live, clean background job (every member is one in
    // 0.4.0), so a ship's start needs no parent work.
    let adm = Id::admiral("MAIN");
    let (_, adm_sid) = w.session(&adm.name(), &admiral_dir, S { status: "busy", ..bg(true, Some(&adm)) });
    w.bind(&adm, &admiral_dir, &adm_sid, "launch");
    let team = format!("session-{}", &adm_sid[..8]);
    w.subagent(&adm_sid, &admiral_dir, "planner-1", "planner", 5.0, true, &team);
    w.subagent(&adm_sid, &admiral_dir, "worker-1", "Draft the order", 4000.0, false, &team);
    w.task(&team, 1, "Plan the sortie", "in_progress");

    // ARGOS's commodore: a live, clean background job.
    let argos = Id::commodore("MAIN", "ARGOS");
    let argos_dir_s = argos_dir.to_string_lossy().into_owned();
    let (_, argos_sid) = w.session(&argos.name(), &argos_dir_s, bg(true, Some(&argos)));
    w.bind(&argos, &argos_dir_s, &argos_sid, "launch");

    // BOREAL's commodore: a background job too, in a commodore-only squadron.
    let boreal = Id::commodore("MAIN", "BOREAL");
    let boreal_dir_s = boreal_dir.to_string_lossy().into_owned();
    let (_, boreal_sid) = w.session(&boreal.name(), &boreal_dir_s, bg(true, Some(&boreal)));
    w.bind(&boreal, &boreal_dir_s, &boreal_sid, "launch");

    // One session shape per ship.
    for (ship, _) in SHIPS {
        let id = Id::ship("MAIN", "ARGOS", ship);
        let cwd = dirs.get(ship).cloned().unwrap_or_default();
        match ship {
            "CLEAN" => {
                let (_, sid) = w.session(&id.name(), &cwd, bg(true, Some(&id)));
                w.bind(&id, &cwd, &sid, "launch");
            }
            "DRIFT" => {
                let (_, sid) = w.session(&id.name(), &cwd, S { name_override: Some(format!("{}-quiet-otter", id.name())), ..bg(true, Some(&id)) });
                w.bind(&id, &cwd, &sid, "launch");
            }
            "NOLABEL" => {
                let (_, sid) = w.session(&id.name(), &cwd, bg(true, None));
                w.bind(&id, &cwd, &sid, "relaunch"); // known by lineage, not by labels
            }
            "TWIN" => {
                let (_, a) = w.session(&id.name(), &cwd, bg(true, Some(&id)));
                let (_, b) = w.session(&id.name(), &cwd, S { status: "busy", ..bg(true, Some(&id)) });
                w.bind(&id, &cwd, &a, "launch");
                w.bind(&id, &cwd, &b, "relaunch");
            }
            "STOPPED" => {
                let (_, sid) = w.session(&id.name(), &cwd, bg(false, Some(&id)));
                w.bind(&id, &cwd, &sid, "launch");
            }
            "BARE" => {
                let (_, sid) = w.session(&id.name(), &cwd, bg(false, None));
                w.bind(&id, &cwd, &sid, "relaunch");
            }
            "FRESH" => {} // nothing at all
            "FAILED" => {
                let (_, sid) = w.session(&id.name(), &cwd, S { state: "failed", ..bg(false, Some(&id)) });
                w.bind(&id, &cwd, &sid, "launch");
            }
            "COPIES" => {
                let (_, sid) = w.session(&id.name(), &cwd, bg(false, Some(&id)));
                w.bind(&id, &cwd, &sid, "launch");
                for _ in 0..2 {
                    let (c_short, c) = w.session(&id.name(), &cwd, bg(false, Some(&id)));
                    // a copy is in the lineage but is not current
                    let ev = env(&base);
                    let _ = crate::registry::update(&ev, |r| {
                        if let Some(rec) = r.identities.get_mut(&id.0) {
                            rec.lineage.push(LineageEntry { session_id: c.clone(), job: Some(c_short.clone()), via: "relaunch".into(), at: w.now, home: None });
                        }
                    });
                }
            }
            "GONE" | "UNTRUSTED" | "NOPROMPT" => {} // the folder or the config is the point
            "ELSEWHERE" => {
                write_json(&crate::sessions::ship_lock_path(&cwd), &json!({"machine": "far-away-box", "since": w.now}));
            }
            "BROKEN" => {
                let (_, sid) = w.next();
                w.bind(&id, &cwd, &sid, "launch"); // a current id with no row and no transcript
            }
            "TAKEN" => {
                // an unlabelled live session holding the ship's name
                w.session(&id.name(), &cwd, bg(true, None));
            }
            "MISLABEL" => {
                let other = Id::ship("MAIN", "ARGOS", "TAKEN"); // an identity already in trouble, so CLEAN stays clean
                let (_, sid) = w.session(&id.name(), &cwd, S { labels_of: Some(&other), ..bg(true, Some(&id)) });
                w.bind(&id, &cwd, &sid, "launch");
            }
            _ => {}
        }
    }

    // A 0.3.x tender (the role is gone, plan-v4 section 8): a stopped job row
    // with tender labels, and a v1 registry entry word for word, which the next
    // load retires (`tender.retired` in check; `nimitz rm` takes the row).
    let tender = Id("MAIN/ARGOS/~fixci".into());
    let mut tl = crate::claude::labels_for(&env(&base), &tender);
    tl.insert("NIMITZ_ROLE".into(), "tender".into());
    tl.insert("NIMITZ_PARENT".into(), "MAIN/ARGOS".into());
    let (_, t_sid) = w.session("[MAIN.ARGOS]-COMMODORE/fixci", &argos_dir_s, S { labels_raw: Some(tl), ..bg(false, None) });
    let reg_path = env(&base).registry_path();
    if let Some(mut reg) = read_json(&reg_path) {
        reg["identities"][&tender.0] = json!({
            "role": "tender", "name": "[MAIN.ARGOS]-COMMODORE/fixci", "cwd": argos_dir_s, "current": t_sid,
            "lineage": [{"session_id": t_sid, "bound_at": w.now, "via": "launch"}],
        });
        write_json(&reg_path, &reg);
    }

    // BETA's admiral: a transcript only, known to the registry (row 6).
    let _ = fs::create_dir_all(&beta_admiral);
    let beta_sid = w.transcript(&beta_admiral, &Id::admiral("BETA").name(), 86400.0);
    w.bind(&Id::admiral("BETA"), &beta_admiral, &beta_sid, "launch");

    // A stowaway: an unlabelled session in the fleet's own folder (a lead's
    // folder is exclusive: a session there is its `lead.intruder` instead).
    let fleet_dir_s = fod.dir.to_string_lossy().into_owned();
    w.session("quick look at the logs", &fleet_dir_s, bg(true, None));

    trust(&base, &w.trusted.clone());
    let mut world = read_json(&base.join("world.json")).unwrap_or(json!({}));
    world["sessions"] = json!(w.sessions.iter().map(|(n, s, j)| json!({"name": n, "session_id": s, "short_id": j})).collect::<Vec<_>>());
    write_json(&base.join("world.json"), &world);
    Ok(())
}

/// Add a fleet whose `fleet.toml` does not parse (issue `config.invalid`).
pub fn break_a_config(base: &Path, fleet: &str) {
    let dir = env(base).fleet_dir(fleet);
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join("fleet.toml"), "fleet = ");
}

/// A live, labelled background session of `id` in `cwd`, bound in the registry
/// as its current session when `bind` (a member added after `create`). Its
/// session id. (The hook binds it anyway; `bind` records nimitz's own `via`.)
pub fn live_session(base: &Path, id: &Id, cwd: &str, bind: bool) -> String {
    let base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let e = env(&base);
    let settings = crate::claude::settings_json(&e, id, true);
    let short = start(&base, cwd, &id.name(), "bypassPermissions", Some(&settings), "startup", true);
    let sid = job_state(&base, &short).and_then(|j| j.get("sessionId").and_then(|v| v.as_str()).map(str::to_string)).unwrap_or_default();
    if bind {
        let now = crate::evidence::now();
        let _ = crate::registry::update(&e, |r| crate::registry::bind_job(r, id, id.role(), &id.name(), cwd, Some(&short), &sid, "launch", now));
    }
    sid
}

fn apply_cfg(e: &crate::env::Env, plan: &Plan) -> Result<(), String> {
    if !plan.ok() {
        return Err(format!("sandbox config refused: {:?}", plan.refusals));
    }
    let out = crate::config::apply_config_plan(e, plan);
    if !out.verified {
        return Err(format!("sandbox config not verified: {:?}", out.issues));
    }
    Ok(())
}

// ------------------------------------------------------------------ the fake claude

struct Args {
    bg: bool,
    print: bool,
    name: Option<String>,
    mode: Option<String>,
    settings: Option<String>,
    resume: Option<String>,
    session_id: Option<String>,
    cont: bool,
    fork: bool,
    prompt: Option<String>,
}

fn parse(args: &[String]) -> Args {
    let mut a = Args { bg: false, print: false, name: None, mode: None, settings: None, resume: None, session_id: None, cont: false, fork: false, prompt: None };
    let mut i = 0;
    while i < args.len() {
        let take = |i: &mut usize| -> Option<String> {
            *i += 1;
            args.get(*i).cloned()
        };
        match args[i].as_str() {
            "--bg" => a.bg = true,
            "-p" | "--print" => a.print = true,
            "-c" | "--continue" => a.cont = true,
            "--fork-session" => a.fork = true,
            "--name" => a.name = take(&mut i),
            "--permission-mode" => a.mode = take(&mut i),
            "--settings" => {
                // several --settings (the wrapper's, then yours) merge, later keys winning
                let next = take(&mut i);
                a.settings = match (a.settings.take(), next) {
                    (Some(prev), Some(next)) => Some(merge_settings(&prev, &next)),
                    (prev, next) => next.or(prev),
                };
            }
            "--resume" => a.resume = take(&mut i),
            "--session-id" => a.session_id = take(&mut i),
            "--input-format" | "--output-format" => {
                take(&mut i);
            }
            s if !s.starts_with('-') => a.prompt = Some(s.to_string()),
            _ => {}
        }
        i += 1;
    }
    a
}

fn merge_settings(prev: &str, next: &str) -> String {
    let (Ok(Value::Object(mut p)), Ok(Value::Object(n))) = (serde_json::from_str::<Value>(prev), serde_json::from_str::<Value>(next)) else {
        return next.to_string();
    };
    for (k, v) in n {
        match (p.get_mut(&k), v) {
            (Some(Value::Object(old)), Value::Object(new)) => old.extend(new),
            (_, v) => {
                p.insert(k, v);
            }
        }
    }
    Value::Object(p).to_string()
}

fn labels_from_settings(settings: Option<&String>) -> std::collections::BTreeMap<String, String> {
    settings
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| v.get("env").and_then(|e| e.as_object()).cloned())
        .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
        .unwrap_or_default()
}

/// One hook call as the fake process `pid` would make it: `reason` for a
/// SessionEnd, `session_file` the process's session file (what the hook reads).
struct Hook<'a> {
    event: &'a str,
    sid: &'a str,
    source: Option<&'a str>,
    reason: Option<&'a str>,
    cwd: &'a str,
    pid: i64,
    session_file: Option<PathBuf>,
}

/// Run the configured hook, or `crate::hook::handle` in process.
fn fire(base: &Path, h: Hook, labels: &std::collections::BTreeMap<String, String>) {
    let mut input = json!({"session_id": h.sid, "hook_event_name": h.event, "cwd": h.cwd, "transcript_path": "", "scratchpad_dir": ""});
    if let Some(s) = h.source {
        input["source"] = json!(s);
    }
    if h.event == "SessionEnd" {
        input["reason"] = json!(h.reason.unwrap_or("other")); // what a `claude stop` gave in probe 3
    }
    let mut vars: HashMap<String, String> = env_for(base).into_iter().chain(labels.iter().map(|(k, v)| (k.clone(), v.clone()))).collect();
    vars.insert("NIMITZ_HOOK_PID".into(), h.pid.to_string());
    vars.insert("NIMITZ_HOOK_SESSION_FILE".into(), h.session_file.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "/nonexistent".into()));
    if let Some(cmd) = read_json(&base.join("hook-cmd.json")).and_then(|v| v.as_array().cloned()) {
        let argv: Vec<String> = cmd.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        if let Some(bin) = argv.first() {
            let mut c = std::process::Command::new(bin);
            c.args(&argv[1..]).arg(h.event).envs(&vars).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null());
            if let Ok(mut child) = c.spawn() {
                if let Some(mut si) = child.stdin.take() {
                    let _ = si.write_all(input.to_string().as_bytes());
                }
                let _ = child.wait();
                return;
            }
        }
    }
    let e = crate::env::Env::from_vars(&vars);
    let _ = crate::hook::handle(&e, h.event, &input.to_string(), &vars);
}

/// The hooks of a job: its stable fake pid and its session file.
fn job_hook<'a>(base: &Path, short: &str, event: &'a str, sid: &'a str, source: Option<&'a str>, cwd: &'a str) -> Hook<'a> {
    Hook { event, sid, source, reason: None, cwd, pid: fake_pid(short), session_file: Some(session_path(base, short)) }
}

fn rows(base: &Path) -> Vec<crate::evidence::SessionRow> {
    let e = env(base);
    crate::evidence::rows_from_files(&e, &mut crate::evidence::FileCache::default()).unwrap_or_default()
}

/// A row by its job id, the conversation it runs now, or its birth id.
fn find(base: &Path, needle: &str) -> Option<crate::evidence::SessionRow> {
    rows(base).into_iter().find(|r| r.short_id.as_deref() == Some(needle) || r.session_ids().any(|s| s == needle))
}

fn job_state(base: &Path, short: &str) -> Option<Value> {
    read_json(&job_file(base, short))
}

fn save_job(base: &Path, short: &str, j: &Value) {
    write_json(&job_file(base, short), j);
}

/// A boolean knob in `world.json` (`copy_on_resume`, `resume_loses_history`).
fn knob(base: &Path, name: &str) -> bool {
    read_json(&base.join("world.json")).and_then(|w| w.get(name).and_then(|v| v.as_bool())).unwrap_or(false)
}

fn next_id(base: &Path) -> (String, String) {
    let mut w = read_json(&base.join("world.json")).unwrap_or(json!({}));
    let n = w.get("next").and_then(|v| v.as_u64()).unwrap_or(1) + 1;
    w["next"] = json!(n);
    write_json(&base.join("world.json"), &w);
    let short = format!("{:08x}", (n * 0x9e37_79b9) as u32 ^ 0x1234_0000);
    (short.clone(), format!("{short}-0000-4000-8000-{n:012x}"))
}

/// The name Claude Code gives a copy started without `--name`: the original's
/// name and two words, e.g. `[MAIN.ARGOS]-CLEAN-melodic-mitten` (BUG-11).
fn copy_name(base: &Path, of: &str) -> String {
    const WORDS: [&str; 8] = ["melodic", "mitten", "quiet", "otter", "amber", "falcon", "brisk", "lantern"];
    let (_, sid) = next_id(base);
    let n = sid.bytes().fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize));
    format!("{of}-{}-{}", WORDS[n % 8], WORDS[(n / 8) % 8])
}

/// The uuid of a conversation's first message, as real transcripts carry
/// (`uuid`, `parentUuid`); a copy of the transcript shares it.
fn root_uuid(sid: &str) -> String {
    format!("{}-root-{}", &sid[..sid.len().min(8)], sid.len())
}

/// A copy's job row records the session it was forked from.
fn record_fork(base: &Path, short: &str, from: &str) {
    if let Some(mut j) = job_state(base, short) {
        j["forkedFrom"] = json!(from);
        save_job(base, short, &j);
    }
}

fn transcript_of(base: &Path, sid: &str) -> Option<PathBuf> {
    fs::read_dir(claude_home(base).join("projects"))
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path().join(format!("{sid}.jsonl")))
        .find(|p| p.is_file())
}

fn transcript_exists(base: &Path, sid: &str) -> bool {
    transcript_of(base, sid).is_some()
}

/// Append one line to a session's transcript (made under `cwd` when new).
fn log_line(base: &Path, cwd: &str, sid: &str, line: &Value) {
    let log = transcript_of(base, sid).unwrap_or_else(|| claude_home(base).join("projects").join(crate::evidence::project_dir_name(cwd)).join(format!("{sid}.jsonl")));
    if let Some(d) = log.parent() {
        let _ = fs::create_dir_all(d);
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&log) {
        let _ = writeln!(f, "{line}");
    }
}

/// A copy of `old`'s transcript under `new` (what `/branch` and `/fork` do).
fn copy_transcript(base: &Path, cwd: &str, old: &str, new: &str) {
    let body = transcript_of(base, old).and_then(|p| fs::read_to_string(p).ok()).unwrap_or_default();
    log_line(base, cwd, new, &json!({"type": "system", "sessionId": new, "source": "fork", "copyOf": old}));
    if let Some(p) = transcript_of(base, new) {
        let _ = fs::write(&p, format!("{body}{}\n", json!({"type": "system", "sessionId": new, "source": "fork"})));
    }
}

fn write_session_file(base: &Path, short: &str, sid: &str, cwd: &str, name: &str, t: f64) {
    write_json(
        &session_path(base, short),
        &json!({"pid": LIVE_PID, "sessionId": sid, "cwd": cwd, "startedAt": ms(t), "kind": "bg", "name": name,
                "jobId": short, "status": "idle", "statusUpdatedAt": ms(t), "bridgeSessionId": null,
                "entrypoint": "cli", "version": CC_VERSION}),
    );
}

/// Make a live background session (a cold launch or a copy) and fire its hooks.
fn start(base: &Path, cwd: &str, name: &str, mode: &str, settings: Option<&String>, source: &str, labels_fire: bool) -> String {
    start_as(base, cwd, name, mode, settings, source, labels_fire, None)
}

/// `start`, optionally keeping a given session id (a resume from a transcript:
/// the new job is named by the short form of that id, cli-03).
#[allow(clippy::too_many_arguments)]
fn start_as(base: &Path, cwd: &str, name: &str, mode: &str, settings: Option<&String>, source: &str, labels_fire: bool, keep: Option<&str>) -> String {
    let (short, sid) = match keep {
        Some(k) => (k[..k.len().min(8)].to_string(), k.to_string()),
        None => next_id(base),
    };
    let t = crate::evidence::now();
    let labels = labels_from_settings(settings);
    let row = json!({"sessionId": sid, "resumeSessionId": sid, "daemonShort": short, "name": name, "nameSource": if name.is_empty() { "auto" } else { "user" },
                     "cwd": cwd, "state": "running", "createdAt": ms(t), "updatedAt": ms(t),
                     "tokens": 0, "inFlight": {"tasks": 0, "queued": 0},
                     "respawnFlags": respawn_flags(name, mode, settings.map(String::as_str))});
    save_job(base, &short, &row);
    write_session_file(base, &short, &sid, cwd, name, t);
    // The conversation: a new id starts a transcript of its own (a copy made
    // with flags has NO history, BUG-53; a `--fork-session` or a copy of a
    // running job gets the original's transcript from its caller,
    // `copy_transcript`); a kept id appends to the one it has, wherever it is.
    log_line(base, cwd, &sid, &json!({"type": "user", "sessionId": sid, "cwd": cwd, "source": source, "uuid": root_uuid(&sid), "parentUuid": null}));
    // A copy with no --settings carries no hooks at all, so it fires none (probe 2b, 2c2).
    if labels_fire {
        fire(base, job_hook(base, &short, "SessionStart", &sid, Some(source), cwd), &labels);
    }
    short
}

/// Wake a stopped job on `sid` (one of its own ids) with its saved options:
/// same short id, `resumeSessionId` moves, SessionStart `resume` with the
/// labels its `respawnFlags` carry.
fn wake(base: &Path, row: &crate::evidence::SessionRow, sid: &str, source: &str) {
    let short = row.short_id.clone().unwrap_or_default();
    let t = crate::evidence::now();
    let Some(mut j) = job_state(base, &short) else { return };
    let (name, _, settings) = flags_of(&j);
    let name = j.get("name").and_then(|n| n.as_str()).map(str::to_string).or(name).unwrap_or_else(|| row.name.clone());
    j["state"] = json!("running");
    j["resumeSessionId"] = json!(sid);
    j["updatedAt"] = json!(ms(t));
    save_job(base, &short, &j);
    write_session_file(base, &short, sid, &row.cwd, &name, t);
    let labels = labels_from_settings(settings.as_ref());
    // the transcript is gone: Claude Code starts the id over (E8, 5a)
    let source = if knob(base, "resume_loses_history") { "startup" } else { source };
    if !labels.is_empty() {
        fire(base, job_hook(base, &short, "SessionStart", sid, Some(source), &row.cwd), &labels);
    }
}

/// End a job's process: the session file goes, the row stays with `state`,
/// and (unless `silent`: a crash, a retire) a SessionEnd fires with its labels.
fn end_job(base: &Path, row: &crate::evidence::SessionRow, state: &str, reason: &str, silent: bool) {
    let short = row.short_id.clone().unwrap_or_default();
    let _ = fs::remove_file(session_path(base, &short));
    if let Some(mut j) = job_state(base, &short) {
        j["state"] = json!(state);
        j["updatedAt"] = json!(ms(crate::evidence::now()));
        save_job(base, &short, &j);
        if !silent {
            let (_, _, settings) = flags_of(&j);
            let labels = labels_from_settings(settings.as_ref());
            let h = Hook { event: "SessionEnd", sid: &row.session_id, source: None, reason: Some(reason), cwd: &row.cwd, pid: fake_pid(&short), session_file: None };
            fire(base, h, &labels);
        }
    }
}

/// What `/clear`, `/branch`, `/resume`, `/rename` and `/fork` do inside an
/// attached job (round 2, area `inside`): the job keeps its short id and pid;
/// clear and branch move the conversation to a new id (SessionEnd + SessionStart
/// with the labels, same pid); `/branch` and `/rename` name the session;
/// `/fork` makes a second job with the labels (SessionStart `fork`).
fn inside(base: &Path, short: &str, verb: &str, arg: Option<&str>) -> i32 {
    let Some(row) = find(base, short).filter(|r| r.short_id.as_deref() == Some(short)) else {
        eprintln!("[fake claude] no job {short}");
        return 1;
    };
    if !row.live() {
        eprintln!("[fake claude] {short} is not running; attach wakes it first");
        return 1;
    }
    let Some(mut j) = job_state(base, short) else { return 1 };
    let (_, mode, settings) = flags_of(&j);
    let labels = labels_from_settings(settings.as_ref());
    let old = row.session_id.clone();
    let t = crate::evidence::now();
    // the conversation moves to `new` in the same process: SessionEnd (old),
    // then SessionStart (new); `follow` moves `resumeSessionId` too
    let move_to = |j: &mut Value, new: &str, end_reason: &str, start_source: &str, name: Option<&str>, follow: bool| {
        fire(base, Hook { reason: Some(end_reason), ..job_hook(base, short, "SessionEnd", &old, None, &row.cwd) }, &labels);
        if follow {
            j["resumeSessionId"] = json!(new);
        }
        if let Some(n) = name {
            j["name"] = json!(n);
            j["nameSource"] = json!("user");
        }
        j["updatedAt"] = json!(ms(t));
        save_job(base, short, j);
        let name = name.map(str::to_string).unwrap_or_else(|| row.name.clone());
        write_session_file(base, short, new, &row.cwd, &name, t);
        fire(base, job_hook(base, short, "SessionStart", new, Some(start_source), &row.cwd), &labels);
    };
    match verb {
        // a fresh conversation; the old one is saved (`/resume` brings it back)
        "clear" => {
            let (_, new) = next_id(base);
            log_line(base, &row.cwd, &new, &json!({"type": "user", "sessionId": new, "source": "clear", "uuid": root_uuid(&new), "parentUuid": null}));
            move_to(&mut j, &new, "clear", "clear", None, true);
            println!("cleared: {short} now runs {new}");
        }
        // a copy of the transcript under a new id; `resumeSessionId` does NOT
        // follow (3.4d: a stop/start would resume the pre-branch conversation)
        "branch" => {
            let (_, new) = next_id(base);
            copy_transcript(base, &row.cwd, &old, &new);
            move_to(&mut j, &new, "resume", "fork", arg, false);
            println!("branched: {short} now runs {new}");
        }
        // `/resume <own other id>`: the conversation moves, same pid, same job
        "resume" => {
            let Some(target) = arg else { return 2 };
            if !transcript_exists(base, target) {
                eprintln!("[fake claude] no conversation {target}");
                return 1;
            }
            move_to(&mut j, target, "resume", "resume", None, true);
            println!("resumed {target} in {short}");
        }
        // `/rename`: the row, the session file and `--name` in respawnFlags (cli-19)
        "rename" => {
            let Some(name) = arg else { return 2 };
            j["name"] = json!(name);
            j["nameSource"] = json!("user");
            j["respawnFlags"] = respawn_flags(name, mode.as_deref().unwrap_or(""), settings.as_deref());
            save_job(base, short, &j);
            if let Some(mut d) = read_json(&session_path(base, short)) {
                d["name"] = json!(name);
                write_json(&session_path(base, short), &d);
            }
            println!("renamed {short} to {name}");
        }
        // `/fork`: a second background job with the labels and a copy of the conversation (3.8)
        "fork" => {
            let (new_short, new) = next_id(base);
            let name = format!("{} (fork)", row.name);
            copy_transcript(base, &row.cwd, &old, &new);
            let nrow = json!({"sessionId": new, "resumeSessionId": new, "daemonShort": new_short, "name": name, "nameSource": "auto",
                              "cwd": row.cwd, "state": "running", "createdAt": ms(t), "updatedAt": ms(t), "interactiveLineage": true,
                              "forkedFrom": old, "tokens": 0, "inFlight": {"tasks": 0, "queued": 0},
                              "respawnFlags": respawn_flags(&name, mode.as_deref().unwrap_or(""), settings.as_deref())});
            save_job(base, &new_short, &nrow);
            write_session_file(base, &new_short, &new, &row.cwd, &name, t);
            if !labels.is_empty() {
                fire(base, job_hook(base, &new_short, "SessionStart", &new, Some("fork"), &row.cwd), &labels);
            }
            println!("forked {short} as {new_short}");
        }
        other => {
            eprintln!("[fake claude] __inside: unknown verb {other}");
            return 2;
        }
    }
    0
}

/// The fake `claude`: agents --json, --bg (cold/resume with copy semantics as probed), stop, rm, attach, --version.
pub fn fake_claude(base: &Path, args: &[String]) -> i32 {
    let base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    log_call(&base, args);
    let a = parse(args);
    let home = claude_home(&base);
    let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();

    if args.iter().any(|x| x == "--version") {
        let v = read_json(&base.join("world.json"))
            .and_then(|w| w.get("cc_version").and_then(|v| v.as_str().map(str::to_string)))
            .unwrap_or(CC_VERSION.into());
        println!("{v} (Claude Code)");
        return 0;
    }
    match args.first().map(String::as_str) {
        Some("agents") => {
            let out: Vec<Value> = rows(&base)
                .iter()
                .map(|r| {
                    // as the real CLI (2.1.292): a stopped job is listed by its
                    // birth id, a live one by the conversation it runs now
                    let listed = if r.pid.is_none() { r.birth_id.clone().unwrap_or_else(|| r.session_id.clone()) } else { r.session_id.clone() };
                    let mut v = json!({"cwd": r.cwd, "kind": r.kind, "sessionId": listed, "name": r.name,
                                       "startedAt": r.started_at.map(ms)});
                    if let Some(id) = &r.short_id {
                        v["id"] = json!(id);
                    }
                    if let Some(p) = r.pid {
                        v["pid"] = json!(p);
                        v["status"] = json!(r.status);
                    }
                    if let Some(s) = &r.state {
                        v["state"] = json!(s);
                    }
                    v
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            return 0;
        }
        Some("stop") | Some("kill") => {
            let Some(target) = args.get(1) else { return 2 };
            let Some(row) = find(&base, target) else {
                eprintln!("[fake claude] no session {target}");
                return 1;
            };
            if !row.live() {
                eprintln!("[fake claude] {target} is not running");
                return 1;
            }
            let short = row.short_id.clone().unwrap_or_default();
            // an idle job keeps `done`; a working one reads `stopped` (cli-01)
            let state = if row.status.as_deref() == Some("busy") { "stopped" } else { "done" };
            end_job(&base, &row, state, "other", false);
            println!("stopped {short}");
            return 0;
        }
        Some("rm") => {
            let Some(target) = args.get(1) else { return 2 };
            let Some(row) = find(&base, target) else {
                eprintln!("[fake claude] no job {target}");
                return 1;
            };
            if row.live() {
                eprintln!("[fake claude] {target} is still running; stop it first");
                return 1;
            }
            let short = row.short_id.clone().unwrap_or_default();
            let _ = fs::remove_dir_all(home.join("jobs").join(&short));
            // Claude Code leaves the pin behind (env-G9); nimitz removes its own (M24)
            println!("removed {short}");
            return 0;
        }
        Some("respawn") => {
            let Some(target) = args.get(1) else { return 2 };
            let Some(row) = find(&base, target).filter(|r| r.short_id.is_some()) else {
                eprintln!("[fake claude] no job {target}");
                return 1;
            };
            if row.live() {
                let _ = fs::remove_file(session_path(&base, row.short_id.as_deref().unwrap_or_default()));
            }
            let sid = row.session_id.clone();
            wake(&base, &row, &sid, "resume");
            println!("respawned {}", row.short_id.clone().unwrap_or_default());
            return 0;
        }
        Some("attach") => {
            // attach on a stopped job wakes it as itself (cli-07)
            if let Some(row) = args.get(1).and_then(|t| find(&base, t)).filter(|r| !r.live() && r.short_id.is_some()) {
                let sid = row.session_id.clone();
                wake(&base, &row, &sid, "resume");
            }
            println!("[fake claude] attached to {} (sandbox: nothing is running)", args.get(1).cloned().unwrap_or_default());
            return 0;
        }
        Some("__inside") => {
            let (Some(short), Some(verb)) = (args.get(1), args.get(2)) else { return 2 };
            return inside(&base, short, verb, args.get(3).map(String::as_str));
        }
        Some("__retire") | Some("__crash") => {
            let Some(target) = args.get(1) else { return 2 };
            let Some(row) = find(&base, target).filter(|r| r.live() && r.short_id.is_some()) else {
                eprintln!("[fake claude] {target} is not a running job");
                return 1;
            };
            let short = row.short_id.clone().unwrap_or_default();
            if args[0] == "__retire" {
                // the daemon's idle retire: no hook, the row untouched (E1)
                let _ = fs::remove_file(session_path(&base, &short));
                if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(home.join("daemon.log")) {
                    let stamp = iso_now();
                    // the real line, verbatim: `[<stamp>] [bg] bg retire <id>: settled, idle 61m`
                    let _ = writeln!(f, "[{stamp}] [bg] bg retire {short}: settled, idle 61m");
                    let _ = writeln!(f, "[{stamp}] [bg] bg settled {short} (done)");
                }
                println!("retired {short}");
            } else {
                // a dead worker: state crashed (or the state given: `blocked`,
                // `done` for a kill -9 that left the state untouched), no hook (cli-30)
                let state = args.get(2).map(String::as_str).unwrap_or("crashed");
                end_job(&base, &row, state, "", true);
                println!("crashed {short}");
            }
            return 0;
        }
        Some("__pin") | Some("__unpin") => {
            let Some(short) = args.get(1) else { return 2 };
            let p = home.join("jobs").join("pins.json");
            let mut pins: Vec<String> = read_json(&p).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
            pins.retain(|x| x != short);
            if args[0] == "__pin" {
                pins.push(short.clone());
            }
            write_json(&p, &json!(pins));
            return 0;
        }
        Some("__end") => {
            // an interactive or SDK process ends (`/exit`, stdin closed)
            let Some(target) = args.get(1) else { return 2 };
            let p = interactive_path(&base, target);
            let Some(d) = read_json(&p) else {
                eprintln!("[fake claude] no interactive session {target}");
                return 1;
            };
            let _ = fs::remove_file(&p);
            let sid = d.get("sessionId").and_then(|s| s.as_str()).unwrap_or(target).to_string();
            let labels = read_json(&p.with_extension("labels")).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
            let _ = fs::remove_file(p.with_extension("labels"));
            let reason = if d.get("entrypoint").and_then(|e| e.as_str()) == Some("sdk-cli") { "other" } else { "prompt_input_exit" };
            let h = Hook { event: "SessionEnd", sid: &sid, source: None, reason: Some(reason), cwd: d.get("cwd").and_then(|c| c.as_str()).unwrap_or(""), pid: fake_pid(&format!("i-{}", &sid[..sid.len().min(8)])), session_file: None };
            fire(&base, h, &labels);
            return 0;
        }
        _ => {}
    }

    if !a.bg {
        if a.cont || a.name.is_some() || a.resume.is_some() {
            return interactive(&base, &cwd, &a);
        }
        eprintln!("[fake claude] unsupported: {args:?}");
        return 2;
    }

    if a.session_id.is_some() {
        eprintln!("warning: --bg manages the session id; ignoring --session-id");
    }
    if !trusted(&base, &cwd) {
        eprintln!("Workspace not trusted. Run `claude` in {cwd} once and accept the trust prompt, then retry.");
        return 1;
    }
    let mode = a.mode.clone().unwrap_or_default();

    match &a.resume {
        None => {
            // no --name: the sandbox leaves the name empty (an unnamed agent)
            let name = a.name.clone().unwrap_or_default();
            let short = start(&base, &cwd, &name, &mode, a.settings.as_ref(), "startup", a.settings.is_some());
            println!("{short}");
            0
        }
        Some(target) => {
            let row = match find(&base, target).filter(|r| r.short_id.is_some()) {
                Some(r) => r,
                // no job row, but the transcript is there: a new job named by
                // the short form of that id, which keeps the history (cli-03,
                // check 1); with flags, those flags (BUG-12)
                None if transcript_exists(&base, target) => {
                    let name = a.name.clone().unwrap_or_default();
                    let short = start_as(&base, &cwd, &name, &a.mode.clone().unwrap_or_default(), a.settings.as_ref(), "resume", a.settings.is_some(), Some(target));
                    println!("resumed {target} as {short}");
                    return 0;
                }
                None => {
                    // probe 2a: the session never runs; the row ends `failed`; only SessionEnd fires
                    let (short, _) = next_id(&base);
                    save_job(&base, &short, &json!({"sessionId": target, "resumeSessionId": target, "name": a.name.clone().unwrap_or_default(), "cwd": cwd,
                                "state": "failed", "createdAt": ms(crate::evidence::now()), "respawnFlags": respawn_flags(a.name.as_deref().unwrap_or(""), &mode, a.settings.as_deref())}));
                    let h = Hook { event: "SessionEnd", sid: target, source: None, reason: Some("other"), cwd: &cwd, pid: fake_pid(&short), session_file: None };
                    fire(&base, h, &labels_from_settings(a.settings.as_ref()));
                    println!("backgrounded · {target}");
                    return 0;
                }
            };
            // A prompt is not a flag: `--bg --resume <id> [prompt]` keeps the id (probe 2c1).
            let flags = a.name.is_some() || a.mode.is_some() || a.settings.is_some() || a.fork;
            let saved = row.short_id.as_deref().and_then(|s| job_state(&base, s));
            let saved_name = saved.as_ref().and_then(|j| j.get("name").and_then(|v| v.as_str()).map(str::to_string));

            if row.live() {
                // the job's process is alive (whatever its state): a copy with a
                // new id under an auto-generated name; `--name` is ignored
                // (2.1.292); with no --settings it fires no hooks (probe 2 / 2b)
                let name = copy_name(&base, saved_name.as_deref().unwrap_or(&row.name));
                let short = start(&base, &cwd, &name, &mode, a.settings.as_ref(), "fork", a.settings.is_some());
                record_fork(&base, &short, &row.session_id);
                copy_transcript(&base, &cwd, &row.session_id, &find(&base, &short).map(|r| r.session_id).unwrap_or_default());
                // the copy's row keeps only the flags passed, never a name:
                // `respawnFlags []` for a bare resume (cli-09), `[--permission-mode X]`
                // for cli-11
                if let Some(mut j) = job_state(&base, &short) {
                    j["respawnFlags"] = respawn_flags("", &mode, a.settings.as_deref());
                    save_job(&base, &short, &j);
                }
                if a.name.is_some() {
                    eprintln!("note: --name is ignored for a copy of a running session");
                }
                println!("session {} is already running in the background, so this started a copy as {short}", row.session_id);
                return 0;
            }
            if !flags && knob(&base, "copy_on_resume") {
                // cli-31: the resume raced the daemon's restart of the job: a copy
                // (no labels, `respawnFlags []`) with the note, and the job itself
                // comes back live right after
                let name = saved_name.clone().unwrap_or_else(|| row.name.clone());
                let short = start(&base, &cwd, &name, "", None, "fork", false);
                record_fork(&base, &short, &row.session_id);
                if let Some(mut j) = job_state(&base, &short) {
                    j["respawnFlags"] = json!([]);
                    save_job(&base, &short, &j);
                }
                wake(&base, &row, target, "resume");
                println!("note: session {} is already running in the background, so this started a copy as {short}. `claude attach {}` opens the original.", row.session_id, row.short_id.clone().unwrap_or_default());
                return 0;
            }
            if !flags {
                // 2.1.292 (acceptance F1): a stopped job wakes only by its BIRTH
                // id (`state.json` `sessionId`), and comes back on the
                // conversation it runs now (`resumeSessionId`). Any other id of
                // its conversation starts a NEW job named after that id: no
                // labels, no respawnFlags, the saved name; SessionStart `resume`
                // with no labels.
                let birth = row.birth_id.clone().unwrap_or_else(|| row.session_id.clone());
                if *target != birth {
                    let name = saved_name.clone().unwrap_or_else(|| row.name.clone());
                    let short = start_as(&base, &cwd, &name, "", None, "resume", true, Some(target));
                    if let Some(mut j) = job_state(&base, &short) {
                        j["respawnFlags"] = json!([]);
                        save_job(&base, &short, &j);
                    }
                    println!("backgrounded · {short} (idle — send a prompt to start)");
                    return 0;
                }
                // probe 2c1: the same job, woken with its saved options
                let now = row.session_id.clone();
                wake(&base, &row, &now, "resume");
                println!("woke session {target} with its saved options (--name, --permission-mode, --settings)");
                return 0;
            }
            // probe 2c2 / 2c3: the flags you passed started a copy; the old row stays
            let name = a.name.clone().unwrap_or_else(|| copy_name(&base, saved_name.as_deref().unwrap_or(&row.name)));
            let mode = a.mode.clone().unwrap_or_default();
            let short = start(&base, &cwd, &name, &mode, a.settings.as_ref(), "fork", a.settings.is_some());
            record_fork(&base, &short, &row.session_id);
            // `--fork-session` copies the conversation (cli-13); other flags
            // alone start an empty copy (BUG-53)
            if a.fork {
                copy_transcript(&base, &cwd, &row.session_id, &find(&base, &short).map(|r| r.session_id).unwrap_or_default());
            }
            println!("the flags you passed started a copy as {short}");
            0
        }
    }
}

/// An interactive session as a mux pane (or an SDK host, `-p`) would run it:
/// it stays live (a session file, no job row) until `__end`. `-c` continues the
/// newest transcript in the folder with the same id (`source: resume`, probe
/// P0.8); `--resume <id>` runs that conversation in this process with no job
/// (R-12; refused while a job runs it, R-13); otherwise a new id (`startup`).
/// The hooks see the labels from `--settings`.
fn interactive(base: &Path, cwd: &str, a: &Args) -> i32 {
    let e = env(base);
    let newest = crate::evidence::transcripts_in(&e, cwd).into_iter().next().map(|(_, sid)| sid);
    let t = crate::evidence::now();
    let labels = labels_from_settings(a.settings.as_ref());
    let (sid, source) = match (&a.resume, newest.filter(|_| a.cont)) {
        (Some(target), _) => {
            if let Some(row) = find(base, target).filter(|r| r.live() && r.short_id.is_some()) {
                // refused, and the refused process still fires a SessionEnd with
                // no labels for the member's id (cli-17, R-13)
                let short = row.short_id.clone().unwrap_or_default();
                eprintln!("Error: That session is running in the background ({short}). Run `claude attach {short}` to open it, or `claude stop {short}` first to resume it here.");
                let (stray, _) = next_id(base);
                let h = Hook { event: "SessionEnd", sid: &row.session_id, source: None, reason: Some("other"), cwd, pid: fake_pid(&stray), session_file: None };
                fire(base, h, &Default::default());
                return 1;
            }
            if !transcript_exists(base, target) {
                eprintln!("[fake claude] no conversation {target}");
                return 1;
            }
            // `--fork-session`: a copy of the conversation under a new id (cli-L8)
            if a.fork {
                let (_, new) = next_id(base);
                copy_transcript(base, cwd, target, &new);
                (new, "fork")
            } else {
                (target.clone(), "resume")
            }
        }
        (None, Some(sid)) => (sid, "resume"),
        (None, None) => (next_id(base).1, "startup"),
    };
    let name = a.name.clone().unwrap_or_else(|| "untitled-session".into());
    log_line(base, cwd, &sid, &json!({"type": "user", "cwd": cwd, "sessionId": sid, "uuid": root_uuid(&sid), "parentUuid": null}));
    let p = interactive_path(base, &sid);
    write_json(
        &p,
        &json!({"pid": LIVE_PID, "sessionId": sid, "cwd": cwd, "startedAt": ms(t), "kind": "interactive", "name": name,
                "jobId": null, "status": "idle", "statusUpdatedAt": ms(t), "bridgeSessionId": null,
                "entrypoint": if a.print { "sdk-cli" } else { "cli" }, "version": CC_VERSION}),
    );
    // the labels the process carries, for its SessionEnd (`__end`)
    write_json(&p.with_extension("labels"), &json!(labels));
    let pid = fake_pid(&format!("i-{}", &sid[..sid.len().min(8)]));
    fire(base, Hook { event: "SessionStart", sid: &sid, source: Some(source), reason: None, cwd, pid, session_file: Some(p.clone()) }, &labels);
    println!("[fake claude] interactive session {sid} in {cwd} ({source})");
    0
}

/// Now as the daemon stamps its log: `2026-10-07T01:47:03.070Z`.
fn iso_now() -> String {
    let t = crate::evidence::now();
    let secs = t as i64;
    let ms = ((t - secs as f64) * 1000.0) as i64;
    // civil from days (Howard Hinnant), the reverse of evidence::parse_iso
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{ms:03}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

fn log_call(base: &Path, args: &[String]) {
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(base.join("calls.log")) {
        let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        let _ = writeln!(f, "{}", json!({"at": crate::evidence::now(), "cwd": cwd, "args": args}));
    }
}
