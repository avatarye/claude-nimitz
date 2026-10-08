//! App state and keys. No terminal here: the run loop feeds keys and data in
//! and carries out the `Effect`s that come back, so tests drive `App` directly.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use nimitz_core::{Id, Issue, Member, Outcome, Role, Severity, Status};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Style;

use crate::actions::{self, Act, Ctx, MessagePlan, Pending, Target};
use crate::data::{pane_for, Bundle};
use crate::forms::{Form, FormKey};
use crate::theme::{bold, mono, st, Icons, ASCII, NERD};
use crate::view::{all_issues, build_nodes, flatten, FleetState, Kind, Row, Section, View};

/// One line of the `!` list.
#[derive(Clone, Debug)]
pub struct NeedItem {
    pub row_id: String,
    pub name: String,
    pub age: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub enum MenuAction {
    Act(Act),
    NewFleet,
    NewSquadron(String),
    NewShip(String, String),
    EditShip(Id),
    EditFleet(String),
    EditSquadron(Id),
    MachineRm(String),
    /// adopt this session as this identity (from a `legacy` issue's fix)
    AdoptAs(String, String),
}

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub key: char,
    pub label: String,
    pub enabled: Result<(), String>,
    pub action: MenuAction,
}

pub enum Modal {
    Help { scroll: usize },
    /// `f`: which fleet states are shown
    FleetFilter { sel: usize },
    Needs { items: Vec<NeedItem>, sel: usize },
    /// `i`: every issue, worst first
    Issues { sel: usize },
    Menu { title: String, items: Vec<MenuItem>, sel: usize },
    Confirm(Pending),
    Message { plan: MessagePlan, text: String },
    Form(Box<Form>),
}

pub enum Effect {
    Quit,
    Refresh,
    /// open the plan's panes, then apply it on a worker thread
    Run(Pending),
    /// edit the focused form field in `$EDITOR`
    Editor,
}

pub struct Toast {
    pub msg: String,
    pub style: Style,
    pub at: Instant,
}

/// What sticks between runs.
#[derive(serde::Serialize, serde::Deserialize, Default, PartialEq, Clone, Debug)]
pub struct Saved {
    pub expanded: HashMap<String, bool>,
    pub filter: String,
    pub fleet_states: Vec<bool>,
    pub flagged_only: bool,
    pub show_stowaways: bool,
    // `split` (the side or bottom detail pane before 0.3.1) is no longer kept;
    // a saved state that has it still loads (unknown fields are ignored)
    /// the fleet tab shown (a fleet id, or view::UNASSIGNED)
    #[serde(default)]
    pub tab: Option<String>,
}

pub struct App {
    pub ctx: Ctx,
    pub bundle: Option<Bundle>,
    pub rows: Vec<Row>,
    pub cursor: usize,
    /// first visible line of the frame list
    pub scroll: usize,
    pub expanded: HashMap<String, bool>,
    pub filter: String,
    pub editing_filter: bool,
    pub show_stowaways: bool,
    pub flagged_only: bool,
    pub fleet_states: [bool; 7],
    /// the fleet tab shown (a fleet id, or view::UNASSIGNED); fixed up on each rebuild
    pub tab: Option<String>,
    pub modal: Option<Modal>,
    pub toast: Option<Toast>,
    pub icons: &'static Icons,
    /// the floating detail panel (`w`); closed at start
    pub detail_open: bool,
    /// fixtures: a frozen clock so ages never move
    pub fixed_now: Option<f64>,
    pub state_path: Option<PathBuf>,
    /// an action running on the worker thread
    pub busy: Option<String>,
    /// lines the detail pane is scrolled down (ctrl-e / ctrl-y); 0 on a new row
    pub detail_scroll: usize,
    detail_for: Option<String>,
    pub page: usize,
    last_g: Option<Instant>,
    last_saved: Option<Saved>,
}

/// A row's name where its context is missing (a toast, a menu title): a
/// member's full expected name (`[F.SQ]-NAME`; the row may show its own part,
/// or a drifted session's name), else the row's own name.
fn full_name(r: &Row) -> String {
    match (&r.node.member, r.node.kind) {
        (Some(id), Kind::Member(_)) => id.name(),
        _ => r.node.name.clone(),
    }
}

pub const HELP: &[(&str, &str)] = &[
    ("tab / shift-tab", "next / previous fleet tab (each fleet, then unassigned stowaways)"),
    ("1 - 9", "fleet tab N"),
    ("j / k", "down / up"),
    ("h / l", "fold or go to parent / unfold or go to first child"),
    ("gg / G", "top / bottom"),
    ("ctrl-d / ctrl-u", "half page down / up"),
    ("ctrl-e / ctrl-y", "scroll the detail panel (when open)"),
    ("space", "fold / unfold"),
    ("z", "show / hide the member's crew that finished over 30 min ago (+N finished)"),
    ("/  n / N", "filter by name, in every fleet; next / previous match"),
    ("enter", "jump to the row's pane (else fold / unfold)"),
    ("J / K", "next / previous row with a pane"),
    ("!", "everything waiting on you"),
    ("i", "every issue, with its fix command"),
    ("a", "start the member, top-down (confirms)"),
    ("u", "up: start everything the fleet runs here (confirms)"),
    ("A", "open: attach to a background session in a new pane"),
    ("x", "actions: stop, relaunch, sweep, rm, adopt, remove; on a fleet: layout, machines"),
    ("m", "message (ships: relayed by their commodore)"),
    ("c / e / D", "create fleet, squadron, ship / edit it / remove from config"),
    ("y", "yank the session id (in i: the fix command)"),
    ("I", "show / hide stowaways"),
    ("f / F", "filter squadrons by state / flagged rows only"),
    ("w", "detail: open or close the floating detail panel (esc closes it too)"),
    ("R", "refresh now"),
    ("esc", "close what is open (a dialog, the detail panel, the / name filter); with nothing open and nothing running, quit"),
    ("? / q", "help / quit"),
];

impl App {
    pub fn new(ctx: Ctx, state_path: Option<PathBuf>, ascii: bool) -> App {
        let mut app = App {
            ctx,
            bundle: None,
            rows: vec![],
            cursor: 0,
            scroll: 0,
            expanded: HashMap::new(),
            filter: String::new(),
            editing_filter: false,
            show_stowaways: true,
            flagged_only: false,
            fleet_states: [true; 7],
            tab: None,
            modal: None,
            toast: None,
            icons: if ascii { &ASCII } else { &NERD },
            detail_open: false,
            fixed_now: None,
            state_path,
            busy: None,
            detail_scroll: 0,
            detail_for: None,
            page: 20,
            last_g: None,
            last_saved: None,
        };
        app.restore();
        app
    }

    pub fn now(&self) -> f64 {
        self.fixed_now.unwrap_or_else(nimitz_core::evidence::now)
    }

    pub fn monitor_only(&self) -> bool {
        !self.bundle.as_ref().map(|b| b.mux_caps.open).unwrap_or(false)
    }

    // ---------------------------------------------------------------- sticky state

    fn saved(&self) -> Saved {
        Saved {
            expanded: self.expanded.clone(),
            filter: self.filter.clone(),
            fleet_states: self.fleet_states.to_vec(),
            flagged_only: self.flagged_only,
            show_stowaways: self.show_stowaways,
            tab: self.tab.clone(),
        }
    }

    fn restore(&mut self) {
        let Some(path) = &self.state_path else { return };
        let Some(s) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Saved>(&b).ok()) else { return };
        self.expanded = s.expanded.clone();
        self.filter = s.filter.clone();
        if s.fleet_states.len() == self.fleet_states.len() {
            self.fleet_states.copy_from_slice(&s.fleet_states);
        }
        self.flagged_only = s.flagged_only;
        self.show_stowaways = s.show_stowaways;
        self.tab = s.tab.clone();
        self.last_saved = Some(s);
    }

    /// Save the sticky state when it changed.
    pub fn persist(&mut self) {
        let now = self.saved();
        if self.last_saved.as_ref() == Some(&now) {
            return;
        }
        let Some(path) = &self.state_path else { return };
        if actions::write_atomic(path, &serde_json::to_vec_pretty(&now).unwrap_or_default()).is_ok() {
            self.last_saved = Some(now);
        }
    }

    // ---------------------------------------------------------------- data

    /// Take a refresh: match panes to members, mark blocked panes as needing you.
    pub fn apply(&mut self, mut b: Bundle) {
        let panes = b.panes.clone();
        let mark = |m: &mut Member| {
            if m.pane.is_none() {
                let sid = m.session.as_ref().map(|s| s.session_id.clone());
                let name = m.session.as_ref().map(|s| s.name.clone()).unwrap_or(m.expected_name.clone());
                m.pane = pane_for(sid.as_deref(), &name, m.cwd.as_deref(), &panes).map(|p| p.pane.pane_id.clone());
            }
            let blocked = m.pane.as_deref().and_then(|id| panes.iter().find(|p| p.pane.pane_id == id)).map(|p| p.state == "blocked").unwrap_or(false);
            let live = m.session.as_ref().map(|s| s.pid.is_some()).unwrap_or(false);
            if blocked && live && m.status != Status::NeedsYou {
                m.status = Status::NeedsYou;
                m.reasons.insert(0, "pane: blocked at an approval or a question (multiplexer)".into());
            }
        };
        for f in &mut b.snap.fleets {
            mark(&mut f.admiral);
            for sq in &mut f.squadrons {
                mark(&mut sq.commodore);
                sq.ships.iter_mut().for_each(mark);
            }
        }
        self.bundle = Some(b);
        self.rebuild();
    }

    pub fn rebuild(&mut self) {
        let keep = self.rows.get(self.cursor).map(|r| r.node.id.clone());
        let now = self.now();
        let Some(b) = &self.bundle else { return };
        // the tab shown must exist: else the first one
        let tabs = crate::view::tabs(&b.snap);
        if !self.tab.as_ref().is_some_and(|t| tabs.contains(t)) {
            self.tab = tabs.first().cloned();
        }
        let view = View {
            filter: &self.filter,
            flagged_only: self.flagged_only,
            fleet_states: self.fleet_states,
            show_stowaways: self.show_stowaways,
            panes: &b.panes,
            icons: self.icons,
            now,
            tab: self.tab.as_deref(),
        };
        let nodes = build_nodes(&b.snap, &view);
        self.rows = flatten(&nodes, &self.expanded);
        if let Some(id) = keep {
            if let Some(i) = self.rows.iter().position(|r| r.node.id == id) {
                self.cursor = i;
            }
        }
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    pub fn current(&self) -> Option<&Row> {
        self.rows.get(self.cursor)
    }

    /// What the selected row is about.
    pub fn target(&self) -> Target {
        let Some(r) = self.current() else { return Target::None };
        match (r.node.kind, &r.node.member) {
            (Kind::FleetBand, Some(id)) => Target::Fleet(id.clone()),
            (Kind::SquadronBand, Some(id)) => Target::Squadron(id.clone()),
            (Kind::Member(_), Some(id)) => Target::Member(id.clone()),
            (Kind::Stowaway, _) => r.node.session_id.clone().map(Target::Stowaway).unwrap_or(Target::None),
            (Kind::ExtraRow, Some(owner)) => {
                Target::ExtraRow { owner: owner.clone(), session_id: r.node.session_id.clone().unwrap_or_default() }
            }
            _ => Target::None,
        }
    }

    /// The fleet the selected row is in.
    pub fn current_fleet(&self) -> Option<String> {
        let r = self.current()?;
        if let Some(id) = &r.node.member {
            return Some(id.fleet().to_string());
        }
        self.rows[..=self.cursor].iter().rev().find(|r| r.section == Section::FleetBand).and_then(|r| r.node.member.as_ref()).map(|i| i.fleet().to_string())
    }

    pub fn current_squadron(&self) -> Option<(String, String)> {
        let id = self.current()?.node.member.clone()?;
        id.squadron().map(|sq| (id.fleet().to_string(), sq.to_string()))
    }

    pub fn notify(&mut self, msg: impl Into<String>, style: Style) {
        self.toast = Some(Toast { msg: msg.into(), style, at: Instant::now() });
    }

    /// Put the cursor on the row with this id, showing its tab first when the
    /// row lives on another one.
    pub fn goto_id(&mut self, id: &str) -> bool {
        if !self.rows.iter().any(|r| r.node.id == id) {
            if let Some(t) = self.tab_of_row(id).filter(|t| Some(t) != self.tab.as_ref()) {
                self.tab = Some(t);
                self.rebuild();
            }
        }
        if let Some(i) = self.rows.iter().position(|r| r.node.id == id) {
            self.cursor = i;
            true
        } else {
            false
        }
    }

    /// The tab a row id belongs to: its stowaway's tab (`SW:<sid>`), else the
    /// fleet its id names (`F:F`, `M:F/SQ/X`, `C:F/SQ/X:crew`, ...).
    pub fn tab_of_row(&self, row_id: &str) -> Option<String> {
        let (kind, rest) = row_id.split_once(':')?;
        let b = self.bundle.as_ref()?;
        if kind == "SW" {
            return b.snap.stowaways.iter().find(|s| s.session_id == rest).map(|s| crate::view::stowaway_tab(&b.snap, s));
        }
        let fleet = rest.split([':', '/']).next()?;
        crate::view::tabs(&b.snap).into_iter().find(|t| t == fleet)
    }

    /// Show a row by id even if a filter or a fold hides it.
    pub fn reveal(&mut self, row_id: &str, member: Option<&Id>) {
        // a row on another tab: show that tab
        let tab = member.map(|id| id.fleet().to_string()).or_else(|| self.tab_of_row(row_id));
        if tab.is_some() && tab != self.tab {
            self.tab = tab;
            self.rebuild();
        }
        if let Some(id) = member {
            self.expanded.insert(format!("F:{}", id.fleet()), true);
            if let Some(sq) = id.squadron() {
                self.expanded.insert(format!("SQ:{}/{sq}", id.fleet()), true);
            }
            if let Some(b) = &self.bundle {
                if let Some(f) = b.snap.fleets.iter().find(|f| f.id.fleet() == id.fleet()) {
                    self.fleet_states[FleetState::of(f).index()] = true;
                }
            }
        }
        if !self.goto_id(row_id) {
            self.filter.clear();
            self.flagged_only = false;
            self.rebuild();
            self.goto_id(row_id);
        }
    }

    /// The tabs as the view has them now.
    pub fn tabs(&self) -> Vec<String> {
        self.bundle.as_ref().map(|b| crate::view::tabs(&b.snap)).unwrap_or_default()
    }

    /// Show tab `i` (wrapping), cursor on its first row.
    fn show_tab(&mut self, i: i64) {
        let tabs = self.tabs();
        if tabs.is_empty() {
            return;
        }
        let i = i.rem_euclid(tabs.len() as i64) as usize;
        if self.tab.as_deref() != Some(tabs[i].as_str()) {
            self.tab = Some(tabs[i].clone());
            self.cursor = 0;
            self.rebuild();
        }
    }

    /// Show the tab named `tab` (a fleet id, or view::UNASSIGNED), if it exists.
    pub fn show_fleet(&mut self, tab: &str) -> bool {
        match self.tabs().iter().position(|t| t == tab) {
            Some(i) => {
                self.show_tab(i as i64);
                true
            }
            None => false,
        }
    }

    fn step_tab(&mut self, by: i64) {
        let tabs = self.tabs();
        let at = self.tab.as_ref().and_then(|t| tabs.iter().position(|x| x == t)).unwrap_or(0) as i64;
        self.show_tab(at + by);
    }

    fn set_expanded(&mut self, id: &str, open: bool) {
        self.expanded.insert(id.to_string(), open);
        self.rebuild();
    }

    fn depth(r: &Row) -> usize {
        match r.section {
            Section::FleetBand | Section::StowBand => 0,
            Section::SquadronBand => 1,
            _ => 2 + r.prefix.chars().count() / 3,
        }
    }

    fn fold(&mut self) {
        let Some(r) = self.current().cloned() else { return };
        if r.expanded {
            self.set_expanded(&r.node.id, false);
        } else {
            let d = Self::depth(&r);
            if d > 0 {
                if let Some(i) = (0..self.cursor).rev().find(|&i| Self::depth(&self.rows[i]) < d) {
                    self.cursor = i;
                }
            }
        }
    }

    fn unfold(&mut self) {
        let Some(r) = self.current().cloned() else { return };
        if r.node.children.is_empty() {
            return;
        }
        if !r.expanded {
            self.set_expanded(&r.node.id, true);
        } else if self.cursor + 1 < self.rows.len() {
            self.cursor += 1;
        }
    }

    /// `z`: open or close the `+N finished` row of the member the cursor is
    /// in (its own row, a crew row, or the fold); kept in the saved state.
    fn toggle_crew_fold(&mut self) {
        let Some(id) = self.current().map(|r| r.node.id.clone()) else { return };
        // member ids never hold a `:` (config: [A-Z0-9_] parts joined by `/`)
        let member = if let Some(m) = id.strip_prefix("M:").or_else(|| id.strip_prefix("CF:")) {
            m.to_string()
        } else if let Some((m, _)) = id.strip_prefix("C:").and_then(|r| r.split_once(':')) {
            m.to_string()
        } else {
            self.notify("z: put the cursor on a member or its crew", st(mono::WARN));
            return;
        };
        let now = self.now();
        let q = self.filter.trim().to_lowercase();
        let count = |q: &str| self.bundle.as_ref().and_then(|b| b.snap.fleets.iter().flat_map(crate::view::fleet_members).find(|m| m.id.0 == member).map(|m| crate::view::folded_crew(&m.crew, now, q))).unwrap_or(0);
        if count(&q) == 0 && count("") > 0 {
            self.notify(format!("the filter shows every old crew of {member}; esc clears it"), st(mono::COMMENT));
            return;
        }
        if count("") == 0 {
            self.notify(format!("{member} has no crew that finished over {} min ago", (crate::view::CREW_RECENT_S / 60.0) as u64), st(mono::COMMENT));
            return;
        }
        let fold = crate::view::crew_fold_id(&member);
        let mrow = format!("M:{member}");
        let member_open = self.rows.iter().find(|r| r.node.id == mrow).map(|r| r.expanded).unwrap_or(false);
        // a collapsed member opens with its fold, so the fold is in view
        let open = !member_open || !self.expanded.get(&fold).copied().unwrap_or(false);
        if open {
            self.expanded.insert(mrow, true);
        }
        self.expanded.insert(fold.clone(), open);
        self.rebuild();
        // closed with the cursor inside it: the cursor goes to the fold row
        if !open && !self.rows.iter().any(|r| r.node.id == id) {
            self.goto_id(&fold);
        }
    }

    fn toggle(&mut self) {
        if let Some(r) = self.current().cloned() {
            if !r.node.children.is_empty() {
                self.set_expanded(&r.node.id, !r.expanded);
            }
        }
    }

    fn step_where(&mut self, dir: i64, pred: impl Fn(&Row) -> bool) -> bool {
        let hits: Vec<usize> = self.rows.iter().enumerate().filter(|(_, r)| pred(r)).map(|(i, _)| i).collect();
        if hits.is_empty() {
            return false;
        }
        let c = self.cursor;
        self.cursor = if dir > 0 {
            *hits.iter().find(|&&i| i > c).unwrap_or(&hits[0])
        } else {
            *hits.iter().rev().find(|&&i| i < c).unwrap_or(hits.last().unwrap())
        };
        true
    }

    // ---------------------------------------------------------------- lists

    pub fn issues(&self) -> Vec<Issue> {
        self.bundle.as_ref().map(|b| all_issues(&b.snap).into_iter().cloned().collect()).unwrap_or_default()
    }

    pub fn needs(&self) -> Vec<NeedItem> {
        let Some(b) = &self.bundle else { return vec![] };
        let now = self.now();
        let mut out = vec![];
        for f in &b.snap.fleets {
            for m in crate::view::fleet_members(f) {
                if m.status == Status::NeedsYou {
                    let s = m.session.as_ref();
                    let since = s.and_then(|s| s.status_since).or(s.and_then(|s| s.started_at));
                    out.push(NeedItem {
                        row_id: format!("M:{}", m.id.0),
                        name: m.expected_name.clone(),
                        age: since.map(|t| crate::theme::age(now - t)).unwrap_or_default(),
                        text: s.and_then(|s| s.waiting_for.clone()).or_else(|| m.reasons.first().cloned()).unwrap_or_default(),
                    });
                }
            }
        }
        out
    }

    fn member(&self, id: &Id) -> Option<&Member> {
        self.bundle.as_ref().and_then(|b| actions::find_member(&b.snap, id))
    }

    /// Where `m` would go for the selected row.
    pub fn message_plan(&self) -> Result<MessagePlan, String> {
        let b = self.bundle.as_ref().ok_or("loading")?;
        actions::plan_message(b.mux_caps, &b.snap, &b.panes, &self.target())
    }

    pub fn available(&self, act: Act) -> Result<(), String> {
        let b = self.bundle.as_ref().ok_or("loading")?;
        actions::available(b.mux_caps, &b.snap, &self.target(), act)
    }

    /// The `legacy` issue's fix on the selected row, as (session, identity):
    /// `nimitz adopt <sid> <id>`.
    pub fn legacy_adopt(&self) -> Option<(String, String)> {
        let b = self.bundle.as_ref()?;
        let issues: Vec<Issue> = match self.target() {
            Target::Member(id) => actions::find_member(&b.snap, &id).map(|m| m.issues.clone()).unwrap_or_default(),
            Target::Stowaway(sid) => b.snap.stowaways.iter().find(|s| s.session_id == sid).map(|s| crate::view::stowaway_issues(&b.snap, s).into_iter().cloned().collect()).unwrap_or_default(),
            _ => vec![],
        };
        issues.iter().filter(|i| i.code == "legacy").find_map(|i| {
            let w: Vec<&str> = i.fix.as_deref()?.split_whitespace().collect();
            match w.as_slice() {
                ["nimitz", "adopt", sid, id] => Some((sid.to_string(), id.to_string())),
                _ => None,
            }
        })
    }

    /// The action menu (`x`) for the selected row.
    pub fn action_menu(&self) -> Vec<MenuItem> {
        let mut v: Vec<MenuItem> = Act::MENU
            .iter()
            .filter(|a| !matches!(a, Act::Start | Act::Open | Act::Up))
            .map(|a| MenuItem { key: a.key(), label: a.label().into(), enabled: self.available(*a), action: MenuAction::Act(*a) })
            .collect();
        if let Some((sid, id)) = self.legacy_adopt() {
            if let Some(it) = v.iter_mut().find(|i| i.key == Act::Adopt.key()) {
                it.label = format!("adopt this legacy session as {id}");
                it.enabled = Ok(());
                it.action = MenuAction::AdoptAs(sid, id);
            }
        }
        if let Target::Fleet(id) = self.target() {
            for a in [Act::Restore, Act::Layout, Act::LayoutShips] {
                v.push(MenuItem { key: a.key(), label: a.label().into(), enabled: self.available(a), action: MenuAction::Act(a) });
            }
            v.push(MenuItem { key: 'M', label: "remove a machine from this fleet".into(), enabled: Ok(()), action: MenuAction::MachineRm(id.0) });
        }
        v
    }

    /// The create menu (`c`) for where the cursor is.
    pub fn create_menu(&self) -> Vec<MenuItem> {
        let mut v = vec![MenuItem { key: 'f', label: "new fleet".into(), enabled: Ok(()), action: MenuAction::NewFleet }];
        match self.current_fleet() {
            Some(f) => v.push(MenuItem { key: 's', label: format!("new squadron in {f}"), enabled: Ok(()), action: MenuAction::NewSquadron(f) }),
            None => v.push(MenuItem { key: 's', label: "new squadron".into(), enabled: Err("select a fleet first".into()), action: MenuAction::NewFleet }),
        }
        match self.current_squadron() {
            Some((f, sq)) => v.push(MenuItem { key: 'p', label: format!("new ship in {f}/{sq}"), enabled: Ok(()), action: MenuAction::NewShip(f, sq) }),
            None => v.push(MenuItem { key: 'p', label: "new ship".into(), enabled: Err("select a squadron first".into()), action: MenuAction::NewFleet }),
        }
        let (label, edit) = match self.edit_action() {
            Ok((l, a)) => (l, Ok(a)),
            Err(e) => ("edit".to_string(), Err(e)),
        };
        v.push(MenuItem {
            key: 'e',
            label,
            enabled: edit.as_ref().map(|_| ()).map_err(|e| e.clone()),
            action: edit.unwrap_or(MenuAction::NewFleet),
        });
        v
    }

    /// What `e` edits on the selected row.
    pub fn edit_action(&self) -> Result<(String, MenuAction), String> {
        match self.target() {
            Target::Member(id) if id.role() == Role::Ship => Ok((format!("edit ship {id}"), MenuAction::EditShip(id))),
            Target::Squadron(id) => Ok((format!("edit squadron {id}"), MenuAction::EditSquadron(id))),
            Target::Member(id) if id.role() == Role::Commodore => Ok((format!("edit squadron {id}"), MenuAction::EditSquadron(id))),
            Target::Fleet(id) => Ok((format!("edit fleet {id}"), MenuAction::EditFleet(id.0))),
            Target::Member(id) if id.role() == Role::Admiral => Ok((format!("edit fleet {id}"), MenuAction::EditFleet(id.0))),
            _ => Err("select a fleet, squadron or ship to edit it".into()),
        }
    }

    fn machine_of(&self, fleet: &str) -> String {
        self.bundle
            .as_ref()
            .and_then(|b| b.snap.fleets.iter().find(|f| f.id.fleet() == fleet))
            .and_then(|f| f.machine.clone())
            .or_else(|| self.bundle.as_ref().and_then(|b| b.snap.machine.clone()))
            .unwrap_or_else(|| "this-machine".into())
    }

    fn open_form(&mut self, mut form: Form) {
        if let Some(b) = &self.bundle {
            form.validate(&self.ctx.env, &b.snap);
        }
        self.modal = Some(Modal::Form(Box::new(form)));
    }

    fn open_or_say(&mut self, f: Result<Form, String>) {
        match f {
            Ok(f) => self.open_form(f),
            Err(e) => self.notify(format!("edit: {e}"), st(mono::WARN)),
        }
    }

    fn run_menu(&mut self, a: MenuAction) -> Option<Effect> {
        match a {
            MenuAction::Act(act) => self.start_act(act),
            MenuAction::NewFleet => {
                let f = Form::new_fleet(&self.ctx.env);
                self.open_form(f);
                None
            }
            MenuAction::NewSquadron(f) => {
                self.open_form(Form::new_squadron(&f));
                None
            }
            MenuAction::NewShip(f, sq) => {
                let m = self.machine_of(&f);
                self.open_form(Form::new_ship(&f, &sq, &m));
                None
            }
            MenuAction::EditShip(id) => {
                let m = self.machine_of(id.fleet());
                let f = Form::edit_ship(&self.ctx.env, &id, &m);
                self.open_or_say(f);
                None
            }
            MenuAction::EditFleet(f) => {
                let f = Form::edit_fleet(&self.ctx.env, &f);
                self.open_or_say(f);
                None
            }
            MenuAction::EditSquadron(id) => {
                let f = Form::edit_squadron(&self.ctx.env, &id);
                self.open_or_say(f);
                None
            }
            MenuAction::MachineRm(f) => {
                let m = self.machine_of(&f);
                self.open_form(Form::machine_rm(&f, &m));
                None
            }
            MenuAction::AdoptAs(sid, id) => {
                let short: String = sid.chars().take(8).collect();
                self.open_form(Form::adopt(&sid, &short, &id));
                None
            }
        }
    }

    /// Plan `act` for the selected row and show the confirm dialog.
    pub fn start_act(&mut self, act: Act) -> Option<Effect> {
        if act == Act::Adopt {
            if let Some((sid, id)) = self.legacy_adopt() {
                return self.run_menu(MenuAction::AdoptAs(sid, id));
            }
            if let Target::Stowaway(sid) = self.target() {
                let short: String = sid.chars().take(8).collect();
                let suggestion = self.bundle.as_ref().and_then(|b| adopt_suggestion(&b.snap, &sid)).unwrap_or_default();
                self.open_form(Form::adopt(&sid, &short, &suggestion));
                return None;
            }
        }
        let Some(b) = &self.bundle else { return None };
        if let (Act::Restore, Target::Fleet(f)) = (act, self.target()) {
            match actions::prepare_restore(&self.ctx, b.mux_caps, &b.snap, &f) {
                Ok(p) => self.modal = Some(Modal::Confirm(p)),
                Err(why) => self.notify(format!("restore: not available. {why}"), st(mono::WARN)),
            }
            return None;
        }
        if let (Act::Layout | Act::LayoutShips, Target::Fleet(f)) = (act, self.target()) {
            match actions::prepare_layout(&self.ctx, b.mux_caps, &b.snap, &b.panes, &f, act == Act::LayoutShips) {
                Ok(p) => self.modal = Some(Modal::Confirm(p)),
                Err(why) => self.notify(format!("layout: not available. {why}"), st(mono::WARN)),
            }
            return None;
        }
        // `A` on a session that is shown already: jump there, open nothing new;
        // its tab with a plain shell: attach there; a tab that runs something
        // else: never typed into (R2)
        if act == Act::Open && self.available(Act::Open).is_ok() {
            match actions::existing_open(&self.ctx, &b.snap, &self.target()) {
                Ok(nimitz_mux::TabState::Shown(pane)) => {
                    match self.ctx.mux.focus(&pane) {
                        Ok(()) => {
                            actions::log(&self.ctx.env, "focus", &pane.pane_id);
                            self.notify(format!("Open already: focused pane {}", pane.pane_id), st(mono::BUSY));
                            if self.ctx.exit_after_jump {
                                return Some(Effect::Quit);
                            }
                        }
                        Err(e) => self.notify(format!("Could not focus {}: {e}", pane.pane_id), st(mono::ERROR)),
                    }
                    return None;
                }
                Ok(nimitz_mux::TabState::Shell(pane)) => {
                    match actions::claim_open(&self.ctx, &b.snap, &self.target(), &pane) {
                        Ok(()) => {
                            actions::log(&self.ctx.env, "attach", &pane.pane_id);
                            self.notify(format!("Attached in its tab's pane {}", pane.pane_id), st(mono::BUSY));
                        }
                        Err(e) => self.notify(format!("Could not attach in pane {}: {e}", pane.pane_id), st(mono::ERROR)),
                    }
                    return None;
                }
                Ok(nimitz_mux::TabState::Taken { pane, running }) => {
                    self.notify(format!("tab.taken: its tab's pane {} runs `{running}`; ctrl-z or quit it there, then A again", pane.pane_id), st(mono::ERROR));
                    return None;
                }
                Ok(nimitz_mux::TabState::Missing) => {}
                Err(i) => {
                    let fix = i.fix.as_deref().map(|f| format!("  fix: {f}")).unwrap_or_default();
                    self.notify(format!("{}: {}{fix}", i.code, i.message), st(mono::ERROR));
                    return None;
                }
            }
        }
        match actions::prepare(&self.ctx, b.mux_caps, &b.snap, &self.target(), act, None) {
            Ok(p) => self.modal = Some(Modal::Confirm(p)),
            Err(why) => self.notify(format!("{}: not available. {why}", act.word()), st(mono::WARN)),
        }
        None
    }

    /// `enter` on a row with a pane: focus it.
    fn jump(&mut self) -> Option<Effect> {
        let r = self.current()?.clone();
        let b = self.bundle.as_ref()?;
        match actions::jump_target(b.mux_caps, r.node.pane_id.as_deref(), &b.panes) {
            Ok(pane) if !self.ctx.live => self.notify(format!("demo data: would focus pane {}", pane.pane_id), st(mono::WARN)),
            Ok(pane) => match self.ctx.mux.focus(&pane) {
                Ok(()) => {
                    actions::log(&self.ctx.env, "focus", &pane.pane_id);
                    if self.ctx.exit_after_jump {
                        return Some(Effect::Quit);
                    }
                    self.notify(format!("Focused {}", pane.pane_id), st(mono::BUSY));
                }
                Err(e) => self.notify(format!("Could not focus {}: {e}", pane.pane_id), st(mono::ERROR)),
            },
            Err(why) => self.notify(format!("No pane to jump to for {}: {why}", full_name(&r)), st(mono::WARN)),
        }
        None
    }

    fn yank(&mut self) {
        let sid = self.current().and_then(|r| r.node.session_id.clone());
        match sid {
            Some(s) => {
                copy_to_clipboard(&s);
                self.notify(format!("Yanked {s}"), st(mono::BUSY));
            }
            None => self.notify("No session on this row.", st(mono::WARN)),
        }
    }

    // ---------------------------------------------------------------- keys

    /// Handle one key; returns work the run loop must do. Sticky state is
    /// saved after every key that changed it.
    pub fn on_key(&mut self, k: KeyEvent) -> Option<Effect> {
        let effect = self.handle_key(k);
        self.persist();
        let now = self.current().map(|r| r.node.id.clone());
        if now != self.detail_for {
            self.detail_for = now;
            self.detail_scroll = 0;
        }
        effect
    }

    fn handle_key(&mut self, k: KeyEvent) -> Option<Effect> {
        if k.kind != KeyEventKind::Press && k.kind != KeyEventKind::Repeat {
            return None;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return Some(Effect::Quit);
        }
        if let Some(m) = self.modal.take() {
            return self.modal_key(m, k);
        }
        if self.editing_filter {
            match k.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.editing_filter = false;
                }
                KeyCode::Enter => self.editing_filter = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => return None,
            }
            self.rebuild();
            return None;
        }
        if k.code == KeyCode::Char('g') && !ctrl {
            if self.last_g.map(|t| t.elapsed() < Duration::from_millis(800)).unwrap_or(false) {
                self.cursor = 0;
                self.last_g = None;
            } else {
                self.last_g = Some(Instant::now());
            }
            return None;
        }
        self.last_g = None;
        let last = self.rows.len().saturating_sub(1);
        match (k.code, ctrl) {
            (KeyCode::Char('q'), _) => return Some(Effect::Quit),
            (KeyCode::Tab, _) => self.step_tab(1),
            (KeyCode::BackTab, _) => self.step_tab(-1),
            (KeyCode::Char(c @ '1'..='9'), false) if ((c as usize) - ('1' as usize)) < self.tabs().len() => {
                self.show_tab(c as i64 - '1' as i64)
            }
            (KeyCode::Char('j') | KeyCode::Down, _) => self.cursor = (self.cursor + 1).min(last),
            (KeyCode::Char('k') | KeyCode::Up, _) => self.cursor = self.cursor.saturating_sub(1),
            (KeyCode::Char('h') | KeyCode::Left, _) => self.fold(),
            (KeyCode::Char('l') | KeyCode::Right, _) => self.unfold(),
            (KeyCode::Char('G') | KeyCode::End, _) => self.cursor = last,
            (KeyCode::Home, _) => self.cursor = 0,
            (KeyCode::Char('e'), true) => self.detail_scroll += 1,
            (KeyCode::Char('y'), true) => self.detail_scroll = self.detail_scroll.saturating_sub(1),
            (KeyCode::Char('d'), true) | (KeyCode::PageDown, _) => self.cursor = (self.cursor + self.page / 2).min(last),
            (KeyCode::Char('u'), true) | (KeyCode::PageUp, _) => self.cursor = self.cursor.saturating_sub(self.page / 2),
            (KeyCode::Char(' '), _) => self.toggle(),
            (KeyCode::Enter, _) => {
                let (pane, kids) = self.current().map(|r| (r.node.pane_id.is_some(), !r.node.children.is_empty())).unwrap_or((false, false));
                if pane || !kids {
                    return self.jump();
                }
                self.toggle();
            }
            (KeyCode::Char('z'), false) => self.toggle_crew_fold(),
            (KeyCode::Char('/'), _) => self.editing_filter = true,
            // esc closes what is open, one thing at a time; with nothing open it quits
            (KeyCode::Esc, _) => {
                if self.detail_open {
                    self.detail_open = false;
                } else if !self.filter.is_empty() {
                    self.filter.clear();
                    self.rebuild();
                } else if let Some(b) = &self.busy {
                    // a reflex esc never ends the program mid-action
                    // (the running line says so; the toast is for the log of what happened)
                    self.notify(format!("{b} is still running; esc quits once it is done (q quits now)"), st(mono::WARN));
                } else {
                    return Some(Effect::Quit);
                }
            }
            (KeyCode::Char('n'), _) => {
                let q = self.filter.to_lowercase();
                self.step_where(1, |r| matches!(r.node.kind, Kind::Member(_)) && (q.is_empty() || r.node.name.to_lowercase().contains(&q)));
            }
            (KeyCode::Char('N'), _) => {
                let q = self.filter.to_lowercase();
                self.step_where(-1, |r| matches!(r.node.kind, Kind::Member(_)) && (q.is_empty() || r.node.name.to_lowercase().contains(&q)));
            }
            (KeyCode::Char('J'), _) => {
                if !self.step_where(1, |r| r.node.pane_id.is_some()) {
                    self.notify("No row has a pane.", st(mono::WARN));
                }
            }
            (KeyCode::Char('K'), _) => {
                if !self.step_where(-1, |r| r.node.pane_id.is_some()) {
                    self.notify("No row has a pane.", st(mono::WARN));
                }
            }
            (KeyCode::Char('!'), _) => self.modal = Some(Modal::Needs { items: self.needs(), sel: 0 }),
            (KeyCode::Char('i'), _) => self.modal = Some(Modal::Issues { sel: 0 }),
            (KeyCode::Char('?'), _) => self.modal = Some(Modal::Help { scroll: 0 }),
            (KeyCode::Char('I'), _) => {
                self.show_stowaways = !self.show_stowaways;
                self.rebuild();
            }
            (KeyCode::Char('f'), _) => self.modal = Some(Modal::FleetFilter { sel: 0 }),
            (KeyCode::Char('F'), _) => {
                self.flagged_only = !self.flagged_only;
                self.rebuild();
            }
            (KeyCode::Char('R'), _) => return Some(Effect::Refresh),
            (KeyCode::Char('w'), _) => {
                self.detail_open = !self.detail_open;
                self.detail_scroll = 0;
            }
            (KeyCode::Char('y'), _) => self.yank(),
            (KeyCode::Char('a'), _) => return self.start_act(Act::Start),
            (KeyCode::Char('u'), _) => {
                if let Some(f) = self.current_fleet() {
                    let b = self.bundle.as_ref()?;
                    match actions::prepare(&self.ctx, b.mux_caps, &b.snap, &Target::Fleet(Id(f)), Act::Up, None) {
                        Ok(p) => self.modal = Some(Modal::Confirm(p)),
                        Err(why) => self.notify(format!("up: not available. {why}"), st(mono::WARN)),
                    }
                }
            }
            (KeyCode::Char('A'), _) => return self.start_act(Act::Open),
            (KeyCode::Char('D'), _) => return self.start_act(Act::ConfigRm),
            (KeyCode::Char('x'), _) => {
                let title = self.current().map(|r| full_name(&r)).unwrap_or_default();
                self.modal = Some(Modal::Menu { title: format!("actions: {title}"), items: self.action_menu(), sel: 0 });
            }
            (KeyCode::Char('c'), _) => self.modal = Some(Modal::Menu { title: "create".into(), items: self.create_menu(), sel: 0 }),
            (KeyCode::Char('e'), _) => match self.edit_action() {
                Ok((_, a)) => return self.run_menu(a),
                Err(why) => self.notify(format!("edit: {why}"), st(mono::WARN)),
            },
            (KeyCode::Char('m'), _) => match self.message_plan() {
                Ok(plan) => self.modal = Some(Modal::Message { plan, text: String::new() }),
                Err(why) => self.notify(format!("message: not available. {why}"), st(mono::WARN)),
            },
            _ => {}
        }
        None
    }

    fn modal_key(&mut self, m: Modal, k: KeyEvent) -> Option<Effect> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let down = matches!(k.code, KeyCode::Char('j') | KeyCode::Down);
        let up = matches!(k.code, KeyCode::Char('k') | KeyCode::Up);
        match m {
            Modal::Help { scroll } => match k.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {}
                _ if down => self.modal = Some(Modal::Help { scroll: scroll + 1 }),
                _ if up => self.modal = Some(Modal::Help { scroll: scroll.saturating_sub(1) }),
                _ => self.modal = Some(Modal::Help { scroll }),
            },
            Modal::FleetFilter { mut sel } => match k.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('f') | KeyCode::Char('q') => {}
                _ if down => {
                    sel = (sel + 1).min(FleetState::ALL.len() - 1);
                    self.modal = Some(Modal::FleetFilter { sel });
                }
                _ if up => self.modal = Some(Modal::FleetFilter { sel: sel.saturating_sub(1) }),
                KeyCode::Char(' ') | KeyCode::Char('x') => {
                    self.fleet_states[sel] = !self.fleet_states[sel];
                    self.rebuild();
                    self.modal = Some(Modal::FleetFilter { sel });
                }
                KeyCode::Char('a') => {
                    self.fleet_states = [true; 7];
                    self.rebuild();
                    self.modal = Some(Modal::FleetFilter { sel });
                }
                _ => self.modal = Some(Modal::FleetFilter { sel }),
            },
            Modal::Needs { items, mut sel } => match k.code {
                KeyCode::Esc | KeyCode::Char('!') | KeyCode::Char('q') => {}
                _ if down => {
                    sel = (sel + 1).min(items.len().saturating_sub(1));
                    self.modal = Some(Modal::Needs { items, sel });
                }
                _ if up => self.modal = Some(Modal::Needs { items, sel: sel.saturating_sub(1) }),
                KeyCode::Enter => {
                    if let Some(it) = items.get(sel) {
                        let id = Id(it.row_id.trim_start_matches("M:").to_string());
                        let row = it.row_id.clone();
                        self.reveal(&row, Some(&id));
                    }
                }
                _ => self.modal = Some(Modal::Needs { items, sel }),
            },
            Modal::Issues { mut sel } => {
                let issues = self.issues();
                match k.code {
                    KeyCode::Esc | KeyCode::Char('i') | KeyCode::Char('q') => {}
                    _ if down => {
                        sel = (sel + 1).min(issues.len().saturating_sub(1));
                        self.modal = Some(Modal::Issues { sel });
                    }
                    _ if up => self.modal = Some(Modal::Issues { sel: sel.saturating_sub(1) }),
                    KeyCode::Char('y') => {
                        match issues.get(sel).and_then(|i| i.fix.clone()) {
                            Some(fix) => {
                                copy_to_clipboard(&fix);
                                self.notify(format!("Yanked {fix}"), st(mono::BUSY));
                            }
                            None => self.notify("This issue has no fix command.", st(mono::WARN)),
                        }
                        self.modal = Some(Modal::Issues { sel });
                    }
                    KeyCode::Enter => {
                        if let Some(i) = issues.get(sel) {
                            match &i.id {
                                Some(id) => {
                                    let row = match id.role() {
                                        Role::Admiral if self.member(id).is_none() => format!("F:{}", id.0),
                                        _ => format!("M:{}", id.0),
                                    };
                                    self.reveal(&row, Some(id));
                                }
                                None if i.code == "stowaway" => {
                                    self.show_stowaways = true;
                                    self.rebuild();
                                    self.goto_id("STOW");
                                }
                                None => self.notify("This issue is about the machine, not one row.", st(mono::WARN)),
                            }
                        }
                    }
                    _ => self.modal = Some(Modal::Issues { sel }),
                }
            }
            Modal::Menu { title, items, mut sel } => {
                let pick = match k.code {
                    KeyCode::Esc | KeyCode::Char('q') => return None,
                    _ if down => {
                        sel = (sel + 1).min(items.len().saturating_sub(1));
                        None
                    }
                    _ if up => {
                        sel = sel.saturating_sub(1);
                        None
                    }
                    KeyCode::Enter => items.get(sel).cloned(),
                    KeyCode::Char(c) => items.iter().find(|i| i.key == c).cloned(),
                    _ => None,
                };
                match pick {
                    Some(it) => match it.enabled {
                        Ok(()) => return self.run_menu(it.action),
                        Err(why) => {
                            self.notify(format!("{}: not available. {why}", it.label), st(mono::WARN));
                            self.modal = Some(Modal::Menu { title, items, sel });
                        }
                    },
                    None => self.modal = Some(Modal::Menu { title, items, sel }),
                }
            }
            Modal::Confirm(p) => match k.code {
                KeyCode::Char('y') | KeyCode::Enter if p.plan.ok() => {
                    if !self.ctx.live {
                        self.notify(format!("{}: demo data, nothing ran. The plan above is what core would do.", p.act.word()), st(mono::WARN));
                        return None;
                    }
                    return Some(Effect::Run(p));
                }
                KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => {}
                _ => self.modal = Some(Modal::Confirm(p)),
            },
            Modal::Message { plan, mut text } => match k.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    if nimitz_mux::sanitize_text(&text).is_empty() {
                        self.notify("Nothing to send: the message is empty once control characters are removed.", st(mono::WARN));
                        self.modal = Some(Modal::Message { plan, text });
                    } else {
                        self.send_message(&plan, &text);
                    }
                }
                KeyCode::Backspace => {
                    text.pop();
                    self.modal = Some(Modal::Message { plan, text });
                }
                KeyCode::Char('u') if ctrl => self.modal = Some(Modal::Message { plan, text: String::new() }),
                KeyCode::Char(c) if !ctrl => {
                    text.push(c);
                    self.modal = Some(Modal::Message { plan, text });
                }
                _ => self.modal = Some(Modal::Message { plan, text }),
            },
            Modal::Form(mut form) => {
                let r = form.on_key(k);
                if let Some(b) = &self.bundle {
                    form.validate(&self.ctx.env, &b.snap);
                }
                match r {
                    FormKey::Cancel => {}
                    FormKey::Stay => self.modal = Some(Modal::Form(form)),
                    FormKey::Editor => {
                        let editable = matches!(form.focused().kind, crate::forms::FieldKind::Long | crate::forms::FieldKind::Text);
                        self.modal = Some(Modal::Form(form));
                        if editable {
                            return Some(Effect::Editor);
                        }
                    }
                    FormKey::Submit => {
                        if form.ok() {
                            self.modal = Some(Modal::Confirm(form.pending()));
                        } else {
                            self.notify("The form has errors; fix the fields marked in red.", st(mono::ERROR));
                            self.modal = Some(Modal::Form(form));
                        }
                    }
                }
            }
        }
        None
    }

    fn send_message(&mut self, plan: &MessagePlan, text: &str) {
        if !self.ctx.live {
            self.notify(format!("demo data: would type into {}'s pane {}", plan.via, plan.pane.pane_id), st(mono::WARN));
            return;
        }
        let body = actions::message_text(plan, text);
        if let Some(job) = &plan.attach_job {
            if !nimitz_mux::runs_exact_attach(self.ctx.mux.as_ref(), &plan.pane, job) {
                self.notify(format!("Not sent to {}: its pane {} no longer runs `claude attach {job}` (the agent view, or something else); nothing was typed", plan.via, plan.pane.pane_id), st(mono::ERROR));
                return;
            }
        }
        match self.ctx.mux.type_text(&plan.pane, &body) {
            Ok(()) if plan.attach_job.is_some() && nimitz_mux::typed_into_view(self.ctx.mux.as_ref(), &plan.pane) => {
                self.notify(format!("Typed into {}'s pane {}, but it runs the agent view now (an /exit raced the typing): check it for a dispatched session", plan.via, plan.pane.pane_id), st(mono::ERROR));
            }
            Ok(()) => {
                actions::log(&self.ctx.env, "message", &format!("{} via {}: {}", plan.to, plan.via, nimitz_mux::sanitize_text(text)));
                let how = if plan.relay { format!(" (relayed by {})", plan.via) } else { String::new() };
                self.notify(format!("Sent to {}{how}", plan.to), st(mono::BUSY));
            }
            Err(e) => self.notify(format!("Not sent to {}: {e}", plan.via), st(mono::ERROR)),
        }
    }

    /// The form field `$EDITOR` edits, its current text.
    pub fn editor_text(&self) -> Option<String> {
        match &self.modal {
            Some(Modal::Form(f)) => Some(f.focused().value.clone()),
            _ => None,
        }
    }

    pub fn editor_done(&mut self, text: String) {
        let snap = self.bundle.as_ref().map(|b| b.snap.clone());
        if let Some(Modal::Form(f)) = &mut self.modal {
            let i = f.focus;
            f.fields[i].value = text.trim_end_matches('\n').to_string();
            if let Some(s) = &snap {
                f.validate(&self.ctx.env, s);
            }
        }
    }

    /// Run loop, step 1 (main thread): open panes. `Ok` means apply may follow.
    pub fn begin(&mut self, p: &Pending) -> Result<(), String> {
        actions::log(&self.ctx.env, p.act.word(), &p.command);
        match actions::open_panes(&self.ctx, p) {
            Err(e) => {
                self.notify(format!("{}: {e}", p.act.word()), st(mono::ERROR));
                return Err(e);
            }
            Ok(opened) if p.layout.is_some() => {
                actions::log(&self.ctx.env, "layout result", &format!("{} pane(s)", opened.len()));
            }
            Ok(_) => {}
        }
        self.busy = Some(format!("{} ...", p.command));
        Ok(())
    }

    /// Run loop, step 2: the worker's outcome.
    pub fn finish(&mut self, p: &Pending, o: &Outcome) {
        self.busy = None;
        let (ok, msg) = actions::outcome_text(p, o);
        actions::log(&self.ctx.env, &format!("{} result", p.act.word()), &msg);
        self.notify(msg, if ok { st(mono::BUSY) } else { bold(mono::ERROR) });
    }

    /// Counts for the top bar: (fleets, live, need you, errors, warnings).
    pub fn counts(&self) -> (usize, usize, usize, usize, usize) {
        let Some(b) = &self.bundle else { return (0, 0, 0, 0, 0) };
        // stowaways are live sessions too
        let mut live = b.snap.stowaways.iter().filter(|s| s.pid.is_some()).count();
        let mut need = 0;
        for f in &b.snap.fleets {
            for m in crate::view::fleet_members(f) {
                live += crate::view::member_live(m) as usize;
                need += (m.status == Status::NeedsYou) as usize;
            }
        }
        let issues = all_issues(&b.snap);
        let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
        let warns = issues.iter().filter(|i| i.severity == Severity::Warn).count();
        (b.snap.fleets.len(), live, need, errors, warns)
    }
}

/// What the adopt form offers for a stowaway (M25): the identity its name
/// spells when the name is a fleet address; else the ship whose folder holds
/// it; else a `F/SQ/<ship>` placeholder for the squadron or fleet whose
/// folders hold it.
fn adopt_suggestion(snap: &nimitz_core::Snapshot, sid: &str) -> Option<String> {
    let s = snap.stowaways.iter().find(|s| s.session_id == sid)?;
    if let Some(id) = nimitz_core::sessions::id_from_name(&s.name) {
        return Some(id.0);
    }
    let same = |c: &str| c.trim_end_matches('/') == s.cwd.trim_end_matches('/');
    for f in &snap.fleets {
        for sq in &f.squadrons {
            if let Some(ship) = sq.ships.iter().find(|m| m.cwd.as_deref().is_some_and(same)) {
                return Some(ship.id.0.clone());
            }
            if sq.commodore.cwd.as_deref().is_some_and(same) {
                return Some(format!("{}/<ship>", sq.id.0));
            }
        }
    }
    nimitz_core::snapshot::stowaway_fleet(snap, s).map(|f| format!("{f}/<squadron>/<ship>"))
}

// ------------------------------------------------------------------ clipboard

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// OSC 52 (reaches the outer terminal through Herdr and tmux), plus pbcopy when there.
/// Off under `cargo test` so tests never touch the clipboard.
fn copy_to_clipboard(s: &str) {
    if cfg!(test) || std::env::var_os("ADMIRALTY_NO_CLIPBOARD").is_some() {
        return;
    }
    use std::io::Write;
    let _ = write!(std::io::stdout(), "\x1b]52;c;{}\x07", base64(s.as_bytes()));
    if let Some(pb) = nimitz_core::env::which("pbcopy") {
        if let Ok(mut c) = std::process::Command::new(pb).stdin(std::process::Stdio::piped()).spawn() {
            if let Some(mut stdin) = c.stdin.take() {
                let _ = stdin.write_all(s.as_bytes());
            }
            let _ = c.wait();
        }
    }
}
