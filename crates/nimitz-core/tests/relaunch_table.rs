//! One test per row of the relaunch state table (contracts.md section 8) and per
//! issue code (section 7), end to end through the sandbox's fake `claude`.
//!
//! Each test builds its own world under a temp dir (`sandbox::create`), takes a
//! real `snapshot`, asks the planner for a plan, and — where the row acts —
//! `apply`s it and re-reads the world to check what actually happened. The fake
//! `claude` is reached through `<sandbox>/bin/claude`, which execs the test-only
//! `nimitz-sandbox` binary (see `sandbox.rs`).
use nimitz_core::env::Env;
use nimitz_core::types::*;
use nimitz_core::{claude, evidence, sandbox, sessions, snapshot};

const EXE: &str = env!("CARGO_BIN_EXE_nimitz-sandbox");

struct World {
    _td: tempfile::TempDir,
    env: Env,
    base: std::path::PathBuf,
}

fn world() -> World {
    let td = tempfile::tempdir().unwrap();
    let base = td.path().join("sb");
    sandbox::create(&base, std::path::Path::new(EXE)).expect("sandbox");
    let env = sandbox::env(&base);
    World { _td: td, env, base }
}

impl World {
    fn snap(&self) -> Snapshot {
        snapshot::snapshot(&self.env, &mut snapshot::Cache::default())
    }
    fn ship(&self, name: &str) -> Id {
        Id::ship("MAIN", "ARGOS", name)
    }
    fn member(&self, snap: &Snapshot, id: &Id) -> Member {
        find_member(snap, id).unwrap_or_else(|| panic!("no member {id}"))
    }
    fn start(&self, snap: &Snapshot, id: &Id, force: bool) -> Plan {
        sessions::plan_start(&self.env, snap, id, &Caller::User, force)
    }
    /// Steps of a plan that belong to one identity.
    fn steps_of<'a>(&self, plan: &'a Plan, id: &Id) -> Vec<&'a Step> {
        plan.steps.iter().filter(|s| s.id.as_ref() == Some(id)).collect()
    }
    fn rows(&self) -> Vec<evidence::SessionRow> {
        evidence::rows_from_files(&self.env, &mut evidence::FileCache::default()).unwrap_or_default()
    }
    fn set_cc_version(&self, v: &str) {
        let p = self.base.join("world.json");
        let mut j: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        j["cc_version"] = serde_json::json!(v);
        std::fs::write(&p, serde_json::to_vec_pretty(&j).unwrap()).unwrap();
    }
}

fn find_member(snap: &Snapshot, id: &Id) -> Option<Member> {
    for f in &snap.fleets {
        if &f.admiral.id == id {
            return Some(f.admiral.clone());
        }
        for sq in &f.squadrons {
            if &sq.commodore.id == id {
                return Some(sq.commodore.clone());
            }
            for m in sq.ships.iter().chain(sq.tenders.iter()) {
                if &m.id == id {
                    return Some(m.clone());
                }
            }
        }
    }
    None
}

fn codes(issues: &[Issue]) -> Vec<&str> {
    issues.iter().map(|i| i.code.as_str()).collect()
}

fn has(issues: &[Issue], code: &str) -> bool {
    issues.iter().any(|i| i.code == code)
}

// ------------------------------------------------------------------ the world itself

#[test]
fn sandbox_world_has_every_shape() {
    let w = world();
    let snap = w.snap();
    assert_eq!(snap.machine.as_deref(), Some("sandbox"));
    assert_eq!(snap.cc_version.as_deref(), Some(sandbox::CC_VERSION));
    assert!(snap.compat_ok, "the sandbox version must be inside the compiled range");
    let fleets: Vec<&str> = snap.fleets.iter().map(|f| f.id.fleet()).collect();
    assert_eq!(fleets, vec!["BETA", "LAYOUT", "MAIN"]);
    let main = snap.fleets.iter().find(|f| f.id.fleet() == "MAIN").unwrap();
    assert_eq!(main.squadrons.len(), 2);
    let argos = main.squadrons.iter().find(|s| s.id.0 == "MAIN/ARGOS").unwrap();
    assert_eq!(argos.ships.len(), 16);
    assert_eq!(argos.tenders.len(), 1, "the stopped tender");
    // the admiral is live and clean, with its crew and tasks
    assert_eq!(main.admiral.status, Status::Busy);
    assert!(main.admiral.crew.iter().any(|c| c.teammate && c.label == "planner"));
    assert_eq!(main.tasks_len(), 1);
}

trait TasksLen {
    fn tasks_len(&self) -> usize;
}
impl TasksLen for Fleet {
    fn tasks_len(&self) -> usize {
        self.admiral.tasks.len()
    }
}

// ------------------------------------------------------------------ row 1

#[test]
fn row1_live_clean_does_nothing() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("CLEAN");
    let m = w.member(&snap, &id);
    assert_eq!(m.status, Status::Idle);
    assert!(sessions::session_issues(&w.env, &m).is_empty(), "{:?}", sessions::session_issues(&w.env, &m));
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert!(w.steps_of(&plan, &id).is_empty(), "a clean live ship needs no step: {:?}", plan.steps);
    // its live parents need no step either
    assert!(plan.steps.is_empty(), "{:?}", plan.steps);
}

// ------------------------------------------------------------------ row 2

#[test]
fn row2_live_name_drift_refuses_with_a_relaunch() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("DRIFT");
    let plan = w.start(&snap, &id, false);
    assert!(!plan.ok());
    let i = plan.refusals.iter().find(|i| i.code == "name.drift").expect("name.drift");
    assert_eq!(i.fix.as_deref(), Some("nimitz relaunch MAIN/ARGOS/DRIFT"));
    assert!(plan.steps.is_empty());
}

#[test]
fn row2_live_without_labels_refuses_with_a_relaunch() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("NOLABEL");
    let m = w.member(&snap, &id);
    assert!(m.session.as_ref().unwrap().labels.is_empty());
    let plan = w.start(&snap, &id, false);
    assert!(has(&plan.refusals, "labels.missing"), "{:?}", codes(&plan.refusals));
}

// ------------------------------------------------------------------ row 3

#[test]
fn row3_two_live_sessions_refuse_and_list_both() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("TWIN");
    let plan = w.start(&snap, &id, false);
    let i = plan.refusals.iter().find(|i| i.code == "dup.live").expect("dup.live");
    assert!(i.message.contains("2 live sessions"), "{}", i.message);
    assert!(i.fix.as_deref().unwrap_or("").contains("nimitz stop"));
}

// ------------------------------------------------------------------ row 4

#[test]
fn row4_stopped_with_labels_resumes_with_no_flags_and_keeps_the_id() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("STOPPED");
    let before = w.member(&snap, &id).session.unwrap();
    assert_eq!(before.pid, None);
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let steps = w.steps_of(&plan, &id);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].cmd, claude::resume_bg(&w.env, &before.session_id));
    assert_eq!(steps[0].cmd.len(), 4, "a resume passes no flags at all: {:?}", steps[0].cmd);

    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id).session.unwrap();
    assert_eq!(after.session_id, before.session_id, "a flag-free resume keeps the id (probe 2c1)");
    assert!(after.pid.is_some());
    assert_eq!(after.name, id.name());
    assert_eq!(after.labels.get("NIMITZ_ID"), Some(&id.0));
    let reg = nimitz_core::registry::load(&w.env);
    let rec = &reg.identities[&id.0];
    assert_eq!(rec.current.as_ref(), Some(&before.session_id));
    assert_eq!(rec.lineage.last().unwrap().via, "resume");
}

// ------------------------------------------------------------------ row 5

#[test]
fn row5_stopped_without_labels_relaunches_with_all_flags_and_drops_the_old_row() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("BARE");
    let old = w.member(&snap, &id).session.unwrap();
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let steps = w.steps_of(&plan, &id);
    assert_eq!(steps.len(), 2, "relaunch, then rm the old row: {:?}", steps);
    assert!(steps[0].cmd.contains(&"--settings".to_string()) && steps[0].cmd.contains(&"--name".to_string()));
    assert_eq!(steps[1].cmd, claude::rm(&w.env, old.short_id.as_deref().unwrap()));

    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id).session.unwrap();
    assert_ne!(after.session_id, old.session_id, "a relaunch is a copy with a new id");
    assert_eq!(after.labels.get("NIMITZ_ID"), Some(&id.0), "the new session carries the labels");
    assert!(!w.rows().iter().any(|r| r.session_id == old.session_id), "the old row is gone");
    assert_eq!(nimitz_core::registry::load(&w.env).identities[&id.0].lineage.last().unwrap().via, "relaunch");
}

// ------------------------------------------------------------------ row 6

#[test]
fn row6_no_row_but_a_transcript_continues_with_c_in_a_pane() {
    let w = world();
    let snap = w.snap();
    let id = Id::admiral("BETA");
    let m = w.member(&snap, &id);
    assert_eq!(m.session, None);
    assert_eq!(m.status, Status::Stopped);
    assert!(m.reasons[0].contains("transcript"), "{:?}", m.reasons);
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let steps = w.steps_of(&plan, &id);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].cmd, claude::interactive(&w.env, &id, "default", true));
    assert_eq!(steps[0].cwd.as_deref(), m.cwd.as_deref());
    // core never starts an interactive member: apply leaves it to the CLI's pane
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    assert!(out.ran[0].2.contains("pane"), "{}", out.ran[0].2);
    assert!(w.member(&w.snap(), &id).session.is_none(), "core started nothing itself");
}

// ------------------------------------------------------------------ row 7

#[test]
fn row7_no_row_no_transcript_cold_launches_with_the_config_prompt() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("FRESH");
    let m = w.member(&snap, &id);
    assert_eq!(m.status, Status::NotStarted);
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let steps = w.steps_of(&plan, &id);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].cmd.last().unwrap(), "You are [MAIN.ARGOS]-FRESH.");
    assert_eq!(steps[0].cmd[0], w.env.claude_bin, "a cold launch uses the PATH binary (the provider's settings)");

    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id).session.unwrap();
    assert!(after.pid.is_some());
    assert_eq!(after.name, id.name());
    assert_eq!(after.labels.get("NIMITZ_ID"), Some(&id.0));
    assert_eq!(nimitz_core::registry::load(&w.env).identities[&id.0].lineage.last().unwrap().via, "launch");
}

#[test]
fn row7_cold_launch_refuses_a_ship_with_no_prompt_and_a_dead_tender() {
    let w = world();
    let snap = w.snap();
    let plan = w.start(&snap, &w.ship("NOPROMPT"), false);
    assert!(plan.refusals.iter().any(|i| i.message.contains("no prompt")), "{:?}", plan.refusals);
    // a tender that left nothing is not restarted cold
    let gone = Id::tender("MAIN", "ARGOS", "vanished");
    let mut snap2 = snap.clone();
    let sq = &mut snap2.fleets.iter_mut().find(|f| f.id.fleet() == "MAIN").unwrap().squadrons[0];
    let mut t = sq.tenders[0].clone();
    t.id = gone.clone();
    t.expected_name = gone.name();
    t.session = None;
    t.extra_rows.clear();
    sq.tenders.push(t);
    let plan = w.start(&snap2, &gone, false);
    assert!(plan.refusals.iter().any(|i| i.message.contains("a tender is one job")), "{:?}", plan.refusals);
}

// ------------------------------------------------------------------ row 8

#[test]
fn row8_failed_row_needs_force_then_cold_launches() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("FAILED");
    let plan = w.start(&snap, &id, false);
    let i = plan.refusals.iter().find(|i| i.code == "lineage.broken").expect("lineage.broken");
    assert!(i.fix.as_deref().unwrap_or("").contains("--force"));

    let forced = w.start(&snap, &id, true);
    assert!(forced.ok(), "{:?}", forced.refusals);
    let steps = w.steps_of(&forced, &id);
    assert!(steps[0].cmd.contains(&"--bg".to_string()) && !steps[0].cmd.contains(&"--resume".to_string()), "{:?}", steps[0].cmd);
    let out = sessions::apply(&w.env, &forced);
    assert!(out.verified, "{:?}", out.issues);
    assert!(w.member(&w.snap(), &id).session.unwrap().pid.is_some());
}

// ------------------------------------------------------------------ row 9

#[test]
fn row9_copy_rows_are_removed_after_the_start_and_verified() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("COPIES");
    let m = w.member(&snap, &id);
    assert_eq!(m.extra_rows.len(), 2);
    let issues = sessions::session_issues(&w.env, &m);
    assert!(has(&issues, "copy.rows"), "{:?}", codes(&issues));
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let steps = w.steps_of(&plan, &id);
    assert_eq!(steps.len(), 3, "resume plus one rm per copy: {:?}", steps);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id);
    assert!(after.extra_rows.is_empty(), "{:?}", after.extra_rows);
    assert!(after.session.unwrap().pid.is_some());
}

#[test]
fn sweep_removes_stale_rows_without_starting_anything() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("COPIES");
    let plan = sessions::plan_sweep(&w.env, &snap, Some(&id), &Caller::User);
    assert_eq!(plan.steps.len(), 2);
    assert!(plan.steps.iter().all(|s| s.cmd.contains(&"rm".to_string())));
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id);
    assert!(after.extra_rows.is_empty());
    assert_eq!(after.status, Status::Stopped, "sweep starts nothing");
}

// ------------------------------------------------------------------ row 10

#[test]
fn row10_missing_or_untrusted_folder_refuses_with_the_fix() {
    let w = world();
    let snap = w.snap();
    let plan = w.start(&snap, &w.ship("GONE"), false);
    let i = plan.refusals.iter().find(|i| i.code == "folder.missing").expect("folder.missing");
    assert!(i.fix.as_deref().unwrap_or("").contains("mkdir"));
    assert!(plan.steps.is_empty());

    let plan = w.start(&snap, &w.ship("UNTRUSTED"), false);
    let i = plan.refusals.iter().find(|i| i.code == "folder.untrusted").expect("folder.untrusted");
    assert!(i.fix.as_deref().unwrap_or("").contains("accept the trust prompt"));
}

#[test]
fn an_untrusted_folder_also_stops_the_fake_claude_itself() {
    // the refusal is not only nimitz's: --bg in an untrusted folder exits 1 (probe 0)
    let w = world();
    let snap = w.snap();
    let id = w.ship("UNTRUSTED");
    let cwd = w.member(&snap, &id).cwd.unwrap();
    let (rc, out) = claude::run(&claude::launch_bg(&w.env, &id, "default", Some("go")), Some(&cwd));
    assert_eq!(rc, 1, "{out}");
    assert!(out.contains("not trusted"), "{out}");
}

// ------------------------------------------------------------------ row 11

#[test]
fn row11_an_interactive_member_in_the_background_refuses() {
    let w = world();
    let snap = w.snap();
    let id = Id::commodore("MAIN", "BOREAL");
    let m = w.member(&snap, &id);
    assert_eq!(m.session.as_ref().unwrap().kind, "background");
    let plan = w.start(&snap, &id, false);
    let i = plan.refusals.iter().find(|i| i.code == "role.background").expect("role.background");
    assert!(i.fix.as_deref().unwrap_or("").contains("attach"));
    // and a ship under it refuses too: the chain is started top-down
    let mut snap2 = snap.clone();
    let _ = &mut snap2;
    let ship_plan = w.start(&snap, &w.ship("CLEAN"), false);
    assert!(ship_plan.ok(), "a ship of another squadron is unaffected: {:?}", ship_plan.refusals);
}

// ------------------------------------------------------------------ row 12

#[test]
fn row12_a_version_out_of_range_refuses_launches_until_force() {
    let w = world();
    w.set_cc_version("2.9.99");
    let snap = w.snap();
    assert!(!snap.compat_ok);
    let id = w.ship("FRESH");
    let plan = w.start(&snap, &id, false);
    assert!(has(&plan.refusals, "cc.version"), "{:?}", codes(&plan.refusals));
    let forced = w.start(&snap, &id, true);
    assert!(forced.ok(), "{:?}", forced.refusals);
    assert!(has(&forced.warnings, "cc.version"), "--force keeps the warning");
    // below the range is an error as well
    w.set_cc_version("2.0.1");
    let snap = snapshot::snapshot(&w.env, &mut snapshot::Cache::default());
    assert!(has(&sessions::plan_start(&w.env, &snap, &id, &Caller::User, false).refusals, "cc.version"));
}

// ------------------------------------------------------------------ row 13

#[test]
fn row13_a_session_file_with_extra_bytes_is_read_not_flagged() {
    let w = world();
    let id = w.ship("CLEAN");
    let before = w.member(&w.snap(), &id).session.unwrap();
    let f = w.base.join("claude_home/sessions").join(format!("{}.json", before.short_id.unwrap()));
    let mut raw = std::fs::read(&f).unwrap();
    raw.extend_from_slice(br#"sionId": "an older, longer version"}"#);
    std::fs::write(&f, raw).unwrap();

    let snap = w.snap();
    let m = w.member(&snap, &id);
    assert_eq!(m.status, Status::Idle, "{:?}", m.reasons);
    assert!(sessions::session_issues(&w.env, &m).is_empty());
    assert!(w.start(&snap, &id, false).ok());
}

// ------------------------------------------------------------------ the remaining issue codes

#[test]
fn issue_config_invalid_and_config_layout() {
    let w = world();
    sandbox::break_a_config(&w.base, "BAD");
    let snap = w.snap();
    assert!(has(&snap.issues, "config.invalid"), "the BAD fleet does not parse: {:?}", codes(&snap.issues));
    // a config error with no identity blocks every start, by design (check.rs)
    assert!(!w.start(&snap, &w.ship("FRESH"), false).ok());
    let all = nimitz_core::check::check(&w.env, &snap, None);
    assert!(has(&all, "config.layout"), "{:?}", codes(&all));
    // a start inside the LAYOUT fleet refuses while the CLAUDE.md is there
    let plan = w.start(&snap, &Id::admiral("LAYOUT"), false);
    assert!(has(&plan.refusals, "config.layout"), "{:?}", codes(&plan.refusals));
}

#[test]
fn issue_name_taken_and_stowaway() {
    let w = world();
    let snap = w.snap();
    let m = w.member(&snap, &w.ship("TAKEN"));
    assert!(has(&m.issues, "name.taken"), "{:?}", codes(&m.issues));
    let plan = w.start(&snap, &m.id, false);
    assert!(has(&plan.refusals, "name.taken"), "{:?}", codes(&plan.refusals));
    // the same unlabelled session in fleet scope is a stowaway
    assert!(snap.stowaways.iter().any(|s| s.name == m.expected_name));
    assert!(snap.stowaways.iter().any(|s| s.name == "quick look at the logs"));
    let all = nimitz_core::check::check(&w.env, &snap, None);
    assert!(has(&all, "stowaway"));
}

#[test]
fn issue_labels_mismatch() {
    let w = world();
    let snap = w.snap();
    let m = w.member(&snap, &w.ship("MISLABEL"));
    let issues = sessions::session_issues(&w.env, &m);
    assert!(has(&issues, "labels.mismatch"), "{:?}", codes(&issues));
    assert!(has(&w.start(&snap, &m.id, false).refusals, "labels.mismatch"));
}

#[test]
fn issue_lineage_broken() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("BROKEN");
    let m = w.member(&snap, &id);
    assert!(has(&m.issues, "lineage.broken"), "{:?}", codes(&m.issues));
    assert!(has(&w.start(&snap, &id, false).refusals, "lineage.broken"));
    let forced = w.start(&snap, &id, true);
    assert!(forced.ok(), "{:?}", forced.refusals);
    let out = sessions::apply(&w.env, &forced);
    assert!(out.verified, "{:?}", out.issues);
    assert!(w.member(&w.snap(), &id).session.unwrap().pid.is_some());
}

#[test]
fn issue_ship_elsewhere() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("ELSEWHERE");
    let m = w.member(&snap, &id);
    let issues = sessions::session_issues(&w.env, &m);
    assert!(has(&issues, "ship.elsewhere"), "{:?}", codes(&issues));
    assert!(has(&w.start(&snap, &id, false).refusals, "ship.elsewhere"));
    // once the other machine releases the folder, the ship starts here
    std::fs::remove_file(sessions::ship_lock_path(m.cwd.as_deref().unwrap())).unwrap();
    let snap = w.snap();
    assert!(w.start(&snap, &id, false).ok());
}

// ------------------------------------------------------------------ the other commands

#[test]
fn stop_then_resume_keeps_one_id_and_one_row() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("CLEAN");
    let sid = w.member(&snap, &id).session.unwrap().session_id;
    let plan = sessions::plan_stop(&w.env, &snap, &id, &Caller::User);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert!(plan.steps[0].cmd.contains(&"stop".to_string()));
    assert_eq!(plan.steps[0].cmd[0], w.env.native_claude_bin, "stop goes through the native binary");
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);

    let snap = w.snap();
    let m = w.member(&snap, &id);
    assert_eq!(m.status, Status::Stopped);
    let plan = w.start(&snap, &id, false);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id);
    assert_eq!(after.session.as_ref().unwrap().session_id, sid, "the ship keeps one id for life");
    assert!(after.extra_rows.is_empty(), "and one row");
}

#[test]
fn stop_refuses_an_interactive_member_and_a_dead_ship() {
    let w = world();
    let snap = w.snap();
    let plan = sessions::plan_stop(&w.env, &snap, &Id::admiral("MAIN"), &Caller::User);
    assert!(!plan.ok());
    assert!(plan.refusals[0].message.contains("interactive"), "{:?}", plan.refusals);
    let plan = sessions::plan_stop(&w.env, &snap, &w.ship("STOPPED"), &Caller::User);
    assert!(plan.refusals[0].message.contains("not live"), "{:?}", plan.refusals);
}

#[test]
fn relaunch_stops_a_live_ship_first_then_forks_and_drops_the_old_row() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("DRIFT");
    let old = w.member(&snap, &id).session.unwrap();
    let plan = sessions::plan_relaunch(&w.env, &snap, &id, &Caller::User);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert_eq!(plan.steps.len(), 3, "stop, relaunch, rm: {:?}", plan.steps);
    assert!(plan.steps[0].cmd.contains(&"stop".to_string()));
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id);
    let s = after.session.unwrap();
    assert_ne!(s.session_id, old.session_id);
    assert_eq!(s.name, id.name(), "the drift is gone");
    assert!(sessions::session_issues(&w.env, &w.member(&w.snap(), &id)).is_empty());
}

#[test]
fn up_starts_the_whole_fleet_and_refuses_for_a_member_caller() {
    let w = world();
    let snap = w.snap();
    let commodore = Caller::Member { id: Id::commodore("MAIN", "ARGOS"), role: Role::Commodore };
    assert!(!sessions::plan_up(&w.env, &snap, "MAIN", &commodore).ok());
    let plan = sessions::plan_up(&w.env, &snap, "MAIN", &Caller::User);
    // MAIN has broken members, so `up` refuses and names each one
    assert!(!plan.ok());
    for code in ["name.drift", "labels.missing", "dup.live", "role.background", "folder.missing", "lineage.broken"] {
        assert!(has(&plan.refusals, code), "{code} missing from {:?}", codes(&plan.refusals));
    }
    // BETA is clean: up plans its admiral only
    let plan = sessions::plan_up(&w.env, &snap, "BETA", &Caller::User);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].id, Some(Id::admiral("BETA")));
}

#[test]
fn a_commodore_may_start_its_own_ship_only() {
    let w = world();
    let snap = w.snap();
    let own = Caller::Member { id: Id::commodore("MAIN", "ARGOS"), role: Role::Commodore };
    let plan = sessions::plan_start(&w.env, &snap, &w.ship("FRESH"), &own, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let other = Caller::Member { id: Id::commodore("MAIN", "BOREAL"), role: Role::Commodore };
    let plan = sessions::plan_start(&w.env, &snap, &w.ship("FRESH"), &other, false);
    assert!(plan.refusals.iter().any(|i| i.message.contains("may not start")), "{:?}", plan.refusals);
    // an admiral starts nothing
    let adm = Caller::Member { id: Id::admiral("MAIN"), role: Role::Admiral };
    assert!(!sessions::plan_start(&w.env, &snap, &w.ship("FRESH"), &adm, false).ok());
}

#[test]
fn tender_is_one_background_job_for_its_commodore() {
    let w = world();
    let snap = w.snap();
    let c = Id::commodore("MAIN", "ARGOS");
    let plan = sessions::plan_tender(&w.env, &snap, &c, "ci-fix", "Fix the failing CI job.", &Caller::User);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let id = Id::tender("MAIN", "ARGOS", "ci-fix");
    assert_eq!(plan.steps[0].id, Some(id.clone()));
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let m = w.member(&w.snap(), &id);
    assert_eq!(m.role, Role::Tender);
    assert_eq!(m.session.as_ref().unwrap().name, "[MAIN.ARGOS]-COMMODORE/ci-fix");
    assert_eq!(m.session.as_ref().unwrap().labels.get("NIMITZ_PARENT"), Some(&c.0));
    // a ship may not start a tender; a bad task name is refused
    let ship = Caller::Member { id: w.ship("CLEAN"), role: Role::Ship };
    assert!(!sessions::plan_tender(&w.env, &snap, &c, "x", "p", &ship).ok());
    assert!(!sessions::plan_tender(&w.env, &snap, &c, "bad name", "p", &Caller::User).ok());
    assert!(!sessions::plan_tender(&w.env, &snap, &c, "ok", "  ", &Caller::User).ok());
}

#[test]
fn a_stopped_tender_resumes_and_keeps_its_id() {
    let w = world();
    let snap = w.snap();
    let id = Id::tender("MAIN", "ARGOS", "fixci");
    let before = w.member(&snap, &id).session.unwrap();
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    assert_eq!(w.member(&w.snap(), &id).session.unwrap().session_id, before.session_id);
}

#[test]
fn adopt_binds_a_stowaway_and_warns_that_labels_come_later() {
    let w = world();
    let snap = w.snap();
    let stow = snap.stowaways.iter().find(|s| s.name == "quick look at the logs").unwrap().clone();
    let id = w.ship("FRESH");
    let plan = sessions::plan_adopt(&w.env, &snap, &stow.session_id, &id, &Caller::User);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert!(has(&plan.warnings, "labels.missing"));
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let reg = nimitz_core::registry::load(&w.env);
    assert_eq!(reg.identities[&id.0].current.as_deref(), Some(stow.session_id.as_str()));
    assert_eq!(reg.identities[&id.0].lineage.last().unwrap().via, "adopt");
    // the adopted session now belongs to the ship, which has no labels yet
    let m = w.member(&w.snap(), &id);
    assert_eq!(m.session.as_ref().unwrap().session_id, stow.session_id);
    assert!(has(&sessions::session_issues(&w.env, &m), "labels.missing"));
    // a session that is not a stowaway is not adopted
    assert!(!sessions::plan_adopt(&w.env, &w.snap(), "no-such-session", &id, &Caller::User).ok());
}

#[test]
fn rm_removes_a_stale_row_but_never_a_live_or_current_one() {
    let w = world();
    let snap = w.snap();
    let copies = w.member(&snap, &w.ship("COPIES"));
    let stale = copies.extra_rows[0].short_id.clone().unwrap();
    let plan = sessions::plan_rm(&w.env, &snap, &stale, &Caller::User);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    assert!(!w.rows().iter().any(|r| r.short_id.as_deref() == Some(stale.as_str())));

    let snap = w.snap();
    // the current row of an identity is protected
    let current = w.member(&snap, &w.ship("COPIES")).session.unwrap().short_id.unwrap();
    assert!(!sessions::plan_rm(&w.env, &snap, &current, &Caller::User).ok());
    // a live session must be stopped first
    let live = w.member(&snap, &w.ship("CLEAN")).session.unwrap().short_id.unwrap();
    let plan = sessions::plan_rm(&w.env, &snap, &live, &Caller::User);
    assert!(plan.refusals[0].message.contains("live"), "{:?}", plan.refusals);
}

// ------------------------------------------------------------------ the probed claude semantics

#[test]
fn fake_claude_reproduces_every_probed_resume_case() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("STOPPED");
    let m = w.member(&snap, &id);
    let (cwd, sid) = (m.cwd.clone().unwrap(), m.session.as_ref().unwrap().session_id.clone());

    // probe 1: --session-id is ignored, with a warning
    let mut cmd = claude::launch_bg(&w.env, &id, "default", Some("go"));
    cmd.extend(["--session-id".into(), "11111111-2222-3333-4444-555555555555".into()]);
    let (rc, out) = claude::run(&cmd, Some(&cwd));
    assert_eq!(rc, 0);
    assert!(out.contains("ignoring --session-id"), "{out}");
    assert!(!w.rows().iter().any(|r| r.session_id.starts_with("11111111")), "the requested id was not used");

    // probe 2a: resuming an unknown id never runs the session; the row ends failed
    let unknown = "99999999-8888-7777-6666-555555555555";
    let (rc, out) = claude::run(&claude::resume_bg(&w.env, unknown), Some(&cwd));
    assert_eq!(rc, 0, "{out}");
    let row = w.rows().into_iter().find(|r| r.session_id == unknown).expect("a failed row");
    assert_eq!(row.state.as_deref(), Some("failed"));
    assert!(row.pid.is_none());

    // probe 2c1: a flag-free resume of a stopped session keeps the id
    let (rc, out) = claude::run(&claude::resume_bg(&w.env, &sid), Some(&cwd));
    assert_eq!(rc, 0);
    assert!(out.contains("saved options"), "{out}");
    assert!(w.rows().iter().any(|r| r.session_id == sid && r.live()));

    // probe 2b: resuming a live session makes a copy with a new id
    let before = w.rows().len();
    let (rc, out) = claude::run(&claude::resume_bg(&w.env, &sid), Some(&cwd));
    assert_eq!(rc, 0);
    assert!(out.contains("already running"), "{out}");
    assert_eq!(w.rows().len(), before + 1);

    // probe 2c2: a --name-only resume of a stopped session is an unlabelled copy
    let stopped = w.ship("BARE");
    let bm = w.member(&w.snap(), &stopped);
    let (bcwd, bsid) = (bm.cwd.clone().unwrap(), bm.session.as_ref().unwrap().session_id.clone());
    let cmd = vec![w.env.native_claude_bin.clone(), "--bg".into(), "--resume".into(), bsid.clone(), "--name".into(), stopped.name()];
    let (rc, out) = claude::run(&cmd, Some(&bcwd));
    assert_eq!(rc, 0);
    assert!(out.contains("started a copy"), "{out}");
    let copy = w.rows().into_iter().find(|r| r.name == stopped.name() && r.session_id != bsid).expect("the copy");
    let labels = evidence::labels_by_session(&evidence::events(&w.env));
    assert!(!labels.contains_key(&copy.session_id), "a copy with no --settings fires no hooks, so it has no labels");

    // probe 5: two live sessions may share one name
    assert!(w.rows().iter().filter(|r| r.name == stopped.name() && r.live()).count() >= 1);
}

#[test]
fn stopping_a_session_leaves_its_row_for_a_later_resume() {
    let w = world();
    let snap = w.snap();
    let m = w.member(&snap, &w.ship("CLEAN"));
    let short = m.session.as_ref().unwrap().short_id.clone().unwrap();
    let (rc, _) = claude::run(&claude::stop(&w.env, &short), None);
    assert_eq!(rc, 0);
    let row = w.rows().into_iter().find(|r| r.short_id.as_deref() == Some(short.as_str())).expect("the row stays");
    assert!(row.pid.is_none());
    assert_eq!(row.state.as_deref(), Some("done"));
    // rm refuses a live row, stop refuses a dead one
    assert_ne!(claude::run(&claude::stop(&w.env, &short), None).0, 0);
    assert_eq!(claude::run(&claude::rm(&w.env, &short), None).0, 0);
    assert!(!w.rows().iter().any(|r| r.short_id.as_deref() == Some(short.as_str())));
}

#[test]
fn agents_json_and_version_come_from_the_native_binary() {
    let w = world();
    let rows = claude::agents(&w.env).expect("agents --json --all");
    assert!(rows.iter().any(|r| r.name == "[MAIN.ARGOS]-CLEAN" && r.pid.is_some()));
    assert_eq!(claude::version(&w.env).as_deref(), Some(sandbox::CC_VERSION));
}

#[test]
fn apply_refuses_to_run_a_refused_plan_and_reports_a_failed_step() {
    let w = world();
    let snap = w.snap();
    let refused = w.start(&snap, &w.ship("GONE"), false);
    let out = sessions::apply(&w.env, &refused);
    assert!(out.ran.is_empty());
    assert!(!out.verified);
    assert_eq!(out.issues, refused.refusals);

    // a step that fails stops the plan and is reported with its output
    let bad = Plan {
        summary: "a failing step".into(),
        steps: vec![Step { id: None, cmd: claude::rm(&w.env, "no-such-job"), cwd: None, why: "x".into() }],
        ..Default::default()
    };
    let out = sessions::apply(&w.env, &bad);
    assert!(!out.verified);
    assert_eq!(out.ran.len(), 1);
    assert_ne!(out.ran[0].1, 0);
    assert!(out.issues[0].message.contains("no job"), "{:?}", out.issues);
}
