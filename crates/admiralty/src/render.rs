//! Drawing: top bar, the frame list, the detail pane, the footer, and every
//! modal (help, confirm with plan steps, menus, issues, needs-you, message,
//! forms). Reads `App`, never changes it except scroll and page size.

use std::time::Duration;

use nimitz_core::{Plan, Severity, Step};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::actions::{shell, MessagePlan, Pending};
use crate::app::{App, MenuItem, Modal, NeedItem, Split, HELP};
use crate::detail::{badge, detail_with_art};
use crate::forms::{FieldKind, Form};
use crate::frames::{frame_lines, header_line, list_max_width, pad, selectable, truncate};
use crate::theme::{bold, mono, severity_color, severity_word, st};
use crate::view::FleetState;

/// Split the body: list and detail side by side when wide, else stacked.
pub fn body_areas(app: &App, body: Rect) -> (Rect, Rect, bool) {
    let side = match app.split {
        Split::Vertical => true,
        Split::Horizontal => false,
        Split::Auto => body.width >= 150,
    };
    if side {
        let w = list_max_width().min(body.width * 62 / 100).max(body.width.min(60));
        let [l, r] = Layout::horizontal([Constraint::Length(w), Constraint::Fill(1)]).areas(body);
        (l, r, true)
    } else {
        let [l, r] = Layout::vertical([Constraint::Percentage(60), Constraint::Fill(1)]).areas(body);
        (l, r, false)
    }
}

pub fn draw(app: &mut App, f: &mut Frame) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::new().bg(mono::BG).fg(mono::FG)), area);
    let [top, body, bottom] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    f.render_widget(Paragraph::new(top_line(app)).style(Style::new().bg(mono::BG_HI)), top);
    let (left, right, _) = body_areas(app, body);
    draw_list(app, f, left);
    let detail = detail_with_art(app, right.width.saturating_sub(2) as usize);
    f.render_widget(
        Paragraph::new(detail).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(st(mono::PURPLE))
                .title(Span::styled(" detail ", bold(mono::PURPLE))),
        ),
        right,
    );
    f.render_widget(Paragraph::new(bottom_line(app)).style(Style::new().bg(mono::BG_HI)), bottom);
    match &app.modal {
        Some(Modal::Help) => draw_help(f, area),
        Some(Modal::Confirm(p)) => draw_confirm(f, area, p, app.ctx.live),
        Some(Modal::Needs { items, sel }) => draw_needs(f, area, items, *sel),
        Some(Modal::Issues { sel }) => draw_issues(app, f, area, *sel),
        Some(Modal::Menu { title, items, sel }) => draw_menu(f, area, title, items, *sel),
        Some(Modal::Message { plan, text }) => draw_message(f, area, plan, text),
        Some(Modal::FleetFilter { sel }) => draw_fleet_filter(f, area, &app.fleet_states, *sel),
        Some(Modal::Form(form)) => draw_form(f, area, form),
        None => {}
    }
}

fn top_line(app: &App) -> Line<'static> {
    let Some(b) = &app.bundle else { return Line::from(vec![badge("ADMIRALTY", mono::GOLD), Span::raw("  loading ...")]) };
    let (fleets, live, need, errors, warns) = app.counts();
    let mut v = vec![badge("\u{2693} ADMIRALTY", mono::GOLD), Span::raw(format!("  {fleets} fleets  {live} live  "))];
    v.push(Span::styled(format!("{need} need you"), if need > 0 { bold(mono::PINK) } else { st(mono::COMMENT) }));
    v.push(Span::raw("   "));
    if errors + warns == 0 {
        v.push(Span::styled("no issues", st(mono::GREEN)));
    } else {
        if errors > 0 {
            v.push(Span::styled(format!("{} {errors} errors", app.icons.issue), bold(mono::PINK)));
            v.push(Span::raw("  "));
        }
        if warns > 0 {
            v.push(Span::styled(format!("{warns} warnings"), bold(mono::ORANGE)));
        }
        v.push(Span::styled("  (i)", st(mono::COMMENT)));
    }
    v.push(Span::raw("   "));
    if app.monitor_only() {
        v.push(badge("MONITOR-ONLY: no multiplexer", mono::ORANGE));
    } else {
        v.push(Span::styled(format!("mux {}", b.mux_name), st(mono::CYAN)));
    }
    if !app.ctx.live {
        v.push(Span::raw(" "));
        v.push(badge("DEMO DATA", mono::PURPLE));
    }
    if !b.snap.compat_ok {
        let ver = b.snap.cc_version.clone().unwrap_or("not found".into());
        v.push(Span::styled(format!("   Claude Code {ver} outside the supported range"), bold(mono::ORANGE)));
    }
    if !app.filter.is_empty() {
        v.push(Span::styled(format!("   filter: {}", app.filter), st(mono::YELLOW)));
    }
    if app.flagged_only {
        v.push(Span::styled("   flagged only", st(mono::PURPLE)));
    }
    if app.fleet_states.iter().any(|on| !on) {
        let shown: Vec<&str> = FleetState::ALL.iter().filter(|x| app.fleet_states[x.index()]).map(|x| x.label()).collect();
        v.push(Span::styled(format!("   fleets: {}", if shown.is_empty() { "none".into() } else { shown.join(", ") }), st(mono::ORANGE)));
    }
    Line::from(v)
}

fn bottom_line(app: &App) -> Line<'static> {
    if app.editing_filter {
        return Line::from(vec![
            Span::styled(" /", bold(mono::YELLOW)),
            Span::raw(app.filter.clone()),
            Span::styled("\u{2588}", st(mono::YELLOW)),
            Span::styled("   enter keep, esc clear", st(mono::COMMENT)),
        ]);
    }
    if let Some(b) = &app.busy {
        return Line::styled(format!(" running: {b}"), bold(mono::YELLOW));
    }
    if let Some(t) = &app.toast {
        if t.at.elapsed() < Duration::from_secs(8) {
            return Line::from(Span::styled(format!(" {}", t.msg), t.style));
        }
    }
    let keys = [
        ("j/k", "move"),
        ("h/l", "fold"),
        ("\u{23ce}", "pane"),
        ("!", "needs you"),
        ("i", "issues"),
        ("a", "start"),
        ("x", "actions"),
        ("m", "message"),
        ("c", "create"),
        ("/", "filter"),
        ("f/F", "filters"),
        ("I", "stowaways"),
        ("v/s", "split"),
        ("?", "help"),
        ("q", "quit"),
    ];
    let mut v = vec![];
    for (k, d) in keys {
        v.push(Span::styled(format!(" {k} "), bold(mono::YELLOW)));
        v.push(Span::styled(format!("{d} "), st(mono::SOFT)));
    }
    Line::from(v)
}

fn draw_list(app: &mut App, f: &mut Frame, area: Rect) {
    let lines = frame_lines(&app.rows, area.width, app.cursor, app.icons);
    let view = area.height.saturating_sub(1) as usize;
    app.page = view.max(2);
    if let Some(pos) = lines.iter().position(|(_, r)| *r == Some(app.cursor)) {
        if pos < app.scroll {
            app.scroll = pos;
        } else if pos >= app.scroll + view {
            app.scroll = pos + 1 - view;
        }
    }
    app.scroll = app.scroll.min(lines.len().saturating_sub(view));
    let mut shown = vec![header_line(area.width)];
    if lines.is_empty() {
        shown.push(Line::raw(""));
        let msg = if app.bundle.as_ref().map(|b| b.snap.fleets.is_empty()).unwrap_or(false) {
            "  No fleets yet.  c  then  f  creates one (or `nimitz fleet new`)."
        } else {
            "  Nothing matches the filters (esc clears /, f shows fleet states, F flagged)."
        };
        shown.push(Line::styled(msg, st(mono::SOFT)));
    }
    shown.extend(lines.into_iter().skip(app.scroll).take(view).map(|(l, _)| l));
    f.render_widget(Paragraph::new(shown), area);
}

// ------------------------------------------------------------------ modals

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn modal_block(title: String, color: ratatui::style::Color) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Thick)
        .border_style(st(color))
        .style(Style::new().bg(mono::BG))
        .title(Span::styled(format!(" {title} "), bold(color)))
}

fn keys_line(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut v = vec![];
    for (k, d) in pairs {
        v.push(Span::styled(k.to_string(), bold(mono::YELLOW)));
        v.push(Span::styled(format!(" {d}   "), st(mono::SOFT)));
    }
    Line::from(v)
}

fn draw_help(f: &mut Frame, area: Rect) {
    let mut lines = vec![Line::styled("Keys (vim style)", bold(mono::FG)), Line::raw("")];
    for (k, d) in HELP {
        lines.push(Line::from(vec![Span::styled(format!("  {k:<18}"), bold(mono::CYAN)), Span::styled(d.to_string(), st(mono::FG))]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("  Pane dots: yellow working, red blocked, teal done, green idle, grey unknown.", st(mono::COMMENT)));
    let r = centered(area, 86, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).block(modal_block("help".into(), mono::CYAN)), r);
}

/// One plan step: what runs, where, and why.
pub fn step_lines(s: &Step, pane: bool) -> Vec<Line<'static>> {
    let what = match s.cmd.first().map(String::as_str) {
        Some("write") => format!("write {}  ({} lines)", s.cmd.get(1).cloned().unwrap_or_default(), s.cmd.get(2).map(|c| c.lines().count()).unwrap_or(0)),
        Some("mkdir" | "archive" | "bind") => s.cmd.join(" "),
        _ => format!("$ {}", shell(&s.cmd)),
    };
    let mut first = vec![Span::styled("  ", Style::new())];
    if pane {
        first.push(Span::styled("[pane] ", bold(mono::CYAN)));
    }
    first.push(Span::styled(what, st(mono::FG)));
    let mut v = vec![Line::from(first)];
    let mut meta = vec![Span::raw("      "), Span::styled(s.why.clone(), st(mono::COMMENT))];
    if let Some(c) = &s.cwd {
        meta.push(Span::styled(format!("   in {c}"), st(mono::DIM)));
    }
    v.push(Line::from(meta));
    v
}

pub fn plan_lines(p: &Plan, pane_steps: &[Step]) -> Vec<Line<'static>> {
    let mut v = vec![];
    if !p.refusals.is_empty() {
        v.push(Line::styled("REFUSED", bold(mono::PINK)));
        for i in &p.refusals {
            v.push(Line::from(vec![Span::styled(format!("  {:<16}", i.code), bold(mono::PINK)), Span::styled(i.message.clone(), st(mono::FG))]));
            if let Some(fx) = &i.fix {
                v.push(Line::from(vec![Span::raw("      fix  "), Span::styled(fx.clone(), bold(mono::CYAN))]));
            }
        }
        v.push(Line::raw(""));
    }
    if !p.warnings.is_empty() {
        for i in &p.warnings {
            v.push(Line::from(vec![Span::styled(format!("  warn {:<11}", i.code), bold(mono::ORANGE)), Span::styled(i.message.clone(), st(mono::SOFT))]));
        }
        v.push(Line::raw(""));
    }
    v.push(Line::styled(format!("STEPS ({})", p.steps.len()), bold(mono::PURPLE)));
    if p.steps.is_empty() {
        v.push(Line::styled("  nothing to do", st(mono::COMMENT)));
    }
    for s in &p.steps {
        v.extend(step_lines(s, pane_steps.contains(s)));
    }
    v
}

fn draw_confirm(f: &mut Frame, area: Rect, p: &Pending, live: bool) {
    let mut lines = vec![
        Line::styled(format!("{}?", p.plan.summary), bold(mono::FG)),
        Line::from(vec![Span::styled("command  ", st(mono::COMMENT)), Span::styled(p.command.clone(), bold(mono::CYAN))]),
        Line::raw(""),
    ];
    let pane_steps: Vec<Step> = p.panes.iter().map(|(s, _)| s.clone()).collect();
    lines.extend(plan_lines(&p.plan, &pane_steps));
    lines.push(Line::raw(""));
    if !p.plan.ok() {
        lines.push(keys_line(&[("esc", "close (core refuses this plan; the fixes are above)")]));
    } else if !live {
        lines.push(keys_line(&[("y", "show what would happen (demo data: nothing runs)"), ("n", "cancel")]));
    } else {
        lines.push(keys_line(&[("y", "run"), ("n", "cancel")]));
    }
    let color = if p.plan.ok() { mono::YELLOW } else { mono::PINK };
    let h = (lines.len() as u16 + 4).min(area.height.saturating_sub(2));
    let r = centered(area, 110, h);
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(modal_block(p.act.word().to_string(), color)), r);
}

fn draw_fleet_filter(f: &mut Frame, area: Rect, on: &[bool; 7], sel: usize) {
    let mut lines = vec![Line::styled("Show fleets that are", bold(mono::FG)), Line::raw("")];
    for (i, s) in FleetState::ALL.iter().enumerate() {
        let mut l = Line::from(vec![
            Span::styled(if on[i] { " [x] " } else { " [ ] " }, bold(if on[i] { mono::GREEN } else { mono::SOFT })),
            Span::styled("  ", Style::new().bg(s.tint())),
            Span::styled(format!(" {}", s.label()), st(s.color())),
        ]);
        if i == sel {
            l = l.style(Style::new().bg(mono::SEL));
        }
        lines.push(l);
    }
    lines.push(Line::raw(""));
    lines.push(keys_line(&[("space", "toggle"), ("a", "all"), ("esc", "close")]));
    let r = centered(area, 46, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).block(modal_block("fleet filter".into(), mono::ORANGE)), r);
}

fn draw_needs(f: &mut Frame, area: Rect, items: &[NeedItem], sel: usize) {
    let mut lines = vec![keys_line(&[("j/k", "move"), ("enter", "go to it"), ("esc", "back")]), Line::raw("")];
    if items.is_empty() {
        lines.push(Line::styled("  nothing is waiting on you", st(mono::COMMENT)));
    }
    for (i, it) in items.iter().enumerate() {
        let mut line = Line::from(vec![
            Span::styled(format!("  {:<30}", it.name), bold(mono::PINK)),
            Span::styled(format!("{:<6}", it.age), st(mono::COMMENT)),
            Span::styled(it.text.replace('\n', " "), st(mono::FG)),
        ]);
        if i == sel {
            line = line.style(Style::new().bg(mono::SEL));
        }
        lines.push(line);
    }
    let r = centered(area, area.width.saturating_sub(8), (items.len() as u16 + 5).max(7));
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).block(modal_block("waiting on you".into(), mono::PINK)), r);
}

fn draw_issues(app: &App, f: &mut Frame, area: Rect, sel: usize) {
    let issues = app.issues();
    let r = centered(area, area.width.saturating_sub(6), area.height.saturating_sub(4));
    let inner_h = r.height.saturating_sub(2) as usize;
    let mut lines = vec![keys_line(&[("j/k", "move"), ("enter", "go to the row"), ("y", "yank the fix"), ("esc", "back")]), Line::raw("")];
    if issues.is_empty() {
        lines.push(Line::styled("  No issues. Every fleet's shape matches its config.", st(mono::GREEN)));
    }
    let width = r.width.saturating_sub(4) as usize;
    let mut body: Vec<Line<'static>> = vec![];
    let mut sel_line = 0;
    for (i, is) in issues.iter().enumerate() {
        if i == sel {
            sel_line = body.len();
        }
        let c = severity_color(is.severity);
        let wh = is.id.as_ref().map(|x| x.0.clone()).unwrap_or_else(|| if is.code == "stowaway" { "stowaways".into() } else { "this machine".into() });
        let mut l = Line::from(vec![
            Span::styled(format!(" {:<6}", severity_word(is.severity).to_uppercase()), bold(c)),
            Span::styled(format!("{:<18}", is.code), bold(c)),
            Span::styled(format!("{:<22}", truncate(&wh, 21)), st(mono::YELLOW)),
            Span::styled(truncate(&is.message, width.saturating_sub(47)), st(mono::FG)),
        ]);
        let mut fx = Line::from(vec![
            Span::raw(" ".repeat(25)),
            Span::styled("fix  ", st(mono::COMMENT)),
            match &is.fix {
                Some(x) => Span::styled(x.clone(), bold(mono::CYAN)),
                None => Span::styled("no command; see the message".to_string(), st(mono::COMMENT)),
            },
        ]);
        l = selectable(l, i == sel);
        fx = selectable(fx, i == sel);
        body.push(l);
        body.push(fx);
    }
    let room = inner_h.saturating_sub(lines.len());
    let start = if sel_line + 2 > room { sel_line + 2 - room } else { 0 };
    lines.extend(body.into_iter().skip(start).take(room));
    let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(modal_block(format!("issues: {} ({errors} errors)", issues.len()), if errors > 0 { mono::PINK } else { mono::ORANGE })),
        r,
    );
}

fn draw_menu(f: &mut Frame, area: Rect, title: &str, items: &[MenuItem], sel: usize) {
    let mut lines = vec![];
    for (i, it) in items.iter().enumerate() {
        let (ks, ls) = match &it.enabled {
            Ok(()) => (bold(mono::YELLOW), st(mono::FG)),
            Err(_) => (bold(mono::DIM), st(mono::COMMENT)),
        };
        let mut spans = vec![Span::styled(format!("  {}   ", it.key), ks), Span::styled(pad(&it.label, 46), ls)];
        if let Err(why) = &it.enabled {
            spans.push(Span::styled(why.clone(), st(mono::COMMENT)));
        }
        lines.push(selectable(Line::from(spans), i == sel));
    }
    lines.push(Line::raw(""));
    lines.push(keys_line(&[("key or enter", "choose (each confirms first)"), ("esc", "close")]));
    let r = centered(area, 120, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).block(modal_block(title.to_string(), mono::YELLOW)), r);
}

fn draw_message(f: &mut Frame, area: Rect, plan: &MessagePlan, text: &str) {
    let how = if plan.relay {
        format!("typed into {}'s pane {}, which relays it with SendMessage", plan.via, plan.pane.pane_id)
    } else {
        format!("typed into its pane {}", plan.pane.pane_id)
    };
    let lines = vec![
        Line::styled(format!("Message {}", plan.to), bold(mono::FG)),
        Line::styled(how, st(mono::COMMENT)),
        Line::raw(""),
        Line::from(vec![Span::styled("> ", bold(mono::ORANGE)), Span::styled(text.to_string(), st(mono::FG)), Span::styled("\u{2588}", st(mono::ORANGE))]),
        Line::raw(""),
        keys_line(&[("enter", "send"), ("esc", "cancel"), ("ctrl-u", "clear")]),
    ];
    let r = centered(area, 96, 9);
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(modal_block("message".into(), mono::ORANGE)), r);
}

fn draw_form(f: &mut Frame, area: Rect, form: &Form) {
    let mut lines = vec![];
    for (i, fl) in form.fields.iter().enumerate() {
        let focused = i == form.focus;
        let label = Span::styled(format!("  {:<14}", fl.label), if focused { bold(mono::YELLOW) } else { st(mono::SOFT) });
        let value = match &fl.kind {
            FieldKind::Fixed => Span::styled(fl.value.clone(), st(mono::COMMENT)),
            FieldKind::Choice(_) => Span::styled(format!("\u{2039} {} \u{203a}", fl.value), if focused { bold(mono::CYAN) } else { st(mono::FG) }),
            FieldKind::Long => {
                let first = fl.value.lines().next().unwrap_or("").to_string();
                let more = fl.value.lines().count().saturating_sub(1);
                let t = if more > 0 { format!("{first}  (+{more} lines)") } else { first };
                Span::styled(t, st(mono::FG))
            }
            FieldKind::Text => Span::styled(fl.value.clone(), st(mono::FG)),
        };
        let mut spans = vec![label, value];
        if focused && matches!(fl.kind, FieldKind::Text | FieldKind::Long) {
            spans.push(Span::styled("\u{2588}", st(mono::YELLOW)));
        }
        let mut l = Line::from(spans);
        if focused {
            l = l.style(Style::new().bg(mono::BG_HI));
        }
        lines.push(l);
        match &fl.error {
            Some(e) => lines.push(Line::styled(format!("  {:<14}\u{2717} {e}", ""), bold(mono::PINK))),
            None if focused && !fl.hint.is_empty() => lines.push(Line::styled(format!("  {:<14}{}", "", fl.hint), st(mono::COMMENT))),
            None => {}
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![Span::styled("command  ", st(mono::COMMENT)), Span::styled(form.command.clone(), bold(mono::CYAN))]));
    lines.push(Line::raw(""));
    if !form.problems.is_empty() {
        lines.push(Line::styled("CORE SAYS NO", bold(mono::PINK)));
        for p in &form.problems {
            lines.push(Line::styled(format!("  \u{2717} {p}"), st(mono::PINK)));
        }
        lines.push(Line::raw(""));
    }
    for w in &form.warnings {
        lines.push(Line::styled(format!("  warn {w}"), st(mono::ORANGE)));
    }
    if form.ok() {
        lines.extend(plan_lines(&form.plan, &[]));
        lines.push(Line::raw(""));
    }
    lines.push(keys_line(&[("tab/\u{2191}\u{2193}", "field"), ("\u{2190}\u{2192}", "choose"), ("ctrl-e", "$EDITOR"), ("ctrl-s / enter on last", "save"), ("esc", "cancel")]));
    let ok_color = if form.ok() { mono::GREEN } else { mono::PINK };
    let h = (lines.len() as u16 + 4).min(area.height.saturating_sub(2));
    let r = centered(area, 112, h);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(modal_block(form.title.clone(), ok_color).title_bottom(Span::styled(if form.ok() { " ready " } else { " fix the fields in red " }, bold(ok_color)))),
        r,
    );
}

/// For tests: the plain text of a line.
pub fn plain(l: &Line) -> String {
    l.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// For tests: a style is bold.
pub fn is_bold(s: Style) -> bool {
    s.add_modifier.contains(Modifier::BOLD)
}
