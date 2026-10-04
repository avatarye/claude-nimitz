//! Herdr integration: find the Herdr pane hosting a session and jump to it.
//!
//! Active only inside Herdr (HERDR_ENV=1). `herdr agent list` reports, per pane,
//! the Claude session id it hosts, so panes map to sessions exactly; the
//! terminal title plus cwd is the fallback. Jumping focuses the pane's
//! workspace, then its tab, then the pane. It only moves focus.

use serde_json::Value;
use std::process::Command;

use crate::model::Agent;

#[derive(Clone, Debug, PartialEq)]
pub struct Pane {
    pub pane_id: String,
    pub tab_id: String,
    pub workspace_id: String,
    pub session_id: Option<String>,
    pub title: String,
    pub cwd: String,
    /// Herdr's lifecycle state for the pane's agent: working | blocked | done | idle | unknown
    pub status: String,
}

/// Herdr's own sidebar dot for a state (src/client/shell.rs, status_icon / status_color, v0.9.3).
pub fn dot(status: &str) -> &'static str {
    match status {
        "working" | "blocked" | "done" => "\u{25cf}", // filled
        "idle" => "\u{25cb}",                          // empty
        _ => "\u{00b7}",                               // unknown
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

pub fn parse_panes(v: &Value) -> Vec<Pane> {
    let Some(list) = v.pointer("/result/agents").and_then(|a| a.as_array()) else { return vec![] };
    list.iter()
        .filter(|a| !s(a, "pane_id").is_empty())
        .map(|a| Pane {
            pane_id: s(a, "pane_id"),
            tab_id: s(a, "tab_id"),
            workspace_id: s(a, "workspace_id"),
            session_id: a.pointer("/agent_session/value").and_then(|x| x.as_str()).map(String::from),
            title: s(a, "terminal_title_stripped"),
            status: s(a, "agent_status"),
            cwd: {
                let f = s(a, "foreground_cwd");
                if f.is_empty() { s(a, "cwd") } else { f }
            },
        })
        .collect()
}

/// Agent panes Herdr knows about; empty outside Herdr or on any error.
pub fn list_panes(bin: Option<&str>) -> Vec<Pane> {
    let Some(bin) = bin else { return vec![] };
    let Ok(out) = Command::new(bin).args(["agent", "list"]).output() else { return vec![] };
    serde_json::from_slice::<Value>(&out.stdout).map(|v| parse_panes(&v)).unwrap_or_default()
}

pub fn pane_for<'a>(a: &Agent, panes: &'a [Pane]) -> Option<&'a Pane> {
    let p = a.primary.as_ref().filter(|p| p.live())?;
    if let Some(hit) = panes.iter().find(|x| x.session_id.as_deref() == Some(p.session_id.as_str())) {
        return Some(hit);
    }
    // fallback: a pane titled with the session name (or a suffixed variant) in the same dir
    panes.iter().find(|x| {
        !x.title.is_empty()
            && (x.title == p.name || x.title.starts_with(&format!("{}-", p.name)))
            && x.cwd.trim_end_matches('/') == p.cwd.trim_end_matches('/')
    })
}

pub fn jump_cmds(bin: &str, pane: &Pane) -> Vec<Vec<String>> {
    let mut cmds = vec![];
    if !pane.workspace_id.is_empty() {
        cmds.push(vec![bin.into(), "workspace".into(), "focus".into(), pane.workspace_id.clone()]);
    }
    if !pane.tab_id.is_empty() {
        cmds.push(vec![bin.into(), "tab".into(), "focus".into(), pane.tab_id.clone()]);
    }
    cmds.push(vec![bin.into(), "agent".into(), "focus".into(), pane.pane_id.clone()]);
    cmds
}

pub fn jump(bin: &str, pane: &Pane) -> (i32, String) {
    for c in jump_cmds(bin, pane) {
        match Command::new(&c[0]).args(&c[1..]).output() {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                let msg = String::from_utf8_lossy(if o.stderr.is_empty() { &o.stdout } else { &o.stderr }).trim().to_string();
                return (o.status.code().unwrap_or(-1), msg);
            }
            Err(e) => return (-1, e.to_string()),
        }
    }
    (0, String::new())
}
