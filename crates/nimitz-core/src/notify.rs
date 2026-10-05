//! Needs-you notices: a ship or tender that blocks on the user (an
//! AskUserQuestion, a permission prompt) has no pane anyone watches, so its
//! commodore is told at once, with the question in full. Owner: core-sessions.
//!
//! The hook only records: it writes `NIMITZ_STATE/needs/<id>.json` and returns;
//! the `nimitz` binary then hands delivery to a detached process (the hook must
//! never block or slow the member). Delivery writes its outcome back into the
//! same file, so `nimitz check` sees a notice that never reached the commodore.
//! The file is cleared when the member moves on (its next prompt, Stop or
//! SessionEnd).
use crate::env::Env;
use crate::types::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A Notification right after a richer notice for the same session is the same
/// prompt seen twice (Claude Code sends both); it is dropped.
pub const DUP_WINDOW_S: f64 = 30.0;
/// A notice still `pending` after this long lost its delivery process.
pub const PENDING_STALE_S: f64 = 900.0;
/// The most text one notice types into a pane.
pub const MAX_TEXT: usize = 4000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub ts: f64,
    /// the blocked member, `F/SQ/SHIP` or `F/SQ/~task`
    pub id: String,
    pub session_id: Option<String>,
    /// question | permission | prompt
    pub kind: String,
    /// the hook event that saw it
    pub event: String,
    pub tool_use_id: Option<String>,
    /// one line: what it waits on, in full (question and options, or tool and input)
    pub detail: String,
    /// pending | delivered | failed
    pub delivery: String,
    /// why a delivery failed, or where it went
    #[serde(default)]
    pub delivery_note: Option<String>,
    #[serde(default)]
    pub delivered_at: Option<f64>,
}

impl Notice {
    /// The text typed into the commodore's pane. It is framed as a nimitz
    /// notice, not the user's words: the member's question is quoted data.
    pub fn text(&self) -> String {
        let id = Id(self.id.clone());
        let what = match self.kind.as_str() {
            "question" => "asks the user",
            "permission" => "asks the user for permission",
            _ => "waits for the user",
        };
        let sid = self.session_id.as_deref().map(|s| format!(", session {}", &s[..s.len().min(8)])).unwrap_or_default();
        let t = format!(
            "[nimitz needs-you] (a notice from nimitz, not from the user) {} ({}{sid}) is blocked: it {what}. Quoted: <<{}>> Tell the user; do not answer it yourself.",
            id.0,
            id.role().as_str(),
            self.detail
        );
        clip(&t, MAX_TEXT)
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(15)).collect();
    out.push_str(" ...[truncated]");
    out
}

/// One line: newlines and runs of blanks become one space.
fn line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn str_at(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_string)
}

/// `AskUserQuestion` input, every question with its options.
fn questions(input: &Value) -> Option<String> {
    let qs = input.get("questions")?.as_array()?;
    let mut parts = vec![];
    for (i, q) in qs.iter().enumerate() {
        let mut p = format!("Q{}: {}", i + 1, line(&str_at(q, "question").unwrap_or_default()));
        if let Some(h) = str_at(q, "header").filter(|h| !h.is_empty()) {
            p.push_str(&format!(" [{}]", line(&h)));
        }
        if q.get("multiSelect").and_then(|m| m.as_bool()) == Some(true) {
            p.push_str(" (choose any)");
        }
        let opts: Vec<String> = q
            .get("options")
            .and_then(|o| o.as_array())
            .map(|o| {
                o.iter()
                    .enumerate()
                    .map(|(j, o)| {
                        let label = line(&str_at(o, "label").unwrap_or_default());
                        match str_at(o, "description").filter(|d| !d.is_empty()) {
                            Some(d) => format!("({}) {label}: {}", j + 1, line(&d)),
                            None => format!("({}) {label}", j + 1),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !opts.is_empty() {
            p.push_str(&format!(" Options: {}", opts.join("; ")));
        }
        parts.push(p);
    }
    (!parts.is_empty()).then(|| parts.join(" | "))
}

/// What this hook call says the member now waits on, if anything. Only ships
/// and tenders: an admiral or commodore sits in a pane the user watches.
pub fn from_hook(event: &str, input: &Value, labels: &BTreeMap<String, String>) -> Option<Notice> {
    let id = Id(labels.get("NIMITZ_ID")?.clone());
    if !matches!(id.role(), Role::Ship | Role::Tender) || labels.get("NIMITZ_ROLE").map(String::as_str) != Some(id.role().as_str()) {
        return None;
    }
    let tool = str_at(input, "tool_name");
    let tool_input = input.get("tool_input").cloned().unwrap_or(Value::Null);
    let (kind, detail) = match event {
        "PreToolUse" if tool.as_deref() == Some("AskUserQuestion") => {
            ("question", questions(&tool_input).unwrap_or_else(|| format!("AskUserQuestion {}", line(&tool_input.to_string()))))
        }
        "PermissionRequest" => {
            let tool = tool.unwrap_or_else(|| "a tool".into());
            ("permission", format!("may it use {tool}? input: {}", clip(&line(&tool_input.to_string()), 1500)))
        }
        // A dialog that blocks (permission_prompt fires after ~6 s unanswered; the
        // elicitation ones are MCP input requests). idle_prompt, agent_completed,
        // auth and quota notices are not a block. No type at all (an older
        // build): the message decides.
        "Notification" => match str_at(input, "notification_type").as_deref() {
            Some("permission_prompt") | Some("elicitation_dialog") | Some("elicitation_url_dialog") | None => {
                ("prompt", line(&str_at(input, "message").filter(|m| !m.is_empty())?))
            }
            Some(_) => return None,
        },
        _ => return None,
    };
    Some(Notice {
        ts: crate::evidence::now(),
        id: id.0,
        session_id: str_at(input, "session_id"),
        kind: kind.into(),
        event: event.into(),
        tool_use_id: str_at(input, "tool_use_id"),
        detail,
        delivery: "pending".into(),
        ..Default::default()
    })
}

/// `NIMITZ_STATE/needs/`.
pub fn dir(env: &Env) -> PathBuf {
    env.state.join("needs")
}

/// `NIMITZ_STATE/needs/<id with / as %2F>.json`.
pub fn path(env: &Env, id: &str) -> PathBuf {
    dir(env).join(format!("{}.json", id.replace('/', "%2F")))
}

pub fn read(env: &Env, id: &str) -> Option<Notice> {
    serde_json::from_slice(&std::fs::read(path(env, id)).ok()?).ok()
}

/// Atomic write (temp + rename): the delivery process and the hook share the file.
pub fn write(env: &Env, n: &Notice) -> std::io::Result<()> {
    let p = path(env, &n.id);
    std::fs::create_dir_all(dir(env))?;
    let tmp = p.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(n).unwrap_or_default())?;
    std::fs::rename(&tmp, &p)
}

/// Record a notice unless it repeats the one on file. Returns the notice to deliver.
pub fn record(env: &Env, n: Notice) -> Option<Notice> {
    if let Some(old) = read(env, &n.id).filter(|o| o.session_id == n.session_id && n.ts - o.ts < DUP_WINDOW_S) {
        let same_call = n.tool_use_id.is_some() && n.tool_use_id == old.tool_use_id;
        // a Notification (or a permission prompt for AskUserQuestion itself) right
        // after the richer notice is the same prompt
        if same_call || n.kind == "prompt" || (old.kind == "question" && n.detail.contains("AskUserQuestion")) {
            return None;
        }
    }
    write(env, &n).ok()?;
    Some(n)
}

/// The member moved on (a new prompt, Stop, SessionEnd): its notice is done.
pub fn clear(env: &Env, id: &str, session_id: Option<&str>) {
    if read(env, id).is_some_and(|n| n.session_id.as_deref() == session_id || session_id.is_none()) {
        let _ = std::fs::remove_file(path(env, id));
    }
}

/// Write the outcome of a delivery, unless the notice on file has moved on.
pub fn set_delivery(env: &Env, n: &Notice, delivery: &str, note: Option<String>) {
    let Some(mut cur) = read(env, &n.id) else { return };
    if cur.ts != n.ts {
        return; // superseded or answered meanwhile
    }
    cur.delivery = delivery.into();
    cur.delivery_note = note;
    if delivery == "delivered" {
        cur.delivered_at = Some(crate::evidence::now());
    }
    let _ = write(env, &cur);
}

/// `notify.failed` for each notice that never reached its commodore (or whose
/// delivery process vanished), with the question so the user can act on it.
pub fn issues(env: &Env) -> Vec<Issue> {
    let mut out = vec![];
    let Ok(rd) = std::fs::read_dir(dir(env)) else { return out };
    let now = crate::evidence::now();
    let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    for p in files {
        let Some(n) = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Notice>(&b).ok()) else { continue };
        let why = match n.delivery.as_str() {
            "failed" => n.delivery_note.clone().unwrap_or_else(|| "delivery failed".into()),
            "pending" if now - n.ts > PENDING_STALE_S => "its delivery never finished".into(),
            _ => continue,
        };
        let id = Id(n.id.clone());
        let commodore = Id::commodore(id.fleet(), id.squadron().unwrap_or_default());
        out.push(Issue {
            code: "notify.failed".into(),
            severity: Severity::Warn,
            message: format!("{} waits for the user and {commodore} was not told ({why}): {}", n.id, clip(&n.detail, 300)),
            fix: Some(format!("nimitz open {}  # answer it there", n.id)),
            id: Some(id),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::test_env;
    use serde_json::json;

    fn labels(id: &str, role: &str) -> BTreeMap<String, String> {
        [("NIMITZ_ID".to_string(), id.to_string()), ("NIMITZ_ROLE".to_string(), role.to_string())].into()
    }

    fn ask() -> Value {
        json!({"session_id": "s1", "hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion", "tool_use_id": "t1",
            "tool_input": {"questions": [{"question": "Register the MCP server\nat user scope?", "header": "MCP add", "multiSelect": false,
                "options": [{"label": "Yes", "description": "every session gets it"}, {"label": "No", "description": ""}]}]}})
    }

    #[test]
    fn a_question_carries_its_text_and_every_option() {
        let n = from_hook("PreToolUse", &ask(), &labels("MAIN/ARGOS/CLEAN", "ship")).unwrap();
        assert_eq!((n.kind.as_str(), n.id.as_str(), n.session_id.as_deref()), ("question", "MAIN/ARGOS/CLEAN", Some("s1")));
        assert_eq!(n.detail, "Q1: Register the MCP server at user scope? [MCP add] Options: (1) Yes: every session gets it; (2) No");
        let t = n.text();
        assert!(t.starts_with("[nimitz needs-you] (a notice from nimitz, not from the user) MAIN/ARGOS/CLEAN (ship, session s1)"), "{t}");
        assert!(!t.contains('\n'), "one line: a newline would submit early");
    }

    #[test]
    fn a_permission_request_carries_the_tool_and_its_input() {
        let input = json!({"session_id": "s1", "tool_name": "Bash", "tool_input": {"command": "rm -rf build"}});
        let n = from_hook("PermissionRequest", &input, &labels("MAIN/ARGOS/~fix", "tender")).unwrap();
        assert_eq!(n.kind, "permission");
        assert!(n.detail.contains("Bash") && n.detail.contains("rm -rf build"), "{}", n.detail);
    }

    #[test]
    fn only_ships_and_tenders_with_whole_labels_notify() {
        assert!(from_hook("PreToolUse", &ask(), &labels("MAIN/ARGOS", "commodore")).is_none(), "a commodore has a pane");
        assert!(from_hook("PreToolUse", &ask(), &labels("MAIN", "admiral")).is_none());
        assert!(from_hook("PreToolUse", &ask(), &labels("MAIN/ARGOS/CLEAN", "commodore")).is_none(), "labels that disagree");
        assert!(from_hook("PreToolUse", &ask(), &BTreeMap::new()).is_none(), "the user's own session");
        // other tools, idle notices and empty messages are not a block
        let read = json!({"tool_name": "Read", "tool_input": {}});
        assert!(from_hook("PreToolUse", &read, &labels("MAIN/ARGOS/CLEAN", "ship")).is_none());
        for t in ["idle_prompt", "agent_completed", "auth_success", "elicitation_complete", "quota_auto_resume_fired"] {
            let n = json!({"notification_type": t, "message": "Claude is waiting for your input"});
            assert!(from_hook("Notification", &n, &labels("MAIN/ARGOS/CLEAN", "ship")).is_none(), "{t}");
        }
        assert!(from_hook("Notification", &json!({}), &labels("MAIN/ARGOS/CLEAN", "ship")).is_none());
    }

    #[test]
    fn the_same_prompt_seen_twice_is_one_notice() {
        let (_td, env) = test_env();
        let ship = labels("MAIN/ARGOS/CLEAN", "ship");
        let first = from_hook("PreToolUse", &ask(), &ship).unwrap();
        assert!(record(&env, first.clone()).is_some());
        // the Notification Claude Code sends for the same dialog
        let note = json!({"session_id": "s1", "notification_type": "permission_prompt", "message": "Claude needs your permission"});
        assert!(record(&env, from_hook("Notification", &note, &ship).unwrap()).is_none());
        assert_eq!(read(&env, "MAIN/ARGOS/CLEAN").unwrap().kind, "question", "the richer notice stays");
        // a new permission prompt is a new notice
        let perm = json!({"session_id": "s1", "tool_name": "Bash", "tool_input": {"command": "make"}});
        assert!(record(&env, from_hook("PermissionRequest", &perm, &ship).unwrap()).is_some());
        // a Notification alone (no richer hook fired) is still a notice
        clear(&env, "MAIN/ARGOS/CLEAN", Some("s1"));
        assert!(record(&env, from_hook("Notification", &note, &ship).unwrap()).is_some());
    }

    #[test]
    fn a_failed_or_lost_delivery_is_a_check_issue_until_the_member_moves_on() {
        let (_td, env) = test_env();
        let n = record(&env, from_hook("PreToolUse", &ask(), &labels("MAIN/ARGOS/CLEAN", "ship")).unwrap()).unwrap();
        assert!(issues(&env).is_empty(), "pending, and young");
        set_delivery(&env, &n, "failed", Some("[MAIN.ARGOS]-COMMODORE is not live in a herdr pane".into()));
        let is = issues(&env);
        assert_eq!(is.len(), 1);
        assert_eq!(is[0].code, "notify.failed");
        assert!(is[0].message.contains("not live in a herdr pane") && is[0].message.contains("Register the MCP server"), "{}", is[0].message);
        assert_eq!(is[0].id.as_ref().map(|i| i.0.as_str()), Some("MAIN/ARGOS/CLEAN"));
        // delivered: no issue
        set_delivery(&env, &n, "delivered", None);
        assert!(issues(&env).is_empty());
        // a pending notice whose delivery process died
        let mut old = read(&env, "MAIN/ARGOS/CLEAN").unwrap();
        old.delivery = "pending".into();
        old.ts -= PENDING_STALE_S + 1.0;
        write(&env, &old).unwrap();
        assert!(issues(&env)[0].message.contains("never finished"));
        // another session's end does not clear it; its own does
        clear(&env, "MAIN/ARGOS/CLEAN", Some("other"));
        assert_eq!(issues(&env).len(), 1);
        clear(&env, "MAIN/ARGOS/CLEAN", Some("s1"));
        assert!(issues(&env).is_empty());
    }

    #[test]
    fn a_stale_delivery_result_does_not_overwrite_a_newer_notice() {
        let (_td, env) = test_env();
        let ship = labels("MAIN/ARGOS/CLEAN", "ship");
        let first = record(&env, from_hook("PreToolUse", &ask(), &ship).unwrap()).unwrap();
        let mut second = from_hook("PermissionRequest", &json!({"session_id": "s1", "tool_name": "Bash"}), &ship).unwrap();
        second.ts = first.ts + 1.0;
        record(&env, second).unwrap();
        set_delivery(&env, &first, "failed", Some("late".into()));
        assert_eq!(read(&env, "MAIN/ARGOS/CLEAN").unwrap().delivery, "pending");
    }

    #[test]
    fn a_huge_question_is_clipped() {
        let mut input = ask();
        input["tool_input"]["questions"][0]["question"] = Value::from("x".repeat(10_000));
        let n = from_hook("PreToolUse", &input, &labels("MAIN/ARGOS/CLEAN", "ship")).unwrap();
        let t = n.text();
        assert!(t.chars().count() <= MAX_TEXT && t.ends_with("[truncated]"), "{}", t.len());
    }
}
