//! `nimitz` end to end against a sandbox world (fake `claude`, fake `herdr`).
//! Every run clears the process env first, so nothing here can reach the real
//! `~/.claude`, `~/.claude-nimitz` or the user's Herdr (contracts.md section 11).
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const EXE: &str = env!("CARGO_BIN_EXE_nimitz");

struct Sb {
    _td: tempfile::TempDir,
    base: PathBuf,
    vars: BTreeMap<String, String>,
}

struct Run {
    code: i32,
    out: String,
}

fn raw(vars: &BTreeMap<String, String>, args: &[&str], extra: &[(&str, &str)], stdin: Option<&str>) -> Run {
    let mut c = Command::new(EXE);
    c.env_clear().env("PATH", "/usr/bin:/bin").env("NIMITZ_BIND_TIMEOUT", "0").envs(vars).args(args);
    for (k, v) in extra {
        c.env(k, v);
    }
    c.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().expect("spawn nimitz");
    if let Some(s) = stdin {
        let _ = child.stdin.take().unwrap().write_all(s.as_bytes());
    }
    let o = child.wait_with_output().unwrap();
    let out = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    Run { code: o.status.code().unwrap_or(-1), out }
}

impl Sb {
    fn new(herdr: bool) -> Sb {
        let td = tempfile::tempdir().unwrap();
        let base = td.path().join("world");
        let mut args = vec!["sandbox", "create", base.to_str().unwrap()];
        if herdr {
            args.push("--herdr");
        }
        let r = raw(&BTreeMap::new(), &args, &[], None);
        assert_eq!(r.code, 0, "{}", r.out);
        let e = raw(&BTreeMap::new(), &["sandbox", "env", base.to_str().unwrap(), "--json"], &[], None);
        let vars: BTreeMap<String, String> = serde_json::from_str(e.out.trim()).expect("sandbox env --json");
        let base = base.canonicalize().unwrap();
        Sb { _td: td, base, vars }
    }
    fn run(&self, args: &[&str]) -> Run {
        raw(&self.vars, args, &[], None)
    }
    fn run_as(&self, id: &str, args: &[&str]) -> Run {
        let role = match id.split('/').count() {
            1 => "admiral",
            2 => "commodore",
            _ if id.contains('~') => "tender",
            _ => "ship",
        };
        raw(&self.vars, args, &[("NIMITZ_ID", id), ("NIMITZ_ROLE", role)], None)
    }
    fn status(&self) -> Value {
        let r = self.run(&["status", "--json"]);
        assert_eq!(r.code, 0, "{}", r.out);
        let start = r.out.find('{').unwrap();
        let mut de = serde_json::Deserializer::from_str(&r.out[start..]).into_iter::<Value>();
        de.next().unwrap().unwrap()
    }
    fn member(&self, id: &str) -> Value {
        let s = self.status();
        for f in s["fleets"].as_array().unwrap() {
            if f["admiral"]["id"] == id {
                return f["admiral"].clone();
            }
            for sq in f["squadrons"].as_array().unwrap() {
                if sq["commodore"]["id"] == id {
                    return sq["commodore"].clone();
                }
                for m in sq["ships"].as_array().unwrap().iter().chain(sq["tenders"].as_array().unwrap()) {
                    if m["id"] == id {
                        return m.clone();
                    }
                }
            }
        }
        panic!("no member {id}");
    }
    fn calls(&self) -> String {
        std::fs::read_to_string(self.base.join("herdr/calls.jsonl")).unwrap_or_default()
    }
    fn herdr_world(&self, f: impl FnOnce(&mut Value)) {
        let p = self.base.join("herdr/world.json");
        let mut w: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        f(&mut w);
        std::fs::write(&p, serde_json::to_vec(&w).unwrap()).unwrap();
    }
    /// Put a pane in the fake herdr that shows `member`'s live session.
    fn pane_for(&self, member: &str) {
        let sid = self.member(member)["session"]["session_id"].as_str().unwrap().to_string();
        self.herdr_world(|w| {
            w["panes"][1]["agent_session"]["value"] = Value::from(sid);
            w["panes"][1]["agent_status"] = Value::from("idle");
        });
    }
    fn files(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut v = vec![];
        walk(&self.base.join("claude_home"), &mut v);
        walk(&self.base.join("nimitz"), &mut v);
        walk(&self.base.join("state"), &mut v);
        v.sort();
        v
    }
}

fn walk(d: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push((p.clone(), std::fs::read(&p).unwrap_or_default()));
        }
    }
}

fn live(m: &Value) -> bool {
    !m["session"]["pid"].is_null()
}

// ------------------------------------------------------------------ basics

#[test]
fn help_unknown_commands_and_flags() {
    let sb = Sb::new(false);
    assert_eq!(sb.run(&["help"]).code, 0);
    assert_eq!(sb.run(&[]).code, 2);
    let r = sb.run(&["frobnicate"]);
    assert_eq!(r.code, 2);
    assert!(r.out.contains("[ERROR] unknown command"), "{}", r.out);
    let r = sb.run(&["start", "MAIN/ARGOS/CLEAN", "--bogus"]);
    assert_eq!(r.code, 2);
    assert!(r.out.contains("unknown flag --bogus"));
    assert_eq!(sb.run(&["start", "main/argos"]).code, 2, "lowercase is not an id");
    for line in sb.run(&["status"]).out.lines() {
        assert!(line.is_ascii(), "no emoji or other non-ASCII in output: {line}");
    }
}

#[test]
fn status_human_and_json() {
    let sb = Sb::new(false);
    let r = sb.run(&["status"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("[OK] machine sandbox; Claude Code 2.1.288 (supported)"), "{}", r.out);
    assert!(r.out.contains("MAIN/ARGOS/CLEAN"));
    assert!(r.out.contains("stowaways"));
    assert!(r.out.contains("(name.drift)"), "status shows session issues too:\n{}", r.out);
    let s = sb.status();
    assert_eq!(s["machine"], "sandbox");
    assert!(s["fleets"].as_array().unwrap().iter().any(|f| f["id"] == "MAIN"));
    assert!(live(&sb.member("MAIN/ARGOS/CLEAN")));
}

#[test]
fn check_reports_errors_and_scopes_to_an_id() {
    let sb = Sb::new(false);
    let all = sb.run(&["check"]);
    assert_eq!(all.code, 1, "the world has errors:\n{}", all.out);
    for code in ["name.drift", "dup.live", "folder.missing", "lineage.broken", "role.background", "config.layout"] {
        assert!(all.out.contains(code), "{code} missing:\n{}", all.out);
    }
    let one = sb.run(&["check", "MAIN/ARGOS/CLEAN"]);
    assert_eq!(one.code, 0, "{}", one.out);
    let j = sb.run(&["check", "MAIN/ARGOS/DRIFT", "--json"]);
    let v: Value = serde_json::from_str(j.out.trim()).expect("check --json prints only JSON");
    assert!(v.as_array().unwrap().iter().any(|i| i["code"] == "name.drift"));
}

// ------------------------------------------------------------------ the gate

#[test]
fn plan_prints_and_changes_nothing() {
    let sb = Sb::new(false);
    let before = sb.files();
    for args in [
        vec!["start", "MAIN/ARGOS/STOPPED", "--plan"],
        vec!["stop", "MAIN/ARGOS/CLEAN", "--plan"],
        vec!["relaunch", "MAIN/ARGOS/NOLABEL", "--plan"],
        vec!["sweep", "--plan"],
        vec!["rm", "MAIN/ARGOS/COPIES", "--plan"],
        vec!["tender", "MAIN/ARGOS", "job", "do it", "--plan"],
        vec!["ship", "new", "MAIN/ARGOS/NEWSHIP", "--dir", "managed", "--prompt", "hi", "--plan"],
        vec!["ship", "rm", "MAIN/ARGOS/CLEAN", "--plan"],
        vec!["fleet", "new", "ZED", "--plan"],
        vec!["machine", "add", "MAIN", "box2", "--hostname", "box2-host", "--plan"],
    ] {
        let r = sb.run(&args);
        assert_eq!(r.code, 0, "{args:?}:\n{}", r.out);
        assert!(r.out.contains("[PLAN]"), "{args:?}:\n{}", r.out);
        assert!(r.out.contains("why:"), "{args:?} explains each step:\n{}", r.out);
    }
    // events.jsonl may not grow either: nothing ran, so no hook fired
    assert_eq!(before, sb.files(), "--plan changed a file");
    let j = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--plan", "--json"]);
    let v: Value = serde_json::from_str(j.out.trim()).unwrap();
    assert_eq!(v["steps"][0]["cmd"][1], "--bg");
}

#[test]
fn mutating_needs_yes_off_a_terminal() {
    let sb = Sb::new(false);
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED"]);
    assert_eq!(r.code, 1);
    assert!(r.out.contains("not a terminal: pass --yes"), "{}", r.out);
    assert!(!live(&sb.member("MAIN/ARGOS/STOPPED")));
}

#[test]
fn refusals_exit_nonzero_with_a_fix() {
    let sb = Sb::new(false);
    let r = sb.run(&["start", "MAIN/ARGOS/DRIFT", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("fix: nimitz relaunch MAIN/ARGOS/DRIFT"), "{}", r.out);
    let r = sb.run(&["start", "MAIN/ARGOS/TWIN", "--yes"]);
    assert_eq!(r.code, 1);
    assert!(r.out.contains("dup.live"), "{}", r.out);
    let r = sb.run(&["start", "MAIN/ARGOS/GONE", "--yes"]);
    assert_eq!(r.code, 1);
    assert!(r.out.contains("folder.missing"), "{}", r.out);
    let r = sb.run(&["start", "MAIN/ARGOS/FAILED", "--yes"]);
    assert_eq!(r.code, 1, "a failed row needs --force:\n{}", r.out);
}

// ------------------------------------------------------------------ sessions

#[test]
fn start_resumes_a_stopped_ship_and_keeps_its_id() {
    let sb = Sb::new(false);
    let sid = sb.member("MAIN/ARGOS/STOPPED")["session"]["session_id"].clone();
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("[OK] verified by re-reading"));
    let m = sb.member("MAIN/ARGOS/STOPPED");
    assert!(live(&m));
    assert_eq!(m["session"]["session_id"], sid);
    // again: nothing to do
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--yes"]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("nothing to do"), "{}", r.out);
}

#[test]
fn stop_relaunch_sweep_rm() {
    let sb = Sb::new(false);
    let r = sb.run(&["stop", "MAIN/ARGOS/CLEAN", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(!live(&sb.member("MAIN/ARGOS/CLEAN")));

    let old = sb.member("MAIN/ARGOS/NOLABEL")["session"]["session_id"].clone();
    let r = sb.run(&["relaunch", "MAIN/ARGOS/NOLABEL", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let m = sb.member("MAIN/ARGOS/NOLABEL");
    assert!(live(&m));
    assert_eq!(m["session"]["session_id"], old, "rm then resume with flags keeps the id and its history (BUG-53)");
    assert_eq!(m["session"]["labels"]["NIMITZ_ID"], "MAIN/ARGOS/NOLABEL");

    assert!(!sb.member("MAIN/ARGOS/COPIES")["extra_rows"].as_array().unwrap().is_empty());
    let r = sb.run(&["sweep", "MAIN/ARGOS/COPIES", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(sb.member("MAIN/ARGOS/COPIES")["extra_rows"].as_array().unwrap().is_empty());

    // one of two live copies, by its short id (the dup.live fix)
    let twin = sb.member("MAIN/ARGOS/TWIN");
    let short = twin["extra_rows"][0]["short_id"].as_str().unwrap().to_string();
    let r = sb.run(&["stop", &short, "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert_eq!(sb.run(&["stop", "nosuchrow", "--yes"]).code, 1);
}

#[test]
fn rm_removes_a_stopped_stowaway() {
    let sb = Sb::new(false);
    // rm refuses a live stowaway, then removes it once stopped
    let s = sb.status();
    let stow = s["stowaways"].as_array().unwrap().iter().find(|x| x["name"] == "quick look at the logs").unwrap()["short_id"].as_str().unwrap().to_string();
    let r = sb.run(&["rm", &stow, "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert_eq!(sb.run(&["stop", &stow, "--yes"]).code, 0);
    let r = sb.run(&["rm", &stow, "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
}

#[test]
fn tender_and_adopt() {
    let sb = Sb::new(false);
    let r = sb.run_as("MAIN/ARGOS", &["tender", "MAIN/ARGOS", "build-docs", "Build", "the", "docs.", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let t = sb.member("MAIN/ARGOS/~build-docs");
    assert!(live(&t));
    assert_eq!(t["session"]["labels"]["NIMITZ_PARENT"], "MAIN/ARGOS");
    assert_eq!(t["session"]["name"], "[MAIN.ARGOS]-COMMODORE/build-docs");
    assert_eq!(sb.run_as("MAIN/BOREAL", &["tender", "MAIN/ARGOS", "x", "go", "--yes"]).code, 1);
    assert_eq!(sb.run_as("MAIN/ARGOS/CLEAN", &["tender", "MAIN/ARGOS", "x", "go", "--yes"]).code, 1);
    assert_eq!(sb.run(&["tender", "MAIN/ARGOS", "x", "--yes"]).code, 1, "no prompt");

    let s = sb.status();
    let stow = s["stowaways"].as_array().unwrap().iter().find(|x| x["name"] == "quick look at the logs").unwrap()["session_id"].as_str().unwrap().to_string();
    let r = sb.run(&["adopt", &stow, "MAIN/ARGOS/FRESH", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("labels.missing"), "warns that labels come at the next relaunch:\n{}", r.out);
    assert_eq!(sb.member("MAIN/ARGOS/FRESH")["session"]["session_id"], Value::from(stow.as_str()));
    assert_eq!(sb.run_as("MAIN/ARGOS", &["adopt", &stow, "MAIN/ARGOS/FRESH", "--yes"]).code, 1);
}

// ------------------------------------------------------------------ caller authority

#[test]
fn a_commodore_starts_its_own_ship_only() {
    let sb = Sb::new(false);
    let r = sb.run_as("MAIN/ARGOS", &["start", "MAIN/ARGOS/STOPPED", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(live(&sb.member("MAIN/ARGOS/STOPPED")));
    let r = sb.run_as("MAIN/BOREAL", &["start", "MAIN/ARGOS/FRESH", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(!live(&sb.member("MAIN/ARGOS/FRESH")));
    assert_eq!(sb.run_as("MAIN/ARGOS", &["start", "BETA", "--yes"]).code, 1, "never an admiral");
    for verb in [vec!["sweep", "MAIN/BOREAL", "--yes"], vec!["up", "MAIN", "--yes"], vec!["fleet", "new", "ZED", "--yes"]] {
        assert_eq!(sb.run_as("MAIN/ARGOS", &verb).code, 1, "{verb:?} is the user's");
    }
    // a commodore sweeps its own squadron (contracts.md section 8)
    let r = sb.run_as("MAIN/ARGOS", &["sweep", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
}

#[test]
fn a_ship_and_an_admiral_start_nothing() {
    let sb = Sb::new(false);
    for who in ["MAIN/ARGOS/CLEAN", "MAIN"] {
        let r = sb.run_as(who, &["start", "MAIN/ARGOS/FRESH", "--yes"]);
        assert_eq!(r.code, 1, "{who}: {}", r.out);
        assert!(r.out.contains("may not start"), "{}", r.out);
        assert_eq!(sb.run_as(who, &["stop", "MAIN/ARGOS/CLEAN", "--yes"]).code, 1);
        assert_eq!(sb.run_as(who, &["tender", "MAIN/ARGOS", "x", "go", "--yes"]).code, 1);
    }
    assert!(!live(&sb.member("MAIN/ARGOS/FRESH")));
    assert!(live(&sb.member("MAIN/ARGOS/CLEAN")));
}

// ------------------------------------------------------------------ CRUD and top-down

fn make_top(sb: &Sb) {
    for args in [
        vec!["fleet", "new", "TOP", "--yes"],
        vec!["squadron", "new", "TOP/SQ", "--brief", "Test squadron.", "--yes"],
        vec!["ship", "new", "TOP/SQ/S1", "--dir", "managed", "--prompt", "You are S1.", "--yes"],
    ] {
        let r = sb.run(&args);
        assert_eq!(r.code, 0, "{args:?}:\n{}", r.out);
    }
}

#[test]
fn start_is_top_down_admiral_commodore_ship() {
    let sb = Sb::new(true);
    make_top(&sb);
    let p = sb.run(&["start", "TOP/SQ/S1", "--plan", "--json"]);
    let v: Value = serde_json::from_str(p.out.trim()).unwrap();
    let ids: Vec<&str> = v["steps"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["TOP", "TOP/SQ", "TOP/SQ/S1"], "{}", p.out);

    // no mux: the interactive members refuse, and nothing starts
    let r = raw(&sb.vars, &["start", "TOP/SQ/S1", "--yes"], &[("NIMITZ_MUX", "none")], None);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("mux.none"), "{}", r.out);
    assert!(!live(&sb.member("TOP/SQ/S1")));

    let r = sb.run(&["start", "TOP/SQ/S1", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let calls = sb.calls();
    let a = calls.find("[TOP]-ADMIRAL").expect("admiral opened");
    let c = calls.find("[TOP.SQ]-COMMODORE").expect("commodore opened");
    assert!(a < c, "admiral before commodore");
    let (oa, oc, os) = (r.out.find("[TOP]-ADMIRAL").unwrap(), r.out.find("[TOP.SQ]-COMMODORE").unwrap(), r.out.find("[TOP.SQ]-S1").unwrap());
    assert!(oa < oc && oc < os, "top-down in the run:\n{}", r.out);
    assert!(live(&sb.member("TOP/SQ/S1")));
}

#[test]
fn crud_round_trip() {
    let sb = Sb::new(false);
    make_top(&sb);
    let fleet_dir = sb.base.join("nimitz/top");
    let sq_toml = || std::fs::read_to_string(fleet_dir.join("squadrons/sq/squadron.toml")).unwrap();
    assert!(sq_toml().contains("S1"));

    assert_eq!(sb.run(&["fleet", "set", "TOP", "--admiral-mode", "auto", "--yes"]).code, 0);
    assert!(std::fs::read_to_string(fleet_dir.join("fleet.toml")).unwrap().contains("permission_mode = \"auto\""));
    let r = sb.run(&["squadron", "set", "TOP/SQ", "--comms", "commodore-only", "--brief", "New brief.", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(sq_toml().contains("commodore-only"));
    assert!(std::fs::read_to_string(fleet_dir.join("squadrons/sq/SQUADRON.md")).unwrap().contains("New brief."));
    let r = sb.run(&["ship", "set", "TOP/SQ/S1", "--mode", "plan", "--depends", "S2", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(sq_toml().contains("plan"));
    assert!(r.out.contains("next relaunch"), "{}", r.out);
    assert_eq!(sb.run(&["ship", "set", "TOP/SQ/NOPE", "--mode", "plan", "--yes"]).code, 1);
    // `--dir <label>=<x>` is the machine-add form: a usage error with the right form
    let r = sb.run(&["ship", "new", "TOP/SQ/S3", "--dir", "sandbox-host=managed", "--prompt", "x", "--yes"]);
    assert_eq!(r.code, 2, "{}", r.out);
    assert!(r.out.contains("use --dir managed --machine sandbox-host"), "{}", r.out);
    // a relative folder is stored absolute, and the stored path is printed
    let r = raw(&sb.vars, &["ship", "new", "TOP/SQ/S3", "--dir", "rel/./s3", "--prompt", "x", "--plan"], &[], None);
    assert!(r.out.contains("folder for sandbox-host: /"), "{}", r.out);
    // an empty --dir is a usage error; removing a folder is explicit
    assert_eq!(sb.run(&["ship", "set", "TOP/SQ/S1", "--dir", "", "--yes"]).code, 2);
    assert_eq!(sb.run(&["ship", "set", "TOP/SQ/S1", "--no-dir", "nowhere", "--yes"]).code, 2);
    let r = sb.run(&["ship", "set", "TOP/SQ/S1", "--no-dir", "sandbox-host", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(!sq_toml().contains("sandbox-host ="), "{}", sq_toml());
    assert_eq!(sb.run(&["ship", "set", "TOP/SQ/S1", "--dir", "managed", "--yes"]).code, 0);
    // --edit opens $EDITOR on the current prompt
    let ed = "sh -c 'printf \" Be brief.\" >> \"$1\"' ed";
    let r = raw(&sb.vars, &["ship", "set", "TOP/SQ/S1", "--edit", "--yes"], &[("EDITOR", ed)], None);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(sq_toml().contains("You are S1. Be brief."), "{}", sq_toml());
    assert_eq!(raw(&sb.vars, &["ship", "set", "TOP/SQ/S1", "--edit", "--prompt", "x", "--yes"], &[("EDITOR", ed)], None).code, 2);
    assert_eq!(sb.run(&["ship", "new", "TOP/SQ/S2", "--yes"]).code, 2, "--dir is required");

    let r = sb.run(&["machine", "add", "TOP", "box2", "--hostname", "box2-host", "--dir", "TOP/SQ/S1=managed", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(std::fs::read_to_string(fleet_dir.join("fleet.toml")).unwrap().contains("box2-host"));
    // this host already has a label: no second one without --hostname
    let r = sb.run(&["machine", "add", "TOP", "box3", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("machine.host"), "{}", r.out);
    assert!(sq_toml().contains("box2"));
    let r = sb.run(&["machine", "rm", "TOP", "box2", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(!std::fs::read_to_string(fleet_dir.join("fleet.toml")).unwrap().contains("box2"));
    assert!(!sq_toml().contains("box2"));
    assert_eq!(sb.run(&["machine", "rm", "TOP", "box2", "--yes"]).code, 1);

    // delete bottom-up; a fleet with squadrons refuses
    assert_eq!(sb.run(&["fleet", "rm", "TOP", "--yes"]).code, 1);
    assert_eq!(sb.run(&["squadron", "rm", "TOP/SQ", "--yes"]).code, 1, "still has a ship");
    assert_eq!(sb.run(&["ship", "rm", "TOP/SQ/S1", "--yes"]).code, 0);
    assert!(!sq_toml().contains("S1"));
    assert_eq!(sb.run(&["squadron", "rm", "TOP/SQ", "--yes"]).code, 0);
    assert_eq!(sb.run(&["fleet", "rm", "TOP", "--yes"]).code, 0);
    assert!(fleet_dir.join("fleet.toml.archived").is_file());
}

#[test]
fn ship_rm_stops_and_removes_rows_first() {
    let sb = Sb::new(false);
    let r = sb.run(&["ship", "rm", "MAIN/ARGOS/CLEAN", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains(" stop ") && r.out.contains(" rm "), "{}", r.out);
    let s = sb.status();
    let argos = &s["fleets"].as_array().unwrap().iter().find(|f| f["id"] == "MAIN").unwrap()["squadrons"][0];
    assert!(!argos["ships"].as_array().unwrap().iter().any(|m| m["id"] == "MAIN/ARGOS/CLEAN"));
}

// ------------------------------------------------------------------ mux commands

#[test]
fn up_needs_a_mux() {
    let sb = Sb::new(false);
    let r = sb.run(&["up", "MAIN", "--plan"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("needs a multiplexer"), "{}", r.out);
    let sb = Sb::new(true);
    let r = sb.run(&["up", "MAIN", "--plan"]);
    assert!(r.out.contains("[PLAN] up MAIN"), "{}", r.out);
}

#[test]
fn open_attaches_a_ship_and_focuses_a_pane() {
    let sb = Sb::new(false);
    assert_eq!(sb.run(&["open", "MAIN/ARGOS/CLEAN"]).code, 1, "no mux");
    let sb = Sb::new(true);
    let r = sb.run(&["open", "MAIN/ARGOS/CLEAN"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(sb.calls().contains(" attach "), "{}", sb.calls());
    sb.pane_for("MAIN/ARGOS");
    let r = sb.run(&["open", "MAIN/ARGOS"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(sb.calls().contains("\"agent\",\"focus\",\"w1:p2\""), "{}", sb.calls());
    assert_eq!(sb.run(&["open", "MAIN"]).code, 1, "live admiral outside the mux");
}

#[test]
fn signal_types_or_relays_and_members_use_sendmessage() {
    let sb = Sb::new(true);
    assert_eq!(sb.run(&["signal", "MAIN/ARGOS", "hello", "--yes"]).code, 1, "no pane yet");
    sb.pane_for("MAIN/ARGOS");
    let r = sb.run(&["signal", "MAIN/ARGOS", "status?", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let r = sb.run(&["signal", "MAIN/ARGOS/CLEAN", "rebase", "please", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(sb.calls().contains("Relay to [MAIN.ARGOS]-CLEAN from the user: rebase please"), "{}", sb.calls());
    let r = sb.run_as("MAIN/ARGOS/CLEAN", &["signal", "MAIN/BOREAL", "hi", "--yes"]);
    assert_eq!(r.code, 1);
    assert!(r.out.contains("comms.denied"), "{}", r.out);
}

#[test]
fn layout_herdr() {
    let sb = Sb::new(false);
    assert_eq!(sb.run(&["layout", "herdr", "--yes"]).code, 1);
    assert_eq!(sb.run(&["layout", "tmux"]).code, 2);
    // NIMITZ_MUX: an unknown value is a usage error naming the valid ones; none says so in the fix
    let r = raw(&sb.vars, &["status"], &[("NIMITZ_MUX", "tmux")], None);
    assert_eq!(r.code, 2);
    assert!(r.out.contains("auto, herdr, none"), "{}", r.out);
    let r = raw(&sb.vars, &["layout", "herdr", "--yes"], &[("NIMITZ_MUX", "none"), ("HERDR_ENV", "1")], None);
    assert!(r.out.contains("NIMITZ_MUX=none turns the multiplexer off") && !r.out.contains("run nimitz inside Herdr"), "{}", r.out);
    let sb = Sb::new(true);
    let r = sb.run(&["layout", "herdr", "BETA", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(sb.calls().contains("[BETA]-ADMIRAL"), "{}", sb.calls());
    let r = sb.run(&["layout", "herdr", "MAIN", "--plan"]);
    assert!(r.out.contains("running already"), "live members are left alone:\n{}", r.out);
    assert_eq!(sb.run_as("MAIN/ARGOS", &["layout", "herdr", "--yes"]).code, 1);
    // --ships: attach tabs for live ships; stopped ones are skipped
    let r = sb.run(&["layout", "herdr", "MAIN", "--ships", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("tab CLEAN in workspace MAIN.ARGOS"), "{}", r.out);
    assert!(r.out.contains("MAIN/ARGOS/STOPPED is not live; no tab"), "{}", r.out);
    assert!(sb.calls().contains(" attach "), "{}", sb.calls());
}

// ------------------------------------------------------------------ hook

const EVENTS: [&str; 8] = ["SessionStart", "SessionEnd", "UserPromptSubmit", "Stop", "Notification", "SubagentStart", "SubagentStop", "PreToolUse"];

#[test]
fn hook_always_exits_zero_even_on_garbage() {
    let sb = Sb::new(false);
    for ev in EVENTS.iter().chain(["", "NoSuchEvent"].iter()) {
        for input in ["", "garbage{{", "[1,2", "null", "{\"session_id\": 5}"] {
            let r = raw(&sb.vars, &["hook", ev], &[], Some(input));
            assert_eq!(r.code, 0, "{ev} {input:?}: {}", r.out);
        }
    }
    // a broken state dir does not fail it either
    let r = raw(&sb.vars, &["hook", "SessionStart"], &[("NIMITZ_STATE", "/dev/null/x")], Some("{\"session_id\":\"s\"}"));
    assert_eq!(r.code, 0);
}

#[test]
fn hook_logs_session_start_with_labels() {
    let sb = Sb::new(false);
    let input = r#"{"session_id":"abc-123","source":"startup","hook_event_name":"SessionStart"}"#;
    let r = raw(&sb.vars, &["hook", "SessionStart"], &[("NIMITZ_ID", "MAIN/ARGOS/CLEAN"), ("NIMITZ_ROLE", "ship")], Some(input));
    assert_eq!(r.code, 0);
    let ev = std::fs::read_to_string(sb.base.join("state/events.jsonl")).unwrap();
    let last: Value = serde_json::from_str(ev.lines().last().unwrap()).unwrap();
    assert_eq!(last["session_id"], "abc-123");
    assert_eq!(last["labels"]["NIMITZ_ID"], "MAIN/ARGOS/CLEAN");
}

#[test]
fn hook_denies_a_signal_the_chain_forbids() {
    let sb = Sb::new(false);
    let msg = |to: &str| format!(r#"{{"session_id":"s","tool_name":"SendMessage","tool_input":{{"to":"{to}","message":"hi"}}}}"#);
    let ship = [("NIMITZ_ID", "MAIN/ARGOS/CLEAN"), ("NIMITZ_ROLE", "ship")];
    let r = raw(&sb.vars, &["hook", "PreToolUse"], &ship, Some(&msg("[MAIN.BOREAL]-COMMODORE")));
    assert_eq!(r.code, 0);
    let v: Value = serde_json::from_str(r.out.trim()).expect("deny JSON");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    // its own commodore: allowed, no output
    let r = raw(&sb.vars, &["hook", "PreToolUse"], &ship, Some(&msg("[MAIN.ARGOS]-COMMODORE")));
    assert_eq!((r.code, r.out.trim()), (0, ""));
    // BOREAL is commodore-only: a ship there may not signal a peer
    let boreal = [("NIMITZ_ID", "MAIN/BOREAL/X"), ("NIMITZ_ROLE", "ship")];
    let r = raw(&sb.vars, &["hook", "PreToolUse"], &boreal, Some(&msg("[MAIN.BOREAL]-Y")));
    assert!(r.out.contains("\"deny\""), "{}", r.out);
    // no labels (the user's own session): not ours to rule
    let r = raw(&sb.vars, &["hook", "PreToolUse"], &[], Some(&msg("[MAIN.BOREAL]-COMMODORE")));
    assert_eq!(r.out.trim(), "");
}

#[test]
fn hook_is_fast() {
    let sb = Sb::new(false);
    let input = r#"{"session_id":"s","source":"startup"}"#;
    let mut t: Vec<f64> = (0..15)
        .map(|_| {
            let s = std::time::Instant::now();
            raw(&sb.vars, &["hook", "SessionStart"], &[("NIMITZ_ID", "MAIN/ARGOS/CLEAN"), ("NIMITZ_ROLE", "ship")], Some(input));
            s.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // the target is < 10 ms on a release build; a debug build under a loaded test run gets headroom
    assert!(t[7] < 60.0, "median hook time {:.1} ms (all: {t:?})", t[7]);
}

// ------------------------------------------------------------------ migrate, init, doctor, sandbox

#[test]
fn migrate_plans_applies_and_is_idempotent() {
    let sb = Sb::new(false);
    let fx = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nimitz-core/tests/fixtures/old-fleets");
    let mut dirs: Vec<String> = std::fs::read_dir(&fx).unwrap().flatten().map(|e| e.path().to_string_lossy().into_owned()).collect();
    dirs.sort();
    let mut args = vec!["migrate", "fleet-skill"];
    args.extend(dirs.iter().map(String::as_str));
    args.extend(["--into", "OLD"]);
    let plan = sb.run(&args);
    assert_eq!(plan.code, 1, "no --yes off a terminal:\n{}", plan.out);
    assert!(plan.out.contains("[PLAN] import"), "{}", plan.out);
    assert!(!sb.base.join("nimitz/old/fleet.toml").exists(), "a plan writes nothing");
    let mut yes = args.clone();
    yes.push("--yes");
    let r = sb.run(&yes);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(sb.base.join("nimitz/old/fleet.toml").is_file());
    let r = sb.run(&yes);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("nothing to do"), "second run changes nothing:\n{}", r.out);
}

#[test]
fn init_and_doctor_in_an_empty_world() {
    let td = tempfile::tempdir().unwrap();
    let b = td.path();
    let p = |x: &str| b.join(x).to_string_lossy().into_owned();
    let sb = Sb::new(false); // only for its fake claude
    let vars: BTreeMap<String, String> = [
        ("HOME", p("home")),
        ("NIMITZ_HOME", p("nimitz")),
        ("NIMITZ_STATE", p("state")),
        ("NIMITZ_CLAUDE_HOME", p("claude_home")),
        ("NIMITZ_CLAUDE_BIN", sb.vars["NIMITZ_CLAUDE_BIN"].clone()),
        ("NIMITZ_NATIVE_CLAUDE_BIN", sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"].clone()),
        ("NIMITZ_HOSTNAME", "box".into()),
        ("NIMITZ_MUX", "none".into()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let d = raw(&vars, &["doctor"], &[], None);
    assert_eq!(d.code, 1);
    assert!(d.out.contains("fix: nimitz init"), "{}", d.out);
    assert_eq!(raw(&vars, &["init", "--label", "Bad Label", "--yes"], &[], None).code, 1);
    let r = raw(&vars, &["init", "--label", "box", "--yes"], &[], None);
    assert!(r.out.contains("run `claude` there once"), "asks for trust, does not launch:\n{}", r.out);
    assert!(b.join("nimitz").is_dir());
    let reg: Value = serde_json::from_str(&std::fs::read_to_string(b.join("state/registry.json")).unwrap()).unwrap();
    assert_eq!(reg["machine"], "box");
    assert_eq!(raw(&vars, &["init", "--label", "other", "--yes"], &[], None).code, 1, "a label is not changed silently");

    // trust it, enable the plugin; doctor still wants crossSessionInbound, and --fix sets it
    std::fs::create_dir_all(b.join("claude_home")).unwrap();
    std::fs::write(b.join("claude_home/.claude.json"), format!(r#"{{"projects":{{"{}":{{"hasTrustDialogAccepted":true}}}}}}"#, p("nimitz"))).unwrap();
    std::fs::write(b.join("claude_home/settings.json"), r#"{"enabledPlugins":{"nimitz@claude-nimitz":true},"model":"x"}"#).unwrap();
    let d = raw(&vars, &["doctor"], &[], None);
    assert_eq!(d.code, 1, "{}", d.out);
    assert!(d.out.contains("crossSessionInbound is unset"), "{}", d.out);
    assert!(d.out.contains("[OK] native claude"), "{}", d.out);
    assert!(d.out.contains("[OK] hooks"), "{}", d.out);
    let f = raw(&vars, &["doctor", "--fix", "--yes"], &[], None);
    assert_eq!(f.code, 0, "{}", f.out);
    let s: Value = serde_json::from_str(&std::fs::read_to_string(b.join("claude_home/settings.json")).unwrap()).unwrap();
    assert_eq!(s["crossSessionInbound"], "accept");
    assert_eq!(s["model"], "x", "other settings kept");
    assert_eq!(raw(&vars, &["doctor"], &[], None).code, 0);
}

#[test]
fn doctor_flags_a_changed_format() {
    let sb = Sb::new(false);
    let sess = sb.base.join("claude_home/sessions");
    let one = std::fs::read_dir(&sess).unwrap().flatten().next().unwrap().path();
    std::fs::write(&one, r#"{"pid": 1, "sessionId": "x"}"#).unwrap();
    let d = sb.run(&["doctor"]);
    assert!(d.out.contains("lacks status, kind"), "{}", d.out);
}

#[test]
fn sandbox_env_tick_and_fakes() {
    let sb = Sb::new(true);
    let e = sb.run(&["sandbox", "env", sb.base.to_str().unwrap()]);
    assert!(e.out.contains("export NIMITZ_HOME="));
    assert!(e.out.contains("export NIMITZ_MUX=herdr"));
    assert_eq!(sb.run(&["sandbox", "env", "/nonexistent-nimitz"]).code, 1);
    let before = sb.member("MAIN")["status"].clone();
    let r = sb.run(&["sandbox", "tick", sb.base.to_str().unwrap(), "--once"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert_ne!(sb.member("MAIN")["status"], before);
    // create refuses a folder that is not a sandbox
    let td = tempfile::tempdir().unwrap();
    std::fs::write(td.path().join("precious.txt"), "x").unwrap();
    assert_eq!(sb.run(&["sandbox", "create", td.path().to_str().unwrap()]).code, 1);
    assert!(td.path().join("precious.txt").is_file());
    // the fakes answer
    let v = raw(&sb.vars, &["__fake-claude", sb.base.to_str().unwrap(), "--version"], &[], None);
    assert!(v.out.contains("2.1.288"), "{}", v.out);
    let h = raw(&sb.vars, &["__fake-herdr", sb.base.join("herdr").to_str().unwrap(), "agent", "list"], &[], None);
    assert_eq!(h.code, 0, "{}", h.out);
}
