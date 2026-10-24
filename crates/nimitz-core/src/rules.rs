//! Role rules and role briefs. Owner: core-sessions.
//!
//! The rules (`rules/*.md`, compiled in) are what a member must know about
//! its role: the roles, paths, plan-first. The SessionStart hook
//! hands them to every labelled session (startup, resume, clear, compact), and
//! a session that runs on older rules gets the update at its next turn
//! (`update_for`, from the UserPromptSubmit hook), so a new nimitz build
//! updates every member live, with no restart and no plugin update. The
//! fleet's harness rules come in a hook entry of their own (`harness_for`,
//! `nimitz hook rules harness`), each under Claude Code's 10,000 characters
//! for one hook's context (docs/design/rules-size.md).
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
pub const VERSION: u32 = 11;

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
         a fleet's `[[aide]]` is removed by `nimitz migrate --shared --yes` (the user's)",
    ),
    (
        8,
        "the fleet's knowledge base is written by the `nimitz:kb-scribe` subagent (a note as JSON, one commit each) \
         and read by `nimitz:kb-ref` (a question, a cited answer), both in every member's session",
    ),
    (
        9,
        "the rules are shorter, with nothing a member does changed; `nimitz start` is in the leads' rules only; \
         the fleet's own rules (its harness) now come as a context of their own, up to 9,500 characters",
    ),
    (
        10,
        "no authority: every member may run every `nimitz` command on any member, and any member may message any member; \
         members are named by paths (`F`, `F/SQ`, `F/SQ/SHIP`; `F/...` everything under one); the user attaches with `nimitz open <id>`; \
         `start` (jobs only), `restart` (relaunch), `stop`, and `muster` (after a reboot, an update or a Herdr closure); \
         a command that stops or relaunches you runs that step last, after your turn ends",
    ),
    (
        11,
        "a lead may edit its own unit's brief and prompts by hand (an admiral: its fleet's CLAUDE.md, its squadrons' SQUADRON.md and prompts/; \
         a commodore: its SQUADRON.md, CLAUDE.md and prompts/), never another unit's files nor machine state; settings change only through nimitz; \
         `ship new|set --prompt-file` copies the file into the squadron's prompts/",
    ),
];

/// The most changelog entries an update lists (older ones describe what
/// later versions changed again).
const CHANGELOG_SHOWN: usize = 3;

/// The changelog entries after `from`, as markdown lines: the last
/// `CHANGELOG_SHOWN`, after a line for the older ones when there are more.
pub fn changelog_since(from: u32) -> String {
    let after: Vec<&(u32, &str)> = CHANGELOG.iter().filter(|(v, _)| *v > from).collect();
    let skip = after.len().saturating_sub(CHANGELOG_SHOWN);
    let older = if skip > 0 { format!("- v{}-v{}: older changes, superseded\n", after[0].0, after[skip - 1].0) } else { String::new() };
    older + &after[skip..].iter().map(|(v, what)| format!("- v{v}: {what}\n")).collect::<String>()
}

/// The text a running member gets when its rules are older than `VERSION`:
/// the header the member recognises, the changelog, the full new rules.
pub fn update_text(id: &Id, from: u32) -> String {
    format!("nimitz rules updated v{from}\u{2192}v{VERSION}\n\nWhat changed:\n{}\nYour role's rules now:\n\n{}", changelog_since(from), text(id))
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
    /// the hash of the fleet's harness rules it got (`harness::rules`; none: none)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
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

/// Read-modify-write `key`'s record under `rules-delivered.lock` (tried for
/// `LOCK_WAIT`), written as a temp file and renamed: `f` gets the record
/// there (None: none yet) and gives the new one. The role rules' hook and the
/// harness's run side by side, so each changes only its own fields. False
/// when the record could not be written.
fn update_record(env: &Env, key: &str, f: impl FnOnce(Option<Delivery>) -> Delivery) -> bool {
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
    let d = f(all.remove(key));
    all.insert(key.to_string(), d);
    let tmp = path.with_extension("json.tmp");
    serde_json::to_vec_pretty(&all).ok().and_then(|b| std::fs::write(&tmp, b).ok()).is_some() && std::fs::rename(&tmp, &path).is_ok()
}

/// Record that `key` now has the role rules `VERSION` (the harness hash it
/// got stays as it was). False when the record could not be written.
pub fn record_delivery(env: &Env, key: &str, session_id: &str, via: &str) -> bool {
    let at = crate::evidence::now();
    update_record(env, key, |old| Delivery { version: VERSION, at, session_id: session_id.to_string(), via: via.to_string(), harness: old.and_then(|d| d.harness) })
}

/// Record that `key` now has the fleet's harness rules of hash `harness`
/// (None: none); the role rules' version stays as it was (a record made
/// here first takes the version the session runs on, `delivered_version`).
pub fn record_harness(env: &Env, key: &str, session_id: &str, via: &str, harness: Option<&str>) -> bool {
    let had = delivered_version(env, key, session_id);
    let at = crate::evidence::now();
    let harness = harness.map(str::to_string);
    update_record(env, key, |old| match old {
        Some(d) => Delivery { harness, ..d },
        None => Delivery { version: had, at, session_id: session_id.to_string(), via: via.to_string(), harness },
    })
}

/// The update `id`'s session (recorded under `key`) must get now: None when
/// it already runs on `VERSION`. Records the delivery; the caller hands the
/// text to the session as context. One text per version per key, whichever
/// per-turn hook event comes first. No record, no update: a record that
/// cannot be written (the state folder is read-only, the lock is held) would
/// hand the same text out at every prompt; `check` shows `rules.old` (and
/// `registry.readonly`) instead. The fleet's harness rules are not in it:
/// they come in their own hook entry (`harness_for`).
pub fn update_for(env: &Env, id: &Id, key: &str, session_id: &str, via: &str) -> Option<String> {
    let had = delivered_version(env, key, session_id);
    if had >= VERSION || !record_delivery(env, key, session_id, via) {
        return None;
    }
    Some(update_text(id, had))
}

/// The fleet's harness rules `id`'s session must get now, from their own
/// hook entry (`nimitz hook rules harness`): at a session start (`start`)
/// always, when there are any; on a later event only when they changed since
/// its record (and one line when they were taken away). Records the hash it
/// gave; on a later event, no record, no update.
pub fn harness_for(env: &Env, id: &Id, key: &str, session_id: &str, via: &str, start: bool) -> Option<String> {
    let now = crate::harness::rules(env, id.fleet());
    let hash = now.as_ref().map(|(_, h)| h.clone());
    if !start {
        let all = read_deliveries(env);
        let had = all.get(key).or_else(|| all.get(&format!("session:{session_id}"))).and_then(|d| d.harness.clone());
        if had == hash || !record_harness(env, key, session_id, via, hash.as_deref()) {
            return None;
        }
    } else {
        record_harness(env, key, session_id, via, hash.as_deref());
    }
    let f = id.fleet();
    match (now, start) {
        (Some((body, _)), true) => Some(body.trim_start().to_string()),
        (Some((body, _)), false) => Some(format!("nimitz rules updated: fleet {f}'s own rules (its harness) changed\n{body}")),
        (None, true) => None,
        (None, false) => Some(format!("nimitz rules updated: fleet {f}'s own rules (its harness) are gone: drop what they said.\n")),
    }
}

/// The most characters nimitz hands a session as one hook's context (Claude
/// Code gives a hook's context whole up to 10,000; above, a 2,000-character
/// preview and a file).
pub const CONTEXT_MAX: usize = 9_800;

/// The rules `id` is given (`nimitz rules`): its role's (`text`), then its
/// fleet's harness rules (`harness::rules`), each its own context.
pub fn text_for(env: &Env, id: &Id) -> String {
    let mut t = text(id);
    if let Some((body, _)) = crate::harness::rules(env, id.fleet()) {
        t.push_str(&body);
    }
    t
}

const COMMON: &str = include_str!("../rules/common.md");
const ADMIRAL: &str = include_str!("../rules/admiral.md");
const COMMODORE: &str = include_str!("../rules/commodore.md");
const SHIP: &str = include_str!("../rules/ship.md");
/// The admiral's and the commodores' part after the common one: `start`
/// (a ship may not run it).
const LEADS: &str = include_str!("../rules/leads.md");

/// The rules for `id`'s role, filled in for `id`.
pub fn text(id: &Id) -> String {
    let role = id.role();
    let body = match role {
        Role::Admiral => format!("{LEADS}{ADMIRAL}"),
        Role::Commodore => format!("{LEADS}{COMMODORE}"),
        Role::Ship => SHIP.to_string(),
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
        assert!(t.contains("`ship new|set|rm\n  HOME/GITHUB/<SHIP>`") && t.contains("[nimitz needs-you]"), "{t}");
        assert!(!placeholder_left(&t), "every placeholder filled: {t}");
        let a = text(&Id::admiral("HOME"));
        assert!(a.contains("## Admiral of HOME") && a.contains("You run fleet HOME"));
        let s = text(&Id::ship("HOME", "GITHUB", "DEV"));
        assert!(s.contains("[HOME.GITHUB]-DEV, the ship in squadron GITHUB") && s.contains("no one watches your pane"));
        assert!(!t.contains("tender") && !a.contains("tender") && !s.contains("tender"), "the role is gone");
        for id in [Id::admiral("A"), Id::commodore("A", "B"), Id::ship("A", "B", "C")] {
            assert!(text(&id).len() < 10_000, "fits SessionStart's 10,000-character context limit");
            assert!(update_text(&id, 0).len() < 10_000, "the whole changelog plus the text fits too");
        }
        // 0.7.3: no authority lists, open messaging, paths, the new verbs
        for r in [&t, &a, &s] {
            assert!(r.contains("Every member may run every `nimitz` command on any member") && r.contains("any member may message any member") && r.contains("`F/...`") && r.contains("`nimitz open <id>`"), "{r}");
            assert!(!r.contains("may not") && !r.contains("Never another") && !r.contains("comms") && !r.contains("-p`") && !r.contains(" -f`"), "{r}");
        }
        assert!(s.contains("`nimitz sweep HOME/GITHUB/DEV`"), "{s}");
        // v11 (issue #9): settings through nimitz; a lead's own unit's brief and prompts by hand
        for r in [&t, &a, &s] {
            assert!(r.contains("Change fleet and squadron settings only through nimitz commands") && r.contains("A lead\n  may edit its own unit's brief and prompts by hand") && r.contains("never another unit's files,\n  never machine state"), "{r}");
            assert!(!r.contains("Never edit fleet files"), "{r}");
        }
        // `start`, `restart`, `muster` are in the leads' rules, not a ship's
        for r in [&t, &a] {
            assert!(r.contains("`nimitz start <path>`") && r.contains("`nimitz restart <path>`") && r.contains("`nimitz muster`"), "{r}");
        }
        assert!(!s.contains("nimitz start <path>"), "{s}");
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
        assert!(since1.starts_with(&format!("- v2-v{}: older changes, superseded\n", VERSION - 3)) && !since1.contains("- v1:"), "{since1}");
        assert_eq!(since1.lines().count(), 4, "the older ones in one line, then the last three: {since1}");
        assert_eq!(changelog_since(VERSION - 2).lines().count(), 2, "{}", changelog_since(VERSION - 2));
        let id = Id::ship("F", "SQ", "S");
        let u = update_text(&id, 1);
        assert!(u.starts_with(&format!("nimitz rules updated v1\u{2192}v{VERSION}\n")), "{u}");
        assert!(u.contains(&since1) && u.ends_with(&text(&id)), "the changelog, then the full text");
    }

    /// The fleet's harness rules come in their own context: whole at every
    /// session start, and a change (or their end) reaches a member at its
    /// next prompt, once. The role rules' record and theirs never undo each
    /// other.
    #[test]
    fn harness_rules_come_on_their_own_and_a_change_is_delivered_once() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        assert_eq!(text_for(&env, &id), text(&id), "no harness: the role rules alone");
        assert!(update_for(&env, &id, "job1", "s1", "SessionStart").is_some());
        assert_eq!(harness_for(&env, &id, "job1", "s1", "SessionStart", true), None, "no harness: nothing");
        assert_eq!(harness_for(&env, &id, "job1", "s1", "UserPromptSubmit", false), None);
        let rules = crate::harness::dir(&env, "MAIN").join("rules");
        std::fs::create_dir_all(&rules).unwrap();
        std::fs::write(rules.join("style.md"), "Answer in haiku.\n").unwrap();
        let t = text_for(&env, &id);
        assert!(t.starts_with(&text(&id)) && t.contains("### style\n\n> Answer in haiku.\n") && t.ends_with("(End of fleet MAIN's own rules.)\n"), "{t}");
        let u = harness_for(&env, &id, "job1", "s1", "UserPromptSubmit", false).expect("the change goes out");
        assert!(u.starts_with("nimitz rules updated: fleet MAIN's own rules (its harness) changed") && u.contains("Answer in haiku."), "{u}");
        assert_eq!(harness_for(&env, &id, "job1", "s1", "UserPromptSubmit", false), None, "once");
        assert_eq!(update_for(&env, &id, "job1", "s1", "UserPromptSubmit"), None, "the role rules' record stands");
        // a session start (a /clear, a /compact) gets them again, whole
        let s = harness_for(&env, &id, "job1", "s1", "SessionStart", true).unwrap();
        assert!(s.starts_with("## Fleet MAIN's own rules") && s.contains("Answer in haiku."), "{s}");
        // the role rules' delivery keeps the harness hash: no resend
        assert!(record_delivery(&env, "job1", "s1", "SessionStart"));
        assert_eq!(harness_for(&env, &id, "job1", "s1", "UserPromptSubmit", false), None);
        std::fs::remove_file(rules.join("style.md")).unwrap();
        let g = harness_for(&env, &id, "job1", "s1", "UserPromptSubmit", false).expect("a rule taken away is a change too");
        assert!(g.contains("are gone"), "{g}");
        assert_eq!(harness_for(&env, &id, "job1", "s1", "UserPromptSubmit", false), None, "once");
        // a harness record made first keeps the version the session runs on
        std::fs::write(rules.join("style.md"), "Answer in haiku.\n").unwrap();
        assert!(harness_for(&env, &id, "job2", "s2", "SessionStart", true).is_some());
        assert_eq!(delivered(&env, "job2").unwrap().version, 0);
        assert!(update_for(&env, &id, "job2", "s2", "UserPromptSubmit").is_some(), "the role rules still go out");
        assert!(delivered(&env, "job2").unwrap().harness.is_some(), "and the harness hash stays");
        // another fleet's member is not touched by MAIN's harness
        assert_eq!(text_for(&env, &Id::ship("BETA", "S", "X")), text(&Id::ship("BETA", "S", "X")));
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
