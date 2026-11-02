//! The mouse. Drawing records what it put where (`Hits`, one per frame), and an
//! event is read against that: no terminal here, and no behaviour of its own.
//! A wheel notch and a double click are keys (`App::on_key`), a click on an
//! entry sets what the arrow keys set; only the list's wheel, which keys cannot
//! do (the view moves, not the cursor), is new. A click never runs a plan:
//! nothing on the confirm panel is hit-testable, and only `y` or `enter` run.

use std::mem::{discriminant, Discriminant};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::app::{App, Effect, Modal};
use crate::frames::{fold_x, Spacing, TIGHT};
use crate::view::Row;

/// Two clicks on one thing closer than this are a double click.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Rows one wheel event scrolls text; a list moves its selection one entry.
const WHEEL_ROWS: usize = 3;

/// What was clicked, to tell a double click: a row by its id (the rows may
/// have moved under the pointer), a popup's entry by its index.
#[derive(Clone, Debug, PartialEq)]
pub enum Spot {
    Row(String),
    Entry(usize),
}

/// What the last frame put where. `App.panel_rows` is the same idea for scrolling.
#[derive(Debug)]
pub struct Hits {
    drawn: bool,
    /// the popup the frame showed: hits are never read against another
    modal: Option<Discriminant<Modal>>,
    /// the part the user sees: a click in the inset is no click
    area: Rect,
    /// each tab's columns on the tab row, and its name
    tabs: Vec<(u16, u16, String)>,
    list: Rect,
    /// the row each line of the whole list selects
    lines: Vec<Option<usize>>,
    /// the first line shown, and how many show
    scroll: usize,
    view: usize,
    spacing: Spacing,
    panels: Vec<Rect>,
    entries: Vec<(Rect, usize)>,
    /// the columns of a `[x]` in the fleet filter
    pub ticks: Option<(u16, u16)>,
    /// the detail panel that scrolls beside (or under) the actions menu
    pub detail: Option<Rect>,
}

impl Default for Hits {
    fn default() -> Hits {
        Hits {
            drawn: false,
            modal: None,
            area: Rect::default(),
            tabs: vec![],
            list: Rect::default(),
            lines: vec![],
            scroll: 0,
            view: 0,
            spacing: TIGHT,
            panels: vec![],
            entries: vec![],
            ticks: None,
            detail: None,
        }
    }
}

impl Hits {
    /// A new frame in `area`, showing `modal`.
    pub fn begin(area: Rect, modal: Option<&Modal>) -> Hits {
        Hits { drawn: true, modal: modal.map(discriminant), area, ..Default::default() }
    }

    pub fn tab(&mut self, from: u16, to: u16, name: String) {
        self.tabs.push((from, to, name));
    }

    pub fn list(&mut self, area: Rect, lines: Vec<Option<usize>>, scroll: usize, view: usize, spacing: Spacing) {
        (self.list, self.lines, self.scroll, self.view, self.spacing) = (area, lines, scroll, view, spacing);
    }

    pub fn panel(&mut self, r: Rect) {
        self.panels.push(r);
    }

    pub fn entry(&mut self, row: Rect, entry: usize) {
        self.entries.push((row, entry));
    }

    /// The entries of a popup `r`: `rows[i]` are entry i's rows of its text,
    /// which shows `view` rows from row `at`, under its top border.
    pub fn entries(&mut self, r: Rect, rows: &[(usize, usize)], at: usize, view: usize) {
        let total = rows.last().map_or(0, |e| e.1);
        let at = at.min(total.saturating_sub(view));
        for (i, (from, to)) in rows.iter().enumerate() {
            for row in (*from).max(at)..(*to).min(at + view) {
                self.entry(Rect { x: r.x + 1, y: r.y + 1 + (row - at) as u16, width: r.width.saturating_sub(2), height: 1 }, i);
            }
        }
    }

    /// Was this drawn, for what is on screen now?
    fn current(&self, modal: Option<&Modal>) -> bool {
        self.drawn && self.modal == modal.map(discriminant)
    }

    fn tab_at(&self, at: Position) -> Option<String> {
        let tab_row = at.y == self.area.y;
        self.tabs.iter().find(|(from, to, _)| tab_row && (*from..*to).contains(&at.x)).map(|t| t.2.clone())
    }

    /// The row under `at`, and whether `at` is on its fold marker.
    fn row_at(&self, at: Position, rows: &[Row]) -> Option<(usize, bool)> {
        if !self.list.contains(at) {
            return None;
        }
        // the list's first line stays blank
        let shown = ((at.y - self.list.y) as usize).checked_sub(1).filter(|s| *s < self.view)?;
        let row = (*self.lines.get(self.scroll + shown)?)?;
        let col = at.x - self.list.x;
        let marker = fold_x(rows.get(row)?, self.spacing).is_some_and(|x| (x..x + 2).contains(&col));
        Some((row, marker))
    }

    fn entry_at(&self, at: Position) -> Option<usize> {
        self.entries.iter().find(|(r, _)| r.contains(at)).map(|e| e.1)
    }

    fn in_panel(&self, at: Position) -> bool {
        self.panels.iter().any(|r| r.contains(at))
    }

    /// The first and last rows that start in the lines from `top`.
    fn rows_from(&self, top: usize) -> Option<(usize, usize)> {
        let starts = |i: &usize| *i == 0 || self.lines[*i - 1] != self.lines[*i];
        let mut rows = (top..(top + self.view).min(self.lines.len())).filter(starts).filter_map(|i| self.lines[i]);
        let first = rows.next()?;
        Some((first, rows.last().unwrap_or(first)))
    }
}

/// Is this a mouse event Admiralty acts on? (The rest doesn't wake a draw.)
pub fn matters(kind: MouseEventKind) -> bool {
    matches!(kind, MouseEventKind::Down(MouseButton::Left) | MouseEventKind::ScrollUp | MouseEventKind::ScrollDown)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

impl App {
    /// Handle one mouse event; returns work the run loop must do.
    pub fn on_mouse(&mut self, m: MouseEvent) -> Option<Effect> {
        self.on_mouse_at(m, Instant::now())
    }

    /// `on_mouse` at a given moment.
    pub fn on_mouse_at(&mut self, m: MouseEvent, now: Instant) -> Option<Effect> {
        let at = Position { x: m.column, y: m.row };
        let hits = self.hits.borrow();
        let ours = self.mouse && hits.current(self.modal.as_ref()) && hits.area.contains(at);
        drop(hits);
        if !ours {
            return None;
        }
        let effect = match m.kind {
            MouseEventKind::ScrollUp => self.wheel(-1, at),
            MouseEventKind::ScrollDown => self.wheel(1, at),
            MouseEventKind::Down(MouseButton::Left) => self.click(at, now),
            _ => None,
        };
        self.after_input();
        effect
    }

    /// One wheel event, `dir` -1 up or 1 down: whatever scrolls under it, a
    /// popup wherever the pointer is. Up and down keys, except on the list and
    /// in a form, where those move the cursor and the focus.
    fn wheel(&mut self, dir: i64, at: Position) -> Option<Effect> {
        self.last_click = None;
        let rows = WHEEL_ROWS as i64 * dir;
        let arrow = key(if dir < 0 { KeyCode::Up } else { KeyCode::Down });
        let (press, times) = match &self.modal {
            None => {
                self.scroll_list(rows);
                return None;
            }
            Some(Modal::Form(_)) => {
                self.panel_scroll(rows);
                return None;
            }
            Some(Modal::Menu { detail: true, .. }) if self.hits.borrow().detail.is_some_and(|r| r.contains(at)) => {
                (KeyEvent::new(KeyCode::Char(if dir < 0 { 'y' } else { 'e' }), KeyModifiers::CONTROL), WHEEL_ROWS)
            }
            Some(Modal::Help { .. } | Modal::Confirm(_) | Modal::Message { .. }) => (arrow, WHEEL_ROWS),
            Some(_) => (arrow, 1),
        };
        (0..times).find_map(|_| self.on_key(press))
    }

    /// Move the list's view `by` lines. The cursor stays on its row while that
    /// shows, else it takes the nearest row that does (`draw_list` follows it).
    fn scroll_list(&mut self, by: i64) {
        let hits = self.hits.borrow();
        let max = hits.lines.len().saturating_sub(hits.view);
        let top = (self.scroll as i64 + by).clamp(0, max as i64) as usize;
        let Some((first, last)) = hits.rows_from(top) else { return };
        let cursor = self.cursor.clamp(first, last);
        drop(hits);
        self.scroll = top;
        self.cursor = cursor;
    }

    fn click(&mut self, at: Position, now: Instant) -> Option<Effect> {
        if self.modal.is_some() {
            return self.click_popup(at, now);
        }
        // a click ends the typing of a `/` filter, as enter does
        if self.editing_filter {
            self.on_key(key(KeyCode::Enter));
        }
        let hits = self.hits.borrow();
        let (tab, row) = (hits.tab_at(at), hits.row_at(at, &self.rows));
        drop(hits);
        if let Some(tab) = tab {
            self.last_click = None;
            self.show_fleet(&tab);
            return None;
        }
        let Some((row, marker)) = row else {
            self.last_click = None;
            return None;
        };
        self.cursor = row;
        if marker {
            self.last_click = None;
            self.toggle();
            return None;
        }
        let spot = Spot::Row(self.rows[row].node.id.clone());
        self.double(spot, now).then(|| self.on_key(key(KeyCode::Enter))).flatten()
    }

    fn click_popup(&mut self, at: Position, now: Instant) -> Option<Effect> {
        let hits = self.hits.borrow();
        let (entry, inside) = (hits.entry_at(at), hits.in_panel(at));
        let tick = hits.ticks.is_some_and(|(from, to)| (from..to).contains(&at.x));
        drop(hits);
        let Some(i) = entry else {
            self.last_click = None;
            return (!inside && self.outside_closes()).then(|| self.on_key(key(KeyCode::Esc))).flatten();
        };
        self.select_entry(i);
        // the `[x]` of the fleet filter: as `space`
        if tick {
            return self.on_key(key(KeyCode::Char(' ')));
        }
        self.double(Spot::Entry(i), now).then(|| self.on_key(key(KeyCode::Enter))).flatten()
    }

    /// Would esc here lose nothing but the popup? A confirm can stand on a
    /// filled-in form, and a form and a typed message hold the user's text.
    fn outside_closes(&self) -> bool {
        match &self.modal {
            Some(Modal::Help { .. } | Modal::FleetFilter { .. } | Modal::Needs { .. } | Modal::Issues { .. } | Modal::Menu { .. } | Modal::Finder(_)) => true,
            Some(Modal::Message { text, .. }) => text.is_empty(),
            Some(Modal::Confirm(_) | Modal::Form(_)) | None => false,
        }
    }

    fn select_entry(&mut self, i: usize) {
        match &mut self.modal {
            Some(Modal::FleetFilter { sel } | Modal::Needs { sel, .. } | Modal::Issues { sel } | Modal::Menu { sel, .. }) => *sel = i,
            Some(Modal::Finder(fd)) => fd.step(i as i64 - fd.sel as i64),
            _ => {}
        }
    }

    /// Note a click on `spot`: is it the second of a double click?
    fn double(&mut self, spot: Spot, now: Instant) -> bool {
        if self.mouse_burst {
            self.last_click = None;
            return false;
        }
        let again = self.last_click.take().is_some_and(|(last, t)| last == spot && now.saturating_duration_since(t) < DOUBLE_CLICK);
        if !again {
            self.last_click = Some((spot, now));
        }
        again
    }
}
