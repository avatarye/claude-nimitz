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
    assert_eq!(i.fix.as_deref(), Some("nimitz restart MAIN/ARGOS/DRIFT"));
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
    assert_eq!(rec.lineage.last().unwrap().via, "launch", "a wake keeps how the conversation was born");
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
fn row6_no_row_but_a_transcript_resumes_it_as_a_background_job_with_every_flag() {
    // 0.4.0 (plan-v4 section 5): a lead's conversation with no job row and a
    // transcript (a 0.3.x interactive lead that ended) is resumed by its exact
    // id with every flag; never `-c`, never a pane
    let w = world();
    let snap = w.snap();
    let id = Id::admiral("BETA");
    let m = w.member(&snap, &id);
    assert_eq!(m.session, None);
    assert_eq!(m.status, Status::Stopped);
    assert!(m.reasons[0].contains("transcript"), "{:?}", m.reasons);
    let transcript_id = nimitz_core::registry::load(&w.env).identities[&id.0].session.clone().unwrap();
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let steps = w.steps_of(&plan, &id);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].cmd, claude::relaunch_bg(&w.env, &id, "", &transcript_id));
    assert!(!steps[0].cmd.contains(&"-c".to_string()) && !steps[0].cmd.contains(&"--permission-mode".to_string()), "never -c; no mode set: Claude Code's own default");
    assert_eq!(steps[0].cwd.as_deref(), m.cwd.as_deref());
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id);
    let s = after.session.as_ref().expect("a job row now");
    assert_eq!((s.kind.as_str(), s.session_id.as_str()), ("background", transcript_id.as_str()), "the same conversation, as a job");
    assert!(s.short_id.is_some() && s.pid.is_some());
    assert!(matches!(after.status, Status::Idle), "{:?}", after.reasons);
    assert!(after.issues.is_empty(), "{:?}", after.issues);
    let rec = nimitz_core::registry::load(&w.env).identities[&id.0].clone();
    assert_eq!((rec.session.as_deref(), rec.job.as_deref()), (Some(transcript_id.as_str()), s.short_id.as_deref()));
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
    // issue #2: the failed row goes after the launch, as --fresh's do
    let old = w.member(&snap, &id).session.unwrap().short_id.expect("the failed row has a short id");
    assert_eq!(steps.len(), 2, "{steps:?}");
    assert!(steps[1].cmd.contains(&"rm".to_string()) && steps[1].cmd.contains(&old), "{:?}", steps[1].cmd);
    let out = sessions::apply(&w.env, &forced);
    assert!(out.verified, "{:?}", out.issues);
    let m = w.member(&w.snap(), &id);
    assert!(m.session.as_ref().unwrap().pid.is_some());
    assert!(m.extra_rows.is_empty(), "{:?}", m.extra_rows);
    assert!(!has(&sessions::session_issues(&w.env, &m), "copy.rows"), "{:?}", codes(&sessions::session_issues(&w.env, &m)));
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
    let plan = sessions::plan_sweep(&w.env, &snap, Some(std::slice::from_ref(&id)), &Caller::User);
    assert_eq!(plan.steps.len(), 2);
    assert!(plan.steps.iter().all(|s| s.cmd.contains(&"rm".to_string())));
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id);
    assert!(after.extra_rows.is_empty());
    assert_eq!(after.status, Status::Parked, "sweep starts nothing (no SessionEnd ever: parked)");
}

#[test]
fn sweep_any_scope_for_anyone_a_members_default_its_own() {
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
    assert!(ids.iter().all(|i| i == &argos || i.parent().as_ref() == Some(&argos)), "its own squadron by default: {ids:?}");
    // any scope, for any caller
    for (target, caller) in [
        (Some(Id::commodore("MAIN", "BOREAL")), own.clone()),
        (Some(Id::admiral("MAIN")), own.clone()),
        (Some(Id::admiral("BETA")), Caller::Member { id: Id::admiral("MAIN"), role: Role::Admiral }),
        (None, Caller::Member { id: w.ship("CLEAN"), role: Role::Ship }),
    ] {
        let p = sessions::plan_sweep(&w.env, &snap, target.as_ref().map(std::slice::from_ref), &caller);
        assert!(p.ok(), "{target:?} {caller:?}: {:?}", p.refusals);
    }
    // an admiral: its own fleet, by default and by name
    let adm = Caller::Member { id: Id::admiral("MAIN"), role: Role::Admiral };
    let plan = sessions::plan_sweep(&w.env, &snap, None, &adm);
    assert!(plan.ok() && plan.steps.iter().all(|s| s.id.as_ref().is_some_and(|i| i.fleet() == "MAIN")), "{:?}", plan);
    assert!(plan.steps.iter().any(|s| s.id.as_ref() == Some(&w.ship("COPIES"))));
    assert!(sessions::plan_sweep(&w.env, &snap, Some(std::slice::from_ref(&argos)), &adm).ok());
    // you: a squadron's `...` sweeps everything under it
    let under = nimitz_core::paths::resolve(&snap, &[nimitz_core::paths::Path::Under(Some(argos.clone()))]).unwrap();
    let plan = sessions::plan_sweep(&w.env, &snap, Some(&under), &Caller::User);
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

    // a start trusts the folder first, whoever runs it
    let plan = w.start(&snap, &w.ship("UNTRUSTED"), false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert_eq!(plan.steps[0].cmd[0], "trust", "{:?}", plan.steps);
    assert!(plan.steps[0].why.contains("hooks and MCP servers"), "{}", plan.steps[0].why);
    let plan = w.start_as(&snap, &w.ship("UNTRUSTED"), &Caller::Member { id: Id("MAIN/ARGOS".into()), role: Role::Commodore });
    assert!(plan.ok() && plan.steps[0].cmd[0] == "trust", "{:?} {:?}", plan.refusals, plan.steps);
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
    let f = w.base.join("claude_home").join("sessions").join(format!("{}.json", before.short_id.unwrap()));
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
    let sq_dir = w.env.fleet_dir("BAD").join("squadrons").join("sq");
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
fn issue_copy_by_name_and_stowaway() {
    let w = world();
    let snap = w.snap();
    // an unlabelled live session in TAKEN's folder with TAKEN's exact name is
    // a copy of TAKEN (plan-v4 section 6, rule c), not name.taken and not a
    // stowaway; a start would make a second live session, so it refuses
    let m = w.member(&snap, &w.ship("TAKEN"));
    assert!(has(&m.issues, "copy") && !has(&m.issues, "name.taken"), "{:?}", codes(&m.issues));
    let plan = w.start(&snap, &m.id, false);
    assert!(!plan.ok() && !has(&plan.refusals, "name.taken"), "{:?}", codes(&plan.refusals));
    assert!(!snap.stowaways.iter().any(|s| s.name == m.expected_name));
    // an unlabelled session in the squadron's own folder is a stowaway
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

/// No row and no transcript: `history.lost` with the `--fresh` fix (one
/// word, one flag; acceptance env E9); `--force` still cold-launches.
#[test]
fn issue_history_lost_with_no_row_and_no_transcript() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("BROKEN");
    let m = w.member(&snap, &id);
    assert!(has(&m.issues, "history.lost") && !has(&m.issues, "lineage.broken"), "{:?}", codes(&m.issues));
    let refused = w.start(&snap, &id, false);
    assert!(has(&refused.refusals, "history.lost"), "{:?}", codes(&refused.refusals));
    assert!(refused.refusals.iter().all(|i| i.fix.as_deref().is_none_or(|f| f.contains("--fresh"))), "{:?}", refused.refusals);
    let fresh = sessions::plan_start_with(&w.env, &snap, &id, &Caller::User, sessions::StartOpts { force: false, fresh: true });
    assert!(fresh.ok() && !has(&fresh.warnings, "history.lost"), "the --fresh plan does not repeat the question: {:?} {:?}", fresh.refusals, fresh.warnings);
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

/// What `nimitz start F/... --parked` plans (the jobs only), as one plan:
/// its steps, its errors as refusals, the rest as warnings (the `up` of 0.6).
fn up(w: &World, snap: &Snapshot, fleet: &str) -> nimitz_core::Plan {
    let opts = nimitz_core::restore::RestoreOpts { parked: true, jobs_only: true, ..Default::default() };
    let rp = nimitz_core::restore::plan_start(&w.env, snap, &Default::default(), None, None, false, &[nimitz_core::paths::Path::Under(Some(Id::admiral(fleet)))], opts, &Caller::User);
    let (errors, warnings): (Vec<Issue>, Vec<Issue>) = rp.plan.warnings.into_iter().partition(|i| i.severity == Severity::Error);
    nimitz_core::Plan { steps: rp.plan.steps, refusals: rp.plan.refusals.into_iter().chain(errors).collect(), warnings, ..Default::default() }
}

#[test]
fn a_fleet_start_wakes_ships_in_depends_on_order() {
    let w = world();
    // BARE depends on COPIES, so COPIES is woken first though BARE comes first in the config
    let sq = w.env.fleet_dir("MAIN").join("squadrons").join("argos").join("squadron.toml");
    let raw = std::fs::read_to_string(&sq).unwrap();
    let at = raw.find("name = \"BARE\"").expect("the BARE ship");
    let (head, tail) = raw.split_at(at);
    std::fs::write(&sq, format!("{head}{}", tail.replacen("depends_on = []", "depends_on = [\"COPIES\"]", 1))).unwrap();

    let snap = w.snap();
    let opts = nimitz_core::restore::RestoreOpts { parked: true, jobs_only: true, ..Default::default() };
    let rp = nimitz_core::restore::plan_start(&w.env, &snap, &Default::default(), None, None, false, &[nimitz_core::paths::Path::Under(Some(Id::admiral("MAIN")))], opts, &Caller::User);
    let order: Vec<String> = rp.steps.iter().map(|s| s.id().0.clone()).collect();
    let pos = |id: &str| order.iter().position(|o| o == id);
    assert!(pos("MAIN/ARGOS/COPIES").is_some() && pos("MAIN/ARGOS/COPIES") < pos("MAIN/ARGOS/BARE"), "{order:?}");
}

#[test]
fn every_start_step_is_a_background_launch_leads_too() {
    // plan-v4 section 7: no member opens in a pane; a lead's cold launch is
    // `claude --bg` with its name, the settings and a prompt
    let w = world();
    let snap = w.snap();
    let beta = sessions::plan_start(&w.env, &snap, &Id::admiral("BETA"), &Caller::User, false);
    assert!(beta.steps[0].cmd.contains(&"--bg".to_string()) && !beta.steps[0].cmd.contains(&"-c".to_string()), "{:?}", beta.steps[0].cmd);
    let fresh = w.start(&snap, &w.ship("FRESH"), false);
    assert!(fresh.steps[0].cmd.contains(&"--bg".to_string()));
    // a lead with no history at all: a cold launch with the lead prompt
    let plan = nimitz_core::config::plan_squadron_new(&w.env, "MAIN", "NEWSQ", "brief", None);
    assert!(nimitz_core::config::apply_config_plan(&w.env, &plan).verified);
    let id = Id::commodore("MAIN", "NEWSQ");
    let plan = w.start(&w.snap(), &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let step = w.steps_of(&plan, &id)[0].clone();
    assert_eq!(step.cmd, claude::launch_bg(&w.env, &id, "", Some(&claude::lead_prompt(&id))));
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let m = w.member(&w.snap(), &id);
    assert_eq!(m.session.as_ref().map(|s| s.kind.as_str()), Some("background"));
    assert_eq!(m.status, Status::Idle);
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
fn stop_takes_a_live_lead_like_a_ship_and_refuses_a_dead_ship() {
    let w = world();
    let snap = w.snap();
    // a lead is a job (0.4.0): `claude stop <job>` through the native binary
    let plan = sessions::plan_stop(&w.env, &snap, &Id::admiral("MAIN"), &Caller::User);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert_eq!(&plan.steps[0].cmd[..2], &[w.env.native_claude_bin.clone(), "stop".to_string()]);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    assert_eq!(w.member(&w.snap(), &Id::admiral("MAIN")).status, Status::Stopped);
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
fn any_member_may_start_any_member() {
    let w = world();
    let snap = w.snap();
    for caller in [
        Caller::Member { id: Id::commodore("MAIN", "ARGOS"), role: Role::Commodore },
        Caller::Member { id: Id::commodore("MAIN", "BOREAL"), role: Role::Commodore },
        Caller::Member { id: Id::admiral("BETA"), role: Role::Admiral },
        Caller::Member { id: w.ship("CLEAN"), role: Role::Ship },
    ] {
        let plan = sessions::plan_start(&w.env, &snap, &w.ship("FRESH"), &caller, false);
        assert!(plan.ok(), "{caller:?}: {:?}", plan.refusals);
    }
}

#[test]
fn a_commodore_revives_its_ship_top_down_while_the_admiral_is_down() {
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
    assert_eq!(w.member(&snap, &adm).status, Status::Parked, "{:?}", w.member(&snap, &adm).reasons);
    let own = Caller::Member { id: Id::commodore("MAIN", "ARGOS"), role: Role::Commodore };
    let ship = w.ship("FRESH");
    // the commodore's start is yours: top-down, the admiral first (woken by its exact id), then the ship
    let plan = sessions::plan_start(&w.env, &snap, &ship, &own, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert_eq!(plan.steps, sessions::plan_start(&w.env, &snap, &ship, &Caller::User, false).steps);
    assert_eq!(plan.steps[0].id.as_ref(), Some(&adm));
    assert_eq!(plan.steps[0].cmd, claude::resume_bg(&w.env, &adm_sid));
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
    // the fix: any member may run it
    let other = Caller::Member { id: Id::commodore("MAIN", "BOREAL"), role: Role::Commodore };
    let plan = sessions::plan_rm(&w.env, &snap, &short, &other);
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

    // a start refuses, a fleet's start leaves it: a second agent in the folder
    let plan = w.start(&snap, &id, false);
    assert!(has(&plan.refusals, "legacy"), "{:?}", plan.refusals);
    assert!(w.steps_of(&plan, &id).is_empty());
    let opts = nimitz_core::restore::RestoreOpts { parked: true, jobs_only: true, ..Default::default() };
    let up = nimitz_core::restore::plan_start(&w.env, &snap, &Default::default(), None, None, false, &[nimitz_core::paths::Path::Under(Some(Id::admiral("MAIN")))], opts, &Caller::User);
    assert!(up.steps.iter().all(|s| s.id() != &id), "a fleet start starts no second FRESH");

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
    let job = w.base.join("claude_home").join("jobs").join(&olds[0]).join("state.json");
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
    let sweep = sessions::plan_sweep(&w.env, &after, Some(std::slice::from_ref(&id)), &Caller::User);
    let rms: Vec<&Step> = sweep.steps.iter().filter(|s| s.why.contains("old fleet")).collect();
    assert_eq!(rms.len(), 2, "{:?}", sweep.steps);
    assert!(sessions::apply(&w.env, &sweep).verified);
    assert!(olds[1..].iter().all(|o| !w.rows().iter().any(|r| r.short_id.as_deref() == Some(o.as_str()))));
}

#[test]
fn a_commodore_whose_old_lead_ran_elsewhere_says_why_it_starts_fresh() {
    let w = world();
    let plan = nimitz_core::config::plan_squadron_new(&w.env, "MAIN", "NEWSQ", "brief", None);
    assert!(nimitz_core::config::apply_config_plan(&w.env, &plan).verified);
    std::fs::write(w.env.fleet_dir("MAIN").join("legacy.toml"), "[names]\n\"[NEWSQ]-LEAD\" = \"MAIN/NEWSQ\"\n").unwrap();
    let id = Id::commodore("MAIN", "NEWSQ");
    let plan = w.start(&w.snap(), &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let step = w.steps_of(&plan, &id)[0].clone();
    assert!(step.cmd.contains(&"--bg".to_string()) && !step.cmd.contains(&"--resume".to_string()), "a cold launch: {:?}", step.cmd);
    assert!(step.why.contains("[NEWSQ]-LEAD") && step.why.contains("another folder"), "{}", step.why);
}

// ------------------------------------------------------------------ retire and layout

#[test]
fn retire_a_live_ship_stops_removes_its_rows_then_the_config_in_one_verified_apply() {
    let w = world();
    let snap = w.snap();
    let id = w.ship("CLEAN");
    let short = w.member(&snap, &id).session.unwrap().short_id.unwrap();
    // anyone, with no refusal on whose it is (the CLI runs a retire of its own caller last, detached)
    let ship = Caller::Member { id: w.ship("DRIFT"), role: Role::Ship };
    assert!(sessions::plan_retire(&w.env, &snap, &ship, &id).ok());
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
fn layout_spec_attaches_live_leads_and_with_ships_live_ships() {
    use nimitz_core::layout;
    let w = world();
    let snap = w.snap();
    // every live lead gets a tab that attaches to its job (plan-v4 section 7)
    let spec = layout::plan(&w.env, &snap, Some("MAIN"), false, false);
    assert_eq!(spec.plan.summary, "layout MAIN/...");
    let labels: Vec<(String, Vec<String>)> = spec.workspaces.iter().map(|w| (w.label.clone(), w.tabs.iter().map(|t| t.label.clone()).collect())).collect();
    assert_eq!(labels, vec![("MAIN".to_string(), vec!["MAIN.ADMIRAL".to_string()]), ("MAIN.ARGOS".into(), vec!["MAIN.ARGOS.COMMODORE".into()]), ("MAIN.BOREAL".into(), vec!["MAIN.BOREAL.COMMODORE".into()])]);
    for t in spec.workspaces.iter().flat_map(|w| w.tabs.iter()) {
        assert_eq!(t.cmd, claude::attach(&w.env, t.session_id.as_deref().map(|s| &s[..8]).unwrap()), "the tab attaches to the lead's job");
        assert_eq!(t.cmd[0], w.env.native_claude_bin);
    }
    assert_eq!(spec.plan.steps.len(), 3);
    // BETA's admiral is not live: no tab, `nimitz start` first
    let spec = layout::plan(&w.env, &snap, Some("BETA"), false, false);
    assert!(spec.workspaces.is_empty(), "{:?}", spec.workspaces);
    let skip = spec.plan.warnings.iter().find(|i| i.code == "layout.skip").unwrap();
    assert!(skip.message.contains("nimitz start BETA"), "{}", skip.message);
    // ships: an attach tab per live ship, in its squadron's workspace
    let mut spec = layout::plan(&w.env, &snap, Some("MAIN"), true, false);
    let argos = spec.workspaces.iter().find(|w| w.label == "MAIN.ARGOS").unwrap();
    let clean = argos.tabs.iter().find(|t| t.label == "MAIN.ARGOS.CLEAN").unwrap().clone();
    assert_eq!(clean.cmd[1], "attach");
    assert!(!argos.tabs.iter().any(|t| t.label == "MAIN.ARGOS.STOPPED"), "a stopped ship gets no tab");
    // a ship already shown in a pane is dropped
    let before = spec.plan.steps.len();
    layout::drop_shown(&mut spec, &[clean.session_id.clone().unwrap()]);
    assert_eq!(spec.plan.steps.len(), before - 1);
    assert!(!spec.workspaces.iter().flat_map(|w| w.tabs.iter()).any(|t| t.id == clean.id));
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
    nimitz_core::os::symlink(&real, &link).unwrap();
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
    // (Windows does not resolve a link in a process's folder: the link path is what is recorded there)
    let recorded_ok = row.cwd.starts_with(real.to_str().unwrap()) || (cfg!(windows) && row.cwd.starts_with(link.to_str().unwrap()));
    assert!(recorded_ok, "real cwd recorded: {}", row.cwd);

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
    let sq_real = real.join("main").join("squadrons").join("argos");
    let (rc, out) = claude::run(&[w.env.native_claude_bin.clone(), "--bg".into(), "--name".into(), "by hand".into(), "x".into()], Some(sq_real.to_str().unwrap()));
    assert_eq!(rc, 0, "{out}");
    assert!(w.snap().stowaways.iter().any(|s| s.name == "by hand"), "a real-path session under the linked home is in fleet scope");

    // trust: a folder trusted under its real path is trusted through the link too
    let t = base.join("trusted-real");
    std::fs::create_dir_all(&t).unwrap();
    let tl = base.join("trusted-link");
    nimitz_core::os::symlink(&t, &tl).unwrap();
    let f = base.join("claude_home").join(".claude.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
    v["projects"][nimitz_core::os::canonical(&t).unwrap().to_str().unwrap()] = serde_json::json!({"hasTrustDialogAccepted": true});
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
    for code in ["labels.missing", "copy.rows", "copy", "dup.live"] {
        assert!(!has(&m.issues, code), "{code}: {:?}", m.issues);
    }
    // the user's own work in a ship's folder is not shown (plan-v4 section 6)
    for s in &strays {
        assert!(!snap.stowaways.iter().any(|x| x.short_id.as_deref() == Some(s.as_str())), "{s} is the user's own, not a stowaway");
    }
    let touches = |p: &Plan| p.steps.iter().any(|s| strays.iter().any(|x| s.cmd.contains(x)));
    let start = w.start(&snap, &id, false);
    assert!(start.ok(), "{:?}", start.refusals);
    assert!(!touches(&start), "{:?}", start.steps);
    let relaunch = sessions::plan_relaunch(&w.env, &snap, &id, &Caller::User);
    assert!(!touches(&relaunch), "{:?}", relaunch.steps);
    let sweep = sessions::plan_sweep(&w.env, &snap, Some(std::slice::from_ref(&id)), &Caller::User);
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
    for f in [w.base.join("claude_home").join("jobs").join(&fork).join("state.json"), w.base.join("claude_home").join("sessions").join(format!("{fork}.json"))] {
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
fn bug51_a_members_start_plans_as_the_users_does() {
    let w = world();
    let snap = w.snap();
    let ship = Caller::Member { id: w.ship("CLEAN"), role: Role::Ship };
    for target in ["DRIFT", "FRESH"] {
        let p = w.start_as(&snap, &w.ship(target), &ship);
        let u = w.start(&snap, &w.ship(target), false);
        assert_eq!((p.refusals, p.steps), (u.refusals, u.steps), "{target}");
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
    // a launch with no trust step first: a worktree is a warning, not a refusal, and the launch fails
    let mut plan = w.start(&w.snap(), &id, false);
    assert!(plan.ok(), "a worktree is a warning, not a refusal: {:?}", plan.refusals);
    plan.steps.retain(|s| s.cmd[0] != "trust");
    let out = sessions::apply(&w.env, &plan);
    assert!(!out.verified);
    let i = out.issues.iter().find(|i| i.code == "step.failed").expect("the launch failed");
    assert!(i.message.contains("Workspace not trusted"), "{}", i.message);
    let fix = i.fix.clone().expect("a fix");
    assert!(fix.starts_with("nimitz trust ") && (fix.contains("/wt") || fix.contains("\\wt")), "{fix}");
    // the start trusts the worktree first, then launches
    let plan = w.start(&w.snap(), &id, false);
    assert_eq!(plan.steps[0].cmd, vec!["trust".to_string(), wt.to_string_lossy().into_owned()]);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
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
    assert!(fix.starts_with("nimitz trust ") && fix.contains(&format!("nimitz start {id}  # resumes from its transcript")), "{fix}");
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
        v["projects"][nimitz_core::os::canonical(&repo).unwrap().to_str().unwrap()] = serde_json::json!({"hasTrustDialogAccepted": true});
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
        assert!(sessions::no_fork(&snap, &up(&w, &snap, f.id.fleet())));
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
fn a_copy_by_name_in_the_folder_is_swept_and_a_holder_elsewhere_is_name_taken() {
    let w = world();
    let snap = w.snap();
    // TAKEN's holder runs in TAKEN's own folder: a copy, swept, never adopted
    let i = w.member(&snap, &w.ship("TAKEN")).issues.into_iter().find(|i| i.code == "copy").unwrap();
    assert_eq!(i.fix.as_deref(), Some("nimitz sweep MAIN/ARGOS/TAKEN"), "{:?}", i.fix);
    // a holder elsewhere may be adopted
    let rand = w.base.join("elsewhere");
    std::fs::create_dir_all(&rand).unwrap();
    let f = w.base.join("claude_home").join(".claude.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
    v["projects"][nimitz_core::os::canonical(&rand).unwrap().to_str().unwrap()] = serde_json::json!({"hasTrustDialogAccepted": true});
    std::fs::write(&f, serde_json::to_vec(&v).unwrap()).unwrap();
    let id = w.ship("FRESH");
    let (rc, out) = claude::run(&[w.env.native_claude_bin.clone(), "--bg".into(), "--name".into(), id.name(), "x".into()], rand.to_str());
    assert_eq!(rc, 0, "{out}");
    let i = w.member(&w.snap(), &id).issues.into_iter().find(|i| i.code == "name.taken").unwrap();
    assert!(i.fix.as_deref().unwrap().contains("or adopt it"), "{:?}", i.fix);
}

// ------------------------------------------------------------------ recovery by exact id (plan-v4 section 5)

impl World {
    fn set_knob(&self, name: &str, on: bool) {
        let p = self.base.join("world.json");
        let mut j: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        j[name] = serde_json::json!(on);
        std::fs::write(&p, serde_json::to_vec_pretty(&j).unwrap()).unwrap();
    }
    /// Remove the transcript of `sid` wherever it is.
    fn lose_transcript(&self, sid: &str) {
        let p = evidence::transcript(&self.env, &mut evidence::FileCache::default(), None, sid).expect("a transcript");
        std::fs::remove_file(p).unwrap();
    }
    fn fake(&self, args: &[&str]) -> (i32, String) {
        let mut cmd = vec![self.env.native_claude_bin.clone()];
        cmd.extend(args.iter().map(|a| a.to_string()));
        claude::run(&cmd, None)
    }
}

/// E8 / 5a / M5: a stopped job whose transcript is gone. A resume would start
/// the id over in silence; nimitz refuses with `history.lost` before any
/// launch, and `--fresh` is the cold launch that starts a new lineage entry.
#[test]
fn e8_a_lost_transcript_is_history_lost_before_any_launch_and_fresh_cold_launches() {
    let w = world();
    let id = w.ship("STOPPED");
    let old = w.member(&w.snap(), &id).session.unwrap();
    w.lose_transcript(&old.session_id);
    let snap = w.snap();
    let plan = w.start(&snap, &id, false);
    assert!(!plan.ok() && plan.steps.is_empty(), "no launch: {plan:?}");
    let i = plan.refusals.iter().find(|i| i.code == "history.lost").expect("history.lost");
    assert_eq!(i.severity, Severity::Error);
    assert_eq!(i.fix.as_deref().map(|f| f.split("  #").next().unwrap()), Some(format!("nimitz start {id} --fresh").as_str()));
    // --force does not get around a lost history; --fresh does, with a cold launch
    assert!(!w.start(&snap, &id, true).ok());
    let plan = sessions::plan_start_with(&w.env, &snap, &id, &Caller::User, sessions::StartOpts { force: false, fresh: true });
    assert!(plan.ok(), "{:?}", plan.refusals);
    let steps = w.steps_of(&plan, &id);
    assert_eq!(steps[0].cmd, claude::launch_bg(&w.env, &id, "", Some("You are [MAIN.ARGOS]-STOPPED.")));
    assert!(steps[1].cmd.contains(&"rm".to_string()) && steps[1].cmd.contains(old.short_id.as_ref().unwrap()), "the old row goes after: {:?}", steps[1].cmd);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id);
    let s = after.session.unwrap();
    assert_ne!(s.session_id, old.session_id, "a new conversation");
    assert!(s.pid.is_some() && after.extra_rows.is_empty(), "live, and the old row is gone");
    let rec = nimitz_core::registry::load(&w.env).identities[&id.0].clone();
    assert_eq!(rec.lineage.last().map(|l| (l.session_id.as_str(), l.via.as_str())), Some((s.session_id.as_str(), "launch")));
    assert_eq!(rec.lineage.len(), 2, "the old entry stays as history");
}

/// M11: a lead that exits before its first prompt leaves no transcript. It
/// gets a cold launch; never a `lineage.broken` that would refuse every start
/// in its fleet.
#[test]
fn m11_a_lead_with_no_transcript_cold_launches_and_blocks_nothing() {
    let w = world();
    let adm = Id::admiral("BETA");
    let sid = nimitz_core::registry::load(&w.env).identities[&adm.0].session.clone().unwrap();
    w.lose_transcript(&sid);
    let snap = w.snap();
    let m = w.member(&snap, &adm);
    assert!(!has(&m.issues, "lineage.broken"), "{:?}", m.issues);
    assert!(!nimitz_core::check::check(&w.env, &snap, Some(&adm)).iter().any(|i| i.code == "lineage.broken"));
    let plan = w.start(&snap, &adm, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let step = w.steps_of(&plan, &adm)[0].clone();
    assert_eq!(step.cmd, claude::launch_bg(&w.env, &adm, "", Some(&claude::lead_prompt(&adm))));
    // a ship's registry session with no row and no transcript stays history.lost (5b, E9)
    let broken = w.member(&snap, &w.ship("BROKEN"));
    assert!(has(&broken.issues, "history.lost"));
}

/// 1g / cli-31 / G8: a resume that races the daemon's restart. Claude Code
/// answers with "started a copy as X": the step fails, the copy is stopped
/// and removed in the same run, and the member is never "[OK] verified".
#[test]
fn g8_a_copy_note_fails_the_step_and_removes_the_copy_in_the_same_run() {
    let w = world();
    let id = w.ship("STOPPED");
    let snap = w.snap();
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    w.set_knob("copy_on_resume", true);
    let out = sessions::apply(&w.env, &plan);
    assert!(!out.verified);
    let i = out.issues.iter().find(|i| i.code == "step.failed").expect("the step failed");
    assert!(i.message.contains("started a copy as") && i.message.contains("stopped and removed"), "{}", i.message);
    let copy = out.ran[0].2.split("copy as ").nth(1).unwrap()[..8].to_string();
    let cmds: Vec<String> = out.ran.iter().map(|(s, _, _)| s.cmd[1..].join(" ")).collect();
    assert!(cmds.contains(&format!("stop {copy}")) && cmds.contains(&format!("rm {copy}")), "{cmds:?}");
    assert!(!w.rows().iter().any(|r| r.short_id.as_deref() == Some(&copy)), "the copy row is gone");
    // the job itself came back live under its own id, with its row alone
    let after = w.member(&w.snap(), &id);
    assert!(after.session.as_ref().unwrap().pid.is_some() && after.extra_rows.is_empty(), "{after:?}");
    assert!(after.issues.is_empty(), "{:?}", after.issues);
}

/// r3-e2e finding 12: a wake right after a stop, under load, raced the
/// daemon's bookkeeping and answered with a copy while the job stayed
/// stopped. The copy goes, the job is waited for, and the wake runs once
/// more: the member comes back under its own job, verified, no copy left.
#[test]
fn a_wake_that_raced_the_daemon_is_retried_once_and_verified() {
    let w = world();
    let id = w.ship("STOPPED");
    let job = w.member(&w.snap(), &id).session.unwrap().short_id.unwrap();
    let plan = w.start(&w.snap(), &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    w.set_knob("copy_once", true);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let resumes = out.ran.iter().filter(|(s, _, _)| s.cmd.iter().any(|a| a == "--resume")).count();
    assert_eq!(resumes, 2, "the wake ran twice: {:?}", out.ran.iter().map(|(s, _, _)| s.cmd.join(" ")).collect::<Vec<_>>());
    let after = w.member(&w.snap(), &id);
    assert_eq!(after.session.as_ref().and_then(|s| s.short_id.as_deref()), Some(job.as_str()));
    assert!(after.session.as_ref().unwrap().pid.is_some() && after.extra_rows.is_empty(), "{after:?}");
}

/// M5 / E8: Claude Code found no history and started the id over: the
/// SessionStart says `startup`, not `resume`. `history.lost`, never verified.
#[test]
fn m5_a_resume_that_starts_fresh_is_history_lost_never_verified() {
    let w = world();
    let id = w.ship("STOPPED");
    let plan = w.start(&w.snap(), &id, false);
    w.set_knob("resume_loses_history", true);
    let out = sessions::apply(&w.env, &plan);
    assert!(!out.verified);
    let i = out.issues.iter().find(|i| i.code == "history.lost").expect("history.lost: {out:?}");
    assert!(i.message.contains("`startup`"), "{}", i.message);
    assert!(i.fix.as_deref().unwrap().contains("--fresh"));
}

/// cli-30: a crashed job (state `crashed`, no pid) is the daemon's to restart
/// for about 20 s. A start in that window waits; restarted, nothing is
/// resumed (a resume would make a copy); not restarted, the resume runs.
#[test]
fn cli30_the_crash_window_waits_for_the_daemon_before_a_resume() {
    let w = world();
    let id = w.ship("CLEAN");
    let short = w.member(&w.snap(), &id).session.unwrap().short_id.unwrap();
    let (rc, _) = w.fake(&["__crash", &short]);
    assert_eq!(rc, 0);
    let snap = w.snap();
    let m = w.member(&snap, &id);
    assert_eq!(m.status, Status::Parked, "{:?}", m.reasons);
    assert!(m.reasons.iter().any(|r| r.contains("crashed")), "{:?}", m.reasons);
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok() && plan.steps[0].cmd.contains(&"--resume".to_string()), "{plan:?}");
    // the daemon restarts it inside the window: the resume is skipped
    let env = Env { crash_window_s: 2.0, ..w.env.clone() };
    let bin = w.env.native_claude_bin.clone();
    let daemon = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        claude::run(&[bin, "respawn".into(), short], None)
    });
    let t = std::time::Instant::now();
    let out = sessions::apply(&env, &plan);
    daemon.join().unwrap();
    assert!(t.elapsed() < std::time::Duration::from_secs(2), "it did not wait the whole window");
    assert!(out.verified, "{:?}", out.issues);
    assert!(out.ran[0].2.contains("the daemon restarted"), "{}", out.ran[0].2);
    let after = w.member(&w.snap(), &id);
    assert!(after.session.as_ref().unwrap().pid.is_some() && after.extra_rows.is_empty());
    // no restart within the window: the resume runs, as itself
    let (rc, _) = w.fake(&["__crash", after.session.as_ref().unwrap().short_id.as_deref().unwrap()]);
    assert_eq!(rc, 0);
    let env = Env { crash_window_s: 0.3, ..w.env.clone() };
    let snap = w.snap();
    let plan = w.start(&snap, &id, false);
    let out = sessions::apply(&env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    assert!(out.ran[0].2.contains("woke session"), "{}", out.ran[0].2);
    let after = w.member(&w.snap(), &id);
    assert_eq!(after.session.as_ref().unwrap().short_id.as_deref(), Some(after.session.as_ref().unwrap().short_id.as_deref().unwrap()));
    assert!(after.extra_rows.is_empty(), "one row for life");
}

/// `wake_steps` is the one builder for waking a member by its exact id (a
/// restore uses it too): it refuses a live job and a member with no row.
#[test]
fn wake_steps_wakes_by_exact_id_and_refuses_a_live_job() {
    let w = world();
    let snap = w.snap();
    let stopped = w.member(&snap, &w.ship("STOPPED"));
    let steps = sessions::wake_steps(&w.env, &stopped, "bypassPermissions", stopped.cwd.as_deref().unwrap()).unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].cmd, claude::resume_bg(&w.env, &stopped.session.as_ref().unwrap().session_id));
    let bare = w.member(&snap, &w.ship("BARE"));
    let steps = sessions::wake_steps(&w.env, &bare, "bypassPermissions", bare.cwd.as_deref().unwrap()).unwrap();
    assert_eq!(steps.len(), 2, "rm the row, then resume with every flag: {steps:?}");
    let live = w.member(&snap, &w.ship("CLEAN"));
    assert_eq!(sessions::wake_steps(&w.env, &live, "", live.cwd.as_deref().unwrap()).unwrap_err().code, "session.live");
    let fresh = w.member(&snap, &w.ship("FRESH"));
    assert_eq!(sessions::wake_steps(&w.env, &fresh, "", fresh.cwd.as_deref().unwrap()).unwrap_err().code, "session.none");
    // a lead woken by its id goes the same way (never -c)
    let adm = w.member(&snap, &Id::admiral("MAIN"));
    assert_eq!(sessions::wake_steps(&w.env, &adm, "", adm.cwd.as_deref().unwrap()).unwrap_err().code, "session.live");
}

// ------------------------------------------------------------------ pins (plan-v4 section 7, M24)

/// AV-1 / M24: a lead's job is pinned after it starts, so the daemon never
/// idle-retires it; a relaunch (rm, then resume) unpins and pins again; a pin
/// nimitz did not make is never touched; ships are not pinned.
#[test]
fn m24_a_leads_job_is_pinned_after_a_start_and_unpinned_when_its_row_goes() {
    let w = world();
    let pins = |w: &World| nimitz_core::evidence::pins(&w.env);
    assert!(pins(&w).is_empty());
    // a stopped lead woken by id: pinned, and remembered as nimitz's own
    let adm = Id::admiral("MAIN");
    let out = sessions::apply(&w.env, &sessions::plan_stop(&w.env, &w.snap(), &adm, &Caller::User));
    assert!(out.verified, "{:?}", out.issues);
    let plan = w.start(&w.snap(), &adm, false);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let job = w.member(&w.snap(), &adm).session.unwrap().short_id.unwrap();
    assert_eq!(pins(&w), [job.clone()]);
    assert_eq!(nimitz_core::pins::own(&w.env), [job.clone()]);
    assert!(out.ran[0].2.ends_with("; pinned"), "{}", out.ran[0].2);
    assert!(w.member(&w.snap(), &adm).session.unwrap().pinned);
    // a cold-launched lead too
    let beta = Id::admiral("BETA");
    let out = sessions::apply(&w.env, &w.start(&w.snap(), &beta, false));
    assert!(out.verified, "{:?}", out.issues);
    let beta_job = w.member(&w.snap(), &beta).session.unwrap().short_id.unwrap();
    assert_eq!(pins(&w), [job.clone(), beta_job.clone()]);
    // a ship is not pinned
    let out = sessions::apply(&w.env, &w.start(&w.snap(), &w.ship("STOPPED"), false));
    assert!(out.verified, "{:?}", out.issues);
    assert_eq!(pins(&w).len(), 2);
    // someone else's pin (the agent view's Ctrl+T) stays through a sweep of that row
    let copies = w.ship("COPIES");
    let extra = w.member(&w.snap(), &copies).extra_rows[0].short_id.clone().unwrap();
    assert_eq!(w.fake(&["__pin", &extra]).0, 0);
    let out = sessions::apply(&w.env, &sessions::plan_sweep(&w.env, &w.snap(), Some(std::slice::from_ref(&copies)), &Caller::User));
    assert!(out.verified, "{:?}", out.issues);
    assert!(!w.rows().iter().any(|r| r.short_id.as_deref() == Some(&extra)), "the row went");
    assert!(pins(&w).contains(&extra), "the user's pin stays: it is not nimitz's");
    // a relaunch of the lead: the rm drops the pin, the resume with flags keeps
    // the job id and pins it again, once
    let out = sessions::apply(&w.env, &sessions::plan_relaunch(&w.env, &w.snap(), &adm, &Caller::User));
    assert!(out.verified, "{:?}", out.issues);
    assert!(out.ran.iter().any(|(_, _, t)| t.ends_with("; unpinned")) && out.ran.last().unwrap().2.ends_with("; pinned"), "{:?}", out.ran);
    let job2 = w.member(&w.snap(), &adm).session.unwrap().short_id.unwrap();
    assert_eq!(job2, job, "Claude Code keeps the id");
    let p = pins(&w);
    assert_eq!(p.iter().filter(|x| **x == job).count(), 1, "{p:?}");
    assert_eq!(p.len(), 3, "{p:?}");
    // retire the lead's row for good: fleet rm needs nothing live, so stop it, then rm its row
    let out = sessions::apply(&w.env, &sessions::plan_stop(&w.env, &w.snap(), &beta, &Caller::User));
    assert!(out.verified, "{:?}", out.issues);
    let out = sessions::apply(&w.env, &sessions::plan_retire(&w.env, &w.snap(), &Caller::User, &beta));
    assert!(out.verified, "{:?}", out.issues);
    assert!(!pins(&w).contains(&beta_job), "retired: unpinned ({:?})", pins(&w));
    assert!(!nimitz_core::pins::own(&w.env).contains(&beta_job));
}

// ------------------------------------------------------------------ 0.3.x interactive leads (plan-v4 section 7)

/// A 0.3.x lead still runs interactive on its conversation (no job): `start`
/// refuses with the /exit message; once it is gone, the same conversation
/// comes back as a background job (`--bg --resume <session> --name
/// --settings`, lineage `via migrate`), pinned.
#[test]
fn a_0_3_x_interactive_lead_is_ended_with_exit_then_started_as_a_job() {
    let w = world();
    let adm = Id::admiral("BETA");
    let cwd = w.member(&w.snap(), &adm).cwd.unwrap();
    let sid = nimitz_core::registry::load(&w.env).identities[&adm.0].session.clone().unwrap();
    // the old lead, as 0.3.x ran it: `claude -c --name --settings` in its folder
    let mut cmd = vec![w.env.claude_bin.clone(), "-c".into(), "--name".into(), adm.name(), "--settings".into(), claude::settings_json(&w.env, &adm, false)];
    let (rc, text) = claude::run(&cmd, Some(&cwd));
    assert_eq!(rc, 0, "{text}");
    cmd.clear();
    let snap = w.snap();
    let m = w.member(&snap, &adm);
    let s = m.session.as_ref().expect("the interactive process is the lead's conversation");
    assert_eq!((s.session_id.as_str(), s.kind.as_str(), s.short_id.as_deref(), s.pid.is_some()), (sid.as_str(), "interactive", None, true));
    assert_eq!(m.status, Status::Idle);
    let held = m.issues.iter().find(|i| i.code == "held").expect("held, not dup.live: {m:?}");
    assert!(held.message.contains("0.3.x interactive lead"), "{}", held.message);
    assert!(!has(&m.issues, "dup.live") && !has(&m.issues, "labels.missing"), "{:?}", m.issues);
    let plan = w.start(&snap, &adm, false);
    assert!(!plan.ok() && plan.steps.is_empty());
    let r = &plan.refusals[0];
    assert_eq!((r.code.as_str(), r.severity), ("held", Severity::Error));
    assert!(r.fix.as_deref().unwrap().starts_with("end the interactive lead with /exit, then run nimitz start BETA again"), "{:?}", r.fix);
    // stop and relaunch cannot touch it either
    assert_eq!(sessions::plan_stop(&w.env, &snap, &adm, &Caller::User).refusals[0].code, "session.interactive");
    assert_eq!(sessions::plan_relaunch(&w.env, &snap, &adm, &Caller::User).refusals[0].code, "session.interactive");
    // /exit, then start: the same conversation as a background job, every flag
    assert_eq!(w.fake(&["__end", &sid]).0, 0);
    let snap = w.snap();
    assert_eq!(w.member(&snap, &adm).status, Status::Stopped);
    let plan = w.start(&snap, &adm, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let step = w.steps_of(&plan, &adm)[0].clone();
    assert_eq!(step.cmd, claude::relaunch_bg(&w.env, &adm, "", &sid));
    assert!(step.why.starts_with(sessions::MIGRATE_LEAD_WHY), "{}", step.why);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &adm);
    let s = after.session.unwrap();
    assert_eq!((s.session_id.as_str(), s.kind.as_str(), s.pinned), (sid.as_str(), "background", true));
    let rec = nimitz_core::registry::load(&w.env).identities[&adm.0].clone();
    assert_eq!((rec.job.as_deref(), rec.lineage.last().map(|l| l.via.as_str())), (s.short_id.as_deref(), Some("migrate")));
    assert!(after.issues.is_empty(), "{:?}", after.issues);
}

/// 2c' / M13, widened to leads: a background lead whose conversation an SDK
/// host grabs reads `held` (never dup.live); start waits for the host to end.
#[test]
fn a_leads_conversation_held_by_an_sdk_host_is_held_not_dup_live() {
    let w = world();
    let adm = Id::admiral("MAIN");
    let snap = w.snap();
    let (cwd, sid) = { let m = w.member(&snap, &adm); (m.cwd.unwrap(), m.session.unwrap().session_id) };
    // the job is live: Claude Code refuses a second process on it (R-13)
    let host = vec![w.env.claude_bin.clone(), "-p".into(), "--resume".into(), sid.clone(), "--name".into(), adm.name(), "--settings".into(), claude::settings_json(&w.env, &adm, false)];
    assert_ne!(claude::run(&host, Some(&cwd)).0, 0, "refused while the job is live");
    let out = sessions::apply(&w.env, &sessions::plan_stop(&w.env, &snap, &adm, &Caller::User));
    assert!(out.verified, "{:?}", out.issues);
    assert_eq!(claude::run(&host, Some(&cwd)).0, 0);
    let snap = w.snap();
    let m = w.member(&snap, &adm);
    assert!(m.session.as_ref().unwrap().short_id.is_some(), "the job row stays the member's row");
    let held = m.issues.iter().find(|i| i.code == "held").expect("held");
    assert!(held.message.contains("sdk-cli") && held.fix.as_deref().unwrap().contains("end that process in its host"), "{held:?}");
    assert!(!has(&m.issues, "dup.live"), "{:?}", m.issues);
    assert_eq!(w.start(&snap, &adm, false).refusals[0].code, "held");
    // the host ends: a plain wake by id
    assert_eq!(w.fake(&["__end", &sid]).0, 0);
    let plan = w.start(&w.snap(), &adm, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert_eq!(w.steps_of(&plan, &adm)[0].cmd, claude::resume_bg(&w.env, &sid));
}

// ------------------------------------------------------------------ lead folders are exclusive (plan-v4 section 6)

/// I10: a live session in a lead's folder that is not the lead's job is
/// `lead.intruder` (Error) with the one fix that works: a background job is
/// stopped then removed; an interactive or SDK process is ended in its
/// terminal (named from the session file when it says). A retired tender's
/// row is `tender.retired` only. The intruder never holds up the lead's own
/// start.
#[test]
fn i10_a_live_session_in_a_lead_folder_is_an_intruder_with_one_fix_per_kind() {
    let w = world();
    let argos = Id::commodore("MAIN", "ARGOS");
    assert!(!has(&w.member(&w.snap(), &argos).issues, "lead.intruder"), "clean to begin with");
    // an unlabelled background job started by hand in ARGOS's folder
    let cwd = w.member(&w.snap(), &argos).cwd.unwrap();
    let (rc, out) = claude::run(&[w.env.native_claude_bin.clone(), "--bg".into(), "--name".into(), "quick look".into(), "x".into()], Some(&cwd));
    assert_eq!(rc, 0, "{out}");
    let snap = w.snap();
    let m = w.member(&snap, &argos);
    let i = m.issues.iter().find(|i| i.code == "lead.intruder").expect("lead.intruder: {m:?}");
    assert_eq!(i.severity, Severity::Error);
    let stray = snap.stowaways.iter().find(|s| s.name == "quick look").expect("still listed among the stowaways");
    let short = stray.short_id.clone().unwrap();
    assert!(i.message.contains(&short) && i.message.contains("a background job"), "{}", i.message);
    assert_eq!(i.fix.as_deref(), Some(format!("nimitz stop {short}  # then nimitz rm {short}").as_str()));
    // one issue per row: check lists it as the lead's intruder, not as a stowaway too
    let all = nimitz_core::check::check_tree(&w.env, &snap, Some(&argos));
    assert!(has(&all, "lead.intruder") && !all.iter().any(|i| i.code == "stowaway" && i.message.contains(&stray.session_id)), "{:?}", codes(&all));
    // the lead's own job is apart from it: its start (nothing to do) and its ships' starts go on
    let plan = w.start(&snap, &w.ship("STOPPED"), false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert!(plan.warnings.iter().any(|i| i.code == "lead.intruder"));
    // the fix works: stop, then rm
    let out = sessions::apply(&w.env, &sessions::plan_stop_session(&w.env, &snap, &short, &Caller::User));
    assert!(out.verified, "{:?}", out.issues);
    let out = sessions::apply(&w.env, &sessions::plan_rm(&w.env, &w.snap(), &short, &Caller::User));
    assert!(out.verified, "{:?}", out.issues);
    assert!(!has(&w.member(&w.snap(), &argos).issues, "lead.intruder"));
    // an interactive process in the admiral's folder: end it in its terminal
    let adm = Id::admiral("MAIN");
    let cwd = w.member(&w.snap(), &adm).cwd.unwrap();
    let (rc, _) = claude::run(&[w.env.claude_bin.clone(), "--name".into(), "a look around".into()], Some(&cwd));
    assert_eq!(rc, 0);
    let m = w.member(&w.snap(), &adm);
    let i = m.issues.iter().find(|i| i.code == "lead.intruder").expect("lead.intruder: {m:?}");
    assert!(i.message.contains("a look around") && i.message.contains("cli"), "{}", i.message);
    assert_eq!(i.fix.as_deref(), Some("end it in its terminal: /exit, or close the host"));
    assert!(!has(&m.issues, "dup.live") && !has(&m.issues, "held"), "{:?}", m.issues);
    // with the terminal named in its session file, the fix names it
    let sess = std::fs::read_dir(w.env.claude_home.join("sessions")).unwrap().flatten().map(|e| e.path()).find(|p| std::fs::read_to_string(p).unwrap_or_default().contains("a look around")).unwrap();
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&sess).unwrap()).unwrap();
    v["tmux"] = serde_json::json!("main:@1.%3");
    std::fs::write(&sess, v.to_string()).unwrap();
    let m = w.member(&w.snap(), &adm);
    let i = m.issues.iter().find(|i| i.code == "lead.intruder").unwrap();
    assert_eq!(i.fix.as_deref(), Some("end it in its terminal (tmux main:@1.%3): /exit, or close the host"));
    // a 0.3.x tender's row in a lead folder stays tender.retired only
    let t = w.rows().into_iter().find(|r| r.name == "[MAIN.ARGOS]-COMMODORE/fixci").unwrap();
    let (rc, _) = w.fake(&["respawn", t.short_id.as_deref().unwrap()]);
    assert_eq!(rc, 0);
    let snap = w.snap();
    assert!(!has(&w.member(&snap, &argos).issues, "lead.intruder"), "{:?}", w.member(&snap, &argos).issues);
    assert!(snap.issues.iter().any(|i| i.code == "tender.retired" && i.message.contains(t.short_id.as_deref().unwrap())));
}

// ------------------------------------------------------------------ name.drift from the job row (M19)

/// 3e / M19: `/rename` in an attached ship drifts the job's name (state.json)
/// and drops `--name` from its respawn flags, so a plain wake keeps the wrong
/// name. A stopped renamed job is `name.drift` too, and a start goes the
/// relabel way (rm the row, resume with every flag): the name comes back.
#[test]
fn m19_a_stopped_renamed_job_is_name_drift_and_a_start_restores_the_name() {
    let w = world();
    let id = w.ship("CLEAN");
    let s = w.member(&w.snap(), &id).session.unwrap();
    let short = s.short_id.clone().unwrap();
    assert_eq!(w.fake(&["__inside", &short, "rename", "sb1"]).0, 0);
    let m = w.member(&w.snap(), &id);
    let live_drift = m.issues.iter().find(|i| i.code == "name.drift").expect("live: name.drift");
    assert!(live_drift.message.contains("live as \"sb1\""), "{}", live_drift.message);
    assert_eq!(w.fake(&["stop", &short]).0, 0);
    let snap = w.snap();
    let m = w.member(&snap, &id);
    assert_eq!(m.status, Status::Stopped);
    let drift = m.issues.iter().find(|i| i.code == "name.drift").expect("stopped: name.drift too");
    assert!(drift.message.contains(&short) && drift.message.contains("\"sb1\"") && drift.message.contains("a wake would keep"), "{}", drift.message);
    assert_eq!(drift.fix.as_deref(), Some(format!("nimitz restart {id}").as_str()));
    // start repairs it: rm the row, then resume with every flag (same id, history, right name)
    let plan = w.start(&snap, &id, false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    let steps = w.steps_of(&plan, &id);
    assert_eq!(steps.len(), 2, "{steps:?}");
    assert!(steps[0].cmd.contains(&"rm".to_string()) && steps[1].cmd.contains(&"--name".to_string()), "{steps:?}");
    assert!(plan.warnings.iter().any(|i| i.code == "name.drift" && i.message.starts_with("will repair")), "{:?}", plan.warnings);
    let out = sessions::apply(&w.env, &plan);
    assert!(out.verified, "{:?}", out.issues);
    let after = w.member(&w.snap(), &id);
    let s2 = after.session.unwrap();
    assert_eq!((s2.name.as_str(), s2.session_id.as_str(), s2.short_id.as_deref()), (id.name().as_str(), s.session_id.as_str(), Some(short.as_str())));
    assert!(after.issues.is_empty(), "{:?}", after.issues);
}

// ------------------------------------------------------------------ review fixes (P2+P3 review)

/// A held parent (a 0.3.x interactive admiral still running) is live for the
/// chain: a ship's start and `up` go on with the warning riding along; only a
/// start of the admiral itself refuses.
#[test]
fn a_held_parent_does_not_block_the_chain() {
    let w = world();
    let adm = Id::admiral("MAIN");
    let snap = w.snap();
    let (cwd, sid) = { let m = w.member(&snap, &adm); (m.cwd.unwrap(), m.session.unwrap().session_id) };
    assert!(sessions::apply(&w.env, &sessions::plan_stop(&w.env, &snap, &adm, &Caller::User)).verified);
    let host = vec![w.env.claude_bin.clone(), "-p".into(), "--resume".into(), sid.clone(), "--name".into(), adm.name(), "--settings".into(), claude::settings_json(&w.env, &adm, false)];
    assert_eq!(claude::run(&host, Some(&cwd)).0, 0);
    let snap = w.snap();
    assert_eq!(w.start(&snap, &adm, false).refusals[0].code, "held", "the admiral itself");
    let plan = w.start(&snap, &w.ship("STOPPED"), false);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert!(plan.steps.iter().all(|s| s.id.as_ref() != Some(&adm)), "nothing for the admiral: {:?}", plan.steps);
    assert!(plan.warnings.iter().any(|i| i.code == "held" && i.id.as_ref() == Some(&adm)), "{:?}", plan.warnings);
    // a fleet's start: the held admiral is said (`lead.interactive`, nothing for it), the ships still come up
    let up = up(&w, &snap, "MAIN");
    assert!(up.refusals.iter().any(|i| i.code == "lead.interactive" && i.id.as_ref() == Some(&adm)), "{:?}", up.refusals);
    assert!(up.steps.iter().any(|s| s.id == Some(w.ship("STOPPED"))) && up.steps.iter().all(|s| s.id.as_ref() != Some(&adm)), "{:?}", up.steps);
}

/// Two crashed jobs in one plan wait out one window together, not one each.
#[test]
fn the_crash_window_is_waited_once_for_every_member_of_a_plan() {
    let w = world();
    let ids = [w.ship("CLEAN"), w.ship("DRIFT")];
    for id in &ids {
        let short = w.member(&w.snap(), id).session.unwrap().short_id.unwrap();
        assert_eq!(w.fake(&["__crash", &short]).0, 0);
    }
    let snap = w.snap();
    let mut plan = up(&w, &snap, "MAIN");
    plan.steps.retain(|s| ids.contains(s.id.as_ref().unwrap()));
    plan.refusals.clear();
    assert_eq!(plan.steps.iter().filter(|s| s.cmd.contains(&"--resume".to_string())).count(), 2, "{:?}", plan.steps);
    let env = Env { crash_window_s: 1.0, ..w.env.clone() };
    let t = std::time::Instant::now();
    let out = sessions::apply(&env, &plan);
    let waited = t.elapsed();
    assert!(waited >= std::time::Duration::from_millis(900) && waited < std::time::Duration::from_millis(1900), "one window for both, not two: {waited:?}");
    assert!(out.issues.iter().all(|i| i.severity != Severity::Error), "{:?}", out.issues);
    for id in &ids {
        assert!(w.member(&w.snap(), id).session.unwrap().pid.is_some(), "{id} woken");
    }
}

/// `--fresh` under a live job refuses: stop it first.
#[test]
fn fresh_on_a_live_member_says_stop_it_first() {
    let w = world();
    let id = w.ship("CLEAN");
    let plan = sessions::plan_start_with(&w.env, &w.snap(), &id, &Caller::User, sessions::StartOpts { force: false, fresh: true });
    assert!(!plan.ok() && plan.steps.is_empty());
    assert_eq!(plan.refusals[0].code, "session.live");
    assert_eq!(plan.refusals[0].fix.as_deref(), Some("nimitz stop MAIN/ARGOS/CLEAN, then nimitz start MAIN/ARGOS/CLEAN --fresh"));
}

/// A wake by attach keeps the relaunch table's checks: a stopped job under a
/// drifted name (or without labels) is not attached; `nimitz start` relabels
/// it first. A clean parked job attaches (and wakes).
#[test]
fn an_attach_never_wakes_a_job_the_start_would_relabel() {
    let w = world();
    let id = w.ship("CLEAN");
    let short = w.member(&w.snap(), &id).session.unwrap().short_id.unwrap();
    assert_eq!(w.fake(&["__inside", &short, "rename", "sb1"]).0, 0);
    assert_eq!(w.fake(&["stop", &short]).0, 0);
    let m = w.member(&w.snap(), &id);
    let e = nimitz_core::layout::attach_tab(&w.env, &m).unwrap_err();
    assert_eq!((e.code.as_str(), e.severity), ("name.drift", Severity::Error));
    assert_eq!(e.fix.as_deref(), Some(format!("nimitz start {id}  # relabels it, then nimitz open {id}").as_str()));
    let bare = w.member(&w.snap(), &w.ship("BARE"));
    assert_eq!(nimitz_core::layout::attach_tab(&w.env, &bare).unwrap_err().code, "labels.missing");
    let parked = w.member(&w.snap(), &w.ship("STOPPED"));
    let t = nimitz_core::layout::attach_tab(&w.env, &parked).expect("a clean stopped job attaches");
    assert!(t.wakes);
}

/// The two pollers `apply` uses around a wake: `row_gone_within` (the daemon's
/// rm lands a moment after `claude rm` returns) and `wait_settled` (the
/// daemon lists a stopped job as running for a moment).
#[test]
fn the_wake_pollers_return_at_once_when_there_is_nothing_to_wait_for() {
    let w = world();
    let snap = w.snap();
    let live = w.member(&snap, &w.ship("CLEAN")).session.unwrap().short_id.unwrap();
    let stopped = w.member(&snap, &w.ship("STOPPED")).session.unwrap().short_id.unwrap();
    let t = std::time::Instant::now();
    assert!(sessions::row_gone_within(&w.env, "00000000", 2.0), "no such row: gone at once");
    assert!(t.elapsed().as_secs_f64() < 1.0);
    let t = std::time::Instant::now();
    assert!(!sessions::row_gone_within(&w.env, &stopped, 0.4), "a row that stays");
    assert!(t.elapsed().as_secs_f64() >= 0.4);
    let t = std::time::Instant::now();
    sessions::wait_settled(&w.env, &stopped, 2.0);
    assert!(t.elapsed().as_secs_f64() < 1.5, "a stopped job is settled");
    let t = std::time::Instant::now();
    sessions::wait_settled(&w.env, &live, 0.5);
    assert!(t.elapsed().as_secs_f64() >= 0.5, "a running job is waited for, up to the limit");
}

