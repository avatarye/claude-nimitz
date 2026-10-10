//! Shared data types (contracts.md sections 3-8). Owned by the integrator:
//! change only together with contracts.md.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

// ------------------------------------------------------------------ roles and ids

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admiral,
    Commodore,
    Ship,
}

/// `admiral | commodore | ship`; a 0.3.x registry's `tender` reads as a ship
/// (the id `F/SQ/~task` is what marks it, and `migrate::retire_tenders` moves
/// such an identity out of the members).
impl<'de> Deserialize<'de> for Role {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Role, D::Error> {
        let s = String::deserialize(d)?;
        match s.as_str() {
            "admiral" => Ok(Role::Admiral),
            "commodore" => Ok(Role::Commodore),
            "ship" | "tender" => Ok(Role::Ship),
            other => Err(serde::de::Error::unknown_variant(other, &["admiral", "commodore", "ship"])),
        }
    }
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admiral => "admiral",
            Role::Commodore => "commodore",
            Role::Ship => "ship",
        }
    }
    /// Admirals and commodores lead; every member, leads too, runs as a
    /// Claude Code background job (plan-v4 section 7). A lead's job is pinned
    /// and attached to in its Herdr tab; a ship's parks and wakes on demand.
    pub fn lead(self) -> bool {
        matches!(self, Role::Admiral | Role::Commodore)
    }
}

/// `F`, `F/SQ`, `F/SQ/SHIP` (contracts.md section 4).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Id(pub String);

impl Id {
    pub fn admiral(fleet: &str) -> Id {
        Id(fleet.to_string())
    }
    pub fn commodore(fleet: &str, sq: &str) -> Id {
        Id(format!("{fleet}/{sq}"))
    }
    pub fn ship(fleet: &str, sq: &str, ship: &str) -> Id {
        Id(format!("{fleet}/{sq}/{ship}"))
    }
    pub fn parts(&self) -> Vec<&str> {
        self.0.split('/').collect()
    }
    pub fn fleet(&self) -> &str {
        self.parts()[0]
    }
    pub fn squadron(&self) -> Option<&str> {
        self.parts().get(1).copied()
    }
    pub fn role(&self) -> Role {
        let p = self.parts();
        match p.len() {
            1 => Role::Admiral,
            2 => Role::Commodore,
            _ => Role::Ship,
        }
    }
    /// The session name (address) for this identity.
    pub fn name(&self) -> String {
        let p = self.parts();
        match (self.role(), p.as_slice()) {
            (Role::Admiral, [f]) => format!("[{f}]-ADMIRAL"),
            (Role::Commodore, [f, sq]) => format!("[{f}.{sq}]-COMMODORE"),
            (_, [f, sq, s]) => format!("[{f}.{sq}]-{s}"),
            _ => self.0.clone(),
        }
    }
    /// The identity above this one in the chain of command.
    pub fn parent(&self) -> Option<Id> {
        let p = self.parts();
        match p.len() {
            1 => None,
            2 => Some(Id(p[0].to_string())),
            _ => Some(Id(format!("{}/{}", p[0], p[1]))),
        }
    }
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ------------------------------------------------------------------ config (TOML)

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineDef {
    pub hostname: String,
    #[serde(default)]
    pub os: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmiralDef {
    #[serde(default)]
    pub permission_mode: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetConfig {
    pub fleet: String,
    #[serde(default)]
    pub machines: BTreeMap<String, MachineDef>,
    #[serde(default)]
    pub admiral: AdmiralDef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Comms {
    /// "open" | "commodore-only"
    Mode(String),
    Allow { allow: Vec<String> },
}

impl Default for Comms {
    fn default() -> Self {
        Comms::Mode("open".into())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipDef {
    pub name: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub prompt_file: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// machine label -> folder, or "managed"
    #[serde(default)]
    pub dirs: BTreeMap<String, String>,
    #[serde(default)]
    pub permission_mode: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SquadronConfig {
    pub squadron: String,
    #[serde(default)]
    pub brief: Option<String>,
    #[serde(default)]
    pub comms: Comms,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub ships: Vec<ShipDef>,
}

/// One fleet on disk: its config, its squadrons, and where they live.
#[derive(Clone, Debug, PartialEq)]
pub struct FleetOnDisk {
    pub dir: PathBuf,
    pub config: FleetConfig,
    pub squadrons: Vec<(PathBuf, SquadronConfig)>,
    /// this machine's label in `config.machines`, if listed
    pub machine: Option<String>,
}

// ------------------------------------------------------------------ registry

/// One conversation an identity had (registry v2, plan-v4 section 2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineageEntry {
    pub session_id: String,
    /// the job (short id) that ran it, when known
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// launch | resume | relaunch | clear | branch | fork | compact | adopt | migrate | hook
    pub via: String,
    /// when it was bound (a v1 file says `bound_at`)
    #[serde(alias = "bound_at")]
    pub at: f64,
    /// the `NIMITZ_HOME` of the process that bound it (as written; stamped by
    /// the registry write). None on an entry an older nimitz wrote: its home
    /// is judged from its job's row while that row exists
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
}

/// An identity's anchor: the job (short id) and the conversation it runs now.
/// A v1 file says `current` for `session` and has no `job`;
/// `registry::load` migrates it (`migrate::registry_v2`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IdentityRecord {
    pub role: Role,
    pub name: String,
    pub cwd: String,
    /// the Claude Code job (short id); None before the first bind
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// the conversation now (session id)
    #[serde(default, alias = "current")]
    pub session: Option<String>,
    #[serde(default)]
    pub lineage: Vec<LineageEntry>,
}

/// The registry format this nimitz writes.
pub const REGISTRY_VERSION: u32 = 2;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    pub version: u32,
    pub machine: String,
    /// when it was last written (epoch seconds): `status` replays SessionStart
    /// lines newer than this (plan-v4 section 3)
    #[serde(default)]
    pub updated_at: f64,
    #[serde(default)]
    pub identities: BTreeMap<String, IdentityRecord>,
    /// identities of roles that no longer exist (0.3.x tenders), kept as
    /// history by `migrate::retire_tenders`; never members, never started
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub retired: BTreeMap<String, IdentityRecord>,
}

impl IdentityRecord {
    /// Every session id the identity ever ran: the one now, then the lineage.
    pub fn session_ids(&self) -> impl Iterator<Item = &str> {
        self.session.iter().map(String::as_str).chain(self.lineage.iter().map(|l| l.session_id.as_str()))
    }
}

// ------------------------------------------------------------------ events

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub ts: f64,
    pub event: String,
    pub session_id: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub pid: Option<i64>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// SessionStart only: the rules version the hook handed the session
    #[serde(default)]
    pub rules: Option<u32>,
    /// SessionStart only, from `sessions/<pid>.json`: the job (short id) the
    /// process runs; null for an interactive or SDK process
    #[serde(default)]
    pub job: Option<String>,
    /// SessionStart only: the session file's `kind` and `entrypoint`
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub entrypoint: Option<String>,
    /// SessionStart only: what the hook did with the registry (`Binding`)
    #[serde(default)]
    pub binding: Option<String>,
    /// SessionStart only: the session's folder (the hook input's `cwd`)
    #[serde(default)]
    pub cwd: Option<String>,
}

// ------------------------------------------------------------------ snapshot

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    NotStarted,
    Stopped,
    /// a job with no live process that nobody stopped (the daemon retired it,
    /// it crashed, or the machine went down): not an error; a start wakes it
    Parked,
    Idle,
    Busy,
    NeedsYou,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    /// stable code, contracts.md section 7 (e.g. "name.drift")
    pub code: String,
    pub severity: Severity,
    pub message: String,
    /// a `nimitz ...` command that fixes it, when there is one
    pub fix: Option<String>,
    /// the identity it is about, if any
    pub id: Option<Id>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub short_id: Option<String>,
    pub pid: Option<i64>,
    /// interactive | background
    pub kind: String,
    /// job state (stale on live rows)
    pub state: Option<String>,
    /// busy | idle | waiting (live only)
    pub status: Option<String>,
    pub waiting_for: Option<String>,
    pub started_at: Option<f64>,
    pub status_since: Option<f64>,
    pub name: String,
    pub cwd: String,
    /// labels found for this session (events or saved options); empty if none
    pub labels: BTreeMap<String, String>,
    /// the job's birth session id (`state.json` `sessionId`) when the
    /// conversation moved on (`/clear`, `/branch`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birth_id: Option<String>,
    /// the session file's `entrypoint` (cli, sdk-cli, claude-vscode, ...)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    /// the job is in `jobs/pins.json`: the daemon never idle-retires it
    #[serde(default)]
    pub pinned: bool,
    /// a copy of the member's conversation (plan-v4 section 6), and how it
    /// was told: `transcript` (shares the conversation's root), `resume` (a
    /// resume of a lineage session id), `name` (the member's exact name or
    /// Claude Code's copy name), `fork` (its job row says so)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy: Option<String>,
    /// the terminal an interactive process runs in (`tmux <pane>`), when the
    /// session file names one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Crew {
    pub agent_id: String,
    pub parent_id: Option<String>,
    pub label: String,
    pub agent_type: Option<String>,
    pub teammate: bool,
    pub running: bool,
    pub last_write: f64,
    /// the Workflow run that started it (`wf_<run>`); none for a plain subagent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
    /// the workflow's name (`meta.name` of its script), when known
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_name: Option<String>,
    /// the workflow phase it runs in
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub subject: String,
    pub status: String,
    pub owner: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub id: Id,
    pub role: Role,
    pub expected_name: String,
    pub cwd: Option<String>,
    pub status: Status,
    /// why the status is what it is, newest evidence first
    pub reasons: Vec<String>,
    pub session: Option<SessionInfo>,
    /// other job rows of this identity (stale rows, copies)
    pub extra_rows: Vec<SessionInfo>,
    pub crew: Vec<Crew>,
    pub tasks: Vec<Task>,
    pub issues: Vec<Issue>,
    /// set by the TUI/CLI from the mux, never by core
    pub pane: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Squadron {
    pub id: Id,
    pub commodore: Member,
    pub ships: Vec<Member>,
    pub comms: Comms,
    pub issues: Vec<Issue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fleet {
    pub id: Id,
    pub dir: String,
    pub machine: Option<String>,
    pub admiral: Member,
    pub squadrons: Vec<Squadron>,
    pub issues: Vec<Issue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub taken_at: f64,
    pub machine: Option<String>,
    pub fleets: Vec<Fleet>,
    /// sessions in fleet scope with no labels
    pub stowaways: Vec<SessionInfo>,
    pub issues: Vec<Issue>,
    pub cc_version: Option<String>,
    /// true when the installed Claude Code is inside compat.toml's range
    pub compat_ok: bool,
}

// ------------------------------------------------------------------ actions

/// One step of a plan: a command `nimitz` will run, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub id: Option<Id>,
    pub cmd: Vec<String>,
    pub cwd: Option<String>,
    pub why: String,
}

/// One member's line in a restore plan: what restore found, and what it
/// does about it (`worker on 2.1.294, installed 2.1.295` -> `relaunch`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Found {
    pub id: Id,
    pub found: String,
    pub action: String,
    /// the plan's verbs for it (`restore::verbs`): its job (relaunch, wake,
    /// launch, fresh, skip, refuse, nothing) and its pane (reattach, open
    /// pane, focus, or empty), and where that pane is
    #[serde(default)]
    pub job: String,
    #[serde(default)]
    pub pane: String,
    #[serde(default)]
    pub place: String,
}

/// What an action would do. `refusals` non-empty means it will not run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub summary: String,
    pub steps: Vec<Step>,
    pub refusals: Vec<Issue>,
    pub warnings: Vec<Issue>,
    /// a restore's per-member findings, shown before the steps
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub found: Vec<Found>,
}

impl Plan {
    pub fn ok(&self) -> bool {
        self.refusals.is_empty()
    }
}

/// Result of applying a plan: what ran and what the re-read state says.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub ran: Vec<(Step, i32, String)>,
    pub verified: bool,
    pub issues: Vec<Issue>,
}

/// Who is asking (from the caller's NIMITZ_* env, or the user when absent).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Caller {
    User,
    Member { id: Id, role: Role },
}
