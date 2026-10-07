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
    fn start_as(&self, snap: &Snapshot, id: &Id, caller: &Caller) -> Plan {
        sessions::plan_start(&self.env, snap, id, caller, false)
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
            for m in &sq.ships {
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
    assert_eq!(rec.session.as_ref(), Some(&before.session_id));
    assert_eq!(rec.lineage.last().unwrap().via, "resume");
}

#[test]
fn row4_a_live_copy_of_a_stopped_member_refuses_instead_of_making_two() {
    let w = world();
    let id = w.ship("STOPPED");
    let m = w.member(&w.snap(), &id);
    let sid = m.session.as_ref().unwrap().session_id.clone();
    // someone forked it by hand: a labelled copy, never bound in the registry
    let (rc, out) = claude::run(&claude::relaunch_bg(&w.env, &id, "default", &sid), m.cwd.as_deref());
    assert_eq!(rc, 0, "{out}");
    let snap = w.snap();
    let m = w.member(&snap, &id);
    assert_eq!(m.session.as_ref().unwrap().session_id, sid, "the registry's current stays the primary");
    assert!(m.extra_rows.iter().any(|r| r.pid.is_some()));
    let plan = w.start(&snap, &id, false);
    assert!(!plan.ok());
    assert!(has(&plan.refusals, "dup.live"), "{:?}", plan.refusals);
    assert!(plan.steps.iter().all(|s| s.id.as_ref() != Some(&id)), "nothing runs for it");
}

// ------------------------------------------------------------------ row 5

#[test]
fn row5_stopped_without_labels_removes_the_row_then_resumes_with_every_flag_keeping_id_and_history() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("BARE");
    let old = w.member(&snap, &id).session.unwrap();
    let history = std::fs::read_to_string(evidence::transcript(&w.env, &mut evidence::FileCache::default(), Some(&old.cwd), &old.session_id).unwrap()).unwrap();
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let steps = w.steps_of(&plan, &id);
    assert_eq!(steps.len(), 2, "rm the row, then resume with every flag (BUG-53): {:?}", steps);
    assert_eq!(steps[0].cmd, claude::rm(&w.env, old.short_id.as_deref().unwrap()));
    assert!(steps[1].cmd.contains(&"--settings".to_string()) && steps[1].cmd.contains(&"--name".to_string()));

    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id).session.unwrap();
    assert_eq!(after.session_id, old.session_id, "with the row gone, Claude Code keeps the id");
    assert_eq!(after.labels.get("NIMITZ_ID"), Some(&id.0), "and takes the new labels");
    let now = std::fs::read_to_string(evidence::transcript(&w.env, &mut evidence::FileCache::default(), Some(&after.cwd), &after.session_id).unwrap()).unwrap();
    assert!(now.starts_with(&history), "the conversation is kept");
    assert_eq!(w.rows().iter().filter(|r| r.session_id == old.session_id).count(), 1, "one row");
    assert_eq!(nimitz_core::registry::load(&w.env).identities[&id.0].lineage.last().unwrap().via, "relaunch");
}

#[test]
fn bug53_a_resume_with_flags_while_the_row_exists_is_an_empty_copy_so_apply_refuses_it() {
    let w = world();
    let id = w.ship("BARE");
    let old = w.member(&w.snap(), &id).session.unwrap();
    // the fake: flags + an existing row = a new id with an empty history
    let (rc, out) = claude::run(&claude::relaunch_bg(&w.env, &id, "", &old.session_id), Some(&old.cwd));
    assert_eq!(rc, 0, "{out}");
    let copy = w.rows().into_iter().find(|r| r.session_id != old.session_id && r.name == id.name()).expect("a copy");
    let log = evidence::transcript(&w.env, &mut evidence::FileCache::default(), Some(&copy.cwd), &copy.session_id).unwrap();
    assert_eq!(std::fs::read_to_string(log).unwrap().lines().count(), 1, "no history");
    // apply never runs such a step: the row must be gone first
    let plan = Plan { summary: "bad".into(), steps: vec![Step { id: Some(w.ship("STOPPED")), cmd: claude::relaunch_bg(&w.env, &w.ship("STOPPED"), "", &w.member(&w.snap(), &w.ship("STOPPED")).session.unwrap().session_id), cwd: None, why: "x".into() }], ..Default::default() };
    let out = sessions::apply(&w.env, &plan);
    assert!(!out.verified && has(&out.issues, "step.failed"), "{:?}", out.issues);
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
    assert_eq!(steps[0].cmd, claude::interactive(&w.env, &id, "", true));
    assert!(!steps[0].cmd.contains(&"--permission-mode".to_string()), "no mode set: Claude Code's own default");
    assert_eq!(steps[0].cwd.as_deref(), m.cwd.as_deref());
    // core never starts an interactive member: apply leaves it to the CLI's pane
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    assert!(out.ran[0].2.contains("pane"), "{}", out.ran[0].2);
    assert!(w.member(&w.snap(), &id).session.is_none(), "core started nothing itself");

    // What the CLI does with the pane step: open it (here: run it in place), then
    // bind and verify. `claude -c` keeps the transcript's id (probe P0.8).
    let transcript_id = nimitz_core::evidence::transcripts_in(&w.env, m.cwd.as_deref().unwrap())[0].1.clone();
    let since = nimitz_core::evidence::now() - 0.001;
    let (rc, text) = claude::run(&steps[0].cmd, steps[0].cwd.as_deref());
    assert_eq!(rc, 0, "{text}");
    let sid = sessions::bind_started(&w.env, &id, steps[0].cwd.as_deref(), since, None, "resume", 5.0).expect("bound and verified");
    assert_eq!(sid, transcript_id, "-c continues the same session");
    let after = w.member(&w.snap(), &id);
    assert_eq!(after.session.as_ref().map(|p| p.kind.as_str()), Some("interactive"));
    assert!(matches!(after.status, Status::Idle), "{:?}", after.reasons);
    assert!(after.issues.is_empty(), "{:?}", after.issues);
    assert_eq!(nimitz_core::registry::load(&w.env).identities[&id.0].session.as_deref(), Some(sid.as_str()));
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
fn row7_cold_launch_refuses_a_ship_with_no_prompt() {
    let w = world();
    let snap = w.snap();
    let plan = w.start(&snap, &w.ship("NOPROMPT"), false);
    assert!(plan.refusals.iter().any(|i| i.message.contains("no prompt")), "{:?}", plan.refusals);
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
    assert_eq!(after.status, Status::Parked, "sweep starts nothing (no SessionEnd ever: parked)");
}

#[test]
fn sweep_authority_you_any_scope_a_commodore_its_own_squadron() {
    let w = world();
    // give STOPPED a stale copy row: a labelled fork, then stopped
    let stopped = w.ship("STOPPED");
    let tm = w.member(&w.snap(), &stopped);
    let tsid = tm.session.as_ref().unwrap().session_id.clone();
    let (rc, out) = claude::run(&claude::relaunch_bg(&w.env, &stopped, "default", &tsid), tm.cwd.as_deref());
    assert_eq!(rc, 0, "{out}");
    let copy = w.member(&w.snap(), &stopped).extra_rows.into_iter().find(|r| r.pid.is_some()).expect("the copy");
    let (rc, out) = claude::run(&claude::stop(&w.env, copy.short_id.as_deref().unwrap()), None);
    assert_eq!(rc, 0, "{out}");

    let snap = w.snap();
    let argos = Id::commodore("MAIN", "ARGOS");
    let own = Caller::Member { id: argos.clone(), role: Role::Commodore };
    let plan = sessions::plan_sweep(&w.env, &snap, None, &own);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let ids: std::collections::BTreeSet<Id> = plan.steps.iter().filter_map(|s| s.id.clone()).collect();
    assert!(ids.contains(&w.ship("COPIES")) && ids.contains(&stopped), "{ids:?}");
    assert!(ids.iter().all(|i| i.parent().as_ref() == Some(&argos)), "only its own ships: {ids:?}");
    // its own squadron by name, or one of its ships: fine
    assert!(sessions::plan_sweep(&w.env, &snap, Some(&argos), &own).ok());
    assert!(sessions::plan_sweep(&w.env, &snap, Some(&w.ship("COPIES")), &own).ok());
    // another squadron, the fleet, or any other caller: refused with `authority`
    for (target, caller) in [
        (Some(Id::commodore("MAIN", "BOREAL")), own.clone()),
        (Some(Id::admiral("MAIN")), own.clone()),
        (Some(Id::admiral("BETA")), Caller::Member { id: Id::admiral("MAIN"), role: Role::Admiral }),
        (None, Caller::Member { id: w.ship("CLEAN"), role: Role::Ship }),
    ] {
        let p = sessions::plan_sweep(&w.env, &snap, target.as_ref(), &caller);
        assert!(has(&p.refusals, "authority"), "{target:?} {caller:?}: {:?}", p.refusals);
        assert!(p.steps.is_empty());
    }
    // an admiral: its own fleet, by default and by name
    let adm = Caller::Member { id: Id::admiral("MAIN"), role: Role::Admiral };
    let plan = sessions::plan_sweep(&w.env, &snap, None, &adm);
    assert!(plan.ok() && plan.steps.iter().all(|s| s.id.as_ref().is_some_and(|i| i.fleet() == "MAIN")), "{:?}", plan);
    assert!(plan.steps.iter().any(|s| s.id.as_ref() == Some(&w.ship("COPIES"))));
    assert!(sessions::plan_sweep(&w.env, &snap, Some(&argos), &adm).ok());
    // you: a squadron id sweeps everything under it
    let plan = sessions::plan_sweep(&w.env, &snap, Some(&argos), &Caller::User);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.snap();
    assert!(w.member(&after, &w.ship("COPIES")).extra_rows.is_empty());
    assert!(w.member(&after, &stopped).extra_rows.is_empty());
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
fn row11_a_lead_that_runs_as_a_background_job_is_a_member_like_any_other() {
    // 0.4.0: every member is a job (plan-v4 section 1); `role.background` is gone
    let w = world();
    let snap = w.snap();
    let id = Id::commodore("MAIN", "BOREAL");
    let m = w.member(&snap, &id);
    assert_eq!(m.session.as_ref().unwrap().kind, "background");
    assert_eq!(m.status, Status::Idle);
    assert!(m.issues.is_empty(), "{:?}", m.issues);
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok() && plan.steps.is_empty(), "live and clean: nothing to do: {plan:?}");
    let ship_plan = w.start(&snap, &w.ship("CLEAN"), false);
    assert!(ship_plan.ok(), "{:?}", ship_plan.refusals);
}

// ------------------------------------------------------------------ row 12

#[test]
fn row12_a_newer_version_warns_and_launches_an_older_one_refuses_until_force() {
    let w = world();
    let id = w.ship("FRESH");
    // newer than tested: a warning that says so, and the launch goes ahead (0.3.2)
    w.set_cc_version("2.9.99");
    let snap = w.snap();
    assert!(!snap.compat_ok);
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let warn = plan.warnings.iter().find(|i| i.code == "cc.version").expect("the version is still named");
    assert!(warn.message.contains("tested up to") && warn.message.contains("launches go ahead"), "{}", warn.message);
    // below the floor: refused, and --force overrides
    w.set_cc_version("2.0.1");
    let snap = snapshot::snapshot(&w.env, &mut snapshot::Cache::default());
    assert!(has(&sessions::plan_start(&w.env, &snap, &id, &Caller::User, false).refusals, "cc.version"));
    let forced = w.start(&snap, &id, true);
    assert!(forced.ok(), "{:?}", forced.refusals);
    assert!(has(&forced.warnings, "cc.version"), "--force keeps the warning");
    // an untested pre-release refuses too
    w.set_cc_version("2.9.99-beta");
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
    // the broken fleet's commodore is live, known by its labels only
    let sq = Id::commodore("BAD", "SQ");
    let sq_dir = w.env.fleet_dir("BAD").join("squadrons/sq");
    std::fs::create_dir_all(&sq_dir).unwrap();
    let sid = sandbox::live_session(&w.base, &sq, sq_dir.to_str().unwrap(), false);
    let snap = w.snap();
    // M8: the broken fleet still shows, with the parse error on it and on its members
    let bad = snap.fleets.iter().find(|f| f.id.fleet() == "BAD").expect("the BAD fleet shows");
    let file = w.env.fleet_dir("BAD").join("fleet.toml").display().to_string();
    let err = bad.issues.iter().find(|i| i.code == "config.invalid").expect("the fleet carries its parse error");
    assert!(err.message.contains(&format!("{file}:1:")), "file, line and parse error: {}", err.message);
    assert_eq!(err.fix.as_deref(), Some(format!("fix {file} line 1, then nimitz check BAD").as_str()));
    assert_eq!(err.id, Some(Id::admiral("BAD")));
    assert!(bad.admiral.reasons[0].starts_with("config unreadable:"), "{:?}", bad.admiral.reasons);
    let m = w.member(&snap, &sq);
    assert_eq!(m.status, Status::Idle, "{:?}", m.reasons);
    assert_eq!(m.session.as_ref().map(|s| s.session_id.as_str()), Some(sid.as_str()));
    assert_eq!(m.cwd.as_deref(), sq_dir.to_str(), "a commodore known only by its labels still has its squadron folder");
    // `check BAD` also runs the NIMITZ_HOME layout rule
    std::fs::write(w.env.home.join("CLAUDE.md"), "x").unwrap();
    assert!(has(&nimitz_core::check::check_tree(&w.env, &snap, Some(&Id::admiral("BAD"))), "config.layout"));
    std::fs::remove_file(w.env.home.join("CLAUDE.md")).unwrap();
    assert!(!snap.issues.iter().any(|i| i.code == "config.invalid"), "the fleet carries the error; not the snapshot too: {:?}", codes(&snap.issues));
    // the other fleets are not blocked: FRESH starts, and its check is clean of it
    assert!(w.start(&snap, &w.ship("FRESH"), false).ok());
    assert!(!has(&nimitz_core::check::check(&w.env, &snap, Some(&w.ship("FRESH"))), "config.invalid"));
    // `check BAD` (and anything under it) names the error; a start there refuses with it
    for target in [Id::admiral("BAD"), sq.clone()] {
        let issues = nimitz_core::check::check_tree(&w.env, &snap, Some(&target));
        let found: Vec<&Issue> = issues.iter().filter(|i| i.code == "config.invalid").collect();
        assert_eq!(found.len(), 1, "once, for {target}: {issues:?}");
        assert_eq!(found[0], err);
        let plan = w.start(&snap, &target, false);
        assert!(!plan.ok() && plan.refusals.contains(err), "{target}: {:?}", plan.refusals);
    }
    let all = nimitz_core::check::check(&w.env, &snap, None);
    assert!(has(&all, "config.layout"), "{:?}", codes(&all));
    let parse: Vec<&Issue> = all.iter().filter(|i| i.message.contains("does not parse")).collect();
    assert_eq!(parse.len(), 1, "{parse:#?}");
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
    // once the other machine releases the folder, the ship starts here and takes
    // the lock itself; stopping it gives the folder back
    let lock = sessions::ship_lock_path(m.cwd.as_deref().unwrap());
    std::fs::remove_file(&lock).unwrap();
    let snap = w.snap();
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    assert_eq!(sessions::lock_holder(m.cwd.as_deref().unwrap()).as_deref(), Some(sandbox::HOST));
    assert!(sessions::session_issues(&w.env, &w.member(&w.snap(), &id)).is_empty(), "our own lock is not an issue");

    let snap = w.snap();
    let out = sessions::apply(&w.env, &sessions::plan_stop(&w.env, &snap, &id, &Caller::User));
    assert!(out.verified, "{:?}", out.issues);
    assert!(!lock.exists(), "a stop releases the folder");
}

#[test]
fn up_starts_ships_in_depends_on_order() {
    let w = world();
    // FRESH depends on STOPPED, so STOPPED comes first even though F sorts before S
    let sq = w.env.fleet_dir("MAIN").join("squadrons/argos/squadron.toml");
    let raw = std::fs::read_to_string(&sq).unwrap();
    let at = raw.find("name = \"FRESH\"").expect("the FRESH ship");
    let (head, tail) = raw.split_at(at);
    std::fs::write(&sq, format!("{head}{}", tail.replacen("depends_on = []", "depends_on = [\"STOPPED\"]", 1))).unwrap();

    let snap = w.snap();
    let plan = sessions::plan_up(&w.env, &snap, "MAIN", &Caller::User);
    let order: Vec<String> = plan.steps.iter().filter_map(|s| s.id.as_ref().map(|i| i.0.clone())).collect();
    let pos = |id: &str| order.iter().position(|o| o == id);
    assert!(pos("MAIN/ARGOS/STOPPED").is_some() && pos("MAIN/ARGOS/STOPPED") < pos("MAIN/ARGOS/FRESH"), "{order:?}");
}

#[test]
fn an_interactive_step_is_marked_for_a_pane_and_a_background_one_is_not() {
    let w = world();
    let snap = w.snap();
    let beta = sessions::plan_start(&w.env, &snap, &Id::admiral("BETA"), &Caller::User, false);
    assert!(sessions::is_pane_step(&beta.steps[0]), "an admiral opens in a pane");
    let fresh = w.start(&snap, &w.ship("FRESH"), false);
    assert!(!sessions::is_pane_step(&fresh.steps[0]), "a ship is a background session");
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
    assert_eq!(plan.steps.len(), 3, "stop, rm, resume with flags: {:?}", plan.steps);
    assert!(plan.steps[0].cmd.contains(&"stop".to_string()));
    assert!(plan.steps[1].cmd.contains(&"rm".to_string()));
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id);
    let s = after.session.unwrap();
    assert_eq!(s.session_id, old.session_id, "same id, history kept (BUG-53)");
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
    for code in ["name.drift", "labels.missing", "dup.live", "folder.missing", "lineage.broken"] {
        assert!(has(&plan.refusals, code), "{code} missing from {:?}", codes(&plan.refusals));
    }
    assert!(!has(&plan.refusals, "role.background"), "a background lead is a job like every member (0.4.0)");
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
    // its admiral may too; another fleet's admiral may not
    let adm = Caller::Member { id: Id::admiral("MAIN"), role: Role::Admiral };
    assert!(sessions::plan_start(&w.env, &snap, &w.ship("FRESH"), &adm, false).ok());
    let beta = Caller::Member { id: Id::admiral("BETA"), role: Role::Admiral };
    assert!(!sessions::plan_start(&w.env, &snap, &w.ship("FRESH"), &beta, false).ok());
}

#[test]
fn a_commodore_revives_its_ship_while_the_admiral_is_down() {
    let w = world();
    // the admiral's process ends: its session file goes away
    let adm = Id::admiral("MAIN");
    let adm_sid = w.member(&w.snap(), &adm).session.unwrap().session_id;
    for f in std::fs::read_dir(w.env.claude_home.join("sessions")).unwrap().flatten() {
        if std::fs::read_to_string(f.path()).unwrap_or_default().contains(&adm_sid) {
            std::fs::remove_file(f.path()).unwrap();
        }
    }
    let snap = w.snap();
    assert_eq!(w.member(&snap, &adm).status, Status::Stopped);
    let own = Caller::Member { id: Id::commodore("MAIN", "ARGOS"), role: Role::Commodore };
    let ship = w.ship("FRESH");
    let plan = sessions::plan_start(&w.env, &snap, &ship, &own, false);
    assert!(plan.ok(), "the admiral being down is not the commodore's to fix: {:?}", plan.refusals);
    assert!(plan.steps.iter().all(|s| s.id.as_ref() == Some(&ship)), "the commodore starts only its ship: {:?}", plan.steps);
    assert!(plan.warnings.iter().any(|i| i.id.as_ref() == Some(&adm)), "{:?}", plan.warnings);
    // you, top-down: the admiral first (a pane step), then the ship
    let plan = sessions::plan_start(&w.env, &snap, &ship, &Caller::User, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert_eq!(plan.steps[0].id.as_ref(), Some(&adm));
    assert!(sessions::is_pane_step(&plan.steps[0]));
    assert_eq!(plan.steps.last().unwrap().id.as_ref(), Some(&ship));
}

/// A 0.3.x tender still running (resumed outside nimitz): no member, no
/// stowaway, one `tender.retired` whose fix stops it first; `nimitz stop
/// <short>` takes it, and then the fix is the rm.
#[test]
fn a_live_0_3_x_tender_is_stopped_then_removed() {
    let w = world();
    let tid = "MAIN/ARGOS/~fixci";
    let row = w.rows().into_iter().find(|r| r.name == "[MAIN.ARGOS]-COMMODORE/fixci").expect("the row");
    let (rc, out) = claude::run(&claude::resume_bg(&w.env, &row.session_id), Some(&row.cwd));
    assert_eq!(rc, 0, "{out}");
    let short = row.short_id.clone().unwrap();
    assert!(w.rows().iter().any(|r| r.session_id == row.session_id && r.live()), "live now");
    let snap = w.snap();
    assert!(find_member(&snap, &Id(tid.into())).is_none());
    assert!(!snap.fleets.iter().flat_map(|f| f.squadrons.iter()).flat_map(|s| s.ships.iter()).any(|m| m.session.iter().chain(m.extra_rows.iter()).any(|r| r.session_id == row.session_id)), "no member's row");
    assert!(!snap.stowaways.iter().any(|s| s.session_id == row.session_id), "no stowaway");
    let issues: Vec<&Issue> = snap.issues.iter().filter(|i| i.code == "tender.retired").collect();
    assert_eq!(issues.len(), 1, "{:?}", snap.issues);
    assert_eq!((issues[0].severity, issues[0].fix.as_deref()), (Severity::Info, Some(format!("nimitz stop {short}  # then nimitz rm {short}").as_str())));
    // rm refuses while live; stop takes it (the commodore may, it is its squadron's)
    assert!(has(&sessions::plan_rm(&w.env, &snap, &short, &Caller::User).refusals, "session.live"));
    let own = Caller::Member { id: Id::commodore("MAIN", "ARGOS"), role: Role::Commodore };
    let plan = sessions::plan_stop_session(&w.env, &snap, &short, &own);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert_eq!(plan.steps[0].id.as_ref().map(|i| i.0.as_str()), Some(tid));
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.snap();
    let i = after.issues.iter().find(|i| i.code == "tender.retired").expect("still history");
    assert_eq!(i.fix.as_deref(), Some(format!("nimitz rm {short}").as_str()));
    assert!(sessions::plan_rm(&w.env, &after, &short, &Caller::User).ok());
}

/// A 0.3.x tender (plan-v4 section 8): its v1 registry entry is retired on
/// load (history only, never a member), its job row is one `tender.retired`
/// (Info) with `nimitz rm <short>` as the fix, and that rm takes it.
#[test]
fn a_0_3_x_tender_is_retired_history_only() {
    let w = world();
    let tid = "MAIN/ARGOS/~fixci";
    let reg = nimitz_core::registry::load(&w.env);
    assert!(!reg.identities.contains_key(tid), "no member");
    let rec = &reg.retired[tid];
    assert_eq!(rec.lineage.len(), 1, "its history stays");
    let sid = rec.session.clone().unwrap();
    let snap = w.snap();
    assert!(find_member(&snap, &Id(tid.into())).is_none());
    assert!(!snap.fleets.iter().flat_map(|f| f.squadrons.iter()).flat_map(|s| s.ships.iter()).any(|m| m.id.0.contains('~')), "never listed");
    assert!(!snap.stowaways.iter().any(|s| s.session_id == sid), "no stowaway either");
    let issues: Vec<&Issue> = snap.issues.iter().filter(|i| i.code == "tender.retired").collect();
    assert_eq!(issues.len(), 1, "{:?}", snap.issues);
    let i = issues[0];
    let short = w.rows().into_iter().find(|r| r.session_id == sid).unwrap().short_id.unwrap();
    assert_eq!((i.severity, i.fix.as_deref()), (Severity::Info, Some(format!("nimitz rm {short}").as_str())), "{i:?}");
    assert_eq!(i.id.as_ref().map(|x| x.0.as_str()), Some(tid));
    assert!(nimitz_core::check::check(&w.env, &snap, None).iter().any(|i| i.code == "tender.retired"), "check shows it once");
    assert!(!snap.issues.iter().any(|i| i.code == "config.invalid" && i.message.contains(tid)), "not twice");
    // the fix: the squadron's commodore may run it (its own squadron), another may not
    let other = Caller::Member { id: Id::commodore("MAIN", "BOREAL"), role: Role::Commodore };
    assert!(has(&sessions::plan_rm(&w.env, &snap, &short, &other).refusals, "authority"));
    let own = Caller::Member { id: Id::commodore("MAIN", "ARGOS"), role: Role::Commodore };
    let plan = sessions::plan_rm(&w.env, &snap, &short, &own);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.snap();
    assert!(!after.issues.iter().any(|i| i.code == "tender.retired"), "{:?}", after.issues);
    assert!(nimitz_core::registry::load(&w.env).retired.contains_key(tid), "the history is not touched by an rm");
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
    assert_eq!(reg.identities[&id.0].session.as_deref(), Some(stow.session_id.as_str()));
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

    // the rule for nimitz: `--bg --resume <id> [prompt]` keeps the id; a prompt is not a flag
    let copies = w.ship("COPIES");
    let cm = w.member(&w.snap(), &copies);
    let csid = cm.session.as_ref().unwrap().session_id.clone();
    let mut cmd = claude::resume_bg(&w.env, &csid);
    cmd.push("carry on".into());
    let (rc, out) = claude::run(&cmd, cm.cwd.as_deref());
    assert_eq!(rc, 0);
    assert!(out.contains("saved options"), "{out}");

    // ACME: the PATH wrapper adds its own --settings, so a resume through it is a
    // copy (any stopped row shows it; this one is the 0.3.x tender's)
    let tr = w.rows().into_iter().find(|r| r.name == "[MAIN.ARGOS]-COMMODORE/fixci" && !r.live()).expect("a stopped row");
    let tsid = tr.session_id.clone();
    assert_ne!(w.env.claude_bin, w.env.native_claude_bin, "the sandbox models the wrapper");
    let cmd = vec![w.env.claude_bin.clone(), "--bg".into(), "--resume".into(), tsid.clone()];
    let (rc, out) = claude::run(&cmd, Some(&tr.cwd));
    assert_eq!(rc, 0);
    assert!(out.contains("started a copy"), "{out}");
    assert!(!w.rows().iter().any(|r| r.session_id == tsid && r.live()), "the original stays stopped");
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

// ------------------------------------------------------------------ legacy (old fleet) sessions

#[test]
fn legacy_old_fleet_session_refuses_then_adopt_stop_start_relaunches_with_labels() {
    let w = world();
    let id = w.ship("FRESH");
    let cwd = w.member(&w.snap(), &id).cwd.unwrap();
    // the migration's map, and the old fleet's [ARGOS]-FRESH running in the folder
    let fleet_dir = w.env.fleet_dir("MAIN");
    std::fs::write(fleet_dir.join("legacy.toml"), "[names]\n\"[ARGOS]-FRESH\" = \"MAIN/ARGOS/FRESH\"\n").unwrap();
    let old = vec![w.env.native_claude_bin.clone(), "--bg".into(), "--name".into(), "[ARGOS]-FRESH".into(), "old prompt".into()];
    let (rc, out) = claude::run(&old, Some(&cwd));
    assert_eq!(rc, 0, "{out}");
    let snap = w.snap();
    let row = snap.stowaways.iter().find(|s| s.name == "[ARGOS]-FRESH").expect("still a stowaway (unlabelled)").clone();

    // check: a `legacy` issue on the member, no `stowaway` line, and no rm offered while live
    let issues = nimitz_core::check::check(&w.env, &snap, None);
    let leg = issues.iter().find(|i| i.code == "legacy" && i.id.as_ref() == Some(&id)).expect("legacy issue");
    assert!(leg.message.contains("old fleet"), "{}", leg.message);
    assert!(leg.fix.as_deref().unwrap().contains(&format!("nimitz adopt {} {id}", row.session_id)));
    assert!(!leg.fix.as_deref().unwrap().contains("rm"), "never rm a live session: {:?}", leg.fix);
    assert!(!issues.iter().any(|i| i.code == "stowaway" && i.message.contains(&row.session_id)));

    // start and up refuse: a second agent in the folder
    let plan = w.start(&snap, &id, false);
    assert!(has(&plan.refusals, "legacy"), "{:?}", plan.refusals);
    assert!(w.steps_of(&plan, &id).is_empty());
    let up = sessions::plan_up(&w.env, &snap, "MAIN", &Caller::User);
    assert!(up.steps.iter().all(|s| s.id.as_ref() != Some(&id)), "up starts no second FRESH");

    // adopt, then stop: the identity owns an unlabelled, stopped session
    let out = sessions::apply(&w.env, &sessions::plan_adopt(&w.env, &snap, &row.session_id, &id, &Caller::User));
    assert!(out.verified, "{:?}", out.issues);
    let snap = w.snap();
    assert_eq!(w.member(&snap, &id).session.as_ref().map(|p| p.session_id.clone()), Some(row.session_id.clone()));
    let out = sessions::apply(&w.env, &sessions::plan_stop(&w.env, &snap, &id, &Caller::User));
    assert!(out.verified, "{:?}", out.issues);

    // the next start is row 5: rm the old row, then resume with every flag:
    // the old fleet's session id becomes the ship's, with its history (BUG-53)
    let snap = w.snap();
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let steps = w.steps_of(&plan, &id);
    assert!(steps[0].cmd.contains(&"rm".to_string()), "{:?}", steps[0].cmd);
    assert!(steps[1].cmd.contains(&"--resume".to_string()) && steps[1].cmd.contains(&"--settings".to_string()), "{:?}", steps[1].cmd);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id);
    let s = after.session.unwrap();
    assert_eq!(s.session_id, row.session_id, "the old session id is now the ship's");
    assert_eq!(s.name, id.name());
    assert_eq!(s.labels.get("NIMITZ_ID"), Some(&id.0));
    assert_eq!(w.rows().iter().filter(|r| r.session_id == row.session_id).count(), 1, "one row");
    assert!(after.issues.is_empty(), "{:?}", after.issues);
}

#[test]
fn a_stowaway_fix_offers_rm_only_once_it_is_stopped() {
    let w = world();
    let snap = w.snap();
    let live = snap.stowaways.iter().find(|s| s.name == "quick look at the logs").unwrap().clone();
    let fix = |snap: &Snapshot| {
        nimitz_core::check::check(&w.env, snap, None)
            .into_iter()
            .find(|i| i.code == "stowaway" && i.message.contains(&live.session_id))
            .and_then(|i| i.fix)
            .unwrap()
    };
    let f = fix(&snap);
    assert!(f.contains("stop") && f.find("stop") < f.find("rm"), "{f}");
    let (rc, out) = claude::run(&claude::stop(&w.env, live.short_id.as_deref().unwrap()), None);
    assert_eq!(rc, 0, "{out}");
    let f = fix(&w.snap());
    assert!(!f.contains("stop") && f.contains("nimitz rm"), "{f}");
}

#[test]
fn a_migrated_ship_continues_its_newest_old_session_and_keeps_the_rest() {
    let w = world();
    let id = w.ship("FRESH");
    let cwd = w.member(&w.snap(), &id).cwd.unwrap();
    std::fs::write(w.env.fleet_dir("MAIN").join("legacy.toml"), "[names]\n\"[ARGOS]-FRESH\" = \"MAIN/ARGOS/FRESH\"\n\"[ARGOS]-LEAD\" = \"MAIN/ARGOS\"\n").unwrap();
    // three stopped sessions of the old [ARGOS]-FRESH in its folder
    let mut olds = vec![];
    for _ in 0..3 {
        let old = vec![w.env.native_claude_bin.clone(), "--bg".into(), "--name".into(), "[ARGOS]-FRESH".into(), "old prompt".into()];
        let (rc, out) = claude::run(&old, Some(&cwd));
        assert_eq!(rc, 0, "{out}");
        let short = out.trim().to_string();
        let (rc, out) = claude::run(&claude::stop(&w.env, &short), None);
        assert_eq!(rc, 0, "{out}");
        olds.push(short);
    }
    // the first one was used last: its job row has the newest updatedAt
    let job = w.base.join("claude_home/jobs").join(&olds[0]).join("state.json");
    let mut j: serde_json::Value = serde_json::from_slice(&std::fs::read(&job).unwrap()).unwrap();
    j["updatedAt"] = serde_json::json!(((evidence::now() + 60.0) * 1000.0) as i64);
    std::fs::write(&job, serde_json::to_vec(&j).unwrap()).unwrap();
    let newest = w.rows().into_iter().find(|r| r.short_id.as_deref() == Some(olds[0].as_str())).unwrap();

    let snap = w.snap();
    assert!(w.member(&snap, &id).issues.iter().filter(|i| i.code == "legacy").all(|i| i.severity == Severity::Info));
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let steps = w.steps_of(&plan, &id);
    assert_eq!(steps.len(), 2, "rm the newest old row, then resume it with flags; the others stay: {:?}", steps);
    assert_eq!(steps[0].cmd, claude::rm(&w.env, &olds[0]));
    assert_eq!(steps[1].cmd, claude::relaunch_bg(&w.env, &id, "", &newest.session_id));
    assert_eq!(steps[1].why, format!("continue the old fleet's session {} (newest of 3): its id becomes the ship's, history kept", olds[0]));
    // up plans the same, not a cold launch
    let up = sessions::plan_up(&w.env, &snap, "MAIN", &Caller::User);
    assert!(up.steps.iter().any(|s| s.id.as_ref() == Some(&id) && s.cmd.contains(&"--resume".to_string())));

    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.snap();
    let m = w.member(&after, &id);
    let s = m.session.as_ref().unwrap();
    assert!(s.pid.is_some() && s.labels.get("NIMITZ_ID") == Some(&id.0) && s.name == id.name());
    assert_eq!(s.session_id, newest.session_id, "the old fleet's session id is now the ship's, history kept (BUG-53)");
    let rec = &nimitz_core::registry::load(&w.env).identities[&id.0];
    assert_eq!(rec.lineage.last().unwrap().via, "migrate");
    for o in &olds[1..] {
        assert!(w.rows().iter().any(|r| r.short_id.as_deref() == Some(o.as_str())), "old row {o} stays");
    }
    assert_eq!(m.issues.iter().filter(|i| i.code == "legacy" && i.severity == Severity::Info).count(), 2, "{:?}", m.issues);

    // sweep offers them (it is a plan you confirm), and removes them when applied
    let sweep = sessions::plan_sweep(&w.env, &after, Some(&id), &Caller::User);
    let rms: Vec<&Step> = sweep.steps.iter().filter(|s| s.why.contains("old fleet")).collect();
    assert_eq!(rms.len(), 2, "{:?}", sweep.steps);
    assert!(sessions::apply(&w.env, &sweep).verified);
    assert!(olds[1..].iter().all(|o| !w.rows().iter().any(|r| r.short_id.as_deref() == Some(o.as_str()))));
}

#[test]
fn a_commodore_whose_old_lead_ran_elsewhere_says_why_it_starts_fresh() {
    let w = world();
    let plan = nimitz_core::config::plan_squadron_new(&w.env, "MAIN", "NEWSQ", "brief", Comms::default());
    assert!(nimitz_core::config::apply_config_plan(&w.env, &plan).verified);
    std::fs::write(w.env.fleet_dir("MAIN").join("legacy.toml"), "[names]\n\"[NEWSQ]-LEAD\" = \"MAIN/NEWSQ\"\n").unwrap();
    let id = Id::commodore("MAIN", "NEWSQ");
    let plan = w.start(&w.snap(), &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let step = w.steps_of(&plan, &id)[0].clone();
    assert!(sessions::is_pane_step(&step));
    assert!(step.why.contains("[NEWSQ]-LEAD") && step.why.contains("another folder"), "{}", step.why);
}

// ------------------------------------------------------------------ retire and layout

#[test]
fn retire_a_live_ship_stops_removes_its_rows_then_the_config_in_one_verified_apply() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("CLEAN");
    let short = w.member(&snap, &id).session.unwrap().short_id.unwrap();
    // you, its admiral or its commodore; not a ship, not another squadron's commodore
    let own = Caller::Member { id: Id::commodore("MAIN", "ARGOS"), role: Role::Commodore };
    assert!(!has(&sessions::plan_retire(&w.env, &snap, &own, &id).refusals, "authority"));
    let boreal = Caller::Member { id: Id::commodore("MAIN", "BOREAL"), role: Role::Commodore };
    assert!(has(&sessions::plan_retire(&w.env, &snap, &boreal, &id).refusals, "authority"));
    let ship = Caller::Member { id: w.ship("DRIFT"), role: Role::Ship };
    assert!(has(&sessions::plan_retire(&w.env, &snap, &ship, &id).refusals, "authority"));
    // a squadron goes by its admiral, never by its own commodore; a fleet only by you
    assert!(has(&sessions::plan_retire(&w.env, &snap, &own, &Id::commodore("MAIN", "ARGOS")).refusals, "authority"));
    let adm = Caller::Member { id: Id::admiral("MAIN"), role: Role::Admiral };
    assert!(has(&sessions::plan_retire(&w.env, &snap, &adm, &Id::admiral("MAIN")).refusals, "authority"));
    assert!(!has(&sessions::plan_retire(&w.env, &snap, &adm, &Id::commodore("MAIN", "ARGOS")).refusals, "authority"));
    let plan = sessions::plan_retire(&w.env, &snap, &Caller::User, &id);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let kinds: Vec<&str> = plan.steps.iter().map(|s| s.cmd.get(1).map(String::as_str).filter(|c| ["stop", "rm"].contains(c)).unwrap_or(s.cmd[0].as_str())).collect();
    assert_eq!(kinds, vec!["stop", "rm", "write"], "{:?}", plan.steps);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    assert!(!w.rows().iter().any(|r| r.short_id.as_deref() == Some(short.as_str())), "the job row is gone");
    let f = nimitz_core::config::load_fleet(&w.env, "MAIN").unwrap();
    assert!(!f.squadrons.iter().any(|(_, s)| s.ships.iter().any(|x| x.name == "CLEAN")), "and the ship is out of the config");
}

#[test]
fn retire_a_squadron_or_fleet_refuses_while_anything_in_it_is_live() {
    let w = world();
    let snap = w.snap();
    let plan = sessions::plan_retire(&w.env, &snap, &Caller::User, &Id::commodore("MAIN", "ARGOS"));
    assert!(!plan.ok());
    assert!(plan.refusals.iter().any(|i| i.message.contains("still has")), "ships left in the config: {:?}", plan.refusals);
    assert!(plan.refusals.iter().any(|i| i.message.contains("is live")), "{:?}", plan.refusals);
    let out = sessions::apply(&w.env, &plan);
    assert!(!out.verified && w.member(&w.snap(), &w.ship("CLEAN")).session.unwrap().pid.is_some(), "nothing ran");
    assert!(!sessions::plan_retire(&w.env, &snap, &Caller::User, &Id::admiral("MAIN")).ok());
}

#[test]
fn layout_spec_lays_out_leads_and_attaches_live_ships() {
    use nimitz_core::layout::{self, TabKind};
    let w = world();
    let snap = w.snap();
    // MAIN's admiral and ARGOS's commodore run already; BOREAL's is in the background (refused, so skipped)
    let spec = layout::plan(&w.env, &snap, Some("MAIN"), false, &Caller::User, false);
    assert!(spec.workspaces.is_empty(), "{:?}", spec.workspaces);
    assert!(spec.plan.warnings.iter().any(|i| i.code == "layout.skip"));
    // BETA's admiral ended with a transcript: a Start tab with `claude -c`
    let spec = layout::plan(&w.env, &snap, Some("BETA"), false, &Caller::User, false);
    assert_eq!(spec.workspaces.len(), 1);
    let t = &spec.workspaces[0].tabs[0];
    assert_eq!((spec.workspaces[0].label.as_str(), t.label.as_str(), &t.kind), ("BETA", "BETA.ADMIRAL", &TabKind::Start));
    assert!(t.cmd.contains(&"-c".to_string()));
    assert_eq!(spec.plan.steps.len(), 1);
    // ships: an Attach tab per live ship, in its squadron's workspace
    let mut spec = layout::plan(&w.env, &snap, Some("MAIN"), true, &Caller::User, false);
    let argos = spec.workspaces.iter().find(|w| w.label == "MAIN.ARGOS").unwrap();
    let clean = argos.tabs.iter().find(|t| t.label == "MAIN.ARGOS.CLEAN").unwrap().clone();
    assert_eq!(clean.kind, TabKind::Attach);
    assert_eq!(clean.cmd[1], "attach");
    assert!(!argos.tabs.iter().any(|t| t.label == "MAIN.ARGOS.STOPPED"), "a stopped ship gets no tab");
    // a ship already shown in a pane is dropped
    let before = spec.plan.steps.len();
    layout::drop_shown(&mut spec, &[clean.session_id.clone().unwrap()]);
    assert_eq!(spec.plan.steps.len(), before - 1);
    assert!(!spec.workspaces.iter().flat_map(|w| w.tabs.iter()).any(|t| t.id == clean.id));
    // only you
    let own = Caller::Member { id: Id::commodore("MAIN", "ARGOS"), role: Role::Commodore };
    assert!(has(&layout::plan(&w.env, &snap, None, false, &own, false).plan.refusals, "authority"));
}

// ------------------------------------------------------------------ a symlinked NIMITZ_HOME

/// The user's real home is `~/.claude-nimitz -> ~/Dropbox/claude-nimitz`, and
/// Claude Code records real paths. Start, check, stowaways and trust must all
/// match a session's real cwd with the member folder written through the link.
#[test]
fn a_symlinked_nimitz_home_starts_checks_finds_stowaways_and_trusts() {
    let w = world();
    let link = w.env.home.clone();
    let base = link.parent().unwrap().to_path_buf();
    let real = base.join("dropbox-nimitz");
    std::fs::rename(&link, &real).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(w.env.home, link, "the env keeps the path as written");

    // a managed ship: its folder is under the link
    let ship = ShipDef { name: "MUD".into(), prompt: Some("hi".into()), dirs: [("sandbox".to_string(), "managed".to_string())].into(), ..Default::default() };
    let p = nimitz_core::config::plan_ship_new(&w.env, "MAIN", "ARGOS", ship);
    assert!(nimitz_core::config::apply_config_plan(&w.env, &p).verified, "{:?}", p.refusals);
    let id = w.ship("MUD");
    let snap = w.snap();
    let m = w.member(&snap, &id);
    assert!(m.cwd.as_deref().unwrap().starts_with(link.to_str().unwrap()), "shown as written: {:?}", m.cwd);

    // start: the fake claude records the real cwd, as Claude Code does
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let row = w.rows().into_iter().find(|r| r.name == id.name()).unwrap();
    assert!(row.cwd.starts_with(real.to_str().unwrap()), "real cwd recorded: {}", row.cwd);

    // check: the live ship is its member's session, with nothing to say
    let snap = w.snap();
    let m = w.member(&snap, &id);
    assert_eq!(m.session.as_ref().map(|s| s.session_id.clone()), Some(row.session_id.clone()));
    assert!(m.issues.is_empty(), "{:?}", m.issues);
    let issues = nimitz_core::check::check(&w.env, &snap, Some(&id));
    assert!(issues.iter().all(|i| i.severity != Severity::Error), "{issues:?}");
    assert!(!snap.stowaways.iter().any(|s| s.session_id == row.session_id));
    // a start again: nothing (live and clean)
    assert!(w.start(&snap, &id, false).steps.iter().all(|s| s.id.as_ref() != Some(&id)));

    // a stowaway made by hand in the commodore's folder (real path) is found
    let sq_real = real.join("main/squadrons/argos");
    let (rc, out) = claude::run(&[w.env.native_claude_bin.clone(), "--bg".into(), "--name".into(), "by hand".into(), "x".into()], Some(sq_real.to_str().unwrap()));
    assert_eq!(rc, 0, "{out}");
    assert!(w.snap().stowaways.iter().any(|s| s.name == "by hand"), "a real-path session under the linked home is in fleet scope");

    // trust: a folder trusted under its real path is trusted through the link too
    let t = base.join("trusted-real");
    std::fs::create_dir_all(&t).unwrap();
    let tl = base.join("trusted-link");
    std::os::unix::fs::symlink(&t, &tl).unwrap();
    let f = base.join("claude_home/.claude.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
    v["projects"][t.canonicalize().unwrap().to_str().unwrap()] = serde_json::json!({"hasTrustDialogAccepted": true});
    std::fs::write(&f, serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(evidence::folder_trusted(&w.env, &mut evidence::FileCache::default(), tl.to_str().unwrap()));
}

// ------------------------------------------------------------------ strays in a ship's folder

/// The real-config regression: other agents the user started in a ship's
/// folder are not the ship. A named stray, an unnamed stray with no fork link,
/// and an old legacy row: start, sweep and relaunch never touch the strays, and
/// the ship gets no labels.missing, copy.rows or dup.live from them.
#[test]
fn strays_in_a_ship_folder_are_never_the_ship() {
    let w = world();
    let id = w.ship("STOPPED");
    let m = w.member(&w.snap(), &id);
    let cwd = m.cwd.clone().unwrap();
    let own_sid = m.session.as_ref().unwrap().session_id.clone();
    std::fs::write(w.env.fleet_dir("MAIN").join("legacy.toml"), "[names]\n\"[ARGOS]-STOPPED\" = \"MAIN/ARGOS/STOPPED\"\n").unwrap();
    let run = |args: &[&str]| {
        let mut c = vec![w.env.native_claude_bin.clone()];
        c.extend(args.iter().map(|s| s.to_string()));
        let (rc, out) = claude::run(&c, Some(&cwd));
        assert_eq!(rc, 0, "{out}");
        out.trim().to_string()
    };
    let named = run(&["--bg", "--name", "publish plan draft spec", "x"]);
    let ragtime = run(&["--bg", "--name", "[ARGOS]-RAGTIME", "x"]);
    let unnamed = run(&["--bg", "x"]);
    let old = run(&["--bg", "--name", "[ARGOS]-STOPPED", "x"]);
    run(&["stop", &old]);
    let strays = [named.clone(), ragtime.clone(), unnamed.clone()];

    let snap = w.snap();
    let m = w.member(&snap, &id);
    assert_eq!(m.session.as_ref().map(|s| s.session_id.clone()), Some(own_sid.clone()), "the ship's own session stays current");
    assert!(m.extra_rows.is_empty(), "no stray is a copy: {:?}", m.extra_rows.iter().map(|r| &r.name).collect::<Vec<_>>());
    for code in ["labels.missing", "copy.rows", "dup.live"] {
        assert!(!has(&m.issues, code), "{code}: {:?}", m.issues);
    }
    for s in &strays {
        assert!(snap.stowaways.iter().any(|x| x.short_id.as_deref() == Some(s.as_str())), "{s} is a stowaway");
    }
    let touches = |p: &Plan| p.steps.iter().any(|s| strays.iter().any(|x| s.cmd.contains(x)));
    let start = w.start(&snap, &id, false);
    assert!(start.ok(), "{:?}", start.refusals);
    assert!(!touches(&start), "{:?}", start.steps);
    let relaunch = sessions::plan_relaunch(&w.env, &snap, &id, &Caller::User);
    assert!(!touches(&relaunch), "{:?}", relaunch.steps);
    let sweep = sessions::plan_sweep(&w.env, &snap, Some(&id), &Caller::User);
    assert!(!touches(&sweep), "{:?}", sweep.steps);
    assert!(sweep.steps.iter().any(|s| s.cmd.contains(&old)), "the old legacy row is offered: {:?}", sweep.steps);
    // and after the start ran, the strays are as they were
    assert!(sessions::apply(&w.env, &start).verified);
    for s in &strays {
        assert!(w.rows().iter().any(|r| r.short_id.as_deref() == Some(s.as_str()) && r.live()), "{s} untouched");
    }

    // a real copy still is one: an unnamed fork of the ship's own session
    let own = w.member(&w.snap(), &id).session.unwrap().session_id;
    let fork = run(&["--bg", "--resume", &own, "--settings", "{}"]);
    let fork = fork.split_whitespace().last().unwrap().to_string();
    let m = w.member(&w.snap(), &id);
    assert!(m.extra_rows.iter().any(|r| r.short_id.as_deref() == Some(fork.as_str())), "named <name>-w-w: {:?}", m.extra_rows);
    // with no name at all, the job row's fork link alone ties it to the ship
    for f in [w.base.join("claude_home/jobs").join(&fork).join("state.json"), w.base.join("claude_home/sessions").join(format!("{fork}.json"))] {
        let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        v["name"] = serde_json::json!("");
        std::fs::write(&f, serde_json::to_vec(&v).unwrap()).unwrap();
    }
    let m = w.member(&w.snap(), &id);
    assert!(m.extra_rows.iter().any(|r| r.short_id.as_deref() == Some(fork.as_str()) && r.name.is_empty()), "unnamed fork: {:?}", m.extra_rows);
}

#[test]
fn bug52_the_labels_mismatch_fix_stops_a_session_in_the_wrong_folder() {
    let w = world();
    let id = w.ship("FRESH");
    let wrong = w.member(&w.snap(), &w.ship("STOPPED")).cwd.unwrap();
    let settings = claude::settings_json(&w.env, &id, true);
    let (rc, out) = claude::run(&[w.env.native_claude_bin.clone(), "--bg".into(), "--name".into(), id.name(), "--settings".into(), settings, "hi".into()], Some(&wrong));
    assert_eq!(rc, 0, "{out}");
    let snap = w.snap();
    let i = w.member(&snap, &id).issues.into_iter().find(|i| i.code == "labels.mismatch").expect("labels.mismatch");
    let fix = i.fix.unwrap();
    let short = fix.strip_prefix("nimitz stop ").expect("a stop fix").trim().to_string();
    let plan = sessions::plan_stop_session(&w.env, &snap, &short, &Caller::User);
    assert!(plan.ok(), "the fix must run: {:?}", plan.refusals);
    assert!(sessions::apply(&w.env, &plan).verified);
    assert!(!w.rows().iter().any(|r| r.short_id.as_deref() == Some(short.as_str()) && r.live()));
    // BUG-54: stopped, it is a Warn (no start in the fleet is blocked) whose fix `nimitz rm` runs
    let snap = w.snap();
    let i = w.member(&snap, &id).issues.into_iter().find(|i| i.code == "labels.mismatch").expect("still reported");
    assert_eq!(i.severity, Severity::Warn);
    assert_eq!(i.fix.as_deref(), Some(format!("nimitz rm {short}").as_str()));
    assert!(w.start(&snap, &w.ship("STOPPED"), false).ok());
    let rm = sessions::plan_rm(&w.env, &snap, &short, &Caller::User);
    assert!(rm.ok(), "{:?}", rm.refusals);
    assert!(sessions::apply(&w.env, &rm).verified);
    assert!(!w.rows().iter().any(|r| r.short_id.as_deref() == Some(short.as_str())));
}

#[test]
fn bug51_a_caller_without_authority_gets_only_the_authority_refusal() {
    let w = world();
    let snap = w.snap();
    let ship = Caller::Member { id: w.ship("CLEAN"), role: Role::Ship };
    for target in ["CLEAN", "DRIFT", "FRESH"] {
        let p = w.start_as(&snap, &w.ship(target), &ship);
        assert_eq!(p.refusals.len(), 1, "{target}: {:?}", p.refusals);
        assert_eq!(p.refusals[0].code, "authority");
        assert!(p.warnings.is_empty() && p.steps.is_empty() && p.refusals[0].fix.is_none(), "{target}: {p:?}");
    }
}

#[test]
fn bug50_a_launch_in_an_untrusted_worktree_fails_with_the_trust_fix() {
    let w = world();
    // a git worktree inside FRESH's (trusted) folder: a parent's trust does not cover it
    let fresh = w.member(&w.snap(), &w.ship("FRESH")).cwd.unwrap();
    let wt = std::path::Path::new(&fresh).join("wt");
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::write(wt.join(".git"), "gitdir: /elsewhere/.git/worktrees/wt\n").unwrap();
    let ship = ShipDef { name: "WT".into(), prompt: Some("hi".into()), dirs: [("sandbox".to_string(), wt.to_string_lossy().into_owned())].into(), ..Default::default() };
    let p = nimitz_core::config::plan_ship_new(&w.env, "MAIN", "ARGOS", ship);
    assert!(p.ok(), "{:?}", p.refusals);
    assert!(nimitz_core::config::apply_config_plan(&w.env, &p).verified);
    let id = w.ship("WT");
    let plan = w.start(&w.snap(), &id, false);
    assert!(plan.ok(), "a worktree is a warning, not a refusal: {:?}", plan.refusals);
    let out = sessions::apply(&w.env, &plan);
    assert!(!out.verified);
    let i = out.issues.iter().find(|i| i.code == "step.failed").expect("the launch failed");
    assert!(i.message.contains("Workspace not trusted"), "{}", i.message);
    let fix = i.fix.clone().expect("a fix");
    assert!(fix.contains("run `claude` once in") && fix.contains("/wt"), "{fix}");
}

#[test]
fn an_interrupted_relaunch_is_recovered_with_the_same_id() {
    let w = world();
    let id = w.ship("BARE");
    let old = w.member(&w.snap(), &id).session.unwrap();
    let plan = w.start(&w.snap(), &id, false);
    assert!(plan.steps[0].cmd.contains(&"rm".to_string()) && plan.steps[1].cmd.contains(&"--resume".to_string()), "{:?}", plan.steps);
    // nimitz "dies" between the rm and the resume: here the resume fails
    // (the folder is untrusted for a moment), which leaves the same state
    let trust: Vec<_> = ["claude_home/.claude.json", "claude_home.json"].iter().map(|f| w.base.join(f)).collect();
    let saved: Vec<Vec<u8>> = trust.iter().map(|f| std::fs::read(f).unwrap()).collect();
    for f in &trust {
        std::fs::write(f, br#"{"projects": {}}"#).unwrap();
    }
    let out = sessions::apply(&w.env, &plan);
    assert!(!out.verified);
    assert!(!w.rows().iter().any(|r| r.session_id == old.session_id), "the row is gone");
    let fix = out.issues.iter().find(|i| i.code == "step.failed").and_then(|i| i.fix.clone()).expect("a fix line");
    assert!(fix.contains("run `claude` once in") && fix.contains(&format!("nimitz start {id}  # resumes from its transcript")), "{fix}");
    for (f, b) in trust.iter().zip(saved) {
        std::fs::write(f, b).unwrap();
    }
    // the next start finds it and says so
    let snap = w.snap();
    let m = w.member(&snap, &id);
    assert!(m.reasons.iter().any(|r| r.contains("interrupted relaunch")), "{:?}", m.reasons);
    let plan = w.start(&snap, &id, false);
    let steps = w.steps_of(&plan, &id);
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert!(steps[0].why.starts_with("recovering an interrupted relaunch"), "{}", steps[0].why);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let s = w.member(&w.snap(), &id).session.unwrap();
    assert_eq!(s.session_id, old.session_id, "same id, history kept");
    assert_eq!(s.labels.get("NIMITZ_ID"), Some(&id.0));
    assert!(sessions::pending_relaunch(&w.env, &id).is_none(), "the marker is cleared after the bind");
}

#[test]
fn two_concurrent_relaunches_do_the_work_once() {
    let w = world();
    let id = w.ship("NOLABEL");
    let snap = w.snap();
    let old = w.member(&snap, &id).session.unwrap().session_id;
    let plan = sessions::plan_relaunch(&w.env, &snap, &id, &Caller::User);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let outs: Vec<Outcome> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..2).map(|_| s.spawn(|| sessions::apply(&w.env, &plan))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let skipped = outs.iter().filter(|o| o.ran.iter().all(|(_, _, t)| t.starts_with("skipped: already relaunched"))).count();
    assert_eq!(skipped, 1, "{:?}", outs.iter().map(|o| o.ran.iter().map(|r| r.2.clone()).collect::<Vec<_>>()).collect::<Vec<_>>());
    assert!(outs.iter().all(|o| o.verified), "{:?}", outs.iter().map(|o| &o.issues).collect::<Vec<_>>());
    let m = w.member(&w.snap(), &id);
    let s = m.session.unwrap();
    assert_eq!(s.session_id, old);
    assert!(s.pid.is_some() && s.labels.get("NIMITZ_ID") == Some(&id.0));
}

#[test]
fn a_plan_that_repairs_labels_says_will_repair() {
    let w = world();
    let snap = w.snap();
    let plan = w.start(&snap, &w.ship("BARE"), false);
    let i = plan.warnings.iter().find(|i| i.code == "labels.missing").expect("the reason is shown");
    assert_eq!(i.severity, Severity::Info);
    assert!(i.message.starts_with("will repair: "), "{}", i.message);
}

#[test]
fn the_fake_refuses_a_plain_git_repo_covered_only_by_a_parent() {
    let w = world();
    let fresh = w.member(&w.snap(), &w.ship("FRESH")).cwd.unwrap();
    let repo = std::path::Path::new(&fresh).join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let (rc, out) = claude::run(&[w.env.native_claude_bin.clone(), "--bg".into(), "x".into()], repo.to_str());
    assert_ne!(rc, 0);
    assert!(out.contains("Workspace not trusted"), "{out}");
    // an entry at the repo root trusts it
    for f in ["claude_home/.claude.json", "claude_home.json"].iter().map(|f| w.base.join(f)) {
        let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        v["projects"][repo.canonicalize().unwrap().to_str().unwrap()] = serde_json::json!({"hasTrustDialogAccepted": true});
        std::fs::write(&f, serde_json::to_vec(&v).unwrap()).unwrap();
    }
    let (rc, out) = claude::run(&[w.env.native_claude_bin.clone(), "--bg".into(), "x".into()], repo.to_str());
    assert_eq!(rc, 0, "{out}");
}

#[test]
fn no_plan_forks_a_session() {
    // every plan the world can make: a resume with flags only after the rm of its row
    let w = world();
    let snap = w.snap();
    for f in &snap.fleets {
        let mut ids = vec![f.admiral.id.clone()];
        for sq in &f.squadrons {
            ids.push(sq.commodore.id.clone());
            ids.extend(sq.ships.iter().map(|m| m.id.clone()));
        }
        for id in ids {
            for force in [false, true] {
                assert!(sessions::no_fork(&snap, &sessions::plan_start(&w.env, &snap, &id, &Caller::User, force)), "start {id}");
            }
            assert!(sessions::no_fork(&snap, &sessions::plan_relaunch(&w.env, &snap, &id, &Caller::User)), "relaunch {id}");
        }
        assert!(sessions::no_fork(&snap, &sessions::plan_up(&w.env, &snap, f.id.fleet(), &Caller::User)));
    }
}

#[test]
fn bug55_an_impostor_with_the_labels_never_counts_as_the_member() {
    let w = world();
    let id = w.ship("CLEAN");
    let elsewhere = w.member(&w.snap(), &w.ship("STOPPED")).cwd.unwrap();
    let impostor = |w: &World| {
        let (rc, out) = claude::run(
            &[w.env.native_claude_bin.clone(), "--bg".into(), "--name".into(), "impostor".into(), "--settings".into(), claude::settings_json(&w.env, &id, true), "hi".into()],
            Some(&elsewhere),
        );
        assert_eq!(rc, 0, "{out}");
        out.trim().to_string()
    };
    // a plan made before the impostor appears still runs to the end: the
    // impostor neither skips the resume as "live already" nor binds
    let plan = sessions::plan_relaunch(&w.env, &w.snap(), &id, &Caller::User);
    let imp = impostor(&w);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?} {:?}", out.ran.iter().map(|r| &r.2).collect::<Vec<_>>(), out.issues);
    assert!(!out.ran.iter().any(|r| r.2.starts_with("skipped")), "{:?}", out.ran);
    let s = w.member(&w.snap(), &id).session.unwrap();
    assert!(s.pid.is_some() && s.cwd.ends_with("clean"), "{s:?}");
    // with the impostor live, start and relaunch refuse before any stop or rm
    let snap = w.snap();
    let p = sessions::plan_relaunch(&w.env, &snap, &id, &Caller::User);
    assert!(!p.ok() && has(&p.refusals, "labels.mismatch"), "{:?}", p.refusals);
    assert_eq!(p.refusals.iter().find(|i| i.code == "labels.mismatch").unwrap().fix.as_deref(), Some(format!("nimitz stop {imp}").as_str()));
    assert!(sessions::apply(&w.env, &p).ran.is_empty(), "nothing ran");
}

#[test]
fn name_taken_by_a_hand_copy_in_the_folder_offers_only_stop() {
    let w = world();
    let snap = w.snap();
    // TAKEN's holder runs in TAKEN's own folder: stop, not adopt
    let i = w.member(&snap, &w.ship("TAKEN")).issues.into_iter().find(|i| i.code == "name.taken").unwrap();
    assert!(!i.fix.as_deref().unwrap().contains("adopt"), "{:?}", i.fix);
    // a holder elsewhere may be adopted
    let rand = w.base.join("elsewhere");
    std::fs::create_dir_all(&rand).unwrap();
    let f = w.base.join("claude_home/.claude.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
    v["projects"][rand.canonicalize().unwrap().to_str().unwrap()] = serde_json::json!({"hasTrustDialogAccepted": true});
    std::fs::write(&f, serde_json::to_vec(&v).unwrap()).unwrap();
    let id = w.ship("FRESH");
    let (rc, out) = claude::run(&[w.env.native_claude_bin.clone(), "--bg".into(), "--name".into(), id.name(), "x".into()], rand.to_str());
    assert_eq!(rc, 0, "{out}");
    let i = w.member(&w.snap(), &id).issues.into_iter().find(|i| i.code == "name.taken").unwrap();
    assert!(i.fix.as_deref().unwrap().contains("or adopt it"), "{:?}", i.fix);
}
