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
    // a job: its row's saved labels decide alone (a job launched without
    // labels carries the daemon's inherited ones in its env, and in its hook
    // lines: `job_backs_labels`); a job whose row is not written yet, or a
    // pre-warmed spare's, has none to show
    if r.short_id.is_some() {
        return saved().unwrap_or_default();
    }
    match r.pid {
        Some(_) => own().or_else(saved).or_else(newest).unwrap_or_default(),
        None => saved().or_else(newest).unwrap_or_default(),
    }
}

/// Do a background job's NIMITZ_* labels come from its own launch? Claude
/// Code's daemon hands its own environment to every job it starts, so a
/// daemon started from a labelled lead's shell gives that lead's labels to
/// every job launched without labels of its own (the user's `claude --bg`, a
/// hand copy, the agent view's draft). A job's labels count only when its row
/// saved them (`--settings` in `respawnFlags`) and they name the same id.
/// `None`: nothing to judge by (no job: an interactive or SDK process, whose
/// env is its labels; or a job whose row is not written yet).
pub fn job_backs_labels(claude_home: &Path, job: Option<&str>, labels: &BTreeMap<String, String>) -> Option<bool> {
    let saved = job_saved_labels(claude_home, job?)?;
    Some(is_labelled(&saved) && saved.get("NIMITZ_ID") == labels.get("NIMITZ_ID"))
}

/// The NIMITZ_* labels a job's launch saved (`respawnFlags --settings`);
/// `None` when the job has no row (yet).
pub fn job_saved_labels(claude_home: &Path, job: &str) -> Option<BTreeMap<String, String>> {
    let d = std::fs::read(claude_home.join("jobs").join(job).join("state.json")).ok().and_then(|b| first_json(&b))?;
    Some(job_labels(&d))
}

/// The key a row's labels and copy verdict are kept under: its job (short
/// id), else its session id. Two jobs can run one conversation (a wake by an
/// id that is not the job's birth id starts a second job on it, acceptance
/// F1), so a session id alone does not name a row.
pub fn row_key(r: &SessionRow) -> String {
    r.short_id.clone().unwrap_or_else(|| r.session_id.clone())
}

/// Labels per row (`row_labels`, under `row_key`), and per session id: where
/// two rows with no job share a conversation (a process holding it), the live
/// one's labels win under the session id. Sessions with no row keep their
/// newest SessionStart's labels under their id.
pub fn session_labels(events: &[Event], rows: &[SessionRow]) -> HashMap<String, BTreeMap<String, String>> {
    let by_sid = labels_by_session(events);
    let by_pid = labels_by_pid_session(events);
    let mut out = by_sid.clone();
    let mut decided: std::collections::HashSet<String> = Default::default();
    for r in rows {
        let mine = row_labels(r, &by_pid, &by_sid);
        if r.short_id.is_some() {
            out.insert(row_key(r), mine.clone());
        }
        if decided.contains(&r.session_id) && r.pid.is_none() {
            continue;
        }
        out.insert(r.session_id.clone(), mine);
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
                // the identity labels only (a launch may also carry nimitz's
                // own state paths: `claude::settings_json`)
                if let (true, Some(x)) = (["NIMITZ_FLEET", "NIMITZ_SQUADRON", "NIMITZ_ROLE", "NIMITZ_ID", "NIMITZ_HOME"].contains(&k.as_str()), x.as_str()) {
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

/// The process ids above this one (its parent, its parent's parent, ...):
/// `/proc/<pid>/stat` on Linux; empty elsewhere.
pub fn ancestors() -> Vec<i64> {
    let mut out = vec![];
    let mut pid = std::process::id() as i64;
    for _ in 0..64 {
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else { break };
        // the field after the `(comm)`, which may hold spaces: state, then ppid
        let Some(ppid) = stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(1)).and_then(|p| p.parse::<i64>().ok()) else { break };
        if ppid <= 0 {
            break;
        }
        out.push(ppid);
        pid = ppid;
    }
    out
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
        let spare = d.get("spare").and_then(|v| v.as_bool()).unwrap_or(false);
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
            spare,
            terminal: terminal_of(&d),
            // a spare (the agent view's draft or "new session") is a pre-warmed
            // process the view claims later: its age is its last touch, so the
            // grace time (`snapshot::TRANSIENT_GRACE_S`) runs from the claim
            appeared_at: if spare { mtime_secs(&p) } else { [birth_secs(&p), mtime_secs(&p), ts(&d, "startedAt")].into_iter().flatten().reduce(f64::min) },
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
        if has_message(&p) {
            return Some(p);
        }
    }
    if let Some(d) = find_session_dir(env, cache, sid) {
        let p = d.with_extension("jsonl");
        if has_message(&p) {
            return Some(p);
        }
    }
    for e in fs::read_dir(&projects).ok()?.filter_map(|e| e.ok()) {
        let p = e.path().join(format!("{sid}.jsonl"));
        if has_message(&p) {
            return Some(p);
        }
    }
    None
}

/// A transcript that holds at least one message (a line with a `uuid`,
/// within `ROOT_BYTES`: a `/branch` copy hoists every file-history snapshot
/// line, up to 4 KB each and many of them, above the first message). A stub
/// `claude stop` writes after the real file was moved away has none: a
/// resume of it would start the conversation over (acceptance env F5), so it
/// is no transcript.
pub fn has_message(p: &Path) -> bool {
    let Ok(f) = fs::File::open(p) else { return false };
    if !f.metadata().map(|m| m.is_file()).unwrap_or(false) {
        return false;
    }
    !root_of_reader(std::io::BufReader::new(f)).is_empty()
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
    /// the Workflow run that started it (`wf_<run>`), with the workflow's
    /// name when known and the agent's phase
    pub workflow: Option<String>,
    pub workflow_name: Option<String>,
    pub phase: Option<String>,
}

/// The session's own folder, `projects/<project>/<sid>`. More than one
/// project can hold a `<sid>` folder (a Workflow run in another cwd leaves
/// one with only `workflows/scripts`): the one beside the transcript
/// (`<sid>.jsonl`) wins, then one with `subagents`, then any.
fn find_session_dir(env: &Env, cache: &mut FileCache, sid: &str) -> Option<PathBuf> {
    if let Some(p) = cache.session_dirs.get(sid) {
        if p.is_dir() {
            return Some(p.clone());
        }
    }
    let rd = fs::read_dir(env.claude_home.join("projects")).ok()?;
    let found: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path().join(sid)).filter(|c| c.is_dir()).collect();
    let rank = |c: &PathBuf| if c.with_extension("jsonl").is_file() { 0 } else if c.join("subagents").is_dir() { 1 } else { 2 };
    let best = found.into_iter().min_by_key(rank)?;
    // only a sure pick is cached: a transcript may appear later
    if rank(&best) == 0 {
        cache.session_dirs.insert(sid.to_string(), best.clone());
    }
    Some(best)
}

/// The session's crew: `subagents/agent-<id>.meta.json`, and the agents
/// Workflow runs started, `subagents/workflows/wf_<run>/agent-<id>.meta.json`
/// (the run's name from `workflows/wf_<run>.json`, written when it ends, or
/// its script `workflows/scripts/<name>-wf_<run>.js`). A workflow agent's log
/// can go on at the top level (`subagents/agent-<id>.jsonl`, no meta): its
/// last write is the newer of the two.
pub fn read_subagents(env: &Env, cache: &mut FileCache, sid: &str) -> Vec<SubAgentRec> {
    let Some(sdir) = find_session_dir(env, cache, sid) else { return vec![] };
    let top = sdir.join("subagents");
    let mut out = vec![];
    if let Ok(rd) = fs::read_dir(top.join("workflows")) {
        let mut runs: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
        runs.sort();
        for run in runs {
            let id = run.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let name = workflow_name(cache, &sdir, &id);
            for mut r in read_agent_dir(cache, &run, Some(&top)) {
                r.workflow = Some(id.clone());
                r.workflow_name = name.clone();
                out.push(r);
            }
        }
    }
    let in_runs: std::collections::HashSet<String> = out.iter().map(|r| r.agent_id.clone()).collect();
    out.extend(read_agent_dir(cache, &top, None).into_iter().filter(|r| !in_runs.contains(&r.agent_id)));
    out.sort_by(|a, b| b.last_write.partial_cmp(&a.last_write).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// The `agent-<id>.meta.json` records in `dir`; `also` is a second folder
/// whose `agent-<id>.jsonl` may hold the newer log.
fn read_agent_dir(cache: &mut FileCache, dir: &Path, also: Option<&Path>) -> Vec<SubAgentRec> {
    let Ok(rd) = fs::read_dir(dir) else { return vec![] };
    let mut out = vec![];
    for e in rd.filter_map(|e| e.ok()) {
        let fname = e.file_name().to_string_lossy().into_owned();
        let Some(agent_id) = fname.strip_prefix("agent-").and_then(|r| r.strip_suffix(".meta.json")) else { continue };
        let m = cache.json(&e.path()).unwrap_or(Value::Null);
        let log = format!("agent-{agent_id}.jsonl");
        let last = [Some(dir.join(&log)), also.map(|a| a.join(&log))]
            .into_iter()
            .flatten()
            .filter_map(|p| mtime_secs(&p))
            .reduce(f64::max)
            .or_else(|| mtime_secs(&e.path()))
            .unwrap_or(0.0);
        out.push(SubAgentRec {
            agent_id: agent_id.to_string(),
            parent_id: s(&m, "parentAgentId"),
            label: s(&m, "name").or_else(|| s(&m, "description")).unwrap_or(agent_id.to_string()),
            agent_type: s(&m, "agentType"),
            team: s(&m, "teamName"),
            teammate: s(&m, "taskKind").as_deref() == Some("in_process_teammate"),
            last_write: last,
            workflow: None,
            workflow_name: None,
            phase: s(&m, "workflowPhase"),
        });
    }
    out
}

/// A Workflow run's name: `workflowName` in `workflows/<run>.json` (written
/// when the run ends), else from its script `workflows/scripts/<name>-<run>.js`.
fn workflow_name(cache: &mut FileCache, sdir: &Path, run: &str) -> Option<String> {
    let wf = sdir.join("workflows");
    if let Some(n) = cache.json(&wf.join(format!("{run}.json"))).and_then(|v| s(&v, "workflowName")) {
        return Some(n);
    }
    let suffix = format!("-{run}.js");
    fs::read_dir(wf.join("scripts"))
        .ok()?
        .filter_map(|e| e.ok())
        .find_map(|e| e.file_name().to_string_lossy().strip_suffix(&suffix).map(String::from))
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
    #[cfg(target_os = "linux")]
    #[test]
    fn the_ancestors_start_at_the_parent() {
        let up = super::ancestors();
        assert_eq!(up.first().copied(), Some(unsafe { libc::getppid() } as i64), "{up:?}");
    }

    use super::*;

    /// A second project's `<sid>` folder (only `workflows/scripts`) is never
    /// taken for the session's, whichever the directory lists first.
    #[test]
    fn the_session_dir_is_the_one_beside_the_transcript() {
        for (real, decoy) in [("-a-real", "-b-decoy"), ("-b-real", "-a-decoy")] {
            let td = tempfile::tempdir().unwrap();
            let vars: HashMap<String, String> = [("NIMITZ_CLAUDE_HOME".to_string(), td.path().to_string_lossy().into_owned())].into();
            let env = Env::from_vars(&vars);
            let sid = "5e55e55e-0000-4000-8000-000000000000";
            let projects = td.path().join("projects");
            fs::create_dir_all(projects.join(decoy).join(sid).join("workflows/scripts")).unwrap();
            fs::create_dir_all(projects.join(real).join(sid).join("subagents")).unwrap();
            fs::write(projects.join(real).join(format!("{sid}.jsonl")), "{}\n").unwrap();
            fs::write(projects.join(real).join(sid).join("subagents/agent-x1.meta.json"), r#"{"description":"x"}"#).unwrap();
            let recs = read_subagents(&env, &mut FileCache::default(), sid);
            assert_eq!(recs.iter().map(|r| r.agent_id.as_str()).collect::<Vec<_>>(), ["x1"], "{real} vs {decoy}");
        }
    }

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
    /// A `/branch` copy hoists every file-history snapshot line (up to 4 KB
    /// each, many of them) above the first message: still a transcript. A
    /// stub (`claude stop` after the file was moved away) is none.
    #[test]
    fn a_branch_copy_with_many_hoisted_snapshots_still_has_a_message() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("branch.jsonl");
        let snap = format!("{{\"type\": \"file-history-snapshot\", \"snapshot\": \"{}\"}}\n", "x".repeat(4000));
        let mut text = snap.repeat(200);
        text.push_str("{\"type\": \"user\", \"uuid\": \"u1\", \"parentUuid\": null, \"sessionId\": \"s\"}\n");
        fs::write(&p, &text).unwrap();
        assert!(text.len() > 64 * 1024);
        assert!(has_message(&p), "200 hoisted 4 KB lines before the first message");
        let stub = td.path().join("stub.jsonl");
        fs::write(&stub, "{\"type\": \"summary\", \"summary\": \"x\", \"leafUuid\": \"y\"}\n{\"type\": \"system\", \"content\": \"stopped\"}\n").unwrap();
        assert!(!has_message(&stub));
        assert!(!has_message(&td.path().join("missing.jsonl")));
    }

}

/// Only a Claude Code process (a stale session file whose pid another
/// program reuses names nothing): `claude`, or the native binary under
/// `.../claude/versions/<v>`, or an npm install's `node .../claude-code/cli.js`.
fn is_claude(argv: &[String]) -> bool {
    argv.first().is_some_and(|a0| Path::new(a0).file_name().is_some_and(|n| n.to_string_lossy().starts_with("claude")) || a0.contains("/claude/versions/"))
        || argv.get(1).is_some_and(|a1| a1.contains("claude-code/cli"))
}

/// The session of the job a pane's foreground attaches to (`claude attach
/// <job>`, a short or full job id): its job row's conversation now
/// (`resumeSessionId`, which `/clear` moves; else `sessionId`). What a pane
/// shows when Herdr claims no session for it: an attach client never runs
/// Herdr's SessionStart hook, so Herdr sees claude there but not which
/// session. Not used against a claim: a claim the attach does not back may be
/// the agent view (R-24).
pub fn session_of_attach(claude_home: &Path, procs: &[(Vec<String>, Option<i64>)]) -> Option<String> {
    procs.iter().filter(|(argv, _)| is_claude(argv)).find_map(|(argv, _)| {
        let job = argv.windows(2).find(|w| w[0] == "attach").map(|w| w[1].as_str())?;
        let row = |short: &str| std::fs::read(claude_home.join("jobs").join(short).join("state.json")).ok().and_then(|b| first_json(&b));
        let v = row(job).or_else(|| job.get(..8).filter(|_| job.len() > 8).and_then(row))?;
        ["resumeSessionId", "sessionId"].iter().find_map(|k| v.get(*k).and_then(|x| x.as_str()).filter(|x| !x.is_empty())).map(String::from)
    })
}

/// The session a pane Herdr claims none for shows, from its foreground alone:
/// the job a `claude attach <job>` attaches to (`session_of_attach`), else
/// the conversation a `claude --resume <sid>` holds, else the session a
/// foreground pid's session file names (`claude -c`, a plain `claude`).
/// The held case shows its member's pane too: nimitz flags it, and the user
/// still jumps to it.
pub fn session_of_unclaimed(claude_home: &Path, procs: &[(Vec<String>, Option<i64>)]) -> Option<String> {
    session_of_attach(claude_home, procs)
        .or_else(|| procs.iter().filter(|(argv, _)| is_claude(argv)).find_map(|(argv, _)| argv.windows(2).find(|w| w[0] == "--resume" || w[0] == "-r").map(|w| w[1].clone()).filter(|s| !s.starts_with('-'))))
        .or_else(|| session_of_foreground(claude_home, procs))
}

/// Does what runs in a pane back its multiplexer's claim that the pane shows
/// session `sid`? Herdr learns a pane's session from its own SessionStart
/// hook, which names the pane by the process's inherited `HERDR_PANE_ID`: a
/// background job carries the pane id of whoever launched it (or the
/// daemon's), so the claim alone can name the wrong pane (a ship's session in
/// its commodore's pane). A claim stands when a foreground process
/// (`procs`: argv and pid) names the session (`--resume <sid>`), attaches to
/// its job (`claude attach <job>`), or has a session file
/// (`sessions/<pid>.json`) that names it.
pub fn claim_backed(claude_home: &Path, procs: &[(Vec<String>, Option<i64>)], sid: &str) -> bool {
    let job_of = |short: &str| -> bool {
        if sid.starts_with(short) {
            return true;
        }
        let row = std::fs::read(claude_home.join("jobs").join(short).join("state.json")).ok().and_then(|b| first_json(&b));
        if row.as_ref().is_some_and(|v| ["sessionId", "resumeSessionId"].iter().any(|k| v.get(k).and_then(|x| x.as_str()) == Some(sid))) {
            return true;
        }
        std::fs::read_dir(claude_home.join("sessions")).into_iter().flatten().flatten().filter_map(|e| std::fs::read(e.path()).ok().and_then(|b| first_json(&b))).any(|v| v.get("jobId").and_then(|x| x.as_str()) == Some(short) && v.get("sessionId").and_then(|x| x.as_str()) == Some(sid))
    };
    procs.iter().any(|(argv, pid)| {
        argv.iter().any(|a| a == sid)
            || argv.windows(2).any(|w| w[0] == "attach" && job_of(&w[1]))
            || pid.is_some_and(|p| {
                std::fs::read(claude_home.join("sessions").join(format!("{p}.json"))).ok().and_then(|b| first_json(&b)).and_then(|v| v.get("sessionId").and_then(|x| x.as_str()).map(|x| x == sid)).unwrap_or(false)
            })
    })
}

/// The session a pane's foreground runs, read from the processes themselves
/// (no multiplexer claim): a foreground pid's session file (an interactive
/// session in the pane). What a pane shows when Herdr's claim is not backed.
/// Not an attach client's job: `session_of_attach` reads that, and the
/// typing paths check the exact attach again (R-24).
pub fn session_of_foreground(claude_home: &Path, procs: &[(Vec<String>, Option<i64>)]) -> Option<String> {
    procs.iter().filter(|(argv, _)| is_claude(argv)).find_map(|(_, pid)| {
        let v = std::fs::read(claude_home.join("sessions").join(format!("{}.json", (*pid)?))).ok().and_then(|b| first_json(&b))?;
        v.get("sessionId").and_then(|x| x.as_str()).map(String::from)
    })
}

#[cfg(test)]
mod claim_tests {
    use super::claim_backed;

    /// A pane's session claim stands on its foreground: the session named in
    /// the argv, an attach of its job (state.json or a session file), or the
    /// foreground pid's session file. A claude process whose own session file
    /// names another session does not back it (a ship's id in its
    /// commodore's pane).
    #[test]
    fn a_claim_stands_only_on_what_runs_in_the_pane() {
        let td = tempfile::tempdir().unwrap();
        let ch = td.path();
        std::fs::create_dir_all(ch.join("sessions")).unwrap();
        std::fs::create_dir_all(ch.join("jobs/aaaa1111")).unwrap();
        std::fs::write(ch.join("jobs/aaaa1111/state.json"), r#"{"sessionId":"aaaa1111-birth","resumeSessionId":"s-after-clear"}"#).unwrap();
        std::fs::write(ch.join("sessions/700.json"), r#"{"pid":700,"sessionId":"s-commodore","jobId":null}"#).unwrap();
        let p = |argv: &[&str], pid: Option<i64>| vec![(argv.iter().map(|x| x.to_string()).collect::<Vec<_>>(), pid)];
        assert!(claim_backed(ch, &p(&["claude", "--resume", "s-ship"], None), "s-ship"));
        assert!(claim_backed(ch, &p(&["claude", "attach", "aaaa1111"], None), "s-after-clear"));
        assert!(claim_backed(ch, &p(&["claude", "attach", "aaaa1111"], None), "aaaa1111-birth"));
        assert!(claim_backed(ch, &p(&["claude"], Some(700)), "s-commodore"));
        // the misattribution: the commodore's process, Herdr says the ship's session
        assert!(!claim_backed(ch, &p(&["claude"], Some(700)), "s-ship"));
        assert!(!claim_backed(ch, &p(&["claude", "attach", "aaaa1111"], None), "s-ship"));
        assert!(!claim_backed(ch, &p(&["zsh"], Some(1)), "s-ship"));
        assert!(!claim_backed(ch, &[], "s-ship"), "an unreadable foreground backs nothing");
        // what the pane runs, from the processes alone
        assert_eq!(super::session_of_foreground(ch, &p(&["claude"], Some(700))).as_deref(), Some("s-commodore"));
        assert_eq!(super::session_of_foreground(ch, &p(&["claude", "attach", "aaaa1111"], None)), None, "an attach may show the agent view");
        assert_eq!(super::session_of_attach(ch, &p(&["/home/u/.local/bin/claude", "attach", "aaaa1111"], None)).as_deref(), Some("s-after-clear"), "the job's session now");
        assert_eq!(super::session_of_attach(ch, &p(&["claude", "attach", "aaaa1111-0000-0000-0000-000000000000"], None)).as_deref(), Some("s-after-clear"), "a full job id");
        assert_eq!(super::session_of_attach(ch, &p(&["claude", "attach", "bbbb2222"], None)), None, "no job row");
        assert_eq!(super::session_of_attach(ch, &p(&["vim", "attach", "aaaa1111"], None)), None, "not claude");
        assert_eq!(super::session_of_attach(ch, &p(&["zsh"], Some(1))), None);
        assert_eq!(super::session_of_unclaimed(ch, &p(&["claude", "attach", "aaaa1111"], None)).as_deref(), Some("s-after-clear"));
        assert_eq!(super::session_of_unclaimed(ch, &p(&["claude", "--resume", "s-held"], None)).as_deref(), Some("s-held"), "a held conversation");
        assert_eq!(super::session_of_unclaimed(ch, &p(&["claude", "-c"], Some(700))).as_deref(), Some("s-commodore"), "by its session file");
        assert_eq!(super::session_of_unclaimed(ch, &p(&["vim", "--resume", "s-held"], None)), None);
        assert_eq!(super::session_of_foreground(ch, &p(&["zsh"], Some(1))), None);
        // a stale session file of a reused pid: the program is not claude
        assert_eq!(super::session_of_foreground(ch, &p(&["vim"], Some(700))), None);
        assert_eq!(super::session_of_foreground(ch, &p(&["/home/u/.local/share/claude/versions/2.1.292"], Some(700))).as_deref(), Some("s-commodore"));
        assert_eq!(super::session_of_foreground(ch, &p(&["node", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"], Some(700))).as_deref(), Some("s-commodore"), "an npm install");
    }
}
