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
/// Every `claude` process nimitz starts begins with none of the caller's
/// session state: a member's labels go only through `--settings`. Claude
/// Code's daemon is started by whichever `claude` call finds none running and
/// keeps that process's environment for every job it starts; a daemon started
/// from a labelled lead's shell labelled every unlabelled job as that lead
/// and gave them its pane and its messaging socket. Removed (a denylist of
/// session state; Claude Code's own: `CLAUDE_CODE_ENTRYPOINT`,
/// `CLAUDE_CODE_MESSAGING_*` are per session, every other `CLAUDE_CODE_*` and
/// `ANTHROPIC_*` is the user's config and passes): nimitz's identity and
/// multiplexer names, every `HERDR_*`, `CLAUDE_PID`, `CLAUDECODE`,
/// `CLAUDE_BG_*`, and the session-scoped `CLAUDE_CODE_*` below.
pub fn scrub_child_env(c: &mut Command) {
    const NAMES: &[&str] = &[
        "NIMITZ_ID", "NIMITZ_ROLE", "NIMITZ_FLEET", "NIMITZ_SQUADRON", "NIMITZ_PARENT",
        "NIMITZ_MUX", "NIMITZ_HERDR_SESSION", "NIMITZ_HERDR_SOCKET", "NIMITZ_HOOK_PID", "NIMITZ_HOOK_SESSION_FILE",
        "CLAUDE_PID", "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_EXECPATH", "CLAUDE_CODE_MESSAGING_SOCKET", "CLAUDE_CODE_MESSAGING_TOKEN",
        "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_BRIDGE_SESSION_ID", "CLAUDE_CODE_CHILD_SESSION",
        // per session too (Claude Code strips them for its own children): the
        // launching job's attended mode, tmp dir and effort level, an SDK host's flags
        "CLAUDE_CODE_SESSION_ATTENDED", "CLAUDE_JOB_DIR", "CLAUDE_EFFORT",
        "CLAUDE_CODE_SDK_HAS_OAUTH_REFRESH", "CLAUDE_CODE_SDK_READS_SESSION_STATE",
        "CLAUDE_CODE_HOST_WORKTREE", "CLAUDE_CODE_HOST_WORKTREE_FENCE",
    ];
    const PREFIXES: &[&str] = &["HERDR_", "CLAUDE_BG_", "CLAUDE_CODE_RESUME_", "CLAUDE_BRIDGE_"];
    for (k, _) in std::env::vars_os() {
        let k = k.to_string_lossy().into_owned();
        if NAMES.contains(&k.as_str()) || PREFIXES.iter().any(|p| k.starts_with(p)) {
            c.env_remove(&k);
        }
    }
}

pub fn run(cmd: &[String], cwd: Option<&str>) -> (i32, String) {
    let Some(bin) = cmd.first() else { return (-1, "empty command".into()) };
    let mut c = Command::new(bin);
    c.args(&cmd[1..]).stdin(Stdio::null());
    scrub_child_env(&mut c);
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
    let mut c = Command::new(&env.native_claude_bin);
    c.args(["agents", "--json", "--all"]).stdin(Stdio::null());
    scrub_child_env(&mut c);
    let out = c
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

/// The labels `nimitz` gives a member's session (contracts.md section 4); an
/// aide's carry its parent (`NIMITZ_PARENT`), an admiral's aide no squadron.
pub fn labels_for(env: &Env, id: &Id) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("NIMITZ_FLEET".to_string(), id.fleet().to_string());
    if let Some(sq) = id.squadron() {
        m.insert("NIMITZ_SQUADRON".to_string(), sq.to_string());
    }
    if let Some(p) = id.parent().filter(|_| id.role() == Role::Aide) {
        m.insert("NIMITZ_PARENT".to_string(), p.0);
    }
    m.insert("NIMITZ_ROLE".to_string(), id.role().as_str().to_string());
    m.insert("NIMITZ_ID".to_string(), id.0.clone());
    m.insert("NIMITZ_HOME".to_string(), env.home.to_string_lossy().into_owned());
    m
}

/// The permission rule every member gets: it may run `nimitz` (the authority
/// table in `sessions::may_run` decides what each role may do with it).
pub const NIMITZ_ALLOW: &str = "Bash(nimitz *)";

/// The git subcommands a writer aide runs in its folder (`git -C <folder>
/// <sub>`) with no prompt: what an ingest needs, nothing that leaves the machine.
/// (No `grep`: `git grep -O<cmd>` runs a program; the Grep tool covers it.)
pub const AIDE_GIT_ALLOW: [&str; 9] = ["status", "add", "commit", "log", "diff", "show", "rev-parse", "ls-files", "restore --staged"];
/// Options of the allowed subcommands that write a file or run a program:
/// denied wherever they appear (`show --output=<file>`, `grep -O<cmd>`).
pub const AIDE_GIT_DENY_OPTS: [&str; 3] = ["--output", "-O", "--open-files-in-pager"];
/// Git that reaches another machine or rewrites the repo's setup: denied in
/// any form (a user's own `Bash(git *)` allow does not reach an aide).
pub const AIDE_GIT_DENY: [&str; 7] = ["push", "remote", "fetch", "pull", "clone", "config", "submodule"];

/// An aide's permissions (docs/design/aide.md 6.2, 6.4): (allow, deny).
/// Every aide may run `nimitz`; a writer the `AIDE_GIT_ALLOW` subcommands as
/// `git -C <its folder> ...` (its edits come from its permission mode), never
/// the `AIDE_GIT_DENY` ones; a snapshot reader may not edit at all. Anything
/// else prompts, and a prompt is a needs-you, not an action.
fn aide_permissions(env: &Env, id: &Id) -> (Vec<String>, Vec<String>) {
    let mut allow = vec![NIMITZ_ALLOW.to_string()];
    let mut deny: Vec<String> = AIDE_GIT_DENY.iter().flat_map(|s| [format!("Bash(git {s})"), format!("Bash(git {s} *)"), format!("Bash(git -C * {s})"), format!("Bash(git -C * {s} *)")]).collect();
    deny.extend(AIDE_GIT_DENY_OPTS.iter().map(|o| format!("Bash(git * {o}*)")));
    let Ok((f, def)) = crate::queue::find(env, id) else { return (allow, deny) };
    if def.snapshot {
        deny.extend(["Edit", "Write", "NotebookEdit"].map(String::from));
        return (allow, deny);
    }
    if let Some(dir) = crate::config::member_dir(env, &f, id) {
        let dir = dir.display();
        allow.extend(AIDE_GIT_ALLOW.iter().flat_map(|s| [format!("Bash(git -C {dir} {s})"), format!("Bash(git -C {dir} {s} *)")]));
    }
    (allow, deny)
}

/// `--settings` JSON: the labels as env, `permissions.allow` for `nimitz`
/// (an aide's own rules: `aide_permissions`), plus `worktree.bgIsolation
/// none` for background sessions. It is per launch and lives in the job's
/// `respawnFlags`, so a restart keeps it and a relaunch (every flag again)
/// re-adds it.
pub fn settings_json(env: &Env, id: &Id, background: bool) -> String {
    let mut vars = labels_for(env, id);
    // nimitz's own places: the member's hook writes where this nimitz reads,
    // whatever env the daemon that runs the job was started with (its env is
    // not the caller's)
    vars.insert("NIMITZ_STATE".to_string(), env.state.to_string_lossy().into_owned());
    vars.insert("NIMITZ_CLAUDE_HOME".to_string(), env.claude_home.to_string_lossy().into_owned());
    // (and the machine name it knows: an alias the caller set holds inside the job's hook)
    vars.insert("NIMITZ_HOSTNAME".to_string(), env.hostname.clone());
    let mut v = serde_json::json!({ "env": vars, "permissions": { "allow": [NIMITZ_ALLOW] } });
    if id.role() == Role::Aide {
        let (allow, deny) = aide_permissions(env, id);
        v["permissions"]["allow"] = serde_json::json!(allow);
        if !deny.is_empty() {
            v["permissions"]["deny"] = serde_json::json!(deny);
        }
    }
    if background {
        v["worktree"] = serde_json::json!({ "bgIsolation": "none" });
    }
    v.to_string()
}

/// The model an aide runs on (`[[aide]] model`, else Sonnet); None for every
/// other member (Claude Code's own default).
pub fn model_for(env: &Env, id: &Id) -> Option<String> {
    if id.role() != Role::Aide {
        return None;
    }
    Some(crate::queue::find(env, id).ok().and_then(|(_, d)| d.model).unwrap_or_else(|| "sonnet".into()))
}

fn model_args(env: &Env, id: &Id) -> Vec<String> {
    model_for(env, id).map(|m| vec!["--model".into(), m]).unwrap_or_default()
}

/// The models the account may use (`availableModels`): managed settings
/// when they say, else the user's `settings.json`. None when neither lists
/// any (no limit). Claude Code silently swaps a model that is not allowed
/// for the default, so an aide's model is checked against this first.
pub fn available_models(env: &Env) -> Option<Vec<String>> {
    let files: Vec<std::path::PathBuf> = MANAGED_SETTINGS.iter().map(std::path::PathBuf::from).chain([env.claude_home.join("settings.json")]).collect();
    models_in(&files)
}

/// Where managed settings live on this OS (the first that exists wins).
#[cfg(windows)]
pub const MANAGED_SETTINGS: [&str; 1] = [r"C:\Program Files\ClaudeCode\managed-settings.json"];
#[cfg(not(windows))]
pub const MANAGED_SETTINGS: [&str; 2] = ["/etc/claude-code/managed-settings.json", "/Library/Application Support/ClaudeCode/managed-settings.json"];

/// Does `allowed` (an `availableModels` list) let `model` run? An exact
/// match (any case) always; a family name (`sonnet`, `opus`, `haiku`: no
/// `-`) matches an entry that is that family or an id of it (`sonnet`,
/// `claude-sonnet-4-5`, `sonnet[1m]`), and a family entry an id of it.
pub fn model_allowed(model: &str, allowed: &[String]) -> bool {
    let model = model.to_lowercase();
    let of = |fam: &str, id: &str| id == fam || id.contains(&format!("-{fam}")) || id.starts_with(fam);
    allowed.iter().map(|a| a.to_lowercase()).any(|a| a == model || (!model.contains('-') && of(&model, &a)) || (!a.contains('-') && of(&a, &model)))
}

/// The first `availableModels` among `files`, in order.
fn models_in(files: &[std::path::PathBuf]) -> Option<Vec<String>> {
    files.iter().find_map(|p| {
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(p).ok()?).ok()?;
        v.get("availableModels")?.as_array().map(|a| a.iter().filter_map(|m| m.as_str().map(str::to_string)).collect())
    })
}

fn mode_args(mode: &str) -> Vec<String> {
    if mode.is_empty() {
        vec![]
    } else {
        vec!["--permission-mode".into(), mode.into()]
    }
}

/// The prompt a lead's cold launch carries (a `--bg` job with no prompt sits
/// "waiting for a prompt"; a copy of such a job fails): who it is, and what
/// to do first. The brief in its folder (CLAUDE.md) and the rules the hook
/// hands it say the rest.
pub fn lead_prompt(id: &Id) -> String {
    match id.role() {
        Role::Aide => aide_prompt(id),
        Role::Admiral => format!(
            "You are {}, the admiral of fleet {}. Read CLAUDE.md in this folder, run `nimitz status`, report the fleet TLDR-first, then wait for the user.",
            id.name(),
            id.fleet()
        ),
        _ => format!(
            "You are {}, the commodore of squadron {} in fleet {}. Read CLAUDE.md and SQUADRON.md in this folder, run `nimitz status`, report your squadron TLDR-first, then wait for the user.",
            id.name(),
            id.squadron().unwrap_or_default(),
            id.fleet()
        ),
    }
}

/// An aide's cold-launch prompt: who it is, and its loop in one line; the
/// SessionStart hook hands it the full rules.
pub fn aide_prompt(id: &Id) -> String {
    format!(
        "You are {}, an aide of {}. Work your inbox: `nimitz inbox claim {id}`, do the item, then `nimitz inbox done` or `nimitz inbox fail`, until claim exits 3. `nimitz rules {id}` has the full text.",
        id.name(),
        id.parent().map(|p| p.name()).unwrap_or_default()
    )
}

/// Cold launch of any member: `claude --bg --name N [--model X] --permission-mode M --settings S [prompt]`
/// (a ship's prompt comes from squadron.toml, a lead's from `lead_prompt`,
/// an aide's from `aide_prompt`; only an aide names a model).
pub fn launch_bg(env: &Env, id: &Id, mode: &str, prompt: Option<&str>) -> Vec<String> {
    let mut c = vec![env.claude_bin.clone(), "--bg".into(), "--name".into(), id.name()];
    c.extend(model_args(env, id));
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
    c.extend(model_args(env, id));
    c.extend(mode_args(mode));
    c.extend(["--settings".into(), settings_json(env, id, true)]);
    c
}

/// Resume a stopped background session keeping its id: native binary, no flags.
pub fn resume_bg(env: &Env, session_id: &str) -> Vec<String> {
    vec![env.native_claude_bin.clone(), "--bg".into(), "--resume".into(), session_id.into()]
}

pub fn stop(env: &Env, short_id: &str) -> Vec<String> {
    vec![env.native_claude_bin.clone(), "stop".into(), short_id.into()]
}

pub fn rm(env: &Env, short_id: &str) -> Vec<String> {
    vec![env.native_claude_bin.clone(), "rm".into(), short_id.into()]
}

/// `claude attach <job>`: what a lead's Herdr tab (and `nimitz open`) runs.
/// Detach is ctrl-z; `/exit` inside an attach opens the agent view.
pub fn attach(env: &Env, short_id: &str) -> Vec<String> {
    vec![env.native_claude_bin.clone(), "attach".into(), short_id.into()]
}

// ------------------------------------------------------------------ reading a launch's output

/// `text` with every ANSI escape sequence removed (FORCE_COLOR, a terminal
/// that is a tty): CSI `ESC [ ... <final>`, OSC `ESC ] ... BEL|ESC \`, and
/// two-byte `ESC x`.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('[') => {
                for d in it.by_ref() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(d) = it.next() {
                    if d == '\u{7}' || (d == '\u{1b}' && it.peek() == Some(&'\\')) {
                        if d == '\u{1b}' {
                            it.next();
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// A short job id as Claude Code prints it (8 hex digits), from the token
/// after `word`: `backgrounded · 9cdceb2d`, `started a copy as 9cdceb2d`.
fn short_after(text: &str, word: &str) -> Option<String> {
    let i = text.find(word)? + word.len();
    let tok = text[i..].trim_start().split(|c: char| !c.is_ascii_hexdigit()).next()?;
    (tok.len() >= 8).then(|| tok[..8].to_string())
}

/// The job Claude Code says it started a COPY as (a stderr note: "... so this
/// started a copy as X", "the flags you passed started a copy as X", "started
/// a copy of that conversation as X"; 2.1.292), if the output carries one.
/// Escape sequences are stripped first (FORCE_COLOR).
pub fn copy_started(text: &str) -> Option<String> {
    let text = plain(text);
    let i = text.find("started a copy")?;
    short_after(&text[i..], " as ")
}

/// The job a launch or wake names: every `--bg` launch and every native wake
/// prints `backgrounded · <short>[ · <name>]` on stdout (2.1.292; notes go to
/// stderr); the sandbox's wake of a job with no row prints `resumed <session>
/// as <short>`. None when the output names no job (an older wording: `woke
/// session <id> ...`). Escape sequences are stripped first (FORCE_COLOR).
pub fn job_started(text: &str) -> Option<String> {
    let text = plain(text);
    short_after(&text, "backgrounded \u{b7}").or_else(|| short_after(&text, " as "))
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
        // a lead launches the same way, with its own prompt (plan-v4 section 7)
        let adm = Id::admiral("F");
        let l = launch_bg(&env, &adm, "", Some(&lead_prompt(&adm)));
        assert_eq!(&l[..4], &["claude", "--bg", "--name", "[F]-ADMIRAL"]);
        assert!(l[5].contains("\"bgIsolation\":\"none\"") && l[5].contains("\"Bash(nimitz *)\""), "{}", l[5]);
        assert!(l.last().unwrap().starts_with("You are [F]-ADMIRAL, the admiral of fleet F."));
        assert!(lead_prompt(&Id::commodore("F", "SQ")).contains("SQUADRON.md"));
    }

    #[test]
    fn an_aide_launches_with_its_model_parent_label_and_permissions() {
        let (td, env) = crate::config::tests::test_env();
        crate::config::tests::make_fleet(&env);
        let repo = td.path().join("kb");
        std::fs::create_dir_all(&repo).unwrap();
        let dirs: BTreeMap<String, String> = [("test-mac".to_string(), repo.to_string_lossy().into_owned())].into();
        crate::config::tests::add_aide(&env, AideDef { name: "KB".into(), dirs: dirs.clone(), ..Default::default() });
        crate::config::tests::add_aide(&env, AideDef { name: "REF".into(), model: Some("haiku".into()), snapshot: true, dirs, ..Default::default() });
        let kb = Id("MAIN/@KB".into());
        // labels: the parent, no squadron
        let l = labels_for(&env, &kb);
        assert_eq!((l["NIMITZ_ROLE"].as_str(), l["NIMITZ_PARENT"].as_str(), l.get("NIMITZ_SQUADRON")), ("aide", "MAIN", None));
        assert!(!labels_for(&env, &Id::ship("MAIN", "ARGOS", "ARGOS")).contains_key("NIMITZ_PARENT"), "only an aide's");
        // --model: the aide's own, else sonnet; on a launch and a relaunch, never a ship's
        let c = launch_bg(&env, &kb, "acceptEdits", Some(&aide_prompt(&kb)));
        assert_eq!(&c[1..8], &["--bg", "--name", "[MAIN]-@KB", "--model", "sonnet", "--permission-mode", "acceptEdits"]);
        assert!(c.last().unwrap().starts_with("You are [MAIN]-@KB, an aide of [MAIN]-ADMIRAL. Work your inbox: `nimitz inbox claim MAIN/@KB`"), "{c:?}");
        let r = relaunch_bg(&env, &Id("MAIN/@REF".into()), "", "sid");
        assert_eq!(&r[4..8], &["--name", "[MAIN]-@REF", "--model", "haiku"]);
        assert!(!launch_bg(&env, &Id::ship("MAIN", "ARGOS", "ARGOS"), "", Some("go")).contains(&"--model".to_string()));
        assert_eq!(model_for(&env, &Id("MAIN/@NONE".into())).as_deref(), Some("sonnet"), "no config: still sonnet");
        // permissions: a writer may run git in its folder; a snapshot reader may not edit
        let perms = |id: &Id| serde_json::from_str::<serde_json::Value>(&settings_json(&env, id, true)).unwrap()["permissions"].clone();
        let real = repo.canonicalize().unwrap();
        let p = perms(&kb);
        let list = |k: &str| p[k].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect::<Vec<_>>();
        let (allow, deny) = (list("allow"), list("deny"));
        assert_eq!(allow[0], NIMITZ_ALLOW);
        for sub in ["status", "add", "commit", "log", "diff", "show", "rev-parse", "ls-files", "restore --staged"] {
            assert!(allow.contains(&format!("Bash(git -C {} {sub})", real.display())) && allow.contains(&format!("Bash(git -C {} {sub} *)", real.display())), "{sub}: {allow:?}");
        }
        assert_eq!(allow.len(), 19, "nothing else: {allow:?}");
        assert!(!allow.iter().any(|a| a.contains(" grep")), "no git grep (-O runs a program): {allow:?}");
        for o in ["--output", "-O", "--open-files-in-pager"] {
            assert!(deny.contains(&format!("Bash(git * {o}*)")), "{o}: {deny:?}");
        }
        assert!(!allow.iter().any(|a| a == "Bash(git *)" || a.contains("push")), "no git outside the list: {allow:?}");
        for sub in ["push", "remote", "fetch", "pull", "clone", "config", "submodule"] {
            for form in [format!("Bash(git {sub})"), format!("Bash(git {sub} *)"), format!("Bash(git -C * {sub})"), format!("Bash(git -C * {sub} *)")] {
                assert!(deny.contains(&form), "{form}: {deny:?}");
            }
        }
        let r = perms(&Id("MAIN/@REF".into()));
        assert_eq!(r["allow"], serde_json::json!([NIMITZ_ALLOW]));
        assert!(["Edit", "Write", "NotebookEdit", "Bash(git push *)"].iter().all(|d| r["deny"].as_array().unwrap().iter().any(|x| x == d)), "{r}");
        assert_eq!(perms(&Id::admiral("MAIN")), serde_json::json!({"allow": [NIMITZ_ALLOW]}));
    }

    #[test]
    fn a_model_is_allowed_by_its_name_or_its_family() {
        let l = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(model_allowed("sonnet", &l(&["opus", "sonnet"])));
        assert!(model_allowed("sonnet", &l(&["claude-sonnet-4-5"])) && model_allowed("sonnet", &l(&["sonnet[1m]"])) && model_allowed("Sonnet", &l(&["SONNET"])));
        assert!(model_allowed("claude-sonnet-4-5", &l(&["claude-sonnet-4-5"])), "an exact id");
        assert!(model_allowed("claude-sonnet-4-5", &l(&["sonnet"])), "a family entry allows its ids");
        assert!(model_allowed("haiku", &l(&["claude-haiku-4-5-20251001"])));
        assert!(!model_allowed("sonnet", &l(&["opus", "claude-opus-4-1"])));
        assert!(!model_allowed("claude-sonnet-4-5", &l(&["claude-sonnet-4-6"])), "another id of the family is not this one");
        assert!(!model_allowed("sonnet", &[]));
    }

    #[test]
    fn available_models_come_from_managed_settings_first() {
        let td = tempfile::tempdir().unwrap();
        let (managed, user, none) = (td.path().join("managed.json"), td.path().join("settings.json"), td.path().join("missing.json"));
        std::fs::write(&user, r#"{"availableModels": ["opus", "sonnet"]}"#).unwrap();
        assert_eq!(models_in(&[none.clone(), user.clone()]), Some(vec!["opus".to_string(), "sonnet".to_string()]));
        // managed settings that say nothing about models leave the user's
        std::fs::write(&managed, r#"{"permissions": {}}"#).unwrap();
        assert_eq!(models_in(&[managed.clone(), user.clone()]).map(|v| v.len()), Some(2));
        std::fs::write(&managed, r#"{"availableModels": ["haiku"]}"#).unwrap();
        assert_eq!(models_in(&[managed, user]), Some(vec!["haiku".to_string()]));
        assert_eq!(models_in(&[none]), None, "no list: no limit");
        let (_td, env) = crate::config::tests::test_env();
        std::fs::create_dir_all(&env.claude_home).unwrap();
        std::fs::write(env.claude_home.join("settings.json"), r#"{"availableModels": ["fable"]}"#).unwrap();
        if !std::path::Path::new("/etc/claude-code/managed-settings.json").exists() {
            assert_eq!(available_models(&env), Some(vec!["fable".to_string()]));
        }
    }

    #[test]
    fn a_launch_output_is_read_for_the_job_and_for_a_copy_note() {
        assert_eq!(copy_started("note: session 9cdceb2d-... is already running in the background, so this started a copy as dc74d940. `claude attach` opens the original."), Some("dc74d940".into()));
        assert_eq!(copy_started("the flags you passed started a copy as 19da599d"), Some("19da599d".into()));
        assert_eq!(copy_started("started a copy of that conversation as 55a9057c"), Some("55a9057c".into()));
        assert_eq!(copy_started("backgrounded \u{b7} 29a81431"), None);
        assert_eq!(job_started("backgrounded \u{b7} 29a81431"), Some("29a81431".into()));
        assert_eq!(job_started("resumed 9cdceb2d-0000-4000-8000-000000000001 as 9cdceb2d"), Some("9cdceb2d".into()));
        assert_eq!(job_started("woke session 942bf763-0000-4000-8000-000000000001 with its saved options"), None);
        assert_eq!(job_started(""), None);
        // the name after the id, and colour (FORCE_COLOR) around it, change nothing
        assert_eq!(job_started("\u{1b}[1mbackgrounded\u{1b}[0m \u{b7} \u{1b}[36m29a81431\u{1b}[0m \u{b7} [F.SQ]-S1"), Some("29a81431".into()));
        assert_eq!(copy_started("\u{1b}[33mnote:\u{1b}[0m session x is already running in the background, so this started a copy as \u{1b}[1mdc74d940\u{1b}[0m."), Some("dc74d940".into()));
        assert_eq!(plain("a\u{1b}]0;title\u{7}b\u{1b}[31mc\u{1b}[0m"), "abc");
    }
}
