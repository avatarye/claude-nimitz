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
    // The Claude Code process (the hook's parent); a sandbox names it.
    let pid: i64 = vars.get("NIMITZ_HOOK_PID").and_then(|p| p.parse().ok()).unwrap_or_else(|| unsafe { libc::getppid() } as i64);
    // A labelled session starts (startup, resume, clear, compact, fork): bind
    // it to its identity (plan-v4 section 3) and hand it its role's rules.
    let mut rules = None;
    let mut session = Value::Null;
    let mut binding = None;
    if event == "SessionStart" {
        // another nimitz home's member: not ours, not even a line
        if crate::registry::other_home(env, &labels) {
            return (out, notice);
        }
        session = session_file(env, vars, pid);
        if let Some(id) = crate::registry::identity_of(env, &labels) {
            rules = Some(crate::rules::VERSION);
            out = json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": crate::rules::text(&id)}}).to_string();
            if let Some(sid) = str_at(&input, "session_id") {
                binding = Some(bind_start(env, &id, &input, &session, &sid).as_str());
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
            "pid": pid,
            "labels": labels,
            "rules": rules,
            // SessionStart: the process's job, kind and entrypoint (per pid, M21:
            // the name is read from the session file later, never from here)
            "job": str_at(&session, "jobId"),
            "kind": str_at(&session, "kind"),
            "entrypoint": str_at(&session, "entrypoint"),
            "binding": binding,
        });
        append_line(env, &line.to_string());
    }
    (out, notice)
}

/// The session file of the Claude Code process (`sessions/<pid>.json`, the
/// first JSON value; a sandbox names the file). Null when missing or partial:
/// the hook tolerates it and binds nothing (`jobId` unknown reads as held).
fn session_file(env: &Env, vars: &HashMap<String, String>, pid: i64) -> Value {
    let path = match vars.get("NIMITZ_HOOK_SESSION_FILE").filter(|p| !p.is_empty()) {
        Some(p) => std::path::PathBuf::from(p),
        None => env.claude_home.join("sessions").join(format!("{pid}.json")),
    };
    std::fs::read(path).ok().and_then(|b| crate::evidence::first_json(&b)).unwrap_or(Value::Null)
}

/// Apply the binding table under the registry lock (one locked read-modify-
/// write). A registry that cannot be written binds nothing; `status` replays
/// the line later (`registry::replay`).
fn bind_start(env: &Env, id: &Id, input: &Value, session: &Value, sid: &str) -> crate::registry::Binding {
    let job = str_at(session, "jobId");
    let source = str_at(input, "source");
    let cwd = str_at(input, "cwd").or_else(|| str_at(session, "cwd")).unwrap_or_default();
    let jobs = env.claude_home.join("jobs");
    let now = crate::evidence::now();
    crate::registry::update(env, |r| {
        // the identity's folder: what the registry recorded, else the config
        // (one TOML read, only for an identity that never bound)
        let folder = match r.identities.get(&id.0).map(|x| x.cwd.clone()).filter(|c| !c.is_empty()) {
            Some(c) => Some(std::path::PathBuf::from(c)),
            None => crate::config::load_fleet(env, id.fleet()).ok().and_then(|f| crate::config::member_dir(env, &f, id)),
        };
        crate::registry::apply_start(r, id, &cwd, folder.as_deref(), job.as_deref(), sid, source.as_deref(), now, |j| jobs.join(j).join("state.json").is_file())
    })
    .unwrap_or(crate::registry::Binding::None)
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

/// The chain-of-command signal table (plan-v3 section 7). Targets that are not
/// fleet names (teammates of an agent team, other sessions) are not ours to
/// rule, except an address nimitz cannot resolve (a socket, BUG-22): a ship
/// under a closed comms rule may not signal what it cannot name.
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
    let deny = |why: String| Err(format!("{why} (nimitz chain of command; sender {})", from.name()));
    let same_sq = to.fleet() == from.fleet() && to.squadron() == from.squadron();
    match from.role() {
        Role::Admiral => match to.role() {
            Role::Commodore if to.fleet() == from.fleet() => Ok(()),
            _ => deny(format!("an admiral signals only its own commodores; ask the commodore of {to_name}")),
        },
        Role::Commodore => match to.role() {
            Role::Admiral if to.fleet() == from.fleet() => Ok(()),
            Role::Ship if same_sq => Ok(()),
            _ => deny(format!("a commodore signals its admiral and its ships, not {to_name}")),
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

        // an admiral signals its own commodores and nothing else
        assert!(!denied(&send(&env, &adm, &argos.name())));
        assert!(denied(&send(&env, &adm, &ship.name())));
        assert!(denied(&send(&env, &adm, &Id::commodore("OTHER", "SQ").name())));
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
        // a ship: its commodore always
        assert!(!denied(&send(&env, &ship, &argos.name())));
        assert!(denied(&send(&env, &ship, &adm.name())), "a ship does not signal the admiral");
        // a `[ref]` suffix (two sessions with one name) does not get around the table
        assert!(denied(&send(&env, &ship, &format!("{} [3fa9c1]", adm.name()))));
        assert!(!denied(&send(&env, &ship, &format!("{} [3fa9c1]", argos.name()))));
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

    /// A SessionStart as Claude Code fires it for the process `pid`, whose
    /// session file says `job` (None: interactive or SDK).
    fn start(env: &Env, id: &Id, sid: &str, source: &str, pid: i64, job: Option<&str>, cwd: &str) -> String {
        let file = env.claude_home.join("sessions").join(format!("{pid}.json"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut d = serde_json::json!({"pid": pid, "sessionId": sid, "cwd": cwd, "kind": if job.is_some() { "bg" } else { "interactive" },
                                       "name": id.name(), "status": "idle", "entrypoint": if job.is_some() { "cli" } else { "sdk-cli" }});
        d["jobId"] = job.map(|j| serde_json::json!(j)).unwrap_or(Value::Null);
        std::fs::write(&file, d.to_string()).unwrap();
        let mut vars = labels_of(env, id);
        vars.insert("NIMITZ_HOOK_PID".into(), pid.to_string());
        let input = serde_json::json!({"session_id": sid, "source": source, "hook_event_name": "SessionStart", "cwd": cwd});
        handle(env, "SessionStart", &input.to_string(), &vars)
    }

    fn job_row(env: &Env, short: &str, sid: &str) {
        let p = env.claude_home.join("jobs").join(short).join("state.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::json!({"sessionId": sid, "resumeSessionId": sid, "state": "running"}).to_string()).unwrap();
    }

    #[test]
    fn a_session_start_binds_job_and_session_under_the_lock_per_the_table() {
        let (_td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let cwd = crate::config::load_fleet(&env, "MAIN").ok().and_then(|f| crate::config::member_dir(&env, &f, &id)).unwrap().to_string_lossy().into_owned();
        job_row(&env, "j1", "s1");
        // startup in a job: bound, via launch
        assert!(start(&env, &id, "s1", "startup", 11, Some("j1"), &cwd).contains("additionalContext"), "the rules still go out");
        let rec = crate::registry::load(&env).identities[&id.0].clone();
        assert_eq!((rec.job.as_deref(), rec.session.as_deref()), (Some("j1"), Some("s1")));
        assert_eq!(rec.cwd, cwd);
        assert_eq!(rec.lineage.iter().map(|l| (l.session_id.as_str(), l.job.as_deref(), l.via.as_str())).collect::<Vec<_>>(), [("s1", Some("j1"), "launch")]);
        let e = crate::evidence::events(&env).pop().unwrap();
        assert_eq!((e.pid, e.job.as_deref(), e.kind.as_deref(), e.entrypoint.as_deref(), e.binding.as_deref()), (Some(11), Some("j1"), Some("bg"), Some("cli"), Some("bound")));
        // /clear in the same job: rebound, via clear; a compact of the same id: nothing new
        start(&env, &id, "s2", "clear", 11, Some("j1"), &cwd);
        start(&env, &id, "s2", "compact", 11, Some("j1"), &cwd);
        let rec = crate::registry::load(&env).identities[&id.0].clone();
        assert_eq!(rec.session.as_deref(), Some("s2"));
        assert_eq!(rec.lineage.iter().map(|l| l.via.as_str()).collect::<Vec<_>>(), ["launch", "clear"]);
        assert_eq!(crate::evidence::events(&env).pop().unwrap().binding.as_deref(), Some("same"));
        // a /branch (source fork) in the same job: via branch
        start(&env, &id, "s3", "fork", 11, Some("j1"), &cwd);
        assert_eq!(crate::registry::load(&env).identities[&id.0].lineage.last().unwrap().via, "branch");
        // another job while j1's row exists: dup.job, nothing rebound
        job_row(&env, "j2", "s9");
        start(&env, &id, "s9", "fork", 12, Some("j2"), &cwd);
        let rec = crate::registry::load(&env).identities[&id.0].clone();
        assert_eq!((rec.job.as_deref(), rec.session.as_deref()), (Some("j1"), Some("s3")));
        assert_eq!(crate::evidence::events(&env).pop().unwrap().binding.as_deref(), Some("dup.job"));
        // no job (an SDK host on its conversation): held, nothing bound
        start(&env, &id, "s3", "resume", 13, None, &cwd);
        assert_eq!(crate::evidence::events(&env).pop().unwrap().binding.as_deref(), Some("held"));
        assert_eq!(crate::registry::load(&env).identities[&id.0].lineage.len(), 3);
        // j1's row gone (claude rm), a new job resumes the conversation: rebound
        std::fs::remove_dir_all(env.claude_home.join("jobs").join("j1")).unwrap();
        start(&env, &id, "s3", "resume", 14, Some("j3"), &cwd);
        let rec = crate::registry::load(&env).identities[&id.0].clone();
        assert_eq!(rec.job.as_deref(), Some("j3"));
        assert_eq!(rec.lineage.last().map(|l| (l.session_id.as_str(), l.job.as_deref(), l.via.as_str())), Some(("s3", Some("j3"), "resume")));
        // the same labels in another folder: an impostor, not bound (BUG-30, BUG-55)
        start(&env, &id, "s7", "startup", 15, Some("j7"), "/somewhere/else");
        assert_eq!(crate::evidence::events(&env).pop().unwrap().binding.as_deref(), Some("elsewhere"));
        assert_eq!(crate::registry::load(&env).identities[&id.0].job.as_deref(), Some("j3"));
    }

    #[test]
    fn labels_of_another_nimitz_home_leave_no_line_and_bind_nothing() {
        let (_td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let mut vars = labels_of(&env, &id);
        vars.insert("NIMITZ_HOME".into(), "/another/nimitz".into());
        let input = serde_json::json!({"session_id": "s1", "source": "startup", "hook_event_name": "SessionStart"});
        assert!(handle(&env, "SessionStart", &input.to_string(), &vars).is_empty(), "no rules either");
        assert!(crate::evidence::events(&env).is_empty());
        assert!(crate::registry::load(&env).identities.is_empty());
        // the real path of this home is this home
        let real = std::fs::canonicalize(&env.home).unwrap().to_string_lossy().into_owned();
        vars.insert("NIMITZ_HOME".into(), real);
        handle(&env, "SessionStart", &input.to_string(), &vars);
        assert_eq!(crate::evidence::events(&env).len(), 1);
    }

    #[test]
    fn a_missing_or_partial_session_file_binds_nothing_and_fails_nothing() {
        let (_td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let mut vars = labels_of(&env, &id);
        vars.insert("NIMITZ_HOOK_PID".into(), "4242".into());
        let input = serde_json::json!({"session_id": "s1", "source": "startup", "hook_event_name": "SessionStart"});
        // no file at all
        assert!(handle(&env, "SessionStart", &input.to_string(), &vars).contains("additionalContext"));
        assert_eq!(crate::evidence::events(&env).pop().unwrap().binding.as_deref(), Some("held"));
        // a file caught mid-write
        let file = env.claude_home.join("sessions").join("4242.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{\"pid\": 4242, \"jobId\": \"j").unwrap();
        handle(&env, "SessionStart", &input.to_string(), &vars);
        let e = crate::evidence::events(&env).pop().unwrap();
        assert_eq!((e.job, e.binding.as_deref()), (None, Some("held")));
        assert!(crate::registry::load(&env).identities.get(&id.0).is_none());
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
        let _ = send(&env, &id, &Id::admiral("MAIN").name());

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
