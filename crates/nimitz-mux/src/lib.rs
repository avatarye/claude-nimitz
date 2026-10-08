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

#[derive(Clone, Debug, Default, PartialEq)]
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

/// One workspace of the multiplexer's shape (Herdr WorkspaceInfo).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapeWorkspace {
    pub id: String,
    pub label: String,
    pub active_tab_id: String,
}

/// One tab of the shape (Herdr TabInfo).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapeTab {
    pub id: String,
    pub workspace_id: String,
    pub label: String,
}

/// One pane of the shape: where it is, its folder, the agent Herdr sees in
/// it (`agents[]`: `agent_session.value` is the Claude session id) and what
/// runs in its foreground (`pane process-info`): `Some(argv)`, empty for an
/// idle shell; `None` when it could not be read (never typed into).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShapePane {
    pub pane: PaneRef,
    pub cwd: String,
    pub title: String,
    pub agent: Option<String>,
    /// the session the pane shows, when its foreground backs the claim
    /// (`evidence::claim_backed`)
    pub session_id: Option<String>,
    /// the multiplexer's own claim, backed or not (Herdr's hook names a pane
    /// by an inherited `HERDR_PANE_ID`); read only to tell the agent view
    /// from the attach it replaced (R-24)
    pub claimed: Option<String>,
    /// the multiplexer's state for the pane's agent (idle|working|blocked|done|unknown)
    pub state: String,
    pub foreground: Option<Vec<String>>,
    /// the pid of what runs in the foreground (`pane process-info`), when read
    pub foreground_pid: Option<i64>,
}

/// The whole shape at once (`herdr api snapshot` + `pane process-info` per
/// pane): what a restore matches members against.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    pub version: String,
    pub protocol: u64,
    pub workspaces: Vec<ShapeWorkspace>,
    pub tabs: Vec<ShapeTab>,
    pub panes: Vec<ShapePane>,
}

impl Shape {
    pub fn tab(&self, tab_id: &str) -> Option<&ShapeTab> {
        self.tabs.iter().find(|t| t.id == tab_id)
    }
    pub fn workspace(&self, ws_id: &str) -> Option<&ShapeWorkspace> {
        self.workspaces.iter().find(|w| w.id == ws_id)
    }
    /// The panes of a tab, in id order.
    pub fn panes_of(&self, tab_id: &str) -> Vec<&ShapePane> {
        let mut v: Vec<&ShapePane> = self.panes.iter().filter(|p| p.pane.tab_id.as_deref() == Some(tab_id)).collect();
        v.sort_by(|a, b| a.pane.pane_id.cmp(&b.pane.pane_id));
        v
    }
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

/// (tab label, command, cwd)
pub type TabPlan = (String, Vec<String>, std::path::PathBuf);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutPlan {
    /// workspace label -> [(tab label, command, cwd)]
    pub workspaces: Vec<(String, Vec<TabPlan>)>,
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

    /// The pane of the tab `place` names (a `Place::Tab`, by its label in its
    /// workspace, loosely), if that tab is open: its shell pane (the first
    /// pane whose foreground is a shell), else its first pane. A tab can hold
    /// a split that runs another program (a Yazi pane): never that one. An
    /// adapter that cannot list tabs knows no tab.
    fn tab(&self, place: &Place) -> Option<PaneRef> {
        let _ = place;
        None
    }

    /// What runs in the foreground of a pane's terminal: the argv of the
    /// foreground process, or empty for an idle shell. `Err` when it cannot be
    /// read (then nothing is typed into the pane).
    fn foreground(&self, pane: &PaneRef) -> Result<Vec<String>, MuxError> {
        let _ = pane;
        Err(MuxError::Unsupported("reading a pane's foreground"))
    }

    /// Run a command line in a pane's shell (`herdr pane run`). Only
    /// `attach` and a restore call it, and only into a pane whose foreground
    /// is a shell.
    fn run_in(&self, pane: &PaneRef, cmd: &[String]) -> Result<(), MuxError> {
        let _ = (pane, cmd);
        Err(MuxError::Unsupported("running a command in a pane"))
    }

    /// The whole shape: workspaces, tabs, panes, each pane's agent session
    /// and, with `foregrounds`, its foreground argv (`herdr api snapshot`,
    /// then `pane process-info` per pane). A restore reads it once with the
    /// foregrounds, then matches members against it; the active set needs
    /// only where the sessions show.
    fn shape(&self, foregrounds: bool) -> Result<Shape, MuxError> {
        let _ = foregrounds;
        Err(MuxError::Unsupported("reading the shape"))
    }

    /// Give a tab its full label (`herdr tab rename`): a restore claims a
    /// legacy short tab (`COMMODORE`) or a new workspace's unlabelled root tab.
    fn rename_tab(&self, tab_id: &str, label: &str) -> Result<(), MuxError> {
        let _ = (tab_id, label);
        Err(MuxError::Unsupported("renaming a tab"))
    }
}

/// Is this the argv of a shell sitting at its prompt (nothing in the
/// foreground but the shell itself)? One list, core's (the restore planner
/// judges panes with the same rule).
pub use nimitz_core::restore::is_shell;

/// What a member's tab shows now (restore-scenarios.md, decision 5).
#[derive(Clone, Debug, PartialEq)]
pub enum TabState {
    /// a pane shows the member's session, or runs its exact attach already
    Shown(PaneRef),
    /// the tab is there and its pane is a plain shell: claim it
    Shell(PaneRef),
    /// the tab is there but its pane runs something else (another agent, a
    /// copy, the agent view, yazi): never typed into
    Taken { pane: PaneRef, running: String },
    /// no tab for it yet
    Missing,
}

/// Find the member's tab: a pane showing `session_id` anywhere (R1), else the
/// pane that already runs `cmd` (`claude attach <job>`, visible while it
/// runs), else the tab `place` names by its full label, judged by what its
/// pane runs. `Err` when the pane's foreground cannot be read: nothing is
/// typed into a pane nimitz cannot see into (R2).
pub fn tab_state(mux: &dyn Mux, cmd: &[String], place: &Place, session_id: Option<&str>) -> Result<TabState, MuxError> {
    if let Some(p) = session_id.and_then(|sid| mux.panes().into_iter().find(|p| p.session_id.as_deref() == Some(sid))) {
        return Ok(TabState::Shown(p.pane));
    }
    let Some(pane) = mux.tab(place) else { return Ok(TabState::Missing) };
    let fg = mux.foreground(&pane)?;
    if is_shell(&fg) {
        return Ok(TabState::Shell(pane));
    }
    // the same attach, by its program name and arguments (`claude attach <job>`)
    let same = cmd.len() > 1 && fg.len() == cmd.len() && fg[1..] == cmd[1..] && program_name(&fg[0]) == program_name(&cmd[0]);
    if same {
        return Ok(TabState::Shown(pane));
    }
    Ok(TabState::Taken { pane, running: fg.join(" ") })
}

fn program_name(p: &str) -> String {
    std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string())
}

/// How `attach` showed a member.
#[derive(Clone, Debug, PartialEq)]
pub enum Attached {
    /// a pane showed it already: focused, nothing ran
    Focused(PaneRef),
    /// its tab's shell pane was claimed: the attach ran there, then focused
    Claimed(PaneRef),
    /// a new tab was opened with the attach
    Opened(PaneRef),
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttachError {
    /// the member's tab runs something else: nothing was typed (`tab.taken`)
    Taken { pane: PaneRef, running: String },
    Mux(MuxError),
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttachError::Taken { pane, running } => write!(f, "its tab's pane {} runs `{running}`, not a shell; nothing was typed into it", pane.pane_id),
            AttachError::Mux(e) => e.fmt(f),
        }
    }
}

impl From<MuxError> for AttachError {
    fn from(e: MuxError) -> Self {
        AttachError::Mux(e)
    }
}

/// Show a member in its tab: focus the pane that shows it (`TabState::Shown`),
/// run `cmd` (`claude attach <job>`) in its tab's shell pane (`Shell`), or
/// open the tab with `cmd` (`Missing`). A taken tab is an error and nothing is
/// typed (R2). The attach-a-lead step of `nimitz open`, `signal`, Admiralty's
/// `A` and a restore.
pub fn attach(mux: &dyn Mux, cmd: &[String], cwd: &Path, place: Place, session_id: Option<&str>) -> Result<Attached, AttachError> {
    match tab_state(mux, cmd, &place, session_id)? {
        TabState::Shown(p) => {
            mux.focus(&p)?;
            Ok(Attached::Focused(p))
        }
        TabState::Shell(p) => {
            mux.run_in(&p, cmd)?;
            mux.focus(&p)?;
            Ok(Attached::Claimed(p))
        }
        TabState::Taken { pane, running } => Err(AttachError::Taken { pane, running }),
        TabState::Missing => Ok(Attached::Opened(mux.open(cmd, cwd, place)?)),
    }
}

pub mod fake;
pub mod herdr;
pub mod none;

/// Text that is safe to type into an agent's pane: escape sequences (CSI, OSC,
/// two-byte ESC) and every other C0/C1 control and DEL removed; newlines, CRs
/// and tabs become spaces (Herdr's `agent prompt` has no paste mode, and a
/// newline would submit early); runs of spaces collapse; ends trimmed.
pub fn sanitize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\u{1b}' => match it.next() {
                // CSI: parameters, then one final byte in @..~
                Some('[') => {
                    for d in it.by_ref() {
                        if ('@'..='~').contains(&d) {
                            break;
                        }
                    }
                }
                // OSC, DCS, PM, APC, SOS: up to BEL or ST (ESC \)
                Some(']') | Some('P') | Some('^') | Some('_') | Some('X') => {
                    while let Some(d) = it.next() {
                        if d == '\u{7}' || (d == '\u{1b}' && it.peek() == Some(&'\\')) {
                            if d == '\u{1b}' {
                                it.next();
                            }
                            break;
                        }
                    }
                }
                _ => {} // ESC + one char
            },
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_control() => {} // other C0, DEL, C1 (incl. the 8-bit CSI)
            c => out.push(c),
        }
    }
    let mut collapsed = String::with_capacity(out.len());
    for w in out.split(' ').filter(|w| !w.is_empty()) {
        if !collapsed.is_empty() {
            collapsed.push(' ');
        }
        collapsed.push_str(w);
    }
    collapsed
}

/// The values `NIMITZ_MUX` may take.
pub const MUX_SETTINGS: [&str; 3] = ["auto", "herdr", "none"];

/// Is the `NIMITZ_MUX` value (if any) one we know?
pub fn valid_mux_setting(vars: &std::collections::HashMap<String, String>) -> Result<(), String> {
    match vars.get("NIMITZ_MUX").map(String::as_str) {
        None | Some("") => Ok(()),
        Some(v) if MUX_SETTINGS.contains(&v) => Ok(()),
        Some(v) => Err(format!("NIMITZ_MUX={v:?} is not a multiplexer setting; use one of: {}", MUX_SETTINGS.join(", "))),
    }
}

/// What to do when no multiplexer could open a pane: the cause, not a guess.
pub fn missing_mux_fix(vars: &std::collections::HashMap<String, String>) -> String {
    let herdr_env = vars.get("HERDR_ENV").map(String::as_str) == Some("1");
    match vars.get("NIMITZ_MUX").map(String::as_str) {
        Some("none") => "NIMITZ_MUX=none turns the multiplexer off: unset it, or set NIMITZ_MUX=auto".into(),
        _ if herdr_env && resolve_herdr_bin(vars).is_none() => {
            "HERDR_ENV=1 is set but no herdr binary was found: put herdr on PATH, or set NIMITZ_HERDR_BIN".into()
        }
        _ => "run nimitz inside Herdr (HERDR_ENV is not 1 here), or run nimitz status inside Herdr once so it records the server".into(),
    }
}

/// `NIMITZ_MUX` override, else Herdr when `HERDR_ENV=1` and a herdr binary
/// resolves (`NIMITZ_HERDR_BIN`, else PATH), else NoMux. Pure in `vars`. (The
/// user's own command outside Herdr passes `active::user_mux_vars`, which
/// names the recorded server with `NIMITZ_MUX=herdr`.)
pub fn detect(vars: &std::collections::HashMap<String, String>) -> Box<dyn Mux> {
    match vars.get("NIMITZ_MUX").map(String::as_str) {
        Some("none") => return Box::new(none::NoMux),
        Some("herdr") => {
            // explicit override: take the configured binary even if it does not resolve yet
            let bin = resolve_herdr_bin(vars).unwrap_or_else(|| "herdr".into());
            return Box::new(herdr::Herdr { bin, session: herdr_session(vars), socket: vars.get("NIMITZ_HERDR_SOCKET").filter(|s| !s.is_empty()).cloned(), claude_home: nimitz_core::env::Env::from_vars(vars).claude_home });
        }
        // `auto` (the default) and unset: detect. An unknown name: safest is no mux
        // (the CLI refuses it first with `valid_mux_setting`).
        Some("auto") | Some("") | None => {}
        Some(_) => return Box::new(none::NoMux),
    }
    if vars.get("HERDR_ENV").map(String::as_str) == Some("1") {
        if let Some(bin) = resolve_herdr_bin(vars) {
            return Box::new(herdr::Herdr { bin, session: herdr_session(vars), socket: vars.get("NIMITZ_HERDR_SOCKET").filter(|s| !s.is_empty()).cloned(), claude_home: nimitz_core::env::Env::from_vars(vars).claude_home });
        }
    }
    Box::new(none::NoMux)
}

/// `NIMITZ_HERDR_SESSION`: the named Herdr session every herdr call gets as
/// `--session` (a test server; the checklist), so a live pane's inherited
/// socket is never the one spoken to. None: Herdr's own resolution.
pub fn herdr_session(vars: &std::collections::HashMap<String, String>) -> Option<String> {
    vars.get("NIMITZ_HERDR_SESSION").filter(|s| !s.is_empty()).cloned()
}

/// `NIMITZ_HERDR_BIN` (checked as a path when it has a `/`, else searched on
/// PATH under that name), else `herdr` on PATH.
fn resolve_herdr_bin(vars: &std::collections::HashMap<String, String>) -> Option<String> {
    let name = vars.get("NIMITZ_HERDR_BIN").map(String::as_str).unwrap_or("herdr");
    if name.contains('/') {
        return executable(std::path::Path::new(name)).then(|| name.to_string());
    }
    let path = vars.get("PATH").map(String::as_str).unwrap_or("");
    path.split(':')
        .filter(|d| !d.is_empty())
        .map(|d| std::path::Path::new(d).join(name))
        .find(|p| executable(p))
        .map(|p| p.to_string_lossy().into_owned())
}

fn executable(p: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn fake_bin_dir() -> tempdir::TempDir {
        let d = tempdir::TempDir::new();
        std::fs::write(d.path().join("herdr"), "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(d.path().join("herdr"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        d
    }

    // minimal temp dir without a dev-dependency
    mod tempdir {
        pub struct TempDir(std::path::PathBuf);
        impl TempDir {
            pub fn new() -> Self {
                static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                let p = std::env::temp_dir().join(format!(
                    "nimitz-mux-test-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ));
                std::fs::create_dir_all(&p).unwrap();
                TempDir(p)
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn detect_override_none_wins_over_herdr_env() {
        let d = fake_bin_dir();
        let v = vars(&[("NIMITZ_MUX", "none"), ("HERDR_ENV", "1"), ("PATH", d.path().to_str().unwrap())]);
        assert_eq!(detect(&v).name(), "none");
    }

    #[test]
    fn detect_override_herdr_without_env() {
        let v = vars(&[("NIMITZ_MUX", "herdr")]);
        let m = detect(&v);
        assert_eq!(m.name(), "herdr");
        assert!(m.caps().open && m.caps().focus && m.caps().type_text && m.caps().layout);
    }

    #[test]
    fn a_named_herdr_session_goes_on_every_call() {
        assert_eq!(herdr_session(&vars(&[("NIMITZ_HERDR_SESSION", "nimitz-e2e")])), Some("nimitz-e2e".into()));
        assert_eq!(herdr_session(&vars(&[("NIMITZ_HERDR_SESSION", "")])), None);
        assert_eq!(herdr::Herdr { bin: "herdr".into(), session: Some("nimitz-e2e".into()), socket: None, claude_home: "/x".into() }.argv(&["api", "snapshot"]), ["--session", "nimitz-e2e", "api", "snapshot"]);
        assert_eq!(herdr::Herdr { bin: "herdr".into(), session: None, socket: None, claude_home: "/x".into() }.argv(&["api", "snapshot"]), ["api", "snapshot"]);
    }

    #[test]
    fn sanitize_strips_controls_and_escapes() {
        assert_eq!(sanitize_text("x\u{3}\u{1b}[A\nsecond line"), "x second line");
        assert_eq!(sanitize_text("a\u{1b}]0;title\u{7}b\u{1b}]2;t\u{1b}\\c"), "abc");
        assert_eq!(sanitize_text("\u{9b}31m red\u{7f}\u{1b}c ok\t\r\n end  "), "31m red ok end");
        assert_eq!(sanitize_text("plain text, kept: [MAIN.ARGOS]-SHIP"), "plain text, kept: [MAIN.ARGOS]-SHIP");
        assert_eq!(sanitize_text("\u{1b}[31m"), "");
    }

    #[test]
    fn detect_auto_is_detection() {
        let d = fake_bin_dir();
        let v = vars(&[("NIMITZ_MUX", "auto"), ("HERDR_ENV", "1"), ("PATH", d.path().to_str().unwrap())]);
        assert_eq!(detect(&v).name(), "herdr");
        assert!(valid_mux_setting(&v).is_ok());
        assert!(valid_mux_setting(&vars(&[("NIMITZ_MUX", "tmux")])).unwrap_err().contains("auto, herdr, none"));
    }

    #[test]
    fn the_fix_names_what_is_missing() {
        assert!(missing_mux_fix(&vars(&[("NIMITZ_MUX", "none"), ("HERDR_ENV", "1")])).contains("NIMITZ_MUX=none"));
        assert!(missing_mux_fix(&vars(&[("HERDR_ENV", "1"), ("PATH", "/nonexistent-dir-nimitz")])).contains("no herdr binary"));
        assert!(missing_mux_fix(&vars(&[])).contains("HERDR_ENV is not 1"));
    }

    #[test]
    fn detect_unknown_override_is_none() {
        let v = vars(&[("NIMITZ_MUX", "tmux"), ("HERDR_ENV", "1")]);
        assert_eq!(detect(&v).name(), "none");
    }

    #[test]
    fn detect_herdr_env_with_binary_on_path() {
        let d = fake_bin_dir();
        let v = vars(&[("HERDR_ENV", "1"), ("PATH", d.path().to_str().unwrap())]);
        assert_eq!(detect(&v).name(), "herdr");
    }

    #[test]
    fn detect_herdr_env_without_binary_is_none() {
        let v = vars(&[("HERDR_ENV", "1"), ("PATH", "/nonexistent-dir-nimitz")]);
        assert_eq!(detect(&v).name(), "none");
    }

    #[test]
    fn detect_no_env_is_none() {
        let d = fake_bin_dir();
        let v = vars(&[("PATH", d.path().to_str().unwrap())]);
        assert_eq!(detect(&v).name(), "none");
    }

    #[test]
    fn detect_nimitz_herdr_bin_path_override() {
        let d = fake_bin_dir();
        let bin = d.path().join("herdr");
        let v = vars(&[("HERDR_ENV", "1"), ("NIMITZ_HERDR_BIN", bin.to_str().unwrap()), ("PATH", "")]);
        assert_eq!(detect(&v).name(), "herdr");
        let v = vars(&[("HERDR_ENV", "1"), ("NIMITZ_HERDR_BIN", "/nonexistent/herdr"), ("PATH", "")]);
        assert_eq!(detect(&v).name(), "none");
    }
}
