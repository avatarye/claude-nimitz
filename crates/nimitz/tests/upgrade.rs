//! An update from nimitz 0.4.0: a real 0.4.0 install's state
//! (`nimitz-core/tests/fixtures/formats/install-0.4.0`, made by the 0.4.0
//! binary in a sandbox) under this build (docs/design/migrations.md section
//! 7). Every run clears the process env first, so nothing here can reach the
//! real `~/.claude`, `~/.claude-nimitz` or `~/.local/state/nimitz`.
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const EXE: &str = env!("CARGO_BIN_EXE_nimitz");
const V: &str = env!("CARGO_PKG_VERSION");

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn raw(vars: &BTreeMap<String, String>, args: &[&str], extra: &[(&str, &str)], stdin: Option<&str>) -> Run {
    let mut c = Command::new(EXE);
    c.env_clear().env("PATH", "/usr/bin:/bin").env("NIMITZ_NOTIFY_RETRY", "0").envs(vars).args(args);
    for (k, v) in extra {
        c.env(k, v);
    }
    c.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().expect("spawn nimitz");
    if let Some(s) = stdin {
        let _ = child.stdin.take().unwrap().write_all(s.as_bytes());
    }
    let o = child.wait_with_output().unwrap();
    Run { code: o.status.code().unwrap_or(-1), out: String::from_utf8_lossy(&o.stdout).into_owned(), err: String::from_utf8_lossy(&o.stderr).into_owned() }
}

/// A sandbox world: made by this build, then (`old`) its fleets, its state
/// and its session files replaced by the 0.4.0 install's, with `@@BASE@@`
/// as this world's path. The sandbox's own job rows, transcripts and
/// folders stay: the 0.4.0 world was made the same way (the same ids).
struct World {
    _td: tempfile::TempDir,
    base: PathBuf,
    vars: BTreeMap<String, String>,
}

impl World {
    fn new(old: bool) -> World {
        let td = tempfile::tempdir().unwrap();
        let base = td.path().join("world");
        let r = raw(&BTreeMap::new(), &["sandbox", "create", base.to_str().unwrap()], &[], None);
        assert_eq!(r.code, 0, "{}{}", r.out, r.err);
        let base = nimitz_core::os::canonical(&base).unwrap();
        let e = raw(&BTreeMap::new(), &["sandbox", "env", base.to_str().unwrap(), "--json"], &[], None);
        let vars: BTreeMap<String, String> = serde_json::from_str(e.out.trim()).expect("sandbox env --json");
        if old {
            for d in ["nimitz", "state", "claude_home/sessions"] {
                std::fs::remove_dir_all(base.join(d)).unwrap();
            }
            let from = fixture();
            for (rel, bytes) in files(&from) {
                let to = base.join(&rel);
                std::fs::create_dir_all(to.parent().unwrap()).unwrap();
                let at = base.to_string_lossy();
                match String::from_utf8(bytes.clone()) {
                    Ok(text) => std::fs::write(&to, text.replace("@@BASE@@", &at)).unwrap(),
                    Err(_) => std::fs::write(&to, bytes).unwrap(),
                }
            }
            let _ = std::fs::remove_file(base.join("README.md"));
        }
        World { _td: td, base, vars }
    }
    fn run(&self, args: &[&str]) -> Run {
        raw(&self.vars, args, &[], None)
    }
    fn state(&self, rel: &str) -> PathBuf {
        self.base.join("state").join(rel)
    }
    fn json(&self, p: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(p).unwrap_or_default()).unwrap_or(Value::Null)
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../nimitz-core/tests/fixtures/formats/install-0.4.0")
}

/// Every file under `dir`: (path relative to it, bytes).
fn files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(root: &Path, d: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.push((p.strip_prefix(root).unwrap().to_path_buf(), std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut v = vec![];
    walk(dir, dir, &mut v);
    v.sort();
    v
}

fn member<'a>(status: &'a Value, id: &str) -> Option<&'a Value> {
    for f in status["fleets"].as_array()? {
        if f["admiral"]["id"] == id {
            return Some(&f["admiral"]);
        }
        for sq in f["squadrons"].as_array()? {
            if sq["commodore"]["id"] == id {
                return Some(&sq["commodore"]);
            }
            if let Some(m) = sq["ships"].as_array()?.iter().find(|m| m["id"] == id) {
                return Some(m);
            }
        }
    }
    None
}

/// The 0.4.0 install under this build: the first command migrates (one
/// stderr line), the manifest records it, the backup is the 0.4.0 file, and
/// every member and its lineage is as it was; `check` and `start --plan`
/// then run on it with nothing more to migrate.
#[test]
fn a_0_4_0_install_migrates_on_its_first_command_then_runs() {
    let w = World::new(true);
    let reg = w.state("registry.json");
    let before = std::fs::read(&reg).unwrap();
    let old: Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(old["version"], 2, "a 0.4.0 registry");
    assert!(!w.state("formats.json").exists());

    let s = w.run(&["status", "--json"]);
    assert_eq!(s.code, 0, "{}{}", s.out, s.err);
    assert_eq!(s.err.lines().filter(|l| l.contains("migrated")).collect::<Vec<_>>(), [format!("[OK] nimitz {V}: migrated registry 2->3 (backups *.pre-{V}; nimitz migrate --plan)")], "{}", s.err);
    // the manifest: this nimitz ran here, every manifest artifact at its version
    let m = w.json(&w.state("formats.json"));
    assert_eq!(m["nimitz"], V);
    assert_eq!(m["artifacts"]["needs"], 2);
    // the backup is the 0.4.0 file, word for word; the log says what ran
    assert_eq!(std::fs::read(w.state(&format!("registry.json.pre-{V}"))).unwrap(), before);
    let log = std::fs::read_to_string(w.state("migrations.log")).unwrap();
    let lines: Vec<Value> = log.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.iter().map(|l| (l["artifact"].as_str().unwrap(), l["from"].as_u64().unwrap(), l["to"].as_u64().unwrap(), l["ok"].as_bool().unwrap())).collect::<Vec<_>>(), [("registry", 2, 3, true)]);
    // the registry: stamped, every identity and its lineage as it was
    let now = w.json(&reg);
    assert_eq!((now["version"].as_u64(), now["min_nimitz"].as_str()), (Some(3), Some("0.5.0")));
    assert_eq!(now["identities"], old["identities"], "members and lineage intact");
    // status shows the 0.4.0 members on their sessions
    let st: Value = serde_json::from_str(s.out.trim()).unwrap();
    for id in ["MAIN", "MAIN/ARGOS", "MAIN/ARGOS/CLEAN", "MAIN/ARGOS/STOPPED"] {
        let m = member(&st, id).unwrap_or_else(|| panic!("no member {id}"));
        assert_eq!(m["session"]["session_id"], old["identities"][id]["session"], "{id}: {m}");
    }
    // this machine is on record in its fleet (shared)
    let rec = w.json(&w.base.join("nimitz/main/machines/sandbox.json"));
    assert_eq!(rec["nimitz"], V);
    // nothing more to migrate: check and start --plan say no more about it
    for args in [&["check"][..], &["start", "--plan"], &["start", "MAIN/ARGOS/CLEAN", "--plan"]] {
        let r = w.run(args);
        assert!(r.code == 0 || r.code == 1, "{args:?}: {}{}", r.out, r.err);
        assert!(!r.err.contains("migrat") && !r.out.contains("format.failed") && !r.out.contains("config.too_new"), "{args:?}: {}{}", r.out, r.err);
    }
    let again = w.run(&["status"]);
    assert!(!again.err.contains("migrated"), "{}", again.err);
    assert_eq!(w.json(&reg)["identities"], old["identities"]);
    assert_eq!(std::fs::read(w.state(&format!("registry.json.pre-{V}"))).unwrap(), before, "the backup is taken once");
    // no fleet here holds an aide: nothing shared is due
    let plan = w.run(&["migrate", "--plan"]);
    assert_eq!(plan.code, 0, "{}{}", plan.out, plan.err);
    assert!(plan.out.contains("nothing to migrate"), "{}", plan.out);
}

/// The hook is on Claude Code's hot path: it never migrates, and it reads
/// the 0.4.0 files as they are.
#[test]
fn the_hook_never_migrates() {
    let w = World::new(true);
    let before = std::fs::read(w.state("registry.json")).unwrap();
    let r = raw(&w.vars, &["hook", "SessionStart"], &[], Some("{}"));
    assert_eq!(r.code, 0);
    assert!(!w.state("formats.json").exists());
    assert_eq!(std::fs::read(w.state("registry.json")).unwrap(), before);
    assert!(!w.base.join("nimitz/main/machines").exists());
}

/// A newer nimitz ran here (its registry, its mark in the manifest): every
/// command but doctor stops, and nothing of it is rewritten.
#[test]
fn a_newer_nimitz_state_stops_every_command_but_doctor() {
    let w = World::new(false);
    let reg = w.state("registry.json");
    let mut v = w.json(&reg);
    v["version"] = Value::from(nimitz_core::REGISTRY_VERSION + 1);
    v["min_nimitz"] = Value::from("99.0.0");
    std::fs::write(&reg, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let mut m = w.json(&w.state("formats.json"));
    m["nimitz"] = Value::from("99.0.0");
    std::fs::write(w.state("formats.json"), m.to_string()).unwrap();
    let raw_reg = std::fs::read(&reg).unwrap();
    let s = w.run(&["status"]);
    assert_eq!(s.code, 1, "{}{}", s.out, s.err);
    assert!(s.err.contains(&format!("{} needs nimitz >= 99.0.0 (this machine runs {V}): update nimitz here", reg.display())), "{}", s.err);
    let d = w.run(&["doctor"]);
    assert!(d.out.contains("needs nimitz >= 99.0.0"), "{}{}", d.out, d.err);
    assert_eq!(std::fs::read(&reg).unwrap(), raw_reg, "never rewritten");
    assert_eq!(w.json(&w.state("formats.json"))["nimitz"], "99.0.0", "the newer mark stays");
    assert!(!std::fs::read_dir(w.base.join("state")).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("corrupt")));
}

/// A 0.6 fleet that holds `[[aide]]` (format 2): it loads, the shared
/// cleanup is due, said by `migrate --plan`, doctor and check, and run only
/// by `migrate --shared --yes` (yours alone): the tables and their stamp
/// go, the rest of the file stays; a fleet stamped for a newer nimitz is
/// refused.
#[test]
fn the_aides_cleanup_waits_for_migrate_shared_yes() {
    let w = World::new(false);
    let fleet_toml = w.base.join("nimitz/main/fleet.toml");
    let kb = w.base.join("dirs/main/kb");
    std::fs::create_dir_all(&kb).unwrap();
    let ok = w.run(&["machine", "add", "MAIN", "far", "--hostname", "far-box", "--yes"]);
    assert_eq!(ok.code, 0, "{}{}", ok.out, ok.err);
    let before = std::fs::read_to_string(&fleet_toml).unwrap();
    let text = format!("format = 2\nmin_nimitz = \"0.5.0\"\n{before}\n[[aide]]\nname = \"KB\"\n\n[aide.dirs]\nsandbox = {:?}\n", kb.to_string_lossy());
    std::fs::write(&fleet_toml, &text).unwrap();

    let plan = w.run(&["migrate", "--plan"]);
    assert_eq!(plan.code, 0, "{}{}", plan.out, plan.err);
    assert!(plan.out.contains(&format!("  shared  fleet: aides dropped  {}", fleet_toml.display())), "{}", plan.out);
    assert!(plan.out.contains("affects every machine"), "{}", plan.out);
    let doctor = w.run(&["doctor"]);
    assert!(doctor.out.contains("[WARN] formats: fleet: aides dropped due on") && doctor.out.contains("fix: nimitz migrate --shared --yes"), "{}", doctor.out);
    assert!(doctor.out.contains("[WARN] MAIN: machine far: no nimitz >= 0.5.0 seen; update it"), "{}", doctor.out);
    let check = w.run(&["check", "MAIN"]);
    assert!(check.out.contains("aide.legacy") && !check.out.contains("format.shared"), "{}", check.out);
    // the local run, and a member, never touch it
    let local = w.run(&["migrate", "--yes"]);
    assert_eq!(local.code, 0, "{}{}", local.out, local.err);
    assert!(local.out.contains("migrate --shared --yes"), "{}", local.out);
    let member = raw(&w.vars, &["migrate", "--shared", "--yes"], &[("NIMITZ_ID", "MAIN"), ("NIMITZ_ROLE", "admiral")], None);
    assert_eq!(member.code, 1, "{}{}", member.out, member.err);
    assert!(member.out.contains("yours alone"), "{}", member.out);
    assert_eq!(std::fs::read_to_string(&fleet_toml).unwrap(), text);
    // yours: the aides and their stamp gone, the rest as it was, the backup beside it
    let shared = w.run(&["migrate", "--shared", "--yes"]);
    assert_eq!(shared.code, 0, "{}{}", shared.out, shared.err);
    assert!(shared.out.contains(&format!("[OK] fleet: aides dropped {}", fleet_toml.display())), "{}", shared.out);
    assert_eq!(std::fs::read_to_string(&fleet_toml).unwrap(), format!("{before}\n"));
    assert_eq!(std::fs::read_to_string(w.base.join(format!("nimitz/main/fleet.toml.pre-{V}"))).unwrap(), text);
    assert!(w.run(&["migrate", "--plan"]).out.contains("nothing to migrate"));
    assert!(!w.run(&["check", "MAIN"]).out.contains("aide.legacy"));
    // a later nimitz's stamp: refused before the strict parse, never rewritten
    let future = format!("min_nimitz = \"99.0.0\"\nfuture_key = 1\n{before}");
    std::fs::write(&fleet_toml, &future).unwrap();
    let st = w.run(&["status"]);
    assert!(st.out.contains(&format!("{} needs nimitz >= 99.0.0 (this machine runs {V}): update nimitz here", fleet_toml.display())), "{}{}", st.out, st.err);
    let set = w.run(&["fleet", "set", "MAIN", "--admiral-mode", "plan", "--yes"]);
    assert_ne!(set.code, 0, "{}{}", set.out, set.err);
    assert_eq!(std::fs::read_to_string(&fleet_toml).unwrap(), future);
}

/// The manifest lost beside a registry a newer nimitz wrote: no command
/// marks this machine migrated, every one stops (a registry too new is never
/// read as an empty one: a start would launch fresh sessions), and that
/// holds even with the mark in place by hand. doctor reports it.
#[test]
fn a_too_new_registry_stops_every_command_even_without_or_with_the_mark() {
    let w = World::new(false);
    let reg = w.state("registry.json");
    let mut v = w.json(&reg);
    v["version"] = Value::from(nimitz_core::REGISTRY_VERSION + 1);
    std::fs::write(&reg, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    std::fs::remove_file(w.state("formats.json")).unwrap();
    let raw_reg = std::fs::read(&reg).unwrap();
    let jobs = || std::fs::read_dir(w.base.join("claude_home/jobs")).unwrap().count();
    let before = jobs();
    for _ in 0..2 {
        let s = w.run(&["status"]);
        assert_eq!(s.code, 1, "{}{}", s.out, s.err);
        assert!(s.err.contains("is format 4, newer than this nimitz reads"), "{}", s.err);
        assert!(w.json(&w.state("formats.json"))["nimitz"].as_str().unwrap_or("").is_empty(), "never marked migrated");
    }
    // the mark set by hand: the first read of the registry stops the command
    std::fs::write(w.state("formats.json"), serde_json::json!({"nimitz": V, "artifacts": {}}).to_string()).unwrap();
    for args in [&["status"][..], &["start", "MAIN/ARGOS/FRESH", "--yes"], &["start", "--plan"]] {
        let r = w.run(args);
        assert_eq!(r.code, 1, "{args:?}: {}{}", r.out, r.err);
        assert!(r.err.contains("config.too_new") && r.err.contains("never reads this registry as an empty one"), "{args:?}: {}", r.err);
    }
    let d = w.run(&["doctor"]);
    assert!(d.out.contains("is format 4, newer than this nimitz reads"), "{}{}", d.out, d.err);
    assert_eq!(std::fs::read(&reg).unwrap(), raw_reg);
    assert_eq!(jobs(), before, "nothing launched");
}

/// A manifest entry a newer nimitz recorded (a format this one does not
/// know) stops every command but doctor too.
#[test]
fn a_too_new_manifest_entry_stops_every_command_but_doctor() {
    let w = World::new(false);
    std::fs::write(w.state("formats.json"), serde_json::json!({"nimitz": "9.0.0", "artifacts": {"needs": 9}}).to_string()).unwrap();
    let s = w.run(&["status"]);
    assert_eq!(s.code, 1, "{}{}", s.out, s.err);
    assert!(s.err.contains("formats.json is format 9, newer than this nimitz reads"), "{}", s.err);
    let d = w.run(&["doctor"]);
    assert!(d.out.contains("formats.json is format 9"), "{}{}", d.out, d.err);
    assert_eq!(w.json(&w.state("formats.json"))["artifacts"]["needs"], 9, "never lowered");
}
