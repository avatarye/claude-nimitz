//! The TUI: fleet tree as a table on the left, detail of the selected row on
//! the right. Updates on its own: a loader thread re-reads every source each
//! interval and the screen redraws as soon as a snapshot lands (and once a
//! second so ages keep ticking). Key handling and drawing are separate from the
//! terminal so tests can drive `App` directly.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::actions::{self, Plan};
use crate::env::Env;
use crate::herdr::{self, Pane};
use crate::model::{Agent, Snapshot, Status, SubAgent};
use crate::present::{age, agent_age, fleet_summary, glyph, short_model, word};
use crate::sources::Cache;

// ------------------------------------------------------------------ styles and icons

/// Monokai palette.
pub mod mono {
    use ratatui::style::Color;
    pub const BG: Color = Color::Rgb(0x27, 0x28, 0x22);
    pub const BG_HI: Color = Color::Rgb(0x3e, 0x3d, 0x32); // line highlight
    pub const SEL: Color = Color::Rgb(0x5b, 0x59, 0x4a); // cursor row
    pub const FG: Color = Color::Rgb(0xf8, 0xf8, 0xf2);
    pub const COMMENT: Color = Color::Rgb(0x75, 0x71, 0x5e);
    pub const DIM: Color = Color::Rgb(0x5a, 0x58, 0x4b);
    pub const SOFT: Color = Color::Rgb(0xcf, 0xcf, 0xc2);
    pub const PINK: Color = Color::Rgb(0xf9, 0x26, 0x72);
    pub const ORANGE: Color = Color::Rgb(0xfd, 0x97, 0x1f);
    pub const YELLOW: Color = Color::Rgb(0xe6, 0xdb, 0x74);
    pub const GREEN: Color = Color::Rgb(0xa6, 0xe2, 0x2e);
    pub const CYAN: Color = Color::Rgb(0x66, 0xd9, 0xef);
    pub const PURPLE: Color = Color::Rgb(0xae, 0x81, 0xff);
    // dark tints for fleet-row backgrounds
    pub const T_PINK: Color = Color::Rgb(0x55, 0x1e, 0x35);
    pub const T_GREEN: Color = Color::Rgb(0x31, 0x40, 0x16);
    pub const T_CYAN: Color = Color::Rgb(0x1d, 0x3a, 0x42);
    pub const T_ORANGE: Color = Color::Rgb(0x4d, 0x33, 0x12);
    pub const T_GREY: Color = Color::Rgb(0x36, 0x36, 0x2f);
    pub const T_DIM: Color = Color::Rgb(0x2d, 0x2e, 0x28);
    pub const GOLD: Color = Color::Rgb(0xff, 0xc6, 0x3a);
    pub const T_GOLD: Color = Color::Rgb(0x4a, 0x3c, 0x14);
    pub const T_PURPLE: Color = Color::Rgb(0x38, 0x2a, 0x55);
}

/// Old xterm-grey call sites mapped onto Monokai shades.
pub fn grey(n: u8) -> Color {
    match n {
        0..=236 => mono::BG_HI,
        237..=242 => mono::DIM,
        243..=247 => mono::COMMENT,
        _ => mono::SOFT,
    }
}
fn st(c: Color) -> Style {
    Style::new().fg(c)
}
fn bold(c: Color) -> Style {
    Style::new().fg(c).add_modifier(Modifier::BOLD)
}
pub fn status_style(s: Status) -> Style {
    match s {
        Status::NeedsYou => bold(mono::PINK),
        Status::Busy => st(mono::GREEN),
        Status::Idle => st(mono::YELLOW),
        Status::Stopped => st(mono::COMMENT),
        Status::NotRunning => st(mono::DIM),
    }
}

/// Nerd Font glyphs (JetBrainsMono Nerd Font); ADMIRALTY_WATCH_ICONS=ascii swaps them out.
pub struct Icons {
    pub fleet: &'static str,
    pub fleet_error: &'static str,
    pub lead: &'static str,
    pub agent: &'static str,
    pub teammate: &'static str,
    pub sub: &'static str,
    pub finished: &'static str,
    pub stale: &'static str,
    pub row: &'static str,
    pub others: &'static str,
    pub completed: &'static str,
    pub in_progress: &'static str,
    pub pending: &'static str,
    pub pane: &'static str,
    pub hub: &'static str,
    pub s_needs: &'static str,
    pub s_busy: &'static str,
    pub s_idle: &'static str,
    pub s_stopped: &'static str,
    pub s_off: &'static str,
}

pub const NERD: Icons = Icons {
    fleet: "\u{f21a}",
    fleet_error: "\u{f071}",
    lead: "\u{f01a5}",
    agent: "\u{f06a9}",
    teammate: "\u{f0c0}",
    sub: "\u{f477}",
    finished: "\u{f187}",
    stale: "\u{f1da}",
    row: "\u{f28d}",
    others: "\u{f059}",
    completed: "\u{f058}",
    in_progress: "\u{f110}",
    pending: "\u{f10c}",
    pane: "\u{f120}",
    hub: "\u{f13d}",
    s_needs: "\u{f0f3}",
    s_busy: "\u{f0e7}",
    s_idle: "\u{f186}",
    s_stopped: "\u{f28d}",
    s_off: "\u{f10c}",
};
pub const ASCII: Icons = Icons {
    fleet: "#",
    fleet_error: "!",
    lead: "L",
    agent: "A",
    teammate: "T",
    sub: "s",
    finished: "+",
    stale: "z",
    row: "-",
    others: "?",
    completed: "v",
    in_progress: "~",
    pending: ".",
    pane: "P",
    hub: "H",
    s_needs: "!",
    s_busy: "*",
    s_idle: "-",
    s_stopped: "x",
    s_off: "o",
};

// Columns: NAME carries a 2-column Herdr-pane gutter and then the tree
// indentation; the rest line up on every row. Each grows with the table's width
// from its minimum up to its maximum, in this order: the small fixed ones,
// then NAME, then DETAIL.
//                 NAME STATUS AGE KIND DETAIL
const COL_MIN: [u16; 5] = [24, 11, 4, 10, 0];
const COL_MAX: [u16; 5] = [58, 14, 5, 10, 64];
const GROW_ORDER: [usize; 5] = [1, 2, 3, 0, 4];
const PANE_GUTTER: usize = 2; // Herdr status dot + space
const HL_SYMBOL: &str = "\u{258c}"; // left bar on the cursor row

/// Column widths for a table whose inner width is `w`.
pub fn col_widths(w: u16) -> [u16; 5] {
    let gaps = (COL_MIN.len() as u16 - 1) + HL_SYMBOL.width() as u16;
    let mut out = COL_MIN;
    let mut extra = w.saturating_sub(COL_MIN.iter().sum::<u16>() + gaps);
    for i in GROW_ORDER {
        let add = (COL_MAX[i] - out[i]).min(extra);
        out[i] += add;
        extra -= add;
    }
    out
}

/// Widest table worth giving space to (all columns at their maximum).
pub fn table_max_width() -> u16 {
    COL_MAX.iter().sum::<u16>() + (COL_MAX.len() as u16 - 1) + HL_SYMBOL.width() as u16 + 2
}

// ------------------------------------------------------------------ tree specs

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Fleet,
    Agent,
    Sub,
    Finished,
    Stale,
    StaleRow,
    Task,
    Others,
    Hub,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: String,
    pub kind: Kind,
    pub agent: Option<String>,
    pub fleet: Option<String>,
    pub sub: Option<String>,
    pub default_expand: bool,
    pub icon: &'static str,
    pub icon_style: Style,
    pub name: String,
    pub name_style: Style,
    pub status: (String, Style),
    pub age: String,
    pub kind_text: String,
    pub pane: bool,
    pub row_bg: Option<Color>,
    /// frame colour for a top-level section (hub, fleet, OTHER)
    pub frame: Option<Color>,
    /// right side of a section's title bar (agent counts as icon + number pairs)
    pub title_right: Vec<Span<'static>>,
    /// Herdr's state for the pane hosting this row's session
    pub pane_status: Option<String>,
    pub detail: Vec<Span<'static>>,
    pub children: Vec<Node>,
}

impl Node {
    fn new(id: String, kind: Kind) -> Node {
        Node {
            id,
            kind,
            agent: None,
            fleet: None,
            sub: None,
            default_expand: false,
            icon: "",
            icon_style: Style::new(),
            name: String::new(),
            name_style: Style::new(),
            status: (String::new(), Style::new()),
            age: String::new(),
            kind_text: String::new(),
            pane: false,
            row_bg: None,
            frame: None,
            title_right: vec![],
            pane_status: None,
            detail: vec![],
            children: vec![],
        }
    }
}

/// One visible line of the flattened tree.
#[derive(Clone, Debug)]
pub struct Row {
    pub node: Node,
    pub depth: usize,
    pub prefix: String, // tree guides
    pub expanded: bool,
}

fn agent_kind(a: &Agent) -> &'static str {
    if a.fleet == crate::model::HUB_TAG {
        "admiralty"
    } else if a.is_lead {
        "lead"
    } else {
        "agent"
    }
}

fn flags_spans(a: &Agent) -> Vec<Span<'static>> {
    let mut v = vec![];
    let running = a.running_subagents();
    if running > 0 {
        v.push(Span::styled(format!("{running} running  "), st(mono::GREEN)));
    }
    if !a.stale.is_empty() {
        v.push(Span::styled(format!("+{} stale  ", a.stale.len()), st(grey(244))));
    }
    for f in &a.flags {
        let c = if *f == crate::model::Flag::DuplicateLive { mono::PINK } else { mono::PURPLE };
        v.push(Span::styled(format!("{}  ", f.text()), st(c)));
    }
    v
}

fn sub_node(akey: &str, s: &SubAgent, now: f64, icons: &Icons) -> Node {
    let r = &s.rec;
    let style = if s.running { st(mono::GREEN) } else { st(grey(246)) };
    let mut n = Node::new(format!("s:{akey}:{}", r.agent_id), Kind::Sub);
    n.agent = Some(akey.into());
    n.sub = Some(r.agent_id.clone());
    n.default_expand = s.running && !r.teammate;
    n.icon = if r.teammate { icons.teammate } else { icons.sub };
    n.icon_style = style;
    n.name = r.label.clone();
    n.name_style = style;
    n.status = (if s.running { "> running".into() } else { ". finished".into() }, style);
    n.age = age(now - r.last_write);
    n.kind_text = "sub-agent".into();
    let model = short_model(r.model.as_deref());
    n.detail = vec![Span::styled(
        [r.agent_type.clone().unwrap_or_default(), model].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" "),
        st(grey(244)),
    )];
    n.children = sub_nodes(akey, &s.children, &r.agent_id, now, icons);
    n
}

fn sub_nodes(akey: &str, nodes: &[SubAgent], parent: &str, now: f64, icons: &Icons) -> Vec<Node> {
    let mut out: Vec<Node> = nodes.iter().filter(|s| s.running).map(|s| sub_node(akey, s, now, icons)).collect();
    // finished sub-agents are listed for an hour after their last write
    let done: Vec<&SubAgent> = nodes.iter().filter(|s| !s.running && now - s.rec.last_write < FINISHED_WINDOW_S).collect();
    if !done.is_empty() {
        let mut g = Node::new(format!("fin:{akey}:{parent}"), Kind::Finished);
        g.agent = Some(akey.into());
        g.icon = icons.finished;
        g.icon_style = st(grey(244));
        g.name = format!("{} finished in the last hour", done.len());
        g.name_style = st(grey(244));
        g.kind_text = "group".into();
        g.children = done.iter().map(|s| sub_node(akey, s, now, icons)).collect();
        out.push(g);
    }
    out
}

fn agent_node(a: &Agent, now: f64, panes: &HashMap<String, Pane>, icons: &Icons) -> Node {
    let pane = panes.get(&a.key);
    let has_pane = pane.is_some();
    let style = status_style(a.status);
    let name_style = if a.is_lead && a.live() { bold(mono::CYAN) } else { style };
    let mut n = Node::new(format!("a:{}", a.key), Kind::Agent);
    n.agent = Some(a.key.clone());
    n.fleet = Some(a.fleet.clone());
    n.default_expand = a.live() && a.subagents.iter().any(|s| s.running);
    n.icon = if a.is_lead { icons.lead } else { icons.agent };
    n.icon_style = name_style;
    n.name = a.name.clone();
    n.name_style = name_style;
    n.status = (format!("{} {}", glyph(a.status), word(a.status)), style);
    n.age = agent_age(a, now);
    n.kind_text = agent_kind(a).into();
    n.pane = has_pane;
    n.pane_status = pane.map(|p| p.status.clone()).filter(|s| !s.is_empty());
    n.detail = flags_spans(a);
    let mut kids: Vec<Node> = a
        .helpers
        .iter()
        .map(|h| {
            let mut k = agent_node(h, now, panes, icons);
            k.kind_text = "sub-agent".into();
            k.detail.insert(0, Span::styled("background session  ", st(mono::COMMENT)));
            k
        })
        .collect();
    kids.extend(sub_nodes(&a.key, &a.subagents, "root", now, icons));
    for t in &a.tasks {
        let (icon, c) = match t.status.as_str() {
            "completed" => (icons.completed, grey(244)),
            "in_progress" => (icons.in_progress, mono::GREEN),
            _ => (icons.pending, mono::YELLOW),
        };
        let mut k = Node::new(format!("t:{}:{}", a.key, t.id), Kind::Task);
        k.agent = Some(a.key.clone());
        k.icon = icon;
        k.icon_style = st(c);
        k.name = t.subject.clone();
        k.status = (t.status.replace('_', " "), st(c));
        k.kind_text = "task".into();
        k.detail = vec![Span::styled(t.owner.clone().unwrap_or_default(), st(grey(244)))];
        kids.push(k);
    }
    if !a.stale.is_empty() {
        let mut g = Node::new(format!("stale:{}", a.key), Kind::Stale);
        g.agent = Some(a.key.clone());
        g.icon = icons.stale;
        g.icon_style = st(grey(244));
        g.name = format!("{} stale sessions", a.stale.len());
        g.name_style = st(grey(244));
        g.kind_text = "group".into();
        for r in &a.stale {
            let mut k = Node::new(format!("st:{}:{}", a.key, r.session_id), Kind::StaleRow);
            k.agent = Some(a.key.clone());
            k.icon = icons.row;
            k.icon_style = st(grey(242));
            k.name = r.name.clone();
            k.name_style = st(grey(244));
            k.status = (if r.live() { "live".into() } else { r.state.clone().unwrap_or("dead".into()) }, st(grey(244)));
            k.age = age(now - r.started_at.unwrap_or(now));
            k.kind_text = "agent".into();
            let id = r.short_id.clone().unwrap_or(r.session_id.chars().take(8).collect());
            k.detail = vec![Span::styled(format!("{id}  {}  {}", r.kind, r.cwd), st(grey(242)))];
            g.children.push(k);
        }
        kids.push(g);
    }
    n.children = kids;
    n
}

/// A fleet's overall state: drives its row tint and the `f` filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FleetState {
    NeedsYou,
    Busy,
    Idle,
    NoLead,
    /// on this machine, nothing live
    Inactive,
    Elsewhere,
    Unreadable,
}

impl FleetState {
    pub const ALL: [FleetState; 7] = [
        FleetState::NeedsYou,
        FleetState::Busy,
        FleetState::Idle,
        FleetState::NoLead,
        FleetState::Inactive,
        FleetState::Elsewhere,
        FleetState::Unreadable,
    ];

    pub fn of(f: &crate::model::Fleet) -> FleetState {
        if f.error.is_some() {
            FleetState::Unreadable
        } else if !f.on_machine() {
            FleetState::Elsewhere
        } else if f.need_count() > 0 {
            FleetState::NeedsYou
        } else if f.live_count() == 0 {
            FleetState::Inactive
        } else if !f.lead().map(|l| l.live()).unwrap_or(false) {
            FleetState::NoLead
        } else if f.busy_count() > 0 {
            FleetState::Busy
        } else {
            FleetState::Idle
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            FleetState::NeedsYou => "needs you",
            FleetState::Busy => "busy",
            FleetState::Idle => "idle",
            FleetState::NoLead => "no lead",
            FleetState::Inactive => "inactive",
            FleetState::Elsewhere => "elsewhere",
            FleetState::Unreadable => "unreadable",
        }
    }
    pub fn tint(self) -> Color {
        match self {
            FleetState::NeedsYou => mono::T_PINK,
            FleetState::Busy => mono::T_GREEN,
            FleetState::Idle => mono::T_CYAN,
            FleetState::NoLead => mono::T_ORANGE,
            FleetState::Inactive => mono::T_GREY,
            FleetState::Elsewhere => mono::T_DIM,
            FleetState::Unreadable => mono::T_PURPLE,
        }
    }
    /// bright colour for the fleet's frame and title
    pub fn color(self) -> Color {
        match self {
            FleetState::NeedsYou => mono::PINK,
            FleetState::Busy => mono::GREEN,
            FleetState::Idle => mono::CYAN,
            FleetState::NoLead => mono::ORANGE,
            FleetState::Inactive => mono::COMMENT,
            FleetState::Elsewhere => mono::DIM,
            FleetState::Unreadable => mono::PURPLE,
        }
    }
    pub fn index(self) -> usize {
        FleetState::ALL.iter().position(|x| *x == self).unwrap()
    }
}

pub struct View<'a> {
    pub filter: &'a str,
    pub show_others: bool,
    pub flagged_only: bool,
    /// fleet states to show, indexed like FleetState::ALL
    pub fleet_states: [bool; 7],
    pub panes: &'a HashMap<String, Pane>,
    pub icons: &'a Icons,
}

pub fn build_nodes(snap: &Snapshot, v: &View) -> Vec<Node> {
    let now = snap.taken_at;
    let q = v.filter.trim().to_lowercase();
    let matches = |a: &Agent| -> bool {
        if v.flagged_only && a.flags.is_empty() && a.status != Status::NeedsYou {
            return false;
        }
        q.is_empty()
            || a.name.to_lowercase().contains(&q)
            || a.fleet.to_lowercase().contains(&q)
            || a.all_subagents().iter().any(|s| s.rec.label.to_lowercase().contains(&q))
    };
    let mut out = vec![];
    for f in &snap.fleets {
        let agents: Vec<&Agent> = f.agents.iter().filter(|a| matches(a)).collect();
        let state = FleetState::of(f);
        if !v.fleet_states[state.index()] {
            continue;
        }
        if (!q.is_empty() || v.flagged_only) && agents.is_empty() {
            continue;
        }
        let mut n = Node::new(format!("f:{}", f.tag), Kind::Fleet);
        n.fleet = Some(f.tag.clone());
        n.default_expand = f.on_machine() && !agents.is_empty();
        let summary = fleet_summary(f);
        n.row_bg = Some(state.tint());
        n.frame = Some(state.color());
        n.title_right = status_counts(f.agents.iter().collect::<Vec<_>>().as_slice(), v.icons);
        if f.error.is_some() {
            n.icon = v.icons.fleet_error;
            n.icon_style = st(mono::PINK);
            n.name_style = st(grey(242));
            n.status = ("unreadable".into(), st(mono::PINK));
            n.detail = vec![Span::styled(summary, st(grey(246)))];
        } else if !f.on_machine() {
            n.icon = v.icons.fleet;
            n.icon_style = st(grey(242));
            n.name_style = st(grey(242));
            n.status = ("elsewhere".into(), st(grey(242)));
            n.detail = vec![Span::styled(summary, st(grey(242)))];
        } else {
            let s = if f.need_count() > 0 { bold(mono::PINK) } else { bold(mono::FG) };
            n.icon = v.icons.fleet;
            n.icon_style = s;
            n.name_style = s;
            n.status = (format!("{} live", f.live_count()), st(if f.live_count() > 0 { mono::GREEN } else { grey(246) }));
            // the title shows counts as icons; keep only the notes that are not counts
            let notes: Vec<&str> = summary
                .split("   ")
                .filter(|p| !p.ends_with(" live") && !p.ends_with(" busy") && !p.ends_with(" need you") && !p.ends_with(" stopped"))
                .collect();
            n.detail = vec![Span::styled(notes.join("   "), st(mono::SOFT))];
        }
        n.name = format!("[{}]", f.tag);
        n.kind_text = "fleet".into();
        n.children = agents.iter().map(|a| agent_node(a, now, v.panes, v.icons)).collect();
        out.push(n);
    }
    if v.show_others {
        let others: Vec<&Agent> = snap.others.iter().filter(|a| matches(a)).collect();
        let mut n = Node::new("others".into(), Kind::Others);
        n.default_expand = true;
        n.icon = v.icons.others;
        n.icon_style = bold(grey(249));
        n.name = "OTHER".into();
        n.name_style = bold(grey(249));
        n.status = (format!("{} sessions", others.len()), st(grey(246)));
        n.kind_text = "group".into();
        n.detail = vec![Span::styled("outside any fleet", st(grey(244)))];
        n.frame = Some(mono::SOFT);
        n.title_right = status_counts(&others, v.icons);
        n.children = others.iter().map(|a| agent_node(a, now, v.panes, v.icons)).collect();
        out.push(n);
    }
    // the hub gets its own frame on top (its sub-agents and tasks inside), then one per fleet
    let hub = &snap.hub_agent;
    let mut root = agent_node(hub, now, v.panes, v.icons);
    root.id = "hub".into();
    root.kind = Kind::Hub;
    root.default_expand = true;
    root.icon = v.icons.hub;
    root.icon_style = bold(mono::ORANGE);
    root.name_style = bold(mono::ORANGE);
    root.row_bg = Some(mono::T_GOLD);
    root.frame = Some(mono::GOLD);
    root.icon_style = bold(mono::GOLD);
    root.name_style = bold(mono::GOLD);
    if hub.primary.is_none() {
        root.status = ("o not running".into(), st(mono::DIM));
    }
    let mut all = vec![root];
    all.extend(out);
    all
}

/// Flatten the visible part of the tree, Textual-style guides included.
pub fn flatten(nodes: &[Node], expanded: &HashMap<String, bool>) -> Vec<Row> {
    fn walk(out: &mut Vec<Row>, nodes: &[Node], depth: usize, lead: &str, exp: &HashMap<String, bool>) {
        let n = nodes.len();
        for (i, node) in nodes.iter().enumerate() {
            let last = i + 1 == n;
            let prefix = if depth == 0 { String::new() } else { format!("{lead}{}", if last { "└─ " } else { "├─ " }) };
            let open = !node.children.is_empty() && *exp.get(&node.id).unwrap_or(&node.default_expand);
            out.push(Row { node: node.clone(), depth, prefix, expanded: open });
            if open {
                let next = if depth == 0 { String::new() } else { format!("{lead}{}", if last { "   " } else { "│  " }) };
                walk(out, &node.children, depth + 1, &next, exp);
            }
        }
    }
    let mut out = vec![];
    walk(&mut out, nodes, 0, "", expanded);
    out
}

fn truncate(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    let mut out = String::new();
    for ch in s.chars() {
        if out.width() + 1 >= w {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

fn pad(s: &str, w: usize) -> String {
    let n = s.width();
    if n >= w { s.to_string() } else { format!("{s}{}", " ".repeat(w - n)) }
}

/// NAME cell: Herdr-pane gutter, tree guides, expand marker, icon, name.
pub fn name_cell(r: &Row, width: usize) -> Line<'static> {
    let marker = if r.node.children.is_empty() {
        if r.node.kind == Kind::Fleet { Span::styled("▶ ", st(mono::DIM)) } else { Span::raw("  ") }
    } else {
        Span::raw(if r.expanded { "▼ " } else { "▶ " })
    };
    let room = width.saturating_sub(PANE_GUTTER + r.prefix.width() + 2 + 2).max(4);
    // Herdr's status dot doubles as the "has a Herdr pane" marker
    let gutter = if r.node.pane {
        let state = r.node.pane_status.as_deref().unwrap_or("unknown");
        vec![Span::styled(format!("{} ", herdr::dot(state)), bold(herdr_color(state)))]
    } else {
        vec![Span::raw("  ")]
    };
    let mut parts = gutter;
    parts.extend(vec![
        Span::styled(r.prefix.clone(), st(grey(244))),
        marker,
        Span::styled(format!("{} ", r.node.icon), r.node.icon_style),
        Span::styled(pad(&truncate(&r.node.name, room), room), r.node.name_style),
    ]);
    Line::from(parts)
}

/// `icon n / icon n / ...` for the agents of a section, zero counts left out.
pub fn status_counts(agents: &[&Agent], icons: &Icons) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = vec![];
    for (status, icon) in [
        (Status::NeedsYou, icons.s_needs),
        (Status::Busy, icons.s_busy),
        (Status::Idle, icons.s_idle),
        (Status::Stopped, icons.s_stopped),
        (Status::NotRunning, icons.s_off),
    ] {
        let n = agents.iter().filter(|a| a.status == status).count();
        if n == 0 {
            continue;
        }
        if !out.is_empty() {
            out.push(Span::styled(" / ", st(mono::COMMENT)));
        }
        out.push(Span::styled(format!("{icon} {n}"), status_style(status).add_modifier(Modifier::BOLD)));
    }
    out
}

/// A row inside a frame: NAME, STATUS, AGE, KIND, DETAIL, padded to the shared column widths.
pub fn row_spans(r: &Row, w: &[u16; 5], inner: usize) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = vec![];
    let mut used = 0usize;
    let mut push = |out: &mut Vec<Span<'static>>, text: String, style: Style| {
        used += text.width();
        out.push(Span::styled(text, style));
    };
    for sp in name_cell(r, w[0] as usize).spans {
        push(&mut out, sp.content.to_string(), sp.style);
    }
    push(&mut out, " ".into(), Style::new());
    push(&mut out, pad(&truncate(&r.node.status.0, w[1] as usize), w[1] as usize), r.node.status.1);
    push(&mut out, " ".into(), Style::new());
    push(&mut out, format!("{:>width$}", truncate(&r.node.age, w[2] as usize), width = w[2] as usize), st(mono::COMMENT));
    push(&mut out, " ".into(), Style::new());
    push(&mut out, pad(&truncate(&r.node.kind_text, w[3] as usize), w[3] as usize), st(mono::COMMENT));
    push(&mut out, " ".into(), Style::new());
    let room = inner.saturating_sub(used);
    let mut left = room;
    for sp in &r.node.detail {
        if left == 0 {
            break;
        }
        let t = truncate(&sp.content, left);
        left = left.saturating_sub(t.width());
        out.push(Span::styled(t, sp.style));
    }
    if left > 0 {
        out.push(Span::raw(" ".repeat(left)));
    }
    out
}

/// How long a finished sub-agent stays listed.
pub const FINISHED_WINDOW_S: f64 = 3600.0;

/// Large ASCII icons for the detail pane, one per row glyph (5 lines each).
const ART_ANCHOR: [&str; 5] = ["    _O_", "     |", "  \\  |  /", "   '-+-'", "  ~~~~~~~"];
const ART_SHIP: [&str; 5] = ["     |\\", "     | \\", "     |__\\", "  ___|_____", "  \\_______/"];
const ART_CROWN: [&str; 5] = ["  .  ^  .", " /\\ / \\ /\\", " \\  V V  /", "  \\_____/", "  [_____]"];
const ART_ROBOT: [&str; 5] = ["    _||_", "   [o  o]", "   | -- |", "  /|____|\\", "    |  |"];
const ART_BOT: [&str; 5] = ["", "   .---.", "  ( o o )", "   | = |", "   '---'"];
const ART_TEAM: [&str; 5] = ["", "  o    o", " /|\\  /|\\", " / \\  / \\", ""];
const ART_TASK_DONE: [&str; 5] = ["  +------+", "  |    / |", "  | \\ /  |", "  |  V   |", "  +------+"];
const ART_TASK_WIP: [&str; 5] = ["  +------+", "  | //// |", "  | //   |", "  |      |", "  +------+"];
const ART_TASK_TODO: [&str; 5] = ["  +------+", "  |      |", "  |      |", "  |      |", "  +------+"];
const ART_ARCHIVE: [&str; 5] = ["  ________", " |________|", "  | ---- |", "  |      |", "  |______|"];
const ART_CLOCK: [&str; 5] = ["   .---.", "  / 12  \\", " |   |_  |", "  \\     /", "   '---'"];
const ART_OTHER: [&str; 5] = ["   ___", "  (__ \\", "    / /", "   |_|", "    o"];

/// The big icon matching a row's glyph.
pub fn big_icon(n: &Node, icons: &Icons) -> Vec<&'static str> {
    let art: &[&str; 5] = match n.kind {
        Kind::Hub => &ART_ANCHOR,
        Kind::Fleet => &ART_SHIP,
        Kind::Others => &ART_OTHER,
        Kind::Finished => &ART_ARCHIVE,
        Kind::Stale | Kind::StaleRow => &ART_CLOCK,
        Kind::Task if n.icon == icons.completed => &ART_TASK_DONE,
        Kind::Task if n.icon == icons.in_progress => &ART_TASK_WIP,
        Kind::Task => &ART_TASK_TODO,
        Kind::Sub if n.icon == icons.teammate => &ART_TEAM,
        Kind::Sub => &ART_BOT,
        Kind::Agent if n.icon == icons.lead => &ART_CROWN,
        Kind::Agent => &ART_ROBOT,
    };
    art.to_vec()
}

/// Cut a line's spans to `w` columns.
fn truncate_spans(spans: Vec<Span<'static>>, w: usize) -> Vec<Span<'static>> {
    let mut out = vec![];
    let mut left = w;
    for sp in spans {
        if left == 0 {
            break;
        }
        let t = truncate(&sp.content, left);
        left = left.saturating_sub(t.width());
        out.push(Span::styled(t, sp.style));
    }
    out
}

/// Herdr's sidebar colours, mapped onto Monokai.
pub fn herdr_color(state: &str) -> Color {
    match state {
        "working" => mono::YELLOW,
        "blocked" => mono::PINK,
        "done" => mono::CYAN,
        "idle" => mono::GREEN,
        _ => mono::COMMENT,
    }
}

// ------------------------------------------------------------------ app state

pub enum Modal {
    /// `f`: choose which fleet states are shown
    FleetFilter { sel: usize },
    /// typing a message; `enter` sends it, `esc` cancels
    Message { plan: actions::MessagePlan, text: String },
    Confirm(Plan),
    Needs { items: Vec<(String, String, Status, String, String)>, sel: usize },
    Help,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    /// side by side when the window is wide enough, else stacked
    Auto,
    /// tree left, detail right (vim `v`)
    Vertical,
    /// tree above, detail below (vim `s`)
    Horizontal,
}

pub enum Effect {
    Quit,
    Refresh,
    RunDetached(Plan),
    RunForeground(Plan),
}

pub struct App {
    pub env: Env,
    pub snap: Option<Snapshot>,
    pub panes: HashMap<String, Pane>,
    pub rows: Vec<Row>,
    pub cursor: usize,
    /// first visible line of the frame list
    pub scroll: usize,
    pub expanded: HashMap<String, bool>,
    pub filter: String,
    pub editing_filter: bool,
    pub show_others: bool,
    pub flagged_only: bool,
    pub fleet_states: [bool; 7],
    pub modal: Option<Modal>,
    pub toast: Option<(String, Style, Instant)>,
    pub in_tmux: bool,
    pub icons: &'static Icons,
    pub split: Split,
    last_g: Option<Instant>,
    page: usize,
    last_saved: Option<Saved>,
}

const HELP: &[(&str, &str)] = &[
    ("j / k", "down / up"),
    ("h / l", "fold or go to parent / unfold or go to first child"),
    ("gg / G", "top / bottom"),
    ("ctrl-d / ctrl-u", "half page down / up"),
    ("space", "expand / collapse"),
    ("/", "filter by name (enter keeps it, esc clears)"),
    ("n / N", "next / previous agent matching the filter"),
    ("!", "everything waiting on you"),
    ("enter", "jump to the agent's Herdr pane (if it has one)"),
    ("a", "activate: resume or cold-launch (confirms)"),
    ("J / K", "next / previous row with a live Herdr pane"),
    ("m", "message a live lead or agent (agents get it through their lead)"),
    ("A", "attach to a live background agent in a terminal (confirms)"),
    ("x", "delete a stopped session that is not a fleet agent (confirms)"),
    ("y", "yank the session id"),
    ("I", "toggle sessions outside any fleet"),
    ("f", "filter fleets by state (space toggles, a shows all)"),
    ("F", "toggle flagged-only"),
    ("v / s", "split: tree beside detail / tree above detail"),
    ("R", "refresh now"),
    ("?", "this help"),
    ("q", "quit"),
];

/// What sticks between runs: folds, filters, split.
#[derive(serde::Serialize, serde::Deserialize, Default, PartialEq, Clone)]
struct Saved {
    expanded: HashMap<String, bool>,
    filter: String,
    fleet_states: Vec<bool>,
    flagged_only: bool,
    show_others: bool,
    split: String,
}

impl App {
    fn saved(&self) -> Saved {
        Saved {
            expanded: self.expanded.clone(),
            filter: self.filter.clone(),
            fleet_states: self.fleet_states.to_vec(),
            flagged_only: self.flagged_only,
            show_others: self.show_others,
            split: match self.split {
                Split::Auto => "auto",
                Split::Vertical => "vertical",
                Split::Horizontal => "horizontal",
            }
            .into(),
        }
    }

    fn restore(&mut self) {
        let Some(s) = std::fs::read(&self.env.state_path).ok().and_then(|b| serde_json::from_slice::<Saved>(&b).ok()) else {
            return;
        };
        self.expanded = s.expanded.clone();
        self.filter = s.filter.clone();
        if s.fleet_states.len() == self.fleet_states.len() {
            self.fleet_states.copy_from_slice(&s.fleet_states);
        }
        self.flagged_only = s.flagged_only;
        self.show_others = s.show_others;
        self.split = match s.split.as_str() {
            "vertical" => Split::Vertical,
            "horizontal" => Split::Horizontal,
            _ => Split::Auto,
        };
        self.last_saved = Some(s);
    }

    /// Write the sticky state when it changed (atomically: temp file + rename).
    pub fn persist(&mut self) {
        let now = self.saved();
        if self.last_saved.as_ref() == Some(&now) {
            return;
        }
        let path = &self.env.state_path;
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(&now).unwrap_or_default()).is_ok() && std::fs::rename(&tmp, path).is_ok() {
            self.last_saved = Some(now);
        }
    }

    pub fn new(env: Env, in_tmux: bool) -> App {
        let ascii = std::env::var("ADMIRALTY_WATCH_ICONS").map(|v| v.eq_ignore_ascii_case("ascii")).unwrap_or(false);
        let mut app = App {
            env,
            snap: None,
            panes: HashMap::new(),
            rows: vec![],
            cursor: 0,
            scroll: 0,
            expanded: HashMap::new(),
            filter: String::new(),
            editing_filter: false,
            show_others: false,
            flagged_only: false,
            fleet_states: [true; 7],
            modal: None,
            toast: None,
            in_tmux,
            icons: if ascii { &ASCII } else { &NERD },
            split: Split::Auto,
            last_g: None,
            page: 20,
            last_saved: None,
        };
        app.restore();
        app
    }

    pub fn apply(&mut self, mut snap: Snapshot, panes: Vec<Pane>) {
        self.panes.clear();
        for a in snap.agents(true) {
            if let Some(p) = herdr::pane_for(a, &panes) {
                self.panes.insert(a.key.clone(), p.clone());
            }
        }
        // Herdr "blocked" = the session sits at an approval or question prompt: that is
        // waiting on you, and the only such signal for interactive sessions (leads)
        let blocked: HashSet<String> =
            self.panes.iter().filter(|(_, p)| p.status == "blocked").map(|(k, _)| k.clone()).collect();
        let mark = |a: &mut Agent| {
            if blocked.contains(&a.key) && a.live() {
                a.status = Status::NeedsYou;
                if a.job.needs.is_none() {
                    a.job.needs = Some("Waiting at an approval or question in its Herdr pane (Herdr reports blocked).".into());
                }
            }
        };
        mark(&mut snap.hub_agent);
        for f in &mut snap.fleets {
            f.agents.iter_mut().for_each(mark);
        }
        snap.others.iter_mut().for_each(mark);
        self.snap = Some(snap);
        self.rebuild();
    }

    pub fn rebuild(&mut self) {
        let keep = self.rows.get(self.cursor).map(|r| r.node.id.clone());
        let Some(snap) = &self.snap else { return };
        let view = View {
            filter: &self.filter,
            show_others: self.show_others,
            flagged_only: self.flagged_only,
            fleet_states: self.fleet_states,
            panes: &self.panes,
            icons: self.icons,
        };
        let nodes = build_nodes(snap, &view);
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

    pub fn current_id(&self) -> Option<String> {
        self.current().map(|r| r.node.id.clone())
    }

    pub fn selected_agent(&self) -> Option<&Agent> {
        let key = self.current()?.node.agent.clone()?;
        self.snap.as_ref()?.agent(&key)
    }

    fn notify(&mut self, msg: impl Into<String>, style: Style) {
        self.toast = Some((msg.into(), style, Instant::now()));
    }

    pub fn goto_id(&mut self, id: &str) -> bool {
        if let Some(i) = self.rows.iter().position(|r| r.node.id == id) {
            self.cursor = i;
            true
        } else {
            false
        }
    }

    fn set_expanded(&mut self, id: &str, open: bool) {
        self.expanded.insert(id.to_string(), open);
        self.rebuild();
    }

    fn fold(&mut self) {
        let Some(r) = self.current().cloned() else { return };
        if r.expanded {
            self.set_expanded(&r.node.id, false);
        } else if r.depth > 0 {
            if let Some(i) = (0..self.cursor).rev().find(|&i| self.rows[i].depth < r.depth) {
                self.cursor = i;
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

    fn step_match(&mut self, dir: i64) {
        let q = self.filter.trim().to_lowercase();
        let hits: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.node.kind == Kind::Agent && (q.is_empty() || r.node.name.to_lowercase().contains(&q)))
            .map(|(i, _)| i)
            .collect();
        if hits.is_empty() {
            return;
        }
        let c = self.cursor;
        self.cursor = if dir > 0 {
            *hits.iter().find(|&&i| i > c).unwrap_or(&hits[0])
        } else {
            *hits.iter().rev().find(|&&i| i < c).unwrap_or(hits.last().unwrap())
        };
    }

    /// Where `m` on the selected row would send a message.
    pub fn message_plan(&self) -> Option<actions::MessagePlan> {
        let a = self.selected_agent()?;
        let snap = self.snap.as_ref()?;
        let lead = snap.fleets.iter().find(|f| f.tag == a.fleet).and_then(|f| f.lead());
        let pane_of = |key: &str| self.panes.get(key).cloned();
        Some(actions::plan_message(&self.env, a, lead, &pane_of))
    }

    /// vim-style J / K: next or previous row hosted in a live Herdr pane.
    fn step_pane(&mut self, dir: i64) {
        let hits: Vec<usize> = self.rows.iter().enumerate().filter(|(_, r)| r.node.pane).map(|(i, _)| i).collect();
        if hits.is_empty() {
            self.notify("No row has a live Herdr pane.", st(mono::YELLOW));
            return;
        }
        let c = self.cursor;
        self.cursor = if dir > 0 {
            *hits.iter().find(|&&i| i > c).unwrap_or(&hits[0])
        } else {
            *hits.iter().rev().find(|&&i| i < c).unwrap_or(hits.last().unwrap())
        };
    }

    fn open_needs(&mut self) {
        let Some(snap) = &self.snap else { return };
        let now = snap.taken_at;
        let mut items = vec![];
        for (title, want) in [("live", Status::NeedsYou), ("stopped", Status::Stopped)] {
            for a in snap.agents(false).into_iter().filter(|a| a.status == want && a.job.needs.is_some()) {
                items.push((
                    a.key.clone(),
                    a.name.clone(),
                    a.status,
                    agent_age(a, now),
                    format!("{title}: {}", a.job.needs.clone().unwrap_or_default().replace('\n', " ")),
                ));
            }
        }
        self.modal = Some(Modal::Needs { items, sel: 0 });
    }

    fn jump_to_agent(&mut self, key: &str) {
        if let Some(snap) = &self.snap {
            if let Some(a) = snap.agent(key) {
                let fid = format!("f:{}", a.fleet);
                self.filter.clear();
                self.flagged_only = false;
                self.expanded.insert(fid, true);
            }
        }
        self.rebuild();
        self.goto_id(&format!("a:{key}"));
    }

    /// Jump to the selected agent's pane; Quit when it worked and `exit_after_jump` is set.
    fn herdr_jump(&mut self) -> Option<Effect> {
        let Some(a) = self.selected_agent() else {
            self.notify("Select an agent first.", st(mono::YELLOW));
            return None;
        };
        let name = a.name.clone();
        let live = a.live();
        let key = a.key.clone();
        let Some(bin) = self.env.herdr_bin.clone() else {
            self.notify("Not running inside Herdr (HERDR_ENV is not 1).", st(mono::YELLOW));
            return None;
        };
        let Some(pane) = self.panes.get(&key).cloned() else {
            let why = if live { "no Herdr pane hosts its session" } else { "it is not running" };
            self.notify(format!("No pane to jump to for {name}: {why}."), st(mono::YELLOW));
            return None;
        };
        let (rc, out) = herdr::jump(&bin, &pane);
        let plan = Plan {
            action: "jump".into(),
            agent: name,
            ok: true,
            summary: format!("focus Herdr pane {}", pane.pane_id),
            cmd: herdr::jump_cmds(&bin, &pane).into_iter().flat_map(|mut c| { c.push(";".into()); c }).collect(),
            cwd: None,
            details: vec![],
            more: vec![],
        };
        actions::log(&self.env, &plan, rc, &out);
        if rc != 0 {
            self.notify(format!("herdr could not focus {}: {out}", pane.pane_id), st(mono::PINK));
            return None;
        }
        self.env.exit_after_jump.then_some(Effect::Quit)
    }

    fn confirm(&mut self, plan: Plan) {
        if plan.ok {
            self.modal = Some(Modal::Confirm(plan));
        } else {
            let msg = format!("{}: not available. {}", plan.action, plan.summary);
            self.notify(msg, st(mono::YELLOW));
        }
    }

    /// Handle one key; returns work the terminal loop must do. Sticky state
    /// (folds, filters, split) is saved after every key that changed it.
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
        // modals first
        if let Some(m) = self.modal.take() {
            match m {
                Modal::FleetFilter { mut sel } => match k.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('f') | KeyCode::Char('q') => {}
                    KeyCode::Char('j') | KeyCode::Down => {
                        sel = (sel + 1).min(FleetState::ALL.len() - 1);
                        self.modal = Some(Modal::FleetFilter { sel });
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.modal = Some(Modal::FleetFilter { sel: sel.saturating_sub(1) });
                    }
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
                Modal::Message { plan, mut text } => match k.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter => {
                        if text.trim().is_empty() {
                            self.modal = Some(Modal::Message { plan, text });
                        } else {
                            let (rc, out) = actions::send_message(&self.env, &plan, text.trim());
                            if rc == 0 {
                                self.notify(format!("Sent {}", plan.summary), st(mono::GREEN));
                            } else {
                                let why = out.lines().last().unwrap_or("").to_string();
                                self.notify(format!("Not sent to {} (exit {rc}): {why}", plan.via), st(mono::PINK));
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        text.pop();
                        self.modal = Some(Modal::Message { plan, text });
                    }
                    KeyCode::Char('u') if ctrl => self.modal = Some(Modal::Message { plan, text: String::new() }),
                    KeyCode::Char(c) => {
                        text.push(c);
                        self.modal = Some(Modal::Message { plan, text });
                    }
                    _ => self.modal = Some(Modal::Message { plan, text }),
                },
                Modal::Help => {
                    if !matches!(k.code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?')) {
                        self.modal = Some(Modal::Help);
                    }
                }
                Modal::Confirm(plan) => match k.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        let fg = plan.action == "attach" && plan.cmd.first().map(|c| c != "tmux").unwrap_or(true);
                        return Some(if fg { Effect::RunForeground(plan) } else { Effect::RunDetached(plan) });
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {}
                    _ => self.modal = Some(Modal::Confirm(plan)),
                },
                Modal::Needs { items, mut sel } => match k.code {
                    KeyCode::Esc | KeyCode::Char('!') | KeyCode::Char('q') => {}
                    KeyCode::Char('j') | KeyCode::Down => {
                        sel = (sel + 1).min(items.len().saturating_sub(1));
                        self.modal = Some(Modal::Needs { items, sel });
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        sel = sel.saturating_sub(1);
                        self.modal = Some(Modal::Needs { items, sel });
                    }
                    KeyCode::Enter => {
                        if let Some(key) = items.get(sel).map(|i| i.0.clone()) {
                            self.jump_to_agent(&key);
                        }
                    }
                    _ => self.modal = Some(Modal::Needs { items, sel }),
                },
            }
            return None;
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
        // vim gg
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
                if self.current().map(|r| matches!(r.node.kind, Kind::Agent | Kind::Hub)).unwrap_or(false) {
                    return self.herdr_jump();
                } else {
                    self.toggle();
                }
            }
            (KeyCode::Char('/'), _) => self.editing_filter = true,
            (KeyCode::Esc, _) => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.rebuild();
                }
            }
            (KeyCode::Char('n'), _) => self.step_match(1),
            (KeyCode::Char('N'), _) => self.step_match(-1),
            (KeyCode::Char('!'), _) => self.open_needs(),
            (KeyCode::Char('?'), _) => self.modal = Some(Modal::Help),
            (KeyCode::Char('I'), _) => {
                self.show_others = !self.show_others;
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
            (KeyCode::Char('y'), _) => match self.selected_agent().and_then(|a| a.primary.clone()) {
                Some(p) => {
                    copy_to_clipboard(&p.session_id);
                    self.notify(format!("Yanked {}", p.session_id), st(mono::GREEN));
                }
                None => self.notify("No session selected.", st(mono::YELLOW)),
            },
            (KeyCode::Char('a'), _) => match self.selected_agent() {
                Some(a) => {
                    let plan = actions::plan_activate(&self.env, a);
                    self.confirm(plan);
                }
                None => self.notify("Select an agent first.", st(mono::YELLOW)),
            },
            (KeyCode::Char('m'), _) => match self.message_plan() {
                Some(plan) if plan.ok => self.modal = Some(Modal::Message { plan, text: String::new() }),
                Some(plan) => self.notify(format!("message: not available. {}", plan.summary), st(mono::YELLOW)),
                None => self.notify("Select an agent first.", st(mono::YELLOW)),
            },
            (KeyCode::Char('A'), _) => match self.selected_agent() {
                Some(a) => {
                    let plan = actions::plan_attach(&self.env, a, self.in_tmux);
                    self.confirm(plan);
                }
                None => self.notify("Select an agent first.", st(mono::YELLOW)),
            },
            (KeyCode::Char('x'), _) => match self.selected_agent() {
                Some(a) => {
                    let plan = actions::plan_delete(&self.env, a);
                    self.confirm(plan);
                }
                None => self.notify("Select a session first.", st(mono::YELLOW)),
            },
            (KeyCode::Char('J'), _) => self.step_pane(1),
            (KeyCode::Char('K'), _) => self.step_pane(-1),
            _ => {}
        }
        None
    }

    /// Run a detached plan (the terminal loop handles foreground ones).
    pub fn run_detached(&mut self, plan: &Plan) {
        let (rc, out) = actions::run_detached(&self.env, plan);
        if plan.action == "delete" {
            // never trust the exit code for rm: check the job rows are really gone
            let left: Vec<String> = std::iter::once(&plan.cmd)
                .chain(plan.more.iter())
                .filter_map(|c| c.last().cloned())
                .filter(|id| self.env.claude_home.join("jobs").join(id).exists())
                .collect();
            if left.is_empty() && rc == 0 {
                self.notify(format!("{}: deleted", plan.agent), st(mono::GREEN));
            } else {
                self.notify(format!("{}: job rows still there: {} ({})", plan.agent, left.join(" "), out.lines().last().unwrap_or("")), st(mono::PINK));
            }
            return;
        }
        let last = out.lines().last().unwrap_or("").to_string();
        if rc == 0 {
            self.notify(format!("{}: done  {last}", plan.agent), st(mono::GREEN));
        } else {
            self.notify(format!("{}: failed (exit {rc})  {last}", plan.agent), st(mono::PINK));
        }
    }

    pub fn after_foreground(&mut self, plan: &Plan, rc: i32) {
        self.notify(format!("Back from {} (exit {rc}).", plan.agent), st(mono::GREEN));
    }

    // ---------------------------------------------------------------- drawing

    pub fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        f.render_widget(Block::new().style(Style::new().bg(mono::BG).fg(mono::FG)), area);
        let [top, body, bottom] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)]).areas(area);
        f.render_widget(Paragraph::new(self.top_line()).style(Style::new().bg(mono::BG_HI)), top);
        let vertical = match self.split {
            Split::Vertical => true,
            Split::Horizontal => false,
            Split::Auto => body.width >= 150,
        };
        let (left, right) = if vertical {
            // the tree takes up to 60% of the width (and never more than its widest useful
            // size), leaving the detail pane the rest
            let w = table_max_width().min(body.width * 3 / 5).max(body.width.min(40));
            let [l, r] = Layout::horizontal([Constraint::Length(w), Constraint::Fill(1)]).areas(body);
            (l, r)
        } else {
            let [l, r] = Layout::vertical([Constraint::Percentage(62), Constraint::Fill(1)]).areas(body);
            (l, r)
        };
        self.page = left.height.saturating_sub(3) as usize;
        self.draw_frames(f, left);
        f.render_widget(
            Paragraph::new(self.detail_with_art(right.width.saturating_sub(2) as usize)).wrap(Wrap { trim: false }).block(
                Block::bordered().border_type(BorderType::Rounded).border_style(st(mono::PURPLE)).title(" detail "),
            ),
            right,
        );
        f.render_widget(Paragraph::new(self.bottom_line()), bottom);
        match &self.modal {
            Some(Modal::Help) => self.draw_help(f, area),
            Some(Modal::Confirm(p)) => draw_confirm(f, area, p),
            Some(Modal::Needs { items, sel }) => draw_needs(f, area, items, *sel),
            Some(Modal::Message { plan, text }) => draw_message(f, area, plan, text),
            Some(Modal::FleetFilter { sel }) => draw_fleet_filter(f, area, &self.fleet_states, *sel),
            None => {}
        }
    }

    fn top_line(&self) -> Line<'static> {
        let Some(s) = &self.snap else { return Line::from(" admiralty-watch  loading...") };
        let mut v = vec![
            Span::styled(" admiralty-watch ", Style::new().fg(mono::BG).bg(mono::CYAN).add_modifier(Modifier::BOLD)),
            Span::raw(format!("  {} fleets   {} live   ", s.fleets.len(), s.live_count())),
            Span::styled(format!("{} need you", s.need_count()), if s.need_count() > 0 { bold(mono::PINK) } else { st(grey(244)) }),
            Span::styled(format!("   hub {}", if s.hub.is_some() { "live" } else { "not running" }), st(grey(246))),
        ];
        if !self.filter.is_empty() {
            v.push(Span::styled(format!("   filter: {}", self.filter), st(mono::YELLOW)));
        }
        if self.flagged_only {
            v.push(Span::styled("   flagged only", st(mono::PURPLE)));
        }
        if self.fleet_states.iter().any(|on| !on) {
            let shown: Vec<&str> =
                FleetState::ALL.iter().filter(|x| self.fleet_states[x.index()]).map(|x| x.label()).collect();
            let txt = if shown.is_empty() { "none".to_string() } else { shown.join(", ") };
            v.push(Span::styled(format!("   fleets: {txt}"), st(mono::ORANGE)));
        }
        for e in &s.errors {
            v.push(Span::styled(format!("   [ERROR] {e}"), bold(mono::PINK)));
        }
        Line::from(v)
    }

    fn bottom_line(&self) -> Line<'static> {
        if self.editing_filter {
            return Line::from(vec![
                Span::styled("/", bold(mono::YELLOW)),
                Span::raw(self.filter.clone()),
                Span::styled("█", st(mono::YELLOW)),
                Span::styled("   enter keep, esc clear", st(grey(244))),
            ]);
        }
        if let Some((msg, style, t)) = &self.toast {
            if t.elapsed() < Duration::from_secs(8) {
                return Line::from(Span::styled(format!(" {msg}"), *style));
            }
        }
        let keys = [("j/k", "move"), ("h/l", "fold"), ("/", "filter"), ("!", "needs-you"), ("⏎", "pane"),
                    ("J/K", "next pane"), ("a", "activate"), ("m", "message"), ("x", "delete"), ("y", "yank"), ("I", "non-fleet"), ("f", "fleet filter"), ("F", "flagged"),
                    ("v/s", "split"), ("?", "help"), ("q", "quit")];
        let mut v = vec![];
        for (k, d) in keys {
            v.push(Span::styled(format!(" {k} "), bold(mono::YELLOW)));
            v.push(Span::styled(format!("{d} "), st(grey(250))));
        }
        Line::from(v)
    }

    /// One line per frame border or row; the `usize` is the row index a line selects.
    pub fn frame_lines(&self, width: u16) -> Vec<(Line<'static>, Option<usize>)> {
        let inner = width.saturating_sub(2) as usize;
        let w = col_widths(inner as u16 + 1); // col_widths keeps 1 column for a cursor bar we no longer draw
        let mut out: Vec<(Line<'static>, Option<usize>)> = vec![];
        let mut i = 0;
        while i < self.rows.len() {
            let head = &self.rows[i];
            let hub = head.node.kind == Kind::Hub;
            let color = head.node.frame.unwrap_or(mono::SOFT);
            let border = if hub { bold(color) } else { st(color) };
            // the hub's frame is double-lined and always drawn, even when empty
            let (v, bl, br, hz) = if hub { ("║", "╚", "╝", "═") } else { ("│", "╰", "╯", "─") };
            let mut end = i + 1;
            while end < self.rows.len() && self.rows[end].depth > 0 {
                end += 1;
            }
            let boxed = end > i + 1 || hub;
            out.push((self.title_line(head, width as usize, boxed, i == self.cursor), Some(i)));
            if boxed {
                if end == i + 1 {
                    let mut spans = vec![Span::styled(v, border)];
                    let note = if head.expanded || head.node.children.is_empty() {
                        "   no sub-agents or tasks right now"
                    } else {
                        "   folded (l to unfold)"
                    };
                    spans.push(Span::styled(pad(note, inner), st(mono::COMMENT)));
                    spans.push(Span::styled(v, border));
                    out.push((Line::from(spans), None));
                }
                for idx in i + 1..end {
                    let mut spans = vec![Span::styled(v, border)];
                    spans.extend(row_spans(&self.rows[idx], &w, inner));
                    spans.push(Span::styled(v, border));
                    let mut line = Line::from(spans);
                    if idx == self.cursor {
                        line = line.style(Style::new().bg(mono::SEL).add_modifier(Modifier::BOLD));
                    }
                    out.push((line, Some(idx)));
                }
                out.push((Line::styled(format!("{bl}{}{br}", hz.repeat(inner)), border), None));
            }
            i = end;
        }
        out
    }

    /// A section's title bar: a tinted band across the whole frame width, with the
    /// name and notes on the left and the agent counts (icon n / icon n) on the right.
    fn title_line(&self, r: &Row, width: usize, boxed: bool, selected: bool) -> Line<'static> {
        let n = &r.node;
        let hub = n.kind == Kind::Hub;
        let color = n.frame.unwrap_or(mono::SOFT);
        let band = Style::new().bg(if selected { mono::SEL } else { n.row_bg.unwrap_or(mono::BG_HI) });
        let marker = if n.children.is_empty() { "▶" } else if r.expanded { "▼" } else { "▶" };
        let mut left = vec![Span::styled(format!(" {marker} "), band.fg(if n.children.is_empty() { mono::DIM } else { color }))];
        if n.pane {
            let state = n.pane_status.as_deref().unwrap_or("unknown");
            left.push(Span::styled(format!("{} ", herdr::dot(state)), band.fg(herdr_color(state)).add_modifier(Modifier::BOLD)));
        }
        left.push(Span::styled(format!("{} ", n.icon), band.patch(n.icon_style)));
        left.push(Span::styled(n.name.clone(), band.patch(n.name_style).add_modifier(Modifier::BOLD)));
        let mut right: Vec<Span<'static>> = n.title_right.iter().map(|s| Span::styled(s.content.clone(), band.patch(s.style))).collect();
        if hub {
            left.push(Span::styled("   fleet hub", band.fg(mono::GOLD).add_modifier(Modifier::ITALIC)));
            right = vec![Span::styled(n.status.0.clone(), band.patch(n.status.1).add_modifier(Modifier::BOLD))];
            if !n.age.is_empty() {
                right.push(Span::styled(format!("  {}", n.age), band.fg(mono::SOFT)));
            }
        } else {
            let notes: String = n.detail.iter().map(|s| s.content.to_string()).collect();
            if !notes.trim().is_empty() {
                left.push(Span::styled(format!("   {}", notes.trim_end()), band.fg(mono::SOFT)));
            }
        }
        right.push(Span::styled(" ", band));
        let (corner_l, corner_r) = match (boxed, hub) {
            (true, true) => ("╔", "╗"),
            (true, false) => ("╭", "╮"),
            _ => (" ", " "),
        };
        let edge = if hub { bold(color) } else { st(color) };
        let room = width.saturating_sub(2);
        let right_w: usize = right.iter().map(|s| s.content.width()).sum();
        let left_room = room.saturating_sub(right_w + 1);
        let mut spans = vec![Span::styled(corner_l, edge.bg(band.bg.unwrap_or(mono::BG)))];
        let mut used = 0;
        for sp in left {
            let wsp = sp.content.width();
            if used + wsp > left_room {
                let cut = truncate(&sp.content, left_room.saturating_sub(used));
                used += cut.width();
                spans.push(Span::styled(cut, sp.style));
                break;
            }
            used += wsp;
            spans.push(sp);
        }
        spans.push(Span::styled(" ".repeat(room.saturating_sub(used + right_w)), band));
        spans.extend(right);
        spans.push(Span::styled(corner_r, edge.bg(band.bg.unwrap_or(mono::BG))));
        let mut line = Line::from(spans);
        if selected {
            line = line.style(Style::new().add_modifier(Modifier::BOLD));
        }
        line
    }

    fn draw_frames(&mut self, f: &mut Frame, area: Rect) {
        let inner = area.width.saturating_sub(2) as usize;
        let w = col_widths(inner as u16 + 1);
        // fixed column header, aligned with the rows inside the frames
        let mut header = vec![Span::raw(" ")];
        for (k, (name, width)) in ["  NAME", "STATUS", "AGE", "KIND", "DETAIL"].iter().zip(w.iter()).enumerate() {
            let text = if k == 2 { format!("{:>w$}", name, w = *width as usize) } else { pad(name, *width as usize) };
            header.push(Span::styled(text, bold(mono::COMMENT)));
            if k < 4 {
                header.push(Span::raw(" "));
            }
        }
        let lines = self.frame_lines(area.width);
        let view = area.height.saturating_sub(1) as usize;
        self.page = view.max(2);
        // keep the cursor's line (and, for a frame title, the frame's first row) in view
        if let Some(pos) = lines.iter().position(|(_, r)| *r == Some(self.cursor)) {
            if pos < self.scroll {
                self.scroll = pos;
            } else if pos >= self.scroll + view {
                self.scroll = pos + 1 - view;
            }
        }
        self.scroll = self.scroll.min(lines.len().saturating_sub(view));
        let mut shown = vec![Line::from(header)];
        shown.extend(lines.into_iter().skip(self.scroll).take(view).map(|(l, _)| l));
        f.render_widget(Paragraph::new(shown), area);
    }

    fn draw_help(&self, f: &mut Frame, area: Rect) {
        let mut lines = vec![Line::styled("Keys (vim style)", bold(mono::FG)), Line::raw("")];
        for (k, d) in HELP {
            lines.push(Line::from(vec![Span::styled(format!("  {k:<18}"), bold(mono::CYAN)), Span::raw(*d)]));
        }
        let r = centered(area, 78, lines.len() as u16 + 2);
        f.render_widget(Clear, r);
        f.render_widget(Paragraph::new(lines).block(Block::bordered().border_type(BorderType::Thick).border_style(st(mono::CYAN))), r);
    }

    /// The detail pane with a large ASCII icon top-left (the same picture as the
    /// row's glyph, in the row's colour) and the first detail lines beside it.
    pub fn detail_with_art(&self, width: usize) -> Vec<Line<'static>> {
        let lines = self.detail_lines();
        let Some(r) = self.current() else { return lines };
        let art = big_icon(&r.node, self.icons);
        let art_w = art.iter().map(|l| l.width()).max().unwrap_or(0) + 3;
        if art.is_empty() || width < art_w + 24 {
            return lines;
        }
        let style = if r.node.kind == Kind::Fleet { r.node.frame.map(bold).unwrap_or_default() } else { r.node.icon_style };
        let mut it = lines.into_iter();
        let mut out = vec![];
        for a in art {
            let mut spans = vec![Span::styled(pad(a, art_w), style)];
            if let Some(l) = it.next() {
                spans.extend(truncate_spans(l.spans, width - art_w));
            }
            out.push(Line::from(spans));
        }
        out.extend(it);
        out
    }

    pub fn detail_lines(&self) -> Vec<Line<'static>> {
        let (Some(r), Some(snap)) = (self.current(), &self.snap) else { return vec![] };
        let now = snap.taken_at;
        match r.node.kind {
            Kind::Fleet => {
                let Some(fl) = snap.fleets.iter().find(|x| Some(&x.tag) == r.node.fleet.as_ref()) else { return vec![] };
                let mut v = vec![Line::styled(fl.tag.clone(), bold(mono::FG)), Line::raw("")];
                v.push(kv("dir", fl.dir.clone()));
                v.push(kv("machine", fl.machine.clone().unwrap_or("not listed for this machine".into())));
                v.push(kv("lead", fl.lead().map(|l| format!("{}  {}", l.name, word(l.status))).unwrap_or("no [TAG]-LEAD session".into())));
                v.push(Line::raw(""));
                v.push(Line::styled(fleet_summary(fl), st(grey(249))));
                if let Some(e) = &fl.error {
                    v.push(Line::styled(format!("[ERROR] {e}"), st(mono::PINK)));
                }
                if !fl.lead().map(|l| l.live()).unwrap_or(false) {
                    v.push(Line::raw(""));
                    v.push(Line::styled("Start the lead yourself: `ccl` in the fleet directory.", st(grey(244))));
                }
                v
            }
            Kind::Sub => {
                let Some(a) = self.selected_agent() else { return vec![] };
                let Some(s) = a.all_subagents().into_iter().find(|s| Some(&s.rec.agent_id) == r.node.sub.as_ref()) else { return vec![] };
                let x = &s.rec;
                vec![
                    Line::styled(x.label.clone(), bold(mono::FG)),
                    Line::raw(""),
                    Line::from(vec![Span::styled(format!("{:<12}", "state"), st(grey(246))),
                                    Span::styled(if s.running { "running" } else { "finished" }, if s.running { st(mono::GREEN) } else { st(grey(246)) })]),
                    kv("kind", if x.teammate { "in-process teammate".into() } else { "sub-agent".into() }),
                    kv("type", x.agent_type.clone().unwrap_or("-".into())),
                    kv("model", x.model.clone().unwrap_or("-".into())),
                    kv("team", x.team.clone().unwrap_or("-".into())),
                    kv("id", x.agent_id.clone()),
                    kv("parent", x.parent_id.clone().unwrap_or("session".into())),
                    kv("last write", format!("{} ago", age(now - x.last_write))),
                    kv("children", s.children.len().to_string()),
                ]
            }
            Kind::Others => vec![Line::styled(
                "Sessions whose name has no known [FLEET]- tag and whose directory belongs to no fleet. Toggle with I.",
                st(grey(249)),
            )],
            _ => match self.selected_agent() {
                Some(a) => {
                    let msg = self.message_plan();
                    agent_detail(&self.env, a, now, self.panes.get(&a.key), self.in_tmux, msg)
                }
                None => vec![],
            },
        }
    }
}

fn kv(k: &str, v: String) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{k:<12}"), st(grey(246))), Span::raw(v)])
}

fn agent_detail(env: &Env, a: &Agent, now: f64, pane: Option<&Pane>, in_tmux: bool, msg: Option<actions::MessagePlan>) -> Vec<Line<'static>> {
    let mut v = vec![Line::styled(a.name.clone(), bold(mono::FG)), Line::raw("")];
    let ag = agent_age(a, now);
    let mut status = vec![Span::styled(format!("{:<12}", "status"), st(grey(246))),
                          Span::styled(word(a.status).to_uppercase(), status_style(a.status))];
    if !ag.is_empty() {
        status.push(Span::styled(
            if a.status_since.is_some() {
                format!("  for {ag}")
            } else if a.primary.as_ref().and_then(|p| p.state.as_deref()) == Some("ended") {
                format!("  last active {ag} ago")
            } else {
                format!("  started {ag} ago")
            },
            st(grey(246)),
        ));
    }
    v.push(Line::from(status));
    if let Some(p) = &a.primary {
        v.push(kv("kind", format!("{}   {}", p.kind, p.short_id.clone().unwrap_or_default())));
        v.push(kv("session", p.session_id.clone()));
        v.push(kv("pid", p.pid.map(|x| x.to_string()).unwrap_or("-  (not running)".into())));
        v.push(kv("cwd", p.cwd.clone()));
    }
    if let Some(e) = &a.expected_dir {
        if a.primary.as_ref().map(|p| p.cwd.trim_end_matches('/') != e.trim_end_matches('/')).unwrap_or(true) {
            v.push(kv("config dir", e.clone()));
        }
    }
    if let Some(p) = pane {
        v.push(Line::from(vec![
            Span::styled(format!("{:<12}", "herdr pane"), st(grey(246))),
            Span::styled(format!("{} {} ", herdr::dot(&p.status), if p.status.is_empty() { "?" } else { &p.status }), st(herdr_color(&p.status))),
            Span::raw(format!("  {}   tab {}   workspace {}", p.pane_id, p.tab_id, p.workspace_id)),
        ]));
    }
    if a.job.tokens.is_some() || a.job.tasks > 0 || a.job.queued > 0 {
        v.push(kv("job", format!("tokens {}   in-flight {}   queued {}", a.job.tokens.unwrap_or(0), a.job.tasks, a.job.queued)));
    }
    if !a.flags.is_empty() {
        v.push(Line::raw(""));
        v.push(Line::styled("FLAGS", bold(mono::PURPLE)));
        for fl in &a.flags {
            v.push(Line::from(vec![Span::styled(format!("  {}: ", fl.text()), st(mono::PURPLE)), Span::raw(fl.help())]));
        }
    }
    if let Some(n) = &a.job.needs {
        v.push(Line::raw(""));
        let title = if a.status == Status::NeedsYou { "NEEDS YOU" } else { "LAST QUESTION (agent is stopped)" };
        v.push(Line::styled(title, bold(mono::PINK)));
        for l in n.lines() {
            v.push(Line::raw(l.to_string()));
        }
    } else if let Some(d) = &a.job.detail {
        v.push(Line::raw(""));
        v.push(Line::styled("DETAIL", bold(mono::FG)));
        v.push(Line::raw(d.clone()));
    }
    if !a.job.timeline.is_empty() {
        v.push(Line::raw(""));
        v.push(Line::styled("TIMELINE", bold(mono::FG)));
        for e in &a.job.timeline {
            let when = e.at.get(5..16).unwrap_or("").replace('T', " ");
            v.push(Line::from(vec![
                Span::styled(format!("  {when}  "), st(grey(246))),
                Span::styled(format!("{:<8} ", e.state), st(mono::YELLOW)),
                Span::raw(e.detail.chars().take(80).collect::<String>()),
            ]));
        }
    }
    let subs = a.all_subagents();
    if !subs.is_empty() {
        let run = subs.iter().filter(|s| s.running).count();
        v.push(Line::raw(""));
        v.push(Line::styled(format!("SUB-AGENTS  {run} running, {} finished", subs.len() - run), bold(mono::FG)));
    }
    if !a.tasks.is_empty() {
        v.push(Line::raw(""));
        v.push(Line::styled("TASKS", bold(mono::FG)));
        for t in &a.tasks {
            let owner = t.owner.as_ref().map(|o| format!("  ({o})")).unwrap_or_default();
            v.push(Line::raw(format!("  {:>3} {:<11} {}{owner}", t.id, t.status, t.subject)));
        }
    }
    if a.is_lead && !a.live() {
        v.push(Line::raw(""));
        let dir = a.expected_dir.clone().unwrap_or_default();
        v.push(Line::styled(format!("Not running. Start it yourself: `ccl` in {dir}"), st(mono::ORANGE)));
    }
    v.push(Line::raw(""));
    v.push(Line::styled("ACTIONS", bold(mono::FG)));
    if let Some(p) = pane {
        v.push(Line::from(vec![Span::styled("  ⏎  ", bold(mono::FG)),
                               Span::raw(format!("jump to its Herdr pane {}", p.pane_id))]));
    } else if env.herdr_bin.is_some() && a.live() {
        v.push(Line::from(vec![Span::styled("  ⏎  ", bold(mono::FG)),
                               Span::styled("unavailable: no Herdr pane hosts this session", st(grey(244)))]));
    }
    if let Some(m) = msg {
        v.push(Line::from(vec![
            Span::styled("  m  ", bold(mono::FG)),
            if m.ok { Span::raw(format!("message {}", m.summary)) } else { Span::styled(format!("unavailable: {}", m.summary), st(grey(244))) },
        ]));
    }
    let act = actions::plan_activate(env, a);
    v.push(plan_line("a", &act));
    let att = actions::plan_attach(env, a, in_tmux);
    if att.ok {
        v.push(plan_line("A", &att));
    }
    let del = actions::plan_delete(env, a);
    if del.ok {
        v.push(plan_line("x", &del));
    }
    v
}

fn plan_line(key: &str, p: &Plan) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {key}  "), bold(mono::FG)),
        if p.ok { Span::raw(p.summary.clone()) } else { Span::styled(format!("unavailable: {}", p.summary), st(grey(244))) },
    ])
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn draw_confirm(f: &mut Frame, area: Rect, p: &Plan) {
    let mut lines = vec![Line::styled(format!("{}?", p.summary), bold(mono::FG)), Line::raw("")];
    for (k, v) in &p.details {
        lines.push(kv(k, v.clone()));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(p.shell().chars().take(600).collect::<String>(), st(grey(246))));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![Span::styled("[y] Run", bold(mono::YELLOW)), Span::raw("     "), Span::styled("[n] Cancel", bold(mono::FG))]));
    let r = centered(area, 96, 16);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered().border_type(BorderType::Thick).border_style(st(mono::YELLOW)).title(format!(" {} ", p.action)),
        ),
        r,
    );
}

fn draw_fleet_filter(f: &mut Frame, area: Rect, on: &[bool; 7], sel: usize) {
    let mut lines = vec![Line::styled("Show fleets that are", bold(mono::FG)), Line::raw("")];
    for (i, s) in FleetState::ALL.iter().enumerate() {
        let mut l = Line::from(vec![
            Span::styled(if on[i] { " [x] " } else { " [ ] " }, bold(if on[i] { mono::GREEN } else { mono::SOFT })),
            Span::styled("  ", Style::new().bg(s.tint())),
            Span::raw(format!(" {}", s.label())),
        ]);
        if i == sel {
            l = l.style(Style::new().bg(mono::SEL));
        }
        lines.push(l);
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("space", bold(mono::YELLOW)),
        Span::styled(" toggle   ", st(mono::SOFT)),
        Span::styled("a", bold(mono::YELLOW)),
        Span::styled(" all   ", st(mono::SOFT)),
        Span::styled("enter/esc", bold(mono::YELLOW)),
        Span::styled(" close", st(mono::SOFT)),
    ]));
    let r = centered(area, 44, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).style(Style::new().bg(mono::BG)).block(
            Block::bordered().border_type(BorderType::Thick).border_style(st(mono::ORANGE)).title(" fleet filter "),
        ),
        r,
    );
}

fn draw_message(f: &mut Frame, area: Rect, plan: &actions::MessagePlan, text: &str) {
    let mut lines = vec![
        Line::styled(format!("Message {}", plan.to), bold(mono::FG)),
        Line::styled(
            if plan.relay {
                format!("goes to {} first, which relays it with SendMessage", plan.via)
            } else {
                format!("typed into {}'s Herdr pane", plan.via)
            },
            st(mono::COMMENT),
        ),
        Line::raw(""),
    ];
    let mut input = vec![Span::styled("> ", bold(mono::ORANGE)), Span::raw(text.to_string())];
    input.push(Span::styled("\u{2588}", st(mono::ORANGE)));
    lines.push(Line::from(input));
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("enter", bold(mono::YELLOW)),
        Span::styled(" send   ", st(mono::SOFT)),
        Span::styled("esc", bold(mono::YELLOW)),
        Span::styled(" cancel   ", st(mono::SOFT)),
        Span::styled("ctrl-u", bold(mono::YELLOW)),
        Span::styled(" clear", st(mono::SOFT)),
    ]));
    let r = centered(area, 90, 9);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).style(Style::new().bg(mono::BG)).block(
            Block::bordered().border_type(BorderType::Thick).border_style(st(mono::ORANGE)).title(" message "),
        ),
        r,
    );
}

fn draw_needs(f: &mut Frame, area: Rect, items: &[(String, String, Status, String, String)], sel: usize) {
    let mut lines = vec![Line::styled("Waiting on you   j/k move   enter: jump to agent   esc: back", bold(mono::FG)), Line::raw("")];
    if items.is_empty() {
        lines.push(Line::styled("  nothing is waiting on you", st(grey(244))));
    }
    for (i, (_, name, status, ag, text)) in items.iter().enumerate() {
        let mut line = Line::from(vec![
            Span::styled(format!("  {:<26}", name), status_style(*status)),
            Span::styled(format!("{:<5}", ag), st(grey(244))),
            Span::raw(text.chars().take(140).collect::<String>()),
        ]);
        if i == sel {
            line = line.style(Style::new().bg(mono::SEL));
        }
        lines.push(line);
    }
    let r = centered(area, area.width.saturating_sub(8), (items.len() as u16 + 5).max(7));
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).block(Block::bordered().border_type(BorderType::Thick).border_style(st(mono::PINK))), r);
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

fn copy_to_clipboard(s: &str) {
    use std::io::Write;
    // OSC 52 reaches the outer terminal (WezTerm) through Herdr/tmux too
    let _ = write!(std::io::stdout(), "\x1b]52;c;{}\x07", base64(s.as_bytes()));
    if let Some(pb) = crate::env::which("pbcopy") {
        if let Ok(mut c) = std::process::Command::new(pb).stdin(std::process::Stdio::piped()).spawn() {
            if let Some(mut stdin) = c.stdin.take() {
                let _ = stdin.write_all(s.as_bytes());
            }
            let _ = c.wait();
        }
    }
}

// ------------------------------------------------------------------ terminal loop

enum Cmd {
    Refresh,
}

/// Loader thread: re-reads every source each `interval`, or at once on Refresh.
fn spawn_loader(env: Env, interval: Duration) -> (mpsc::Sender<Cmd>, mpsc::Receiver<(Snapshot, Vec<Pane>)>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut cache = Cache::default();
        loop {
            let snap = crate::model::snapshot(&env, &mut cache);
            let panes = herdr::list_panes(env.herdr_bin.as_deref());
            if tx.send((snap, panes)).is_err() {
                return;
            }
            match cmd_rx.recv_timeout(interval) {
                Ok(Cmd::Refresh) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    (cmd_tx, rx)
}

pub fn run(env: Env, interval: Duration) -> std::io::Result<()> {
    let in_tmux = std::env::var("TMUX").map(|v| !v.is_empty()).unwrap_or(false) && crate::env::which("tmux").is_some();
    let mut app = App::new(env.clone(), in_tmux);
    let (cmd_tx, data_rx) = spawn_loader(env, interval);
    let mut terminal = ratatui::init();
    let mut dirty = true;
    let mut last_draw = Instant::now();
    let result = loop {
        // newest snapshot wins; older queued ones are skipped
        let mut latest = None;
        while let Ok(d) = data_rx.try_recv() {
            latest = Some(d);
        }
        if let Some((snap, panes)) = latest {
            app.apply(snap, panes);
            dirty = true;
        }
        // redraw once a second regardless, so ages and toasts stay current
        if dirty || last_draw.elapsed() >= Duration::from_secs(1) {
            if let Err(e) = terminal.draw(|f| app.draw(f)) {
                break Err(e);
            }
            dirty = false;
            last_draw = Instant::now();
        }
        match event::poll(Duration::from_millis(200)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => break Err(e),
        }
        let ev = match event::read() {
            Ok(e) => e,
            Err(e) => break Err(e),
        };
        dirty = true;
        let Event::Key(k) = ev else { continue };
        match app.on_key(k) {
            Some(Effect::Quit) => break Ok(()),
            Some(Effect::Refresh) => {
                let _ = cmd_tx.send(Cmd::Refresh);
            }
            Some(Effect::RunDetached(plan)) => {
                app.run_detached(&plan);
                let _ = cmd_tx.send(Cmd::Refresh);
            }
            Some(Effect::RunForeground(plan)) => {
                ratatui::restore();
                let rc = actions::run_foreground(&app.env, &plan);
                terminal = ratatui::init();
                let _ = terminal.clear();
                app.after_foreground(&plan, rc);
                let _ = cmd_tx.send(Cmd::Refresh);
            }
            None => {}
        }
    };
    ratatui::restore();
    result
}

pub fn visible_ids(app: &App) -> HashSet<String> {
    app.rows.iter().map(|r| r.node.id.clone()).collect()
}
