//! The layout of fleets in a multiplexer, as data (contracts.md section 6.3 of
//! plan-v3; section 9). Core never depends on the mux: the CLI and Admiralty map
//! a `LayoutSpec` onto `nimitz_mux::LayoutPlan`. Owner: core-sessions.
//!
//! One workspace per fleet (its admiral) and one per squadron (its commodore).
//! Every member is a background job (plan-v4 section 7), so every tab attaches
//! (`claude attach <job>`; detaching leaves the job running; an attach of a
//! parked job wakes it, cli-07): a lead with a job gets its tab (`F.ADMIRAL`,
//! `F.SQ.COMMODORE`); with `ships`, a live ship too. A member with no job gets
//! no tab (`nimitz start` first); the mux layer drops the tabs whose session
//! already has a pane (`TabSpec::session_id`) and never types into a pane that
//! runs something (`nimitz_mux::tab_state`).
use crate::env::Env;
use crate::types::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabSpec {
    pub label: String,
    pub id: Id,
    /// `claude attach <job>`
    pub cmd: Vec<String>,
    pub cwd: String,
    /// the session the tab shows (skip it if a pane has it already)
    pub session_id: Option<String>,
    /// the job is parked or stopped: the attach wakes it as itself (cli-07)
    pub wakes: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSpec {
    pub label: String,
    pub tabs: Vec<TabSpec>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutSpec {
    pub workspaces: Vec<WorkspaceSpec>,
    /// what will run (one step per tab), with skips as `layout.skip` warnings
    pub plan: Plan,
}

/// Workspace label: `F` for the admiral, `F.SQ` for everything in a squadron.
pub fn workspace_of(id: &Id) -> String {
    match id.squadron() {
        Some(sq) => format!("{}.{sq}", id.fleet()),
        None => id.fleet().to_string(),
    }
}

/// Tab label, the member's full place: `F.ADMIRAL`, `F.SQ.COMMODORE`,
/// `F.SQ.SHIP`.
/// (Before 0.3.1 the tab was only the last part; `legacy_tab_of` gives that,
/// so a layout still finds an old tab.)
pub fn tab_of(id: &Id) -> String {
    format!("{}.{}", workspace_of(id), legacy_tab_of(id))
}

/// The short tab label of 0.3.0 and before: ADMIRAL, COMMODORE or the ship's
/// name.
pub fn legacy_tab_of(id: &Id) -> String {
    match id.role() {
        Role::Admiral => "ADMIRAL".into(),
        Role::Commodore => "COMMODORE".into(),
        Role::Ship => id.parts().get(2).map(|s| s.to_string()).unwrap_or_default(),
    }
}

fn skip(id: &Id, message: String) -> Issue {
    Issue { code: "layout.skip".into(), severity: Severity::Warn, message, fix: None, id: Some(id.clone()) }
}

fn live(m: &Member) -> bool {
    m.session.as_ref().map(|s| s.pid.is_some()).unwrap_or(false)
}

/// The tab that attaches to `m`'s job (`claude attach <job>` in its folder,
/// labelled `tab_of`): what `nimitz layout`, `nimitz open`, Admiralty's `A`
/// and a restore run in the member's tab. A parked or stopped job is woken by
/// the attach itself (cli-07; `wakes`), so a lead with a shell tab needs no
/// resume first; its transcript must be there (else `history.lost`, as for a
/// resume). A member with no job row, or one that runs with no job (an
/// interactive or SDK process), gets a `layout.skip` instead, with what to do.
pub fn attach_tab(env: &Env, m: &Member) -> Result<TabSpec, Issue> {
    let id = &m.id;
    let Some(s) = m.session.as_ref() else {
        return Err(skip(id, format!("{id} has no job; no tab (`nimitz start {id}` first)")));
    };
    let Some(short) = &s.short_id else {
        return Err(skip(id, format!("{id} runs in a process with no job ({}); nothing to attach to", s.entrypoint.as_deref().unwrap_or(&s.kind))));
    };
    let wakes = !live(m);
    if wakes {
        // the relaunch table's checks hold for a wake by attach too: a drifted
        // name or missing labels need the relabel (`nimitz start` does it),
        // and a lost transcript the cold launch
        if let Some(i) = crate::sessions::session_issues(env, m).into_iter().find(|i| matches!(i.code.as_str(), "name.drift" | "labels.missing")) {
            return Err(Issue { fix: Some(format!("nimitz start {id}  # relabels it, then nimitz open {id}")), ..i });
        }
        if crate::evidence::transcript(env, &mut crate::evidence::FileCache::default(), m.cwd.as_deref(), &s.session_id).is_none() {
            return Err(Issue {
                code: "history.lost".into(),
                severity: Severity::Error,
                message: format!("{id}'s conversation {} has no transcript; an attach would wake it with no history", s.session_id),
                fix: Some(format!("nimitz start {id} --fresh")),
                id: Some(id.clone()),
            });
        }
    }
    Ok(TabSpec {
        label: tab_of(id),
        id: id.clone(),
        cmd: crate::claude::attach(env, short),
        cwd: m.cwd.clone().unwrap_or_else(|| ".".into()),
        session_id: Some(s.session_id.clone()),
        wakes,
    })
}

impl LayoutSpec {
    fn push(&mut self, tab: TabSpec, why: String) {
        let ws = workspace_of(&tab.id);
        self.plan.steps.push(Step { id: Some(tab.id.clone()), cmd: tab.cmd.clone(), cwd: Some(tab.cwd.clone()), why });
        match self.workspaces.iter_mut().find(|w| w.label == ws) {
            Some(w) => w.tabs.push(tab),
            None => self.workspaces.push(WorkspaceSpec { label: ws, tabs: vec![tab] }),
        }
    }

    fn attach(&mut self, env: &Env, m: &Member) {
        match attach_tab(env, m) {
            Ok(tab) => {
                let wake = if tab.wakes { "; it wakes the parked job" } else { "" };
                let why = format!("tab {} in workspace {}: attach{wake}; detaching (ctrl-z) leaves it running", tab.label, workspace_of(&m.id));
                self.push(tab, why);
            }
            Err(i) => self.plan.warnings.push(i),
        }
    }
}

/// The layout for `fleet` (or every fleet that runs here).
pub fn plan(env: &Env, snap: &Snapshot, fleet: Option<&str>, ships: bool, force: bool) -> LayoutSpec {
    let _ = force; // every tab attaches; nothing is started or forced here
    let mut spec = LayoutSpec::default();
    spec.plan.summary = format!("layout herdr {}", fleet.unwrap_or("(every fleet here)"));
    for f in snap.fleets.iter().filter(|f| fleet.map(|x| x == f.id.fleet()).unwrap_or(true)) {
        if f.machine.is_none() {
            spec.plan.warnings.push(skip(&f.id, format!("fleet {} does not run on this machine; no layout", f.id)));
            continue;
        }
        // in tab order: the admiral, then each commodore and (with `ships`) its ships
        spec.attach(env, &f.admiral);
        for sq in &f.squadrons {
            spec.attach(env, &sq.commodore);
            if ships {
                // a ship parks on purpose: only a live one gets a tab here
                for m in sq.ships.iter() {
                    if live(m) {
                        spec.attach(env, m);
                    } else {
                        spec.plan.warnings.push(skip(&m.id, format!("{} is not live; no tab (`nimitz start {}` first)", m.id, m.id)));
                    }
                }
            }
        }
    }
    spec
}

/// Drop the tabs whose session already has a pane (the mux knows which), with
/// a `layout.skip` warning each. The step list follows.
pub fn drop_shown(spec: &mut LayoutSpec, shown_sessions: &[String]) {
    let mut dropped: Vec<Id> = vec![];
    for w in &mut spec.workspaces {
        w.tabs.retain(|t| {
            let keep = !t.session_id.as_ref().map(|s| shown_sessions.contains(s)).unwrap_or(false);
            if !keep {
                dropped.push(t.id.clone());
            }
            keep
        });
    }
    spec.workspaces.retain(|w| !w.tabs.is_empty());
    for id in dropped {
        spec.plan.steps.retain(|s| !(s.id.as_ref() == Some(&id) && s.cmd.get(1).map(|c| c == "attach").unwrap_or(false)));
        spec.plan.warnings.push(skip(&id, format!("{id} already has a pane; `nimitz open {id}` shows it")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_is_the_members_full_place() {
        assert_eq!(tab_of(&Id::ship("F", "SQ", "S")), "F.SQ.S");
        assert_eq!(tab_of(&Id::commodore("F", "SQ")), "F.SQ.COMMODORE");
        assert_eq!(tab_of(&Id::admiral("F")), "F.ADMIRAL");
    }
}
