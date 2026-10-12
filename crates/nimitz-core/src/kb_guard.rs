//! The kb-guard (`nimitz hook kb-guard`, a plugin-level PreToolUse hook on
//! every tool): plugin subagents ignore a `hooks:` in their frontmatter, so
//! the limits of the `kb-scribe` and `kb-ref` subagents (docs/design/kb.md
//! section 3) are held here, by the payload's `agent_type`, whatever the
//! session's permission mode. What an agent may do is allowed (so a member
//! in the background never stops on a permission prompt for its KB); the
//! rest is denied with the reason. Any other agent, and the session itself,
//! get no answer at all.
//!
//! - kb-scribe: Bash only as one `nimitz kb path|status|grep|hash|commit|restore`
//!   (no `;`, `&`, `|`, redirects, globs or comments; `$`, backticks and `\`
//!   only inside single quotes; no `--fleet`); Read
//!   inside the KB but `.git`; Edit and Write inside the KB, never `.git*`,
//!   `.claude`, a CLAUDE.md, `agent/`, the top-level docs, or an existing
//!   `raw/` file.
//! - kb-ref: Bash only as one `nimitz kb path|status|head|show|grep`; Read
//!   inside the KB but `.git`; nothing else.
use crate::env::Env;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Scribe,
    Ref,
}

impl Agent {
    /// The agent a payload's `agent_type` names: only `nimitz:kb-scribe` and
    /// `nimitz:kb-ref`, as Claude Code names the plugin's agents.
    pub fn of(agent_type: &str) -> Option<Agent> {
        // the plugin's own, as Claude Code names them: a project's or the
        // user's agent of the same bare name is not ours to allow
        match agent_type {
            "nimitz:kb-scribe" => Some(Agent::Scribe),
            "nimitz:kb-ref" => Some(Agent::Ref),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Agent::Scribe => "kb-scribe",
            Agent::Ref => "kb-ref",
        }
    }

    fn verbs(self) -> &'static [&'static str] {
        match self {
            Agent::Scribe => &["path", "status", "grep", "hash", "commit", "restore"],
            Agent::Ref => &["path", "status", "head", "show", "grep"],
        }
    }
}

/// The words of a command, or why it is not one plain command: quotes are
/// read as a shell reads them, and anything that would make the shell do
/// more than run one program with those words is refused.
pub fn words(cmd: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = vec![];
    let mut cur: Option<String> = None;
    let mut quote: Option<char> = None;
    for c in cmd.trim().chars() {
        if c == '\n' || c == '\r' || c == '\0' {
            return Err("one line: a newline is not allowed in the command".into());
        }
        // `$`, backticks and `\` are literal only inside single quotes
        if (c == '$' || c == '`' || c == '\\') && quote != Some('\'') {
            return Err(format!("{c:?} is allowed only inside single quotes (one plain `nimitz kb ...` command)"));
        }
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.get_or_insert_with(String::new).push(c),
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    cur.get_or_insert_with(String::new);
                }
                ' ' | '\t' => {
                    if let Some(w) = cur.take() {
                        out.push(w);
                    }
                }
                ';' | '&' | '|' | '<' | '>' | '(' | ')' | '{' | '}' | '*' | '?' | '[' | ']' | '#' | '~' | '!' => {
                    return Err(format!("{c:?} outside quotes is not allowed (one plain `nimitz kb ...` command; quote a title)"));
                }
                c => cur.get_or_insert_with(String::new).push(c),
            },
        }
    }
    if quote.is_some() {
        return Err("an unclosed quote".into());
    }
    out.extend(cur);
    Ok(out)
}

/// A path the tool names, made real (symlinks followed as far as it exists,
/// the rest appended), and its place under `root` (already real), or why not.
fn under(root: &Path, file: &str) -> Result<PathBuf, String> {
    let p = Path::new(file);
    if !p.is_absolute() {
        return Err(format!("{file}: not an absolute path"));
    }
    if p.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err(format!("{file}: no `..` or `.`"));
    }
    let mut existing = p.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = vec![];
    let real = loop {
        if let Ok(r) = crate::os::canonical(&existing) {
            break r;
        }
        match (existing.file_name().map(|n| n.to_os_string()), existing.parent()) {
            (Some(n), Some(up)) => {
                rest.push(n);
                existing = up.to_path_buf();
            }
            _ => return Err(format!("{file}: no such folder")),
        }
    };
    // a name past what resolves that is there all the same is a link that
    // leads nowhere yet (a write through it would land wherever it points)
    let mut full = real;
    for n in rest.into_iter().rev() {
        full = full.join(n);
        if std::fs::symlink_metadata(&full).is_ok() {
            return Err(format!("{file}: {} is there but does not resolve (a link that leads nowhere, or a file used as a folder)", full.display()));
        }
    }
    full.strip_prefix(root).map(Path::to_path_buf).map_err(|_| format!("{file} is outside the knowledge base ({})", root.display()))
}

fn is_git(rel: &Path) -> bool {
    rel.components().next().is_some_and(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case(".git"))
}

/// The decision on one tool call of `agent`: Ok(why it may), Err(why not).
/// `root`: the KB's folder here, real (None: no KB for the session's fleet,
/// or no fleet).
pub fn decide(agent: Agent, root: Option<&Path>, tool: &str, input: &Value) -> Result<String, String> {
    let name = agent.name();
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    let no_kb = || format!("{name}: no knowledge base for this session (outside a fleet, or the fleet names none here): stop and report it");
    match tool {
        "Bash" => {
            let w = words(s("command")).map_err(|e| format!("{name}: {e}"))?;
            let verb = match (w.first().map(String::as_str), w.get(1).map(String::as_str), w.get(2)) {
                (Some("nimitz"), Some("kb"), Some(v)) => v.as_str(),
                _ => return Err(format!("{name}: Bash runs only `nimitz kb {}`, one command per call", agent.verbs().join("|"))),
            };
            if !agent.verbs().contains(&verb) {
                return Err(format!("{name}: `nimitz kb {verb}` is not {name}'s (it runs {})", agent.verbs().join(", ")));
            }
            // the session's own fleet only: never another one named on the line
            if w.iter().any(|x| x == "--fleet" || x.starts_with("--fleet=")) {
                return Err(format!("{name}: no --fleet: the knowledge base is the session's own fleet's"));
            }
            // outside a fleet only `path`, which says there is no KB
            if root.is_none() && verb != "path" {
                return Err(no_kb());
            }
            Ok(format!("{name}: nimitz kb {verb}"))
        }
        "Read" => {
            let root = root.ok_or_else(no_kb)?;
            let rel = under(root, s("file_path")).map_err(|e| format!("{name}: {e}"))?;
            if is_git(&rel) {
                return Err(format!("{name}: .git is nimitz's to read (nimitz kb)"));
            }
            Ok(format!("{name}: read {}", rel.display()))
        }
        "Edit" | "Write" | "MultiEdit" if agent == Agent::Scribe => {
            let root = root.ok_or_else(no_kb)?;
            let rel = under(root, s("file_path")).map_err(|e| format!("{name}: {e}"))?;
            let r = rel.to_string_lossy().replace('\\', "/");
            if let Some(off) = crate::kb::off_limits(&r) {
                return Err(format!("{name}: {r} is off limits ({off})"));
            }
            if r.to_lowercase().starts_with("raw/") && std::fs::symlink_metadata(root.join(&rel)).is_ok() {
                return Err(format!("{name}: {r}: an existing raw/ file is never changed (a capture is always a new file)"));
            }
            Ok(format!("{name}: write {r}"))
        }
        "Edit" | "Write" | "MultiEdit" => Err(format!("{name} is read-only: it never writes")),
        t => Err(format!("{name} has no {t}: it uses {}", if agent == Agent::Scribe { "Bash (nimitz kb), Read, Edit and Write" } else { "Bash (nimitz kb) and Read" })),
    }
}

/// Claude Code's own tools for a subagent's work with its caller: delivering
/// its result (`SubagentHandback`, which Claude Code gives a subagent in auto
/// mode whatever its `tools:` say), its own todo list, loading a tool's schema
/// (the tool it loads still meets the guard). They reach no file, command or
/// network: the guard gives them no answer, and Claude Code decides as for any
/// session. Never an explicit allow: only auto mode's review may allow
/// `SubagentHandback`, and Claude Code turns a hook's allow into a deny.
/// (Denying it kept the agents' reports from their callers, 0.7.0.) The task
/// tools stay denied: they write the session's task list and fire its hooks.
pub const CALLER_TOOLS: [&str; 3] = ["SubagentHandback", "TodoWrite", "ToolSearch"];

/// The session's fleet, by its labels judged as every nimitz command judges
/// them (`caller_from_env`: partial or disagreeing labels are no member's).
fn fleet_of(vars: &HashMap<String, String>) -> Option<String> {
    match crate::sessions::caller_from_env(vars) {
        crate::types::Caller::Member { id, .. } if id.0 != crate::sessions::UNVERIFIED => Some(id.fleet().to_string()),
        crate::types::Caller::Member { .. } => None,
        crate::types::Caller::User => None,
    }
}

/// `nimitz hook kb-guard`: the hook's answer for `stdin` (empty: no
/// answer, not one of the KB agents).
pub fn hook(env: &Env, stdin: &str, vars: &HashMap<String, String>) -> String {
    // the cheap test first: this hook runs on every tool call of every session
    if !stdin.contains("kb-scribe") && !stdin.contains("kb-ref") {
        return String::new();
    }
    let input: Value = serde_json::from_str(stdin.trim()).unwrap_or(Value::Null);
    let Some(agent) = input.get("agent_type").and_then(Value::as_str).and_then(Agent::of) else { return String::new() };
    let tool = input.get("tool_name").and_then(Value::as_str).unwrap_or("");
    if CALLER_TOOLS.contains(&tool) {
        return String::new();
    }
    let root = fleet_of(vars).and_then(|f| crate::fleet_settings::kb(env, &f).ok()).and_then(|k| crate::os::canonical(&k.dir).ok());
    let (decision, reason) = match decide(agent, root.as_deref(), tool, input.get("tool_input").unwrap_or(&Value::Null)) {
        Ok(why) => ("allow", why),
        Err(why) => ("deny", why),
    };
    json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": decision, "permissionDecisionReason": reason}}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_one_plain_nimitz_kb_call() {
        assert_eq!(words("nimitz kb commit --title \"A; b\" --paths raw/c.md,wiki/x.md").unwrap(), ["nimitz", "kb", "commit", "--title", "A; b", "--paths", "raw/c.md,wiki/x.md"]);
        assert_eq!(words("nimitz kb grep 'two words' wiki").unwrap(), ["nimitz", "kb", "grep", "two words", "wiki"]);
        for bad in ["nimitz kb path; rm -rf /", "nimitz kb path && x", "nimitz kb path | sh", "nimitz kb show $(x)", "nimitz kb show `x`", "nimitz kb grep \"$HOME\"", "nimitz kb grep \"a\\\\b\"", "nimitz kb path > /tmp/x", "nimitz kb grep *", "nimitz kb path # x", "nimitz kb path\nrm x", "nimitz kb grep 'x", "(nimitz kb path)", "nimitz kb show ~/.ssh/id", "nimitz kb grep {a,b}"] {
            assert!(words(bad).is_err(), "{bad}");
        }
        assert_eq!(words("nimitz kb commit --title 'Costs $5, `x` \\ y'").unwrap()[4], "Costs $5, `x` \\ y", "literal in single quotes");
        let input = |c: &str| json!({"command": c});
        let td = tempfile::tempdir().unwrap();
        let r = Some(td.path());
        assert!(decide(Agent::Scribe, r, "Bash", &input("nimitz kb commit --note x")).is_ok());
        assert!(decide(Agent::Ref, r, "Bash", &input("nimitz kb show wiki/_index.md --at abc123")).is_ok());
        // another fleet's KB, or none: only `path` (which says why)
        for c in ["nimitz kb commit --fleet HOME --paths x", "nimitz kb path --fleet=HOME"] {
            assert!(decide(Agent::Scribe, r, "Bash", &input(c)).unwrap_err().contains("no --fleet"), "{c}");
        }
        assert!(decide(Agent::Scribe, None, "Bash", &input("nimitz kb path")).is_ok());
        assert!(decide(Agent::Scribe, None, "Bash", &input("nimitz kb restore --paths x")).unwrap_err().contains("no knowledge base"));
        for (a, c) in [(Agent::Ref, "nimitz kb commit --note x"), (Agent::Scribe, "nimitz kb clone --yes"), (Agent::Scribe, "nimitz kb sync"), (Agent::Scribe, "git status"), (Agent::Scribe, "/tmp/nimitz kb path"), (Agent::Scribe, "nimitz status"), (Agent::Ref, "nimitz kb")] {
            assert!(decide(a, r, "Bash", &input(c)).is_err(), "{a:?} {c}");
        }
    }

    #[test]
    fn reads_and_writes_stay_inside_the_kb_and_off_its_rules() {
        let td = tempfile::tempdir().unwrap();
        let root = crate::os::canonical(td.path()).unwrap().join("kb");
        for d in ["wiki", "raw", "agent", ".git"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("raw/old.md"), "x").unwrap();
        let p = |rel: &str| json!({"file_path": root.join(rel).to_string_lossy()});
        let r = Some(root.as_path());
        assert!(decide(Agent::Scribe, r, "Write", &p("wiki/new/page.md")).is_ok(), "a new folder too");
        assert!(decide(Agent::Scribe, r, "Edit", &p("wiki/page.md")).is_ok());
        assert!(decide(Agent::Scribe, r, "Write", &p("raw/captures/c1.md")).is_ok());
        for rel in ["CLAUDE.md", "wiki/CLAUDE.md", "agent/skills/x.md", ".git/config", ".gitignore", "wiki/.gitattributes", "README.md", ".claude/settings.json", "raw/old.md"] {
            assert!(decide(Agent::Scribe, r, "Write", &p(rel)).is_err(), "{rel}");
        }
        assert!(decide(Agent::Scribe, r, "Read", &p("CLAUDE.md")).is_ok(), "the rules are read, never written");
        assert!(decide(Agent::Ref, r, "Read", &p("agent/page-templates.md")).is_ok());
        assert!(decide(Agent::Ref, r, "Read", &p(".git/config")).is_err());
        assert!(decide(Agent::Ref, r, "Write", &p("wiki/x.md")).is_err(), "kb-ref never writes");
        // outside, through `..`, through a link that leads out, relative: refused
        let out = json!({"file_path": td.path().join("elsewhere.md").to_string_lossy()});
        assert!(decide(Agent::Scribe, r, "Read", &out).is_err());
        assert!(decide(Agent::Scribe, r, "Write", &json!({"file_path": format!("{}/wiki/../../x", root.display())})).is_err());
        assert!(decide(Agent::Scribe, r, "Write", &json!({"file_path": "wiki/x.md"})).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(td.path(), root.join("wiki/link")).unwrap();
            assert!(decide(Agent::Scribe, r, "Write", &p("wiki/link/escape.md")).is_err());
            // a link to a file not there yet (a pushed commit could hold one)
            std::os::unix::fs::symlink(td.path().join("not-yet.md"), root.join("wiki/dangling.md")).unwrap();
            assert!(decide(Agent::Scribe, r, "Write", &p("wiki/dangling.md")).unwrap_err().contains("does not resolve"));
            std::os::unix::fs::symlink(td.path().join("nope.md"), root.join("raw/cap-link.md")).unwrap();
            assert!(decide(Agent::Scribe, r, "Write", &p("raw/cap-link.md")).is_err());
        }
        // no KB: nothing but Bash (whose `nimitz kb path` says why)
        assert!(decide(Agent::Scribe, None, "Read", &p("wiki/x.md")).unwrap_err().contains("no knowledge base"));
        assert!(decide(Agent::Scribe, r, "Glob", &json!({})).is_err() && decide(Agent::Ref, r, "WebFetch", &json!({})).is_err());
    }

    #[test]
    fn the_hook_answers_only_for_the_kb_agents() {
        let (td, env) = crate::config::tests::test_env();
        let vars: HashMap<String, String> = HashMap::new();
        let call = |agent: Option<&str>, tool: &str, input: Value| {
            let mut v = json!({"hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": input});
            if let Some(a) = agent {
                v["agent_type"] = json!(a);
            }
            hook(&env, &v.to_string(), &vars)
        };
        assert_eq!(call(None, "Bash", json!({"command": "rm -rf kb-scribe"})), "", "the session itself: no answer");
        // the fleet by the labels: whole ones find MAIN's KB; partial ones none
        crate::config::tests::make_fleet(&env);
        let kb = td.path().join("kb");
        std::fs::create_dir_all(&kb).unwrap();
        for (k, v) in [("kb.remote", json!("/srv/kb.git")), ("kb.dirs.test-mac", json!(kb.to_string_lossy()))] {
            assert!(crate::config::apply_config_plan(&env, &crate::fleet_settings::plan_set(&env, "MAIN", k, Some(v), true)).verified);
        }
        let whole: HashMap<String, String> = [("NIMITZ_ID", "MAIN/ARGOS/ARGOS"), ("NIMITZ_ROLE", "ship")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let partial: HashMap<String, String> = [("NIMITZ_FLEET", "MAIN")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let read = json!({"hook_event_name": "PreToolUse", "agent_type": "nimitz:kb-ref", "tool_name": "Read", "tool_input": {"file_path": kb.join("wiki/x.md").to_string_lossy()}}).to_string();
        assert!(hook(&env, &read, &whole).contains("\"allow\""), "{}", hook(&env, &read, &whole));
        assert!(hook(&env, &read, &partial).contains("no knowledge base"));
        // labels that disagree (an id with no role, or another role): unverified, no KB
        for bad in [vec![("NIMITZ_ID", "MAIN/ARGOS/ARGOS")], vec![("NIMITZ_ID", "MAIN/ARGOS/ARGOS"), ("NIMITZ_ROLE", "admiral")]] {
            let v: HashMap<String, String> = bad.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            assert!(hook(&env, &read, &v).contains("no knowledge base"), "{bad:?}");
        }
        assert_eq!(call(Some("general-purpose"), "Bash", json!({"command": "ls"})), "");
        assert_eq!(call(Some("kb-scribe"), "Write", json!({"file_path": "/tmp/x"})), "", "a bare name is a project's agent, not the plugin's");
        // delivering its result to its caller is never the guard's to stop
        for t in CALLER_TOOLS {
            assert_eq!(call(Some("nimitz:kb-scribe"), t, json!({"message": "done"})), "", "{t}");
            assert_eq!(call(Some("nimitz:kb-ref"), t, json!({"message": "done"})), "", "{t}");
        }
        // every other tool still is: what reaches files, commands, the network, other agents
        for t in ["Glob", "Grep", "WebFetch", "WebSearch", "Agent", "Task", "NotebookEdit", "SendMessage", "TaskCreate", "TaskUpdate", "TaskList", "TaskGet", "TaskOutput", "TaskStop", "mcp__x__y"] {
            let v: Value = serde_json::from_str(&call(Some("nimitz:kb-ref"), t, json!({}))).unwrap();
            assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny", "{t}");
        }
        let a: Value = serde_json::from_str(&call(Some("nimitz:kb-scribe"), "Bash", json!({"command": "nimitz kb path"}))).unwrap();
        assert_eq!(a["hookSpecificOutput"]["permissionDecision"], "allow");
        let d: Value = serde_json::from_str(&call(Some("nimitz:kb-ref"), "Read", json!({"file_path": "/etc/passwd"}))).unwrap();
        assert_eq!(d["hookSpecificOutput"]["permissionDecision"], "deny", "{d}");
        assert!(d["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().contains("no knowledge base"));
    }
}
