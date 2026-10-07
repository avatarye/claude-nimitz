//! `nimitz hook <Event>` logic: append an Event; for PreToolUse SendMessage, decide
//! allow/deny from the comms rule (contracts.md sections 5, plan-v3 section 7);
//! when a ship blocks on the user (AskUserQuestion, PermissionRequest,
//! a blocking Notification), record a needs-you notice for its commodore
//! (`notify`). Owner: core-sessions.
//!
//! The hook runs inside every Claude Code session, so it must be fast and must
//! never fail the session: errors are swallowed, the exit code is the CLI's job
//! (always 0). One `write` of one line with `O_APPEND`, so concurrent hooks never
//! interleave. PreToolUse/PostToolUse are logged only for `SendMessage` and
//! `AskUserQuestion` (they fire for every tool call; nothing else reads them).
use crate::env::Env;
use crate::types::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;

const LABELS: [&str; 5] = ["NIMITZ_FLEET", "NIMITZ_SQUADRON", "NIMITZ_ROLE", "NIMITZ_ID", "NIMITZ_HOME"];

/// The per-turn events that hand a running member a rules update
/// (`rules::update_for`, once per version per job, never per prompt_id: a
/// mid-turn cross-session message repeats the running turn's prompt_id).
/// UserPromptSubmit fires for every turn start of a background session (a
/// typed prompt, a cross-session message, a task notification; plan-v4
/// section 12.1), so nothing else is needed; a second event would go here,
/// with its entry in the plugin's hooks.json.
pub const UPDATE_EVENTS: &[&str] = &["UserPromptSubmit"];

/// `out` (empty, or a decision JSON) with `additionalContext` added to its
/// `hookSpecificOutput`, so a rules update rides along with a deny.
fn with_context(out: &str, event: &str, text: &str) -> String {
    let mut v: Value = serde_json::from_str(out).unwrap_or(Value::Null);
    if !v.is_object() {
        v = json!({"hookSpecificOutput": {"hookEventName": event}});
    }
    v["hookSpecificOutput"]["additionalContext"] = Value::from(text);
    v.to_string()
}

/// Handle one hook call. `stdin` is Claude Code's hook JSON; `vars` the hook's env.
/// Returns the text to print on stdout (empty, or a decision JSON). Never panics.
pub fn handle(env: &Env, event: &str, stdin: &str, vars: &HashMap<String, String>) -> String {
    handle_full(env, event, stdin, vars).0
}

/// `handle`, plus the needs-you notice this call recorded (the caller delivers
/// it without blocking the session). Never panics.
pub fn handle_full(env: &Env, event: &str, stdin: &str, vars: &HashMap<String, String>) -> (String, Option<crate::notify::Notice>) {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle_inner(env, event, stdin, vars))).unwrap_or_default()
}

fn handle_inner(env: &Env, event: &str, stdin: &str, vars: &HashMap<String, String>) -> (String, Option<crate::notify::Notice>) {
    let input: Value = serde_json::from_str(stdin.trim()).unwrap_or(Value::Null);
    let mut labels: BTreeMap<String, String> =
        LABELS.iter().filter_map(|k| vars.get(*k).filter(|v| !v.is_empty()).map(|v| (k.to_string(), v.clone()))).collect();
    // An inherited NIMITZ_HOME (a user who exports it in their shell) is not a
    // label: such a session stays unlabelled, a stowaway (contracts.md section 7).
    if !crate::evidence::is_labelled(&labels) {
        labels.clear();
    }
    let tool = str_at(&input, "tool_name");
    let is_signal = tool.as_deref() == Some("SendMessage");
    let to = input.get("tool_input").and_then(|t| str_at(t, "to").or_else(|| str_at(t, "recipient")));

    let mut out = String::new();
    let mut reason = str_at(&input, "reason");
    if event == "PreToolUse" && is_signal {
        // The sender is its NIMITZ_ID only when NIMITZ_ROLE agrees with the
        // id's shape (as `sessions::caller_from_env` judges it): a 0.3.x tender
        // (`F/SQ/~task`, role `tender`) is no sender with a role.
        let sender = match crate::sessions::caller_from_env(&labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect()) {
            Caller::Member { id, .. } if id.0 != crate::sessions::UNVERIFIED => Some(id),
            _ => None,
        };
        let verdict = match (to.as_deref(), sender) {
            (Some(target), Some(sender)) => Some(may_signal(env, &sender, target)),
            // labels without an id, or that disagree (partial): fail closed for fleet targets
            (Some(target), None) if !labels.is_empty() && (crate::sessions::id_from_name(target).is_some() || target.contains(':')) => {
                Some(Err("the sender's NIMITZ_* labels are partial (no NIMITZ_ID, or NIMITZ_ROLE does not match it); nimitz cannot judge this signal".to_string()))
            }
            _ => None,
        };
        if let Some(Err(why)) = verdict {
            reason = Some(format!("denied: {why}"));
            out = json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": why,
            }})
            .to_string();
        }
    }
    // A ship now blocks on the user: record it for its commodore.
    // Moving on (a prompt, a stop, the end) closes its notice.
    let mut notice = None;
    if let Some(n) = crate::notify::from_hook(event, &input, &labels) {
        reason = Some(format!("needs-you: {}", n.kind));
        notice = crate::notify::record(env, n);
    } else if matches!(event, "UserPromptSubmit" | "Stop" | "SessionEnd") {
        if let Some(id) = labels.get("NIMITZ_ID") {
            crate::notify::clear(env, id, str_at(&input, "session_id").as_deref());
        }
    }
    // A labelled session starts (startup, resume, clear, compact): hand it its
    // role's rules and record the delivery. On a per-turn event, a session
    // that runs on older rules gets the update (once per version per job).
    let mut rules = None;
    let member = labels.get("NIMITZ_ID").map(|i| Id(i.clone())).filter(|i| labels.get("NIMITZ_ROLE").map(String::as_str) == Some(i.role().as_str()));
    let ppid = unsafe { libc::getppid() } as i64;
    if let Some(id) = member {
        let sid = str_at(&input, "session_id").unwrap_or_default();
        if event == "SessionStart" {
            rules = Some(crate::rules::VERSION);
            crate::rules::record_delivery(env, &crate::rules::delivery_key(env, ppid, &sid), &sid, event);
            out = json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": crate::rules::text(&id)}}).to_string();
        } else if UPDATE_EVENTS.contains(&event) {
            if let Some(text) = crate::rules::update_for(env, &id, &crate::rules::delivery_key(env, ppid, &sid), &sid, event) {
                rules = Some(crate::rules::VERSION);
                out = with_context(&out, event, &text);
            }
        }
    }
    let tool_event = matches!(event, "PreToolUse" | "PostToolUse" | "PostToolUseFailure");
    if !tool_event || is_signal || tool.as_deref() == Some("AskUserQuestion") {
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
            "pid": ppid,
            "labels": labels,
            "rules": rules,
        });
        append_line(env, &line.to_string());
    }
    (out, notice)
}

fn str_at(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_string)
}

/// events.jsonl is compacted when it grows past this (BUG-44): a status reads
/// the whole file, and must stay fast.
pub const EVENTS_MAX_BYTES: u64 = 4 * 1024 * 1024;
/// What a compaction keeps besides every session's newest SessionStart.
pub const EVENTS_KEEP_RECENT: usize = 2000;

fn flock(f: &std::fs::File, op: i32) {
    use std::os::unix::io::AsRawFd;
    unsafe {
        libc::flock(f.as_raw_fd(), op);
    }
}

fn append_line(env: &Env, line: &str) {
    let path = env.events_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // appends share the lock; a compaction takes it alone
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path.with_extension("lock")).ok();
    if let Some(l) = &lock {
        flock(l, libc::LOCK_SH);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).read(true).open(&path) {
        // a line left half-written (full disk) must not swallow this one (BUG-49)
        let lead = match f.metadata().map(|m| m.len()).unwrap_or(0) {
            0 => "",
            n => {
                use std::io::{Read, Seek, SeekFrom};
                let mut last = [0u8; 1];
                let ok = f.seek(SeekFrom::Start(n - 1)).is_ok() && f.read_exact(&mut last).is_ok();
                if ok && last[0] != b'\n' { "\n" } else { "" }
            }
        };
        let _ = f.write_all(format!("{lead}{line}\n").as_bytes());
        let big = f.metadata().map(|m| m.len() > EVENTS_MAX_BYTES).unwrap_or(false);
        drop(f);
        if let (true, Some(l)) = (big, &lock) {
            flock(l, libc::LOCK_UN);
            flock(l, libc::LOCK_EX);
            compact_events(&path);
        }
    }
}

/// Keep every session's newest SessionStart (the binding and the labels) and
/// its newest SessionEnd, plus the last EVENTS_KEEP_RECENT lines; write a temp
/// file and rename it over events.jsonl. Called under the exclusive lock.
pub fn compact_events(path: &std::path::Path) {
    let Ok(raw) = std::fs::read(path) else { return };
    if (raw.len() as u64) <= EVENTS_MAX_BYTES {
        return; // someone else compacted it while we waited
    }
    let text = String::from_utf8_lossy(&raw);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let recent_from = lines.len().saturating_sub(EVENTS_KEEP_RECENT);
    let mut newest: std::collections::HashMap<(String, String), usize> = Default::default();
    for (i, l) in lines.iter().enumerate().take(recent_from) {
        if let Some(e) = crate::evidence::parse_event_line(l) {
            if matches!(e.event.as_str(), "SessionStart" | "SessionEnd") {
                if let Some(sid) = e.session_id {
                    newest.insert((sid, e.event), i);
                }
            }
        }
    }
    let mut keep: Vec<usize> = newest.into_values().collect();
    keep.extend(recent_from..lines.len());
    keep.sort_unstable();
    keep.dedup();
    let mut out = String::with_capacity(keep.len() * 200);
    for i in keep {
        out.push_str(lines[i]);
        out.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    if std::fs::write(&tmp, out).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// A `uds:<socket>` address (real Claude Code 2.1.288 delivers SendMessage to a
/// session's socket): the name of the session that owns the socket, from the
/// session files (`messagingSocketPath`, or the pid in `<pid>.sock`).
fn resolve_socket(env: &Env, to: &str) -> Option<String> {
    let path = to.strip_prefix("uds:")?;
    let pid: Option<i64> = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse().ok());
    for e in std::fs::read_dir(env.claude_home.join("sessions")).ok()?.filter_map(|e| e.ok()) {
        let Some(v) = std::fs::read(e.path()).ok().and_then(|b| crate::evidence::first_json(&b)) else { continue };
        let sock = v.get("messagingSocketPath").and_then(|x| x.as_str());
        let vpid = v.get("pid").and_then(|x| x.as_i64());
        if sock == Some(path) || (pid.is_some() && vpid == pid) {
            return v.get("name").and_then(|x| x.as_str()).map(str::to_string);
        }
    }
    None
}

/// The chain-of-command signal table (plan-v4 section 12): an admiral, every
/// member of its own fleet; a commodore, its admiral and its ships; a ship,
/// its commodore, its admiral (a reply) and other ships as the squadron's
/// comms rule allows. Targets that are not fleet names (teammates of an agent
/// team, other sessions) are not ours to rule, except an address nimitz
/// cannot resolve (a socket, BUG-22): a ship under a closed comms rule may
/// not signal what it cannot name.
pub fn may_signal(env: &Env, from: &Id, to_name: &str) -> Result<(), String> {
    if to_name.contains(':') && !to_name.starts_with('[') {
        let resolved = resolve_socket(env, to_name);
        if let Some(name) = resolved.as_deref().filter(|n| crate::sessions::id_from_name(n).is_some()) {
            return may_signal(env, from, name);
        }
        let closed = match from.role() {
            Role::Ship => !matches!(squadron_comms(env, from), Comms::Mode(m) if m == "open"),
            _ => false,
        };
        return if closed {
            Err(format!(
                "{to_name} is not a fleet address nimitz can resolve{}; a {} under this squadron's comms rule signals only named members (nimitz chain of command; sender {})",
                resolved.map(|n| format!(" (it is {n:?})")).unwrap_or_default(),
                from.role().as_str(),
                from.name()
            ))
        } else {
            Ok(())
        };
    }
    let Some(to) = crate::sessions::id_from_name(to_name) else { return Ok(()) };
    signal_table(from, &to, &|| squadron_comms(env, from))
}

/// The table itself, for two fleet ids; `comms` gives the sender's squadron
/// rule when a ship signals a ship (read only then).
pub fn signal_table(from: &Id, to: &Id, comms: &dyn Fn() -> Comms) -> Result<(), String> {
    let to_name = to.name();
    let deny = |why: String| Err(format!("{why} (nimitz chain of command; sender {})", from.name()));
    let same_sq = to.fleet() == from.fleet() && to.squadron() == from.squadron();
    match from.role() {
        Role::Admiral => match to.role() {
            _ if to.fleet() == from.fleet() && to != from => Ok(()),
            _ => deny(format!("an admiral signals the members of its own fleet, not {to_name}")),
        },
        Role::Commodore => match to.role() {
            Role::Admiral if to.fleet() == from.fleet() => Ok(()),
            Role::Ship if same_sq => Ok(()),
            _ => deny(format!("a commodore signals its admiral and its ships, not {to_name}")),
        },
        Role::Ship => match to.role() {
            Role::Commodore if same_sq => Ok(()),
            // a reply: the admiral may signal a ship, and the ship answers it
            Role::Admiral if to.fleet() == from.fleet() => Ok(()),
            Role::Ship => match comms() {
                Comms::Mode(m) if m == "open" && same_sq => Ok(()),
                Comms::Mode(m) if m == "open" => deny(format!("comms is open within the squadron only; {to_name} is in another squadron")),
                Comms::Mode(m) if m == "commodore-only" => {
                    deny(format!("squadron comms is commodore-only; signal your commodore, not {to_name}"))
                }
                Comms::Allow { allow } if allow.iter().any(|a| *a == to.0 || *a == to.name()) => Ok(()),
                Comms::Allow { .. } => deny(format!("{to_name} is not in the squadron's comms allow list")),
                Comms::Mode(m) => deny(format!("unknown comms rule {m:?}; only the commodore may be signalled")),
            },
            _ => deny(format!("a ship signals its commodore, its admiral (a reply) and, by the comms rule, other ships; not {to_name}")),
        },
    }
}

/// The sender's squadron `comms` rule; on any doubt the strictest rule.
fn squadron_comms(env: &Env, from: &Id) -> Comms {
    let strict = Comms::Mode("commodore-only".into());
    let Ok(f) = crate::config::load_fleet(env, from.fleet()) else { return strict };
    f.squadrons.into_iter().find(|(_, s)| Some(s.squadron.as_str()) == from.squadron()).map(|(_, s)| s.comms).unwrap_or(strict)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::{make_fleet, test_env};

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn labels_of(env: &Env, id: &Id) -> HashMap<String, String> {
        crate::claude::labels_for(env, id).into_iter().collect()
    }

    fn send(env: &Env, from: &Id, to: &str) -> String {
        let input = serde_json::json!({
            "session_id": "s1", "hook_event_name": "PreToolUse", "tool_name": "SendMessage",
            "tool_input": {"to": to, "message": "status?"}, "scratchpad_dir": "/x",
        });
        handle(env, "PreToolUse", &input.to_string(), &labels_of(env, from))
    }

    fn denied(out: &str) -> bool {
        serde_json::from_str::<Value>(out)
            .ok()
            .and_then(|v| v["hookSpecificOutput"]["permissionDecision"].as_str().map(str::to_string))
            .as_deref()
            == Some("deny")
    }

    /// MAIN with squadron ARGOS (comms open) and ship ARGOS; plus BOREAL, comms
    /// commodore-only, with ships ONE and TWO.
    fn world() -> (tempfile::TempDir, Env) {
        let (td, env) = test_env();
        make_fleet(&env);
        let plan = crate::config::plan_squadron_new(&env, "MAIN", "BOREAL", "Quiet squadron.", Comms::Mode("commodore-only".into()));
        crate::config::tests::apply_ok(&env, &plan);
        for name in ["ONE", "TWO"] {
            let ship = ShipDef {
                name: name.into(),
                prompt: Some(format!("You are {name}.")),
                dirs: [("test-mac".to_string(), "managed".to_string())].into(),
                ..Default::default()
            };
            crate::config::tests::apply_ok(&env, &crate::config::plan_ship_new(&env, "MAIN", "BOREAL", ship));
        }
        (td, env)
    }

    #[test]
    fn the_signal_table_holds() {
        let (_td, env) = world();
        let adm = Id::admiral("MAIN");
        let argos = Id::commodore("MAIN", "ARGOS");
        let ship = Id::ship("MAIN", "ARGOS", "ARGOS");
        let one = Id::ship("MAIN", "BOREAL", "ONE");

        // an admiral signals every member of its own fleet (commodores and ships), and nothing else
        assert!(!denied(&send(&env, &adm, &argos.name())));
        assert!(!denied(&send(&env, &adm, &ship.name())), "0.4.0: the admiral reaches its ships");
        assert!(!denied(&send(&env, &adm, &one.name())));
        assert!(denied(&send(&env, &adm, &Id::commodore("OTHER", "SQ").name())));
        assert!(denied(&send(&env, &adm, &Id::ship("OTHER", "SQ", "S").name())));
        assert!(denied(&send(&env, &adm, &adm.name())), "not itself");
        // a commodore: its admiral, its ships
        assert!(!denied(&send(&env, &argos, &adm.name())));
        assert!(!denied(&send(&env, &argos, &ship.name())));
        assert!(denied(&send(&env, &argos, &one.name())), "not a ship of another squadron");
        // a 0.3.x tender's labels (`~task` id, role `tender`) give no authority at
        // all: denied to its commodore and to a sibling ship alike
        let tender: HashMap<String, String> = [("NIMITZ_FLEET", "MAIN"), ("NIMITZ_SQUADRON", "ARGOS"), ("NIMITZ_ROLE", "tender"), ("NIMITZ_ID", "MAIN/ARGOS/~fix"), ("NIMITZ_HOME", "/h")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        for to in [argos.name(), ship.name(), "sock:/tmp/x".to_string()] {
            let input = serde_json::json!({"session_id": "s1", "hook_event_name": "PreToolUse", "tool_name": "SendMessage", "tool_input": {"to": to, "message": "done"}});
            let out = handle(&env, "PreToolUse", &input.to_string(), &tender);
            assert!(denied(&out) && out.contains("NIMITZ_ROLE does not match"), "{to}: {out}");
        }
        // a ship: its commodore always, and its admiral (a reply)
        assert!(!denied(&send(&env, &ship, &argos.name())));
        assert!(!denied(&send(&env, &ship, &adm.name())), "0.4.0: a ship replies to its admiral");
        assert!(denied(&send(&env, &ship, &Id::admiral("OTHER").name())), "not another fleet's admiral");
        assert!(denied(&send(&env, &ship, &Id::commodore("MAIN", "BOREAL").name())), "not another squadron's commodore");
        // a `[ref]` suffix (two sessions with one name) does not get around the table
        assert!(denied(&send(&env, &ship, &format!("{} [3fa9c1]", Id::commodore("MAIN", "BOREAL").name()))));
        assert!(!denied(&send(&env, &ship, &format!("{} [3fa9c1]", argos.name()))));
    }

    /// A SessionStart delivers the rules and records it; a prompt in a session
    /// that runs on older rules gets the update once; a current one gets nothing.
    #[test]
    fn a_running_member_gets_a_rules_update_at_its_next_prompt() {
        let (_td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let prompt = serde_json::json!({"session_id": "s-up", "hook_event_name": "UserPromptSubmit", "prompt": "go on"}).to_string();
        // an older nimitz started it: a SessionStart line with rules v1, no record
        let line = serde_json::json!({"ts": 1.0, "event": "SessionStart", "session_id": "s-up", "source": "startup", "pid": 1,
            "labels": crate::claude::labels_for(&env, &id), "rules": 1});
        std::fs::create_dir_all(&env.state).unwrap();
        std::fs::write(env.events_path(), format!("{line}\n")).unwrap();
        let out = handle(&env, "UserPromptSubmit", &prompt, &labels_of(&env, &id));
        let v: Value = serde_json::from_str(&out).expect("a hook JSON");
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
        assert!(ctx.starts_with(&format!("nimitz rules updated v1\u{2192}v{}", crate::rules::VERSION)), "{ctx}");
        assert!(ctx.contains("- v2: the tender role is gone") && ctx.contains(&crate::rules::text(&id)), "changelog and the full text: {ctx}");
        assert!(!ctx.contains("- v1:"), "only the entries after v1");
        let d = crate::rules::delivered(&env, "session:s-up").expect("recorded by session id: no job here");
        assert_eq!((d.version, d.via.as_str(), d.session_id.as_str()), (crate::rules::VERSION, "UserPromptSubmit", "s-up"));
        assert_eq!(crate::evidence::events(&env).last().unwrap().rules, Some(crate::rules::VERSION), "the line says what went out");
        // the next prompt: nothing (once per version per key, whatever the prompt_id)
        assert!(handle(&env, "UserPromptSubmit", &prompt, &labels_of(&env, &id)).is_empty());
        let other_turn = serde_json::json!({"session_id": "s-up", "hook_event_name": "UserPromptSubmit", "prompt": "<cross-session-message from-name=\"x\">hi</cross-session-message>", "prompt_id": "p2"}).to_string();
        assert!(handle(&env, "UserPromptSubmit", &other_turn, &labels_of(&env, &id)).is_empty());
        // a SessionStart of any source (compact, clear, resume) re-delivers the full rules and resets the record
        for source in ["compact", "clear", "resume"] {
            let start = serde_json::json!({"session_id": "s-up", "source": source}).to_string();
            assert!(handle(&env, "SessionStart", &start, &labels_of(&env, &id)).contains("additionalContext"), "{source}");
            assert_eq!(crate::rules::delivered(&env, "session:s-up").unwrap().via, "SessionStart", "{source}");
        }
        // a session that starts now is current, so its prompts get nothing either
        let start = serde_json::json!({"session_id": "s-new", "source": "startup"}).to_string();
        handle(&env, "SessionStart", &start, &labels_of(&env, &id));
        assert_eq!(crate::rules::delivered(&env, "session:s-new").unwrap().via, "SessionStart");
        let prompt2 = prompt.replace("s-up", "s-new");
        assert!(handle(&env, "UserPromptSubmit", &prompt2, &labels_of(&env, &id)).is_empty());
        // no SessionStart line and no record: v0, the whole changelog
        let prompt3 = prompt.replace("s-up", "s-none");
        let out = handle(&env, "UserPromptSubmit", &prompt3, &labels_of(&env, &id));
        assert!(out.contains("updated v0\\u2192") || out.contains("updated v0→"), "{out}");
        // an unlabelled session never gets one
        assert!(handle(&env, "UserPromptSubmit", &prompt.replace("s-up", "s-user"), &vars(&[])).is_empty());
    }

    /// The record is keyed by the job id when the hook's parent runs as a job
    /// (`sessions/<pid>.json` `jobId`), so it follows a resume or /clear.
    #[test]
    fn a_delivery_is_keyed_by_the_job_when_there_is_one() {
        let (_td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let ppid = unsafe { libc::getppid() } as i64;
        let sessions = env.claude_home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join(format!("{ppid}.json")), r#"{"pid": 1, "sessionId": "s-a", "jobId": "9cdceb2d", "kind": "bg"}"#).unwrap();
        handle(&env, "SessionStart", r#"{"session_id": "s-a", "source": "startup"}"#, &labels_of(&env, &id));
        let d = crate::rules::delivered(&env, "9cdceb2d").expect("keyed by the job");
        assert_eq!((d.version, d.session_id.as_str()), (crate::rules::VERSION, "s-a"));
        assert!(crate::rules::delivered(&env, "session:s-a").is_none());
        // /clear moved the job to a new session id: the record still says current
        let prompt = serde_json::json!({"session_id": "s-b", "hook_event_name": "UserPromptSubmit"}).to_string();
        assert!(handle(&env, "UserPromptSubmit", &prompt, &labels_of(&env, &id)).is_empty());
        assert_eq!(crate::rules::delivery_key(&env, ppid, "s-b"), "9cdceb2d");
        assert_eq!(crate::rules::delivery_key(&env, 1, "s-b"), "session:s-b", "no session file: by session id");
    }

    /// A rules update rides along with a PreToolUse deny when PreToolUse is an
    /// update event (it is not yet; `with_context` is what makes it one line).
    #[test]
    fn a_decision_and_a_rules_update_share_one_output() {
        let deny = serde_json::json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": "no"}}).to_string();
        let v: Value = serde_json::from_str(&with_context(&deny, "PreToolUse", "rules")).unwrap();
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(v["hookSpecificOutput"]["additionalContext"], "rules");
        let v: Value = serde_json::from_str(&with_context("", "UserPromptSubmit", "rules")).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
        assert_eq!(v["hookSpecificOutput"]["additionalContext"], "rules");
    }

    #[test]
    fn the_comms_rule_decides_ship_to_ship() {
        let (_td, env) = world();
        let one = Id::ship("MAIN", "BOREAL", "ONE");
        let two = Id::ship("MAIN", "BOREAL", "TWO");
        let argos_ship = Id::ship("MAIN", "ARGOS", "ARGOS");

        // BOREAL is commodore-only
        let out = send(&env, &one, &two.name());
        assert!(denied(&out));
        assert!(out.contains("commodore-only"), "{out}");
        // ARGOS is open, but only inside the squadron
        assert!(denied(&send(&env, &argos_ship, &two.name())), "open means open within the squadron");

        // an allow list may name a ship of another squadron
        let sq_path = crate::config::load_fleet(&env, "MAIN")
            .unwrap()
            .squadrons
            .into_iter()
            .find(|(_, s)| s.squadron == "BOREAL")
            .map(|(d, _)| d.join("squadron.toml"))
            .unwrap();
        let raw = std::fs::read_to_string(&sq_path).unwrap();
        std::fs::write(&sq_path, raw.replace("comms = \"commodore-only\"", "comms = { allow = [\"MAIN/ARGOS/ARGOS\"] }")).unwrap();
        assert!(!denied(&send(&env, &one, &argos_ship.name())), "the allow list lets it through");
        assert!(denied(&send(&env, &one, &two.name())), "and nothing else");
    }

    #[test]
    fn a_target_that_is_not_a_fleet_address_is_not_ours_to_rule() {
        let (_td, env) = world();
        let ship = Id::ship("MAIN", "ARGOS", "ARGOS");
        // teammates and ad-hoc sessions are the session's own business
        assert!(send(&env, &ship, "reviewer").is_empty());
        assert!(send(&env, &ship, "[ADMIRALTY]").is_empty());
        // and an unlabelled sender is never judged
        let input = serde_json::json!({"tool_name": "SendMessage", "tool_input": {"to": "[MAIN]-ADMIRAL"}});
        assert!(handle(&env, "PreToolUse", &input.to_string(), &vars(&[])).is_empty());
    }

    #[test]
    fn a_broken_config_falls_back_to_the_strictest_rule() {
        let (_td, env) = world();
        let one = Id::ship("MAIN", "BOREAL", "ONE");
        let two = Id::ship("MAIN", "BOREAL", "TWO");
        std::fs::write(env.fleet_dir("MAIN").join("fleet.toml"), "fleet = ").unwrap();
        assert!(denied(&send(&env, &one, &two.name())));
    }

    #[test]
    fn every_event_is_appended_as_one_line_with_its_labels() {
        let (_td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let start = serde_json::json!({"session_id": "s9", "source": "resume", "hook_event_name": "SessionStart"});
        // a labelled start gets its role's rules as context, and the line says which version
        let out: Value = serde_json::from_str(&handle(&env, "SessionStart", &start.to_string(), &labels_of(&env, &id))).unwrap();
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "SessionStart");
        assert_eq!(out["hookSpecificOutput"]["additionalContext"], crate::rules::text(&id));
        // a tool call that is not a signal is not logged (they fire for every tool)
        let read = serde_json::json!({"session_id": "s9", "tool_name": "Read"});
        handle(&env, "PreToolUse", &read.to_string(), &labels_of(&env, &id));
        // a denied signal is logged with its reason
        let _ = send(&env, &id, &Id::admiral("OTHER").name());

        let events = crate::evidence::events(&env);
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0].event, "SessionStart");
        assert_eq!(events[0].session_id.as_deref(), Some("s9"));
        assert_eq!(events[0].source.as_deref(), Some("resume"));
        assert_eq!(events[0].labels.get("NIMITZ_ID").map(String::as_str), Some("MAIN/ARGOS/ARGOS"));
        assert_eq!(events[0].rules, Some(crate::rules::VERSION));
        assert!(events[0].pid.unwrap_or(0) > 0, "the pid is the Claude Code process (the hook's parent)");
        assert_eq!(events[1].tool_name.as_deref(), Some("SendMessage"));
        assert!(events[1].reason.as_deref().unwrap_or("").starts_with("denied:"), "{:?}", events[1].reason);
    }

    #[test]
    fn an_unlabelled_or_mislabelled_start_gets_no_rules() {
        let (_td, env) = world();
        let start = r#"{"session_id": "s1", "source": "startup"}"#;
        assert!(handle(&env, "SessionStart", start, &vars(&[])).is_empty(), "the user's own session");
        let wrong = vars(&[("NIMITZ_ID", "MAIN/ARGOS/ARGOS"), ("NIMITZ_ROLE", "commodore")]);
        assert!(handle(&env, "SessionStart", start, &wrong).is_empty(), "role and id disagree");
        assert_eq!(crate::evidence::events(&env).last().unwrap().rules, None);
    }

    #[test]
    fn an_inherited_nimitz_home_alone_is_not_a_label() {
        let (_td, env) = world();
        let start = serde_json::json!({"session_id": "s7", "source": "startup"});
        handle(&env, "SessionStart", &start.to_string(), &vars(&[("NIMITZ_HOME", "/h"), ("NIMITZ_FLEET", "MAIN")]));
        let events = crate::evidence::events(&env);
        assert!(events[0].labels.is_empty(), "{:?}", events[0].labels);
        // and an older line that logged it anyway reads as unlabelled
        let old = vec![Event { labels: [("NIMITZ_HOME".to_string(), "/h".to_string())].into(), ..events[0].clone() }];
        assert!(crate::evidence::labels_by_session(&old).values().all(|l| !crate::evidence::is_labelled(l)));
    }

    #[test]
    fn a_half_written_line_does_not_swallow_the_next_event() {
        let (_td, env) = world();
        std::fs::create_dir_all(env.state.clone()).unwrap();
        std::fs::write(env.events_path(), "{\"ts\": 1, \"event\": \"Sto").unwrap(); // disk was full
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        handle(&env, "SessionStart", r#"{"session_id": "s1", "source": "startup"}"#, &labels_of(&env, &id));
        let events = crate::evidence::events(&env);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn a_big_event_log_is_compacted_and_keeps_every_binding() {
        let (_td, env) = world();
        std::fs::create_dir_all(env.state.clone()).unwrap();
        let mut body = String::new();
        let n = 40_000;
        for i in 0..n {
            let ev = if i % 20 == 0 { "SessionStart" } else { "Stop" };
            body.push_str(&format!(
                "{{\"ts\": {i}.0, \"event\": \"{ev}\", \"session_id\": \"s{}\", \"pid\": 1, \"labels\": {{\"NIMITZ_ID\": \"MAIN/ARGOS/S{}\", \"NIMITZ_ROLE\": \"ship\"}}, \"pad\": \"{}\"}}\n",
                i / 20, i / 20, "x".repeat(60)
            ));
        }
        std::fs::write(env.events_path(), &body).unwrap();
        assert!(body.len() as u64 > EVENTS_MAX_BYTES);
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        handle(&env, "Stop", r#"{"session_id": "last"}"#, &labels_of(&env, &id));
        let size = std::fs::metadata(env.events_path()).unwrap().len();
        assert!(size < EVENTS_MAX_BYTES / 2, "compacted to {size} bytes");
        let events = crate::evidence::events(&env);
        let labels = crate::evidence::labels_by_session(&events);
        for s in [0, 1, 500, n / 20 - 1] {
            assert_eq!(labels.get(&format!("s{s}")).and_then(|l| l.get("NIMITZ_ID")).map(String::as_str), Some(format!("MAIN/ARGOS/S{s}").as_str()), "binding of s{s} kept");
        }
        assert_eq!(events.last().unwrap().session_id.as_deref(), Some("last"));
    }

    #[test]
    fn a_hook_never_fails_the_session() {
        let (_td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        // malformed stdin, unknown event, missing fields: no panic, no output
        for stdin in ["", "not json", "[]", r#"{"tool_name": 7, "tool_input": {"to": 3}}"#] {
            assert!(handle(&env, "PreToolUse", stdin, &labels_of(&env, &id)).is_empty(), "{stdin}");
        }
        assert!(handle(&env, "SomeNewEventInANewVersion", "{}", &labels_of(&env, &id)).is_empty());
        // an unwritable state dir loses the line but still returns
        let bad = Env { state: std::path::PathBuf::from("/dev/null/nope"), ..env.clone() };
        assert!(handle(&bad, "SessionStart", "{}", &labels_of(&env, &id)).contains("additionalContext"), "the rules still go out");
    }

    #[test]
    fn concurrent_hooks_do_not_interleave_their_lines() {
        let (_td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let n = 40;
        std::thread::scope(|s| {
            for i in 0..n {
                let (env, id) = (env.clone(), id.clone());
                s.spawn(move || {
                    let input = serde_json::json!({"session_id": format!("s{i}"), "hook_event_name": "Stop"});
                    handle(&env, "Stop", &input.to_string(), &labels_of(&env, &id));
                });
            }
        });
        let events = crate::evidence::events(&env);
        assert_eq!(events.len(), n, "every line is whole and readable");
        let mut ids: Vec<String> = events.iter().filter_map(|e| e.session_id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }
}
