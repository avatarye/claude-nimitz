//! Where admiralty-watch reads from. Every path can be overridden, which is how
//! the sandbox points the whole tool at a fictional world.

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Env {
    /// dir holding the `<fleet>/FLEET_CONFIG.json` siblings
    pub root: PathBuf,
    /// Claude Code home (`~/.claude`)
    pub claude_home: PathBuf,
    /// binary used for control actions and the CLI fallback
    pub claude_bin: String,
    pub hostnames: Vec<String>,
    pub action_log: PathBuf,
    /// a sub-agent written to this recently counts as running
    pub active_window_s: f64,
    /// Some(bin) when running inside Herdr
    pub herdr_bin: Option<String>,
    /// read `claude agents --json --all` instead of the session files
    pub force_cli: bool,
    /// quit after a successful Herdr jump (for a popup that should get out of the way)
    pub exit_after_jump: bool,
    /// where folds and filters are kept between runs
    pub state_path: PathBuf,
    /// the real Claude Code binary for job-mutating commands (`rm`): on an ACME box
    /// the `claude` on PATH is a wrapper that swallows them
    pub native_claude_bin: String,
}

/// Newest `~/.localtools/tools/claude-code/<version>/claude`, else None.
fn localtools_claude() -> Option<String> {
    let dir = home().join(".localtools/tools/claude-code");
    let mut vs: Vec<(Vec<u64>, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join("claude").is_file())
        .map(|p| {
            let v = p.file_name().map(|n| n.to_string_lossy().split('.').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
            (v, p.join("claude"))
        })
        .collect();
    vs.sort();
    vs.pop().map(|(_, p)| p.to_string_lossy().into_owned())
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn expand(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(p),
    }
}

pub fn which(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

fn local_hostnames() -> Vec<String> {
    let mut buf = [0u8; 256];
    let name = unsafe {
        if libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..end]).into_owned()
        } else {
            String::new()
        }
    };
    let mut out = vec![];
    if !name.is_empty() {
        let l = name.to_lowercase();
        out.push(l.split('.').next().unwrap_or("").to_string());
        out.push(l);
    }
    out.sort();
    out.dedup();
    out
}

impl Env {
    /// Build from a map of variables (the process environment in normal use).
    pub fn from_vars(v: &HashMap<String, String>) -> Env {
        let get = |k: &str| v.get(k).filter(|s| !s.is_empty()).cloned();
        let hostnames = match get("ADMIRALTY_WATCH_HOSTNAME") {
            Some(h) => vec![h.to_lowercase()],
            None => local_hostnames(),
        };
        let herdr_bin = if v.get("HERDR_ENV").map(|s| s == "1").unwrap_or(false) {
            get("ADMIRALTY_WATCH_HERDR_BIN").or_else(|| which("herdr"))
        } else {
            None
        };
        Env {
            root: expand(&get("ADMIRALTY_WATCH_ROOT").unwrap_or("~/Dropbox/claude-orchestrators".into())),
            claude_home: expand(&get("ADMIRALTY_WATCH_CLAUDE_HOME").unwrap_or("~/.claude".into())),
            claude_bin: get("ADMIRALTY_WATCH_CLAUDE_BIN").or_else(|| which("claude")).unwrap_or("claude".into()),
            hostnames,
            action_log: expand(
                &get("ADMIRALTY_WATCH_ACTION_LOG").unwrap_or("~/.local/state/admiralty-watch/actions.log".into()),
            ),
            active_window_s: 90.0,
            herdr_bin,
            force_cli: get("ADMIRALTY_WATCH_SOURCE").map(|s| s == "cli").unwrap_or(false),
            exit_after_jump: get("ADMIRALTY_WATCH_EXIT_AFTER_JUMP").map(|s| s == "1").unwrap_or(false),
            state_path: expand(&get("ADMIRALTY_WATCH_STATE").unwrap_or("~/.local/state/admiralty-watch/ui.json".into())),
            native_claude_bin: get("ADMIRALTY_WATCH_NATIVE_CLAUDE_BIN")
                .or_else(localtools_claude)
                .or_else(|| get("ADMIRALTY_WATCH_CLAUDE_BIN"))
                .or_else(|| which("claude"))
                .unwrap_or("claude".into()),
        }
    }

    pub fn from_process() -> Env {
        Env::from_vars(&std::env::vars().collect())
    }
}
