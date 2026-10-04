//! Readers for every place fleet state lives.
//!
//! Session rows come straight from Claude Code's own files (`sessions/<pid>.json`
//! for live processes, `jobs/<id>/state.json` for background job rows), which is
//! what `claude agents --json --all` reads; liveness is `kill(pid, 0)`. That keeps
//! a refresh to a few milliseconds of CPU instead of starting the Node CLI every
//! 2 s. The CLI is still used when those files are missing or when
//! ADMIRALTY_WATCH_SOURCE=cli.
//!
//! Everything under ~/.claude is internal to Claude Code and may change in any
//! release, so every reader tolerates missing files and fields.

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::env::Env;

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

pub fn mtime_secs(p: &Path) -> Option<f64> {
    mtime(p).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs_f64())
}

fn s(v: &Value, k: &str) -> Option<String> {
    match v.get(k) {
        Some(Value::String(x)) if !x.is_empty() => Some(x.clone()),
        _ => None,
    }
}

/// Epoch milliseconds (number) or an ISO-8601 string, to epoch seconds.
fn ts(v: &Value, k: &str) -> Option<f64> {
    match v.get(k)? {
        Value::Number(n) => n.as_f64().map(|ms| ms / 1000.0),
        Value::String(x) => parse_iso(x),
        _ => None,
    }
}

/// Minimal `YYYY-MM-DDTHH:MM:SS[.fff]Z` parser (UTC), enough for Claude Code's stamps.
pub fn parse_iso(x: &str) -> Option<f64> {
    let b = x.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, z: usize| x.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    // days from civil (Howard Hinnant)
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let mut secs = (days * 86400 + h * 3600 + mi * 60 + se) as f64;
    if let Some(frac) = x.get(19..).and_then(|r| r.strip_prefix('.')) {
        let digits: String = frac.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(v) = format!("0.{digits}").parse::<f64>() {
            secs += v;
        }
    }
    Some(secs)
}

// ------------------------------------------------------------------ cache

/// JSON files re-read only when their mtime changes.
#[derive(Default)]
pub struct Cache {
    json: HashMap<PathBuf, (SystemTime, Value)>,
    session_dirs: HashMap<String, PathBuf>,
    titles: HashMap<PathBuf, (SystemTime, Option<String>)>,
}

impl Cache {
    pub fn json(&mut self, p: &Path) -> Option<Value> {
        let m = mtime(p)?;
        if let Some((t, v)) = self.json.get(p) {
            if *t == m {
                return Some(v.clone());
            }
        }
        let v = first_json(&fs::read(p).ok()?)?;
        self.json.insert(p.to_path_buf(), (m, v.clone()));
        Some(v)
    }
}

/// The first JSON value in `raw`, ignoring anything after it. Claude Code
/// sometimes rewrites a session file in place without truncating it, leaving
/// the tail of a longer earlier version behind the new object; a strict parse
/// would drop a live session (`claude agents --json` itself does).
pub fn first_json(raw: &[u8]) -> Option<Value> {
    serde_json::Deserializer::from_slice(raw).into_iter::<Value>().next()?.ok()
}

pub fn tail_jsonl(p: &Path, n: usize) -> Vec<Value> {
    let Ok(mut f) = fs::File::open(p) else { return vec![] };
    let size = f.seek(SeekFrom::End(0)).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(size.saturating_sub(64 * 1024)));
    let mut buf = String::new();
    let mut raw = vec![];
    if f.read_to_end(&mut raw).is_err() {
        return vec![];
    }
    buf.push_str(&String::from_utf8_lossy(&raw));
    let mut out: Vec<Value> = buf
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v.is_object())
        .take(n)
        .collect();
    out.reverse();
    out
}

// ------------------------------------------------------------------ fleet configs

#[derive(Clone, Debug)]
pub struct DeclaredAgent {
    pub name: String,
    pub short: String,
    pub dir: Option<String>,
    pub prompt: Option<String>,
    pub permission_mode: String,
    pub bg_isolation: bool,
}

#[derive(Clone, Debug)]
pub struct FleetConfig {
    pub tag: String,
    pub dir: PathBuf,
    pub machine: Option<String>,
    pub agents: Vec<DeclaredAgent>,
    pub error: Option<String>,
}

pub fn match_machine(machines: &Value, hostnames: &[String]) -> Option<String> {
    let obj = machines.as_object()?;
    for (label, m) in obj {
        let h = s(m, "hostname").unwrap_or_default().to_lowercase();
        if !h.is_empty() && (hostnames.contains(&h) || hostnames.iter().any(|x| x == h.split('.').next().unwrap_or(""))) {
            return Some(label.clone());
        }
    }
    None
}

pub fn read_fleets(env: &Env, cache: &mut Cache) -> Vec<FleetConfig> {
    let mut paths: Vec<PathBuf> = fs::read_dir(&env.root)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path().join("FLEET_CONFIG.json")).filter(|p| p.is_file()).collect())
        .unwrap_or_default();
    paths.sort();
    let mut out = vec![];
    for p in paths {
        let dir = p.parent().unwrap().to_path_buf();
        let dname = dir.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
        let data = cache.json(&p);
        let Some(data) = data.filter(|d| s(d, "fleet").is_some()) else {
            out.push(FleetConfig {
                tag: dname.trim_end_matches("-fleet").to_uppercase(),
                dir,
                machine: None,
                agents: vec![],
                error: Some("FLEET_CONFIG.json unreadable".into()),
            });
            continue;
        };
        let tag = s(&data, "fleet").unwrap().to_uppercase();
        let label = match_machine(data.get("machines").unwrap_or(&Value::Null), &env.hostnames);
        let mut agents = vec![];
        for a in data.get("agents").and_then(|x| x.as_array()).cloned().unwrap_or_default() {
            let short = s(&a, "name").unwrap_or("?".into()).to_uppercase();
            let per = |k: &str| label.as_ref().and_then(|l| a.get(k).and_then(|m| m.get(l)).cloned());
            agents.push(DeclaredAgent {
                name: format!("[{tag}]-{short}"),
                short,
                dir: per("dirs").and_then(|v| v.as_str().map(String::from)),
                prompt: s(&a, "prompt"),
                permission_mode: per("permission_mode")
                    .and_then(|v| v.as_str().map(String::from))
                    .filter(|x| !x.is_empty())
                    .unwrap_or("bypassPermissions".into()),
                bg_isolation: per("bg_isolation").map(|v| v.as_bool().unwrap_or(false) || v.is_string()).unwrap_or(false),
            });
        }
        out.push(FleetConfig { tag, dir, machine: label, agents, error: None });
    }
    out
}

// ------------------------------------------------------------------ session rows

#[derive(Clone, Debug, PartialEq)]
pub struct SessionRow {
    pub session_id: String,
    pub name: String,
    pub cwd: String,
    /// interactive | background
    pub kind: String,
    /// background job id (what `attach` / `logs` take)
    pub short_id: Option<String>,
    /// present only while the process is alive
    pub pid: Option<i64>,
    /// background job state; stale on live rows
    pub state: Option<String>,
    /// busy / idle on live rows
    pub status: Option<String>,
    pub status_since: Option<f64>,
    pub started_at: Option<f64>,
    /// set while a live session sits at a prompt (`status: waiting`), e.g. "permission prompt"
    pub waiting_for: Option<String>,
}

impl SessionRow {
    pub fn live(&self) -> bool {
        self.pid.is_some()
    }
}

pub fn pid_alive(pid: i64) -> bool {
    if pid <= 0 || pid > i32::MAX as i64 {
        return false;
    }
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn norm_kind(k: &str) -> String {
    match k {
        "bg" | "background" => "background".into(),
        "" => "?".into(),
        other => other.into(),
    }
}

/// Rows from the files `claude agents --json --all` reads. None when the
/// layout is not there (a different Claude Code version, or no Claude Code).
pub fn rows_from_files(env: &Env, cache: &mut Cache) -> Option<Vec<SessionRow>> {
    let sess_dir = env.claude_home.join("sessions");
    let jobs_dir = env.claude_home.join("jobs");
    if !sess_dir.is_dir() && !jobs_dir.is_dir() {
        return None;
    }
    let mut rows: Vec<SessionRow> = vec![];
    let mut by_sid: HashMap<String, usize> = HashMap::new();
    if let Ok(rd) = fs::read_dir(&jobs_dir) {
        for e in rd.filter_map(|e| e.ok()) {
            let jid = e.file_name().to_string_lossy().into_owned();
            let Some(d) = cache.json(&e.path().join("state.json")) else { continue };
            let Some(sid) = s(&d, "sessionId") else { continue };
            by_sid.insert(sid.clone(), rows.len());
            rows.push(SessionRow {
                session_id: sid,
                name: s(&d, "name").unwrap_or_default(),
                cwd: s(&d, "cwd").unwrap_or_default(),
                kind: "background".into(),
                short_id: Some(jid),
                pid: None,
                state: s(&d, "state"),
                status: None,
                status_since: None,
                started_at: ts(&d, "createdAt"),
                waiting_for: None,
            });
        }
    }
    if let Ok(rd) = fs::read_dir(&sess_dir) {
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.extension().map(|x| x != "json").unwrap_or(true) {
                continue;
            }
            let Some(d) = cache.json(&p) else { continue };
            let Some(pid) = d.get("pid").and_then(|v| v.as_i64()) else { continue };
            if !pid_alive(pid) {
                continue;
            }
            let Some(sid) = s(&d, "sessionId") else { continue };
            let kind = norm_kind(&s(&d, "kind").unwrap_or_default());
            let live = SessionRow {
                session_id: sid.clone(),
                name: s(&d, "name").unwrap_or_default(),
                cwd: s(&d, "cwd").unwrap_or_default(),
                kind: kind.clone(),
                short_id: s(&d, "jobId"),
                pid: Some(pid),
                state: None,
                status: s(&d, "status"),
                status_since: ts(&d, "statusUpdatedAt"),
                started_at: ts(&d, "startedAt"),
                waiting_for: if s(&d, "status").as_deref() == Some("waiting") {
                    Some(s(&d, "waitingFor").unwrap_or("input".into()))
                } else {
                    None
                },
            };
            match by_sid.get(&sid) {
                // a live background process: its job row gains pid and status
                Some(&i) if kind == "background" => {
                    let r = &mut rows[i];
                    r.pid = live.pid;
                    r.status = live.status;
                    r.status_since = live.status_since;
                    r.waiting_for = live.waiting_for;
                    if !live.name.is_empty() {
                        r.name = live.name;
                    }
                }
                _ => rows.push(live),
            }
        }
    }
    Some(rows)
}

pub fn parse_cli_rows(v: &Value) -> Vec<SessionRow> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    Some(SessionRow {
                        session_id: s(r, "sessionId")?,
                        name: s(r, "name").unwrap_or_default(),
                        cwd: s(r, "cwd").unwrap_or_default(),
                        kind: norm_kind(&s(r, "kind").unwrap_or_default()),
                        short_id: s(r, "id"),
                        pid: r.get("pid").and_then(|p| p.as_i64()),
                        state: s(r, "state"),
                        status: s(r, "status"),
                        status_since: None,
                        started_at: ts(r, "startedAt"),
                        waiting_for: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn rows_from_cli(env: &Env) -> Result<Vec<SessionRow>, String> {
    let out = Command::new(&env.claude_bin)
        .args(["agents", "--json", "--all"])
        .output()
        .map_err(|e| format!("`claude agents --json` failed: {e}"))?;
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|_| {
        let err = String::from_utf8_lossy(&out.stderr);
        format!("`claude agents --json` gave no JSON: {}", err.lines().last().unwrap_or("no output"))
    })?;
    if !v.is_array() {
        return Err("`claude agents --json` did not return a list".into());
    }
    Ok(parse_cli_rows(&v))
}

/// Session rows plus the name of the source used.
pub fn read_sessions(env: &Env, cache: &mut Cache) -> Result<(Vec<SessionRow>, &'static str), String> {
    if !env.force_cli {
        if let Some(rows) = rows_from_files(env, cache) {
            return Ok((rows, "files"));
        }
    }
    rows_from_cli(env).map(|r| (r, "cli"))
}

// ------------------------------------------------------------------ per-session detail

#[derive(Clone, Debug, Default)]
pub struct TimelineEntry {
    pub at: String,
    pub state: String,
    pub detail: String,
}

#[derive(Clone, Debug, Default)]
pub struct JobState {
    pub state: Option<String>,
    pub detail: Option<String>,
    pub needs: Option<String>,
    pub tokens: Option<i64>,
    pub tasks: i64,
    pub queued: i64,
    pub timeline: Vec<TimelineEntry>,
}

pub fn read_job(env: &Env, cache: &mut Cache, short_id: Option<&str>) -> JobState {
    let Some(id) = short_id else { return JobState::default() };
    let jdir = env.claude_home.join("jobs").join(id);
    let d = cache.json(&jdir.join("state.json")).unwrap_or(Value::Null);
    let inflight = d.get("inFlight").cloned().unwrap_or(Value::Null);
    let mut timeline: Vec<TimelineEntry> = tail_jsonl(&jdir.join("timeline.jsonl"), 5)
        .iter()
        .map(|r| TimelineEntry {
            at: s(r, "at").unwrap_or_default(),
            state: s(r, "state").unwrap_or_default(),
            detail: s(r, "detail").unwrap_or_default(),
        })
        .collect();
    timeline.reverse(); // newest first
    JobState {
        state: s(&d, "state"),
        detail: s(&d, "detail"),
        needs: s(&d, "needs"),
        tokens: d.get("tokens").and_then(|v| v.as_i64()),
        tasks: inflight.get("tasks").and_then(|v| v.as_i64()).unwrap_or(0),
        queued: inflight.get("queued").and_then(|v| v.as_i64()).unwrap_or(0),
        timeline,
    }
}

// ------------------------------------------------------------------ sub-agents

#[derive(Clone, Debug)]
pub struct SubAgentRec {
    pub agent_id: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub team: Option<String>,
    pub teammate: bool,
    pub last_write: f64,
}

fn find_session_dir(env: &Env, cache: &mut Cache, sid: &str) -> Option<PathBuf> {
    if let Some(p) = cache.session_dirs.get(sid) {
        if p.is_dir() {
            return Some(p.clone());
        }
    }
    let rd = fs::read_dir(env.claude_home.join("projects")).ok()?;
    for e in rd.filter_map(|e| e.ok()) {
        let cand = e.path().join(sid);
        if cand.is_dir() {
            cache.session_dirs.insert(sid.to_string(), cand.clone());
            return Some(cand);
        }
    }
    None
}

pub fn read_subagents(env: &Env, cache: &mut Cache, sid: &str) -> Vec<SubAgentRec> {
    let Some(sdir) = find_session_dir(env, cache, sid) else { return vec![] };
    let Ok(rd) = fs::read_dir(sdir.join("subagents")) else { return vec![] };
    let mut out = vec![];
    for e in rd.filter_map(|e| e.ok()) {
        let fname = e.file_name().to_string_lossy().into_owned();
        let Some(agent_id) = fname.strip_prefix("agent-").and_then(|r| r.strip_suffix(".meta.json")) else { continue };
        let m = cache.json(&e.path()).unwrap_or(Value::Null);
        let log = e.path().with_file_name(format!("agent-{agent_id}.jsonl"));
        let last = mtime_secs(&log).or_else(|| mtime_secs(&e.path())).unwrap_or(0.0);
        out.push(SubAgentRec {
            agent_id: agent_id.to_string(),
            parent_id: s(&m, "parentAgentId"),
            label: s(&m, "name").or_else(|| s(&m, "description")).unwrap_or(agent_id.to_string()),
            agent_type: s(&m, "agentType"),
            model: s(&m, "model"),
            team: s(&m, "teamName"),
            teammate: s(&m, "taskKind").as_deref() == Some("in_process_teammate"),
            last_write: last,
        });
    }
    out
}

// ------------------------------------------------------------------ task lists

#[derive(Clone, Debug)]
pub struct TaskItem {
    pub id: String,
    pub subject: String,
    pub status: String,
    pub owner: Option<String>,
}

pub fn read_tasks(env: &Env, cache: &mut Cache, team: &str) -> Vec<TaskItem> {
    let Ok(rd) = fs::read_dir(env.claude_home.join("tasks").join(team)) else { return vec![] };
    let mut out = vec![];
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.extension().map(|x| x != "json").unwrap_or(true) {
            continue;
        }
        let Some(d) = cache.json(&p).filter(|d| d.is_object()) else { continue };
        let stem = p.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
        out.push(TaskItem {
            id: s(&d, "id").unwrap_or(stem),
            subject: s(&d, "subject").or_else(|| s(&d, "description")).unwrap_or("?".into()),
            status: s(&d, "status").unwrap_or("pending".into()),
            owner: s(&d, "owner"),
        });
    }
    out.sort_by_key(|t| (t.id.parse::<i64>().unwrap_or(i64::MAX), t.id.clone()));
    out
}

// ------------------------------------------------------------------ stopped leads

/// Claude Code's project-dir name for a cwd: every non-alphanumeric character becomes `-`.
pub fn project_dir_name(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// A session's name: `<session>/custom-title.json`, else the last `customTitle`
/// record near the start or end of the transcript (transcripts can be large).
fn transcript_title(cache: &mut Cache, jsonl: &Path) -> Option<String> {
    let sid_dir = jsonl.with_extension("");
    if let Some(t) = cache.json(&sid_dir.join("custom-title.json")).and_then(|v| s(&v, "customTitle")) {
        return Some(t);
    }
    let m = mtime(jsonl)?;
    if let Some((t, title)) = cache.titles.get(jsonl) {
        if *t == m {
            return title.clone();
        }
    }
    let mut f = fs::File::open(jsonl).ok()?;
    let size = f.seek(SeekFrom::End(0)).ok()?;
    let mut found = None;
    for start in [0, size.saturating_sub(256 * 1024)] {
        let _ = f.seek(SeekFrom::Start(start));
        let mut raw = vec![0u8; (256 * 1024).min(size as usize)];
        let n = f.read(&mut raw).unwrap_or(0);
        for line in String::from_utf8_lossy(&raw[..n]).lines() {
            if line.contains("\"customTitle\"") {
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    if let Some(t) = s(&v, "customTitle") {
                        found = Some(t);
                    }
                }
            }
        }
    }
    cache.titles.insert(jsonl.to_path_buf(), (m, found.clone()));
    found
}

/// The most recent lead session for a fleet dir, from the transcripts Claude Code
/// keeps after an interactive session ends: the newest transcript in the fleet
/// dir's project whose name starts with `[TAG]-LEAD`. Only the newest dozen are
/// looked at. The row has no pid (it is not running); `started_at` is the
/// transcript's last write, i.e. when the lead was last active.
pub fn last_lead_session(env: &Env, cache: &mut Cache, tag: &str, fleet_dir: &str) -> Option<SessionRow> {
    let proj = env.claude_home.join("projects").join(project_dir_name(fleet_dir.trim_end_matches('/')));
    let mut files: Vec<(f64, PathBuf)> = fs::read_dir(&proj)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "jsonl").unwrap_or(false))
        .filter_map(|p| mtime_secs(&p).map(|t| (t, p)))
        .collect();
    files.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let want = format!("[{}]-LEAD", tag.to_uppercase());
    for (t, p) in files.into_iter().take(12) {
        let Some(title) = transcript_title(cache, &p) else { continue };
        if title.to_uppercase() == want || title.to_uppercase().starts_with(&format!("{want}-")) {
            return Some(SessionRow {
                session_id: p.file_stem()?.to_string_lossy().into_owned(),
                name: title,
                cwd: fleet_dir.to_string(),
                kind: "interactive".into(),
                short_id: None,
                pid: None,
                state: Some("ended".into()),
                status: None,
                status_since: None,
                started_at: Some(t),
                waiting_for: None,
            });
        }
    }
    None
}
