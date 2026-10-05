//! The layout of fleets in a multiplexer, as data (contracts.md section 6.3 of
//! plan-v3; section 9). Core never depends on the mux: the CLI and Admiralty map
//! a `LayoutSpec` onto `nimitz_mux::LayoutPlan`. Owner: core-sessions.
//!
//! One workspace per fleet (its admiral) and one per squadron (its commodore).
//! A member that is not running gets a tab with the step `nimitz start` would
//! run for it (a pane step); a running one is left where it is. With `ships`, a
//! live ship gets a tab that attaches to it; the mux layer drops the ones that
//! already have a pane (`TabSpec::session_id`).
use crate::env::Env;
use crate::types::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum TabKind {
    /// open the member's start command (admiral, commodore)
    Start,
    /// attach to a live background ship; detaching leaves it running
    Attach,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabSpec {
    pub label: String,
    pub id: Id,
    pub kind: TabKind,
    pub cmd: Vec<String>,
    pub cwd: String,
    /// the live session an Attach tab shows (skip it if a pane has it already)
    pub session_id: Option<String>,
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
/// `F.SQ.SHIP` or `F.SQ.~task`. (Before 0.3.1 the tab was only the last part;
/// `legacy_tab_of` gives that, so a layout still finds an old tab.)
pub fn tab_of(id: &Id) -> String {
    format!("{}.{}", workspace_of(id), legacy_tab_of(id))
}

/// The short tab label of 0.3.0 and before: ADMIRAL, COMMODORE, the ship's
/// name or `~task`.
pub fn legacy_tab_of(id: &Id) -> String {
    match id.role() {
        Role::Admiral => "ADMIRAL".into(),
        Role::Commodore => "COMMODORE".into(),
        _ => id.parts().get(2).map(|s| s.to_string()).unwrap_or_default(),
    }
}

fn skip(id: &Id, message: String) -> Issue {
    Issue { code: "layout.skip".into(), severity: Severity::Warn, message, fix: None, id: Some(id.clone()) }
}

fn live(m: &Member) -> bool {
    m.session.as_ref().map(|s| s.pid.is_some()).unwrap_or(false)
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
}

/// The layout for `fleet` (or every fleet that runs here). You lay out any
/// fleet; an admiral its own.
pub fn plan(env: &Env, snap: &Snapshot, fleet: Option<&str>, ships: bool, caller: &Caller, force: bool) -> LayoutSpec {
    let mut spec = LayoutSpec {
        plan: Plan { summary: format!("layout herdr {}", fleet.unwrap_or("(every fleet here)")), ..Default::default() },
        ..Default::default()
    };
    // you lay out any fleet; an admiral its own (by default too); nobody else
    let fleet = match (caller, fleet) {
        (Caller::User, f) => f,
        (Caller::Member { id, role: Role::Admiral }, None) => Some(id.fleet()),
        (Caller::Member { id, role: Role::Admiral }, Some(f)) if f == id.fleet() => Some(f),
        (Caller::Member { id, .. }, _) => {
            spec.plan.refusals.push(Issue {
                code: "authority".into(),
                severity: Severity::Error,
                message: format!("`nimitz layout` lays out whole fleets: yours, or an admiral's for its own fleet; not {}'s", id.name()),
                fix: None,
                id: None,
            });
            return spec;
        }
    };
    for f in snap.fleets.iter().filter(|f| fleet.map(|x| x == f.id.fleet()).unwrap_or(true)) {
        if f.machine.is_none() {
            spec.plan.warnings.push(skip(&f.id, format!("fleet {} does not run on this machine; no layout", f.id)));
            continue;
        }
        let leads = std::iter::once(&f.admiral).chain(f.squadrons.iter().map(|s| &s.commodore));
        for m in leads {
            let id = &m.id;
            if live(m) {
                spec.plan.warnings.push(skip(id, format!("{id} is running already; `nimitz open {id}` shows it")));
                continue;
            }
            let p = crate::sessions::plan_start(env, snap, id, caller, force);
            let step = p.steps.iter().find(|s| s.id.as_ref() == Some(id) && crate::sessions::is_pane_step(s)).cloned();
            match (p.ok(), step) {
                (true, Some(step)) => {
                    let tab = TabSpec {
                        label: tab_of(id),
                        id: id.clone(),
                        kind: TabKind::Start,
                        cmd: step.cmd.clone(),
                        cwd: step.cwd.clone().unwrap_or_else(|| ".".into()),
                        session_id: None,
                    };
                    let why = format!("tab {} in workspace {}", tab_of(id), workspace_of(id));
                    spec.push(tab, why);
                }
                _ => spec.plan.warnings.extend(p.refusals.into_iter().map(|r| Issue { severity: Severity::Warn, ..r })),
            }
        }
        if ships {
            for m in f.squadrons.iter().flat_map(|s| s.ships.iter()) {
                let id = &m.id;
                let Some(s) = m.session.as_ref().filter(|_| live(m)) else {
                    spec.plan.warnings.push(skip(id, format!("{id} is not live; no tab (`nimitz start {id}` first)")));
                    continue;
                };
                let Some(short) = &s.short_id else {
                    spec.plan.warnings.push(skip(id, format!("{id} has no job id to attach to")));
                    continue;
                };
                let tab = TabSpec {
                    label: tab_of(id),
                    id: id.clone(),
                    kind: TabKind::Attach,
                    cmd: crate::claude::attach(env, short),
                    cwd: m.cwd.clone().unwrap_or_else(|| ".".into()),
                    session_id: Some(s.session_id.clone()),
                };
                let why = format!("tab {} in workspace {}: attach; detaching leaves it running", tab_of(id), workspace_of(id));
                spec.push(tab, why);
            }
        }
    }
    spec
}

/// Drop the Attach tabs whose session already has a pane (the mux knows which),
/// with a `layout.skip` warning each. The step list follows.
pub fn drop_shown(spec: &mut LayoutSpec, shown_sessions: &[String]) {
    let mut dropped: Vec<Id> = vec![];
    for w in &mut spec.workspaces {
        w.tabs.retain(|t| {
            let keep = t.kind != TabKind::Attach || !t.session_id.as_ref().map(|s| shown_sessions.contains(s)).unwrap_or(false);
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
