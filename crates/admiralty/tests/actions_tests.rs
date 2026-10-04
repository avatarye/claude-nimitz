mod common;

use admiralty::actions::{self, SETTINGS_NO_ISOLATION};
use admiralty::herdr;
use common::*;

fn arg_after<'a>(cmd: &'a [String], flag: &str) -> &'a str {
    &cmd[cmd.iter().position(|a| a == flag).unwrap() + 1]
}

#[test]
fn activate_resumes_newest_row_for_name_and_dir() {
    let sb = sandbox();
    let s = snap(&sb);
    let a = s.agent("ALPHA/[ALPHA]-ARCHIVE").unwrap();
    let p = actions::plan_activate(&sb.env, a);
    assert!(p.ok);
    assert_eq!(arg_after(&p.cmd, "--resume"), a.primary.as_ref().unwrap().session_id);
    assert_eq!(arg_after(&p.cmd, "--name"), "[ALPHA]-ARCHIVE");
    assert_eq!(arg_after(&p.cmd, "--permission-mode"), "bypassPermissions");
    assert!(p.cmd.iter().any(|c| c == SETTINGS_NO_ISOLATION));
    assert_eq!(p.cwd.as_deref(), a.declared.as_ref().unwrap().dir.as_deref());
}

#[test]
fn activate_cold_uses_prompt_lead_line_and_mode() {
    let sb = sandbox();
    let s = snap(&sb);
    let p = actions::plan_activate(&sb.env, s.agent("ALPHA/[ALPHA]-BEACON").unwrap());
    assert!(p.ok && !p.cmd.iter().any(|c| c == "--resume"));
    let last = p.cmd.last().unwrap();
    assert!(last.starts_with("You are [ALPHA]-BEACON") && last.contains("[ALPHA]-LEAD"));
    assert_eq!(arg_after(&p.cmd, "--permission-mode"), "auto");
}

#[test]
fn activate_refusals() {
    let sb = sandbox();
    let s = snap(&sb);
    let plan = |k: &str| actions::plan_activate(&sb.env, s.agent(k).unwrap());
    assert!(!plan("ALPHA/LEAD").ok);
    assert!(!plan("ALPHA/[ALPHA]-SCOUT").ok);
    assert!(!plan("ALPHA/[ALPHA]-NOPROMPT").ok);
    let g = plan("BRAVO/[BRAVO]-GHOST");
    assert!(!g.ok && g.summary.contains("does not exist"));
}

#[test]
fn activate_runs_fake_claude_and_agent_goes_live() {
    let sb = sandbox();
    let s = snap(&sb);
    let p = actions::plan_activate(&sb.env, s.agent("ALPHA/[ALPHA]-ARCHIVE").unwrap());
    let (rc, out) = actions::run_detached(&sb.env, &p);
    assert_eq!(rc, 0, "{out}");
    let calls = lines(&sb.base.join("calls.log"));
    let last = calls.last().unwrap();
    assert!(last["args"].as_array().unwrap().iter().any(|a| a == "--bg"));
    assert!(last["cwd"].as_str().unwrap().ends_with("alpha/archive"));
    let log = lines(&sb.env.action_log);
    assert_eq!(log.last().unwrap()["action"], "activate");
    assert_eq!(log.last().unwrap()["rc"], 0);
    let after = snap(&sb);
    let a = after.agent("ALPHA/[ALPHA]-ARCHIVE").unwrap();
    assert!(a.live() && a.stale.len() == 3);
}

#[test]
fn attach_plans() {
    let sb = sandbox();
    let s = snap(&sb);
    let forge = s.agent("ALPHA/[ALPHA]-FORGE").unwrap();
    let p = actions::plan_attach(&sb.env, forge, false);
    assert!(p.ok);
    assert_eq!(&p.cmd[p.cmd.len() - 2..], ["attach".to_string(), forge.primary.as_ref().unwrap().short_id.clone().unwrap()]);
    assert!(!actions::plan_attach(&sb.env, s.agent("ALPHA/LEAD").unwrap(), false).ok);
    assert!(!actions::plan_attach(&sb.env, s.agent("ALPHA/[ALPHA]-ARCHIVE").unwrap(), false).ok);
    let t = actions::plan_attach(&sb.env, forge, true);
    assert_eq!(&t.cmd[..3], ["tmux", "split-window", "-h"]);
}

#[test]
fn herdr_panes_match_by_session_then_title() {
    let sb = sandbox();
    let s = snap(&sb);
    let mut panes = herdr::list_panes(sb.env.herdr_bin.as_deref());
    assert_eq!(panes.len(), 4);
    let id = |k: &str, ps: &[herdr::Pane]| herdr::pane_for(s.agent(k).unwrap(), ps).map(|p| p.pane_id.clone());
    assert_eq!(id("ALPHA/LEAD", &panes).as_deref(), Some("w2:p2"));
    assert_eq!(id("ALPHA/[ALPHA]-FORGE", &panes).as_deref(), Some("w2:p3"));
    assert_eq!(id("ALPHA/[ALPHA]-SCOUT", &panes), None);
    assert_eq!(id("ALPHA/[ALPHA]-ARCHIVE", &panes), None);
    for p in &mut panes {
        p.session_id = None;
    }
    assert_eq!(id("BRAVO/LEAD", &panes).as_deref(), Some("w3:p4"));
}

#[test]
fn herdr_off_outside_herdr() {
    let mut vars = admiralty::sandbox::env_for(&sandbox().base);
    vars.insert("HERDR_ENV".into(), "0".into());
    assert!(admiralty::env::Env::from_vars(&vars).herdr_bin.is_none());
}

#[test]
fn delete_only_dead_non_fleet_sessions() {
    let sb = sandbox();
    let s = snap(&sb);
    let plan = |k: &str| actions::plan_delete(&sb.env, s.agent(k).unwrap());
    // refused: fleet agents (declared), leads, the hub, live sessions, transcript-only sessions
    assert!(!plan("ALPHA/[ALPHA]-ARCHIVE").ok && plan("ALPHA/[ALPHA]-ARCHIVE").summary.contains("fleet agent"));
    assert!(!plan("ALPHA/LEAD").ok);
    assert!(!plan("HUB").ok);
    assert!(!plan("ALPHA/[ALPHA]-ROGUE").ok && plan("ALPHA/[ALPHA]-ROGUE").summary.contains("live"));
    assert!(!plan("DELTA/LEAD").ok); // stopped lead, transcript only
    // allowed: a dead background session under a lead, and one in OTHER
    let p = plan("DELTA/quick look at delta logs");
    assert!(p.ok, "{}", p.summary);
    assert_eq!(&p.cmd[1..2], ["rm"]);
    assert_eq!(p.cmd[0], sb.env.native_claude_bin, "rm goes through the native binary");
    let other = s.others.iter().find(|a| a.name == "[ZULU]-UNKNOWN").unwrap();
    assert!(actions::plan_delete(&sb.env, other).ok);
}

#[test]
fn delete_removes_the_job_rows() {
    let sb = sandbox();
    let s = snap(&sb);
    let a = s.agent("DELTA/quick look at delta logs").unwrap();
    let id = a.primary.as_ref().unwrap().short_id.clone().unwrap();
    let (rc, _) = actions::run_detached(&sb.env, &actions::plan_delete(&sb.env, a));
    assert_eq!(rc, 0);
    assert!(!sb.base.join("claude_home/jobs").join(&id).exists());
    let after = snap(&sb);
    assert!(after.agent("DELTA/quick look at delta logs").is_none());
    assert_eq!(lines(&sb.env.action_log).last().unwrap()["action"], "delete");
}
