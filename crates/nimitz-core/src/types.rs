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
    /// Admirals and commodores are interactive sessions; ships background.
    pub fn interactive(self) -> bool {
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
    /// 0.3.x tender lines, read and ignored (`check` says so), never written back
    #[serde(default, skip_serializing)]
    pub tenders: Option<toml::Value>,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineageEntry {
    pub session_id: String,
    pub bound_at: f64,
    /// launch | resume | relaunch | adopt | migrate
    pub via: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IdentityRecord {
    pub role: Role,
    pub name: String,
    pub cwd: String,
    pub current: Option<String>,
    #[serde(default)]
    pub lineage: Vec<LineageEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    pub version: u32,
    pub machine: String,
    #[serde(default)]
    pub identities: BTreeMap<String, IdentityRecord>,
    /// identities of roles that no longer exist (0.3.x tenders), kept as
    /// history by `migrate::retire_tenders`; never members, never started
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub retired: BTreeMap<String, IdentityRecord>,
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
}

// ------------------------------------------------------------------ snapshot

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    NotStarted,
    Stopped,
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

/// What an action would do. `refusals` non-empty means it will not run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub summary: String,
    pub steps: Vec<Step>,
    pub refusals: Vec<Issue>,
    pub warnings: Vec<Issue>,
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
