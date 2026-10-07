//! Readers: session files, job rows, transcripts, sub-agents, task lists, events.jsonl (contracts.md section 6). Owner: core-sessions.
//!
//! Everything under the Claude Code home is internal to Claude Code and may
//! change in any release, so every reader tolerates missing files, missing
//! fields, unknown fields and trailing bytes, and reads only the fields it needs.
//! Nothing here writes.
//!
//! Rows come from the files `claude agents --json --all` reads
//! (`sessions/<pid>.json` for live processes, `jobs/<id>/state.json` for job
//! rows); liveness is `kill(pid, 0)`. That keeps a refresh to a few
//! milliseconds; the CLI (`crate::claude::agents`) is the cross-check.
use crate::env::Env;
use crate::types::*;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

pub fn mtime_secs(p: &Path) -> Option<f64> {
    mtime(p).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs_f64())
}

/// When a file was made (epoch seconds): its birth time. None when the file
/// system keeps none ("cannot tell"; a later write is no answer).
pub fn birth_secs(p: &Path) -> Option<f64> {
    fs::metadata(p).ok()?.created().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs_f64())
}

pub(crate) fn s(v: &Value, k: &str) -> Option<String> {
    match v.get(k) {
        Some(Value::String(x)) if !x.is_empty() => Some(x.clone()),
        _ => None,
    }
}

/// `ts` for readers outside this module.
pub fn stamp(v: &Value, k: &str) -> Option<f64> {
    ts(v, k)
}

/// Epoch milliseconds (number) or an ISO-8601 string, to epoch seconds. An
/// absurd stamp (before 2001, or more than a day ahead) is no stamp (BUG-45).
fn ts(v: &Value, k: &str) -> Option<f64> {
    let t = match v.get(k)? {
        Value::Number(n) => n.as_f64().map(|ms| ms / 1000.0),
        Value::String(x) => parse_iso(x),
        _ => None,
    }?;
    (t.is_finite() && t > 1.0e9 && t < now() + 86400.0).then_some(t)
}

/// Minimal `YYYY-MM-DDTHH:MM:SS[.fff]Z` parser (UTC), enough for Claude Code's stamps.
pub fn parse_iso(x: &str) -> Option<f64> {
    if x.len() < 19 {
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

/// The first JSON value in `raw`, ignoring anything after it. Claude Code
/// sometimes rewrites a session file in place without truncating it, leaving
/// the tail of a longer earlier version behind the new object; a strict parse
/// would drop a live session (`claude agents --json` itself does).
pub fn first_json(raw: &[u8]) -> Option<Value> {
    serde_json::Deserializer::from_slice(raw).into_iter::<Value>().next()?.ok()
}

// ------------------------------------------------------------------ cache

/// File reads keyed by mtime, so a refresh every 2 s re-reads only what changed.
#[derive(Default)]
pub struct FileCache {
    json: HashMap<PathBuf, (SystemTime, Value)>,
    session_dirs: HashMap<String, PathBuf>,
    titles: HashMap<PathBuf, (SystemTime, Option<String>)>,
    trust: HashMap<PathBuf, (SystemTime, Vec<PathBuf>)>,
    /// `transcript_root` per transcript, by mtime
    roots: HashMap<PathBuf, (SystemTime, Vec<String>)>,
    /// lineage transcripts not found, and when (`transcript_missing_cached`)
    missing: HashMap<String, f64>,
    events: EventTail,
}

#[derive(Default)]
struct EventTail {
    path: PathBuf,
    /// the file's inode: a compaction (BUG-44) replaces the file
    ino: u64,
    offset: u64,
    rows: Vec<Event>,
}

impl FileCache {
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

// ------------------------------------------------------------------ events

pub fn events_path(env: &Env) -> PathBuf {
    env.state.join("events.jsonl")
}

pub(crate) fn parse_event_line(line: &str) -> Option<Event> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let ev = s(&v, "event")?;
    let labels = v
        .get("labels")
        .and_then(|l| l.as_object())
        .map(|o| o.iter().filter_map(|(k, x)| Some((k.clone(), x.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    Some(Event {
        ts: v.get("ts").and_then(|t| t.as_f64()).unwrap_or(0.0),
        event: ev,
        session_id: s(&v, "session_id"),
        source: s(&v, "source"),
        reason: s(&v, "reason"),
        agent_id: s(&v, "agent_id"),
        agent_type: s(&v, "agent_type"),
        tool_name: s(&v, "tool_name"),
        to: s(&v, "to"),
        pid: v.get("pid").and_then(|p| p.as_i64()),
        labels,
        rules: v.get("rules").and_then(|r| r.as_u64()).map(|r| r as u32),
        job: s(&v, "job"),
        kind: s(&v, "kind"),
        entrypoint: s(&v, "entrypoint"),
        binding: s(&v, "binding"),
        cwd: s(&v, "cwd"),
    })
}

/// Read events.jsonl (tolerant: bad lines are skipped).
pub fn events(env: &Env) -> Vec<Event> {
    let Ok(raw) = fs::read(events_path(env)) else { return vec![] };
    String::from_utf8_lossy(&raw).lines().filter_map(parse_event_line).collect()
}

/// Same as `events`, but reads only what was appended since the last call.
pub fn events_cached(env: &Env, cache: &mut FileCache) -> Vec<Event> {
    let p = events_path(env);
    let t = &mut cache.events;
    if t.path != p {
        *t = EventTail { path: p.clone(), ..Default::default() };
    }
    let Ok(mut f) = fs::File::open(&p) else {
        t.offset = 0;
        t.rows.clear();
        return vec![];
    };
    let size = f.seek(SeekFrom::End(0)).unwrap_or(0);
    let ino = { use std::os::unix::fs::MetadataExt; f.metadata().map(|m| m.ino()).unwrap_or(0) };
    if ino != t.ino {
        t.ino = ino;
        t.offset = 0;
        t.rows.clear();
    }
    if size < t.offset {
        t.offset = 0;
        t.rows.clear();
    }
    if size > t.offset {
        let _ = f.seek(SeekFrom::Start(t.offset));
        let mut raw = vec![];
        if f.read_to_end(&mut raw).is_ok() {
            // a partial last line waits for the next call
            let end = raw.iter().rposition(|&b| b == b'\n').map(|i| i + 1).unwrap_or(0);
            t.rows.extend(String::from_utf8_lossy(&raw[..end]).lines().filter_map(parse_event_line));
            t.offset += end as u64;
        }
    }
    t.rows.clone()
}

/// Does this env carry nimitz labels? Only `NIMITZ_ID` or `NIMITZ_ROLE` make a
/// label (contracts.md section 7): an inherited `NIMITZ_HOME` alone does not.
pub fn is_labelled(labels: &BTreeMap<String, String>) -> bool {
    ["NIMITZ_ID", "NIMITZ_ROLE"].iter().any(|k| labels.get(*k).map(|v| !v.is_empty()).unwrap_or(false))
}

/// Labels per session id. The newest SessionStart of a session decides: a
/// later resume of the same id with no labels (real Claude Code continues the
/// id after `--resume <id> --name X`) makes it unlabelled, and the map then
/// holds an empty entry for it. Other events count only for a session whose
/// SessionStart line was lost. Labels without `NIMITZ_ID`/`NIMITZ_ROLE`
/// (`is_labelled`) are no labels.
pub fn labels_by_session(events: &[Event]) -> HashMap<String, BTreeMap<String, String>> {
    let mut out: HashMap<String, BTreeMap<String, String>> = HashMap::new();
    let mut started: std::collections::HashSet<String> = Default::default();
    for e in events {
        let Some(sid) = &e.session_id else { continue };
        let labels = if is_labelled(&e.labels) { e.labels.clone() } else { BTreeMap::new() };
        if e.event == "SessionStart" {
            started.insert(sid.clone());
            out.insert(sid.clone(), labels);
        } else if !started.contains(sid) && !labels.is_empty() {
            out.insert(sid.clone(), labels);
        }
    }
    out
}

/// Labels per process: the newest SessionStart of each (pid, session id).
/// Labels are kept per pid (plan-v4 section 3): a second process on a
/// member's conversation (a `-p --resume`, an SDK host) does not strip the
/// member's own process of its labels.
pub fn labels_by_pid_session(events: &[Event]) -> HashMap<(i64, String), BTreeMap<String, String>> {
    let mut out = HashMap::new();
    for e in events.iter().filter(|e| e.event == "SessionStart") {
        let (Some(pid), Some(sid)) = (e.pid, &e.session_id) else { continue };
        let labels = if is_labelled(&e.labels) { e.labels.clone() } else { BTreeMap::new() };
        out.insert((pid, sid.clone()), labels);
    }
    out
}

/// The labels of one row. Live: the SessionStart of its own process (pid and
/// session id), else what its job row saved (`respawnFlags --settings`), else
/// the newest SessionStart of the session. Stopped: the job row's saved labels
/// decide (a resume restores them; a stray process on the conversation does
/// not change them), else the newest SessionStart of the session.
pub fn row_labels(r: &SessionRow, by_pid: &HashMap<(i64, String), BTreeMap<String, String>>, by_sid: &HashMap<String, BTreeMap<String, String>>) -> BTreeMap<String, String> {
    let saved = || is_labelled(&r.job_labels).then(|| r.job_labels.clone());
    let newest = || by_sid.get(&r.session_id).cloned();
    let own = || r.pid.and_then(|p| by_pid.get(&(p, r.session_id.clone())).cloned());
    match r.pid {
        Some(_) => own().or_else(saved).or_else(newest).unwrap_or_default(),
        None => saved().or_else(newest).unwrap_or_default(),
    }
}

/// Labels per session id, row by row (`row_labels`); where two rows share a
/// conversation (a job row and a process holding it), the live one's labels
/// win. Sessions with no row keep their newest SessionStart's labels.
pub fn session_labels(events: &[Event], rows: &[SessionRow]) -> HashMap<String, BTreeMap<String, String>> {
    let by_sid = labels_by_session(events);
    let by_pid = labels_by_pid_session(events);
    let mut out = by_sid.clone();
    let mut decided: std::collections::HashSet<String> = Default::default();
    for r in rows {
        if decided.contains(&r.session_id) && r.pid.is_none() {
            continue;
        }
        out.insert(r.session_id.clone(), row_labels(r, &by_pid, &by_sid));
        if r.pid.is_some() {
            decided.insert(r.session_id.clone());
        }
    }
    out
}

/// `jobs/pins.json`: the jobs the daemon never idle-retires (a list of short ids).
pub fn pins(env: &Env) -> Vec<String> {
    fs::read(env.claude_home.join("jobs").join("pins.json"))
        .ok()
        .and_then(|b| first_json(&b))
        .and_then(|v| v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()))
        .unwrap_or_default()
}

/// The daemon's last `bg retire <short>: <why>` line in `daemon.log` (under the
/// Claude home; `[<iso stamp>] [bg] bg retire <short>: settled, idle 61m`): the
/// job was parked by the daemon, no SessionEnd fired. A line older than
/// `since` (the job's last SessionStart) is an earlier retire, not this one;
/// a line with no stamp counts.
pub fn daemon_retired(env: &Env, short: &str, since: Option<f64>) -> Option<String> {
    let raw = fs::read(env.claude_home.join("daemon.log")).ok()?;
    let key = format!("retire {short}");
    String::from_utf8_lossy(&raw)
        .lines()
        .filter_map(|l| {
            let i = l.find(&key)?;
            if !l[..i].trim_end().ends_with("bg") && !l[..i].trim_end().ends_with("[bg]") {
                return None;
            }
            let stamp = l.strip_prefix('[').and_then(|r| r.split_once(']')).and_then(|(t, _)| parse_iso(t));
            if let (Some(t), Some(s)) = (stamp, since) {
                if t < s {
                    return None;
                }
            }
            Some(l[i + key.len()..].trim_start_matches(':').trim().to_string())
        })
        .last()
}

/// The NIMITZ_* labels in a job row's saved launch options: `savedSettings` (a
/// `--settings` JSON string) or `respawnFlags` (argv: `--settings <json>`).
pub(crate) fn job_labels(d: &Value) -> BTreeMap<String, String> {
    let mut settings: Vec<String> = vec![];
    if let Some(x) = s(d, "savedSettings") {
        settings.push(x);
    }
    match d.get("respawnFlags") {
        Some(Value::Array(a)) => {
            let argv: Vec<&str> = a.iter().filter_map(|x| x.as_str()).collect();
            for w in argv.windows(2) {
                if w[0] == "--settings" {
                    settings.push(w[1].to_string());
                }
            }
        }
        Some(Value::Object(o)) => {
            if let Some(x) = o.get("settings").and_then(|x| x.as_str()) {
                settings.push(x.to_string());
            }
        }
        _ => {}
    }
    let mut out = BTreeMap::new();
    for raw in settings {
        let Ok(v) = serde_json::from_str::<Value>(&raw) else { continue };
        if let Some(env) = v.get("env").and_then(|e| e.as_object()) {
            for (k, x) in env {
                if let (true, Some(x)) = (k.starts_with("NIMITZ_"), x.as_str()) {
                    out.insert(k.clone(), x.to_string());
                }
            }
        }
    }
    if is_labelled(&out) { out } else { BTreeMap::new() }
}

// ------------------------------------------------------------------ session rows

/// One session as the files show it.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionRow {
    pub session_id: String,
    pub name: String,
    pub cwd: String,
    /// interactive | background
    pub kind: String,
    /// background job id (what `attach`, `stop`, `rm` take)
    pub short_id: Option<String>,
    /// present only while the process is alive
    pub pid: Option<i64>,
    /// background job state; stale on live rows
    pub state: Option<String>,
    /// busy / idle / waiting on live rows
    pub status: Option<String>,
    pub status_since: Option<f64>,
    pub started_at: Option<f64>,
    /// set while a live session sits at a prompt (`status: waiting`)
    pub waiting_for: Option<String>,
    /// labels the job row saved with its launch options
    pub job_labels: BTreeMap<String, String>,
    /// a session file for this row exists but cannot be read (caught mid-write):
    /// its process may be live, so the row is neither live nor stopped
    pub unreadable: bool,
    /// the session this one was forked from, when the job row records it
    pub forked_from: Option<String>,
    /// Remote Control is connected: the session file's `bridgeSessionId` (M9)
    pub bridge_session_id: Option<String>,
    /// the job's birth session id (`state.json` `sessionId`) when the
    /// conversation moved on (`resumeSessionId`, or the live session file)
    pub birth_id: Option<String>,
    /// the session file's `entrypoint` (cli, sdk-cli, claude-vscode, ...)
    pub entrypoint: Option<String>,
    /// the job is in `jobs/pins.json`
    pub pinned: bool,
    /// the session file says `spare: true` (a pre-warmed process the agent
    /// view claims as its "new session" or draft)
    pub spare: bool,
    /// where an interactive process runs, from the session file (`tmux`
    /// `<session>:@<window>.%<pane>`, a Herdr pane)
    pub terminal: Option<String>,
    /// when the session file appeared (epoch seconds): the earliest of its
    /// birth time, its mtime and `startedAt`
    pub appeared_at: Option<f64>,
}

impl SessionRow {
    pub fn live(&self) -> bool {
        self.pid.is_some()
    }
    pub fn info(&self, labels: BTreeMap<String, String>) -> SessionInfo {
        SessionInfo {
            session_id: self.session_id.clone(),
            short_id: self.short_id.clone(),
            pid: self.pid,
            kind: self.kind.clone(),
            state: self.state.clone(),
            status: if self.unreadable && self.pid.is_none() { Some("unreadable".into()) } else { self.status.clone() },
            waiting_for: self.waiting_for.clone(),
            started_at: self.started_at,
            status_since: self.status_since,
            name: self.name.clone(),
            cwd: self.cwd.clone(),
            labels,
            birth_id: self.birth_id.clone(),
            entrypoint: self.entrypoint.clone(),
            pinned: self.pinned,
            copy: None,
            terminal: self.terminal.clone(),
        }
    }
    /// Every session id this row answers to: the conversation now and the birth id.
    pub fn session_ids(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.session_id.as_str()).chain(self.birth_id.as_deref())
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

/// Claude Code's own processes (`kind` daemon, daemon-worker): never a member,
/// never a stowaway, so their session files are not rows.
fn internal_kind(k: &str) -> bool {
    matches!(k, "daemon" | "daemon-worker")
}

/// The terminal an interactive process runs in, as the session file names it:
/// `tmux` (`<session>:@<window>.%<pane>`), else any `herdr*` field.
fn terminal_of(d: &Value) -> Option<String> {
    if let Some(t) = s(d, "tmux") {
        return Some(format!("tmux {t}"));
    }
    d.as_object()?.iter().find(|(k, v)| k.starts_with("herdr") && v.is_string()).map(|(k, v)| format!("{k} {}", v.as_str().unwrap_or_default()))
}

/// Rows from the files `claude agents --json --all` reads. None when the
/// layout is not there (a different Claude Code version, or no Claude Code).
///
/// A job row (`jobs/<id>/state.json`) and a live session file join by **job
/// id** (the session file's `jobId`), never by session id (plan-v4 section 4):
/// the conversation a job runs now is the live session file's `sessionId`,
/// else `state.json` `resumeSessionId`, else its birth `sessionId`. A live
/// process with no job (interactive, SDK) is its own row, even on a job's
/// conversation.
pub fn rows_from_files(env: &Env, cache: &mut FileCache) -> Option<Vec<SessionRow>> {
    let sess_dir = env.claude_home.join("sessions");
    let jobs_dir = env.claude_home.join("jobs");
    if !sess_dir.is_dir() && !jobs_dir.is_dir() {
        return None;
    }
    let list = |d: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(d).map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default();
        v.sort();
        v
    };
    let pins = pins(env);
    let mut rows: Vec<SessionRow> = vec![];
    let mut by_job: HashMap<String, usize> = HashMap::new();
    for jd in list(&jobs_dir) {
        let jid = jd.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
        if jid == "pins.json" {
            continue;
        }
        let Some(d) = cache.json(&jd.join("state.json")) else { continue };
        let Some(born) = s(&d, "sessionId") else { continue };
        let now = s(&d, "resumeSessionId").unwrap_or_else(|| born.clone());
        by_job.insert(jid.clone(), rows.len());
        rows.push(SessionRow {
            birth_id: (now != born).then_some(born),
            session_id: now,
            name: s(&d, "name").unwrap_or_default(),
            cwd: s(&d, "cwd").unwrap_or_default(),
            kind: "background".into(),
            pinned: pins.iter().any(|p| *p == jid),
            short_id: Some(jid),
            pid: None,
            state: s(&d, "state"),
            status: None,
            // on a stopped row: when the job last changed
            status_since: ts(&d, "updatedAt"),
            started_at: ts(&d, "createdAt"),
            waiting_for: None,
            job_labels: job_labels(&d),
            unreadable: false,
            forked_from: ["forkedFrom", "forkedFromSessionId", "parentSessionId", "resumedFrom"].iter().find_map(|k| s(&d, k)),
            bridge_session_id: None,
            entrypoint: None,
            spare: false,
            terminal: None,
            appeared_at: None,
        });
    }
    for p in list(&sess_dir) {
        if p.extension().map(|x| x != "json").unwrap_or(true) {
            continue;
        }
        let Some(d) = cache.json(&p) else {
            // empty or truncated (caught mid-write): the process may be live. A
            // file named after a job id marks that row; one named after a pid
            // (real Claude Code) is left to the `claude agents` cross-check.
            let stem = p.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some(&i) = by_job.get(&stem) {
                rows[i].unreadable = true;
            }
            continue;
        };
        let Some(pid) = d.get("pid").and_then(|v| v.as_i64()) else { continue };
        if !pid_alive(pid) {
            continue;
        }
        let Some(sid) = s(&d, "sessionId") else { continue };
        let kind = norm_kind(&s(&d, "kind").unwrap_or_default());
        if internal_kind(&kind) {
            continue;
        }
        let status = s(&d, "status");
        let job = s(&d, "jobId");
        let live = SessionRow {
            session_id: sid.clone(),
            name: s(&d, "name").unwrap_or_default(),
            cwd: s(&d, "cwd").unwrap_or_default(),
            kind: kind.clone(),
            pinned: job.as_ref().map(|j| pins.contains(j)).unwrap_or(false),
            short_id: job.clone(),
            pid: Some(pid),
            state: None,
            status: status.clone(),
            status_since: ts(&d, "statusUpdatedAt"),
            started_at: ts(&d, "startedAt"),
            waiting_for: if status.as_deref() == Some("waiting") { Some(s(&d, "waitingFor").unwrap_or("input".into())) } else { None },
            job_labels: BTreeMap::new(),
            unreadable: false,
            forked_from: None,
            bridge_session_id: s(&d, "bridgeSessionId"),
            birth_id: None,
            entrypoint: s(&d, "entrypoint"),
            spare: d.get("spare").and_then(|v| v.as_bool()).unwrap_or(false),
            terminal: terminal_of(&d),
            appeared_at: [birth_secs(&p), mtime_secs(&p), ts(&d, "startedAt")].into_iter().flatten().reduce(f64::min),
        };
        match job.as_ref().and_then(|j| by_job.get(j)) {
            // a live background process: its job row gains pid, status and the
            // conversation it runs now
            Some(&i) => {
                let r = &mut rows[i];
                let born = r.birth_id.take().or_else(|| Some(r.session_id.clone())).filter(|b| *b != sid);
                r.birth_id = born;
                r.session_id = live.session_id;
                r.pid = live.pid;
                r.status = live.status;
                r.status_since = live.status_since;
                r.waiting_for = live.waiting_for;
                r.bridge_session_id = live.bridge_session_id;
                r.entrypoint = live.entrypoint;
                r.spare = live.spare;
                r.terminal = live.terminal;
                r.appeared_at = live.appeared_at;
                if !live.name.is_empty() {
                    r.name = live.name;
                }
            }
            None => rows.push(live),
        }
    }
    Some(rows)
}

/// Job folders whose `state.json` exists but does not parse: their session is
/// unknown, so nothing may be planned on a guess (BUG-14).
pub fn corrupt_jobs(env: &Env) -> Vec<String> {
    let Ok(rd) = fs::read_dir(env.claude_home.join("jobs")) else { return vec![] };
    let mut out: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path().join("state.json");
            fs::read(&p).map(|b| first_json(&b).map(|v| !v.is_object()).unwrap_or(true)).unwrap_or(false)
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// Rows from `claude agents --json --all` output.
pub fn parse_cli_rows(v: &Value) -> Vec<SessionRow> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let kind = norm_kind(&s(r, "kind").unwrap_or_default());
                    if internal_kind(&kind) {
                        return None;
                    }
                    Some(SessionRow {
                        session_id: s(r, "sessionId")?,
                        name: s(r, "name").unwrap_or_default(),
                        cwd: s(r, "cwd").unwrap_or_default(),
                        kind,
                        short_id: s(r, "id"),
                        pid: r.get("pid").and_then(|p| p.as_i64()),
                        state: s(r, "state"),
                        status: s(r, "status"),
                        status_since: None,
                        started_at: ts(r, "startedAt"),
                        waiting_for: None,
                        job_labels: BTreeMap::new(),
                        unreadable: false,
                        forked_from: None,
                        bridge_session_id: s(r, "bridgeSessionId"),
                        birth_id: None,
                        entrypoint: None,
                        pinned: false,
                        spare: false,
                        terminal: None,
                        appeared_at: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ------------------------------------------------------------------ job detail

#[derive(Clone, Debug, Default, PartialEq)]
pub struct JobState {
    pub state: Option<String>,
    pub detail: Option<String>,
    /// the question a job left for you
    pub needs: Option<String>,
}

pub fn read_job(env: &Env, cache: &mut FileCache, short_id: Option<&str>) -> JobState {
    let Some(id) = short_id else { return JobState::default() };
    let d = cache.json(&env.claude_home.join("jobs").join(id).join("state.json")).unwrap_or(Value::Null);
    JobState { state: s(&d, "state"), detail: s(&d, "detail"), needs: s(&d, "needs") }
}

// ------------------------------------------------------------------ transcripts

/// Claude Code's project-dir name for a cwd: every non-alphanumeric character becomes `-`.
pub fn project_dir_name(cwd: &str) -> String {
    cwd.trim_end_matches('/').chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// The project-dir names a folder's transcripts may live under: Claude Code
/// names it after the real path, so a symlinked folder has two candidates.
fn project_dirs(cwd: &str) -> Vec<String> {
    let mut v = vec![project_dir_name(cwd)];
    let real = project_dir_name(&crate::env::canon(Path::new(cwd)).to_string_lossy());
    if real != v[0] {
        v.push(real);
    }
    v
}

/// The transcript of a session: under its folder's project dir first, then anywhere.
pub fn transcript(env: &Env, cache: &mut FileCache, cwd: Option<&str>, sid: &str) -> Option<PathBuf> {
    let projects = env.claude_home.join("projects");
    for c in cwd.map(project_dirs).unwrap_or_default() {
        let p = projects.join(c).join(format!("{sid}.jsonl"));
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(d) = find_session_dir(env, cache, sid) {
        let p = d.with_extension("jsonl");
        if p.is_file() {
            return Some(p);
        }
    }
    for e in fs::read_dir(&projects).ok()?.filter_map(|e| e.ok()) {
        let p = e.path().join(format!("{sid}.jsonl"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// How much of a transcript the root reader reads at most: the metadata
/// lines (custom-title, agent-name, mode) come first, then one
/// `file-history-snapshot` per turn (a copy hoists every one of them above
/// its first uuid line), then the attachments and the first user message.
const ROOT_BYTES: usize = 4 * 1024 * 1024;
/// How long a lineage transcript that was not found stays "missing" before
/// `transcript_missing` looks for it again (the lookup scans every project dir).
const MISSING_AGAIN_S: f64 = 60.0;

/// The root of a conversation: the uuids near the top of its transcript (the
/// first line that carries a `uuid`, the chain root, and the first `user` or
/// `assistant` message with its `parentUuid`). A copy of a conversation
/// (`/branch`, `/fork`, `--fork-session`, `--bg -c`) copies the transcript,
/// so it shares these; a fresh conversation (`/clear`) has its own. Only the
/// head of the file is read (`ROOT_BYTES`; lines without a uuid cost nothing
/// but their bytes), cached by (path, mtime). Empty when the transcript has
/// no message yet.
pub fn transcript_root(cache: &mut FileCache, jsonl: &Path) -> Vec<String> {
    let Some(m) = mtime(jsonl) else { return vec![] };
    if let Some((t, root)) = cache.roots.get(jsonl) {
        if *t == m {
            return root.clone();
        }
    }
    let out = fs::File::open(jsonl).map(|f| root_of_reader(std::io::BufReader::new(f))).unwrap_or_default();
    cache.roots.insert(jsonl.to_path_buf(), (m, out.clone()));
    out
}

fn root_of_reader(r: impl std::io::BufRead) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut read = 0usize;
    for line in r.lines().map_while(Result::ok) {
        read += line.len() + 1;
        if read > ROOT_BYTES {
            break;
        }
        if !line.contains("\"uuid\"") {
            continue;
        }
        {
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            let Some(uuid) = s(&v, "uuid") else { continue };
            let message = matches!(s(&v, "type").as_deref(), Some("user") | Some("assistant"));
            // the chain root (the first line with a uuid) and the first message
            if out.is_empty() || message {
                out.push(uuid);
                out.extend(s(&v, "parentUuid"));
            }
            if message {
                break;
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `transcript`, for a lineage conversation whose file may be gone: a miss is
/// remembered for `MISSING_AGAIN_S`, so the scan of every project dir is not
/// repeated on each refresh.
pub fn transcript_missing_cached(env: &Env, cache: &mut FileCache, cwd: Option<&str>, sid: &str) -> Option<PathBuf> {
    let t = now();
    if cache.missing.get(sid).is_some_and(|at| t - at < MISSING_AGAIN_S) {
        return None;
    }
    let found = transcript(env, cache, cwd, sid);
    if found.is_none() {
        cache.missing.insert(sid.to_string(), t);
    } else {
        cache.missing.remove(sid);
    }
    found
}

/// Transcripts in a folder's project dir, newest first: (last write, session id).
pub fn transcripts_in(env: &Env, cwd: &str) -> Vec<(f64, String)> {
    let mut out: Vec<(f64, String)> = transcript_files_in(env, cwd)
        .into_iter()
        .filter_map(|p| Some((mtime_secs(&p)?, p.file_stem()?.to_string_lossy().into_owned())))
        .collect();
    out.dedup_by(|a, b| a.1 == b.1);
    out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// The transcript files (`<sid>.jsonl`) in a folder's project dir(s).
pub fn transcript_files_in(env: &Env, cwd: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = vec![];
    for name in project_dirs(cwd) {
        let proj = env.claude_home.join("projects").join(name);
        out.extend(
            fs::read_dir(&proj)
                .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|x| x == "jsonl").unwrap_or(false)).collect::<Vec<_>>())
                .unwrap_or_default(),
        );
    }
    out
}

/// A session's name from its transcript: `<session>/custom-title.json`, else
/// the last `customTitle` record near the start or end (transcripts can be large).
pub fn transcript_title(cache: &mut FileCache, jsonl: &Path) -> Option<String> {
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

// ------------------------------------------------------------------ sub-agents and tasks

#[derive(Clone, Debug)]
pub struct SubAgentRec {
    pub agent_id: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub agent_type: Option<String>,
    pub team: Option<String>,
    pub teammate: bool,
    pub last_write: f64,
}

fn find_session_dir(env: &Env, cache: &mut FileCache, sid: &str) -> Option<PathBuf> {
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

pub fn read_subagents(env: &Env, cache: &mut FileCache, sid: &str) -> Vec<SubAgentRec> {
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
            team: s(&m, "teamName"),
            teammate: s(&m, "taskKind").as_deref() == Some("in_process_teammate"),
            last_write: last,
        });
    }
    out.sort_by(|a, b| b.last_write.partial_cmp(&a.last_write).unwrap_or(std::cmp::Ordering::Equal));
    out
}

pub fn read_tasks(env: &Env, cache: &mut FileCache, team: &str) -> Vec<Task> {
    let Ok(rd) = fs::read_dir(env.claude_home.join("tasks").join(team)) else { return vec![] };
    let mut out = vec![];
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.extension().map(|x| x != "json").unwrap_or(true) {
            continue;
        }
        let Some(d) = cache.json(&p).filter(|d| d.is_object()) else { continue };
        let stem = p.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
        out.push(Task {
            id: s(&d, "id").unwrap_or(stem),
            subject: s(&d, "subject").or_else(|| s(&d, "description")).unwrap_or("?".into()),
            status: s(&d, "status").unwrap_or("pending".into()),
            owner: s(&d, "owner"),
        });
    }
    out.sort_by_key(|t| (t.id.parse::<i64>().unwrap_or(i64::MAX), t.id.clone()));
    out
}

// ------------------------------------------------------------------ trust

/// Whether Claude Code trusts `dir` (it or a parent has `hasTrustDialogAccepted`),
/// from `<claude_home>/.claude.json` (the `CLAUDE_CONFIG_DIR` layout) and, when
/// the Claude home is `~/.claude`, from `~/.claude.json`. Inside a git
/// repository Claude Code keys the trust on the repository (its root or
/// below), so a parent above the root does not count (M16). The trusted roots
/// are cached by mtime (the real file is megabytes).
pub fn folder_trusted(env: &Env, cache: &mut FileCache, dir: &str) -> bool {
    let mut files = vec![env.claude_home.join(".claude.json")];
    if env.claude_home.file_name().map(|n| n == ".claude").unwrap_or(false) {
        if let Some(parent) = env.claude_home.parent() {
            files.push(parent.join(".claude.json"));
        }
    }
    // Claude Code records real paths (/private/tmp for /tmp): the keys come in both forms
    let keys = crate::env::trust_keys(Path::new(dir.trim_end_matches('/')));
    files.iter().any(|f| trusted_roots(cache, f).iter().any(|root| keys.contains(root)))
}

fn trusted_roots(cache: &mut FileCache, f: &Path) -> Vec<PathBuf> {
    let Some(m) = mtime(f) else { return vec![] };
    if let Some((t, roots)) = cache.trust.get(f) {
        if *t == m {
            return roots.clone();
        }
    }
    let roots: Vec<PathBuf> = fs::read(f)
        .ok()
        .and_then(|b| first_json(&b))
        .and_then(|v| {
            v.get("projects").and_then(|p| p.as_object()).map(|projects| {
                projects
                    .iter()
                    .filter(|(_, p)| p.get("hasTrustDialogAccepted").and_then(|x| x.as_bool()) == Some(true))
                    .map(|(k, _)| PathBuf::from(k.trim_end_matches('/')))
                    .collect()
            })
        })
        .unwrap_or_default();
    cache.trust.insert(f.to_path_buf(), (m, roots.clone()));
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_json_ignores_trailing_bytes() {
        let v = first_json(br#"{"pid": 1, "sessionId": "a"}ionId": "old"}"#).unwrap();
        assert_eq!(v["sessionId"], "a");
        assert!(first_json(b"not json").is_none());
        assert!(first_json(b"").is_none());
    }

    #[test]
    fn iso_and_project_dir() {
        assert_eq!(parse_iso("1970-01-01T00:01:00.500Z"), Some(60.5));
        assert_eq!(parse_iso("short"), None);
        assert_eq!(project_dir_name("/a/b.c_d/"), "-a-b-c-d");
    }

    #[test]
    fn event_lines_tolerate_junk_and_new_fields() {
        let e = parse_event_line(r#"{"ts": 1.5, "event": "SessionStart", "session_id": "s", "labels": {"NIMITZ_ID": "F"}, "scratchpad_dir": "/x"}"#).unwrap();
        assert_eq!(e.labels.get("NIMITZ_ID").map(String::as_str), Some("F"));
        assert!(parse_event_line("{\"event\": 3}").is_none());
        assert!(parse_event_line("garbage").is_none());
    }

    #[test]
    fn the_root_of_a_real_shaped_transcript_head_is_found_past_the_snapshots() {
        // a copy hoists every file-history-snapshot (one per turn) above its
        // first uuid line: 200 of them must not hide the root
        let mut head = String::new();
        head.push_str("{\"type\": \"custom-title\", \"customTitle\": \"[E2E.SQ]-S1\", \"sessionId\": \"s\"}\n");
        head.push_str("{\"type\": \"agent-name\", \"agentName\": \"S1\", \"sessionId\": \"s\"}\n");
        head.push_str("{\"type\": \"mode\", \"mode\": \"default\", \"sessionId\": \"s\"}\n");
        for i in 0..200 {
            head.push_str(&format!("{{\"type\": \"file-history-snapshot\", \"messageId\": \"m{i:04}\", \"snapshot\": {{\"trackedFileBackups\": {{}}, \"timestamp\": \"2026-10-07T00:00:00.000Z\"}}, \"isSnapshotUpdate\": false}}\n"));
        }
        head.push_str("{\"parentUuid\": null, \"isSidechain\": false, \"attachment\": {}, \"type\": \"attachment\", \"uuid\": \"f0afe4e5\", \"timestamp\": \"t\"}\n");
        head.push_str("{\"parentUuid\": \"f0afe4e5\", \"isSidechain\": false, \"attachment\": {}, \"type\": \"attachment\", \"uuid\": \"0061d5f7\", \"timestamp\": \"t\"}\n");
        head.push_str("{\"parentUuid\": \"0061d5f7\", \"isSidechain\": false, \"type\": \"user\", \"message\": {\"role\": \"user\", \"content\": \"reply ok\"}, \"uuid\": \"05fb9112\", \"timestamp\": \"t\"}\n");
        head.push_str("{\"parentUuid\": \"05fb9112\", \"type\": \"assistant\", \"uuid\": \"30290373\"}\n");
        let root = root_of_reader(std::io::Cursor::new(head.as_bytes()));
        assert_eq!(root, ["0061d5f7", "05fb9112", "f0afe4e5"]);
        // no message yet: no root
        assert!(root_of_reader(std::io::Cursor::new(b"{\"type\": \"custom-title\"}\n")).is_empty());
        // the byte bound stops a file whose head is all snapshots
        let big = "{\"type\": \"file-history-snapshot\", \"snapshot\": \"x\"}\n".repeat(ROOT_BYTES / 40);
        assert!(root_of_reader(std::io::Cursor::new(format!("{big}{{\"type\": \"user\", \"uuid\": \"late\"}}\n").as_bytes())).is_empty());
    }

    #[test]
    fn cli_rows_parse() {
        let v: Value = serde_json::from_str(r#"[{"sessionId": "s1", "id": "j1", "kind": "bg", "pid": 4, "name": "n", "cwd": "/c"}, {"id": "no-session"}]"#).unwrap();
        let rows = parse_cli_rows(&v);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "background");
        assert!(rows[0].live());
    }

    /// The daemon's retire reasons seen so far, verbatim (2.1.292): `settled,
    /// idle 61m`, `idle-prompt, idle 61m`, and `abandoned-stale, idle 48h,
    /// worker 2.1.289 (daemon 2.1.292)`. The parser takes any text after the
    /// colon; the newest line of the job counts, and none older than `since`.
    #[test]
    fn daemon_retire_reason_is_any_text_after_the_colon() {
        let (_td, env) = crate::config::tests::test_env();
        std::fs::create_dir_all(&env.claude_home).unwrap();
        std::fs::write(
            env.claude_home.join("daemon.log"),
            "[2026-09-19T22:10:17.241Z] [bg] bg retire 0abc1234: idle-prompt, idle 61m\n\
             [2026-09-19T22:10:17.505Z] [bg] bg settled 0abc1234 (done)\n\
             [2026-10-07T01:47:03.070Z] [bg] bg retire 9cdceb2d: settled, idle 61m\n\
             [2026-10-07T05:14:03.119Z] [bg] bg retire 0abc1234: abandoned-stale, idle 48h, worker 2.1.289 (daemon 2.1.292)\n\
             [2026-10-07T05:14:03.594Z] [bg] bg settled 0abc1234 (done)\n",
        )
        .unwrap();
        assert_eq!(daemon_retired(&env, "0abc1234", None).as_deref(), Some("abandoned-stale, idle 48h, worker 2.1.289 (daemon 2.1.292)"));
        assert_eq!(daemon_retired(&env, "9cdceb2d", None).as_deref(), Some("settled, idle 61m"));
        // a job started after its last retire: that line is an earlier retire, not this one
        let after = parse_iso("2026-10-07T05:20:00.000Z").unwrap();
        assert_eq!(daemon_retired(&env, "0abc1234", Some(after)), None);
        let between = parse_iso("2026-10-01T00:00:00.000Z").unwrap();
        assert_eq!(daemon_retired(&env, "0abc1234", Some(between)).as_deref(), Some("abandoned-stale, idle 48h, worker 2.1.289 (daemon 2.1.292)"));
        assert_eq!(daemon_retired(&env, "deadbeef", None), None);
    }
}
