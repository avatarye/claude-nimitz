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
        .filter_map(|l| ["not-started", "stopped", "idle", "busy", "needs-you", "unknown"].iter().filter_map(|w| l.find(&format!(" {w} "))).min())
        .collect();
    assert!(cols.len() > 5 && cols.iter().all(|c| *c == cols[0]), "status column moves: {cols:?}\n{out}");
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
    assert_eq!(r.out.matches("[TOP]-ADMIRAL").count(), 1, "the launch line is printed once (in the plan):\n{}", r.out);
    // the panes never bound (the fake runs nothing): a second start focuses them, --force opens anew
    let r = sb.run(&["start", "TOP", "--yes"]);
    assert!(r.out.contains("already opening in pane"), "{}", r.out);
    let n = |c: &str| c.lines().filter(|l| l.contains("\"pane\",\"run\"") && l.contains("[TOP]-ADMIRAL")).count();
    assert_eq!(n(&sb.calls()), 1, "{}", sb.calls());
    sb.run(&["start", "TOP", "--yes", "--force"]);
    assert_eq!(n(&sb.calls()), 2, "{}", sb.calls());
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
    assert!(r.out.contains("its tab MAIN.ARGOS.CLEAN is open already"), "{}", r.out);
    let r = sb.run(&["open", "MAIN/ARGOS/CLEAN"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("is open already; focused pane"), "{}", r.out);
    assert_eq!(tabs(&sb), 1, "no second tab");
    assert!(sb.calls().contains("\"agent\",\"focus\""), "{}", sb.calls());
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
    let r = sb.run(&["layout", "herdr", "BETA", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(sb.calls().contains("[BETA]-ADMIRAL"), "{}", sb.calls());
    let r = sb.run(&["layout", "herdr", "MAIN", "--plan"]);
    assert!(r.out.contains("running already"), "live members are left alone:\n{}", r.out);
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
