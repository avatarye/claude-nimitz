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
use unicode_width::UnicodeWidthStr;

use crate::actions::{shell, MessagePlan, Pending};
use crate::app::{App, MenuItem, Modal, NeedItem, HELP};
use crate::detail::{badge, detail_with_art};
use crate::forms::{FieldKind, Form};
use crate::frames::{frame_lines, header_line, pad, selectable, wrap};
use crate::theme::{bold, mono, severity_color, severity_word, st};
use crate::view::FleetState;

/// Where the floating detail panel sits over the list: the right part of the
/// body, full height, 45% of the width (48 to 90 columns, never the whole list).
pub fn detail_area(body: Rect) -> Rect {
    let w = (body.width * 45 / 100).clamp(48, 90).min(body.width.saturating_sub(4)).max(body.width.min(20));
    Rect { x: body.x + body.width - w, y: body.y, width: w, height: body.height }
}

pub fn draw(app: &mut App, f: &mut Frame) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::new().bg(mono::BG).fg(mono::FG)), area);
    // fleet tabs on top; the ADMIRALTY status bar sits right above the key bar
    let [tabs, body, top, bottom] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1), Constraint::Length(1)]).areas(area);
    f.render_widget(Paragraph::new(top_line(app, top.width as usize)).style(Style::new().bg(mono::BG_HI)), top);
    f.render_widget(Paragraph::new(tab_line(app, tabs.width as usize)).style(Style::new().bg(mono::BG)), tabs);
    // the list takes the whole body; `w` floats the detail panel over its right part
    draw_list(app, f, body);
    if app.detail_open {
        let right = detail_area(body);
        let compact = right.height < 16;
        let detail = detail_with_art(app, right.width.saturating_sub(2) as usize, compact);
        let room = right.height.saturating_sub(2) as usize;
        app.detail_scroll = app.detail_scroll.min(detail.len().saturating_sub(room));
        let more = detail.len() > room + app.detail_scroll;
        let title = if more || app.detail_scroll > 0 { " detail  ctrl-e/ctrl-y scroll  w/esc close " } else { " detail  w/esc close " };
        f.render_widget(Clear, right);
        f.render_widget(
            Paragraph::new(detail).scroll((app.detail_scroll as u16, 0)).wrap(Wrap { trim: false }).style(Style::new().bg(mono::BG)).block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .border_style(st(mono::ACCENT))
                    .title(Span::styled(title, bold(mono::ACCENT))),
            ),
            right,
        );
    }
    f.render_widget(Paragraph::new(bottom_line(app, bottom.width as usize)).style(Style::new().bg(mono::BG_HI)), bottom);
    match &app.modal {
        Some(Modal::Help { scroll }) => draw_help(f, area, *scroll, app.icons),
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

/// One tab per fleet, then `unassigned` when stowaways sit in no fleet's
/// folders: `1 HOME`, with `!n` where members need you. The shown tab is
/// inverted. A tab that does not fit is cut whole; the shown one always shows.
fn tab_line(app: &App, width: usize) -> Line<'static> {
    let Some(b) = &app.bundle else { return Line::default() };
    let need = |tab: &str| -> usize {
        b.snap.fleets.iter().filter(|f| f.id.0 == tab).flat_map(crate::view::fleet_members).filter(|m| m.status == nimitz_core::Status::NeedsYou).count()
    };
    let tabs = crate::view::tabs(&b.snap);
    let mut parts: Vec<(Vec<Span<'static>>, bool)> = vec![];
    let searching = !app.filter.trim().is_empty();
    if searching {
        parts.push((vec![Span::styled(" search: every fleet ".to_string(), Style::new().fg(mono::BG).bg(mono::WARN).add_modifier(Modifier::BOLD)), Span::raw(" ")], true));
    }
    for (i, t) in tabs.iter().enumerate() {
        let on = !searching && app.tab.as_deref() == Some(t.as_str());
        let key = if i < 9 { format!("{} ", i + 1) } else { String::new() };
        let style = if on { Style::new().fg(mono::BG).bg(mono::ACCENT).add_modifier(Modifier::BOLD) } else { st(mono::SOFT) };
        let mut sp = vec![Span::styled(format!(" {key}{} ", crate::view::tab_title(t)), style)];
        let n = need(t);
        if n > 0 {
            sp.push(Span::styled(format!("!{n} "), bold(mono::NEEDS)));
        }
        sp.push(Span::raw(" "));
        parts.push((sp, on));
    }
    let w = |v: &[Span]| v.iter().map(|s| s.content.width()).sum::<usize>();
    let mut used = parts.iter().filter(|(_, on)| *on).map(|(sp, _)| w(sp)).sum::<usize>();
    let mut out = vec![];
    for (sp, on) in parts {
        let n = w(&sp);
        if on || used + n <= width {
            if !on {
                used += n;
            }
            out.extend(sp);
        }
    }
    Line::from(out)
}

fn top_line(app: &App, width: usize) -> Line<'static> {
    let Some(b) = &app.bundle else { return Line::from(vec![badge("ADMIRALTY", mono::STRONG), Span::raw("  loading ...")]) };
    // the longest form whose essentials fit: words, then short words, then
    // letters; optional pieces follow while whole ones fit, never cut mid-word
    let w = |v: &[Span]| v.iter().map(|s| s.content.width()).sum::<usize>();
    let level = (0..3).find(|l| w(&top_spans(app, b, *l).0) <= width).unwrap_or(2);
    let (essential, optional) = top_spans(app, b, level);
    let mut out = vec![];
    let mut used = 0;
    for sp in essential {
        let n = sp.content.width();
        if used + n > width {
            break;
        }
        used += n;
        out.push(sp);
    }
    for sp in optional {
        let n = sp.content.width();
        if used + n <= width {
            used += n;
            out.push(sp);
        }
    }
    Line::from(out)
}

/// What hides rows right now, and the key that undoes it; `None` when nothing does.
fn filter_marker(app: &App, level: u8) -> Option<String> {
    let mut what: Vec<String> = vec![];
    let mut keys: Vec<&str> = vec![];
    if !app.filter.is_empty() {
        what.push(format!("\"{}\"", app.filter));
        keys.push("esc");
    }
    if app.flagged_only {
        what.push("flagged".into());
        keys.push("F");
    }
    let hidden = app.fleet_states.iter().filter(|on| !**on).count();
    if hidden > 0 {
        what.push(match level {
            0 => {
                let off: Vec<&str> = FleetState::ALL.iter().filter(|x| !app.fleet_states[x.index()]).map(|x| x.label()).collect();
                format!("no {}", off.join(", "))
            }
            _ => format!("{hidden} states"),
        });
        keys.push("f");
    }
    if !app.show_stowaways {
        what.push("no stowaways".into());
        keys.push("I");
    }
    if what.is_empty() {
        return None;
    }
    Some(match level {
        2 => format!(" FILTERED ({}) ", keys.join(" ")),
        _ => format!(" filtered: {} ({} clears) ", what.join(", "), keys.join("/")),
    })
}

/// The top bar at a level of detail (0 words, 1 short words, 2 letters), as
/// (what always shows, what shows when there is room).
fn top_spans(app: &App, b: &crate::data::Bundle, level: u8) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let (fleets, live, need, errors, warns) = app.counts();
    let pick = |full: String, short: String, tiny: String| match level {
        0 => full,
        1 => short,
        _ => tiny,
    };
    // what decides what you can do comes first, so a narrow window keeps it
    let mut v = vec![badge(&format!("{} ADMIRALTY", app.icons.fleet), mono::STRONG)];
    if app.monitor_only() {
        v.push(badge(if level == 2 { "NO MUX" } else { "MONITOR-ONLY" }, mono::WARN));
    } else {
        v.push(Span::styled(format!(" mux {}", b.mux_name), bold(mono::MUTED)));
    }
    if !app.ctx.live {
        v.push(Span::raw(" "));
        v.push(badge("DEMO", mono::MUTED));
    }
    // a filter (maybe saved from the last run) hides rows: always say so, before the counts
    if let Some(f) = filter_marker(app, level) {
        v.push(Span::raw(" "));
        v.push(Span::styled(f, Style::new().fg(mono::BG).bg(mono::WARN).add_modifier(Modifier::BOLD)));
    }
    v.push(Span::raw(pick(format!("  {fleets} fleets  {live} live  "), format!(" {fleets} fleets {live} live "), format!(" {fleets}f {live} live "))));
    // sessions labelled for a fleet whose config does not load: not lost, just not listed
    let unlisted = crate::view::all_issues(&b.snap).iter().filter(|i| i.code == "config.invalid" && i.message.contains("which no config declares")).count();
    if unlisted > 0 {
        v.push(Span::styled(pick(format!("+{unlisted} outside the config  "), format!("+{unlisted} unlisted  "), format!("+{unlisted}? ")), bold(mono::WARN)));
    }
    v.push(Span::styled(pick(format!("{need} need you"), format!("{need} need you"), format!("{need}!")), if need > 0 { crate::theme::chip(mono::NEEDS) } else { st(mono::COMMENT) }));
    v.push(Span::raw(if level == 0 { "  " } else { " " }));
    if errors + warns == 0 {
        v.push(Span::styled(pick("no issues".into(), "no issues".into(), "ok".into()), st(mono::MUTED)));
    } else {
        if errors > 0 {
            v.push(Span::styled(pick(format!("{} {errors} errors", app.icons.issue), format!("{} {errors} err", app.icons.issue), format!("{errors}e")), bold(mono::ERROR)));
            v.push(Span::raw(if level == 0 { "  " } else { " " }));
        }
        if warns > 0 {
            v.push(Span::styled(pick(format!("{warns} warnings"), format!("{warns} warn"), format!("{warns}w")), bold(mono::WARN)));
        }
        if level == 0 {
            v.push(Span::styled(" (i)", st(mono::COMMENT)));
        }
    }
    let mut opt = vec![];
    if !b.snap.compat_ok {
        let ver = b.snap.cc_version.clone().unwrap_or("not found".into());
        let c = nimitz_core::compat::supported();
        let newer = !ver.contains('-') && nimitz_core::compat::position(&c, &ver) == 1;
        let (full, short) = if newer {
            (format!("   Claude Code {ver} newer than tested ({})", c.max), format!("   CC {ver} untested"))
        } else {
            (format!("   Claude Code {ver} outside the supported range"), format!("   CC {ver} unsupported"))
        };
        opt.push(Span::styled(pick(full, short, " CC!".into()), bold(if newer { mono::WARN } else { mono::ERROR })));
    }
    (v, opt)
}

fn bottom_line(app: &App, width: usize) -> Line<'static> {
    if app.editing_filter {
        return Line::from(vec![
            Span::styled(" /", bold(mono::ACCENT)),
            Span::raw(app.filter.clone()),
            Span::styled("\u{2588}", st(mono::ACCENT)),
            Span::styled("   enter keep, esc clear", st(mono::COMMENT)),
        ]);
    }
    if let Some(b) = &app.busy {
        return Line::styled(format!(" running: {b}"), bold(mono::BUSY));
    }
    if let Some(t) = &app.toast {
        if t.at.elapsed() < Duration::from_secs(8) {
            return Line::from(Span::styled(format!(" {}", t.msg), t.style));
        }
    }
    let key = |k: &str, d: &str| vec![Span::styled(format!(" {k} "), bold(mono::ACCENT)), Span::styled(format!("{d} "), st(mono::SOFT))];
    // `? help` first and `q quit` at the right, at every width; the rest fills what is left
    let right: Vec<Span<'static>> = key("q", "quit");
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let keys = [
        ("?", "help"),
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
        ("w", "detail"),
    ];
    let mut v: Vec<Span<'static>> = vec![];
    let mut used = 0;
    for (k, d) in keys {
        let item = key(k, d);
        let w: usize = item.iter().map(|s| s.content.width()).sum();
        if used + w + right_w > width && used > 0 {
            break;
        }
        used += w;
        v.extend(item);
    }
    v.push(Span::raw(" ".repeat(width.saturating_sub(used + right_w))));
    v.extend(right);
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
        v.push(Span::styled(k.to_string(), bold(mono::ACCENT)));
        v.push(Span::styled(format!(" {d}   "), st(mono::SOFT)));
    }
    Line::from(v)
}

fn draw_help(f: &mut Frame, area: Rect, scroll: usize, icons: &crate::theme::Icons) {
    let r = centered(area, 90, HELP.len() as u16 + 32);
    let width = r.width.saturating_sub(2) as usize;
    let mut lines = vec![Line::styled("Keys (vim style)   j/k scroll, esc closes", bold(mono::FG)), Line::raw("")];
    for (k, d) in HELP {
        lines.extend(wrap(vec![Span::styled(format!("  {k:<18}"), bold(mono::ACCENT)), Span::styled(d.to_string(), st(mono::FG))], width, 20));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("What the icons mean", bold(mono::FG)));
    let legend: [(&str, Style, &str); 22] = [
        (icons.issue, bold(mono::ERROR), "errors (orange) and warnings (yellow); `i` lists them with fixes"),
        (icons.legacy, bold(mono::WARN), "a session of an old fleet-skill fleet; `x` then `d` adopts it"),
        (icons.s_needs, bold(mono::NEEDS), "needs you: waiting at a prompt or a question"),
        (icons.s_busy, bold(mono::BUSY), "busy"),
        (icons.s_idle, bold(mono::FG), "idle"),
        (icons.s_stopped, st(mono::COMMENT), "stopped (has a job row)"),
        (icons.s_off, st(mono::DIM), "not started"),
        (icons.s_unknown, st(mono::MUTED), "unknown: the evidence does not say"),
        (icons.fleet, bold(mono::STRONG), "fleet"),
        (icons.admiral, bold(mono::STRONG), "admiral"),
        (icons.squadron, st(mono::FG), "squadron"),
        (icons.commodore, bold(mono::STRONG), "commodore"),
        (icons.ship, st(mono::FG), "ship"),
        (icons.tender, st(mono::FG), "tender (one job for a commodore)"),
        (icons.crew, st(mono::BUSY), "crew: a sub-agent inside a session"),
        (icons.teammate, st(mono::BUSY), "teammate"),
        (icons.stowaway, bold(mono::WARN), "stowaway: a session in fleet scope with no labels"),
        ("\u{25cf}", bold(mono::BUSY), "pane dot: working (green), blocked (pink), done (grey)"),
        ("\u{25cb}", bold(mono::FG), "pane dot: idle"),
        ("\u{00b7}", st(mono::COMMENT), "pane dot: unknown"),
        ("\u{25be}", st(mono::COMMENT), "unfolded (h folds)"),
        ("\u{25b8}", st(mono::COMMENT), "folded (l unfolds)"),
    ];
    for (icon, style, what) in legend {
        lines.extend(wrap(vec![Span::styled(format!("  {icon}  "), style), Span::styled(what.to_string(), st(mono::SOFT))], width, 5));
    }
    lines.extend(wrap(vec![Span::styled("  Band counts read `icon n`; under 100 columns only needs-you and errors stay.", st(mono::COMMENT))], width, 2));
    let room = r.height.saturating_sub(2) as usize;
    let scroll = scroll.min(lines.len().saturating_sub(room));
    let more = lines.len() > room + scroll;
    let shown: Vec<Line> = lines.into_iter().skip(scroll).take(room).collect();
    f.render_widget(Clear, r);
    let title = if more { "help (j for more)".to_string() } else { "help".to_string() };
    f.render_widget(Paragraph::new(shown).block(modal_block(title, mono::ACCENT)), r);
}

/// One plan step: what runs, where, and why (wrapped to `width`, indent kept).
pub fn step_lines(s: &Step, pane: bool, width: usize) -> Vec<Line<'static>> {
    let what = match s.cmd.first().map(String::as_str) {
        Some("write") => format!("write {}  ({} lines)", s.cmd.get(1).cloned().unwrap_or_default(), s.cmd.get(2).map(|c| c.lines().count()).unwrap_or(0)),
        Some("mkdir" | "archive" | "bind") => s.cmd.join(" "),
        _ => format!("$ {}", shell(&s.cmd)),
    };
    let mut first = vec![Span::styled("  ", Style::new())];
    if pane {
        first.push(Span::styled("[pane] ", bold(mono::ACCENT)));
    }
    first.push(Span::styled(what, st(mono::FG)));
    let mut v = wrap(first, width, 6);
    let mut meta = vec![Span::raw("      "), Span::styled(s.why.clone(), st(mono::COMMENT))];
    if let Some(c) = &s.cwd {
        meta.push(Span::styled(format!("   in {c}"), st(mono::DIM)));
    }
    v.extend(wrap(meta, width, 6));
    v
}

/// A plan as lines `width` wide: refusals with fixes, warnings, steps.
pub fn plan_lines(p: &Plan, pane_steps: &[Step], width: usize) -> Vec<Line<'static>> {
    let mut v = vec![];
    if !p.refusals.is_empty() {
        v.push(Line::styled("REFUSED", bold(mono::ERROR)));
        for i in &p.refusals {
            v.extend(wrap(vec![Span::styled(format!("  {:<16} ", i.code), bold(mono::ERROR)), Span::styled(i.message.clone(), st(mono::FG))], width, 19));
            if let Some(fx) = &i.fix {
                v.extend(wrap(vec![Span::raw("      fix  "), Span::styled(fx.clone(), bold(mono::ACCENT))], width, 11));
            }
        }
        v.push(Line::raw(""));
    }
    if !p.warnings.is_empty() {
        for i in &p.warnings {
            v.extend(wrap(vec![Span::styled(format!("  warn {:<12}", i.code), bold(mono::WARN)), Span::styled(i.message.clone(), st(mono::SOFT))], width, 19));
        }
        v.push(Line::raw(""));
    }
    v.push(Line::styled(format!("STEPS ({})", p.steps.len()), bold(mono::STRONG)));
    if p.steps.is_empty() {
        v.push(Line::styled("  nothing to do", st(mono::COMMENT)));
    }
    for s in &p.steps {
        v.extend(step_lines(s, pane_steps.contains(s), width));
    }
    v
}

fn draw_confirm(f: &mut Frame, area: Rect, p: &Pending, live: bool) {
    let w = 110u16.min(area.width.saturating_sub(2));
    let width = w.saturating_sub(2) as usize;
    let mut lines = vec![Line::styled(format!("{}?", p.plan.summary), bold(mono::FG))];
    lines.extend(wrap(vec![Span::styled("command  ", st(mono::COMMENT)), Span::styled(p.command.clone(), bold(mono::ACCENT))], width, 9));
    lines.push(Line::raw(""));
    let pane_steps: Vec<Step> = p.panes.iter().map(|(s, _)| s.clone()).collect();
    lines.extend(plan_lines(&p.plan, &pane_steps, width));
    lines.push(Line::raw(""));
    if !p.plan.ok() {
        lines.push(keys_line(&[("esc", "close (core refuses this plan; the fixes are above)")]));
    } else if !live {
        lines.push(keys_line(&[("y", "show what would happen (demo data: nothing runs)"), ("n", "cancel")]));
    } else {
        lines.push(keys_line(&[("y", "run"), ("n", "cancel")]));
    }
    let color = if p.plan.ok() { mono::ACCENT } else { mono::ERROR };
    let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let r = centered(area, w, h);
    // keep the keys line on screen when the plan is long
    let room = r.height.saturating_sub(2) as usize;
    if lines.len() > room && room > 2 {
        let keys = lines.pop().unwrap();
        lines.truncate(room - 2);
        lines.push(Line::styled("  ... (the rest of the plan does not fit; widen the window)", st(mono::COMMENT)));
        lines.push(keys);
    }
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).block(modal_block(p.act.word().to_string(), color)), r);
}

fn draw_fleet_filter(f: &mut Frame, area: Rect, on: &[bool; 7], sel: usize) {
    let mut lines = vec![Line::styled("Show fleets that are", bold(mono::FG)), Line::raw("")];
    for (i, s) in FleetState::ALL.iter().enumerate() {
        let mut l = Line::from(vec![
            Span::styled(if on[i] { " [x] " } else { " [ ] " }, bold(if on[i] { mono::ACCENT } else { mono::SOFT })),
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
    f.render_widget(Paragraph::new(lines).block(modal_block("squadrons by state".into(), mono::ACCENT)), r);
}

fn draw_needs(f: &mut Frame, area: Rect, items: &[NeedItem], sel: usize) {
    let mut lines = vec![keys_line(&[("j/k", "move"), ("enter", "go to it"), ("esc", "back")]), Line::raw("")];
    if items.is_empty() {
        lines.push(Line::styled("  nothing is waiting on you", st(mono::COMMENT)));
    }
    for (i, it) in items.iter().enumerate() {
        let mut line = Line::from(vec![
            Span::styled(format!("  {:<30}", it.name), bold(mono::NEEDS)),
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
    f.render_widget(Paragraph::new(lines).block(modal_block("waiting on you".into(), mono::NEEDS)), r);
}

fn draw_issues(app: &App, f: &mut Frame, area: Rect, sel: usize) {
    let issues = app.issues();
    let r = centered(area, area.width.saturating_sub(4).max(40), area.height.saturating_sub(2));
    let inner_h = r.height.saturating_sub(2) as usize;
    let width = r.width.saturating_sub(2) as usize;
    let mut lines = vec![keys_line(&[("j/k", "move"), ("enter", "go"), ("y", "yank fix"), ("esc", "back")]), Line::raw("")];
    if issues.is_empty() {
        lines.push(Line::styled("  No issues. Every fleet's shape matches its config.", st(mono::MUTED)));
    }
    let mut body: Vec<Line<'static>> = vec![];
    let (mut sel_from, mut sel_to) = (0, 0);
    for (i, is) in issues.iter().enumerate() {
        let start = body.len();
        let c = severity_color(is.severity);
        let wh = is.id.as_ref().map(|x| x.0.clone()).unwrap_or_else(|| if is.code == "stowaway" || is.code == "legacy" { "stowaways".into() } else { "this machine".into() });
        let mut block = vec![Line::from(vec![
            Span::styled(format!(" {:<6}", severity_word(is.severity).to_uppercase()), bold(c)),
            Span::styled(format!("{:<18}", is.code), bold(c)),
            Span::styled(wh, st(mono::MUTED)),
        ])];
        block.extend(wrap(vec![Span::raw("   "), Span::styled(is.message.clone(), st(mono::FG))], width, 3));
        block.extend(wrap(
            vec![
                Span::styled("   fix  ", st(mono::COMMENT)),
                match &is.fix {
                    Some(x) => Span::styled(x.clone(), bold(mono::ACCENT)),
                    None => Span::styled("no command; see the message".to_string(), st(mono::COMMENT)),
                },
            ],
            width,
            8,
        ));
        for l in block {
            body.push(selectable(l, i == sel));
        }
        if i == sel {
            (sel_from, sel_to) = (start, body.len());
        }
    }
    let room = inner_h.saturating_sub(lines.len());
    let start = if sel_to > room { (sel_to - room).max(0).min(sel_from) } else { 0 };
    lines.extend(body.into_iter().skip(start).take(room));
    let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(modal_block(format!("issues: {} ({errors} errors)", issues.len()), if errors > 0 { mono::ERROR } else { mono::WARN })),
        r,
    );
}

fn draw_menu(f: &mut Frame, area: Rect, title: &str, items: &[MenuItem], sel: usize) {
    let mut lines = vec![];
    for (i, it) in items.iter().enumerate() {
        let (ks, ls) = match &it.enabled {
            Ok(()) => (bold(mono::ACCENT), st(mono::FG)),
            Err(_) => (bold(mono::DIM), st(mono::COMMENT)),
        };
        let mut spans = vec![Span::styled(format!("  {}   ", it.key), ks), Span::styled(pad(&it.label, 46), ls)];
        if let Err(why) = &it.enabled {
            let why = if why == crate::actions::MONITOR_ONLY { "monitor-only: no multiplexer".to_string() } else { why.clone() };
            spans.push(Span::styled(why, st(mono::COMMENT)));
        }
        lines.push(selectable(Line::from(spans), i == sel));
    }
    lines.push(Line::raw(""));
    lines.push(keys_line(&[("key or enter", "choose (each confirms first)"), ("esc", "close")]));
    let r = centered(area, 104, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).block(modal_block(title.to_string(), mono::ACCENT)), r);
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
        // what is typed is shown as it will be sent: no control characters
        Line::from(vec![Span::styled("> ", bold(mono::ACCENT)), Span::styled(nimitz_mux::sanitize_text(text), st(mono::FG)), Span::styled("\u{2588}", st(mono::ACCENT))]),
        Line::raw(""),
        Line::from(vec![Span::styled("command  ", st(mono::COMMENT)), Span::styled(crate::actions::signal_command(plan, if text.is_empty() { "..." } else { text }), bold(mono::ACCENT))]),
        keys_line(&[("enter", "send"), ("esc", "cancel"), ("ctrl-u", "clear")]),
    ];
    let r = centered(area, 96, 10);
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(modal_block("message".into(), mono::ACCENT)), r);
}

fn draw_form(f: &mut Frame, area: Rect, form: &Form) {
    let mut lines = vec![];
    for (i, fl) in form.fields.iter().enumerate() {
        let focused = i == form.focus;
        let label = Span::styled(format!("  {:<14}", fl.label), if focused { bold(mono::ACCENT) } else { st(mono::SOFT) });
        let value = match &fl.kind {
            FieldKind::Fixed => Span::styled(fl.value.clone(), st(mono::COMMENT)),
            FieldKind::Choice(_) => Span::styled(format!("\u{2039} {} \u{203a}", fl.value), if focused { bold(mono::ACCENT) } else { st(mono::FG) }),
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
            spans.push(Span::styled("\u{2588}", st(mono::ACCENT)));
        }
        let mut l = Line::from(spans);
        if focused {
            l = l.style(Style::new().bg(mono::BG_HI));
        }
        lines.push(l);
        match &fl.error {
            Some(e) => lines.push(Line::styled(format!("  {:<14}\u{2717} {e}", ""), bold(mono::ERROR))),
            None if focused && !fl.hint.is_empty() => lines.push(Line::styled(format!("  {:<14}{}", "", fl.hint), st(mono::COMMENT))),
            None => {}
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![Span::styled("command  ", st(mono::COMMENT)), Span::styled(form.command.clone(), bold(mono::ACCENT))]));
    lines.push(Line::raw(""));
    if !form.problems.is_empty() {
        lines.push(Line::styled("CORE SAYS NO", bold(mono::ERROR)));
        for p in &form.problems {
            lines.push(Line::styled(format!("  \u{2717} {p}"), st(mono::ERROR)));
        }
        lines.push(Line::raw(""));
    }
    for w in &form.warnings {
        lines.push(Line::styled(format!("  warn {w}"), st(mono::WARN)));
    }
    if form.ok() {
        lines.extend(plan_lines(&form.plan, &[], 110));
        lines.push(Line::raw(""));
    }
    lines.push(keys_line(&[("tab/\u{2191}\u{2193}", "field"), ("\u{2190}\u{2192}", "choose"), ("ctrl-e", "$EDITOR"), ("ctrl-s / enter on last", "save"), ("esc", "cancel")]));
    let ok_color = if form.ok() { mono::BUSY } else { mono::ERROR };
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
