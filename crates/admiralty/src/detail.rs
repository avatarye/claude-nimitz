//! The detail pane: a large picture of the row's kind top-left, then what the
//! row is, why its status is what it is, its issues with their fixes, and what
//! each key would do here (or why it will not).

use nimitz_core::{Fleet, Issue, Member, SessionInfo, Snapshot, Squadron};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::actions::{find_member, Target};
use crate::app::App;
use crate::frames::{pad, truncate_spans, wrap};
use crate::theme::{art, bold, mono, pane_color, pane_dot, severity_color, severity_word, st, status_style, status_word, Art};
use crate::view::{fleet_issues, fleet_members, squadron_members, FleetState, Kind, Node};

const KEY_W: usize = 12;

fn kv(k: &str, v: impl Into<String>) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{k:<KEY_W$}"), st(mono::COMMENT)), Span::styled(v.into(), st(mono::FG))])
}

fn heading(t: &str) -> Line<'static> {
    Line::styled(t.to_string(), bold(mono::STRONG))
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
        Kind::Note => Art::Issue,
        Kind::Crew if n.kind_text == "teammate" => Art::Teammate,
        Kind::Crew | Kind::CrewFold | Kind::Workflow => Art::Crew,
        Kind::Task if n.status.0.ends_with(crate::view::Work::Done.word()) => Art::TaskDone,
        Kind::Task if n.status.0.ends_with(crate::view::Work::Busy.word()) => Art::TaskWip,
        Kind::Task => Art::TaskTodo,
        Kind::Member(r) => match r {
            nimitz_core::Role::Admiral => Art::Admiral,
            nimitz_core::Role::Commodore => Art::Commodore,
            nimitz_core::Role::Ship => Art::Ship,
        },
    }
}

/// The pane's lines with the picture beside the first ones.
/// `compact`: a short pane gets no picture and no blank lines, so the reasons
/// and the fixes come first.
pub fn detail_with_art(app: &App, width: usize, compact: bool) -> Vec<Line<'static>> {
    let lines = detail_lines(app, width);
    if compact {
        return lines.into_iter().filter(|l| !l.spans.iter().all(|s| s.content.trim().is_empty())).collect();
    }
    let Some(r) = app.current() else { return lines };
    let pic = art(art_for(&r.node));
    let art_w = pic.iter().map(|l| l.width()).max().unwrap_or(0) + 3;
    if width < art_w + 28 {
        return lines;
    }
    let style = match r.node.kind {
        Kind::FleetBand => bold(mono::STRONG),
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

/// The finder's preview of the member or stowaway it has selected (its row
/// need not be in the list): the detail pane's lines, without the ACTIONS.
pub fn preview_lines(app: &App, row_id: &str, width: usize) -> Vec<Line<'static>> {
    let Some(b) = &app.bundle else { return vec![] };
    let now = app.now();
    if let Some(id) = row_id.strip_prefix("M:") {
        return find_member(&b.snap, &nimitz_core::Id(id.to_string())).map(|m| member_detail(app, m, now, width)).unwrap_or_default();
    }
    if let Some(sid) = row_id.strip_prefix("SW:") {
        return b.snap.stowaways.iter().find(|s| s.session_id == sid).map(|s| stowaway_detail(&b.snap, s, now, width)).unwrap_or_default();
    }
    vec![]
}

pub fn detail_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let (Some(r), Some(b)) = (app.current(), &app.bundle) else { return vec![Line::styled("loading ...", st(mono::COMMENT))] };
    let snap = &b.snap;
    let now = app.now();
    match (&r.node.kind, app.target()) {
        (Kind::FleetBand, Target::Fleet(id)) => match snap.fleets.iter().find(|f| f.id == id) {
            Some(f) => fleet_detail(f, now, width),
            None => broken_detail(snap, &id, width),
        },
        (Kind::Note, _) => match &r.node.member {
            Some(id) => broken_detail(snap, id, width),
            None => vec![],
        },
        (Kind::SquadronBand, Target::Squadron(id)) => {
            match snap.fleets.iter().flat_map(|f| f.squadrons.iter().map(move |s| (f, s))).find(|(_, s)| s.id == id) {
                Some((f, sq)) => squadron_detail(f, sq, width),
                None => broken_detail(snap, &id, width),
            }
        }
        (Kind::Member(_), Target::Member(id)) => match find_member(snap, &id) {
            Some(m) => member_detail(app, m, now, width),
            None => vec![],
        },
        (Kind::Stowaway, Target::Stowaway(sid)) => match snap.stowaways.iter().find(|s| s.session_id == sid) {
            Some(s) => stowaway_detail(snap, s, now, width),
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
            Line::styled("STOWAWAYS", bold(mono::WARN)),
            blank(),
            Line::styled("Sessions in a fleet's folders that carry no NIMITZ_* labels.", st(mono::SOFT)),
            Line::styled("nimitz did not start them, so nothing binds them to an identity.", st(mono::SOFT)),
            blank(),
            Line::styled("On a stowaway: d adopt it as a ship, x rm its job row,", st(mono::SOFT)),
            Line::styled("A attach to it. I hides this group.", st(mono::SOFT)),
        ],
        (Kind::CrewFold, _) => vec![
            Line::styled(r.node.name.clone(), bold(mono::FG)),
            blank(),
            kv("last write", format!("{} ago", r.node.age)),
            blank(),
            Line::styled(format!("Crew of this conversation done over {} min ago.", (crate::view::CREW_RECENT_S / 60.0) as u64), st(mono::SOFT)),
            Line::styled("z shows or hides them; the choice is kept per member.", st(mono::SOFT)),
        ],
        (Kind::Workflow, _) => vec![
            Line::styled(r.node.name.clone(), bold(mono::FG)),
            blank(),
            kv("state", r.node.status.0.split_once(' ').map(|(_, w)| w.to_string()).unwrap_or_default()),
            kv("agents", r.node.detail.first().map(|s| s.content.to_string()).unwrap_or_default()),
            kv("run", r.node.detail.get(1).map(|s| s.content.trim().to_string()).unwrap_or_default()),
            kv("last write", format!("{} ago", r.node.age)),
            blank(),
            Line::styled("The agents a Workflow run started in this session; z opens or closes it.", st(mono::COMMENT)),
        ],
        (Kind::Crew, _) => {
            let mut v = vec![Line::styled(r.node.name.clone(), bold(mono::FG)), blank()];
            v.extend(r.node.facts.iter().filter(|(_, x)| !x.is_empty()).map(|(k, x)| kv(k, x.clone())));
            v.push(kv("last write", format!("{} ago", r.node.age)));
            v.push(blank());
            v.push(Line::styled("Crew run inside their session; reach them through it.", st(mono::COMMENT)));
            v
        }
        (Kind::Task, _) => {
            let mut v = vec![Line::styled(r.node.name.clone(), bold(mono::FG)), blank()];
            v.extend(r.node.facts.iter().filter(|(_, x)| !x.is_empty()).map(|(k, x)| kv(k, x.clone())));
            v
        }
        _ => vec![],
    }
}

/// A fleet or squadron whose config does not load: the errors in full.
fn broken_detail(snap: &Snapshot, id: &nimitz_core::Id, width: usize) -> Vec<Line<'static>> {
    let fleet = id.role() == nimitz_core::Role::Admiral;
    let errs: Vec<&Issue> = crate::view::all_issues(snap)
        .into_iter()
        .filter(|i| i.code == "config.invalid" && !i.message.contains("which no config declares"))
        .filter(|i| i.id.as_ref().map(|x| x == id || (fleet && x.fleet() == id.fleet())).unwrap_or(false))
        .collect();
    let mut v = vec![
        Line::styled(format!("{} {}", if fleet { "FLEET" } else { "SQUADRON" }, id.0), bold(mono::ERROR)),
        blank(),
        Line::styled("Its config does not load, so nimitz cannot show or start its members.", st(mono::SOFT)),
        Line::styled("Their sessions keep running; fix the file and the frame comes back.", st(mono::SOFT)),
        blank(),
        heading(&format!("CONFIG ERRORS ({})", errs.len())),
    ];
    for e in &errs {
        for l in e.message.lines() {
            v.extend(wrap(vec![Span::raw("  "), Span::styled(l.to_string(), st(mono::FG))], width, 2));
        }
        if let Some(fx) = &e.fix {
            v.extend(wrap(vec![Span::styled("  fix  ", st(mono::COMMENT)), Span::styled(fx.clone(), bold(mono::ACCENT))], width, 7));
        }
    }
    let n = crate::view::unlisted_sessions(snap, id);
    if n > 0 {
        v.push(blank());
        v.push(Line::styled(format!("{n} session(s) are labelled for {} but outside the config; `i` lists them.", id.0), bold(mono::WARN)));
    }
    v
}

fn issues_block(title: &str, issues: &[&Issue], width: usize) -> Vec<Line<'static>> {
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
        v.extend(wrap(vec![Span::raw(" ".repeat(4)), Span::styled(i.message.clone(), st(mono::FG))], width, 4));
        if let Some(fx) = &i.fix {
            v.extend(wrap(vec![Span::raw(" ".repeat(4)), Span::styled("fix  ", st(mono::COMMENT)), Span::styled(fx.clone(), bold(mono::ACCENT))], width, 9));
        }
    }
    v
}

fn fleet_detail(f: &Fleet, now: f64, width: usize) -> Vec<Line<'static>> {
    let state = FleetState::of(f);
    let members = fleet_members(f);
    let live = members.iter().filter(|m| crate::view::member_live(m)).count();
    let mut v = vec![
        Line::from(vec![Span::styled(format!("FLEET [{}]", f.id.0), bold(mono::STRONG))]),
        blank(),
        Line::from(vec![Span::styled(format!("{:<KEY_W$}", "state"), st(mono::COMMENT)), Span::styled(state.label().to_uppercase(), bold(state.color()))]),
        kv("machine", f.machine.clone().unwrap_or_else(|| "not in fleet.toml machines: does not run here".into())),
        kv("folder", f.dir.clone()),
        kv("admiral", format!("{}  {}{}", f.admiral.expected_name, status_word(f.admiral.status), member_age(&f.admiral, now))),
        kv("squadrons", f.squadrons.iter().map(|s| s.id.squadron().unwrap_or("?").to_string()).collect::<Vec<_>>().join(", ")),
        kv("members", format!("{} declared, {live} live", members.len())),
    ];
    v.extend(issues_block("FLEET ISSUES", &f.issues.iter().collect::<Vec<_>>(), width));
    let all = fleet_issues(f);
    if all.len() > f.issues.len() {
        v.push(blank());
        v.push(Line::styled(format!("{} more issues on its members; `i` lists them all.", all.len() - f.issues.len()), st(mono::SOFT)));
    }
    v
}

fn squadron_detail(f: &Fleet, sq: &Squadron, width: usize) -> Vec<Line<'static>> {
    let state = FleetState::of_squadron(f, sq);
    let mut v = vec![
        Line::styled(format!("SQUADRON {}", sq.id.0), bold(mono::FG)),
        blank(),
        Line::from(vec![Span::styled(format!("{:<KEY_W$}", "state"), st(mono::COMMENT)), Span::styled(state.squadron_label().to_uppercase(), bold(state.color()))]),
        kv("commodore", format!("{}  {}", sq.commodore.expected_name, status_word(sq.commodore.status))),
        kv("ships", sq.ships.iter().map(|s| s.id.parts().last().unwrap_or(&"?").to_string()).collect::<Vec<_>>().join(", ")),
    ];
    v.extend(issues_block("SQUADRON ISSUES", &sq.issues.iter().collect::<Vec<_>>(), width));
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
        Some(t) => format!("  for {}", crate::view::age_or_unknown(now - t)),
        None if s.is_some() => "  since ? (no believable time)".into(),
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
            Span::styled(s.name.clone(), bold(mono::WARN)),
            Span::styled(format!("   expected {e}"), st(mono::SOFT)),
        ])),
        _ => v.push(kv("name", s.name.clone())),
    }
    v.push(kv("cwd", s.cwd.clone()));
    if let Some(t) = s.started_at {
        v.push(kv("started", format!("{} ago", crate::view::age_or_unknown(now - t))));
    }
    if s.labels.is_empty() {
        v.push(Line::from(vec![Span::styled(format!("{:<KEY_W$}", "labels"), st(mono::COMMENT)), Span::styled("none", bold(mono::WARN))]));
    } else {
        let id = s.labels.get("NIMITZ_ID").cloned().unwrap_or_default();
        v.push(kv("labels", format!("NIMITZ_ID={id}  (+{} more)", s.labels.len().saturating_sub(1))));
    }
    v
}

fn member_detail(app: &App, m: &Member, now: f64, width: usize) -> Vec<Line<'static>> {
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
        v.push(Line::styled("NEEDS YOU", bold(mono::NEEDS)));
        v.push(Line::styled(format!("  {w}"), st(mono::FG)));
    }
    if !m.reasons.is_empty() {
        v.push(blank());
        v.push(heading("WHY THIS STATUS"));
        for r in &m.reasons {
            v.extend(wrap(vec![Span::styled("  \u{00b7} ", st(mono::COMMENT)), Span::styled(r.clone(), st(mono::SOFT))], width, 4));
        }
    }
    v.extend(issues_block("ISSUES", &m.issues.iter().collect::<Vec<_>>(), width));
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
                Span::styled(format!("  {:<10}", x.short_id.clone().unwrap_or_default()), if live { bold(mono::ERROR) } else { st(mono::SOFT) }),
                Span::styled(if live { "live copy".into() } else { x.state.clone().unwrap_or("stopped".into()) }, if live { bold(mono::ERROR) } else { st(mono::COMMENT) }),
            ]));
        }
    }
    if !m.crew.is_empty() {
        use crate::view::{crew_work, Work};
        // how many in each state, the busy first; zero counts left out
        let counts: Vec<String> = [Work::Busy, Work::Failed, Work::Idle, Work::Done]
            .into_iter()
            .map(|w| (w, m.crew.iter().filter(|c| crew_work(c) == w).count()))
            .filter(|(_, n)| *n > 0)
            .map(|(w, n)| format!("{n} {}", w.word()))
            .collect();
        v.push(blank());
        let runs: std::collections::BTreeSet<&str> = m.crew.iter().filter_map(|c| c.workflow.as_deref()).collect();
        let wf = if runs.is_empty() { String::new() } else { format!(" ({} in {} workflow run{})", m.crew.iter().filter(|c| c.workflow.is_some()).count(), runs.len(), if runs.len() == 1 { "" } else { "s" }) };
        v.push(heading(&format!("CREW  {}{wf}", counts.join(", "))));
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

fn stowaway_detail(snap: &Snapshot, s: &SessionInfo, now: f64, width: usize) -> Vec<Line<'static>> {
    let mut v = vec![
        Line::styled(if s.name.is_empty() { s.session_id.clone() } else { s.name.clone() }, bold(mono::WARN)),
        blank(),
        Line::from(vec![
            Span::styled(format!("{:<KEY_W$}", "status"), st(mono::COMMENT)),
            Span::styled(if s.pid.is_some() { "LIVE STOWAWAY" } else { "STOPPED STOWAWAY" }, bold(mono::WARN)),
        ]),
        Line::styled("A session in fleet scope with no NIMITZ_* labels.", st(mono::SOFT)),
    ];
    let issues: Vec<&Issue> = crate::view::stowaway_issues(snap, s);
    if issues.iter().any(|i| i.code == "legacy") {
        v.push(Line::styled("Its name says it belongs to an old fleet-skill fleet: adopt it to bring it under nimitz.", st(mono::WARN)));
    }
    v.extend(issues_block("ISSUES", &issues, width));
    v.push(blank());
    v.push(heading("SESSION"));
    v.extend(session_lines(s, now, None));
    v
}

fn extra_detail(owner: &str, s: &SessionInfo, now: f64) -> Vec<Line<'static>> {
    let live = s.pid.is_some();
    let mut v = vec![
        Line::styled(format!("job row {}", s.short_id.clone().unwrap_or_default()), bold(if live { mono::ERROR } else { mono::FG })),
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

/// The last line of the detail under `x`: what enter on the row does (the
/// ACTIONS block's first line before 0.7.8). None on a row that only folds.
pub fn jump_hint(app: &App) -> Option<Line<'static>> {
    let n = &app.current()?.node;
    let focus = app.bundle.as_ref().map(|b| b.mux_caps.focus).unwrap_or(false);
    let session = matches!(n.kind, Kind::Member(_) | Kind::Stowaway);
    let (text, style) = match (&n.pane_id, focus) {
        (Some(p), true) => (format!("jump to its pane {p}"), st(mono::FG)),
        (_, false) if session => (crate::actions::MONITOR_ONLY.to_string(), st(mono::WARN)),
        (None, true) if session => ("no pane hosts this session".to_string(), st(mono::COMMENT)),
        _ => return None,
    };
    Some(Line::from(vec![Span::styled("\u{23ce} on the row  ".to_string(), bold(mono::ACCENT)), Span::styled(text, style)]))
}

/// Style for a `[demo]` / `[monitor-only]` style badge.
pub fn badge(text: &str, bg: ratatui::style::Color) -> Span<'static> {
    Span::styled(format!(" {text} "), Style::new().fg(mono::BG).bg(bg).add_modifier(Modifier::BOLD))
}
