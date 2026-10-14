//! From `nimitz_core::Snapshot` to what the screen shows: a tree of nodes
//! (fleet band -> admiral + squadron frames -> members -> crew/tasks), then a
//! flat list of rows with tree guides. Pure data: no terminal here, so tests
//! drive it directly.

use std::collections::HashMap;

use nimitz_core::{Crew, Fleet, Issue, Member, Role, SessionInfo, Severity, Snapshot, Squadron, Status, Task};
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

    /// Every band sits on the one raised background: no tinted bands.
    pub fn tint(self) -> Color {
        mono::BG_HI
    }

    /// the state word's colour
    pub fn color(self) -> Color {
        match self {
            FleetState::NeedsYou => mono::NEEDS,
            FleetState::Broken => mono::ERROR,
            FleetState::Busy => mono::BUSY,
            FleetState::Idle => mono::FG,
            FleetState::NoAdmiral => mono::WARN,
            FleetState::Inactive => mono::MUTED,
            FleetState::Elsewhere => mono::FAINT,
        }
    }

    /// the frame: neutral, unless it needs you or does not load
    pub fn frame(self) -> Color {
        match self {
            FleetState::NeedsYou => mono::NEEDS,
            FleetState::Broken => mono::ERROR,
            _ => mono::LINE,
        }
    }

    pub fn index(self) -> usize {
        FleetState::ALL.iter().position(|x| *x == self).unwrap()
    }
}

/// Every member of a fleet, chain-of-command order.
pub fn fleet_members(f: &Fleet) -> Vec<&Member> {
    f.members().collect()
}

pub fn squadron_members(sq: &Squadron) -> Vec<&Member> {
    let mut v = vec![&sq.commodore];
    v.extend(sq.ships.iter());
    v
}

/// Every issue in a fleet: the fleet's own, the squadrons', every member's.
pub fn fleet_issues(f: &Fleet) -> Vec<&Issue> {
    let mut v: Vec<&Issue> = f.issues.iter().collect();
    v.extend(f.squadrons.iter().flat_map(|sq| sq.issues.iter()));
    v.extend(f.members().flat_map(|m| m.issues.iter()));
    v
}

/// Every issue in the snapshot, member issues included, each once (core may
/// put one finding on a squadron and on each of its members), worst first.
pub fn all_issues(snap: &Snapshot) -> Vec<&Issue> {
    let mut v: Vec<&Issue> = snap.issues.iter().collect();
    for f in &snap.fleets {
        v.extend(fleet_issues(f));
    }
    let mut seen = std::collections::HashSet::new();
    v.retain(|i| seen.insert((i.code.clone(), i.id.clone(), i.message.clone())));
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
    /// a member's finished crew older than `CREW_RECENT_S`, folded into one
    /// `+N finished` row (`z` opens and closes it)
    CrewFold,
    /// a Workflow run's agents under one row: its name, live / total, phase
    Workflow,
    Task,
    /// an extra job row of the identity (copy, stale row)
    ExtraRow,
    Stowaway,
    /// a line of text across the frame (a config error, in full)
    Note,
}

/// What moves on a row in the live loop (`theme::anim`).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Anim {
    #[default]
    None,
    /// busy: the status glyph spins
    Spin,
    /// busy waiting on a background shell: slow dots
    Dots,
    /// a running crew: the spinner, dimmer
    SpinDim,
    /// needs you: the chip pulses, about once a second
    Pulse,
    /// a squadron bar whose status just got worse: one pulse from this frame
    Flash(u64),
}

#[derive(Clone, Debug)]
pub struct Node {
    /// the name is cut from the left when it does not fit (a full name in a
    /// search: its own part stays)
    pub cut_left: bool,
    /// what moves on this row in the live loop
    pub anim: Anim,
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
    /// band only: the counts, laid out by `frames` to fit the band
    pub counts: Vec<Count>,
    /// per DETAIL span, its rank when the cell is too narrow: spans go whole,
    /// the highest rank first; empty: cut the end
    pub drop_rank: Vec<u8>,
    pub children: Vec<Node>,
}

impl Node {
    fn new(id: String, kind: Kind) -> Node {
        Node {
            id,
            kind,
            member: None,
            session_id: None,
            cut_left: false,
            anim: Anim::None,
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
            counts: vec![],
            drop_rank: vec![],
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
    /// the tab shown: a fleet id, or UNASSIGNED; None shows every fleet
    pub tab: Option<&'a str>,
}

/// The tab of stowaways that no one fleet's folders hold (`~` is never in a fleet name).
pub const UNASSIGNED: &str = "~UNASSIGNED";

/// What a tab is called on screen.
pub fn tab_title(tab: &str) -> String {
    if tab == UNASSIGNED {
        "unassigned".into()
    } else {
        tab.to_string()
    }
}

/// The tabs, in order: every fleet (a broken one too), then UNASSIGNED when a
/// stowaway belongs to no fleet.
pub fn tabs(snap: &Snapshot) -> Vec<String> {
    let mut out: Vec<String> = snap.fleets.iter().map(|f| f.id.0.clone()).collect();
    for f in broken_fleets(snap) {
        if !out.contains(&f.0) {
            out.push(f.0);
        }
    }
    if snap.stowaways.iter().any(|s| nimitz_core::snapshot::stowaway_fleet(snap, s).is_none()) {
        out.push(UNASSIGNED.into());
    }
    out
}

/// The tab a stowaway shows in.
pub fn stowaway_tab(snap: &Snapshot, s: &SessionInfo) -> String {
    nimitz_core::snapshot::stowaway_fleet(snap, s).unwrap_or_else(|| UNASSIGNED.into())
}

fn worst_of(issues: &[Issue]) -> Option<Severity> {
    issues.iter().map(|i| i.severity).max()
}

/// A session's age; `?` when it has a session but core has no believable time
/// for it (BUG-45), empty when there is no session at all.
fn member_age(m: &Member, now: f64) -> String {
    match &m.session {
        Some(s) => session_age(s, now),
        None => String::new(),
    }
}

fn session_age(s: &SessionInfo, now: f64) -> String {
    match s.status_since.or(s.started_at) {
        Some(t) => age_or_unknown(now - t),
        None => "?".into(),
    }
}

/// `age`, or `?` for a time that is missing or not believable.
pub fn age_or_unknown(seconds: f64) -> String {
    if seconds.is_finite() && seconds >= 0.0 {
        age(seconds)
    } else {
        "?".into()
    }
}

fn crew_node(parent: &str, c: &Crew, kids: &HashMap<String, Vec<&Crew>>, now: f64, icons: &Icons, depth: usize) -> Node {
    let style = if c.running { st(mono::BUSY) } else { st(mono::COMMENT) };
    let mut n = Node::new(format!("C:{parent}:{}", c.agent_id), Kind::Crew);
    n.icon = if c.teammate { icons.teammate } else { icons.crew };
    n.icon_style = style;
    n.name = c.label.clone();
    n.name_style = style;
    n.status = (if c.running { "> running".into() } else { ". finished".into() }, style);
    if c.running {
        n.anim = Anim::SpinDim;
    }
    n.age = if c.last_write > 0.0 { age_or_unknown(now - c.last_write) } else { "?".into() };
    n.kind_text = if c.teammate { "teammate".into() } else { "crew".into() };
    let phase = c.phase.as_ref().map(|p| format!("{p}  ")).unwrap_or_default();
    n.detail = vec![Span::styled(format!("{phase}{}", c.agent_type.clone().unwrap_or_default()), st(mono::COMMENT))];
    if depth < 32 {
        if let Some(ch) = kids.get(&c.agent_id) {
            n.children = ch.iter().map(|k| crew_node(parent, k, kids, now, icons, depth + 1)).collect();
        }
    }
    n
}

/// Finished crew stay in view this long after their last write; older ones
/// fold into the member's `+N finished` row. A conversation that lives for
/// days keeps every crew it ever ran: the list only grows.
pub const CREW_RECENT_S: f64 = 30.0 * 60.0;

/// The id of member `parent`'s `+N finished` row.
pub fn crew_fold_id(parent: &str) -> String {
    format!("CF:{parent}")
}

/// A member's crew as a tree: (children by parent id, the roots), each root
/// with whether it shows (`true`) or folds into `+N finished`: it shows while
/// it runs, for `CREW_RECENT_S` after it finished, while any of its nested
/// crew runs, or while the name filter `q` matches it or a nested crew.
fn crew_roots<'a>(crew: &'a [Crew], now: f64, q: &str) -> (HashMap<String, Vec<&'a Crew>>, Vec<(&'a Crew, bool)>) {
    let ids: std::collections::HashSet<&str> = crew.iter().map(|c| c.agent_id.as_str()).collect();
    let mut kids: HashMap<String, Vec<&Crew>> = HashMap::new();
    let mut roots: Vec<&Crew> = vec![];
    for c in crew {
        match c.parent_id.as_deref().filter(|p| ids.contains(p)) {
            Some(p) => kids.entry(p.to_string()).or_default().push(c),
            None => roots.push(c),
        }
    }
    fn subtree<'a>(c: &'a Crew, kids: &HashMap<String, Vec<&'a Crew>>, depth: usize, out: &mut Vec<&'a Crew>) {
        out.push(c);
        if depth < 32 {
            for k in kids.get(&c.agent_id).into_iter().flatten() {
                subtree(k, kids, depth + 1, out);
            }
        }
    }
    let judged = roots
        .into_iter()
        .map(|c| {
            let mut all = vec![];
            subtree(c, &kids, 0, &mut all);
            let visible = c.running || now - c.last_write < CREW_RECENT_S || all.iter().any(|x| x.running) || (!q.is_empty() && all.iter().any(|x| x.label.to_lowercase().contains(q)));
            (c, visible)
        })
        .collect();
    (kids, judged)
}

/// A member's crew split: the plain subagents, and the agents of each
/// Workflow run (by run id, in id order).
fn split_runs(crew: &[Crew]) -> (Vec<Crew>, Vec<(String, Vec<&Crew>)>) {
    let mut runs: std::collections::BTreeMap<String, Vec<&Crew>> = std::collections::BTreeMap::new();
    let mut plain = vec![];
    for c in crew {
        match &c.workflow {
            Some(w) => runs.entry(w.clone()).or_default().push(c),
            None => plain.push(c.clone()),
        }
    }
    (plain, runs.into_iter().collect())
}

/// A Workflow run is one root: it shows while any of its agents runs, for
/// `CREW_RECENT_S` after its last write, or while the filter `q` matches one
/// of its agents or the workflow's name; else it folds into `+N finished`.
fn run_visible(agents: &[&Crew], now: f64, q: &str) -> bool {
    let last = agents.iter().map(|c| c.last_write).fold(0.0, f64::max);
    agents.iter().any(|c| c.running)
        || now - last < CREW_RECENT_S
        || (!q.is_empty() && agents.iter().any(|c| c.label.to_lowercase().contains(q) || c.workflow_name.as_deref().is_some_and(|n| n.to_lowercase().contains(q))))
}

/// The id of member `parent`'s row for Workflow run `run`.
pub fn workflow_row_id(parent: &str, run: &str) -> String {
    format!("WF:{parent}:{run}")
}

/// A Workflow run's row: the workflow's name (else the run id), live /
/// total, the phase of its newest agent; its agents under it, open while
/// one runs.
fn workflow_node(parent: &str, run: &str, agents: &[&Crew], now: f64, icons: &Icons) -> Node {
    let live = agents.iter().filter(|c| c.running).count();
    let newest = agents.iter().max_by(|a, b| a.last_write.partial_cmp(&b.last_write).unwrap_or(std::cmp::Ordering::Equal));
    let style = if live > 0 { st(mono::BUSY) } else { st(mono::COMMENT) };
    let mut n = Node::new(workflow_row_id(parent, run), Kind::Workflow);
    n.icon = icons.crew;
    n.icon_style = style;
    n.name = agents.iter().find_map(|c| c.workflow_name.clone()).unwrap_or_else(|| run.to_string());
    n.name_style = style;
    n.status = (if live > 0 { "> running".into() } else { ". finished".into() }, style);
    if live > 0 {
        n.anim = Anim::SpinDim;
    }
    n.age = newest.filter(|c| c.last_write > 0.0).map(|c| age_or_unknown(now - c.last_write)).unwrap_or_else(|| "?".into());
    n.kind_text = "workflow".into();
    let phase = newest.and_then(|c| c.phase.clone()).map(|p| format!("   {p}")).unwrap_or_default();
    n.detail = vec![
        Span::styled(format!("{live} live / {} total", agents.len()), style),
        Span::styled(format!("{phase}   {run}"), st(mono::COMMENT)),
    ];
    let owned: Vec<Crew> = agents.iter().map(|c| (*c).clone()).collect();
    let (kids, roots) = crew_roots(&owned, now, "");
    let mut children: Vec<Node> = roots.into_iter().map(|(c, _)| crew_node(parent, c, &kids, now, icons, 0)).collect();
    children.sort_by(|a, b| a.name.cmp(&b.name));
    n.children = children;
    n.default_expand = live > 0;
    n
}

/// How many top-level crew of `crew` (a Workflow run counts as one) fold
/// into `+N finished` under the name filter `q` (lowercase; empty for none).
pub fn folded_crew(crew: &[Crew], now: f64, q: &str) -> usize {
    let (plain, runs) = split_runs(crew);
    crew_roots(&plain, now, q).1.iter().filter(|(_, v)| !v).count() + runs.iter().filter(|(_, a)| !run_visible(a, now, q)).count()
}

fn crew_nodes(parent: &str, crew: &[Crew], now: f64, icons: &Icons, q: &str) -> Vec<Node> {
    let (plain, runs) = split_runs(crew);
    let (kids, roots) = crew_roots(&plain, now, q);
    let (mut shown, mut old): (Vec<Node>, Vec<Node>) = (vec![], vec![]);
    let mut youngest = 0.0f64;
    for (c, visible) in roots {
        let node = crew_node(parent, c, &kids, now, icons, 0);
        if visible {
            shown.push(node);
        } else {
            youngest = youngest.max(c.last_write);
            old.push(node);
        }
    }
    for (run, agents) in &runs {
        let node = workflow_node(parent, run, agents, now, icons);
        if run_visible(agents, now, q) {
            shown.push(node);
        } else {
            youngest = youngest.max(agents.iter().map(|c| c.last_write).fold(0.0, f64::max));
            old.push(node);
        }
    }
    let by = |a: &Node, b: &Node| a.kind_text.cmp(&b.kind_text).then(a.name.cmp(&b.name));
    shown.sort_by(by);
    if !old.is_empty() {
        old.sort_by(by);
        let style = st(mono::COMMENT);
        let mut n = Node::new(crew_fold_id(parent), Kind::CrewFold);
        n.icon = icons.crew;
        n.icon_style = style;
        n.name = format!("+{} finished", old.len());
        n.name_style = style;
        n.status = (". finished".into(), style);
        n.age = if youngest > 0.0 { age_or_unknown(now - youngest) } else { "?".into() };
        n.kind_text = "crew".into();
        n.detail = vec![Span::styled(format!("finished over {} min ago; z shows them", (CREW_RECENT_S / 60.0) as u64), style)];
        n.children = old;
        shown.push(n);
    }
    shown
}

fn task_node(parent: &str, t: &Task, icons: &Icons) -> Node {
    let (icon, c) = match t.status.as_str() {
        "completed" => (icons.completed, mono::COMMENT),
        "in_progress" => (icons.in_progress, mono::BUSY),
        _ => (icons.pending, mono::MUTED),
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
    let style = if live { st(mono::ERROR) } else { st(mono::DIM) };
    let mut n = Node::new(format!("X:{parent}:{}", s.session_id), Kind::ExtraRow);
    n.member = Some(nimitz_core::Id(parent.to_string()));
    n.session_id = Some(s.session_id.clone());
    n.icon = icons.row;
    n.icon_style = style;
    n.name = s.short_id.clone().unwrap_or_else(|| s.session_id.chars().take(8).collect());
    n.name_style = style;
    n.status = (if live { "! live copy".into() } else { s.state.clone().unwrap_or("dead".into()) }, style);
    n.age = s.started_at.map(|t| age_or_unknown(now - t)).unwrap_or_else(|| "?".into());
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
        return vec![Span::styled(format!("{} {}  ", icons.legacy, codes.join(", ")), bold(mono::WARN))];
    }
    vec![Span::styled(format!("{} {}  ", icons.issue, codes.join(", ")), bold(theme::severity_color(w)))]
}

fn member_node(m: &Member, v: &View) -> Node {
    let live = member_live(m);
    let style = status_style(m.status);
    // a name is text, never a chip: needs-you in its hue, a live session strong,
    // the rest by its status (faint when not started)
    let name_style = if m.status == Status::NeedsYou {
        bold(mono::NEEDS)
    } else if live {
        bold(mono::STRONG)
    } else {
        st(theme::status_color(m.status))
    };
    let mut n = Node::new(format!("M:{}", m.id.0), Kind::Member(m.role));
    n.member = Some(m.id.clone());
    n.session_id = m.session.as_ref().map(|s| s.session_id.clone());
    n.icon = v.icons.role(m.role);
    n.icon_style = name_style;
    // inside its fleet's tab and its squadron's box a row names only its own
    // part (`BRIM`, `COMMODORE`, `ADMIRAL`); the full name where that context
    // is missing (a search across fleets), and a session that runs under
    // another name (a drift, a copy) shows that name in full
    let full = m.session.as_ref().map(|s| s.name.clone()).filter(|s| !s.is_empty()).unwrap_or(m.expected_name.clone());
    let searching = !v.filter.trim().is_empty();
    // the member's own `[F.SQ]-` prefix, as its expected name carries it
    let own_prefix = m.expected_name.strip_suffix(&short_name(&m.id)).filter(|p| p.ends_with('-'));
    n.name = if searching {
        // in a search the full name, cut from the left when it must be cut,
        // so the part that tells rows apart stays
        n.cut_left = true;
        full
    } else if full == m.expected_name {
        short_name(&m.id)
    } else if let Some(rest) = own_prefix.and_then(|p| full.strip_prefix(p)).filter(|rest| rest.starts_with(&format!("{}-", short_name(&m.id)))) {
        // a drift of the member's own name (`ANVIL-2`: its name, a dash, more):
        // the drift shows. Names are [A-Z0-9_]+, so `<own>-...` is never a
        // sibling's; a sibling's name (`HAMMER`, or `ANVIL2` on ANVIL) stays
        // in full, never a second row that acts on this member
        rest.to_string()
    } else {
        full
    };
    n.name_style = name_style;
    n.status = (format!("{} {}", v.icons.status(m.status), status_word(m.status)), style);
    n.anim = match m.status {
        Status::NeedsYou => Anim::Pulse,
        Status::Busy if m.session.as_ref().and_then(|s| s.status.as_deref()) == Some("shell") => Anim::Dots,
        Status::Busy => Anim::Spin,
        _ => Anim::None,
    };
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
        n.detail.push(Span::styled(why, st(mono::NEEDS)));
    } else if n.detail.is_empty() {
        let running = m.crew.iter().filter(|c| c.running).count();
        if running > 0 {
            n.detail.push(Span::styled(format!("{running} crew running"), st(mono::BUSY)));
        } else if let Some(t) = &m.topic {
            // what the job is doing; the reasons stay in the detail pane
            n.detail.push(Span::styled(t.clone(), st(mono::COMMENT)));
        }
    }
    let key = m.id.0.clone();
    n.children = crew_nodes(&key, &m.crew, v.now, v.icons, &v.filter.trim().to_lowercase());
    n.children.extend(m.tasks.iter().map(|t| task_node(&key, t, v.icons)));
    n.children.extend(m.extra_rows.iter().map(|s| extra_row_node(&key, s, v.now, v.icons)));
    n.default_expand = m.crew.iter().any(|c| c.running) || m.extra_rows.iter().any(|s| s.pid.is_some());
    n
}

/// One count on a band's right side: `icon n`.
#[derive(Clone, Debug, PartialEq)]
pub struct Count {
    pub icon: &'static str,
    pub n: usize,
    pub style: Style,
    /// kept on a narrow band (needs you, errors)
    pub key: bool,
}

impl Count {
    /// `icon n`. A glyph a terminal may draw two cells wide gets a blank cell
    /// to spill into, or it covers the number (an ASCII icon needs none).
    pub fn text(&self) -> String {
        let gap = if crate::backend::risky(self.icon) { " " } else { "" };
        format!("{}{gap}{}", self.icon, self.n)
    }
}

/// A band's counts: errors, other issues, then members by status, zero counts
/// left out. `frames` lays them out (all of them, or the key ones when narrow).
pub fn status_counts(members: &[&Member], issues: &[&Issue], icons: &Icons) -> Vec<Count> {
    let mut out = vec![];
    let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
    // info issues are not counted: they stay muted on their own rows
    let others = issues.iter().filter(|i| i.severity == Severity::Warn).count();
    if errors > 0 {
        out.push(Count { icon: icons.issue, n: errors, style: bold(mono::ERROR), key: true });
    }
    if others > 0 {
        out.push(Count { icon: icons.issue, n: others, style: bold(mono::WARN), key: false });
    }
    for status in [Status::NeedsYou, Status::Busy, Status::Idle, Status::Stopped, Status::Parked, Status::NotStarted, Status::Unknown] {
        let n = members.iter().filter(|m| m.status == status).count();
        if n > 0 {
            out.push(Count { icon: icons.status(status), n, style: status_style(status).add_modifier(Modifier::BOLD), key: status == Status::NeedsYou });
        }
    }
    // needs-you first among the key ones, so a narrow band reads "needs you, errors"
    out.sort_by_key(|c| if c.key && c.icon == icons.s_needs { 0 } else if c.key { 1 } else { 2 });
    out
}

/// A member's own part of its name, for a row inside its fleet and
/// squadron: `ADMIRAL`, `COMMODORE`, or the ship's name. (`layout::legacy_tab_of`
/// says the same for a 0.3.x tab label, from the third part of the id; this
/// takes the last, the name's own part whatever the depth.)
pub fn short_name(id: &nimitz_core::Id) -> String {
    match id.role() {
        Role::Admiral => "ADMIRAL".into(),
        Role::Commodore => "COMMODORE".into(),
        _ => id.0.rsplit('/').next().unwrap_or(&id.0).to_string(),
    }
}

/// The colour of a squadron's top border: the worst of its commodore and
/// ships, in this order (theme A): needs you (pink) > error (orange: an issue
/// at Error) > warning (yellow: an issue at Warn) > busy (green) > neutral.
/// Idle, stopped, parked, not started and unknown are neutral; info issues
/// do not count. A squadron whose config does not load is an error itself.
pub fn squadron_status_color(members: &[&Member]) -> ratatui::style::Color {
    match squadron_rank(members) {
        4 => mono::NEEDS,
        3 => mono::ERROR,
        2 => mono::WARN,
        1 => mono::BUSY,
        _ => mono::LINE,
    }
}

/// A squadron's rank, its own config error included (an error).
pub fn squadron_rank_of(sq: &Squadron) -> u8 {
    let members: Vec<&Member> = std::iter::once(&sq.commodore).chain(sq.ships.iter()).collect();
    let broken = sq.issues.iter().any(|i| is_load_error(i) && i.id.as_ref() == Some(&sq.id));
    squadron_rank(&members).max(if broken { 3 } else { 0 })
}

/// The squadron's status as a rank in that order (4 needs you .. 0 neutral).
pub fn squadron_rank(members: &[&Member]) -> u8 {
    let rank = |m: &Member| -> u8 {
        if m.status == Status::NeedsYou {
            4
        } else if m.issues.iter().any(|i| i.severity == Severity::Error) {
            3
        } else if m.issues.iter().any(|i| i.severity == Severity::Warn) {
            2
        } else if m.status == Status::Busy {
            1
        } else {
            0
        }
    };
    members.iter().map(|m| rank(m)).max().unwrap_or(0)
}

fn squadron_node(f: &Fleet, sq: &Squadron, v: &View, matches: &dyn Fn(&Member) -> bool) -> Option<Node> {
    let members: Vec<&Member> = squadron_members(sq);
    let shown: Vec<&&Member> = members.iter().filter(|m| matches(m)).collect();
    if shown.is_empty() && (!v.filter.is_empty() || v.flagged_only) {
        return None;
    }
    let own_issues: Vec<&Issue> = sq.issues.iter().chain(members.iter().flat_map(|m| m.issues.iter())).collect();
    let state = FleetState::of_squadron(f, sq);
    let load_errors: Vec<&Issue> = sq.issues.iter().filter(|i| is_load_error(i) && i.id.as_ref() == Some(&sq.id)).collect();
    let color = if load_errors.is_empty() { state.color() } else { mono::ERROR };
    let frame = if load_errors.is_empty() { squadron_status_color(&members) } else { mono::ERROR };
    let sq_name = sq.id.squadron().unwrap_or("?").to_string();
    let mut n = Node::new(format!("SQ:{}", sq.id.0), Kind::SquadronBand);
    n.member = Some(sq.id.clone());
    n.default_expand = true;
    n.icon = v.icons.squadron;
    n.icon_style = bold(mono::MUTED);
    n.name = sq_name;
    n.name_style = bold(mono::STRONG);
    n.kind_text = "squadron".into();
    n.frame = Some(frame);
    n.band_bg = Some(state.tint());
    n.worst = worst_of(&sq.issues);
    n.issue_count = sq.issues.len();
    n.detail = vec![
        Span::styled(state.squadron_label().to_string(), bold(color)),
        Span::styled(format!("   ships {}", sq.ships.len()), st(mono::SOFT)),
    ];
    if let Some(i) = sq.issues.first() {
        n.detail.push(Span::styled(format!("   {} {}", v.icons.issue, i.code), bold(theme::severity_color(i.severity))));
    }
    n.counts = status_counts(&members, &own_issues, v.icons);
    // a squadron.toml that does not load: the error in full, above its members
    n.children = error_notes(&sq.id, &load_errors);
    n.children.extend(shown.iter().map(|m| member_node(m, v)));
    Some(n)
}

fn stow_node(s: &SessionInfo, snap: &Snapshot, v: &View) -> Node {
    let live = s.pid.is_some();
    let style = if live { bold(mono::WARN) } else { st(mono::COMMENT) };
    let mut n = Node::new(format!("SW:{}", s.session_id), Kind::Stowaway);
    n.session_id = Some(s.session_id.clone());
    n.icon = v.icons.stowaway;
    n.icon_style = style;
    n.name = if s.name.is_empty() { s.session_id.chars().take(8).collect() } else { s.name.clone() };
    n.name_style = style;
    n.status = (
        if live { format!("{} live", v.icons.s_busy) } else { format!("{} dead", v.icons.s_stopped) },
        if live { st(mono::WARN) } else { st(mono::COMMENT) },
    );
    n.age = s.started_at.map(|t| age_or_unknown(v.now - t)).unwrap_or_else(|| "?".into());
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
        n.detail.push(Span::styled(format!("{} legacy  ", v.icons.legacy), bold(mono::WARN)));
    } else if let Some(w) = n.worst {
        // the row's only severity mark (NAME has none)
        let codes: Vec<&str> = issues.iter().map(|i| i.code.as_str()).collect();
        n.detail.push(Span::styled(format!("{} {}  ", v.icons.issue, codes.join(", ")), bold(theme::severity_color(w))));
    }
    n.detail.push(Span::styled(format!("no labels  {}", s.cwd), st(mono::SOFT)));
    if let Some(fx) = fix {
        n.detail.push(Span::styled(format!("   fix: {fx}"), st(mono::ACCENT)));
    }
    n
}

/// A config error that stops a fleet or squadron loading (as opposed to a
/// session labelled for an identity the config lacks).
fn is_load_error(i: &Issue) -> bool {
    i.code == "config.invalid" && !i.message.contains("which no config declares")
}

/// Sessions labelled for identities under `id` that the config does not declare.
pub fn unlisted_sessions(snap: &Snapshot, id: &nimitz_core::Id) -> usize {
    all_issues(snap)
        .iter()
        .filter(|i| i.code == "config.invalid" && i.message.contains("which no config declares"))
        .filter(|i| i.id.as_ref().map(|x| x == id || x.0.starts_with(&format!("{}/", id.0))).unwrap_or(false))
        .count()
}

/// Fleets named by load errors that are not in the snapshot (core lists a
/// fleet whose fleet.toml does not parse with the error on `Fleet.issues`;
/// this catches an error that names a fleet core left out).
pub fn broken_fleets(snap: &Snapshot) -> Vec<nimitz_core::Id> {
    let mut out: Vec<nimitz_core::Id> = vec![];
    for i in all_issues(snap).into_iter().filter(|i| is_load_error(i)) {
        let Some(id) = &i.id else { continue };
        let f = nimitz_core::Id::admiral(id.fleet());
        if !snap.fleets.iter().any(|x| x.id == f) && !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

/// Squadrons of `f` named by load errors that the fleet does not have.
fn broken_squadrons(snap: &Snapshot, f: &Fleet) -> Vec<nimitz_core::Id> {
    let mut out: Vec<nimitz_core::Id> = vec![];
    for i in snap.issues.iter().chain(f.issues.iter()).filter(|i| is_load_error(i)) {
        let Some(id) = &i.id else { continue };
        let Some(sq) = id.squadron() else { continue };
        let sid = nimitz_core::Id::commodore(id.fleet(), sq);
        if id.fleet() == f.id.fleet() && !f.squadrons.iter().any(|s| s.id == sid) && !out.contains(&sid) {
            out.push(sid);
        }
    }
    out
}

fn note(id: &nimitz_core::Id, k: usize, text: String, style: Style) -> Node {
    let mut c = Node::new(format!("N:{}:{k}", id.0), Kind::Note);
    c.member = Some(id.clone());
    c.name = text;
    c.name_style = style;
    c
}

/// Note rows with config errors in full, line by line, and their fixes.
fn error_notes(id: &nimitz_core::Id, errors: &[&Issue]) -> Vec<Node> {
    let mut out = vec![];
    for e in errors {
        for line in e.message.lines().filter(|l| !l.trim().is_empty()) {
            out.push(note(id, out.len(), line.to_string(), st(mono::FG)));
        }
        if let Some(fx) = &e.fix {
            out.push(note(id, out.len(), format!("fix: {fx}"), bold(mono::ACCENT)));
        }
    }
    out
}

/// A fleet or squadron whose config does not load: a purple frame with the
/// error in full, line by line, and how many sessions now run outside it.
fn broken_band(snap: &Snapshot, id: &nimitz_core::Id, v: &View) -> Node {
    let fleet = id.role() == Role::Admiral;
    let mut n = Node::new(format!("{}:{}", if fleet { "F" } else { "SQ" }, id.0), if fleet { Kind::FleetBand } else { Kind::SquadronBand });
    n.member = Some(id.clone());
    n.default_expand = true;
    n.icon = if fleet { v.icons.fleet } else { v.icons.squadron };
    n.icon_style = bold(mono::ERROR);
    n.name = if fleet { format!("[{}]", id.0) } else { id.squadron().unwrap_or("?").to_string() };
    n.name_style = bold(mono::ERROR);
    n.kind_text = if fleet { "fleet".into() } else { "squadron".into() };
    n.frame = Some(mono::ERROR);
    n.band_bg = Some(mono::BG_HI);
    n.worst = Some(Severity::Error);
    n.detail = vec![Span::styled("config does not load", bold(mono::ERROR))];
    let errors: Vec<&Issue> = all_issues(snap)
        .into_iter()
        .filter(|i| is_load_error(i) && i.id.as_ref().map(|x| x == id || (fleet && x.fleet() == id.fleet())).unwrap_or(false))
        .collect();
    n.issue_count = errors.len();
    n.counts = vec![Count { icon: v.icons.issue, n: errors.len(), style: bold(mono::ERROR), key: true }];
    n.children = error_notes(id, &errors);
    let unlisted = unlisted_sessions(snap, id);
    if unlisted > 0 {
        let k = n.children.len();
        n.children.push(note(id, k, 
            format!("{unlisted} session(s) labelled for {} keep running outside the config (i lists them)", id.0),
            bold(mono::WARN),
        ));
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
    // a name filter searches every fleet; otherwise the tab's fleet only
    let on_tab = |fleet: &str| !q.is_empty() || v.tab.map(|t| t == fleet).unwrap_or(true);
    let mut out = vec![];
    for f in snap.fleets.iter().filter(|f| on_tab(f.id.fleet())) {
        let state = FleetState::of(f);
        // The `f` state filter works on squadrons: a tab shows one fleet, so
        // hiding the fleet itself would only blank the tab (BUG: squadron filter).
        let shown_any = fleet_members(f).iter().any(|m| matches(m));
        if (!q.is_empty() || v.flagged_only) && !shown_any {
            continue;
        }
        let issues: Vec<&Issue> = fleet_issues(f);
        // a fleet.toml that does not load: the error frame, the error in full above its members
        let load_errors: Vec<&Issue> = f.issues.iter().filter(|i| is_load_error(i) && i.id.as_ref() == Some(&f.id)).collect();
        let broken = !load_errors.is_empty();
        let mut n = Node::new(format!("F:{}", f.id.0), Kind::FleetBand);
        n.member = Some(f.id.clone());
        n.default_expand = f.machine.is_some() || broken;
        n.icon = v.icons.fleet;
        n.icon_style = bold(if broken { mono::ERROR } else { mono::STRONG });
        n.name = format!("[{}]", f.id.0);
        n.name_style = bold(if broken { mono::ERROR } else { mono::STRONG });
        n.kind_text = "fleet".into();
        n.frame = Some(if broken { mono::ERROR } else { state.frame() });
        n.band_bg = Some(if broken { mono::BG_HI } else { state.tint() });
        n.worst = worst_of(&f.issues);
        n.issue_count = f.issues.len();
        let place = f.machine.clone().unwrap_or_else(|| "not on this machine".into());
        n.detail = if broken {
            vec![Span::styled("config does not load", bold(mono::ERROR)), Span::styled(format!("   {}", f.dir), st(mono::SOFT))]
        } else {
            vec![
                Span::styled(state.label().to_string(), bold(state.color())),
                Span::styled(format!("   {place}   {}", f.dir), st(mono::SOFT)),
            ]
        };
        n.counts = status_counts(&fleet_members(f), &issues, v.icons);
        n.children = error_notes(&f.id, &load_errors);
        if matches(&f.admiral) {
            n.children.push(member_node(&f.admiral, v));
        }
        let mut hidden = 0;
        for sq in &f.squadrons {
            if !v.fleet_states[FleetState::of_squadron(f, sq).index()] {
                hidden += 1;
                continue;
            }
            if let Some(sn) = squadron_node(f, sq, v, &matches) {
                n.children.push(sn);
            }
        }
        if hidden > 0 {
            let k = n.children.len();
            n.children.push(note(&f.id, k, format!("{hidden} squadron(s) hidden by the state filter (f changes it)"), st(mono::MUTED)));
        }
        // squadrons the config names but could not load: a broken frame each
        for sq in broken_squadrons(snap, f) {
            n.children.push(broken_band(snap, &sq, v));
        }
        out.push(n);
    }
    // fleets whose config does not load: core leaves them out; show them broken
    if q.is_empty() {
        for fid in broken_fleets(snap).into_iter().filter(|f| on_tab(f.fleet())) {
            out.push(broken_band(snap, &fid, v));
        }
    }
    // a fleet's tab lists the stowaways in its folders; UNASSIGNED the rest
    let stowaways: Vec<&SessionInfo> = snap.stowaways.iter().filter(|s| v.tab.map(|t| stowaway_tab(snap, s) == t).unwrap_or(true)).collect();
    if v.show_stowaways && !stowaways.is_empty() && q.is_empty() && !v.flagged_only {
        let live = stowaways.iter().filter(|s| s.pid.is_some()).count();
        let mut n = Node::new("STOW".into(), Kind::StowBand);
        n.default_expand = true;
        // a warning while one is live; nothing to shout about otherwise
        let hue = if live > 0 { mono::WARN } else { mono::MUTED };
        n.icon = v.icons.stowaway;
        n.icon_style = bold(hue);
        n.name = "STOWAWAYS".into();
        n.name_style = bold(hue);
        n.kind_text = "group".into();
        // its bar: yellow while a stowaway is live, else neutral
        n.frame = Some(if live > 0 { mono::WARN } else { mono::LINE });
        n.band_bg = Some(mono::BG_HI);
        let scope = match v.tab {
            Some(UNASSIGNED) => "sessions under the nimitz home, in no fleet's folders, with no labels — adopt or remove",
            Some(_) => "sessions in this fleet's folders with no labels — adopt or remove",
            None => "sessions in fleet scope with no labels — adopt or remove",
        };
        n.detail = vec![Span::styled(scope, st(mono::SOFT))];
        n.title_right = vec![Span::styled(format!("{live} live / {} total", stowaways.len()), bold(hue))];
        n.children = stowaways.iter().map(|s| stow_node(s, snap, v)).collect();
        out.push(n);
    }
    out
}

// ------------------------------------------------------------------ rows

/// Which frame a row lives in; the renderer draws borders from this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    /// a fleet's title band (one tinted line; no frame)
    FleetBand,
    /// a row under the fleet band (the admiral and its crew, in its heavy box)
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
