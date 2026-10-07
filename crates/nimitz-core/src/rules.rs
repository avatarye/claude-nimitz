//! Role rules and role briefs. Owner: core-sessions.
//!
//! The rules (`rules/*.md`, compiled in) are what a member must know about
//! its role: the authority table, the chain, plan-first. The SessionStart hook
//! hands them to every labelled session (startup, resume, clear, compact), so a
//! new nimitz build updates every member at its next start without touching a
//! file. `VERSION` is logged with each SessionStart; `check` names a live
//! session that started on older rules.
//!
//! The brief (`CLAUDE.md` in an admiral's or commodore's folder) holds only
//! who the member is, and the user's own notes. Every brief nimitz ever
//! generated is listed here, so `doctor --fix` can replace an old generated
//! one and leave a hand-edited one alone.
use crate::env::Env;
use crate::types::*;
use std::path::PathBuf;

/// Bump when the rules text changes in a way members must pick up.
pub const VERSION: u32 = 1;

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
    let where_ = match role {
        Role::Admiral => format!("of fleet {}", id.fleet()),
        Role::Commodore => format!("of squadron {sq} in fleet {}", id.fleet()),
        _ => format!("in squadron {sq} of fleet {}", id.fleet()),
    };
    format!("{COMMON}{body}")
        .replace("{VERSION}", &VERSION.to_string())
        .replace("{ID}", &id.0)
        .replace("{NAME}", &id.name())
        .replace("{ROLE}", role.as_str())
        .replace("{WHERE}", &where_)
        .replace("{FLEET}", id.fleet())
        .replace("{SQ}", sq)
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
/// replaces it); `rules.old` for each live member whose session started on
/// older rules (its next start or /compact picks them up).
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
    let events = crate::evidence::events(env);
    for f in &snap.fleets {
        let mut ms = vec![&f.admiral];
        for sq in &f.squadrons {
            ms.push(&sq.commodore);
            ms.extend(sq.ships.iter());
        }
        for m in ms {
            let Some(s) = m.session.as_ref().filter(|s| s.pid.is_some()) else { continue };
            // the newest SessionStart of this session (startup, resume, clear, compact)
            let Some(start) = events.iter().rev().find(|e| e.event == "SessionStart" && e.session_id.as_deref() == Some(s.session_id.as_str())) else {
                continue;
            };
            let had = start.rules.unwrap_or(0);
            if had < VERSION {
                out.push(Issue {
                    code: "rules.old".into(),
                    severity: Severity::Info,
                    message: format!("{} started on nimitz rules v{had}; the rules are now v{VERSION}", m.id),
                    fix: Some(format!("/compact (or /clear) in its session, or nimitz relaunch {}", m.id)),
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

    #[test]
    fn each_role_gets_its_rules_filled_in() {
        let t = text(&Id::commodore("HOME", "GITHUB"));
        assert!(t.starts_with(&format!("# nimitz fleet rules (rules v{VERSION}) for HOME/GITHUB")), "{t}");
        assert!(t.contains("You are [HOME.GITHUB]-COMMODORE, the commodore of squadron GITHUB in fleet HOME"));
        assert!(t.contains("`ship new|set|rm HOME/GITHUB/<SHIP>`") && t.contains("[nimitz needs-you]"));
        assert!(!t.contains('{'), "every placeholder filled: {t}");
        let a = text(&Id::admiral("HOME"));
        assert!(a.contains("## Admiral of HOME") && a.contains("Never another fleet"));
        let s = text(&Id::ship("HOME", "GITHUB", "DEV"));
        assert!(s.contains("[HOME.GITHUB]-DEV, the ship in squadron GITHUB") && s.contains("no one watches your pane"));
        assert!(!t.contains("tender") && !a.contains("tender") && !s.contains("tender"), "the role is gone");
        for id in [Id::admiral("A"), Id::commodore("A", "B"), Id::ship("A", "B", "C")] {
            assert!(text(&id).len() < 10_000, "fits SessionStart's 10,000-character context limit");
        }
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
