//! A fictional world for testing: fleet configs, a fake ~/.claude, and fake
//! `claude` / `herdr` commands (this same binary, in a hidden mode) that record
//! calls instead of starting or focusing anything.
//!
//! ```text
//! admiralty-watch sandbox create /tmp/aw-sb    # build (or rebuild) it
//! admiralty-watch --sandbox /tmp/aw-sb         # watch it
//! admiralty-watch sandbox tick /tmp/aw-sb      # keep it changing (ctrl-c)
//! ```
//!
//! Live sandbox sessions record pid 1 (always alive), so the real `kill(pid, 0)`
//! liveness check works unchanged; stopped ones simply have no session file.
//! Nothing here reads or writes the real ~/.claude or the real fleets.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::sources::{self, now};

pub const HOST: &str = "sandbox-host";
const LIVE_PID: i64 = 1;

fn write_json(p: &Path, v: &Value) {
    if let Some(d) = p.parent() {
        let _ = fs::create_dir_all(d);
    }
    let _ = fs::write(p, serde_json::to_vec_pretty(v).unwrap());
}

fn set_mtime(p: &Path, t: f64) {
    if let Ok(f) = fs::File::options().write(true).open(p) {
        let _ = f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs_f64(t.max(0.0)));
    }
}

fn iso(t: f64) -> String {
    // UTC `YYYY-MM-DDTHH:MM:SS.000Z` from epoch seconds (civil from days)
    let secs = t as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

struct World {
    base: PathBuf,
    now: f64,
    seq: u64,
    sessions: Vec<(String, String)>, // (name, session id) of every session created
}

#[derive(Default)]
struct S<'a> {
    kind: &'a str, // "background" | "interactive"
    live: bool,
    status: &'a str,
    started_ago: f64,
    state: &'a str,
    job: Option<Value>,
    timeline: Vec<(f64, &'a str, &'a str)>,
}

impl World {
    fn root(&self) -> PathBuf {
        self.base.join("fleets")
    }
    fn home(&self) -> PathBuf {
        self.base.join("claude_home")
    }
    fn wdir(&self, parts: &[&str]) -> String {
        let mut p = self.base.join("dirs");
        for x in parts {
            p = p.join(x);
        }
        let _ = fs::create_dir_all(&p);
        p.to_string_lossy().into_owned()
    }
    fn id(&mut self) -> (String, String) {
        self.seq += 1;
        let short = format!("{:08x}", (self.seq * 0x9e37_79b9) as u32 ^ 0x5a5a_0000);
        (short.clone(), format!("{short}-5b0c-4d1e-9f2a-{:012x}", self.seq))
    }

    fn fleet(&self, tag: &str, agents: &[Value], on_machine: bool) -> String {
        let d = self.root().join(format!("{}-fleet", tag.to_lowercase()));
        let mut machines = serde_json::Map::new();
        if on_machine {
            machines.insert("sandbox".into(), json!({"hostname": HOST, "os": "darwin"}));
        }
        machines.insert("elsewhere".into(), json!({"hostname": "far-away-box", "os": "linux"}));
        let label = if on_machine { "sandbox" } else { "elsewhere" };
        let list: Vec<Value> = agents
            .iter()
            .map(|a| {
                let dirs = match a.get("dir") {
                    Some(Value::String(dir)) => json!({ label: dir }),
                    _ => json!({}),
                };
                json!({"name": a["name"], "prompt": a.get("prompt").cloned().unwrap_or(Value::Null), "dirs": dirs,
                       "permission_mode": a.get("permission_mode").cloned().unwrap_or(json!({})),
                       "bg_isolation": {}, "depends_on": []})
            })
            .collect();
        write_json(&d.join("FLEET_CONFIG.json"), &json!({"fleet": tag, "instructions": "FLEET.md", "machines": machines, "agents": list}));
        d.to_string_lossy().into_owned()
    }

    fn session(&mut self, name: &str, cwd: &str, o: S) -> String {
        let (short, sid) = self.id();
        let kind = if o.kind.is_empty() { "background" } else { o.kind };
        let started = self.now - if o.started_ago == 0.0 { 3600.0 } else { o.started_ago };
        if kind == "background" {
            let jd = self.home().join("jobs").join(&short);
            let mut job = o.job.unwrap_or(json!({"state": if o.state.is_empty() {"blocked"} else {o.state},
                                                 "tokens": 1200, "inFlight": {"tasks": 0, "queued": 0}}));
            job["sessionId"] = json!(sid);
            job["name"] = json!(name);
            job["cwd"] = json!(cwd);
            job["createdAt"] = json!((started * 1000.0) as i64);
            write_json(&jd.join("state.json"), &job);
            if !o.timeline.is_empty() {
                let mut f = fs::File::create(jd.join("timeline.jsonl")).unwrap();
                for (ago, st, det) in &o.timeline {
                    let _ = writeln!(f, "{}", json!({"at": iso(self.now - ago), "state": st, "detail": det, "text": ""}));
                }
            }
        }
        if o.live {
            let mut d = json!({"pid": LIVE_PID, "sessionId": sid, "cwd": cwd, "startedAt": (started * 1000.0) as i64,
                               "kind": if kind == "background" {"bg"} else {"interactive"}, "name": name,
                               "status": if o.status.is_empty() {"idle"} else {o.status},
                               "statusUpdatedAt": ((self.now - o.started_ago.min(600.0)) * 1000.0) as i64});
            if kind == "background" {
                d["jobId"] = json!(short);
            }
            write_json(&self.home().join("sessions").join(format!("{short}.json")), &d);
        }
        self.sessions.push((name.to_string(), sid.clone()));
        sid
    }

    #[allow(clippy::too_many_arguments)]
    fn subagent(&self, sid: &str, cwd: &str, id: &str, label: &str, ago: f64, parent: Option<&str>, teammate: bool, team: Option<&str>) {
        let proj = format!("-{}", cwd.trim_matches('/').replace(['/', '.', '_'], "-"));
        let sd = self.home().join("projects").join(proj).join(sid).join("subagents");
        let mut meta = json!({"agentType": "general-purpose", "description": label, "model": "opus",
                              "spawnDepth": if parent.is_some() {1} else {0},
                              "requestShape": if teammate {"background"} else {"foreground"}});
        if teammate {
            meta["name"] = json!(label);
            meta["taskKind"] = json!("in_process_teammate");
            meta["teamName"] = json!(team);
        }
        if let Some(p) = parent {
            meta["parentAgentId"] = json!(p);
        }
        write_json(&sd.join(format!("agent-{id}.meta.json")), &meta);
        let log = sd.join(format!("agent-{id}.jsonl"));
        let _ = fs::write(&log, format!("{}\n", json!({"agentId": id, "type": "user"})));
        set_mtime(&log, self.now - ago);
    }

    /// A finished interactive session: transcript + custom-title.json, nothing else.
    fn lead_transcript(&mut self, cwd: &str, name: &str, ago: f64) {
        let (_, sid) = self.id();
        let proj = self.home().join("projects").join(sources::project_dir_name(cwd));
        let log = proj.join(format!("{sid}.jsonl"));
        let _ = fs::create_dir_all(&proj);
        let _ = fs::write(&log, format!("{}\n{}\n", json!({"type": "custom-title", "customTitle": name, "sessionId": sid}),
                                         json!({"type": "user", "cwd": cwd, "sessionId": sid})));
        write_json(&proj.join(&sid).join("custom-title.json"), &json!({"customTitle": name}));
        set_mtime(&log, self.now - ago);
    }

    fn task(&self, team: &str, id: i64, subject: &str, status: &str, owner: Option<&str>) {
        write_json(
            &self.home().join("tasks").join(team).join(format!("{id}.json")),
            &json!({"id": id.to_string(), "subject": subject, "status": status, "owner": owner, "blocks": [], "blockedBy": []}),
        );
    }
}

pub fn create(base: &Path, exe: &Path) -> Result<usize, String> {
    if base.exists() {
        if !base.join("world.json").exists() {
            return Err(format!("{} exists and is not a sandbox; refusing to overwrite.", base.display()));
        }
        fs::remove_dir_all(base).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let base = base.canonicalize().map_err(|e| e.to_string())?;
    let mut w = World { base: base.clone(), now: now(), seq: 0, sessions: vec![] };
    let bg = |live: bool, status: &'static str, ago: f64| S { live, status, started_ago: ago, ..Default::default() };

    // ALPHA: everything going on at once
    let scout = w.wdir(&["alpha", "scout"]);
    let forge = w.wdir(&["alpha", "forge"]);
    let archive = w.wdir(&["alpha", "archive"]);
    let beacon = w.wdir(&["alpha", "beacon"]);
    let noprompt = w.wdir(&["alpha", "noprompt"]);
    let alpha = w.fleet(
        "ALPHA",
        &[
            json!({"name": "SCOUT", "dir": scout, "prompt": "You are [ALPHA]-SCOUT."}),
            json!({"name": "FORGE", "dir": forge, "prompt": "You are [ALPHA]-FORGE."}),
            json!({"name": "ARCHIVE", "dir": archive, "prompt": "You are [ALPHA]-ARCHIVE."}),
            json!({"name": "BEACON", "dir": beacon, "prompt": "You are [ALPHA]-BEACON, the signal agent.",
                   "permission_mode": {"sandbox": "auto"}}),
            json!({"name": "NOPROMPT", "dir": noprompt}),
        ],
        true,
    );
    let lead = w.session("[ALPHA]-LEAD", &alpha, S { kind: "interactive", live: true, status: "busy", started_ago: 7200.0, ..Default::default() });
    let team = format!("session-{}", &lead[..8]);
    w.subagent(&lead, &alpha, "aplanner-11", "planner", 5.0, None, true, Some(&team));
    w.subagent(&lead, &alpha, "a77aa01", "Survey north ridge", 2.0, Some("aplanner-11"), false, None);
    w.subagent(&lead, &alpha, "a77aa02", "Survey south ridge", 40.0, Some("aplanner-11"), false, None);
    w.subagent(&lead, &alpha, "a77aa03", "Old survey pass", 5000.0, Some("aplanner-11"), false, None);
    w.subagent(&lead, &alpha, "areviewer-22", "reviewer", 3000.0, None, true, Some(&team));
    for i in 0..6 {
        w.subagent(&lead, &alpha, &format!("adone{i:02}"), &format!("Finished errand {i}"), 4000.0 + i as f64 * 300.0, None, false, None);
    }
    w.task(&team, 1, "Map the ridge line", "completed", Some("planner"));
    w.task(&team, 2, "Write the survey report", "in_progress", Some("planner"));
    w.task(&team, 3, "Review the survey report", "pending", Some("reviewer"));

    let s = w.session("[ALPHA]-SCOUT", &scout, S {
        job: Some(json!({"state": "working", "detail": "mapping sector 7", "tokens": 48211, "inFlight": {"tasks": 2, "queued": 1}})),
        timeline: vec![(60.0, "working", "mapping sector 7"), (900.0, "working", "mapping sector 6")],
        ..bg(true, "busy", 1800.0)
    });
    w.subagent(&s, &scout, "ab1", "Fetch elevation tiles", 3.0, None, false, None);
    w.subagent(&s, &scout, "ab2", "Stitch tiles", 20.0, Some("ab1"), false, None);
    w.session("[ALPHA]-FORGE", &forge, S {
        job: Some(json!({"state": "blocked", "detail": "build needs a decision",
                         "needs": "Ship forge 2.0 with the new alloy table, or hold for the regression run?",
                         "tokens": 15000, "inFlight": {"tasks": 0, "queued": 0}})),
        timeline: vec![(300.0, "blocked", "build needs a decision"), (1200.0, "working", "building forge 2.0")],
        ..bg(true, "idle", 5400.0)
    });
    w.session("[ALPHA]-ARCHIVE", &archive, S { state: "done", ..bg(false, "", 86400.0 * 9.0) });
    w.session("[ALPHA]-ARCHIVE", &archive, S { state: "failed", ..bg(false, "", 86400.0 * 5.0) });
    w.session("[ALPHA]-ARCHIVE", &archive, S {
        job: Some(json!({"state": "blocked", "detail": "index rebuild paused",
                         "needs": "Delete the 2019 snapshots before the rebuild? (yes/no)", "tokens": 9000,
                         "inFlight": {"tasks": 0, "queued": 0}})),
        timeline: vec![(86400.0 * 2.0 - 600.0, "blocked", "index rebuild paused")],
        ..bg(false, "", 86400.0 * 2.0)
    });
    let rogue = w.wdir(&["elsewhere", "rogue"]);
    w.session("[ALPHA]-ROGUE", &rogue, bg(true, "busy", 600.0));

    // BRAVO: lead with a name variant, a duplicate-live agent, a missing dir
    let twin = w.wdir(&["bravo", "twin"]);
    let ghost = base.join("dirs/bravo/does-not-exist").to_string_lossy().into_owned();
    let bravo = w.fleet(
        "BRAVO",
        &[json!({"name": "TWIN", "dir": twin, "prompt": "You are [BRAVO]-TWIN."}),
          json!({"name": "GHOST", "dir": ghost, "prompt": "You are GHOST."})],
        true,
    );
    w.session("[BRAVO]-LEAD-quiet-otter", &bravo, S { kind: "interactive", live: true, status: "idle", started_ago: 86400.0, ..Default::default() });
    w.session("[BRAVO]-TWIN", &twin, bg(true, "idle", 4000.0));
    w.session("[BRAVO]-TWIN", &twin, bg(true, "busy", 300.0));

    // CHARLIE: not deployed on this machine
    w.fleet("CHARLIE", &[json!({"name": "FAR", "dir": "/remote/far"})], false);

    // DELTA: no lead; an off-config agent; an untagged session in the fleet dir
    let drift_cfg = w.wdir(&["delta", "drift"]);
    let drift_real = w.wdir(&["delta", "drift-somewhere-else"]);
    let delta = w.fleet("DELTA", &[json!({"name": "DRIFT", "dir": drift_cfg, "prompt": "You are [DELTA]-DRIFT."})], true);
    w.session("[DELTA]-DRIFT", &drift_real, bg(true, "idle", 7000.0));
    w.session("quick look at delta logs", &delta, S { state: "done", ..bg(false, "", 86400.0 * 3.0) });
    // DELTA's lead ran once and ended: it left only a transcript (no session file, no job)
    w.lead_transcript(&delta, "[DELTA]-LEAD", 86400.0 * 4.0);

    // ECHO: unreadable config
    let echo = w.root().join("echo-fleet");
    let _ = fs::create_dir_all(&echo);
    let _ = fs::write(echo.join("FLEET_CONFIG.json"), "{ not json");

    // the hub and sessions that belong to no fleet
    let adm = base.join("admiralty");
    let _ = fs::create_dir_all(&adm);
    w.session("[ADMIRALTY]", &adm.to_string_lossy(), S { kind: "interactive", live: true, status: "busy", started_ago: 3000.0, ..Default::default() });
    let scratch = w.wdir(&["scratch"]);
    w.session("scratch notes", &scratch, S { kind: "interactive", live: true, status: "idle", started_ago: 900.0, ..Default::default() });
    let zulu = w.wdir(&["zulu"]);
    w.session("[ZULU]-UNKNOWN", &zulu, S { state: "done", ..bg(false, "", 86400.0) });

    // Herdr panes: the hub, both leads, and FORGE attached in a pane of its own
    let sid_of = |n: &str| w.sessions.iter().rev().find(|(name, _)| name == n).map(|(_, s)| s.clone()).unwrap();
    let mut panes = vec![];
    for (i, (name, ws, cwd, status)) in [
        ("[ADMIRALTY]", "w1", adm.to_string_lossy().into_owned(), "working"),
        ("[ALPHA]-LEAD", "w2", alpha.clone(), "idle"),
        ("[ALPHA]-FORGE", "w2", forge.clone(), "done"),
        ("[BRAVO]-LEAD-quiet-otter", "w3", bravo.clone(), "working"),
    ]
    .iter()
    .enumerate()
    {
        panes.push(json!({"agent": "claude", "agent_session": {"agent": "claude", "kind": "id", "value": sid_of(name)},
                          "agent_status": status, "cwd": cwd, "foreground_cwd": cwd,
                          "pane_id": format!("{ws}:p{}", i + 1), "tab_id": format!("{ws}:t1"), "workspace_id": ws,
                          "terminal_title_stripped": name}));
    }
    write_json(&base.join("world.json"), &json!({"herdr_panes": panes, "next": 0}));

    let bin = base.join("bin");
    fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    for (tool, mode) in [("claude", "__fake-claude"), ("herdr", "__fake-herdr")] {
        let p = bin.join(tool);
        let q = |x: &Path| crate::actions::shell_quote(&x.to_string_lossy());
        fs::write(&p, format!("#!/bin/sh\nexec {} {mode} {} \"$@\"\n", q(exe), q(&base))).map_err(|e| e.to_string())?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    Ok(w.sessions.len())
}

pub fn env_for(base: &Path) -> HashMap<String, String> {
    let b = base.canonicalize().unwrap_or(base.to_path_buf());
    let p = |x: &str| b.join(x).to_string_lossy().into_owned();
    HashMap::from([
        ("ADMIRALTY_WATCH_ROOT".into(), p("fleets")),
        ("ADMIRALTY_WATCH_CLAUDE_HOME".into(), p("claude_home")),
        ("ADMIRALTY_WATCH_CLAUDE_BIN".into(), p("bin/claude")),
        ("ADMIRALTY_WATCH_NATIVE_CLAUDE_BIN".into(), p("bin/claude")),
        ("ADMIRALTY_WATCH_HOSTNAME".into(), HOST.into()),
        ("ADMIRALTY_WATCH_ACTION_LOG".into(), p("actions.log")),
        ("ADMIRALTY_WATCH_HERDR_BIN".into(), p("bin/herdr")),
        ("ADMIRALTY_WATCH_STATE".into(), p("ui-state.json")),
        // the fake herdr stands in, so the sandbox always exercises it
        ("HERDR_ENV".into(), "1".into()),
    ])
}

fn log_call(base: &Path, file: &str, args: &[String]) {
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(base.join(file)) {
        let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        let _ = writeln!(f, "{}", json!({"at": now(), "cwd": cwd, "args": args}));
    }
}

/// `claude` stand-in. Serves `agents --json` from the sandbox files and turns
/// `--bg` into a new live job row. Never starts a real session.
pub fn fake_claude(base: &Path, args: &[String]) -> i32 {
    log_call(base, "calls.log", args);
    let home = base.join("claude_home");
    if args.first().map(|a| a == "agents").unwrap_or(false) && args.iter().any(|a| a == "--json") {
        if std::env::var("FAKE_CLAUDE_FAIL").is_ok() {
            eprintln!("fake failure");
            return 1;
        }
        let env = crate::env::Env { claude_home: home, ..crate::env::Env::from_vars(&env_for(base)) };
        let rows = sources::rows_from_files(&env, &mut sources::Cache::default()).unwrap_or_default();
        let out: Vec<Value> = rows
            .iter()
            .map(|r| {
                let mut v = json!({"cwd": r.cwd, "kind": r.kind, "sessionId": r.session_id, "name": r.name,
                                   "startedAt": r.started_at.map(|t| (t * 1000.0) as i64)});
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
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }
    if args.first().map(|a| a == "rm").unwrap_or(false) {
        let Some(id) = args.get(1) else { return 2 };
        let jd = home.join("jobs").join(id);
        if !jd.is_dir() {
            eprintln!("[fake claude] no job {id}");
            return 1;
        }
        let _ = fs::remove_dir_all(&jd);
        for e in fs::read_dir(home.join("sessions")).into_iter().flatten().filter_map(|e| e.ok()) {
            if fs::read_to_string(e.path()).map(|t| t.contains(&format!("\"jobId\": \"{id}\""))).unwrap_or(false) {
                let _ = fs::remove_file(e.path());
            }
        }
        println!("removed {id}");
        return 0;
    }
    if args.first().map(|a| a == "attach").unwrap_or(false) {
        println!("[fake claude] attached to {} (sandbox: nothing is running).", args.get(1).cloned().unwrap_or_default());
        if std::env::var("FAKE_CLAUDE_NONINTERACTIVE").is_err() {
            print!("press Enter to detach ");
            let _ = std::io::stdout().flush();
            let mut s = String::new();
            let _ = std::io::stdin().read_line(&mut s);
        }
        return 0;
    }
    if args.iter().any(|a| a == "--bg") {
        let name = args.iter().position(|a| a == "--name").and_then(|i| args.get(i + 1)).cloned().unwrap_or("unnamed".into());
        let resumed = args.iter().any(|a| a == "--resume");
        let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        let mut wj: Value = fs::read(base.join("world.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
        let n = wj["next"].as_u64().unwrap_or(0) + 1;
        wj["next"] = json!(n);
        write_json(&base.join("world.json"), &wj);
        let short = format!("f{:07x}", (now() as u64 ^ (n * 7919)) & 0xfff_ffff);
        let sid = format!("{short}-0000-4000-8000-{n:012x}");
        let t = (now() * 1000.0) as i64;
        let detail = if resumed { "resumed, waiting for a message" } else { "cold launch, waiting for a message" };
        write_json(&home.join("jobs").join(&short).join("state.json"),
                   &json!({"state": "blocked", "detail": detail, "tokens": 0, "inFlight": {"tasks": 0, "queued": 0},
                           "sessionId": sid, "name": name, "cwd": cwd, "createdAt": t}));
        write_json(&home.join("sessions").join(format!("{short}.json")),
                   &json!({"pid": LIVE_PID, "sessionId": sid, "cwd": cwd, "startedAt": t, "kind": "bg", "name": name,
                           "jobId": short, "status": "idle", "statusUpdatedAt": t}));
        println!("{short}");
        return 0;
    }
    eprintln!("[fake claude] unsupported: {args:?}");
    2
}

/// `herdr` stand-in: serves `agent list` from world.json and records focus calls.
pub fn fake_herdr(base: &Path, args: &[String]) -> i32 {
    log_call(base, "herdr_calls.log", args);
    let wj: Value = fs::read(base.join("world.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
    let panes = wj["herdr_panes"].as_array().cloned().unwrap_or_default();
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match a.as_slice() {
        ["agent", "list"] => {
            println!("{}", json!({"id": "cli:agent:list", "result": {"agents": panes}}));
            0
        }
        ["agent", "prompt", target, _text, ..] => {
            if panes.iter().any(|p| p["pane_id"].as_str() == Some(target)) {
                println!("{}", json!({"id": "cli:agent:prompt", "result": {"submitted": target}}));
                0
            } else {
                eprintln!("{}", json!({"error": {"code": "not_found", "message": format!("unknown target {target}")}}));
                1
            }
        }
        [kind @ ("workspace" | "tab" | "agent"), "focus", target] => {
            let field = match *kind { "workspace" => "workspace_id", "tab" => "tab_id", _ => "pane_id" };
            if panes.iter().any(|p| p[field].as_str() == Some(target)) {
                println!("{}", json!({"id": "cli:focus", "result": {"focused": target}}));
                0
            } else {
                eprintln!("{}", json!({"error": {"code": "not_found", "message": format!("unknown target {target}")}}));
                1
            }
        }
        _ => {
            eprintln!("[fake herdr] unsupported: {args:?}");
            2
        }
    }
}

/// Keep the sandbox moving: touch running sub-agents and flip SCOUT busy/idle.
pub fn tick(base: &Path) -> ! {
    println!("[OK] ticking {} every 2s, ctrl-c to stop", base.display());
    let home = base.join("claude_home");
    let mut n: u64 = 0;
    loop {
        n += 1;
        let t = now();
        if let Ok(projects) = fs::read_dir(home.join("projects")) {
            for proj in projects.filter_map(|e| e.ok()) {
                for sess in fs::read_dir(proj.path()).into_iter().flatten().filter_map(|e| e.ok()) {
                    for id in ["a77aa01", "ab1"] {
                        let log = sess.path().join("subagents").join(format!("agent-{id}.jsonl"));
                        if log.exists() {
                            set_mtime(&log, t);
                        }
                    }
                }
            }
        }
        for e in fs::read_dir(home.join("sessions")).into_iter().flatten().filter_map(|e| e.ok()) {
            let p = e.path();
            let Some(mut d) = fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { continue };
            if d["name"] == "[ALPHA]-SCOUT" {
                d["status"] = json!(if n % 4 < 2 { "busy" } else { "idle" });
                d["statusUpdatedAt"] = json!((t * 1000.0) as i64);
                write_json(&p, &d);
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
