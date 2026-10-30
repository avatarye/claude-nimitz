//! Which API provider a member's Claude Code talks to (issue #12), and the
//! per-machine settings that pin it: `H/machines.toml`, `[machines.<label>]`
//! with `claude_bin` (the cold-launch binary, and the native one unless
//! `native_claude_bin` names it; `NIMITZ_CLAUDE_BIN` and
//! `NIMITZ_NATIVE_CLAUDE_BIN` still win) and `provider` (`anthropic`, `bedrock`
//! or `vertex`).
//!
//! Claude Code picks its provider from its env once every settings `env`
//! block is merged (probe-12, 2.1.312): a `CLAUDE_CODE_USE_*` selector, none
//! for the Anthropic API (`firstParty`). Three readings, none trusted alone:
//! - before a cold launch, Claude Code's own resolver: `<claude_bin>
//!   --settings <the launch's JSON> auth status --json` (`apiProvider`), run
//!   in the member's folder with the launch's env (`preflight`);
//! - after it, the worker's real view: its SessionStart hook records the
//!   stdin `model` and every selector (`record`, `NIMITZ_STATE/provider/`),
//!   rewritten on every wake;
//! - never the job's `providerEnv`: it misses `--settings` env.
use crate::env::Env;
use crate::types::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// The providers a machine may expect: (the name nimitz takes, Claude Code's `apiProvider`).
pub const PROVIDERS: [(&str, &str); 3] = [("anthropic", "firstParty"), ("bedrock", "bedrock"), ("vertex", "vertex")];

/// Claude Code's provider selectors, in the order it reads them, each with
/// its `apiProvider` (2.1.312); none set is `firstParty`.
pub const SELECTORS: [(&str, &str); 7] = [
    ("CLAUDE_CODE_USE_BEDROCK", "bedrock"),
    ("CLAUDE_CODE_USE_VERTEX", "vertex"),
    ("CLAUDE_CODE_USE_FOUNDRY", "foundry"),
    ("CLAUDE_CODE_USE_ANTHROPIC_AWS", "anthropicAws"),
    ("CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD", "anthropicGoogleCloud"),
    ("CLAUDE_CODE_USE_MANTLE", "mantle"),
    ("CLAUDE_CODE_USE_GATEWAY", "gateway"),
];

/// The env the SessionStart record keeps besides the selectors (no secrets).
pub const RECORDED: [&str; 7] = ["CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST", "ANTHROPIC_BASE_URL", "AWS_REGION", "AWS_PROFILE", "ANTHROPIC_VERTEX_PROJECT_ID", "CLOUD_ML_REGION", "ANTHROPIC_BEDROCK_REGION_PREFIX"];

/// `apiProvider` of a provider name (`anthropic` -> `firstParty`).
pub fn api_of(name: &str) -> Option<&'static str> {
    PROVIDERS.iter().find(|(n, _)| *n == name).map(|(_, a)| *a)
}

/// The name nimitz shows for an `apiProvider` (`firstParty` -> `anthropic`).
pub fn name_of(api: &str) -> String {
    PROVIDERS.iter().find(|(_, a)| *a == api).map(|(n, _)| n.to_string()).unwrap_or_else(|| api.to_string())
}

/// Claude Code's truthy env value.
fn truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

/// The `apiProvider` an env selects: the first selector set, else `firstParty`.
pub fn from_selectors(get: impl Fn(&str) -> Option<String>) -> &'static str {
    SELECTORS.iter().find(|(k, _)| get(k).is_some_and(|v| truthy(&v))).map(|(_, p)| *p).unwrap_or("firstParty")
}

/// The provider family a model id names: `bedrock` for a region prefix
/// (`us.`, `eu.`, `apac.`, `global.`, `anthropic.`) or a Bedrock ARN,
/// `vertex` for an `@` version, `firstParty` for a bare `claude-*`; None
/// when it says nothing (no model, an alias).
pub fn model_family(model: &str) -> Option<&'static str> {
    let m = model.trim().to_ascii_lowercase();
    let bedrock = ["us.", "eu.", "apac.", "global.", "jp.", "au.", "anthropic."].iter().any(|p| m.starts_with(p)) || m.starts_with("arn:aws:bedrock");
    if bedrock {
        Some("bedrock")
    } else if m.starts_with("claude-") && m.contains('@') {
        Some("vertex")
    } else if m.starts_with("claude-") {
        Some("firstParty")
    } else {
        None
    }
}

// ------------------------------------------------------------------ machines.toml

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_bin: Option<String>,
    /// the binary for resume, stop and rm where it is not `claude_bin` (a
    /// file with this key carries `NATIVE_BIN_MIN_NIMITZ`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_claude_bin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// The oldest nimitz that knows `native_claude_bin` (0.7.16 refuses the key).
pub const NATIVE_BIN_MIN_NIMITZ: &str = "0.7.17";

/// `H/machines.toml`: each machine's own settings, by label (shared: a lead
/// may set another machine's; only that machine uses them). A nimitz older
/// than 0.7.16 does not read it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machines {
    /// the file's format (`formats`, `machines`; absent = 1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<u32>,
    /// the oldest nimitz that may read the file (`formats::guard`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_nimitz: Option<String>,
    #[serde(default)]
    pub machines: BTreeMap<String, MachineSettings>,
}

pub fn machines_path(home: &std::path::Path) -> PathBuf {
    home.join("machines.toml")
}

/// The machines file (none: empty). As every shared file (migrations.md
/// section 5): its `format` and `min_nimitz` read leniently first (a newer
/// nimitz's file is never read further), then the strict parse. Anything
/// but "not there" is an error: never "no settings".
pub fn load_machines(home: &std::path::Path) -> Result<Machines, String> {
    let p = machines_path(home);
    match std::fs::read_to_string(&p) {
        Ok(t) => {
            crate::formats::guard_toml("machines", &p, &t).map_err(|e| e.to_string())?;
            toml::from_str(&t).map_err(|e| format!("{}: {e}", p.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Machines::default()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

/// THE machine label (every reader: `crud::machine_label`, `this_machine`,
/// `claude_bin_for`): the registry's `machine` under `state` (as it is:
/// never changed here), else, with no registry yet, `host_label` (what a
/// fresh registry records). A registry a newer nimitz wrote, or one that
/// does not read, is an error: never a fall back to another label.
pub fn machine_label_at(state: &std::path::Path, hostname: &str) -> Result<String, String> {
    let p = state.join("registry.json");
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(host_label(hostname)),
        Err(e) => return Err(format!("{}: {e}", p.display())),
    };
    crate::formats::guard_json_bytes("registry", &p, &bytes, "version").map_err(|e| e.to_string())?;
    let v = crate::evidence::first_json(&bytes).ok_or_else(|| format!("{}: does not parse", p.display()))?;
    Ok(v.get("machine").and_then(Value::as_str).filter(|m| !m.is_empty()).map(str::to_string).unwrap_or_else(|| host_label(hostname)))
}

/// The label a host name gives, the one rule (`machine_label_at`, a fresh
/// registry): lowercase, anything but `[a-z0-9_-]` a `-`, up to its first
/// dot; one that is no valid label (a Windows device name, nothing left) is
/// `this-machine`.
pub fn host_label(hostname: &str) -> String {
    let l: String = hostname.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
    let l = l.split('.').next().unwrap_or("").to_string();
    if crate::config::valid_label(&l) { l } else { "this-machine".into() }
}

/// This machine's label (`machine_label_at`).
pub fn this_machine(env: &Env) -> Result<String, String> {
    machine_label_at(&env.state, &env.hostname)
}

/// This machine's settings: none set is the default; a machines file or a
/// registry that cannot be read is an error (`provider.unreadable`).
pub fn settings(env: &Env) -> Result<MachineSettings, String> {
    // the file first: with none (or no machine in it) nothing is set, and no
    // label is needed (an optional machine never reads the registry here)
    let m = load_machines(&env.home)?;
    if m.machines.is_empty() {
        return Ok(MachineSettings::default());
    }
    let label = this_machine(env)?;
    Ok(m.machines.get(&label).cloned().unwrap_or_default())
}

/// The provider this machine expects (a name: `bedrock`), if set.
pub fn expected(env: &Env) -> Result<Option<String>, String> {
    Ok(settings(env)?.provider)
}

/// The refusal when the machine's settings cannot be read: nothing is
/// enforced from a file nimitz cannot read, so nothing launches.
pub fn unreadable(id: Option<&Id>, e: &str) -> Issue {
    Issue {
        code: "provider.unreadable".into(),
        severity: Severity::Error,
        message: format!("this machine's provider settings cannot be read ({e}); nothing is launched or woken while they cannot"),
        fix: Some("fix the file it names: machines.toml (`nimitz machine set` rewrites it) or the registry (`nimitz doctor`); one a newer nimitz wrote: update this nimitz".into()),
        id: id.cloned(),
    }
}

/// The (`claude_bin`, `native_claude_bin`) the machines file sets for this
/// machine (its label by `machine_label_at`): read while the Env is built, so
/// it cannot use one. None when unset or unreadable (the launch's checks then
/// refuse). On Windows an npm `claude.cmd` is followed to its `claude.exe`,
/// as on PATH.
pub fn machine_bins(home: &std::path::Path, state: &std::path::Path, hostname: &str) -> (Option<String>, Option<String>) {
    // (every nimitz process builds an Env, every hook too: no file, no parse)
    if !machines_path(home).exists() {
        return (None, None);
    }
    let Ok(m) = load_machines(home) else { return (None, None) };
    let set = |b: &Option<String>| b.as_deref().is_some_and(|b| !b.is_empty());
    if !m.machines.values().any(|s| set(&s.claude_bin) || set(&s.native_claude_bin)) {
        return (None, None);
    }
    let Ok(label) = machine_label_at(state, hostname) else { return (None, None) };
    let Some(s) = m.machines.get(&label) else { return (None, None) };
    let real = |b: &Option<String>| b.clone().filter(|b| !b.is_empty()).map(|b| crate::env::npm_target(std::path::Path::new(&b)).map(|p| p.to_string_lossy().into_owned()).unwrap_or(b));
    (real(&s.claude_bin), real(&s.native_claude_bin))
}

/// This machine's `claude_bin` (`machine_bins`).
pub fn claude_bin_for(home: &std::path::Path, state: &std::path::Path, hostname: &str) -> Option<String> {
    machine_bins(home, state, hostname).0
}

/// Every machine label nimitz knows: this machine's and every fleet's.
fn known_labels(env: &Env) -> Vec<String> {
    let mut v: Vec<String> = this_machine(env).into_iter().collect();
    for f in crate::config::load_all(env).0 {
        v.extend(f.config.machines.keys().cloned());
    }
    v.sort();
    v.dedup();
    v
}

/// `nimitz machine set <label> [--claude-bin P] [--native-bin P] [--provider P]`
/// (`Some("")` clears one). The file is shared; a machine reads its own label's.
pub fn plan_machine_set(env: &Env, label: &str, claude_bin: Option<&str>, native_bin: Option<&str>, provider: Option<&str>) -> Plan {
    let mut plan = Plan { summary: format!("set machine {label}"), ..Default::default() };
    let refuse = |plan: &mut Plan, msg: String, fix: Option<String>| plan.refusals.push(Issue { code: "config.invalid".into(), severity: Severity::Error, message: msg, fix, id: None });
    if label.is_empty() || !label.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        refuse(&mut plan, format!("{label:?} is not a machine label (letters, digits, . _ -)"), None);
        return plan;
    }
    if claude_bin.is_none() && native_bin.is_none() && provider.is_none() {
        refuse(&mut plan, "nothing to set".into(), Some(format!("nimitz machine set {label} --provider anthropic|bedrock|vertex [--claude-bin PATH] [--native-bin PATH]")));
        return plan;
    }
    if let Some(p) = provider.filter(|p| !p.is_empty() && api_of(p).is_none()) {
        refuse(&mut plan, format!("--provider {p}: use anthropic, bedrock or vertex"), None);
        return plan;
    }
    for (flag, bin) in [("--claude-bin", claude_bin), ("--native-bin", native_bin)] {
        let Some(b) = bin.filter(|b| !b.is_empty()) else { continue };
        if !std::path::Path::new(b).is_absolute() {
            refuse(&mut plan, format!("{flag} {b}: an absolute path"), None);
            return plan;
        }
        if this_machine(env).is_ok_and(|me| me == label) && !std::path::Path::new(b).is_file() {
            refuse(&mut plan, format!("{flag} {b}: no such file on this machine"), Some("nimitz doctor  # lists the claude binaries it finds".into()));
            return plan;
        }
    }
    let mut m = match load_machines(&env.home) {
        Ok(m) => m,
        Err(e) => {
            refuse(&mut plan, format!("cannot read the machines file: {e}"), Some(format!("fix or remove {}", machines_path(&env.home).display())));
            return plan;
        }
    };
    // a machine set up ahead of time is fine; a typo should be seen
    let known = known_labels(env);
    if !known.iter().any(|k| k == label) {
        plan.warnings.push(Issue {
            code: "machine.unknown".into(),
            severity: Severity::Warn,
            message: format!("{label} is neither this machine nor any fleet's (known: {})", if known.is_empty() { "none".into() } else { known.join(", ") }),
            fix: Some(format!("check the label; a machine set up ahead of time: nimitz machine add <F> {label}")),
            id: None,
        });
    }
    let before = m.clone();
    let s = m.machines.entry(label.to_string()).or_default();
    if let Some(b) = claude_bin {
        s.claude_bin = Some(b.to_string()).filter(|b| !b.is_empty());
    }
    if let Some(b) = native_bin {
        s.native_claude_bin = Some(b.to_string()).filter(|b| !b.is_empty());
    }
    if let Some(p) = provider {
        s.provider = Some(p.to_string()).filter(|p| !p.is_empty());
    }
    if m.machines.get(label).is_some_and(|s| *s == MachineSettings::default()) {
        m.machines.remove(label);
    }
    // an older nimitz refuses the key: it says "newer nimitz needed" instead
    if m.machines.values().any(|s| s.native_claude_bin.is_some()) && m.min_nimitz.as_deref().is_none_or(|v| crate::compat::older(v, NATIVE_BIN_MIN_NIMITZ)) {
        m.min_nimitz = Some(NATIVE_BIN_MIN_NIMITZ.to_string());
    }
    if m != before {
        let text = match toml::to_string_pretty(&m) {
            Ok(t) => t,
            Err(e) => {
                refuse(&mut plan, format!("the machines file cannot be written ({e})"), None);
                return plan;
            }
        };
        plan.steps.push(Step { id: None, cmd: vec!["write".into(), machines_path(&env.home).to_string_lossy().into_owned(), text], cwd: None, why: format!("machine {label}'s settings") });
    }
    plan
}

// ------------------------------------------------------------------ the preflight

/// Claude Code's own answer, before a cold launch: `<bin> --settings <S>
/// auth status --json` with the launch's binary, `--settings`, folder and
/// env (`claude::run`'s). Its `apiProvider`, or why it has none.
pub fn preflight(cmd: &[String], cwd: Option<&str>) -> Result<String, String> {
    let bin = cmd.first().ok_or("no command")?;
    let mut argv = vec![bin.clone()];
    // every --settings the launch passes, in order, before the subcommand
    let mut i = 1;
    while i < cmd.len() {
        if cmd[i] == "--settings" {
            if let Some(s) = cmd.get(i + 1) {
                argv.extend(["--settings".to_string(), s.clone()]);
            }
            i += 1;
        }
        i += 1;
    }
    argv.extend(["auth".to_string(), "status".into(), "--json".into()]);
    let (rc, text) = run_bounded(&argv, cwd, preflight_timeout())?;
    let v = text.find('{').and_then(|at| crate::evidence::first_json(text[at..].as_bytes()));
    match v.as_ref().and_then(|v| v.get("apiProvider")).and_then(Value::as_str) {
        Some(p) => Ok(p.to_string()),
        None => Err(format!("`{} auth status --json` gave no apiProvider (exit {rc}): {}", bin, text.chars().take(200).collect::<String>())),
    }
}

/// How long the preflight waits for `auth status` (20 s; `NIMITZ_PREFLIGHT_TIMEOUT`
/// in seconds, for tests): past it, the answer is unconfirmed.
fn preflight_timeout() -> std::time::Duration {
    let s = std::env::var("NIMITZ_PREFLIGHT_TIMEOUT").ok().and_then(|v| v.parse::<f64>().ok()).filter(|s| *s > 0.0).unwrap_or(20.0);
    std::time::Duration::from_secs_f64(s)
}

/// `claude::run`, but given up (the process killed) after `limit`: `Err`
/// says so. Its output is read while it runs (a thread per pipe, so a large
/// output never blocks it), and the reads count against the same limit (a
/// child left holding a pipe never hangs the preflight).
fn run_bounded(cmd: &[String], cwd: Option<&str>, limit: std::time::Duration) -> Result<(i32, String), String> {
    use std::io::Read;
    let bin = cmd.first().ok_or("no command")?;
    let mut c = std::process::Command::new(bin);
    c.args(&cmd[1..]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    crate::claude::scrub_child_env(&mut c);
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    let mut child = c.spawn().map_err(|e| format!("{bin}: {e}"))?;
    let gave_up = || format!("`{bin} auth status --json` gave no answer within {} s (killed)", limit.as_secs_f64());
    let (tx, rx) = std::sync::mpsc::channel::<(u8, String)>();
    let drain = |n: u8, pipe: Option<Box<dyn Read + Send>>, tx: std::sync::mpsc::Sender<(u8, String)>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            let _ = tx.send((n, String::from_utf8_lossy(&buf).into_owned()));
        });
    };
    drain(0, child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>), tx.clone());
    drain(1, child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>), tx);
    let deadline = std::time::Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(gave_up());
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => return Err(format!("{bin}: {e}")),
        }
    };
    // both pipes, within what is left of the limit
    let mut parts = [String::new(), String::new()];
    for _ in 0..2 {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok((n, text)) => parts[n as usize] = text,
            Err(_) => return Err(gave_up()),
        }
    }
    Ok((status.code().unwrap_or(-1), format!("{}{}", parts[0], parts[1]).trim().to_string()))
}

/// Is a cold launch `cmd` (in `cwd`) allowed on this machine? Its provider
/// by Claude Code's own resolver against the machine's expected one; with
/// none set, a Bedrock launch (or a machine with a member recorded on
/// Bedrock) needs one first. `Err` is the refusal; `Ok` may carry a warning.
/// An answer that cannot be read refuses (fail-closed) where a provider is
/// named or a member runs Bedrock; on an optional machine (neither) it is a
/// warning and the launch goes ahead (an older Claude Code may have no
/// `auth status --json`). Settings that cannot be read refuse.
pub fn check_launch(env: &Env, id: Option<&Id>, cmd: &[String], cwd: Option<&str>) -> Result<Option<Issue>, Issue> {
    // (an optional machine needs no label: only for the messages)
    let expected = match expected(env) {
        Ok(x) => x,
        Err(e) => return Err(unreadable(id, &e)),
    };
    let label = this_machine(env).unwrap_or_else(|_| "<this machine>".into());
    let set = format!("nimitz machine set {label} --provider bedrock|anthropic|vertex");
    let issue = |code: &str, message: String, fix: String| Issue { code: code.into(), severity: Severity::Error, message, fix: Some(fix), id: id.cloned() };
    let who = id.map(|i| i.to_string()).unwrap_or_else(|| "the member".into());
    let got = match preflight(cmd, cwd) {
        Ok(p) => p,
        Err(e) if !strict(env, expected.as_deref()) => {
            return Ok(Some(Issue {
                code: "provider.unconfirmed".into(),
                severity: Severity::Warn,
                message: format!("{who}'s provider could not be confirmed before its launch ({e}); machine {label} names none, so it launches"),
                fix: Some(format!("nimitz doctor; {set}  # then every launch here is checked")),
                id: id.cloned(),
            }))
        }
        Err(e) => {
            return Err(issue(
                "provider.unconfirmed",
                format!("{who}'s provider cannot be confirmed before its launch, so it is not launched: {e}"),
                match expected {
                    Some(_) => format!("check that `claude auth status --json` works in {}; nimitz doctor", cwd.unwrap_or(".")),
                    None => format!("nimitz doctor; {set}"),
                },
            ))
        }
    };
    match expected.as_deref() {
        Some(want) if api_of(want) == Some(got.as_str()) => Ok(None),
        Some(want) => Err(issue(
            "provider.mismatch",
            format!("{who} would launch on {} but machine {label} expects {want}; nothing was launched", name_of(&got)),
            format!("fix the settings that select {} (the shell env, ~/.claude/settings.json, the folder's .claude settings or the fleet's harness settings), or {set}", name_of(&got)),
        )),
        None if got == "bedrock" || recorded_on(env, "bedrock") => Err(issue(
            "provider.unset",
            format!("{who} would launch on {} and machine {label} has no expected provider; a machine that runs Bedrock members must name its provider first", name_of(&got)),
            format!("nimitz machine set {label} --provider bedrock"),
        )),
        None => Ok(None),
    }
}

/// Does an unreadable answer refuse here (fail-closed)? Where a provider is
/// named, or a member runs on Bedrock; an optional machine only warns.
pub fn strict(env: &Env, expected: Option<&str>) -> bool {
    expected.is_some() || recorded_on(env, "bedrock")
}

/// After a start bound session `sid` of `id` (a step that began at `since`):
/// the SessionStart record of THIS start (`at` >= `since`: a relaunch keeps
/// the session id, so an older record says nothing) against the machine's
/// expected provider, waited for a moment. A provider its selectors give
/// that differs is the refusal; a model that disagrees with them (a custom
/// `ANTHROPIC_MODEL`, an alias) is a warning. No record is a warning after
/// a cold launch with its prompt (the preflight checked it) and the refusal
/// after a start that sends none (`wake`: a wake or a relaunch, so this is
/// the gate there). No expected provider, nothing to compare.
pub fn check_started(env: &Env, id: &Id, sid: &str, since: f64, wake: bool) -> Result<Option<Issue>, Issue> {
    let (label, want) = match this_machine(env).and_then(|l| expected(env).map(|e| (l, e))) {
        Ok((l, Some(w))) => (l, w),
        Ok((_, None)) => return Ok(None),
        Err(e) => return Err(unreadable(Some(id), &e)),
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(if wake { 10 } else { 3 });
    let rec = loop {
        match session_record(env, sid).filter(|r| r.at >= since) {
            Some(r) => break Some(r),
            None if std::time::Instant::now() >= deadline => break None,
            None => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    };
    let what = if wake { "its job is stopped (its row and conversation are kept); nothing was sent to it" } else { "its job is stopped and removed, but its first prompt may already have been sent" };
    let Some(r) = rec else {
        let message = format!("{id}'s SessionStart left no provider record for this start of session {sid}");
        return match wake {
            false => Ok(Some(Issue {
                code: "provider.unrecorded".into(),
                severity: Severity::Warn,
                message: format!("{message}; its provider is checked by the preflight only"),
                fix: Some(format!("nimitz doctor  # the hooks; then nimitz check {id}")),
                id: Some(id.clone()),
            })),
            true => Err(Issue {
                code: "provider.unrecorded".into(),
                severity: Severity::Error,
                message: format!("{message}, so its provider is unconfirmed; {what}"),
                fix: Some(format!("nimitz doctor  # the hooks must run in its folder; then nimitz start {id}")),
                id: Some(id.clone()),
            }),
        };
    };
    if api_of(&want) == Some(r.provider.as_str()) {
        return Ok((!r.model_agrees()).then(|| Issue {
            code: "provider.model".into(),
            severity: Severity::Warn,
            message: format!("{id} runs on {want}, but its model {} is not a {want} model", r.model.as_deref().unwrap_or_default()),
            fix: Some(format!("nimitz check {id}  # a custom ANTHROPIC_MODEL or alias is fine; anything else: the folder's and managed settings")),
            id: Some(id.clone()),
        }));
    }
    Err(Issue {
        code: "provider.mismatch".into(),
        severity: Severity::Error,
        message: format!(
            "{id} started on {} (model {}) although machine {label} expects {want} and its launch was checked (managed settings?); {what}",
            name_of(&r.provider),
            r.model.as_deref().unwrap_or("unknown")
        ),
        fix: Some(match wake {
            false => format!("check the managed settings (/etc/claude-code, /Library/Application Support/ClaudeCode, MDM) and `claude auth status --json` in its folder; then nimitz start {id}"),
            true => format!("check the managed settings and `claude auth status --json` in its folder; then nimitz restart {id}  # relaunches it with every flag, checked first"),
        }),
        id: Some(id.clone()),
    })
}

/// The selector `--settings` gets so a member runs on this machine's
/// provider: `CLAUDE_CODE_USE_BEDROCK=1` or `_VERTEX=1`; none for anthropic
/// (or no provider named, or settings that cannot be read: the launch's
/// checks refuse those). Settings env beats the shell's and the folder's
/// local settings, and the job's `respawnFlags` keep it for every wake.
pub fn forced_selector(env: &Env) -> Option<(&'static str, &'static str)> {
    forced_for(expected(env).ok()??.as_str())
}

fn forced_for(provider: &str) -> Option<(&'static str, &'static str)> {
    match provider {
        "bedrock" => Some(("CLAUDE_CODE_USE_BEDROCK", "1")),
        "vertex" => Some(("CLAUDE_CODE_USE_VERTEX", "1")),
        _ => None,
    }
}

/// `wake_as_saved` for a row that may have no job (no saved flags to read):
/// as saved only where nothing is checked.
pub fn wakes_as_saved(env: &Env, job: Option<&str>) -> bool {
    match job {
        Some(j) => wake_as_saved(env, j),
        None => !gated(env),
    }
}

/// May a stopped job be woken as it is (its saved flags), on this machine?
/// Where a provider is named, the provider its saved `--settings` env
/// selects (every `--settings` merged in order, the later winning) must be
/// that one; else (a job from before 0.7.16, or from before `machine set`)
/// it is relaunched instead, through the preflight and with the selector
/// forced. Optional machines: yes.
pub fn wake_as_saved(env: &Env, job: &str) -> bool {
    let Ok(Some(want)) = expected(env) else { return true };
    let Some(d) = std::fs::read(env.claude_home.join("jobs").join(job).join("state.json")).ok().and_then(|b| crate::evidence::first_json(&b)) else { return false };
    let mut merged: BTreeMap<String, String> = BTreeMap::new();
    // the launcher's provider env, saved with the job (a wake uses it), under
    // every --settings env
    if let Some(pe) = d.get("providerEnv").and_then(Value::as_object) {
        for (k, v) in pe {
            if let Some(v) = v.as_str() {
                merged.insert(k.clone(), v.to_string());
            }
        }
    }
    let mut settings: Vec<String> = vec![];
    if let Some(x) = d.get("savedSettings").and_then(Value::as_str) {
        settings.push(x.to_string());
    }
    if let Some(Value::Array(a)) = d.get("respawnFlags") {
        let argv: Vec<&str> = a.iter().filter_map(|x| x.as_str()).collect();
        for w in argv.windows(2).filter(|w| w[0] == "--settings") {
            settings.push(w[1].to_string());
        }
    }
    for raw in settings {
        if let Some(env) = serde_json::from_str::<Value>(&raw).ok().and_then(|v| v.get("env").and_then(Value::as_object).cloned()) {
            for (k, v) in env {
                if let Some(v) = v.as_str() {
                    merged.insert(k, v.to_string());
                }
            }
        }
    }
    api_of(&want) == Some(from_selectors(|k| merged.get(k).cloned()))
}

// ------------------------------------------------------------------ the record

/// What a member's worker saw at its SessionStart (`NIMITZ_STATE/provider/<F.SQ.SHIP>.json`;
/// an unlabelled session's: `provider/sessions/<session id>.json`, for `adopt`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub session_id: String,
    pub at: f64,
    /// the hook's stdin `model`
    #[serde(default)]
    pub model: Option<String>,
    /// the selectors and `RECORDED` env set in the worker
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// the `apiProvider` the selectors give
    pub provider: String,
}

impl Record {
    /// Does the model agree with the provider (a model that says nothing does)?
    pub fn model_agrees(&self) -> bool {
        match self.model.as_deref().and_then(model_family) {
            None => true,
            Some(f) => f == self.provider || (f == "firstParty" && !matches!(self.provider.as_str(), "bedrock" | "vertex")),
        }
    }
}

fn records_dir(env: &Env) -> PathBuf {
    env.state.join("provider")
}

fn session_record_path(env: &Env, sid: &str) -> PathBuf {
    let safe: String = sid.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    records_dir(env).join("sessions").join(format!("{safe}.json"))
}

/// A record from a SessionStart: the provider the selectors give, the model
/// and the selectors; a member's (`id` not empty) keeps the `RECORDED` env
/// too (region, profile, base URL), a session that is no member's not.
fn record_from(id: &str, sid: &str, input: &Value, vars: &HashMap<String, String>) -> Record {
    let mut kept = BTreeMap::new();
    let extra: &[&str] = if id.is_empty() { &[] } else { &RECORDED };
    for k in SELECTORS.iter().map(|(k, _)| *k).chain(extra.iter().copied()) {
        if let Some(v) = vars.get(k).filter(|v| !v.is_empty()) {
            kept.insert(k.to_string(), v.clone());
        }
    }
    Record {
        id: id.to_string(),
        session_id: sid.to_string(),
        at: crate::evidence::now(),
        model: input.get("model").and_then(Value::as_str).map(str::to_string).filter(|m| !m.is_empty()),
        provider: from_selectors(|k| kept.get(k).cloned()).to_string(),
        env: kept,
    }
}

/// The SessionStart hook's record of session `sid`, member `id`'s: its model
/// and provider env, the provider they select. Every record is per session
/// (a copy row of the member, a /fork, writes its own: none clobbers the
/// member's live one). Never fails the hook.
pub fn record(env: &Env, id: &Id, sid: &str, input: &Value, vars: &HashMap<String, String>) {
    if !sid.is_empty() {
        let _ = crate::config::atomic_write(&session_record_path(env, sid), &record_text(&record_from(&id.0, sid, input, vars)));
    }
}

/// The same for a session that is no member's (a stowaway `adopt` may bind).
pub fn record_session(env: &Env, sid: &str, input: &Value, vars: &HashMap<String, String>) {
    if !sid.is_empty() {
        let _ = crate::config::atomic_write(&session_record_path(env, sid), &record_text(&record_from("", sid, input, vars)));
    }
}

/// Session `sid`'s record (a member's or not).
pub fn session_record(env: &Env, sid: &str) -> Option<Record> {
    serde_json::from_slice(&std::fs::read(session_record_path(env, sid)).ok()?).ok()
}

/// Member `id`'s newest record, of any of its sessions.
pub fn record_of(env: &Env, id: &Id) -> Option<Record> {
    records(env).into_iter().filter(|r| r.id == id.0).max_by(|a, b| a.at.total_cmp(&b.at))
}

/// Every record on this machine.
pub fn records(env: &Env) -> Vec<Record> {
    let Ok(rd) = std::fs::read_dir(records_dir(env).join("sessions")) else { return vec![] };
    rd.flatten().filter(|e| e.path().is_file()).filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()).collect()
}

/// Has a member on this machine been recorded on `api`?
pub fn recorded_on(env: &Env, api: &str) -> bool {
    records(env).iter().any(|r| !r.id.is_empty() && r.provider == api)
}

/// When the process of live session `s` started: its session file's
/// `startedAt` (`claude_home/sessions/<pid>.json`), else the process's own
/// start. A bg job's `started_at` is its job's creation, which a wake keeps,
/// so it says nothing about this process. None when neither can be read.
pub fn live_started(env: &Env, s: &SessionInfo) -> Option<f64> {
    process_started(env, &s.session_id, s.pid?)
}

/// `live_started` by session id and pid.
pub fn process_started(env: &Env, session_id: &str, pid: i64) -> Option<f64> {
    let from_file = std::fs::read(env.claude_home.join("sessions").join(format!("{pid}.json")))
        .ok()
        .and_then(|b| crate::evidence::first_json(&b))
        .filter(|v| v.get("sessionId").and_then(Value::as_str).is_none_or(|x| x == session_id))
        .and_then(|v| v.get("startedAt").and_then(Value::as_f64))
        .map(|ms| ms / 1000.0);
    from_file.or_else(|| crate::os::process_birth_ms(pid).map(|ms| ms as f64 / 1000.0))
}

/// Does this machine gate starts and deliveries by provider? Where it names
/// one, or its settings cannot be read (then everything refuses).
pub fn gated(env: &Env) -> bool {
    !matches!(expected(env), Ok(None))
}

/// May stowaway session `sid` be adopted on this machine? Where a provider is
/// named, a record that shows another provider refuses; no record is a
/// warning (bound, unverified until its next restart: no delivery reaches it
/// meanwhile); on an optional machine a missing record is a warning too.
pub fn check_adopt(env: &Env, id: &Id, sid: &str) -> Result<Option<Issue>, Issue> {
    let want = match expected(env) {
        Ok(w) => w,
        Err(e) => return Err(unreadable(Some(id), &e)),
    };
    let rec = session_record(env, sid);
    let fresh = format!("stop it ({sid}), then nimitz start {id}  # a cold launch, checked first");
    match (want, rec) {
        (Some(w), Some(r)) if api_of(&w) == Some(r.provider.as_str()) => Ok(None),
        (Some(w), Some(r)) => Err(Issue {
            code: "provider.mismatch".into(),
            severity: Severity::Error,
            message: format!("session {sid} runs on {} but this machine expects {w}; it is not adopted", name_of(&r.provider)),
            fix: Some(fresh),
            id: Some(id.clone()),
        }),
        // bound, unverified: no delivery reaches it (`delivery_gate`), and its
        // next restart goes through the preflight or the wake gate
        (Some(w), None) => Ok(Some(Issue {
            code: "provider.unrecorded".into(),
            severity: Severity::Warn,
            message: format!("session {sid} has no provider record (its SessionStart ran before nimitz 0.7.16, or without nimitz's hooks), so it is not shown to run on {w}: {id} is unverified until its next restart, and nimitz delivers nothing to it until then"),
            fix: Some(format!("nimitz restart {id}  # when it suits: relaunched with every flag, checked first, same conversation")),
            id: Some(id.clone()),
        })),
        (None, None) => Ok(Some(Issue {
            code: "provider.unrecorded".into(),
            severity: Severity::Warn,
            message: format!("session {sid} has no provider record; adopted unchecked (this machine names no provider)"),
            fix: None,
            id: Some(id.clone()),
        })),
        (None, Some(_)) => Ok(None),
    }
}

/// May nimitz deliver to member `id` (a notice, a signal: its inbox or its
/// pane) while it runs `live`? Where a provider is named, only once the
/// SessionStart record of that session (since its process started) shows
/// that provider: a woken job is not reached before its gate has passed
/// (#12). Optional machines: yes.
pub fn delivery_gate(env: &Env, id: &Id, live: Option<&SessionInfo>) -> Result<(), Issue> {
    let want = match expected(env) {
        Ok(Some(w)) => w,
        Ok(None) => return Ok(()),
        Err(e) => return Err(unreadable(Some(id), &e)),
    };
    let Some(s) = live else { return Ok(()) };
    // since THIS process started (a wake keeps the session id, and the job's
    // creation time): unknown, nothing is delivered
    let Some(started) = live_started(env, s) else {
        return Err(Issue {
            code: "provider.unrecorded".into(),
            severity: Severity::Error,
            message: format!("{id}'s session {} has no readable start (no session file, no process start), so its provider record cannot be tied to it: nothing is delivered to it", s.session_id),
            fix: Some(format!("nimitz status {id}; then nimitz restart {id}")),
            id: Some(id.clone()),
        });
    };
    // (a session file's start is in ms precision; the hook's record later)
    let rec = session_record(env, &s.session_id).filter(|r| r.at >= started - 1.0);
    match rec {
        Some(r) if api_of(&want) == Some(r.provider.as_str()) => Ok(()),
        Some(r) => Err(Issue {
            code: "provider.mismatch".into(),
            severity: Severity::Error,
            message: format!("{id} runs on {} but this machine expects {want}: nothing is delivered to it", name_of(&r.provider)),
            fix: Some(format!("nimitz stop {id}; then nimitz start {id}  # checked first")),
            id: Some(id.clone()),
        }),
        None => Err(Issue {
            code: "provider.unrecorded".into(),
            severity: Severity::Error,
            message: format!("{id}'s session {} has no provider record since it started, so it is not shown to run on {want}: nothing is delivered to it yet", s.session_id),
            fix: Some(format!("in a moment (its SessionStart may still run); else nimitz doctor, then nimitz restart {id}")),
            id: Some(id.clone()),
        }),
    }
}

/// May nimitz post into session `sid` (process `pid`, no member's: `signal
/// session:`)? Where a provider is named, only once that session's record
/// since its process started shows that provider, as `delivery_gate` for a
/// member. Optional machines: yes.
pub fn session_gate(env: &Env, sid: &str, pid: i64) -> Result<(), Issue> {
    let want = match expected(env) {
        Ok(Some(w)) => w,
        Ok(None) => return Ok(()),
        Err(e) => return Err(unreadable(None, &e)),
    };
    let refuse = |code: &str, message: String| Issue { code: code.into(), severity: Severity::Error, message, fix: Some(format!("nimitz doctor; the session's own `claude auth status` (this machine expects {want})")), id: None };
    let Some(started) = process_started(env, sid, pid) else {
        return Err(refuse("provider.unrecorded", format!("session {sid} has no readable start, so its provider record cannot be tied to it: nothing is posted")));
    };
    match session_record(env, sid).filter(|r| r.at >= started - 1.0) {
        Some(r) if api_of(&want) == Some(r.provider.as_str()) => Ok(()),
        Some(r) => Err(refuse("provider.mismatch", format!("session {sid} runs on {} but this machine expects {want}: nothing is posted", name_of(&r.provider)))),
        None => Err(refuse("provider.unrecorded", format!("session {sid} has no provider record since it started (no nimitz hook ran there), so it is not shown to run on {want}: nothing is posted"))),
    }
}

/// The gate again just before a delivery goes out (`signal`): to the
/// `planned` session, which must still be member `id`'s live one (`now`),
/// on the process `pid` it goes to, shown on record on this machine's
/// provider. Where nothing is checked: yes.
pub fn recheck(env: &Env, id: &Id, planned: Option<&SessionInfo>, now: Option<&SessionInfo>, pid: Option<i64>) -> Result<(), Issue> {
    if !gated(env) {
        return Ok(());
    }
    let refuse = |message: String, fix: Option<String>| Issue { code: "provider.unrecorded".into(), severity: Severity::Error, message, fix, id: Some(id.clone()) };
    let Some(planned) = planned else { return Err(refuse(format!("{id} had no session at the plan; nothing is delivered"), None)) };
    if now.map(|s| (s.session_id.as_str(), s.pid)) != Some((planned.session_id.as_str(), planned.pid)) {
        return Err(refuse(
            format!("{id}'s session changed since the plan (it was {}); nothing is delivered to a session the plan did not check", planned.session_id),
            Some(format!("nimitz signal {id} <text>  # again")),
        ));
    }
    let Some(pid) = pid else { return Err(refuse(format!("{id}'s session {} has no process to tie its record to; nothing is delivered", planned.session_id), None)) };
    session_gate(env, &planned.session_id, pid).map_err(|i| Issue { id: Some(id.clone()), ..i })
}

/// Drop the records of sessions that no row has any more (`sweep`): the
/// session ids given are the ones still there, read at `read_at`; a record
/// written since (a session that started meanwhile) stays. The count dropped.
pub fn prune_sessions(env: &Env, still: &std::collections::HashSet<String>, read_at: f64) -> usize {
    let Ok(rd) = std::fs::read_dir(records_dir(env).join("sessions")) else { return 0 };
    let mut n = 0;
    for e in rd.flatten() {
        let Some(r) = serde_json::from_slice::<Record>(&std::fs::read(e.path()).unwrap_or_default()).ok() else { continue };
        if !still.contains(&r.session_id) && r.at < read_at && std::fs::remove_file(e.path()).is_ok() {
            n += 1;
        }
    }
    n
}

// ------------------------------------------------------------------ check

/// `check`/`status`: each live member of this machine (its id, the session
/// it runs) whose latest record, of that session, differs from the provider
/// the machine expects (or, with none set, from the one most of its members
/// run on), or whose model disagrees with it.
pub fn issues(env: &Env, live: &[(Id, String)]) -> Vec<Issue> {
    let recs: Vec<Record> = live.iter().filter_map(|(id, sid)| session_record(env, sid).map(|r| Record { id: id.0.clone(), ..r })).collect();
    let expected = match expected(env) {
        Ok(e) => e.and_then(|e| api_of(&e)).map(str::to_string),
        // nothing is compared against settings that cannot be read; every
        // launch and wake refuses meanwhile (`provider.unreadable`)
        Err(e) => return vec![unreadable(None, &e)],
    };
    let majority = || {
        let mut n: BTreeMap<&str, usize> = BTreeMap::new();
        for r in &recs {
            *n.entry(r.provider.as_str()).or_default() += 1;
        }
        let max = n.values().copied().max().unwrap_or(0);
        let top: Vec<&str> = n.iter().filter(|(_, c)| **c == max).map(|(p, _)| *p).collect();
        (top.len() == 1 && recs.len() > 1).then(|| top[0].to_string())
    };
    let (want, how) = match &expected {
        Some(e) => (Some(e.clone()), "the machine expects"),
        None => (majority(), "most members here run on"),
    };
    let label = this_machine(env).unwrap_or_default();
    let mut out = vec![];
    for r in &recs {
        let id = Id(r.id.clone());
        let fresh = format!("nimitz stop {id}, then nimitz start {id} --fresh  # drops its conversation: ask the user first");
        if let Some(w) = want.as_deref().filter(|w| *w != r.provider) {
            out.push(Issue {
                code: "provider.mismatch".into(),
                severity: Severity::Warn,
                message: format!("{id} runs on {} ({}), but {how} {} (machine {label})", name_of(&r.provider), r.model.as_deref().unwrap_or("model unknown"), name_of(w)),
                fix: Some(fresh),
                id: Some(id),
            });
        } else if !r.model_agrees() {
            out.push(Issue {
                code: "provider.model".into(),
                severity: Severity::Warn,
                message: format!("{id}'s model {} is not a {} model", r.model.as_deref().unwrap_or_default(), name_of(&r.provider)),
                fix: Some(format!("nimitz check {id}  # a custom ANTHROPIC_MODEL or alias is fine; anything else: the folder's and managed settings")),
                id: Some(id),
            });
        }
    }
    out
}

// ------------------------------------------------------------------ doctor

/// The `claude` binaries `doctor` lists (never writes): what PATH finds, the
/// native installer's launcher, the configured binaries, the native
/// installer's versions (oldest first), an npm global install.
pub fn claude_bin_candidates(env: &Env) -> Vec<String> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    candidates_in(env, &home)
}

fn candidates_in(env: &Env, home: &std::path::Path) -> Vec<String> {
    let mut c: Vec<String> = vec![];
    let mut add = |p: Option<String>| {
        if let Some(p) = p.filter(|p| std::path::Path::new(p).is_file() && !c.contains(p)) {
            c.push(p);
        }
    };
    add(crate::env::which("claude"));
    add(Some(home.join(".local/bin").join(crate::os::exe_name("claude")).to_string_lossy().into_owned()));
    add(Some(env.native_claude_bin.clone()));
    add(Some(env.claude_bin.clone()));
    // a version is a file, or a folder that holds the binary
    let mut versions: Vec<PathBuf> =
        std::fs::read_dir(home.join(".local/share/claude/versions")).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    // by number, each part's digits: 2.1.99 < 2.1.288 < 2.1.288-beta
    let number = |v: &PathBuf| -> Vec<u64> {
        let name = v.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        name.split('.').filter_map(|x| x.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()).collect()
    };
    versions.sort_by_cached_key(|v| (number(v), v.clone()));
    for v in versions {
        add(Some(v.join(crate::os::exe_name("claude")).to_string_lossy().into_owned()));
        add(Some(v.to_string_lossy().into_owned()));
    }
    for dir in [home.join(".npm-global/bin"), PathBuf::from("/usr/local/bin")] {
        add(Some(dir.join(crate::os::exe_name("claude")).to_string_lossy().into_owned()));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        add(crate::env::npm_target(&PathBuf::from(appdata).join("npm/claude.cmd")).map(|p| p.to_string_lossy().into_owned()));
    }
    c
}

/// Where `env.claude_bin` came from: `NIMITZ_CLAUDE_BIN`, the machines
/// file, or PATH.
pub fn claude_bin_source(env: &Env, vars: &HashMap<String, String>) -> &'static str {
    if vars.get("NIMITZ_CLAUDE_BIN").is_some_and(|v| !v.is_empty()) {
        "NIMITZ_CLAUDE_BIN"
    } else if claude_bin_for(&env.home, &env.state, &env.hostname).is_some() {
        "machines.toml"
    } else {
        "PATH"
    }
}

/// A record as the hook writes it (the shape pinned by the formats test).
pub fn record_text(r: &Record) -> String {
    serde_json::to_string_pretty(r).unwrap_or_default()
}

/// `json!` of a record, for `status --json`.
pub fn record_json(r: &Record) -> Value {
    json!({"provider": name_of(&r.provider), "model": r.model, "session_id": r.session_id, "at": r.at})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_and_models() {
        let env = |pairs: &[(&str, &str)]| {
            let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            from_selectors(|k| m.get(k).cloned())
        };
        assert_eq!(env(&[]), "firstParty");
        assert_eq!(env(&[("CLAUDE_CODE_USE_BEDROCK", "1")]), "bedrock");
        assert_eq!(env(&[("CLAUDE_CODE_USE_BEDROCK", "0")]), "firstParty");
        assert_eq!(env(&[("CLAUDE_CODE_USE_VERTEX", "true"), ("CLAUDE_CODE_USE_BEDROCK", "1")]), "bedrock", "Bedrock is read first");
        assert_eq!(env(&[("CLAUDE_CODE_USE_GATEWAY", "yes")]), "gateway");
        assert_eq!(model_family("us.anthropic.claude-opus-5-5"), Some("bedrock"));
        assert_eq!(model_family("arn:aws:bedrock:us-east-1:1:inference-profile/x"), Some("bedrock"));
        assert_eq!(model_family("claude-opus-5-5"), Some("firstParty"));
        assert_eq!(model_family("claude-opus-5-5@20260101"), Some("vertex"));
        assert_eq!(model_family("opus"), None);
        assert_eq!((api_of("anthropic"), name_of("firstParty"), name_of("gateway")), (Some("firstParty"), "anthropic".into(), "gateway".into()));
        let r = |p: &str, m: &str| Record { provider: p.into(), model: Some(m.into()), ..Default::default() };
        assert!(r("bedrock", "us.anthropic.claude-opus-5-5").model_agrees());
        assert!(!r("firstParty", "us.anthropic.claude-opus-5-5").model_agrees());
        assert!(!r("bedrock", "claude-opus-5-5").model_agrees());
        assert!(r("gateway", "claude-opus-5-5").model_agrees());
    }

    #[test]
    fn machine_set_writes_clears_and_refuses() {
        let td = tempfile::tempdir().unwrap();
        let vars: HashMap<String, String> = [("HOME", "h"), ("NIMITZ_HOME", "n"), ("NIMITZ_STATE", "s"), ("NIMITZ_CLAUDE_HOME", "c"), ("NIMITZ_HOSTNAME", "box.local"), ("NIMITZ_CLAUDE_BIN", "/x/claude"), ("NIMITZ_NATIVE_CLAUDE_BIN", "/x/claude")]
            .iter()
            .map(|(k, v)| (k.to_string(), td.path().join(v).to_string_lossy().into_owned()))
            .collect();
        let mut vars = vars;
        vars.insert("NIMITZ_HOSTNAME".into(), "box.local".into());
        let env = Env::from_vars(&vars);
        // with no registry yet: the host name, as a fresh registry records it
        let me = this_machine(&env).unwrap();
        assert_eq!(me, "box", "the host name's label: up to its first dot");
        let apply = |p: &Plan| assert!(crate::config::apply_config_plan(&env, p).verified, "{p:?}");
        assert!(!plan_machine_set(&env, &me, None, None, Some("openai")).ok());
        assert!(!plan_machine_set(&env, &me, None, None, None).ok());
        assert!(!plan_machine_set(&env, &me, Some("claude"), None, None).ok(), "relative");
        assert!(!plan_machine_set(&env, &me, Some("/nowhere/claude"), None, None).ok(), "this machine's must exist");
        apply(&plan_machine_set(&env, "work", Some("/nowhere/claude"), None, Some("bedrock")));
        apply(&plan_machine_set(&env, &me, None, None, Some("anthropic")));
        assert_eq!(expected(&env).unwrap().as_deref(), Some("anthropic"));
        let m = load_machines(&env.home).unwrap();
        assert_eq!(m.machines["work"], MachineSettings { claude_bin: Some("/nowhere/claude".into()), provider: Some("bedrock".into()), ..Default::default() });
        // one label for every reader: a registry naming the machine `work`
        std::fs::create_dir_all(&env.state).unwrap();
        std::fs::write(env.state.join("registry.json"), json!({"version": 3, "machine": "work"}).to_string()).unwrap();
        assert_eq!(this_machine(&env).unwrap(), "work");
        assert_eq!(claude_bin_for(&env.home, &env.state, "box.local"), Some("/nowhere/claude".into()));
        // both binaries: the machine's, with no variable saying otherwise
        let mut bare = vars.clone();
        bare.remove("NIMITZ_CLAUDE_BIN");
        bare.remove("NIMITZ_NATIVE_CLAUDE_BIN");
        let e = Env::from_vars(&bare);
        assert_eq!((e.claude_bin.as_str(), e.native_claude_bin.as_str()), ("/nowhere/claude", "/nowhere/claude"));
        assert_eq!(expected(&env).unwrap().as_deref(), Some("bedrock"));
        std::fs::remove_file(env.state.join("registry.json")).unwrap();
        // a label nobody knows: a warning, not a refusal
        let p = plan_machine_set(&env, "elsewhere", None, None, Some("vertex"));
        assert!(p.ok() && p.warnings.iter().any(|w| w.code == "machine.unknown" && w.message.contains("known: box")), "{p:?}");
        assert!(plan_machine_set(&env, &me, None, None, Some("anthropic")).steps.is_empty(), "nothing changes");
        apply(&plan_machine_set(&env, &me, None, None, Some("")));
        assert!(!load_machines(&env.home).unwrap().machines.contains_key(&me), "an empty entry goes");
        // format 1 needs no min_nimitz (older nimitz ignore the file); one a
        // newer nimitz may set is kept on every rewrite
        assert!(load_machines(&env.home).unwrap().min_nimitz.is_none());
        let text = std::fs::read_to_string(machines_path(&env.home)).unwrap();
        std::fs::write(machines_path(&env.home), format!("min_nimitz = \"0.7.15\"\n{text}")).unwrap();
        apply(&plan_machine_set(&env, &me, None, None, Some("vertex")));
        assert_eq!(load_machines(&env.home).unwrap().min_nimitz.as_deref(), Some("0.7.15"));
    }

    #[test]
    fn issues_flag_the_odd_member_out() {
        let td = tempfile::tempdir().unwrap();
        let vars: HashMap<String, String> = [("HOME", "h"), ("NIMITZ_HOME", "n"), ("NIMITZ_STATE", "s"), ("NIMITZ_CLAUDE_HOME", "c")]
            .iter()
            .map(|(k, v)| (k.to_string(), td.path().join(v).to_string_lossy().into_owned()))
            .chain([("NIMITZ_HOSTNAME".to_string(), "box".to_string()), ("NIMITZ_CLAUDE_BIN".into(), "/x".into()), ("NIMITZ_NATIVE_CLAUDE_BIN".into(), "/x".into())])
            .collect();
        let env = Env::from_vars(&vars);
        let ids = [Id::admiral("A"), Id::commodore("A", "S"), Id::ship("A", "S", "T")];
        // each member's session is its own (`sid-<n>`)
        let sid = |id: &Id| format!("sid-{}", ids.iter().position(|x| x == id).unwrap());
        let rec = |id: &Id, bedrock: bool, model: &str| {
            let v: HashMap<String, String> = if bedrock { [("CLAUDE_CODE_USE_BEDROCK".to_string(), "1".to_string())].into() } else { HashMap::new() };
            record(&env, id, &sid(id), &json!({"model": model}), &v);
        };
        rec(&ids[0], true, "us.anthropic.claude-opus-5-5");
        rec(&ids[1], true, "us.anthropic.claude-opus-5-5");
        rec(&ids[2], false, "claude-opus-5-5");
        assert_eq!(record_of(&env, &ids[2]).unwrap().provider, "firstParty");
        assert!(recorded_on(&env, "bedrock"));
        // no expected provider: the majority (Bedrock) flags the odd one
        let live: Vec<(Id, String)> = ids.iter().map(|i| (i.clone(), sid(i))).collect();
        let i = issues(&env, &live);
        assert_eq!(i.len(), 1, "{i:?}");
        assert!(i[0].code == "provider.mismatch" && i[0].id.as_ref() == Some(&ids[2]) && i[0].fix.as_deref().unwrap().contains("--fresh"), "{i:?}");
        // expected anthropic: the two Bedrock ones
        crate::config::apply_config_plan(&env, &plan_machine_set(&env, &this_machine(&env).unwrap(), None, None, Some("anthropic")));
        let i = issues(&env, &live);
        assert_eq!(i.iter().filter_map(|i| i.id.clone()).collect::<Vec<_>>(), ids[..2].to_vec());
        // a model that disagrees with its selectors
        rec(&ids[2], false, "us.anthropic.claude-opus-5-5");
        assert!(issues(&env, &live[2..]).iter().any(|i| i.code == "provider.model"));
        // a record of another session (an old one, another home's) does not count
        assert!(issues(&env, &[(ids[2].clone(), "other".into())]).is_empty());
    }

    fn temp_env(td: &std::path::Path) -> Env {
        let vars: HashMap<String, String> = [("HOME", "h"), ("NIMITZ_HOME", "n"), ("NIMITZ_STATE", "s"), ("NIMITZ_CLAUDE_HOME", "c")]
            .iter()
            .map(|(k, v)| (k.to_string(), td.join(v).to_string_lossy().into_owned()))
            .chain([("NIMITZ_HOSTNAME".to_string(), "box".to_string()), ("NIMITZ_CLAUDE_BIN".into(), "/x".into()), ("NIMITZ_NATIVE_CLAUDE_BIN".into(), "/x".into())])
            .collect();
        Env::from_vars(&vars)
    }

    /// Review B1/S7: a machines file or a registry that cannot be read is
    /// never "no settings": every launch refuses (`provider.unreadable`).
    #[test]
    fn unreadable_or_newer_settings_refuse_everything() {
        let td = tempfile::tempdir().unwrap();
        let env = temp_env(td.path());
        std::fs::create_dir_all(&env.home).unwrap();
        let launch = || check_launch(&env, None, &["/nowhere/claude".to_string()], None);
        for bad in ["[machines.box]\nprovider = \"bedrock\"\nsurprise = 1\n", "format = 2\n[machines.box]\nprovider = \"bedrock\"\n", "min_nimitz = \"9.9.9\"\n[machines.box]\n", "not toml ["] {
            std::fs::write(machines_path(&env.home), bad).unwrap();
            assert!(expected(&env).is_err(), "{bad}");
            assert_eq!(launch().unwrap_err().code, "provider.unreadable", "{bad}");
            assert!(forced_selector(&env).is_none());
            assert_eq!(issues(&env, &[])[0].code, "provider.unreadable");
        }
        // a min_nimitz this nimitz meets reads
        std::fs::write(machines_path(&env.home), "min_nimitz = \"0.7.15\"\n[machines.box]\nprovider = \"bedrock\"\n").unwrap();
        assert_eq!(expected(&env).unwrap().as_deref(), Some("bedrock"));
        // a registry a newer nimitz wrote: no label at all, not another one
        std::fs::create_dir_all(&env.state).unwrap();
        std::fs::write(env.state.join("registry.json"), json!({"version": 99, "machine": "box"}).to_string()).unwrap();
        assert!(this_machine(&env).is_err());
        assert_eq!(launch().unwrap_err().code, "provider.unreadable");
        assert_eq!(claude_bin_for(&env.home, &env.state, "box"), None);
    }

    /// Review S3, B2, S5: a start's record counts only from its own start; a
    /// wake with no record or the wrong one is refused; a saved launch
    /// without the expected selector is not woken as it is; adopt needs the
    /// session's own record where a provider is named.
    #[test]
    fn records_of_this_start_wakes_and_adopt() {
        let td = tempfile::tempdir().unwrap();
        let env = temp_env(td.path());
        let id = Id::ship("A", "S", "T");
        let bedrock: HashMap<String, String> = [("CLAUDE_CODE_USE_BEDROCK".to_string(), "1".to_string())].into();
        // optional machine: nothing to compare
        assert!(check_started(&env, &id, "sid", 0.0, true).unwrap().is_none());
        crate::config::apply_config_plan(&env, &plan_machine_set(&env, "box", None, None, Some("bedrock")));
        record(&env, &id, "sid", &json!({"model": "us.anthropic.claude-opus-5-5"}), &bedrock);
        let at = record_of(&env, &id).unwrap().at;
        assert!(check_started(&env, &id, "sid", at - 1.0, false).unwrap().is_none());
        // a relaunch keeps the session id: the old record is not this start's
        let later = at + 1.0;
        assert_eq!(check_started(&env, &id, "sid", later, false).unwrap().unwrap().code, "provider.unrecorded", "after a launch: a warning");
        let e = check_started(&env, &id, "sid", later, true).unwrap_err();
        assert!(e.code == "provider.unrecorded" && e.message.contains("nothing was sent"), "after a wake: refused: {e:?}");
        record(&env, &id, "sid", &json!({}), &HashMap::new());
        let e = check_started(&env, &id, "sid", at, true).unwrap_err();
        assert!(e.code == "provider.mismatch" && e.message.contains("row and conversation are kept"), "{e:?}");
        // the saved launch flags of a job, for a wake
        let job = |flags: Value| {
            let d = env.claude_home.join("jobs").join("j1");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("state.json"), json!({"respawnFlags": flags}).to_string()).unwrap();
        };
        job(json!(["--settings", r#"{"env":{"NIMITZ_ID":"A/S/T"}}"#]));
        assert!(!wake_as_saved(&env, "j1"), "a pre-0.7.16 job: relaunched instead");
        job(json!(["--settings", r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}"#, "--settings", r#"{"env":{"NIMITZ_ID":"A/S/T","CLAUDE_CODE_USE_BEDROCK":"1"}}"#]));
        assert!(wake_as_saved(&env, "j1"));
        job(json!(["--settings", r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}"#, "--settings", r#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"0"}}"#]));
        assert!(!wake_as_saved(&env, "j1"), "the later --settings wins");
        assert!(!wake_as_saved(&env, "nojob"));
        // adopt: the stowaway's own record
        let w = check_adopt(&env, &id, "stow").unwrap().unwrap();
        assert!(w.code == "provider.unrecorded" && w.severity == Severity::Warn && w.message.contains("unverified until its next restart"), "{w:?}");
        record_session(&env, "stow", &json!({}), &HashMap::new());
        assert_eq!(check_adopt(&env, &id, "stow").unwrap_err().code, "provider.mismatch");
        record_session(&env, "stow", &json!({}), &bedrock);
        assert!(check_adopt(&env, &id, "stow").unwrap().is_none());
        // a session that is no member's: no member on record on it, no env kept but the selectors
        let stow = session_record(&env, "stow").unwrap();
        assert!(stow.id.is_empty() && stow.env.keys().all(|k| k.starts_with("CLAUDE_CODE_USE_")), "{stow:?}");
        // optional machine: a missing record only warns
        crate::config::apply_config_plan(&env, &plan_machine_set(&env, "box", None, None, Some("")));
        assert_eq!(check_adopt(&env, &id, "other").unwrap().unwrap().severity, Severity::Warn);
        assert!(wake_as_saved(&env, "nojob"));
        // providerEnv (the launcher's, saved with the job) counts for a wake, under --settings
        crate::config::apply_config_plan(&env, &plan_machine_set(&env, "box", None, None, Some("bedrock")));
        let d = env.claude_home.join("jobs").join("j2");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("state.json"), json!({"providerEnv": {"CLAUDE_CODE_USE_BEDROCK": "1"}, "respawnFlags": ["--settings", r#"{"env":{"NIMITZ_ID":"A/S/T"}}"#]}).to_string()).unwrap();
        assert!(wake_as_saved(&env, "j2"));
    }

    /// Review BB: the delivery gate ties a record to THIS process of the
    /// session (its session file's startedAt): a wake keeps the session id and
    /// the job's creation time, so the previous start's record does not count;
    /// no start at all refuses.
    #[test]
    fn the_delivery_gate_takes_the_records_of_this_process_only() {
        let td = tempfile::tempdir().unwrap();
        let env = temp_env(td.path());
        let id = Id::ship("A", "S", "T");
        crate::config::apply_config_plan(&env, &plan_machine_set(&env, "box", None, None, Some("bedrock")));
        let bedrock: HashMap<String, String> = [("CLAUDE_CODE_USE_BEDROCK".to_string(), "1".to_string())].into();
        record(&env, &id, "sid", &json!({}), &bedrock);
        let at = session_record(&env, "sid").unwrap().at;
        let s = |pid: Option<i64>| SessionInfo {
            session_id: "sid".into(), short_id: Some("j".into()), pid, kind: "background".into(), state: None, status: None, waiting_for: None,
            // the job's creation (a wake keeps it): long before
            started_at: Some(at - 3600.0), status_since: None, name: String::new(), cwd: String::new(), labels: Default::default(), birth_id: None, entrypoint: None, pinned: false, copy: None, terminal: None,
        };
        let sessions = env.claude_home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        // this process started before the record: delivered
        std::fs::write(sessions.join("777001.json"), json!({"sessionId": "sid", "startedAt": (at - 10.0) * 1000.0}).to_string()).unwrap();
        assert!(delivery_gate(&env, &id, Some(&s(Some(777001)))).is_ok());
        // woken since: the record is the previous start's
        std::fs::write(sessions.join("777001.json"), json!({"sessionId": "sid", "startedAt": (at + 10.0) * 1000.0}).to_string()).unwrap();
        assert_eq!(delivery_gate(&env, &id, Some(&s(Some(777001)))).unwrap_err().code, "provider.unrecorded");
        // no pid, so no start: refused, never "since 1970"
        assert_eq!(delivery_gate(&env, &id, Some(&s(None))).unwrap_err().code, "provider.unrecorded");
    }

    /// Review 0.7.16: an optional machine never needs the registry (a bad one
    /// refuses nothing there), and the launch claude is not looked up with no
    /// machines file; a row with no job wakes as saved only where nothing is
    /// checked; sweep keeps records newer than its read of the rows.
    #[test]
    fn optional_machines_skip_the_registry_and_sweep_keeps_new_records() {
        let td = tempfile::tempdir().unwrap();
        let env = temp_env(td.path());
        std::fs::create_dir_all(&env.state).unwrap();
        std::fs::write(env.state.join("registry.json"), json!({"version": 99}).to_string()).unwrap();
        assert_eq!(expected(&env), Ok(None), "no machines file: no provider, no label needed");
        assert!(check_launch(&env, None, &["/nowhere/claude".to_string()], None).is_ok(), "an optional machine: a warning at most");
        assert_eq!(claude_bin_for(&env.home, &env.state, "box"), None);
        assert!(wakes_as_saved(&env, None));
        std::fs::remove_file(env.state.join("registry.json")).unwrap();
        crate::config::apply_config_plan(&env, &plan_machine_set(&env, &this_machine(&env).unwrap(), None, None, Some("bedrock")));
        assert!(!wakes_as_saved(&env, None), "no job, no saved flags: relaunched where it is checked");
        // prune: gone sessions only, and only records older than the read
        record_session(&env, "old", &json!({}), &HashMap::new());
        let read_at = crate::evidence::now();
        std::thread::sleep(std::time::Duration::from_millis(20));
        record_session(&env, "new", &json!({}), &HashMap::new());
        record_session(&env, "kept", &json!({}), &HashMap::new());
        let still: std::collections::HashSet<String> = ["kept".to_string()].into();
        assert_eq!(prune_sessions(&env, &still, read_at), 1);
        assert!(session_record(&env, "old").is_none() && session_record(&env, "new").is_some() && session_record(&env, "kept").is_some());
        // the one label rule: a host name with a dot and odd characters
        assert_eq!(host_label("Box Two.local"), "box-two");
        assert_eq!(host_label("con"), "this-machine");
    }

    /// 0.7.16 last review: `signal session:` and the re-check before a
    /// delivery tie a record to the process it goes to (a stale record, a
    /// start that cannot be read: refused); a changed session is refused;
    /// a machines file that does not list this machine still needs a
    /// readable registry.
    #[test]
    fn the_session_gate_and_the_recheck_before_posting() {
        let td = tempfile::tempdir().unwrap();
        let env = temp_env(td.path());
        crate::config::apply_config_plan(&env, &plan_machine_set(&env, "box", None, None, Some("bedrock")));
        let bedrock: HashMap<String, String> = [("CLAUDE_CODE_USE_BEDROCK".to_string(), "1".to_string())].into();
        let me = std::process::id() as i64;
        record_session(&env, "s1", &json!({}), &bedrock);
        // this process started before the record: posted
        assert!(session_gate(&env, "s1", me).is_ok());
        // a stale record (from before this process started)
        let stale = Record { at: 1.0, ..session_record(&env, "s1").unwrap() };
        crate::config::atomic_write(&session_record_path(&env, "s1"), &record_text(&stale)).unwrap();
        assert_eq!(session_gate(&env, "s1", me).unwrap_err().code, "provider.unrecorded");
        // a process whose start cannot be read (gone)
        record_session(&env, "s1", &json!({}), &bedrock);
        assert!(session_gate(&env, "s1", 2147483632).unwrap_err().message.contains("no readable start"));
        // the re-check: the planned session, still the live one, on its process
        let id = Id::ship("A", "S", "T");
        let s = |sid: &str| SessionInfo {
            session_id: sid.into(), short_id: Some("j".into()), pid: Some(me), kind: "background".into(), state: None, status: None, waiting_for: None,
            started_at: None, status_since: None, name: String::new(), cwd: String::new(), labels: Default::default(), birth_id: None, entrypoint: None, pinned: false, copy: None, terminal: None,
        };
        assert!(recheck(&env, &id, Some(&s("s1")), Some(&s("s1")), Some(me)).is_ok());
        let e = recheck(&env, &id, Some(&s("s1")), Some(&s("s2")), Some(me)).unwrap_err();
        assert!(e.message.contains("changed since the plan"), "{e:?}");
        assert!(recheck(&env, &id, Some(&s("s1")), None, Some(me)).is_err(), "gone since the plan");
        // a machines file that does not list this machine, an unreadable registry: refused
        crate::config::apply_config_plan(&env, &plan_machine_set(&env, "box", None, None, Some("")));
        crate::config::apply_config_plan(&env, &plan_machine_set(&env, "other", None, None, Some("bedrock")));
        std::fs::create_dir_all(&env.state).unwrap();
        std::fs::write(env.state.join("registry.json"), "not json").unwrap();
        assert!(expected(&env).is_err());
        assert_eq!(check_launch(&env, None, &["/nowhere/claude".to_string()], None).unwrap_err().code, "provider.unreadable");
    }

    /// `--native-bin`: the rules of `--claude-bin`, its own place in the
    /// order, and a file an older nimitz says it is too old for.
    #[test]
    fn native_bin_sets_clears_and_comes_before_claude_bin() {
        let td = tempfile::tempdir().unwrap();
        let mut vars: HashMap<String, String> =
            ["HOME", "NIMITZ_HOME", "NIMITZ_STATE", "NIMITZ_CLAUDE_HOME"].iter().map(|k| (k.to_string(), td.path().join(k).to_string_lossy().into_owned())).collect();
        vars.insert("NIMITZ_HOSTNAME".into(), "box".into());
        let env = Env::from_vars(&vars);
        let apply = |p: &Plan| assert!(crate::config::apply_config_plan(&env, p).verified, "{p:?}");
        let me = this_machine(&env).unwrap();
        assert!(!plan_machine_set(&env, &me, None, Some("claude"), None).ok(), "relative");
        assert!(!plan_machine_set(&env, &me, None, Some("/nowhere/claude"), None).ok(), "this machine's must exist");
        assert!(plan_machine_set(&env, "other", None, Some("/nowhere/claude"), None).ok(), "another machine's need not");
        let bin = |n: &str| {
            let p = td.path().join(n);
            std::fs::write(&p, "x").unwrap();
            p.to_string_lossy().into_owned()
        };
        let (wrapper, real) = (bin("wrapper"), bin("real"));
        apply(&plan_machine_set(&env, &me, Some(&wrapper), Some(&real), None));
        assert_eq!(load_machines(&env.home).unwrap().min_nimitz.as_deref(), Some(NATIVE_BIN_MIN_NIMITZ));
        // the order: the variable, the machine's native binary, its claude_bin
        let e = Env::from_vars(&vars);
        assert_eq!((e.claude_bin.as_str(), e.native_claude_bin.as_str()), (wrapper.as_str(), real.as_str()));
        let mut given = vars.clone();
        given.insert("NIMITZ_NATIVE_CLAUDE_BIN".into(), "/x/native".into());
        assert_eq!(Env::from_vars(&given).native_claude_bin, "/x/native");
        // '' clears it; the file's floor stays, a raise is never lowered
        apply(&plan_machine_set(&env, &me, None, Some(""), None));
        assert_eq!(Env::from_vars(&vars).native_claude_bin, wrapper);
        assert_eq!(load_machines(&env.home).unwrap().min_nimitz.as_deref(), Some(NATIVE_BIN_MIN_NIMITZ));
    }

    /// The versions the doctor lists sort as numbers: 2.1.99 before 2.1.288.
    #[test]
    fn candidates_list_the_versions_in_number_order() {
        let td = tempfile::tempdir().unwrap();
        let vars: HashMap<String, String> =
            ["HOME", "NIMITZ_HOME", "NIMITZ_STATE", "NIMITZ_CLAUDE_HOME"].iter().map(|k| (k.to_string(), td.path().join(k).to_string_lossy().into_owned())).collect();
        let env = Env::from_vars(&vars);
        let home = td.path().join("HOME");
        let versions = home.join(".local/share/claude/versions");
        std::fs::create_dir_all(versions.join("2.1.300")).unwrap();
        std::fs::write(versions.join("2.1.300").join(crate::os::exe_name("claude")), "x").unwrap();
        for v in ["2.1.288-beta", "2.1.288", "2.1.99"] {
            std::fs::write(versions.join(v), "x").unwrap();
        }
        let got: Vec<String> = candidates_in(&env, &home).into_iter().filter(|c| std::path::Path::new(c).starts_with(&versions)).collect();
        let want = [versions.join("2.1.99"), versions.join("2.1.288"), versions.join("2.1.288-beta"), versions.join("2.1.300").join(crate::os::exe_name("claude"))];
        assert_eq!(got, want.map(|p| p.to_string_lossy().into_owned()));
    }
}
