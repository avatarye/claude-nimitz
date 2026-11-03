//! Where the TUI's data comes from. Everything is behind `DataSource` so tests
//! and previews inject fixtures, and the real binary reads nimitz-core and
//! nimitz-mux. The TUI itself never parses Claude Code files.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use nimitz_core::Snapshot;
use nimitz_mux::{Caps, Mux, Pane};

/// One refresh: the core snapshot plus what the multiplexer sees.
#[derive(Clone, Debug)]
pub struct Bundle {
    pub snap: Snapshot,
    pub panes: Vec<Pane>,
    /// "herdr" | "none" | fixture name
    pub mux_name: String,
    pub mux_caps: Caps,
    /// a quick first look (session files only, no `claude agents`); the full
    /// snapshot follows at once
    pub partial: bool,
    /// each fleet's knowledge base on this machine, by fleet id (a fleet not
    /// looked at yet has none: its bar shows no KB until it is)
    pub kb: BTreeMap<String, KbLook>,
}

/// A fleet's KB as the loader last looked at it: its clone's facts, or why
/// there is none (no `kb` in its settings, no clone here, another repo).
pub type KbLook = Result<nimitz_core::kb::Facts, String>;

/// How often the loader looks at a fleet's KB clone again (it runs git
/// there: never more often than this, and never on the draw path).
pub const KB_EVERY: Duration = Duration::from_secs(30);

/// One refresh at a time; the loader thread owns the source (it is built there,
/// so a multiplexer adapter never crosses threads).
pub trait DataSource {
    fn load(&mut self) -> Bundle;
}

// ------------------------------------------------------------------ fixtures

/// Serves a fixed Bundle (demo world or a JSON file); refreshes return it unchanged.
pub struct FixtureSource {
    pub bundle: Bundle,
}

impl DataSource for FixtureSource {
    fn load(&mut self) -> Bundle {
        self.bundle.clone()
    }
}

/// A fixture file: the Snapshot plus panes and mux capabilities, all JSON.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct FixtureFile {
    pub snapshot: Snapshot,
    #[serde(default)]
    pub panes: Vec<FixturePane>,
    #[serde(default)]
    pub mux_name: Option<String>,
    #[serde(default)]
    pub mux_open: bool,
    #[serde(default)]
    pub mux_focus: bool,
    #[serde(default)]
    pub mux_type_text: bool,
    #[serde(default)]
    pub mux_layout: bool,
    /// each fleet's KB, by fleet id: `{"Ok": {facts}}` or `{"Err": "why"}`
    #[serde(default)]
    pub kb: BTreeMap<String, KbLook>,
}

/// serde mirror of `nimitz_mux::Pane` (which has no serde derives).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct FixturePane {
    pub pane_id: String,
    #[serde(default)]
    pub tab_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub state: String,
}

impl From<FixturePane> for Pane {
    fn from(p: FixturePane) -> Pane {
        Pane {
            pane: nimitz_mux::PaneRef { pane_id: p.pane_id, tab_id: p.tab_id, workspace_id: p.workspace_id },
            session_id: p.session_id,
            title: p.title,
            cwd: p.cwd,
            state: p.state,
            inferred: false,
        }
    }
}

pub fn load_fixture_file(path: &PathBuf) -> Result<Bundle, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let f: FixtureFile = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Bundle {
        snap: f.snapshot,
        panes: f.panes.into_iter().map(Pane::from).collect(),
        mux_name: f.mux_name.unwrap_or_else(|| "fixture".into()),
        mux_caps: Caps { open: f.mux_open, focus: f.mux_focus, type_text: f.mux_type_text, layout: f.mux_layout },
        partial: false,
        kb: f.kb,
    })
}

// ------------------------------------------------------------------ the real thing

/// Reads nimitz-core's snapshot and the detected multiplexer's panes.
pub struct CoreSource {
    pub env: nimitz_core::env::Env,
    pub cache: nimitz_core::snapshot::Cache,
    pub mux: Box<dyn Mux>,
    /// the next load is the first: make it quick
    pub first: bool,
    /// each fleet's KB and when it was looked at (`KB_EVERY`)
    pub kb: HashMap<String, (Instant, KbLook)>,
}

impl CoreSource {
    pub fn detect(vars: &HashMap<String, String>) -> CoreSource {
        let env = nimitz_core::env::Env::from_vars(vars);
        // outside a Herdr pane, the Herdr server the user's commands recorded
        // (`active::user_mux_vars`): Admiralty sees the leads' panes wherever it runs
        let mux = nimitz_mux::detect(&nimitz_core::active::user_mux_vars(&env, vars));
        CoreSource { env, cache: nimitz_core::snapshot::Cache::default(), mux, first: false, kb: HashMap::new() }
    }
}

/// Fold core's integrity check into the snapshot: each finding goes on the
/// member it names (or the fleet, or the machine), once.
pub fn merge_check(snap: &mut Snapshot, found: Vec<nimitz_core::Issue>) {
    let same = |a: &nimitz_core::Issue, b: &nimitz_core::Issue| a.code == b.code && a.id == b.id && a.message == b.message;
    let seen = |snap: &Snapshot, i: &nimitz_core::Issue| crate::view::all_issues(snap).into_iter().any(|x| same(x, i));
    for i in found {
        if seen(snap, &i) {
            continue;
        }
        let mut placed = false;
        if let Some(id) = i.id.clone() {
            for f in &mut snap.fleets {
                if f.id.fleet() != id.fleet() {
                    continue;
                }
                let on_member = f.members_mut().find(|m| m.id == id).map(|m| m.issues.push(i.clone())).is_some();
                if !on_member {
                    f.issues.push(i.clone());
                }
                placed = true;
                break;
            }
        }
        if !placed {
            snap.issues.push(i);
        }
    }
}

impl DataSource for CoreSource {
    fn load(&mut self) -> Bundle {
        if std::mem::take(&mut self.first) {
            // `claude agents` (the slow part: it starts the CLI) is skipped: a
            // throwaway cache and no binary, so the real cache is untouched
            let mut quick = self.env.clone();
            quick.native_claude_bin = "/nonexistent/claude".into();
            quick.claude_bin = "/nonexistent/claude".into();
            let mut snap = nimitz_core::snapshot::snapshot(&quick, &mut nimitz_core::snapshot::Cache::default());
            snap.compat_ok = true; // not known yet: no warning until it is
            return Bundle { snap, panes: self.mux.panes(), mux_name: self.mux.name().to_string(), mux_caps: self.mux.caps(), partial: true, kb: BTreeMap::new() };
        }
        let mut snap = nimitz_core::snapshot::snapshot(&self.env, &mut self.cache);
        let mut found = nimitz_core::check::check(&self.env, &snap, None);
        for f in &snap.fleets {
            for m in crate::view::fleet_members(f) {
                found.extend(nimitz_core::sessions::session_issues(&self.env, m));
            }
        }
        merge_check(&mut snap, found);
        let kb = self.kb_looks(&snap);
        Bundle {
            snap,
            panes: self.mux.panes(),
            mux_name: self.mux.name().to_string(),
            mux_caps: self.mux.caps(),
            partial: false,
            kb,
        }
    }
}

impl CoreSource {
    /// Each fleet's KB here: looked at again once its last look is
    /// `KB_EVERY` old (on the loader thread, never the draw's); a fleet gone
    /// is forgotten.
    fn kb_looks(&mut self, snap: &Snapshot) -> BTreeMap<String, KbLook> {
        let now = Instant::now();
        self.kb.retain(|k, _| snap.fleets.iter().any(|f| f.id.0 == *k));
        let mut out = BTreeMap::new();
        for f in &snap.fleets {
            let fresh = self.kb.get(&f.id.0).is_some_and(|(at, _)| now.duration_since(*at) < KB_EVERY);
            if !fresh {
                let look = if f.machine.is_none() { Err("the fleet does not run on this machine".to_string()) } else { nimitz_core::kb::look(&self.env, f.id.fleet()) };
                self.kb.insert(f.id.0.clone(), (now, look));
            }
            if let Some((_, l)) = self.kb.get(&f.id.0) {
                out.insert(f.id.0.clone(), l.clone());
            }
        }
        out
    }
}

/// Pane for a member's current session, by session id first, then title+cwd.
pub fn pane_for<'a>(session_id: Option<&str>, name: &str, cwd: Option<&str>, panes: &'a [Pane]) -> Option<&'a Pane> {
    if let Some(id) = session_id {
        if let Some(hit) = panes.iter().find(|p| p.session_id.as_deref() == Some(id)) {
            return Some(hit);
        }
    }
    panes.iter().find(|p| {
        !p.title.is_empty()
            && (p.title == name || p.title.starts_with(&format!("{name}-")))
            && cwd.map(|c| p.cwd.trim_end_matches('/') == c.trim_end_matches('/')).unwrap_or(false)
    })
}
