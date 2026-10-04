//! The tree as a list of frames: one gold double-line frame per fleet (its
//! admiral the first row inside), a rounded frame per squadron nested in it, and
//! a pink frame for stowaways. Every frame's title is a full-width tinted band.
//! Columns NAME / STATUS / AGE / KIND / DETAIL start at the same x in every
//! frame, so they line up down the whole list. Pure: rows in, lines out.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::theme::{bold, mono, pane_color, pane_dot, severity_color, st, Icons};
use crate::view::{Row, Section};

//                 NAME STATUS AGE KIND DETAIL
pub const COL_MIN: [u16; 5] = [30, 12, 4, 9, 0];
pub const COL_MAX: [u16; 5] = [58, 14, 5, 10, 96];
/// how the columns grow: the small fixed ones first, then NAME and DETAIL take
/// turns, so a narrow window still shows some DETAIL (column, up to width)
const GROW: [(usize, u16); 7] = [(1, 14), (2, 5), (3, 10), (0, 40), (4, 40), (0, 58), (4, 96)];
/// columns a frame takes on each side of a row: outer border + inner border (or a space)
pub const FRAME_SIDE: usize = 2;

/// Column widths for rows `w` columns wide (one space between columns).
pub fn col_widths(w: u16) -> [u16; 5] {
    let gaps = COL_MIN.len() as u16 - 1;
    let mut out = COL_MIN;
    let mut extra = w.saturating_sub(COL_MIN.iter().sum::<u16>() + gaps);
    for (i, upto) in GROW {
        let add = upto.saturating_sub(out[i]).min(extra);
        out[i] += add;
        extra -= add;
    }
    out
}

/// The widest list worth giving room to (all columns at their maximum, plus frames).
pub fn list_max_width() -> u16 {
    COL_MAX.iter().sum::<u16>() + (COL_MAX.len() as u16 - 1) + 2 * FRAME_SIDE as u16
}

pub fn truncate(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let mut out = String::new();
    for ch in s.chars() {
        if out.width() + ch.to_string().width() + 1 > w {
            break;
        }
        out.push(ch);
    }
    out.push('\u{2026}');
    out
}

pub fn pad(s: &str, w: usize) -> String {
    let n = s.width();
    if n >= w {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(w - n))
    }
}

/// Cut spans to `w` columns.
pub fn truncate_spans(spans: Vec<Span<'static>>, w: usize) -> Vec<Span<'static>> {
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

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// NAME cell: pane dot, issue mark, tree guides, fold marker, icon, name.
pub fn name_cell(r: &Row, width: usize, icons: &Icons) -> Vec<Span<'static>> {
    let n = &r.node;
    let dot = match &n.pane_state {
        Some(state) => Span::styled(format!("{} ", pane_dot(state)), bold(pane_color(state))),
        None => Span::raw("  "),
    };
    let mark = match n.worst {
        _ if n.legacy => Span::styled(format!("{} ", icons.legacy), bold(mono::ORANGE)),
        Some(sev) => Span::styled(format!("{} ", icons.issue), bold(severity_color(sev))),
        None => Span::raw("  "),
    };
    let marker = if n.children.is_empty() {
        "  "
    } else if r.expanded {
        "\u{25be} "
    } else {
        "\u{25b8} "
    };
    let room = width.saturating_sub(4 + r.prefix.width() + 2 + n.icon.width() + 1).max(3);
    vec![
        dot,
        mark,
        Span::styled(r.prefix.clone(), st(mono::DIM)),
        Span::styled(marker, st(mono::COMMENT)),
        Span::styled(format!("{} ", n.icon), n.icon_style),
        Span::styled(pad(&truncate(&n.name, room), room), n.name_style),
    ]
}

/// A row: NAME, STATUS, AGE, KIND, DETAIL, exactly `inner` columns wide.
pub fn row_spans(r: &Row, w: &[u16; 5], inner: usize, icons: &Icons) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = truncate_spans(name_cell(r, w[0] as usize, icons), w[0] as usize);
    let used_name = spans_width(&out);
    if used_name < w[0] as usize {
        out.push(Span::raw(" ".repeat(w[0] as usize - used_name)));
    }
    let n = &r.node;
    out.push(Span::raw(" "));
    out.push(Span::styled(pad(&truncate(&n.status.0, w[1] as usize), w[1] as usize), n.status.1));
    out.push(Span::raw(" "));
    out.push(Span::styled(format!("{:>width$}", truncate(&n.age, w[2] as usize), width = w[2] as usize), st(mono::COMMENT)));
    out.push(Span::raw(" "));
    out.push(Span::styled(pad(&truncate(&n.kind_text, w[3] as usize), w[3] as usize), st(mono::COMMENT)));
    out.push(Span::raw(" "));
    let used = spans_width(&out);
    let room = inner.saturating_sub(used);
    let detail = truncate_spans(n.detail.clone(), room);
    let dw = spans_width(&detail);
    out.extend(detail);
    if dw < room {
        out.push(Span::raw(" ".repeat(room - dw)));
    }
    out
}

/// The column header, aligned with the rows inside the frames.
pub fn header_line(width: u16) -> Line<'static> {
    let inner = (width as usize).saturating_sub(2 * FRAME_SIDE);
    let w = col_widths(inner as u16);
    let mut spans = vec![Span::raw(" ".repeat(FRAME_SIDE))];
    for (k, (name, cw)) in ["    NAME", "STATUS", "AGE", "KIND", "DETAIL"].iter().zip(w.iter()).enumerate() {
        let text = if k == 2 { format!("{:>w$}", name, w = *cw as usize) } else { pad(name, *cw as usize) };
        spans.push(Span::styled(text, bold(mono::COMMENT)));
        if k < 4 {
            spans.push(Span::raw(" "));
        }
    }
    Line::from(truncate_spans(spans, width as usize))
}

/// A title band `width` columns wide including its two corner cells.
/// `pad` columns before the right corner keep top-level and nested bands' counts in one column.
fn band(r: &Row, width: usize, corners: (&str, &str), edge: Style, selected: bool, pad_right: usize) -> Vec<Span<'static>> {
    let n = &r.node;
    let color = n.frame.unwrap_or(mono::SOFT);
    let bg = if selected { mono::SEL } else { n.band_bg.unwrap_or(mono::BG_HI) };
    let b = Style::new().bg(bg);
    let marker = if n.children.is_empty() {
        "\u{25b8}"
    } else if r.expanded {
        "\u{25be}"
    } else {
        "\u{25b8}"
    };
    let mut left = vec![Span::styled(format!(" {marker} "), b.fg(if n.children.is_empty() { mono::DIM } else { color }))];
    if let Some(state) = &n.pane_state {
        left.push(Span::styled(format!("{} ", pane_dot(state)), b.fg(pane_color(state)).add_modifier(Modifier::BOLD)));
    }
    left.push(Span::styled(format!("{} ", n.icon), b.patch(n.icon_style)));
    left.push(Span::styled(n.name.clone(), b.patch(n.name_style).add_modifier(Modifier::BOLD)));
    if !n.detail.is_empty() {
        left.push(Span::styled("   ", b));
        left.extend(n.detail.iter().map(|s| Span::styled(s.content.clone(), b.patch(s.style))));
    }
    let mut right: Vec<Span<'static>> = n.title_right.iter().map(|s| Span::styled(s.content.clone(), b.patch(s.style))).collect();
    right.push(Span::styled(" ".repeat(pad_right), b));
    let room = width.saturating_sub(2);
    let right_w = spans_width(&right);
    let left = truncate_spans(left, room.saturating_sub(right_w + 1));
    let used = spans_width(&left);
    let mut out = vec![Span::styled(corners.0.to_string(), edge.bg(bg))];
    out.extend(left);
    out.push(Span::styled(" ".repeat(room.saturating_sub(used + right_w)), b));
    out.extend(right);
    out.push(Span::styled(corners.1.to_string(), edge.bg(bg)));
    out
}

/// Gold for a fleet's frame; dimmed for a fleet that does not run here.
fn fleet_edge(r: &Row) -> Style {
    if r.node.frame == Some(mono::DIM) {
        st(mono::GOLD_DIM)
    } else {
        bold(mono::GOLD)
    }
}

/// The cursor row: selection background, and dim text lifted so it stays readable.
pub fn selectable(line: Line<'static>, selected: bool) -> Line<'static> {
    if !selected {
        return line;
    }
    let spans = line
        .spans
        .into_iter()
        .map(|s| match s.style.fg {
            Some(c) if c == mono::DIM || c == mono::COMMENT => Span::styled(s.content, s.style.fg(mono::SOFT)),
            _ => s,
        })
        .collect::<Vec<_>>();
    Line::from(spans).style(Style::new().bg(mono::SEL).add_modifier(Modifier::BOLD))
}

/// One line per band, row or border; the `usize` is the row a line selects.
pub fn frame_lines(rows: &[Row], width: u16, cursor: usize, icons: &Icons) -> Vec<(Line<'static>, Option<usize>)> {
    let width = width as usize;
    let inner = width.saturating_sub(2 * FRAME_SIDE);
    let w = col_widths(inner as u16);
    let mut out: Vec<(Line<'static>, Option<usize>)> = vec![];
    let mut i = 0;
    while i < rows.len() {
        let head = &rows[i];
        let (corners, v, bottom, edge) = match head.section {
            Section::FleetBand => (("\u{2554}", "\u{2557}"), "\u{2551}", ("\u{255a}", "\u{2550}", "\u{255d}"), fleet_edge(head)),
            _ => {
                let c = head.node.frame.unwrap_or(mono::PINK);
                (("\u{256d}", "\u{256e}"), "\u{2502}", ("\u{2570}", "\u{2500}", "\u{256f}"), st(c))
            }
        };
        if !out.is_empty() {
            out.push((Line::raw(""), None));
        }
        out.push((Line::from(band(head, width, corners, edge, i == cursor, 2)), Some(i)));
        i += 1;
        let mut sq_open: Option<Style> = None;
        while i < rows.len() && !matches!(rows[i].section, Section::FleetBand | Section::StowBand) {
            let r = &rows[i];
            match r.section {
                Section::SquadronBand => {
                    if let Some(e) = sq_open.take() {
                        out.push((sq_bottom(v, edge, e, inner), None));
                    }
                    let c = r.node.frame.unwrap_or(mono::SOFT);
                    let e = st(c);
                    let boxed = r.expanded;
                    let corners = if boxed { ("\u{256d}", "\u{256e}") } else { (" ", " ") };
                    let mut spans = vec![Span::styled(v, edge)];
                    spans.extend(band(r, inner + 2, corners, e, i == cursor, 1));
                    spans.push(Span::styled(v, edge));
                    out.push((Line::from(spans), Some(i)));
                    if boxed {
                        sq_open = Some(e);
                    }
                }
                Section::SquadronBody => {
                    let e = sq_open.unwrap_or(edge);
                    let mut spans = vec![Span::styled(v, edge), Span::styled("\u{2502}", e)];
                    spans.extend(row_spans(r, &w, inner, icons));
                    spans.push(Span::styled("\u{2502}", e));
                    spans.push(Span::styled(v, edge));
                    out.push((selectable(Line::from(spans), i == cursor), Some(i)));
                }
                _ => {
                    if let Some(e) = sq_open.take() {
                        out.push((sq_bottom(v, edge, e, inner), None));
                    }
                    let mut spans = vec![Span::styled(v, edge), Span::raw(" ")];
                    spans.extend(row_spans(r, &w, inner, icons));
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(v, edge));
                    out.push((selectable(Line::from(spans), i == cursor), Some(i)));
                }
            }
            i += 1;
        }
        if let Some(e) = sq_open.take() {
            out.push((sq_bottom(v, edge, e, inner), None));
        }
        let (bl, hz, br) = bottom;
        out.push((Line::styled(format!("{bl}{}{br}", hz.repeat(width.saturating_sub(2))), edge), None));
    }
    out
}

fn sq_bottom(v: &str, edge: Style, e: Style, inner: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled(v.to_string(), edge),
        Span::styled(format!("\u{2570}{}\u{256f}", "\u{2500}".repeat(inner)), e),
        Span::styled(v.to_string(), edge),
    ])
}
