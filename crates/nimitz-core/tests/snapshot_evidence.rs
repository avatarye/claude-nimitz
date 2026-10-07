//! The snapshot and its evidence (contracts.md section 6): status with reasons,
//! crew, tasks, stowaways, extra rows, the caches, and tolerance of the files
//! Claude Code writes (missing fields, unknown fields, trailing bytes).
use nimitz_core::env::Env;
use nimitz_core::types::*;
use nimitz_core::{evidence, sandbox, snapshot};
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
    fn ship(&self, snap: &Snapshot, name: &str) -> Member {
        snap.fleets
            .iter()
            .flat_map(|f| f.squadrons.iter())
            .flat_map(|s| s.ships.iter())
            .find(|m| m.id == Id::ship("MAIN", "ARGOS", name))
            .cloned()
            .unwrap_or_else(|| panic!("no ship {name}"))
    }
    fn session_file(&self, short: &str) -> std::path::PathBuf {
        self.base.join("claude_home/sessions").join(format!("{short}.json"))
    }
    fn job_file(&self, short: &str) -> std::path::PathBuf {
        self.base.join("claude_home/jobs").join(short).join("state.json")
    }
    fn patch(&self, p: &std::path::Path, f: impl FnOnce(&mut Value)) {
        let mut v: Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
        f(&mut v);
        std::fs::write(p, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    }
}

#[test]
fn status_says_why_it_is_what_it_is() {
    let w = world();
    let snap = w.snap();

    // live and idle
    let clean = w.ship(&snap, "CLEAN");
    assert_eq!(clean.status, Status::Idle);
    assert!(clean.reasons[0].contains("status idle"), "{:?}", clean.reasons);

    // live and busy (the second TWIN row)
    let twin = w.ship(&snap, "TWIN");
    assert!(twin.session.is_some() && !twin.extra_rows.is_empty());

    // stopped: the job row with no live pid
    let stopped = w.ship(&snap, "STOPPED");
    assert_eq!(stopped.status, Status::Stopped);
    assert!(stopped.reasons[0].contains("no live pid"), "{:?}", stopped.reasons);

    // never started
    let fresh = w.ship(&snap, "FRESH");
    assert_eq!(fresh.status, Status::NotStarted);
    assert_eq!(fresh.session, None);
    assert!(fresh.reasons[0].contains("no session"), "{:?}", fresh.reasons);

    // waiting at a prompt: NeedsYou, with what it waits for
    let short = clean.session.as_ref().unwrap().short_id.clone().unwrap();
    w.patch(&w.session_file(&short), |v| {
        v["status"] = json!("waiting");
        v["waitingFor"] = json!("permission prompt");
    });
    let m = w.ship(&w.snap(), "CLEAN");
    assert_eq!(m.status, Status::NeedsYou);
    assert!(m.reasons[0].contains("permission prompt"), "{:?}", m.reasons);
    assert_eq!(m.session.unwrap().waiting_for.as_deref(), Some("permission prompt"));

    // a question left in the job row: NeedsYou even while the session sits idle
    w.patch(&w.session_file(&short), |v| {
        v["status"] = json!("idle");
        v.as_object_mut().unwrap().remove("waitingFor");
    });
    w.patch(&w.job_file(&short), |v| v["needs"] = json!("Ship it, or hold for the regression run?"));
    let m = w.ship(&w.snap(), "CLEAN");
    assert_eq!(m.status, Status::NeedsYou);
    assert!(m.reasons[0].contains("regression run"), "{:?}", m.reasons);
}

#[test]
fn crew_and_tasks_come_from_the_live_session() {
    let w = world();
    let snap = w.snap();
    let admiral = &snap.fleets.iter().find(|f| f.id.fleet() == "MAIN").unwrap().admiral;
    let teammate = admiral.crew.iter().find(|c| c.teammate).expect("a teammate");
    assert_eq!(teammate.label, "planner");
    assert!(teammate.running, "it wrote seconds ago");
    let old = admiral.crew.iter().find(|c| !c.teammate).expect("a finished sub-agent");
    assert!(!old.running, "it last wrote an hour ago");
    assert_eq!(admiral.tasks.len(), 1);
    assert_eq!(admiral.tasks[0].status, "in_progress");
    // a stopped member reports no crew
    assert!(w.ship(&snap, "STOPPED").crew.is_empty());
}

#[test]
fn stowaways_are_only_unlabelled_sessions_in_fleet_scope() {
    let w = world();
    let snap = w.snap();
    assert!(snap.stowaways.iter().any(|s| s.name == "quick look at the logs"));
    assert!(snap.stowaways.iter().all(|s| s.labels.is_empty()));

    // the same session outside every fleet folder is not a stowaway
    let stow = snap.stowaways.iter().find(|s| s.name == "quick look at the logs").unwrap().clone();
    let elsewhere = w.base.join("dirs/not-a-fleet");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let short = stow.short_id.clone().unwrap();
    for p in [w.session_file(&short), w.job_file(&short)] {
        w.patch(&p, |v| v["cwd"] = json!(elsewhere.to_string_lossy()));
    }
    let snap = w.snap();
    assert!(!snap.stowaways.iter().any(|s| s.session_id == stow.session_id));
}

#[test]
fn a_labelled_session_of_an_unknown_identity_is_reported_not_hidden() {
    let w = world();
    // a ship that the config no longer declares, still running with its labels
    let ghost = Id::ship("MAIN", "ARGOS", "RETIRED");
    let labels = nimitz_core::claude::labels_for(&w.env, &ghost);
    let line = json!({"ts": evidence::now(), "event": "SessionStart", "session_id": "ghost-1", "labels": labels});
    let p = w.base.join("state/events.jsonl");
    let mut raw = std::fs::read_to_string(&p).unwrap();
    raw.push_str(&format!("{line}\n"));
    std::fs::write(&p, raw).unwrap();
    std::fs::write(
        w.job_file("ghostjob").parent().map(|d| {
            std::fs::create_dir_all(d).unwrap();
            d.join("state.json")
        }).unwrap(),
        serde_json::to_vec(&json!({"sessionId": "ghost-1", "name": ghost.name(), "cwd": "/x", "state": "done"})).unwrap(),
    )
    .unwrap();

    let snap = w.snap();
    let i = snap.issues.iter().find(|i| i.message.contains("ghost-1")).expect("an issue about the ghost");
    assert_eq!(i.code, "config.invalid");
    assert_eq!(i.id.as_ref(), Some(&ghost));
    assert!(!snap.stowaways.iter().any(|s| s.session_id == "ghost-1"), "it has labels, so it is not a stowaway");
}

#[test]
fn the_readers_tolerate_whatever_claude_code_writes() {
    let w = world();
    let snap = w.snap();
    let short = w.ship(&snap, "CLEAN").session.unwrap().short_id.unwrap();

    // unknown fields, and fields of a type we do not expect
    let f = w.session_file(&short);
    w.patch(&f, |v| {
        v["scratchpad_dir"] = json!("/tmp/scratch");
        v["peerFeatures"] = json!(["a", "b"]);
    });
    // then the tail of an older, longer version of the file behind the new object
    let mut raw = std::fs::read(&f).unwrap();
    raw.extend_from_slice(br#"{"pid": 999}"#);
    std::fs::write(&f, raw).unwrap();
    w.patch(&w.job_file(&short), |v| {
        v["somethingNew"] = json!({"nested": true});
        v["state"] = json!(7); // a number where a string belongs
    });
    // a session file with no pid at all, and one that is not JSON
    std::fs::write(w.session_file("nopid"), br#"{"sessionId": "x"}"#).unwrap();
    std::fs::write(w.session_file("junk"), b"<html>not json</html>").unwrap();
    // junk and half-written lines in events.jsonl
    let p = w.base.join("state/events.jsonl");
    let mut raw = std::fs::read_to_string(&p).unwrap();
    raw.push_str("garbage\n{\"event\": 3}\n{\"event\": \"Stop\", \"session_id\": \"half\"");
    std::fs::write(&p, raw).unwrap();

    let snap = w.snap();
    let m = w.ship(&snap, "CLEAN");
    assert_eq!(m.status, Status::Idle, "the live session is still read: {:?}", m.reasons);
    assert_eq!(m.session.as_ref().unwrap().state, None);
    assert!(nimitz_core::sessions::session_issues(&w.env, &m).is_empty());
    // and the whole check still runs
    let _ = nimitz_core::check::check(&w.env, &snap, None);
}

#[test]
fn a_refresh_is_cheap_and_picks_up_new_events() {
    let w = world();
    let mut cache = snapshot::Cache::default();
    let first = std::time::Instant::now();
    let snap = snapshot::snapshot(&w.env, &mut cache);
    let cold = first.elapsed();
    assert!(!snap.fleets.is_empty());

    let warm_start = std::time::Instant::now();
    for _ in 0..5 {
        snapshot::snapshot(&w.env, &mut cache);
    }
    let warm = warm_start.elapsed() / 5;
    assert!(warm.as_millis() < 100, "a cached refresh must fit a 2 s loop: {warm:?} (cold {cold:?})");

    // a new event line is seen by the next refresh (the tail is incremental)
    let ghost = Id::ship("MAIN", "ARGOS", "FRESH");
    let labels = nimitz_core::claude::labels_for(&w.env, &ghost);
    let line = json!({"ts": evidence::now(), "event": "SessionStart", "session_id": "new-1", "labels": labels});
    let p = w.base.join("state/events.jsonl");
    let mut raw = std::fs::read_to_string(&p).unwrap();
    raw.push_str(&format!("{line}\n"));
    std::fs::write(&p, raw).unwrap();
    let before = evidence::events(&w.env).len();
    assert!(evidence::events(&w.env).iter().any(|e| e.session_id.as_deref() == Some("new-1")));
    assert_eq!(evidence::events(&w.env).len(), before, "reading twice does not duplicate");
}

#[test]
fn extra_rows_hold_the_copies_and_the_current_row_stays_the_primary() {
    let w = world();
    let snap = w.snap();
    let m = w.ship(&snap, "COPIES");
    let reg = nimitz_core::registry::load(&w.env);
    let current = reg.identities[&m.id.0].current.clone().unwrap();
    assert_eq!(m.session.as_ref().unwrap().session_id, current, "the registry decides which row is the primary");
    assert_eq!(m.extra_rows.len(), 2);
    assert!(m.extra_rows.iter().all(|r| r.session_id != current));
}

#[test]
fn a_fleet_that_does_not_run_here_is_listed_without_folders() {
    let w = world();
    // point MAIN's machine table at another host
    let p = w.env.fleet_dir("MAIN").join("fleet.toml");
    let raw = std::fs::read_to_string(&p).unwrap();
    std::fs::write(&p, raw.replace(sandbox::HOST, "some-other-box")).unwrap();
    let snap = w.snap();
    let main = snap.fleets.iter().find(|f| f.id.fleet() == "MAIN").unwrap();
    assert_eq!(main.machine, None);
    let ship = main.squadrons[0].ships.iter().find(|m| m.id.0.ends_with("/FRESH")).unwrap();
    assert_eq!(ship.cwd, None, "a ship with no folder here does not run here");
    // and `up` refuses for a fleet that is not on this machine
    let plan = nimitz_core::sessions::plan_up(&w.env, &snap, "MAIN", &Caller::User);
    assert!(plan.refusals.iter().any(|i| i.message.contains("does not run on this machine")), "{:?}", plan.refusals);
}

#[test]
fn a_status_nobody_knows_is_unknown_and_a_stop_says_why_it_ended() {
    let w = world();
    let clean = w.ship(&w.snap(), "CLEAN");
    let short = clean.session.as_ref().unwrap().short_id.clone().unwrap();
    // a status value from a newer Claude Code: Unknown, never a guess
    w.patch(&w.session_file(&short), |v| v["status"] = json!("compacting"));
    let m = w.ship(&w.snap(), "CLEAN");
    assert_eq!(m.status, Status::Unknown);
    assert!(m.reasons[0].contains("compacting"), "{:?}", m.reasons);

    // a stop through the (fake) native binary: the SessionEnd reason shows up
    let (rc, out) = nimitz_core::claude::run(&nimitz_core::claude::stop(&w.env, &short), None);
    assert_eq!(rc, 0, "{out}");
    let m = w.ship(&w.snap(), "CLEAN");
    assert_eq!(m.status, Status::Stopped);
    assert!(m.reasons.iter().any(|r| r.contains("SessionEnd") && r.contains("other")), "{:?}", m.reasons);
}

#[test]
fn every_member_carries_its_session_issues() {
    let w = world();
    let snap = w.snap();
    for (ship, code) in [("DRIFT", "name.drift"), ("NOLABEL", "labels.missing"), ("TWIN", "dup.live"), ("MISLABEL", "labels.mismatch"), ("COPIES", "copy.rows"), ("TAKEN", "name.taken"), ("BROKEN", "lineage.broken")] {
        let m = w.ship(&snap, ship);
        assert!(m.issues.iter().any(|i| i.code == code), "{ship}: {:?}", m.issues);
    }
    assert!(w.ship(&snap, "CLEAN").issues.is_empty());
    let boreal = &snap.fleets.iter().find(|f| f.id.fleet() == "MAIN").unwrap().squadrons.iter().find(|s| s.id.squadron() == Some("BOREAL")).unwrap().commodore;
    assert!(boreal.issues.iter().any(|i| i.code == "role.background"), "{:?}", boreal.issues);
}

#[test]
fn check_reports_each_session_issue_once() {
    let w = world();
    let snap = w.snap();
    let issues = nimitz_core::check::check(&w.env, &snap, None);
    let drift = Id::ship("MAIN", "ARGOS", "DRIFT");
    let n = issues.iter().filter(|i| i.code == "name.drift" && i.id.as_ref() == Some(&drift)).count();
    assert_eq!(n, 1, "{issues:?}");
}

#[test]
fn the_machine_label_survives_removing_every_fleet() {
    let w = world();
    nimitz_core::registry::update(&w.env, |r| r.machine = "west-mac".into()).unwrap();
    for e in std::fs::read_dir(&w.env.home).unwrap().flatten() {
        if e.path().join("fleet.toml").is_file() {
            std::fs::remove_dir_all(e.path()).unwrap();
        }
    }
    let snap = w.snap();
    assert!(snap.fleets.is_empty());
    assert_eq!(snap.machine.as_deref(), Some("west-mac"));
}

// ------------------------------------------------------------------ 0.4.0 status words (plan-v4 section 4)

#[test]
fn status_shell_is_busy_and_every_job_state_reads() {
    let w = world();
    let clean = w.ship(&w.snap(), "CLEAN");
    let short = clean.session.as_ref().unwrap().short_id.clone().unwrap();
    // `shell`: an idle turn with background work still running
    w.patch(&w.session_file(&short), |v| v["status"] = json!("shell"));
    let m = w.ship(&w.snap(), "CLEAN");
    assert_eq!(m.status, Status::Busy);
    assert!(m.reasons[0].contains("shell"), "{:?}", m.reasons);
    // every job state Claude Code writes is read as it is: a live row keeps its
    // session-file word, a row with no live pid is stopped with the state named
    const STATES: [&str; 10] = ["starting", "running", "resuming", "adopted", "crashed", "working", "blocked", "done", "stopped", "failed"];
    w.patch(&w.session_file(&short), |v| v["status"] = json!("idle"));
    for state in STATES {
        w.patch(&w.job_file(&short), |v| v["state"] = json!(state));
        let m = w.ship(&w.snap(), "CLEAN");
        assert_eq!(m.status, Status::Idle, "live with state {state}: {:?}", m.reasons);
        assert!(!m.issues.iter().any(|i| i.code.starts_with("job.")), "state {state}: {:?}", m.issues);
    }
    let stopped = w.ship(&w.snap(), "STOPPED");
    let short = stopped.session.as_ref().unwrap().short_id.clone().unwrap();
    for state in STATES {
        w.patch(&w.job_file(&short), |v| v["state"] = json!(state));
        let m = w.ship(&w.snap(), "STOPPED");
        assert_eq!(m.status, Status::Stopped, "no pid, state {state}: {:?}", m.reasons);
        assert!(m.reasons[0].contains(&format!("state {state}")), "{:?}", m.reasons);
    }
    // a state from a newer Claude Code degrades the same way: a word, never an error
    w.patch(&w.job_file(&short), |v| v["state"] = json!("hibernating"));
    let m = w.ship(&w.snap(), "STOPPED");
    assert_eq!(m.status, Status::Stopped);
    assert!(!m.issues.iter().any(|i| i.code.starts_with("job.")), "{:?}", m.issues);
}

#[test]
fn daemon_session_files_are_neither_members_nor_stowaways() {
    let w = world();
    let clean = w.ship(&w.snap(), "CLEAN");
    let short = clean.session.as_ref().unwrap().short_id.clone().unwrap();
    let cwd = clean.cwd.clone().unwrap();
    // Claude Code's own processes keep session files too: in a member's folder,
    // even with the member's own session id, they are not rows
    for (kind, name) in [("daemon", "d1"), ("daemon-worker", "d2")] {
        std::fs::write(
            w.base.join("claude_home/sessions").join(format!("{name}.json")),
            json!({"pid": sandbox::LIVE_PID, "sessionId": format!("{name}-0000-4000-8000-000000000000"), "cwd": cwd, "kind": kind,
                   "name": "", "status": "idle", "startedAt": (evidence::now() * 1000.0) as i64}).to_string(),
        )
        .unwrap();
    }
    let snap = w.snap();
    let m = w.ship(&snap, "CLEAN");
    assert_eq!(m.status, Status::Idle);
    assert!(m.extra_rows.is_empty(), "{:?}", m.extra_rows);
    assert!(!m.issues.iter().any(|i| i.code == "dup.live"), "{:?}", m.issues);
    assert!(!snap.stowaways.iter().any(|s| s.session_id.starts_with("d1") || s.session_id.starts_with("d2")), "{:?}", snap.stowaways);
    let rows = evidence::rows_from_files(&w.env, &mut evidence::FileCache::default()).unwrap();
    assert!(rows.iter().all(|r| r.kind == "background" || r.kind == "interactive"), "{:?}", rows.iter().map(|r| &r.kind).collect::<Vec<_>>());
    assert!(rows.iter().any(|r| r.short_id.as_deref() == Some(short.as_str())));
    // the CLI cross-check drops them too
    let v: Value = json!([{"sessionId": "x", "kind": "daemon-worker", "pid": 1}, {"sessionId": "y", "kind": "bg", "pid": 1}]);
    let rows = evidence::parse_cli_rows(&v);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session_id, "y");
}

#[test]
fn a_live_session_with_remote_control_is_rc_exposed() {
    let w = world();
    let clean = w.ship(&w.snap(), "CLEAN");
    let short = clean.session.as_ref().unwrap().short_id.clone().unwrap();
    assert!(!clean.issues.iter().any(|i| i.code == "rc.exposed"), "bridgeSessionId null: nothing");
    w.patch(&w.session_file(&short), |v| v["bridgeSessionId"] = json!("session_01EXAMPLE0000000000000000"));
    let m = w.ship(&w.snap(), "CLEAN");
    let i = m.issues.iter().find(|i| i.code == "rc.exposed").expect("rc.exposed");
    assert_eq!(i.severity, Severity::Warn);
    assert!(i.message.contains(&short) && i.message.contains("session_01EXAMPLE0000000000000000"), "{}", i.message);
    let fix = i.fix.as_deref().unwrap();
    assert!(fix.contains("/remote-control") && fix.contains("claude.ai/code"), "{fix}");
    assert_eq!(i.id, Some(m.id.clone()));
    // a warning: the check lists it, a start of the (live, clean) ship is not refused by it
    let snap = w.snap();
    assert!(nimitz_core::check::check(&w.env, &snap, Some(&m.id)).iter().any(|i| i.code == "rc.exposed"));
    let plan = nimitz_core::sessions::plan_start(&w.env, &snap, &m.id, &Caller::User, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert!(plan.warnings.iter().any(|i| i.code == "rc.exposed"));
    // the session disconnected (or stopped): the warning goes
    w.patch(&w.session_file(&short), |v| v["bridgeSessionId"] = json!(null));
    assert!(!w.ship(&w.snap(), "CLEAN").issues.iter().any(|i| i.code == "rc.exposed"));
}
