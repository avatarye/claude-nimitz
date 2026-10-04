//! From `nimitz_core::Snapshot` to what the screen shows: a tree of nodes
//! (fleet band -> admiral + squadron frames -> members -> crew/tasks), then a
//! flat list of rows with tree guides. Pure data: no terminal here, so tests
//! drive it directly.

use std::collections::HashMap;

use nimitz_core::{Comms, Crew, Fleet, Issue, Member, Role, SessionInfo, Severity, Snapshot, Squadron, Status, Task};
use nimitz_mux::Pane;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::data::pane_for;
use crate::theme::{self, age, bold, mono, st, status_style, status_word, Icons};

// ------------------------------------------------------------------ fleet state

/// A fleet's overall state: drives its band colour and the `f` filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FleetState {
    NeedsYou,
    /// an Error-severity issue anywhere in the fleet
    Broken,
    Busy,
    Idle,
    /// on this machine, admiral not live
    NoAdmiral,
    /// on this machine, nothing live
    Inactive,
    /// not deployed on this machine
    Elsewhere,
}

impl FleetState {
    pub const ALL: [FleetState; 7] = [
        FleetState::NeedsYou,
        FleetState::Broken,
        FleetState::Busy,
        FleetState::Idle,
        FleetState::NoAdmiral,
        FleetState::Inactive,
        FleetState::Elsewhere,
    ];

    pub fn of(f: &Fleet) -> FleetState {
        if f.machine.is_none() {
            return FleetState::Elsewhere;
        }
        Self::of_group(&fleet_members(f), &fleet_issues(f), Some(&f.admiral))
    }

    /// A squadron's state: the same scale, its commodore standing in for the admiral.
    pub fn of_squadron(f: &Fleet, sq: &Squadron) -> FleetState {
        if f.machine.is_none() {
            return FleetState::Elsewhere;
        }
        let mut issues: Vec<&Issue> = sq.issues.iter().collect();
        for m in squadron_members(sq) {
            issues.extend(m.issues.iter());
        }
        Self::of_group(&squadron_members(sq), &issues, Some(&sq.commodore))
    }

    fn of_group(members: &[&Member], issues: &[&Issue], lead: Option<&Member>) -> FleetState {
        let live = |m: &Member| member_live(m);
        if members.iter().any(|m| m.status == Status::NeedsYou) {
            FleetState::NeedsYou
        } else if issues.iter().any(|i| i.severity == Severity::Error) {
            FleetState::Broken
        } else if !members.iter().any(|m| live(m)) {
            FleetState::Inactive
        } else if !lead.map(live).unwrap_or(true) {
            FleetState::NoAdmiral
        } else if members.iter().any(|m| m.status == Status::Busy) {
            FleetState::Busy
        } else {
            FleetState::Idle
        }
    }

    /// The label inside a squadron, where the lead is the commodore.
    pub fn squadron_label(self) -> &'static str {
        match self {
            FleetState::NoAdmiral => "no commodore",
            other => other.label(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FleetState::NeedsYou => "needs you",
            FleetState::Broken => "broken",
            FleetState::Busy => "busy",
            FleetState::Idle => "idle",
            FleetState::NoAdmiral => "no admiral",
            FleetState::Inactive => "inactive",
            FleetState::Elsewhere => "elsewhere",
        }
    }

    pub fn tint(self) -> Color {
        match self {
            FleetState::NeedsYou => mono::T_PINK,
            FleetState::Broken => mono::T_PURPLE,
            FleetState::Busy => mono::T_GREEN,
            FleetState::Idle => mono::T_CYAN,
            FleetState::NoAdmiral => mono::T_ORANGE,
            FleetState::Inactive => mono::T_GREY,
            FleetState::Elsewhere => mono::T_DIM,
        }
    }

    /// bright colour for the fleet's frame and title
    pub fn color(self) -> Color {
        match self {
            FleetState::NeedsYou => mono::PINK,
            FleetState::Broken => mono::PURPLE,
            FleetState::Busy => mono::GREEN,
            FleetState::Idle => mono::CYAN,
            FleetState::NoAdmiral => mono::ORANGE,
            FleetState::Inactive => mono::COMMENT,
            FleetState::Elsewhere => mono::DIM,
        }
    }

    pub fn index(self) -> usize {
        FleetState::ALL.iter().position(|x| *x == self).unwrap()
    }
}

/// Every member of a fleet, chain-of-command order.
pub fn fleet_members(f: &Fleet) -> Vec<&Member> {
    let mut v = vec![&f.admiral];
    for sq in &f.squadrons {
        v.push(&sq.commodore);
        v.extend(sq.ships.iter());
        v.extend(sq.tenders.iter());
    }
    v
}

pub fn squadron_members(sq: &Squadron) -> Vec<&Member> {
    let mut v = vec![&sq.commodore];
    v.extend(sq.ships.iter());
    v.extend(sq.tenders.iter());
    v
}

/// Every issue in a fleet: the fleet's own, the squadrons', every member's.
pub fn fleet_issues(f: &Fleet) -> Vec<&Issue> {
    let mut v: Vec<&Issue> = f.issues.iter().collect();
    v.extend(f.admiral.issues.iter());
    for sq in &f.squadrons {
        v.extend(sq.issues.iter());
        for m in squadron_members(sq) {
            v.extend(m.issues.iter());
        }
    }
    v
}

/// Every issue in the snapshot, member issues included, worst first.
pub fn all_issues(snap: &Snapshot) -> Vec<&Issue> {
    let mut v: Vec<&Issue> = snap.issues.iter().collect();
    for f in &snap.fleets {
        v.extend(fleet_issues(f));
    }
    v.sort_by(|a, b| b.severity.cmp(&a.severity));
    v
}

pub fn member_live(m: &Member) -> bool {
    m.session.as_ref().map(|s| s.pid.is_some()).unwrap_or(false)
}

// ------------------------------------------------------------------ nodes

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    FleetBand,
    SquadronBand,
    StowBand,
    Member(Role),
    Crew,
    Task,
    /// an extra job row of the identity (copy, stale row)
    ExtraRow,
    Stowaway,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: String,
    pub kind: Kind,
    /// nimitz identity this row is about (member rows and bands)
    pub member: Option<nimitz_core::Id>,
    pub session_id: Option<String>,
    pub default_expand: bool,
    pub icon: &'static str,
    pub icon_style: Style,
    pub name: String,
    pub name_style: Style,
    pub status: (String, Style),
    pub age: String,
    pub kind_text: String,
    pub detail: Vec<Span<'static>>,
    /// worst severity among this row's own issues
    pub worst: Option<Severity>,
    pub issue_count: usize,
    /// a `legacy` issue (a session of the old fleet-skill fleets) and no error
    pub legacy: bool,
    /// the multiplexer pane hosting this row's session, and its state
    pub pane_id: Option<String>,
    pub pane_state: Option<String>,
    /// band only: bright frame colour
    pub frame: Option<Color>,
    /// band only: dark band tint
    pub band_bg: Option<Color>,
    /// band only: right side of the title (status counts)
    pub title_right: Vec<Span<'static>>,
    pub children: Vec<Node>,
}

impl Node {
    fn new(id: String, kind: Kind) -> Node {
        Node {
            id,
            kind,
            member: None,
            session_id: None,
            default_expand: false,
            icon: "",
            icon_style: Style::new(),
            name: String::new(),
            name_style: Style::new(),
            status: (String::new(), Style::new()),
            age: String::new(),
            kind_text: String::new(),
            detail: vec![],
            worst: None,
            issue_count: 0,
            legacy: false,
            pane_id: None,
            pane_state: None,
            frame: None,
            band_bg: None,
            title_right: vec![],
            children: vec![],
        }
    }
}

pub struct View<'a> {
    pub filter: &'a str,
    /// only members with issues or NeedsYou
    pub flagged_only: bool,
    /// fleet states to show, indexed like FleetState::ALL
    pub fleet_states: [bool; 7],
    /// show the stowaway frame
    pub show_stowaways: bool,
    pub panes: &'a [Pane],
    pub icons: &'a Icons,
    pub now: f64,
    /// count cells every band shows (`count_slots`)
    pub slots: [bool; 7],
}

fn worst_of(issues: &[Issue]) -> Option<Severity> {
    issues.iter().map(|i| i.severity).max()
}

fn member_age(m: &Member, now: f64) -> String {
    let s = m.session.as_ref();
    match s.and_then(|s| s.status_since).or(s.and_then(|s| s.started_at)) {
        Some(t) => age(now - t),
        None => String::new(),
    }
}

fn crew_node(parent: &str, c: &Crew, kids: &HashMap<String, Vec<&Crew>>, now: f64, icons: &Icons, depth: usize) -> Node {
    let style = if c.running { st(mono::GREEN) } else { st(mono::COMMENT) };
    let mut n = Node::new(format!("C:{parent}:{}", c.agent_id), Kind::Crew);
    n.icon = if c.teammate { icons.teammate } else { icons.crew };
    n.icon_style = style;
    n.name = c.label.clone();
    n.name_style = style;
    n.status = (if c.running { "> running".into() } else { ". finished".into() }, style);
    n.age = age(now - c.last_write);
    n.kind_text = if c.teammate { "teammate".into() } else { "crew".into() };
    n.detail = vec![Span::styled(c.agent_type.clone().unwrap_or_default(), st(mono::COMMENT))];
    if depth < 32 {
        if let Some(ch) = kids.get(&c.agent_id) {
            n.children = ch.iter().map(|k| crew_node(parent, k, kids, now, icons, depth + 1)).collect();
        }
    }
    n
}

fn crew_nodes(parent: &str, crew: &[Crew], now: f64, icons: &Icons) -> Vec<Node> {
    let ids: std::collections::HashSet<&str> = crew.iter().map(|c| c.agent_id.as_str()).collect();
    let mut kids: HashMap<String, Vec<&Crew>> = HashMap::new();
    let mut roots: Vec<&Crew> = vec![];
    for c in crew {
        match c.parent_id.as_deref().filter(|p| ids.contains(p)) {
            Some(p) => kids.entry(p.to_string()).or_default().push(c),
            None => roots.push(c),
        }
    }
    let mut out: Vec<Node> = roots.iter().map(|c| crew_node(parent, c, &kids, now, icons, 0)).collect();
    out.sort_by(|a, b| a.kind_text.cmp(&b.kind_text).then(a.name.cmp(&b.name)));
    out
}

fn task_node(parent: &str, t: &Task, icons: &Icons) -> Node {
    let (icon, c) = match t.status.as_str() {
        "completed" => (icons.completed, mono::COMMENT),
        "in_progress" => (icons.in_progress, mono::GREEN),
        _ => (icons.pending, mono::YELLOW),
    };
    let mut n = Node::new(format!("T:{parent}:{}", t.id), Kind::Task);
    n.icon = icon;
    n.icon_style = st(c);
    n.name = t.subject.clone();
    n.name_style = st(mono::SOFT);
    n.status = (t.status.replace('_', " "), st(c));
    n.kind_text = "task".into();
    n.detail = vec![Span::styled(t.owner.clone().unwrap_or_default(), st(mono::COMMENT))];
    n
}

fn extra_row_node(parent: &str, s: &SessionInfo, now: f64, icons: &Icons) -> Node {
    let live = s.pid.is_some();
    let style = if live { st(mono::PINK) } else { st(mono::DIM) };
    let mut n = Node::new(format!("X:{parent}:{}", s.session_id), Kind::ExtraRow);
    n.member = Some(nimitz_core::Id(parent.to_string()));
    n.session_id = Some(s.session_id.clone());
    n.icon = icons.row;
    n.icon_style = style;
    n.name = s.short_id.clone().unwrap_or_else(|| s.session_id.chars().take(8).collect());
    n.name_style = style;
    n.status = (if live { "! live copy".into() } else { s.state.clone().unwrap_or("dead".into()) }, style);
    n.age = s.started_at.map(|t| age(now - t)).unwrap_or_default();
    n.kind_text = "copy".into();
    n.detail = vec![Span::styled(format!("{}  {}", s.kind, s.cwd), st(mono::DIM))];
    n
}

/// A row marked legacy: it has a `legacy` issue and no error.
pub fn is_legacy(issues: &[&Issue]) -> bool {
    issues.iter().any(|i| i.code == "legacy") && !issues.iter().any(|i| i.severity == Severity::Error)
}

/// The snapshot-level issues about one stowaway (by its name or short id in the message or fix).
pub fn stowaway_issues<'a>(snap: &'a Snapshot, s: &SessionInfo) -> Vec<&'a Issue> {
    let short = s.short_id.clone().unwrap_or_else(|| s.session_id.chars().take(8).collect());
    snap.issues
        .iter()
        .filter(|i| i.id.is_none() || i.code == "legacy")
        .filter(|i| {
            (!s.name.is_empty() && i.message.contains(&format!("`{}`", s.name)))
                || i.fix.as_deref().map(|f| f.split_whitespace().any(|w| w == short || w == s.session_id)).unwrap_or(false)
        })
        .collect()
}

/// Issue markers for a member's DETAIL cell: `⚠ code, code` in the worst colour.
fn issue_spans(issues: &[Issue], icons: &Icons) -> Vec<Span<'static>> {
    let Some(w) = worst_of(issues) else { return vec![] };
    let codes: Vec<&str> = issues.iter().map(|i| i.code.as_str()).collect();
    if is_legacy(&issues.iter().collect::<Vec<_>>()) {
        return vec![Span::styled(format!("{} {}  ", icons.legacy, codes.join(", ")), bold(mono::ORANGE))];
    }
    vec![Span::styled(format!("{} {}  ", icons.issue, codes.join(", ")), bold(theme::severity_color(w)))]
}

fn member_node(m: &Member, v: &View) -> Node {
    let live = member_live(m);
    let style = status_style(m.status);
    let name_style = match m.role {
        Role::Admiral if live => bold(mono::GOLD),
        Role::Commodore if live => bold(mono::CYAN),
        _ => style,
    };
    let mut n = Node::new(format!("M:{}", m.id.0), Kind::Member(m.role));
    n.member = Some(m.id.clone());
    n.session_id = m.session.as_ref().map(|s| s.session_id.clone());
    n.icon = v.icons.role(m.role);
    n.icon_style = name_style;
    n.name = m.session.as_ref().map(|s| s.name.clone()).filter(|s| !s.is_empty()).unwrap_or(m.expected_name.clone());
    n.name_style = name_style;
    n.status = (format!("{} {}", v.icons.status(m.status), status_word(m.status)), style);
    n.age = member_age(m, v.now);
    n.kind_text = m.role.as_str().into();
    n.worst = worst_of(&m.issues);
    n.issue_count = m.issues.len();
    n.legacy = is_legacy(&m.issues.iter().collect::<Vec<_>>());
    if let Some(p) = m.pane.as_deref().and_then(|id| v.panes.iter().find(|p| p.pane.pane_id == id)) {
        n.pane_id = Some(p.pane.pane_id.clone());
        n.pane_state = Some(p.state.clone());
    }
    n.detail = issue_spans(&m.issues, v.icons);
    if m.status == Status::NeedsYou {
        let why = m
            .session
            .as_ref()
            .and_then(|s| s.waiting_for.clone())
            .or_else(|| m.reasons.first().cloned())
            .unwrap_or_default();
        n.detail.push(Span::styled(why, st(mono::PINK)));
    } else if n.detail.is_empty() {
        let running = m.crew.iter().filter(|c| c.running).count();
        if running > 0 {
            n.detail.push(Span::styled(format!("{running} crew running"), st(mono::GREEN)));
        } else if let Some(r) = m.reasons.first() {
            n.detail.push(Span::styled(r.clone(), st(mono::COMMENT)));
        }
    }
    let key = m.id.0.clone();
    n.children = crew_nodes(&key, &m.crew, v.now, v.icons);
    n.children.extend(m.tasks.iter().map(|t| task_node(&key, t, v.icons)));
    n.children.extend(m.extra_rows.iter().map(|s| extra_row_node(&key, s, v.now, v.icons)));
    n.default_expand = m.crew.iter().any(|c| c.running) || m.extra_rows.iter().any(|s| s.pid.is_some());
    n
}

/// `icon n / icon n` for a band's right side: issues, then each status. Each
/// kind in `slots` (the kinds present anywhere on screen) has a fixed cell,
/// blank when zero, so the counts of every band line up in columns.
pub fn status_counts(members: &[&Member], issues: usize, worst: Option<Severity>, icons: &Icons, slots: &[bool; 7]) -> Vec<Span<'static>> {
    let c = theme::severity_color(worst.unwrap_or(Severity::Warn));
    let mut cells: Vec<Option<Span<'static>>> = vec![(issues > 0).then(|| Span::styled(format!("{} {issues:<2}", icons.issue), bold(c)))];
    for status in SLOT_STATUSES {
        let n = members.iter().filter(|m| m.status == status).count();
        cells.push((n > 0).then(|| Span::styled(format!("{} {n:<2}", icons.status(status)), status_style(status).add_modifier(Modifier::BOLD))));
    }
    let cells: Vec<Option<Span<'static>>> = cells.into_iter().zip(slots.iter()).filter(|(_, on)| **on).map(|(c, _)| c).collect();
    let mut out: Vec<Span<'static>> = vec![];
    let n = cells.len();
    for (i, cell) in cells.iter().enumerate() {
        let shown = cell.is_some();
        out.push(cell.clone().unwrap_or_else(|| Span::raw(" ".repeat(SLOT_W))));
        if i + 1 < n {
            // a slash only between two neighbouring counts that are both shown
            let next = cells[i + 1].is_some();
            out.push(Span::styled(if shown && next { " / " } else { "   " }, st(mono::DIM)));
        }
    }
    out
}

const SLOT_STATUSES: [Status; 6] = [Status::NeedsYou, Status::Busy, Status::Idle, Status::Stopped, Status::NotStarted, Status::Unknown];
/// `icon nn`: counts up to 99 keep the columns
const SLOT_W: usize = 4;

/// Which count cells any band needs: issues, then each status in `SLOT_STATUSES`.
pub fn count_slots(snap: &Snapshot) -> [bool; 7] {
    let mut on = [false; 7];
    on[0] = !all_issues(snap).is_empty();
    for f in &snap.fleets {
        for m in fleet_members(f) {
            if let Some(i) = SLOT_STATUSES.iter().position(|s| *s == m.status) {
                on[i + 1] = true;
            }
        }
    }
    on
}

fn comms_text(c: &Comms) -> String {
    match c {
        Comms::Mode(m) => m.clone(),
        Comms::Allow { allow } => format!("allow: {}", allow.join(", ")),
    }
}

fn squadron_node(f: &Fleet, sq: &Squadron, v: &View, matches: &dyn Fn(&Member) -> bool) -> Option<Node> {
    let members: Vec<&Member> = squadron_members(sq);
    let shown: Vec<&&Member> = members.iter().filter(|m| matches(m)).collect();
    if shown.is_empty() && (!v.filter.is_empty() || v.flagged_only) {
        return None;
    }
    let own_issues: Vec<&Issue> = sq.issues.iter().chain(members.iter().flat_map(|m| m.issues.iter())).collect();
    let worst = own_issues.iter().map(|i| i.severity).max();
    let state = FleetState::of_squadron(f, sq);
    let color = state.color();
    let sq_name = sq.id.squadron().unwrap_or("?").to_string();
    let mut n = Node::new(format!("SQ:{}", sq.id.0), Kind::SquadronBand);
    n.member = Some(sq.id.clone());
    n.default_expand = true;
    n.icon = v.icons.squadron;
    n.icon_style = bold(color);
    n.name = sq_name;
    n.name_style = bold(mono::FG);
    n.kind_text = "squadron".into();
    n.frame = Some(color);
    n.band_bg = Some(state.tint());
    n.worst = worst_of(&sq.issues);
    n.issue_count = sq.issues.len();
    n.detail = vec![
        Span::styled(state.squadron_label().to_string(), bold(color)),
        Span::styled(format!("   comms {}", comms_text(&sq.comms)), st(mono::SOFT)),
    ];
    if let Some(i) = sq.issues.first() {
        n.detail.push(Span::styled(format!("   {} {}", v.icons.issue, i.code), bold(theme::severity_color(i.severity))));
    }
    n.title_right = status_counts(&members, own_issues.len(), worst, v.icons, &v.slots);
    n.children = shown.iter().map(|m| member_node(m, v)).collect();
    Some(n)
}

fn stow_node(s: &SessionInfo, snap: &Snapshot, v: &View) -> Node {
    let live = s.pid.is_some();
    let style = if live { bold(mono::PINK) } else { st(mono::COMMENT) };
    let mut n = Node::new(format!("SW:{}", s.session_id), Kind::Stowaway);
    n.session_id = Some(s.session_id.clone());
    n.icon = v.icons.stowaway;
    n.icon_style = style;
    n.name = if s.name.is_empty() { s.session_id.chars().take(8).collect() } else { s.name.clone() };
    n.name_style = style;
    n.status = (
        if live { format!("{} live", v.icons.s_busy) } else { format!("{} dead", v.icons.s_stopped) },
        if live { st(mono::PINK) } else { st(mono::COMMENT) },
    );
    n.age = s.started_at.map(|t| age(v.now - t)).unwrap_or_default();
    n.kind_text = "stowaway".into();
    if let Some(p) = pane_for(Some(&s.session_id), &n.name, Some(&s.cwd), v.panes) {
        n.pane_id = Some(p.pane.pane_id.clone());
        n.pane_state = Some(p.state.clone());
    }
    // the snapshot-level issues about this stowaway carry the fix
    let issues = stowaway_issues(snap, s);
    n.worst = issues.iter().map(|i| i.severity).max();
    n.issue_count = issues.len();
    n.legacy = is_legacy(&issues);
    let fix = issues.iter().find_map(|i| i.fix.clone());
    n.detail = vec![];
    if n.legacy {
        n.detail.push(Span::styled(format!("{} legacy  ", v.icons.legacy), bold(mono::ORANGE)));
    }
    n.detail.push(Span::styled(format!("no labels  {}", s.cwd), st(mono::SOFT)));
    if let Some(fx) = fix {
        n.detail.push(Span::styled(format!("   fix: {fx}"), st(mono::CYAN)));
    }
    n
}

pub fn build_nodes(snap: &Snapshot, v: &View) -> Vec<Node> {
    let q = v.filter.trim().to_lowercase();
    let matches = |m: &Member| -> bool {
        if v.flagged_only && m.issues.is_empty() && m.status != Status::NeedsYou {
            return false;
        }
        q.is_empty()
            || m.id.0.to_lowercase().contains(&q)
            || m.expected_name.to_lowercase().contains(&q)
            || m.session.as_ref().map(|s| s.name.to_lowercase().contains(&q)).unwrap_or(false)
            || m.crew.iter().any(|c| c.label.to_lowercase().contains(&q))
    };
    let mut out = vec![];
    for f in &snap.fleets {
        let state = FleetState::of(f);
        if !v.fleet_states[state.index()] {
            continue;
        }
        let shown_any = fleet_members(f).iter().any(|m| matches(m));
        if (!q.is_empty() || v.flagged_only) && !shown_any {
            continue;
        }
        let issues: Vec<&Issue> = fleet_issues(f);
        let worst = issues.iter().map(|i| i.severity).max();
        let mut n = Node::new(format!("F:{}", f.id.0), Kind::FleetBand);
        n.member = Some(f.id.clone());
        n.default_expand = f.machine.is_some();
        n.icon = v.icons.fleet;
        n.icon_style = bold(mono::GOLD);
        n.name = format!("[{}]", f.id.0);
        n.name_style = bold(mono::GOLD);
        n.kind_text = "fleet".into();
        n.frame = Some(state.color());
        n.band_bg = Some(state.tint());
        n.worst = worst_of(&f.issues);
        n.issue_count = f.issues.len();
        let place = f.machine.clone().unwrap_or_else(|| "not on this machine".into());
        n.detail = vec![
            Span::styled(state.label().to_string(), bold(state.color())),
            Span::styled(format!("   {place}   {}", f.dir), st(mono::SOFT)),
        ];
        n.title_right = status_counts(&fleet_members(f), issues.len(), worst, v.icons, &v.slots);
        if matches(&f.admiral) {
            n.children.push(member_node(&f.admiral, v));
        }
        for sq in &f.squadrons {
            if let Some(sn) = squadron_node(f, sq, v, &matches) {
                n.children.push(sn);
            }
        }
        out.push(n);
    }
    if v.show_stowaways && !snap.stowaways.is_empty() && q.is_empty() && !v.flagged_only {
        let live = snap.stowaways.iter().filter(|s| s.pid.is_some()).count();
        let mut n = Node::new("STOW".into(), Kind::StowBand);
        n.default_expand = true;
        n.icon = v.icons.stowaway;
        n.icon_style = bold(mono::PINK);
        n.name = "STOWAWAYS".into();
        n.name_style = bold(mono::PINK);
        n.kind_text = "group".into();
        n.frame = Some(mono::PINK);
        n.band_bg = Some(mono::T_PINK);
        n.detail = vec![Span::styled("sessions in fleet scope with no labels — adopt or remove", st(mono::SOFT))];
        n.title_right = vec![Span::styled(format!("{live} live / {} total", snap.stowaways.len()), bold(mono::PINK))];
        n.children = snap.stowaways.iter().map(|s| stow_node(s, snap, v)).collect();
        out.push(n);
    }
    out
}

// ------------------------------------------------------------------ rows

/// Which frame a row lives in; the renderer draws borders from this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    /// a fleet's title band (opens the outer, double-line frame)
    FleetBand,
    /// a row directly inside the fleet frame (admiral, its crew)
    FleetBody,
    /// a squadron's title band (opens the inner frame)
    SquadronBand,
    /// a row inside a squadron frame
    SquadronBody,
    /// the stowaway group's title band
    StowBand,
    /// a row inside the stowaway frame
    StowBody,
}

/// One visible line of the flattened tree.
#[derive(Clone, Debug)]
pub struct Row {
    pub node: Node,
    pub section: Section,
    /// tree guides, relative to the row's frame
    pub prefix: String,
    pub expanded: bool,
}

pub fn flatten(nodes: &[Node], expanded: &HashMap<String, bool>) -> Vec<Row> {
    fn open(node: &Node, exp: &HashMap<String, bool>) -> bool {
        !node.children.is_empty() && *exp.get(&node.id).unwrap_or(&node.default_expand)
    }
    // guides restart below each band; members sit at guide depth 0
    fn walk(out: &mut Vec<Row>, nodes: &[Node], section: Section, depth: usize, lead: &str, exp: &HashMap<String, bool>) {
        let n = nodes.len();
        for (i, node) in nodes.iter().enumerate() {
            let last = i + 1 == n;
            match node.kind {
                Kind::SquadronBand => {
                    let o = open(node, exp);
                    out.push(Row { node: node.clone(), section: Section::SquadronBand, prefix: String::new(), expanded: o });
                    if o {
                        walk(out, &node.children, Section::SquadronBody, 0, "", exp);
                    }
                }
                _ => {
                    let prefix = if depth == 0 { String::new() } else { format!("{lead}{}", if last { "└─ " } else { "├─ " }) };
                    let o = open(node, exp);
                    out.push(Row { node: node.clone(), section, prefix, expanded: o });
                    if o {
                        let next = if depth == 0 { String::new() } else { format!("{lead}{}", if last { "   " } else { "│  " }) };
                        walk(out, &node.children, section, depth + 1, &next, exp);
                    }
                }
            }
        }
    }
    let mut out = vec![];
    for node in nodes {
        let (band, body) = match node.kind {
            Kind::StowBand => (Section::StowBand, Section::StowBody),
            _ => (Section::FleetBand, Section::FleetBody),
        };
        let o = open(node, expanded);
        out.push(Row { node: node.clone(), section: band, prefix: String::new(), expanded: o });
        if o {
            walk(&mut out, &node.children, body, 0, "", expanded);
        }
    }
    out
}
