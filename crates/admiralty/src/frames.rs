//! The tree as a list of sections, each a filled bar in one grammar (`bar`:
//! fold marker, glyph, NAME, role, state, size; KB and tallies on the right)
//! over a rounded box of its rows: the fleet's bar over its admiral and
//! crew, a squadron's over its commodore and ships, the stowaways'. Each
//! bar is filled with the worst state in its section. Columns NAME / STATUS / AGE / DETAIL start at the same x
//! in every box, so they line up down the whole list. Spacing (blank lines
//! between boxes, padding inside them, wider column gaps) is the first thing
//! to go when the width or the height runs short (`Spacing`). Pure: rows in,
//! lines out.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::theme::{bold, mono, pane_color, pane_dot, st, Icons};
use crate::view::{Kind, Row, Section};

//                 NAME STATUS AGE DETAIL
pub const COL_MIN: [u16; 4] = [12, 12, 4, 0];
/// NAME, STATUS and AGE stop here however wide the window: DETAIL takes all
/// the rest (no maximum)
pub const COL_MAX: [u16; 4] = [29, 14, 5, u16::MAX];
/// how the columns grow: the small fixed ones first, then NAME and DETAIL take
/// turns, so a narrow window still shows some DETAIL (column, up to width)
/// DETAIL first gets enough for an issue code or two
const GROW: [(usize, u16); 7] = [(3, 22), (1, 14), (2, 5), (0, 20), (3, 40), (0, 29), (3, u16::MAX)];
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
const KEEP_NAME: u16 = 20;
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
/// gaps while STATUS and AGE keep their tight widths, NAME keeps
/// `KEEP_NAME` and DETAIL `KEEP_DETAIL` columns (or all they had tight):
/// from 71 columns. The blank lines between boxes are on here: `layout` drops them
/// only when the list does not fit the height.
pub fn spacing_for(width: usize) -> Spacing {
    let tight = col_widths_gap(TIGHT.inner(width) as u16, TIGHT.gap as u16);
    let roomy = col_widths_gap(ROOMY.inner(width) as u16, ROOMY.gap as u16);
    // (DETAIL must exist at all: at the minimum widths both spacings agree
    // on nothing but cut columns)
    if roomy[3] > 0 && tight[1..3] == roomy[1..3] && roomy[0] >= KEEP_NAME.min(tight[0]) && roomy[3] >= KEEP_DETAIL.min(tight[3]) {
        ROOMY
    } else {
        Spacing { blank: true, ..TIGHT }
    }
}

/// Column widths for rows `w` columns wide (one space between columns).
pub fn col_widths(w: u16) -> [u16; 4] {
    col_widths_gap(w, 1)
}

/// Column widths for rows `w` columns wide, `gap` spaces between columns.
pub fn col_widths_gap(w: u16, gap: u16) -> [u16; 4] {
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


/// `s` cut to `w` columns from the left (`…ARGOS]-BOLT`).
pub fn truncate_left(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let mut kept: Vec<char> = vec![];
    for ch in s.chars().rev() {
        if kept.iter().collect::<String>().width() + ch.to_string().width() + 1 > w {
            break;
        }
        kept.push(ch);
    }
    std::iter::once('\u{2026}').chain(kept.into_iter().rev()).collect()
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

/// `spans` in `w` columns, whole: the highest `rank` goes first (the order
/// stays); what one span alone cannot fit is cut.
pub fn drop_spans(spans: &[Span<'static>], rank: &[u8], w: usize) -> Vec<Span<'static>> {
    let mut keep: Vec<bool> = vec![true; spans.len()];
    let width = |keep: &[bool]| spans.iter().zip(keep).filter(|(_, k)| **k).map(|(s, _)| s.content.width()).sum::<usize>();
    while width(&keep) > w && keep.iter().filter(|k| **k).count() > 1 {
        let worst = (0..spans.len()).filter(|i| keep[*i]).max_by_key(|i| (rank[*i], *i)).unwrap_or(0);
        keep[worst] = false;
    }
    truncate_spans(spans.iter().zip(&keep).filter(|(_, k)| **k).map(|(s, _)| s.clone()).collect(), w)
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// The column a row's fold marker starts at, from the left of the list (`None`:
/// a row that doesn't fold). `bar` and `name_cell` draw it there.
pub fn fold_x(r: &Row, sp: Spacing) -> Option<u16> {
    if r.node.children.is_empty() || r.node.kind == Kind::Note {
        return None;
    }
    Some(match r.section {
        Section::FleetBand | Section::SquadronBand | Section::StowBand => 2,
        _ => (sp.side() + 2 + r.prefix.width()) as u16,
    })
}

/// NAME cell: pane dot, tree guides, fold marker, kind icon, name. (The
/// issue glyph, coloured by severity, is DETAIL's first word.)
pub fn name_cell(r: &Row, width: usize, _icons: &Icons) -> Vec<Span<'static>> {
    let n = &r.node;
    let dot = match &n.pane_state {
        Some(state) => Span::styled(format!("{} ", pane_dot(state)), bold(pane_color(state))),
        None => Span::raw("  "),
    };
    let marker = if n.children.is_empty() {
        "  "
    } else if r.expanded {
        "\u{25be} "
    } else {
        "\u{25b8} "
    };
    let room = width.saturating_sub(2 + r.prefix.width() + 2 + n.icon.width() + 1).max(3);
    vec![
        dot,
        Span::styled(r.prefix.clone(), st(mono::DIM)),
        Span::styled(marker, st(mono::COMMENT)),
        Span::styled(format!("{} ", n.icon), n.icon_style),
        Span::styled(pad(&if n.cut_left { truncate_left(&n.name, room) } else { truncate(&n.name, room) }, room), n.name_style),
    ]
}

/// A row's lines: one for a member, as many as it takes for a note (an error
/// shown in full, wrapped with its indent).
fn body_spans(r: &Row, w: &[u16; 4], inner: usize, gap: usize, icons: &Icons, frame: Option<u64>) -> Vec<Vec<Span<'static>>> {
    if r.node.kind != crate::view::Kind::Note {
        return vec![row_spans_anim(r, w, inner, gap, icons, frame)];
    }
    let lead = r.node.name.len() - r.node.name.trim_start().len();
    // a note sits under its member's row, in step with a ship's indent
    let base = 4 + r.prefix.len() - r.prefix.trim_start_matches(' ').len();
    wrap(vec![Span::styled(format!("{}{}", " ".repeat(base), r.node.name), r.node.name_style)], inner, base + lead)
        .into_iter()
        .map(|l| {
            let used: usize = l.spans.iter().map(|s| s.content.width()).sum();
            let mut v = l.spans;
            v.push(Span::raw(" ".repeat(inner.saturating_sub(used))));
            v
        })
        .collect()
}

/// A row: NAME, STATUS, AGE, DETAIL, exactly `inner` columns wide.
pub fn row_spans(r: &Row, w: &[u16; 4], inner: usize, icons: &Icons) -> Vec<Span<'static>> {
    row_spans_gap(r, w, inner, 1, icons)
}

/// `row_spans` with `gap` spaces between the columns.
pub fn row_spans_gap(r: &Row, w: &[u16; 4], inner: usize, gap: usize, icons: &Icons) -> Vec<Span<'static>> {
    row_spans_anim(r, w, inner, gap, icons, None)
}

/// The STATUS cell at animation frame `frame` (`None`: still): the glyph's
/// replacement and colour (one cell), and the cell's style. Only the glyph,
/// or the needs-you chip's fill, moves: the word keeps its colour and the
/// layout never moves.
fn status_cell(r: &Row, frame: Option<u64>, icons: &Icons) -> (Option<(&'static str, Style)>, Style) {
    use crate::theme::anim::{mix, tri, BUSY_DIM, NEEDS_HI};
    use crate::view::Anim;
    let style = r.node.status.1;
    let Some(f) = frame else { return (None, style) };
    let at = |set: &[&'static str], k: u64| set[(k % set.len() as u64) as usize];
    match r.node.anim {
        Anim::Spin => (Some((at(icons.spin, f), style.fg(mono::BUSY))), style),
        Anim::SpinDim => (Some((at(icons.spin, f), style.fg(BUSY_DIM))), style),
        // a quarter of the speed, and quieter
        Anim::Dots => (Some((at(icons.wait, f / 4), style.fg(BUSY_DIM))), style),
        // the chip's fill breathes between pink and a lighter pink, once a second
        Anim::Pulse if style.bg == Some(mono::NEEDS) => (None, style.bg(mix(mono::NEEDS, NEEDS_HI, tri(f, 10)))),
        _ => (None, style),
    }
}

/// `row_spans_gap` at animation frame `frame`.
fn row_spans_anim(r: &Row, w: &[u16; 4], inner: usize, gap: usize, icons: &Icons, frame: Option<u64>) -> Vec<Span<'static>> {
    let sep = " ".repeat(gap);
    let mut out: Vec<Span<'static>> = truncate_spans(name_cell(r, w[0] as usize, icons), w[0] as usize);
    let used_name = spans_width(&out);
    if used_name < w[0] as usize {
        out.push(Span::raw(" ".repeat(w[0] as usize - used_name)));
    }
    let n = &r.node;
    out.push(Span::raw(sep.clone()));
    let (glyph, status_style) = status_cell(r, frame, icons);
    let cell = pad(&truncate(&n.status.0, w[1] as usize), w[1] as usize);
    match (glyph, cell.split_once(' ')) {
        // the glyph (the first word) swapped, one cell for one cell
        (Some((g, gs)), Some((first, rest))) if first.width() == 1 => {
            out.push(Span::styled(g, gs));
            out.push(Span::styled(format!(" {rest}"), status_style));
        }
        _ => out.push(Span::styled(cell, status_style)),
    }
    out.push(Span::raw(sep.clone()));
    out.push(Span::styled(format!("{:>width$}", truncate(&n.age, w[2] as usize), width = w[2] as usize), st(mono::COMMENT)));
    out.push(Span::raw(sep));
    let used = spans_width(&out);
    let room = inner.saturating_sub(used);
    let detail = if n.drop_rank.len() == n.detail.len() && !n.drop_rank.is_empty() { drop_spans(&n.detail, &n.drop_rank, room) } else { truncate_spans(n.detail.clone(), room) };
    let dw = spans_width(&detail);
    out.extend(detail);
    if dw < room {
        out.push(Span::raw(" ".repeat(room - dw)));
    }
    // below the columns' minimums (33 columns) NAME, STATUS and AGE alone are
    // wider than the row: cut it, never past the box's border
    if used > inner {
        return truncate_spans(out, inner);
    }
    out
}


/// A band `width` columns wide: under this, the fleet's tab shows only the
/// key counts (needs you, errors).
pub const NARROW_BAND: usize = 100;

/// The column the member rows' STATUS cell starts at, in a list `width`
/// columns wide with spacing `sp`: the box's side and padding, NAME, a gap.
/// A bar puts its state there, glyph over glyph.
pub fn status_x(width: usize, sp: Spacing) -> usize {
    let w = col_widths_gap(sp.inner(width) as u16, sp.gap as u16);
    sp.side() + w[0] as usize + sp.gap
}

/// What the bars on screen share: the size's width, so every bar's right
/// block (size and slots) has one width and its slots line up.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BarCols {
    pub size: usize,
}

pub fn bar_cols(rows: &[Row]) -> BarCols {
    BarCols { size: rows.iter().filter_map(|r| r.node.head.as_ref()).map(|h| h.size.width()).max().unwrap_or(0) }
}

/// The parts a bar shows besides its title (always there, cut last).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BarFit {
    pub state: bool,
    /// the size and every slot, as one block
    pub block: bool,
    pub kb: bool,
    /// how many of the KB's details (age, pages)
    pub kb_parts: usize,
}

/// The title's column (after the end cell, the fold marker and the glyph).
const TITLE_X: usize = 6;

/// The right block: the size, right-aligned in the screen's widest, then a
/// count per slot (`view::slot_glyphs`), zeros too, two columns each.
fn bar_block(h: &crate::view::Header, cols: BarCols, icons: &Icons) -> String {
    let slots: Vec<String> = crate::view::slot_glyphs(icons).iter().zip(h.slots).map(|(g, n)| format!("{g}{n:>2}")).collect();
    format!("{:>w$}  {}", h.size, slots.join(" "), w = cols.size)
}

/// The right side at a fit: the block, or nothing.
fn bar_right(h: &crate::view::Header, fit: BarFit, cols: BarCols, icons: &Icons) -> String {
    if fit.block {
        bar_block(h, cols, icons)
    } else {
        String::new()
    }
}

/// The KB part at a fit (the fleet's bar only), after the title.
fn bar_kb(h: &crate::view::Header, fit: BarFit) -> Option<String> {
    h.kb.as_ref().filter(|_| fit.kb).map(|k| k.text(fit.kb_parts))
}

/// The room the title has at a fit: up to the state's column, else up to the right side.
fn title_room(h: &crate::view::Header, width: usize, sx: usize, fit: BarFit, right_w: usize) -> usize {
    if fit.state && h.state.is_some() {
        sx.saturating_sub(TITLE_X + 1)
    } else {
        width.saturating_sub(2 + TITLE_X + if right_w > 0 { right_w + 2 } else { 0 })
    }
}

/// The title in `room` columns: in full, else without its fleet's part, else cut.
fn bar_title(h: &crate::view::Header, room: usize) -> String {
    if h.title.width() <= room {
        return h.title.clone();
    }
    match &h.short {
        Some(s) if s.width() <= room => s.clone(),
        _ => truncate(h.short.as_ref().unwrap_or(&h.title), room),
    }
}

/// Does the bar fit `width` columns with these parts (beside the state the
/// title may be cut, else it is whole or shortened)? Text runs from column
/// 1 to `width - 3`. The KB sits two columns after the title and ends a
/// column before the state; the
/// state starts at the rows' STATUS column; two columns at least before the
/// right block.
pub fn bar_fits(r: &Row, width: usize, sp: Spacing, cols: BarCols, icons: &Icons, fit: BarFit) -> bool {
    let Some(h) = r.node.head.as_ref() else { return true };
    let sx = status_x(width, sp);
    let right_w = bar_right(h, fit, cols, icons).width();
    let room = title_room(h, width, sx, fit, right_w);
    // beside the state the title may be cut (the state keeps its column); without it, whole or shortened
    let title_ok = if fit.state && h.state.is_some() { room >= 4 } else { h.title.width() <= room || h.short.as_ref().is_some_and(|s| s.width() <= room) };
    let lead_end = TITLE_X + bar_title(h, room).width() + bar_kb(h, fit).map_or(0, |k| 2 + k.width());
    let left_end = match (&h.state, fit.state) {
        (Some(s), true) => {
            if lead_end + 1 > sx {
                return false;
            }
            sx + s.width()
        }
        _ => lead_end,
    };
    let end = width.saturating_sub(2);
    title_ok && if right_w == 0 { left_end <= end } else { left_end + 2 + right_w <= end }
}

/// The parts a bar shows at `width`: each one that fits beside those before
/// it, in this order: the state, the block, the KB, the KB's details. A part
/// is left out only when adding it would overflow (or push the state off
/// the STATUS column).
pub fn bar_fit(r: &Row, width: usize, sp: Spacing, cols: BarCols, icons: &Icons) -> BarFit {
    let Some(h) = r.node.head.as_ref() else { return BarFit::default() };
    let mut fit = BarFit::default();
    let add = |cur: BarFit, next: BarFit| if bar_fits(r, width, sp, cols, icons, next) { next } else { cur };
    if h.state.is_some() {
        fit = add(fit, BarFit { state: true, ..fit });
    }
    fit = add(fit, BarFit { block: true, ..fit });
    if let Some(kb) = &h.kb {
        fit = add(fit, BarFit { kb: true, ..fit });
        for n in 1..=kb.parts.len() {
            if fit.kb && fit.kb_parts == n - 1 {
                fit = add(fit, BarFit { kb_parts: n, ..fit });
            }
        }
    }
    fit
}

/// A section's bar (the fleet's, a squadron's, the stowaways'), one grammar:
/// `▾ ♛ MAIN - admiral  KB ✓ 3m   ☾ idle ...   1 crew  ♟ 0 ⚠ 0 ⚡ 1 ☾ 1 ○ 0`.
/// The title, the fleet's KB after it, the state in the member rows'
/// STATUS column (`status_x`), and a right block of one width on every bar.
/// Filled with the worst state in its section (neutral: the muted step, so
/// it still reads as a bar), its text in `INK`. Parts go only when they do
/// not fit (`bar_fit`); it never wraps nor passes its end. On the cursor the
/// fill and the text stay, so the contrast does too: the text is underlined
/// and the end cells take the selection colour.
pub fn bar(r: &Row, width: usize, selected: bool, frame: Option<u64>, sp: Spacing, cols: BarCols, icons: &Icons) -> Line<'static> {
    let Some(h) = r.node.head.as_ref() else { return Line::raw(" ".repeat(width)) };
    let mut fill = h.fill;
    // its status just got worse: one pulse toward white over a second, then it holds
    if let (Some(f), crate::view::Anim::Flash(from)) = (frame, r.node.anim) {
        if f >= from && f - from < 10 {
            fill = crate::theme::anim::mix(fill, mono::STRONG, 0.45 * crate::theme::anim::tri(f - from, 10));
        }
    }
    let mut text = Style::new().fg(mono::INK).bg(fill).add_modifier(Modifier::BOLD);
    if selected {
        text = text.add_modifier(Modifier::UNDERLINED);
    }
    let fit = bar_fit(r, width, sp, cols, icons);
    let sx = status_x(width, sp);
    let right = bar_right(h, fit, cols, icons);
    let room = title_room(h, width, sx, fit, right.width());
    let marker = if r.expanded && !r.node.children.is_empty() { "\u{25be}" } else { "\u{25b8}" };
    // the text from column 1: what lands at column c is at width c - 1
    let mut s = format!(" {marker} {} {}", h.glyph, bar_title(h, room));
    if let Some(kb) = bar_kb(h, fit) {
        s.push_str("  ");
        s.push_str(&kb);
    }
    if let (Some(state), true) = (&h.state, fit.state) {
        s = pad(&s, sx - 1);
        s.push_str(state);
    }
    let end = width.saturating_sub(2);
    if !right.is_empty() {
        s = pad(&s, end.saturating_sub(right.width()) - 1);
        s.push_str(&right);
    }
    // never past the end: the space before the end cell, then the end cell
    let s = pad(&truncate(&s, end.saturating_sub(1)), end.saturating_sub(1));
    let cap = |sel: bool| Span::styled(" ", if sel { Style::new().bg(mono::ACCENT) } else { text });
    Line::from(vec![cap(selected), Span::styled(format!("{s} "), text), cap(selected)])
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
    layout(rows, width, None, cursor, icons, None).0
}

/// The list's lines and the spacing they use: the width's spacing, and the
/// blank lines between boxes only while everything fits in `height` lines
/// (spacing goes before any row does).
/// `frame`: the animation frame of the live loop (`None`: still).
pub fn layout(rows: &[Row], width: u16, height: Option<usize>, cursor: usize, icons: &Icons, frame: Option<u64>) -> (Vec<(Line<'static>, Option<usize>)>, Spacing) {
    let mut sp = spacing_for(width as usize);
    let mut lines = lines_with(rows, width as usize, cursor, icons, sp, frame);
    if sp.blank && height.is_some_and(|h| lines.len() > h) {
        sp.blank = false;
        lines = lines_with(rows, width as usize, cursor, icons, sp, frame);
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

/// Every section's box under its bar: rounded, neutral (the bar carries the state).
const ROUND: Edges = Edges { v: "\u{2502}", bottom: ("\u{2570}", "\u{2500}", "\u{256f}"), style: Style::new().fg(mono::LINE) };

fn lines_with(rows: &[Row], width: usize, cursor: usize, icons: &Icons, sp: Spacing, frame: Option<u64>) -> Vec<(Line<'static>, Option<usize>)> {
    let inner = sp.inner(width);
    let w = col_widths_gap(inner as u16, sp.gap as u16);
    let pad = " ".repeat(sp.pad);
    let cols = bar_cols(rows);
    let mut out: Vec<(Line<'static>, Option<usize>)> = vec![];
    // a row inside a box: border, padding, the row, padding, border
    let boxed_row = |out: &mut Vec<(Line<'static>, Option<usize>)>, r: &Row, i: usize, e: Edges| {
        for body in body_spans(r, &w, inner, sp.gap, icons, frame) {
            let mut spans = vec![Span::styled(e.v, e.style), Span::raw(pad.clone())];
            spans.extend(body);
            spans.push(Span::raw(pad.clone()));
            spans.push(Span::styled(e.v, e.style));
            out.push((selectable(Line::from(spans), i == cursor), Some(i)));
        }
    };
    // the admiral's box with nothing in it (the filters hide the admiral)
    let empty_box = |out: &mut Vec<(Line<'static>, Option<usize>)>, e: Edges| {
        let text = truncate("  the admiral is hidden by the filters", inner + 2 * sp.pad);
        let rest = (inner + 2 * sp.pad).saturating_sub(text.width());
        out.push((Line::from(vec![Span::styled(e.v, e.style), Span::styled(format!("{text}{}", " ".repeat(rest)), st(mono::FAINT)), Span::styled(e.v, e.style)]), None));
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
        out.push((bar(head, width, i == cursor, frame, sp, cols, icons), Some(i)));
        i += 1;
        if head.section == Section::StowBand {
            // the stowaways: their bar, then their box
            let mut any = false;
            while i < rows.len() && !matches!(rows[i].section, Section::FleetBand | Section::StowBand) {
                boxed_row(&mut out, &rows[i], i, ROUND);
                any = true;
                i += 1;
            }
            if any && head.expanded {
                close(&mut out, ROUND);
            }
            continue;
        }
        // the fleet's bar is its admiral's section's: the admiral and its crew
        // in the box under it, then each squadron's bar and box
        let mut open: Option<Edges> = if head.expanded { Some(ROUND) } else { None };
        // the box open is the admiral's
        let mut admiral_open = head.expanded;
        let mut members_seen = false;
        // rows in the admiral's box (a box with none says why)
        let mut in_admiral = 0usize;
        while i < rows.len() && !matches!(rows[i].section, Section::FleetBand | Section::StowBand) {
            let r = &rows[i];
            match r.section {
                Section::SquadronBand => {
                    if let Some(e) = open.take() {
                        if admiral_open && in_admiral == 0 {
                            empty_box(&mut out, e);
                        }
                        close(&mut out, e);
                    }
                    admiral_open = false;
                    blank(&mut out);
                    members_seen = true;
                    // the bar carries the status; the sides stay neutral
                    out.push((bar(r, width, i == cursor, frame, sp, cols, icons), Some(i)));
                    if r.expanded {
                        open = Some(ROUND);
                    }
                }
                Section::SquadronBody => match open {
                    Some(e) => boxed_row(&mut out, r, i, e),
                    None => boxed_row(&mut out, r, i, Edges { v: " ", bottom: (" ", " ", " "), style: Style::new() }),
                },
                _ if r.node.kind == crate::view::Kind::Note => {
                    // a fleet's note (its config does not load, squadrons the state
                    // filter hides), in full: before any member (the fleet's
                    // config error), inside the admiral's box, under its bar;
                    // after (squadrons the state filter hides), below the boxes,
                    // a gap from a box above (open, or a collapsed squadron's line)
                    if !members_seen {
                        if let Some(e) = open {
                            boxed_row(&mut out, r, i, e);
                            in_admiral += 1;
                            i += 1;
                            continue;
                        }
                    }
                    if let Some(e) = open.filter(|_| admiral_open && in_admiral == 0) {
                        empty_box(&mut out, e);
                    }
                    admiral_open = false;
                    let after_box = open.take().map(|e| close(&mut out, e)).is_some() || rows.get(i.wrapping_sub(1)).is_some_and(|p| p.section == Section::SquadronBand);
                    if after_box {
                        blank(&mut out);
                    }
                    for body in body_spans(r, &w, inner, sp.gap, icons, frame) {
                        let mut spans = vec![Span::raw(" ".repeat(sp.side()))];
                        spans.extend(body);
                        out.push((selectable(Line::from(spans), i == cursor), Some(i)));
                    }
                }
                _ => {
                    // the admiral and its crew, in the box under the fleet's bar
                    members_seen = true;
                    in_admiral += 1;
                    boxed_row(&mut out, r, i, open.unwrap_or(ROUND));
                }
            }
            i += 1;
        }
        if let Some(e) = open.take() {
            if admiral_open && in_admiral == 0 {
                empty_box(&mut out, e);
            }
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
            // what is left of the word, its width kept: a word as long as a
            // whole output is cut in one pass
            let mut rest: &str = &word;
            let mut rest_w = word.width();
            let trail = rest_w.saturating_sub(word.trim_end().width());
            loop {
                let ww = rest_w.saturating_sub(trail);
                if used + ww <= width || (used <= indent && ww > width - indent) {
                    // fits, or a single word longer than a line: cut it
                    let room = width - used;
                    if rest_w > room && ww > room {
                        let cut: String = {
                            // measured whole: an emoji sequence is wider than its chars
                            let mut c = String::new();
                            for ch in rest.chars() {
                                c.push(ch);
                                if c.width() > room {
                                    c.pop();
                                    break;
                                }
                            }
                            // never nothing: the word must get shorter
                            if c.is_empty() {
                                c.extend(rest.chars().next());
                            }
                            // a path or file:line breaks after a `/` or `:` when it can
                            match c.rfind(['/', ':']) {
                                Some(i) if i + 1 > c.len() / 2 => c[..=i].to_string(),
                                _ => c,
                            }
                        };
                        rest = &rest[cut.len()..];
                        // widths of parts need not add up: the end is measured
                        rest_w = if rest.len() < 4 * width { rest.width() } else { rest_w.saturating_sub(cut.width()) };
                        cur.push(Span::styled(cut, style));
                        lines.push(Line::from(std::mem::take(&mut cur)));
                        cur.push(Span::raw(" ".repeat(indent)));
                        used = indent;
                        continue;
                    }
                    used += rest_w;
                    cur.push(Span::styled(rest.to_string(), style));
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
