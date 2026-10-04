//! Multiplexer adapters (contracts.md section 9). Owner: mux agent.
//! Only Herdr is built now; `None` is monitor-only.
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Caps {
    pub open: bool,
    pub focus: bool,
    pub type_text: bool,
    pub layout: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneRef {
    pub pane_id: String,
    pub tab_id: Option<String>,
    pub workspace_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pane {
    pub pane: PaneRef,
    pub session_id: Option<String>,
    pub title: String,
    pub cwd: String,
    /// the multiplexer's own state for the pane's agent (Herdr: working|blocked|done|idle|unknown)
    pub state: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    /// a new tab in the workspace named here (created if missing)
    Tab { workspace: String, label: String },
    /// split next to an existing pane
    Split { next_to: PaneRef },
    /// a new workspace
    Workspace { label: String },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutPlan {
    /// workspace label -> [(tab label, command, cwd)]
    pub workspaces: Vec<(String, Vec<(String, Vec<String>, std::path::PathBuf)>)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MuxError {
    Unsupported(&'static str),
    Failed(String),
}

impl std::fmt::Display for MuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MuxError::Unsupported(what) => write!(f, "no multiplexer: {what} needs one (run inside Herdr)"),
            MuxError::Failed(m) => f.write_str(m),
        }
    }
}

pub trait Mux {
    fn name(&self) -> &'static str;
    fn caps(&self) -> Caps;
    fn panes(&self) -> Vec<Pane>;
    fn open(&self, cmd: &[String], cwd: &Path, place: Place) -> Result<PaneRef, MuxError>;
    fn focus(&self, pane: &PaneRef) -> Result<(), MuxError>;
    fn type_text(&self, pane: &PaneRef, text: &str) -> Result<(), MuxError>;
    fn layout(&self, plan: &LayoutPlan) -> Result<Vec<PaneRef>, MuxError>;
}

pub mod herdr;
pub mod none;

/// `NIMITZ_MUX` override, else Herdr when `HERDR_ENV=1`, else None.
pub fn detect(_vars: &std::collections::HashMap<String, String>) -> Box<dyn Mux> {
    todo!("mux agent")
}
