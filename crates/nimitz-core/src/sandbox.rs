//! Sandbox world (fake NIMITZ_HOME, fake ~/.claude, fake `claude`) for tests and demos.
//! The fake `claude` is `nimitz __fake-claude <sandbox> ...` (contracts.md section 10). Owner: core-sessions.
//!
//! The world covers every row of the relaunch state table (contracts.md section
//! 8) and every issue code of section 7, so one test per row can drive the real
//! planner and `apply` end to end. Nothing here touches the real `~/.claude`,
//! the real `NIMITZ_HOME` or any real session.
//!
//! The fake `claude` reproduces what the P0 probes measured on both builds
//! (`docs/probes/README.md`), including the awkward parts:
//!
//! - `--bg` ignores `--session-id` and says so on stderr (probe 1);
//! - `--resume <unknown id>`: the session never runs, the row ends `failed`, and
//!   only SessionEnd fires (probe 2a);
//! - `--resume` of a **live** session makes a copy with a new id (probe 2, 2b);
//! - `--resume` of a stopped session with **no flags** keeps the id and the
//!   saved `--name`, `--permission-mode` and `--settings` (probe 2c1);
//! - any flag on a resume makes a copy that carries only the flags passed, so a
//!   `--name`-only resume has no labels and fires no hooks (probe 2c2, 2c3);
//! - `--bg` refuses an untrusted folder (probe 0);
//! - two live background sessions may share one name (probe 5).
//!
//! Live sandbox sessions record pid 1 (always alive), so the real `kill(pid, 0)`
//! liveness check works unchanged; a stopped session simply has no session file.
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
    // sessions ran here before: Claude Code accepted this folder already
    let proj = claude_home(base).join("projects");
    let ran_here = [cwd.to_string(), real.to_string_lossy().into_owned()]
        .iter()
        .any(|c| fs::read_dir(proj.join(crate::evidence::project_dir_name(c))).map(|mut d| d.next().is_some()).unwrap_or(false));
    // inside a git repo, a parent's trust is not enough, unless sessions ran
    // here before (Claude Code accepted the repo then; core-config's ran_here)
    let floor = crate::env::git_root(&real).filter(|_| !ran_here);
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
        let labels = o.labels_of.or(o.labels).map(|id| crate::claude::labels_for(&env(&self.base), id));
        let home = claude_home(&self.base);
        if kind == "background" {
            let mut row = json!({
                "sessionId": sid, "name": name, "cwd": cwd,
                "state": if o.state.is_empty() { if o.live { "running" } else { "done" } } else { o.state },
                "createdAt": ms(started), "tokens": 1200, "inFlight": {"tasks": 0, "queued": 0},
                // what a flag-free resume restores (probe 2c1)
                "savedName": name, "savedMode": "bypassPermissions",
            });
            if let Some(l) = &labels {
                row["savedSettings"] = json!(json!({"env": l, "worktree": {"bgIsolation": "none"}}).to_string());
            }
            if let Some(n) = o.needs {
                row["needs"] = json!(n);
            }
            write_json(&home.join("jobs").join(&short).join("state.json"), &row);
        }
        if o.live {
            let mut d = json!({
                "pid": LIVE_PID, "sessionId": sid, "cwd": cwd, "startedAt": ms(started),
                "kind": if kind == "background" { "bg" } else { "interactive" }, "name": name,
                "status": if o.status.is_empty() { "idle" } else { o.status },
                "statusUpdatedAt": ms(self.now - 60.0),
            });
            if o.status == "waiting" {
                d["waitingFor"] = json!("permission prompt");
            }
            if kind == "background" {
                d["jobId"] = json!(short);
            }
            write_json(&home.join("sessions").join(format!("{short}.json")), &d);
        }
        if let Some(l) = labels {
            self.event("SessionStart", &sid, Some("startup"), &l, started);
        }
        // every session that ran has a transcript in its folder's project dir
        let proj = home.join("projects").join(crate::evidence::project_dir_name(cwd));
        let _ = fs::create_dir_all(&proj);
        let _ = fs::write(proj.join(format!("{sid}.jsonl")), format!("{}\n", json!({"type": "user", "cwd": cwd, "sessionId": sid})));
        self.sessions.push((name, sid.clone(), short.clone()));
        (short, sid)
    }

    fn event(&self, event: &str, sid: &str, source: Option<&str>, labels: &std::collections::BTreeMap<String, String>, ts: f64) {
        append_event(&self.base, &json!({
            "ts": ts, "event": event, "session_id": sid, "source": source,
            "pid": LIVE_PID, "labels": labels,
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
            format!("{}\n{}\n", json!({"type": "custom-title", "customTitle": name, "sessionId": sid}), json!({"type": "user", "cwd": cwd, "sessionId": sid})),
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

    fn bind(&self, id: &Id, cwd: &str, sid: &str, via: &str) {
        let e = env(&self.base);
        let _ = crate::registry::update(&e, |r| crate::registry::bind(r, id, id.role(), &id.name(), cwd, sid, via, self.now));
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
    ("TAKEN", "its name is held by another live session (name.taken)"),
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
    apply_cfg(&e, &crate::config::plan_squadron_new(&e, "MAIN", "ARGOS", "Everything the relaunch table needs.", Comms::default()))?;
    apply_cfg(&e, &crate::config::plan_squadron_new(&e, "MAIN", "BOREAL", "A squadron whose commodore went to the background.", Comms::Mode("commodore-only".into())))?;

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

    // BETA: an admiral that ended and left only its transcript (row 6).
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

    // The admiral: interactive, live, clean (so a ship's start needs no parent work).
    let adm = Id::admiral("MAIN");
    let (_, adm_sid) = w.session(&adm.name(), &admiral_dir, S { kind: "interactive", live: true, status: "busy", labels: Some(&adm), ..Default::default() });
    w.bind(&adm, &admiral_dir, &adm_sid, "launch");
    let team = format!("session-{}", &adm_sid[..8]);
    w.subagent(&adm_sid, &admiral_dir, "planner-1", "planner", 5.0, true, &team);
    w.subagent(&adm_sid, &admiral_dir, "worker-1", "Draft the order", 4000.0, false, &team);
    w.task(&team, 1, "Plan the sortie", "in_progress");

    // ARGOS's commodore: interactive, live, clean.
    let argos = Id::commodore("MAIN", "ARGOS");
    let argos_dir_s = argos_dir.to_string_lossy().into_owned();
    let (_, argos_sid) = w.session(&argos.name(), &argos_dir_s, S { kind: "interactive", live: true, labels: Some(&argos), ..Default::default() });
    w.bind(&argos, &argos_dir_s, &argos_sid, "launch");

    // BOREAL's commodore: a background session (role.background).
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
                    let (_, c) = w.session(&id.name(), &cwd, bg(false, Some(&id)));
                    // a copy is in the lineage but is not current
                    let ev = env(&base);
                    let _ = crate::registry::update(&ev, |r| {
                        if let Some(rec) = r.identities.get_mut(&id.0) {
                            rec.lineage.push(LineageEntry { session_id: c.clone(), bound_at: w.now, via: "relaunch".into() });
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
                let other = Id::ship("MAIN", "ARGOS", "CLEAN");
                let (_, sid) = w.session(&id.name(), &cwd, S { labels_of: Some(&other), ..bg(true, Some(&id)) });
                w.bind(&id, &cwd, &sid, "launch");
            }
            _ => {}
        }
    }

    // A tender: stopped with its labels (resume keeps its id).
    let tender = Id::tender("MAIN", "ARGOS", "fixci");
    let (_, t_sid) = w.session(&tender.name(), &argos_dir_s, bg(false, Some(&tender)));
    w.bind(&tender, &argos_dir_s, &t_sid, "launch");

    // BETA's admiral: a transcript only (row 6).
    let _ = fs::create_dir_all(&beta_admiral);
    w.transcript(&beta_admiral, &Id::admiral("BETA").name(), 86400.0);

    // A stowaway: an unlabelled session in the fleet's own folder.
    w.session("quick look at the logs", &argos_dir_s, bg(true, None));

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
    name: Option<String>,
    mode: Option<String>,
    settings: Option<String>,
    resume: Option<String>,
    session_id: Option<String>,
    cont: bool,
    prompt: Option<String>,
}

fn parse(args: &[String]) -> Args {
    let mut a = Args { bg: false, name: None, mode: None, settings: None, resume: None, session_id: None, cont: false, prompt: None };
    let mut i = 0;
    while i < args.len() {
        let take = |i: &mut usize| -> Option<String> {
            *i += 1;
            args.get(*i).cloned()
        };
        match args[i].as_str() {
            "--bg" => a.bg = true,
            "-c" | "--continue" => a.cont = true,
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

/// Run the configured hook, or `crate::hook::handle` in process.
fn fire(base: &Path, event: &str, sid: &str, source: Option<&str>, labels: &std::collections::BTreeMap<String, String>, cwd: &str) {
    let mut input = json!({"session_id": sid, "hook_event_name": event, "cwd": cwd, "transcript_path": "", "scratchpad_dir": ""});
    if let Some(s) = source {
        input["source"] = json!(s);
    }
    if event == "SessionEnd" {
        input["reason"] = json!("other"); // what a `claude stop` gave in probe 3
    }
    let vars: HashMap<String, String> = env_for(base).into_iter().chain(labels.iter().map(|(k, v)| (k.clone(), v.clone()))).collect();
    if let Some(cmd) = read_json(&base.join("hook-cmd.json")).and_then(|v| v.as_array().cloned()) {
        let argv: Vec<String> = cmd.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        if let Some(bin) = argv.first() {
            let mut c = std::process::Command::new(bin);
            c.args(&argv[1..]).arg(event).envs(&vars).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null());
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
    let _ = crate::hook::handle(&e, event, &input.to_string(), &vars);
}

fn rows(base: &Path) -> Vec<crate::evidence::SessionRow> {
    let e = env(base);
    crate::evidence::rows_from_files(&e, &mut crate::evidence::FileCache::default()).unwrap_or_default()
}

fn find(base: &Path, needle: &str) -> Option<crate::evidence::SessionRow> {
    rows(base).into_iter().find(|r| r.session_id == needle || r.short_id.as_deref() == Some(needle))
}

fn job_state(base: &Path, short: &str) -> Option<Value> {
    read_json(&claude_home(base).join("jobs").join(short).join("state.json"))
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

/// A copy's job row records the session it was forked from.
fn record_fork(base: &Path, short: &str, from: &str) {
    let p = claude_home(base).join("jobs").join(short).join("state.json");
    if let Some(mut j) = read_json(&p) {
        j["forkedFrom"] = json!(from);
        write_json(&p, &j);
    }
}

fn transcript_exists(base: &Path, sid: &str) -> bool {
    fs::read_dir(claude_home(base).join("projects"))
        .map(|rd| rd.filter_map(|e| e.ok()).any(|e| e.path().join(format!("{sid}.jsonl")).is_file()))
        .unwrap_or(false)
}

/// Make a live background session (a cold launch or a copy) and fire its hooks.
fn start(base: &Path, cwd: &str, name: &str, mode: &str, settings: Option<&String>, source: &str, labels_fire: bool) -> String {
    start_as(base, cwd, name, mode, settings, source, labels_fire, None)
}

/// `start`, optionally keeping a given session id (a resume from a transcript).
#[allow(clippy::too_many_arguments)]
fn start_as(base: &Path, cwd: &str, name: &str, mode: &str, settings: Option<&String>, source: &str, labels_fire: bool, keep: Option<&str>) -> String {
    let (short, sid) = match keep {
        Some(k) => (k[..k.len().min(8)].to_string(), k.to_string()),
        None => next_id(base),
    };
    let t = crate::evidence::now();
    let labels = labels_from_settings(settings);
    let mut row = json!({"sessionId": sid, "name": name, "cwd": cwd, "state": "running", "createdAt": ms(t),
                         "tokens": 0, "inFlight": {"tasks": 0, "queued": 0}, "savedName": name, "savedMode": mode});
    if let Some(s) = settings {
        row["savedSettings"] = json!(s);
    }
    write_json(&claude_home(base).join("jobs").join(&short).join("state.json"), &row);
    write_json(
        &claude_home(base).join("sessions").join(format!("{short}.json")),
        &json!({"pid": LIVE_PID, "sessionId": sid, "cwd": cwd, "startedAt": ms(t), "kind": "bg", "name": name,
                "jobId": short, "status": "idle", "statusUpdatedAt": ms(t)}),
    );
    // The conversation: a new id starts an empty transcript (a copy has NO
    // history, BUG-53); a kept id appends to the one it has, wherever it is.
    let existing = fs::read_dir(claude_home(base).join("projects"))
        .ok()
        .and_then(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path().join(format!("{sid}.jsonl"))).find(|p| p.is_file()));
    let log = existing.unwrap_or_else(|| claude_home(base).join("projects").join(crate::evidence::project_dir_name(cwd)).join(format!("{sid}.jsonl")));
    if let Some(d) = log.parent() {
        let _ = fs::create_dir_all(d);
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&log) {
        let _ = writeln!(f, "{}", json!({"type": "system", "sessionId": sid, "cwd": cwd, "source": source}));
    }
    // A copy with no --settings carries no hooks at all, so it fires none (probe 2b, 2c2).
    if labels_fire {
        fire(base, "SessionStart", &sid, Some(source), &labels, cwd);
    }
    short
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
                    let mut v = json!({"cwd": r.cwd, "kind": r.kind, "sessionId": r.session_id, "name": r.name,
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
        Some("stop") => {
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
            let _ = fs::remove_file(home.join("sessions").join(format!("{short}.json")));
            if let Some(mut j) = job_state(&base, &short) {
                j["state"] = json!("done");
                write_json(&home.join("jobs").join(&short).join("state.json"), &j);
                let labels = labels_from_settings(j.get("savedSettings").and_then(|s| s.as_str()).map(str::to_string).as_ref());
                fire(&base, "SessionEnd", &row.session_id, None, &labels, &row.cwd);
            }
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
            println!("removed {short}");
            return 0;
        }
        Some("attach") => {
            println!("[fake claude] attached to {} (sandbox: nothing is running)", args.get(1).cloned().unwrap_or_default());
            return 0;
        }
        _ => {}
    }

    if !a.bg {
        if a.cont || a.name.is_some() {
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
            let row = match find(&base, target) {
                Some(r) => r,
                // no job row, but the transcript is there: real Claude Code
                // continues that same id (with the flags passed; BUG-12)
                None if transcript_exists(&base, target) => {
                    let name = a.name.clone().unwrap_or_default();
                    let short = start_as(&base, &cwd, &name, &a.mode.clone().unwrap_or_default(), a.settings.as_ref(), "resume", a.settings.is_some(), Some(target));
                    println!("resumed {target} as {short}");
                    return 0;
                }
                None => {
                    // probe 2a: the session never runs; the row ends `failed`; only SessionEnd fires
                    let (short, _) = next_id(&base);
                    write_json(
                        &home.join("jobs").join(&short).join("state.json"),
                        &json!({"sessionId": target, "name": a.name.clone().unwrap_or_default(), "cwd": cwd,
                                "state": "failed", "createdAt": ms(crate::evidence::now())}),
                    );
                    fire(&base, "SessionEnd", target, None, &labels_from_settings(a.settings.as_ref()), &cwd);
                    println!("backgrounded · {target}");
                    return 0;
                }
            };
            // A prompt is not a flag: `--bg --resume <id> [prompt]` keeps the id (probe 2c1).
            let flags = a.name.is_some() || a.mode.is_some() || a.settings.is_some();
            let saved = row.short_id.as_deref().and_then(|s| job_state(&base, s));
            let saved_name = saved.as_ref().and_then(|j| j.get("savedName").and_then(|v| v.as_str()).map(str::to_string));
            let saved_settings = saved.as_ref().and_then(|j| j.get("savedSettings").and_then(|v| v.as_str()).map(str::to_string));

            if row.live() {
                // probe 2 / 2b: a copy with a new id and only the flags passed (an
                // auto-generated name without --name); with no --settings it fires no hooks
                let name = a.name.clone().unwrap_or_else(|| copy_name(&base, saved_name.as_deref().unwrap_or(&row.name)));
                let short = start(&base, &cwd, &name, &mode, a.settings.as_ref(), "fork", a.settings.is_some());
                record_fork(&base, &short, &row.session_id);
                println!("session {} is already running in the background, so this started a copy as {short}", row.session_id);
                return 0;
            }
            if !flags {
                // probe 2c1: the same id, with its saved options and labels
                let name = saved_name.unwrap_or_else(|| row.name.clone());
                let short = row.short_id.clone().unwrap_or_default();
                let t = crate::evidence::now();
                if let Some(mut j) = saved.clone() {
                    j["state"] = json!("running");
                    write_json(&home.join("jobs").join(&short).join("state.json"), &j);
                }
                write_json(
                    &home.join("sessions").join(format!("{short}.json")),
                    &json!({"pid": LIVE_PID, "sessionId": row.session_id, "cwd": row.cwd, "startedAt": ms(t), "kind": "bg",
                            "name": name, "jobId": short, "status": "idle", "statusUpdatedAt": ms(t)}),
                );
                let labels = labels_from_settings(saved_settings.as_ref());
                if !labels.is_empty() {
                    fire(&base, "SessionStart", &row.session_id, Some("resume"), &labels, &row.cwd);
                }
                println!("woke session {} with its saved options (--name, --permission-mode, --settings)", row.session_id);
                return 0;
            }
            // probe 2c2 / 2c3: the flags you passed started a copy; the old row stays
            let name = a.name.clone().unwrap_or_else(|| copy_name(&base, saved_name.as_deref().unwrap_or(&row.name)));
            let mode = a.mode.clone().unwrap_or_default();
            let short = start(&base, &cwd, &name, &mode, a.settings.as_ref(), "fork", a.settings.is_some());
            record_fork(&base, &short, &row.session_id);
            println!("the flags you passed started a copy as {short}");
            0
        }
    }
}

/// An interactive session as a mux pane would run it: it stays live (a session
/// file, no job row) until the test ends. `-c` continues the newest transcript
/// in the folder with the same id (`source: resume`, probe P0.8); otherwise a
/// new id (`startup`). The hooks see the labels from `--settings`.
fn interactive(base: &Path, cwd: &str, a: &Args) -> i32 {
    let e = env(base);
    let newest = crate::evidence::transcripts_in(&e, cwd).into_iter().next().map(|(_, sid)| sid);
    let (sid, source) = match newest.filter(|_| a.cont) {
        Some(sid) => (sid, "resume"),
        None => (next_id(base).1, "startup"),
    };
    let t = crate::evidence::now();
    let name = a.name.clone().unwrap_or_else(|| "untitled-session".into());
    let proj = claude_home(base).join("projects").join(crate::evidence::project_dir_name(cwd));
    let _ = fs::create_dir_all(&proj);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(proj.join(format!("{sid}.jsonl"))) {
        let _ = writeln!(f, "{}", json!({"type": "user", "cwd": cwd, "sessionId": sid}));
    }
    write_json(
        &claude_home(base).join("sessions").join(format!("i-{}.json", &sid[..8])),
        &json!({"pid": LIVE_PID, "sessionId": sid, "cwd": cwd, "startedAt": ms(t), "kind": "interactive", "name": name,
                "status": "idle", "statusUpdatedAt": ms(t)}),
    );
    let labels = labels_from_settings(a.settings.as_ref());
    fire(base, "SessionStart", &sid, Some(source), &labels, cwd);
    println!("[fake claude] interactive session {sid} in {cwd} ({source})");
    0
}

fn log_call(base: &Path, args: &[String]) {
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(base.join("calls.log")) {
        let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        let _ = writeln!(f, "{}", json!({"at": crate::evidence::now(), "cwd": cwd, "args": args}));
    }
}
