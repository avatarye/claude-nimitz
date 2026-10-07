//! Running the `claude` CLI: launch, resume, stop, rm, agents (contracts.md sections 2, 8). Owner: core-sessions.
//!
//! Two binaries (contracts.md section 2):
//! - `env.claude_bin` (PATH `claude`; on ACME a wrapper that adds Bedrock
//!   `--settings`) for cold launches, so the saved options carry the provider;
//! - `env.native_claude_bin` for resume, stop, rm, agents and `--version`. A
//!   resume through the wrapper always makes a copy (the wrapper adds flags).
//!
//! The argv builders here are the only place that knows the exact flags.
use crate::env::Env;
use crate::types::*;
use std::collections::BTreeMap;
use std::process::{Command, Stdio};

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: Option<String>,
    pub session_id: String,
    pub name: String,
    pub cwd: String,
    pub kind: String,
    pub pid: Option<i64>,
    pub state: Option<String>,
    pub status: Option<String>,
    pub started_at: Option<f64>,
}

impl Row {
    pub fn to_session_row(&self) -> crate::evidence::SessionRow {
        crate::evidence::SessionRow {
            session_id: self.session_id.clone(),
            name: self.name.clone(),
            cwd: self.cwd.clone(),
            kind: self.kind.clone(),
            short_id: self.id.clone(),
            pid: self.pid,
            state: self.state.clone(),
            status: self.status.clone(),
            status_since: None,
            started_at: self.started_at,
            waiting_for: None,
            job_labels: Default::default(),
            unreadable: false,
            forked_from: None,
            bridge_session_id: None,
            birth_id: None,
            entrypoint: None,
            pinned: false,
            spare: false,
            terminal: None,
            appeared_at: None,
        }
    }
}

/// Run a command to completion with stdin closed: (exit code, stdout + stderr).
pub fn run(cmd: &[String], cwd: Option<&str>) -> (i32, String) {
    let Some(bin) = cmd.first() else { return (-1, "empty command".into()) };
    let mut c = Command::new(bin);
    c.args(&cmd[1..]).stdin(Stdio::null());
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    match c.output() {
        Ok(o) => (
            o.status.code().unwrap_or(-1),
            format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_string(),
        ),
        Err(e) => (-1, format!("{bin}: {e}")),
    }
}

/// `claude agents --json --all` through the native binary.
pub fn agents(env: &Env) -> Result<Vec<Row>, String> {
    let out = Command::new(&env.native_claude_bin)
        .args(["agents", "--json", "--all"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("`claude agents --json --all` failed: {e}"))?;
    let v = crate::evidence::first_json(&out.stdout).ok_or_else(|| {
        let err = String::from_utf8_lossy(&out.stderr);
        format!("`claude agents --json --all` gave no JSON: {}", err.lines().last().unwrap_or("no output"))
    })?;
    if !v.is_array() {
        return Err("`claude agents --json --all` did not return a list".into());
    }
    Ok(crate::evidence::parse_cli_rows(&v)
        .into_iter()
        .map(|r| Row {
            id: r.short_id,
            session_id: r.session_id,
            name: r.name,
            cwd: r.cwd,
            kind: r.kind,
            pid: r.pid,
            state: r.state,
            status: r.status,
            started_at: r.started_at,
        })
        .collect())
}

/// The native binary's version, e.g. "2.1.288".
pub fn version(env: &Env) -> Option<String> {
    crate::compat::installed(env)
}

// ------------------------------------------------------------------ labels and argv

/// The labels `nimitz` gives a member's session (contracts.md section 4).
pub fn labels_for(env: &Env, id: &Id) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("NIMITZ_FLEET".to_string(), id.fleet().to_string());
    if let Some(sq) = id.squadron() {
        m.insert("NIMITZ_SQUADRON".to_string(), sq.to_string());
    }
    m.insert("NIMITZ_ROLE".to_string(), id.role().as_str().to_string());
    m.insert("NIMITZ_ID".to_string(), id.0.clone());
    m.insert("NIMITZ_HOME".to_string(), env.home.to_string_lossy().into_owned());
    m
}

/// The permission rule every member gets: it may run `nimitz` (the authority
/// table in `sessions::may_run` decides what each role may do with it).
pub const NIMITZ_ALLOW: &str = "Bash(nimitz *)";

/// `--settings` JSON: the labels as env, `permissions.allow` for `nimitz`,
/// plus `worktree.bgIsolation none` for background sessions. It is per
/// launch and lives in the job's `respawnFlags`, so a restart keeps it and a
/// relaunch (every flag again) re-adds it.
pub fn settings_json(env: &Env, id: &Id, background: bool) -> String {
    let mut v = serde_json::json!({ "env": labels_for(env, id), "permissions": { "allow": [NIMITZ_ALLOW] } });
    if background {
        v["worktree"] = serde_json::json!({ "bgIsolation": "none" });
    }
    v.to_string()
}

fn mode_args(mode: &str) -> Vec<String> {
    if mode.is_empty() {
        vec![]
    } else {
        vec!["--permission-mode".into(), mode.into()]
    }
}

/// Cold launch of a ship: `claude --bg --name N --permission-mode M --settings S [prompt]`.
pub fn launch_bg(env: &Env, id: &Id, mode: &str, prompt: Option<&str>) -> Vec<String> {
    let mut c = vec![env.claude_bin.clone(), "--bg".into(), "--name".into(), id.name()];
    c.extend(mode_args(mode));
    c.extend(["--settings".into(), settings_json(env, id, true)]);
    if let Some(p) = prompt.filter(|p| !p.is_empty()) {
        c.push(p.to_string());
    }
    c
}

/// Relaunch from an old session with every flag: a copy with a new id
/// (`source: fork`) that carries the labels. Through the PATH binary, as a cold launch.
pub fn relaunch_bg(env: &Env, id: &Id, mode: &str, from_session: &str) -> Vec<String> {
    let mut c = vec![env.claude_bin.clone(), "--bg".into(), "--resume".into(), from_session.into(), "--name".into(), id.name()];
    c.extend(mode_args(mode));
    c.extend(["--settings".into(), settings_json(env, id, true)]);
    c
}

/// Resume a stopped background session keeping its id: native binary, no flags.
pub fn resume_bg(env: &Env, session_id: &str) -> Vec<String> {
    vec![env.native_claude_bin.clone(), "--bg".into(), "--resume".into(), session_id.into()]
}

/// An admiral or commodore: `claude [-c] --name N [--permission-mode M] --settings S`,
/// run in a terminal (a mux pane), never in the background.
pub fn interactive(env: &Env, id: &Id, mode: &str, cont: bool) -> Vec<String> {
    let mut c = vec![env.claude_bin.clone()];
    if cont {
        c.push("-c".into());
    }
    c.extend(["--name".into(), id.name()]);
    c.extend(mode_args(mode));
    c.extend(["--settings".into(), settings_json(env, id, false)]);
    c
}

pub fn stop(env: &Env, short_id: &str) -> Vec<String> {
    vec![env.native_claude_bin.clone(), "stop".into(), short_id.into()]
}

pub fn rm(env: &Env, short_id: &str) -> Vec<String> {
    vec![env.native_claude_bin.clone(), "rm".into(), short_id.into()]
}

pub fn attach(env: &Env, short_id: &str) -> Vec<String> {
    vec![env.native_claude_bin.clone(), "attach".into(), short_id.into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_argv_carries_name_mode_labels_and_no_isolation() {
        let vars = [("NIMITZ_HOME", "/h"), ("NIMITZ_CLAUDE_BIN", "claude"), ("NIMITZ_NATIVE_CLAUDE_BIN", "/n/claude")];
        let env = Env::from_vars(&vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
        let id = Id::ship("F", "SQ", "S");
        let c = launch_bg(&env, &id, "auto", Some("go"));
        assert_eq!(&c[..4], &["claude", "--bg", "--name", "[F.SQ]-S"]);
        assert_eq!(&c[4..6], &["--permission-mode", "auto"]);
        let v: serde_json::Value = serde_json::from_str(&c[7]).unwrap();
        assert_eq!(v["env"]["NIMITZ_ID"], "F/SQ/S");
        assert_eq!(v["worktree"]["bgIsolation"], "none");
        assert_eq!(v["permissions"]["allow"], serde_json::json!([NIMITZ_ALLOW]), "every member may run nimitz");
        assert_eq!(c.last().unwrap(), "go");
        // resume: native binary, no flags at all
        assert_eq!(resume_bg(&env, "sid"), vec!["/n/claude", "--bg", "--resume", "sid"]);
        // a relaunch passes every flag again, the permission rule with them
        let r = relaunch_bg(&env, &id, "", "sid");
        assert_eq!(&r[..4], &["claude", "--bg", "--resume", "sid"]);
        assert!(r.last().unwrap().contains("\"Bash(nimitz *)\""), "{r:?}");
        // interactive sessions have no worktree key, but the permission rule too
        let i = interactive(&env, &Id::commodore("F", "SQ"), "", true);
        assert_eq!(&i[..4], &["claude", "-c", "--name", "[F.SQ]-COMMODORE"]);
        assert!(!i[5].contains("worktree") && i[5].contains("\"Bash(nimitz *)\""), "{}", i[5]);
    }
}
