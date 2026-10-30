//! The format rule kept (docs/design/migrations.md section 6): every format
//! change has its step and its fixture, and the shape of every owned type is
//! pinned to its artifact's version. `NIMITZ_BLESS=1` regenerates the pinned
//! shapes (after the version, the step and the fixture are in).
use nimitz_core::formats::{self, Apply, Kept, ARTIFACTS};
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/formats")
}

/// Does `bytes` (an artifact at its current version) read with the current types?
fn parses(artifact: &str, bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let e = |e: &dyn std::fmt::Display| e.to_string();
    match artifact {
        "fleet" => toml::from_str::<nimitz_core::FleetConfig>(text).map(|_| ()).map_err(|x| e(&x)),
        "squadron" => toml::from_str::<nimitz_core::SquadronConfig>(text).map(|_| ()).map_err(|x| e(&x)),
        "registry" => nimitz_core::registry::parse(text).map(|_| ()).map_err(|x| e(&x)),
        "active" => serde_json::from_str::<nimitz_core::active::ActiveSet>(text).map(|_| ()).map_err(|x| e(&x)),
        "needs" => serde_json::from_str::<nimitz_core::notify::Notice>(text).map(|_| ()).map_err(|x| e(&x)),
        "rules-delivered" => serde_json::from_str::<std::collections::BTreeMap<String, nimitz_core::rules::Delivery>>(text).map(|_| ()).map_err(|x| e(&x)),
        "restore" => serde_json::from_str::<nimitz_core::restore::Progress>(text).map(|_| ()).map_err(|x| e(&x)),
        "events" => text.lines().try_for_each(|l| serde_json::from_str::<nimitz_core::Event>(l).map(|_| ()).map_err(|x| e(&x))),
        // Admiralty's `app::Saved` is read in crates/admiralty/tests/formats.rs
        // (core does not know the type); here: a JSON object stamped current
        "admiralty-ui" => serde_json::from_str::<serde_json::Value>(text)
            .map_err(|x| e(&x))
            .and_then(|v| (v["format"] == formats::current("admiralty-ui")).then_some(()).ok_or_else(|| format!("format {}", v["format"]))),
        other => Err(format!("no reader for artifact {other} in tests/formats.rs `parses`: add one")),
    }
}

/// For every artifact and every version N below its current one: a step
/// N -> N+1 and a fixture `tests/fixtures/formats/<artifact>/vN.*`; the chain
/// from each fixture reaches the current version, and the result parses with
/// the current types.
#[test]
fn every_format_change_has_a_step_and_a_fixture() {
    let mut missing = vec![];
    for a in ARTIFACTS.iter().filter(|a| a.kept != Kept::Own) {
        assert!(a.oldest <= a.current && a.oldest >= 1, "{}: oldest {} current {}", a.name, a.oldest, a.current);
        for s in a.steps {
            assert_eq!((s.artifact, s.scope, s.to), (a.name, a.scope, s.from + 1), "{}: a step names its artifact and its scope and moves one version", a.name);
        }
        for n in a.oldest..a.current {
            if a.step_from(n).is_none() {
                missing.push(format!("{}: no step {n} -> {} in formats::ARTIFACTS", a.name, n + 1));
            }
            let dir = fixtures().join(a.name);
            let found: Vec<PathBuf> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().is_some_and(|f| f.to_string_lossy().starts_with(&format!("v{n}."))))
                .collect();
            if found.is_empty() {
                missing.push(format!("{}: no fixture {}/v{n}.* (a file of version {n}, from a real install when one exists)", a.name, dir.display()));
            }
            for f in found {
                let mut bytes = std::fs::read(&f).unwrap();
                if let Some(v) = a.version_of(&bytes) {
                    assert_eq!(v, n, "{} is not version {n}", f.display());
                }
                let mut v = n;
                while v < a.current {
                    let Some(s) = a.step_from(v) else { break };
                    bytes = match s.apply {
                        Apply::Bytes(apply) => apply(&bytes).unwrap_or_else(|e| panic!("{} {}->{} on {}: {e}", a.name, s.from, s.to, f.display())),
                        Apply::Nothing => bytes,
                        Apply::Folder(apply) => {
                            let td = tempfile::tempdir().unwrap();
                            let p = td.path().join(f.file_name().unwrap());
                            std::fs::write(&p, &bytes).unwrap();
                            apply(td.path()).unwrap_or_else(|e| panic!("{} {}->{} on {}: {e}", a.name, s.from, s.to, f.display()));
                            std::fs::read(&p).unwrap()
                        }
                    };
                    v = s.to;
                    if let Some(got) = a.version_of(&bytes) {
                        assert_eq!(got, v, "{} {}->{} on {} stamped version {got}", a.name, s.from, s.to, f.display());
                    }
                }
                assert_eq!(v, a.current, "the chain from {} stops at {v}", f.display());
                if let Err(e) = parses(a.name, &bytes) {
                    missing.push(format!("{}: the chain from {} does not parse with the current types: {e}", a.name, f.display()));
                }
                // idempotent: the chain's last step on its own output changes nothing
                if let Some(Apply::Bytes(apply)) = a.step_from(a.current - 1).map(|s| s.apply) {
                    assert_eq!(apply(&bytes).unwrap(), bytes, "{}: the last step on its own output", a.name);
                }
            }
        }
    }
    assert!(missing.is_empty(), "a format moved without its step or its fixture:\n  {}", missing.join("\n  "));
}

/// `tests/fixtures/formats/shapes/<artifact>.v<version>.<ext>` holds `text`;
/// `NIMITZ_BLESS=1` writes it (and drops the artifact's shapes of other versions).
pub fn pin(shapes: &Path, artifact: &str, ext: &str, ty: &str, text: &str) -> Option<String> {
    let v = formats::current(artifact);
    let p = shapes.join(format!("{artifact}.v{v}.{ext}"));
    if std::env::var("NIMITZ_BLESS").is_ok_and(|b| b == "1") {
        std::fs::create_dir_all(shapes).unwrap();
        for e in std::fs::read_dir(shapes).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let other = n.strip_prefix(&format!("{artifact}.v")).and_then(|r| r.split('.').next()).and_then(|m| m.parse::<u32>().ok()).is_some_and(|m| m != v);
            if other {
                std::fs::remove_file(e.path()).unwrap();
            }
        }
        std::fs::write(&p, text).unwrap();
        return None;
    }
    // (a checkout with CRLF line ends holds the same shape)
    match std::fs::read_to_string(&p).map(|g| g.replace("\r\n", "\n")) {
        Ok(golden) if golden == text => None,
        Ok(golden) => Some(format!(
            "the shape of {ty} changed (artifact {artifact}, version {v}): bump {artifact}'s version in formats::ARTIFACTS, add the step {v} -> {}, add the fixture tests/fixtures/formats/{artifact}/v{v}.* (a file as version {v} wrote it), then run NIMITZ_BLESS=1 cargo test --test formats to regenerate {}\n--- golden\n{golden}\n--- now\n{text}",
            v + 1,
            p.display()
        )),
        Err(_) => Some(format!("no pinned shape {} for {ty}: run NIMITZ_BLESS=1 cargo test --test formats", p.display())),
    }
}

fn json<T: serde::de::DeserializeOwned + serde::Serialize>(example: serde_json::Value) -> String {
    let t: T = serde_json::from_value(example).unwrap();
    serde_json::to_string_pretty(&t).unwrap() + "\n"
}

fn toml_of<T: serde::de::DeserializeOwned + serde::Serialize>(example: &str) -> String {
    let t: T = toml::from_str(example).unwrap();
    toml::to_string_pretty(&t).unwrap()
}

/// Every `Role`: the match fails to compile when one is added, so the
/// registry example below gets the new one (and its golden moves).
fn every_role() -> Vec<nimitz_core::Role> {
    use nimitz_core::Role;
    let all = vec![Role::Admiral, Role::Commodore, Role::Ship];
    for r in &all {
        match r {
            Role::Admiral | Role::Commodore | Role::Ship => {}
        }
    }
    all
}

/// Every `Comms` form, the same way.
/// A populated example of each owned type, serialized, against the golden
/// named for its artifact and current version: a field added, renamed or
/// dropped moves the golden, and the message says what to do.
#[test]
fn formats_shapes_are_pinned() {
    use serde_json::json;
    let shapes = fixtures().join("shapes");
    let fleet = r#"
format = 2
min_nimitz = "0.5.0"
fleet = "MAIN"
[machines.work]
hostname = "west-mac"
os = "macos"
[admiral]
permission_mode = "plan"
"#;
    let squadron = r#"
format = 2
min_nimitz = "0.7.12"
squadron = "ARGOS"
brief = "SQUADRON.md"
comms = "open"
permission_mode = "acceptEdits"
effort = "high"
[[ships]]
name = "CLEAN"
prompt = "hi"
prompt_file = "p.md"
depends_on = ["MAIN/ARGOS/OTHER"]
effort = "max"
[ships.dirs]
work = "managed"
[ships.permission_mode]
work = "plan"
"#;
    let mut registry = json!({
        "version": 3, "min_nimitz": "0.5.0", "machine": "work", "updated_at": 1.5,
        "identities": {"MAIN/ARGOS/CLEAN": {"role": "ship", "name": "[MAIN.ARGOS]-CLEAN", "cwd": "/w", "job": "ab12cd34", "session": "s2",
            "lineage": [{"session_id": "s1", "job": "ab12cd34", "via": "launch", "at": 1.0, "home": "/h"}, {"session_id": "s2", "job": "ab12cd34", "via": "clear", "at": 2.0, "home": "/h"}]}},
        "retired": {"MAIN/ARGOS/~t": {"role": "ship", "name": "t", "cwd": "/w", "session": "s0", "lineage": []}},
    });
    // one record of every role
    for role in every_role() {
        let id = match role {
            nimitz_core::Role::Admiral => nimitz_core::Id::admiral("MAIN"),
            nimitz_core::Role::Commodore => nimitz_core::Id::commodore("MAIN", "ARGOS"),
            nimitz_core::Role::Ship => nimitz_core::Id::ship("MAIN", "ARGOS", "DRIFT"),
        };
        registry["identities"][&id.0] = json!({"role": role, "name": id.name(), "cwd": "/w", "job": "j", "session": "s", "lineage": [{"session_id": "s", "job": "j", "via": "launch", "at": 1.0}]});
    }
    let active = json!({"version": 1, "at": 1.5, "machine": "work", "herdr": {"session": "NIMITZ", "socket": "/s"},
        "members": {"MAIN": {"job": "ab12cd34", "session": "s1", "status": "idle", "waiting_for": "you", "attached": true,
            "place": {"workspace_id": "w1", "workspace": "MAIN", "tab_id": "t1", "tab": "MAIN", "pane_id": "p1"}}}});
    let notice = json!({"ts": 1.5, "id": "MAIN/@KB", "session_id": "s1", "kind": "dead", "event": "inbox", "tool_use_id": "t1", "detail": "d",
        "delivery": "delivered", "delivery_note": "n", "delivered_at": 2.0, "item": "000000000017"});
    let delivered = json!({"ab12cd34": {"version": 5, "at": 1.5, "session_id": "s1", "via": "SessionStart", "harness": "ff"}});
    let progress = json!({"run": "r1", "started": 1.5, "fleets": ["MAIN"], "total": 3, "done": 1, "steps": ["a", "b", "c"]});
    let event = json!({"ts": 1.5, "event": "SessionStart", "session_id": "s1", "source": "startup", "reason": "r", "agent_id": "a", "agent_type": "t",
        "tool_name": "SendMessage", "to": "x", "pid": 42, "labels": {"NIMITZ_ID": "MAIN"}, "rules": 5, "job": "ab12cd34", "kind": "bg", "entrypoint": "cli",
        "binding": "bound", "cwd": "/w"});
    // (`comms` is read and never written, 0.7.3: an older nimitz reads the
    // missing field as its default, so no version step)
    let squadrons = toml_of::<nimitz_core::SquadronConfig>(squadron);
    let main = nimitz_core::Id::admiral("MAIN");
    let shown = [
        ("fleet", "toml", "FleetConfig", toml_of::<nimitz_core::FleetConfig>(fleet)),
        ("squadron", "toml", "SquadronConfig", squadrons),
        ("fleet-settings", "json", "fleet_settings::render", {
            let mut m = serde_json::Map::new();
            nimitz_core::fleet_settings::set(&mut m, "kb.remote", Some(json!("git@github.com:u/kb.git"))).unwrap();
            nimitz_core::fleet_settings::set(&mut m, "kb.dirs.work", Some(json!("~/kb"))).unwrap();
            nimitz_core::fleet_settings::render(&m)
        }),
        ("legacy", "toml", "config::legacy_toml", nimitz_core::config::legacy_toml(&[("[GENERAL]-HOME".to_string(), nimitz_core::Id::ship("MAIN", "GENERAL", "HOME"))].into())),
        ("ship-lock", "json", "sessions::ship_lock_line", nimitz_core::sessions::ship_lock_line("work", &nimitz_core::Id::ship("MAIN", "ARGOS", "CLEAN"), "s1", Some(42), 1.5).to_string() + "\n"),
        ("pending", "json", "sessions::pending_record", nimitz_core::sessions::pending_record(&main, "s1", 1.5).to_string() + "\n"),
        ("inbox-records", "json", "inbox::record_value", nimitz_core::inbox::record_value("/tmp/s.sock", 42, 1.5).to_string() + "\n"),
        ("own-pins", "json", "pins::list_text", nimitz_core::pins::list_text(&["ab12cd34".into(), "ef56ab78".into()]) + "\n"),
        ("registry", "json", "Registry", nimitz_core::registry::render(&nimitz_core::registry::parse(&registry.to_string()).unwrap()).unwrap() + "\n"),
        ("active", "json", "active::ActiveSet", json::<nimitz_core::active::ActiveSet>(active)),
        ("needs", "json", "notify::Notice", json::<nimitz_core::notify::Notice>(notice)),
        ("rules-delivered", "json", "rules::Delivery", json::<std::collections::BTreeMap<String, nimitz_core::rules::Delivery>>(delivered)),
        ("restore", "json", "restore::Progress", json::<nimitz_core::restore::Progress>(progress)),
        ("events", "json", "Event", json::<nimitz_core::Event>(event)),
        ("machines", "toml", "provider::Machines", toml_of::<nimitz_core::provider::Machines>("[machines.work]\nclaude_bin = \"/opt/claude\"\nprovider = \"bedrock\"\n")),
        ("provider-records", "json", "provider::Record", {
            let r: nimitz_core::provider::Record = serde_json::from_value(json!({"id": "MAIN", "session_id": "s1", "at": 1.5, "model": "us.anthropic.claude-opus-5-5",
                "env": {"CLAUDE_CODE_USE_BEDROCK": "1", "AWS_REGION": "us-east-1"}, "provider": "bedrock"})).unwrap();
            nimitz_core::provider::record_text(&r) + "\n"
        }),
    ];
    let failures: Vec<String> = shown.iter().filter_map(|(a, ext, ty, text)| pin(&shapes, a, ext, ty, text)).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Every squadron.toml written before 0.7.3 holds `comms`: each of its forms
/// still parses (read and ignored), and nothing writes it again.
#[test]
fn an_old_squadron_comms_still_parses_and_is_never_written() {
    for comms in [r#""open""#, r#""commodore-only""#, r#"{ allow = ["MAIN/ARGOS/CLEAN", "MAIN/BOREAL"] }"#] {
        let text = format!("squadron = \"ARGOS\"\nbrief = \"SQUADRON.md\"\ncomms = {comms}\n");
        let sq: nimitz_core::SquadronConfig = toml::from_str(&text).unwrap_or_else(|e| panic!("{comms}: {e}"));
        assert_eq!(sq.squadron, "ARGOS");
        assert!(!toml::to_string_pretty(&sq).unwrap().contains("comms"), "{comms}");
    }
}
