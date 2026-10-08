//! The tree as a list of boxes. A fleet is one tinted band line (its state
//! and counts; the fleet's tab already frames it), then its admiral in a
//! heavy box (`┏━ ADMIRAL ━┓`), then a rounded box per squadron whose top
//! border carries its title in the squadron's status colour, and a box for
//! stowaways. Columns NAME / STATUS / AGE / KIND / DETAIL start at the same x
//! in every box, so they line up down the whole list. Spacing (blank lines
//! between boxes, padding inside them, wider column gaps) is the first thing
//! to go when the width or the height runs short (`Spacing`). Pure: rows in,
//! lines out.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::theme::{bold, mono, pane_color, pane_dot, severity_color, st, Icons};
use crate::view::{Row, Section};

//                 NAME STATUS AGE KIND DETAIL
pub const COL_MIN: [u16; 5] = [24, 12, 4, 9, 0];
pub const COL_MAX: [u16; 5] = [58, 14, 5, 10, 96];
/// how the columns grow: the small fixed ones first, then NAME and DETAIL take
/// turns, so a narrow window still shows some DETAIL (column, up to width)
/// DETAIL first gets enough for an issue code or two
const GROW: [(usize, u16); 8] = [(4, 22), (1, 14), (2, 5), (3, 10), (0, 40), (4, 40), (0, 58), (4, 96)];
/// The room around the content: blank lines between boxes, padding inside a
/// box (columns each side), the gap between columns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    pub blank: bool,
    pub pad: usize,
    pub gap: usize,
}

/// Nothing but the content.
pub const TIGHT: Spacing = Spacing { blank: false, pad: 0, gap: 1 };
/// Room to breathe, when it costs no content.
pub const ROOMY: Spacing = Spacing { blank: true, pad: 1, gap: 2 };
/// NAME keeps this much, and DETAIL the room for an issue code or two
/// (its first step in `GROW`), before the padding and gaps go.
const KEEP_NAME: u16 = 40;
const KEEP_DETAIL: u16 = 22;

impl Spacing {
    /// Columns a box takes on each side of a row: its border and the padding.
    pub fn side(self) -> usize {
        1 + self.pad
    }
    /// The width the columns get in a list `width` columns wide.
    pub fn inner(self, width: usize) -> usize {
        width.saturating_sub(2 * self.side())
    }
}

/// The spacing a list `width` columns wide affords: the padding and wider
/// gaps while STATUS, AGE and KIND keep their tight widths, NAME keeps
/// `KEEP_NAME` and DETAIL `KEEP_DETAIL` columns (or all they had tight):
/// from 103 columns. The blank lines between boxes are on here: `layout` drops them
/// only when the list does not fit the height.
pub fn spacing_for(width: usize) -> Spacing {
    let tight = col_widths_gap(TIGHT.inner(width) as u16, TIGHT.gap as u16);
    let roomy = col_widths_gap(ROOMY.inner(width) as u16, ROOMY.gap as u16);
    // (DETAIL must exist at all: at the minimum widths both spacings agree
    // on nothing but cut columns)
    if roomy[4] > 0 && tight[1..4] == roomy[1..4] && roomy[0] >= KEEP_NAME.min(tight[0]) && roomy[4] >= KEEP_DETAIL.min(tight[4]) {
        ROOMY
    } else {
        Spacing { blank: true, ..TIGHT }
    }
}

/// Column widths for rows `w` columns wide (one space between columns).
pub fn col_widths(w: u16) -> [u16; 5] {
    col_widths_gap(w, 1)
}

/// Column widths for rows `w` columns wide, `gap` spaces between columns.
pub fn col_widths_gap(w: u16, gap: u16) -> [u16; 5] {
    let gaps = (COL_MIN.len() as u16 - 1) * gap;
    let mut out = COL_MIN;
    let mut extra = w.saturating_sub(COL_MIN.iter().sum::<u16>() + gaps);
    for (i, upto) in GROW {
        let add = upto.saturating_sub(out[i]).min(extra);
        out[i] += add;
        extra -= add;
    }
    out
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
        _ if n.legacy => Span::styled(format!("{} ", icons.legacy), bold(mono::WARN)),
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

/// A row's lines: one for a member, as many as it takes for a note (an error
/// shown in full, wrapped with its indent).
fn body_spans(r: &Row, w: &[u16; 5], inner: usize, gap: usize, icons: &Icons) -> Vec<Vec<Span<'static>>> {
    if r.node.kind != crate::view::Kind::Note {
        return vec![row_spans_gap(r, w, inner, gap, icons)];
    }
    let lead = r.node.name.len() - r.node.name.trim_start().len();
    wrap(vec![Span::styled(format!("    {}", r.node.name), r.node.name_style)], inner, 4 + lead)
        .into_iter()
        .map(|l| {
            let used: usize = l.spans.iter().map(|s| s.content.width()).sum();
            let mut v = l.spans;
            v.push(Span::raw(" ".repeat(inner.saturating_sub(used))));
            v
        })
        .collect()
}

/// A row: NAME, STATUS, AGE, KIND, DETAIL, exactly `inner` columns wide.
pub fn row_spans(r: &Row, w: &[u16; 5], inner: usize, icons: &Icons) -> Vec<Span<'static>> {
    row_spans_gap(r, w, inner, 1, icons)
}

/// `row_spans` with `gap` spaces between the columns.
pub fn row_spans_gap(r: &Row, w: &[u16; 5], inner: usize, gap: usize, icons: &Icons) -> Vec<Span<'static>> {
    let sep = " ".repeat(gap);
    let mut out: Vec<Span<'static>> = truncate_spans(name_cell(r, w[0] as usize, icons), w[0] as usize);
    let used_name = spans_width(&out);
    if used_name < w[0] as usize {
        out.push(Span::raw(" ".repeat(w[0] as usize - used_name)));
    }
    let n = &r.node;
    out.push(Span::raw(sep.clone()));
    out.push(Span::styled(pad(&truncate(&n.status.0, w[1] as usize), w[1] as usize), n.status.1));
    out.push(Span::raw(sep.clone()));
    out.push(Span::styled(format!("{:>width$}", truncate(&n.age, w[2] as usize), width = w[2] as usize), st(mono::COMMENT)));
    out.push(Span::raw(sep.clone()));
    out.push(Span::styled(pad(&truncate(&n.kind_text, w[3] as usize), w[3] as usize), st(mono::COMMENT)));
    out.push(Span::raw(sep));
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

/// The column header, aligned with the rows inside the boxes.
pub fn header_line(width: u16) -> Line<'static> {
    header_line_with(width, spacing_for(width as usize))
}

/// `header_line` for the spacing the list is drawn with.
pub fn header_line_with(width: u16, sp: Spacing) -> Line<'static> {
    let inner = sp.inner(width as usize);
    let w = col_widths_gap(inner as u16, sp.gap as u16);
    let mut spans = vec![Span::raw(" ".repeat(sp.side()))];
    for (k, (name, cw)) in ["    NAME", "STATUS", "AGE", "KIND", "DETAIL"].iter().zip(w.iter()).enumerate() {
        let text = if k == 2 { format!("{:>w$}", name, w = *cw as usize) } else { pad(name, *cw as usize) };
        spans.push(Span::styled(text, bold(mono::COMMENT)));
        if k < 4 {
            spans.push(Span::raw(" ".repeat(sp.gap)));
        }
    }
    Line::from(truncate_spans(spans, width as usize))
}

/// A title band `width` columns wide including its two corner cells.
/// Bands narrower than this show only the key counts (needs you, errors).
pub const NARROW_BAND: usize = 100;

/// `pad` columns before the right corner keep top-level and nested bands' counts in one column.
/// With `rule` the band is a box's top border instead: no tint, the gaps
/// drawn as `─` in the edge colour, the title inside the line.
fn band(r: &Row, width: usize, corners: (&str, &str), edge: Style, selected: bool, pad_right: usize, rule: bool) -> Vec<Span<'static>> {
    let n = &r.node;
    let color = n.frame.unwrap_or(mono::SOFT);
    let bg = if selected { mono::SEL } else if rule { mono::BG } else { n.band_bg.unwrap_or(mono::BG_HI) };
    let b = Style::new().bg(bg);
    let fill = |k: usize| if rule { Span::styled("\u{2500}".repeat(k), edge.bg(bg)) } else { Span::styled(" ".repeat(k), b) };
    // on the cursor band, faint and muted text is lifted so it stays readable
    let lift = |s: Style| match s.fg {
        Some(c) if selected && (c == mono::FAINT || c == mono::MUTED) => s.fg(mono::FG),
        _ => s,
    };
    let marker = if n.children.is_empty() {
        "\u{25b8}"
    } else if r.expanded {
        "\u{25be}"
    } else {
        "\u{25b8}"
    };
    let mut left = if rule { vec![fill(1)] } else { vec![] };
    left.push(Span::styled(format!(" {marker} "), lift(b.fg(if n.children.is_empty() { mono::FAINT } else { color }))));
    if let Some(state) = &n.pane_state {
        left.push(Span::styled(format!("{} ", pane_dot(state)), b.fg(pane_color(state)).add_modifier(Modifier::BOLD)));
    }
    left.push(Span::styled(format!("{} ", n.icon), lift(b.patch(n.icon_style))));
    left.push(Span::styled(n.name.clone(), lift(b.patch(n.name_style)).add_modifier(Modifier::BOLD)));
    if !n.detail.is_empty() {
        left.push(Span::styled("   ", b));
        left.extend(n.detail.iter().map(|s| Span::styled(s.content.clone(), lift(b.patch(s.style)))));
    }
    if rule {
        left.push(Span::styled(" ", b));
    }
    let mut right: Vec<Span<'static>> = n.title_right.iter().map(|s| Span::styled(s.content.clone(), lift(b.patch(s.style)))).collect();
    // counts: `icon n`, slashes between them, right-aligned as a group; a narrow
    // band keeps only needs-you and errors
    let narrow = width < NARROW_BAND;
    let shown: Vec<&crate::view::Count> = n.counts.iter().filter(|c| !narrow || c.key).collect();
    for (i, c) in shown.iter().enumerate() {
        if i > 0 {
            right.push(Span::styled(" / ", lift(b.fg(mono::FAINT))));
        }
        right.push(Span::styled(format!("{}{}", c.icon, c.n), lift(b.patch(c.style))));
    }
    // on a border line the counts sit in a gap of their own
    if rule && !right.is_empty() {
        right.insert(0, Span::styled(" ", b));
        right.push(Span::styled(" ", b));
    }
    right.push(fill(pad_right));
    let room = width.saturating_sub(2);
    let right_w = spans_width(&right);
    let left = truncate_spans(left, room.saturating_sub(right_w + 1));
    let used = spans_width(&left);
    let mut out = vec![Span::styled(corners.0.to_string(), edge.bg(bg))];
    out.extend(left);
    out.push(fill(room.saturating_sub(used + right_w)));
    out.extend(right);
    out.push(Span::styled(corners.1.to_string(), edge.bg(bg)));
    out
}

/// A fleet's frame: neutral, or the hue of what it needs (its frame colour);
/// error when its config does not load.
fn fleet_edge(r: &Row) -> Style {
    if r.node.children.iter().any(|c| c.kind == crate::view::Kind::Note) {
        bold(mono::ERROR) // its config does not load
    } else {
        st(r.node.frame.unwrap_or(mono::LINE))
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
            Some(c) if c == mono::FAINT || c == mono::MUTED => Span::styled(s.content, s.style.fg(mono::FG)),
            _ => s,
        })
        .collect::<Vec<_>>();
    Line::from(spans).style(Style::new().bg(mono::SEL).add_modifier(Modifier::BOLD))
}

/// One line per band, row or border; the `usize` is the row a line selects.
/// The spacing is the width's (`spacing_for`), blank lines included.
pub fn frame_lines(rows: &[Row], width: u16, cursor: usize, icons: &Icons) -> Vec<(Line<'static>, Option<usize>)> {
    layout(rows, width, None, cursor, icons).0
}

/// The list's lines and the spacing they use: the width's spacing, and the
/// blank lines between boxes only while everything fits in `height` lines
/// (spacing goes before any row does).
pub fn layout(rows: &[Row], width: u16, height: Option<usize>, cursor: usize, icons: &Icons) -> (Vec<(Line<'static>, Option<usize>)>, Spacing) {
    let mut sp = spacing_for(width as usize);
    let mut lines = lines_with(rows, width as usize, cursor, icons, sp);
    if sp.blank && height.is_some_and(|h| lines.len() > h) {
        sp.blank = false;
        lines = lines_with(rows, width as usize, cursor, icons, sp);
    }
    (lines, sp)
}

/// A box's edges: its side glyph and style, and its bottom.
#[derive(Clone, Copy)]
struct Edges {
    v: &'static str,
    bottom: (&'static str, &'static str, &'static str),
    style: Style,
}

const ROUND: (&str, (&str, &str, &str)) = ("\u{2502}", ("\u{2570}", "\u{2500}", "\u{256f}"));
const HEAVY: (&str, (&str, &str, &str)) = ("\u{2503}", ("\u{2517}", "\u{2501}", "\u{251b}"));

fn lines_with(rows: &[Row], width: usize, cursor: usize, icons: &Icons, sp: Spacing) -> Vec<(Line<'static>, Option<usize>)> {
    let inner = sp.inner(width);
    let w = col_widths_gap(inner as u16, sp.gap as u16);
    let pad = " ".repeat(sp.pad);
    let mut out: Vec<(Line<'static>, Option<usize>)> = vec![];
    // a row inside a box: border, padding, the row, padding, border
    let boxed_row = |out: &mut Vec<(Line<'static>, Option<usize>)>, r: &Row, i: usize, e: Edges| {
        for body in body_spans(r, &w, inner, sp.gap, icons) {
            let mut spans = vec![Span::styled(e.v, e.style), Span::raw(pad.clone())];
            spans.extend(body);
            spans.push(Span::raw(pad.clone()));
            spans.push(Span::styled(e.v, e.style));
            out.push((selectable(Line::from(spans), i == cursor), Some(i)));
        }
    };
    let close = |out: &mut Vec<(Line<'static>, Option<usize>)>, e: Edges| {
        let (l, h, r) = e.bottom;
        out.push((Line::styled(format!("{l}{}{r}", h.repeat(width.saturating_sub(2))), e.style), None));
    };
    let blank = |out: &mut Vec<(Line<'static>, Option<usize>)>| {
        if sp.blank && out.last().is_some_and(|(l, _)| !l.spans.is_empty()) {
            out.push((Line::raw(""), None));
        }
    };
    let mut i = 0;
    while i < rows.len() {
        let head = &rows[i];
        // a blank line between fleets and before the stowaways, as spacing
        blank(&mut out);
        let stow = head.section == Section::StowBand;
        if stow {
            // the stowaways: a rounded box, its band the top border
            let e = Edges { v: ROUND.0, bottom: ROUND.1, style: st(head.node.frame.unwrap_or(mono::LINE)) };
            let corners = if head.expanded { ("\u{256d}", "\u{256e}") } else { ("\u{2576}", "\u{2574}") };
            out.push((Line::from(band(head, width, corners, e.style, i == cursor, 1, true)), Some(i)));
            i += 1;
            let mut any = false;
            while i < rows.len() && !matches!(rows[i].section, Section::FleetBand | Section::StowBand) {
                boxed_row(&mut out, &rows[i], i, e);
                any = true;
                i += 1;
            }
            if any {
                close(&mut out, e);
            }
            continue;
        }
        // the fleet: one tinted band line, no frame (its tab frames it)
        // (its counts end in the column the squadrons' do, before their `─╮`)
        out.push((Line::from(band(head, width, (" ", " "), fleet_edge(head), i == cursor, 2, false)), Some(i)));
        i += 1;
        // what is open: the admiral's box, or a squadron's
        let mut open: Option<Edges> = None;
        while i < rows.len() && !matches!(rows[i].section, Section::FleetBand | Section::StowBand) {
            let r = &rows[i];
            match r.section {
                Section::SquadronBand => {
                    if let Some(e) = open.take() {
                        close(&mut out, e);
                    }
                    blank(&mut out);
                    let e = Edges { v: ROUND.0, bottom: ROUND.1, style: st(r.node.frame.unwrap_or(mono::LINE)) };
                    let corners = if r.expanded { ("\u{256d}", "\u{256e}") } else { ("\u{2576}", "\u{2574}") };
                    out.push((Line::from(band(r, width, corners, e.style, i == cursor, 1, true)), Some(i)));
                    if r.expanded {
                        // the sides stay neutral: the top border carries the status
                        open = Some(Edges { style: st(mono::LINE), ..e });
                    }
                }
                Section::SquadronBody => match open {
                    Some(e) => boxed_row(&mut out, r, i, e),
                    None => boxed_row(&mut out, r, i, Edges { v: " ", bottom: (" ", " ", " "), style: Style::new() }),
                },
                _ if r.node.kind == crate::view::Kind::Note => {
                    // a fleet's note (its config does not load, squadrons the state
                    // filter hides), in full, outside any box
                    if let Some(e) = open.take() {
                        close(&mut out, e);
                    }
                    // a gap from the box above; a broken fleet's error stays
                    // right under its band
                    if rows.get(i.wrapping_sub(1)).is_some_and(|p| p.section != Section::FleetBand) {
                        blank(&mut out);
                    }
                    for body in body_spans(r, &w, inner, sp.gap, icons) {
                        let mut spans = vec![Span::raw(" ".repeat(sp.side()))];
                        spans.extend(body);
                        out.push((selectable(Line::from(spans), i == cursor), Some(i)));
                    }
                }
                _ => {
                    // the admiral and its crew: a heavy box with a bright title
                    if open.is_none() {
                        blank(&mut out);
                        let e = Edges { v: HEAVY.0, bottom: HEAVY.1, style: st(mono::FG) };
                        let title = " ADMIRAL ";
                        let rest = width.saturating_sub(3 + title.width());
                        out.push((
                            Line::from(vec![
                                Span::styled("\u{250f}\u{2501}", e.style),
                                Span::styled(title, bold(mono::STRONG)),
                                Span::styled(format!("{}\u{2513}", "\u{2501}".repeat(rest)), e.style),
                            ]),
                            None,
                        ));
                        open = Some(e);
                    }
                    boxed_row(&mut out, r, i, open.unwrap());
                }
            }
            i += 1;
        }
        if let Some(e) = open.take() {
            close(&mut out, e);
        }
    }
    out
}

/// Wrap styled text to `width` columns at spaces; continuation lines start
/// with `indent` spaces, so wrapped text keeps its column.
pub fn wrap(spans: Vec<Span<'static>>, width: usize, indent: usize) -> Vec<Line<'static>> {
    let width = width.max(indent + 8);
    let mut lines: Vec<Line<'static>> = vec![];
    let mut cur: Vec<Span<'static>> = vec![];
    let mut used = 0usize;
    for sp in spans {
        let style = sp.style;
        // split into words, each with its trailing spaces
        let mut words: Vec<String> = vec![];
        let mut w = String::new();
        for ch in sp.content.chars() {
            if ch == ' ' {
                w.push(ch);
            } else {
                if w.ends_with(' ') {
                    words.push(std::mem::take(&mut w));
                }
                w.push(ch);
            }
        }
        if !w.is_empty() {
            words.push(w);
        }
        for word in words {
            let mut word = word;
            loop {
                let ww = word.trim_end().width();
                if used + ww <= width || (used <= indent && ww > width - indent) {
                    // fits, or a single word longer than a line: cut it
                    let room = width - used;
                    if word.width() > room && ww > room {
                        let cut: String = {
                            let mut c = String::new();
                            for ch in word.chars() {
                                if c.width() + ch.to_string().width() > room {
                                    break;
                                }
                                c.push(ch);
                            }
                            // a path or file:line breaks after a `/` or `:` when it can
                            match c.rfind(['/', ':']) {
                                Some(i) if i + 1 > c.len() / 2 => c[..=i].to_string(),
                                _ => c,
                            }
                        };
                        word = word[cut.len()..].to_string();
                        cur.push(Span::styled(cut, style));
                        lines.push(Line::from(std::mem::take(&mut cur)));
                        cur.push(Span::raw(" ".repeat(indent)));
                        used = indent;
                        continue;
                    }
                    used += word.width();
                    cur.push(Span::styled(word, style));
                    break;
                }
                lines.push(Line::from(std::mem::take(&mut cur)));
                cur.push(Span::raw(" ".repeat(indent)));
                used = indent;
            }
        }
    }
    if !cur.is_empty() {
        lines.push(Line::from(cur));
    }
    lines
}
