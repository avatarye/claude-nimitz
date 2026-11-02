//! Drawing: top bar, the frame list, the detail pane, the footer, and every
//! modal (help, confirm with plan steps, menus, issues, needs-you, message,
//! forms). Reads `App`, never changes it except scroll and page size.

use std::time::Duration;

use nimitz_core::{Plan, Severity, Step};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::actions::{shell, MessagePlan, Pending};
use crate::app::{App, MenuItem, Modal, NeedItem, HELP};
use crate::detail::{badge, detail_with_art, preview_lines};
use crate::finder::{Cand, Finder, Hit};
use crate::forms::{FieldKind, Form};
use crate::frames::{layout, pad, selectable, truncate_spans, wrap};
use crate::theme::{bold, mono, severity_color, severity_word, st};
use crate::view::FleetState;

/// Every popup has one of three widths and takes the height of what it shows,
/// in the middle of the body; long, it stops at `PANEL_MARGIN` and scrolls.
const PANEL_SMALL: u16 = 50;
const PANEL_STD: u16 = 100;
const PANEL_WIDE: u16 = 160;
/// Columns and rows left free around a popup.
const PANEL_MARGIN: (u16, u16) = (3, 1);
/// A popup's frame and padding, in columns: what its text does not get.
const PANEL_FRAME: u16 = 4;
/// The finder keeps one height: it would jump as its results change.
const FINDER_HEIGHT: u16 = 40;
/// A two-part popup's list keeps the room for the finder's key lines.
const LIST_MIN: u16 = 74;

/// A popup `w` by `h` at most, in the middle of the body.
fn panel(area: Rect, body: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(body.width.saturating_sub(2 * PANEL_MARGIN.0));
    let h = h.min(body.height.saturating_sub(2 * PANEL_MARGIN.1));
    Rect { x: area.x + (area.width.saturating_sub(w)) / 2, y: body.y + (body.height - h) / 2, width: w, height: h }
}

/// The columns a popup's text gets.
fn text_width(r: Rect) -> u16 {
    r.width.saturating_sub(PANEL_FRAME)
}

/// `lines` as screen rows `width` wide, so a popup knows its height exactly.
fn wrapped(lines: &[Line<'static>], width: u16) -> Vec<Line<'static>> {
    lines
        .iter()
        .flat_map(|l| {
            if l.width() <= width as usize {
                return vec![l.clone()];
            }
            wrap(l.spans.clone(), width as usize, 0).into_iter().map(|w| w.style(l.style)).collect()
        })
        .collect()
}

/// The top row that keeps rows `from..to` in a view of `view` rows.
fn follow(from: usize, to: usize, view: usize) -> usize {
    if to > view {
        (to - view).min(from)
    } else {
        0
    }
}

/// A popup's text in `r`: `body`, from row `at`, a scrollbar when it does not
/// fit, and `keys` always under it. Returns the most it scrolls and the rows
/// it shows.
fn scrolled(f: &mut Frame, r: Rect, block: Block<'static>, body: Vec<Line<'static>>, keys: Vec<Line<'static>>, at: usize) -> (usize, usize) {
    let view = (r.height as usize).saturating_sub(2 + keys.len());
    let max = body.len().saturating_sub(view);
    let at = at.min(max);
    let mut shown: Vec<Line<'static>> = body.into_iter().skip(at).take(view).collect();
    shown.extend(keys);
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(shown).block(block), r);
    border_bar(f, r, r.y + 1, view, max, at);
    (max, view)
}

/// The keys of a popup, a blank line above them.
fn keys_under(pairs: &[(&str, &str)]) -> Vec<Line<'static>> {
    vec![Line::raw(""), keys_line(pairs)]
}

/// A count of rows as a height.
fn rows16(n: usize) -> u16 {
    n.min(u16::MAX as usize / 2) as u16
}

/// A wide popup's two parts, from 120 columns: a list and the detail beside it.
fn split(group: Rect) -> (Rect, Rect) {
    let lw = (group.width * 55 / 100).max(LIST_MIN);
    (Rect { width: lw, ..group }, Rect { x: group.x + lw, width: group.width - lw, ..group })
}

/// The finder's panels, one group: the list and (from 120 columns) the detail
/// beside it, the same height. Returns the list, then the detail.
pub fn finder_areas(area: Rect, body: Rect) -> (Rect, Option<Rect>) {
    let group = panel(area, body, PANEL_WIDE, FINDER_HEIGHT);
    if body.width < 120 {
        return (group, None);
    }
    let (list, detail) = split(group);
    (list, Some(detail))
}

pub fn draw(app: &mut App, f: &mut Frame) {
    let full = f.area();
    f.render_widget(Block::new().style(Style::new().bg(mono::BG).fg(mono::FG)), full);
    // the part the user sees: Herdr's overlay hides its last rows and columns
    let area = Rect { width: full.width.saturating_sub(app.inset.1), height: full.height.saturating_sub(app.inset.0), ..full };
    // fleet tabs on top; two status rows at the bottom: the fleets (ADMIRALTY)
    // over the versions, the machine and the refresh
    let [tabs, body, top, bottom] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1), Constraint::Length(1)]).areas(area);
    f.render_widget(Paragraph::new(top_line(app, top.width as usize)).style(Style::new().bg(mono::BG_HI)), top);
    f.render_widget(Paragraph::new(tab_line(app, tabs.width as usize)).style(Style::new().bg(mono::BG)), tabs);
    // the list takes the whole body (a row's detail is under `x`)
    draw_list(app, f, body);
    f.render_widget(Paragraph::new(bottom_line(app, bottom.width as usize)).style(Style::new().bg(mono::BG_HI)), bottom);
    match &app.modal {
        Some(Modal::Help { scroll }) => draw_help(app, f, area, body, *scroll),
        Some(Modal::Confirm(p)) => draw_confirm(app, f, area, body, p),
        Some(Modal::Needs { items, sel }) => draw_needs(f, area, body, items, *sel),
        Some(Modal::Issues { sel }) => draw_issues(app, f, area, body, *sel),
        Some(Modal::Menu { title, items, sel, detail }) => {
            let (title, items, sel, detail) = (title.clone(), items.clone(), *sel, *detail);
            draw_menu(app, f, area, body, &title, &items, sel, detail)
        }
        Some(Modal::Message { plan, text, output, back }) => draw_message(app, f, area, body, plan, text, output.as_deref(), *back),
        Some(Modal::FleetFilter { sel }) => draw_fleet_filter(f, area, body, &app.fleet_states, *sel),
        Some(Modal::Form(form)) => draw_form(f, area, body, form),
        Some(Modal::Finder(fd)) => draw_finder(app, f, area, body, fd),
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
        // the shown fleet's state word and counts sit on its tab (the list
        // has no fleet line); the others keep only how many need you
        let band = app.rows.iter().find(|r| r.node.kind == crate::view::Kind::FleetBand && on);
        if let Some(r) = band {
            if let Some(state) = r.node.detail.first() {
                sp.push(Span::styled(format!(" {}", state.content), state.style));
            }
            let narrow = width < crate::frames::NARROW_BAND;
            let shown: Vec<&crate::view::Count> = r.node.counts.iter().filter(|c| !narrow || c.key).collect();
            if !shown.is_empty() {
                sp.push(Span::raw("  "));
            }
            for (k, c) in shown.iter().enumerate() {
                if k > 0 {
                    sp.push(Span::styled(" / ", st(mono::FAINT)));
                }
                sp.push(Span::styled(c.text(), c.style));
            }
            sp.push(Span::raw(" "));
        } else {
            let n = need(t);
            if n > 0 {
                sp.push(Span::styled(format!("!{n} "), bold(mono::NEEDS)));
            }
        }
        sp.push(Span::raw(" "));
        if on {
            // the name, then its state and counts on dark cyan up to the next
            // tab: one strip. A chip's fill, or a word's colour, stays the
            // text colour where it reads on dark cyan (`readable_on`), else FG
            for s in sp.iter_mut().skip(1) {
                let c = s.style.bg.or(s.style.fg).unwrap_or(mono::FG);
                let bold = s.style.add_modifier.contains(Modifier::BOLD);
                s.style = s.style.bg(mono::ACCENT_DIM).fg(crate::theme::readable_on(c, mono::ACCENT_DIM, bold));
            }
        }
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
    // the shown tab is kept whole, cut cleanly when even it does not fit
    Line::from(truncate_spans(out, width))
}

fn top_line(app: &App, width: usize) -> Line<'static> {
    let Some(b) = &app.bundle else { return Line::from(vec![badge("ADMIRALTY", mono::STRONG), Span::raw("  loading ...")]) };
    // the longest form whose essentials fit: words, then short words, then
    // letters; optional pieces follow while whole ones fit, never cut mid-word
    let w = |v: &[Span]| v.iter().map(|s| s.content.width()).sum::<usize>();
    // `? keys` goes before the essentials do
    let level = (0..3).find(|l| w(&top_spans(app, b, *l).0) + w(&key_hint("?", "keys")) <= width);
    let (essential, optional) = top_spans(app, b, level.unwrap_or(2));
    balanced(essential, optional, if level.is_some() { key_hint("?", "keys") } else { vec![] }, width)
}

/// A key and what it does, as the status rows show it at their right.
fn key_hint(k: &str, d: &str) -> Vec<Span<'static>> {
    vec![Span::styled(format!(" {k} "), bold(mono::ACCENT)), Span::styled(format!("{d} "), st(mono::SOFT))]
}

/// One status row: `essential` from the left (cut at the edge), `optional`
/// pieces while whole ones fit, `right` flush to the right edge whenever it fits.
fn balanced(essential: Vec<Span<'static>>, optional: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let room = width.saturating_sub(right_w);
    let mut out = vec![];
    let mut used = 0;
    for sp in essential {
        let n = sp.content.width();
        if used + n > room {
            break;
        }
        used += n;
        out.push(sp);
    }
    for sp in optional {
        let n = sp.content.width();
        if used + n <= room {
            used += n;
            out.push(sp);
        }
    }
    if right_w <= width {
        out.push(Span::raw(" ".repeat(room - used)));
        out.extend(right);
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

/// The ADMIRALTY row at a level of detail (0 words, 1 short words, 2
/// letters), as (what always shows, what shows when there is room).
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
    (v, vec![])
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
        return Line::styled(format!(" running (esc waits for it, q quits now): {b}"), bold(mono::BUSY));
    }
    if let Some(t) = &app.toast {
        if t.at.elapsed() < Duration::from_secs(8) {
            return Line::from(Span::styled(format!(" {}", t.msg), t.style));
        }
    }
    let Some(b) = &app.bundle else { return Line::raw("") };
    let w = |v: &[Span]| v.iter().map(|s| s.content.width()).sum::<usize>();
    let level = (0..3).find(|l| {
        let (e, _, r) = status_spans(app, b, *l);
        w(&e) + w(&r) <= width
    });
    let (essential, optional, right) = status_spans(app, b, level.unwrap_or(2));
    balanced(essential, optional, if level.is_some() { right } else { vec![] }, width)
}

/// The second status row at a level of detail (0 words, 1 short words, 2
/// letters): the nimitz Admiralty runs (flagged when it is not Admiralty's
/// own), Claude Code and its support, the machine; at the right how stale the
/// data is (only when no load landed for 3 intervals) and the local date and
/// time; as (what always shows, what shows when there is room, the right part).
fn status_spans(app: &App, b: &crate::data::Bundle, level: u8) -> (Vec<Span<'static>>, Vec<Span<'static>>, Vec<Span<'static>>) {
    let pick = |full: String, short: String, tiny: String| match level {
        0 => full,
        1 => short,
        _ => tiny,
    };
    let gap = "  ";
    let ours = env!("CARGO_PKG_VERSION");
    let mut v = vec![];
    match app.nimitz_version.lock().ok().and_then(|v| v.clone()) {
        // the read failed: the nimitz it runs is unknown
        Some(n) if n == "?" => v.push(Span::styled(pick(" Nimitz: ?".into(), " v?".into(), " v?".into()), bold(mono::WARN))),
        Some(n) if n != ours => v.push(Span::styled(
            pick(format!(" Nimitz: v{n} (admiralty v{ours}, mismatch)"), format!(" v{n} \u{2260} v{ours}"), format!(" {n}\u{2260}{ours}")),
            bold(mono::WARN),
        )),
        // not read yet, live: nothing claimed until `nimitz --version` answers
        None if app.ctx.live => v.push(Span::styled(pick(" Nimitz: \u{2026}".into(), " v\u{2026}".into(), " v\u{2026}".into()), st(mono::MUTED))),
        // the same, or the demo (which never reads it): Admiralty's own, the
        // nimitz of its release
        _ => v.push(Span::styled(pick(format!(" Nimitz: v{ours}"), format!(" v{ours}"), format!(" v{ours}")), st(mono::MUTED))),
    }
    let ver = b.snap.cc_version.as_ref().map(|v| format!("v{v}")).unwrap_or("not found".into());
    if b.snap.compat_ok {
        v.push(Span::styled(pick(format!("{gap}Claude Code: {ver} (supported)"), format!("{gap}CC {ver}"), format!("{gap}CC {ver}")), st(mono::MUTED)));
    } else {
        let c = nimitz_core::compat::supported();
        let raw = b.snap.cc_version.clone().unwrap_or_default();
        let newer = !raw.is_empty() && !raw.contains('-') && nimitz_core::compat::position(&c, &raw) == 1;
        let (full, short) = if raw.is_empty() {
            (format!("{gap}Claude Code: not found"), format!("{gap}CC not found"))
        } else if newer {
            (format!("{gap}Claude Code: {ver} (newer than tested, v{})", c.max), format!("{gap}CC {ver} untested"))
        } else {
            (format!("{gap}Claude Code: {ver} (unsupported)"), format!("{gap}CC {ver} unsupported"))
        };
        v.push(Span::styled(pick(full, short, format!("{gap}CC!")), bold(if newer { mono::WARN } else { mono::ERROR })));
    }
    if let Some(m) = &b.snap.machine {
        v.push(Span::styled(pick(format!("{gap}Machine: {m}"), format!("{gap}{m}"), format!("{gap}{m}")), st(mono::MUTED)));
    }
    let mut right = vec![];
    // a refresh that has not landed for 3 intervals: the rows may be old
    let age = app.now() - b.snap.taken_at;
    if app.ctx.live && age > 3.0 * app.interval.as_secs_f64() {
        right.push(Span::styled(pick(format!("stale {}", ago(age)), format!("stale {}", ago(age)), "stale".into()), bold(mono::WARN)));
        right.push(Span::raw(gap));
    }
    match app.wall() {
        Some(w) => right.push(Span::styled(pick(w.full(), w.short(), w.time()) + " ", st(mono::MUTED))),
        // no clock (the C library cannot say): the stale warning, if any, at the edge
        None if !right.is_empty() => {
            right.pop();
            right.push(Span::raw(" "));
        }
        None => {}
    }
    (v, vec![], right)
}

/// Seconds as `2s`, `5m` or `3h`.
fn ago(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        _ => format!("{}h", s / 3600),
    }
}

fn draw_list(app: &mut App, f: &mut Frame, area: Rect) {
    let view = area.height.saturating_sub(1) as usize;
    let (lines, _) = layout(&app.rows, area.width, Some(view), app.cursor, app.icons, app.frame());
    app.page = view.max(2);
    if let Some(pos) = lines.iter().position(|(_, r)| *r == Some(app.cursor)) {
        if pos < app.scroll {
            app.scroll = pos;
        } else if pos >= app.scroll + view {
            app.scroll = pos + 1 - view;
        }
    }
    app.scroll = app.scroll.min(lines.len().saturating_sub(view));
    // the first row stays blank: the fleet tabs never sit on the first box's border
    let mut shown = vec![Line::raw("")];
    if lines.is_empty() {
        shown.push(Line::raw(""));
        let msg = if app.bundle.as_ref().map(|b| b.snap.fleets.is_empty()).unwrap_or(false) {
            "  No fleets yet.  c  then  f  creates one (or `nimitz fleet new`)."
        } else {
            "  Nothing matches the filters (esc clears /, f shows fleet states, F flagged)."
        };
        shown.push(Line::styled(msg, st(mono::SOFT)));
    }
    let mut on_screen = vec![];
    shown.extend(lines.into_iter().skip(app.scroll).take(view).map(|(l, r)| {
        if let Some(r) = r {
            on_screen.push(r);
        }
        l
    }));
    app.shown_rows = Some(on_screen);
    f.render_widget(Paragraph::new(shown), area);
}

// ------------------------------------------------------------------ modals

fn modal_block(title: String, color: ratatui::style::Color) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Thick)
        .border_style(st(color))
        .padding(Padding::horizontal(1))
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

fn draw_help(app: &App, f: &mut Frame, area: Rect, body: Rect, scroll: usize) {
    let icons = app.icons;
    let width = text_width(panel(area, body, PANEL_STD, 1)) as usize;
    let mut lines = vec![Line::styled("Keys (vim style)", bold(mono::FG)), Line::raw("")];
    for (k, d) in HELP {
        lines.extend(wrap(vec![Span::styled(format!("  {k:<18}"), bold(mono::ACCENT)), Span::styled(d.to_string(), st(mono::FG))], width, 20));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("What the icons mean", bold(mono::FG)));
    let legend: [(&str, Style, &str); 22] = [
        (icons.issue, bold(mono::ERROR), "errors (orange) and warnings (yellow); `i` lists them with fixes. A crew: failed (its log ends in an API error)"),
        (icons.legacy, bold(mono::WARN), "a session of an old fleet-skill fleet; `x` then `d` adopts it"),
        (icons.s_needs, bold(mono::NEEDS), "needs you: waiting at a prompt or a question"),
        (icons.s_busy, bold(mono::BUSY), "busy (a crew or a task: at work)"),
        (icons.s_idle, bold(mono::FG), "idle (a teammate: between turns, waiting for a message)"),
        (icons.s_stopped, st(mono::COMMENT), "stopped (has a job row); a crew or a task: done"),
        (icons.s_parked, st(mono::COMMENT), "parked (idle an hour, the daemon retired it; wakes on a prompt)"),
        (icons.s_off, st(mono::DIM), "not started; a task: pending"),
        (icons.s_unknown, st(mono::MUTED), "unknown: the evidence does not say"),
        (icons.fleet, bold(mono::STRONG), "fleet"),
        (icons.admiral, bold(mono::STRONG), "admiral"),
        (icons.squadron, st(mono::FG), "squadron"),
        (icons.commodore, bold(mono::STRONG), "commodore"),
        (icons.ship, st(mono::FG), "ship"),
        (icons.crew, st(mono::BUSY), "crew: a sub-agent inside a session (NAME its agent type, DETAIL what it does)"),
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
    let r = panel(area, body, PANEL_STD, rows16(lines.len()) + 4);
    let rows = scrolled(f, r, modal_block("keys".into(), mono::ACCENT), lines, keys_under(&[("j/k", "scroll"), ("? or esc", "close")]), scroll);
    app.panel_rows.set(rows);
}

/// One plan step: what runs, where, and why (wrapped to `width`, indent kept).
pub fn step_lines(s: &Step, pane: bool, width: usize) -> Vec<Line<'static>> {
    let what = match s.cmd.first().map(String::as_str) {
        Some("write") => format!("write {}  ({} lines)", s.cmd.get(1).cloned().unwrap_or_default(), s.cmd.get(2).map(|c| c.lines().count()).unwrap_or(0)),
        Some("mkdir" | "archive" | "bind" | "trust") => s.cmd.join(" "),
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
            v.extend(wrap(vec![Span::styled(format!("  warn {:<12} ", i.code), bold(mono::WARN)), Span::styled(i.message.clone(), st(mono::SOFT))], width, 19));
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

fn draw_confirm(app: &App, f: &mut Frame, area: Rect, body: Rect, p: &Pending) {
    let live = app.ctx.live;
    let width = text_width(panel(area, body, PANEL_STD, 1)) as usize;
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
    // the keys stay on screen; a long plan scrolls above them
    let mut keys = lines.pop().unwrap_or_default();
    let r = panel(area, body, PANEL_STD, rows16(lines.len()) + 3);
    if lines.len() + 3 > r.height as usize {
        keys.spans.extend(keys_line(&[("j/k", "scroll")]).spans);
    }
    app.panel_rows.set(scrolled(f, r, modal_block(p.act.word().to_string(), color), lines, vec![keys], app.detail_scroll));
}

fn draw_fleet_filter(f: &mut Frame, area: Rect, body: Rect, on: &[bool; 7], sel: usize) {
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
    let keys = keys_under(&[("space", "toggle"), ("a", "all"), ("esc", "close")]);
    let r = panel(area, body, PANEL_SMALL, rows16(lines.len()) + 4);
    let at = follow(sel + 2, sel + 3, (r.height as usize).saturating_sub(4));
    scrolled(f, r, modal_block("squadrons by state".into(), mono::ACCENT), lines, keys, at);
}

fn draw_needs(f: &mut Frame, area: Rect, body: Rect, items: &[NeedItem], sel: usize) {
    let width = text_width(panel(area, body, PANEL_STD, 1)) as usize;
    let mut lines = vec![];
    if items.is_empty() {
        lines.push(Line::styled("nothing is waiting on you", st(mono::COMMENT)));
    }
    for (i, it) in items.iter().enumerate() {
        let spans = vec![
            Span::styled(format!("{:<30}", it.name), bold(mono::NEEDS)),
            Span::styled(format!("{:<6}", it.age), st(mono::COMMENT)),
            Span::styled(it.text.replace('\n', " "), st(mono::FG)),
        ];
        lines.push(selectable(Line::from(truncate_spans(spans, width)), i == sel));
    }
    let keys = keys_under(&[("j/k", "move"), ("enter", "row"), ("ctrl-o", "pane"), ("esc", "back")]);
    let r = panel(area, body, PANEL_STD, rows16(lines.len()) + 4);
    let at = follow(sel, sel + 1, (r.height as usize).saturating_sub(4));
    scrolled(f, r, modal_block("waiting on you".into(), mono::NEEDS), lines, keys, at);
}

fn draw_issues(app: &App, f: &mut Frame, area: Rect, body_r: Rect, sel: usize) {
    let issues = app.issues();
    let width = text_width(panel(area, body_r, PANEL_WIDE, 1)) as usize;
    let mut body: Vec<Line<'static>> = vec![];
    if issues.is_empty() {
        body.push(Line::styled("No issues. Every fleet's shape matches its config.", st(mono::MUTED)));
    }
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
    let keys = keys_under(&[("j/k", "move"), ("enter", "row"), ("ctrl-o", "pane"), ("y", "yank fix"), ("esc", "back")]);
    let r = panel(area, body_r, PANEL_WIDE, rows16(body.len()) + 4);
    let at = follow(sel_from, sel_to, (r.height as usize).saturating_sub(4));
    let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
    let block = modal_block(format!("issues: {} ({errors} errors)", issues.len()), if errors > 0 { mono::ERROR } else { mono::WARN });
    scrolled(f, r, block, body, keys, at);
}

/// A menu's rows in `width` columns: one per entry (a closed one says why right
/// after its label). A long entry wraps under itself, or is cut under 120
/// columns (`one_row`), where the detail needs the height. Also the selected
/// entry's rows.
fn menu_lines(items: &[MenuItem], sel: usize, width: usize, one_row: bool) -> (Vec<Line<'static>>, (usize, usize)) {
    let mut lines = vec![];
    let mut sel_rows = (0, 0);
    for (i, it) in items.iter().enumerate() {
        let (ks, ls) = match &it.enabled {
            Ok(()) => (bold(mono::ACCENT), st(mono::FG)),
            Err(_) => (bold(mono::DIM), st(mono::COMMENT)),
        };
        let mut spans = vec![Span::styled(format!("{}   ", it.key), ks), Span::styled(it.label.clone(), ls)];
        if let Err(why) = &it.enabled {
            let why = if why == crate::actions::MONITOR_ONLY { "monitor-only: no multiplexer".to_string() } else { why.clone() };
            spans.push(Span::styled(format!("  ({why})"), st(mono::DIM)));
        }
        let from = lines.len();
        if one_row {
            lines.push(selectable(Line::from(truncate_spans(spans, width)), i == sel));
        } else {
            lines.extend(wrap(spans, width, 4).into_iter().map(|l| selectable(l, i == sel)));
        }
        if i == sel {
            sel_rows = (from, lines.len());
        }
    }
    (lines, sel_rows)
}

/// The row's detail in `width` columns, with where `enter` goes.
fn menu_detail(app: &App, width: u16, compact: bool) -> Vec<Line<'static>> {
    let mut text = detail_with_art(app, width as usize, compact);
    if let Some(hint) = crate::detail::jump_hint(app) {
        text.extend([Line::raw(""), hint]);
    }
    text
}

/// A menu; the actions menu (`detail`) has the row's detail beside it, the
/// same height (under it below 120 columns), scrolled by `app.detail_scroll`.
#[allow(clippy::too_many_arguments)]
fn draw_menu(app: &mut App, f: &mut Frame, area: Rect, body: Rect, title: &str, items: &[MenuItem], sel: usize, detail: bool) {
    let one_row = body.width < 120;
    let menu_h = |width: u16| rows16(menu_lines(items, sel, width as usize, one_row).0.len()) + 4;
    let menu = |r: Rect, h: u16, f: &mut Frame| {
        let r = Rect { height: h, ..r };
        let (lines, (from, to)) = menu_lines(items, sel, text_width(r) as usize, one_row);
        let at = follow(from, to, (r.height as usize).saturating_sub(4));
        scrolled(f, r, modal_block(title.to_string(), mono::ACCENT), lines, keys_under(&[("key or enter", "choose"), ("esc", "close")]), at);
    };
    if detail && body.width >= 120 {
        let (list, right) = split(panel(area, body, PANEL_WIDE, 1));
        let text = menu_detail(app, text_width(right), false);
        let h = menu_h(text_width(list)).max(rows_of(&text, text_width(right)) + 2);
        let group = panel(area, body, list.width + right.width, h);
        let (list, right) = split(group);
        menu(list, group.height, f);
        detail_scrolled(app, f, right, text);
        return;
    }
    let width = text_width(panel(area, body, PANEL_STD, 1));
    let menu_h = menu_h(width);
    let left = body.height.saturating_sub(menu_h);
    let whole = panel(area, body, PANEL_STD, menu_h);
    // under the menu, a panel only when it holds a few lines (borders and three)
    if !detail || left < 5 {
        menu(whole, whole.height, f);
        return;
    }
    // a short panel drops the picture and the blank lines: the reasons first
    let text = menu_detail(app, text_width(whole), left < 16);
    let det_h = left.min(rows_of(&text, text_width(whole)) + 2);
    let y = body.y + (body.height - menu_h - det_h) / 2;
    menu(Rect { y, ..whole }, menu_h, f);
    detail_scrolled(app, f, Rect { y: y + menu_h, height: det_h, ..whole }, text);
}

/// The detail panel with `app.detail_scroll` kept in reach.
fn detail_scrolled(app: &mut App, f: &mut Frame, r: Rect, text: Vec<Line<'static>>) {
    app.detail_scroll = app.detail_scroll.min((rows_of(&text, text_width(r)) as usize).saturating_sub(r.height.saturating_sub(2) as usize));
    detail_box(f, r, text, Some(app.detail_scroll));
}

/// Screen rows `text` takes in `width` columns.
fn rows_of(text: &[Line<'static>], width: u16) -> u16 {
    rows16(wrapped(text, width).len())
}

/// The detail panel (beside the actions menu and the finder), scrolled down
/// `scroll` lines when it scrolls (its title then names the keys, if there is
/// more than it shows).
fn detail_box(f: &mut Frame, r: Rect, text: Vec<Line<'static>>, scroll: Option<usize>) {
    let room = r.height.saturating_sub(2) as usize;
    let rows = wrapped(&text, text_width(r));
    let total = rows.len();
    let more = scroll.is_some_and(|s| total > room + s || s > 0);
    let title = if more { "detail  ctrl-e/ctrl-y PgDn/PgUp scroll" } else { "detail" };
    let at = scroll.unwrap_or(0);
    let shown: Vec<Line<'static>> = rows.into_iter().skip(at).take(room).collect();
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(shown).block(modal_block(title.into(), mono::ACCENT)), r);
    if scroll.is_some() {
        border_bar(f, r, r.y + 1, room, total.saturating_sub(room), at);
    }
}

/// A popup's scrollbar, on its right border over `view` rows from row `y`:
/// `max` rows to scroll, at row `at`; none when all of it shows.
fn border_bar(f: &mut Frame, r: Rect, y: u16, view: usize, max: usize, at: usize) {
    if max == 0 {
        return;
    }
    let rail = Rect { y, height: view as u16, ..r };
    let mut state = ScrollbarState::new(max).viewport_content_length(view).position(at);
    let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight).begin_symbol(None).end_symbol(None).thumb_style(st(mono::ACCENT)).track_style(st(mono::COMMENT));
    f.render_stateful_widget(bar, rail, &mut state);
}

/// The finder: the query, the ranked list in columns with the matched letters
/// lit, and beside it (when the screen is wide enough) the selected one's
/// detail, the same height.
fn draw_finder(app: &App, f: &mut Frame, area: Rect, body: Rect, fd: &Finder) {
    let (r, right) = finder_areas(area, body);
    let inner_w = text_width(r) as usize;
    // a query wider than the line shows its end, where the cursor is
    let room_q = inner_w.saturating_sub(4);
    let q: Vec<char> = fd.query.chars().collect();
    let shown_q = if q.len() > room_q { format!("\u{2026}{}", q[q.len() + 1 - room_q.max(1)..].iter().collect::<String>()) } else { fd.query.clone() };
    let mut lines = vec![
        Line::from(vec![Span::styled("> ", bold(mono::ACCENT)), Span::styled(shown_q, st(mono::FG)), Span::styled("\u{2588}", st(mono::ACCENT))]),
        // what the toggles hide is never silent, nor what they are
        Line::from(vec![
            Span::styled(format!("{} / {}{}", fd.hits.len(), fd.cands.len(), fd.hidden.text()), st(mono::COMMENT)),
            Span::styled(format!("  {}", fd.toggles.words(fd.fleet_only())), st(mono::SOFT)),
        ]),
    ];
    let cols = finder_columns(fd, inner_w);
    lines.push(Line::styled(finder_head(cols), bold(mono::COMMENT)));
    let room = (r.height as usize).saturating_sub(2 + lines.len() + 2);
    let start = (fd.sel + 1).saturating_sub(room);
    for (i, h) in fd.hits.iter().enumerate().skip(start).take(room) {
        let line = finder_row(&fd.cands[h.cand], h, cols, app.icons);
        lines.push(selectable(Line::from(truncate_spans(line, inner_w)), i == fd.sel));
    }
    if fd.hits.is_empty() {
        // the hint only when a row without a pane would match
        let why = if fd.hidden.no_pane_matching > 0 { "  nothing matches (ctrl-a: rows without a pane too)" } else { "  nothing matches" };
        lines.push(Line::styled(why, st(mono::SOFT)));
    }
    // the keys lines at the bottom (inside the border: two lines fewer)
    while lines.len() < (r.height as usize).saturating_sub(5) {
        lines.push(Line::raw(""));
    }
    lines.push(Line::raw(""));
    lines.push(keys_line(&[("\u{2191}\u{2193} ctrl-n/p", "move"), ("enter", "row"), ("ctrl-o", "pane"), ("esc", "close"), ("ctrl-u", "clear")]));
    lines.push(toggle_keys(&fd.toggles, fd.tab.as_deref().is_some_and(|t| t != crate::view::UNASSIGNED)));
    f.render_widget(Clear, r);
    let title = if fd.toggles.topic { "find: path, name, status, topic" } else { "find: path, name, status" };
    f.render_widget(Paragraph::new(lines).block(modal_block(title.into(), mono::ACCENT)), r);
    if let (Some(c), Some(right)) = (fd.selected(), right) {
        detail_box(f, right, preview_lines(app, &c.row_id, text_width(right) as usize), None);
    }
}

/// The finder's columns, as the list's: NAME, RANK, STATUS and, with the room
/// left, TOPIC. NAME takes what its longest path needs.
#[derive(Clone, Copy)]
struct FinderCols {
    name: usize,
    topic: usize,
}

const FINDER_PAD: usize = 2;
const FINDER_GAP: usize = 2;
/// an icon and `commodore`
const FINDER_RANK: usize = 11;
/// an icon and `needs you`
const FINDER_STATUS: usize = 11;

fn finder_columns(fd: &Finder, inner: usize) -> FinderCols {
    let fixed = FINDER_PAD + FINDER_RANK + FINDER_STATUS + 2 * FINDER_GAP;
    let room = inner.saturating_sub(fixed);
    let widest = fd.cands.iter().map(|c| { let (a, b) = c.name_range(); b - a }).max().unwrap_or(0);
    let name = widest.clamp(20.min(room), room.max(1));
    // a topic narrower than this says nothing
    let left = room.saturating_sub(name + FINDER_GAP);
    FinderCols { name, topic: if left >= 12 { left } else { 0 } }
}

fn finder_head(c: FinderCols) -> String {
    let mut s = format!("{}{:<n$}{}{:<r$}{}{:<t$}", " ".repeat(FINDER_PAD), "NAME", " ".repeat(FINDER_GAP), "RANK", " ".repeat(FINDER_GAP), "STATUS", n = c.name, r = FINDER_RANK, t = FINDER_STATUS);
    if c.topic > 0 {
        s.push_str(&format!("{}TOPIC", " ".repeat(FINDER_GAP)));
    }
    s
}

/// The chars `range` of `c.text` in `w` columns, padded, the matched letters
/// lit.
fn finder_cell(c: &Cand, h: &Hit, range: (usize, usize), w: usize) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = vec![];
    let mut run = String::new();
    let mut run_style = None;
    for (k, ch) in c.text.chars().enumerate().take(range.1).skip(range.0) {
        let hit = h.idx.binary_search(&(k as u32)).is_ok();
        let style = if hit { bold(mono::ACCENT).add_modifier(Modifier::UNDERLINED) } else { c.style_at(k) };
        if run_style.is_some_and(|s| s != style) {
            spans.push(Span::styled(std::mem::take(&mut run), run_style.unwrap()));
        }
        run_style = Some(style);
        run.push(ch);
    }
    if let Some(s) = run_style {
        spans.push(Span::styled(run, s));
    }
    let mut cut = truncate_spans(spans, w);
    let used: usize = cut.iter().map(|s| s.content.width()).sum();
    cut.push(Span::raw(" ".repeat(w.saturating_sub(used))));
    cut
}

fn finder_row(c: &Cand, h: &Hit, cols: FinderCols, icons: &crate::theme::Icons) -> Vec<Span<'static>> {
    let gap = || Span::raw(" ".repeat(FINDER_GAP));
    let (rank_icon, rank) = match c.role {
        Some(r) => (icons.role(r), r.as_str()),
        None => (icons.stowaway, "stowaway"),
    };
    let status_icon = match c.status {
        Some(s) => icons.status(s),
        None if c.text.ends_with("live") => icons.s_busy,
        None => icons.s_stopped,
    };
    let (sa, _) = c.status_range();
    let mut v = vec![Span::raw(" ".repeat(FINDER_PAD))];
    v.extend(finder_cell(c, h, c.name_range(), cols.name));
    v.push(gap());
    let rank_style = if c.stowaway { st(mono::WARN) } else { st(mono::SOFT) };
    v.push(Span::styled(pad(&format!("{rank_icon} {rank}"), FINDER_RANK), rank_style));
    v.push(gap());
    v.push(Span::styled(format!("{status_icon} "), c.style_at(sa)));
    v.extend(finder_cell(c, h, c.status_range(), FINDER_STATUS.saturating_sub(status_icon.width() + 1)));
    if let Some(r) = c.topic_range().filter(|_| cols.topic > 0) {
        v.push(gap());
        v.extend(finder_cell(c, h, r, cols.topic));
    }
    v
}

/// The finder's toggle keys (`ctrl- a pane only · t topic ...`), each lit
/// while on; `f this fleet` dim when no fleet's tab is shown (`fleet_tab`:
/// it would filter nothing).
fn toggle_keys(t: &crate::finder::Toggles, fleet_tab: bool) -> Line<'static> {
    let on = [t.pane_only, t.topic, t.no_stowaways, t.this_fleet, t.ships_only];
    let mut v = vec![Span::styled("ctrl-", bold(mono::ACCENT))];
    for (i, ((k, d), on)) in crate::finder::TOGGLE_KEYS.iter().zip(on).enumerate() {
        if i > 0 {
            v.push(Span::styled(" \u{b7} ", st(mono::COMMENT)));
        }
        let idle = *k == "ctrl-f" && !fleet_tab;
        v.push(Span::styled(k.trim_start_matches("ctrl-").to_string(), if idle { st(mono::COMMENT) } else { bold(mono::ACCENT) }));
        v.push(Span::styled(format!(" {d}"), if idle { st(mono::COMMENT) } else if on { bold(mono::FG) } else { st(mono::SOFT) }));
    }
    Line::from(v)
}

fn draw_message(app: &App, f: &mut Frame, area: Rect, body: Rect, plan: &MessagePlan, text: &str, output: Option<&str>, back: usize) {
    let how = format!("typed into its pane {}", plan.pane.pane_id);
    let head = vec![Line::styled(format!("Message {}", plan.to), bold(mono::FG)), Line::styled(how, st(mono::COMMENT))];
    let mut keys = vec![("enter", "send"), ("esc", "cancel"), ("ctrl-u", "clear")];
    if output.is_some() {
        keys.insert(0, ("PgUp/PgDn \u{2191}\u{2193}", "scroll"));
    }
    let foot = vec![
        Line::raw(""),
        // what is typed is shown as it will be sent: no control characters
        Line::from(vec![Span::styled("> ", bold(mono::ACCENT)), Span::styled(nimitz_mux::strip_controls(text), st(mono::FG)), Span::styled("\u{2588}", st(mono::ACCENT))]),
        Line::raw(""),
        Line::from(vec![Span::styled("command  ", st(mono::COMMENT)), Span::styled(crate::actions::signal_command(plan, if text.is_empty() { "..." } else { text }), bold(mono::ACCENT))]),
        keys_line(&keys),
    ];
    let w = text_width(panel(area, body, PANEL_STD, 1));
    let foot = wrapped(&foot, w);
    let Some(output) = output else {
        let lines = [head, foot].concat();
        let r = panel(area, body, PANEL_STD, rows16(lines.len()) + 2);
        f.render_widget(Clear, r);
        f.render_widget(Paragraph::new(lines).block(modal_block("message".into(), mono::ACCENT)), r);
        return;
    };
    let rows = app.output_rows(output, w as usize, output_rows);
    let r = panel(area, body, PANEL_STD, rows16(rows.len()).saturating_add(rows16(foot.len()) + 4));
    let block = modal_block("message".into(), mono::ACCENT);
    let inner = block.inner(r);
    f.render_widget(Clear, r);
    f.render_widget(block, r);
    // a foot too long to show keeps its end: the keys
    let foot_h = rows16(foot.len()).min(inner.height.saturating_sub(3));
    let foot: Vec<Line<'static>> = foot[foot.len() - foot_h as usize..].to_vec();
    let [top, mid, bottom] = Layout::vertical([Constraint::Length(2), Constraint::Fill(1), Constraint::Length(foot_h)]).areas(inner);
    f.render_widget(Paragraph::new(head), top);
    f.render_widget(Paragraph::new(foot), bottom);
    let view = mid.height as usize;
    let max = rows.len().saturating_sub(view);
    app.panel_rows.set((max, view));
    let top_row = max - back.min(max);
    f.render_widget(Paragraph::new(rows[top_row..].iter().take(view).cloned().collect::<Vec<_>>()), mid);
    border_bar(f, r, mid.y, view, max, top_row);
}

/// A member's output as screen rows, `width` wide: no control characters, and
/// no marks that reorder the text around them.
fn output_rows(output: &str, width: usize) -> Vec<Line<'static>> {
    output
        .lines()
        .flat_map(|l| {
            let clean = nimitz_mux::strip_controls(&l.replace('\t', "    "));
            let clean: String = clean.chars().filter(|c| !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')).collect();
            if clean.is_empty() {
                vec![Line::raw("")]
            } else {
                crate::frames::wrap(vec![Span::styled(clean, st(mono::FG))], width.max(1), 0)
            }
        })
        .collect()
}

fn draw_form(f: &mut Frame, area: Rect, body: Rect, form: &Form) {
    let width = text_width(panel(area, body, PANEL_STD, 1));
    let mut lines = vec![];
    let mut focus = 0;
    for (i, fl) in form.fields.iter().enumerate() {
        let focused = i == form.focus;
        if focused {
            focus = lines.len();
        }
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
        lines.extend(plan_lines(&form.plan, &[], width as usize));
        lines.push(Line::raw(""));
    }
    let keys = vec![keys_line(&[("tab/\u{2191}\u{2193}", "field"), ("\u{2190}\u{2192}", "choose"), ("ctrl-e", "$EDITOR"), ("ctrl-s / enter on last", "save"), ("esc", "cancel")])];
    let ok_color = if form.ok() { mono::BUSY } else { mono::ERROR };
    let focus = rows_of(&lines[..focus], width) as usize;
    let rows = wrapped(&lines, width);
    let keys = wrapped(&keys, width);
    let r = panel(area, body, PANEL_STD, rows16(rows.len() + keys.len()) + 2);
    let at = follow(focus, focus + 3, (r.height as usize).saturating_sub(2 + keys.len()));
    let block = modal_block(form.title.clone(), ok_color).title_bottom(Span::styled(if form.ok() { " ready " } else { " fix the fields in red " }, st(ok_color)));
    scrolled(f, r, block, rows, keys, at);
}

/// For tests: the plain text of a line.
pub fn plain(l: &Line) -> String {
    l.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// A line with 24-bit ANSI colour (what `dump --ansi` prints), reset at its end.
pub fn ansi(l: &Line) -> String {
    let mut out = String::new();
    for sp in &l.spans {
        out.push_str(&sgr(l.style.patch(sp.style)));
        out.push_str(&sp.content);
    }
    out.push_str("\u{1b}[0m");
    out
}

/// A drawn screen, a line per row: its symbols, or (`colour`) with ANSI SGR
/// codes wherever a cell's style changes.
pub fn screen_lines(buf: &ratatui::buffer::Buffer, colour: bool) -> Vec<String> {
    let a = buf.area;
    (0..a.height)
        .map(|y| {
            let mut out = String::new();
            let mut last: Option<Style> = None;
            for x in 0..a.width {
                let c = &buf[(x, y)];
                let style = Style::new().fg(c.fg).bg(c.bg).add_modifier(c.modifier);
                if colour && last != Some(style) {
                    out.push_str(&sgr(style));
                    last = Some(style);
                }
                out.push_str(c.symbol());
            }
            if colour {
                out.push_str("\u{1b}[0m");
            }
            out
        })
        .collect()
}

/// The SGR sequence for `s` (a reset first, then its colours and weight).
fn sgr(s: Style) -> String {
    let mut codes = vec!["0".to_string()];
    let rgb = |c: Option<ratatui::style::Color>, base: u8| match c {
        Some(ratatui::style::Color::Rgb(r, g, b)) => Some(format!("{base};2;{r};{g};{b}")),
        _ => None,
    };
    codes.extend(rgb(s.fg, 38));
    codes.extend(rgb(s.bg, 48));
    let m = s.add_modifier;
    for (bit, code) in [(Modifier::BOLD, "1"), (Modifier::DIM, "2"), (Modifier::ITALIC, "3"), (Modifier::UNDERLINED, "4"), (Modifier::REVERSED, "7")] {
        if m.contains(bit) {
            codes.push(code.into());
        }
    }
    format!("\u{1b}[{}m", codes.join(";"))
}

/// For tests: a style is bold.
pub fn is_bold(s: Style) -> bool {
    s.add_modifier.contains(Modifier::BOLD)
}

#[cfg(test)]
mod separator_tests {
    use super::*;
    use crate::app::{MenuAction, MenuItem};
    use nimitz_core::{Issue, Plan, Severity};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn text(buf: &ratatui::buffer::Buffer) -> String {
        let a = buf.area;
        (0..a.height).map(|y| (0..a.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    /// A menu label longer than its column still gets a separator before the
    /// reason it is disabled (acceptance F4: "stale job rowsno stale job rows").
    #[test]
    fn a_long_menu_label_is_separated_from_its_reason() {
        let items = vec![
            MenuItem { key: 'w', label: "sweep: stop and remove copies, remove stale job rows".into(), enabled: Err("no stale job rows".into()), action: MenuAction::NewFleet },
            MenuItem { key: 'l', label: "layout --ships: and a tab attaching each live ship".into(), enabled: Err(crate::actions::MONITOR_ONLY.into()), action: MenuAction::NewFleet },
        ];
        let mut t = Terminal::new(TestBackend::new(140, 12)).unwrap();
        t.draw(|f| f.render_widget(Paragraph::new(menu_lines(&items, 0, 96, false).0), f.area())).unwrap();
        let shown = text(t.backend().buffer());
        assert!(shown.contains("stale job rows  (no stale job rows)"), "{shown}");
        assert!(shown.contains("each live ship  (monitor-only: no multiplexer)"), "{shown}");
    }

    /// A warning code longer than its column still gets a space before the
    /// message (acceptance F4: "labels.missinga199ee34-...").
    #[test]
    fn a_long_warning_code_is_separated_from_its_message() {
        let plan = Plan {
            summary: "adopt".into(),
            warnings: vec![Issue { code: "labels.missing".into(), severity: Severity::Warn, message: "a199ee34-0000 has no labels".into(), fix: None, id: None }],
            ..Default::default()
        };
        let lines = plan_lines(&plan, &[], 120);
        let shown: String = lines.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert!(shown.contains("labels.missing a199ee34-0000"), "{shown}");
    }
}
