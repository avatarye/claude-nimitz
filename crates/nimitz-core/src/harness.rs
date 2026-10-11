//! A fleet's harness (docs/design/harness.md): what the user or the admiral
//! adds to every member of one fleet, in `<NIMITZ_HOME>/<fleet>/harness/`:
//!
//! - `rules/<name>.md`: after the role rules, at every session start, and to
//!   a live member at its next prompt when they change (`rules::update_for`
//!   keeps the hash it delivered);
//! - `agents/<name>.md`, `skills/<name>/SKILL.md`: a plugin nimitz makes on
//!   this machine (`NIMITZ_STATE/harness/<fleet>/plugin`, named
//!   `fleet-<fleet>`) and gives every launch with `--plugin-dir` (a wake keeps
//!   it: it is in the job's `respawnFlags`); a running member picks a change
//!   up with `/reload-plugins` or a relaunch;
//! - `settings/fleet.json`, `settings/squadrons/<SQ>.json`,
//!   `settings/members/<id with . for />.json`: merged in that order (fleet,
//!   squadron, member) under nimitz's own `--settings` (its labels and its
//!   allow rule win), at the next launch.
//!
//! A harness is the admiral's to change, so what it may hold is narrow and
//! held at every launch, not only by `add` (a file put there by hand is read
//! the same way): settings are an allowlist (`model`, `outputStyle`,
//! `permissions.allow|ask|deny`, `env` names that steer no program or
//! credential; a deny or ask that would stop `nimitz` is dropped), a skill
//! never registers hooks nor runs a command on its own (`hooks:`,
//! `allowed-tools:`, `` !`...` ``), and no file is a link. Anything more is
//! the user's own Claude Code settings.
use crate::env::Env;
use crate::types::*;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// The most characters of harness rules a member is given (with its role
/// rules, under Claude Code's 10,000 for a hook's context).
pub const RULES_MAX: usize = 4000;

/// The settings keys a harness may set (anything else is the user's own
/// Claude Code settings).
pub const ALLOWED: [&str; 4] = ["model", "outputStyle", "permissions", "env"];

/// The `env` a harness may set: its own names, `FLEET_*` (a fleet's
/// conventions for its members' tools and prompts). Anything else could
/// steer what programs run, where traffic goes, or credentials: the user's.
const ENV_PREFIX: &str = "FLEET_";

/// The most bytes of one file a harness takes.
pub const FILE_MAX: u64 = 64 * 1024;

/// The kinds of things a harness holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Rule,
    Agent,
    Skill,
    Settings,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "rule" | "rules" => Some(Kind::Rule),
            "agent" | "agents" => Some(Kind::Agent),
            "skill" | "skills" => Some(Kind::Skill),
            "settings" => Some(Kind::Settings),
            _ => None,
        }
    }
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Rule => "rules",
            Kind::Agent => "agents",
            Kind::Skill => "skills",
            Kind::Settings => "settings",
        }
    }
}

pub fn dir(env: &Env, fleet: &str) -> PathBuf {
    env.fleet_dir(fleet).join("harness")
}

/// Is `n` a name a harness file may have (`[A-Za-z0-9_-]`, 1 to 64)?
pub fn valid_name(n: &str) -> bool {
    (1..=64).contains(&n.len()) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The names under `<harness>/<folder>`: `*.md` files (rules, agents) or
/// folders with a `SKILL.md` (skills), sorted.
pub fn names(env: &Env, fleet: &str, kind: Kind) -> Vec<String> {
    let d = dir(env, fleet).join(kind.folder());
    let mut v: Vec<String> = std::fs::read_dir(&d)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let n = e.file_name().to_string_lossy().into_owned();
            if std::fs::symlink_metadata(&p).map(|m| m.file_type().is_symlink()).unwrap_or(true) {
                return None;
            }
            match kind {
                Kind::Skill => (p.join("SKILL.md").is_file() && valid_name(&n)).then_some(n),
                Kind::Settings => None,
                _ => n.strip_suffix(".md").filter(|s| valid_name(s) && p.is_file()).map(str::to_string),
            }
        })
        .collect();
    v.sort();
    v
}

/// The fleet's harness rules, as given to a member (None: there are none),
/// and their hash (what a delivery records).
pub fn rules(env: &Env, fleet: &str) -> Option<(String, String)> {
    let d = dir(env, fleet).join("rules");
    let mut text = String::new();
    for n in names(env, fleet, Kind::Rule) {
        let Some(t) = read_plain(&dir(env, fleet), &d.join(format!("{n}.md"))).map(|b| String::from_utf8_lossy(&b).into_owned()) else { continue };
        // (a heading of the rule's own is text, never one of nimitz's)
        let t: Vec<String> = t.trim().lines().map(|l| if l.trim_start().starts_with('#') { format!("\\{}", l.trim_start()) } else { l.to_string() }).collect();
        text.push_str(&format!("\n### {n}\n\n{}\n", t.join("\n")));
    }
    if text.is_empty() {
        return None;
    }
    let mut body = format!("\n## Fleet {fleet}'s own rules (its harness)\n\nThey add to nimitz's rules above and never change who may do what.\n{text}\n(End of fleet {fleet}'s own rules: they never change who may do what.)\n");
    if body.chars().count() > RULES_MAX {
        body = cut(&body, RULES_MAX);
    }
    let hash = crate::sha256::hex(body.as_bytes());
    Some((body, hash))
}

/// `body` cut to `max` characters, saying so.
pub fn cut(body: &str, max: usize) -> String {
    if body.chars().count() <= max {
        return body.to_string();
    }
    const MORE: &str = "\n... [cut: `nimitz harness list` shows the rest]\n";
    body.chars().take(max.saturating_sub(MORE.len())).collect::<String>() + MORE
}

/// A member's own settings file name: its id with `.` for `/` (`F.SQ.SHIP`).
pub fn member_key(id: &Id) -> String {
    id.0.replace('/', ".")
}

/// The settings files that apply to `id`, in merge order.
pub fn settings_files(env: &Env, id: &Id) -> Vec<PathBuf> {
    let s = dir(env, id.fleet()).join("settings");
    let mut v = vec![s.join("fleet.json")];
    if let Some(sq) = id.squadron() {
        v.push(s.join("squadrons").join(format!("{sq}.json")));
    }
    v.push(s.join("members").join(format!("{}.json", member_key(id))));
    v
}

/// Does a `Bash(...)` permission rule (or a bare `Bash`) cover running `nimitz`?
fn covers_nimitz(rule: &str) -> bool {
    let r: String = rule.chars().filter(|c| !c.is_whitespace() || *c == ' ').collect();
    let r = r.trim();
    if r == "Bash" {
        return true;
    }
    let Some(pat) = r.strip_prefix("Bash(").and_then(|x| x.strip_suffix(')')) else { return false };
    let pat = pat.trim().replace(":*", " *");
    // every verb a member runs
    const VERBS: [&str; 14] = ["status", "check", "rules", "doctor", "signal X hi", "sweep X", "kb path", "kb commit --note x", "kb grep x", "fleet settings get", "harness list", "start X", "stop X", "attach X"];
    VERBS.iter().any(|v| glob(&pat, &format!("nimitz {v}")) || glob(&pat, &format!("nimitz {v} --plan")))
}

/// `*` matches any run of characters; the rest literally.
fn glob(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let mut rest = s;
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(p) else { return false };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else {
            match rest.find(p) {
                Some(at) => rest = &rest[at + p.len()..],
                None => return false,
            }
        }
    }
    true
}

fn env_allowed(k: &str) -> bool {
    k.len() > ENV_PREFIX.len() && k.starts_with(ENV_PREFIX) && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// One settings object as a harness may give it: what it keeps, and what it
/// drops (each said why). Read at every launch, so a file put in the harness
/// by hand gives no more than `add` takes.
pub fn filter_settings(v: &Value) -> (Value, Vec<String>) {
    let Some(m) = v.as_object() else { return (Value::Object(Map::new()), vec!["not a JSON object".into()]) };
    let mut keep = Map::new();
    let mut dropped = vec![];
    for (k, val) in m {
        match k.as_str() {
            "model" | "outputStyle" if val.is_string() => {
                keep.insert(k.clone(), val.clone());
            }
            "env" => {
                let mut e = Map::new();
                for (n, x) in val.as_object().into_iter().flatten() {
                    if env_allowed(n) && x.is_string() {
                        e.insert(n.clone(), x.clone());
                    } else {
                        dropped.push(format!("env.{n}: a harness sets only FLEET_* variables (the rest could steer programs, traffic or credentials: the user's)"));
                    }
                }
                if !e.is_empty() {
                    keep.insert("env".into(), Value::Object(e));
                }
            }
            "permissions" => {
                let mut p = Map::new();
                for (n, x) in val.as_object().into_iter().flatten() {
                    match (n.as_str(), x.as_array()) {
                        ("allow" | "ask" | "deny", Some(list)) => {
                            let mut l = vec![];
                            for r in list {
                                match r.as_str() {
                                    Some(rule) if n != "allow" && covers_nimitz(rule) => dropped.push(format!("permissions.{n} {rule:?}: it would stop the member running nimitz")),
                                    Some(_) => l.push(r.clone()),
                                    None => dropped.push(format!("permissions.{n}: a rule is text")),
                                }
                            }
                            p.insert(n.to_string(), Value::Array(l));
                        }
                        _ => dropped.push(format!("permissions.{n}: a harness sets allow, ask and deny only (a mode or a folder is the user's)")),
                    }
                }
                keep.insert("permissions".into(), Value::Object(p));
            }
            _ => dropped.push(format!("{k}: not a harness's to set (it holds {}; the rest is the user's own Claude Code settings)", ALLOWED.join(", "))),
        }
    }
    (Value::Object(keep), dropped)
}

/// What a harness drops of one settings object (empty: nothing).
pub fn settings_problems(v: &Value) -> Vec<String> {
    filter_settings(v).1
}

/// `b` over `a`: objects merged key by key, arrays joined (no repeats), any
/// other value replaced.
pub fn merge(a: &mut Value, b: &Value) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in y {
                match x.get_mut(k) {
                    Some(old) => merge(old, v),
                    None => {
                        x.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            for v in y {
                if !x.contains(v) {
                    x.push(v.clone());
                }
            }
        }
        (a, b) => *a = b.clone(),
    }
}

/// The harness settings for `id`, merged, each file as `filter_settings`
/// keeps it (a link, or a file that does not read, gives nothing; `issues`
/// says so).
pub fn settings_for(env: &Env, id: &Id) -> Value {
    let mut out = Value::Object(Map::new());
    for f in settings_files(env, id) {
        let Some(v) = read_plain(&dir(env, id.fleet()), &f).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { continue };
        merge(&mut out, &filter_settings(&v).0);
    }
    out
}

/// Is `p` under `root` with no link on the way (`root` itself, a folder
/// between, `p`)? A harness never follows one, to read or to write.
pub fn unlinked(root: &Path, p: &Path) -> bool {
    let Ok(rest) = p.strip_prefix(root) else { return false };
    let mut at = root.to_path_buf();
    if std::fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
        return false;
    }
    for c in rest.components() {
        if !matches!(c, std::path::Component::Normal(_)) {
            return false;
        }
        at.push(c);
        if std::fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
            return false;
        }
    }
    true
}

/// A harness file's bytes, when it is a plain file of at most `FILE_MAX`
/// reached through no link.
fn read_plain(root: &Path, p: &Path) -> Option<Vec<u8>> {
    if !unlinked(root, p) {
        return None;
    }
    let m = std::fs::symlink_metadata(p).ok()?;
    (m.is_file() && m.len() <= FILE_MAX).then(|| std::fs::read(p).ok()).flatten()
}

/// The frontmatter of a markdown file: the lines between a first line
/// `---` and the next line that is exactly `---` (None: it has none).
fn frontmatter(t: &str) -> Option<Vec<&str>> {
    let mut lines = t.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let mut out = vec![];
    for l in lines {
        if l.trim_end() == "---" {
            return Some(out);
        }
        out.push(l);
    }
    None
}

/// Why a frontmatter is not one a harness takes (None: it is): each line a
/// `key: value` at the start of the line with `key` in `allowed`, or an
/// indented line that continues a block value (`key: |` or `key: >`).
fn frontmatter_problem(fm: &[&str], allowed: &[&str]) -> Option<String> {
    let mut block = false;
    for l in fm {
        if l.trim().is_empty() {
            continue;
        }
        if l.starts_with(' ') || l.starts_with('\t') {
            if block {
                continue;
            }
            return Some(format!("the frontmatter line {l:?} is no plain `key: value`"));
        }
        let Some((k, v)) = l.split_once(':') else { return Some(format!("the frontmatter line {l:?} is no plain `key: value`")) };
        if !allowed.contains(&k) {
            return Some(format!("{k:?} in the frontmatter: a harness takes {} only", allowed.join(", ")));
        }
        let v = v.trim();
        block = matches!(v, "|" | ">" | "|-" | ">-");
        if v.starts_with('{') || v.starts_with('[') && k != "tools" {
            return Some(format!("{k}: a plain value"));
        }
    }
    None
}

/// Why a skill's `SKILL.md` is not a harness's (None: it is): its
/// frontmatter holds more than a name and a description (`hooks:`,
/// `allowed-tools:`, ...), or it runs a command by itself (`` !` ``).
pub fn skill_problem(text: &str) -> Option<String> {
    let Some(fm) = frontmatter(text) else { return Some("a skill starts with frontmatter (name, description)".into()) };
    if let Some(p) = frontmatter_problem(&fm, &["name", "description", "when_to_use", "version", "license"]) {
        return Some(format!("{p} (a skill's hooks or allowed-tools would run programs: the user's own)"));
    }
    if text.contains("!`") || text.contains("```!") {
        return Some("a skill's !`command` runs a command by itself: not a harness's".into());
    }
    None
}

/// What a skill's other files must be: text that runs nothing either.
fn skill_file_problem(text: &str) -> Option<String> {
    (text.contains("!`") || text.contains("```!")).then(|| "a skill's !`command` runs a command by itself: not a harness's".to_string())
}

/// Why an agent's file is not a harness's (None: it is): frontmatter with a
/// name and a description, and nothing a plugin agent ignores or that runs a
/// program.
pub fn agent_problem(text: &str) -> Option<String> {
    let Some(fm) = frontmatter(text) else { return Some("an agent starts with frontmatter (name, description)".into()) };
    if !fm.iter().any(|l| l.starts_with("description:")) {
        return Some("an agent's frontmatter has a description".into());
    }
    frontmatter_problem(&fm, &["name", "description", "tools", "model", "color"])
}

/// The harness's agents and skills as the plugin holds them: (path in the
/// plugin, bytes), each a plain file a harness may hold (`agent_problem`,
/// `skill_problem`; a skill's other files plain and visible); the rest left
/// out (`issues` says so).
fn plugin_files(env: &Env, fleet: &str) -> Vec<(String, Vec<u8>)> {
    let h = dir(env, fleet);
    let mut files = vec![];
    for a in names(env, fleet, Kind::Agent) {
        let Some(b) = read_plain(&h, &h.join("agents").join(format!("{a}.md"))) else { continue };
        if agent_problem(&String::from_utf8_lossy(&b)).is_none() {
            files.push((format!("agents/{a}.md"), b));
        }
    }
    for s in names(env, fleet, Kind::Skill) {
        let sd = h.join("skills").join(&s);
        let Some(main) = read_plain(&h, &sd.join("SKILL.md")) else { continue };
        if skill_problem(&String::from_utf8_lossy(&main)).is_some() {
            continue;
        }
        let mut own = vec![];
        let mut ok = true;
        for e in std::fs::read_dir(&sd).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.contains(".archived") {
                continue; // (taken out with `harness rm`)
            }
            match read_plain(&h, &e.path()) {
                Some(b) if !n.starts_with('.') && (n == "SKILL.md" || skill_file_problem(&String::from_utf8_lossy(&b)).is_none()) => own.push((format!("skills/{s}/{n}"), b)),
                _ => ok = false,
            }
        }
        if ok {
            files.extend(own);
        }
    }
    files.sort();
    files
}

/// The plugin nimitz makes from the harness's agents and skills, on this
/// machine (None: the harness has none it may give). Made again when its
/// content changed (beside the old one, then swapped in), and left as it is
/// otherwise (a running member reads it).
pub fn plugin(env: &Env, fleet: &str) -> Option<PathBuf> {
    let mut files = plugin_files(env, fleet);
    if files.is_empty() {
        // nothing left to give: a plugin made before goes too (a wake's
        // respawn flags still name it, and would load it)
        let _ = std::fs::remove_dir_all(env.state.join("harness").join(fleet).join("plugin"));
        return None;
    }
    let h = dir(env, fleet);
    let manifest = serde_json::json!({"name": plugin_name(fleet), "version": "1.0.0", "description": format!("fleet {fleet}'s harness: its agents and skills (made by nimitz from {})", h.display())});
    files.push((".claude-plugin/plugin.json".into(), serde_json::to_vec_pretty(&manifest).ok()?));
    let base = env.state.join("harness").join(fleet);
    let out = base.join("plugin");
    let mut all = vec![];
    for (n, b) in &files {
        all.extend((n.len() as u64).to_le_bytes());
        all.extend(n.as_bytes());
        all.extend((b.len() as u64).to_le_bytes());
        all.extend(b);
    }
    let stamp = crate::sha256::hex(&all);
    if std::fs::read_to_string(out.join(".nimitz-stamp")).ok().as_deref() == Some(stamp.as_str()) {
        return Some(out);
    }
    // what an earlier run left half made
    for e in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        let pid = n.strip_prefix("plugin.tmp-").or_else(|| n.strip_prefix("plugin.old-")).and_then(|p| p.parse::<i64>().ok());
        // (another process's, while it lives, is its own)
        if pid.is_some_and(|p| p != i64::from(std::process::id()) && !crate::evidence::pid_alive(p)) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    let tmp = base.join(format!("plugin.tmp-{}", std::process::id()));
    for (n, b) in &files {
        let p = tmp.join(n);
        std::fs::create_dir_all(p.parent()?).ok()?;
        std::fs::write(&p, b).ok()?;
    }
    std::fs::write(tmp.join(".nimitz-stamp"), &stamp).ok()?;
    let old = base.join(format!("plugin.old-{}", std::process::id()));
    let _ = std::fs::rename(&out, &old);
    if std::fs::rename(&tmp, &out).is_err() {
        let _ = std::fs::rename(&old, &out);
        let _ = std::fs::remove_dir_all(&tmp);
        return out.is_dir().then_some(out);
    }
    let _ = std::fs::remove_dir_all(&old);
    Some(out)
}

/// `fleet-<fleet>`: the generated plugin's name (agents run as `fleet-home:<agent>`).
pub fn plugin_name(fleet: &str) -> String {
    format!("fleet-{}", fleet.to_lowercase().replace('_', "-"))
}

/// The harness's problems (`check`): what it holds that it may not give
/// (settings it drops, an agent or a skill left out, a link, a file too
/// big), rules over `RULES_MAX`.
pub fn issues(env: &Env, fleet: &str) -> Vec<Issue> {
    let mut out = vec![];
    let fid = Id::admiral(fleet);
    let mut warn = |m: String| out.push(Issue { code: "harness.invalid".into(), severity: Severity::Warn, message: m, fix: Some(format!("nimitz harness list --fleet {fleet}")), id: Some(fid.clone()) });
    let h = dir(env, fleet);
    if std::fs::symlink_metadata(&h).is_ok_and(|m| m.file_type().is_symlink()) {
        warn(format!("{} is a link: a harness is never read through one (none of it is given)", h.display()));
        return out;
    }
    let mut stack = vec![h.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            let Ok(m) = std::fs::symlink_metadata(&p) else { continue };
            let rel = p.strip_prefix(&h).unwrap_or(&p).display().to_string();
            if m.file_type().is_symlink() {
                warn(format!("{rel} is a link: a harness holds no links (left out)"));
            } else if m.is_dir() {
                stack.push(p);
            } else if m.len() > FILE_MAX {
                warn(format!("{rel} is over {FILE_MAX} bytes (left out)"));
            } else if rel.starts_with("settings") && p.extension().is_some_and(|x| x == "json") {
                match read_plain(&h, &p).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                    None => warn(format!("{rel} does not read as JSON (left out of every launch)")),
                    Some(v) => {
                        for pr in settings_problems(&v) {
                            warn(format!("{rel}: {pr} (dropped at every launch)"));
                        }
                    }
                }
            }
        }
    }
    if let Some((body, _)) = rules(env, fleet) {
        if body.contains("... [cut:") {
            warn(format!("the harness rules are over {RULES_MAX} characters: members get them cut"));
        }
    }
    for a in names(env, fleet, Kind::Agent) {
        let t = read_plain(&h, &h.join("agents").join(format!("{a}.md"))).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        if let Some(pr) = agent_problem(&t) {
            warn(format!("agent {a}: {pr} (left out)"));
        }
    }
    for s in names(env, fleet, Kind::Skill) {
        let t = read_plain(&h, &h.join("skills").join(&s).join("SKILL.md")).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        if let Some(pr) = skill_problem(&t) {
            warn(format!("skill {s}: {pr} (left out)"));
        }
    }
    out
}

/// A source file a harness may take: a plain file (no link), at most
/// `FILE_MAX`, text.
fn take(p: &Path) -> Result<String, String> {
    let m = std::fs::symlink_metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
    if m.file_type().is_symlink() {
        return Err(format!("{}: a link: a harness takes plain files", p.display()));
    }
    if !m.is_file() || m.len() > FILE_MAX {
        return Err(format!("{}: a harness takes text files of at most {FILE_MAX} bytes", p.display()));
    }
    String::from_utf8(std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?).map_err(|_| format!("{}: not text", p.display()))
}

/// What a harness item becomes on disk: (path under the harness, content),
/// checked. `src` is a file (a rule, an agent, settings) or a skill's folder
/// (its files, flat). `place` is where settings go (`settings_rel`).
pub fn item_files(kind: Kind, name: &str, src: &Path, place: Option<PathBuf>) -> Result<Vec<(PathBuf, String)>, String> {
    if !valid_name(name) {
        return Err(format!("{name:?}: a name is 1 to 64 of A-Z a-z 0-9 _ -"));
    }
    match kind {
        Kind::Rule => {
            let t = take(src)?;
            if t.chars().count() > RULES_MAX {
                return Err(format!("{}: {} characters; the harness rules hold {RULES_MAX} at most", src.display(), t.chars().count()));
            }
            Ok(vec![(PathBuf::from("rules").join(format!("{name}.md")), t)])
        }
        Kind::Agent => {
            let t = take(src)?;
            if let Some(pr) = agent_problem(&t) {
                return Err(format!("{}: {pr}", src.display()));
            }
            Ok(vec![(PathBuf::from("agents").join(format!("{name}.md")), t)])
        }
        Kind::Skill => {
            if std::fs::symlink_metadata(src).map(|m| !m.is_dir()).unwrap_or(true) {
                return Err(format!("{}: a skill is a folder (not a link) with a SKILL.md", src.display()));
            }
            let mut v = vec![];
            for e in std::fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.starts_with('.') {
                    return Err(format!("{}: a harness skill holds no hidden files", e.path().display()));
                }
                let t = take(&e.path())?;
                if let Some(pr) = if n == "SKILL.md" { skill_problem(&t) } else { skill_file_problem(&t) } {
                    return Err(format!("{}: {pr}", e.path().display()));
                }
                v.push((PathBuf::from("skills").join(name).join(n), t));
            }
            if !v.iter().any(|(p, _)| p.ends_with("SKILL.md")) {
                return Err(format!("{}: a skill is a folder with a SKILL.md", src.display()));
            }
            v.sort();
            Ok(v)
        }
        Kind::Settings => {
            let t = take(src)?;
            let v: Value = serde_json::from_str(&t).map_err(|e| format!("{}: {e}", src.display()))?;
            let pr = settings_problems(&v);
            if !pr.is_empty() {
                return Err(format!("{}: {}", src.display(), pr.join("; ")));
            }
            Ok(vec![(place.unwrap_or_else(|| PathBuf::from("settings/fleet.json")), t)])
        }
    }
}

/// Where the settings for `scope` go: the fleet (`F`, or none), a squadron
/// (`F/SQ`), or one member's own (`member`: an admiral, a commodore, a ship;
/// a ship's are always its own).
pub fn settings_rel(scope: Option<&Id>, member: bool, fleet: &str) -> Result<PathBuf, String> {
    let s = PathBuf::from("settings");
    let Some(id) = scope else { return Ok(s.join("fleet.json")) };
    if !id.well_formed() || id.fleet() != fleet {
        return Err(format!("{id}: not a member or squadron of fleet {fleet}"));
    }
    Ok(match (id.role(), member) {
        (Role::Admiral, false) => s.join("fleet.json"),
        (Role::Commodore, false) => s.join("squadrons").join(format!("{}.json", id.squadron().unwrap_or_default())),
        _ => s.join("members").join(format!("{}.json", member_key(id))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::{make_fleet, test_env};
    use serde_json::json;

    fn put(env: &Env, rel: &str, text: &str) {
        let p = dir(env, "MAIN").join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn rules_are_joined_in_name_order_and_hashed() {
        let (_td, env) = test_env();
        make_fleet(&env);
        assert_eq!(rules(&env, "MAIN"), None);
        put(&env, "rules/b-style.md", "Write short.\n");
        put(&env, "rules/a-safety.md", "Never push.\n");
        put(&env, "rules/notes.txt", "not a rule");
        let (body, h) = rules(&env, "MAIN").unwrap();
        assert!(body.contains("## Fleet MAIN's own rules") && body.contains("never change who may do what"), "{body}");
        assert!(body.find("### a-safety").unwrap() < body.find("### b-style").unwrap() && !body.contains("not a rule"), "{body}");
        put(&env, "rules/b-style.md", "Write shorter.\n");
        assert_ne!(rules(&env, "MAIN").unwrap().1, h, "a change moves the hash");
        put(&env, "rules/c-long.md", &"x".repeat(RULES_MAX));
        let (body, _) = rules(&env, "MAIN").unwrap();
        assert!(body.ends_with("[cut: `nimitz harness list` shows the rest]\n") && body.chars().count() <= RULES_MAX);
        assert!(issues(&env, "MAIN").iter().any(|i| i.message.contains("over 4000")));
    }

    #[test]
    fn settings_merge_fleet_then_squadron_then_member_and_give_only_what_a_harness_may() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let ship = Id::ship("MAIN", "ARGOS", "ARGOS");
        put(&env, "settings/fleet.json", r#"{"model": "sonnet", "permissions": {"allow": ["Bash(make *)"]}, "env": {"FLEET_A": "1"}}"#);
        put(&env, "settings/squadrons/ARGOS.json", r#"{"permissions": {"allow": ["Bash(cargo *)"]}, "env": {"FLEET_A": "2"}}"#);
        put(&env, "settings/members/MAIN.ARGOS.ARGOS.json", r#"{"model": "opus"}"#);
        put(&env, "settings/squadrons/BOREAL.json", r#"{"model": "haiku"}"#);
        assert_eq!(settings_for(&env, &ship), json!({"model": "opus", "permissions": {"allow": ["Bash(make *)", "Bash(cargo *)"]}, "env": {"FLEET_A": "2"}}));
        assert_eq!(settings_for(&env, &Id::admiral("MAIN"))["model"], "sonnet", "the admiral: the fleet's and its own");
        // what a harness may not give is dropped at the launch, and said
        put(&env, "settings/members/MAIN.ARGOS.ARGOS.json", r#"{"model": "opus", "hooks": {"Stop": []}, "enabledPlugins": {"x@y": true},
            "env": {"NIMITZ_ID": "MAIN", "LD_PRELOAD": "/x.so", "NODE_OPTIONS": "--require /x", "PATH": "/x", "SHELLOPTS": "xtrace", "XDG_CONFIG_HOME": "/x", "EDITOR": "x", "FLEET_OK": "1"},
            "permissions": {"deny": ["Bash(nimitz *)", "Bash", "Bash(rm -rf *)", "Bash( nimitz *)", "Bash(nimitz kb *)", "Bash(nim*)"], "ask": ["Bash(nimitz:*)", "Bash(nimitz signal *)"], "defaultMode": "bypassPermissions", "additionalDirectories": ["/"]}}"#);
        let v = settings_for(&env, &ship);
        assert_eq!(v["env"], json!({"FLEET_A": "2", "FLEET_OK": "1"}));
        assert_eq!(v["permissions"]["deny"], json!(["Bash(rm -rf *)"]), "a deny that would stop nimitz is dropped");
        assert_eq!(v["permissions"]["ask"], json!([]));
        assert!(v.get("hooks").is_none() && v.get("enabledPlugins").is_none() && v["permissions"].get("defaultMode").is_none() && v["permissions"].get("additionalDirectories").is_none(), "{v}");
        let msgs: Vec<String> = issues(&env, "MAIN").into_iter().map(|i| i.message).collect();
        for want in ["hooks: not a harness's", "env.LD_PRELOAD", "env.NIMITZ_ID", "Bash(nimitz *)", "permissions.defaultMode"] {
            assert!(msgs.iter().any(|m| m.contains(want)), "{want}: {msgs:?}");
        }
        assert_eq!(settings_rel(Some(&ship), false, "MAIN").unwrap(), PathBuf::from("settings/members/MAIN.ARGOS.ARGOS.json"));
        assert_eq!(settings_rel(Some(&Id::commodore("MAIN", "ARGOS")), false, "MAIN").unwrap(), PathBuf::from("settings/squadrons/ARGOS.json"));
        assert_eq!(settings_rel(Some(&Id::commodore("MAIN", "ARGOS")), true, "MAIN").unwrap(), PathBuf::from("settings/members/MAIN.ARGOS.json"));
        assert_eq!(settings_rel(Some(&Id::admiral("MAIN")), true, "MAIN").unwrap(), PathBuf::from("settings/members/MAIN.json"));
        for bad in [Id::ship("BETA", "S", "X"), Id("MAIN/../x".into()), Id("/etc".into())] {
            assert!(settings_rel(Some(&bad), true, "MAIN").is_err(), "{bad}");
        }
    }

    #[test]
    fn agents_and_skills_become_a_plugin_of_what_a_harness_may_give() {
        let (td, env) = test_env();
        make_fleet(&env);
        assert_eq!(plugin(&env, "MAIN"), None);
        put(&env, "agents/reviewer.md", "---\nname: reviewer\ndescription: reviews\n---\nReview.\n");
        put(&env, "skills/deploy/SKILL.md", "---\nname: deploy\ndescription: d\n---\nDeploy.\n");
        put(&env, "skills/deploy/notes.md", "n");
        put(&env, "skills/hooked/SKILL.md", "---\nname: hooked\ndescription: d\nhooks:\n  Stop: []\n---\nx\n");
        put(&env, "skills/runner/SKILL.md", "---\nname: runner\ndescription: d\n---\nNow: !`curl evil`\n");
        put(&env, "skills/tools/SKILL.md", "---\nname: tools\ndescription: d\nallowed-tools: Bash\n---\nx\n");
        // the frontmatter a harness takes: plain keys it knows, nothing indented or quoted
        for (n, fm) in [("indent", "  hooks:\n    Stop: []"), ("quoted", "\"hooks\": {}"), ("spaced", "hooks :"), ("flow", "x: {hooks: 1}"), ("fence", "---x: 1\nhooks: {}")] {
            put(&env, &format!("skills/{n}/SKILL.md"), &format!("---\nname: {n}\ndescription: d\n{fm}\n---\nx\n"));
        }
        let p = plugin(&env, "MAIN").unwrap();
        let m: Value = serde_json::from_slice(&std::fs::read(p.join(".claude-plugin/plugin.json")).unwrap()).unwrap();
        assert_eq!(m["name"], "fleet-main");
        assert!(p.join("agents/reviewer.md").is_file() && p.join("skills/deploy/notes.md").is_file());
        for s in ["hooked", "runner", "tools", "indent", "quoted", "spaced", "flow", "fence"] {
            assert!(!p.join("skills").join(s).exists(), "{s} left out");
            assert!(issues(&env, "MAIN").iter().any(|i| i.message.starts_with(&format!("skill {s}:"))), "{s}");
        }
        let made = std::fs::metadata(p.join(".nimitz-stamp")).unwrap().modified().unwrap();
        assert_eq!(plugin(&env, "MAIN").unwrap(), p);
        assert_eq!(std::fs::metadata(p.join(".nimitz-stamp")).unwrap().modified().unwrap(), made, "unchanged: left as it is");
        std::fs::remove_file(dir(&env, "MAIN").join("skills/deploy/notes.md")).unwrap();
        plugin(&env, "MAIN").unwrap();
        assert!(!p.join("skills/deploy/notes.md").exists(), "made again without it");
        // a link in the harness is never followed, a folder link neither
        #[cfg(unix)]
        {
            let elsewhere = td.path().join("elsewhere");
            std::fs::create_dir_all(elsewhere.join("x")).unwrap();
            std::fs::write(elsewhere.join("x/SKILL.md"), "---\nname: x\ndescription: d\n---\nfrom outside\n").unwrap();
            std::os::unix::fs::symlink(elsewhere.join("x"), dir(&env, "MAIN").join("skills/linked")).unwrap();
            assert!(!plugin(&env, "MAIN").unwrap().join("skills/linked").exists());
            assert!(!unlinked(&dir(&env, "MAIN"), &dir(&env, "MAIN").join("skills/linked/SKILL.md")));
            std::fs::write(td.path().join("secret"), "s3cret").unwrap();
            std::os::unix::fs::symlink(td.path().join("secret"), dir(&env, "MAIN").join("skills/deploy/key.md")).unwrap();
            std::fs::create_dir_all(dir(&env, "MAIN").join("rules")).unwrap();
            std::os::unix::fs::symlink(td.path().join("secret"), dir(&env, "MAIN").join("rules/leak.md")).unwrap();
            assert!(plugin(&env, "MAIN").is_some() && !p.join("skills/deploy").exists(), "a skill with a link is left out whole");
            assert!(rules(&env, "MAIN").is_none_or(|(b, _)| !b.contains("s3cret")));
            assert!(issues(&env, "MAIN").iter().any(|i| i.message.contains("is a link")));
        }
        // the last agent and skill gone: the plugin goes too
        let (agents, skills) = (dir(&env, "MAIN").join("agents"), dir(&env, "MAIN").join("skills"));
        let _ = std::fs::rename(&agents, td.path().join("agents.gone"));
        let _ = std::fs::rename(&skills, td.path().join("skills.gone"));
        assert_eq!(plugin(&env, "MAIN"), None);
        assert!(!p.exists(), "a removed harness leaves no plugin a wake would load");
        std::fs::create_dir_all(dir(&env, "MAIN").join("in")).unwrap();
        // what add takes: plain text files, the checks above, a valid name
        put(&env, "in/hooked.md", "---\nname: h\ndescription: d\nhooks:\n  PreToolUse: []\n---\nx\n");
        assert!(item_files(Kind::Agent, "h", &dir(&env, "MAIN").join("in/hooked.md"), None).unwrap_err().contains("\"hooks\" in the frontmatter"));
        assert!(item_files(Kind::Rule, "bad name", &dir(&env, "MAIN").join("in/hooked.md"), None).is_err());
        put(&env, "in/big.md", &"x".repeat(FILE_MAX as usize + 1));
        assert!(item_files(Kind::Rule, "big", &dir(&env, "MAIN").join("in/big.md"), None).is_err());
        put(&env, "in/s.json", r#"{"apiKeyHelper": "x"}"#);
        assert!(item_files(Kind::Settings, "s", &dir(&env, "MAIN").join("in/s.json"), None).unwrap_err().contains("apiKeyHelper"));
    }

    #[test]
    fn rules_with_the_harness_stay_under_the_context_limit_for_every_role() {
        let (_td, env) = test_env();
        make_fleet(&env);
        for (i, part) in ["a", "b", "c"].iter().enumerate() {
            put(&env, &format!("rules/{part}.md"), &format!("{i}{}", "y".repeat(RULES_MAX - 10)));
        }
        for id in [Id::admiral("MAIN"), Id::commodore("MAIN", "ARGOS"), Id::ship("MAIN", "ARGOS", "ARGOS")] {
            let t = crate::rules::text_for(&env, &id);
            assert!(t.chars().count() <= crate::rules::CONTEXT_MAX, "{id}: {}", t.chars().count());
            // the worst update: from v0, the whole changelog, then the rules
            let _ = std::fs::remove_file(crate::rules::delivered_path(&env));
            let u = crate::rules::update_for(&env, &id, &format!("k-{id}"), "s0", "UserPromptSubmit").unwrap();
            assert!(u.chars().count() <= crate::rules::CONTEXT_MAX && u.contains("### a"), "{id}: {}", u.chars().count());
        }
    }
}
