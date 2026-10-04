//! Control actions: activate (resume or cold-launch) a fleet agent, and attach to one.
//!
//! The launch recipe mirrors the fleet skill (`launch / restore`) so the two
//! cannot drift: resume the newest background row for (name, dir); cold-launch
//! with the config prompt plus the lead line only when no such row exists.
//! Leads are never launched from here; only the user starts a lead.
//!
//! Every action is planned first (a refusal or the exact command), confirmed by
//! the user, then run and logged.

use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::env::Env;
use crate::model::{Agent, Status};

pub const SETTINGS_NO_ISOLATION: &str = r#"{"worktree":{"bgIsolation":"none"}}"#;

#[derive(Clone, Debug)]
pub struct Plan {
    pub action: String,
    pub agent: String,
    pub ok: bool,
    pub summary: String,
    pub cmd: Vec<String>,
    pub cwd: Option<String>,
    pub details: Vec<(String, String)>,
    /// further commands run after `cmd`, in order (e.g. one `rm` per job row)
    pub more: Vec<Vec<String>>,
}

impl Plan {
    fn refuse(action: &str, a: &Agent, why: impl Into<String>) -> Plan {
        Plan { action: action.into(), agent: a.name.clone(), ok: false, summary: why.into(), cmd: vec![], cwd: None, details: vec![], more: vec![] }
    }

    pub fn shell(&self) -> String {
        let join = |c: &Vec<String>| c.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
        let line = std::iter::once(&self.cmd).chain(self.more.iter()).map(join).collect::<Vec<_>>().join("; ");
        match &self.cwd {
            Some(c) => format!("(cd {} && {line})", shell_quote(c)),
            None => line,
        }
    }
}

pub fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }
}

pub fn lead_line(fleet: &str) -> String {
    format!("Your fleet lead is [{fleet}]-LEAD. Report completion and blockers to it by that name with SendMessage.")
}

pub fn plan_activate(env: &Env, a: &Agent) -> Plan {
    let act = "activate";
    if a.fleet == crate::model::HUB_TAG {
        return Plan::refuse(act, a, "The hub is started by you with `ccl` in the admiralty directory, never from here.");
    }
    if a.fleet == "OTHER" {
        return Plan::refuse(act, a, "Not a fleet agent.");
    }
    if a.is_lead {
        return Plan::refuse(act, a, "Leads are started by you with `ccl` in the fleet directory, never from here.");
    }
    if !matches!(a.status, Status::Stopped | Status::NotRunning) {
        return Plan::refuse(act, a, format!("{} is already running.", a.name));
    }
    let d = a.declared.as_ref();
    let Some(dir) = d.and_then(|d| d.dir.clone()).or_else(|| a.primary.as_ref().map(|p| p.cwd.clone())) else {
        return Plan::refuse(act, a, "No directory for this agent on this machine in FLEET_CONFIG.json.");
    };
    if !Path::new(&dir).is_dir() {
        return Plan::refuse(act, a, format!("Directory does not exist: {dir}"));
    }
    let mut rows: Vec<_> = a
        .primary
        .iter()
        .chain(a.stale.iter())
        .filter(|r| r.kind == "background" && r.cwd.trim_end_matches('/') == dir.trim_end_matches('/'))
        .collect();
    rows.sort_by(|x, y| x.started_at.partial_cmp(&y.started_at).unwrap_or(std::cmp::Ordering::Equal));
    let mode = d.map(|d| d.permission_mode.clone()).unwrap_or("bypassPermissions".into());
    let mut cmd = vec![env.claude_bin.clone(), "--bg".into(), "--name".into(), a.name.clone(), "--permission-mode".into(), mode.clone()];
    let mut details = vec![("dir".to_string(), dir.clone()), ("permission".to_string(), mode)];
    let summary;
    let mut prompt = None;
    if let Some(last) = rows.last() {
        cmd.extend(["--resume".into(), last.session_id.clone()]);
        details.push(("resume".into(), format!("{}  (newest row for this name and dir)", last.session_id)));
        summary = format!("Resume {} from its last session", a.name);
    } else {
        let Some(p) = d.and_then(|d| d.prompt.clone()) else {
            return Plan::refuse(
                act,
                a,
                "No prior session and no prompt recorded in FLEET_CONFIG.json; a cold launch needs one. Ask the fleet lead.",
            );
        };
        details.push(("prompt".into(), "from FLEET_CONFIG.json + lead line (cold launch, no prior session)".into()));
        summary = format!("Cold-launch {} (no prior session found)", a.name);
        prompt = Some(format!("{p}\n\n{}", lead_line(&a.fleet)));
    }
    if !d.map(|d| d.bg_isolation).unwrap_or(false) {
        cmd.extend(["--settings".into(), SETTINGS_NO_ISOLATION.into()]);
    }
    if let Some(p) = prompt {
        cmd.push(p);
    }
    Plan { action: act.into(), agent: a.name.clone(), ok: true, summary, cmd, cwd: Some(dir), details, more: vec![] }
}

/// Deleting a session = removing its background job rows with `claude rm <id>`.
/// Only dead sessions that are not fleet agents qualify. `rm` is job-mutating and
/// goes through the native Claude Code binary: on an Acme (ACME) box the
/// `claude` wrapper on PATH swallows `rm` as a chat prompt and changes nothing.
pub fn plan_delete(env: &Env, a: &Agent) -> Plan {
    let act = "delete";
    if a.fleet == crate::model::HUB_TAG {
        return Plan::refuse(act, a, "The hub is never deleted from here.");
    }
    if a.is_lead {
        return Plan::refuse(act, a, "Leads are fleet agents and are never deleted from here.");
    }
    if a.declared.is_some() {
        return Plan::refuse(act, a, format!("{} is a fleet agent (declared in FLEET_CONFIG.json); it is never deleted from here.", a.name));
    }
    let rows: Vec<&crate::sources::SessionRow> = a.primary.iter().chain(a.stale.iter()).collect();
    if rows.iter().any(|r| r.live()) {
        return Plan::refuse(act, a, format!("{} is live. Only stopped sessions can be deleted.", a.name));
    }
    let ids: Vec<String> = rows.iter().filter(|r| r.kind == "background").filter_map(|r| r.short_id.clone()).collect();
    if ids.is_empty() {
        return Plan::refuse(act, a, "Nothing to delete: this session left no background job row, only its transcript.");
    }
    let bin = env.native_claude_bin.clone();
    let mut cmds: Vec<Vec<String>> = ids.iter().map(|id| vec![bin.clone(), "rm".into(), id.clone()]).collect();
    let first = cmds.remove(0);
    Plan {
        action: act.into(),
        agent: a.name.clone(),
        ok: true,
        summary: format!("Delete {} ({} job row{})", a.name, ids.len(), if ids.len() == 1 { "" } else { "s" }),
        cmd: first,
        cwd: None,
        details: vec![
            ("job rows".into(), ids.join(" ")),
            ("binary".into(), bin),
            ("keeps".into(), "the transcripts (`claude --resume <session>` still works)".into()),
        ],
        more: cmds,
    }
}

pub fn plan_attach(env: &Env, a: &Agent, in_tmux: bool) -> Plan {
    let act = "attach";
    let Some(p) = a.primary.as_ref().filter(|p| p.live()) else {
        return Plan::refuse(act, a, format!("{} is not running. Activate it first (a).", a.name));
    };
    let Some(short) = p.short_id.clone().filter(|_| p.kind == "background") else {
        return Plan::refuse(
            act,
            a,
            format!(
                "{} is an interactive session: go to its own terminal. Claude Code has no supported command to message it from outside.",
                a.name
            ),
        );
    };
    let attach = vec![env.claude_bin.clone(), "attach".into(), short.clone()];
    let (cmd, how) = if in_tmux {
        let line = attach.iter().map(|x| shell_quote(x)).collect::<Vec<_>>().join(" ");
        (vec!["tmux".into(), "split-window".into(), "-h".into(), line], "new tmux pane")
    } else {
        (attach, "this terminal (admiralty-watch suspends until you detach)")
    };
    Plan {
        action: act.into(),
        agent: a.name.clone(),
        ok: true,
        summary: format!("Attach to {} in a terminal", a.name),
        cmd,
        cwd: None,
        details: vec![("session".into(), short), ("opens in".into(), how.into())],
        more: vec![],
    }
}

fn stamp() -> String {
    // local time via libc, `YYYY-MM-DDTHH:MM:SS`
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

pub fn log(env: &Env, plan: &Plan, rc: i32, out: &str) {
    if let Some(dir) = env.action_log.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tail: String = out.chars().rev().take(2000).collect::<Vec<_>>().into_iter().rev().collect();
    let rec = json!({"at": stamp(), "action": plan.action, "agent": plan.agent, "cwd": plan.cwd,
                     "cmd": plan.cmd, "rc": rc, "out": tail});
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&env.action_log) {
        let _ = writeln!(f, "{rec}");
    }
}

/// Run a planned command that returns on its own (`claude --bg`, `tmux split`).
pub fn run_detached(env: &Env, plan: &Plan) -> (i32, String) {
    let (mut rc, mut out) = (0, String::new());
    for cmd in std::iter::once(&plan.cmd).chain(plan.more.iter()) {
        let mut c = Command::new(&cmd[0]);
        c.args(&cmd[1..]).stdin(Stdio::null());
        if let Some(d) = &plan.cwd {
            c.current_dir(d);
        }
        let (r, o) = match c.output() {
            Ok(o) => (
                o.status.code().unwrap_or(-1),
                format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_string(),
            ),
            Err(e) => (-1, e.to_string()),
        };
        if r != 0 && rc == 0 {
            rc = r;
        }
        if !o.is_empty() {
            out = if out.is_empty() { o } else { format!("{out}\n{o}") };
        }
    }
    log(env, plan, rc, &out);
    (rc, out)
}

/// Run in the user's terminal (attach without tmux). The caller leaves the TUI first.
pub fn run_foreground(env: &Env, plan: &Plan) -> i32 {
    let mut c = Command::new(&plan.cmd[0]);
    c.args(&plan.cmd[1..]);
    if let Some(d) = &plan.cwd {
        c.current_dir(d);
    }
    let rc = c.status().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
    log(env, plan, rc, "");
    rc
}

// ------------------------------------------------------------------ messages

/// Where a message typed in the TUI goes. Leads (and the hub) get it straight
/// into their Herdr pane; an agent gets it through its fleet lead, which relays
/// it with SendMessage. Delivery is `herdr agent prompt <pane> <text>`, which
/// Herdr refuses while the target is blocked on an approval.
#[derive(Clone, Debug)]
pub struct MessagePlan {
    pub ok: bool,
    /// the agent the user means to reach
    pub to: String,
    /// the session that actually receives the text (the agent itself, or its lead)
    pub via: String,
    pub relay: bool,
    pub pane: Option<crate::herdr::Pane>,
    pub summary: String,
}

pub fn plan_message(env: &Env, a: &Agent, lead: Option<&Agent>, pane_of: &dyn Fn(&str) -> Option<crate::herdr::Pane>) -> MessagePlan {
    let refuse = |why: String| MessagePlan { ok: false, to: a.name.clone(), via: String::new(), relay: false, pane: None, summary: why };
    if env.herdr_bin.is_none() {
        return refuse("Messages go through Herdr; not running inside Herdr (HERDR_ENV is not 1).".into());
    }
    if !a.live() {
        return refuse(format!("{} is not running. Activate it first (a).", a.name));
    }
    let direct = a.is_lead || a.fleet == crate::model::HUB_TAG;
    if direct {
        return match pane_of(&a.key) {
            Some(p) => MessagePlan {
                ok: true,
                to: a.name.clone(),
                via: a.name.clone(),
                relay: false,
                summary: format!("to {} in Herdr pane {}", a.name, p.pane_id),
                pane: Some(p),
            },
            None => refuse(format!("No Herdr pane hosts {}; there is no other supported way to type into it.", a.name)),
        };
    }
    if a.fleet == "OTHER" {
        return refuse("Not a fleet agent: there is no lead to relay through.".into());
    }
    let Some(lead) = lead.filter(|l| l.live()) else {
        return refuse(format!("[{}]-LEAD is not running, so nothing can relay to {}.", a.fleet, a.name));
    };
    match pane_of(&lead.key) {
        Some(p) => MessagePlan {
            ok: true,
            to: a.name.clone(),
            via: lead.name.clone(),
            relay: true,
            summary: format!("to {} via {} (Herdr pane {}), which relays it", a.name, lead.name, p.pane_id),
            pane: Some(p),
        },
        None => refuse(format!("No Herdr pane hosts {}, so the message cannot be handed to it.", lead.name)),
    }
}

/// The text the receiving session gets.
pub fn message_text(plan: &MessagePlan, text: &str) -> String {
    if plan.relay {
        format!(
            "[admiralty-watch] Message from the user for {}. Relay it to {} with SendMessage, unchanged:\n\n{}",
            plan.to, plan.to, text
        )
    } else {
        text.to_string()
    }
}

pub fn send_message(env: &Env, plan: &MessagePlan, text: &str) -> (i32, String) {
    let (Some(bin), Some(pane)) = (env.herdr_bin.as_ref(), plan.pane.as_ref()) else {
        return (-1, plan.summary.clone());
    };
    let body = message_text(plan, text);
    let cmd = vec![bin.clone(), "agent".into(), "prompt".into(), pane.pane_id.clone(), body];
    let (rc, out) = match Command::new(&cmd[0]).args(&cmd[1..]).stdin(Stdio::null()).output() {
        Ok(o) => (
            o.status.code().unwrap_or(-1),
            format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_string(),
        ),
        Err(e) => (-1, e.to_string()),
    };
    let logged = Plan {
        action: "message".into(),
        agent: plan.to.clone(),
        ok: true,
        summary: plan.summary.clone(),
        cmd,
        cwd: None,
        details: vec![],
        more: vec![],
    };
    log(env, &logged, rc, &out);
    (rc, out)
}
