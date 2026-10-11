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
//! - `settings/fleet.json`, `settings/admiral.json`, `settings/<SQ>.json`,
//!   `settings/<SQ>/COMMODORE.json`, `settings/<SQ>/<SHIP>.json`: merged in
//!   that order (fleet, squadron, member) under nimitz's own `--settings`
//!   (its labels and its allow rule win), at the next launch.
//!
//! Settings that run programs or steer nimitz are refused (`FORBIDDEN`).
use crate::env::Env;
use crate::types::*;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// The most characters of harness rules a member is given (with its role
/// rules, under Claude Code's 10,000 for a hook's context).
pub const RULES_MAX: usize = 4000;

/// Settings keys a harness never sets: each runs a program, or is nimitz's.
pub const FORBIDDEN: [&str; 7] = ["hooks", "apiKeyHelper", "awsAuthRefresh", "awsCredentialExport", "otelHeadersHelper", "statusLine", "disableAllHooks"];

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
        let Ok(t) = std::fs::read_to_string(d.join(format!("{n}.md"))) else { continue };
        text.push_str(&format!("\n### {n}\n\n{}\n", t.trim()));
    }
    if text.is_empty() {
        return None;
    }
    let mut body = format!("\n## Fleet {fleet}'s own rules (its harness)\n{text}");
    if body.chars().count() > RULES_MAX {
        body = body.chars().take(RULES_MAX).collect::<String>() + "\n... [cut: `nimitz harness list` shows the rest]\n";
    }
    let hash = crate::sha256::hex(body.as_bytes());
    Some((body, hash))
}

/// The settings files that apply to `id`, in merge order.
pub fn settings_files(env: &Env, id: &Id) -> Vec<PathBuf> {
    let s = dir(env, id.fleet()).join("settings");
    let mut v = vec![s.join("fleet.json")];
    match (id.role(), id.squadron()) {
        (Role::Admiral, _) => v.push(s.join("admiral.json")),
        (Role::Commodore, Some(sq)) => v.extend([s.join(format!("{sq}.json")), s.join(sq).join("COMMODORE.json")]),
        (Role::Ship, Some(sq)) => v.extend([s.join(format!("{sq}.json")), s.join(sq).join(format!("{}.json", id.parts().get(2).copied().unwrap_or("")))]),
        _ => {}
    }
    v
}

/// What is wrong with one settings object (empty: nothing).
pub fn settings_problems(v: &Value) -> Vec<String> {
    let Some(m) = v.as_object() else { return vec!["not a JSON object".into()] };
    let mut out: Vec<String> = FORBIDDEN.iter().filter(|k| m.contains_key(**k)).map(|k| format!("{k}: a harness never sets it (it runs a program, or is nimitz's)")).collect();
    if let Some(env) = m.get("env").and_then(Value::as_object) {
        for k in env.keys().filter(|k| k.starts_with("NIMITZ_") || k.starts_with("HERDR_") || k.starts_with("CLAUDE_CODE_") || *k == "CLAUDECODE") {
            out.push(format!("env.{k}: nimitz's or Claude Code's own, never a harness's"));
        }
    }
    out
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

/// The harness settings for `id`, merged; a file that does not read or that
/// sets what a harness may not is left out (`issues` says so).
pub fn settings_for(env: &Env, id: &Id) -> Value {
    let mut out = Value::Object(Map::new());
    for f in settings_files(env, id) {
        let Some(v) = std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { continue };
        if settings_problems(&v).is_empty() {
            merge(&mut out, &v);
        }
    }
    out
}

/// The plugin nimitz makes from the harness's agents and skills, on this
/// machine (None: the harness has none). Made again when its content
/// changed, and left as it is otherwise (a running member reads it).
pub fn plugin(env: &Env, fleet: &str) -> Option<PathBuf> {
    let (agents, skills) = (names(env, fleet, Kind::Agent), names(env, fleet, Kind::Skill));
    if agents.is_empty() && skills.is_empty() {
        return None;
    }
    let h = dir(env, fleet);
    let mut files: Vec<(String, Vec<u8>)> = vec![];
    for a in &agents {
        files.push((format!("agents/{a}.md"), std::fs::read(h.join("agents").join(format!("{a}.md"))).ok()?));
    }
    for s in &skills {
        let sd = h.join("skills").join(s);
        for e in std::fs::read_dir(&sd).ok()?.flatten().filter(|e| e.path().is_file()) {
            let n = e.file_name().to_string_lossy().into_owned();
            files.push((format!("skills/{s}/{n}"), std::fs::read(e.path()).ok()?));
        }
    }
    let manifest = serde_json::json!({"name": plugin_name(fleet), "version": "1.0.0", "description": format!("fleet {fleet}'s harness: its agents and skills (made by nimitz from {})", h.display())});
    files.push((".claude-plugin/plugin.json".into(), serde_json::to_vec_pretty(&manifest).ok()?));
    files.sort();
    let out = env.state.join("harness").join(fleet).join("plugin");
    let stamp: String = crate::sha256::hex(&files.iter().flat_map(|(n, b)| n.as_bytes().iter().chain(b.iter()).copied().chain([0u8])).collect::<Vec<u8>>());
    if std::fs::read_to_string(out.join(".nimitz-stamp")).ok().as_deref() == Some(stamp.as_str()) {
        return Some(out);
    }
    // made beside it, then swapped in
    let tmp = out.with_file_name(format!("plugin.tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    for (n, b) in &files {
        let p = tmp.join(n);
        std::fs::create_dir_all(p.parent()?).ok()?;
        std::fs::write(&p, b).ok()?;
    }
    std::fs::write(tmp.join(".nimitz-stamp"), &stamp).ok()?;
    let old = out.with_file_name(format!("plugin.old-{}", std::process::id()));
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

/// The harness's problems (`check`): settings that do not read or set what
/// a harness may not, rules over `RULES_MAX`, an agent without frontmatter.
pub fn issues(env: &Env, fleet: &str) -> Vec<Issue> {
    let mut out = vec![];
    let fid = Id::admiral(fleet);
    let mut warn = |m: String| out.push(Issue { code: "harness.invalid".into(), severity: Severity::Warn, message: m, fix: Some(format!("nimitz harness list --fleet {fleet}")), id: Some(fid.clone()) });
    let sd = dir(env, fleet).join("settings");
    let mut stack = vec![sd];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "json") {
                continue; // (an archived file, a note)
            }
            match std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                None => warn(format!("{} does not read as JSON: left out of every launch", p.display())),
                Some(v) => {
                    for pr in settings_problems(&v) {
                        warn(format!("{}: {pr}: the file is left out of every launch", p.display()));
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
        let t = std::fs::read_to_string(dir(env, fleet).join("agents").join(format!("{a}.md"))).unwrap_or_default();
        if !t.starts_with("---") || !t.contains("\ndescription:") {
            warn(format!("agent {a}: no frontmatter with a description (Claude Code skips it)"));
        }
    }
    out
}

/// What a harness item becomes on disk: (published path under the harness,
/// its content), checked. `src` is a file (a rule, an agent, settings) or a
/// skill's folder (its files, flat).
pub fn item_files(kind: Kind, name: &str, src: &Path, scope: Option<&Id>, fleet: &str) -> Result<Vec<(PathBuf, String)>, String> {
    if !valid_name(name) {
        return Err(format!("{name:?}: a name is 1 to 64 of A-Z a-z 0-9 _ -"));
    }
    let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
    match kind {
        Kind::Rule => {
            let t = read(src)?;
            if t.chars().count() > RULES_MAX {
                return Err(format!("{}: {} characters; the harness rules hold {RULES_MAX} at most", src.display(), t.chars().count()));
            }
            Ok(vec![(PathBuf::from("rules").join(format!("{name}.md")), t)])
        }
        Kind::Agent => {
            let t = read(src)?;
            if !t.starts_with("---") || !t.contains("\ndescription:") {
                return Err(format!("{}: an agent starts with frontmatter (name, description)", src.display()));
            }
            if t.split("\n---").next().is_some_and(|fm| fm.contains("\nhooks:")) {
                return Err(format!("{}: a plugin agent's hooks are ignored by Claude Code; a harness agent has none", src.display()));
            }
            Ok(vec![(PathBuf::from("agents").join(format!("{name}.md")), t)])
        }
        Kind::Skill => {
            if !src.join("SKILL.md").is_file() {
                return Err(format!("{}: a skill is a folder with a SKILL.md", src.display()));
            }
            let mut v = vec![];
            for e in std::fs::read_dir(src).map_err(|e| e.to_string())?.flatten() {
                let p = e.path();
                if p.is_dir() {
                    return Err(format!("{}: a harness skill's files sit in its folder (no subfolders)", p.display()));
                }
                v.push((PathBuf::from("skills").join(name).join(e.file_name()), read(&p)?));
            }
            v.sort();
            Ok(v)
        }
        Kind::Settings => {
            let t = read(src)?;
            let v: Value = serde_json::from_str(&t).map_err(|e| format!("{}: {e}", src.display()))?;
            let pr = settings_problems(&v);
            if !pr.is_empty() {
                return Err(format!("{}: {}", src.display(), pr.join("; ")));
            }
            Ok(vec![(settings_rel(scope, fleet)?, t)])
        }
    }
}

/// Where the settings for `scope` go (None or the fleet: `fleet.json`).
pub fn settings_rel(scope: Option<&Id>, fleet: &str) -> Result<PathBuf, String> {
    let s = PathBuf::from("settings");
    let Some(id) = scope else { return Ok(s.join("fleet.json")) };
    if id.fleet() != fleet {
        return Err(format!("{id} is not in fleet {fleet}"));
    }
    Ok(match (id.role(), id.squadron()) {
        (Role::Admiral, _) => s.join("fleet.json"),
        (Role::Commodore, Some(sq)) => s.join(format!("{sq}.json")),
        (Role::Ship, Some(sq)) => s.join(sq).join(format!("{}.json", id.parts()[2])),
        _ => return Err(format!("{id}: no settings place")),
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
        assert!(body.contains("## Fleet MAIN's own rules") && body.find("### a-safety").unwrap() < body.find("### b-style").unwrap() && !body.contains("not a rule"), "{body}");
        put(&env, "rules/b-style.md", "Write shorter.\n");
        assert_ne!(rules(&env, "MAIN").unwrap().1, h, "a change moves the hash");
        put(&env, "rules/c-long.md", &"x".repeat(RULES_MAX));
        assert!(rules(&env, "MAIN").unwrap().0.ends_with("[cut: `nimitz harness list` shows the rest]\n"));
        assert!(issues(&env, "MAIN").iter().any(|i| i.message.contains("over 4000")));
    }

    #[test]
    fn settings_merge_fleet_then_squadron_then_member_and_refuse_programs() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let ship = Id::ship("MAIN", "ARGOS", "ARGOS");
        put(&env, "settings/fleet.json", r#"{"model": "sonnet", "permissions": {"allow": ["Bash(make *)"]}, "env": {"A": "1"}}"#);
        put(&env, "settings/ARGOS.json", r#"{"permissions": {"allow": ["Bash(cargo *)"]}, "env": {"A": "2"}}"#);
        put(&env, "settings/ARGOS/ARGOS.json", r#"{"model": "opus"}"#);
        put(&env, "settings/BOREAL.json", r#"{"model": "haiku"}"#);
        let v = settings_for(&env, &ship);
        assert_eq!(v, json!({"model": "opus", "permissions": {"allow": ["Bash(make *)", "Bash(cargo *)"]}, "env": {"A": "2"}}));
        assert_eq!(settings_for(&env, &Id::admiral("MAIN"))["model"], "sonnet", "the admiral: the fleet's only");
        // a file that runs a program, or sets nimitz's env: left out, and said
        put(&env, "settings/ARGOS/ARGOS.json", r#"{"model": "opus", "hooks": {"Stop": []}}"#);
        put(&env, "settings/ARGOS.json", r#"{"env": {"NIMITZ_ID": "MAIN"}}"#);
        assert_eq!(settings_for(&env, &ship)["model"], "sonnet");
        let i = issues(&env, "MAIN");
        assert!(i.iter().any(|i| i.message.contains("hooks: a harness never sets it")) && i.iter().any(|i| i.message.contains("env.NIMITZ_ID")), "{i:?}");
        assert_eq!(settings_rel(Some(&ship), "MAIN").unwrap(), PathBuf::from("settings/ARGOS/ARGOS.json"));
        assert!(settings_rel(Some(&Id::ship("BETA", "S", "X")), "MAIN").is_err());
    }

    #[test]
    fn agents_and_skills_become_a_plugin_made_again_only_when_they_change() {
        let (_td, env) = test_env();
        make_fleet(&env);
        assert_eq!(plugin(&env, "MAIN"), None);
        put(&env, "agents/reviewer.md", "---\nname: reviewer\ndescription: reviews\n---\nReview.\n");
        put(&env, "skills/deploy/SKILL.md", "---\nname: deploy\ndescription: d\n---\nDeploy.\n");
        put(&env, "skills/deploy/notes.md", "n");
        let p = plugin(&env, "MAIN").unwrap();
        let m: Value = serde_json::from_slice(&std::fs::read(p.join(".claude-plugin/plugin.json")).unwrap()).unwrap();
        assert_eq!(m["name"], "fleet-main");
        assert!(p.join("agents/reviewer.md").is_file() && p.join("skills/deploy/notes.md").is_file());
        let made = std::fs::metadata(p.join(".nimitz-stamp")).unwrap().modified().unwrap();
        assert_eq!(plugin(&env, "MAIN").unwrap(), p);
        assert_eq!(std::fs::metadata(p.join(".nimitz-stamp")).unwrap().modified().unwrap(), made, "unchanged: left as it is");
        std::fs::remove_file(dir(&env, "MAIN").join("skills/deploy/notes.md")).unwrap();
        plugin(&env, "MAIN").unwrap();
        assert!(!p.join("skills/deploy/notes.md").exists(), "made again without it");
        assert!(item_files(Kind::Agent, "x", &dir(&env, "MAIN").join("skills/deploy/notes.md"), None, "MAIN").is_err());
        put(&env, "in/hooked.md", "---\nname: h\ndescription: d\nhooks:\n  PreToolUse: []\n---\nx\n");
        assert!(item_files(Kind::Agent, "h", &dir(&env, "MAIN").join("in/hooked.md"), None, "MAIN").unwrap_err().contains("hooks are ignored"));
        assert!(item_files(Kind::Rule, "bad name", &dir(&env, "MAIN").join("in/hooked.md"), None, "MAIN").is_err());
    }
}
