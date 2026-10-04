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
        _ => "run nimitz inside Herdr (HERDR_ENV is not 1 here)".into(),
    }
}

/// `NIMITZ_MUX` override, else Herdr when `HERDR_ENV=1` and a herdr binary
/// resolves (`NIMITZ_HERDR_BIN`, else PATH), else NoMux. Pure in `vars`.
pub fn detect(vars: &std::collections::HashMap<String, String>) -> Box<dyn Mux> {
    match vars.get("NIMITZ_MUX").map(String::as_str) {
        Some("none") => return Box::new(none::NoMux),
        Some("herdr") => {
            // explicit override: take the configured binary even if it does not resolve yet
            let bin = resolve_herdr_bin(vars).unwrap_or_else(|| "herdr".into());
            return Box::new(herdr::Herdr { bin });
        }
        // `auto` (the default) and unset: detect. An unknown name: safest is no mux
        // (the CLI refuses it first with `valid_mux_setting`).
        Some("auto") | Some("") | None => {}
        Some(_) => return Box::new(none::NoMux),
    }
    if vars.get("HERDR_ENV").map(String::as_str) == Some("1") {
        if let Some(bin) = resolve_herdr_bin(vars) {
            return Box::new(herdr::Herdr { bin });
        }
    }
    Box::new(none::NoMux)
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
