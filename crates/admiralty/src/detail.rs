//! The detail pane: a large picture of the row's kind top-left, then what the
//! row is, why its status is what it is, its issues with their fixes, and what
//! each key would do here (or why it will not).

use nimitz_core::{Comms, Fleet, Issue, Member, SessionInfo, Snapshot, Squadron};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::actions::{find_member, Act, Target};
use crate::app::App;
use crate::frames::{pad, truncate_spans};
use crate::theme::{age, art, bold, mono, pane_color, pane_dot, severity_color, severity_word, st, status_style, status_word, Art};
use crate::view::{fleet_issues, fleet_members, squadron_members, FleetState, Kind, Node};

const KEY_W: usize = 12;

fn kv(k: &str, v: impl Into<String>) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{k:<KEY_W$}"), st(mono::COMMENT)), Span::styled(v.into(), st(mono::FG))])
}

fn heading(t: &str) -> Line<'static> {
    Line::styled(t.to_string(), bold(mono::PURPLE))
}

fn blank() -> Line<'static> {
    Line::raw("")
}

pub fn art_for(n: &Node) -> Art {
    match n.kind {
        Kind::FleetBand => Art::Fleet,
        Kind::SquadronBand => Art::Squadron,
        Kind::StowBand | Kind::Stowaway => Art::Stowaway,
        Kind::ExtraRow => Art::StaleRow,
        Kind::Crew if n.kind_text == "teammate" => Art::Teammate,
        Kind::Crew => Art::Crew,
        Kind::Task if n.status.0 == "completed" => Art::TaskDone,
        Kind::Task if n.status.0 == "in progress" => Art::TaskWip,
        Kind::Task => Art::TaskTodo,
        Kind::Member(r) => match r {
            nimitz_core::Role::Admiral => Art::Admiral,
            nimitz_core::Role::Commodore => Art::Commodore,
            nimitz_core::Role::Ship => Art::Ship,
            nimitz_core::Role::Tender => Art::Tender,
        },
    }
}

/// The pane's lines with the picture beside the first ones.
pub fn detail_with_art(app: &App, width: usize) -> Vec<Line<'static>> {
    let lines = detail_lines(app);
    let Some(r) = app.current() else { return lines };
    let pic = art(art_for(&r.node));
    let art_w = pic.iter().map(|l| l.width()).max().unwrap_or(0) + 3;
    if width < art_w + 28 {
        return lines;
    }
    let style = match r.node.kind {
        Kind::FleetBand => bold(mono::GOLD),
        Kind::SquadronBand | Kind::StowBand => r.node.frame.map(bold).unwrap_or_default(),
        _ => r.node.icon_style.add_modifier(Modifier::BOLD),
    };
    let mut it = lines.into_iter();
    let mut out = vec![];
    for a in pic.iter() {
        let mut spans = vec![Span::styled(pad(a, art_w), style)];
        if let Some(l) = it.next() {
            spans.extend(truncate_spans(l.spans, width - art_w));
        }
        out.push(Line::from(spans));
    }
    out.extend(it);
    out
}

pub fn detail_lines(app: &App) -> Vec<Line<'static>> {
    let (Some(r), Some(b)) = (app.current(), &app.bundle) else { return vec![Line::styled("loading ...", st(mono::COMMENT))] };
    let snap = &b.snap;
    let now = app.now();
    let mut v = match (&r.node.kind, app.target()) {
        (Kind::FleetBand, Target::Fleet(id)) => match snap.fleets.iter().find(|f| f.id == id) {
            Some(f) => fleet_detail(f, now),
            None => vec![],
        },
        (Kind::SquadronBand, Target::Squadron(id)) => {
            match snap.fleets.iter().flat_map(|f| f.squadrons.iter().map(move |s| (f, s))).find(|(_, s)| s.id == id) {
                Some((f, sq)) => squadron_detail(f, sq),
                None => vec![],
            }
        }
        (Kind::Member(_), Target::Member(id)) => match find_member(snap, &id) {
            Some(m) => member_detail(app, m, now),
            None => vec![],
        },
        (Kind::Stowaway, Target::Stowaway(sid)) => match snap.stowaways.iter().find(|s| s.session_id == sid) {
            Some(s) => stowaway_detail(snap, s, now),
            None => vec![],
        },
        (Kind::ExtraRow, Target::ExtraRow { owner, session_id }) => {
            let s = find_member(snap, &owner).and_then(|m| m.extra_rows.iter().find(|x| x.session_id == session_id)).cloned();
            match s {
                Some(s) => extra_detail(&owner.name(), &s, now),
                None => vec![],
            }
        }
        (Kind::StowBand, _) => vec![
            Line::styled("STOWAWAYS", bold(mono::PINK)),
            blank(),
            Line::styled("Sessions in a fleet's folders that carry no NIMITZ_* labels.", st(mono::SOFT)),
            Line::styled("nimitz did not start them, so nothing binds them to an identity.", st(mono::SOFT)),
            blank(),
            Line::styled("On a stowaway: d adopt it as a ship or tender, x rm its job row,", st(mono::SOFT)),
            Line::styled("A attach to it. I hides this group.", st(mono::SOFT)),
        ],
        (Kind::Crew, _) => vec![
            Line::styled(r.node.name.clone(), bold(mono::FG)),
            blank(),
            kv("state", r.node.status.0.trim_start_matches(['>', '.', ' ']).to_string()),
            kv("kind", r.node.kind_text.clone()),
            kv("type", r.node.detail.iter().map(|s| s.content.to_string()).collect::<String>()),
            kv("last write", format!("{} ago", r.node.age)),
            blank(),
            Line::styled("Crew run inside their session; reach them through it.", st(mono::COMMENT)),
        ],
        (Kind::Task, _) => vec![
            Line::styled(r.node.name.clone(), bold(mono::FG)),
            blank(),
            kv("status", r.node.status.0.clone()),
            kv("owner", r.node.detail.iter().map(|s| s.content.to_string()).collect::<String>()),
        ],
        _ => vec![],
    };
    if matches!(r.node.kind, Kind::FleetBand | Kind::SquadronBand | Kind::Member(_) | Kind::Stowaway | Kind::ExtraRow) {
        v.extend(actions_block(app, &r.node));
    }
    v
}

fn comms_text(c: &Comms) -> String {
    match c {
        Comms::Mode(m) => m.clone(),
        Comms::Allow { allow } => format!("allow {}", allow.join(", ")),
    }
}

fn issues_block(title: &str, issues: &[&Issue]) -> Vec<Line<'static>> {
    if issues.is_empty() {
        return vec![];
    }
    let mut v = vec![blank(), heading(&format!("{title} ({})", issues.len()))];
    for i in issues {
        let c = severity_color(i.severity);
        v.push(Line::from(vec![
            Span::styled(format!("  {:<6}", severity_word(i.severity).to_uppercase()), bold(c)),
            Span::styled(i.code.clone(), bold(c)),
        ]));
        v.push(Line::from(vec![Span::raw(" ".repeat(8)), Span::styled(i.message.clone(), st(mono::FG))]));
        if let Some(fx) = &i.fix {
            v.push(Line::from(vec![Span::raw(" ".repeat(8)), Span::styled("fix  ", st(mono::COMMENT)), Span::styled(fx.clone(), bold(mono::CYAN))]));
        }
    }
    v
}

fn fleet_detail(f: &Fleet, now: f64) -> Vec<Line<'static>> {
    let state = FleetState::of(f);
    let members = fleet_members(f);
    let live = members.iter().filter(|m| crate::view::member_live(m)).count();
    let mut v = vec![
        Line::from(vec![Span::styled(format!("FLEET [{}]", f.id.0), bold(mono::GOLD))]),
        blank(),
        Line::from(vec![Span::styled(format!("{:<KEY_W$}", "state"), st(mono::COMMENT)), Span::styled(state.label().to_uppercase(), bold(state.color()))]),
        kv("machine", f.machine.clone().unwrap_or_else(|| "not in fleet.toml machines: does not run here".into())),
        kv("folder", f.dir.clone()),
        kv("admiral", format!("{}  {}{}", f.admiral.expected_name, status_word(f.admiral.status), member_age(&f.admiral, now))),
        kv("squadrons", f.squadrons.iter().map(|s| s.id.squadron().unwrap_or("?").to_string()).collect::<Vec<_>>().join(", ")),
        kv("members", format!("{} declared, {live} live", members.len())),
    ];
    v.extend(issues_block("FLEET ISSUES", &f.issues.iter().collect::<Vec<_>>()));
    let all = fleet_issues(f);
    if all.len() > f.issues.len() {
        v.push(blank());
        v.push(Line::styled(format!("{} more issues on its members; `i` lists them all.", all.len() - f.issues.len()), st(mono::SOFT)));
    }
    v
}

fn squadron_detail(f: &Fleet, sq: &Squadron) -> Vec<Line<'static>> {
    let state = FleetState::of_squadron(f, sq);
    let mut v = vec![
        Line::styled(format!("SQUADRON {}", sq.id.0), bold(mono::FG)),
        blank(),
        Line::from(vec![Span::styled(format!("{:<KEY_W$}", "state"), st(mono::COMMENT)), Span::styled(state.squadron_label().to_uppercase(), bold(state.color()))]),
        kv("commodore", format!("{}  {}", sq.commodore.expected_name, status_word(sq.commodore.status))),
        kv("comms", comms_text(&sq.comms)),
        kv("ships", sq.ships.iter().map(|s| s.id.parts().last().unwrap_or(&"?").to_string()).collect::<Vec<_>>().join(", ")),
    ];
    if !sq.tenders.is_empty() {
        v.push(kv("tenders", sq.tenders.iter().map(|s| s.id.parts().last().unwrap_or(&"?").trim_start_matches('~').to_string()).collect::<Vec<_>>().join(", ")));
    }
    v.extend(issues_block("SQUADRON ISSUES", &sq.issues.iter().collect::<Vec<_>>()));
    let member_issues: usize = squadron_members(sq).iter().map(|m| m.issues.len()).sum();
    if member_issues > 0 {
        v.push(blank());
        v.push(Line::styled(format!("{member_issues} issues on its members (marked in the list)."), st(mono::SOFT)));
    }
    v
}

fn member_age(m: &Member, now: f64) -> String {
    let s = m.session.as_ref();
    match s.and_then(|s| s.status_since).or(s.and_then(|s| s.started_at)) {
        Some(t) => format!("  for {}", age(now - t)),
        None => String::new(),
    }
}

fn session_lines(s: &SessionInfo, now: f64, expected_name: Option<&str>) -> Vec<Line<'static>> {
    let mut v = vec![
        kv("session", s.session_id.clone()),
        kv("kind", format!("{}   {}", s.kind, s.short_id.clone().unwrap_or_default())),
        kv("pid", s.pid.map(|x| x.to_string()).unwrap_or_else(|| "-  (not running)".into())),
    ];
    match expected_name {
        Some(e) if e != s.name => v.push(Line::from(vec![
            Span::styled(format!("{:<KEY_W$}", "name"), st(mono::COMMENT)),
            Span::styled(s.name.clone(), bold(mono::PINK)),
            Span::styled(format!("   expected {e}"), st(mono::SOFT)),
        ])),
        _ => v.push(kv("name", s.name.clone())),
    }
    v.push(kv("cwd", s.cwd.clone()));
    if let Some(t) = s.started_at {
        v.push(kv("started", format!("{} ago", age(now - t))));
    }
    if s.labels.is_empty() {
        v.push(Line::from(vec![Span::styled(format!("{:<KEY_W$}", "labels"), st(mono::COMMENT)), Span::styled("none", bold(mono::ORANGE))]));
    } else {
        let id = s.labels.get("NIMITZ_ID").cloned().unwrap_or_default();
        v.push(kv("labels", format!("NIMITZ_ID={id}  (+{} more)", s.labels.len().saturating_sub(1))));
    }
    v
}

fn member_detail(app: &App, m: &Member, now: f64) -> Vec<Line<'static>> {
    let mut v = vec![
        Line::styled(m.expected_name.clone(), bold(mono::FG)),
        blank(),
        Line::from(vec![
            Span::styled(format!("{:<KEY_W$}", "status"), st(mono::COMMENT)),
            Span::styled(status_word(m.status).to_uppercase(), status_style(m.status).add_modifier(Modifier::BOLD)),
            Span::styled(member_age(m, now), st(mono::COMMENT)),
        ]),
        kv("identity", format!("{}   {}", m.id.0, m.role.as_str())),
    ];
    if let Some(c) = &m.cwd {
        v.push(kv("folder", c.clone()));
    }
    if let Some(w) = m.session.as_ref().and_then(|s| s.waiting_for.clone()) {
        v.push(blank());
        v.push(Line::styled("NEEDS YOU", bold(mono::PINK)));
        v.push(Line::styled(format!("  {w}"), st(mono::FG)));
    }
    if !m.reasons.is_empty() {
        v.push(blank());
        v.push(heading("WHY THIS STATUS"));
        for r in &m.reasons {
            v.push(Line::from(vec![Span::styled("  \u{00b7} ", st(mono::COMMENT)), Span::styled(r.clone(), st(mono::SOFT))]));
        }
    }
    v.extend(issues_block("ISSUES", &m.issues.iter().collect::<Vec<_>>()));
    if let Some(s) = &m.session {
        v.push(blank());
        v.push(heading("SESSION"));
        v.extend(session_lines(s, now, Some(&m.expected_name)));
    }
    if let Some(pid) = &m.pane {
        if let Some(p) = app.bundle.as_ref().and_then(|b| b.panes.iter().find(|p| &p.pane.pane_id == pid)) {
            v.push(Line::from(vec![
                Span::styled(format!("{:<KEY_W$}", "pane"), st(mono::COMMENT)),
                Span::styled(format!("{} {}", pane_dot(&p.state), if p.state.is_empty() { "?" } else { &p.state }), bold(pane_color(&p.state))),
                Span::styled(
                    format!("   {}   tab {}   workspace {}", p.pane.pane_id, p.pane.tab_id.clone().unwrap_or("-".into()), p.pane.workspace_id.clone().unwrap_or("-".into())),
                    st(mono::SOFT),
                ),
            ]));
        }
    }
    if !m.extra_rows.is_empty() {
        v.push(blank());
        v.push(heading(&format!("OTHER JOB ROWS ({})", m.extra_rows.len())));
        for x in &m.extra_rows {
            let live = x.pid.is_some();
            v.push(Line::from(vec![
                Span::styled(format!("  {:<10}", x.short_id.clone().unwrap_or_default()), if live { bold(mono::PINK) } else { st(mono::SOFT) }),
                Span::styled(if live { "live copy".into() } else { x.state.clone().unwrap_or("stopped".into()) }, if live { bold(mono::PINK) } else { st(mono::COMMENT) }),
            ]));
        }
    }
    if !m.crew.is_empty() {
        let run = m.crew.iter().filter(|c| c.running).count();
        v.push(blank());
        v.push(heading(&format!("CREW  {run} running, {} finished", m.crew.len() - run)));
    }
    if !m.tasks.is_empty() {
        v.push(blank());
        v.push(heading("TASKS"));
        for t in &m.tasks {
            let owner = t.owner.as_ref().map(|o| format!("  ({o})")).unwrap_or_default();
            v.push(Line::styled(format!("  {:>3} {:<11} {}{owner}", t.id, t.status.replace('_', " "), t.subject), st(mono::SOFT)));
        }
    }
    v
}

fn stowaway_detail(snap: &Snapshot, s: &SessionInfo, now: f64) -> Vec<Line<'static>> {
    let mut v = vec![
        Line::styled(if s.name.is_empty() { s.session_id.clone() } else { s.name.clone() }, bold(mono::PINK)),
        blank(),
        Line::from(vec![
            Span::styled(format!("{:<KEY_W$}", "status"), st(mono::COMMENT)),
            Span::styled(if s.pid.is_some() { "LIVE STOWAWAY" } else { "STOPPED STOWAWAY" }, bold(mono::PINK)),
        ]),
        Line::styled("A session in fleet scope with no NIMITZ_* labels.", st(mono::SOFT)),
    ];
    let issues: Vec<&Issue> = crate::view::stowaway_issues(snap, s);
    if issues.iter().any(|i| i.code == "legacy") {
        v.push(Line::styled("Its name says it belongs to an old fleet-skill fleet: adopt it to bring it under nimitz.", st(mono::ORANGE)));
    }
    v.extend(issues_block("ISSUES", &issues));
    v.push(blank());
    v.push(heading("SESSION"));
    v.extend(session_lines(s, now, None));
    v
}

fn extra_detail(owner: &str, s: &SessionInfo, now: f64) -> Vec<Line<'static>> {
    let live = s.pid.is_some();
    let mut v = vec![
        Line::styled(format!("job row {}", s.short_id.clone().unwrap_or_default()), bold(if live { mono::PINK } else { mono::FG })),
        blank(),
        Line::styled(
            if live { format!("A second live session for {owner}: one identity, two sessions.") } else { format!("A stale job row of {owner} (a copy or an old session).") },
            st(mono::SOFT),
        ),
        blank(),
        heading("SESSION"),
    ];
    v.extend(session_lines(s, now, None));
    v
}

/// The ACTIONS block: each key, and what it does here or why it will not.
fn actions_block(app: &App, n: &Node) -> Vec<Line<'static>> {
    let mut v = vec![blank(), heading("ACTIONS")];
    // keys closed only for want of a multiplexer are listed once, at the end
    let mut no_mux: Vec<String> = vec![];
    let mut line = |v: &mut Vec<Line<'static>>, key: &str, r: Result<String, String>| match r {
        Ok(t) => v.push(Line::from(vec![Span::styled(format!("  {key:<4}"), bold(mono::YELLOW)), Span::styled(t, st(mono::FG))])),
        Err(why) if why == crate::actions::MONITOR_ONLY => no_mux.push(key.to_string()),
        Err(why) => v.push(Line::from(vec![Span::styled(format!("  {key:<4}"), bold(mono::DIM)), Span::styled(why, st(mono::COMMENT))])),
    };
    let jump = match (&n.pane_id, app.bundle.as_ref().map(|b| b.mux_caps.focus).unwrap_or(false)) {
        (Some(p), true) => Ok(format!("jump to its pane {p}")),
        (_, false) if matches!(n.kind, Kind::Member(_) | Kind::Stowaway) => Err(crate::actions::MONITOR_ONLY.to_string()),
        (None, true) if matches!(n.kind, Kind::Member(_) | Kind::Stowaway) => Err("no pane hosts this session".into()),
        _ => Err("fold / unfold".into()),
    };
    line(&mut v, "\u{23ce}", jump);
    let mut acts: Vec<Act> = vec![Act::Start, Act::Open, Act::Stop, Act::Relaunch, Act::Sweep, Act::Rm, Act::Adopt];
    if matches!(n.kind, Kind::FleetBand) {
        acts.insert(0, Act::Up);
    }
    let member_like = matches!(n.kind, Kind::Member(_) | Kind::FleetBand | Kind::SquadronBand);
    for a in acts {
        let r = app.available(a);
        if a == Act::Adopt {
            if let Some((_, id)) = app.legacy_adopt() {
                line(&mut v, "d", Ok(format!("adopt this legacy session as {id}")));
                continue;
            }
        }
        if r.is_ok() || (member_like && matches!(a, Act::Start | Act::Open)) {
            line(&mut v, &a.key().to_string(), r.map(|_| a.label().to_string()));
        }
    }
    if member_like {
        match app.message_plan() {
            Ok(p) if p.relay => line(&mut v, "m", Ok(format!("message {} (relayed by {})", p.to, p.via))),
            Ok(p) => line(&mut v, "m", Ok(format!("message {} in its pane", p.to))),
            Err(why) => line(&mut v, "m", Err(why)),
        }
    }
    if !no_mux.is_empty() {
        v.push(Line::from(vec![
            Span::styled(format!("  {:<4}", no_mux.join(" ")), bold(mono::DIM)),
            Span::styled(format!("  {}", crate::actions::MONITOR_ONLY), st(mono::ORANGE)),
        ]));
    }
    v.push(Line::styled("  x   all actions   c create   e edit   D remove from config", st(mono::COMMENT)));
    v
}

/// Style for a `[demo]` / `[monitor-only]` style badge.
pub fn badge(text: &str, bg: ratatui::style::Color) -> Span<'static> {
    Span::styled(format!(" {text} "), Style::new().fg(mono::BG).bg(bg).add_modifier(Modifier::BOLD))
}
