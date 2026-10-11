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
    // The Claude Code process (the hook's parent); a sandbox names it. A hook
    // written as one command string runs under `sh -c`: then the parent is a
    // shell and the Claude Code process is its parent (r3-e2e.md finding 7).
    let given: i64 = vars.get("NIMITZ_HOOK_PID").and_then(|p| p.parse().ok()).unwrap_or_else(crate::os::parent_pid);
    let pid = if vars.get("NIMITZ_HOOK_SESSION_FILE").is_some_and(|p| !p.is_empty()) {
        given // the sandbox names the session file; the pid is its own
    } else {
        let has_file = |p: i64| env.claude_home.join("sessions").join(format!("{p}.json")).is_file();
        if cfg!(windows) {
            // Git Bash runs a command string as bash under bash (r3-e2e.md finding 7, Windows)
            claude_pid_through(given, has_file, parent_of, |p| crate::os::exe_of(p).is_some_and(|e| crate::restore::is_shell(&[e])))
        } else {
            claude_pid(given, has_file, parent_of)
        }
    };
    // A background job's labels count only when its own launch saved them: the
    // daemon passes its environment on, so a daemon started from a labelled
    // lead's shell labels every unlabelled job as that lead
    // (`evidence::job_backs_labels`). A job whose row is not written yet is
    // judged later (no binding now; `status` replays the line).
    let session = session_file(env, vars, pid);
    let job_of_session = str_at(&session, "jobId");
    // (a pre-warmed spare, the agent view's draft, is never anyone's member)
    if session.get("spare").and_then(Value::as_bool) == Some(true) {
        labels.clear();
    }
    // The env's labels go on the event line as they were (a replay judges
    // them again against the job's row); every effect here (binding, rules,
    // signal authority, needs-you) uses only labels the job's row backs. A
    // job whose row is not written yet is waited for a moment, then left
    // unlabelled for now: no rules, no binding; its first prompt gets the
    // rules (`rules::update_for`) and `status` replays the line.
    let line_labels = labels.clone();
    if !labels.is_empty() {
        if let Some(job) = job_of_session.as_deref() {
            let mut saved = crate::evidence::job_saved_labels(&env.claude_home, job);
            // (only a SessionStart waits: a later event of a job whose row is gone pays nothing)
            let deadline = std::time::Instant::now() + if event == "SessionStart" { ROW_WAIT } else { std::time::Duration::ZERO };
            while saved.is_none() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(50));
                saved = crate::evidence::job_saved_labels(&env.claude_home, job);
            }
            labels = match saved {
                // the launch's own labels, not the env's (an inherited key the
                // settings did not name stays out)
                Some(sv) if crate::evidence::is_labelled(&sv) && sv.get("NIMITZ_ID") == labels.get("NIMITZ_ID") => sv,
                _ => BTreeMap::new(),
            };
        }
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
    // A labelled session starts (startup, resume, clear, compact, fork): bind
    // it to its identity (plan-v4 section 3), hand it its role's rules and
    // record the delivery. On a per-turn event, a session that runs on older
    // rules gets the update (once per version per job).
    let mut rules = None;
    let mut binding = None;
    // one clock for the line's `ts` and the lineage `at`: a line the hook
    // bound is older than the registry it wrote, so `status` never replays it
    let now = crate::evidence::now();
    if event == "SessionStart" {
        // another nimitz home's member: not ours, not even a line
        if crate::registry::other_home(env, &labels) {
            return (out, notice);
        }
        if let Some(id) = crate::registry::identity_of(env, &labels) {
            let sid = str_at(&input, "session_id").unwrap_or_default();
            // where the member listens: nimitz posts notices and signals there
            if let Some(sock) = vars.get("CLAUDE_CODE_MESSAGING_SOCKET") {
                crate::inbox::record(env, &sid, pid, sock.strip_prefix("uds:").unwrap_or(sock));
            }
            rules = Some(crate::rules::VERSION);
            crate::rules::record_delivery(env, &crate::rules::delivery_key(env, pid, &sid), &sid, event);
            // an aide also gets its inbox as it stands and the cron to ask for
            // (aide.md 5.1, wake 2): a relaunch, a /clear or a compact goes back to work
            let context = match id.role() {
                Role::Aide => format!("{}\n\n{}", crate::rules::text(&id), aide_start_context(env, &id, str_at(&session, "jobId").as_deref(), now)),
                _ => crate::rules::text(&id),
            };
            out = json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": context}}).to_string();
            binding = Some(match str_at(&input, "session_id") {
                // no session file (missing, or caught mid-write): bind nothing
                // now; `status` replays the line and judges the job from the rows
                Some(_) if session.is_null() => "none",
                Some(sid) => bind_start(env, &id, &input, &session, &sid, now).as_str(),
                None => "none",
            });
            // the member is live now: the active set says so (a lost note is
            // made good by the next `status`)
            if matches!(binding, Some("bound") | Some("same")) {
                let _ = crate::active::note_start(env, &id, str_at(&session, "jobId").as_deref(), &sid);
            }
        }
    } else if event == "SessionEnd" && reason.as_deref() != Some("clear") {
        // a member's process ended (a `claude stop`, `/stop`, an exit): out of
        // the active set; a `/clear` ends only the conversation, the job goes on
        if let (Some(id), Some(sid)) = (crate::registry::identity_of(env, &labels), str_at(&input, "session_id")) {
            let _ = crate::active::note_end(env, &id, &sid, None);
        }
    } else if UPDATE_EVENTS.contains(&event) && labels.get("NIMITZ_HOME").map_or(true, |h| h.is_empty() || std::path::Path::new(h) == env.home || crate::env::canon(std::path::Path::new(h)) == crate::env::canon(&env.home)) {
        // (another nimitz home's member gets no update from this home)
        let member = labels.get("NIMITZ_ID").map(|i| Id(i.clone())).filter(|i| labels.get("NIMITZ_ROLE").map(String::as_str) == Some(i.role().as_str()));
        if let Some(id) = member {
            let sid = str_at(&input, "session_id").unwrap_or_default();
            if let Some(text) = crate::rules::update_for(env, &id, &crate::rules::delivery_key(env, pid, &sid), &sid, event) {
                rules = Some(crate::rules::VERSION);
                out = with_context(&out, event, &text);
            }
        }
    }
    let tool_event = matches!(event, "PreToolUse" | "PostToolUse" | "PostToolUseFailure");
    if !tool_event || is_signal || tool.as_deref() == Some("AskUserQuestion") {
        let line = json!({
            "ts": now,
            "event": event,
            "session_id": str_at(&input, "session_id"),
            "source": str_at(&input, "source"),
            "reason": reason,
            "agent_id": str_at(&input, "agent_id"),
            "agent_type": str_at(&input, "agent_type"),
            "tool_name": tool,
            "to": to,
            "pid": pid,
            "labels": line_labels,
            "rules": rules,
            // SessionStart: the process's job, kind and entrypoint (per pid, M21:
            // the name is read from the session file later, never from here)
            "job": str_at(&session, "jobId"),
            "kind": if event == "SessionStart" && session.is_null() { Some("unknown".to_string()) } else { str_at(&session, "kind") },
            "entrypoint": str_at(&session, "entrypoint"),
            "binding": binding,
            "cwd": if event == "SessionStart" { str_at(&input, "cwd").or_else(|| str_at(&session, "cwd")) } else { None },
        });
        append_line(env, &line.to_string());
    }
    (out, notice)
}

/// The Claude Code process behind a hook: `ppid` (the hook's parent) when
/// `sessions/<ppid>.json` exists, else the parent's parent when ITS session
/// file exists (a hook given as one command string runs under `sh -c`, and
/// the shell is the parent), else `ppid` as before. `has_file` says whether a
/// pid's session file exists; `parent_of` reads a pid's parent.
pub fn claude_pid(ppid: i64, has_file: impl Fn(i64) -> bool, parent_of: impl Fn(i64) -> Option<i64>) -> i64 {
    claude_pid_through(ppid, has_file, parent_of, |_| false)
}

/// The most shells between a hook and its Claude Code process
/// (`claude_pid_through`).
pub const SHELL_HOPS: usize = 4;

/// `claude_pid`, but a parent that is a shell (`is_shell`) is walked
/// through, up to `SHELL_HOPS` shells: a command string on Windows runs
/// under Git Bash's `bash.exe`, which starts another `bash.exe`. Only shells
/// are walked through, so a session started from another session's Bash
/// tool never binds to that outer session.
pub fn claude_pid_through(ppid: i64, has_file: impl Fn(i64) -> bool, parent_of: impl Fn(i64) -> Option<i64>, is_shell: impl Fn(i64) -> bool) -> i64 {
    if has_file(ppid) {
        return ppid;
    }
    let mut cur = ppid;
    for hop in 0..=SHELL_HOPS {
        match parent_of(cur) {
            Some(up) if up > 1 && has_file(up) => return up,
            Some(up) if up > 1 && hop < SHELL_HOPS && is_shell(cur) => cur = up,
            _ => break,
        }
    }
    ppid
}

/// A process's parent pid (`os::parent_of`). None for a dead pid and where
/// the system gives no answer.
pub fn parent_of(pid: i64) -> Option<i64> {
    crate::os::parent_of(pid)
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

/// How long the SessionStart hook waits for a new job's row (its launch's
/// saved labels) before it leaves the session unlabelled for now.
const ROW_WAIT: std::time::Duration = std::time::Duration::from_millis(500);

/// How long the hook waits for `registry.lock`: a hook never hangs a session.
const LOCK_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

/// Apply the binding table under the registry lock (one locked read-modify-
/// write, at most `LOCK_BUDGET` for the lock). A registry that cannot be
/// read, locked or written binds nothing (`none`); `status` replays the line
/// later (`registry::replay`).
fn bind_start(env: &Env, id: &Id, input: &Value, session: &Value, sid: &str, now: f64) -> crate::registry::Binding {
    let job = str_at(session, "jobId");
    let source = str_at(input, "source");
    let cwd = str_at(input, "cwd").or_else(|| str_at(session, "cwd")).unwrap_or_default();
    crate::registry::try_update(env, LOCK_BUDGET, |r| {
        let folder = crate::registry::identity_folder(env, r, id);
        crate::registry::apply_start(r, id, &cwd, folder.as_deref(), job.as_deref(), sid, source.as_deref(), now, |j| crate::registry::job_row_of_this_home(env, j))
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


fn append_line(env: &Env, line: &str) {
    let path = env.events_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // appends share the lock; a compaction takes it alone
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path.with_extension("lock")).ok();
    if let Some(l) = &lock {
        let _ = l.lock_shared();
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
            let _ = l.unlock();
            let _ = l.lock();
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
/// comms rule allows; an aide, its parent only (and a parent its aide). Targets that are not fleet names (teammates of an agent
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
            // an aide signals its parent only: never what nimitz cannot name
            Role::Aide => true,
            Role::Admiral | Role::Commodore => false,
        };
        return if closed {
            Err(format!(
                "{to_name} is not a fleet address nimitz can resolve{}; {} signals only named members (nimitz chain of command; sender {})",
                resolved.map(|n| format!(" (it is {n:?})")).unwrap_or_default(),
                if from.role() == Role::Aide { "an aide" } else { "a ship under this squadron's comms rule" },
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
            Role::Aide if to.parent().as_ref() == Some(from) => Ok(()),
            _ => deny(format!("a commodore signals its admiral and its ships, not {to_name}")),
        },
        Role::Ship => match to.role() {
            Role::Commodore if same_sq => Ok(()),
            // a reply: the admiral may signal a ship, and the ship answers it
            Role::Admiral if to.fleet() == from.fleet() => Ok(()),
            Role::Aide if to.parent().as_ref() == Some(from) => Ok(()),
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
        // an aide reports to its parent and to nobody else (docs/design/aide.md section 7)
        Role::Aide => match from.parent() {
            Some(p) if *to == p => Ok(()),
            _ => deny(format!("an aide signals its parent only, not {to_name}")),
        },
    }
}

/// The sender's squadron `comms` rule; on any doubt the strictest rule.
fn squadron_comms(env: &Env, from: &Id) -> Comms {
    let strict = Comms::Mode("commodore-only".into());
    let Ok(f) = crate::config::load_fleet(env, from.fleet()) else { return strict };
    f.squadrons.into_iter().find(|(_, s)| Some(s.squadron.as_str()) == from.squadron()).map(|(_, s)| s.comms).unwrap_or(strict)
}

/// What an aide sees at each session start: its inbox now, and what to do
/// first (drain it; ask for the 15-minute cron, which a session loses at a
/// relaunch, a /clear or a compact).
pub fn aide_start_context(env: &Env, id: &Id, job: Option<&str>, now: f64) -> String {
    let state = match crate::queue::Queue::of(env, id) {
        Ok(q) => {
            let st = q.peek_status(now, false);
            let mut parts = vec![format!("{} pending", st.pending)];
            for c in &st.claimed {
                let n = c.id.trim_start_matches('0');
                // the lease is the job's, which a /clear or a compact keeps
                if job.is_some_and(|j| j == c.by) || c.by == id.0 {
                    parts.push(format!("item {n} is yours, claimed before this session started: finish it first (`nimitz inbox done|fail {id} {n}`; its payload: `nimitz inbox list {id}`)"));
                } else {
                    let left = ((c.until - now) / 60.0).ceil().max(0.0) as i64;
                    parts.push(format!("item {n} claimed by {} (its lease ends in {left}m)", c.by));
                }
            }
            if !st.dead.is_empty() {
                parts.push(format!("{} dead (your admiral decides: retry or drop)", st.dead.len()));
            }
            if let Some(o) = st.oldest_age_s.filter(|_| st.pending > 0) {
                parts.push(format!("the oldest {}m", (o / 60.0).floor() as i64));
            }
            parts.join(", ")
        }
        Err(e) => format!("unreadable ({e})"),
    };
    format!(
        "[nimitz inbox] {id}: {state}.\nNow: if this session has no cron for it yet, ask for one (CronCreate, every 15 minutes, prompt \"drain your inbox\"); then drain your inbox (`nimitz inbox claim {id}` until it exits 3). An item claimed by an older session of yours comes back when its lease ends."
    )
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
        let plan = crate::config::plan_squadron_new(&env, "MAIN", "BOREAL", "Quiet squadron.", Comms::Mode("commodore-only".into()), None);
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
        // an aide: its parent, both ways, and nobody else (aide.md section 7)
        let kb = Id("MAIN/@KB".into());
        assert!(!denied(&send(&env, &kb, &adm.name())));
        assert!(!denied(&send(&env, &adm, &kb.name())));
        for to in [argos.name(), ship.name(), Id::admiral("OTHER").name(), Id("MAIN/@REF".into()).name(), "sock:/tmp/x".to_string()] {
            assert!(denied(&send(&env, &kb, &to)), "{to}");
        }
        assert!(denied(&send(&env, &argos, &kb.name())) && denied(&send(&env, &ship, &kb.name())), "not its parent");
        // a commodore's aide (later): its commodore, both ways
        let x = Id("MAIN/ARGOS/@X".into());
        assert!(signal_table(&x, &argos, &|| Comms::default()).is_ok() && signal_table(&argos, &x, &|| Comms::default()).is_ok());
        assert!(signal_table(&ship, &x, &|| Comms::default()).is_err() && signal_table(&x, &ship, &|| Comms::default()).is_err());
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
        let ppid = crate::os::parent_pid();
        let sessions = env.claude_home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join(format!("{ppid}.json")), r#"{"pid": 1, "sessionId": "s-a", "jobId": "9cdceb2d", "kind": "bg"}"#).unwrap();
        job_row(&env, "9cdceb2d", "s-a");
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

    /// A job row of the ship MAIN/ARGOS/ARGOS as nimitz launches it (its
    /// labels saved in `respawnFlags`).
    fn job_row(env: &Env, short: &str, sid: &str) {
        job_row_of_home(env, short, sid, &Id::ship("MAIN", "ARGOS", "ARGOS"), &env.home.to_string_lossy());
    }

    /// A job row with no labels of its own: a job the daemon started for an
    /// unlabelled launch (it still carries the daemon's inherited labels in
    /// its env, so in its hook's env).
    fn bare_job_row(env: &Env, short: &str, sid: &str) {
        let p = env.claude_home.join("jobs").join(short).join("state.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::json!({"sessionId": sid, "resumeSessionId": sid, "state": "running", "respawnFlags": []}).to_string()).unwrap();
    }

    /// The daemon hands its environment to every job it starts; a daemon
    /// started from a labelled lead's shell labels an unlabelled `claude --bg`
    /// as that lead. Such a job's labels never count: no binding, no rules, no
    /// signal authority.
    #[test]
    fn a_job_that_inherited_a_leads_labels_from_the_daemon_is_unlabelled() {
        let (_td, env) = world();
        let lead = Id::commodore("MAIN", "ARGOS");
        let cwd = "/tmp/users-own-work".to_string();
        bare_job_row(&env, "u1", "s-user");
        let out = start(&env, &lead, "s-user", "startup", 21, Some("u1"), &cwd);
        assert!(!out.contains("additionalContext"), "no rules for an inherited label: {out}");
        assert!(crate::registry::load(&env).identities.get(&lead.0).map_or(true, |r| r.session.as_deref() != Some("s-user")), "never bound to the lead");
        let e = crate::evidence::events(&env).pop().unwrap();
        assert!(e.binding.is_none(), "nothing judged a member: {:?}", e.binding);
        // (the line keeps the env's labels; a replay judges them against the row)
        assert_eq!(crate::registry::replay(&env, &crate::evidence::events(&env)), 0, "a replay binds nothing either");
        // and its SendMessage is not the lead's
        let mut v = labels_of(&env, &lead);
        v.insert("NIMITZ_HOOK_PID".into(), "21".into());
        let input = serde_json::json!({"tool_name": "SendMessage", "tool_input": {"to": "[MAIN.ARGOS]-ARGOS"}});
        assert!(handle(&env, "PreToolUse", &input.to_string(), &v).is_empty(), "not judged as the commodore");
        // a job whose row is not written yet (after the hook's short wait): no
        // rules, nothing bound now; the row then shows: a bare one (an
        // inherited label) never binds on replay, a launch's one does
        // (both in the lead's own folder: only the labels' backing decides)
        let lead_dir = crate::registry::identity_folder(&env, &crate::registry::load(&env), &lead).unwrap().to_string_lossy().into_owned();
        let out = start(&env, &lead, "s-early", "startup", 22, Some("u2"), &lead_dir);
        assert!(!out.contains("additionalContext"), "no rules on an unbacked label: {out}");
        assert!(crate::evidence::events(&env).pop().unwrap().binding.is_none());
        bare_job_row(&env, "u2", "s-early");
        assert_eq!(crate::registry::replay(&env, &crate::evidence::events(&env)), 0, "an inherited label never binds on replay");
        let out = start(&env, &lead, "s-late", "startup", 23, Some("u3"), &lead_dir);
        assert!(!out.contains("additionalContext"));
        job_row_of_home(&env, "u3", "s-late", &lead, &env.home.to_string_lossy());
        assert_eq!(crate::registry::replay(&env, &crate::evidence::events(&env)), 1, "a launch's row written late binds on replay");
        assert_eq!(crate::registry::load(&env).identities[&lead.0].job.as_deref(), Some("u3"));
    }

    /// The agent view's pre-warmed spare is never anyone's member, whatever
    /// labels it inherited: no rules, no binding, no signal authority.
    #[test]
    fn a_spare_is_never_labelled_in_the_hook() {
        let (_td, env) = world();
        let lead = Id::commodore("MAIN", "ARGOS");
        let cwd = "/tmp/lead-folder".to_string();
        // a spare whose own row even saved the lead's labels: still nobody's
        job_row_of_home(&env, "sp1", "s-spare", &lead, &env.home.to_string_lossy());
        let file = env.claude_home.join("sessions").join("31.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, serde_json::json!({"pid": 31, "sessionId": "s-spare", "jobId": "sp1", "spare": true, "kind": "bg", "cwd": cwd}).to_string()).unwrap();
        let mut v = labels_of(&env, &lead);
        v.insert("NIMITZ_HOOK_PID".into(), "31".into());
        let input = serde_json::json!({"session_id": "s-spare", "source": "startup", "hook_event_name": "SessionStart", "cwd": cwd});
        assert!(!handle(&env, "SessionStart", &input.to_string(), &v).contains("additionalContext"));
        assert!(crate::registry::load(&env).identities.get(&lead.0).map_or(true, |r| r.session.as_deref() != Some("s-spare")));
        let send = serde_json::json!({"tool_name": "SendMessage", "tool_input": {"to": "[MAIN.ARGOS]-ARGOS"}});
        assert!(handle(&env, "PreToolUse", &send.to_string(), &v).is_empty(), "no commodore authority");
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
        job_row(&env, "j3", "s3");
        start(&env, &id, "s3", "resume", 14, Some("j3"), &cwd);
        let rec = crate::registry::load(&env).identities[&id.0].clone();
        assert_eq!(rec.job.as_deref(), Some("j3"));
        assert_eq!(rec.lineage.last().map(|l| (l.session_id.as_str(), l.job.as_deref(), l.via.as_str())), Some(("s3", Some("j3"), "resume")));
        // the same labels in another folder: an impostor, not bound (BUG-30, BUG-55)
        job_row(&env, "j7", "s7");
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

    /// A job row as a real launch leaves it: `respawnFlags` carry the
    /// `--settings` whose env names the labels, `home` included.
    fn job_row_of_home(env: &Env, short: &str, sid: &str, id: &Id, home: &str) {
        let mut labels = crate::claude::labels_for(env, id);
        labels.insert("NIMITZ_HOME".into(), home.into());
        let settings = serde_json::json!({"env": labels}).to_string();
        let p = env.claude_home.join("jobs").join(short).join("state.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::json!({"sessionId": sid, "resumeSessionId": sid, "state": "running", "respawnFlags": ["--settings", settings]}).to_string()).unwrap();
    }

    /// Acc-D F1, reproduced: two homes share one `NIMITZ_STATE` (the hook's
    /// state comes from a project settings env; its home is the session's
    /// own `--settings` label). The hook of a session labelled for ANOTHER
    /// home runs with that home as its env: every job of this home is
    /// "other" to it. It used to load the registry stripped of them and save
    /// that: the leads and the ship's history gone, and the daemon-restarted
    /// ship then bound alone with a lone `resume`. Now a writer works on
    /// what is on disk, and a write that drops an identity is refused.
    #[test]
    fn a_hook_of_another_home_in_a_shared_state_never_strips_this_homes_registry() {
        let (_td, env) = world();
        let home = env.home.to_string_lossy().into_owned();
        let adm = Id::admiral("MAIN");
        let com = Id::commodore("MAIN", "ARGOS");
        let s1 = Id::ship("MAIN", "ARGOS", "ARGOS");
        let s2 = Id::ship("MAIN", "BOREAL", "ONE");
        let folder = |id: &Id| crate::config::load_fleet(&env, "MAIN").ok().and_then(|f| crate::config::member_dir(&env, &f, id)).unwrap().to_string_lossy().into_owned();
        for (id, job, sid, pid) in [(&adm, "70528a56", "sa", 21), (&com, "b14fca85", "sc", 22), (&s1, "e0441656", "s1", 23)] {
            job_row_of_home(&env, job, sid, id, &home);
            start(&env, id, sid, "startup", pid, Some(job), &folder(id));
        }
        let before = std::fs::read_to_string(env.registry_path()).unwrap();
        assert_eq!(crate::registry::load(&env).identities.len(), 3);
        // the other home's session: an SDK host (no job) in S2's folder, its
        // labels and its env naming /elsewhere, the same state and claude home
        let other = Env { home: std::path::PathBuf::from("/elsewhere/nimitz"), ..env.clone() };
        let file = env.claude_home.join("sessions").join("24.json");
        std::fs::write(&file, serde_json::json!({"pid": 24, "sessionId": "s2", "cwd": folder(&s2), "kind": "interactive", "entrypoint": "sdk-cli", "jobId": null}).to_string()).unwrap();
        let mut vars = labels_of(&env, &s2);
        vars.insert("NIMITZ_HOME".into(), "/elsewhere/nimitz".into());
        vars.insert("NIMITZ_HOOK_PID".into(), "24".into());
        let input = serde_json::json!({"session_id": "s2", "source": "resume", "hook_event_name": "SessionStart", "cwd": folder(&s2)});
        handle(&other, "SessionStart", &input.to_string(), &vars);
        let e = crate::evidence::events(&env).pop().unwrap();
        assert_eq!((e.labels.get("NIMITZ_HOME").map(String::as_str), e.binding.as_deref()), (Some("/elsewhere/nimitz"), Some("held")));
        assert_eq!(std::fs::read_to_string(env.registry_path()).unwrap(), before, "the registry is not written at all: nothing bound");
        // the same hook bound to a job of its home: the one record it touched
        // changes, every other stays byte for byte
        job_row_of_home(&env, "f0f0f0f0", "s2", &s2, "/elsewhere/nimitz");
        std::fs::write(&file, serde_json::json!({"pid": 24, "sessionId": "s2", "cwd": folder(&s2), "kind": "bg", "entrypoint": "cli", "jobId": "f0f0f0f0"}).to_string()).unwrap();
        let input = serde_json::json!({"session_id": "s2", "source": "startup", "hook_event_name": "SessionStart", "cwd": folder(&s2)});
        handle(&other, "SessionStart", &input.to_string(), &vars);
        assert_eq!(crate::evidence::events(&env).pop().unwrap().binding.as_deref(), Some("bound"));
        let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(env.registry_path()).unwrap()).unwrap();
        let was: serde_json::Value = serde_json::from_str(&before).unwrap();
        for id in [&adm, &com, &s1] {
            assert_eq!(raw["identities"][&id.0], was["identities"][&id.0], "{id} changed");
        }
        assert_eq!(raw["identities"][&s2.0]["job"], "f0f0f0f0", "on disk, the other home's binding sits next to this home's");
        // this home's readers never see it; this home's own records are whole
        let mine = crate::registry::load(&env);
        assert_eq!(mine.identities.len(), 3, "{:?}", mine.identities.keys());
        assert_eq!(mine.identities[&s1.0].lineage.iter().map(|l| l.via.as_str()).collect::<Vec<_>>(), ["launch"]);
        // the daemon restarts S1 (its env is the daemon's, its home label this one): same, nothing lost
        start(&env, &s1, "s1", "resume", 25, Some("e0441656"), &folder(&s1));
        assert_eq!(crate::evidence::events(&env).pop().unwrap().binding.as_deref(), Some("same"));
        let after = crate::registry::load(&env);
        assert_eq!(after.identities.len(), 3);
        assert_eq!(after.identities[&s1.0].lineage.iter().map(|l| l.via.as_str()).collect::<Vec<_>>(), ["launch"]);
        assert!(after.identities.contains_key(&adm.0) && after.identities.contains_key(&com.0));
    }

    /// The other side of a shared state: a binding the other home's hook
    /// left on an identity of this home never keeps this home's own job from
    /// binding (its row is not a row of this home, so no `dup.job`), and its
    /// folder (the record's `cwd` is the other home's) never makes this
    /// home's own session an impostor.
    #[test]
    fn a_foreign_binding_on_an_identity_neither_dups_nor_misplaces_this_homes_own_start() {
        let (_td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let cwd = crate::config::load_fleet(&env, "MAIN").ok().and_then(|f| crate::config::member_dir(&env, &f, &id)).unwrap().to_string_lossy().into_owned();
        job_row_of_home(&env, "f0f0f0f0", "s-theirs", &id, "/elsewhere/nimitz");
        let other = Env { home: std::path::PathBuf::from("/elsewhere/nimitz"), ..env.clone() };
        std::fs::create_dir_all(env.claude_home.join("sessions")).unwrap();
        std::fs::write(env.claude_home.join("sessions").join("31.json"), serde_json::json!({"pid": 31, "sessionId": "s-theirs", "cwd": "/elsewhere/ship", "kind": "bg", "entrypoint": "cli", "jobId": "f0f0f0f0"}).to_string()).unwrap();
        let mut vars = labels_of(&env, &id);
        vars.insert("NIMITZ_HOME".into(), "/elsewhere/nimitz".into());
        vars.insert("NIMITZ_HOOK_PID".into(), "31".into());
        handle(&other, "SessionStart", &serde_json::json!({"session_id": "s-theirs", "source": "startup", "cwd": "/elsewhere/ship"}).to_string(), &vars);
        assert_eq!(crate::evidence::events(&env).pop().unwrap().binding.as_deref(), Some("bound"));
        assert!(crate::registry::load(&env).identities.get(&id.0).is_none(), "nothing of this home's: no record in this home's view");
        // this home's own launch of the identity
        job_row_of_home(&env, "a1a1a1a1", "s-mine", &id, &env.home.to_string_lossy());
        start(&env, &id, "s-mine", "startup", 32, Some("a1a1a1a1"), &cwd);
        assert_eq!(crate::evidence::events(&env).pop().unwrap().binding.as_deref(), Some("bound"));
        let rec = crate::registry::load(&env).identities[&id.0].clone();
        assert_eq!((rec.job.as_deref(), rec.session.as_deref(), rec.cwd.as_str()), (Some("a1a1a1a1"), Some("s-mine"), cwd.as_str()));
        assert_eq!(rec.lineage.iter().map(|l| (l.job.as_deref(), l.via.as_str())).collect::<Vec<_>>(), [(Some("a1a1a1a1"), "launch")]);
        // and on disk both bindings sit side by side
        let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(env.registry_path()).unwrap()).unwrap();
        assert_eq!(raw["identities"][&id.0]["lineage"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_missing_or_partial_session_file_binds_nothing_now_and_status_judges_it_from_the_rows() {
        let (_td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let cwd = crate::config::load_fleet(&env, "MAIN").ok().and_then(|f| crate::config::member_dir(&env, &f, &id)).unwrap().to_string_lossy().into_owned();
        let mut vars = labels_of(&env, &id);
        vars.insert("NIMITZ_HOOK_PID".into(), "4242".into());
        let input = serde_json::json!({"session_id": "s1", "source": "startup", "hook_event_name": "SessionStart", "cwd": cwd});
        // no file at all: the rules still go out, the line is replayable
        assert!(handle(&env, "SessionStart", &input.to_string(), &vars).contains("additionalContext"));
        let e = crate::evidence::events(&env).pop().unwrap();
        assert_eq!((e.kind.as_deref(), e.binding.as_deref(), e.cwd.as_deref()), (Some("unknown"), Some("none"), Some(cwd.as_str())));
        assert!(crate::registry::load(&env).identities.get(&id.0).is_none());
        // a file caught mid-write: the same
        let file = env.claude_home.join("sessions").join("4242.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "{\"pid\": 4242, \"jobId\": \"j").unwrap();
        handle(&env, "SessionStart", &input.to_string(), &vars);
        let e = crate::evidence::events(&env).pop().unwrap();
        assert_eq!((e.job, e.kind.as_deref(), e.binding.as_deref()), (None, Some("unknown"), Some("none")));
        // replay judges the job from the rows: the job whose state.json runs s1
        job_row(&env, "j5", "s1");
        assert_eq!(crate::registry::replay(&env, &crate::evidence::events(&env)), 1);
        let rec = crate::registry::load(&env).identities[&id.0].clone();
        assert_eq!((rec.job.as_deref(), rec.session.as_deref()), (Some("j5"), Some("s1")));
        // a line of a conversation no job runs is held: nothing bound
        let input = serde_json::json!({"session_id": "s8", "source": "resume", "hook_event_name": "SessionStart", "cwd": cwd});
        handle(&env, "SessionStart", &input.to_string(), &vars);
        assert_eq!(crate::registry::replay(&env, &crate::evidence::events(&env)), 0);
        assert_eq!(crate::registry::load(&env).identities[&id.0].session.as_deref(), Some("s1"));
    }

    #[test]
    fn a_bound_line_is_older_than_the_registry_so_status_never_replays_it() {
        let (_td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let cwd = crate::config::load_fleet(&env, "MAIN").ok().and_then(|f| crate::config::member_dir(&env, &f, &id)).unwrap().to_string_lossy().into_owned();
        job_row(&env, "j1", "s1");
        start(&env, &id, "s1", "startup", 11, Some("j1"), &cwd);
        let reg = crate::registry::load(&env);
        let e = crate::evidence::events(&env).pop().unwrap();
        assert!(e.ts <= reg.updated_at, "{} <= {}", e.ts, reg.updated_at);
        assert_eq!(reg.identities[&id.0].lineage[0].at, e.ts, "one clock for the line and the lineage");
        assert_eq!(crate::registry::replay(&env, &crate::evidence::events(&env)), 0);
    }

    #[test]
    fn the_hook_gives_up_on_a_held_registry_lock_and_says_none() {
        let (_td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let cwd = crate::config::load_fleet(&env, "MAIN").ok().and_then(|f| crate::config::member_dir(&env, &f, &id)).unwrap().to_string_lossy().into_owned();
        job_row(&env, "j1", "s1");
        // another nimitz holds the lock for longer than the hook's budget
        std::fs::create_dir_all(&env.state).unwrap();
        let held = std::fs::File::create(env.registry_lock_path()).unwrap();
        held.lock().unwrap();
        let budget = std::time::Duration::from_millis(100);
        let err = crate::registry::try_update(&env, budget, |_| ()).unwrap_err();
        assert!(err.to_string().contains("registry lock"), "{err}");
        drop(held);
        assert!(crate::registry::try_update(&env, budget, |_| ()).is_ok());
        start(&env, &id, "s1", "startup", 11, Some("j1"), &cwd);
        assert_eq!(crate::evidence::events(&env).pop().unwrap().binding.as_deref(), Some("bound"));
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
        let (td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        // malformed stdin, unknown event, missing fields: no panic, no output
        for stdin in ["", "not json", "[]", r#"{"tool_name": 7, "tool_input": {"to": 3}}"#] {
            assert!(handle(&env, "PreToolUse", stdin, &labels_of(&env, &id)).is_empty(), "{stdin}");
        }
        assert!(handle(&env, "SomeNewEventInANewVersion", "{}", &labels_of(&env, &id)).is_empty());
        // an unwritable state dir loses the line but still returns
        // (a folder under a plain file: `/dev/null/...` is a real folder on Windows)
        let file = td.path().join("a-file");
        std::fs::write(&file, "").unwrap();
        let bad = Env { state: file.join("nope"), ..env.clone() };
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

    /// A hook given as one command string runs under `sh -c`: the parent is a
    /// shell, the Claude Code process is its parent. The pure rule, then the
    /// real chain: NIMITZ_HOOK_PID names this test process (no session file),
    /// whose parent (the test runner) has one; the line carries the parent.
    #[test]
    fn a_hook_under_a_shell_finds_the_session_through_the_parents_parent() {
        let files = |p: i64| p == 300;
        let chain = |p: i64| match p {
            100 => Some(200),
            200 => Some(300),
            300 => Some(1),
            _ => None,
        };
        assert_eq!(claude_pid(300, files, chain), 300, "the parent itself");
        assert_eq!(claude_pid(200, files, chain), 300, "the parent's parent");
        assert_eq!(claude_pid(100, files, chain), 100, "one step only, else as given");
        assert_eq!(claude_pid(999, files, chain), 999, "an unknown pid stays");
        assert_eq!(claude_pid(200, |_| false, chain), 200, "no file anywhere: as given");
        assert_eq!(claude_pid(5, |p| p == 1, |_| Some(1)), 5, "never pid 1");
        // Windows: nimitz <- bash 100 <- bash 200 <- claude 300; only shells are walked through
        let shells = |p: i64| p == 100 || p == 200;
        assert_eq!(claude_pid_through(100, files, chain, shells), 300, "two shells deep");
        assert_eq!(claude_pid_through(100, files, chain, |p| p == 200), 100, "a parent that is no shell stops the walk");
        assert_eq!(claude_pid_through(100, |p| p == 1, chain, |_| true), 100, "never pid 1");
        // the real chain through /proc
        let me = std::process::id() as i64;
        let runner = parent_of(me).expect("/proc gives the parent");
        assert!(runner > 1 && parent_of(runner).is_some());
        let (_td, env) = world();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let folder = crate::registry::identity_folder(&env, &crate::registry::load(&env), &id).expect("the ship's folder");
        let cwd = folder.to_string_lossy().into_owned();
        std::fs::create_dir_all(env.claude_home.join("sessions")).unwrap();
        std::fs::write(
            env.claude_home.join("sessions").join(format!("{runner}.json")),
            serde_json::json!({"pid": runner, "sessionId": "s-sh", "jobId": "9cdceb2d", "kind": "background", "entrypoint": "cli", "cwd": cwd}).to_string(),
        )
        .unwrap();
        job_row(&env, "9cdceb2d", "s-sh");
        let mut v = labels_of(&env, &id);
        v.insert("NIMITZ_HOOK_PID".into(), me.to_string());
        let start = serde_json::json!({"session_id": "s-sh", "source": "startup", "cwd": cwd}).to_string();
        handle(&env, "SessionStart", &start, &v);
        let e = crate::evidence::events(&env).pop().expect("a line");
        assert_eq!((e.pid, e.job.as_deref(), e.kind.as_deref()), (Some(runner), Some("9cdceb2d"), Some("background")), "{e:?}");
        assert_eq!(crate::registry::load(&env).identities[&id.0].job.as_deref(), Some("9cdceb2d"));
    }
}
