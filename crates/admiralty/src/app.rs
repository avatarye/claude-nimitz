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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    /// side by side when the window is wide enough, else stacked
    Auto,
    /// list left, detail right (vim `v`)
    Vertical,
    /// list above, detail below (vim `s`)
    Horizontal,
}

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
    Tender(Id),
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
    Help,
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
    pub split: String,
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
    pub modal: Option<Modal>,
    pub toast: Option<Toast>,
    pub icons: &'static Icons,
    pub split: Split,
    /// fixtures: a frozen clock so ages never move
    pub fixed_now: Option<f64>,
    pub state_path: Option<PathBuf>,
    /// an action running on the worker thread
    pub busy: Option<String>,
    pub page: usize,
    last_g: Option<Instant>,
    last_saved: Option<Saved>,
}

pub const HELP: &[(&str, &str)] = &[
    ("j / k", "down / up"),
    ("h / l", "fold or go to parent / unfold or go to first child"),
    ("gg / G", "top / bottom"),
    ("ctrl-d / ctrl-u", "half page down / up"),
    ("space", "fold / unfold"),
    ("/  n / N", "filter by name; next / previous match"),
    ("enter", "jump to the row's pane (else fold / unfold)"),
    ("J / K", "next / previous row with a pane"),
    ("!", "everything waiting on you"),
    ("i", "every issue, with its fix command"),
    ("a", "start the member, top-down (confirms)"),
    ("u", "up: start everything the fleet runs here (confirms)"),
    ("A", "open: attach to a background session in a new pane"),
    ("x", "actions: stop, relaunch, sweep, rm, adopt, remove; on a fleet: layout, machines"),
    ("m", "message (ships: relayed by their commodore)"),
    ("c / e / D", "create fleet, squadron, ship, tender / edit it / remove from config"),
    ("y", "yank the session id (in i: the fix command)"),
    ("I", "show / hide stowaways"),
    ("f / F", "filter fleets by state / flagged rows only"),
    ("v / s", "split: detail beside / below"),
    ("R", "refresh now"),
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
            modal: None,
            toast: None,
            icons: if ascii { &ASCII } else { &NERD },
            split: Split::Auto,
            fixed_now: None,
            state_path,
            busy: None,
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
            split: match self.split {
                Split::Auto => "auto",
                Split::Vertical => "vertical",
                Split::Horizontal => "horizontal",
            }
            .into(),
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
        self.split = match s.split.as_str() {
            "vertical" => Split::Vertical,
            "horizontal" => Split::Horizontal,
            _ => Split::Auto,
        };
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
                sq.tenders.iter_mut().for_each(mark);
            }
        }
        self.bundle = Some(b);
        self.rebuild();
    }

    pub fn rebuild(&mut self) {
        let keep = self.rows.get(self.cursor).map(|r| r.node.id.clone());
        let now = self.now();
        let Some(b) = &self.bundle else { return };
        let view = View {
            filter: &self.filter,
            flagged_only: self.flagged_only,
            fleet_states: self.fleet_states,
            show_stowaways: self.show_stowaways,
            panes: &b.panes,
            icons: self.icons,
            now,
            slots: crate::view::count_slots(&b.snap),
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

    pub fn goto_id(&mut self, id: &str) -> bool {
        if let Some(i) = self.rows.iter().position(|r| r.node.id == id) {
            self.cursor = i;
            true
        } else {
            false
        }
    }

    /// Show a row by id even if a filter or a fold hides it.
    pub fn reveal(&mut self, row_id: &str, member: Option<&Id>) {
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
            for a in [Act::Layout, Act::LayoutShips] {
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
        let tender = match self.target() {
            Target::Squadron(id) => Ok(id),
            Target::Member(id) if id.role() != Role::Admiral && id.squadron().is_some() => Ok(Id::commodore(id.fleet(), id.squadron().unwrap_or_default())),
            _ => Err("select a squadron or its commodore".to_string()),
        };
        v.push(MenuItem {
            key: 't',
            label: match &tender {
                Ok(id) => format!("new tender for {}", id.name()),
                Err(_) => "new tender".into(),
            },
            enabled: tender.as_ref().map(|_| ()).map_err(|e| e.clone()),
            action: tender.map(MenuAction::Tender).unwrap_or(MenuAction::NewFleet),
        });
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
            Err(e) => self.notify(format!("edit: {e}"), st(mono::YELLOW)),
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
            MenuAction::Tender(id) => {
                self.open_form(Form::tender(&id));
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
                let suggestion = self
                    .issues()
                    .iter()
                    .filter(|i| i.code == "stowaway")
                    .find(|i| i.fix.as_deref().map(|f| f.contains(&short)).unwrap_or(false))
                    .and_then(|i| i.fix.as_deref().and_then(|f| f.split_whitespace().nth(3).map(String::from)))
                    .unwrap_or_default();
                self.open_form(Form::adopt(&sid, &short, &suggestion));
                return None;
            }
        }
        let Some(b) = &self.bundle else { return None };
        match actions::prepare(&self.ctx, b.mux_caps, &b.snap, &self.target(), act, None) {
            Ok(p) => self.modal = Some(Modal::Confirm(p)),
            Err(why) => self.notify(format!("{}: not available. {why}", act.word()), st(mono::YELLOW)),
        }
        None
    }

    /// `enter` on a row with a pane: focus it.
    fn jump(&mut self) -> Option<Effect> {
        let r = self.current()?.clone();
        let b = self.bundle.as_ref()?;
        match actions::jump_target(b.mux_caps, r.node.pane_id.as_deref(), &b.panes) {
            Ok(pane) if !self.ctx.live => self.notify(format!("demo data: would focus pane {}", pane.pane_id), st(mono::YELLOW)),
            Ok(pane) => match self.ctx.mux.focus(&pane) {
                Ok(()) => {
                    actions::log(&self.ctx.env, "focus", &pane.pane_id);
                    if self.ctx.exit_after_jump {
                        return Some(Effect::Quit);
                    }
                    self.notify(format!("Focused {}", pane.pane_id), st(mono::GREEN));
                }
                Err(e) => self.notify(format!("Could not focus {}: {e}", pane.pane_id), st(mono::PINK)),
            },
            Err(why) => self.notify(format!("No pane to jump to for {}: {why}", r.node.name), st(mono::YELLOW)),
        }
        None
    }

    fn yank(&mut self) {
        let sid = self.current().and_then(|r| r.node.session_id.clone());
        match sid {
            Some(s) => {
                copy_to_clipboard(&s);
                self.notify(format!("Yanked {s}"), st(mono::GREEN));
            }
            None => self.notify("No session on this row.", st(mono::YELLOW)),
        }
    }

    // ---------------------------------------------------------------- keys

    /// Handle one key; returns work the run loop must do. Sticky state is
    /// saved after every key that changed it.
    pub fn on_key(&mut self, k: KeyEvent) -> Option<Effect> {
        let effect = self.handle_key(k);
        self.persist();
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
            (KeyCode::Char('j') | KeyCode::Down, _) => self.cursor = (self.cursor + 1).min(last),
            (KeyCode::Char('k') | KeyCode::Up, _) => self.cursor = self.cursor.saturating_sub(1),
            (KeyCode::Char('h') | KeyCode::Left, _) => self.fold(),
            (KeyCode::Char('l') | KeyCode::Right, _) => self.unfold(),
            (KeyCode::Char('G') | KeyCode::End, _) => self.cursor = last,
            (KeyCode::Home, _) => self.cursor = 0,
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
            (KeyCode::Char('/'), _) => self.editing_filter = true,
            (KeyCode::Esc, _) => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.rebuild();
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
                    self.notify("No row has a pane.", st(mono::YELLOW));
                }
            }
            (KeyCode::Char('K'), _) => {
                if !self.step_where(-1, |r| r.node.pane_id.is_some()) {
                    self.notify("No row has a pane.", st(mono::YELLOW));
                }
            }
            (KeyCode::Char('!'), _) => self.modal = Some(Modal::Needs { items: self.needs(), sel: 0 }),
            (KeyCode::Char('i'), _) => self.modal = Some(Modal::Issues { sel: 0 }),
            (KeyCode::Char('?'), _) => self.modal = Some(Modal::Help),
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
            (KeyCode::Char('v'), _) => self.split = Split::Vertical,
            (KeyCode::Char('s'), _) => self.split = Split::Horizontal,
            (KeyCode::Char('y'), _) => self.yank(),
            (KeyCode::Char('a'), _) => return self.start_act(Act::Start),
            (KeyCode::Char('u'), _) => {
                if let Some(f) = self.current_fleet() {
                    let b = self.bundle.as_ref()?;
                    match actions::prepare(&self.ctx, b.mux_caps, &b.snap, &Target::Fleet(Id(f)), Act::Up, None) {
                        Ok(p) => self.modal = Some(Modal::Confirm(p)),
                        Err(why) => self.notify(format!("up: not available. {why}"), st(mono::YELLOW)),
                    }
                }
            }
            (KeyCode::Char('A'), _) => return self.start_act(Act::Open),
            (KeyCode::Char('D'), _) => return self.start_act(Act::ConfigRm),
            (KeyCode::Char('x'), _) => {
                let title = self.current().map(|r| r.node.name.clone()).unwrap_or_default();
                self.modal = Some(Modal::Menu { title: format!("actions: {title}"), items: self.action_menu(), sel: 0 });
            }
            (KeyCode::Char('c'), _) => self.modal = Some(Modal::Menu { title: "create".into(), items: self.create_menu(), sel: 0 }),
            (KeyCode::Char('e'), _) => match self.edit_action() {
                Ok((_, a)) => return self.run_menu(a),
                Err(why) => self.notify(format!("edit: {why}"), st(mono::YELLOW)),
            },
            (KeyCode::Char('m'), _) => match self.message_plan() {
                Ok(plan) => self.modal = Some(Modal::Message { plan, text: String::new() }),
                Err(why) => self.notify(format!("message: not available. {why}"), st(mono::YELLOW)),
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
            Modal::Help => {
                if !matches!(k.code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?')) {
                    self.modal = Some(Modal::Help);
                }
            }
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
                                self.notify(format!("Yanked {fix}"), st(mono::GREEN));
                            }
                            None => self.notify("This issue has no fix command.", st(mono::YELLOW)),
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
                                None => self.notify("This issue is about the machine, not one row.", st(mono::YELLOW)),
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
                            self.notify(format!("{}: not available. {why}", it.label), st(mono::YELLOW));
                            self.modal = Some(Modal::Menu { title, items, sel });
                        }
                    },
                    None => self.modal = Some(Modal::Menu { title, items, sel }),
                }
            }
            Modal::Confirm(p) => match k.code {
                KeyCode::Char('y') | KeyCode::Enter if p.plan.ok() => {
                    if !self.ctx.live {
                        self.notify(format!("{}: demo data, nothing ran. The plan above is what core would do.", p.act.word()), st(mono::YELLOW));
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
                    if text.trim().is_empty() {
                        self.modal = Some(Modal::Message { plan, text });
                    } else {
                        self.send_message(&plan, text.trim());
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
                            self.notify("The form has errors; fix the fields marked in red.", st(mono::PINK));
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
            self.notify(format!("demo data: would type into {}'s pane {}", plan.via, plan.pane.pane_id), st(mono::YELLOW));
            return;
        }
        let body = actions::message_text(plan, text);
        match self.ctx.mux.type_text(&plan.pane, &body) {
            Ok(()) => {
                actions::log(&self.ctx.env, "message", &format!("{} via {}: {text}", plan.to, plan.via));
                let how = if plan.relay { format!(" (relayed by {})", plan.via) } else { String::new() };
                self.notify(format!("Sent to {}{how}", plan.to), st(mono::GREEN));
            }
            Err(e) => self.notify(format!("Not sent to {}: {e}", plan.via), st(mono::PINK)),
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
        if let Err(e) = actions::open_panes(&self.ctx, p) {
            self.notify(format!("{}: {e}", p.act.word()), st(mono::PINK));
            return Err(e);
        }
        self.busy = Some(format!("{} ...", p.command));
        Ok(())
    }

    /// Run loop, step 2: the worker's outcome.
    pub fn finish(&mut self, p: &Pending, o: &Outcome) {
        self.busy = None;
        let (ok, msg) = actions::outcome_text(p, o);
        actions::log(&self.ctx.env, &format!("{} result", p.act.word()), &msg);
        self.notify(msg, if ok { st(mono::GREEN) } else { bold(mono::PINK) });
    }

    /// Counts for the top bar: (fleets, live, need you, errors, warnings).
    pub fn counts(&self) -> (usize, usize, usize, usize, usize) {
        let Some(b) = &self.bundle else { return (0, 0, 0, 0, 0) };
        let mut live = 0;
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
