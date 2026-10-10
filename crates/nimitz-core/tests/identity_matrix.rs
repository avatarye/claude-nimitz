//! One test per row of the session-mapping coverage matrix in the identity
//! area (`docs/design/session-mapping/index.html`, round 2; plan-v4 sections
//! 2-4): the member is kept (same job), rebound (the conversation moved) or
//! flagged (`dup.job`, `held`), as the spec says. Each test names its matrix
//! row id. Everything runs through the sandbox's fake `claude`, which models
//! the job model the report measured (`sandbox.rs`).
use nimitz_core::env::Env;
use nimitz_core::types::*;
use nimitz_core::{claude, evidence, registry, sandbox, sessions, snapshot};
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
    fn member(&self, id: &Id) -> Member {
        let snap = self.snap();
        snap.fleets
            .iter()
            .flat_map(|f| std::iter::once(&f.admiral).chain(f.squadrons.iter().flat_map(|s| std::iter::once(&s.commodore).chain(s.ships.iter()))))
            .find(|m| &m.id == id)
            .cloned()
            .unwrap_or_else(|| panic!("no member {id}"))
    }
    fn rec(&self, id: &Id) -> IdentityRecord {
        registry::load(&self.env).identities.get(&id.0).cloned().unwrap_or_else(|| panic!("{id} not in the registry"))
    }
    /// The fake `claude` (the native binary), in `cwd`.
    fn claude(&self, args: &[&str], cwd: Option<&str>) -> (i32, String) {
        let mut cmd = vec![self.env.native_claude_bin.clone()];
        cmd.extend(args.iter().map(|a| a.to_string()));
        claude::run(&cmd, cwd)
    }
    fn claude_ok(&self, args: &[&str], cwd: Option<&str>) -> String {
        let (rc, out) = self.claude(args, cwd);
        assert_eq!(rc, 0, "claude {args:?}: {out}");
        out
    }
    fn job_file(&self, short: &str) -> Value {
        let p = self.base.join("claude_home/jobs").join(short).join("state.json");
        serde_json::from_slice(&std::fs::read(&p).unwrap_or_else(|_| panic!("no job row {short}"))).unwrap()
    }
    fn events(&self) -> Vec<Event> {
        evidence::events(&self.env)
    }
    fn last_start(&self, sid: &str) -> Event {
        self.events().into_iter().rev().find(|e| e.event == "SessionStart" && e.session_id.as_deref() == Some(sid)).expect("a SessionStart")
    }
    fn start(&self, id: &Id) -> Outcome {
        let snap = self.snap();
        let plan = sessions::plan_start(&self.env, &snap, id, &Caller::User, false);
        assert!(plan.ok(), "start {id}: {:?}", plan.refusals);
        let out = sessions::apply(&self.env, &plan);
        assert!(out.verified, "start {id}: {:?}", out.issues);
        out
    }
}

/// The live CLEAN ship: (id, job, session id, folder).
fn clean(w: &W) -> (Id, String, String, String) {
    let id = w.ship("CLEAN");
    let m = w.member(&id);
    let s = m.session.clone().unwrap();
    assert_eq!(m.status, Status::Idle);
    (id, s.short_id.unwrap(), s.session_id, m.cwd.unwrap())
}

fn lineage(rec: &IdentityRecord) -> Vec<(String, String)> {
    rec.lineage.iter().map(|l| (l.session_id.clone(), l.via.clone())).collect()
}

fn codes(m: &Member) -> Vec<&str> {
    m.issues.iter().map(|i| i.code.as_str()).collect()
}

// ------------------------------------------------------------------ stop, kill, respawn, native resume

#[test]
fn cli_01_claude_stop_keeps_the_job_and_the_member_reads_stopped() {
    let w = world();
    let (id, job, sid, _) = clean(&w);
    w.claude_ok(&["stop", &job], None);
    let m = w.member(&id);
    assert_eq!(m.status, Status::Stopped, "{:?}", m.reasons);
    assert!(m.reasons.iter().any(|r| r.contains("SessionEnd") && r.contains("other")), "{:?}", m.reasons);
    assert!(m.issues.is_empty(), "{:?}", m.issues);
    let rec = w.rec(&id);
    assert_eq!(rec.job.as_deref(), Some(job.as_str()), "the job stays the anchor");
    assert_eq!(rec.session.as_deref(), Some(sid.as_str()));
    assert_eq!(w.job_file(&job)["respawnFlags"].as_array().unwrap().len(), 6, "name, mode and labels kept for the next wake");
}

#[test]
fn cli_02_claude_kill_is_stop() {
    let w = world();
    let (id, job, _, _) = clean(&w);
    w.claude_ok(&["kill", &job], None);
    let m = w.member(&id);
    assert_eq!(m.status, Status::Stopped, "{:?}", m.reasons);
    assert_eq!(w.rec(&id).job.as_deref(), Some(job.as_str()));
}

#[test]
fn cli_04_claude_respawn_keeps_the_job_and_rebinds_nothing_new() {
    let w = world();
    let (id, job, sid, _) = clean(&w);
    let before = lineage(&w.rec(&id));
    w.claude_ok(&["respawn", &job], None);
    let m = w.member(&id);
    assert_eq!(m.status, Status::Idle, "{:?}", m.reasons);
    assert_eq!(m.session.as_ref().unwrap().short_id.as_deref(), Some(job.as_str()));
    let e = w.last_start(&sid);
    assert_eq!(e.source.as_deref(), Some("resume"));
    assert_eq!(e.job.as_deref(), Some(job.as_str()));
    assert_eq!(e.binding.as_deref(), Some("same"), "the same job on the same conversation: nothing to rebind");
    assert_eq!(lineage(&w.rec(&id)), before, "no lineage entry for a respawn");
    // and the stopped job respawns the same way
    w.claude_ok(&["stop", &job], None);
    w.claude_ok(&["respawn", &job], None);
    assert_eq!(w.member(&id).status, Status::Idle);
    assert_eq!(w.rec(&id).job.as_deref(), Some(job.as_str()));
}

#[test]
fn cli_08_native_bg_resume_wakes_the_same_job_with_its_labels() {
    let w = world();
    let (id, job, sid, cwd) = clean(&w);
    w.claude_ok(&["stop", &job], None);
    let out = w.claude_ok(&["--bg", "--resume", &sid], Some(&cwd));
    assert!(out.contains("woke session"), "{out}");
    let m = w.member(&id);
    assert_eq!(m.status, Status::Idle, "{:?}", m.reasons);
    let s = m.session.unwrap();
    assert_eq!((s.short_id.as_deref(), s.session_id.as_str()), (Some(job.as_str()), sid.as_str()));
    assert_eq!(s.labels.get("NIMITZ_ID").map(String::as_str), Some(id.0.as_str()));
    assert!(m.issues.is_empty(), "{:?}", m.issues);
    let rec = w.rec(&id);
    assert_eq!(rec.job.as_deref(), Some(job.as_str()));
    assert_eq!(lineage(&rec).last().map(|(_, v)| v.as_str()), Some("launch"), "same conversation: no new lineage entry");
}

#[test]
fn n10_nimitz_start_of_a_stopped_ship_is_the_native_resume_and_records_resume() {
    let w = world();
    let (id, job, sid, _) = clean(&w);
    w.claude_ok(&["stop", &job], None);
    let out = w.start(&id);
    assert!(out.ran.iter().any(|(s, _, _)| s.cmd.contains(&"--resume".to_string()) && !s.cmd.contains(&"--settings".to_string())), "{:?}", out.ran);
    let rec = w.rec(&id);
    assert_eq!((rec.job.as_deref(), rec.session.as_deref()), (Some(job.as_str()), Some(sid.as_str())));
    assert_eq!(lineage(&rec), vec![(sid.clone(), "launch".into())], "a wake of the conversation it already runs keeps how it was born");
    assert_eq!(w.member(&id).status, Status::Idle);
}

#[test]
fn inside_3_4d_then_e1_a_branch_then_a_retire_is_parked() {
    let w = world();
    let (id, job, _, _) = clean(&w);
    let out = w.claude_ok(&["__inside", &job, "branch", "b1"], None);
    let new = out.split_whitespace().last().unwrap().to_string();
    w.claude_ok(&["__inside", &job, "rename", &id.name()], None); // the branch named the session
    w.claude_ok(&["__retire", &job], None);
    let m = w.member(&id);
    assert_eq!(m.status, Status::Parked, "{:?}", m.reasons);
    assert!(m.reasons.iter().any(|r| r.contains("daemon retired it")), "{:?}", m.reasons);
    assert!(m.issues.is_empty(), "{:?}", m.issues);
    // a start continues the branch: the registry's conversation is the branch,
    // and a native wake would bring the pre-branch conversation back
    // (`resumeSessionId` follows /clear only), so the row goes and the branch
    // is resumed with every flag under its own short id (acceptance inside FAIL-1)
    assert_eq!(w.rec(&id).session.as_deref(), Some(new.as_str()));
    let out = w.start(&id);
    assert!(out.ran.iter().any(|(s, _, _)| s.cmd.contains(&"--settings".to_string()) && s.cmd.contains(&new)), "{:?}", out.ran);
    let s = w.member(&id).session.unwrap();
    assert_eq!((s.short_id.as_deref(), s.session_id.as_str()), (Some(&new[..8]), new.as_str()));
    let job = new[..8].to_string();
    // a branch, then a stop: stopped (the SessionEnd came from the job's own pid)
    w.claude_ok(&["__inside", &job, "branch", "b2"], None);
    w.claude_ok(&["stop", &job], None);
    let m = w.member(&id);
    assert_eq!(m.status, Status::Stopped, "{:?}", m.reasons);
}

#[test]
fn inside_3_4d_then_cli_30_a_branch_then_a_crash_is_parked() {
    let w = world();
    let (id, job, _, _) = clean(&w);
    w.claude_ok(&["__inside", &job, "branch", "b1"], None);
    w.claude_ok(&["__crash", &job], None);
    let m = w.member(&id);
    assert_eq!(m.status, Status::Parked, "{:?}", m.reasons);
    assert!(m.reasons.iter().any(|r| r.contains("state crashed")), "{:?}", m.reasons);
    // the branch named the job "b1" (3.4c): that is name.drift, stopped or not (M19); nothing else
    assert_eq!(codes(&m), ["name.drift"], "{:?}", m.issues);
}

#[test]
fn e1_an_old_retire_line_does_not_name_a_later_crash() {
    let w = world();
    let (id, job, _, _) = clean(&w);
    w.claude_ok(&["__retire", &job], None);
    assert!(w.member(&id).reasons.iter().any(|r| r.contains("daemon retired it")));
    w.start(&id);
    w.claude_ok(&["__crash", &job], None);
    let m = w.member(&id);
    assert_eq!(m.status, Status::Parked, "{:?}", m.reasons);
    assert!(m.reasons.iter().any(|r| r.contains("state crashed")), "the retire line is older than the last start: {:?}", m.reasons);
    assert!(!m.reasons.iter().any(|r| r.contains("daemon retired it")), "{:?}", m.reasons);
}

#[cfg(unix)]
#[test]
fn a_registry_that_cannot_be_read_is_never_written_over_by_the_hook() {
    use std::os::unix::fs::PermissionsExt;
    let w = world();
    let (id, job, sid, cwd) = clean(&w);
    let before = std::fs::read_to_string(w.env.registry_path()).unwrap();
    let n = registry::load(&w.env).identities.len();
    assert!(n > 5);
    std::fs::set_permissions(w.env.registry_path(), std::fs::Permissions::from_mode(0o000)).unwrap();
    // the hook runs (a respawn fires a SessionStart) and must not wipe the file
    w.claude_ok(&["respawn", &job], None);
    assert!(registry::update(&w.env, |_| ()).is_err(), "an unreadable registry is an error, never a fresh one");
    std::fs::set_permissions(w.env.registry_path(), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(std::fs::read_to_string(w.env.registry_path()).unwrap(), before, "the file is untouched");
    assert_eq!(registry::load(&w.env).identities.len(), n);
    let e = w.last_start(&sid);
    assert_eq!(e.binding.as_deref(), Some("none"), "the line says the write was lost");
    // readable again: status replays the line (nothing new here: the same job and conversation)
    let m = w.member(&id);
    assert_eq!(m.session.as_ref().unwrap().short_id.as_deref(), Some(job.as_str()));
    assert_eq!(m.status, Status::Idle);
    let _ = cwd;
}

// ------------------------------------------------------------------ retire, pin, crash

#[test]
fn e1_an_idle_retire_parks_the_job_and_a_start_wakes_it() {
    let w = world();
    let (id, job, sid, _) = clean(&w);
    w.claude_ok(&["__retire", &job], None);
    let m = w.member(&id);
    assert_eq!(m.status, Status::Parked, "{:?}", m.reasons);
    assert!(m.reasons.iter().any(|r| r.contains("daemon retired it") && r.contains("idle 61m")), "{:?}", m.reasons);
    assert!(m.issues.is_empty(), "parked is not an error: {:?}", m.issues);
    assert!(!m.session.as_ref().unwrap().pinned);
    // check shows parked and no error
    let issues = nimitz_core::check::check(&w.env, &w.snap(), Some(&id));
    assert!(issues.iter().all(|i| i.severity != Severity::Error), "{issues:?}");
    // start wakes it by id: same job, same conversation
    w.start(&id);
    let m = w.member(&id);
    assert_eq!(m.status, Status::Idle);
    let s = m.session.unwrap();
    assert_eq!((s.short_id.as_deref(), s.session_id.as_str()), (Some(job.as_str()), sid.as_str()));
}

#[test]
fn e2_a_pinned_job_shows_pinned() {
    let w = world();
    let (id, job, _, _) = clean(&w);
    w.claude_ok(&["__pin", &job], None);
    let m = w.member(&id);
    assert!(m.session.as_ref().unwrap().pinned);
    assert!(m.reasons.iter().any(|r| r.contains("pinned")), "{:?}", m.reasons);
    w.claude_ok(&["__unpin", &job], None);
    assert!(!w.member(&id).session.unwrap().pinned);
}

#[test]
fn cli_30_a_dead_worker_is_parked_not_an_error_and_a_restart_keeps_the_job() {
    let w = world();
    let (id, job, sid, _) = clean(&w);
    w.claude_ok(&["__crash", &job], None);
    let m = w.member(&id);
    assert_eq!(m.status, Status::Parked, "{:?}", m.reasons);
    assert!(m.reasons.iter().any(|r| r.contains("state crashed")), "{:?}", m.reasons);
    assert!(m.issues.is_empty(), "{:?}", m.issues);
    // the daemon restarts it (same id, SessionStart resume with labels)
    w.claude_ok(&["respawn", &job], None);
    let m = w.member(&id);
    assert_eq!(m.status, Status::Idle);
    assert_eq!(m.session.as_ref().unwrap().session_id, sid);
    assert_eq!(w.rec(&id).job.as_deref(), Some(job.as_str()));
}

// ------------------------------------------------------------------ /clear, /branch, /resume, /rename

#[test]
fn cli_20_clear_in_an_attached_ship_keeps_the_job_and_rebinds_the_conversation() {
    let w = world();
    let (id, job, old, _) = clean(&w);
    let out = w.claude_ok(&["__inside", &job, "clear"], None);
    let new = out.split_whitespace().last().unwrap().to_string();
    assert_ne!(new, old);
    // the member is live on the same job, on the new conversation
    let m = w.member(&id);
    assert_eq!(m.status, Status::Idle, "{:?}", m.reasons);
    let s = m.session.as_ref().unwrap();
    assert_eq!(s.short_id.as_deref(), Some(job.as_str()));
    assert_eq!(s.session_id, new);
    assert_eq!(s.birth_id.as_deref(), Some(old.as_str()), "the job's birth id is kept");
    assert!(m.extra_rows.is_empty(), "no copy: {:?}", m.extra_rows);
    assert!(m.issues.is_empty(), "{:?}", m.issues);
    // the registry followed: job kept, session moved, lineage +clear
    let rec = w.rec(&id);
    assert_eq!((rec.job.as_deref(), rec.session.as_deref()), (Some(job.as_str()), Some(new.as_str())));
    assert_eq!(lineage(&rec), vec![(old.clone(), "launch".into()), (new.clone(), "clear".into())]);
    assert_eq!(w.last_start(&new).binding.as_deref(), Some("bound"));
    assert_eq!(w.job_file(&job)["resumeSessionId"], json!(new), "a stop/start resumes the post-clear conversation");
    // a stop and a start keep the job on the post-clear conversation
    w.claude_ok(&["stop", &job], None);
    assert_eq!(w.member(&id).status, Status::Stopped);
    w.start(&id);
    let s = w.member(&id).session.unwrap();
    assert_eq!((s.short_id.as_deref(), s.session_id.as_str()), (Some(job.as_str()), new.as_str()));
}

#[test]
fn inside_3_4d_branch_in_an_attached_ship_keeps_the_job_and_binds_via_branch() {
    let w = world();
    let (id, job, old, _) = clean(&w);
    let out = w.claude_ok(&["__inside", &job, "branch", "b1"], None);
    let new = out.split_whitespace().last().unwrap().to_string();
    let m = w.member(&id);
    assert_eq!(m.status, Status::Idle, "{:?}", m.reasons);
    let s = m.session.as_ref().unwrap();
    assert_eq!((s.short_id.as_deref(), s.session_id.as_str()), (Some(job.as_str()), new.as_str()));
    assert!(m.extra_rows.is_empty());
    // the branch name became the session name: name.drift (M19), nothing else
    assert_eq!(codes(&m), vec!["name.drift"], "{:?}", m.issues);
    let rec = w.rec(&id);
    assert_eq!((rec.job.as_deref(), rec.session.as_deref()), (Some(job.as_str()), Some(new.as_str())));
    assert_eq!(lineage(&rec).last().cloned(), Some((new.clone(), "branch".into())));
    assert_eq!(w.job_file(&job)["resumeSessionId"], json!(old), "Claude Code's row still points at the pre-branch conversation");
    // `/resume <old>` returns: same job, lineage +resume, no drift
    w.claude_ok(&["__inside", &job, "resume", &old], None);
    let m = w.member(&id);
    assert_eq!(m.session.as_ref().unwrap().session_id, old);
    assert_eq!(lineage(&w.rec(&id)).last().cloned(), Some((old.clone(), "resume".into())));
}

#[test]
fn inside_3_4b_rename_is_name_drift_with_a_relaunch_fix() {
    let w = world();
    let (id, job, _, _) = clean(&w);
    w.claude_ok(&["__inside", &job, "rename", "RENAMED"], None);
    let m = w.member(&id);
    let i = m.issues.iter().find(|i| i.code == "name.drift").expect("name.drift");
    assert!(i.fix.as_deref().unwrap_or("").contains("relaunch"));
    assert_eq!(w.job_file(&job)["respawnFlags"][1], json!("RENAMED"), "/rename rewrites --name in respawnFlags (cli-19)");
    assert_eq!(w.rec(&id).job.as_deref(), Some(job.as_str()), "the job is kept");
}

// ------------------------------------------------------------------ rm + resume, adopt

#[test]
fn cli_03_claude_rm_then_a_resume_is_a_new_job_named_by_the_short_resumed_id_and_rebinds() {
    let w = world();
    let (id, job, sid, cwd) = clean(&w);
    w.claude_ok(&["stop", &job], None);
    w.claude_ok(&["rm", &job], None);
    let m = w.member(&id);
    assert_eq!(m.status, Status::Stopped, "{:?}", m.reasons);
    assert!(m.reasons.iter().any(|r| r.contains("transcript")), "{:?}", m.reasons);
    // nimitz start resumes with every flag (row 5): Claude Code makes a job named by the short id
    let out = w.start(&id);
    assert!(out.ran.iter().any(|(s, _, _)| s.cmd.contains(&"--resume".to_string()) && s.cmd.contains(&"--settings".to_string())), "{:?}", out.ran);
    let m = w.member(&id);
    assert_eq!(m.status, Status::Idle);
    let s = m.session.unwrap();
    assert_eq!(s.session_id, sid, "history kept");
    assert_eq!(s.short_id.as_deref(), Some(&sid[..8]), "the new job is the short form of the resumed id");
    let rec = w.rec(&id);
    assert_eq!(rec.job.as_deref(), Some(&sid[..8]), "rebound: the old job's row is gone");
    // the same short id (the short form of the birth id) on the same conversation: one entry, born by launch
    assert_eq!(lineage(&rec), vec![(sid.clone(), "launch".into())]);
    let _ = cwd;
}

#[test]
fn e11_rm_then_a_by_hand_resume_of_the_post_clear_id_rebinds_to_the_new_job() {
    let w = world();
    let (id, job, _, cwd) = clean(&w);
    let out = w.claude_ok(&["__inside", &job, "clear"], None);
    let new = out.split_whitespace().last().unwrap().to_string();
    w.claude_ok(&["stop", &job], None);
    w.claude_ok(&["rm", &job], None);
    // by hand: resume the post-clear id with the labels; the old job dir is gone, so the hook rebinds
    let settings = claude::settings_json(&w.env, &id, true);
    w.claude_ok(&["--bg", "--resume", &new, "--name", &id.name(), "--settings", &settings], Some(&cwd));
    let m = w.member(&id);
    assert_eq!(m.status, Status::Idle, "{:?}", m.reasons);
    assert_eq!(m.session.as_ref().unwrap().short_id.as_deref(), Some(&new[..8]));
    assert_eq!(w.rec(&id).job.as_deref(), Some(&new[..8]));
    assert_eq!(w.last_start(&new).binding.as_deref(), Some("bound"));
}

#[test]
fn n25_adopt_binds_the_stowaways_job_and_a_relaunch_keeps_it() {
    let w = world();
    let snap = w.snap();
    let stow = snap.stowaways.iter().find(|s| s.name == "quick look at the logs").unwrap().clone();
    let id = w.ship("FRESH");
    let plan = sessions::plan_adopt(&w.env, &snap, &stow.session_id, &id, &Caller::User);
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert!(sessions::apply(&w.env, &plan).verified);
    let rec = w.rec(&id);
    assert_eq!(rec.job, stow.short_id, "the adopted session's job is the anchor");
    assert_eq!(lineage(&rec).last().cloned(), Some((stow.session_id.clone(), "adopt".into())));
    let m = w.member(&id);
    assert_eq!(m.session.as_ref().unwrap().short_id, stow.short_id);
    assert!(codes(&m).contains(&"labels.missing"), "{:?}", m.issues);
}

// ------------------------------------------------------------------ two jobs, one identity

#[test]
fn inside_3_8_a_fork_is_a_second_job_with_the_labels_dup_job_and_the_member_is_kept() {
    let w = world();
    let (id, job, sid, _) = clean(&w);
    let out = w.claude_ok(&["__inside", &job, "fork"], None);
    let copy = out.split_whitespace().last().unwrap().to_string();
    assert_ne!(copy, job);
    // the registry keeps the member's own job
    let rec = w.rec(&id);
    assert_eq!((rec.job.as_deref(), rec.session.as_deref()), (Some(job.as_str()), Some(sid.as_str())));
    assert_eq!(lineage(&rec).len(), 1);
    // the copy's SessionStart carries the labels and is dup.job evidence
    let e = w.events().into_iter().rev().find(|e| e.event == "SessionStart" && e.job.as_deref() == Some(copy.as_str())).expect("the fork's start");
    assert_eq!(e.binding.as_deref(), Some("dup.job"));
    assert_eq!(e.labels.get("NIMITZ_ID").map(String::as_str), Some(id.0.as_str()));
    // the member's row is its job; the copy is an extra live row: dup.live, never the primary
    let m = w.member(&id);
    assert_eq!(m.session.as_ref().unwrap().short_id.as_deref(), Some(job.as_str()));
    assert_eq!(m.extra_rows.len(), 1);
    assert_eq!(m.extra_rows[0].short_id.as_deref(), Some(copy.as_str()));
    assert!(codes(&m).contains(&"dup.live"), "{:?}", m.issues);
    // stop and remove the copy: clean again, the job unchanged
    w.claude_ok(&["stop", &copy], None);
    w.claude_ok(&["rm", &copy], None);
    let m = w.member(&id);
    assert!(m.issues.is_empty(), "{:?}", m.issues);
    assert_eq!(w.rec(&id).job.as_deref(), Some(job.as_str()));
}

#[test]
fn cli_10_a_flagged_resume_copy_with_the_labels_is_dup_job_while_the_row_exists() {
    let w = world();
    let (id, job, sid, cwd) = clean(&w);
    w.claude_ok(&["stop", &job], None);
    let settings = claude::settings_json(&w.env, &id, true);
    let out = w.claude_ok(&["--bg", "--resume", &sid, "--name", &id.name(), "--settings", &settings], Some(&cwd));
    assert!(out.contains("started a copy"), "{out}");
    let copy = out.split_whitespace().last().unwrap().to_string();
    assert_eq!(w.rec(&id).job.as_deref(), Some(job.as_str()), "the stopped job keeps the identity");
    let e = w.events().into_iter().rev().find(|e| e.event == "SessionStart" && e.job.as_deref() == Some(copy.as_str())).unwrap();
    assert_eq!(e.binding.as_deref(), Some("dup.job"));
    let m = w.member(&id);
    assert_eq!(m.session.as_ref().unwrap().short_id.as_deref(), Some(job.as_str()));
    assert_eq!(m.status, Status::Stopped);
    // the copy is a live extra row: a start refuses (dup.live) until it is stopped
    let plan = sessions::plan_start(&w.env, &w.snap(), &id, &Caller::User, false);
    assert!(plan.refusals.iter().any(|i| i.code == "dup.live"), "{:?}", plan.refusals);
}

#[test]
fn cli_09_a_resume_of_a_live_job_is_a_copy_under_an_auto_name_and_the_member_stays_live() {
    let w = world();
    let (id, job, sid, cwd) = clean(&w);
    // a `done` job whose process is alive is a live idle member, not parked, not stopped
    let p = w.base.join("claude_home/jobs").join(&job).join("state.json");
    let mut j: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    j["state"] = json!("done");
    std::fs::write(&p, j.to_string()).unwrap();
    assert_eq!(w.member(&id).status, Status::Idle);
    // `--bg --resume <id> --name X` against it: a copy, the name ignored (2.1.292)
    let (rc, out) = w.claude(&["--bg", "--resume", &sid, "--name", &id.name()], Some(&cwd));
    assert_eq!(rc, 0, "{out}");
    assert!(out.contains("started a copy") && out.contains("--name is ignored"), "{out}");
    let copy = out.lines().find(|l| l.contains("started a copy")).unwrap().split_whitespace().last().unwrap().to_string();
    let m = w.member(&id);
    assert_eq!(m.status, Status::Idle, "{:?}", m.reasons);
    assert_eq!(m.session.as_ref().unwrap().short_id.as_deref(), Some(job.as_str()), "the member keeps its job");
    assert_eq!(w.rec(&id).job.as_deref(), Some(job.as_str()));
    let row = w.job_file(&copy);
    assert_ne!(row["name"], json!(id.name()), "an auto-generated name, not --name");
    assert!(row["name"].as_str().unwrap().starts_with(&id.name()), "{}", row["name"]);
    assert_eq!(row["forkedFrom"], json!(sid));
    assert_eq!(row["respawnFlags"], json!([]), "a copy of a running job carries no flags (cli-09)");
    // the copy is unlabelled (no --settings): a copy row of the member, never its session
    assert!(m.extra_rows.iter().any(|r| r.short_id.as_deref() == Some(copy.as_str())), "{:?}", m.extra_rows);
}

// ------------------------------------------------------------------ held, other home, unlabelled, M21

#[test]
fn r_12_an_sdk_host_on_the_members_conversation_is_held_and_the_job_stays_the_row() {
    let w = world();
    let (id, job, sid, cwd) = clean(&w);
    w.claude_ok(&["stop", &job], None);
    let settings = claude::settings_json(&w.env, &id, false);
    w.claude_ok(&["-p", "--resume", &sid, "--name", &id.name(), "--settings", &settings], Some(&cwd));
    let e = w.last_start(&sid);
    assert_eq!(e.binding.as_deref(), Some("held"));
    assert_eq!(e.entrypoint.as_deref(), Some("sdk-cli"));
    assert_eq!(e.job, None);
    let rec = w.rec(&id);
    assert_eq!(rec.job.as_deref(), Some(job.as_str()), "not rebound");
    let m = w.member(&id);
    assert_eq!(m.session.as_ref().unwrap().short_id.as_deref(), Some(job.as_str()), "the member's row is its job, not the holder");
    assert_eq!(m.status, Status::Stopped, "{:?} {:?}", m.reasons, m.session);
    let held = m.issues.iter().find(|i| i.code == "held").expect("held");
    assert_eq!(held.severity, Severity::Warn);
    assert!(held.message.contains("sdk-cli") && held.fix.as_deref().unwrap_or("").contains("nimitz start"), "{held:?}");
    assert!(!codes(&m).contains(&"copy.rows") && !codes(&m).contains(&"dup.live"), "{:?}", m.issues);
    // the holder ends: clean stopped member, a native resume brings it back
    w.claude_ok(&["__end", &sid], None);
    let m = w.member(&id);
    assert!(m.issues.is_empty(), "{:?}", m.issues);
    w.start(&id);
    assert_eq!(w.member(&id).session.unwrap().short_id.as_deref(), Some(job.as_str()));
}

#[test]
fn cli_28_an_unlabelled_session_is_logged_only_and_binds_nothing() {
    let w = world();
    let (id, job, _, cwd) = clean(&w);
    let before = registry::load(&w.env);
    w.claude_ok(&["--bg", "--name", "scratch", "--settings", "{}", "hi"], Some(&cwd));
    let e = w.events().into_iter().rev().find(|e| e.event == "SessionStart" && e.labels.is_empty()).expect("logged");
    assert_eq!(e.binding, None);
    assert!(e.job.is_some(), "the line still says which job");
    let after = registry::load(&w.env);
    assert_eq!(after.identities, before.identities, "the registry did not move");
    assert_eq!(w.member(&id).session.unwrap().short_id.as_deref(), Some(job.as_str()));
}

#[test]
fn cli_17_m21_a_session_end_from_a_pid_without_a_session_start_is_ignored() {
    let w = world();
    let (id, _, sid, cwd) = clean(&w);
    // a refused `-p --resume` of the live job fires a SessionEnd with no labels
    let (rc, out) = w.claude(&["-p", "--resume", &sid, "hi"], Some(&cwd));
    assert_eq!(rc, 1, "{out}");
    let end = w.events().into_iter().rev().find(|e| e.event == "SessionEnd").expect("the stray end");
    assert_eq!(end.session_id.as_deref(), Some(sid.as_str()));
    assert!(end.labels.is_empty());
    // the member is untouched: live, labelled, no issue
    let m = w.member(&id);
    assert_eq!(m.status, Status::Idle, "{:?}", m.reasons);
    assert!(m.issues.is_empty(), "{:?}", m.issues);
    assert_eq!(m.session.as_ref().unwrap().labels.get("NIMITZ_ID").map(String::as_str), Some(id.0.as_str()));
    // and once retired, that stray end does not make it `stopped`
    w.claude_ok(&["__retire", m.session.as_ref().unwrap().short_id.as_deref().unwrap()], None);
    assert_eq!(w.member(&id).status, Status::Parked);
}

// ------------------------------------------------------------------ the registry itself

#[test]
fn a_v1_registry_migrates_on_first_load_and_the_file_is_kept_on_the_first_write() {
    let w = world();
    let (id, job, sid, _) = clean(&w);
    // rewrite the registry as 0.3.x wrote it
    let p = w.env.registry_path();
    let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["version"] = json!(1);
    v.as_object_mut().unwrap().remove("updated_at");
    for (_, rec) in v["identities"].as_object_mut().unwrap() {
        let r = rec.as_object_mut().unwrap();
        if r.contains_key("current") {
            continue; // the sandbox's 0.3.x tender entry is v1 already
        }
        let cur = r.remove("session").unwrap_or(Value::Null);
        r.remove("job");
        r.insert("current".into(), cur);
        for l in r["lineage"].as_array_mut().unwrap() {
            let at = l["at"].clone();
            let lo = l.as_object_mut().unwrap();
            lo.remove("at");
            lo.remove("job");
            lo.insert("bound_at".into(), at);
        }
    }
    std::fs::write(&p, v.to_string()).unwrap();
    // a read migrates in memory and writes nothing
    let r = registry::load(&w.env);
    assert_eq!(r.version, REGISTRY_VERSION);
    let rec = &r.identities[&id.0];
    assert_eq!((rec.job.as_deref(), rec.session.as_deref()), (Some(job.as_str()), Some(sid.as_str())));
    assert_eq!(rec.lineage[0].job.as_deref(), Some(job.as_str()));
    assert!(std::fs::read_to_string(&p).unwrap().contains("\"current\""), "a read never writes");
    assert!(!registry::v1_path(&w.env).exists());
    assert!(registry::health(&w.env).is_empty());
    // the next locked update saves v2 and keeps the v1 file
    registry::update(&w.env, |_| ()).unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    assert_eq!(saved["version"], json!(2));
    assert_eq!(saved["identities"][&id.0]["job"], json!(job));
    assert_eq!(saved["identities"][&id.0]["session"], json!(sid));
    assert!(registry::v1_path(&w.env).is_file());
    assert!(std::fs::read_to_string(registry::v1_path(&w.env)).unwrap().contains("\"current\""));
    // status reads the migrated shape the same way
    assert_eq!(w.member(&id).session.unwrap().short_id.as_deref(), Some(job.as_str()));
}

#[test]
fn status_replays_a_session_start_whose_registry_write_was_lost() {
    let w = world();
    let (id, job, old, _) = clean(&w);
    // a /clear whose hook could not write the registry: the line is there, the registry is behind
    let out = w.claude_ok(&["__inside", &job, "clear"], None);
    let new = out.split_whitespace().last().unwrap().to_string();
    registry::update(&w.env, |r| {
        let rec = r.identities.get_mut(&id.0).unwrap();
        rec.session = Some(old.clone());
        rec.lineage.truncate(1);
    })
    .unwrap();
    // the line is older than the registry now: make it newer, as a lost write leaves it
    let p = w.env.events_path();
    let raw = std::fs::read_to_string(&p).unwrap();
    let bumped: Vec<String> = raw
        .lines()
        .map(|l| {
            let mut v: Value = serde_json::from_str(l).unwrap();
            if v["session_id"] == json!(new) && v["event"] == json!("SessionStart") {
                v["ts"] = json!(evidence::now() + 5.0);
                v["binding"] = json!("none");
            }
            v.to_string()
        })
        .collect();
    std::fs::write(&p, format!("{}\n", bumped.join("\n"))).unwrap();
    assert_eq!(w.rec(&id).session.as_deref(), Some(old.as_str()));
    let m = w.member(&id); // status
    assert_eq!(m.session.as_ref().unwrap().session_id, new);
    let rec = w.rec(&id);
    assert_eq!(rec.session.as_deref(), Some(new.as_str()), "replayed under the lock");
    assert_eq!(lineage(&rec).last().cloned(), Some((new.clone(), "clear".into())));
    // nothing more to replay: a second status writes nothing
    let stamp = registry::load(&w.env).updated_at;
    let _ = w.snap();
    assert_eq!(registry::load(&w.env).updated_at, stamp);
}

#[test]
fn cli_l1_labels_are_kept_per_pid_so_a_stray_process_does_not_strip_the_member() {
    // unit: the member's own process (pid 7) started labelled; a `-p --resume`
    // of its conversation (pid 9) started with none; the live row (pid 7) keeps its labels
    let ev = |pid: i64, labelled: bool| Event {
        ts: pid as f64,
        event: "SessionStart".into(),
        session_id: Some("s1".into()),
        pid: Some(pid),
        labels: if labelled { [("NIMITZ_ID".to_string(), "F/SQ/S".to_string()), ("NIMITZ_ROLE".to_string(), "ship".to_string())].into() } else { Default::default() },
        ..Default::default()
    };
    let events = vec![ev(7, true), ev(9, false)];
    let row = |pid: Option<i64>, job: bool| evidence::SessionRow {
        session_id: "s1".into(),
        name: "[F.SQ]-S".into(),
        cwd: "/w".into(),
        kind: "background".into(),
        short_id: job.then(|| "j1".to_string()),
        pid,
        state: None,
        status: None,
        status_since: None,
        started_at: None,
        waiting_for: None,
        // the member's launch saved its labels (0.4.0: a job's labels are those)
        job_labels: if job { [("NIMITZ_ID".to_string(), "F/SQ/S".to_string()), ("NIMITZ_ROLE".to_string(), "ship".to_string())].into() } else { Default::default() },
        unreadable: false,
        forked_from: None,
        bridge_session_id: None,
        birth_id: None,
        entrypoint: None,
        pinned: false,
        spare: false,
        terminal: None,
        appeared_at: None,
    };
    let labels = evidence::session_labels(&events, &[row(Some(7), true)]);
    assert_eq!(labels["s1"].get("NIMITZ_ID").map(String::as_str), Some("F/SQ/S"));
    // the stray process (a `-p --resume`, no job) reads unlabelled
    let labels = evidence::session_labels(&events, &[row(Some(9), false)]);
    assert!(labels["s1"].is_empty());
    // a stopped row with no job and no saved labels: the newest SessionStart decides (as before)
    let labels = evidence::session_labels(&events, &[row(None, false)]);
    assert!(labels["s1"].is_empty());
}
