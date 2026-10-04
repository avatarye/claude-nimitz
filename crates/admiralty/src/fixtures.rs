//! A realistic fixture world: three fleets, every status, every issue code of
//! contracts.md section 7, crew, tenders, stowaways and panes. Tests and the
//! `--demo` flag render from this; nothing here touches the real machine.

use std::collections::BTreeMap;

use nimitz_core::{Comms, Crew, Fleet, Id, Issue, Member, Role, SessionInfo, Severity, Snapshot, Squadron, Status, Task};
use nimitz_mux::{Caps, Pane, PaneRef};

use crate::data::Bundle;

/// The demo clock; ages in tests and screenshots never move.
pub const NOW: f64 = 1_790_000_000.0;

fn labels(id: &Id) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert("NIMITZ_FLEET".into(), id.fleet().to_string());
    if let Some(sq) = id.squadron() {
        m.insert("NIMITZ_SQUADRON".into(), sq.to_string());
    }
    m.insert("NIMITZ_ROLE".into(), id.role().as_str().to_string());
    m.insert("NIMITZ_ID".into(), id.0.clone());
    m
}

#[allow(clippy::too_many_arguments)]
fn session(id: &Id, sid: &str, pid: Option<i64>, kind: &str, status: Option<&str>, waiting: Option<&str>, age_s: f64, with_labels: bool) -> SessionInfo {
    SessionInfo {
        session_id: format!("{sid}-0000-4000-8000-000000000000"),
        short_id: Some(sid.chars().take(8).collect()),
        pid,
        kind: kind.into(),
        state: if pid.is_some() { Some("running".into()) } else { Some("ended".into()) },
        status: status.map(String::from),
        waiting_for: waiting.map(String::from),
        started_at: Some(NOW - age_s),
        status_since: Some(NOW - age_s / 3.0),
        name: id.name(),
        cwd: cwd_of(id),
        labels: if with_labels { labels(id) } else { BTreeMap::new() },
    }
}

fn cwd_of(id: &Id) -> String {
    match id.role() {
        Role::Admiral => format!("/home/u/.claude-nimitz/{}/admiral", id.fleet().to_lowercase()),
        Role::Commodore => format!("/home/u/.claude-nimitz/{}/squadrons/{}", id.fleet().to_lowercase(), id.squadron().unwrap_or("")),
        _ => format!("/home/u/work/{}", id.parts().last().unwrap_or(&"x").trim_start_matches('~').to_lowercase()),
    }
}

fn member(id: Id, status: Status, reasons: &[&str], session: Option<SessionInfo>) -> Member {
    Member {
        expected_name: id.name(),
        role: id.role(),
        cwd: Some(cwd_of(&id)),
        id,
        status,
        reasons: reasons.iter().map(|s| s.to_string()).collect(),
        session,
        extra_rows: vec![],
        crew: vec![],
        tasks: vec![],
        issues: vec![],
        pane: None,
    }
}

fn issue(code: &str, sev: Severity, msg: &str, fix: Option<&str>, id: Option<&Id>) -> Issue {
    Issue { code: code.into(), severity: sev, message: msg.into(), fix: fix.map(String::from), id: id.cloned() }
}

fn crew(agent_id: &str, parent: Option<&str>, label: &str, agent_type: &str, teammate: bool, running: bool, age_s: f64) -> Crew {
    Crew {
        agent_id: agent_id.into(),
        parent_id: parent.map(String::from),
        label: label.into(),
        agent_type: Some(agent_type.into()),
        teammate,
        running,
        last_write: NOW - age_s,
    }
}

fn pane(pane_id: &str, session: Option<&str>, title: &str, state: &str) -> Pane {
    Pane {
        pane: PaneRef { pane_id: pane_id.into(), tab_id: Some(format!("tab-{pane_id}")), workspace_id: Some(format!("ws-{pane_id}")) },
        session_id: session.map(|s| format!("{s}-0000-4000-8000-000000000000")),
        title: title.into(),
        cwd: String::new(),
        state: state.into(),
    }
}

// ------------------------------------------------------------------ fleets

fn fleet_main() -> Fleet {
    let f = "MAIN";
    let mut admiral = member(
        Id::admiral(f),
        Status::Idle,
        &["session file: status idle for 4m", "pid 4100 alive"],
        Some(session(&Id::admiral(f), "aa11aa11", Some(4100), "interactive", Some("idle"), None, 7200.0, true)),
    );
    admiral.crew = vec![crew("tm-scribe", None, "minutes-scribe", "general-purpose", true, true, 40.0)];

    // ARGOS: a healthy, busy squadron
    let sq = Id::commodore(f, "ARGOS");
    let mut commodore = member(
        sq.clone(),
        Status::Busy,
        &["session file: status busy for 1m"],
        Some(session(&sq, "ab22ab22", Some(4200), "interactive", Some("busy"), None, 5400.0, true)),
    );
    commodore.tasks = vec![
        Task { id: "1".into(), subject: "data importer".into(), status: "in_progress".into(), owner: Some("BOLT".into()) },
        Task { id: "2".into(), subject: "write squadron brief".into(), status: "completed".into(), owner: None },
    ];
    let bolt_id = Id::ship(f, "ARGOS", "BOLT");
    let mut bolt = member(
        bolt_id.clone(),
        Status::Busy,
        &["session file: status busy for 12m"],
        Some(session(&bolt_id, "ac33ac33", Some(4310), "background", Some("busy"), None, 9000.0, true)),
    );
    bolt.crew = vec![
        crew("sa-1", None, "fix layout bug", "general-purpose", false, true, 60.0),
        crew("sa-2", Some("sa-1"), "run unit tests", "Explore", false, true, 20.0),
        crew("sa-3", None, "summarize probe logs", "general-purpose", false, false, 2400.0),
    ];
    bolt.tasks = vec![Task { id: "7".into(), subject: "importer: parser edge cases".into(), status: "pending".into(), owner: None }];
    let boom_id = Id::ship(f, "ARGOS", "BOOM");
    let boom = member(
        boom_id.clone(),
        Status::Idle,
        &["session file: status idle for 2h"],
        Some(session(&boom_id, "ad44ad44", Some(4320), "background", Some("idle"), None, 10800.0, true)),
    );
    let tender_id = Id::tender(f, "ARGOS", "ingest-logs");
    let tender = member(
        tender_id.clone(),
        Status::Busy,
        &["session file: status busy for 5m"],
        Some(session(&tender_id, "ae55ae55", Some(4330), "background", Some("busy"), None, 600.0, true)),
    );
    let argos = Squadron {
        id: sq,
        commodore,
        ships: vec![bolt, boom],
        tenders: vec![tender],
        comms: Comms::Mode("open".into()),
        issues: vec![],
    };

    // KITS: one waiting commodore, one stopped ship, one ship that needs you
    let sq = Id::commodore(f, "KITS");
    let commodore = member(
        sq.clone(),
        Status::NeedsYou,
        &["session file: status waiting, waitingFor permission prompt"],
        Some(session(&sq, "af66af66", Some(4400), "interactive", Some("waiting"), Some("permission prompt"), 3000.0, true)),
    );
    let docs_id = Id::ship(f, "KITS", "DOCS");
    let docs = member(
        docs_id.clone(),
        Status::Stopped,
        &["job row completed, no pid", "ended 3h ago"],
        Some(session(&docs_id, "b077b077", None, "background", None, None, 14400.0, true)),
    );
    let pipe_id = Id::ship(f, "KITS", "PIPELINE");
    let pipeline = member(
        pipe_id.clone(),
        Status::NeedsYou,
        &["session file: status waiting, waitingFor question", "asked 9m ago"],
        Some(session(&pipe_id, "b188b188", Some(4420), "background", Some("waiting"), Some("question: ship the flaky test fix?"), 2400.0, true)),
    );
    let kits = Squadron {
        id: sq,
        commodore,
        ships: vec![docs, pipeline],
        tenders: vec![],
        comms: Comms::Mode("commodore-only".into()),
        issues: vec![],
    };

    Fleet {
        id: Id::admiral(f),
        dir: "/home/u/.claude-nimitz/main".into(),
        machine: Some("west-mac".into()),
        admiral,
        squadrons: vec![argos, kits],
        issues: vec![],
    }
}

/// Every issue code of contracts.md section 7 lives somewhere in this fleet.
fn fleet_forge() -> Fleet {
    let f = "FORGE";
    let adm_id = Id::admiral(f);
    let mut admiral = member(
        adm_id.clone(),
        Status::Busy,
        &["session file: status busy", "job row says background"],
        Some(session(&adm_id, "ca11ca11", Some(5100), "background", Some("busy"), None, 1800.0, true)),
    );
    // core's wording (sessions::session_issues)
    admiral.issues = vec![issue(
        "role.background",
        Severity::Error,
        "FORGE (an admiral) is a background session",
        Some("claude attach ca11ca11  # and keep it in a terminal"),
        Some(&adm_id),
    )];

    // IRON: session-shape breakage
    let sq = Id::commodore(f, "IRON");
    let mut commodore = member(
        sq.clone(),
        Status::Stopped,
        &["job row, no pid", "resumed with flags earlier: labels gone"],
        Some(session(&sq, "cb22cb22", None, "interactive", None, None, 90000.0, false)),
    );
    commodore.issues = vec![issue(
        "labels.missing",
        Severity::Warn,
        "current session has no NIMITZ_* labels (resumed with flags)",
        Some("nimitz relaunch FORGE/IRON"),
        Some(&sq),
    )];
    let anvil_id = Id::ship(f, "IRON", "ANVIL");
    let mut anvil = member(
        anvil_id.clone(),
        Status::Busy,
        &["session file: status busy", "a second live session holds this identity"],
        Some(session(&anvil_id, "cc33cc33", Some(5310), "background", Some("busy"), None, 4000.0, true)),
    );
    anvil.session.as_mut().unwrap().name = "[FORGE.IRON]-ANVIL-2".into(); // drifted
    anvil.extra_rows = vec![session(&anvil_id, "cc33dupe", Some(5311), "background", Some("idle"), None, 3200.0, true)];
    anvil.issues = vec![
        issue("name.drift", Severity::Error, "live session is named [FORGE.IRON]-ANVIL-2, expected [FORGE.IRON]-ANVIL", Some("nimitz relaunch FORGE/IRON/ANVIL"), Some(&anvil_id)),
        issue("dup.live", Severity::Error, "two live sessions for this identity (cc33ac33, cc33dupe)", Some("nimitz stop cc33dupe"), Some(&anvil_id)),
    ];
    let hammer_id = Id::ship(f, "IRON", "HAMMER");
    let mut hammer = member(
        hammer_id.clone(),
        Status::Unknown,
        &["registry's current session id has no job row and no transcript"],
        None,
    );
    hammer.extra_rows = vec![
        session(&hammer_id, "cd44cd44", None, "background", None, None, 200000.0, true),
        session(&hammer_id, "cd44copy", None, "background", None, None, 190000.0, true),
    ];
    hammer.issues = vec![
        issue("lineage.broken", Severity::Error, "current session id is gone; start only with --force", Some("nimitz start FORGE/IRON/HAMMER --force"), Some(&hammer_id)),
        issue("copy.rows", Severity::Warn, "2 extra job rows of this identity", Some("nimitz sweep FORGE/IRON/HAMMER"), Some(&hammer_id)),
    ];
    let quench_id = Id::ship(f, "IRON", "QUENCH");
    let mut quench = member(quench_id.clone(), Status::NotStarted, &["no job row, no transcript"], None);
    quench.issues = vec![
        issue("folder.missing", Severity::Error, "/home/u/work/quench does not exist", Some("nimitz ship set FORGE/IRON/QUENCH --dir <folder>"), Some(&quench_id)),
        issue("ship.elsewhere", Severity::Error, "managed folder locked by home-server", None, Some(&quench_id)),
    ];
    let t_id = Id::tender(f, "IRON", "hotfix");
    let mut hotfix = member(
        t_id.clone(),
        Status::Stopped,
        &["job row failed"],
        Some(session(&t_id, "ce55ce55", None, "background", None, None, 50000.0, true)),
    );
    hotfix.session.as_mut().unwrap().labels.insert("NIMITZ_ID".into(), "FORGE/MOLD/~hotfix".into());
    hotfix.issues = vec![issue("labels.mismatch", Severity::Error, "labels name FORGE/MOLD/~hotfix, registry says FORGE/IRON/~hotfix", Some("nimitz relaunch FORGE/IRON/~hotfix"), Some(&t_id))];
    let iron = Squadron {
        id: sq,
        commodore,
        ships: vec![anvil, hammer, quench],
        tenders: vec![hotfix],
        comms: Comms::Mode("open".into()),
        issues: vec![],
    };

    // MOLD: config-shape breakage
    let sq = Id::commodore(f, "MOLD");
    let commodore = member(sq.clone(), Status::NotStarted, &["no job row, no transcript"], None);
    let cast_id = Id::ship(f, "MOLD", "CAST");
    let mut cast = member(
        cast_id.clone(),
        Status::Idle,
        &["session file: status idle"],
        Some(session(&cast_id, "cf66cf66", Some(5410), "background", Some("idle"), None, 7000.0, true)),
    );
    cast.issues = vec![issue("name.taken", Severity::Error, "[FORGE.MOLD]-CAST is held by a live session of another identity (d077d077)", Some("nimitz stop d077d077"), Some(&cast_id))];
    let flask_id = Id::ship(f, "MOLD", "FLASK");
    let mut flask = member(flask_id.clone(), Status::NotStarted, &["no job row, no transcript"], None);
    flask.issues = vec![issue("folder.untrusted", Severity::Error, "/home/u/work/flask is not trusted; run `claude` there once", None, Some(&flask_id))];
    let mold = Squadron {
        id: sq.clone(),
        commodore,
        ships: vec![cast, flask],
        tenders: vec![],
        comms: Comms::Allow { allow: vec!["MAIN/ARGOS/BOLT".into()] },
        issues: vec![issue("config.invalid", Severity::Error, "squadron.toml: ships[1] duplicate name CAST", Some("nimitz check FORGE/MOLD"), Some(&sq))],
    };

    Fleet {
        id: Id::admiral(f),
        dir: "/home/u/.claude-nimitz/forge".into(),
        machine: Some("west-mac".into()),
        admiral,
        squadrons: vec![iron, mold],
        issues: vec![issue("config.layout", Severity::Warn, "a CLAUDE.md sits in the fleet folder; every session under it inherits it", Some("nimitz doctor --fix"), Some(&Id::admiral(f)))],
    }
}

/// A fleet whose shape exists here but whose sessions run on another machine.
fn fleet_lab() -> Fleet {
    let f = "LAB";
    let admiral = member(Id::admiral(f), Status::NotStarted, &["this machine is not in fleet.toml machines"], None);
    let sq = Id::commodore(f, "OPTICS");
    let commodore = member(sq.clone(), Status::NotStarted, &["not deployed on this machine"], None);
    let lens_id = Id::ship(f, "OPTICS", "LENS");
    let lens = member(lens_id, Status::NotStarted, &["no dirs entry for this machine"], None);
    Fleet {
        id: Id::admiral(f),
        dir: "/home/u/.claude-nimitz/lab".into(),
        machine: None,
        admiral,
        squadrons: vec![Squadron { id: sq, commodore, ships: vec![lens], tenders: vec![], comms: Comms::default(), issues: vec![] }],
        issues: vec![],
    }
}

fn stowaways() -> (Vec<SessionInfo>, Vec<Issue>) {
    let s1 = SessionInfo {
        session_id: "ee11ee11-0000-4000-8000-000000000000".into(),
        short_id: Some("ee11ee11".into()),
        pid: Some(6100),
        kind: "background".into(),
        state: Some("running".into()),
        status: Some("busy".into()),
        waiting_for: None,
        started_at: Some(NOW - 1200.0),
        status_since: Some(NOW - 300.0),
        name: "scratch-run".into(),
        cwd: "/home/u/.claude-nimitz/forge/squadrons/IRON".into(),
        labels: BTreeMap::new(),
    };
    let s2 = SessionInfo {
        session_id: "ef22ef22-0000-4000-8000-000000000000".into(),
        short_id: Some("ef22ef22".into()),
        pid: None,
        kind: "background".into(),
        state: Some("completed".into()),
        status: None,
        waiting_for: None,
        started_at: Some(NOW - 90000.0),
        status_since: None,
        name: "one-off probe".into(),
        cwd: "/home/u/.claude-nimitz/main/admiral".into(),
        labels: BTreeMap::new(),
    };
    // a session of the old fleet-skill fleets, recognised by its name
    let s3 = SessionInfo {
        session_id: "ab99ab99-0000-4000-8000-000000000000".into(),
        short_id: Some("ab99ab99".into()),
        pid: Some(6300),
        kind: "interactive".into(),
        state: None,
        status: Some("idle".into()),
        waiting_for: None,
        started_at: Some(NOW - 30000.0),
        status_since: Some(NOW - 600.0),
        name: "[KITS]-LEAD".into(),
        cwd: "/home/u/.claude-nimitz/main/squadrons/KITS".into(),
        labels: BTreeMap::new(),
    };
    let issues = vec![
        issue("legacy", Severity::Warn, "`[KITS]-LEAD` is a session of the old fleet-skill fleet KITS", Some("nimitz adopt ab99ab99 MAIN/KITS"), None),
        issue("stowaway", Severity::Warn, "live session `scratch-run` in FORGE scope has no labels", Some("nimitz adopt ee11ee11 FORGE/IRON/<ship>"), None),
        issue("stowaway", Severity::Info, "dead session `one-off probe` in MAIN scope has no labels", Some("nimitz rm ef22ef22"), None),
    ];
    (vec![s1, s2, s3], issues)
}

// ------------------------------------------------------------------ bundles

pub fn demo_snapshot() -> Snapshot {
    let (stow, mut issues) = stowaways();
    issues.push(issue("cc.version", Severity::Warn, "Claude Code 2.1.291 is above the supported range 2.1.288-2.1.289", Some("nimitz doctor"), None));
    Snapshot {
        taken_at: NOW,
        machine: Some("west-mac".into()),
        fleets: vec![fleet_main(), fleet_forge(), fleet_lab()],
        stowaways: stow,
        issues,
        cc_version: Some("2.1.291".into()),
        compat_ok: false,
    }
}

/// Demo world inside a full multiplexer (every pane state appears once).
pub fn demo() -> Bundle {
    Bundle {
        snap: demo_snapshot(),
        panes: vec![
            pane("p1", Some("aa11aa11"), "[MAIN]-ADMIRAL", "idle"),
            pane("p2", Some("ab22ab22"), "[MAIN.ARGOS]-COMMODORE", "working"),
            pane("p3", Some("af66af66"), "[MAIN.KITS]-COMMODORE", "blocked"),
            pane("p4", Some("ca11ca11"), "[FORGE]-ADMIRAL", "done"),
            pane("p5", Some("ee11ee11"), "scratch-run", "unknown"),
        ],
        mux_name: "herdr".into(),
        mux_caps: Caps { open: true, focus: true, type_text: true, layout: true },
        partial: false,
    }
}

/// The same world with no multiplexer: Admiralty must go monitor-only.
pub fn demo_monitor_only() -> Bundle {
    Bundle { snap: demo_snapshot(), panes: vec![], mux_name: "none".into(), mux_caps: Caps::default(), partial: false }
}

/// An Env for demo data that points nowhere real: plans read empty folders,
/// and nothing runs (the demo `Ctx` is not live).
pub fn demo_env() -> nimitz_core::env::Env {
    let base = std::env::temp_dir().join("admiralty-demo-nowhere");
    let p = |x: &str| base.join(x).to_string_lossy().into_owned();
    let vars = std::collections::HashMap::from([
        ("HOME".to_string(), p("home")),
        ("NIMITZ_HOME".to_string(), p("nimitz")),
        ("NIMITZ_STATE".to_string(), p("state")),
        ("NIMITZ_CLAUDE_HOME".to_string(), p("claude_home")),
        ("NIMITZ_CLAUDE_BIN".to_string(), "/nonexistent/claude".to_string()),
        ("NIMITZ_NATIVE_CLAUDE_BIN".to_string(), "/nonexistent/claude".to_string()),
        ("NIMITZ_HOSTNAME".to_string(), "west-mac".to_string()),
        ("NIMITZ_MUX".to_string(), "none".to_string()),
    ]);
    nimitz_core::env::Env::from_vars(&vars)
}
