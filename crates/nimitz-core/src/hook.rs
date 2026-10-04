//! `nimitz hook <Event>` logic: append an Event; for PreToolUse SendMessage, decide
//! allow/deny from the comms rule (contracts.md sections 5, plan-v3 section 7). Owner: core-sessions.
//!
//! The hook runs inside every Claude Code session, so it must be fast and must
//! never fail the session: errors are swallowed, the exit code is the CLI's job
//! (always 0). One `write` of one line with `O_APPEND`, so concurrent hooks never
//! interleave. PreToolUse/PostToolUse are logged only for `SendMessage` (they fire
//! for every tool call; nothing else reads them).
use crate::env::Env;
use crate::types::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;

const LABELS: [&str; 6] = ["NIMITZ_FLEET", "NIMITZ_SQUADRON", "NIMITZ_ROLE", "NIMITZ_ID", "NIMITZ_PARENT", "NIMITZ_HOME"];

/// Handle one hook call. `stdin` is Claude Code's hook JSON; `vars` the hook's env.
/// Returns the text to print on stdout (empty, or a decision JSON). Never panics.
pub fn handle(env: &Env, event: &str, stdin: &str, vars: &HashMap<String, String>) -> String {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle_inner(env, event, stdin, vars))).unwrap_or_default()
}

fn handle_inner(env: &Env, event: &str, stdin: &str, vars: &HashMap<String, String>) -> String {
    let input: Value = serde_json::from_str(stdin.trim()).unwrap_or(Value::Null);
    let labels: BTreeMap<String, String> =
        LABELS.iter().filter_map(|k| vars.get(*k).filter(|v| !v.is_empty()).map(|v| (k.to_string(), v.clone()))).collect();
    let tool = str_at(&input, "tool_name");
    let is_signal = tool.as_deref() == Some("SendMessage");
    let to = input.get("tool_input").and_then(|t| str_at(t, "to").or_else(|| str_at(t, "recipient")));

    let mut out = String::new();
    let mut reason = str_at(&input, "reason");
    if event == "PreToolUse" && is_signal {
        if let (Some(target), Some(sender)) = (to.as_deref(), labels.get("NIMITZ_ID")) {
            if let Err(why) = may_signal(env, &Id(sender.clone()), target) {
                reason = Some(format!("denied: {why}"));
                out = json!({"hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": why,
                }})
                .to_string();
            }
        }
    }
    let tool_event = matches!(event, "PreToolUse" | "PostToolUse" | "PostToolUseFailure");
    if !tool_event || is_signal {
        let line = json!({
            "ts": crate::evidence::now(),
            "event": event,
            "session_id": str_at(&input, "session_id"),
            "source": str_at(&input, "source"),
            "reason": reason,
            "agent_id": str_at(&input, "agent_id"),
            "agent_type": str_at(&input, "agent_type"),
            "tool_name": tool,
            "to": to,
            "pid": unsafe { libc::getppid() } as i64,
            "labels": labels,
        });
        append_line(env, &line.to_string());
    }
    out
}

fn str_at(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_string)
}

fn append_line(env: &Env, line: &str) {
    let path = env.events_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

/// The chain-of-command signal table (plan-v3 section 7). Targets that are not
/// fleet names (teammates of an agent team, other sessions) are not ours to rule.
pub fn may_signal(env: &Env, from: &Id, to_name: &str) -> Result<(), String> {
    let Some(to) = crate::sessions::id_from_name(to_name) else { return Ok(()) };
    let deny = |why: String| Err(format!("{why} (nimitz chain of command; sender {})", from.name()));
    let same_sq = to.fleet() == from.fleet() && to.squadron() == from.squadron();
    match from.role() {
        Role::Admiral => match to.role() {
            Role::Commodore if to.fleet() == from.fleet() => Ok(()),
            _ => deny(format!("an admiral signals only its own commodores; ask the commodore of {to_name}")),
        },
        Role::Commodore => match to.role() {
            Role::Admiral if to.fleet() == from.fleet() => Ok(()),
            Role::Ship | Role::Tender if same_sq => Ok(()),
            _ => deny(format!("a commodore signals its admiral, its ships and its tenders, not {to_name}")),
        },
        Role::Tender => match to.role() {
            Role::Commodore if same_sq => Ok(()),
            _ => deny(format!("a tender signals only its commodore, not {to_name}")),
        },
        Role::Ship => match to.role() {
            Role::Commodore if same_sq => Ok(()),
            Role::Ship => match squadron_comms(env, from) {
                Comms::Mode(m) if m == "open" && same_sq => Ok(()),
                Comms::Mode(m) if m == "open" => deny(format!("comms is open within the squadron only; {to_name} is in another squadron")),
                Comms::Mode(m) if m == "commodore-only" => {
                    deny(format!("squadron comms is commodore-only; signal your commodore, not {to_name}"))
                }
                Comms::Allow { allow } if allow.iter().any(|a| *a == to.0 || *a == to.name()) => Ok(()),
                Comms::Allow { .. } => deny(format!("{to_name} is not in the squadron's comms allow list")),
                Comms::Mode(m) => deny(format!("unknown comms rule {m:?}; only the commodore may be signalled")),
            },
            _ => deny(format!("a ship signals its commodore and, by the comms rule, other ships; not {to_name}")),
        },
    }
}

/// The sender's squadron `comms` rule; on any doubt the strictest rule.
fn squadron_comms(env: &Env, from: &Id) -> Comms {
    let strict = Comms::Mode("commodore-only".into());
    let Ok(f) = crate::config::load_fleet(env, from.fleet()) else { return strict };
    f.squadrons.into_iter().find(|(_, s)| Some(s.squadron.as_str()) == from.squadron()).map(|(_, s)| s.comms).unwrap_or(strict)
}
