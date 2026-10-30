//! Which API provider a member's Claude Code talks to (issue #12), and the
//! per-machine settings that pin it: `H/machines.toml`, `[machines.<label>]`
//! with `claude_bin` (the cold-launch binary; `NIMITZ_CLAUDE_BIN` still wins)
//! and `provider` (`anthropic`, `bedrock` or `vertex`).
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// `H/machines.toml`: each machine's own settings, by label (shared: a lead
/// may set another machine's; only that machine uses them).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machines {
    /// the file's format (`formats`, `machines`; absent = 1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<u32>,
    #[serde(default)]
    pub machines: BTreeMap<String, MachineSettings>,
}

pub fn machines_path(home: &std::path::Path) -> PathBuf {
    home.join("machines.toml")
}

/// The machines file (none: empty); an unreadable one is an error.
pub fn load_machines(home: &std::path::Path) -> Result<Machines, String> {
    let p = machines_path(home);
    match std::fs::read_to_string(&p) {
        Ok(t) => {
            let m: Machines = toml::from_str(&t).map_err(|e| format!("{}: {e}", p.display()))?;
            // a newer nimitz's file is never read further nor rewritten
            match m.format.filter(|f| *f > crate::formats::current("machines")) {
                Some(f) => Err(format!("{}: format {f}, written by a newer nimitz", p.display())),
                None => Ok(m),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Machines::default()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

/// This machine's label: the registry's, else the host name's first part.
pub fn this_machine(env: &Env) -> String {
    let r = crate::registry::load(env);
    if !r.machine.is_empty() {
        return r.machine;
    }
    host_label(&env.hostname)
}

/// The label a host name gives (`crud::machine_label`'s rule).
fn host_label(hostname: &str) -> String {
    let l: String = hostname.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
    l.split('.').next().unwrap_or("this-machine").to_string()
}

/// This machine's settings (none when unset or unreadable).
pub fn this_settings(env: &Env) -> MachineSettings {
    load_machines(&env.home).ok().and_then(|m| m.machines.get(&this_machine(env)).cloned()).unwrap_or_default()
}

/// The provider this machine expects (a name: `bedrock`), if set.
pub fn expected(env: &Env) -> Option<String> {
    this_settings(env).provider
}

/// The `claude_bin` the machines file sets for the machine whose registry
/// is under `state` (or whose host name is `hostname`): read while the Env
/// is built, so it cannot use one.
pub fn claude_bin_for(home: &std::path::Path, state: &std::path::Path, hostname: &str) -> Option<String> {
    let m = load_machines(home).ok()?;
    let label = std::fs::read(state.join("registry.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.get("machine").and_then(Value::as_str).map(str::to_string))
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| host_label(hostname));
    m.machines.get(&label)?.claude_bin.clone().filter(|b| !b.is_empty())
}

/// `nimitz machine set <label> [--claude-bin P] [--provider P]` (`Some("")`
/// clears either). The file is shared; a machine reads its own label's.
pub fn plan_machine_set(env: &Env, label: &str, claude_bin: Option<&str>, provider: Option<&str>) -> Plan {
    let mut plan = Plan { summary: format!("set machine {label}"), ..Default::default() };
    let refuse = |plan: &mut Plan, msg: String, fix: Option<String>| plan.refusals.push(Issue { code: "config.invalid".into(), severity: Severity::Error, message: msg, fix, id: None });
    if label.is_empty() || !label.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        refuse(&mut plan, format!("{label:?} is not a machine label (letters, digits, . _ -)"), None);
        return plan;
    }
    if claude_bin.is_none() && provider.is_none() {
        refuse(&mut plan, "nothing to set".into(), Some(format!("nimitz machine set {label} --provider anthropic|bedrock|vertex [--claude-bin PATH]")));
        return plan;
    }
    if let Some(p) = provider.filter(|p| !p.is_empty() && api_of(p).is_none()) {
        refuse(&mut plan, format!("--provider {p}: use anthropic, bedrock or vertex"), None);
        return plan;
    }
    if let Some(b) = claude_bin.filter(|b| !b.is_empty()) {
        if !std::path::Path::new(b).is_absolute() {
            refuse(&mut plan, format!("--claude-bin {b}: an absolute path"), None);
            return plan;
        }
        if label == this_machine(env) && !std::path::Path::new(b).is_file() {
            refuse(&mut plan, format!("--claude-bin {b}: no such file on this machine"), Some("nimitz doctor  # lists the claude binaries it finds".into()));
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
    let before = m.clone();
    let s = m.machines.entry(label.to_string()).or_default();
    if let Some(b) = claude_bin {
        s.claude_bin = Some(b.to_string()).filter(|b| !b.is_empty());
    }
    if let Some(p) = provider {
        s.provider = Some(p.to_string()).filter(|p| !p.is_empty());
    }
    if m.machines.get(label).is_some_and(|s| *s == MachineSettings::default()) {
        m.machines.remove(label);
    }
    if m != before {
        let text = toml::to_string_pretty(&m).unwrap_or_default();
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
    let (rc, text) = crate::claude::run(&argv, cwd);
    let v = text.find('{').and_then(|at| crate::evidence::first_json(text[at..].as_bytes()));
    match v.as_ref().and_then(|v| v.get("apiProvider")).and_then(Value::as_str) {
        Some(p) => Ok(p.to_string()),
        None => Err(format!("`{} auth status --json` gave no apiProvider (exit {rc}): {}", bin, text.chars().take(200).collect::<String>())),
    }
}

/// Is a cold launch `cmd` (in `cwd`) allowed on this machine? Its provider
/// by Claude Code's own resolver against the machine's expected one; with
/// none set, a Bedrock launch (or a machine with a member recorded on
/// Bedrock) needs one first. `Err` is the refusal; `Ok` the provider.
pub fn check_launch(env: &Env, id: Option<&Id>, cmd: &[String], cwd: Option<&str>) -> Result<String, Issue> {
    let label = this_machine(env);
    let set = format!("nimitz machine set {label} --provider bedrock|anthropic|vertex");
    let issue = |code: &str, message: String, fix: String| Issue { code: code.into(), severity: Severity::Error, message, fix: Some(fix), id: id.cloned() };
    let who = id.map(|i| i.to_string()).unwrap_or_else(|| "the member".into());
    let expected = expected(env);
    let got = match preflight(cmd, cwd) {
        Ok(p) => p,
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
        Some(want) if api_of(want) == Some(got.as_str()) => Ok(got),
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
        None => Ok(got),
    }
}

/// After a cold launch bound session `sid` of `id`: its SessionStart record
/// against the machine's expected provider (waited for a moment). A
/// mismatch is the refusal (the caller stops and removes the job); no
/// record is a warning; no expected provider, nothing to compare.
pub fn check_started(env: &Env, id: &Id, sid: &str) -> Result<Option<Issue>, Issue> {
    let Some(want) = expected(env) else { return Ok(None) };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let rec = loop {
        match record_of(env, id).filter(|r| r.session_id == sid) {
            Some(r) => break Some(r),
            None if std::time::Instant::now() >= deadline => break None,
            None => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    };
    let label = this_machine(env);
    let Some(r) = rec else {
        return Ok(Some(Issue {
            code: "provider.unrecorded".into(),
            severity: Severity::Warn,
            message: format!("{id}'s SessionStart left no provider record for session {sid}; its provider is checked by the preflight only"),
            fix: Some(format!("nimitz doctor  # the hooks; then nimitz check {id}")),
            id: Some(id.clone()),
        }));
    };
    if api_of(&want) == Some(r.provider.as_str()) && r.model_agrees() {
        return Ok(None);
    }
    Err(Issue {
        code: "provider.mismatch".into(),
        severity: Severity::Error,
        message: format!(
            "{id} started on {} (model {}) although machine {label} expects {want} and Claude Code's own check before the launch agreed (managed settings?); its job is stopped and removed, but its first prompt may already have been sent",
            name_of(&r.provider),
            r.model.as_deref().unwrap_or("unknown")
        ),
        fix: Some(format!("check the managed settings (/etc/claude-code, /Library/Application Support/ClaudeCode, MDM) and `claude auth status --json` in its folder; then nimitz start {id}")),
        id: Some(id.clone()),
    })
}

/// The selector `--settings` gets so a member runs on this machine's
/// provider: `CLAUDE_CODE_USE_BEDROCK=1` or `_VERTEX=1`; none for anthropic
/// (or no provider named). Settings env beats the shell's and the folder's
/// local settings, and the job's `respawnFlags` keep it for every wake.
pub fn forced_selector(env: &Env) -> Option<(&'static str, &'static str)> {
    match expected(env)?.as_str() {
        "bedrock" => Some(("CLAUDE_CODE_USE_BEDROCK", "1")),
        "vertex" => Some(("CLAUDE_CODE_USE_VERTEX", "1")),
        _ => None,
    }
}

// ------------------------------------------------------------------ the record

/// What a member's worker saw at its SessionStart (`NIMITZ_STATE/provider/<F.SQ.SHIP>.json`).
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

fn record_path(env: &Env, id: &Id) -> PathBuf {
    records_dir(env).join(format!("{}.json", id.0.replace('/', ".")))
}

/// The SessionStart hook's record of member `id` (session `sid`): its model
/// and provider env, the provider they select. Never fails the hook.
pub fn record(env: &Env, id: &Id, sid: &str, input: &Value, vars: &HashMap<String, String>) {
    let mut kept = BTreeMap::new();
    for k in SELECTORS.iter().map(|(k, _)| *k).chain(RECORDED) {
        if let Some(v) = vars.get(k).filter(|v| !v.is_empty()) {
            kept.insert(k.to_string(), v.clone());
        }
    }
    let r = Record {
        id: id.0.clone(),
        session_id: sid.to_string(),
        at: crate::evidence::now(),
        model: input.get("model").and_then(Value::as_str).map(str::to_string).filter(|m| !m.is_empty()),
        provider: from_selectors(|k| kept.get(k).cloned()).to_string(),
        env: kept,
    };
    let _ = crate::config::atomic_write(&record_path(env, id), &record_text(&r));
}

/// Member `id`'s latest record.
pub fn record_of(env: &Env, id: &Id) -> Option<Record> {
    serde_json::from_slice(&std::fs::read(record_path(env, id)).ok()?).ok()
}

/// Every record on this machine.
pub fn records(env: &Env) -> Vec<Record> {
    let Ok(rd) = std::fs::read_dir(records_dir(env)) else { return vec![] };
    rd.flatten().filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()).collect()
}

/// Has a member on this machine been recorded on `api`?
pub fn recorded_on(env: &Env, api: &str) -> bool {
    records(env).iter().any(|r| r.provider == api)
}

// ------------------------------------------------------------------ check

/// `check`/`status`: each live member of this machine (its id, the session
/// it runs) whose latest record, of that session, differs from the provider
/// the machine expects (or, with none set, from the one most of its members
/// run on), or whose model disagrees with it.
pub fn issues(env: &Env, live: &[(Id, String)]) -> Vec<Issue> {
    let recs: Vec<Record> = live.iter().filter_map(|(id, sid)| record_of(env, id).filter(|r| r.session_id == *sid)).collect();
    let expected = expected(env).and_then(|e| api_of(&e)).map(str::to_string);
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
    let label = this_machine(env);
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
                fix: Some(fresh),
                id: Some(id),
            });
        }
    }
    out
}

// ------------------------------------------------------------------ doctor

/// The `claude` binaries `doctor` lists (never writes): what PATH finds, the
/// native installer's launcher, a localtools install, an npm global install.
pub fn claude_bin_candidates(env: &Env) -> Vec<String> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
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
    if let Ok(rd) = std::fs::read_dir(home.join(".localtools/tools/claude-code")) {
        for e in rd.flatten() {
            add(Some(e.path().join("claude").to_string_lossy().into_owned()));
        }
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
        let me = this_machine(&env);
        let apply = |p: &Plan| assert!(crate::config::apply_config_plan(&env, p).verified, "{p:?}");
        assert!(!plan_machine_set(&env, &me, None, Some("openai")).ok());
        assert!(!plan_machine_set(&env, &me, None, None).ok());
        assert!(!plan_machine_set(&env, &me, Some("claude"), None).ok(), "relative");
        assert!(!plan_machine_set(&env, &me, Some("/nowhere/claude"), None).ok(), "this machine's must exist");
        apply(&plan_machine_set(&env, "work", Some("/nowhere/claude"), Some("bedrock")));
        apply(&plan_machine_set(&env, &me, None, Some("anthropic")));
        assert_eq!(expected(&env).as_deref(), Some("anthropic"));
        let m = load_machines(&env.home).unwrap();
        assert_eq!(m.machines["work"], MachineSettings { claude_bin: Some("/nowhere/claude".into()), provider: Some("bedrock".into()) });
        assert_eq!(claude_bin_for(&env.home, &env.state, "work"), Some("/nowhere/claude".into()));
        assert!(plan_machine_set(&env, &me, None, Some("anthropic")).steps.is_empty(), "nothing changes");
        apply(&plan_machine_set(&env, &me, None, Some("")));
        assert!(!load_machines(&env.home).unwrap().machines.contains_key(&me), "an empty entry goes");
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
        let rec = |id: &Id, bedrock: bool, model: &str| {
            let v: HashMap<String, String> = if bedrock { [("CLAUDE_CODE_USE_BEDROCK".to_string(), "1".to_string())].into() } else { HashMap::new() };
            record(&env, id, "sid", &json!({"model": model}), &v);
        };
        rec(&ids[0], true, "us.anthropic.claude-opus-5-5");
        rec(&ids[1], true, "us.anthropic.claude-opus-5-5");
        rec(&ids[2], false, "claude-opus-5-5");
        assert_eq!(record_of(&env, &ids[2]).unwrap().provider, "firstParty");
        assert!(recorded_on(&env, "bedrock"));
        // no expected provider: the majority (Bedrock) flags the odd one
        let live: Vec<(Id, String)> = ids.iter().map(|i| (i.clone(), "sid".to_string())).collect();
        let i = issues(&env, &live);
        assert_eq!(i.len(), 1, "{i:?}");
        assert!(i[0].code == "provider.mismatch" && i[0].id.as_ref() == Some(&ids[2]) && i[0].fix.as_deref().unwrap().contains("--fresh"), "{i:?}");
        // expected anthropic: the two Bedrock ones
        crate::config::apply_config_plan(&env, &plan_machine_set(&env, &this_machine(&env), None, Some("anthropic")));
        let i = issues(&env, &live);
        assert_eq!(i.iter().filter_map(|i| i.id.clone()).collect::<Vec<_>>(), ids[..2].to_vec());
        // a model that disagrees with its selectors
        rec(&ids[2], false, "us.anthropic.claude-opus-5-5");
        assert!(issues(&env, &live[2..]).iter().any(|i| i.code == "provider.model"));
        // a record of another session (an old one, another home's) does not count
        assert!(issues(&env, &[(ids[2].clone(), "other".into())]).is_empty());
    }
}
