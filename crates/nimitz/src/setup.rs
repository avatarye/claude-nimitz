//! `nimitz init` and `nimitz doctor [--fix]` (plan-v3 5.2, 8.1).
use crate::run::refusal;
use crate::{out, Ctx};
use nimitz_core::{Plan, Step};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The events the binary needs the plugin to register (plugin/hooks/hooks.json).
pub const HOOK_EVENTS: [&str; 9] = [
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "Stop",
    "Notification",
    "SubagentStart",
    "SubagentStop",
    "PreToolUse",
    "PermissionRequest",
];

/// The PreToolUse matchers the binary needs: the signal rule and the
/// needs-you notice. A per-turn rules update on PreToolUse
/// (`hook::UPDATE_EVENTS`) needs an entry with no matcher as well.
const PRE_TOOL_MATCHERS: [&str; 2] = ["SendMessage", "AskUserQuestion"];

/// Every (event, matcher) the plugin must register; `None` is "any entry".
fn needed_hooks() -> Vec<(&'static str, Option<&'static str>)> {
    let mut v: Vec<(&str, Option<&str>)> = HOOK_EVENTS.iter().map(|e| (*e, None)).collect();
    v.extend(PRE_TOOL_MATCHERS.iter().map(|m| ("PreToolUse", Some(*m))));
    if nimitz_core::hook::UPDATE_EVENTS.contains(&"PreToolUse") {
        v.push(("PreToolUse", Some("")));
    }
    v
}

/// The (event, matcher) pairs a hooks.json (the plugin's, or a settings
/// file's `hooks`) registers with a `nimitz hook <Event>` command. A missing
/// matcher reads as "" (every tool).
fn registered_hooks(hooks: &Value) -> Vec<(String, String)> {
    let mut out = vec![];
    let Some(events) = hooks.as_object() else { return out };
    for (event, entries) in events {
        for entry in entries.as_array().into_iter().flatten() {
            let calls_nimitz = entry["hooks"].as_array().into_iter().flatten().any(|h| {
                let cmd = h["command"].as_str().unwrap_or("");
                let args: Vec<&str> = h["args"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                cmd.contains("nimitz") && (args.contains(&"hook") || cmd.contains(" hook "))
            });
            if calls_nimitz {
                out.push((event.clone(), entry["matcher"].as_str().unwrap_or("").to_string()));
            }
        }
    }
    out
}

/// What `needed_hooks` lacks in `registered`, named for the user.
fn missing_hooks(registered: &[(String, String)]) -> Vec<String> {
    needed_hooks()
        .into_iter()
        .filter(|(e, m)| !registered.iter().any(|(re, rm)| re == e && m.map(|m| m == rm || rm.is_empty() || rm == "*").unwrap_or(true)))
        .map(|(e, m)| match m {
            Some("") => format!("{e} (every tool)"),
            Some(m) => format!("{e} (matcher {m})"),
            None => e.to_string(),
        })
        .collect()
}

/// The installed nimitz plugin's hooks.json: `NIMITZ_PLUGIN_HOOKS` (tests),
/// else the `installPath` of the `nimitz@...` entry in Claude Code's
/// `plugins/installed_plugins.json`.
fn plugin_hooks_path(ctx: &Ctx) -> Option<PathBuf> {
    if let Some(p) = ctx.vars.get("NIMITZ_PLUGIN_HOOKS").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let installed = read_json(&ctx.env.claude_home.join("plugins/installed_plugins.json"))?;
    let plugins = installed.get("plugins")?.as_object()?;
    let (_, entries) = plugins.iter().find(|(k, _)| k.starts_with("nimitz@"))?;
    let path = entries.as_array()?.iter().find_map(|e| e["installPath"].as_str())?;
    Some(PathBuf::from(path).join("hooks/hooks.json"))
}

/// Fields nimitz reads in Claude Code's files (the pinned formats).
const SESSION_FIELDS: [&str; 4] = ["pid", "sessionId", "status", "kind"];
const JOB_FIELDS: [&str; 4] = ["sessionId", "state", "name", "cwd"];

fn settings_path(ctx: &Ctx) -> PathBuf {
    ctx.env.claude_home.join("settings.json")
}

fn read_json(p: &Path) -> Option<Value> {
    std::fs::read(p).ok().and_then(|b| nimitz_core::evidence::first_json(&b))
}

fn trusted(ctx: &Ctx, dir: &Path) -> bool {
    nimitz_core::evidence::folder_trusted(&ctx.env, &mut Default::default(), &dir.to_string_lossy())
}

// ------------------------------------------------------------------ init

pub fn init(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--label"])?;
    ctx.args.at_most(0)?;
    let label = ctx.args.value("--label").map(str::to_string).unwrap_or_else(|| crate::crud::machine_label(ctx));
    let mut plan = Plan { summary: format!("init nimitz on this machine as {label}"), ..Default::default() };
    if ctx.caller != nimitz_core::Caller::User {
        plan.refusals.push(refusal("authority", "`nimitz init` is yours alone".into(), None, None));
    }
    if !nimitz_core::config::valid_label(&label) {
        plan.refusals.push(refusal("config.invalid", format!("machine label {label:?} is not [a-z0-9._-]+"), Some("nimitz init --label <label>".into()), None));
    }
    // registry::load fills in a label when there is no file yet; only a saved one counts.
    let saved = ctx.env.registry_path().is_file();
    let reg = nimitz_core::registry::load(&ctx.env);
    if saved && !reg.machine.is_empty() && reg.machine != label {
        plan.refusals.push(refusal(
            "config.invalid",
            format!("this machine is already labelled {:?}", reg.machine),
            Some(format!("nimitz init --label {}", reg.machine)),
            None,
        ));
    }
    let mk = |p: &Path, why: &str| Step { id: None, cmd: vec!["mkdir".into(), p.to_string_lossy().into_owned()], cwd: None, why: why.into() };
    if !ctx.env.home.is_dir() {
        plan.steps.push(mk(&ctx.env.home, "the data root (fleets live here)"));
    }
    if !ctx.env.state.is_dir() {
        plan.steps.push(mk(&ctx.env.state, "per-machine registry and events (never synced)"));
    }
    if !saved || reg.machine != label {
        plan.steps.push(Step {
            id: None,
            cmd: vec!["label".into(), label.clone()],
            cwd: None,
            why: format!("record this machine's label in {}", ctx.env.registry_path().display()),
        });
    }
    let code = match ctx.gate(&plan) {
        out::Go::Exit(0) if plan.steps.is_empty() => 0,
        out::Go::Exit(c) => return Ok(c),
        out::Go::Run => {
            for s in &plan.steps {
                let r = match s.cmd[0].as_str() {
                    "mkdir" => std::fs::create_dir_all(&s.cmd[1]).map_err(|e| e.to_string()),
                    _ => nimitz_core::registry::update(&ctx.env, |r| {
                        r.version = 1;
                        r.machine = label.clone();
                    })
                    .map_err(|e| e.to_string()),
                };
                match r {
                    Ok(()) => out::ok(&s.cmd.join(" ")),
                    Err(e) => {
                        out::error(&format!("{}: {e}", s.cmd.join(" ")));
                        return Ok(1);
                    }
                }
            }
            0
        }
    };
    if !trusted(ctx, &ctx.env.home) {
        out::warn(&format!(
            "Claude Code does not trust {} yet: run `claude` there once, accept the trust dialog, exit, then `nimitz doctor`",
            ctx.env.home.display()
        ));
    }
    let d = doctor_report(ctx, false);
    Ok(code.max(d))
}

// ------------------------------------------------------------------ doctor

struct Report {
    errors: usize,
    fixes: Plan,
}

impl Report {
    fn ok(&mut self, s: &str) {
        out::ok(s);
    }
    fn warn(&mut self, s: &str, fix: Option<&str>) {
        out::warn(s);
        if let Some(f) = fix {
            out::line(&format!("       fix: {f}"));
        }
    }
    fn err(&mut self, s: &str, fix: Option<&str>) {
        self.errors += 1;
        out::error(s);
        if let Some(f) = fix {
            out::line(&format!("       fix: {f}"));
        }
    }
}

/// `nimitz rules [<id>]`: the rules the SessionStart hook hands `id` (a member
/// caller: its own when no id is given). Read-only.
pub fn rules(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    ctx.args.at_most(1)?;
    let id = match (ctx.args.pos.first(), &ctx.caller) {
        (Some(x), _) => crate::members::parse_id(x)?,
        (None, nimitz_core::Caller::Member { id, .. }) => id.clone(),
        (None, nimitz_core::Caller::User) => return Err("missing id (F, F/SQ or F/SQ/SHIP)".into()),
    };
    print!("{}", nimitz_core::rules::text(&id));
    Ok(0)
}

pub fn doctor(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--fix"])?;
    ctx.args.at_most(0)?;
    // The report is every member's; the fixes (the user's settings.json, the
    // briefs) are yours alone.
    if ctx.args.flag("--fix") && ctx.caller != nimitz_core::Caller::User {
        let plan = Plan {
            summary: "doctor fixes".into(),
            refusals: vec![refusal("authority", "`nimitz doctor --fix` is yours alone; a member runs `nimitz doctor` (the report) only".into(), None, None)],
            ..Default::default()
        };
        return Ok(match ctx.gate(&plan) {
            out::Go::Exit(c) => c.max(1),
            out::Go::Run => 1,
        });
    }
    Ok(doctor_report(ctx, ctx.args.flag("--fix")))
}

fn doctor_report(ctx: &Ctx, fix: bool) -> i32 {
    let mut r = Report { errors: 0, fixes: Plan { summary: "doctor fixes".into(), ..Default::default() } };
    let env = &ctx.env;

    // Data root, label, trust.
    if env.home.is_dir() {
        r.ok(&format!("data root {}", env.home.display()));
    } else {
        r.err(&format!("no data root at {}", env.home.display()), Some("nimitz init"));
    }
    let reg = nimitz_core::registry::load(env);
    if reg.machine.is_empty() || !env.registry_path().is_file() {
        r.err("this machine has no label", Some("nimitz init --label <label>"));
    } else {
        r.ok(&format!("machine label {}", reg.machine));
    }
    if env.home.is_dir() {
        if trusted(ctx, &env.home) {
            r.ok(&format!("Claude Code trusts {}", env.home.display()));
        } else {
            r.err(&format!("Claude Code does not trust {}", env.home.display()), Some(&format!("cd {} && claude   # accept the trust dialog, then exit", out::quote(&env.home.to_string_lossy()))));
        }
    }

    // The native binary and the supported range.
    let range = nimitz_core::compat::supported();
    let native_v = nimitz_core::compat::installed(env);
    match &native_v {
        None => r.err(&format!("native claude {} does not answer --version", env.native_claude_bin), Some("set NIMITZ_NATIVE_CLAUDE_BIN")),
        Some(v) => match nimitz_core::compat::position(&range, v) {
            0 => r.ok(&format!("native claude {} is {v} (supported {} - {})", resolved(&env.native_claude_bin), range.min, range.max)),
            1 => r.warn(
                &format!("Claude Code {v} is newer than nimitz is tested with (tested up to {}); launches go ahead", range.max),
                Some("nothing to do; report anything odd, and the tested range widens with the next nimitz"),
            ),
            _ => r.err(&format!("Claude Code {v} is older than the supported floor {}", range.min), Some("update Claude Code")),
        },
    }
    // The launch binary (cold launches, interactive members): NIMITZ_CLAUDE_BIN or `claude` on PATH.
    let launch = resolved(&env.claude_bin);
    let launch_v = std::process::Command::new(&env.claude_bin)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| nimitz_core::compat::parse_version(&String::from_utf8_lossy(&o.stdout)));
    let from = if ctx.vars.contains_key("NIMITZ_CLAUDE_BIN") { "NIMITZ_CLAUDE_BIN" } else { "first `claude` on PATH" };
    match &launch_v {
        None => r.err(&format!("launch claude {launch} ({from}) does not answer --version"), Some("set NIMITZ_CLAUDE_BIN")),
        Some(v) => {
            r.ok(&format!("launch claude {launch} ({from}) is {v}"));
            if is_shim(&env.claude_bin) || is_shim(&launch) {
                r.warn(&format!("launch claude {launch} is a version-manager shim; its version can change with the folder"), Some("set NIMITZ_CLAUDE_BIN to the real binary"));
            }
            if let Some(n) = &native_v {
                if !same_version(v, n) {
                    r.warn(&format!("launch claude is {v} but the native claude is {n}: launches and resumes run different builds"), Some("set NIMITZ_CLAUDE_BIN / NIMITZ_NATIVE_CLAUDE_BIN to one install"));
                }
            }
        }
    }

    // Hooks: the plugin, or nimitz hook commands in the user settings.
    let settings = read_json(&settings_path(ctx)).unwrap_or(json!({}));
    let plugin_on = settings
        .get("enabledPlugins")
        .and_then(Value::as_object)
        .map(|m| m.iter().any(|(k, v)| k.starts_with("nimitz@") && v.as_bool() == Some(true)))
        .unwrap_or(false);
    let in_settings = settings.get("hooks").map(registered_hooks).unwrap_or_default();
    let missing = missing_hooks(&in_settings);
    if plugin_on {
        // The installed copy of the plugin must register every event the
        // binary needs; a plugin updated on disk is read again by /reload-plugins.
        match plugin_hooks_path(ctx) {
            Some(p) => match read_json(&p) {
                Some(v) => {
                    let lacks = missing_hooks(&registered_hooks(v.get("hooks").unwrap_or(&Value::Null)));
                    if lacks.is_empty() {
                        r.ok(&format!("hooks: the nimitz plugin is enabled and {} registers every event nimitz needs", p.display()));
                    } else {
                        r.err(
                            &format!("hooks: the installed nimitz plugin ({}) does not register {}", p.display(), lacks.join(", ")),
                            Some("/reload-plugins in each live session (a new session reads the plugin again), then nimitz doctor"),
                        );
                    }
                }
                None => r.err(&format!("hooks: the nimitz plugin is enabled but {} is missing or not JSON", p.display()), Some(&plugin_install_fix())),
            },
            None => r.warn("hooks: the nimitz plugin is enabled; its installed copy is not listed in plugins/installed_plugins.json, so its hooks were not checked", Some(&plugin_install_fix())),
        }
    } else if missing.is_empty() {
        r.ok("hooks: nimitz hook commands are in the user settings");
    } else {
        r.err(
            &format!("hooks: the nimitz plugin is not enabled in {}", settings_path(ctx).display()),
            Some(&plugin_install_fix()),
        );
    }
    if nimitz_core::env::which("nimitz").is_none() {
        r.warn("`nimitz` is not on PATH; the plugin's hooks call it by name", Some("put the nimitz binary on PATH"));
    }

    // crossSessionInbound: ships and commodores must take each other's signals.
    match settings.get("crossSessionInbound").and_then(Value::as_str) {
        Some("accept") => r.ok("crossSessionInbound is accept"),
        other => {
            let msg = format!("crossSessionInbound is {}; signals between sessions are not delivered", other.unwrap_or("unset"));
            r.err(&msg, Some("nimitz doctor --fix"));
            let mut s = settings.clone();
            if !s.is_object() {
                s = json!({});
            }
            s["crossSessionInbound"] = json!("accept");
            r.fixes.steps.push(Step {
                id: None,
                cmd: vec!["write".into(), settings_path(ctx).to_string_lossy().into_owned(), serde_json::to_string_pretty(&s).unwrap_or_default()],
                cwd: None,
                why: "set crossSessionInbound to accept in the user settings".into(),
            });
        }
    }

    // Role briefs (CLAUDE.md of admirals and commodores): an older generated
    // one is replaced; a hand-edited one is left alone (the rules reach the
    // session at its start either way).
    let mut current = 0;
    for (id, p, state) in nimitz_core::rules::briefs(env) {
        use nimitz_core::rules::BriefState;
        let write = |r: &mut Report, why: &str| {
            r.fixes.steps.push(Step {
                id: Some(id.clone()),
                cmd: vec!["write".into(), p.to_string_lossy().into_owned(), nimitz_core::rules::brief(&id)],
                cwd: None,
                why: format!("{why} {}", p.display()),
            })
        };
        match state {
            BriefState::Current => current += 1,
            BriefState::Old => {
                r.err(&format!("{id}: {} was written by an older nimitz", p.display()), Some("nimitz doctor --fix"));
                write(&mut r, "write the current brief to");
            }
            BriefState::Missing => {
                r.err(&format!("{id}: no brief at {}", p.display()), Some("nimitz doctor --fix"));
                write(&mut r, "write the brief");
            }
            BriefState::Custom => r.warn(
                &format!("{id}: {} is edited by hand; nimitz leaves it alone (the role's rules still arrive at session start)", p.display()),
                Some(&format!("compare it with `nimitz rules {id}`")),
            ),
        }
    }
    if current > 0 {
        r.ok(&format!("{current} role brief(s) current (rules v{})", nimitz_core::rules::VERSION));
    }

    // Pinned formats: only the fields nimitz reads.
    let (mut seen, mut bad) = (0, vec![]);
    let sessions = env.claude_home.join("sessions");
    for e in std::fs::read_dir(&sessions).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().map(|x| x != "json").unwrap_or(true) {
            continue;
        }
        seen += 1;
        lacks(&p, &SESSION_FIELDS, &mut bad);
    }
    for e in std::fs::read_dir(env.claude_home.join("jobs")).into_iter().flatten().flatten() {
        let p = e.path().join("state.json");
        if p.is_file() {
            seen += 1;
            lacks(&p, &JOB_FIELDS, &mut bad);
        }
    }
    if bad.is_empty() {
        r.ok(&format!("pinned formats: {seen} session/job file(s) carry every field nimitz reads"));
    } else {
        for b in bad.iter().take(5) {
            r.err(&format!("pinned formats: {b}"), None);
        }
    }

    if fix && !r.fixes.steps.is_empty() {
        let fixes = r.fixes.clone();
        match ctx.gate(&fixes) {
            out::Go::Exit(c) => return c.max(1),
            out::Go::Run => {
                for s in &fixes.steps {
                    let path = Path::new(&s.cmd[1]);
                    let tmp = path.with_extension("json.nimitz-tmp");
                    let done = path.parent().map(std::fs::create_dir_all).unwrap_or(Ok(())).and_then(|_| std::fs::write(&tmp, &s.cmd[2])).and_then(|_| std::fs::rename(&tmp, path));
                    match done {
                        Ok(()) => {
                            out::ok(&format!("fixed: {}", s.why));
                            r.errors -= 1;
                        }
                        Err(e) => out::error(&format!("fix failed: {e}")),
                    }
                }
            }
        }
    }
    if r.errors == 0 {
        out::ok("doctor: ready");
        0
    } else {
        out::error(&format!("doctor: {} problem(s)", r.errors));
        1
    }
}

/// The marketplace to add: this build's repo when it is still on disk, else GitHub.
pub fn plugin_install_fix() -> String {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = match repo.canonicalize() {
        Ok(r) if r.join(".claude-plugin/marketplace.json").is_file() => out::quote(&r.to_string_lossy()),
        _ => "avatarye/claude-nimitz".to_string(),
    };
    format!("claude plugin marketplace add {source} && claude plugin install nimitz@claude-nimitz")
}

/// Where a binary is: its path, and `-> <target>` when that is a symlink.
fn resolved(bin: &str) -> String {
    let Some(p) = (if bin.contains('/') { Some(PathBuf::from(bin)) } else { nimitz_core::env::which(bin).map(PathBuf::from) }) else {
        return bin.to_string();
    };
    match p.canonicalize() {
        Ok(real) if real != p => format!("{} -> {}", p.display(), real.display()),
        _ => p.to_string_lossy().into_owned(),
    }
}

/// A version-manager shim (mise, asdf, ...) rather than a real install.
fn is_shim(path: &str) -> bool {
    path.contains("/shims/") || path.contains("/mise/") || path.contains("/.asdf/")
}

/// `2.1.288.1000` (ACME) and `2.1.288` name one build.
fn same_version(a: &str, b: &str) -> bool {
    a == b || a.starts_with(&format!("{b}.")) || b.starts_with(&format!("{a}."))
}

fn lacks(p: &Path, fields: &[&str], bad: &mut Vec<String>) {
    match read_json(p) {
        None => bad.push(format!("{} is not JSON", p.display())),
        Some(v) => {
            let miss: Vec<&str> = fields.iter().copied().filter(|f| v.get(*f).is_none()).collect();
            if !miss.is_empty() {
                bad.push(format!("{} lacks {}", p.display(), miss.join(", ")));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_shims_and_the_plugin_source() {
        assert!(same_version("2.1.288.1000", "2.1.288"));
        assert!(same_version("2.1.288", "2.1.288"));
        assert!(!same_version("2.1.289", "2.1.288"));
        assert!(!same_version("2.1.2881", "2.1.288"));
        assert!(is_shim("/Users/x/.local/share/mise/shims/claude"));
        assert!(is_shim("/Users/x/.asdf/shims/claude"));
        assert!(!is_shim("/Users/x/.localtools/bin/claude"));
        let fix = plugin_install_fix();
        assert!(fix.starts_with("claude plugin marketplace add ") && fix.ends_with("claude plugin install nimitz@claude-nimitz"), "{fix}");
        assert!(!fix.contains('<'), "no placeholder: {fix}");
    }

    /// The plugin in this repo registers every event the binary needs; a
    /// copy that lacks one (an older install) is named, matcher and all.
    #[test]
    fn the_repo_plugin_registers_every_needed_hook() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin/hooks/hooks.json");
        let v = read_json(&p).expect("plugin/hooks/hooks.json");
        let reg = registered_hooks(&v["hooks"]);
        assert!(missing_hooks(&reg).is_empty(), "{:?}", missing_hooks(&reg));
        assert!(reg.contains(&("PreToolUse".to_string(), "SendMessage".to_string())), "{reg:?}");
        // drop UserPromptSubmit and the AskUserQuestion matcher: both named
        let mut old = v["hooks"].clone();
        old.as_object_mut().unwrap().remove("UserPromptSubmit");
        old["PreToolUse"].as_array_mut().unwrap().retain(|e| e["matcher"] != "AskUserQuestion");
        let lacks = missing_hooks(&registered_hooks(&old));
        assert_eq!(lacks, vec!["UserPromptSubmit".to_string(), "PreToolUse (matcher AskUserQuestion)".to_string()]);
        // an entry with no matcher covers every tool; a command that is not nimitz counts for nothing
        let any = serde_json::json!({"PreToolUse": [{"hooks": [{"type": "command", "command": "nimitz", "args": ["hook", "PreToolUse"]}]}],
            "Stop": [{"hooks": [{"type": "command", "command": "other-tool", "args": ["hook", "Stop"]}]}]});
        let reg = registered_hooks(&any);
        assert_eq!(reg, vec![("PreToolUse".to_string(), String::new())]);
        let lacks = missing_hooks(&reg);
        assert!(lacks.contains(&"Stop".to_string()) && !lacks.iter().any(|l| l.starts_with("PreToolUse")), "{lacks:?}");
    }
}
