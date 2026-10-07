//! The stowaway rule of 0.4.0 (plan-v4 section 6), one test per row of the
//! session-mapping coverage matrix where one exists
//! (`docs/design/session-mapping/index.html`, round 2): the user's own work in
//! a ship's folder is not shown; a copy of a ship's conversation (by its
//! transcript, a resume of a lineage id, or its name) is the ship's extra row
//! with `copy` set and one working fix per kind; `sweep` stops and removes
//! background copies; another home's sessions never appear; Claude Code's
//! transient processes (spares, pickers) are ignored for a grace time.
//! Everything runs through the sandbox's fake `claude` (`sandbox.rs`).
use nimitz_core::env::Env;
use nimitz_core::types::*;
use nimitz_core::{check, claude, evidence, sandbox, sessions, snapshot};
use serde_json::{json, Value};

const EXE: &str = env!("CARGO_BIN_EXE_nimitz-sandbox");

struct W {
    _td: tempfile::TempDir,
    env: Env,
    base: std::path::PathBuf,
}

fn world() -> W {
    let td = tempfile::tempdir().unwrap();
    let base = td.path().join("sb");
    sandbox::create(&base, std::path::Path::new(EXE)).expect("sandbox");
    let env = sandbox::env(&base);
    W { _td: td, env, base }
}

impl W {
    fn snap(&self) -> Snapshot {
        snapshot::snapshot(&self.env, &mut snapshot::Cache::default())
    }
    fn ship(&self, name: &str) -> Id {
        Id::ship("MAIN", "ARGOS", name)
    }
    fn member(&self, snap: &Snapshot, id: &Id) -> Member {
        snap.fleets
            .iter()
            .flat_map(|f| std::iter::once(&f.admiral).chain(f.squadrons.iter().flat_map(|s| std::iter::once(&s.commodore).chain(s.ships.iter()))))
            .find(|m| &m.id == id)
            .cloned()
            .unwrap_or_else(|| panic!("no member {id}"))
    }
    /// The fake `claude` (the native binary), in `cwd`: (exit code, output).
    fn claude(&self, args: &[&str], cwd: &str) -> (i32, String) {
        let mut cmd = vec![self.env.native_claude_bin.clone()];
        cmd.extend(args.iter().map(|a| a.to_string()));
        claude::run(&cmd, Some(cwd))
    }
    fn claude_ok(&self, args: &[&str], cwd: &str) -> String {
        let (rc, out) = self.claude(args, cwd);
        assert_eq!(rc, 0, "claude {args:?}: {out}");
        out
    }
    /// The short id a `--bg` launch printed (the last word of its output).
    fn short_of(out: &str) -> String {
        out.split_whitespace().last().unwrap().to_string()
    }
    /// A session file written by hand (a process no hook spoke for), in `cwd`.
    fn session_file(&self, name: &str, cwd: &str, extra: Value) -> String {
        let sid = format!("{name}-0000-4000-8000-000000000000");
        let mut d = json!({"pid": sandbox::LIVE_PID, "sessionId": sid, "cwd": cwd, "startedAt": (evidence::now() * 1000.0) as i64,
                           "kind": "interactive", "name": name, "jobId": null, "status": "idle", "entrypoint": "cli"});
        for (k, v) in extra.as_object().unwrap() {
            d[k] = v.clone();
        }
        std::fs::write(self.base.join("claude_home/sessions").join(format!("{name}.json")), serde_json::to_vec_pretty(&d).unwrap()).unwrap();
        sid
    }
    /// Let Claude Code trust a folder (`.claude.json`).
    fn trust(&self, dir: &str) {
        let f = self.base.join("claude_home/.claude.json");
        let mut v: Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        v["projects"][dir] = json!({"hasTrustDialogAccepted": true});
        std::fs::write(&f, serde_json::to_vec(&v).unwrap()).unwrap();
    }
}

fn codes(issues: &[Issue]) -> Vec<&str> {
    issues.iter().map(|i| i.code.as_str()).collect()
}

fn has(issues: &[Issue], code: &str) -> bool {
    issues.iter().any(|i| i.code == code)
}

fn stowaway(snap: &Snapshot, short_or_sid: &str) -> bool {
    snap.stowaways.iter().any(|s| s.short_id.as_deref() == Some(short_or_sid) || s.session_id == short_or_sid)
}

fn copy_of<'a>(m: &'a Member, short_or_sid: &str) -> Option<&'a SessionInfo> {
    m.extra_rows.iter().find(|r| r.short_id.as_deref() == Some(short_or_sid) || r.session_id == short_or_sid)
}

/// The live CLEAN ship: (id, job, session id, folder).
fn clean(w: &W) -> (Id, String, String, String) {
    let id = w.ship("CLEAN");
    let m = w.member(&w.snap(), &id);
    let s = m.session.expect("CLEAN is live");
    (id, s.short_id.unwrap(), s.session_id, m.cwd.unwrap())
}

/// The stopped STOPPED ship: (id, job, session id, folder).
fn stopped(w: &W) -> (Id, String, String, String) {
    let id = w.ship("STOPPED");
    let m = w.member(&w.snap(), &id);
    let s = m.session.expect("STOPPED has a row");
    (id, s.short_id.unwrap(), s.session_id, m.cwd.unwrap())
}

// ------------------------------------------------------------------ the user's own work

#[test]
fn cli_28_the_users_own_interactive_session_in_a_ship_folder_is_not_shown() {
    let w = world();
    let (id, _, _, cwd) = clean(&w);
    let out = w.claude_ok(&["--name", "ship1-46"], &cwd);
    assert!(out.contains("startup"), "{out}");
    let snap = w.snap();
    assert!(snap.stowaways.iter().all(|s| s.name != "ship1-46"), "{:?}", snap.stowaways);
    let m = w.member(&snap, &id);
    assert!(m.extra_rows.is_empty(), "{:?}", m.extra_rows);
    assert!(m.issues.is_empty(), "{:?}", m.issues);
    assert!(!check::check(&w.env, &snap, None).iter().any(|i| i.code == "stowaway" && i.message.contains("ship1-46")));
}

#[test]
fn r_11_an_sdk_session_of_the_users_own_in_a_ship_folder_is_not_shown() {
    let w = world();
    let (id, _, _, cwd) = clean(&w);
    w.claude_ok(&["-p", "--name", "E2E5-SDK-STOWAWAY"], &cwd);
    let snap = w.snap();
    assert!(snap.stowaways.iter().all(|s| s.name != "E2E5-SDK-STOWAWAY"), "{:?}", snap.stowaways);
    assert!(w.member(&snap, &id).issues.is_empty());
}

#[test]
fn cli_12_a_fresh_background_job_of_the_users_own_in_a_ship_folder_is_not_shown() {
    let w = world();
    let (id, _, _, cwd) = clean(&w);
    let short = W::short_of(&w.claude_ok(&["--bg", "--name", "my own job", "x"], &cwd));
    let snap = w.snap();
    assert!(!stowaway(&snap, &short));
    let m = w.member(&snap, &id);
    assert!(copy_of(&m, &short).is_none() && m.issues.is_empty(), "{:?} {:?}", m.extra_rows, m.issues);
    // the same session in the squadron's own folder (no ship's) is a stowaway, as before
    let sq = w.member(&snap, &Id::commodore("MAIN", "ARGOS")).cwd.unwrap();
    for p in [w.base.join("claude_home/jobs").join(&short).join("state.json"), w.base.join("claude_home/sessions").join(format!("{short}.json"))] {
        let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        v["cwd"] = json!(sq);
        std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    }
    assert!(stowaway(&w.snap(), &short));
}

// ------------------------------------------------------------------ copies

#[test]
fn cli_10_a_fork_under_another_name_is_told_by_its_transcript() {
    let w = world();
    let (id, _, sid, cwd) = stopped(&w);
    // a fork under the name given: not the ship's name, so only the transcript
    // (the conversation's root) ties it to the ship
    let out = w.claude_ok(&["--bg", "--resume", &sid, "--fork-session", "--name", "X-E2E2-CLI"], &cwd);
    let copy = W::short_of(&out);
    let snap = w.snap();
    assert!(!stowaway(&snap, &copy), "{:?}", snap.stowaways);
    let m = w.member(&snap, &id);
    let r = copy_of(&m, &copy).expect("the copy is STOPPED's extra row");
    assert_eq!(r.copy.as_deref(), Some("transcript"));
    assert_eq!(r.name, "X-E2E2-CLI");
    let i = m.issues.iter().find(|i| i.code == "copy").expect("a copy issue");
    assert_eq!(i.severity, Severity::Warn);
    assert!(i.message.contains(&copy) && i.message.contains("transcript"), "{i:?}");
    assert_eq!(i.fix.as_deref(), Some("nimitz sweep MAIN/ARGOS/STOPPED"));
    assert!(!has(&m.issues, "name.taken") && !has(&m.issues, "copy.rows"), "{:?}", codes(&m.issues));
    // the member's own session is untouched
    assert_eq!(m.session.as_ref().map(|s| s.session_id.as_str()), Some(sid.as_str()));
}

#[test]
fn cli_13_a_fork_session_copy_of_a_stopped_ship_is_a_copy() {
    let w = world();
    let (id, _, sid, cwd) = stopped(&w);
    let copy = W::short_of(&w.claude_ok(&["--bg", "--resume", &sid, "--fork-session"], &cwd));
    let snap = w.snap();
    assert!(!stowaway(&snap, &copy));
    let m = w.member(&snap, &id);
    assert!(copy_of(&m, &copy).is_some_and(|r| r.copy.is_some()), "{:?}", m.extra_rows);
    assert!(!has(&m.issues, "name.taken"), "{:?}", codes(&m.issues));
}

#[test]
fn cli_l8_an_interactive_fork_is_a_copy_ended_in_its_terminal() {
    let w = world();
    let (id, _, sid, cwd) = stopped(&w);
    let out = w.claude_ok(&["--resume", &sid, "--fork-session", "--name", "[MAIN.ARGOS]-STOPPED"], &cwd);
    assert!(out.contains("(fork)"), "{out}");
    let fork = out.split("interactive session ").nth(1).unwrap().split_whitespace().next().unwrap().to_string();
    let snap = w.snap();
    assert!(!stowaway(&snap, &fork));
    let m = w.member(&snap, &id);
    let r = copy_of(&m, &fork).expect("the fork is STOPPED's extra row");
    assert!(r.short_id.is_none() && r.pid.is_some());
    assert_eq!(r.copy.as_deref(), Some("transcript"));
    assert!(!has(&m.issues, "held") && !has(&m.issues, "dup.live") && !has(&m.issues, "name.taken"), "{:?}", codes(&m.issues));
    let i = m.issues.iter().find(|i| i.code == "copy").unwrap();
    assert!(i.fix.as_deref().unwrap().starts_with("end it in its terminal"), "{i:?}");
    // sweep cannot end it: it says so, with the same fix, and runs nothing
    let plan = sessions::plan_sweep(&w.env, &snap, Some(&id), &Caller::User);
    assert!(plan.steps.is_empty(), "{:?}", plan.steps);
    assert!(plan.warnings.iter().any(|i| i.code == "copy" && i.fix.as_deref().unwrap().starts_with("end it in its terminal")), "{:?}", plan.warnings);
}

#[test]
fn cli_27_a_session_carrying_the_ships_exact_name_in_its_folder_is_a_copy() {
    let w = world();
    let (id, job, _, cwd) = clean(&w);
    let copy = W::short_of(&w.claude_ok(&["--bg", "--name", &id.name(), "x"], &cwd));
    let snap = w.snap();
    assert!(!stowaway(&snap, &copy));
    let m = w.member(&snap, &id);
    assert_eq!(m.session.as_ref().and_then(|s| s.short_id.as_deref()), Some(job.as_str()), "the ship keeps its job");
    assert_eq!(m.status, Status::Idle);
    assert_eq!(copy_of(&m, &copy).and_then(|r| r.copy.as_deref()), Some("name"), "{:?}", m.extra_rows);
    assert!(has(&m.issues, "copy") && !has(&m.issues, "name.taken") && !has(&m.issues, "dup.live"), "{:?}", codes(&m.issues));
    // the same name in another folder is name.taken, as before
    let elsewhere = w.base.join("dirs/elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let elsewhere = elsewhere.canonicalize().unwrap().to_string_lossy().into_owned();
    w.trust(&elsewhere);
    let other = W::short_of(&w.claude_ok(&["--bg", "--name", &id.name(), "x"], &elsewhere));
    let m = w.member(&w.snap(), &id);
    assert!(copy_of(&m, &other).is_none());
    assert!(has(&m.issues, "name.taken"), "{:?}", codes(&m.issues));
}

#[test]
fn cli_24_a_resume_of_an_older_lineage_conversation_is_a_copy() {
    let w = world();
    let (id, job, born, cwd) = clean(&w);
    // `/clear` moves the ship to a new conversation; the old id stays in its lineage
    w.claude_ok(&["__inside", &job, "clear"], &cwd);
    w.claude_ok(&["stop", &job], &cwd);
    let m = w.member(&w.snap(), &id);
    let now = m.session.as_ref().unwrap().session_id.clone();
    assert_ne!(now, born);
    // the user resumes the old conversation by hand, in a process with no job
    let out = w.claude_ok(&["--resume", &born], &cwd);
    assert!(out.contains("(resume)"), "{out}");
    let snap = w.snap();
    assert!(!stowaway(&snap, &born));
    let m = w.member(&snap, &id);
    assert_eq!(m.session.as_ref().map(|s| s.session_id.as_str()), Some(now.as_str()), "the ship's conversation is still the one after /clear");
    let r = copy_of(&m, &born).expect("the resumed old conversation is a copy");
    assert_eq!(r.copy.as_deref(), Some("resume"));
    assert!(!has(&m.issues, "held"), "an older conversation is a copy, not the member held: {:?}", codes(&m.issues));
    let i = m.issues.iter().find(|i| i.code == "copy").unwrap();
    assert!(i.message.contains("lineage") && i.fix.as_deref().unwrap().starts_with("end it in its terminal"), "{i:?}");
    // the member's own conversation in a process with no job stays `held` (M13)
    w.claude_ok(&["__end", &born], &cwd);
    w.claude_ok(&["--resume", &now], &cwd);
    let m = w.member(&w.snap(), &id);
    assert!(has(&m.issues, "held") && !has(&m.issues, "copy"), "{:?}", codes(&m.issues));
}

// ------------------------------------------------------------------ sweep

#[test]
fn cli_g4_sweep_stops_and_removes_background_copies_after_one_confirm() {
    let w = world();
    let (id, job, sid, cwd) = stopped(&w);
    let live_copy = W::short_of(&w.claude_ok(&["--bg", "--name", &id.name(), "x"], &cwd));
    let stopped_copy = W::short_of(&w.claude_ok(&["--bg", "--resume", &sid, "--fork-session"], &cwd));
    w.claude_ok(&["stop", &stopped_copy], &cwd);
    let snap = w.snap();
    let m = w.member(&snap, &id);
    assert_eq!(m.issues.iter().filter(|i| i.code == "copy").count(), 2, "{:?}", m.issues);
    let plan = sessions::plan_sweep(&w.env, &snap, Some(&id), &Caller::User);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let cmds: Vec<String> = plan.steps.iter().map(|s| s.cmd[1..].join(" ")).collect();
    assert!(cmds.contains(&format!("stop {live_copy}")) && cmds.contains(&format!("rm {live_copy}")), "{cmds:?}");
    assert!(cmds.contains(&format!("rm {stopped_copy}")), "{cmds:?}");
    assert!(!cmds.iter().any(|c| c.contains(&job)), "the member's own row stays: {cmds:?}");
    assert!(plan.warnings.iter().all(|i| i.code != "dup.live"), "a live copy is swept, not skipped: {:?}", plan.warnings);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let rows = evidence::rows_from_files(&w.env, &mut evidence::FileCache::default()).unwrap();
    assert!(!rows.iter().any(|r| r.short_id.as_deref() == Some(live_copy.as_str()) || r.short_id.as_deref() == Some(stopped_copy.as_str())));
    assert!(rows.iter().any(|r| r.short_id.as_deref() == Some(job.as_str())));
    let m = w.member(&w.snap(), &id);
    assert!(!has(&m.issues, "copy") && m.extra_rows.is_empty(), "{:?}", m.issues);
    // a commodore sweeps its own ships' copies too; a ship only itself, and
    // never stops a live copy (the user's own fork may be at work): it
    // removes stopped copies and names the live one for the user or commodore
    let live_again = W::short_of(&w.claude_ok(&["--bg", "--name", &id.name(), "x"], &cwd));
    let stopped_again = W::short_of(&w.claude_ok(&["--bg", "--resume", &sid, "--fork-session"], &cwd));
    w.claude_ok(&["stop", &stopped_again], &cwd);
    let me = Caller::Member { id: Id::commodore("MAIN", "ARGOS"), role: Role::Commodore };
    let plan = sessions::plan_sweep(&w.env, &w.snap(), Some(&id), &me);
    assert!(plan.ok() && plan.steps.iter().any(|s| s.cmd[1..].join(" ") == format!("stop {live_again}")), "{:?}", plan.steps);
    let ship = Caller::Member { id: id.clone(), role: Role::Ship };
    let plan = sessions::plan_sweep(&w.env, &w.snap(), Some(&id), &ship);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert!(!plan.steps.iter().any(|s| s.cmd.get(1).map(String::as_str) == Some("stop")), "a ship's sweep has no stop step: {:?}", plan.steps);
    assert!(plan.steps.iter().any(|s| s.cmd[1..].join(" ") == format!("rm {stopped_again}")), "{:?}", plan.steps);
    assert!(!plan.steps.iter().any(|s| s.cmd.contains(&live_again)), "{:?}", plan.steps);
    let warn = plan.warnings.iter().find(|i| i.code == "copy" && i.message.contains(&live_again)).expect("the live copy is named");
    assert_eq!(warn.fix.as_deref(), Some("nimitz sweep MAIN/ARGOS/STOPPED  # you, or its commodore"));
    assert!(!sessions::plan_sweep(&w.env, &w.snap(), Some(&w.ship("CLEAN")), &ship).ok());
}

#[test]
fn a_start_with_a_live_copy_refuses_and_names_it() {
    let w = world();
    let (id, _, _, cwd) = stopped(&w);
    let copy = W::short_of(&w.claude_ok(&["--bg", "--name", &id.name(), "x"], &cwd));
    let snap = w.snap();
    let plan = sessions::plan_start(&w.env, &snap, &id, &Caller::User, false);
    assert!(has(&plan.refusals, "dup.live"), "{:?}", codes(&plan.refusals));
    assert!(plan.refusals.iter().any(|i| i.message.contains(&copy)), "{:?}", plan.refusals);
    // after a sweep, the start goes ahead as a plain resume
    let sweep = sessions::plan_sweep(&w.env, &snap, Some(&id), &Caller::User);
    assert!(sessions::apply(&w.env, &sweep).verified);
    let plan = sessions::plan_start(&w.env, &w.snap(), &id, &Caller::User, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
}

// ------------------------------------------------------------------ another home

#[test]
fn nimitz_g7_sessions_labelled_for_another_home_never_appear() {
    let w = world();
    let (id, _, _, cwd) = clean(&w);
    let sq = w.member(&w.snap(), &Id::commodore("MAIN", "ARGOS")).cwd.unwrap();
    w.trust(&sq);
    let mut labels = claude::labels_for(&w.env, &id);
    labels.insert("NIMITZ_HOME".into(), "/elsewhere/nimitz".into());
    let settings = json!({"env": labels}).to_string();
    let a = W::short_of(&w.claude_ok(&["--bg", "--name", &id.name(), "--settings", &settings, "x"], &cwd));
    let b = W::short_of(&w.claude_ok(&["--bg", "--name", "[OTHER]-ADMIRAL", "--settings", &settings, "x"], &sq));
    let snap = w.snap();
    for short in [&a, &b] {
        assert!(!stowaway(&snap, short), "{short} is another home's");
        let all = check::check(&w.env, &snap, None);
        assert!(!all.iter().any(|i| i.message.contains(short.as_str()) || i.fix.as_deref().is_some_and(|f| f.contains(short.as_str()))), "{short} gets an issue: {all:?}");
    }
    let m = w.member(&snap, &id);
    assert!(m.extra_rows.is_empty() && m.issues.is_empty(), "{:?} {:?}", m.extra_rows, m.issues);
    assert!(!snap.issues.iter().any(|i| i.code == "config.invalid"), "{:?}", snap.issues);
    // and `status --json` shows no row for them
    let s = serde_json::to_string(&snap).unwrap();
    assert!(!s.contains(&a) && !s.contains(&b));
}

// ------------------------------------------------------------------ transient processes

#[test]
fn cli_05_a_spare_session_file_is_ignored_for_the_grace_time() {
    let w = world();
    let sq = w.member(&w.snap(), &Id::commodore("MAIN", "ARGOS")).cwd.unwrap();
    // the agent view's "new session": `spare: true`, no labels, in a fleet folder
    let sid = w.session_file("28f3d8a7", &sq, json!({"spare": true, "kind": "bg"}));
    // the real draft fires a SessionStart `startup` the moment the view claims it (byhand 2c-live)
    let line = json!({"ts": evidence::now(), "event": "SessionStart", "session_id": sid, "source": "startup", "reason": null, "pid": sandbox::LIVE_PID, "labels": {}, "rules": null, "job": null, "kind": "interactive", "entrypoint": "cli", "binding": null, "cwd": sq});
    let mut all = std::fs::read_to_string(w.env.events_path()).unwrap_or_default();
    all.push_str(&format!("{line}\n"));
    std::fs::write(w.env.events_path(), all).unwrap();
    let snap = w.snap();
    assert!(!stowaway(&snap, &sid), "{:?}", snap.stowaways);
    assert!(!check::check(&w.env, &snap, None).iter().any(|i| i.code == "stowaway" && i.message.contains(&sid)));
    // after the grace time it is classified as normal: a stowaway in the squadron
    // folder. A spare's age is its last touch (the view claims a pre-warmed
    // process: `startedAt` is old long before), so the file's mtime ages it
    let old = ((evidence::now() - snapshot::TRANSIENT_GRACE_S - 60.0) * 1000.0) as i64;
    w.session_file("28f3d8a7", &sq, json!({"spare": true, "kind": "bg", "startedAt": old}));
    assert!(!stowaway(&w.snap(), &sid), "an old startedAt alone does not age a spare");
    let f = std::fs::File::options().write(true).open(w.env.claude_home.join("sessions/28f3d8a7.json")).unwrap();
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(snapshot::TRANSIENT_GRACE_S as u64 + 60)).unwrap();
    assert!(stowaway(&w.snap(), &sid));
}

#[test]
fn r_10_a_picker_with_no_transcript_and_no_session_start_is_ignored_for_the_grace_time() {
    let w = world();
    let sq = w.member(&w.snap(), &Id::commodore("MAIN", "ARGOS")).cwd.unwrap();
    let sid = w.session_file("98651002", &sq, json!({"name": "nimitz-e2e-r2-remote-3e", "tmux": "probe:@1.%1"}));
    let snap = w.snap();
    assert!(!stowaway(&snap, &sid), "{:?}", snap.stowaways);
    let old = ((evidence::now() - snapshot::TRANSIENT_GRACE_S - 60.0) * 1000.0) as i64;
    w.session_file("98651002", &sq, json!({"name": "nimitz-e2e-r2-remote-3e", "tmux": "probe:@1.%1", "startedAt": old}));
    let snap = w.snap();
    assert!(stowaway(&snap, &sid));
    // a process with no job is ended in its terminal, never `nimitz stop`; in a
    // lead's folder it is that lead's `lead.intruder` (one issue per row)
    let issues = check::check(&w.env, &snap, None);
    assert!(!issues.iter().any(|i| i.code == "stowaway" && i.message.contains(&sid)), "{issues:?}");
    let i = issues.into_iter().find(|i| i.code == "lead.intruder" && i.message.contains("98651002") && i.id == Some(Id::commodore("MAIN", "ARGOS"))).unwrap();
    let fix = i.fix.unwrap();
    assert_eq!(fix, "end it in its terminal (tmux probe:@1.%1): /exit, or close the host");
    assert!(!fix.contains("nimitz stop"), "{fix}");
}

#[test]
fn inside_3_11_the_agent_views_spare_in_a_ship_folder_is_never_shown() {
    let w = world();
    let (id, _, _, cwd) = clean(&w);
    let old = ((evidence::now() - snapshot::TRANSIENT_GRACE_S - 60.0) * 1000.0) as i64;
    // young or old, an unlabelled session in a ship's folder is the user's own
    for started in [None, Some(old)] {
        let mut extra = json!({"spare": true, "kind": "bg"});
        if let Some(t) = started {
            extra["startedAt"] = json!(t);
        }
        let sid = w.session_file("178c24f4", &cwd, extra);
        let snap = w.snap();
        assert!(!stowaway(&snap, &sid));
        assert!(w.member(&snap, &id).issues.is_empty());
    }
}

#[test]
fn a_live_background_stowaway_keeps_the_stop_then_rm_fix() {
    let w = world();
    let snap = w.snap();
    let i = check::check(&w.env, &snap, None).into_iter().find(|i| i.code == "stowaway" && i.message.contains("quick look at the logs")).unwrap();
    let fix = i.fix.unwrap();
    assert!(fix.starts_with("nimitz stop ") && fix.contains(", then nimitz rm "), "{fix}");
}

#[test]
fn a_dispatched_job_that_was_a_pre_warmed_spare_is_never_hidden() {
    let w = world();
    let (id, job, _, _) = clean(&w);
    // the daemon claimed a spare for the ship's job: the session file keeps `spare: true`
    let p = w.base.join("claude_home/sessions").join(format!("{job}.json"));
    let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["spare"] = json!(true);
    v["startedAt"] = json!((evidence::now() * 1000.0) as i64);
    std::fs::write(&p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let m = w.member(&w.snap(), &id);
    assert_eq!(m.status, Status::Idle, "{:?}", m.reasons);
    assert_eq!(m.session.as_ref().and_then(|s| s.short_id.as_deref()), Some(job.as_str()));
}
