mod common;

use admiralty::env::Env;
use admiralty::model::{self, Flag, Status};
use admiralty::present::dump;
use admiralty::sources::{self, Cache, SessionRow};
use common::*;

#[test]
fn fleets_alphabetical_and_machine() {
    let sb = sandbox();
    let s = snap(&sb);
    let tags: Vec<_> = s.fleets.iter().map(|f| f.tag.as_str()).collect();
    assert_eq!(tags, ["ALPHA", "BRAVO", "CHARLIE", "DELTA", "ECHO"]);
    let charlie = s.fleets.iter().find(|f| f.tag == "CHARLIE").unwrap();
    assert!(!charlie.on_machine() && charlie.agents.is_empty());
    assert!(s.fleets.iter().find(|f| f.tag == "ECHO").unwrap().error.is_some());
    assert_eq!(s.source, "files");
}

#[test]
fn statuses() {
    let sb = sandbox();
    let s = snap(&sb);
    let st = |k: &str| s.agent(k).unwrap_or_else(|| panic!("{k}")).status;
    assert_eq!(st("ALPHA/LEAD"), Status::Busy);
    assert_eq!(st("ALPHA/[ALPHA]-SCOUT"), Status::Busy);
    assert_eq!(st("ALPHA/[ALPHA]-FORGE"), Status::NeedsYou);
    assert_eq!(st("ALPHA/[ALPHA]-ARCHIVE"), Status::Stopped);
    assert_eq!(st("ALPHA/[ALPHA]-BEACON"), Status::NotRunning);
    assert_eq!(st("BRAVO/LEAD"), Status::Idle);
    assert_eq!(s.need_count(), 1);
}

#[test]
fn lead_by_name_only_and_flags() {
    let sb = sandbox();
    let s = snap(&sb);
    // DELTA's untagged session in the fleet dir is not promoted; its lead row comes
    // from the transcript the ended [DELTA]-LEAD session left, and it is stopped
    let delta = s.fleets.iter().find(|f| f.tag == "DELTA").unwrap();
    let lead = delta.lead().unwrap();
    assert_eq!(lead.name, "[DELTA]-LEAD");
    assert_eq!(lead.status, Status::Stopped);
    assert!(!lead.live() && lead.primary.as_ref().unwrap().state.as_deref() == Some("ended"));
    let has = |k: &str, f: Flag| s.agent(k).unwrap().flags.contains(&f);
    assert!(has("BRAVO/LEAD", Flag::NameVariant));
    assert!(has("ALPHA/[ALPHA]-ARCHIVE", Flag::LeftQuestion));
    assert!(has("BRAVO/[BRAVO]-TWIN", Flag::DuplicateLive));
    assert!(has("DELTA/[DELTA]-DRIFT", Flag::OffConfig));
}

#[test]
fn only_declared_agents_are_fleet_agents_the_rest_hang_under_the_lead() {
    let sb = sandbox();
    let s = snap(&sb);
    let alpha = s.fleets.iter().find(|f| f.tag == "ALPHA").unwrap();
    let names: Vec<&str> = alpha.agents.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["[ALPHA]-LEAD", "[ALPHA]-ARCHIVE", "[ALPHA]-BEACON", "[ALPHA]-FORGE", "[ALPHA]-NOPROMPT", "[ALPHA]-SCOUT"]);
    // [ALPHA]-ROGUE (tagged, not in FLEET_CONFIG.json) is a sub-agent of the lead
    let lead = alpha.lead().unwrap();
    let rogue = lead.helpers.iter().find(|h| h.name == "[ALPHA]-ROGUE").unwrap();
    assert!(rogue.live() && rogue.flags.is_empty());
    assert!(s.agent("ALPHA/[ALPHA]-ROGUE").is_some(), "still selectable and actionable");
    // an untagged session in DELTA's dir hangs under DELTA's (stopped) lead
    let delta = s.fleets.iter().find(|f| f.tag == "DELTA").unwrap();
    assert!(delta.agents.iter().all(|a| a.name != "quick look at delta logs"));
    assert_eq!(delta.lead().unwrap().helpers[0].name, "quick look at delta logs");
}

#[test]
fn stale_and_duplicates() {
    let sb = sandbox();
    let s = snap(&sb);
    let a = s.agent("ALPHA/[ALPHA]-ARCHIVE").unwrap();
    assert_eq!(a.stale.len(), 2);
    assert!(a.stale.iter().all(|r| r.started_at < a.primary.as_ref().unwrap().started_at));
    let twin = s.agent("BRAVO/[BRAVO]-TWIN").unwrap();
    assert!(twin.live() && twin.status == Status::Busy && twin.stale.len() == 1);
}

#[test]
fn subagents_nested_running_and_tasks() {
    let sb = sandbox();
    let s = snap(&sb);
    let lead = s.agent("ALPHA/LEAD").unwrap();
    let planner = lead.subagents.iter().find(|x| x.rec.label == "planner").unwrap();
    assert!(planner.running && planner.rec.teammate);
    let mut kids: Vec<_> = planner.children.iter().map(|c| (c.rec.label.as_str(), c.running)).collect();
    kids.sort();
    assert_eq!(kids, [("Old survey pass", false), ("Survey north ridge", true), ("Survey south ridge", true)]);
    assert_eq!(lead.subagents.iter().filter(|x| !x.running).count(), 7);
    let st: Vec<_> = lead.tasks.iter().map(|t| t.status.as_str()).collect();
    assert_eq!(st, ["completed", "in_progress", "pending"]);
}

#[test]
fn hub_and_others() {
    let sb = sandbox();
    let s = snap(&sb);
    assert!(s.hub.is_some());
    let mut names: Vec<_> = s.others.iter().map(|a| a.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["[ZULU]-UNKNOWN", "scratch notes"]);
}

#[test]
fn files_and_cli_sources_agree() {
    let sb = sandbox();
    let files = snap(&sb);
    let cli_env = Env { force_cli: true, ..sb.env.clone() };
    let cli = model::snapshot(&cli_env, &mut Cache::default());
    assert_eq!(cli.source, "cli");
    let mask = |s: String| s.lines().skip(1).map(|l| l.split_whitespace().filter(|w| !w.ends_with(['s', 'm', 'h', 'd']) || w.len() > 4).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>();
    assert_eq!(mask(dump(&files, true, true)), mask(dump(&cli, true, true)));
}

#[test]
fn interactive_row_wins_over_dead_job_row() {
    let sb = sandbox();
    let mut cache = Cache::default();
    let row = |kind: &str, pid: Option<i64>| SessionRow {
        session_id: "s1".into(), name: "[ALPHA]-LEAD".into(), cwd: "/x".into(), kind: kind.into(),
        short_id: None, pid, state: None, status: Some("idle".into()), status_since: None, started_at: Some(100.0), waiting_for: None,
    };
    let cfg = sources::read_fleets(&sb.env, &mut cache);
    let s = model::build(&sb.env, &mut cache, &cfg, vec![row("background", None), row("interactive", Some(1))], vec![], 200.0);
    let lead = s.agent("ALPHA/LEAD").unwrap();
    assert!(lead.live() && lead.stale.is_empty());
}

#[test]
fn cli_failure_is_reported_and_configs_still_show() {
    let sb = sandbox();
    std::fs::remove_dir_all(sb.base.join("claude_home")).unwrap();
    let env = Env { claude_bin: "/nonexistent/claude".into(), ..sb.env.clone() };
    let s = model::snapshot(&env, &mut Cache::default());
    assert!(!s.errors.is_empty() && s.errors[0].contains("agents --json"));
    assert_eq!(s.fleets[0].tag, "ALPHA");
}

#[test]
fn malformed_internal_files_degrade() {
    let sb = sandbox();
    for e in std::fs::read_dir(sb.base.join("claude_home/jobs")).unwrap() {
        std::fs::write(e.unwrap().path().join("state.json"), "{ broken").unwrap();
    }
    let s = snap(&sb);
    assert_eq!(s.agent("ALPHA/LEAD").unwrap().status, Status::Busy); // interactive rows unaffected
}

#[test]
fn dump_text() {
    let sb = sandbox();
    let t = dump(&snap(&sb), true, true);
    assert!(t.contains("ALPHA") && t.contains("needs: Ship forge 2.0") && t.contains("OTHER SESSIONS"));
}

#[test]
fn iso_parse() {
    assert_eq!(sources::parse_iso("1970-01-01T00:00:00Z"), Some(0.0));
    assert_eq!(sources::parse_iso("2026-10-01T00:00:00.500Z"), Some(1790812800.5));
}


#[test]
fn stopped_lead_from_transcript_only_when_no_live_lead() {
    let sb = sandbox();
    let s = snap(&sb);
    // ALPHA's lead is live: no transcript row is added next to it
    assert!(s.agent("ALPHA/LEAD").unwrap().stale.is_empty());
    let d = s.agent("DELTA/LEAD").unwrap();
    assert!(d.is_lead && d.flags.is_empty());
    let age = s.taken_at - d.primary.as_ref().unwrap().started_at.unwrap();
    assert!((age - 86400.0 * 4.0).abs() < 60.0, "started_at is the transcript's last write");
    assert!(!admiralty::actions::plan_activate(&sb.env, d).ok);
}

fn session_file(sb: &Sb, name: &str) -> std::path::PathBuf {
    std::fs::read_dir(sb.base.join("claude_home/sessions"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| std::fs::read_to_string(p).unwrap().contains(&format!("\"name\": \"{name}\"")))
        .unwrap()
}

#[test]
fn session_file_with_trailing_garbage_still_counts_as_live() {
    // Claude Code can rewrite a session file in place without truncating it
    let sb = sandbox();
    let p = session_file(&sb, "[ALPHA]-LEAD");
    let mut raw = std::fs::read_to_string(&p).unwrap();
    raw.push_str("80,\"waitingFor\":\"permission prompt\"}");
    std::fs::write(&p, raw).unwrap();
    let s = snap(&sb);
    let lead = s.agent("ALPHA/LEAD").unwrap();
    assert!(lead.live() && lead.status == Status::Busy);
}

#[test]
fn waiting_status_means_needs_you() {
    let sb = sandbox();
    let p = session_file(&sb, "[BRAVO]-LEAD-quiet-otter");
    let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    v["status"] = "waiting".into();
    v["waitingFor"] = "permission prompt".into();
    std::fs::write(&p, v.to_string()).unwrap();
    let s = snap(&sb);
    let lead = s.agent("BRAVO/LEAD").unwrap();
    assert_eq!(lead.status, Status::NeedsYou);
    assert_eq!(lead.job.needs.as_deref(), Some("Waiting for permission prompt in its session."));
}
