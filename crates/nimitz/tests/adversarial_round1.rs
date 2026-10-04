//! Adversarial round 1 (docs/test/report-round1.md): one regression test per
//! BUG that the sandbox can show. Each asserts the RIGHT behaviour, so it fails
//! today; `#[ignore = "BUG-nn"]` keeps the suite green until the fix lands
//! (`cargo test --test adversarial_round1 -- --ignored` runs them).
//! Every run clears the process env, so nothing reaches the real `~/.claude`,
//! `~/.claude-nimitz` or the user's Herdr (contracts.md section 11).
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

fn raw(vars: &BTreeMap<String, String>, args: &[&str], extra: &[(&str, &str)], stdin: Option<&str>, cwd: Option<&Path>) -> Run {
    let mut c = Command::new(EXE);
    c.env_clear().env("PATH", "/usr/bin:/bin").env("NIMITZ_BIND_TIMEOUT", "0").envs(vars).args(args);
    for (k, v) in extra {
        c.env(k, v);
    }
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    c.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().expect("spawn nimitz");
    if let Some(s) = stdin {
        let _ = child.stdin.take().unwrap().write_all(s.as_bytes());
    }
    let o = child.wait_with_output().unwrap();
    Run { code: o.status.code().unwrap_or(-1), out: format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)) }
}

impl Sb {
    fn new(herdr: bool) -> Sb {
        let td = tempfile::tempdir().unwrap();
        let base = td.path().join("world");
        let mut args = vec!["sandbox", "create", base.to_str().unwrap()];
        if herdr {
            args.push("--herdr");
        }
        let r = raw(&BTreeMap::new(), &args, &[], None, None);
        assert_eq!(r.code, 0, "{}", r.out);
        let e = raw(&BTreeMap::new(), &["sandbox", "env", base.to_str().unwrap(), "--json"], &[], None, None);
        let vars: BTreeMap<String, String> = serde_json::from_str(e.out.trim()).expect("sandbox env --json");
        let base = base.canonicalize().unwrap();
        Sb { _td: td, base, vars }
    }
    fn run(&self, args: &[&str]) -> Run {
        raw(&self.vars, args, &[], None, None)
    }
    fn run_env(&self, args: &[&str], extra: &[(&str, &str)]) -> Run {
        raw(&self.vars, args, extra, None, None)
    }
    /// The sandbox's fake (native) `claude`, run by hand in `cwd`, like a careless user.
    fn claude(&self, cwd: &Path, args: &[&str]) -> String {
        let o = Command::new(self.base.join("bin/claude"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .envs(&self.vars)
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap();
        format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
    }
    fn dir(&self, rel: &str) -> PathBuf {
        self.base.join(rel)
    }
    fn squadron_toml(&self) -> PathBuf {
        self.dir("nimitz/main/squadrons/argos/squadron.toml")
    }
    fn edit(&self, p: &Path, f: impl FnOnce(String) -> String) {
        let s = std::fs::read_to_string(p).unwrap();
        std::fs::write(p, f(s)).unwrap();
    }
    fn trust(&self, dir: &Path) {
        for f in [self.dir("claude_home/.claude.json"), self.dir("claude_home.json")] {
            let mut v: Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
            v["projects"][dir.to_string_lossy().as_ref()] = serde_json::json!({"hasTrustDialogAccepted": true});
            std::fs::write(&f, serde_json::to_vec(&v).unwrap()).unwrap();
        }
    }
    fn status(&self) -> Value {
        let r = self.run(&["status", "--json"]);
        assert_eq!(r.code, 0, "{}", r.out);
        let start = r.out.find('{').unwrap();
        serde_json::Deserializer::from_str(&r.out[start..]).into_iter::<Value>().next().unwrap().unwrap()
    }
    fn member(&self, id: &str) -> Option<Value> {
        let s = self.status();
        for f in s["fleets"].as_array()? {
            if f["admiral"]["id"] == id {
                return Some(f["admiral"].clone());
            }
            for sq in f["squadrons"].as_array()? {
                if sq["commodore"]["id"] == id {
                    return Some(sq["commodore"].clone());
                }
                for m in sq["ships"].as_array()?.iter().chain(sq["tenders"].as_array()?) {
                    if m["id"] == id {
                        return Some(m.clone());
                    }
                }
            }
        }
        None
    }
    fn sid(&self, id: &str) -> String {
        self.member(id).and_then(|m| m["session"]["session_id"].as_str().map(str::to_string)).expect("member has a session")
    }
    /// Live rows (session files with pid) whose folder is `cwd`.
    fn live_in(&self, cwd: &Path) -> usize {
        let mut n = 0;
        for e in std::fs::read_dir(self.dir("claude_home/sessions")).unwrap().flatten() {
            let v: Value = match std::fs::read(e.path()).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
                Some(v) => v,
                None => continue,
            };
            if v["cwd"].as_str() == Some(cwd.to_str().unwrap()) {
                n += 1;
            }
        }
        n
    }
    fn stowaways(&self) -> Vec<Value> {
        self.status()["stowaways"].as_array().cloned().unwrap_or_default()
    }
}

fn hook_decision(out: &str) -> String {
    let t = out.trim();
    if t.is_empty() {
        return "allow".into();
    }
    let v: Value = serde_json::from_str(t).unwrap_or(Value::Null);
    v["hookSpecificOutput"]["permissionDecision"].as_str().unwrap_or("?").to_string()
}

// ------------------------------------------------------------------ A. starting and order

#[test]
#[ignore = "BUG-01"]
fn bug01_nimitz_mux_auto_still_detects_herdr() {
    // contracts.md section 2: NIMITZ_MUX defaults to `auto`; saying so must not turn the mux off.
    let sb = Sb::new(true);
    std::fs::remove_file(sb.dir("claude_home/sessions/c46d79b9.json")).unwrap(); // the admiral is down
    let r = sb.run_env(&["start", "MAIN", "--yes"], &[("NIMITZ_MUX", "auto")]);
    assert!(!r.out.contains("mux.none"), "NIMITZ_MUX=auto with HERDR_ENV=1 and a herdr binary fell back to none:\n{}", r.out);
}

#[test]
#[ignore = "BUG-02"]
fn bug02_two_concurrent_starts_make_one_session() {
    let sb = Sb::new(false);
    let a = { let v = sb.vars.clone(); std::thread::spawn(move || raw(&v, &["start", "MAIN/ARGOS/FRESH", "--yes"], &[], None, None)) };
    let b = { let v = sb.vars.clone(); std::thread::spawn(move || raw(&v, &["start", "MAIN/ARGOS/FRESH", "--yes"], &[], None, None)) };
    let (a, b) = (a.join().unwrap(), b.join().unwrap());
    let n = sb.live_in(&sb.dir("dirs/main/fresh"));
    assert_eq!(n, 1, "two `nimitz start` at once left {n} live sessions for one ship\n{}\n{}", a.out, b.out);
}

#[test]
#[ignore = "BUG-03"]
fn bug03_concurrent_starts_of_a_stopped_ship_make_no_copies() {
    let sb = Sb::new(false);
    let hs: Vec<_> = (0..3)
        .map(|_| {
            let v = sb.vars.clone();
            std::thread::spawn(move || raw(&v, &["start", "MAIN/ARGOS/STOPPED", "--yes"], &[], None, None))
        })
        .collect();
    let outs: Vec<Run> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    let n = sb.live_in(&sb.dir("dirs/main/stopped"));
    let copies = outs.iter().filter(|r| r.out.contains("started a copy")).count();
    assert!(n == 1 && copies == 0, "{n} live sessions, {copies} copies reported as [OK]");
}

#[test]
#[ignore = "BUG-04"]
fn bug04_a_symlinked_path_to_a_trusted_folder_is_trusted() {
    let sb = Sb::new(false);
    let link = sb.dir("link-to-fresh");
    std::os::unix::fs::symlink(sb.dir("dirs/main/fresh"), &link).unwrap();
    let r = sb.run(&["ship", "set", "MAIN/ARGOS/FRESH", "--dir", link.to_str().unwrap(), "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let c = sb.run(&["check", "MAIN/ARGOS/FRESH"]);
    assert!(!c.out.contains("folder.untrusted"), "the same folder through a symlink (/tmp -> /private/tmp) reads as untrusted:\n{}", c.out);
}

#[test]
#[ignore = "BUG-05"]
fn bug05_an_empty_dir_is_refused() {
    let sb = Sb::new(false);
    let r = sb.run(&["ship", "set", "MAIN/ARGOS/FRESH", "--dir", "", "--yes"]);
    assert_ne!(r.code, 0, "`--dir ''` silently removed the ship's folder on this machine:\n{}", r.out);
}

#[test]
#[ignore = "BUG-06"]
fn bug06_partial_labels_do_not_make_the_caller_the_user() {
    let sb = Sb::new(false);
    for extra in [&[("NIMITZ_ROLE", "ship")][..], &[("NIMITZ_ID", "MAIN/ARGOS/CLEAN")][..], &[("NIMITZ_ROLE", "commodore"), ("NIMITZ_ID", "MAIN/ARGOS/CLEAN")][..]] {
        let r = sb.run_env(&["start", "MAIN/ARGOS/STOPPED", "--plan"], extra);
        assert!(r.out.contains("authority"), "labels {extra:?} were trusted as the user:\n{}", r.out);
    }
}

#[test]
#[ignore = "BUG-07"]
fn bug07_messages_say_what_was_refused() {
    let sb = Sb::new(false);
    let s = sb.run(&["status"]);
    assert!(!s.out.contains("an commodore"), "grammar: {}", s.out.lines().find(|l| l.contains("an commodore")).unwrap_or(""));
    let r = sb.run_env(&["stop", "MAIN/ARGOS/CLEAN", "--yes"], &[("NIMITZ_ROLE", "commodore"), ("NIMITZ_ID", "MAIN/BOREAL")]);
    assert!(!r.out.contains("may not start"), "a refused stop says `may not start`:\n{}", r.out);
}

// ------------------------------------------------------------------ B. breaking a live session

#[test]
#[ignore = "BUG-08"]
fn bug08_name_taken_fix_works_for_a_holder_outside_the_fleet() {
    let sb = Sb::new(false);
    let rand = sb.dir("home/rand");
    std::fs::create_dir_all(&rand).unwrap();
    sb.trust(&rand);
    sb.claude(&rand, &["--bg", "--name", "[MAIN.ARGOS]-STOPPED", "do y"]);
    let c = sb.run(&["check", "MAIN/ARGOS/STOPPED"]);
    let fix = c.out.lines().find(|l| l.contains("fix:") && l.contains("nimitz stop")).expect("a name.taken fix");
    let short = fix.split("nimitz stop ").nth(1).unwrap().split_whitespace().next().unwrap();
    let r = sb.run(&["stop", short, "--yes"]);
    assert_eq!(r.code, 0, "the fix `{}` does not run:\n{}", fix.trim(), r.out);
}

#[test]
#[ignore = "BUG-09"]
fn bug09_refusals_carry_their_own_code() {
    let sb = Sb::new(false);
    let r = sb.run(&["rm", "060f8274", "--yes"]); // a live stowaway
    assert!(!r.out.contains("(config.invalid)"), "a live-session refusal is filed as config.invalid:\n{}", r.out);
}

#[test]
#[ignore = "BUG-10"]
fn bug10_relaunch_refuses_while_the_name_is_taken() {
    let sb = Sb::new(false);
    let sid = sb.sid("MAIN/ARGOS/STOPPED");
    sb.claude(&sb.dir("dirs/main/stopped"), &["--bg", "--resume", &sid, "--name", "[MAIN.ARGOS]-STOPPED"]);
    sb.run(&["relaunch", "MAIN/ARGOS/STOPPED", "--yes"]);
    let n = sb.live_in(&sb.dir("dirs/main/stopped"));
    assert_eq!(n, 1, "relaunch went ahead with name.taken: {n} live sessions now answer to [MAIN.ARGOS]-STOPPED");
}

#[test]
#[ignore = "BUG-11"]
fn bug11_a_hand_made_copy_is_tied_to_its_ship() {
    let sb = Sb::new(false);
    let sid = sb.sid("MAIN/ARGOS/CLEAN");
    sb.claude(&sb.dir("dirs/main/clean"), &["--bg", "--resume", &sid]); // resume of a live ship: a copy
    let c = sb.run(&["check", "MAIN/ARGOS/CLEAN"]);
    assert!(c.out.contains("copy.rows") || c.out.contains("dup.live"), "the copy shows only as an anonymous stowaway; check is green:\n{}", c.out);
}

#[test]
#[ignore = "BUG-12"]
fn bug12_a_row_less_ship_with_a_transcript_is_not_cold_launched() {
    let sb = Sb::new(false);
    sb.claude(&sb.dir("dirs/main/stopped"), &["rm", "d5a94781"]);
    let m = sb.member("MAIN/ARGOS/STOPPED").unwrap();
    let reasons = m["reasons"].to_string();
    assert!(!reasons.contains("no transcript"), "the transcript is there, status says it is not: {reasons}");
    let p = sb.run(&["start", "MAIN/ARGOS/STOPPED", "--plan"]);
    assert!(p.out.contains("--resume"), "start throws the history away with a cold launch:\n{}", p.out);
}

#[test]
#[ignore = "BUG-13"]
fn bug13_an_unreadable_session_file_is_not_stopped() {
    let sb = Sb::new(false);
    std::fs::write(sb.dir("claude_home/sessions/2287e6e4.json"), b"").unwrap(); // a session file caught mid-write
    let m = sb.member("MAIN/ARGOS/CLEAN").unwrap();
    assert_ne!(m["status"], "Stopped", "a live ship with an unreadable session file reads as stopped: {}", m["reasons"]);
    let p = sb.run(&["start", "MAIN/ARGOS/CLEAN", "--plan"]);
    assert!(!p.out.contains("--resume"), "start would resume a session whose process may be live:\n{}", p.out);
}

#[test]
#[ignore = "BUG-14"]
fn bug14_a_corrupt_job_row_is_reported() {
    let sb = Sb::new(false);
    std::fs::write(sb.dir("claude_home/jobs/d5a94781/state.json"), b"not json{").unwrap();
    let c = sb.run(&["check", "MAIN/ARGOS/STOPPED"]);
    assert_ne!(c.code, 0, "a corrupt job row is ignored and the ship would be cold-launched:\n{}", c.out);
}

// ------------------------------------------------------------------ C. config and folders

#[test]
#[ignore = "BUG-15"]
fn bug15_one_broken_squadron_does_not_hide_the_fleet() {
    let sb = Sb::new(false);
    sb.edit(&sb.squadron_toml(), |s| s + "\nname = \"oops\n");
    assert!(sb.member("MAIN/BOREAL").is_some(), "a syntax error in ARGOS's squadron.toml hid the admiral and BOREAL");
    let s = sb.run(&["status"]);
    assert!(!s.out.contains("which no config declares"), "every live session got a `nimitz rm` fix:\n{}", s.out.lines().take(30).collect::<Vec<_>>().join("\n"));
}

#[test]
#[ignore = "BUG-16"]
fn bug16_two_ships_in_one_folder_are_reported() {
    let sb = Sb::new(false);
    let clean = sb.dir("dirs/main/clean");
    let copies = sb.dir("dirs/main/copies");
    sb.edit(&sb.squadron_toml(), |s| s.replace(copies.to_str().unwrap(), clean.to_str().unwrap()));
    let c = sb.run(&["check", "MAIN/ARGOS/COPIES"]);
    assert!(c.out.contains("config.layout"), "COPIES and CLEAN share a folder; check is silent:\n{}", c.out);
}

#[test]
#[ignore = "BUG-17"]
fn bug17_every_inherited_claude_md_is_reported() {
    for rel in ["nimitz/CLAUDE.md", "nimitz/main/squadrons/CLAUDE.md", "nimitz/main/ships/CLAUDE.md", "nimitz/main/.claude/CLAUDE.md"] {
        let sb = Sb::new(false);
        let p = sb.dir(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "inherited by every session below\n").unwrap();
        let c = sb.run(&["check"]);
        assert!(c.out.contains(p.to_str().unwrap()), "{rel} is not reported");
    }
}

#[test]
#[ignore = "BUG-18"]
fn bug18_a_corrupt_registry_is_kept_and_reported() {
    let sb = Sb::new(false);
    let reg = sb.dir("state/registry.json");
    std::fs::write(&reg, b"{garbage").unwrap();
    let r = sb.run(&["start", "MAIN/ARGOS/FRESH", "--yes"]);
    let kept = std::fs::read_dir(sb.dir("state")).unwrap().flatten().any(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        n != "registry.json" && n.starts_with("registry") && std::fs::read(e.path()).unwrap_or_default() == b"{garbage"
    });
    assert!(kept || r.out.to_lowercase().contains("registry"), "the corrupt registry was replaced in silence; every lineage is gone:\n{}", r.out);
}

#[test]
#[ignore = "BUG-19"]
fn bug19_a_stowaway_can_be_adopted_as_a_tender() {
    let sb = Sb::new(false);
    let r = sb.run(&["adopt", "060f8274", "MAIN/ARGOS/~adopted", "--yes"]);
    assert_eq!(r.code, 0, "plan-v3 5.3 says adopt makes a ship or a tender:\n{}", r.out);
}

#[test]
#[ignore = "BUG-20"]
fn bug20_machine_add_does_not_copy_this_hostname() {
    let sb = Sb::new(false);
    sb.run(&["machine", "add", "MAIN", "second", "--yes"]);
    let t = std::fs::read_to_string(sb.dir("nimitz/main/fleet.toml")).unwrap();
    assert!(t.matches("hostname = \"sandbox-host\"").count() <= 1, "two machine labels now name this host:\n{t}");
}

#[test]
#[ignore = "BUG-21"]
fn bug21_a_stale_ship_lock_is_noticed_on_its_own_machine() {
    let sb = Sb::new(false);
    assert_eq!(sb.run(&["ship", "new", "MAIN/ARGOS/MUD", "--dir", "managed", "--prompt", "hi", "--yes"]).code, 0);
    assert_eq!(sb.run(&["start", "MAIN/ARGOS/MUD", "--yes"]).code, 0);
    let short = sb.sid("MAIN/ARGOS/MUD")[..8].to_string();
    let dir = sb.dir("nimitz/main/ships/argos/mud");
    sb.claude(&dir, &["stop", &short]); // stopped by hand, or it crashed
    let lock = dir.join(".nimitz-ship.lock");
    let c = sb.run(&["check", "MAIN/ARGOS/MUD"]);
    assert!(!lock.exists() || c.out.contains("lock"), "a lock for a stopped ship stays, and blocks every other machine; this one says nothing:\n{}", c.out);
}

// ------------------------------------------------------------------ D/E/F

#[test]
#[ignore = "BUG-22"]
fn bug22_a_uds_address_does_not_bypass_the_comms_rule() {
    // Real Claude Code 2.1.288 delivers SendMessage to `uds:/tmp/cc-socks/<pid>.sock`.
    let sb = Sb::new(false);
    let input = r#"{"session_id":"s","tool_name":"SendMessage","tool_input":{"to":"uds:/tmp/cc-socks/999999.sock","message":"hi"}}"#;
    let r = raw(&sb.vars, &["hook", "PreToolUse"], &[("NIMITZ_ID", "MAIN/BOREAL/X"), ("NIMITZ_ROLE", "ship"), ("NIMITZ_FLEET", "MAIN"), ("NIMITZ_SQUADRON", "BOREAL")], Some(input), None);
    assert_eq!(hook_decision(&r.out), "deny", "a ship of a commodore-only squadron signals an unknown session by socket");
}

#[test]
#[ignore = "BUG-23"]
fn bug23_labels_in_the_job_row_count_when_session_start_is_lost() {
    let sb = Sb::new(false);
    assert_eq!(sb.run(&["start", "MAIN/ARGOS/FRESH", "--yes"]).code, 0);
    // SessionStart lost and the registry never bound it (state not writable at the time)
    let ev = sb.dir("state/events.jsonl");
    sb.edit(&ev, |s| s.lines().filter(|l| !(l.contains("\"SessionStart\"") && l.contains("FRESH"))).map(|l| format!("{l}\n")).collect());
    let reg = sb.dir("state/registry.json");
    sb.edit(&reg, |s| {
        let mut v: Value = serde_json::from_str(&s).unwrap();
        v["identities"].as_object_mut().unwrap().remove("MAIN/ARGOS/FRESH");
        v.to_string()
    });
    let st = sb.stowaways();
    assert!(!st.iter().any(|s| s["name"] == "[MAIN.ARGOS]-FRESH"), "a labelled ship is called an unlabelled stowaway: {st:?}");
}

#[test]
#[ignore = "BUG-24"]
fn bug24_check_of_a_fleet_or_squadron_covers_its_members() {
    let sb = Sb::new(false);
    for id in ["MAIN", "MAIN/ARGOS"] {
        let c = sb.run(&["check", id]);
        assert!(c.out.contains("MAIN/ARGOS/DRIFT"), "`nimitz check {id}` misses its ships' errors:\n{}", c.out);
    }
}

#[test]
#[ignore = "BUG-26"]
fn bug26_a_ship_may_not_take_the_commodores_address() {
    let sb = Sb::new(false);
    let r = sb.run(&["ship", "new", "MAIN/ARGOS/COMMODORE", "--dir", "managed", "--prompt", "x", "--yes"]);
    assert_ne!(r.code, 0, "ship COMMODORE is named [MAIN.ARGOS]-COMMODORE, the commodore's address:\n{}", r.out);
}

#[test]
#[ignore = "BUG-27"]
fn bug27_the_missing_squadron_fix_runs() {
    let sb = Sb::new(false);
    let r = sb.run(&["ship", "new", "MAIN/NOPE/X", "--dir", "managed", "--prompt", "x", "--yes"]);
    let fix = r.out.lines().find(|l| l.contains("fix:")).unwrap_or("").to_string();
    assert!(fix.contains("squadron new MAIN/NOPE"), "the fix is not valid syntax: {fix}");
}

#[test]
#[ignore = "BUG-29"]
fn bug29_a_session_that_lost_its_labels_is_flagged() {
    // Real Claude Code: after its row is removed, `--resume <id> --name X` continues the SAME
    // id with no labels (SessionStart source resume, labels {}).
    let sb = Sb::new(false);
    let sid = sb.sid("MAIN/ARGOS/CLEAN");
    let line = serde_json::json!({"ts": 4.0e9, "event": "SessionStart", "session_id": sid, "source": "resume", "reason": null,
        "agent_id": null, "tool_name": null, "to": null, "pid": 1, "labels": {}});
    let ev = sb.dir("state/events.jsonl");
    sb.edit(&ev, |s| format!("{s}{line}\n"));
    let c = sb.run(&["check", "MAIN/ARGOS/CLEAN"]);
    assert!(c.out.contains("labels.missing"), "the newest SessionStart carried no labels; nimitz still trusts an older one:\n{}", c.out);
}

#[test]
#[ignore = "BUG-30"]
fn bug30_a_labelled_session_in_the_wrong_folder_is_reported() {
    let sb = Sb::new(false);
    let settings = r#"{"env":{"NIMITZ_FLEET":"MAIN","NIMITZ_SQUADRON":"ARGOS","NIMITZ_ROLE":"ship","NIMITZ_ID":"MAIN/ARGOS/FRESH"}}"#;
    sb.claude(&sb.dir("dirs/main/stopped"), &["--bg", "--name", "[MAIN.ARGOS]-FRESH", "--settings", settings, "hi"]);
    let c = sb.run(&["check", "MAIN/ARGOS/FRESH"]);
    assert!(c.out.to_lowercase().contains("folder"), "FRESH runs in STOPPED's folder and nimitz accepts it:\n{}", c.out);
}

#[test]
#[ignore = "BUG-31"]
fn bug31_a_row_in_another_folder_than_the_config_is_reported() {
    let sb = Sb::new(false);
    let moved = sb.dir("dirs/main/stopped-moved");
    std::fs::rename(sb.dir("dirs/main/stopped"), &moved).unwrap();
    sb.trust(&moved);
    assert_eq!(sb.run(&["ship", "set", "MAIN/ARGOS/STOPPED", "--dir", moved.to_str().unwrap(), "--yes"]).code, 0);
    let c = sb.run(&["check", "MAIN/ARGOS/STOPPED"]);
    // Real Claude Code resumes in the row's saved cwd: "Workspace not trusted ... could not be resolved on disk".
    assert_ne!(c.code, 0, "the job row's folder no longer exists; check is green and start will fail:\n{}", c.out);
}

// ------------------------------------------------------------------ H. migration

#[test]
#[ignore = "BUG-25"]
fn bug25_migration_reports_a_ship_rooted_at_the_home_folder() {
    // general-fleet's HOME agent lives in the home folder: every other ship nests in it.
    let sb = Sb::new(false);
    let fx = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nimitz-core/tests/fixtures/old-fleets");
    let dirs: Vec<String> = ["general-fleet", "argos-fleet"].iter().map(|d| fx.join(d).to_string_lossy().into_owned()).collect();
    let mut args = vec!["migrate", "fleet-skill"];
    args.extend(dirs.iter().map(String::as_str));
    args.extend(["--into", "OLD", "--yes"]);
    let host = [("NIMITZ_HOSTNAME", "3f2c1d0e9b7a")];
    let m = sb.run_env(&args, &host);
    assert_eq!(m.code, 0, "{}", m.out);
    let c = sb.run_env(&["check", "OLD/ARGOS/ARGOS"], &host);
    if c.out.contains("config.layout") {
        assert!(m.out.contains("config.layout") || m.out.to_lowercase().contains("nest"), "the import made layout errors and said nothing:\n{}", m.out);
    }
}
