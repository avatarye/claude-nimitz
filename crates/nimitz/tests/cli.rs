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
    c.env_clear().env("PATH", "/usr/bin:/bin").envs(vars).args(args);
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
                for m in sq["ships"].as_array().unwrap() {
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
    /// Run the sandbox's fake `claude` in `cwd` (the native binary).
    fn claude(&self, cwd: &Path, args: &[&str]) -> String {
        let o = Command::new(self.base.join("bin/claude")).args(args).current_dir(cwd).env_clear().env("PATH", "/usr/bin:/bin").envs(&self.vars).output().expect("fake claude");
        format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
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
fn status_columns_line_up_with_long_ids() {
    let sb = Sb::new(false);
    for a in [vec!["fleet", "new", "UPS", "--yes"], vec!["squadron", "new", "UPS/SQUADRON_WITH_A_LONG_NAME", "--yes"]] {
        assert_eq!(sb.run(&a).code, 0);
    }
    let r = sb.run(&["ship", "new", "UPS/SQUADRON_WITH_A_LONG_NAME/SHIP_LONG", "--dir", "managed", "--prompt", "hi", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let out = sb.run(&["status"]).out;
    let cols: Vec<usize> = out
        .lines()
        .take_while(|l| !l.starts_with("stowaways"))
        .filter(|l| l.starts_with("  ") && !l.trim_start().starts_with('['))
        .filter_map(|l| ["not-started", "stopped", "parked", "idle", "busy", "needs-you", "unknown"].iter().filter_map(|w| l.find(&format!(" {w} "))).min())
        .collect();
    assert!(cols.len() > 5 && cols.iter().all(|c| *c == cols[0]), "status column moves: {cols:?}\n{out}");
}

#[test]
fn check_reports_errors_and_scopes_to_an_id() {
    let sb = Sb::new(false);
    let all = sb.run(&["check"]);
    assert_eq!(all.code, 1, "the world has errors:\n{}", all.out);
    for code in ["name.drift", "dup.live", "folder.missing", "history.lost", "config.layout"] {
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
fn adopt() {
    let sb = Sb::new(false);
    let s = sb.status();
    let stow = s["stowaways"].as_array().unwrap().iter().find(|x| x["name"] == "quick look at the logs").unwrap()["session_id"].as_str().unwrap().to_string();
    let r = sb.run(&["adopt", &stow, "MAIN/ARGOS/FRESH", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("labels.missing"), "warns that labels come at the next relaunch:\n{}", r.out);
    assert_eq!(sb.member("MAIN/ARGOS/FRESH")["session"]["session_id"], Value::from(stow.as_str()));
    assert_eq!(sb.run_as("MAIN/ARGOS", &["adopt", &stow, "MAIN/ARGOS/FRESH", "--yes"]).code, 1);
    // a start of the adopted ship refuses with name.drift and labels.missing, each once (acceptance F6)
    let r = sb.run(&["start", "MAIN/ARGOS/FRESH", "--plan"]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert_eq!(r.out.matches("(name.drift)").count(), 1, "{}", r.out);
    assert_eq!(r.out.matches("(labels.missing)").count(), 1, "{}", r.out);
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
        assert_eq!(sb.run_as("MAIN/ARGOS", &verb).code, 1, "{verb:?} is not a commodore's");
    }
    // a commodore sweeps its own squadron (contracts.md section 8)
    let r = sb.run_as("MAIN/ARGOS", &["sweep", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
}

#[test]
fn a_ship_starts_nothing_and_an_admiral_only_in_its_own_fleet() {
    let sb = Sb::new(false);
    // a ship, and another fleet's admiral: nothing here
    for who in ["MAIN/ARGOS/CLEAN", "BETA"] {
        let r = sb.run_as(who, &["start", "MAIN/ARGOS/FRESH", "--yes"]);
        assert_eq!(r.code, 1, "{who}: {}", r.out);
        assert!(r.out.contains("may not start"), "{}", r.out);
        assert_eq!(sb.run_as(who, &["stop", "MAIN/ARGOS/CLEAN", "--yes"]).code, 1);
    }
    assert!(!live(&sb.member("MAIN/ARGOS/FRESH")));
    assert!(live(&sb.member("MAIN/ARGOS/CLEAN")));
    // MAIN's admiral: any member of MAIN, with the same plan step first
    let r = sb.run_as("MAIN", &["start", "MAIN/ARGOS/FRESH", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let r = sb.run_as("MAIN", &["start", "MAIN/ARGOS/FRESH", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(live(&sb.member("MAIN/ARGOS/FRESH")));
    assert_eq!(sb.run_as("MAIN", &["stop", "MAIN/ARGOS/FRESH", "--yes"]).code, 0);
    assert_eq!(sb.run_as("MAIN", &["sweep", "--plan"]).code, 0, "its own fleet");
    assert_eq!(sb.run_as("MAIN", &["sweep", "BETA", "--plan"]).code, 1, "not another fleet");
    assert_eq!(sb.run_as("MAIN", &["up", "BETA", "--plan"]).code, 1);
    assert_eq!(sb.run_as("MAIN", &["stop", "MAIN", "--yes"]).code, 1, "not itself");
    // a caller without authority gets only the authority refusal (BUG-51): no
    // other refusal, and never a fix only the user may run
    let r = sb.run_as("MAIN/ARGOS/CLEAN", &["start", "MAIN/ARGOS/DRIFT", "--plan"]);
    assert!(r.out.contains("(authority)") && !r.out.contains("name.drift"), "{}", r.out);
    assert!(!r.out.lines().any(|l| l.trim_start().starts_with("fix: nimitz relaunch")), "{}", r.out);
    let r = sb.run_as("MAIN/ARGOS/CLEAN", &["start", "MAIN/ARGOS/CLEAN", "--plan"]);
    assert_ne!(r.code, 0, "nothing to do is still not the caller's to do: {}", r.out);
}

/// plan-v4 section 12.1, the role table: every command the binary has
/// (`nimitz __commands`, the dispatch table) runs as an admiral, a commodore
/// and a ship, with `--plan` and `--yes`. A row the role may run never says
/// `(authority)` (it may refuse for another reason); every other row exits 1
/// with `(authority)`. A command that is not in this table fails the test.
#[test]
fn every_command_goes_by_the_role_table() {
    // (command, the rest of the argv, admiral MAIN, commodore MAIN/ARGOS, ship MAIN/ARGOS/CLEAN)
    const TABLE: &[(&str, &[&str], [bool; 3])] = &[
        ("init", &["--label", "box"], [false, false, false]),
        ("doctor", &[], [true, true, true]),
        ("doctor", &["--fix"], [false, false, false]),
        ("rules", &[], [true, true, true]),
        ("status", &[], [true, true, true]),
        ("check", &["MAIN"], [true, true, true]),
        ("fleet", &["new", "ZED"], [false, false, false]),
        ("fleet", &["set", "MAIN", "--admiral-mode", "auto"], [true, false, false]),
        ("fleet", &["rm", "BETA"], [false, false, false]),
        ("squadron", &["new", "MAIN/EXTRA", "--brief", "x"], [true, false, false]),
        ("squadron", &["set", "MAIN/ARGOS", "--comms", "open"], [true, true, false]),
        ("squadron", &["rm", "MAIN/ARGOS"], [true, false, false]),
        ("ship", &["new", "MAIN/ARGOS/NEWBIE", "--dir", "managed", "--prompt", "x"], [true, true, false]),
        ("ship", &["set", "MAIN/ARGOS/CLEAN", "--mode", "auto"], [true, true, false]),
        ("ship", &["rm", "MAIN/ARGOS/FRESH"], [true, true, false]),
        ("machine", &["add", "MAIN", "box2", "--hostname", "box2"], [false, false, false]),
        ("machine", &["rm", "MAIN", "box2"], [false, false, false]),
        ("migrate", &["fleet-skill", "/nonexistent-nimitz", "--into", "MAIN"], [false, false, false]),
        ("up", &["MAIN"], [true, false, false]),
        ("start", &["MAIN/ARGOS/STOPPED"], [true, true, false]),
        ("stop", &["MAIN/ARGOS/CLEAN"], [true, true, false]),
        ("relaunch", &["MAIN/ARGOS/CLEAN"], [true, true, false]),
        ("rm", &["MAIN/ARGOS/STOPPED"], [true, true, false]),
        ("sweep", &["MAIN/ARGOS/CLEAN"], [true, true, true]),
        ("sweep", &["MAIN/ARGOS"], [true, true, false]),
        ("sweep", &["MAIN"], [true, false, false]),
        ("adopt", &["00000000-0000", "MAIN/ARGOS/FRESH"], [true, true, false]),
        // every member signals with SendMessage: no authority refusal, comms.denied instead
        ("signal", &["MAIN/ARGOS", "hi"], [true, true, true]),
        ("open", &["MAIN/ARGOS/CLEAN"], [false, false, false]),
        ("layout", &["herdr", "MAIN"], [true, false, false]),
        ("restore", &["MAIN"], [true, false, false]),
        ("sandbox", &["env", "/nonexistent-nimitz"], [false, false, false]),
    ];
    let roles = ["MAIN", "MAIN/ARGOS", "MAIN/ARGOS/CLEAN"];
    let listed = Sb::new(false).run(&["__commands"]);
    assert_eq!(listed.code, 0, "{}", listed.out);
    for cmd in listed.out.lines().map(str::trim).filter(|l| !l.is_empty()) {
        assert!(TABLE.iter().any(|(c, _, _)| *c == cmd), "`nimitz {cmd}` has no row in the role table (plan-v4 section 12.1)");
    }
    for (i, who) in roles.iter().enumerate() {
        // one world per role and form, so a `--yes` run changes nothing for the next row
        for form in ["--plan", "--yes"] {
            let sb = Sb::new(false);
            for (cmd, rest, may) in TABLE {
                let mut args = vec![*cmd];
                args.extend(rest.iter().copied());
                args.push(form);
                let r = sb.run_as(who, &args);
                let authority = r.out.contains("(authority)");
                if may[i] {
                    assert!(!authority, "{who} may run `nimitz {}`:\n{}", args.join(" "), r.out);
                } else {
                    assert!(authority && r.code != 0, "{who} may not run `nimitz {}` (exit {}):\n{}", args.join(" "), r.code, r.out);
                }
            }
        }
    }
}

#[test]
fn config_changes_go_by_the_same_scopes_and_whole_fleets_stay_yours() {
    let sb = Sb::new(false);
    let ok = |who: &str, args: &[&str]| {
        let r = sb.run_as(who, args);
        assert_eq!(r.code, 0, "{who} {args:?}:\n{}", r.out);
    };
    let no = |who: &str, args: &[&str]| {
        let r = sb.run_as(who, args);
        assert_eq!(r.code, 1, "{who} {args:?}:\n{}", r.out);
        assert!(r.out.contains("(authority)"), "{who} {args:?}:\n{}", r.out);
    };
    // a commodore: its squadron's settings and its ships
    ok("MAIN/ARGOS", &["ship", "new", "MAIN/ARGOS/NEWBIE", "--dir", "managed", "--prompt", "You are NEWBIE.", "--yes"]);
    ok("MAIN/ARGOS", &["ship", "set", "MAIN/ARGOS/NEWBIE", "--mode", "auto", "--plan"]);
    ok("MAIN/ARGOS", &["squadron", "set", "MAIN/ARGOS", "--comms", "open", "--plan"]);
    no("MAIN/ARGOS", &["ship", "new", "MAIN/BOREAL/X", "--dir", "managed", "--prompt", "x", "--plan"]);
    no("MAIN/ARGOS", &["squadron", "new", "MAIN/EXTRA", "--brief", "x", "--plan"]);
    no("MAIN/ARGOS", &["squadron", "rm", "MAIN/ARGOS", "--plan"]);
    no("MAIN/ARGOS", &["fleet", "set", "MAIN", "--admiral-mode", "auto", "--plan"]);
    // an admiral: its own fleet
    ok("MAIN", &["squadron", "new", "MAIN/EXTRA", "--brief", "Extra squadron.", "--yes"]);
    ok("MAIN", &["ship", "rm", "MAIN/ARGOS/NEWBIE", "--yes"]);
    ok("MAIN", &["fleet", "set", "MAIN", "--admiral-mode", "auto", "--plan"]);
    no("MAIN", &["ship", "new", "BETA/SQ/X", "--dir", "managed", "--prompt", "x", "--plan"]);
    // a whole fleet, machines, migration: yours alone
    no("MAIN", &["fleet", "new", "ZED", "--plan"]);
    no("MAIN", &["fleet", "rm", "MAIN", "--plan"]);
    no("MAIN", &["machine", "add", "MAIN", "box2", "--hostname", "box2", "--plan"]);
    // ships change no config
    no("MAIN/ARGOS/CLEAN", &["ship", "set", "MAIN/ARGOS/CLEAN", "--mode", "auto", "--plan"]);
    // a refusal's fix is shown only when the caller may run it
    let r = sb.run_as("MAIN/ARGOS", &["start", "MAIN/ARGOS/DRIFT", "--plan"]);
    assert!(r.out.contains("fix: nimitz relaunch MAIN/ARGOS/DRIFT"), "its own ship's fix is its own:\n{}", r.out);
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

    // every member is a background job (0.4.0): no multiplexer needed, nothing
    // opens a pane; the leads launch `--bg` with their prompt, top-down
    let r = raw(&sb.vars, &["start", "TOP/SQ/S1", "--yes"], &[("NIMITZ_MUX", "none")], None);
    assert_eq!(r.code, 0, "{}", r.out);
    let calls = std::fs::read_to_string(sb.base.join("calls.log")).unwrap_or_default();
    let a = calls.find("[TOP]-ADMIRAL").expect("admiral launched");
    let c = calls.find("[TOP.SQ]-COMMODORE").expect("commodore launched");
    let s = calls.find("[TOP.SQ]-S1").expect("ship launched");
    assert!(a < c && c < s, "top-down:\n{calls}");
    assert!(!calls.contains("\"-c\"") && !sb.calls().contains("pane"), "never `-c`, never a pane:\n{calls}\n{}", sb.calls());
    for id in ["TOP", "TOP/SQ", "TOP/SQ/S1"] {
        let m = sb.member(id);
        assert!(live(&m), "{id}: {m}");
        assert_eq!(m["session"]["kind"], "background", "{id}");
    }
    assert_eq!(r.out.matches("--name [TOP]-ADMIRAL").count(), 1, "the launch line is printed once (in the plan):\n{}", r.out);
    // a second start finds everything live and does nothing
    let r = sb.run(&["start", "TOP", "--yes"]);
    assert!(r.out.contains("nothing to do"), "{}", r.out);
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
fn up_needs_no_mux() {
    // 0.4.0: every member is a job, so `up` runs anywhere; `layout herdr` attaches later
    let sb = Sb::new(false);
    let r = sb.run(&["up", "BETA", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("[PLAN] up BETA") && r.out.contains("--bg") && !r.out.contains("mux.none"), "{}", r.out);
    let r = sb.run(&["up", "BETA", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(live(&sb.member("BETA")));
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
    // a lead is a job too: `open` attaches to it in its tab (F.ADMIRAL)
    let r = sb.run(&["open", "MAIN"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("attached in pane") && sb.calls().contains("MAIN.ADMIRAL"), "{}\n{}", r.out, sb.calls());
    // no job yet: say so, with the start
    let r = sb.run(&["open", "BETA"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("has no job") && r.out.contains("nimitz start BETA"), "{}", r.out);
    // a parked job: the attach wakes it (cli-07)
    let short = sb.member("MAIN/ARGOS/CLEAN")["session"]["short_id"].as_str().unwrap().to_string();
    sb.claude(&sb.base.join("dirs/main/clean"), &["__retire", &short]);
    assert_eq!(sb.member("MAIN/ARGOS/CLEAN")["status"], "Parked");
    // its attach client ended with the job: the tab's pane is a shell again
    let pane = sb.calls().lines().find(|l| l.contains("\"pane\",\"run\"") && l.contains(&format!("attach {short}"))).and_then(|l| l.split("\"pane\",\"run\",\"").nth(1)).map(|r| r.split('"').next().unwrap().to_string()).expect("the pane that ran the attach");
    sb.herdr_world(|w| {
        for p in w["panes"].as_array_mut().unwrap().iter_mut().filter(|p| p["pane_id"] == pane) {
            p["foreground"] = Value::from(vec!["zsh"]);
        }
    });
    let r = sb.run(&["open", "MAIN/ARGOS/CLEAN", "--plan"]);
    assert!(r.out.contains("it wakes the parked job") && r.out.contains("attach there"), "{}", r.out);
    let r = sb.run(&["open", "MAIN/ARGOS/CLEAN"]);
    assert_eq!(r.code, 0, "{}", r.out);
    // (the fake herdr types the line but runs nothing; real Herdr runs the attach, which wakes the job)
    assert_eq!(sb.calls().lines().filter(|l| l.contains("\"pane\",\"run\"") && l.contains(&format!("attach {short}"))).count(), 2, "{}", sb.calls());
}

#[test]
fn open_reuses_the_tab_it_opened_before() {
    let sb = Sb::new(true);
    let tabs = |sb: &Sb| -> usize {
        let w: Value = serde_json::from_slice(&std::fs::read(sb.base.join("herdr/world.json")).unwrap()).unwrap();
        w["tabs"].as_array().unwrap().iter().filter(|t| t["label"] == "MAIN.ARGOS.CLEAN").count()
    };
    let r = sb.run(&["open", "MAIN/ARGOS/CLEAN"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("attached in pane"), "{}", r.out);
    assert_eq!(tabs(&sb), 1);
    // again: the tab named MAIN.ARGOS.CLEAN is there; focus it, open nothing
    let r = sb.run(&["open", "MAIN/ARGOS/CLEAN", "--plan"]);
    assert!(r.out.contains("its tab MAIN.ARGOS.CLEAN shows it already"), "{}", r.out);
    let r = sb.run(&["open", "MAIN/ARGOS/CLEAN"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("is open already; focused pane"), "{}", r.out);
    // the attach client ended (ctrl-z): the tab's pane is a plain shell again; the
    // attach runs there, no second tab (R-02)
    let pane = sb.calls().lines().find(|l| l.contains("\"pane\",\"run\"")).and_then(|l| l.split("\"pane\",\"run\",\"").nth(1)).map(|r| r.split('"').next().unwrap().to_string()).unwrap();
    sb.herdr_world(|w| {
        for p in w["panes"].as_array_mut().unwrap().iter_mut().filter(|p| p["pane_id"] == pane) {
            p["foreground"] = Value::from(vec!["zsh"]);
        }
    });
    let r = sb.run(&["open", "MAIN/ARGOS/CLEAN"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("attached in its tab's pane"), "{}", r.out);
    assert_eq!(tabs(&sb), 1, "no second tab");
    // something else runs in it (yazi, another agent, the agent view): never typed into (R-14, R-33)
    sb.herdr_world(|w| {
        for p in w["panes"].as_array_mut().unwrap().iter_mut().filter(|p| p["pane_id"] == pane) {
            p["foreground"] = Value::from(vec!["yazi"]);
        }
    });
    let before = sb.calls().lines().filter(|l| l.contains("\"pane\",\"run\"")).count();
    let r = sb.run(&["open", "MAIN/ARGOS/CLEAN"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(tab.taken)") && r.out.contains("runs `yazi`") && r.out.contains(&format!("in pane {pane}")), "{}", r.out);
    assert_eq!(sb.calls().lines().filter(|l| l.contains("\"pane\",\"run\"")).count(), before, "nothing typed");
    assert_eq!(tabs(&sb), 1, "no second tab");
    assert_eq!(tabs(&sb), 1, "no second tab");
    assert!(sb.calls().contains("\"agent\",\"focus\""), "{}", sb.calls());
}

#[test]
fn signal_types_or_relays_and_members_use_sendmessage() {
    let sb = Sb::new(true);
    // no tab shows the commodore yet: one is attached first; the text is typed
    // only once a pane shows the session (the fake herdr never shows one, so
    // nothing is typed and the user is told)
    let r = sb.run(&["signal", "MAIN/ARGOS", "hello", "--plan"]);
    assert!(r.out.contains("has no tab yet") && r.out.contains(" attach "), "{}", r.out);
    let r = raw(&sb.vars, &["signal", "MAIN/ARGOS", "hello", "--yes"], &[("NIMITZ_SIGNAL_WAIT", "0")], None);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("attached in pane") && r.out.contains("nothing was typed"), "{}", r.out);
    assert!(sb.calls().contains("MAIN.ARGOS.COMMODORE") && !sb.calls().contains("\"agent\",\"prompt\""), "{}", sb.calls());
    sb.pane_for("MAIN/ARGOS");
    let r = sb.run(&["signal", "MAIN/ARGOS", "status?", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let r = sb.run(&["signal", "MAIN/ARGOS/CLEAN", "rebase", "please", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(sb.calls().contains("Relay to [MAIN.ARGOS]-CLEAN from the user: rebase please"), "{}", sb.calls());
    let r = sb.run_as("MAIN/ARGOS/CLEAN", &["signal", "MAIN/BOREAL", "hi", "--yes"]);
    assert_eq!(r.code, 1);
    assert!(r.out.contains("comms.denied"), "{}", r.out);
    // 0.4.0: a ship replies to its admiral, and the admiral reaches a ship; both
    // still go by SendMessage, which the refusal says
    for (who, to) in [("MAIN/ARGOS/CLEAN", "MAIN"), ("MAIN", "MAIN/ARGOS/CLEAN")] {
        let r = sb.run_as(who, &["signal", to, "hi", "--yes"]);
        assert_eq!(r.code, 1);
        assert!(r.out.contains("the chain allows it") && r.out.contains("SendMessage"), "{who} -> {to}: {}", r.out);
    }
}

#[test]
fn a_ship_runs_the_read_only_commands_and_sweeps_itself() {
    let sb = Sb::new(false);
    let ship = "MAIN/ARGOS/CLEAN";
    for args in [vec!["status"], vec!["check", "MAIN"], vec!["rules"], vec!["doctor"]] {
        let r = sb.run_as(ship, &args);
        assert!(!r.out.contains("(authority)"), "{args:?}: {}", r.out);
    }
    let r = sb.run_as(ship, &["sweep", ship, "--plan"]);
    assert_eq!(r.code, 0, "its own stale rows: {}", r.out);
    let r = sb.run_as(ship, &["sweep", "MAIN/ARGOS", "--plan"]);
    assert_eq!(r.code, 1);
    assert!(r.out.contains("(authority)") && r.out.contains("may sweep only itself"), "{}", r.out);
    // a fix a ship may not run itself is marked for its commodore / the user
    let r = sb.run_as(ship, &["start", "MAIN/ARGOS/STOPPED", "--plan"]);
    assert!(r.out.contains("(authority)"), "{}", r.out);
    assert!(!r.out.lines().any(|l| l.trim_start().starts_with("fix: nimitz start")), "{}", r.out);
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
    // every live lead gets a tab that attaches to its job; a lead that is not
    // live gets none (`nimitz start` first)
    let r = sb.run(&["layout", "herdr", "MAIN", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("tab MAIN.ADMIRAL in workspace MAIN") && r.out.contains("tab MAIN.ARGOS.COMMODORE in workspace MAIN.ARGOS"), "{}", r.out);
    assert!(sb.calls().contains(" attach ") && sb.calls().contains("MAIN.ADMIRAL") && !sb.calls().contains("\"-c\""), "{}", sb.calls());
    let r = sb.run(&["layout", "herdr", "BETA", "--plan"]);
    assert!(r.out.contains("BETA has no job") && r.out.contains("nimitz start BETA"), "{}", r.out);
    // again: the tabs are there already (by label), so nothing is attached twice
    let r = sb.run(&["layout", "herdr", "MAIN", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let runs = sb.calls().lines().filter(|l| l.contains("\"pane\",\"run\"") && l.contains("attach")).count();
    assert_eq!(runs, 3, "one attach per lead, once:\n{}", sb.calls());
    assert_eq!(sb.run_as("MAIN/ARGOS", &["layout", "herdr", "--yes"]).code, 1);
    // --ships: attach tabs for live ships; stopped ones are skipped
    let r = sb.run(&["layout", "herdr", "MAIN", "--ships", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("tab MAIN.ARGOS.CLEAN in workspace MAIN.ARGOS"), "{}", r.out);
    assert!(r.out.contains("MAIN/ARGOS/STOPPED is not live; no tab"), "{}", r.out);
    assert!(sb.calls().contains(" attach "), "{}", sb.calls());
}

// ------------------------------------------------------------------ hook

const EVENTS: [&str; 9] =
    ["SessionStart", "SessionEnd", "UserPromptSubmit", "Stop", "Notification", "SubagentStart", "SubagentStop", "PreToolUse", "PermissionRequest"];

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

// ------------------------------------------------------------------ needs-you

const ASK: &str = r#"{"session_id":"s-clean","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_use_id":"t1",
  "tool_input":{"questions":[{"question":"Ship the fix to main?","header":"Ship","multiSelect":false,
  "options":[{"label":"Yes","description":"merge now"},{"label":"No","description":"wait for review"}]}]}}"#;
const CLEAN: [(&str, &str); 3] = [("NIMITZ_ID", "MAIN/ARGOS/CLEAN"), ("NIMITZ_ROLE", "ship"), ("NIMITZ_NOTIFY_RETRY", "0")];

impl Sb {
    /// The notice on file for `id` once its delivery has finished (or None).
    fn notice(&self, id: &str) -> Option<Value> {
        let p = self.base.join("state/needs").join(format!("{}.json", id.replace('/', "%2F")));
        let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let v: Option<Value> = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok());
            match &v {
                Some(n) if n["delivery"] == "pending" && std::time::Instant::now() < end => {}
                _ => return v,
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    fn prompts(&self, pane: &str) -> Vec<String> {
        let w: Value = serde_json::from_slice(&std::fs::read(self.base.join("herdr/world.json")).unwrap()).unwrap();
        let p = w["panes"].as_array().unwrap().iter().find(|p| p["pane_id"] == pane).cloned().unwrap_or_default();
        p["prompts"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
    }
}

#[test]
fn a_ship_that_asks_the_user_tells_its_commodore_with_the_question_in_full() {
    let sb = Sb::new(true);
    sb.pane_for("MAIN/ARGOS");
    let t = std::time::Instant::now();
    let r = raw(&sb.vars, &["hook", "PreToolUse"], &CLEAN, Some(ASK));
    assert!(t.elapsed() < std::time::Duration::from_secs(2), "the hook does not wait for the delivery");
    assert_eq!((r.code, r.out.trim()), (0, ""), "no decision: the question still shows");
    let n = sb.notice("MAIN/ARGOS/CLEAN").expect("a notice");
    assert_eq!(n["delivery"], "delivered", "{n}");
    let typed = sb.prompts("w1:p2");
    assert_eq!(typed.len(), 1, "{typed:?}");
    let t = &typed[0];
    assert!(t.starts_with("[nimitz needs-you] (a notice from nimitz, not from the user) MAIN/ARGOS/CLEAN (ship"), "{t}");
    assert!(t.contains("Q1: Ship the fix to main? [Ship] Options: (1) Yes: merge now; (2) No: wait for review"), "{t}");
    // the same dialog's Notification is not a second notice
    let note = r#"{"session_id":"s-clean","notification_type":"permission_prompt","message":"Claude needs your input"}"#;
    raw(&sb.vars, &["hook", "Notification"], &CLEAN, Some(note));
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(sb.prompts("w1:p2").len(), 1);
    // the event log says why
    let ev = std::fs::read_to_string(sb.base.join("state/events.jsonl")).unwrap();
    assert!(ev.lines().any(|l| l.contains("\"needs-you: question\"") && l.contains("AskUserQuestion")), "{ev}");
    // answered: the next Stop closes the notice
    raw(&sb.vars, &["hook", "Stop"], &CLEAN, Some(r#"{"session_id":"s-clean"}"#));
    assert!(sb.notice("MAIN/ARGOS/CLEAN").is_none());
}

#[test]
fn a_ship_permission_prompt_reaches_its_commodore() {
    let sb = Sb::new(true);
    sb.pane_for("MAIN/ARGOS");
    let input = r#"{"session_id":"s-clean","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"git push --force"}}"#;
    let ship = [("NIMITZ_ID", "MAIN/ARGOS/CLEAN"), ("NIMITZ_ROLE", "ship"), ("NIMITZ_NOTIFY_RETRY", "0")];
    let r = raw(&sb.vars, &["hook", "PermissionRequest"], &ship, Some(input));
    assert_eq!((r.code, r.out.trim()), (0, ""), "no decision: the prompt still shows");
    assert_eq!(sb.notice("MAIN/ARGOS/CLEAN").unwrap()["delivery"], "delivered");
    let typed = sb.prompts("w1:p2").join("\n");
    assert!(typed.contains("MAIN/ARGOS/CLEAN (ship") && typed.contains("permission") && typed.contains("git push --force"), "{typed}");
}

/// A 0.3.x tender's job row (plan-v4 section 8): `check` shows it once as
/// `tender.retired` (Info) with `nimitz rm <short>`; status lists no tender;
/// the id is no longer an id at all.
#[test]
fn a_retired_tender_row_is_checked_once_and_removed_by_rm() {
    let sb = Sb::new(false);
    let s = sb.status();
    for sq in s["fleets"].as_array().unwrap().iter().flat_map(|f| f["squadrons"].as_array().unwrap()) {
        assert!(sq.get("tenders").is_none(), "no tenders key: {sq}");
        assert!(sq["ships"].as_array().unwrap().iter().all(|m| !m["id"].as_str().unwrap().contains('~')));
    }
    let r = sb.run(&["check"]);
    let lines: Vec<&str> = r.out.lines().collect();
    let at: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.contains("tender.retired")).map(|(i, _)| i).collect();
    assert_eq!(at.len(), 1, "{}", r.out);
    assert!(lines[at[0]].starts_with("[INFO]") && lines[at[0]].contains("MAIN/ARGOS/~fixci"), "{}", lines[at[0]]);
    let short = lines[at[0] + 1].trim().strip_prefix("fix: nimitz rm ").map(|t| t.split_whitespace().next().unwrap().to_string()).unwrap_or_else(|| panic!("the fix: {}", lines[at[0] + 1]));
    assert_eq!(sb.run(&["start", "MAIN/ARGOS/~fixci", "--plan"]).code, 2, "not an id");
    let r = sb.run(&["rm", &short, "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(!sb.run(&["check"]).out.contains("tender.retired"));
}

#[test]
fn a_notice_that_cannot_be_delivered_is_a_check_issue() {
    let sb = Sb::new(true);
    // the commodore is live but in no pane
    raw(&sb.vars, &["hook", "PreToolUse"], &CLEAN, Some(ASK));
    let n = sb.notice("MAIN/ARGOS/CLEAN").unwrap();
    assert_eq!(n["delivery"], "failed", "{n}");
    assert!(n["delivery_note"].as_str().unwrap().contains("not live in a herdr pane"), "{n}");
    let r = sb.run(&["check", "MAIN/ARGOS/CLEAN"]);
    assert!(r.out.contains("notify.failed") && r.out.contains("Ship the fix to main?"), "{}", r.out);
    // a pane busy with its own dialog is never typed into
    sb.pane_for("MAIN/ARGOS");
    sb.herdr_world(|w| w["panes"][1]["agent_status"] = Value::from("blocked"));
    raw(&sb.vars, &["hook", "Stop"], &CLEAN, Some(r#"{"session_id":"s-clean"}"#));
    raw(&sb.vars, &["hook", "PreToolUse"], &CLEAN, Some(ASK));
    let n = sb.notice("MAIN/ARGOS/CLEAN").unwrap();
    assert!(n["delivery_note"].as_str().unwrap().contains("blocked on a dialog of its own"), "{n}");
    assert!(sb.prompts("w1:p2").is_empty());
    // no multiplexer in the member's env
    raw(&sb.vars, &["hook", "Stop"], &CLEAN, Some(r#"{"session_id":"s-clean"}"#));
    let mut nomux = CLEAN.to_vec();
    nomux.push(("NIMITZ_MUX", "none"));
    raw(&sb.vars, &["hook", "PreToolUse"], &nomux, Some(ASK));
    let n = sb.notice("MAIN/ARGOS/CLEAN").unwrap();
    assert!(n["delivery_note"].as_str().unwrap().contains("no multiplexer"), "{n}");
}

#[test]
fn only_the_member_itself_delivers_its_notice() {
    let sb = Sb::new(true);
    sb.pane_for("MAIN/ARGOS");
    raw(&sb.vars, &["hook", "PreToolUse"], &CLEAN, Some(ASK));
    sb.notice("MAIN/ARGOS/CLEAN");
    let before = sb.prompts("w1:p2").len();
    // the user, another ship, a commodore: refused, nothing typed
    assert_eq!(raw(&sb.vars, &["__notify", "MAIN/ARGOS/CLEAN"], &[], None).code, 2);
    assert_eq!(sb.run_as("MAIN/ARGOS/OTHER", &["__notify", "MAIN/ARGOS/CLEAN"]).code, 2);
    assert_eq!(sb.run_as("MAIN/ARGOS", &["__notify", "MAIN/ARGOS/CLEAN"]).code, 2);
    assert_eq!(sb.prompts("w1:p2").len(), before);
    // an admiral's or commodore's own question is not a notice (the user watches its pane)
    let lead = [("NIMITZ_ID", "MAIN/ARGOS"), ("NIMITZ_ROLE", "commodore")];
    raw(&sb.vars, &["hook", "PreToolUse"], &lead, Some(ASK));
    assert!(sb.notice("MAIN/ARGOS").is_none());
}

/// Acc-D F6: a daemon-respawned ship inherits the daemon's env, the user's
/// live `HERDR_*`, in whatever world its labels name. Its own `status`
/// records no Herdr identity from that env, and its needs-you delivery never
/// goes by it alone: nothing explicit and no server recorded by the user's
/// own command means no multiplexer; a recorded server is the `--session`
/// of every call of the delivery.
#[test]
fn a_delivery_never_uses_an_inherited_herdr_env_alone() {
    let sb = Sb::new(true);
    sb.pane_for("MAIN/ARGOS");
    let active = PathBuf::from(&sb.vars["NIMITZ_STATE"]).join("active.json");
    // (the user's own status above, inside the fake Herdr, recorded its
    // identity: forget it, as a world the user never looked at from Herdr)
    let mut set: Value = serde_json::from_slice(&std::fs::read(&active).unwrap()).unwrap();
    set["herdr"] = Value::Null;
    std::fs::write(&active, set.to_string()).unwrap();
    // the respawned ship's env: HERDR_* inherited, no NIMITZ_MUX of its own
    let mut inherited = sb.vars.clone();
    inherited.remove("NIMITZ_MUX");
    inherited.insert("HERDR_SESSION".into(), "NIMITZ".into());
    inherited.insert("HERDR_SOCKET_PATH".into(), "/live/herdr.sock".into());
    let r = raw(&inherited, &["status"], &CLEAN, None);
    assert_eq!(r.code, 0, "{}", r.out);
    let act: Value = serde_json::from_slice(&std::fs::read(&active).unwrap()).unwrap();
    assert!(act["herdr"].is_null(), "a member's status records no identity from its env: {act}");
    raw(&inherited, &["hook", "PreToolUse"], &CLEAN, Some(ASK));
    let n = sb.notice("MAIN/ARGOS/CLEAN").unwrap();
    assert_eq!(n["delivery"], "failed", "{n}");
    assert!(n["delivery_note"].as_str().unwrap().contains("no multiplexer"), "{n}");
    assert!(sb.prompts("w1:p2").is_empty(), "nothing typed anywhere");
    // the user's own status inside Herdr records the server (by name here);
    // the ship's next delivery goes to that server, named on every call
    let r = raw(&sb.vars, &["status"], &[("NIMITZ_HERDR_SESSION", "nimitz-test")], None);
    assert_eq!(r.code, 0, "{}", r.out);
    let act: Value = serde_json::from_slice(&std::fs::read(&active).unwrap()).unwrap();
    assert_eq!(act["herdr"]["session"], "nimitz-test", "{act}");
    raw(&inherited, &["hook", "Stop"], &CLEAN, Some(r#"{"session_id":"s-clean"}"#));
    let before = sb.calls().lines().count();
    raw(&inherited, &["hook", "PreToolUse"], &CLEAN, Some(ASK));
    let n = sb.notice("MAIN/ARGOS/CLEAN").unwrap();
    assert_eq!(n["delivery"], "delivered", "{n}");
    assert!(!sb.prompts("w1:p2").is_empty());
    let log = sb.calls();
    let calls: Vec<&str> = log.lines().skip(before).collect();
    assert!(!calls.is_empty() && calls.iter().all(|c| c.contains("\"--session\",\"nimitz-test\"")), "{calls:?}");
    // and the ship's status, run again, leaves the recorded identity alone
    raw(&inherited, &["status"], &CLEAN, None);
    let act: Value = serde_json::from_slice(&std::fs::read(&active).unwrap()).unwrap();
    assert_eq!(act["herdr"]["session"], "nimitz-test", "{act}");
}

/// A member's own command never reaches a Herdr through its inherited env
/// (a daemon-restarted worker has the daemon's, the user's live HERDR_*):
/// only an explicit NIMITZ_MUX / NIMITZ_HERDR_SESSION, or the server the
/// user's own command recorded (by its session name, or the default
/// server's socket). The herdr child gets only that server: no inherited
/// HERDR_SESSION or HERDR_SOCKET_PATH.
#[test]
fn a_members_command_never_reaches_herdr_through_its_inherited_env() {
    let sb = Sb::new(true);
    let active = PathBuf::from(&sb.vars["NIMITZ_STATE"]).join("active.json");
    let forget = |herdr: Value| {
        let mut set: Value = std::fs::read(&active).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(serde_json::json!({"version": 1, "members": {}}));
        set["herdr"] = herdr;
        std::fs::write(&active, set.to_string()).unwrap();
    };
    let mut inherited = sb.vars.clone();
    inherited.remove("NIMITZ_MUX");
    inherited.insert("HERDR_ENV".into(), "1".into());
    inherited.insert("HERDR_SESSION".into(), "NIMITZ".into());
    inherited.insert("HERDR_SOCKET_PATH".into(), "/live/herdr.sock".into());
    inherited.insert("HERDR_CLIENT_SOCKET_PATH".into(), "/live/herdr-client.sock".into());
    // nothing recorded: a ship's status and a commodore's up make no herdr call
    forget(Value::Null);
    let before = sb.calls().lines().count();
    assert_eq!(raw(&inherited, &["status"], &CLEAN, None).code, 0);
    raw(&inherited, &["up", "MAIN", "--plan"], &[("NIMITZ_ID", "MAIN/ARGOS"), ("NIMITZ_ROLE", "commodore")], None);
    assert_eq!(sb.calls().lines().count(), before, "no herdr call from a member's inherited env: {}", sb.calls());
    // nothing recorded: a delivery fails with the fix, and types nothing
    raw(&inherited, &["hook", "PreToolUse"], &CLEAN, Some(ASK));
    assert_eq!(sb.calls().lines().count(), before, "{}", sb.calls());
    let n = sb.notice("MAIN/ARGOS/CLEAN").unwrap();
    assert_eq!(n["delivery"], "failed", "{n}");
    assert!(n["delivery_note"].as_str().unwrap().contains("run nimitz status inside Herdr once"), "{n}");
    // what the member's herdr CHILD gets (the fake logs its env per call)
    let child_env = |from: usize| -> Vec<(Value, Value)> {
        sb.calls().lines().skip(from).map(|l| serde_json::from_str::<Value>(l).unwrap()).map(|c| (c["args"].clone(), c["env"].clone())).collect()
    };
    // a record of the default server (no session name) beside an inherited
    // HERDR_SESSION: every call goes to the recorded socket, no session name
    forget(serde_json::json!({"socket": "/recorded/herdr.sock"}));
    let from = sb.calls().lines().count();
    assert_eq!(raw(&inherited, &["status"], &CLEAN, None).code, 0);
    let got = child_env(from);
    assert!(!got.is_empty(), "the member's status read the recorded server");
    for (args, env) in &got {
        assert_eq!(env["HERDR_SOCKET_PATH"], "/recorded/herdr.sock", "{args} {env}");
        assert!(env.get("HERDR_SESSION").is_none() && env.get("HERDR_CLIENT_SOCKET_PATH").is_none() && !args.to_string().contains("--session"), "{args} {env}");
    }
    // a record by session name (a test server): --session on every call, and
    // neither the inherited HERDR_SESSION nor HERDR_SOCKET_PATH reaches the child
    forget(serde_json::json!({"session": "nimitz-test"}));
    let from = sb.calls().lines().count();
    assert_eq!(raw(&inherited, &["status"], &CLEAN, None).code, 0);
    let got = child_env(from);
    assert!(!got.is_empty());
    for (args, env) in &got {
        assert!(args.to_string().contains("\"--session\",\"nimitz-test\""), "{args}");
        assert!(env.get("HERDR_SESSION").is_none() && env.get("HERDR_SOCKET_PATH").is_none() && env.get("HERDR_CLIENT_SOCKET_PATH").is_none(), "{args} {env}");
    }
    // the user's own command still detects from its env
    let r = raw(&inherited, &["status"], &[], None);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(sb.calls().lines().count() > before, "the user's status reads the shape");
}

#[test]
fn a_labelled_session_gets_its_rules_at_start_and_rules_prints_them() {
    let sb = Sb::new(false);
    let r = sb.run(&["rules", "MAIN/ARGOS"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("## Commodore of MAIN/ARGOS") && r.out.contains("[nimitz needs-you]"), "{}", r.out);
    assert_eq!(sb.run(&["rules"]).code, 2, "the user names the id");
    let own = sb.run_as("MAIN/ARGOS/CLEAN", &["rules"]);
    assert!(own.out.contains("## Ship of MAIN/ARGOS"), "a member: its own: {}", own.out);
    // the SessionStart hook hands the same text over as context
    let input = r#"{"session_id":"s-r","source":"compact","hook_event_name":"SessionStart"}"#;
    let h = raw(&sb.vars, &["hook", "SessionStart"], &[("NIMITZ_ID", "MAIN/ARGOS"), ("NIMITZ_ROLE", "commodore")], Some(input));
    let v: Value = serde_json::from_str(h.out.trim()).expect("hook JSON");
    assert_eq!(v["hookSpecificOutput"]["additionalContext"].as_str().unwrap(), r.out);
    // a live member whose session started before versioned rules (the sandbox's
    // SessionStart lines carry no `rules`): check names it (Info, nothing to
    // do: the hook hands it the update with its next prompt)
    let delivered = sb.base.join("state/rules-delivered.json");
    let c = sb.run(&["check", "MAIN/ARGOS/CLEAN"]);
    assert!(c.out.contains("rules.old") && c.out.contains("next prompt") && !c.out.contains("/compact"), "{}", c.out);
    // its next prompt: the UserPromptSubmit hook hands over the changelog and the full text, once
    let clean = sb.member("MAIN/ARGOS/CLEAN");
    let sid = clean["session"]["session_id"].as_str().unwrap();
    let prompt = format!(r#"{{"session_id":"{sid}","hook_event_name":"UserPromptSubmit","prompt":"go"}}"#);
    let labels = [("NIMITZ_ID", "MAIN/ARGOS/CLEAN"), ("NIMITZ_ROLE", "ship")];
    let u = raw(&sb.vars, &["hook", "UserPromptSubmit"], &labels, Some(&prompt));
    let v: Value = serde_json::from_str(u.out.trim()).expect("hook JSON");
    let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
    assert!(ctx.starts_with(&format!("nimitz rules updated v0→v{}", nimitz_core::rules::VERSION)) && ctx.contains("## Ship of MAIN/ARGOS"), "{ctx}");
    assert!(ctx.contains("- v1:") && ctx.contains("- v2:"), "the whole changelog from v0: {ctx}");
    assert!(raw(&sb.vars, &["hook", "UserPromptSubmit"], &labels, Some(&prompt)).out.trim().is_empty(), "once per version");
    let rec: Value = serde_json::from_str(&std::fs::read_to_string(&delivered).unwrap()).unwrap();
    assert_eq!(rec[format!("session:{sid}")]["version"], nimitz_core::rules::VERSION, "no job id readable here: keyed by session");
    let c = sb.run(&["check", "MAIN/ARGOS/CLEAN"]);
    assert!(!c.out.contains("MAIN/ARGOS/CLEAN: runs on nimitz rules"), "delivered; its parents still wait for their next prompt:\n{}", c.out);
    assert!(c.out.contains("MAIN/ARGOS: runs on nimitz rules v0"), "{}", c.out);
}

#[test]
fn version_prints_the_crate_version() {
    for flag in ["--version", "-V", "version"] {
        let r = raw(&BTreeMap::new(), &[flag], &[], None);
        assert_eq!((r.code, r.out.trim()), (0, format!("nimitz {}", env!("CARGO_PKG_VERSION")).as_str()), "{flag}");
    }
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
    assert_eq!(reg["version"], nimitz_core::REGISTRY_VERSION, "init writes the format this nimitz writes, not a v1 file");
    assert_eq!(raw(&vars, &["init", "--label", "other", "--yes"], &[], None).code, 1, "a label is not changed silently");

    // trust it, enable the plugin; doctor still wants crossSessionInbound, and --fix sets it
    std::fs::create_dir_all(b.join("claude_home")).unwrap();
    std::fs::write(b.join("claude_home/.claude.json"), format!(r#"{{"projects":{{"{}":{{"hasTrustDialogAccepted":true}}}}}}"#, p("nimitz"))).unwrap();
    std::fs::write(b.join("claude_home/settings.json"), r#"{"enabledPlugins":{"nimitz@claude-nimitz":true},"model":"x"}"#).unwrap();
    // a fleet whose admiral brief an older nimitz wrote, and a squadron brief edited by hand
    for args in [vec!["fleet", "new", "TOP", "--yes"], vec!["squadron", "new", "TOP/SQ", "--brief", "x", "--yes"]] {
        let r = raw(&vars, &args, &[], None);
        assert_eq!(r.code, 0, "{args:?}: {}", r.out);
    }
    let adm = b.join("nimitz/top/admiral/CLAUDE.md");
    let old = "# [TOP]-ADMIRAL\n\nYou are the admiral of fleet TOP. You watch the fleet and talk to its\ncommodores (questions only). You never start, stop or edit anything:\n`nimitz status` for the picture, and the user for every change.\n";
    std::fs::write(&adm, old).unwrap();
    let sq = b.join("nimitz/top/squadrons/sq/CLAUDE.md");
    std::fs::write(&sq, "# mine\n").unwrap();
    let d = raw(&vars, &["doctor"], &[], None);
    assert_eq!(d.code, 1, "{}", d.out);
    assert!(d.out.contains("crossSessionInbound is unset"), "{}", d.out);
    assert!(d.out.contains("TOP: ") && d.out.contains("written by an older nimitz"), "{}", d.out);
    assert!(d.out.contains("TOP/SQ: ") && d.out.contains("edited by hand"), "{}", d.out);
    assert!(d.out.contains("[OK] native claude"), "{}", d.out);
    assert!(d.out.contains("[WARN] hooks: the nimitz plugin is enabled; its installed copy is not listed"), "no installed_plugins.json yet:\n{}", d.out);
    // the installed plugin's hooks.json: found through installed_plugins.json; every event present is OK
    let repo_hooks = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin/hooks/hooks.json");
    let install = b.join("claude_home/plugins/cache/claude-nimitz/nimitz/0.4.0");
    std::fs::create_dir_all(install.join("hooks")).unwrap();
    std::fs::copy(&repo_hooks, install.join("hooks/hooks.json")).unwrap();
    std::fs::write(
        b.join("claude_home/plugins/installed_plugins.json"),
        format!(r#"{{"version": 2, "plugins": {{"nimitz@claude-nimitz": [{{"scope": "user", "installPath": "{}", "version": "0.4.0"}}]}}}}"#, install.display()),
    )
    .unwrap();
    let d = raw(&vars, &["doctor"], &[], None);
    assert!(d.out.contains("[OK] hooks: the nimitz plugin is enabled and") && d.out.contains("registers every event nimitz needs"), "{}", d.out);
    // an older installed copy lacks an event: named, with /reload-plugins as the fix (NIMITZ_PLUGIN_HOOKS points doctor at a file)
    let mut old: Value = serde_json::from_str(&std::fs::read_to_string(&repo_hooks).unwrap()).unwrap();
    old["hooks"].as_object_mut().unwrap().remove("UserPromptSubmit");
    let old_path = b.join("old-hooks.json");
    std::fs::write(&old_path, old.to_string()).unwrap();
    let d = raw(&vars, &["doctor"], &[("NIMITZ_PLUGIN_HOOKS", old_path.to_str().unwrap())], None);
    assert!(d.out.contains("does not register UserPromptSubmit") && d.out.contains("fix: /reload-plugins"), "{}", d.out);
    // a copy whose SessionStart entry is one command string: a Warn, reinstall + /reload-plugins
    let mut stringy: Value = serde_json::from_str(&std::fs::read_to_string(&repo_hooks).unwrap()).unwrap();
    stringy["hooks"]["SessionStart"] = serde_json::json!([{"hooks": [{"type": "command", "command": "nimitz hook SessionStart", "timeout": 5}]}]);
    let stringy_path = b.join("stringy-hooks.json");
    std::fs::write(&stringy_path, stringy.to_string()).unwrap();
    let d = raw(&vars, &["doctor"], &[("NIMITZ_PLUGIN_HOOKS", stringy_path.to_str().unwrap())], None);
    assert!(d.out.contains("[WARN] hooks:") && d.out.contains("gives SessionStart as a command string") && d.out.contains("claude plugin install nimitz@claude-nimitz, then /reload-plugins"), "{}", d.out);
    assert!(d.out.contains("registers every event nimitz needs"), "still registered: {}", d.out);
    let f = raw(&vars, &["doctor", "--fix", "--yes"], &[], None);
    assert_eq!(f.code, 0, "{}", f.out);
    let s: Value = serde_json::from_str(&std::fs::read_to_string(b.join("claude_home/settings.json")).unwrap()).unwrap();
    assert_eq!(s["crossSessionInbound"], "accept");
    assert_eq!(s["model"], "x", "other settings kept");
    let brief = std::fs::read_to_string(&adm).unwrap();
    assert!(brief.contains("`nimitz rules TOP` prints them") && !brief.contains("questions only"), "{brief}");
    assert_eq!(std::fs::read_to_string(&sq).unwrap(), "# mine\n", "a hand-edited brief is left alone");
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
    // a job row with no name (a copy of a running job): not a format problem (acceptance F5)
    std::fs::write(&one, r#"{"pid": 1, "sessionId": "x", "status": "idle", "kind": "bg"}"#).unwrap();
    let jobs = sb.base.join("claude_home/jobs/0a0a0a0a");
    std::fs::create_dir_all(&jobs).unwrap();
    std::fs::write(jobs.join("state.json"), r#"{"sessionId": "0a0a0a0a-0000-4000-8000-000000000000", "state": "done", "cwd": "/w"}"#).unwrap();
    let d = sb.run(&["doctor"]);
    assert!(!d.out.contains("lacks") && d.out.contains("carry every field nimitz reads"), "{}", d.out);
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

// ------------------------------------------------------------------ 0.4.0: one broken fleet.toml blocks only its fleet (M8), conflict copies (M23)

#[test]
fn a_broken_fleet_toml_blocks_its_own_fleet_only() {
    let sb = Sb::new(false);
    let bad = sb.base.join("nimitz/bad");
    std::fs::create_dir_all(bad.join("admiral")).unwrap();
    let file = bad.join("fleet.toml");
    std::fs::write(&file, "fleet = \"BAD\"\n\n[m").unwrap();
    let file = file.display().to_string();
    // status: the other fleets as before, plus the broken one with its error
    let st = sb.run(&["status"]);
    assert_eq!(st.code, 0, "{}", st.out);
    for want in ["MAIN  (", "BETA  (", "BAD  (", &format!("{file}:3:"), "(config.invalid)", &format!("fix: fix {file} line 3, then nimitz check BAD")] {
        assert!(st.out.contains(want), "{want:?} missing:\n{}", st.out);
    }
    assert_eq!(st.out.matches("config.invalid").count(), 1, "once:\n{}", st.out);
    // check of the broken fleet names the file, the line and the parse error
    let c = sb.run(&["check", "BAD"]);
    assert_eq!(c.code, 1, "{}", c.out);
    assert!(c.out.contains(&format!("{file}:3:")) && c.out.contains("unclosed table"), "{}", c.out);
    assert!(c.out.contains("[ERROR] check BAD: 1 error(s)"), "{}", c.out);
    // the healthy fleets check and plan as before
    let c = sb.run(&["check", "MAIN/ARGOS/FRESH"]);
    assert!(!c.out.contains("config.invalid"), "{}", c.out);
    let p = sb.run(&["start", "MAIN/ARGOS/FRESH", "--plan"]);
    assert_eq!(p.code, 0, "{}", p.out);
    assert!(p.out.contains("[PLAN]"), "{}", p.out);
    // anything aimed at the broken fleet refuses with that error and the one fix
    for args in [vec!["start", "BAD", "--plan"], vec!["up", "BAD", "--plan"], vec!["fleet", "set", "BAD", "--admiral-mode", "auto", "--plan"]] {
        let r = sb.run(&args);
        assert_eq!(r.code, 1, "{args:?}:\n{}", r.out);
        assert!(r.out.contains(&format!("{file}:3:")) && r.out.contains(&format!("fix {file} line 3")), "{args:?}:\n{}", r.out);
    }
}

#[test]
fn a_dropbox_conflicted_copy_of_a_config_is_a_warning_with_both_names() {
    let sb = Sb::new(false);
    let sq_dir = sb.base.join("nimitz/main/squadrons/argos");
    let copy = sq_dir.join("squadron (box2's conflicted copy 2026-10-07).toml");
    std::fs::write(&copy, "squadron = \"ARGOS\"\n").unwrap();
    let c = sb.run(&["check", "MAIN/ARGOS"]);
    let line = c.out.lines().find(|l| l.contains("config.conflict")).unwrap_or_else(|| panic!("no config.conflict:\n{}", c.out)).to_string();
    assert!(line.starts_with("[WARN]"), "{line}");
    assert!(line.contains(&copy.display().to_string()) && line.contains(&sq_dir.join("squadron.toml").display().to_string()), "{line}");
    assert!(c.out.contains("fix: compare the two files, keep one, delete the other"), "{}", c.out);
    // not in another squadron's or ship's check
    assert!(!sb.run(&["check", "MAIN/BOREAL"]).out.contains("config.conflict"));
    std::fs::remove_file(&copy).unwrap();
    assert!(!sb.run(&["check", "MAIN/ARGOS"]).out.contains("config.conflict"));
}

/// E8: a lost transcript is `history.lost` from the CLI too, and `--fresh` is
/// the way on (a cold launch, verified).
#[test]
fn start_refuses_a_lost_history_and_fresh_cold_launches() {
    let sb = Sb::new(false);
    let sid = sb.member("MAIN/ARGOS/STOPPED")["session"]["session_id"].as_str().unwrap().to_string();
    let log = walkdir(&sb.base.join("claude_home/projects")).into_iter().find(|p| p.file_name().map(|n| n.to_string_lossy() == format!("{sid}.jsonl")).unwrap_or(false)).expect("the transcript");
    std::fs::remove_file(log).unwrap();
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--plan"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(history.lost)") && r.out.contains("nimitz start MAIN/ARGOS/STOPPED --fresh"), "{}", r.out);
    assert!(!r.out.contains("--resume"), "no launch planned:\n{}", r.out);
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--fresh", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("verified by re-reading"), "{}", r.out);
    let m = sb.member("MAIN/ARGOS/STOPPED");
    assert!(live(&m) && m["session"]["session_id"] != sid, "{m}");
}

fn walkdir(d: &Path) -> Vec<PathBuf> {
    let mut v = vec![];
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            v.extend(walkdir(&p));
        } else {
            v.push(p);
        }
    }
    v
}

/// `/clear` in a ship, then stop, then start: the wake names the job's BIRTH
/// id (Claude Code 2.1.292 wakes a stopped job only by that id; any other id
/// starts a new unlabelled job), the same job comes back on the post-clear
/// conversation with its labels, and nothing is removed (acceptance F1).
#[test]
fn clear_then_stop_then_start_wakes_the_same_job_by_its_birth_id() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    let (job, birth) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).args(args).output().unwrap();
    assert!(fake(&["__inside", &job, "clear"]).status.success());
    let m = sb.member("MAIN/ARGOS/CLEAN");
    let now = m["session"]["session_id"].as_str().unwrap().to_string();
    assert_ne!(now, birth);
    assert_eq!(m["session"]["short_id"], job.as_str(), "the same job after /clear");
    let r = sb.run(&["stop", "MAIN/ARGOS/CLEAN", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    // the plan: the native resume by the birth id, no rm of the job's own row
    let r = sb.run(&["start", "MAIN/ARGOS/CLEAN", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains(&format!("--bg --resume {birth}")) && !r.out.contains(&format!("--resume {now}")), "{}", r.out);
    assert!(!r.out.contains(" rm ") && !r.out.contains("stale"), "the job's own row is not a copy: {}", r.out);
    let r = sb.run(&["start", "MAIN/ARGOS/CLEAN", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains(&format!("woke session {birth}")) && r.out.contains("verified by re-reading"), "{}", r.out);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert!(live(&m), "{m}");
    assert_eq!((m["session"]["short_id"].as_str(), m["session"]["session_id"].as_str()), (Some(job.as_str()), Some(now.as_str())), "{m}");
    assert_eq!(m["session"]["labels"]["NIMITZ_ID"], "MAIN/ARGOS/CLEAN");
    assert!(m["extra_rows"].as_array().unwrap().is_empty() && m["issues"].as_array().unwrap().is_empty(), "{m}");
    let r = sb.run(&["check", "MAIN/ARGOS/CLEAN"]);
    assert_eq!(r.code, 0, "{}", r.out);
}

/// The wrong wake by hand (`--bg --resume <conversation now>` of a stopped
/// job, what nimitz did before F1): Claude Code starts a new unlabelled job
/// named after that id. nimitz shows it as a copy of the ship, keeps the
/// ship's own row as its session, and `sweep` removes the copy, never the
/// own row.
#[test]
fn a_wake_by_a_non_birth_id_is_a_copy_and_sweep_keeps_the_own_row() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    let job = m["session"]["short_id"].as_str().unwrap().to_string();
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).args(args).output().unwrap();
    assert!(fake(&["__inside", &job, "clear"]).status.success());
    let now = sb.member("MAIN/ARGOS/CLEAN")["session"]["session_id"].as_str().unwrap().to_string();
    assert_eq!(sb.run(&["stop", "MAIN/ARGOS/CLEAN", "--yes"]).code, 0);
    // (from the ship's folder, as nimitz's own wake and a user at a prompt would)
    let o = Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).current_dir(m["cwd"].as_str().unwrap()).args(["--bg", "--resume", &now]).output().unwrap();
    let out = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert!(out.contains("backgrounded ·") && !out.contains("woke session"), "{out}");
    let copy = now[..8].to_string();
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert_eq!(m["session"]["short_id"], job.as_str(), "the own row stays the member's session: {m}");
    let extra = m["extra_rows"].as_array().unwrap();
    assert_eq!(extra.len(), 1, "{m}");
    assert_eq!(extra[0]["short_id"], copy.as_str());
    assert!(!extra[0]["copy"].is_null(), "the new job is a copy: {}", extra[0]);
    assert!(m["issues"].as_array().unwrap().iter().any(|i| i["code"] == "copy"), "{m}");
    assert!(!m["issues"].as_array().unwrap().iter().any(|i| i["code"] == "dup.live" || i["code"] == "labels.missing"), "{m}");
    // a start plan never drops the own row
    let r = sb.run(&["start", "MAIN/ARGOS/CLEAN", "--plan"]);
    assert!(!r.out.contains(&format!("rm {job}")), "{}", r.out);
    // sweep: the copy goes (stopped first), the own row stays
    let r = sb.run(&["sweep", "MAIN/ARGOS/CLEAN", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains(&format!("stop {copy}")) && r.out.contains(&format!("rm {copy}")) && !r.out.contains(&format!("rm {job}")), "{}", r.out);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert_eq!(m["session"]["short_id"], job.as_str(), "{m}");
    assert!(m["extra_rows"].as_array().unwrap().is_empty(), "{m}");
    // and the right wake still works after it
    let r = sb.run(&["start", "MAIN/ARGOS/CLEAN", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert_eq!(sb.member("MAIN/ARGOS/CLEAN")["session"]["session_id"], now.as_str());
}

/// `squadron new --mode M` sets the squadron's default permission mode for
/// its ships (contracts.md section 3), as USAGE says; a bad mode is refused
/// (acceptance F2).
#[test]
fn squadron_new_takes_a_mode() {
    let sb = Sb::new(false);
    let r = sb.run(&["squadron", "new", "MAIN/MODED", "--brief", "x", "--mode", "default", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let toml = std::fs::read_to_string(sb.base.join("nimitz/main/squadrons/moded/squadron.toml")).unwrap();
    assert!(toml.contains("permission_mode = \"default\""), "{toml}");
    let r = sb.run(&["squadron", "new", "MAIN/MODED2", "--brief", "x", "--mode", "bogus", "--plan"]);
    assert_ne!(r.code, 0);
    assert!(r.out.contains("config.invalid") && r.out.contains("bogus"), "{}", r.out);
}

/// `start` of a live member and `up` with every member live say why the plan
/// is empty, with --plan and with --yes (acceptance F3, M25).
#[test]
fn empty_plans_say_why() {
    let sb = Sb::new(false);
    for flag in ["--plan", "--yes"] {
        let r = sb.run(&["start", "MAIN/ARGOS/CLEAN", flag]);
        assert_eq!(r.code, 0, "{}", r.out);
        assert!(r.out.contains("nothing to do: MAIN/ARGOS/CLEAN is live"), "{flag}: {}", r.out);
    }
    assert_eq!(sb.run(&["up", "BETA", "--yes"]).code, 0);
    for flag in ["--plan", "--yes"] {
        let r = sb.run(&["up", "BETA", flag]);
        assert_eq!(r.code, 0, "{}", r.out);
        assert!(r.out.contains("nothing to do: every member of BETA on this machine (1) is live"), "{flag}: {}", r.out);
    }
}

/// `/branch` (or `/resume`) inside a ship, then stop, then start: Claude Code
/// wakes a stopped job on its `resumeSessionId`, which follows `/clear` only,
/// so a native wake would bring the pre-branch conversation back in silence.
/// nimitz removes the row and resumes the conversation the member ran last
/// with every flag, and the plan says which (acceptance inside FAIL-1,
/// r3-e2e finding 11).
#[test]
fn branch_then_stop_then_start_resumes_the_branch_with_every_flag() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    let (job, birth) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).args(args).output().unwrap();
    assert!(fake(&["__inside", &job, "branch", "[MAIN.ARGOS]-CLEAN"]).status.success());
    let m = sb.member("MAIN/ARGOS/CLEAN");
    let branch = m["session"]["session_id"].as_str().unwrap().to_string();
    assert_ne!(branch, birth, "{m}");
    assert_eq!(sb.run(&["stop", "MAIN/ARGOS/CLEAN", "--yes"]).code, 0);
    let r = sb.run(&["start", "MAIN/ARGOS/CLEAN", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains(&format!("rm {job}")) && r.out.contains(&format!("--resume {branch} --name")), "{}", r.out);
    assert!(r.out.contains("resumeSessionId follows /clear only") && r.out.contains(&branch[..8]), "the plan says which conversation: {}", r.out);
    assert!(!r.out.contains(&format!("--bg --resume {birth}\n")), "no native wake onto the older conversation: {}", r.out);
    let r = sb.run(&["start", "MAIN/ARGOS/CLEAN", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("verified by re-reading"), "{}", r.out);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert!(live(&m), "{m}");
    assert_eq!(m["session"]["session_id"], branch.as_str(), "the branch is what runs: {m}");
    assert_eq!(m["session"]["labels"]["NIMITZ_ID"], "MAIN/ARGOS/CLEAN");
    assert!(m["extra_rows"].as_array().unwrap().is_empty() && m["issues"].as_array().unwrap().is_empty(), "{m}");
}

/// The agent view's draft spare in a LEAD folder (`spare: true`, no job, no
/// labels, a pre-warmed process the view claimed just now) is ignored for
/// the grace time, as in a ship folder: no `lead.intruder`, no stowaway
/// (acceptance inside FAIL-2).
#[test]
fn a_draft_spare_in_a_lead_folder_is_not_an_intruder() {
    let sb = Sb::new(false);
    let adm = sb.member("MAIN");
    let dir = adm["cwd"].as_str().unwrap().to_string();
    // pre-warmed an hour ago (startedAt), claimed now (the file's mtime)
    let old = (nimitz_core::evidence::now() - 3600.0) * 1000.0;
    let f = PathBuf::from(&sb.vars["NIMITZ_CLAUDE_HOME"]).join("sessions/424242.json");
    // the shape cli-L3b saw: the view claims a pre-warmed `bg-spare` job (a `jobId`, its own id as the name)
    std::fs::write(&f, serde_json::json!({"pid": 1, "sessionId": "spare-draft-0000", "jobId": "spare-dr", "cwd": dir, "kind": "bg", "entrypoint": "cli", "spare": true, "name": "spare-dr", "status": "idle", "startedAt": old}).to_string()).unwrap();
    // the real draft fires a SessionStart `startup` the moment the view claims it (byhand 2c-live)
    let ev = PathBuf::from(&sb.vars["NIMITZ_STATE"]).join("events.jsonl");
    let line = serde_json::json!({"ts": nimitz_core::evidence::now(), "event": "SessionStart", "session_id": "spare-draft-0000", "source": "startup", "reason": null, "pid": 1, "labels": {}, "rules": null, "job": null, "kind": "interactive", "entrypoint": "cli", "binding": null, "cwd": dir});
    let mut all = std::fs::read_to_string(&ev).unwrap_or_default();
    all.push_str(&format!("{line}\n"));
    std::fs::write(&ev, all).unwrap();
    let s = sb.status();
    let text = s.to_string();
    assert!(!text.contains("spare-draft-0000"), "the draft spare shows: {text}");
    let adm = sb.member("MAIN");
    assert!(!adm["issues"].as_array().unwrap().iter().any(|i| i["code"] == "lead.intruder"), "{adm}");
    let r = sb.run(&["check", "MAIN"]);
    assert!(!r.out.contains("lead.intruder") && !r.out.contains("spare-draft-0000"), "{}", r.out);
}

/// `/fork` in an attached lead: the fork is a second live job in the lead's
/// exclusive folder that is not its registry job: `lead.intruder` with the
/// one fix (stop, then rm the fork), never `dup.live` or a copy row of the
/// lead, and the ships under it still start (acceptance inside FAIL-3).
#[test]
fn a_fork_of_a_lead_is_its_intruder_and_ships_still_start() {
    let sb = Sb::new(false);
    let adm = sb.member("MAIN");
    let job = adm["session"]["short_id"].as_str().unwrap().to_string();
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).args(args).output().unwrap();
    let o = fake(&["__inside", &job, "fork"]);
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    let fork = out.trim().rsplit(' ').next().unwrap().to_string();
    assert!(o.status.success() && fork.len() == 8, "{out}");
    let adm = sb.member("MAIN");
    assert_eq!(adm["session"]["short_id"], job.as_str(), "the lead keeps its job: {adm}");
    let codes: Vec<&str> = adm["issues"].as_array().unwrap().iter().filter_map(|i| i["code"].as_str()).collect();
    assert!(codes.contains(&"lead.intruder"), "{adm}");
    assert!(!codes.contains(&"dup.live") && !codes.contains(&"copy.rows"), "{adm}");
    let intruder = adm["issues"].as_array().unwrap().iter().find(|i| i["code"] == "lead.intruder").unwrap();
    assert!(intruder["message"].as_str().unwrap().contains(&fork), "{intruder}");
    assert_eq!(intruder["fix"], format!("nimitz stop {fork}  # then nimitz rm {fork}"));
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--plan"]);
    assert_eq!(r.code, 0, "a ship under the lead still starts: {}", r.out);
    assert!(r.out.contains("--bg --resume"), "{}", r.out);
    // the fix works: stop + rm the fork, the lead is clean
    assert_eq!(sb.run(&["stop", &fork, "--yes"]).code, 0);
    assert_eq!(sb.run(&["rm", &fork, "--yes"]).code, 0);
    assert!(sb.member("MAIN")["issues"].as_array().unwrap().iter().all(|i| i["code"] != "lead.intruder"));
}

fn lead_labels(sb: &Sb) -> String {
    serde_json::json!({"env": {"NIMITZ_FLEET": "MAIN", "NIMITZ_ROLE": "admiral", "NIMITZ_ID": "MAIN", "NIMITZ_HOME": sb.vars["NIMITZ_HOME"]}}).to_string()
}

/// The registry job stays the lead beside an OLDER labelled stray: a relaunch
/// (rm + resume with every flag) binds a job that started later than a stray
/// launched by hand with the lead's labels and name; the stray is the
/// intruder, and the fix never names the real admiral (re-review 1).
#[test]
fn the_registry_job_stays_the_lead_beside_an_older_labelled_stray() {
    let sb = Sb::new(false);
    let adm = sb.member("MAIN");
    let dir = adm["cwd"].as_str().unwrap().to_string();
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).current_dir(&dir).args(args).output().unwrap();
    let o = fake(&["--bg", "--name", "[MAIN]-ADMIRAL", "--settings", &lead_labels(&sb), "ok"]);
    let stray = String::from_utf8_lossy(&o.stdout).trim().to_string();
    assert!(o.status.success() && stray.len() == 8, "{stray}");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let r = sb.run(&["relaunch", "MAIN", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let adm = sb.member("MAIN");
    let job = adm["session"]["short_id"].as_str().unwrap().to_string();
    assert_ne!(job, stray, "the stray is not the lead: {adm}");
    let intruder = adm["issues"].as_array().unwrap().iter().find(|i| i["code"] == "lead.intruder").expect("the stray is the intruder");
    assert!(intruder["message"].as_str().unwrap().contains(&stray) && !intruder["fix"].as_str().unwrap().contains(&job), "{intruder}");
    assert!(!adm["issues"].as_array().unwrap().iter().any(|i| i["code"] == "dup.live"), "{adm}");
}

/// A lead that `/branch`ed inside its own job, beside an older stray with its
/// name: the registry job stays the lead (re-review 1).
#[test]
fn a_lead_that_branched_beside_an_older_stray_keeps_its_job() {
    let sb = Sb::new(false);
    let adm = sb.member("MAIN");
    let (job, dir) = (adm["session"]["short_id"].as_str().unwrap().to_string(), adm["cwd"].as_str().unwrap().to_string());
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).current_dir(&dir).args(args).output().unwrap();
    let o = fake(&["--bg", "--name", "[MAIN]-ADMIRAL", "--settings", &lead_labels(&sb), "ok"]);
    let stray = String::from_utf8_lossy(&o.stdout).trim().to_string();
    assert!(o.status.success() && stray.len() == 8, "{stray}");
    assert!(fake(&["__inside", &job, "branch", "[MAIN]-ADMIRAL"]).status.success());
    let adm = sb.member("MAIN");
    assert_eq!(adm["session"]["short_id"], job.as_str(), "{adm}");
    let intruder = adm["issues"].as_array().unwrap().iter().find(|i| i["code"] == "lead.intruder").expect("the stray is the intruder");
    assert!(intruder["message"].as_str().unwrap().contains(&stray), "{intruder}");
}

/// The real job `/branch`ed after a relaunch (Claude Code renames it
/// "<prompt> (Branch)") beside an older stray with the lead's name: the
/// registry job stays the lead (the name-based handover is gone; only a job
/// whose first lineage entry says `via branch` hands over) (merge minor 1).
#[test]
fn a_renamed_registry_job_stays_the_lead_beside_an_older_stray() {
    let sb = Sb::new(false);
    let adm = sb.member("MAIN");
    let dir = adm["cwd"].as_str().unwrap().to_string();
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).current_dir(&dir).args(args).output().unwrap();
    let o = fake(&["--bg", "--name", "[MAIN]-ADMIRAL", "--settings", &lead_labels(&sb), "ok"]);
    let stray = String::from_utf8_lossy(&o.stdout).trim().to_string();
    assert!(o.status.success() && stray.len() == 8, "{stray}");
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(sb.run(&["relaunch", "MAIN", "--yes"]).code, 0);
    let job = sb.member("MAIN")["session"]["short_id"].as_str().unwrap().to_string();
    assert!(fake(&["__inside", &job, "branch", "x (Branch)"]).status.success());
    let adm = sb.member("MAIN");
    assert_eq!(adm["session"]["short_id"], job.as_str(), "the registry job stays, renamed or not: {adm}");
    assert_eq!(adm["session"]["name"], "x (Branch)");
    let codes: Vec<&str> = adm["issues"].as_array().unwrap().iter().filter_map(|i| i["code"].as_str()).collect();
    assert!(codes.contains(&"lead.intruder") && codes.contains(&"name.drift") && !codes.contains(&"dup.live"), "{adm}");
    let intruder = adm["issues"].as_array().unwrap().iter().find(|i| i["code"] == "lead.intruder").unwrap();
    assert!(intruder["message"].as_str().unwrap().contains(&stray), "{intruder}");
}

/// A crashed ship that also has a stale copy row: the start waits the crash
/// window for the SHIP's job (the plan's rm of the copy row names another
/// job) (merge minor 2).
#[test]
fn a_crashed_ship_with_a_stale_copy_row_is_waited_for() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    let (job, sid, dir) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string(), m["cwd"].as_str().unwrap().to_string());
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).current_dir(&dir).args(args).output().unwrap();
    // a stale copy row: a copy of the live ship (a resume with a flag), then stopped
    let o = fake(&["--bg", "--resume", &sid, "--name", "[MAIN.ARGOS]-CLEAN"]);
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    let copy = out.split("copy as ").nth(1).map(|s| s[..8].to_string()).expect(&out);
    assert!(fake(&["stop", &copy]).status.success());
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert!(m["extra_rows"].as_array().unwrap().iter().any(|r| r["short_id"] == copy.as_str() && r["pid"].is_null()), "{m}");
    assert!(fake(&["__crash", &job]).status.success());
    let t0 = std::time::Instant::now();
    let r = raw(&sb.vars, &["start", "MAIN/ARGOS/CLEAN", "--yes"], &[("NIMITZ_CRASH_WINDOW", "2")], None);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(t0.elapsed().as_secs_f64() >= 2.0, "no crash window wait with a stale copy row in the plan: {:.1}s\n{}", t0.elapsed().as_secs_f64(), r.out);
    assert!(r.out.contains(&format!("rm {copy}")), "the stale copy row is dropped: {}", r.out);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert!(live(&m) && m["session"]["short_id"] == job.as_str() && m["extra_rows"].as_array().unwrap().is_empty(), "{m}");
}

/// A stopped ship whose conversation a process with no job runs (a `claude
/// --resume <sid>` by hand in its folder): `held` only, with its one fix;
/// `stop` and `relaunch` refuse `session.interactive`, never `dup.live` with
/// a `nimitz stop <sid>` that cannot work (acceptance cli-18).
#[test]
fn a_held_member_gives_held_only_and_stop_or_relaunch_say_session_interactive() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/STOPPED");
    let (sid, dir) = (m["session"]["session_id"].as_str().unwrap().to_string(), m["cwd"].as_str().unwrap().to_string());
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).current_dir(&dir).args(args).output().unwrap();
    assert!(fake(&["--resume", &sid]).status.success());
    let m = sb.member("MAIN/ARGOS/STOPPED");
    let codes: Vec<&str> = m["issues"].as_array().unwrap().iter().filter_map(|i| i["code"].as_str()).collect();
    assert_eq!(codes, vec!["held"], "{m}");
    assert!(m["issues"][0]["fix"].as_str().unwrap().contains("end that process"), "{m}");
    for verb in ["stop", "relaunch"] {
        let r = sb.run(&[verb, "MAIN/ARGOS/STOPPED", "--plan"]);
        assert_ne!(r.code, 0, "{}", r.out);
        assert!(r.out.contains("session.interactive") && r.out.contains("end that process") && !r.out.contains("dup.live") && !r.out.contains("session.none"), "{verb}: {}", r.out);
    }
    assert!(fake(&["__end", &sid]).status.success());
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("--bg --resume"), "{}", r.out);
}

/// A lead whose job is live is stopped as ever, even while an older
/// conversation of its lineage is open interactively by hand (the held rule
/// is for a member whose own job is not live).
#[test]
fn stop_of_a_live_lead_beside_an_interactive_older_conversation_stops_its_job() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN");
    let (job, birth, dir) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string(), m["cwd"].as_str().unwrap().to_string());
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).current_dir(&dir).args(args).output().unwrap();
    assert!(fake(&["__inside", &job, "clear"]).status.success());
    let mid = sb.member("MAIN")["session"]["session_id"].as_str().unwrap().to_string();
    assert_ne!(mid, birth);
    assert!(fake(&["__inside", &job, "clear"]).status.success());
    // an older conversation of its lineage, opened interactively by hand in the folder
    let o = fake(&["--resume", &mid]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let held = sb.member("MAIN");
    assert!(held["extra_rows"].as_array().unwrap().iter().any(|r| r["session_id"] == mid.as_str()), "the interactive process is on the lead's lineage: {held}");
    let r = sb.run(&["stop", "MAIN", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains(&format!("stop {job}")) && !r.out.contains("session.interactive"), "{}", r.out);
    // relaunch keeps refusing while a process with no job holds a
    // conversation of the lineage, and names that process (not a 0.3.x lead)
    let r = sb.run(&["relaunch", "MAIN", "--plan"]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("session.interactive") && r.out.contains(&mid) && !r.out.contains("0.3.x") && r.out.contains("then nimitz relaunch MAIN"), "{}", r.out);
    // and status says only to end that process (the live job needs nothing more)
    let st = sb.member("MAIN");
    let held = st["issues"].as_array().unwrap().iter().find(|i| i["code"] == "held").expect("held");
    assert_eq!(held["fix"], "end that process in its host", "{st}");
    assert!(fake(&["__end", &mid]).status.success());
}

/// `/clear`, then a crash: the start never plans `rm <own job>` for the
/// member's own row (acceptance cli-31: before F1 the own row read as a stale
/// copy once it was parked after a /clear).
#[test]
fn clear_then_crash_then_start_never_drops_the_own_row() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    let job = m["session"]["short_id"].as_str().unwrap().to_string();
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).args(args).output().unwrap();
    assert!(fake(&["__inside", &job, "clear"]).status.success());
    let now = sb.member("MAIN/ARGOS/CLEAN")["session"]["session_id"].as_str().unwrap().to_string();
    assert!(fake(&["__crash", &job]).status.success());
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert_eq!((m["status"].as_str(), m["session"]["short_id"].as_str()), (Some("Parked"), Some(job.as_str())), "{m}");
    assert!(m["extra_rows"].as_array().unwrap().is_empty() && !m["issues"].as_array().unwrap().iter().any(|i| i["code"] == "copy.rows"), "{m}");
    let r = sb.run(&["start", "MAIN/ARGOS/CLEAN", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(!r.out.contains(&format!("rm {job}")) && !r.out.contains("stale"), "{}", r.out);
    let r = raw(&sb.vars, &["start", "MAIN/ARGOS/CLEAN", "--yes"], &[("NIMITZ_CRASH_WINDOW", "0")], None);
    assert_eq!(r.code, 0, "{}", r.out);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert!(live(&m) && m["session"]["short_id"] == job.as_str() && m["session"]["session_id"] == now.as_str(), "{m}");
}

/// An interactive process with a lead's name in the lead's folder (`claude
/// --name "<lead name>"`, or an interactive fork): `lead.intruder` with the
/// terminal fix, and never a `name.taken` on the same row (acceptance cli-L8, K1).
#[test]
fn an_interactive_process_with_a_leads_name_is_its_intruder_only() {
    let sb = Sb::new(false);
    let adm = sb.member("MAIN");
    let dir = adm["cwd"].as_str().unwrap().to_string();
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).current_dir(&dir).args(args).output().unwrap();
    assert!(fake(&["--name", "[MAIN]-ADMIRAL"]).status.success());
    let adm = sb.member("MAIN");
    let codes: Vec<&str> = adm["issues"].as_array().unwrap().iter().filter_map(|i| i["code"].as_str()).collect();
    assert!(codes.contains(&"lead.intruder") && !codes.contains(&"name.taken") && !codes.contains(&"dup.live"), "{adm}");
    let intruder = adm["issues"].as_array().unwrap().iter().find(|i| i["code"] == "lead.intruder").unwrap();
    assert!(intruder["fix"].as_str().unwrap().contains("end it in its terminal"), "{intruder}");
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--plan"]);
    assert_eq!(r.code, 0, "a ship under the lead still starts: {}", r.out);
}

/// `nimitz stop <copy>` plans stop + rm together, as `sweep` does: a copy
/// that was never prompted has no name, no labels and no transcript once
/// stopped, and no command would see its row again (acceptance cli-N1).
#[test]
fn stop_of_a_copy_removes_its_row_too() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    let (job, sid, dir) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string(), m["cwd"].as_str().unwrap().to_string());
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).current_dir(&dir).args(args).output().unwrap();
    let o = fake(&["--bg", "--resume", &sid, "--permission-mode", "default"]);
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    let copy = out.split("copy as ").nth(1).map(|s| s[..8].to_string()).expect(&out);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert!(m["extra_rows"].as_array().unwrap().iter().any(|r| r["short_id"] == copy.as_str() && !r["copy"].is_null()), "{m}");
    let r = sb.run(&["stop", &copy, "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains(&format!("stop {copy}")) && r.out.contains(&format!("rm {copy}")), "{}", r.out);
    let r = sb.run(&["stop", &copy, "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert!(live(&m) && m["session"]["short_id"] == job.as_str() && m["extra_rows"].as_array().unwrap().is_empty(), "{m}");
    assert!(!PathBuf::from(&sb.vars["NIMITZ_CLAUDE_HOME"]).join("jobs").join(&copy).exists(), "the copy's row is gone");
}

/// A closed pipe (`nimitz status --json | head`) ends nimitz quietly, never
/// with a panic (acceptance B observation).
#[test]
fn a_closed_pipe_never_panics() {
    use std::os::unix::process::ExitStatusExt;
    let sb = Sb::new(false);
    let mut c = Command::new(EXE);
    c.env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).args(["status", "--json"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    drop(child.stdout.take()); // the reader goes away at once
    let o = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!err.contains("panicked") && !err.contains("Broken pipe"), "{err}");
    assert!(o.status.code() == Some(0) || o.status.signal() == Some(13) || o.status.code() == Some(141), "{:?}", o.status);
}

/// Herdr's own hook names a pane by the process's inherited HERDR_PANE_ID,
/// so a background ship launched from its commodore's pane reports the
/// commodore's pane as its own. nimitz takes a pane's session only when the
/// pane's foreground backs it: the ship is never shown in the lead's pane.
#[test]
fn a_ship_is_never_shown_in_its_commodores_pane_on_herdrs_word_alone() {
    let sb = Sb::new(true);
    sb.pane_for("MAIN/ARGOS");
    let commodore = sb.member("MAIN/ARGOS")["session"]["session_id"].as_str().unwrap().to_string();
    let ship = sb.member("MAIN/ARGOS/CLEAN")["session"]["session_id"].as_str().unwrap().to_string();
    // the commodore runs in the pane; Herdr's agent list says the ship's session
    sb.herdr_world(|w| {
        w["panes"][1]["foreground"] = serde_json::json!(["claude", "--resume", commodore]);
        w["panes"][1]["agent_session"]["value"] = Value::from(ship.as_str());
    });
    let s = sb.status();
    let fleets = s["fleets"].as_array().unwrap();
    let all: Vec<&Value> = fleets.iter().flat_map(|f| f["squadrons"].as_array().unwrap().iter().flat_map(|q| q["ships"].as_array().unwrap().iter())).collect();
    let clean = all.iter().find(|m| m["id"] == "MAIN/ARGOS/CLEAN").unwrap();
    assert!(clean["pane"].is_null(), "the ship is not in the commodore's pane: {clean}");
    // a backed claim still stands: the pane's foreground names the ship's session
    sb.herdr_world(|w| w["panes"][1]["foreground"] = serde_json::json!(["claude", "--resume", ship]));
    let s = sb.status();
    let clean = s["fleets"].as_array().unwrap().iter().flat_map(|f| f["squadrons"].as_array().unwrap().iter().flat_map(|q| q["ships"].as_array().unwrap().iter())).find(|m| m["id"] == "MAIN/ARGOS/CLEAN").cloned().unwrap();
    assert_eq!(clean["pane"], "w1:p2", "{clean}");
    // the pid path, as on real Herdr: an interactive commodore (the pane's
    // foreground pid names its session file), the ship's claim on the pane:
    // the ship is not there, the commodore is (its own process says so)
    sb.herdr_world(|w| {
        w["panes"][1]["foreground"] = serde_json::json!(["claude"]);
        w["panes"][1]["pid"] = Value::from(424242);
        w["panes"][1]["agent_session"]["value"] = Value::from(ship.as_str());
    });
    std::fs::write(sb.base.join("claude_home/sessions/424242.json"), serde_json::json!({"pid": 424242, "sessionId": commodore, "kind": "interactive"}).to_string()).unwrap();
    let s = sb.status();
    let ships: Vec<&Value> = s["fleets"].as_array().unwrap().iter().flat_map(|f| f["squadrons"].as_array().unwrap().iter().flat_map(|q| q["ships"].as_array().unwrap().iter())).collect();
    assert!(ships.iter().find(|m| m["id"] == "MAIN/ARGOS/CLEAN").unwrap()["pane"].is_null());
    let cmd = s["fleets"].as_array().unwrap().iter().flat_map(|f| f["squadrons"].as_array().unwrap().iter()).find(|q| q["commodore"]["id"] == "MAIN/ARGOS").unwrap()["commodore"].clone();
    assert_eq!(cmd["pane"], "w1:p2", "the commodore, from its own process: {cmd}");
    std::fs::remove_file(sb.base.join("claude_home/sessions/424242.json")).unwrap();
}

/// Every claude nimitz starts begins with none of the caller's session state:
/// a commodore (labels, its Herdr pane, its messaging socket in the env) that
/// starts a ship hands none of it to the launch, so a daemon that launch
/// happens to start never labels unlabelled jobs as that commodore. Account
/// settings (`CLAUDE_CODE_USE_*`) pass.
#[test]
fn a_claude_nimitz_starts_inherits_no_labels_pane_or_messaging_state() {
    let sb = Sb::new(false);
    let log = sb.base.join("calls.log");
    let from = std::fs::read_to_string(&log).unwrap_or_default().lines().count();
    let r = raw(
        &sb.vars,
        &["start", "MAIN/ARGOS/STOPPED", "--yes"],
        &[
            ("NIMITZ_ID", "MAIN/ARGOS"),
            ("NIMITZ_ROLE", "commodore"),
            ("HERDR_ENV", "1"),
            ("HERDR_PANE_ID", "w4:p1"),
            ("HERDR_SESSION", "NIMITZ"),
            ("HERDR_SOCKET_PATH", "/live/herdr.sock"),
            ("CLAUDE_CODE_MESSAGING_SOCKET", "/run/user/1000/cc-socks/1.sock"),
            ("CLAUDE_CODE_ENTRYPOINT", "cli"),
            ("CLAUDE_PID", "1"),
            ("CLAUDE_CODE_SESSION_ATTENDED", "1"),
            ("CLAUDE_JOB_DIR", "/home/u/.claude/jobs/x"),
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
            ("CLAUDE_CODE_OAUTH_TOKEN", "tok"),
            ("CLAUDE_CODE_SUBAGENT_MODEL", "haiku"),
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.out);
    let calls: Vec<Value> = std::fs::read_to_string(&log).unwrap().lines().skip(from).map(|l| serde_json::from_str(l).unwrap()).collect();
    let launch = calls.iter().find(|c| c["args"].as_array().unwrap().iter().any(|a| a == "--resume" || a == "--bg")).expect("the launch reached claude");
    let env = launch["env"].as_object().unwrap();
    for k in ["NIMITZ_ID", "NIMITZ_ROLE", "HERDR_ENV", "HERDR_PANE_ID", "HERDR_SESSION", "HERDR_SOCKET_PATH", "CLAUDE_CODE_MESSAGING_SOCKET", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_PID", "CLAUDE_CODE_SESSION_ATTENDED"] {
        assert!(!env.contains_key(k), "{k} reached the claude child: {launch}");
    }
    // the user's own config passes (auth, provider, tuning): only session state goes
    for (k, v) in [("CLAUDE_CODE_USE_BEDROCK", "1"), ("CLAUDE_CODE_OAUTH_TOKEN", "tok"), ("CLAUDE_CODE_SUBAGENT_MODEL", "haiku")] {
        assert_eq!(env.get(k).and_then(Value::as_str), Some(v), "{k} must pass: {launch}");
    }
    // every claude call of the run (agents, stop, rm, resume) is scrubbed alike
    for c in &calls {
        assert!(!c["env"].as_object().unwrap().keys().any(|k| k.starts_with("NIMITZ_ID") || k == "HERDR_PANE_ID"), "{c}");
    }
}

/// The user's own nimitz outside Herdr (no HERDR_ENV, no NIMITZ_MUX: a plain
/// terminal) reads the Herdr server the user's commands recorded, named on
/// every call; a recorded server that is not reachable fails a restore with a
/// fix that names it (and the --jobs-only way), never a bare error.
#[test]
fn the_users_nimitz_outside_herdr_uses_the_recorded_server() {
    let sb = Sb::new(true);
    let active = PathBuf::from(&sb.vars["NIMITZ_STATE"]).join("active.json");
    std::fs::write(&active, r#"{"version": 1, "at": 1.0, "herdr": {"session": "rec"}, "members": {}}"#).unwrap();
    let mut outside = sb.vars.clone();
    outside.remove("NIMITZ_MUX");
    outside.remove("HERDR_ENV");
    let from = sb.calls().lines().count();
    let r = raw(&outside, &["status"], &[], None);
    assert_eq!(r.code, 0, "{}", r.out);
    let calls: Vec<String> = sb.calls().lines().skip(from).map(String::from).collect();
    assert!(!calls.is_empty() && calls.iter().all(|c| c.contains("\"--session\",\"rec\"")), "the recorded server, named: {calls:?}");
    // the recorded server gone: a restore says which, and what works
    let dead = sb.base.join("dead-herdr");
    std::fs::write(&dead, "#!/bin/sh\necho '{\"error\":{\"code\":\"no_server\",\"message\":\"herdr server not running\"}}' >&2\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&dead, std::fs::Permissions::from_mode(0o755)).unwrap();
    outside.insert("NIMITZ_HERDR_BIN".into(), dead.to_string_lossy().into_owned());
    let r = raw(&outside, &["restore", "MAIN", "--plan"], &[], None);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("mux.failed") && r.out.contains("recorded in active.json (session rec)") && r.out.contains("herdr --session rec") && r.out.contains("nimitz restore MAIN --jobs-only"), "{}", r.out);
}

/// A hook always exits 0, even when Claude Code has closed its pipe before
/// the hook writes its context (no SIGPIPE death, no panic).
#[test]
fn a_hook_with_a_closed_pipe_still_exits_zero() {
    let sb = Sb::new(false);
    let mut c = Command::new(EXE);
    c.env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).envs([("NIMITZ_ID", "MAIN/ARGOS"), ("NIMITZ_ROLE", "commodore")]).args(["hook", "SessionStart"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    drop(child.stdout.take()); // Claude Code went away before reading
    child.stdin.take().unwrap().write_all(br#"{"session_id":"s-pipe","source":"compact","hook_event_name":"SessionStart"}"#).unwrap();
    let o = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!err.contains("panicked"), "{err}");
    assert_eq!(o.status.code(), Some(0), "{:?}", o.status);
}

/// `nimitz rm <lead>` with no stale rows says what removes its own row: a
/// relaunch replaces it, the retire command removes it (acceptance B, item 7).
#[test]
fn rm_of_a_lead_points_at_the_retire_command() {
    let sb = Sb::new(false);
    let r = sb.run(&["rm", "MAIN", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("its own row") && r.out.contains("nimitz fleet rm MAIN retires the member") && r.out.contains("nimitz relaunch MAIN"), "{}", r.out);
    let r = sb.run(&["rm", "MAIN/ARGOS", "--plan"]);
    assert!(r.out.contains("nimitz squadron rm MAIN/ARGOS"), "{}", r.out);
}

/// No row and no transcript is `history.lost` with the `--fresh` fix (one
/// word, one flag, as with a row and no transcript); the `--fresh` plan does
/// not repeat the error (acceptance env E9).
#[test]
fn no_row_and_no_transcript_is_history_lost_with_the_fresh_fix() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/STOPPED");
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).args(args).output().unwrap();
    assert!(fake(&["rm", &job]).status.success());
    for p in walkdir(&sb.base.join("claude_home/projects")).into_iter().filter(|p| p.file_name().map(|n| n.to_string_lossy() == format!("{sid}.jsonl")).unwrap_or(false)) {
        std::fs::remove_file(p).unwrap();
    }
    let m = sb.member("MAIN/ARGOS/STOPPED");
    let codes: Vec<&str> = m["issues"].as_array().unwrap().iter().filter_map(|i| i["code"].as_str()).collect();
    assert!(codes.contains(&"history.lost") && !codes.contains(&"lineage.broken"), "{m}");
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--plan"]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert_eq!(r.out.matches("(history.lost)").count(), 1, "{}", r.out);
    assert!(r.out.contains("nimitz start MAIN/ARGOS/STOPPED --fresh") && !r.out.contains("lineage.broken") && !r.out.contains("--force"), "{}", r.out);
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--fresh", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("cold launch") && !r.out.contains("history.lost") && !r.out.contains("[ERROR]"), "{}", r.out);
}

/// A killed job whose state stayed `blocked` (or `done`) is in the crash
/// window too while its process went away just now: a start waits for the
/// daemon's restart before any resume (acceptance env F3).
#[test]
fn a_job_killed_in_state_blocked_is_in_the_crash_window() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).args(args).output().unwrap();
    assert!(fake(&["__crash", &job, "blocked"]).status.success());
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert_eq!((m["status"].as_str(), m["session"]["state"].as_str()), (Some("Parked"), Some("blocked")), "{m}");
    let t0 = std::time::Instant::now();
    let r = raw(&sb.vars, &["start", "MAIN/ARGOS/CLEAN", "--yes"], &[("NIMITZ_CRASH_WINDOW", "2")], None);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(t0.elapsed().as_secs_f64() >= 2.0, "no crash window wait for a blocked row: {:.1}s", t0.elapsed().as_secs_f64());
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert!(live(&m) && m["session"]["session_id"] == sid.as_str() && m["session"]["short_id"] == job.as_str(), "{m}");
    // a job stopped on purpose (a SessionEnd) just now is not in the window
    assert_eq!(sb.run(&["stop", "MAIN/ARGOS/CLEAN", "--yes"]).code, 0);
    let t0 = std::time::Instant::now();
    let r = raw(&sb.vars, &["start", "MAIN/ARGOS/CLEAN", "--yes"], &[("NIMITZ_CRASH_WINDOW", "5")], None);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(t0.elapsed().as_secs_f64() < 4.0, "a stopped job waited the crash window: {:.1}s", t0.elapsed().as_secs_f64());
}

/// A stub transcript (what `claude stop` writes after the real file was moved
/// away: no message line) is no transcript: `history.lost` before any launch
/// (acceptance env F5).
#[test]
fn a_stub_transcript_is_history_lost_before_the_launch() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/STOPPED");
    let sid = m["session"]["session_id"].as_str().unwrap().to_string();
    let log = walkdir(&sb.base.join("claude_home/projects")).into_iter().find(|p| p.file_name().map(|n| n.to_string_lossy() == format!("{sid}.jsonl")).unwrap_or(false)).expect("the transcript");
    std::fs::write(&log, format!("{{\"type\":\"summary\",\"summary\":\"stub\",\"leafUuid\":\"x\"}}\n{{\"type\":\"system\",\"sessionId\":\"{sid}\",\"content\":\"stopped\"}}\n")).unwrap();
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--plan"]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("history.lost") && r.out.contains("--fresh") && !r.out.contains("--bg --resume"), "{}", r.out);
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--yes"]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(!live(&sb.member("MAIN/ARGOS/STOPPED")), "launched anyway");
}

/// `nimitz rm <short>` of a member's own current row names the commands that
/// do remove it: a relaunch replaces it, a retire removes it (acceptance env F7).
#[test]
fn rm_of_a_members_own_row_names_relaunch_and_retire() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/STOPPED");
    let job = m["session"]["short_id"].as_str().unwrap().to_string();
    let r = sb.run(&["rm", &job, "--plan"]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("lineage.current") && r.out.contains("MAIN/ARGOS/STOPPED's current session"), "{}", r.out);
    assert!(r.out.contains("nimitz relaunch MAIN/ARGOS/STOPPED") && r.out.contains("nimitz ship rm MAIN/ARGOS/STOPPED retires the member"), "{}", r.out);
    let adm = sb.member("MAIN");
    let r = sb.run(&["rm", adm["session"]["short_id"].as_str().unwrap(), "--plan"]);
    assert!(r.out.contains("session.live") || r.out.contains("nimitz fleet rm MAIN"), "{}", r.out);
}

/// `/branch`, stop, then `relaunch` (the fix of name.drift and labels.missing):
/// the relabel goes onto the branch too, never the pre-branch conversation
/// (review of the acceptance fixes, item 2).
#[test]
fn branch_then_stop_then_relaunch_resumes_the_branch() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    let (job, birth) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).args(args).output().unwrap();
    assert!(fake(&["__inside", &job, "branch", "[MAIN.ARGOS]-CLEAN"]).status.success());
    let branch = sb.member("MAIN/ARGOS/CLEAN")["session"]["session_id"].as_str().unwrap().to_string();
    assert_eq!(sb.run(&["stop", "MAIN/ARGOS/CLEAN", "--yes"]).code, 0);
    let r = sb.run(&["relaunch", "MAIN/ARGOS/CLEAN", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains(&format!("rm {job}")) && r.out.contains(&format!("--resume {branch} --name")) && !r.out.contains(&format!("--resume {birth}")), "{}", r.out);
    let r = sb.run(&["relaunch", "MAIN/ARGOS/CLEAN", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert!(live(&m) && m["session"]["session_id"] == branch.as_str(), "{m}");
}

/// The registry names a conversation that is not later in the lineage than the
/// one the job would wake (a lost hook line, a hand edit): the job's own
/// conversation wins, with a `lineage.stale` warning and the relaunch fix
/// (review item 5).
#[test]
fn a_stale_registry_session_wakes_the_row_and_warns() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    let (job, birth) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).args(args).output().unwrap();
    assert!(fake(&["__inside", &job, "clear"]).status.success());
    let now = sb.member("MAIN/ARGOS/CLEAN")["session"]["session_id"].as_str().unwrap().to_string();
    assert_eq!(sb.run(&["stop", "MAIN/ARGOS/CLEAN", "--yes"]).code, 0);
    // the registry points back at the birth conversation (earlier in the lineage than the row's)
    let reg_path = PathBuf::from(&sb.vars["NIMITZ_STATE"]).join("registry.json");
    let mut reg: Value = serde_json::from_slice(&std::fs::read(&reg_path).unwrap()).unwrap();
    reg["identities"]["MAIN/ARGOS/CLEAN"]["session"] = Value::from(birth.as_str());
    std::fs::write(&reg_path, reg.to_string()).unwrap();
    let m = sb.member("MAIN/ARGOS/CLEAN");
    let stale = m["issues"].as_array().unwrap().iter().find(|i| i["code"] == "lineage.stale").expect("lineage.stale");
    assert_eq!(stale["severity"], "Warn");
    assert_eq!(stale["fix"], "nimitz relaunch MAIN/ARGOS/CLEAN");
    let r = sb.run(&["start", "MAIN/ARGOS/CLEAN", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains(&format!("--bg --resume {birth}")) && !r.out.contains("--name") && r.out.contains("lineage.stale"), "a native wake, warned: {}", r.out);
    let r = sb.run(&["start", "MAIN/ARGOS/CLEAN", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert_eq!(m["session"]["session_id"], now.as_str(), "the job's conversation: {m}");
}

/// A lead with no registry job and a live /fork beside its own live job: the
/// earlier job is the lead's, the fork its intruder; never the other way
/// round (review item 7).
#[test]
fn a_leads_fork_with_no_registry_job_is_still_the_intruder() {
    let sb = Sb::new(false);
    let adm = sb.member("MAIN");
    let job = adm["session"]["short_id"].as_str().unwrap().to_string();
    let reg_path = PathBuf::from(&sb.vars["NIMITZ_STATE"]).join("registry.json");
    let mut reg: Value = serde_json::from_slice(&std::fs::read(&reg_path).unwrap()).unwrap();
    reg["identities"]["MAIN"].as_object_mut().unwrap().remove("job");
    for l in reg["identities"]["MAIN"]["lineage"].as_array_mut().unwrap() {
        l.as_object_mut().unwrap().remove("job");
    }
    std::fs::write(&reg_path, reg.to_string()).unwrap();
    let fake = |args: &[&str]| Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).args(args).output().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100)); // the fork starts later than the lead
    let o = fake(&["__inside", &job, "fork"]);
    let fork = String::from_utf8_lossy(&o.stdout).trim().rsplit(' ').next().unwrap().to_string();
    let adm = sb.member("MAIN");
    assert_eq!(adm["session"]["short_id"], job.as_str(), "the earlier job is the lead's: {adm}");
    let intruder = adm["issues"].as_array().unwrap().iter().find(|i| i["code"] == "lead.intruder").expect("lead.intruder");
    assert!(intruder["message"].as_str().unwrap().contains(&fork) && !intruder["message"].as_str().unwrap().contains(&job), "{intruder}");
    assert!(!adm["issues"].as_array().unwrap().iter().any(|i| i["code"] == "dup.live"), "{adm}");
}

/// A job killed in its first turn has no transcript on disk yet. A start right
/// after is the daemon's crash window (apply waits for its restart), never
/// `history.lost` with a `--fresh` fix that would make a duplicate (e2e case 6).
#[test]
fn a_crashed_job_is_resumed_not_judged_history_lost() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    let (short, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    let o = Command::new(&sb.vars["NIMITZ_NATIVE_CLAUDE_BIN"]).env_clear().env("PATH", "/usr/bin:/bin").envs(&sb.vars).args(["__crash", &short]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let log = walkdir(&sb.base.join("claude_home/projects")).into_iter().find(|p| p.file_name().map(|n| n.to_string_lossy() == format!("{sid}.jsonl")).unwrap_or(false)).expect("the transcript");
    std::fs::remove_file(&log).unwrap();
    let r = sb.run(&["start", "MAIN/ARGOS/CLEAN", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains(&format!("--resume {sid}")) && !r.out.contains("history.lost") && !r.out.contains("--fresh"), "{}", r.out);
    // the window closes with no restart and the transcript is still gone: no
    // raw resume (it would run with no history); history.lost with the --fresh fix
    let r = raw(&sb.vars, &["start", "MAIN/ARGOS/CLEAN", "--yes"], &[("NIMITZ_CRASH_WINDOW", "0")], None);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("history.lost") && r.out.contains("nimitz start MAIN/ARGOS/CLEAN --fresh") && r.out.contains("not run"), "{}", r.out);
    assert!(!live(&sb.member("MAIN/ARGOS/CLEAN")), "a resume ran anyway");
    let r = raw(&sb.vars, &["start", "MAIN/ARGOS/CLEAN", "--fresh", "--yes"], &[("NIMITZ_CRASH_WINDOW", "0")], None);
    assert_eq!(r.code, 0, "{}", r.out);
    let m = sb.member("MAIN/ARGOS/CLEAN");
    assert!(live(&m) && m["session"]["session_id"] != sid.as_str(), "{m}");
}

/// A record that holds this home's own history and a binding another home's
/// hook made on top of it keeps the history: only the foreign job, session
/// and lineage entries are stripped on load.
#[test]
fn a_foreign_binding_is_stripped_and_this_homes_history_kept() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/STOPPED");
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    // another home's job: a done row (shaped like fd52a81e's) whose saved labels name /elsewhere/nimitz
    let other = "f0f0f0f0";
    let other_sid = "f0f0f0f0-5b0c-4d1e-9f2a-0000000000f0";
    let jobs = PathBuf::from(&sb.vars["NIMITZ_CLAUDE_HOME"]).join("jobs");
    let mut d: Value = serde_json::from_slice(&std::fs::read(jobs.join("fd52a81e").join("state.json")).unwrap()).unwrap();
    d["sessionId"] = Value::from(other_sid);
    d["resumeSessionId"] = Value::Null;
    let flags = d["respawnFlags"].as_array_mut().unwrap();
    let i = flags.iter().position(|x| x == "--settings").unwrap();
    let mut s: Value = serde_json::from_str(flags[i + 1].as_str().unwrap()).unwrap();
    s["env"]["NIMITZ_HOME"] = Value::from("/elsewhere/nimitz");
    flags[i + 1] = Value::from(s.to_string());
    std::fs::create_dir_all(jobs.join(other)).unwrap();
    std::fs::write(jobs.join(other).join("state.json"), d.to_string()).unwrap();
    // its hook bound the ship to it, on top of the ship's own history (the
    // entry carries that home; an older nimitz's would not, and the row says)
    let reg_path = PathBuf::from(&sb.vars["NIMITZ_STATE"]).join("registry.json");
    let mut reg: Value = serde_json::from_slice(&std::fs::read(&reg_path).unwrap()).unwrap();
    let rec = &mut reg["identities"]["MAIN/ARGOS/STOPPED"];
    rec["job"] = Value::from(other);
    rec["session"] = Value::from(other_sid);
    rec["lineage"].as_array_mut().unwrap().push(serde_json::json!({"session_id": other_sid, "job": other, "via": "launch", "at": 1791400000.0, "home": "/elsewhere/nimitz"}));
    std::fs::write(&reg_path, reg.to_string()).unwrap();
    let m = sb.member("MAIN/ARGOS/STOPPED");
    assert_eq!((m["session"]["short_id"].as_str(), m["session"]["session_id"].as_str()), (Some(job.as_str()), Some(sid.as_str())), "{m}");
    assert!(matches!(m["status"].as_str(), Some("Stopped" | "Parked")), "{m}");
    assert!(!m.to_string().contains(other) && !m.to_string().contains(other_sid), "{m}");
    // a start wakes this home's conversation
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--plan"]);
    assert!(r.out.contains(&format!("--resume {sid}")), "{}", r.out);
    // the other home's row gone: its binding stays out (the entry's stamp says whose it is)
    std::fs::remove_dir_all(jobs.join(other)).unwrap();
    let m = sb.member("MAIN/ARGOS/STOPPED");
    assert_eq!((m["session"]["short_id"].as_str(), m["session"]["session_id"].as_str()), (Some(job.as_str()), Some(sid.as_str())), "{m}");
    assert!(!m.to_string().contains(other_sid), "{m}");
}

/// A job labelled for another `NIMITZ_HOME` never appears (plan-v4 section
/// 6), not even through the registry: its hook runs with that home in its
/// own env (the label is the env), so with one `NIMITZ_STATE` shared by two
/// homes it binds the identity here. The reader drops such a record (e2e
/// case 11: the member stayed "Stopped" on the other home's transcript and a
/// start would have resumed the other home's conversation).
#[test]
fn a_job_labelled_for_another_home_never_binds_the_member() {
    let sb = Sb::new(false);
    let m = sb.member("MAIN/ARGOS/STOPPED");
    let (job, sid) = (m["session"]["short_id"].as_str().unwrap().to_string(), m["session"]["session_id"].as_str().unwrap().to_string());
    assert!(matches!(m["status"].as_str(), Some("Stopped" | "Parked")), "{m}");
    // the job's saved labels and its hook lines name another home (they always agree: one --settings)
    let relabel = |v: &mut Value| v["env"]["NIMITZ_HOME"] = Value::from("/elsewhere/nimitz");
    let p = PathBuf::from(&sb.vars["NIMITZ_CLAUDE_HOME"]).join("jobs").join(&job).join("state.json");
    let mut d: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    let flags = d["respawnFlags"].as_array_mut().unwrap();
    let i = flags.iter().position(|x| x == "--settings").unwrap();
    let mut s: Value = serde_json::from_str(flags[i + 1].as_str().unwrap()).unwrap();
    relabel(&mut s);
    flags[i + 1] = Value::from(s.to_string());
    std::fs::write(&p, d.to_string()).unwrap();
    let ev = PathBuf::from(&sb.vars["NIMITZ_STATE"]).join("events.jsonl");
    let lines: Vec<String> = std::fs::read_to_string(&ev)
        .unwrap()
        .lines()
        .map(|l| {
            let mut e: Value = serde_json::from_str(l).unwrap();
            if e["session_id"] == sid.as_str() {
                e["labels"]["NIMITZ_HOME"] = Value::from("/elsewhere/nimitz");
            }
            e.to_string()
        })
        .collect();
    std::fs::write(&ev, lines.join("\n") + "\n").unwrap();
    // (and its lineage entries, stamped by that hook's write)
    let reg_path = PathBuf::from(&sb.vars["NIMITZ_STATE"]).join("registry.json");
    let mut reg: Value = serde_json::from_slice(&std::fs::read(&reg_path).unwrap()).unwrap();
    for l in reg["identities"]["MAIN/ARGOS/STOPPED"]["lineage"].as_array_mut().unwrap() {
        if l["job"] == job.as_str() {
            l["home"] = Value::from("/elsewhere/nimitz");
        }
    }
    std::fs::write(&reg_path, reg.to_string()).unwrap();
    let m = sb.member("MAIN/ARGOS/STOPPED");
    assert_eq!(m["status"], "NotStarted", "{m}");
    assert!(m["session"].is_null() && m["extra_rows"].as_array().unwrap().is_empty(), "{m}");
    let r = sb.run(&["status", "--json"]);
    assert!(!r.out.contains(&job) && !r.out.contains(&sid), "the other home's job shows: {}", r.out);
    assert!(m["issues"].as_array().unwrap().is_empty(), "no issue, no fix: {m}");
    // a start is a cold launch, never a resume of the other home's conversation
    let r = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("cold launch") && !r.out.contains("--resume"), "{}", r.out);
    // a read never writes: the record is still in the file until the next update
    let raw = std::fs::read_to_string(PathBuf::from(&sb.vars["NIMITZ_STATE"]).join("registry.json")).unwrap();
    assert!(raw.contains(&job));
}

/// The commodore's attach pane with no Herdr claim (the live shape: an
/// attach client runs no Herdr hook) shows the commodore, but the agent view
/// (`/exit` inside the attach, R-24) looks the same, and text typed there
/// would dispatch a new session: a needs-you delivery and `nimitz signal`
/// type nothing, and say why.
#[test]
fn an_unclaimed_attach_pane_is_never_typed_into() {
    let sb = Sb::new(true);
    let job = sb.member("MAIN/ARGOS")["session"]["short_id"].as_str().unwrap().to_string();
    let bin = sb.base.join("bin/claude").to_string_lossy().into_owned();
    sb.herdr_world(|w| {
        let p = &mut w["panes"][1];
        p.as_object_mut().unwrap().remove("agent_session");
        p["agent"] = Value::from("claude");
        p["agent_status"] = Value::from("idle");
        p["foreground"] = serde_json::json!([bin, "attach", job]);
    });
    raw(&sb.vars, &["hook", "PreToolUse"], &CLEAN, Some(ASK));
    let n = sb.notice("MAIN/ARGOS/CLEAN").unwrap();
    assert_eq!(n["delivery"], "failed", "{n}");
    assert!(n["delivery_note"].as_str().unwrap().contains("cannot tell it from the agent view"), "{n}");
    let r = sb.run(&["signal", "MAIN/ARGOS", "hello", "--yes"]);
    assert_ne!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("(tab.unverified)") && r.out.contains("nimitz open MAIN/ARGOS"), "{}", r.out);
    assert!(sb.prompts("w1:p2").is_empty(), "nothing typed");
    // the pane itself is still the commodore's: status shows it there
    assert_eq!(sb.member("MAIN/ARGOS")["pane"], Value::from("w1:p2"));
}

/// A Unix socket that stands for a session's inbox: every line it gets.
fn inbox_listener(path: &Path) -> std::sync::mpsc::Receiver<String> {
    let l = std::os::unix::net::UnixListener::bind(path).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for c in l.incoming().flatten() {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut { c }, &mut s);
            let _ = tx.send(s);
        }
    });
    rx
}

impl Sb {
    /// `id`'s live session listens on a fresh inbox socket (recorded as its
    /// SessionStart hook records it), and runs Claude Code `version`. The
    /// listener is this test process, so the session's pid becomes ours (the
    /// inbox posts only to a socket the session's own process listens on).
    fn inbox_for(&self, id: &str, version: &str) -> std::sync::mpsc::Receiver<String> {
        let m = self.member(id);
        let (sid, short) = (m["session"]["session_id"].as_str().unwrap().to_string(), m["session"]["short_id"].as_str().unwrap().to_string());
        let me = std::process::id() as i64;
        for e in std::fs::read_dir(self.base.join("claude_home/sessions")).unwrap().flatten() {
            let mut v: Value = serde_json::from_slice(&std::fs::read(e.path()).unwrap()).unwrap_or_default();
            if v["sessionId"] == sid.as_str() {
                v["pid"] = Value::from(me);
                std::fs::write(e.path(), v.to_string()).unwrap();
            }
        }
        let sock = self.base.join(format!("inbox-{short}.sock"));
        let rx = inbox_listener(&sock);
        let dir = PathBuf::from(&self.vars["NIMITZ_STATE"]).join("inbox");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{sid}.json")), serde_json::json!({"socket": sock, "pid": me, "at": 1.0}).to_string()).unwrap();
        // the sandbox's session files are not named by pid: the job row's cliVersion
        let file = self.base.join("claude_home/jobs").join(short).join("state.json");
        let mut v: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        v["cliVersion"] = Value::from(version);
        std::fs::write(&file, v.to_string()).unwrap();
        assert_eq!(self.member(id)["session"]["pid"], Value::from(me), "the session runs as this process now");
        rx
    }
}

/// A needs-you notice goes into the lead's inbox socket as a cross-session
/// message from nimitz, nothing typed. `nimitz signal` types while the lead's
/// pane can take it (the user's own words keep their authority) and posts to
/// the inbox only when it cannot, saying it is a request, not an approval.
#[test]
fn a_notice_and_a_signal_go_to_the_leads_inbox_not_its_pane() {
    let sb = Sb::new(true);
    sb.pane_for("MAIN/ARGOS");
    let rx = sb.inbox_for("MAIN/ARGOS", "2.1.293");
    raw(&sb.vars, &["hook", "PreToolUse"], &CLEAN, Some(ASK));
    let n = sb.notice("MAIN/ARGOS/CLEAN").unwrap();
    assert_eq!(n["delivery"], "delivered", "{n}");
    assert!(n["delivery_note"].as_str().unwrap().contains("inbox"), "{n}");
    let line: Value = serde_json::from_str(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().trim()).unwrap();
    let c = line["message"]["content"].as_str().unwrap();
    assert!(c.starts_with("<cross-session-message from=\"uds:/nonexistent/nimitz-no-reply.sock\" from-name=\"nimitz\">") && c.contains("[nimitz needs-you]") && c.contains("Ship the fix to main?"), "{c}");
    // a signal is typed while the lead's pane can take it (the user's own
    // words); its inbox only when the pane cannot (known by its attach alone)
    let r = sb.run(&["signal", "MAIN/ARGOS", "typed one", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(sb.prompts("w1:p2").iter().any(|p| p.contains("typed one")), "{}", r.out);
    let job = sb.member("MAIN/ARGOS")["session"]["short_id"].as_str().unwrap().to_string();
    let bin = sb.base.join("bin/claude").to_string_lossy().into_owned();
    sb.herdr_world(|w| {
        let p = &mut w["panes"][1];
        p.as_object_mut().unwrap().remove("agent_session");
        p["foreground"] = serde_json::json!([bin, "attach", job]);
    });
    let typed = sb.prompts("w1:p2").len();
    let r = sb.run(&["signal", "MAIN/ARGOS", "hello there", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("posted to MAIN/ARGOS's inbox"), "{}", r.out);
    let line: Value = serde_json::from_str(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().trim()).unwrap();
    let c = line["message"]["content"].as_str().unwrap();
    assert!(c.contains("[nimitz signal]") && c.contains("not the user's approval") && c.contains("hello there"), "{c}");
    assert_eq!(sb.prompts("w1:p2").len(), typed, "nothing more typed");
}

/// A lead on a Claude Code older than the verified line format gets the
/// typing path, and the note says why.
#[test]
fn an_older_claude_code_gets_the_typing_path() {
    let sb = Sb::new(true);
    sb.pane_for("MAIN/ARGOS");
    let rx = sb.inbox_for("MAIN/ARGOS", "2.1.292");
    raw(&sb.vars, &["hook", "PreToolUse"], &CLEAN, Some(ASK));
    let n = sb.notice("MAIN/ARGOS/CLEAN").unwrap();
    assert_eq!(n["delivery"], "delivered", "{n}");
    assert!(n["delivery_note"].as_str().unwrap().contains("typed into") && n["delivery_note"].as_str().unwrap().contains("older than 2.1.293"), "{n}");
    assert!(rx.recv_timeout(std::time::Duration::from_millis(300)).is_err(), "nothing sent to the inbox");
    assert!(!sb.prompts("w1:p2").is_empty());
}

/// With no multiplexer that types, `nimitz signal` posts to the lead's inbox,
/// as a request (the output says so).
#[test]
fn with_no_multiplexer_a_signal_goes_to_the_inbox_as_a_request() {
    let sb = Sb::new(true);
    let rx = sb.inbox_for("MAIN/ARGOS", "2.1.293");
    let r = raw(&sb.vars, &["signal", "MAIN/ARGOS", "no mux here", "--yes"], &[("NIMITZ_MUX", "none")], None);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("posted to MAIN/ARGOS's inbox") && r.out.contains("not as your own input"), "{}", r.out);
    let line: Value = serde_json::from_str(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().trim()).unwrap();
    assert!(line["message"]["content"].as_str().unwrap().contains("no mux here"));
}
