//! Role rules and role briefs. Owner: core-sessions.
//!
//! The rules (`rules/*.md`, compiled in) are what a member must know about
//! its role: the authority table, the chain, plan-first. The SessionStart hook
//! hands them to every labelled session (startup, resume, clear, compact), and
//! a session that runs on older rules gets the update at its next turn
//! (`update_for`, from the UserPromptSubmit hook), so a new nimitz build
//! updates every member live, with no restart and no plugin update.
//! `NIMITZ_STATE/rules-delivered.json` records, per job (else per session),
//! the version last delivered; `check` names a live member still on older
//! rules (Info: the update comes with its next prompt).
//!
//! The brief (`CLAUDE.md` in an admiral's or commodore's folder) holds only
//! who the member is, and the user's own notes. Every brief nimitz ever
//! generated is listed here, so `doctor --fix` can replace an old generated
//! one and leave a hand-edited one alone.
use crate::env::Env;
use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Bump when the rules text changes in a way members must pick up, and add
/// the version's entry to `CHANGELOG`.
pub const VERSION: u32 = 7;

/// What changed in each rules version, oldest first (one entry per version;
/// a member on v{N} gets the entries after N with the new text).
pub const CHANGELOG: &[(u32, &str)] = &[
    (1, "first versioned rules: the chain of command, plan-first, the authority table, needs-you notices"),
    (
        2,
        "the tender role is gone (one-off work: a subagent in the lead's session, or a ship); \
         the admiral may signal every member of its fleet, and a ship may reply to its admiral; \
         every member may run `nimitz` (ships: the read-only commands, `signal` within comms, `sweep` of itself); \
         every member, leads too, is a Claude Code background job (the user attaches to a lead's tab with `nimitz open`; detach is ctrl-z, \
         and `/exit` inside an attach opens the agent view); \
         `nimitz restore <F>` brings a fleet back from the Herdr shape after a loss (an admiral may restore its own fleet)",
    ),
    (
        3,
        "`nimitz restore [<F>]` is the one command after a Claude Code update (it relaunches workers on an older version, then reattaches), \
         a Herdr closure or crash (it reattaches) and a reboot (it wakes the leads and the active ships; `--all` every ship); \
         `--restart` relaunches every live member; it never relaunches the member that runs it; a commodore may restore its own ships; \
         `up` and `layout herdr` are deprecated; never start, resume or stop a member with `claude` yourself (a copy, a held conversation, a stowaway, the daemon's env)",
    ),
    (
        4,
        "one verb, `nimitz start [<target>]`, brings any member up and ready (a ship, a squadron, a fleet, or every fleet here; \
         after a Claude Code update, a Herdr closure or a reboot too); `-f` relaunches the live ones; `-p` (a pane, focused) is the user's; \
         `nimitz stop` is its opposite; `restore`, `relaunch`, `open`, `up` and `layout herdr` are deprecated; \
         a start never relaunches the member that runs it (`start.self`)",
    ),
    (
        5,
        "aides: an always-on member `F/@NAME` (`[F]-@NAME`) attached to a parent, with a durable inbox \
         (`nimitz inbox post|list|status <aide>`, `nimitz ask <aide>`); an aide works its inbox \
         (`inbox claim`, then `done` or `fail`, until exit 3) and signals its parent only, and its parent it; \
         the admiral starts and stops its aides and retries or drops their dead letters",
    ),
    (
        6,
        "aides are added, changed and removed with `nimitz aide new|set|rm` (you or the fleet's admiral); \
         an aide is rung when work arrives (a `[nimitz inbox]` line), its session start shows its inbox, \
         and it asks for its 15-minute cron only when the session has none; \
         an aide runs no git: `nimitz aide grep|status` read its folder, and a note ends in `nimitz inbox done ... --commit --title --paths --pages`, \
         which commits exactly those paths; a reader answers a `wiki.query.v1` from a snapshot `claim` moves to the repo's last commit, \
         and nimitz posts each gap of its answer to the notes keeper",
    ),
    (
        7,
        "aides are gone: no `F/@NAME` member, no `nimitz aide`, `inbox` or `ask`; \
         a fleet's `[[aide]]` is removed by `nimitz migrate --shared --yes` (the user's); \
         the fleet's knowledge base is written by the `nimitz:kb-scribe` subagent (a note as JSON, one commit each) \
         and read by `nimitz:kb-ref` (a question, a cited answer), both in every member's session",
    ),
];

/// The changelog entries after `from`, as markdown lines.
pub fn changelog_since(from: u32) -> String {
    CHANGELOG.iter().filter(|(v, _)| *v > from).map(|(v, what)| format!("- v{v}: {what}\n")).collect()
}

/// The text a running member gets when its rules are older than `VERSION`:
/// the header the member recognises, the changelog, the full new rules.
pub fn update_text(id: &Id, from: u32) -> String {
    format!("nimitz rules updated v{from}\u{2192}v{VERSION}\n\nWhat changed:\n{}\nThe full rules now:\n\n{}", changelog_since(from), text(id))
}

// ------------------------------------------------------------------ delivery record

/// One entry of `rules-delivered.json`: the version a job (or session) last got.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    pub version: u32,
    pub at: f64,
    pub session_id: String,
    /// the hook event that delivered it (SessionStart, UserPromptSubmit, ...)
    pub via: String,
}

/// `NIMITZ_STATE/rules-delivered.json`.
pub fn delivered_path(env: &Env) -> PathBuf {
    env.state.join("rules-delivered.json")
}

/// The key a delivery is recorded under: the job id when the hook's parent
/// (`ppid`, the Claude Code process) runs as a job (`sessions/<pid>.json`
/// `jobId`), else `session:<id>`. The job id outlives stops, resumes, `/clear`
/// and `/compact`, so one record follows the identity through all of them.
pub fn delivery_key(env: &Env, ppid: i64, session_id: &str) -> String {
    let p = env.claude_home.join("sessions").join(format!("{ppid}.json"));
    let job = std::fs::read(p).ok().and_then(|b| crate::evidence::first_json(&b)).and_then(|v| v.get("jobId").and_then(|j| j.as_str()).map(str::to_string));
    match job.filter(|j| !j.is_empty()) {
        Some(j) => j,
        None => format!("session:{session_id}"),
    }
}

fn read_deliveries(env: &Env) -> BTreeMap<String, Delivery> {
    std::fs::read(delivered_path(env)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// The delivery recorded under `key`, if any.
pub fn delivered(env: &Env, key: &str) -> Option<Delivery> {
    read_deliveries(env).remove(key)
}

/// The rules version `key` runs on: its record (or the session's own, when
/// the hook could not read a job id), else the `rules` of the session's
/// newest SessionStart line (a session an older nimitz started, with no
/// record), else 0.
pub fn delivered_version(env: &Env, key: &str, session_id: &str) -> u32 {
    let all = read_deliveries(env);
    if let Some(d) = all.get(key).or_else(|| all.get(&format!("session:{session_id}"))) {
        return d.version;
    }
    crate::evidence::events(env)
        .iter()
        .rev()
        .find(|e| e.event == "SessionStart" && e.session_id.as_deref() == Some(session_id))
        .and_then(|e| e.rules)
        .unwrap_or(0)
}

/// How long a hook tries for `rules-delivered.lock` before it gives up (the
/// hook must stay fast, and an update it cannot record is not handed out).
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// Record that `key` now has `VERSION`: read-modify-write under
/// `rules-delivered.lock` (tried for `LOCK_WAIT`), written as a temp file and
/// renamed. False when the record could not be written.
pub fn record_delivery(env: &Env, key: &str, session_id: &str, via: &str) -> bool {
    let path = delivered_path(env);
    let Some(dir) = path.parent() else { return false };
    let _ = std::fs::create_dir_all(dir);
    let Ok(lock) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path.with_extension("lock")) else { return false };
    let locked = {
        let until = std::time::Instant::now() + LOCK_WAIT;
        loop {
            if lock.try_lock().is_ok() {
                break true;
            }
            if std::time::Instant::now() >= until {
                break false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    };
    if !locked {
        return false;
    }
    let mut all = read_deliveries(env);
    all.insert(key.to_string(), Delivery { version: VERSION, at: crate::evidence::now(), session_id: session_id.to_string(), via: via.to_string() });
    let tmp = path.with_extension("json.tmp");
    serde_json::to_vec_pretty(&all).ok().and_then(|b| std::fs::write(&tmp, b).ok()).is_some() && std::fs::rename(&tmp, &path).is_ok()
}

/// The update `id`'s session (recorded under `key`) must get now: None when
/// it already runs on `VERSION`. Records the delivery; the caller hands the
/// text to the session as context. One text per version per key, whichever
/// per-turn hook event comes first. No record, no update: a record that
/// cannot be written (the state folder is read-only, the lock is held) would
/// hand the same text out at every prompt; `check` shows `rules.old` (and
/// `registry.readonly`) instead.
pub fn update_for(env: &Env, id: &Id, key: &str, session_id: &str, via: &str) -> Option<String> {
    let had = delivered_version(env, key, session_id);
    if had >= VERSION || !record_delivery(env, key, session_id, via) {
        return None;
    }
    Some(update_text(id, had))
}

const COMMON: &str = include_str!("../rules/common.md");
const ADMIRAL: &str = include_str!("../rules/admiral.md");
const COMMODORE: &str = include_str!("../rules/commodore.md");
const SHIP: &str = include_str!("../rules/ship.md");

/// The rules for `id`'s role, filled in for `id`.
pub fn text(id: &Id) -> String {
    let role = id.role();
    let body = match role {
        Role::Admiral => ADMIRAL,
        Role::Commodore => COMMODORE,
        Role::Ship => SHIP,
    };
    let sq = id.squadron().unwrap_or_default();
    let parent = id.parent().unwrap_or_else(|| id.clone());
    let where_ = match role {
        Role::Admiral => format!("of fleet {}", id.fleet()),
        Role::Commodore => format!("of squadron {sq} in fleet {}", id.fleet()),
        Role::Ship => format!("in squadron {sq} of fleet {}", id.fleet()),
    };
    format!("{COMMON}{body}")
        .replace("{VERSION}", &VERSION.to_string())
        .replace("{ID}", &id.0)
        .replace("{NAME}", &id.name())
        .replace("{ROLE}", role.as_str())
        .replace("{WHERE}", &where_)
        .replace("{FLEET}", id.fleet())
        .replace("{SQ}", sq)
        .replace("{PARENT_NAME}", &parent.name())
        .replace("{PARENT}", &parent.0)
}

/// The brief nimitz writes for an admiral or commodore now.
pub fn brief(id: &Id) -> String {
    match id.role() {
        Role::Admiral => format!(
            "# [{f}]-ADMIRAL\n\n\
             You are the admiral of fleet {f}. nimitz gives you your role's rules at\n\
             every session start (`nimitz rules {f}` prints them). Notes for this\n\
             fleet go below.\n",
            f = id.fleet()
        ),
        _ => format!(
            "# [{f}.{sq}]-COMMODORE\n\n\
             You are the commodore of squadron {sq} in fleet {f}. Your brief is\n\
             SQUADRON.md. nimitz gives you your role's rules at every session start\n\
             (`nimitz rules {f}/{sq}` prints them). Notes for this squadron go below.\n",
            f = id.fleet(),
            sq = id.squadron().unwrap_or_default()
        ),
    }
}

/// Every brief nimitz generated before this one, oldest first, word for word
/// (so the 0.3.x wording stays, tenders and all: it is how an old brief is
/// recognised).
fn old_briefs(id: &Id) -> Vec<String> {
    let f = id.fleet();
    match id.role() {
        Role::Admiral => vec![
            format!(
                "# [{f}]-ADMIRAL\n\n\
                 You are the admiral of fleet {f}. You watch the fleet and talk to its\n\
                 commodores (questions only). You never start, stop or edit anything:\n\
                 `nimitz status` for the picture, and the user for every change.\n"
            ),
            format!(
                "# [{f}]-ADMIRAL\n\n\
                 You are the admiral of fleet {f}. You watch the fleet, ask and order\n\
                 its commodores, and run `nimitz` inside your own fleet (`--plan` first,\n\
                 `--yes` once the user agrees). `nimitz status` gives the picture.\n"
            ),
        ],
        _ => {
            let sq = id.squadron().unwrap_or_default();
            vec![
                format!(
                    "# [{f}.{sq}]-COMMODORE\n\n\
                     You are the commodore of squadron {sq} in fleet {f}. Your brief is\n\
                     SQUADRON.md. You dispatch work to your ships with signals, start a ship\n\
                     you need with `nimitz start {f}/{sq}/<SHIP>`, and run one-off jobs as\n\
                     tenders with `nimitz tender {f}/{sq} <task>`.\n"
                ),
                format!(
                    "# [{f}.{sq}]-COMMODORE\n\n\
                     You are the commodore of squadron {sq} in fleet {f}. Your brief is\n\
                     SQUADRON.md. You take orders from your admiral and the user, dispatch work\n\
                     to your ships with signals, start a ship you need with\n\
                     `nimitz start {f}/{sq}/<SHIP>`, and run one-off jobs as tenders with\n\
                     `nimitz tender {f}/{sq} <task>`.\n"
                ),
            ]
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BriefState {
    Current,
    /// an older generated brief, word for word: safe to replace
    Old,
    /// edited by hand (or never generated): left alone
    Custom,
    Missing,
}

/// The brief of every admiral and commodore in the config: (id, path, state).
pub fn briefs(env: &Env) -> Vec<(Id, PathBuf, BriefState)> {
    let (fleets, _) = crate::config::load_all(env);
    let mut out = vec![];
    for f in &fleets {
        let mut leads = vec![(Id::admiral(&f.config.fleet), f.dir.join("admiral"))];
        for (d, s) in &f.squadrons {
            leads.push((Id::commodore(&f.config.fleet, &s.squadron), d.clone()));
        }
        for (id, dir) in leads {
            let p = dir.join("CLAUDE.md");
            let state = match std::fs::read_to_string(&p) {
                Err(_) => BriefState::Missing,
                Ok(t) if t == brief(&id) => BriefState::Current,
                Ok(t) if old_briefs(&id).contains(&t) => BriefState::Old,
                Ok(_) => BriefState::Custom,
            };
            out.push((id, p, state));
        }
    }
    out
}

/// `brief.old` for each generated brief an older nimitz wrote (doctor --fix
/// replaces it); `rules.old` (Info) for each live member still on older
/// rules: the hook hands it the update with its next prompt.
pub fn issues(env: &Env, snap: &Snapshot) -> Vec<Issue> {
    let mut out = vec![];
    for (id, p, state) in briefs(env) {
        if state == BriefState::Old {
            out.push(Issue {
                code: "brief.old".into(),
                severity: Severity::Warn,
                message: format!("{} was written by an older nimitz and says less than the rules now do", p.display()),
                fix: Some("nimitz doctor --fix".into()),
                id: Some(id),
            });
        }
    }
    for f in &snap.fleets {
        for m in f.members() {
            let Some(s) = m.session.as_ref().filter(|s| s.pid.is_some()) else { continue };
            // the record is keyed as the hook keys it: the job, else the session
            let key = s.short_id.clone().unwrap_or_else(|| format!("session:{}", s.session_id));
            let had = delivered_version(env, &key, &s.session_id);
            if had < VERSION {
                out.push(Issue {
                    code: "rules.old".into(),
                    severity: Severity::Info,
                    message: format!("runs on nimitz rules v{had}; it gets v{VERSION} with its next prompt (nothing to do)"),
                    fix: None,
                    id: Some(m.id.clone()),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::{make_fleet, test_env};

    /// A `{NAME}` placeholder (capitals) still in the text; JSON braces are not one.
    fn placeholder_left(t: &str) -> bool {
        t.split('{').skip(1).any(|r| r.split_once('}').is_some_and(|(w, _)| !w.is_empty() && w.chars().all(|c| c.is_ascii_uppercase() || c == '_')))
    }

    #[test]
    fn each_role_gets_its_rules_filled_in() {
        let t = text(&Id::commodore("HOME", "GITHUB"));
        assert!(t.starts_with(&format!("# nimitz fleet rules (rules v{VERSION}) for HOME/GITHUB")), "{t}");
        assert!(t.contains("You are [HOME.GITHUB]-COMMODORE, the commodore of squadron GITHUB in fleet HOME"));
        assert!(t.contains("`ship new|set|rm HOME/GITHUB/<SHIP>`") && t.contains("[nimitz needs-you]"));
        assert!(!placeholder_left(&t), "every placeholder filled: {t}");
        let a = text(&Id::admiral("HOME"));
        assert!(a.contains("## Admiral of HOME") && a.contains("Never another fleet"));
        let s = text(&Id::ship("HOME", "GITHUB", "DEV"));
        assert!(s.contains("[HOME.GITHUB]-DEV, the ship in squadron GITHUB") && s.contains("no one watches your pane"));
        assert!(!t.contains("tender") && !a.contains("tender") && !s.contains("tender"), "the role is gone");
        for id in [Id::admiral("A"), Id::commodore("A", "B"), Id::ship("A", "B", "C")] {
            assert!(text(&id).len() < 10_000, "fits SessionStart's 10,000-character context limit");
            assert!(update_text(&id, 0).len() < 10_000, "the whole changelog plus the text fits too");
        }
        // 0.4.0 wording: comms and nimitz for every member
        assert!(a.contains("signal a ship of your fleet directly"), "{a}");
        assert!(s.contains("[HOME]-ADMIRAL` only to reply") && s.contains("`sweep HOME/GITHUB/DEV`"), "{s}");
        assert!(t.contains("Every member may run `nimitz`"), "{t}");
        // 0.7: the knowledge base through its two subagents, in every role
        for r in [&t, &a, &s] {
            assert!(r.contains("`nimitz:kb-scribe`") && r.contains("`nimitz:kb-ref`"), "{r}");
        }
        assert!(s.contains("\"id\":\n  \"HOME/GITHUB/DEV\"") || s.contains("\"HOME/GITHUB/DEV\"}"), "{s}");
        // 0.7: aides are gone
        assert!(!t.contains("aide") && !a.contains("aide") && !s.contains("aide"), "{a}");
    }

    #[test]
    fn the_changelog_has_every_version_and_the_update_names_the_jump() {
        let versions: Vec<u32> = CHANGELOG.iter().map(|(v, _)| *v).collect();
        assert_eq!(versions, (1..=VERSION).collect::<Vec<u32>>(), "one entry per version, oldest first, up to VERSION");
        assert!(changelog_since(VERSION).is_empty());
        let since1 = changelog_since(1);
        assert!(since1.starts_with("- v2: ") && !since1.contains("- v1:"), "{since1}");
        let id = Id::ship("F", "SQ", "S");
        let u = update_text(&id, 1);
        assert!(u.starts_with(&format!("nimitz rules updated v1\u{2192}v{VERSION}\n")), "{u}");
        assert!(u.contains(&since1) && u.ends_with(&text(&id)), "the changelog, then the full text");
    }

    #[test]
    fn a_delivery_is_recorded_once_per_version_and_read_back() {
        let (td, env) = test_env();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        assert_eq!(delivered(&env, "job1"), None);
        assert_eq!(delivered_version(&env, "job1", "s1"), 0, "no record, no SessionStart line: v0");
        // the first ask delivers and records; the second gets nothing
        let u = update_for(&env, &id, "job1", "s1", "UserPromptSubmit").expect("an update");
        assert!(u.starts_with("nimitz rules updated v0"));
        let d = delivered(&env, "job1").unwrap();
        assert_eq!((d.version, d.via.as_str(), d.session_id.as_str()), (VERSION, "UserPromptSubmit", "s1"));
        assert!(d.at > 0.0);
        assert_eq!(update_for(&env, &id, "job1", "s1", "UserPromptSubmit"), None);
        // a second key does not touch the first; the file is one JSON object
        record_delivery(&env, "session:s2", "s2", "SessionStart");
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(delivered_path(&env)).unwrap()).unwrap();
        assert_eq!(raw["job1"]["version"], VERSION);
        assert_eq!(raw["session:s2"]["via"], "SessionStart");
        // a job key falls back to the session's own record when only that exists
        assert_eq!(delivered_version(&env, "job9", "s2"), VERSION);
        // an unreadable record file is no record (the update goes out again, never a panic)
        std::fs::write(delivered_path(&env), "{not json").unwrap();
        assert!(update_for(&env, &id, "job1", "s1", "UserPromptSubmit").is_some());
        // no record, no update: a state folder that cannot be written hands nothing out
        // (else the same text would go out at every prompt); check's rules.old shows it
        // (a folder under a plain file: no system can make it; `/dev/null/...` is a real folder on Windows)
        let file = td.path().join("a-file");
        std::fs::write(&file, "").unwrap();
        let bad = Env { state: file.join("nope"), ..env.clone() };
        assert!(!record_delivery(&bad, "job1", "s1", "UserPromptSubmit"));
        assert_eq!(update_for(&bad, &id, "job2", "s2", "UserPromptSubmit"), None);
        // a lock another hook holds: tried for a moment, then skipped, never a hang
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(delivered_path(&env).with_extension("lock")).unwrap();
        lock.lock().unwrap();
        let t = std::time::Instant::now();
        assert_eq!(update_for(&env, &id, "job3", "s3", "UserPromptSubmit"), None);
        assert!(t.elapsed() < std::time::Duration::from_secs(3), "{:?}", t.elapsed());
        lock.unlock().unwrap();
        assert!(update_for(&env, &id, "job3", "s3", "UserPromptSubmit").is_some(), "free again: delivered and recorded");
    }

    #[test]
    fn a_live_member_on_older_rules_is_rules_old_until_delivered() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let mut snap = crate::check::tests::snap_for(&env, None);
        let s = crate::check::tests::live_session(&id.name());
        snap.fleets[0].squadrons[0].ships[0].session = Some(s.clone());
        let old = |env: &Env| -> Vec<Issue> { issues(env, &snap).into_iter().filter(|i| i.code == "rules.old").collect() };
        // started by an older nimitz: no record, a SessionStart line with v1
        let line = serde_json::json!({"ts": 1.0, "event": "SessionStart", "session_id": s.session_id, "source": "startup", "pid": 1,
            "labels": crate::claude::labels_for(&env, &id), "rules": 1});
        std::fs::create_dir_all(&env.state).unwrap();
        std::fs::write(env.events_path(), format!("{line}\n")).unwrap();
        let o = old(&env);
        assert_eq!(o.len(), 1, "{o:?}");
        assert_eq!(o[0].severity, Severity::Info);
        assert!(o[0].message.contains("v1") && o[0].message.contains("next prompt") && o[0].fix.is_none(), "{:?}", o[0]);
        // the hook delivered at its next prompt, keyed by its job: gone
        record_delivery(&env, s.short_id.as_deref().unwrap(), &s.session_id, "UserPromptSubmit");
        assert!(old(&env).is_empty());
        // keyed by the session id (the hook read no job id): gone too
        let _ = std::fs::remove_file(delivered_path(&env));
        assert_eq!(old(&env).len(), 1);
        record_delivery(&env, &format!("session:{}", s.session_id), &s.session_id, "SessionStart");
        assert!(old(&env).is_empty());
    }

    #[test]
    fn an_old_generated_brief_is_old_and_a_hand_edited_one_is_left_alone() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let state = |env: &Env, id: &Id| briefs(env).into_iter().find(|(i, _, _)| i == id).map(|(_, p, s)| (p, s)).unwrap();
        let adm = Id::admiral("MAIN");
        let (p, s) = state(&env, &adm);
        assert_eq!(s, BriefState::Current, "a new fleet gets the current brief");
        std::fs::write(&p, &old_briefs(&adm)[0]).unwrap();
        assert_eq!(state(&env, &adm).1, BriefState::Old);
        let snap = crate::check::tests::snap_for(&env, None);
        assert!(issues(&env, &snap).iter().any(|i| i.code == "brief.old" && i.id.as_ref() == Some(&adm)));
        std::fs::write(&p, format!("{}\nMy own note.\n", brief(&adm))).unwrap();
        assert_eq!(state(&env, &adm).1, BriefState::Custom);
        assert!(!issues(&env, &snap).iter().any(|i| i.code == "brief.old"));
    }
}
