//! Aide inboxes: a durable FIFO per aide, owned by nimitz
//! (docs/design/aide.md section 4). One file per item under
//! `NIMITZ_STATE/queues/<aide id, / as %2F>/{pending,claimed,done,dead}/`;
//! every operation runs under `queue.lock` (a dedicated lock file) and every
//! write is a temp file plus a rename, so a crash, a reboot, a `/clear` or a
//! relaunch of the aide loses nothing.
//!
//! - post: validate the type (an allowlist with versioned schemas), the size,
//!   the depth cap and the sender's rate; take the next sequence number.
//! - claim: expire stale leases first, then the oldest pending item gets a
//!   lease and `tries += 1`; one claim at a time per aide.
//! - done / fail: done stores the result; fail returns the item to pending,
//!   or dead-letters it at `max_tries` (or `--permanent`).
//! - request and reply (section 4.6): a type with `reply: true` records who
//!   asked and a reply token (only its sha256 is kept); the answer is stored
//!   with the item and only the asker (same member, same session or job, the
//!   token) or the user may read it.
//!
//! A queue's clock is the caller's `now`, so tests drive time.
use crate::env::Env;
use crate::types::{Id, InboxDef};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------ ids

/// An aide id: a parent's id plus `@NAME` (`HOME/@KB`). The parent's parts
/// and the name are upper-case names (`Id::well_formed`); `@` is in no
/// other id, so no member id reads as an aide id.
pub fn aide_id(s: &str) -> Result<Id, String> {
    let id = Id(s.to_string());
    if id.role() != crate::types::Role::Aide {
        return Err(format!("{s:?} is not an aide id (F/@NAME)"));
    }
    if !id.well_formed() {
        return Err(format!("{s:?} is not an aide id (F/@NAME, upper-case names)"));
    }
    Ok(id)
}

/// The aide's parent (`HOME` for `HOME/@KB`).
pub fn parent_of(aide: &Id) -> Id {
    Id(aide.0.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default())
}

/// An item id as given (`17`, `000000000017`) in its one form: twelve
/// digits. Anything else is refused (never a path).
pub fn item_id(s: &str) -> Result<String, String> {
    s.trim().parse::<u64>().map(|n| format!("{n:012}")).map_err(|_| format!("{s:?} is not an item id (a number, e.g. 17)"))
}

/// `HOME/@KB` -> `HOME%2F@KB`
pub fn escape(id: &Id) -> String {
    id.0.replace('/', "%2F")
}

/// The aide's config (`[[aide]]` in its fleet's `fleet.toml`). Only an
/// admiral's aide exists for now.
pub fn find(env: &Env, aide: &Id) -> Result<(crate::types::FleetOnDisk, crate::types::AideDef), String> {
    let parent = parent_of(aide);
    if parent.parts().len() != 1 {
        return Err(format!("{aide}: only an admiral's aide exists for now (F/@NAME)"));
    }
    let f = crate::config::load_fleet(env, parent.fleet()).map_err(|i| i.message)?;
    let name = aide.0.rsplit('@').next().unwrap_or_default();
    let def = crate::config::find_aide(&f, aide).cloned().ok_or_else(|| format!("no aide {aide} in fleet {} ([[aide]] name = {name:?})", parent.fleet()))?;
    Ok((f, def))
}

// ------------------------------------------------------------------ types

/// One item type: its name (versioned), whether it is a request, its size
/// cap and its validator.
pub struct TypeDef {
    pub name: &'static str,
    pub reply: bool,
    pub max_bytes: usize,
    pub validate: fn(&Value) -> Result<(), String>,
}

/// The item types nimitz knows. An aide's `types` picks from these.
pub const TYPES: &[TypeDef] = &[
    TypeDef { name: "wiki.note.v1", reply: false, max_bytes: 72 * 1024, validate: note_v1 },
    TypeDef { name: "wiki.query.v1", reply: true, max_bytes: 8 * 1024, validate: query_v1 },
];

pub fn type_def(name: &str) -> Option<&'static TypeDef> {
    TYPES.iter().find(|t| t.name == name)
}

/// The most a stored result may hold.
pub const MAX_RESULT: usize = 40 * 1024;

fn obj<'a>(v: &'a Value, known: &[&str]) -> Result<&'a serde_json::Map<String, Value>, String> {
    let o = v.as_object().ok_or("the payload is not a JSON object")?;
    if let Some(k) = o.keys().find(|k| !known.contains(&k.as_str())) {
        return Err(format!("unknown field {k:?} (known: {})", known.join(", ")));
    }
    Ok(o)
}

fn text(o: &serde_json::Map<String, Value>, k: &str, need: bool, max: usize) -> Result<(), String> {
    match o.get(k) {
        None if need => Err(format!("{k} is required")),
        None => Ok(()),
        Some(Value::String(s)) if need && s.trim().is_empty() => Err(format!("{k} is empty")),
        Some(Value::String(s)) if s.chars().count() > max => Err(format!("{k} is longer than {max} characters")),
        Some(Value::String(_)) => Ok(()),
        Some(_) => Err(format!("{k} is not a string")),
    }
}

fn one_of(o: &serde_json::Map<String, Value>, k: &str, allowed: &[&str]) -> Result<(), String> {
    match o.get(k) {
        None => Ok(()),
        Some(Value::String(s)) if allowed.contains(&s.as_str()) => Ok(()),
        Some(_) => Err(format!("{k} is one of {}", allowed.join(" | "))),
    }
}

fn slug(o: &serde_json::Map<String, Value>, k: &str) -> Result<(), String> {
    match o.get(k) {
        None => Ok(()),
        Some(Value::String(s)) if !s.is_empty() && s.len() <= 40 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') => Ok(()),
        Some(_) => Err(format!("{k} is a slug ([a-z0-9-]+, at most 40)")),
    }
}

fn strings(o: &serde_json::Map<String, Value>, k: &str, max: usize) -> Result<(), String> {
    match o.get(k) {
        None => Ok(()),
        Some(Value::Array(a)) if a.len() <= max && a.iter().all(|x| x.as_str().is_some_and(|s| s.len() <= 2000)) => Ok(()),
        Some(_) => Err(format!("{k} is a list of at most {max} strings")),
    }
}

/// `wiki.note.v1` (aide.md 4.4): a note for the notes keeper.
fn note_v1(v: &Value) -> Result<(), String> {
    let o = obj(v, &["title", "body", "kind", "domain", "tags", "links", "source"])?;
    text(o, "title", false, 200)?;
    text(o, "body", true, 64 * 1024)?;
    one_of(o, "kind", &["decision", "fact", "howto", "gotcha", "question", "preference", "gap"])?;
    slug(o, "domain")?;
    strings(o, "tags", 20)?;
    strings(o, "links", 50)?;
    match o.get("source") {
        None => Ok(()),
        Some(s) => text(obj(s, &["label"])?, "label", false, 500),
    }
}

/// `wiki.query.v1` (aide.md 4.4): a question for the reference desk.
fn query_v1(v: &Value) -> Result<(), String> {
    let o = obj(v, &["question", "scope", "length"])?;
    text(o, "question", true, 2000)?;
    text(o, "scope", false, 200)?;
    one_of(o, "length", &["short", "medium", "long"])
}

// ------------------------------------------------------------------ limits

#[derive(Clone, Debug, PartialEq)]
pub enum Policy {
    Open,
    Fleet,
    Parent,
}

/// An aide's limits, from its `[aide.inbox]`.
#[derive(Clone, Debug)]
pub struct Limits {
    pub types: Vec<String>,
    pub policy: Policy,
    pub depth_cap: usize,
    /// (posts, window seconds)
    pub per_sender: (u32, f64),
    pub lease_s: f64,
    pub max_tries: u32,
    pub keep_done_s: f64,
}

/// `30s`, `15m`, `2h`, `7d` -> seconds.
pub fn duration(s: &str) -> Result<f64, String> {
    let s = s.trim();
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: f64 = n.parse().map_err(|_| format!("{s:?} is not a duration (30s, 15m, 2h, 7d)"))?;
    let k = match unit {
        "s" | "" => 1.0,
        "m" => 60.0,
        "h" => 3600.0,
        "d" => 86400.0,
        _ => return Err(format!("{s:?} is not a duration (30s, 15m, 2h, 7d)")),
    };
    Ok(n * k)
}

impl Limits {
    pub fn from_def(d: &InboxDef) -> Result<Limits, String> {
        for t in &d.types {
            type_def(t).ok_or_else(|| format!("unknown inbox type {t:?} (known: {})", TYPES.iter().map(|t| t.name).collect::<Vec<_>>().join(", ")))?;
        }
        let policy = match d.policy.as_str() {
            "open" => Policy::Open,
            "fleet" => Policy::Fleet,
            "parent" => Policy::Parent,
            p => return Err(format!("inbox policy {p:?}: open, fleet or parent")),
        };
        let (n, w) = d.per_sender.split_once('/').ok_or_else(|| format!("per_sender {:?}: posts/window, e.g. 60/h", d.per_sender))?;
        let n: u32 = n.trim().parse().map_err(|_| format!("per_sender {:?}: posts/window, e.g. 60/h", d.per_sender))?;
        let w = w.trim();
        // `60/h` is one hour; `60/15m` fifteen minutes
        let w = if w.starts_with(|c: char| c.is_ascii_digit()) { duration(w)? } else { duration(&format!("1{w}"))? };
        Ok(Limits { types: d.types.clone(), policy, depth_cap: d.depth_cap, per_sender: (n, w), lease_s: duration(&d.lease)?, max_tries: d.max_tries.max(1), keep_done_s: duration(&d.keep_done)? })
    }
}

// ------------------------------------------------------------------ items

/// Who posted (aide.md 4.2). Provenance is built from this, never from the payload.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sender {
    /// member | session | process | user
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// `--from` text (a process), or the session's name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub verified: bool,
}

impl Sender {
    /// The rate-limit key: one per member, session, process label, or the user.
    pub fn key(&self) -> String {
        match self.kind.as_str() {
            "member" => format!("member:{}", self.id.as_deref().unwrap_or("?")),
            "session" => format!("session:{}", self.session.as_deref().unwrap_or("?")),
            "process" => format!("process:{}", self.label.as_deref().unwrap_or("?")),
            _ => "user".into(),
        }
    }
    /// One line: `HOME/GITHUB/NIMITZ_DEV`, `session 9aacf927`, `process "x"`, `the user`.
    pub fn line(&self) -> String {
        match self.kind.as_str() {
            "member" => self.id.clone().unwrap_or_default(),
            "session" => format!("session {}", self.session.as_deref().map(|s| &s[..s.len().min(8)]).unwrap_or("?")),
            "process" => format!("process {:?}{}", self.label.as_deref().unwrap_or(""), if self.verified { "" } else { " (unverified)" }),
            _ => "the user".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub by: String,
    pub at: f64,
    pub until: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub at: f64,
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A request's reply: the token's hash (the token itself is printed once at
/// post) and whether the answer was delivered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplyTo {
    pub token_sha256: String,
    /// `nimitz ask --wait` returns the answer itself until then: no peer message
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_until: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
}

/// A queue's `queue.json`: its format, its own id (`uid`), its aide.
pub fn meta(uid: &str, aide: &Id) -> Value {
    serde_json::json!({"format": crate::formats::current("queues"), "uid": uid, "aide": aide.0})
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    /// `<queue uid>:<id>`: unique across a reset queue (a lost `seq` or a
    /// wiped folder restarts the ids; the uid changes with it). What an aide
    /// records in its work (`@KB`'s commit trailer `inbox-item: <ref>`).
    #[serde(default, rename = "ref")]
    pub reference: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub posted_at: f64,
    pub sender: Sender,
    pub payload: Value,
    #[serde(default)]
    pub tries: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<Claim>,
    #[serde(default)]
    pub history: Vec<Event>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<ReplyTo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Item {
    fn note(&mut self, at: f64, event: &str, note: Option<String>) {
        self.history.push(Event { at, event: event.into(), note });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Pending,
    Claimed,
    Done,
    Dead,
}

impl State {
    pub const ALL: [State; 4] = [State::Pending, State::Claimed, State::Done, State::Dead];
    pub fn dir(self) -> &'static str {
        match self {
            State::Pending => "pending",
            State::Claimed => "claimed",
            State::Done => "done",
            State::Dead => "dead",
        }
    }
    pub fn parse(s: &str) -> Option<State> {
        State::ALL.into_iter().find(|x| x.dir() == s)
    }
    /// When an item is found in two folders (a crash between the write of the
    /// new one and the removal of the old), the later state wins.
    fn rank(self) -> u8 {
        match self {
            State::Pending => 0,
            State::Claimed => 1,
            State::Done | State::Dead => 2,
        }
    }
}

/// Why an operation was refused: the code, the message, the exit code
/// (aide.md 4.3: 3 empty, 4 backpressure, 5 not done yet, 6 dead).
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
    pub exit: i32,
}

fn no(code: &'static str, exit: i32, message: impl Into<String>) -> Refusal {
    Refusal { code, message: message.into(), exit }
}

/// What `post` made: the id, the reply token (a request only; printed once),
/// whether the key matched an item already there.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Posted {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    pub duplicate: bool,
}

// ------------------------------------------------------------------ status

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClaimedRow {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub by: String,
    pub until: f64,
    pub tries: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeadRow {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub sender: String,
    pub tries: u32,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SenderRow {
    pub sender: String,
    pub used: u32,
    pub limit: u32,
    pub window_s: f64,
    /// seconds until its oldest post leaves the window
    pub frees_in_s: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecentRow {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub state: String,
    pub sender: String,
    pub age_s: f64,
    /// a reply type's result (the user's view only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

/// `inbox status --json`: what Admiralty's aide row and `status` read.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub aide: String,
    pub pending: usize,
    pub claimed: Vec<ClaimedRow>,
    /// done within `keep_done`
    pub done: usize,
    pub dead: Vec<DeadRow>,
    /// the oldest pending item's age
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_age_s: Option<f64>,
    /// the earliest lease end of a claimed item
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_until: Option<f64>,
    /// files that did not read as items, set aside under `quarantine/`
    #[serde(default)]
    pub quarantined: usize,
    /// files in pending or claimed that do not read as items, not set aside
    /// yet (a peek moves nothing; the next command that housekeeps does)
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unreadable: usize,
    /// how long done items (and their results) stay: the inbox's `keep_done`
    #[serde(default)]
    pub keep_done_s: f64,
    /// this aide's queue cannot be read (its config): the row says why
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// reply results not delivered (the asker was gone)
    pub undelivered: usize,
    pub depth_cap: usize,
    pub senders: Vec<SenderRow>,
    /// the newest few items, newest first
    pub recent: Vec<RecentRow>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Status {
    /// `3 pending · 1 claimed · oldest 4m · 1 dead` (what status and Admiralty show).
    pub fn line(&self) -> String {
        if let Some(e) = &self.error {
            return format!("cannot read its queue: {e}");
        }
        let mut parts = vec![];
        if self.pending == 0 && self.claimed.is_empty() {
            parts.push("empty".to_string());
        } else {
            parts.push(format!("{} pending", self.pending));
        }
        if !self.claimed.is_empty() {
            parts.push(format!("{} claimed", self.claimed.len()));
        }
        if let Some(o) = self.oldest_age_s {
            parts.push(format!("oldest {}", human(o)));
        }
        if !self.dead.is_empty() {
            parts.push(format!("{} dead", self.dead.len()));
        }
        if self.quarantined > 0 {
            parts.push(format!("{} unreadable (quarantine/)", self.quarantined));
        }
        if self.unreadable > 0 {
            parts.push(format!("{} unreadable", self.unreadable));
        }
        if self.undelivered > 0 {
            parts.push(format!("{} answer{} to pick up", self.undelivered, if self.undelivered == 1 { "" } else { "s" }));
        }
        parts.join(" · ")
    }
}

// ------------------------------------------------------------------ the queue

pub struct Queue {
    pub aide: Id,
    pub dir: PathBuf,
    pub limits: Limits,
}

/// How many items `recent` lists.
const RECENT: usize = 8;

impl Queue {
    pub fn open(env: &Env, aide: &Id, limits: Limits) -> Queue {
        Queue { aide: aide.clone(), dir: env.state.join("queues").join(escape(aide)), limits }
    }

    /// The queue of a configured aide.
    pub fn of(env: &Env, aide: &Id) -> Result<Queue, String> {
        let (_, def) = find(env, aide)?;
        Ok(Queue::open(env, aide, Limits::from_def(&def.inbox)?))
    }

    fn lock(&self) -> Result<std::fs::File, Refusal> {
        let io = |e: std::io::Error| no("inbox.io", 1, format!("{}: {e}", self.dir.display()));
        for s in State::ALL {
            std::fs::create_dir_all(self.dir.join(s.dir())).map_err(io)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
        }
        let mut o = std::fs::OpenOptions::new();
        o.create(true).truncate(false).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let f = o.open(self.dir.join("queue.lock")).map_err(io)?;
        f.lock().map_err(io)?;
        Ok(f)
    }

    fn path(&self, s: State, id: &str) -> PathBuf {
        self.dir.join(s.dir()).join(format!("{id}.json"))
    }

    fn read(p: &Path) -> Option<Item> {
        std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }

    fn write(&self, s: State, item: &Item) -> Result<(), Refusal> {
        let p = self.path(s, &item.id);
        let text = serde_json::to_string_pretty(item).map_err(|e| no("inbox.io", 1, e.to_string()))?;
        write_private(&p, &text).map_err(|e| no("inbox.io", 1, format!("{}: {e}", p.display())))
    }

    /// Write `item` in `to`, then remove it from `from` (a crash in between
    /// leaves both; the later state wins on read).
    fn shift(&self, from: State, to: State, item: &Item) -> Result<(), Refusal> {
        self.write(to, item)?;
        match std::fs::remove_file(self.path(from, &item.id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(no("inbox.io", 1, format!("item {} is now {} but its {} copy stays: {e}", item.id, to.dir(), from.dir()))),
            _ => Ok(()),
        }
    }

    fn ids(&self, s: State) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(self.dir.join(s.dir()))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".json")).map(str::to_string))
            .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
            .collect();
        out.sort();
        out
    }

    /// Every item, each once (in its latest state), oldest first.
    fn all(&self) -> Vec<(State, Item)> {
        let mut by: std::collections::BTreeMap<String, (State, Item)> = Default::default();
        for s in State::ALL {
            for id in self.ids(s) {
                let Some(item) = Self::read(&self.path(s, &id)) else { continue };
                match by.get(&id) {
                    Some((had, _)) if had.rank() >= s.rank() => {}
                    _ => {
                        by.insert(id, (s, item));
                    }
                }
            }
        }
        by.into_values().collect()
    }

    /// Under the lock: drop a copy left in an earlier state, return expired
    /// leases to pending (or dead at `max_tries`), prune old done items.
    fn housekeep(&self, now: f64) -> Result<(), Refusal> {
        // a file that does not read as an item would block the queue (a
        // claimed one is "busy" for ever, a pending head fails every claim):
        // set aside, named so none overwrites another. Only these two states
        // (a done or dead file blocks nothing, and they are the many).
        for s in [State::Claimed, State::Pending] {
            for id in self.ids(s) {
                if Self::read(&self.path(s, &id)).is_none() {
                    let q = self.dir.join("quarantine");
                    let _ = std::fs::create_dir_all(&q);
                    let _ = std::fs::rename(self.path(s, &id), q.join(format!("{}-{id}-{}.json", s.dir(), (now * 1000.0) as u64)));
                }
            }
        }
        for s in State::ALL {
            for id in self.ids(s) {
                let later = State::ALL.into_iter().any(|t| t.rank() > s.rank() && self.path(t, &id).exists());
                if later {
                    let _ = std::fs::remove_file(self.path(s, &id));
                }
            }
        }
        for id in self.ids(State::Claimed) {
            let Some(mut item) = Self::read(&self.path(State::Claimed, &id)) else { continue };
            if item.claim.as_ref().is_some_and(|c| c.until > now) {
                continue;
            }
            item.claim = None;
            if item.tries >= self.limits.max_tries {
                item.reason = Some(format!("its lease ran out {} times (the aide stopped or crashed on it)", item.tries));
                item.finished_at = Some(now);
                item.note(now, "dead", item.reason.clone());
                self.shift(State::Claimed, State::Dead, &item)?;
            } else {
                item.note(now, "expired", None);
                self.shift(State::Claimed, State::Pending, &item)?;
            }
        }
        for id in self.ids(State::Done) {
            let Some(item) = Self::read(&self.path(State::Done, &id)) else { continue };
            if item.finished_at.unwrap_or(item.posted_at) + self.limits.keep_done_s < now {
                let _ = std::fs::remove_file(self.path(State::Done, &id));
            }
        }
        Ok(())
    }

    fn next_id(&self) -> Result<String, Refusal> {
        let p = self.dir.join("seq");
        let n: u64 = std::fs::read_to_string(&p).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(1);
        // never below an id on disk (a lost seq file must not reuse one)
        let dropped = self.dir.join("dead").join("dropped");
        let dropped_ids = std::fs::read_dir(&dropped).into_iter().flatten().flatten().filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".json")).and_then(|n| n.parse::<u64>().ok()));
        let top = State::ALL.into_iter().flat_map(|s| self.ids(s)).filter_map(|i| i.parse::<u64>().ok()).chain(dropped_ids).max().unwrap_or(0);
        let n = n.max(top + 1);
        write_private(&p, &format!("{}\n", n + 1)).map_err(|e| no("inbox.io", 1, format!("{}: {e}", p.display())))?;
        Ok(format!("{n:012}"))
    }

    /// The queue's own id, made once (`queue.json`): an item's `ref` is
    /// `<uid>:<id>`, so a queue reset (ids restart) never repeats a ref.
    fn uid(&self) -> Result<String, Refusal> {
        let p = self.dir.join("queue.json");
        if let Some(u) = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v.get("uid").and_then(Value::as_str).map(str::to_string)) {
            return Ok(u);
        }
        let u = token().map_err(|e| no("inbox.io", 1, e))?[..8].to_string();
        write_private(&p, &meta(&u, &self.aide).to_string()).map_err(|e| no("inbox.io", 1, e.to_string()))?;
        Ok(u)
    }

    fn senders_path(&self) -> PathBuf {
        self.dir.join("senders.json")
    }

    fn senders(&self, now: f64) -> std::collections::BTreeMap<String, Vec<f64>> {
        let mut m: std::collections::BTreeMap<String, Vec<f64>> = std::fs::read(self.senders_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let w = self.limits.per_sender.1;
        for v in m.values_mut() {
            v.retain(|t| *t > now - w);
        }
        m.retain(|_, v| !v.is_empty());
        m
    }

    /// Queue an item. Every sender is rate-limited (aide.md 4.5).
    /// `wait_until`: the asker waits for the answer itself (`ask --wait`)
    /// until then, so no peer message duplicates it.
    pub fn post(&self, sender: &Sender, kind: &str, payload: &Value, key: Option<&str>, wait_until: Option<f64>, now: f64) -> Result<Posted, Refusal> {
        let t = type_def(kind).ok_or_else(|| no("inbox.type", 4, format!("unknown type {kind:?} (known: {})", TYPES.iter().map(|t| t.name).collect::<Vec<_>>().join(", "))))?;
        if !self.limits.types.iter().any(|x| x == kind) {
            return Err(no("inbox.type", 4, format!("{} takes {}, not {kind}", self.aide, self.limits.types.join(", "))));
        }
        let size = serde_json::to_vec(payload).map(|b| b.len()).unwrap_or(usize::MAX);
        if size > t.max_bytes {
            return Err(no("inbox.size", 4, format!("the payload is {size} bytes; {kind} takes at most {}", t.max_bytes)));
        }
        (t.validate)(payload).map_err(|e| no("inbox.schema", 4, format!("{kind}: {e}")))?;
        let _l = self.lock()?;
        self.housekeep(now)?;
        if let Some(k) = key {
            if let Some((_, it)) = self.all().into_iter().find(|(s, it)| *s != State::Dead && it.key.as_deref() == Some(k)) {
                return Ok(Posted { id: it.id, token: None, duplicate: true });
            }
        }
        let depth = self.ids(State::Pending).len() + self.ids(State::Claimed).len();
        if depth >= self.limits.depth_cap {
            return Err(no("inbox.full", 4, format!("{} holds {depth} items (its cap); try later", self.aide)));
        }
        let mut senders = self.senders(now);
        let (limit, window) = self.limits.per_sender;
        let mine = senders.entry(sender.key()).or_default();
        if mine.len() as u32 >= limit {
            let free = mine.iter().cloned().fold(f64::MAX, f64::min) + window - now;
            return Err(no("inbox.rate", 4, format!("{} posted {limit} items in the last {}; the next fits in {}", sender.line(), human(window), human(free.max(1.0)))));
        }
        mine.push(now);
        let uid = self.uid()?;
        let id = self.next_id()?;
        let token = if t.reply { Some(token().map_err(|e| no("inbox.io", 1, e))?) } else { None };
        // the sender's count first: a failed post must not slip past the rate
        let text = serde_json::to_string(&senders).unwrap_or_else(|_| "{}".into());
        write_private(&self.senders_path(), &text).map_err(|e| no("inbox.io", 1, e.to_string()))?;
        let item = Item {
            reference: format!("{uid}:{id}"),
            id: id.clone(),
            kind: kind.into(),
            key: key.map(str::to_string),
            posted_at: now,
            sender: sender.clone(),
            payload: payload.clone(),
            tries: 0,
            claim: None,
            history: vec![Event { at: now, event: "posted".into(), note: None }],
            result: None,
            reply_to: token.as_ref().map(|t| ReplyTo { token_sha256: crate::sha256::hex(t.as_bytes()), waiting_until: wait_until, delivered_at: None, delivery: None }),
            finished_at: None,
            reason: None,
        };
        self.write(State::Pending, &item)?;
        Ok(Posted { id, token, duplicate: false })
    }

    /// The oldest pending item, leased to `by`; None when the queue is empty.
    /// One claim at a time: a second while one is held is refused.
    pub fn claim(&self, by: &str, now: f64) -> Result<Option<Item>, Refusal> {
        let _l = self.lock()?;
        self.housekeep(now)?;
        if let Some(id) = self.ids(State::Claimed).first() {
            return Err(no("inbox.busy", 1, format!("item {id} is still claimed; finish it (done or fail) first")));
        }
        let Some(id) = self.ids(State::Pending).into_iter().next() else { return Ok(None) };
        let mut item = Self::read(&self.path(State::Pending, &id)).ok_or_else(|| no("inbox.io", 1, format!("item {id} cannot be read")))?;
        item.tries += 1;
        item.claim = Some(Claim { by: by.into(), at: now, until: now + self.limits.lease_s });
        item.note(now, "claimed", Some(format!("by {by}, try {}", item.tries)));
        self.shift(State::Pending, State::Claimed, &item)?;
        Ok(Some(item))
    }

    /// The claimed item `id`, held by `by` (None: the user, who may finish
    /// any): a stale session cannot finish an item re-leased to another.
    fn claimed(&self, id: &str, by: Option<&str>, now: f64) -> Result<Item, Refusal> {
        self.housekeep(now)?;
        let item = Self::read(&self.path(State::Claimed, id)).ok_or_else(|| match self.get(id) {
            Some((s, _)) => no("inbox.state", 1, format!("item {id} is {}, not claimed (its lease may have run out)", s.dir())),
            None => no("inbox.unknown", 1, format!("no item {id} in {}", self.aide)),
        })?;
        if let (Some(b), Some(c)) = (by, item.claim.as_ref()) {
            if c.by != b {
                return Err(no("inbox.lease", 1, format!("item {id} is leased to {}, not {b} (the lease ran out and it was claimed again)", c.by)));
            }
        }
        Ok(item)
    }

    /// Extend the lease of a claimed item.
    pub fn heartbeat(&self, id: &str, by: Option<&str>, now: f64) -> Result<Item, Refusal> {
        let _l = self.lock()?;
        let mut item = self.claimed(id, by, now)?;
        if let Some(c) = item.claim.as_mut() {
            c.until = now + self.limits.lease_s;
        }
        self.write(State::Claimed, &item)?;
        Ok(item)
    }

    /// A claimed item is done, with its result (a reply type's answer).
    pub fn done(&self, id: &str, by: Option<&str>, result: Option<Value>, now: f64) -> Result<Item, Refusal> {
        if let Some(r) = &result {
            let n = serde_json::to_vec(r).map(|b| b.len()).unwrap_or(usize::MAX);
            if n > MAX_RESULT {
                return Err(no("inbox.size", 4, format!("the result is {n} bytes; at most {MAX_RESULT}")));
            }
        }
        let _l = self.lock()?;
        let mut item = self.claimed(id, by, now)?;
        if type_def(&item.kind).is_some_and(|t| t.reply) && result.is_none() {
            return Err(no("inbox.result", 2, format!("{} is a request: done needs --result (the answer)", item.kind)));
        }
        item.claim = None;
        item.result = result;
        item.finished_at = Some(now);
        item.note(now, "done", None);
        self.shift(State::Claimed, State::Done, &item)?;
        Ok(item)
    }

    /// A claimed item failed: back to pending, or dead at `max_tries` (or
    /// `permanent`). Returns the item and whether it is dead.
    pub fn fail(&self, id: &str, by: Option<&str>, reason: &str, permanent: bool, now: f64) -> Result<(Item, bool), Refusal> {
        let _l = self.lock()?;
        let mut item = self.claimed(id, by, now)?;
        item.claim = None;
        item.note(now, "failed", Some(reason.into()));
        let dead = permanent || item.tries >= self.limits.max_tries;
        if dead {
            item.reason = Some(reason.into());
            item.finished_at = Some(now);
            self.shift(State::Claimed, State::Dead, &item)?;
        } else {
            self.shift(State::Claimed, State::Pending, &item)?;
        }
        Ok((item, dead))
    }

    /// A dead item goes back to pending, its tries reset.
    pub fn retry(&self, id: &str, now: f64) -> Result<Item, Refusal> {
        let _l = self.lock()?;
        self.housekeep(now)?;
        let mut item = Self::read(&self.path(State::Dead, id)).ok_or_else(|| no("inbox.state", 1, format!("item {id} is not a dead letter")))?;
        item.tries = 0;
        item.reason = None;
        item.finished_at = None;
        item.note(now, "retried", None);
        self.shift(State::Dead, State::Pending, &item)?;
        Ok(item)
    }

    /// A dead or pending item is removed (kept under `dead/dropped/`).
    pub fn drop_item(&self, id: &str, now: f64) -> Result<Item, Refusal> {
        let _l = self.lock()?;
        self.housekeep(now)?;
        let (from, mut item) = [State::Dead, State::Pending]
            .into_iter()
            .find_map(|s| Self::read(&self.path(s, id)).map(|i| (s, i)))
            .ok_or_else(|| no("inbox.state", 1, format!("item {id} is neither dead nor pending")))?;
        item.note(now, "dropped", None);
        let kept = self.dir.join("dead").join("dropped").join(format!("{id}.json"));
        write_private(&kept, &serde_json::to_string_pretty(&item).unwrap_or_default()).map_err(|e| no("inbox.io", 1, e.to_string()))?;
        let _ = std::fs::remove_file(self.path(from, id));
        Ok(item)
    }

    /// An item in its latest state.
    pub fn get(&self, id: &str) -> Option<(State, Item)> {
        let mut best: Option<(State, Item)> = None;
        for s in State::ALL {
            if let Some(i) = Self::read(&self.path(s, id)) {
                if best.as_ref().is_none_or(|(b, _)| s.rank() > b.rank()) {
                    best = Some((s, i));
                }
            }
        }
        best
    }

    /// Every item, oldest first (optionally one state).
    pub fn list(&self, state: Option<State>) -> Vec<(State, Item)> {
        self.all().into_iter().filter(|(s, _)| state.is_none_or(|x| x == *s)).collect()
    }

    /// Record a reply's delivery (or why it was not delivered).
    pub fn mark_delivered(&self, id: &str, note: &str, delivered: bool, now: f64) -> Result<(), Refusal> {
        let _l = self.lock()?;
        let Some(mut item) = Self::read(&self.path(State::Done, id)) else { return Ok(()) };
        if let Some(r) = item.reply_to.as_mut() {
            if delivered {
                r.delivered_at = Some(now);
            }
            r.delivery = Some(note.into());
        }
        item.note(now, if delivered { "delivered" } else { "undelivered" }, Some(note.into()));
        self.write(State::Done, &item)
    }

    /// The summary Admiralty and `inbox status` read. `results`: include
    /// reply results in `recent` (the user's view only).
    pub fn status(&self, now: f64, results: bool) -> Status {
        // housekeeping under the lock (it waits for a post or claim in progress)
        if let Ok(_l) = self.lock() {
            let _ = self.housekeep(now);
        }
        self.summarize(self.all(), now, results)
    }

    /// `status` with no lock and no write: what housekeeping would do (an
    /// expired lease back to pending, or dead at `max_tries`; a done item
    /// past `keep_done` gone) is done in memory only. What a snapshot reads:
    /// a look never changes state.
    pub fn peek_status(&self, now: f64, results: bool) -> Status {
        let all = self
            .all()
            .into_iter()
            .filter_map(|(s, mut i)| match s {
                State::Claimed if !i.claim.as_ref().is_some_and(|c| c.until > now) => {
                    i.claim = None;
                    if i.tries >= self.limits.max_tries {
                        i.reason = Some(format!("its lease ran out {} times (the aide stopped or crashed on it)", i.tries));
                        Some((State::Dead, i))
                    } else {
                        Some((State::Pending, i))
                    }
                }
                State::Done if i.finished_at.unwrap_or(i.posted_at) + self.limits.keep_done_s < now => None,
                _ => Some((s, i)),
            })
            .collect();
        self.summarize(all, now, results)
    }

    fn summarize(&self, all: Vec<(State, Item)>, now: f64, results: bool) -> Status {
        // (a file in pending or claimed that does not read as an item is set
        // aside by housekeeping; a peek counts it where it lies, if it is
        // still there: one a command just moved on is no unreadable file)
        let unread = [State::Pending, State::Claimed]
            .into_iter()
            .flat_map(|s| self.ids(s).into_iter().map(move |id| (s, id)))
            .filter(|(s, id)| {
                let p = self.path(*s, id);
                Self::read(&p).is_none() && p.is_file()
            })
            .count();
        let of = |st: State| all.iter().filter(move |(s, _)| *s == st).map(|(_, i)| i);
        let (limit, window) = self.limits.per_sender;
        let senders = self
            .senders(now)
            .into_iter()
            .map(|(k, v)| SenderRow { sender: k, used: v.len() as u32, limit, window_s: window, frees_in_s: (v.iter().cloned().fold(f64::MAX, f64::min) + window - now).max(0.0) })
            .collect();
        let mut recent: Vec<&(State, Item)> = all.iter().collect();
        recent.sort_by(|a, b| b.1.id.cmp(&a.1.id));
        Status {
            aide: self.aide.0.clone(),
            pending: of(State::Pending).count(),
            claimed: of(State::Claimed)
                .map(|i| ClaimedRow { id: i.id.clone(), kind: i.kind.clone(), by: i.claim.as_ref().map(|c| c.by.clone()).unwrap_or_default(), until: i.claim.as_ref().map(|c| c.until).unwrap_or(0.0), tries: i.tries })
                .collect(),
            done: of(State::Done).count(),
            dead: of(State::Dead).map(|i| DeadRow { id: i.id.clone(), kind: i.kind.clone(), sender: i.sender.line(), tries: i.tries, reason: i.reason.clone().unwrap_or_default() }).collect(),
            lease_until: of(State::Claimed).filter_map(|i| i.claim.as_ref().map(|c| c.until)).fold(None, |a: Option<f64>, x| Some(a.map_or(x, |a| a.min(x)))),
            quarantined: std::fs::read_dir(self.dir.join("quarantine")).map(|d| d.count()).unwrap_or(0),
            unreadable: unread,
            keep_done_s: self.limits.keep_done_s,
            error: None,
            oldest_age_s: of(State::Pending).map(|i| now - i.posted_at).fold(None, |a: Option<f64>, x| Some(a.map_or(x, |a| a.max(x)))),
            undelivered: of(State::Done).filter(|i| i.reply_to.as_ref().is_some_and(|r| r.delivered_at.is_none())).count(),
            depth_cap: self.limits.depth_cap,
            senders,
            recent: recent
                .into_iter()
                .take(RECENT)
                .map(|(s, i)| RecentRow {
                    id: i.id.clone(),
                    kind: i.kind.clone(),
                    state: s.dir().into(),
                    sender: i.sender.line(),
                    age_s: now - i.posted_at,
                    result: if results && type_def(&i.kind).is_some_and(|t| t.reply) { i.result.clone() } else { None },
                })
                .collect(),
        }
    }
}

/// May `who` read item `item`'s result? The same member, the same session or
/// job, a holder of the reply token, or the user (aide.md 4.6).
pub fn may_read(item: &Item, who: &Sender, token: Option<&str>) -> bool {
    if let (Some(t), Some(r)) = (token, item.reply_to.as_ref()) {
        if crate::sha256::hex(t.as_bytes()) == r.token_sha256 {
            return true;
        }
    }
    let s = &item.sender;
    match who.kind.as_str() {
        "user" => true,
        "member" => (s.kind == "member" && s.id.is_some() && s.id == who.id) || (who.session.is_some() && s.session == who.session),
        "session" => (who.session.is_some() && s.session == who.session) || (who.job.is_some() && s.job == who.job),
        _ => false,
    }
}

/// 128 random bits as hex (`os::random`). No random source: an error,
/// never a guessable token.
fn token() -> Result<String, String> {
    let mut b = [0u8; 16];
    crate::os::random(&mut b).map_err(|e| format!("no random source for a reply token: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// `90` -> `1m30s`, `7200` -> `2h`.
pub fn human(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => if s % 60 == 0 { format!("{}m", s / 60) } else { format!("{}m{:02}s", s / 60, s % 60) },
        3600..=86399 => if s % 3600 == 0 { format!("{}h", s / 3600) } else { format!("{}h{:02}m", s / 3600, s % 3600 / 60) },
        _ => format!("{}d", s / 86400),
    }
}

/// Temp file in the same folder (0600), synced, then renamed.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.tmp-{}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    drop(f);
    crate::os::replace(&tmp, path)
}

// ------------------------------------------------------------------ delivery

/// The live session that asked (by its session id, else its job's current
/// session): (session id, pid).
fn live_session(env: &Env, s: &Sender) -> Option<(String, i64)> {
    for e in std::fs::read_dir(env.claude_home.join("sessions")).into_iter().flatten().flatten() {
        let Some(v) = std::fs::read(e.path()).ok().and_then(|b| crate::evidence::first_json(&b)) else { continue };
        let sid = v.get("sessionId").and_then(Value::as_str).unwrap_or_default();
        let job = v.get("jobId").and_then(Value::as_str);
        let pid = v.get("pid").and_then(Value::as_i64).unwrap_or(0);
        let same = s.session.as_deref() == Some(sid) || (s.job.is_some() && job == s.job.as_deref());
        if same && pid > 1 && crate::evidence::pid_alive(pid) {
            return Some((sid.to_string(), pid));
        }
    }
    None
}

/// How much of an answer the reply message carries inline.
pub const INLINE_REPLY: usize = 8 * 1024;

/// Deliver a done request's answer to the one who asked, once: a peer
/// message through its own messaging socket while it is live (aide.md 4.6).
/// The aide never addresses the requester; nimitz does, here.
pub fn deliver(env: &Env, q: &Queue, item: &Item, now: f64) -> Result<String, String> {
    if !type_def(&item.kind).is_some_and(|t| t.reply) || item.reply_to.as_ref().is_some_and(|r| r.delivered_at.is_some()) {
        return Ok("nothing to deliver".into());
    }
    if item.reply_to.as_ref().and_then(|r| r.waiting_until).is_some_and(|w| w > now) {
        return Ok("the asker waits for it (`nimitz ask --wait` returns it)".into());
    }
    let r = (|| {
        let (sid, pid) = live_session(env, &item.sender).ok_or("the asker is not live; the answer waits for `nimitz inbox result`")?;
        if !crate::inbox::format_ok(crate::inbox::receiver_version(env, Some(pid), item.sender.job.as_deref()).as_deref()) {
            return Err("the asker's Claude Code predates the message line; the answer waits for `nimitz inbox result`".to_string());
        }
        let socks = crate::inbox::candidates(env, &sid, pid);
        if socks.is_empty() {
            return Err("no messaging socket for the asker; the answer waits for `nimitz inbox result`".to_string());
        }
        let answer = item.result.as_ref().and_then(|v| v.get("answer").and_then(Value::as_str).map(str::to_string)).unwrap_or_else(|| item.result.as_ref().map(|v| v.to_string()).unwrap_or_default());
        let body = if answer.len() <= INLINE_REPLY {
            format!("[nimitz reply] {} answered your request {} ({}).\n\n{answer}", q.aide, item.id, item.kind)
        } else {
            format!("[nimitz reply] {} answered your request {} ({}); it is long: run `nimitz inbox result {} {}`.", q.aide, item.id, item.kind, q.aide, item.id)
        };
        crate::inbox::send(env, &socks, pid, &body).map(|s| format!("delivered to session {} through {}", &sid[..sid.len().min(8)], s.display()))
    })();
    match &r {
        Ok(note) => q.mark_delivered(&item.id, note, true, now),
        Err(why) => q.mark_delivered(&item.id, why, false, now),
    }
    .map_err(|e| e.message)?;
    r
}

// ------------------------------------------------------------------ who asks

/// Who is calling, for a post or a read: the labels (a member, an aide), else
/// a Claude Code session among the caller's ancestors (a non-member session:
/// the user's mod), else `--from` (an unverified process), else the user at a
/// terminal.
pub fn sender_of(env: &Env, vars: &std::collections::HashMap<String, String>, from: Option<&str>) -> Sender {
    let session = session_above(env, vars);
    let (sid, job, cwd) = match &session {
        Some(v) => (
            v.get("sessionId").and_then(Value::as_str).map(str::to_string),
            v.get("jobId").and_then(Value::as_str).map(str::to_string),
            v.get("cwd").and_then(Value::as_str).map(str::to_string),
        ),
        None => (None, None, None),
    };
    let get = |k: &str| vars.get(k).filter(|v| !v.is_empty()).cloned();
    if let (Some(id), Some(_)) = (get("NIMITZ_ID"), get("NIMITZ_ROLE")) {
        if matches!(crate::sessions::caller_from_env(vars), crate::types::Caller::Member { id: ref m, .. } if m.0 == id) {
            return Sender { kind: "member".into(), id: Some(id), session: sid, job, cwd, label: None, verified: true };
        }
        return Sender { kind: "process".into(), label: Some(format!("unverified labels {id}")), session: sid, job, cwd, ..Default::default() };
    }
    if sid.is_some() {
        let label = session.as_ref().and_then(|v| v.get("name").and_then(Value::as_str)).map(str::to_string);
        return Sender { kind: "session".into(), id: None, session: sid, job, cwd, label, verified: true };
    }
    match from {
        Some(f) => Sender { kind: "process".into(), label: Some(f.to_string()), verified: false, ..Default::default() },
        // a process cut off from whatever started it (`setsid`, `nohup ... &`:
        // re-parented to init or a subreaper) proves nothing: not the user
        None if detached() => Sender { kind: "process".into(), label: Some("detached".into()), verified: false, ..Default::default() },
        None => Sender { kind: "user".into(), verified: true, ..Default::default() },
    }
}

/// Re-parented to init or to a subreaper (`systemd --user`): the process
/// that started it is gone, so its ancestry says nothing (Linux; elsewhere
/// false).
fn detached() -> bool {
    let Some(parent) = crate::evidence::ancestors().into_iter().next() else { return false };
    parent == 1 || std::fs::read_to_string(format!("/proc/{parent}/comm")).is_ok_and(|c| c.trim() == "systemd")
}

/// The Claude Code session this process runs inside, when one is above it
/// (its session file, as `sender_of` reads the ancestry): its session id,
/// or `?` when the file names none.
pub fn session_above_id(env: &Env, vars: &std::collections::HashMap<String, String>) -> Option<String> {
    session_above(env, vars).map(|v| v.get("sessionId").and_then(Value::as_str).unwrap_or("?").to_string())
}

/// The session file of the nearest Claude Code process above this one (one
/// whose `procStart` names another process is a stale file: skipped).
fn session_above(env: &Env, vars: &std::collections::HashMap<String, String>) -> Option<Value> {
    // tests only (a debug build): the ancestry is given, a sandbox has no
    // Claude Code above it. A release build never reads it, so no caller
    // can name another session as its own.
    if cfg!(debug_assertions) {
        if let Some(pids) = vars.get("NIMITZ_TEST_ANCESTORS") {
            return pids.split(',').filter_map(|p| p.trim().parse::<i64>().ok()).find_map(|p| read_session(env, p));
        }
    }
    crate::evidence::ancestors().into_iter().find_map(|p| read_session(env, p))
}

fn read_session(env: &Env, pid: i64) -> Option<Value> {
    std::fs::read(env.claude_home.join("sessions").join(format!("{pid}.json"))).ok().and_then(|b| crate::evidence::first_json(&b)).filter(|d| crate::evidence::session_is_its_process(pid, d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::test_env;
    use serde_json::json;

    fn limits(types: &[&str]) -> Limits {
        let d = InboxDef { types: types.iter().map(|s| s.to_string()).collect(), per_sender: "3/m".into(), lease: "60s".into(), max_tries: 2, keep_done: "1h".into(), depth_cap: 4, ..Default::default() };
        Limits::from_def(&d).unwrap()
    }

    fn q(env: &Env) -> Queue {
        Queue::open(env, &aide_id("HOME/@KB").unwrap(), limits(&["wiki.note.v1", "wiki.query.v1"]))
    }

    fn member(id: &str) -> Sender {
        Sender { kind: "member".into(), id: Some(id.into()), verified: true, ..Default::default() }
    }

    fn note(body: &str) -> Value {
        json!({"body": body})
    }

    #[test]
    fn aide_ids_are_a_parent_and_an_at_name() {
        assert!(aide_id("HOME/@KB").is_ok());
        assert!(aide_id("HOME/GITHUB/@X").is_ok());
        for bad in ["HOME", "HOME/KB", "@KB", "HOME/@kb", "HOME/@", "home/@KB", "A/B/C/D/@X"] {
            assert!(aide_id(bad).is_err(), "{bad}");
        }
        assert_eq!(parent_of(&aide_id("HOME/@KB").unwrap()).0, "HOME");
        assert_eq!(escape(&aide_id("HOME/@KB").unwrap()), "HOME%2F@KB");
    }

    #[test]
    fn fifo_lease_done_and_the_order_survives_a_lost_seq_file() {
        let (_td, env) = test_env();
        let q = q(&env);
        let a = q.post(&member("HOME"), "wiki.note.v1", &note("a"), None, None, 100.0).unwrap();
        let b = q.post(&member("HOME/X"), "wiki.note.v1", &note("b"), None, None, 101.0).unwrap();
        assert!(a.id < b.id && a.token.is_none(), "a note is fire and forget");
        let got = q.claim("s1", 102.0).unwrap().unwrap();
        assert_eq!((got.id.as_str(), got.tries), (a.id.as_str(), 1));
        assert_eq!(q.claim("s1", 103.0).unwrap_err().code, "inbox.busy", "one claim at a time");
        q.done(&a.id, None, Some(json!({"commit": "abc"})), 104.0).unwrap();
        assert_eq!(q.get(&a.id).unwrap().0, State::Done);
        // the seq file is lost: ids never go backwards
        std::fs::remove_file(q.dir.join("seq")).unwrap();
        let c = q.post(&member("HOME"), "wiki.note.v1", &note("c"), None, None, 105.0).unwrap();
        assert!(c.id > b.id);
        assert_eq!(q.claim("s1", 106.0).unwrap().unwrap().id, b.id, "FIFO");
    }

    /// A peek (what a snapshot reads) sees what housekeeping would do, and
    /// writes nothing: the files stay where they are.
    #[test]
    fn a_peek_expires_leases_in_memory_only() {
        let (_td, env) = test_env();
        let q = q(&env);
        let a = q.post(&member("HOME"), "wiki.note.v1", &note("a"), None, None, 0.0).unwrap();
        q.post(&member("HOME"), "wiki.note.v1", &note("b"), None, None, 1.0).unwrap();
        q.claim("s1", 2.0).unwrap();
        let p = q.peek_status(10.0, false);
        assert_eq!((p.pending, p.claimed.len(), p.claimed[0].by.as_str()), (1, 1, "s1"));
        assert_eq!(p.line(), "1 pending · 1 claimed · oldest 9s");
        let snapshot = |q: &Queue| State::ALL.map(|s| q.ids(s));
        let before = snapshot(&q);
        // the lease ran out: back to pending in the peek, still claimed on disk
        let p = q.peek_status(100.0, false);
        assert_eq!((p.pending, p.claimed.len(), p.dead.len()), (2, 0, 0));
        assert_eq!(snapshot(&q), before, "a peek moves nothing");
        assert!(q.path(State::Claimed, &a.id).exists());
        // at max_tries it reads as dead; done items past keep_done are gone
        let mut i = q.get(&a.id).unwrap().1;
        i.tries = 2;
        q.write(State::Claimed, &i).unwrap();
        assert_eq!(q.peek_status(100.0, false).dead.len(), 1);
        std::fs::write(q.path(State::Pending, "000000000099"), "not json").unwrap();
        let p = q.peek_status(100.0, false);
        assert_eq!((p.unreadable, p.quarantined), (1, 0), "an unreadable file counts where it lies");
        assert!(p.line().ends_with("1 unreadable") && !p.line().contains("quarantine"), "{}", p.line());
        assert_eq!(p.keep_done_s, 3600.0, "the inbox's keep_done");
        assert!(q.path(State::Pending, "000000000099").exists());
        // `status` housekeeps for real (and sets the file aside)
        let st = q.status(100.0, false);
        assert_eq!((st.dead.len(), st.unreadable, st.quarantined), (1, 0, 1));
        assert!(st.line().contains("1 unreadable (quarantine/)"), "{}", st.line());
        assert!(!q.path(State::Claimed, &a.id).exists());
        assert_eq!(Status { error: Some("bad".into()), ..Default::default() }.line(), "cannot read its queue: bad");
        assert_eq!(Status::default().line(), "empty");
    }

    #[test]
    fn an_expired_lease_returns_the_item_then_dead_letters_it() {
        let (_td, env) = test_env();
        let q = q(&env);
        let a = q.post(&member("HOME"), "wiki.note.v1", &note("a"), None, None, 0.0).unwrap();
        q.claim("s1", 1.0).unwrap();
        // the aide crashed: 60 s later the lease is gone
        let again = q.claim("s2", 62.0).unwrap().unwrap();
        assert_eq!((again.id.as_str(), again.tries), (a.id.as_str(), 2));
        assert!(again.history.iter().any(|e| e.event == "expired"));
        // max_tries 2: the next expiry is a dead letter
        assert!(q.claim("s3", 124.0).unwrap().is_none());
        let (s, item) = q.get(&a.id).unwrap();
        assert_eq!(s, State::Dead);
        assert!(item.reason.unwrap().contains("lease ran out"));
        let st = q.status(125.0, false);
        assert_eq!((st.dead.len(), st.pending), (1, 0));
        // retry: back to pending, tries reset; drop: gone (kept aside)
        q.retry(&a.id, 126.0).unwrap();
        assert_eq!(q.get(&a.id).unwrap().1.tries, 0);
        q.drop_item(&a.id, 127.0).unwrap();
        assert!(q.get(&a.id).is_none());
        assert!(q.dir.join("dead/dropped").join(format!("{}.json", a.id)).exists());
    }

    #[test]
    fn fail_returns_to_pending_then_dead_or_dead_at_once_when_permanent() {
        let (_td, env) = test_env();
        let q = q(&env);
        let a = q.post(&member("HOME"), "wiki.note.v1", &note("a"), None, None, 0.0).unwrap();
        q.claim("s", 1.0).unwrap();
        assert!(!q.fail(&a.id, None, "repo dirty", false, 2.0).unwrap().1);
        q.claim("s", 3.0).unwrap();
        assert!(q.fail(&a.id, None, "repo dirty", false, 4.0).unwrap().1, "the second try was the last");
        let b = q.post(&member("HOME"), "wiki.note.v1", &note("b"), None, None, 5.0).unwrap();
        q.claim("s", 6.0).unwrap();
        assert!(q.fail(&b.id, None, "bad payload", true, 7.0).unwrap().1);
        assert_eq!(q.status(8.0, false).dead.len(), 2);
    }

    #[test]
    fn backpressure_type_schema_size_depth_rate_and_dedupe() {
        let (_td, env) = test_env();
        let q = Queue::open(&env, &aide_id("HOME/@KB").unwrap(), limits(&["wiki.note.v1"]));
        let m = member("HOME/X");
        assert_eq!(q.post(&m, "wiki.query.v1", &json!({"question": "q"}), None, None, 0.0).unwrap_err().code, "inbox.type");
        assert_eq!(q.post(&m, "nope.v1", &note("x"), None, None, 0.0).unwrap_err().code, "inbox.type");
        assert_eq!(q.post(&m, "wiki.note.v1", &json!({"title": "no body"}), None, None, 0.0).unwrap_err().code, "inbox.schema");
        assert_eq!(q.post(&m, "wiki.note.v1", &json!({"body": "x", "domain": "Not A Slug"}), None, None, 0.0).unwrap_err().code, "inbox.schema");
        assert_eq!(q.post(&m, "wiki.note.v1", &json!({"body": "x", "kind": "rumour"}), None, None, 0.0).unwrap_err().code, "inbox.schema");
        assert_eq!(q.post(&m, "wiki.note.v1", &json!({"body": "x", "extra": 1}), None, None, 0.0).unwrap_err().code, "inbox.schema");
        assert_eq!(q.post(&m, "wiki.note.v1", &note(&"x".repeat(80 * 1024)), None, None, 0.0).unwrap_err().code, "inbox.size");
        // every sender is rate-limited, members too: 3 a minute
        for i in 0..3 {
            q.post(&m, "wiki.note.v1", &note(&format!("n{i}")), None, None, i as f64).unwrap();
        }
        let r = q.post(&m, "wiki.note.v1", &note("n3"), None, None, 10.0).unwrap_err();
        assert_eq!((r.code, r.exit), ("inbox.rate", 4));
        // another sender still fits until the depth cap (4)
        let other = member("HOME/Y");
        q.post(&other, "wiki.note.v1", &note("o1"), None, None, 11.0).unwrap();
        assert_eq!(q.post(&other, "wiki.note.v1", &note("o2"), None, None, 12.0).unwrap_err().code, "inbox.full");
        // the window moves on
        q.claim("s", 70.0).unwrap();
        let first = q.list(Some(State::Claimed))[0].1.id.clone();
        q.done(&first, None, None, 71.0).unwrap();
        assert!(q.post(&m, "wiki.note.v1", &note("later"), Some("k1"), None, 72.0).is_ok());
        // a key already queued is not queued again
        let d = q.post(&other, "wiki.note.v1", &note("same"), Some("k1"), None, 73.0).unwrap();
        assert!(d.duplicate);
    }

    #[test]
    fn a_request_has_a_token_and_only_its_asker_reads_the_answer() {
        let (_td, env) = test_env();
        let q = Queue::open(&env, &aide_id("HOME/@REF").unwrap(), limits(&["wiki.query.v1"]));
        let asker = Sender { kind: "session".into(), session: Some("s-ask".into()), job: Some("j-ask".into()), verified: true, ..Default::default() };
        let p = q.post(&asker, "wiki.query.v1", &json!({"question": "what is the lease?", "length": "short"}), None, None, 0.0).unwrap();
        let token = p.token.clone().expect("a request gets a token");
        let item = q.get(&p.id).unwrap().1;
        assert_ne!(item.reply_to.as_ref().unwrap().token_sha256, token, "only the hash is stored");
        assert!(!std::fs::read_to_string(q.path(State::Pending, &p.id)).unwrap().contains(&token));
        q.claim("ref", 1.0).unwrap();
        assert_eq!(q.done(&p.id, None, None, 2.0).unwrap_err().code, "inbox.result", "a request is done with its answer");
        let done = q.done(&p.id, None, Some(json!({"answer": "15 minutes [[lease]]", "citations": ["wiki/lease.md"]})), 3.0).unwrap();
        assert!(may_read(&done, &asker, None));
        let same_job_new_session = Sender { kind: "session".into(), session: Some("s-new".into()), job: Some("j-ask".into()), verified: true, ..Default::default() };
        assert!(may_read(&done, &same_job_new_session, None), "the same job after a /clear");
        let stranger = Sender { kind: "session".into(), session: Some("s-other".into()), job: Some("j-other".into()), verified: true, ..Default::default() };
        assert!(!may_read(&done, &stranger, None));
        assert!(!may_read(&done, &member("HOME/X"), None));
        assert!(may_read(&done, &stranger, Some(&token)), "the token opens it");
        assert!(!may_read(&done, &stranger, Some("0000")));
        assert!(may_read(&done, &Sender { kind: "user".into(), verified: true, ..Default::default() }, None));
        // nobody live to deliver to: it waits for `inbox result`
        assert!(deliver(&env, &q, &done, 4.0).is_err());
        let st = q.status(5.0, true);
        assert_eq!(st.undelivered, 1);
        assert!(st.recent[0].result.is_some(), "the user's view shows the result");
        assert!(q.status(5.0, false).recent[0].result.is_none(), "others' do not");
    }

    #[test]
    fn done_items_are_pruned_after_keep_done_and_a_crash_copy_is_resolved() {
        let (_td, env) = test_env();
        let q = q(&env);
        let a = q.post(&member("HOME"), "wiki.note.v1", &note("a"), None, None, 0.0).unwrap();
        q.claim("s", 1.0).unwrap();
        q.done(&a.id, None, None, 2.0).unwrap();
        // a crash left the pending copy too: the later state wins and the copy goes
        let item = q.get(&a.id).unwrap().1;
        q.write(State::Pending, &item).unwrap();
        assert_eq!(q.get(&a.id).unwrap().0, State::Done);
        assert_eq!(q.status(3.0, false).pending, 0);
        assert!(!q.path(State::Pending, &a.id).exists());
        // keep_done is 1h
        assert_eq!(q.status(3700.0, false).done, 0);
        assert!(q.get(&a.id).is_none());
    }

    #[test]
    fn concurrent_posts_get_unique_ordered_ids() {
        let (_td, env) = test_env();
        let d = InboxDef { types: vec!["wiki.note.v1".into()], per_sender: "1000/m".into(), depth_cap: 1000, ..Default::default() };
        let aide = aide_id("HOME/@KB").unwrap();
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let (env, aide, d) = (env.clone(), aide.clone(), d.clone());
                std::thread::spawn(move || {
                    let q = Queue::open(&env, &aide, Limits::from_def(&d).unwrap());
                    (0..10).map(|i| q.post(&member(&format!("HOME/T{t}")), "wiki.note.v1", &note(&format!("{t}-{i}")), None, None, 0.0).unwrap().id).collect::<Vec<_>>()
                })
            })
            .collect();
        let mut ids: Vec<String> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "no id twice");
        assert_eq!(Queue::open(&env, &aide, Limits::from_def(&d).unwrap()).list(Some(State::Pending)).len(), 80);
    }

    #[test]
    fn item_ids_take_one_form_and_never_a_path() {
        assert_eq!(item_id("17").unwrap(), "000000000017");
        assert_eq!(item_id("000000000017").unwrap(), "000000000017");
        for bad in ["../../x", "17/..", "-1", "", "1e3", "HOME%2F@REF/claimed/000000000042"] {
            assert!(item_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn two_claimers_never_take_the_same_item() {
        let (_td, env) = test_env();
        let q0 = q(&env);
        q0.post(&member("HOME"), "wiki.note.v1", &note("only"), None, None, 0.0).unwrap();
        let got: Vec<bool> = (0..6)
            .map(|i| {
                let env = env.clone();
                std::thread::spawn(move || q(&env).claim(&format!("s{i}"), 1.0).ok().flatten().is_some())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        assert_eq!(got.iter().filter(|x| **x).count(), 1, "{got:?}");
    }

    #[test]
    fn a_stale_holder_cannot_finish_an_item_leased_again() {
        let (_td, env) = test_env();
        let q = q(&env);
        let a = q.post(&member("HOME"), "wiki.note.v1", &note("a"), None, None, 0.0).unwrap();
        q.claim("old-session", 1.0).unwrap();
        q.claim("new-session", 70.0).unwrap(); // the old lease ran out
        assert_eq!(q.done(&a.id, Some("old-session"), None, 71.0).unwrap_err().code, "inbox.lease");
        assert_eq!(q.fail(&a.id, Some("old-session"), "x", false, 71.0).unwrap_err().code, "inbox.lease");
        q.done(&a.id, Some("new-session"), None, 72.0).unwrap();
    }

    #[test]
    fn an_unreadable_file_is_set_aside_not_left_to_block_the_queue() {
        let (_td, env) = test_env();
        let q = q(&env);
        let a = q.post(&member("HOME"), "wiki.note.v1", &note("a"), None, None, 0.0).unwrap();
        std::fs::write(q.dir.join("claimed/000000000099.json"), "{ not json").unwrap();
        std::fs::write(q.dir.join("pending/000000000000.json"), "").unwrap();
        assert_eq!(q.claim("s", 1.0).unwrap().unwrap().id, a.id, "neither busy nor stuck");
        assert_eq!(q.status(2.0, false).quarantined, 2);
    }

    #[test]
    fn a_reset_queue_restarts_its_ids_but_never_a_ref() {
        let (_td, env) = test_env();
        let q0 = q(&env);
        let a = q0.post(&member("HOME"), "wiki.note.v1", &note("a"), None, None, 0.0).unwrap();
        let ref_a = q0.get(&a.id).unwrap().1.reference;
        assert!(ref_a.ends_with(&format!(":{}", a.id)), "{ref_a}");
        // a dropped item's id is never reused
        q0.drop_item(&a.id, 1.0).unwrap();
        std::fs::remove_file(q0.dir.join("seq")).unwrap();
        let b = q0.post(&member("HOME"), "wiki.note.v1", &note("b"), None, None, 2.0).unwrap();
        assert!(b.id > a.id);
        // the folder is wiped: the ids restart, the refs do not repeat
        std::fs::remove_dir_all(&q0.dir).unwrap();
        let c = q0.post(&member("HOME"), "wiki.note.v1", &note("c"), None, None, 3.0).unwrap();
        assert_eq!(c.id, a.id);
        assert_ne!(q0.get(&c.id).unwrap().1.reference, ref_a);
    }

    #[cfg(unix)] // the asker's socket here is a Unix one (a named pipe on Windows)
    #[test]
    fn the_answer_reaches_the_live_asker_once_and_not_while_ask_waits() {
        use std::io::Read;
        let (td, env) = test_env();
        let q = Queue::open(&env, &aide_id("HOME/@REF").unwrap(), limits(&["wiki.query.v1"]));
        // the asker: a live session (this process) with a messaging socket
        let me = std::process::id() as i64;
        let sock = td.path().join(format!("{me}.sock"));
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let sessions = env.claude_home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join(format!("{me}.json")), json!({"pid": me, "sessionId": "s-live", "jobId": "jlive001", "version": "2.1.296"}).to_string()).unwrap();
        crate::inbox::record(&env, "s-live", me, &sock.to_string_lossy());
        let asker = Sender { kind: "session".into(), session: Some("s-live".into()), job: Some("jlive001".into()), verified: true, ..Default::default() };
        let t = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut s = String::new();
            c.read_to_string(&mut s).unwrap();
            s
        });
        // `ask --wait` until 100: no peer message while it waits
        let p = q.post(&asker, "wiki.query.v1", &json!({"question": "q"}), None, Some(100.0), 0.0).unwrap();
        q.claim("ref", 1.0).unwrap();
        let done = q.done(&p.id, None, Some(json!({"answer": "the answer [[page]]"})), 2.0).unwrap();
        assert!(deliver(&env, &q, &done, 2.0).unwrap().contains("waits"));
        // past the wait (ask gave up): delivered, once
        let note = deliver(&env, &q, &done, 101.0).unwrap();
        assert!(note.contains("delivered to session s-live"), "{note}");
        let line = t.join().unwrap();
        assert!(line.contains("[nimitz reply] HOME/@REF answered your request") && line.contains("the answer [[page]]"), "{line}");
        let now_done = q.get(&p.id).unwrap().1;
        assert!(now_done.reply_to.as_ref().unwrap().delivered_at.is_some());
        assert_eq!(deliver(&env, &q, &now_done, 102.0).unwrap(), "nothing to deliver");
        assert_eq!(q.status(103.0, false).undelivered, 0);
    }

    #[test]
    fn limits_parse_durations_and_rates() {
        assert_eq!(duration("15m").unwrap(), 900.0);
        assert_eq!(duration("7d").unwrap(), 604800.0);
        assert!(duration("soon").is_err());
        let l = Limits::from_def(&InboxDef::default()).unwrap();
        assert_eq!((l.per_sender, l.lease_s, l.keep_done_s, l.max_tries), ((60, 3600.0), 900.0, 604800.0, 3));
        assert!(Limits::from_def(&InboxDef { types: vec!["wiki.nope.v9".into()], ..Default::default() }).is_err());
        assert!(Limits::from_def(&InboxDef { policy: "anyone".into(), ..Default::default() }).is_err());
        let w = Limits::from_def(&InboxDef { per_sender: "60/15m".into(), ..Default::default() }).unwrap();
        assert_eq!(w.per_sender, (60, 900.0), "a window with a number");
        let w = Limits::from_def(&InboxDef { per_sender: "10/30s".into(), ..Default::default() }).unwrap();
        assert_eq!(w.per_sender, (10, 30.0));
        assert_eq!(human(90.0), "1m30s");
        assert_eq!(human(7200.0), "2h");
    }
}
