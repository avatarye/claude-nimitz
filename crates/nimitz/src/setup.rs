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
    // the KB agents' guard, on every tool (`nimitz hook kb-guard`)
    v.push((KB_GUARD, Some("")));
    // the fleet's harness rules, an entry of their own (`nimitz hook rules harness`)
    v.extend(HARNESS_RULES.iter().map(|e| (*e, None)));
    v
}

/// How `registered_hooks` names the entries that run `nimitz hook rules`
/// (the fleet's harness rules, at a session start and at each prompt).
const HARNESS_RULES: [&str; 2] = ["SessionStart:rules", "UserPromptSubmit:rules"];

/// How `registered_hooks` names the PreToolUse entry that runs `nimitz hook kb-guard`.
const KB_GUARD: &str = "PreToolUse:kb-guard";

/// The (event, matcher) pairs a hooks.json (the plugin's, or a settings
/// file's `hooks`) registers with a `nimitz hook <Event>` command. A missing
/// matcher reads as "" (every tool). The PreToolUse entry that runs `nimitz
/// hook kb-guard` is named `KB_GUARD` (it is not the PreToolUse handler).
fn registered_hooks(hooks: &Value) -> Vec<(String, String)> {
    let mut out = vec![];
    let Some(events) = hooks.as_object() else { return out };
    for (event, entries) in events {
        for entry in entries.as_array().into_iter().flatten() {
            for h in entry["hooks"].as_array().into_iter().flatten() {
                let cmd = h["command"].as_str().unwrap_or("");
                let args: Vec<&str> = h["args"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                if !cmd.contains("nimitz") || !(args.contains(&"hook") || cmd.contains(" hook ")) {
                    continue;
                }
                let guard = args.contains(&"kb-guard") || cmd.contains(" hook kb-guard");
                let harness = args.get(1) == Some(&"rules") || cmd.contains(" hook rules");
                let name = if guard && event == "PreToolUse" {
                    KB_GUARD.to_string()
                } else if harness {
                    format!("{event}:rules")
                } else {
                    event.clone()
                };
                out.push((name, entry["matcher"].as_str().unwrap_or("").to_string()));
            }
        }
    }
    out.dedup();
    out
}

/// The events whose `nimitz hook` entry is one command string (`"command":
/// "nimitz hook X"`, no `args`). Claude Code runs such a hook under `sh -c`,
/// so the hook's parent is a shell: the pid-chain fallback usually finds the
/// Claude Code process, but the plugin's own form is `command` + `args`
/// (r3-e2e.md finding 7).
fn string_form_hooks(hooks: &Value) -> Vec<String> {
    let mut out = vec![];
    let Some(events) = hooks.as_object() else { return out };
    for (event, entries) in events {
        let stringy = entries.as_array().into_iter().flatten().any(|entry| {
            entry["hooks"].as_array().into_iter().flatten().any(|h| {
                let cmd = h["command"].as_str().unwrap_or("");
                h.get("args").is_none() && cmd.contains("nimitz") && cmd.contains(" hook ")
            })
        });
        if stringy {
            out.push(event.clone());
        }
    }
    out.sort(); // named in one order, whatever order the file has
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
/// (`name` is not pinned: Claude Code writes none for a copy of a running
/// job, and nimitz reads a missing one as "", acceptance F5)
const JOB_FIELDS: [&str; 3] = ["sessionId", "state", "cwd"];

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
    if !nimitz_core::config::valid_label(&label) {
        plan.refusals.push(refusal("config.invalid", format!("machine label {label:?} is not {}", nimitz_core::config::LABEL_RULE), Some("nimitz init --label <label>".into()), None));
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
    // the data root holds every lead's folder: Claude Code must trust it
    if !trusted(ctx, &ctx.env.home) {
        match nimitz_core::trust::too_broad(&ctx.env, &ctx.env.home) {
            None => plan.steps.push(nimitz_core::trust::step(&ctx.env, None, &ctx.env.home)),
            Some(why) => plan.warnings.push(nimitz_core::Issue { code: "trust.broad".into(), severity: nimitz_core::Severity::Warn, message: why, fix: None, id: None }),
        }
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
                    "trust" => nimitz_core::trust::write(&ctx.env, Path::new(&s.cmd[1])).map(|_| ()),
                    _ => nimitz_core::registry::update(&ctx.env, |r| {
                        r.version = nimitz_core::REGISTRY_VERSION;
                        r.min_nimitz = Some(nimitz_core::REGISTRY_MIN_NIMITZ.into());
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
    let d = doctor_report(ctx, false);
    Ok(code.max(d))
}

// ------------------------------------------------------------------ trust

/// `nimitz trust <folder>`: Claude Code's trust for the folder's git root
/// (or the folder outside git), written as Claude Code writes it: it lets
/// the folder's own .claude settings, hooks and MCP servers run.
pub fn trust(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    let dir = ctx.args.need(0, "folder")?;
    ctx.args.at_most(1)?;
    let p = Path::new(dir);
    let mut plan = Plan { summary: format!("trust {}", nimitz_core::trust::shown(&ctx.env, p).display()), ..Default::default() };
    if let Some(why) = nimitz_core::trust::too_broad(&ctx.env, p) {
        plan.refusals.push(refusal("trust.broad", why, None, None));
    } else if !p.is_dir() {
        plan.refusals.push(refusal("folder.missing", format!("{dir} is not a folder"), Some(format!("mkdir -p {}", out::quote(dir))), None));
    } else if !nimitz_core::trust::is_written(&ctx.env, p) {
        plan.steps.push(nimitz_core::trust::step(&ctx.env, None, p));
    }
    Ok(match ctx.gate(&plan) {
        out::Go::Exit(c) => c,
        out::Go::Run => out::outcome(&plan, &nimitz_core::config::apply_config_plan(&ctx.env, &plan)),
    })
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
    fn info(&mut self, s: &str, fix: Option<&str>) {
        out::line(&format!("[INFO] {s}"));
        if let Some(f) = fix {
            out::line(&format!("       fix: {f}"));
        }
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

/// `nimitz rules [<path>...]`: the rules each member the paths cover is
/// given at a session start (a member caller: its own when no path is
/// given): its role's, then its fleet's harness rules, which come from their
/// own hook entry. Several members: each under a `== <id> ==` line. Read-only.
pub fn rules(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&[])?;
    let ids = match (ctx.args.pos.is_empty(), nimitz_core::sessions::verified(&ctx.caller)) {
        (false, _) => {
            let paths = crate::members::parse_paths(&ctx.args.pos)?;
            match nimitz_core::paths::resolve(&ctx.snap(), &paths) {
                Ok(ids) => ids,
                Err(i) => {
                    out::issue(&i);
                    return Ok(1);
                }
            }
        }
        (true, Some(id)) => vec![id.clone()],
        (true, None) => return Err(format!("missing path ({})", nimitz_core::paths::FORMS)),
    };
    for (n, id) in ids.iter().enumerate() {
        if ids.len() > 1 {
            println!("{}== {id} ==", if n > 0 { "\n" } else { "" });
        }
        print!("{}", nimitz_core::rules::text_for(&ctx.env, id));
    }
    Ok(0)
}

pub fn doctor(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--fix"])?;
    ctx.args.at_most(0)?;
    Ok(doctor_report(ctx, ctx.args.flag("--fix")))
}

/// Herdr's config: `HERDR_CONFIG_PATH`, else `~/.config/herdr/config.toml`
/// (Windows: `%APPDATA%\herdr\config.toml`) (read-only). Checked when the
/// file is there or nimitz runs with Herdr.
pub fn herdr_config_path(ctx: &Ctx) -> PathBuf {
    let var = |k: &str| ctx.vars.get(k).filter(|p| !p.is_empty()).cloned();
    match var("HERDR_CONFIG_PATH") {
        Some(p) => PathBuf::from(p),
        None if cfg!(windows) && var("APPDATA").is_some() => PathBuf::from(var("APPDATA").unwrap_or_default()).join("herdr").join("config.toml"),
        None => PathBuf::from(var("HOME").unwrap_or_default()).join(".config").join("herdr").join("config.toml"),
    }
}

/// Is `[session] resume_agents_on_restore = false` set in the Herdr config
/// `text`? `None` when the file does not parse.
pub fn herdr_resume_off(text: &str) -> Option<bool> {
    let v: toml::Value = toml::from_str(text).ok()?;
    Some(v.get("session").and_then(|s| s.get("resume_agents_on_restore")).and_then(toml::Value::as_bool) == Some(false))
}

fn herdr_resume_check(ctx: &Ctx, r: &mut Report) {
    let path = herdr_config_path(ctx);
    let with_herdr = ctx.vars.get("HERDR_ENV").map(String::as_str) == Some("1") || ctx.vars.get("NIMITZ_MUX").map(String::as_str) == Some("herdr");
    if !path.is_file() && !with_herdr {
        return;
    }
    let fix = format!("set resume_agents_on_restore = false under [session] in {}  # nimitz never edits it", path.display());
    let effect = "after a Herdr restart Herdr runs `claude --resume` in every agent pane, which holds a stopped or parked member's conversation interactively; `nimitz start` then cannot wake it until you /exit there (tab.resumed)";
    match std::fs::read_to_string(&path).ok().map(|t| herdr_resume_off(&t)) {
        Some(Some(true)) => r.ok(&format!("Herdr {}: resume_agents_on_restore is off", path.display())),
        Some(Some(false)) => r.warn(&format!("Herdr {}: resume_agents_on_restore is on (the default): {effect}", path.display()), Some(&fix)),
        Some(None) => r.warn(&format!("Herdr {} does not parse; resume_agents_on_restore may be on: {effect}", path.display()), Some(&fix)),
        None => r.warn(&format!("no Herdr config at {}, so resume_agents_on_restore is on (the default): {effect}", path.display()), Some(&fix)),
    }
}

/// Running nimitz and Admiralty processes older than the binaries installed
/// now (issue #1): an Admiralty started before `cargo install` replaced it
/// keeps the old nimitz-core in memory, and an old core reads a newer
/// registry as corrupt, keeps it aside and rebuilds it in its old shape.
/// The installed binaries are this nimitz and the `admiralty` beside it.
fn stale_processes(r: &mut Report) {
    if !nimitz_core::os::SCANS_PROCESSES {
        // macOS and Windows would need a process walk nimitz has nowhere else
        r.info("older-process check: not done on this OS (quit every Admiralty and long-running nimitz started before an upgrade)", None);
        return;
    }
    let stale: Vec<String> = nimitz_core::os::stale_processes().iter().map(|s| format!("{} pid {} ({})", s.name, s.pid, s.why)).collect();
    if !stale.is_empty() {
        r.warn(
            &format!("{} process(es) run an older nimitz than the one installed: {}; an older nimitz reads a newer registry as corrupt, keeps it aside and rebuilds it in its old shape", stale.len(), stale.join("; ")),
            Some("quit them (q in Admiralty, or kill <pid>), then start them again"),
        );
    }
}

/// The registry files set aside as corrupt (issue #1). One that reads fine
/// was set aside by an older nimitz: `--fix` fills what the registry lacks
/// from it (`registry::restore_from_kept`) and archives it. One that does
/// not parse stays an error, with its own fix.
fn kept_registries(env: &nimitz_core::env::Env, r: &mut Report) {
    let plan = match nimitz_core::registry::restore_from_kept(env, true) {
        Ok(p) => p,
        Err(e) => {
            r.err(&format!("registry: the kept files could not be read: {e}"), None);
            return;
        }
    };
    for k in &plan.newer {
        r.warn(&format!("registry: {} was written by a newer nimitz and set aside by an older one; this nimitz never reads it", k.display()), Some("keep it; install that newer nimitz to read it"));
    }
    for k in &plan.left {
        r.err(&format!("registry: {} did not parse when it was set aside, and still does not; starts are blocked until it is removed", k.display()), Some(&format!("check the members with `nimitz status`, then rm {}*", k.display())));
    }
    if plan.archived.is_empty() {
        return;
    }
    let names: Vec<String> = plan.archived.iter().map(|k| k.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect();
    // an older nimitz still running sets the restored file aside again: it goes first
    let stale = nimitz_core::os::stale_processes();
    if !stale.is_empty() {
        r.err(
            &format!(
                "registry: {} kept file(s) set aside by an older nimitz read fine ({}), but {} still runs: a restore now would be set aside again",
                names.len(),
                names.join(", "),
                stale.iter().map(nimitz_core::os::Stale::short).collect::<Vec<_>>().join(", ")
            ),
            Some("quit them, then nimitz doctor --fix"),
        );
        return;
    }
    let fields = if plan.filled.is_empty() { "nothing is missing from the registry".to_string() } else { format!("{} missing field(s) can be filled from them: {}", plan.filled.len(), plan.filled.join(", ")) };
    r.err(
        &format!("registry: {} kept file(s) set aside by an older nimitz read fine ({}); {fields}; starts are blocked until they are archived", names.len(), names.join(", ")),
        Some("nimitz doctor --fix"),
    );
    r.fixes.steps.push(Step {
        id: None,
        cmd: vec!["registry-restore".into()],
        cwd: None,
        why: format!("fill what the registry lacks from {} (never over a value it has), then archive them as .restored", names.join(", ")),
    });
}

/// The provider (#12): what this machine expects, what Claude Code's own
/// resolver gives for a launch here (no member's settings), what its members
/// were recorded on, and the `claude` binaries a `--claude-bin` could name
/// (listed, never written).
fn provider_lines(ctx: &Ctx, r: &mut Report) {
    use nimitz_core::provider;
    let env = &ctx.env;
    let label = provider::this_machine(env);
    let expected = provider::expected(env);
    let here = provider::preflight(&[env.claude_bin.clone()], Some(&env.home.to_string_lossy()));
    let recorded: std::collections::BTreeMap<String, usize> = provider::records(env).iter().fold(Default::default(), |mut m, rec| {
        *m.entry(provider::name_of(&rec.provider)).or_default() += 1;
        m
    });
    let members = if recorded.is_empty() { "no member recorded yet".to_string() } else { recorded.iter().map(|(p, n)| format!("{n} on {p}")).collect::<Vec<_>>().join(", ") };
    let set = format!("nimitz machine set {label} --provider anthropic|bedrock|vertex");
    match (&expected, &here) {
        (Some(want), Ok(got)) if provider::api_of(want) == Some(got.as_str()) => r.ok(&format!("provider: machine {label} expects {want}; a launch here gives {want}; members: {members}")),
        (Some(want), Ok(got)) => r.err(&format!("provider: machine {label} expects {want}, but a launch here gives {} (cold launches are refused)", provider::name_of(got)), Some("fix the settings that select the provider, or nimitz machine set")),
        (None, Ok(got)) if got == "bedrock" || provider::recorded_on(env, "bedrock") => {
            r.err(&format!("provider: machine {label} names none, and it runs Bedrock (a launch here gives {}; members: {members}): cold launches are refused", provider::name_of(got)), Some(&format!("nimitz machine set {label} --provider bedrock")))
        }
        (None, Ok(got)) => r.info(&format!("provider: machine {label} names none (optional here); a launch here gives {}; members: {members}", provider::name_of(got)), Some(&set)),
        (_, Err(e)) => r.warn(&format!("provider: machine {label} expects {}; Claude Code's own answer could not be read: {e}", expected.as_deref().unwrap_or("none")), Some("check `claude auth status --json`")),
    }
    let cands = provider::claude_bin_candidates(env);
    if !cands.is_empty() {
        r.info(&format!("claude binaries here (for --claude-bin; nothing is written): {}", cands.join(", ")), Some(&format!("nimitz machine set {label} --claude-bin <one of them>")));
    }
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
    stale_processes(&mut r);
    kept_registries(env, &mut r);
    if env.home.is_dir() {
        if trusted(ctx, &env.home) {
            r.ok(&format!("Claude Code trusts {}", env.home.display()));
        } else {
            // the real path: Claude Code resolves a linked data root, and (Windows) compares its case
            let real = nimitz_core::env::canon(&env.home);
            match nimitz_core::trust::too_broad(env, &env.home) {
                None => {
                    r.err(&format!("Claude Code does not trust {}", real.display()), Some("nimitz doctor --fix"));
                    r.fixes.steps.push(nimitz_core::trust::step(env, None, &env.home));
                }
                Some(why) => r.err(&format!("Claude Code does not trust {}; {why}", real.display()), None),
            }
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
    // The launch binary (cold launches): NIMITZ_CLAUDE_BIN or `claude` on PATH.
    let launch = resolved(&env.claude_bin);
    let mut launch_cmd = std::process::Command::new(&env.claude_bin);
    launch_cmd.arg("--version");
    nimitz_core::claude::scrub_child_env(&mut launch_cmd);
    let launch_v = launch_cmd
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| nimitz_core::compat::parse_version(&String::from_utf8_lossy(&o.stdout)));
    let from = match nimitz_core::provider::claude_bin_source(env, &ctx.vars) {
        "NIMITZ_CLAUDE_BIN" => "NIMITZ_CLAUDE_BIN",
        "machines.toml" => "this machine's claude_bin",
        _ => "first `claude` on PATH",
    };
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

    provider_lines(ctx, &mut r);

    // Herdr's agent resume on restart (finding 10): with it on, a restart
    // runs `claude --resume` in every agent pane, which holds a stopped or
    // parked member's conversation interactively; a restore then cannot wake
    // it until that process ends (`tab.resumed`). nimitz never edits that file.
    herdr_resume_check(ctx, &mut r);

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
                    let hooks = v.get("hooks").unwrap_or(&Value::Null);
                    let lacks = missing_hooks(&registered_hooks(hooks));
                    let stringy = string_form_hooks(hooks);
                    if !stringy.is_empty() {
                        // a shell between Claude Code and the hook: the pid chain
                        // usually recovers it, but the plugin's form is command + args
                        r.warn(
                            &format!("hooks: the installed nimitz plugin ({}) gives {} as a command string; such a hook runs under sh -c, so its parent is a shell, not the session", p.display(), stringy.join(", ")),
                            Some(&format!("{}, then /reload-plugins in each live session", plugin_install_fix())),
                        );
                    }
                    if lacks.is_empty() {
                        r.ok(&format!("hooks: the nimitz plugin is enabled and {} registers every event nimitz needs", p.display()));
                    } else {
                        r.err(
                            &format!("hooks: the installed nimitz plugin ({}) does not register {}", p.display(), lacks.join(", ")),
                            Some(PLUGIN_UPDATE_FIX),
                        );
                    }
                    // sessions load Claude Code's cached copy, not the repo: one
                    // older than this binary lacks what it needs
                    let cached = p.parent().and_then(Path::parent).and_then(|d| read_json(&d.join(".claude-plugin/plugin.json"))).and_then(|v| v["version"].as_str().map(str::to_string));
                    if let Some(v) = cached.filter(|v| v != env!("CARGO_PKG_VERSION")) {
                        r.warn(&format!("plugin: the cached nimitz plugin is {v}, this nimitz is {} (sessions load the cached copy, {})", env!("CARGO_PKG_VERSION"), p.parent().and_then(Path::parent).map(|d| d.display().to_string()).unwrap_or_default()), Some(PLUGIN_UPDATE_FIX));
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
    // nimitz reads past bytes left behind a session file's object; Claude
    // Code does not, and then every resume on the machine is a copy (issue #6)
    for b in nimitz_core::evidence::unparseable_sessions(env) {
        if b.blocks {
            r.err(&format!("Claude Code cannot parse session file {}; while it stays broken, every resume on this machine starts a copy", b.describe()), Some(&b.fix()));
        } else {
            // Claude Code skips a file whose process is gone (or newer than it)
            r.info(&format!("a stale unreadable session file, harmless: {}", b.describe()), Some(&b.fix()));
        }
    }
    match nimitz_core::trust::format_check(env) {
        Ok(n) => r.ok(&format!("pinned formats: Claude Code's trust table reads as nimitz writes it ({n} trusted folder key(s))")),
        Err(e) => bad.push(format!("{e} (nimitz trust needs projects.<path>.hasTrustDialogAccepted)")),
    }
    if bad.is_empty() {
        r.ok(&format!("pinned formats: {seen} session/job file(s) carry every field nimitz reads"));
    } else {
        for b in bad.iter().take(5) {
            r.err(&format!("pinned formats: {b}"), None);
        }
    }

    formats_check(env, &mut r);

    if fix && !r.fixes.steps.is_empty() {
        let fixes = r.fixes.clone();
        match ctx.gate(&fixes) {
            out::Go::Exit(c) => return c.max(1),
            out::Go::Run => {
                for s in &fixes.steps {
                    if s.cmd[0] == "registry-restore" {
                        match nimitz_core::registry::restore_from_kept(&ctx.env, false) {
                            Ok(done) => {
                                out::ok(&format!("fixed: {} field(s) filled, {} kept file(s) archived", done.filled.len(), done.archived.len()));
                                r.errors -= 1;
                            }
                            Err(e) => out::error(&format!("fix failed: {e}")),
                        }
                        continue;
                    }
                    if s.cmd[0] == "trust" {
                        match nimitz_core::trust::write(&ctx.env, Path::new(&s.cmd[1])) {
                            Ok(_) => {
                                out::ok(&format!("fixed: {}", s.why));
                                r.errors -= 1;
                            }
                            Err(e) => out::error(&format!("fix failed: {e}")),
                        }
                        continue;
                    }
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

/// Formats (docs/design/migrations.md): a step that failed (Error, with its
/// log line and the backup), a file a newer nimitz wrote (Error), the local
/// steps due (Info), the shared ones (Warn: each affects every machine), and
/// the machines of each fleet no nimitz with the guard was seen on (Warn).
fn formats_check(env: &nimitz_core::env::Env, r: &mut Report) {
    use nimitz_core::formats::{self, Scope};
    let mut quiet = true;
    for f in formats::failed(env) {
        let i = f.issue();
        r.err(&format!("formats: {}", i.message), i.fix.as_deref());
        quiet = false;
    }
    for t in formats::too_new(env) {
        let i = t.issue(None);
        r.err(&format!("formats: {}", i.message), i.fix.as_deref());
        quiet = false;
    }
    let due = formats::due(env, formats::ARTIFACTS, None);
    let local = formats::moved(&due.iter().filter(|d| d.scope == Scope::Local).cloned().collect::<Vec<_>>());
    if !local.is_empty() {
        let when = if formats::up_to_date(env) { "nimitz migrate --yes applies them" } else { "the next nimitz command applies them (or nimitz migrate --yes)" };
        r.info(&format!("formats: local step(s) due here: {}; {when}", local.join(", ")), None);
        quiet = false;
    }
    for d in due.iter().filter(|d| d.scope == Scope::Shared) {
        let every = d.min_nimitz.map(|m| format!("each one needs nimitz >= {m} first")).unwrap_or_else(|| "update each one first".into());
        r.warn(&format!("formats: {} due on {} ({}); it affects every machine: {every}", d.label(), d.file.display(), d.summary), Some("nimitz migrate --shared --yes"));
        quiet = false;
    }
    for f in nimitz_core::config::load_all(env).0 {
        for i in formats::machine_issues(&f) {
            r.warn(&format!("{}: {}", f.config.fleet, i.message), i.fix.as_deref());
            quiet = false;
        }
    }
    if quiet {
        r.ok(&format!("formats: every file here is at the format nimitz {} writes; every listed machine runs it", formats::NIMITZ));
    }
}

/// The fix for a cached plugin that is old or lacks a hook: sessions load
/// Claude Code's cached copy (`~/.claude/plugins/cache/claude-nimitz/nimitz/<version>/`),
/// never the repo, so the marketplace and the plugin are updated, then read again.
const PLUGIN_UPDATE_FIX: &str = "claude plugin marketplace update claude-nimitz && claude plugin update nimitz@claude-nimitz, then /reload-plugins in each live session";

/// The marketplace to add: this build's repo when it is still on disk, else GitHub.
pub fn plugin_install_fix() -> String {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = match nimitz_core::os::canonical(&repo) {
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
    match nimitz_core::os::canonical(&p) {
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
        assert_eq!(lacks, vec!["UserPromptSubmit".to_string(), "PreToolUse (matcher AskUserQuestion)".to_string(), "UserPromptSubmit:rules".to_string()]);
        // an entry with no matcher covers every tool; a command that is not nimitz counts for nothing
        let any = serde_json::json!({"PreToolUse": [{"hooks": [{"type": "command", "command": "nimitz", "args": ["hook", "PreToolUse"]}]}],
            "Stop": [{"hooks": [{"type": "command", "command": "other-tool", "args": ["hook", "Stop"]}]}]});
        let reg = registered_hooks(&any);
        assert_eq!(reg, vec![("PreToolUse".to_string(), String::new())]);
        let lacks = missing_hooks(&reg);
        assert!(lacks.contains(&"Stop".to_string()) && !lacks.iter().any(|l| l.starts_with("PreToolUse (")), "{lacks:?}");
        assert!(lacks.contains(&"PreToolUse:kb-guard (every tool)".to_string()), "{lacks:?}");
        // the guard's own entry (every tool) is no PreToolUse handler: the matchers are still missing
        let guard_only = serde_json::json!({"PreToolUse": [{"matcher": "*", "hooks": [{"type": "command", "command": "nimitz", "args": ["hook", "kb-guard"]}]}]});
        let lacks = missing_hooks(&registered_hooks(&guard_only));
        assert!(lacks.contains(&"PreToolUse (matcher SendMessage)".to_string()) && !lacks.iter().any(|l| l.contains("kb-guard")), "{lacks:?}");
        // the harness rules' entry is no SessionStart handler, and a plugin without it lacks it
        let rules_only = serde_json::json!({"SessionStart": [{"hooks": [{"type": "command", "command": "nimitz", "args": ["hook", "rules", "harness"]}]}]});
        let lacks = missing_hooks(&registered_hooks(&rules_only));
        assert!(lacks.contains(&"SessionStart".to_string()) && !lacks.contains(&"SessionStart:rules".to_string()), "{lacks:?}");
        let mut no_rules = v["hooks"].clone();
        no_rules["UserPromptSubmit"].as_array_mut().unwrap().retain(|e| e["hooks"][0]["args"][1] != "rules");
        assert_eq!(missing_hooks(&registered_hooks(&no_rules)), vec!["UserPromptSubmit:rules".to_string()]);
    }

    /// A hook given as one command string is named (the repo plugin has none).
    #[test]
    fn a_command_string_hook_is_named() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin/hooks/hooks.json");
        let v = read_json(&p).expect("plugin/hooks/hooks.json");
        assert!(string_form_hooks(&v["hooks"]).is_empty());
        let mixed = serde_json::json!({
            "SessionStart": [{"hooks": [{"type": "command", "command": "/x/nimitz hook SessionStart", "timeout": 5}]}],
            "Stop": [{"hooks": [{"type": "command", "command": "nimitz", "args": ["hook", "Stop"]}]}],
            "PreToolUse": [{"matcher": "SendMessage", "hooks": [{"type": "command", "command": "nimitz hook PreToolUse"}]}],
            "Notification": [{"hooks": [{"type": "command", "command": "other-tool hook Notification"}]}]});
        assert_eq!(string_form_hooks(&mixed), vec!["PreToolUse".to_string(), "SessionStart".to_string()]);
        // such entries still count as registered: the fallback finds the session
        assert!(registered_hooks(&mixed).contains(&("SessionStart".to_string(), String::new())));
    }
}
