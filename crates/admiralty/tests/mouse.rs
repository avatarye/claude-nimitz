//! The mouse on fixtures: what a click, a double click and the wheel do on
//! the list and in popups, read against the frame `draw` recorded.

mod common;

use std::time::{Duration, Instant};

use admiralty::app::{App, Effect, Modal};
use admiralty::fixtures;
use common::*;
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

fn ev(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
    MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE }
}

fn click_at(app: &mut App, x: u16, y: u16, at: Instant) -> Option<Effect> {
    app.on_mouse_at(ev(MouseEventKind::Down(MouseButton::Left), x, y), at)
}

fn click(app: &mut App, x: u16, y: u16) -> Option<Effect> {
    click_at(app, x, y, Instant::now())
}

/// Two clicks 50 ms apart, well after any click before; the second's effect.
fn double(app: &mut App, x: u16, y: u16) -> Option<Effect> {
    let t = Instant::now() + Duration::from_secs(5);
    click_at(app, x, y, t);
    click_at(app, x, y, t + Duration::from_millis(50))
}

fn wheel(app: &mut App, down: bool, x: u16, y: u16) -> Option<Effect> {
    app.on_mouse(ev(if down { MouseEventKind::ScrollDown } else { MouseEventKind::ScrollUp }, x, y))
}

/// The cell where `needle` starts on the first line holding it.
fn cell(ls: &[String], needle: &str) -> (u16, u16) {
    let y = ls.iter().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("no {needle}:\n{}", ls.join("\n")));
    (col_of(ls, needle, needle) as u16, y as u16)
}

/// An effect, as a word to compare.
fn kind(e: &Option<Effect>) -> &'static str {
    match e {
        None => "none",
        Some(Effect::Quit) => "quit",
        Some(Effect::Refresh) => "refresh",
        Some(Effect::Run(_)) => "run",
        Some(Effect::Editor) => "editor",
        Some(Effect::Mouse(_)) => "mouse",
    }
}

fn id(app: &App) -> String {
    app.current().map(|r| r.node.id.clone()).unwrap_or_default()
}

#[test]
fn a_click_selects_the_row_under_it() {
    let mut t = demo();
    tab(&mut t.app, "MAIN");
    for (w, h) in [(80u16, 40u16), (120, 40), (200, 60)] {
        let ls = lines(&draw(&mut t.app, w, h));
        let (x, y) = cell(&ls, "BOOM");
        assert!(click(&mut t.app, x, y).is_none());
        assert_eq!(id(&t.app), "M:MAIN/ARGOS/BOOM", "at {w}");
        // anywhere on the row, the far end too
        goto(&mut t.app, "M:MAIN/ARGOS/BOLT");
        draw(&mut t.app, w, h);
        click(&mut t.app, w - 1, y);
        assert_eq!(id(&t.app), "M:MAIN/ARGOS/BOOM", "at {w}");
        // a blank line or a border selects nothing
        let before = id(&t.app);
        click(&mut t.app, 5, 1);
        assert_eq!(id(&t.app), before);
    }
}

/// The fold marker is where `frames::fold_x` says, on every row at every width,
/// and a click on it folds and unfolds that row.
#[test]
fn a_click_on_a_fold_marker_folds_and_unfolds() {
    for fleet in ["MAIN", "FORGE", "LAB"] {
        for (w, h) in [(40u16, 60u16), (80, 60), (120, 60), (200, 80)] {
            let mut t = demo();
            tab(&mut t.app, fleet);
            let buf = draw(&mut t.app, w, h);
            let view = (h - 3 - 1) as usize;
            let (all, sp) = admiralty::frames::layout(&t.app.rows, w, Some(view), t.app.cursor, t.app.icons, None);
            for (k, (_, row)) in all.iter().enumerate().take(view) {
                let Some(r) = row.and_then(|i| t.app.rows.get(i)) else { continue };
                if let Some(x) = admiralty::frames::fold_x(r, sp) {
                    let sym = buf[(x, (k + 2) as u16)].symbol().to_string();
                    assert!(sym == "\u{25be}" || sym == "\u{25b8}", "{fleet} at {w}: {} has {sym:?} at {x}:\n{}", r.node.id, lines(&buf).join("\n"));
                }
            }
        }
    }
    let mut t = demo();
    tab(&mut t.app, "MAIN");
    let ls = lines(&draw(&mut t.app, 120, 50));
    let (x, y) = cell(&ls, "ARGOS");
    let at = ls[y as usize].find('\u{25be}').map(|b| ls[y as usize][..b].chars().count() as u16).expect("ARGOS's bar has a marker");
    assert!(at < x);
    let n = t.app.rows.len();
    click(&mut t.app, at, y);
    assert_eq!(id(&t.app), "SQ:MAIN/ARGOS");
    assert!(!t.app.current().unwrap().expanded && t.app.rows.len() < n, "folded");
    draw(&mut t.app, 120, 50);
    // two quick clicks on a marker are two folds, never an enter
    click(&mut t.app, at, y);
    assert!(t.app.current().unwrap().expanded && t.app.rows.len() == n, "unfolded");
}

#[test]
fn a_double_click_is_enter() {
    for row in ["M:MAIN/ARGOS", "SQ:MAIN/ARGOS", "M:MAIN/ARGOS/BOLT"] {
        let mut keyed = fixture_app(fixtures::demo(), true);
        let mut clicked = fixture_app(fixtures::demo(), true);
        for t in [&mut keyed, &mut clicked] {
            tab(&mut t.app, "MAIN");
            goto(&mut t.app, row);
        }
        let by_key = press(&mut keyed.app, KeyCode::Enter);
        let ls = lines(&draw(&mut clicked.app, 120, 50));
        // the line that selects it, the cursor elsewhere first
        clicked.app.cursor = clicked.app.rows.len() - 1;
        let y = (1..ls.len()).find(|y| {
            click(&mut clicked.app, 60, *y as u16);
            id(&clicked.app) == row
        });
        let y = y.unwrap_or_else(|| panic!("no line selects {row}")) as u16;
        let by_mouse = double(&mut clicked.app, 60, y);
        assert_eq!(kind(&by_key), kind(&by_mouse), "{row}");
        assert_eq!(toast(&keyed.app), toast(&clicked.app), "{row}");
        assert_eq!(*keyed.mux.calls.lock().unwrap(), *clicked.mux.calls.lock().unwrap(), "{row}");
        assert_eq!(keyed.app.rows.len(), clicked.app.rows.len(), "{row}");
        assert_eq!(keyed.app.modal.is_some(), clicked.app.modal.is_some(), "{row}");
    }
    // the commodore's pane: focused, as enter does
    let mut t = fixture_app(fixtures::demo(), true);
    tab(&mut t.app, "MAIN");
    let ls = lines(&draw(&mut t.app, 120, 50));
    let (x, y) = cell(&ls, "COMMODORE");
    let at = Instant::now();
    click_at(&mut t.app, x, y, at);
    // too slow: two clicks, no enter
    click_at(&mut t.app, x, y, at + Duration::from_millis(900));
    assert!(t.mux.calls.lock().unwrap().is_empty());
    double(&mut t.app, x, y);
    assert_eq!(*t.mux.calls.lock().unwrap(), ["focus p2"]);
}

#[test]
fn the_wheel_moves_the_view_and_takes_the_cursor_along() {
    let mut t = demo();
    tab(&mut t.app, "MAIN");
    let top = lines(&draw(&mut t.app, 120, 16));
    assert_eq!(t.app.scroll, 0);
    wheel(&mut t.app, true, 30, 5);
    let after = lines(&draw(&mut t.app, 120, 16));
    assert_eq!(t.app.scroll, 3, "three lines a notch");
    assert_ne!(top[2], after[2]);
    assert!(t.app.shown_rows.as_ref().unwrap().contains(&t.app.cursor), "the cursor stays in view");
    for _ in 0..50 {
        wheel(&mut t.app, true, 30, 5);
        draw(&mut t.app, 120, 16);
    }
    let bottom = t.app.scroll;
    assert!(bottom > 3);
    wheel(&mut t.app, true, 30, 5);
    draw(&mut t.app, 120, 16);
    assert_eq!(t.app.scroll, bottom, "stops at the end");
    for _ in 0..50 {
        wheel(&mut t.app, false, 30, 5);
        draw(&mut t.app, 120, 16);
    }
    assert_eq!(t.app.scroll, 0);
    assert_eq!(lines(&draw(&mut t.app, 120, 16))[2], top[2]);
}

#[test]
fn the_wheel_scrolls_popups() {
    let mut t = demo();
    keys(&mut t.app, "?");
    draw(&mut t.app, 100, 20);
    wheel(&mut t.app, true, 0, 0);
    assert!(matches!(t.app.modal, Some(Modal::Help { scroll: 3 })));
    wheel(&mut t.app, false, 50, 10);
    assert!(matches!(t.app.modal, Some(Modal::Help { scroll: 0 })));
    // the issues list: the selection, one a notch
    press(&mut t.app, KeyCode::Esc);
    keys(&mut t.app, "i");
    draw(&mut t.app, 100, 30);
    wheel(&mut t.app, true, 50, 10);
    assert!(matches!(t.app.modal, Some(Modal::Issues { sel: 1 })));
    // the detail beside the actions menu
    press(&mut t.app, KeyCode::Esc);
    tab(&mut t.app, "MAIN");
    goto(&mut t.app, "M:MAIN/ARGOS/BOLT");
    keys(&mut t.app, "x");
    let ls = lines(&draw(&mut t.app, 160, 24));
    let (x, y) = cell(&ls, " detail ");
    wheel(&mut t.app, true, x, y + 2);
    assert_eq!(t.app.detail_scroll, 3, "{}", ls.join("\n"));
    assert!(matches!(t.app.modal, Some(Modal::Menu { sel: 0, .. })), "the menu's selection stays");
    let (x, y) = cell(&ls, "actions: ");
    wheel(&mut t.app, true, x, y + 2);
    assert!(matches!(t.app.modal, Some(Modal::Menu { sel: 1, .. })));
}

/// The confirm panel: no click anywhere, single or double, runs its plan or
/// closes it; the wheel only scrolls it; `y` still runs.
#[test]
fn no_click_runs_a_plan() {
    let mut t = fixture_app(fixtures::demo(), true);
    tab(&mut t.app, "MAIN");
    goto(&mut t.app, "M:MAIN/ARGOS/BOOM");
    keys(&mut t.app, "x");
    press(&mut t.app, KeyCode::Char('s'));
    assert!(matches!(t.app.modal, Some(Modal::Confirm(_))), "{}", toast(&t.app));
    let (w, h) = (100u16, 24u16);
    for y in 0..h {
        for x in 0..w {
            draw(&mut t.app, w, h);
            assert!(double(&mut t.app, x, y).is_none(), "a double click at {x},{y}");
            assert!(wheel(&mut t.app, y % 2 == 0, x, y).is_none());
            assert!(matches!(t.app.modal, Some(Modal::Confirm(_))), "still asking after {x},{y}");
        }
    }
    assert!(t.mux.calls.lock().unwrap().is_empty());
    assert!(matches!(press(&mut t.app, KeyCode::Char('y')), Some(Effect::Run(_))), "y runs");
}

#[test]
fn a_click_picks_a_menu_entry_and_a_double_click_chooses_it() {
    let mut keyed = demo();
    let mut clicked = demo();
    for t in [&mut keyed, &mut clicked] {
        tab(&mut t.app, "MAIN");
        goto(&mut t.app, "M:MAIN/ARGOS/BOLT");
        keys(&mut t.app, "x");
    }
    let ls = lines(&draw(&mut clicked.app, 100, 40));
    let (x, y) = cell(&ls, "s   ");
    click(&mut clicked.app, x + 4, y);
    let Some(Modal::Menu { sel, items, .. }) = &clicked.app.modal else { panic!() };
    assert_eq!(items[*sel].key, 's');
    let sel = *sel;
    for _ in 0..sel {
        press(&mut keyed.app, KeyCode::Down);
    }
    draw(&mut clicked.app, 100, 40);
    let by_mouse = double(&mut clicked.app, x + 4, y);
    let by_key = press(&mut keyed.app, KeyCode::Enter);
    assert_eq!(kind(&by_key), kind(&by_mouse));
    assert!(matches!(clicked.app.modal, Some(Modal::Confirm(_))), "stop asks first: {}", toast(&clicked.app));
    assert_eq!(toast(&keyed.app), toast(&clicked.app));
}

#[test]
fn a_click_outside_closes_only_what_esc_closes_safely() {
    let mut t = demo();
    tab(&mut t.app, "MAIN");
    let before = id(&t.app);
    for k in ["?", "i", "!", "x", "c", "f", " "] {
        keys(&mut t.app, k);
        assert!(t.app.modal.is_some(), "{k}");
        draw(&mut t.app, 200, 60);
        // inside the popup: it stays
        click(&mut t.app, 100, 30);
        assert!(t.app.modal.is_some(), "{k}: a click inside");
        draw(&mut t.app, 200, 60);
        assert!(click(&mut t.app, 0, 30).is_none());
        assert!(t.app.modal.is_none(), "{k}: a click outside closes it");
        assert_eq!(id(&t.app), before, "{k}: the click went to the popup, not the row under it");
    }
    // a form holds what was typed: it stays
    keys(&mut t.app, "c");
    press(&mut t.app, KeyCode::Char('f'));
    assert!(matches!(t.app.modal, Some(Modal::Form(_))));
    draw(&mut t.app, 200, 60);
    click(&mut t.app, 0, 30);
    assert!(matches!(t.app.modal, Some(Modal::Form(_))), "a form stays");
}

#[test]
fn a_click_on_a_tab_shows_it() {
    let mut t = demo();
    tab(&mut t.app, "MAIN");
    let ls = lines(&draw(&mut t.app, 200, 40));
    let (x, _) = cell(&ls, "FORGE");
    click(&mut t.app, x, 0);
    assert_eq!(t.app.tab.as_deref(), Some("FORGE"));
    let ls = lines(&draw(&mut t.app, 200, 40));
    let (x, _) = cell(&ls, "MAIN");
    click(&mut t.app, x, 0);
    assert_eq!(t.app.tab.as_deref(), Some("MAIN"));
}

#[test]
fn the_finder_and_the_fleet_filter_take_clicks() {
    let mut t = demo();
    keys(&mut t.app, " ");
    let ls = lines(&draw(&mut t.app, 160, 50));
    let Some(Modal::Finder(fd)) = &t.app.modal else { panic!() };
    assert!(fd.hits.len() > 2);
    let head = ls.iter().position(|l| l.contains("NAME") && l.contains("RANK")).unwrap() as u16;
    click(&mut t.app, 40, head + 2);
    let Some(Modal::Finder(fd)) = &t.app.modal else { panic!() };
    assert_eq!(fd.sel, 1);
    let row = fd.selected().unwrap().row_id.clone();
    draw(&mut t.app, 160, 50);
    double(&mut t.app, 40, head + 2);
    assert!(t.app.modal.is_none(), "enter lands");
    assert_eq!(id(&t.app), row);
    // the fleet filter: a click on a [x] turns it over, as space
    keys(&mut t.app, "f");
    let ls = lines(&draw(&mut t.app, 120, 40));
    let y = ls.iter().position(|l| l.contains("[x]")).unwrap() as u16 + 1;
    let x = col_of(&ls, "[x]", "[x]") as u16;
    let on = t.app.fleet_states;
    click(&mut t.app, x, y);
    assert_eq!(t.app.fleet_states[1], !on[1]);
    assert!(matches!(t.app.modal, Some(Modal::FleetFilter { sel: 1 })));
}

/// Herdr's overlay hides the inset: the frame and the clicks both leave it out.
#[test]
fn the_inset_is_neither_drawn_nor_clicked() {
    let mut t = demo();
    tab(&mut t.app, "MAIN");
    t.app.inset = (3, 4);
    keys(&mut t.app, "?");
    draw(&mut t.app, 100, 30);
    for (x, y) in [(99, 10), (97, 2), (10, 28), (99, 29)] {
        click(&mut t.app, x, y);
        assert!(t.app.modal.is_some(), "{x},{y} is under the overlay's border");
    }
    click(&mut t.app, 0, 10);
    assert!(t.app.modal.is_none(), "inside the view, outside the popup");
    let ls = lines(&draw(&mut t.app, 100, 30));
    let (x, y) = cell(&ls, "BOOM");
    click(&mut t.app, x, y);
    assert_eq!(id(&t.app), "M:MAIN/ARGOS/BOOM");
}

#[test]
fn no_draw_and_tiny_windows_never_panic() {
    let all = |app: &mut App, w: u16, h: u16| {
        for y in 0..h.saturating_add(2) {
            for x in 0..w.saturating_add(2) {
                double(app, x, y);
                wheel(app, true, x, y);
                wheel(app, false, x, y);
            }
        }
        double(app, u16::MAX, u16::MAX);
    };
    // nothing drawn yet: nothing to hit
    let mut t = demo();
    let before = id(&t.app);
    all(&mut t.app, 30, 10);
    assert_eq!(id(&t.app), before);
    assert!(t.app.modal.is_none());
    for (w, h) in [(1u16, 1u16), (2, 3), (5, 4), (12, 6), (33, 8), (50, 5)] {
        for k in ["", "?", "x", "i", "!", "f", " ", "c"] {
            let mut t = demo();
            tab(&mut t.app, "MAIN");
            goto(&mut t.app, "M:MAIN/ARGOS/BOLT");
            keys(&mut t.app, k);
            draw(&mut t.app, w, h);
            all(&mut t.app, w, h);
            draw(&mut t.app, w, h);
        }
    }
}

#[test]
fn m_turns_the_mouse_off_and_on() {
    let mut t = demo();
    tab(&mut t.app, "MAIN");
    let ls = lines(&draw(&mut t.app, 120, 40));
    let (x, y) = cell(&ls, "BOOM");
    assert!(matches!(press(&mut t.app, KeyCode::Char('M')), Some(Effect::Mouse(false))));
    assert!(toast(&t.app).contains("selects text"));
    let before = id(&t.app);
    click(&mut t.app, x, y);
    assert_eq!(id(&t.app), before, "off: a click is the terminal's");
    assert!(matches!(press(&mut t.app, KeyCode::Char('M')), Some(Effect::Mouse(true))));
    click(&mut t.app, x, y);
    assert_eq!(id(&t.app), "M:MAIN/ARGOS/BOOM");
    assert!(admiralty::app::HELP.iter().any(|(k, d)| *k == "M" && d.contains("ADMIRALTY_MOUSE")));
}

/// Review of 0.7.20: clicks queued behind other input (main was busy) never
/// make a double click.
#[test]
fn queued_clicks_make_no_double_click() {
    let mut t = demo();
    draw(&mut t.app, 120, 40);
    let rows = lines(&draw(&mut t.app, 120, 40));
    let y = rows.iter().position(|l| l.contains("COMMODORE")).unwrap() as u16;
    let down = ratatui::crossterm::event::MouseEvent {
        kind: ratatui::crossterm::event::MouseEventKind::Down(ratatui::crossterm::event::MouseButton::Left),
        column: 12,
        row: y,
        modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
    };
    t.app.mouse_burst = true;
    assert!(t.app.on_mouse(down).is_none());
    draw(&mut t.app, 120, 40);
    t.app.mouse_burst = false;
    assert!(t.app.on_mouse(down).is_none(), "the second of two queued clicks is a single click");
    assert!(t.app.modal.is_none(), "{:?}", t.app.modal.is_some());
}
