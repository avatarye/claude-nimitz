//! TUI tests on fixtures: frames, alignment, several fleets, widths, splits,
//! keys, modals, forms, monitor-only mode, issue visibility, pane dots.

mod common;

use admiralty::actions::{self, Act, Target};
use admiralty::app::{Effect, Modal};
use admiralty::fixtures;
use admiralty::forms::Form;
use admiralty::theme::mono;
use admiralty::view::Kind;
use common::*;
use nimitz_core::{Id, Status};
use ratatui::crossterm::event::KeyCode;

// ------------------------------------------------------------------ frames

/// A fleet has no line of its own: its tab carries its state and counts,
/// its row is the top of the admiral's heavy box (bright title); each
/// squadron's title bar is filled with its status colour, the text dark.
#[test]
fn the_fleet_is_its_tab_the_admiral_a_heavy_box_squadrons_filled_bars() {
    let mut t = demo();
    for fleet in ["MAIN", "FORGE", "LAB"] {
        tab(&mut t.app, fleet);
        let ls = lines(&draw(&mut t.app, 200, 60));
        assert!(!ls.iter().any(|l| l.starts_with('\u{2554}') || l.starts_with('\u{2551}') || l.starts_with('\u{255a}')), "no double frame on {fleet}");
        assert!(!t.app.rows.is_empty() && ls.iter().skip(2).next().is_some_and(|l| l.contains("ADMIRAL ")), "{fleet}: the list starts with the admiral's box:\n{}", ls.join("\n"));
    }
    tab(&mut t.app, "MAIN");
    let buf = draw(&mut t.app, 200, 60);
    let ls = lines(&buf);
    assert!(ls[0].contains("MAIN") && ls[0].contains("needs you"), "the tab carries the state: {}", ls[0]);
    let y = ls.iter().position(|l| l.starts_with("\u{250f}\u{2501} \u{25be} [MAIN] ADMIRAL ")).expect("the admiral's heavy box, named for its fleet");
    assert_eq!(buf[(0, y as u16)].fg, mono::FG, "a heavier border");
    assert_eq!(buf[(5, y as u16)].fg, mono::STRONG, "a brighter title");
    assert!(buf[(5, y as u16)].modifier.contains(ratatui::style::Modifier::BOLD));
    let adm = ls.iter().position(|l| l.starts_with('\u{2503}') && l.contains("ADMIRAL   ")).unwrap();
    assert!(adm > y && ls[adm].starts_with('\u{2503}'), "{}", ls[adm]);
    assert!(ls[adm + 2].contains("@KB"), "its aide in its box, after its crew: {}", ls[adm + 2]);
    assert!(ls[adm + 3].starts_with('\u{2517}'), "the box closes after its crew and its aide: {}", ls[adm + 3]);
    // the fleet row is that top line: fleet actions keep a row to stand on
    goto(&mut t.app, "F:MAIN");
    assert_eq!(t.app.current().unwrap().node.kind, Kind::FleetBand);
    // squadron bars: filled in the status colour, dark text
    let kits = ls.iter().position(|l| l.contains("KITS") && l.contains("comms")).unwrap();
    let x = ls[kits].find("KITS").unwrap() as u16;
    assert_eq!((buf[(x, kits as u16)].bg, buf[(x, kits as u16)].fg), (mono::NEEDS, mono::INK), "KITS needs you");
    assert_eq!(buf[(0, (kits + 1) as u16)].fg, mono::LINE, "the sides stay neutral");
    let argos = ls.iter().position(|l| l.contains("ARGOS") && l.contains("comms")).unwrap();
    let b = t.app.bundle.clone().unwrap();
    let argos_sq = b.snap.fleets.iter().flat_map(|f| f.squadrons.iter()).find(|q| q.id.0 == "MAIN/ARGOS").unwrap();
    let members: Vec<&nimitz_core::Member> = std::iter::once(&argos_sq.commodore).chain(argos_sq.ships.iter()).collect();
    let want = match admiralty::view::squadron_status_color(&members) {
        c if c == mono::LINE => mono::MUTED,
        c => c,
    };
    assert_eq!(buf[(2, argos as u16)].bg, want);
    assert!(ls[argos - 1].trim().is_empty() && ls[kits - 1].trim().is_empty(), "blank lines between the boxes");
    let s = ls.iter().position(|l| l.contains("STOWAWAYS")).unwrap();
    assert_eq!(buf[(2, s as u16)].bg, mono::WARN, "the stowaways: yellow while one is live");
    // the cursor on a bar keeps its fill and dark text (the contrast holds)
    goto(&mut t.app, "SQ:MAIN/KITS");
    let buf = draw(&mut t.app, 200, 60);
    let ls = lines(&buf);
    let kits = ls.iter().position(|l| l.contains("KITS") && l.contains("comms")).unwrap();
    let x = ls[kits].find("KITS").unwrap() as u16;
    let c = &buf[(x, kits as u16)];
    assert_eq!((c.bg, c.fg), (mono::NEEDS, mono::INK), "selected: same fill, same text");
    assert!(c.modifier.contains(ratatui::style::Modifier::UNDERLINED), "selected: underlined");
    assert_eq!(buf[(0, kits as u16)].bg, mono::ACCENT, "selected: the bar's end in the cursor colour");
}

/// Dark text on every fill passes WCAG AA (4.5:1); the neutral fill is the
/// muted step, so it still reads as a bar.
#[test]
fn the_bar_text_passes_contrast_on_every_fill() {
    let rgb = |c: ratatui::style::Color| match c {
        ratatui::style::Color::Rgb(r, g, b) => [r, g, b],
        _ => panic!("{c:?}"),
    };
    let lum = |c| {
        let f = |v: u8| {
            let v = v as f64 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        let [r, g, b] = rgb(c);
        0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
    };
    for fill in [mono::NEEDS, mono::ERROR, mono::WARN, mono::BUSY, mono::MUTED] {
        let (a, b) = (lum(fill), lum(mono::INK));
        let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
        assert!(ratio >= 4.5, "{fill:?}: {ratio:.2}");
    }
    assert!(lum(mono::MUTED) > lum(mono::BG) * 3.0, "the neutral bar stands out from the background");
}

/// The order of a squadron's colour: needs you > error > warning > busy >
/// neutral; parked, stopped and not started are neutral.
#[test]
fn a_squadrons_colour_is_its_worst_member_in_the_theme_order() {
    use nimitz_core::{Issue, Severity};
    let b = fixtures::demo();
    let base = b.snap.fleets[0].squadrons[0].commodore.clone();
    let with = |status: Status, sev: Option<Severity>| {
        let mut m = base.clone();
        m.status = status;
        m.issues = sev.map(|s| vec![Issue { code: "x".into(), severity: s, message: "m".into(), fix: None, id: None }]).unwrap_or_default();
        m
    };
    let c = |ms: &[nimitz_core::Member]| admiralty::view::squadron_status_color(&ms.iter().collect::<Vec<_>>());
    assert_eq!(c(&[with(Status::Idle, None), with(Status::Parked, None), with(Status::Stopped, None), with(Status::NotStarted, None)]), mono::LINE);
    assert_eq!(c(&[with(Status::Idle, Some(Severity::Info)), with(Status::Busy, None)]), mono::BUSY);
    assert_eq!(c(&[with(Status::Busy, None), with(Status::Idle, Some(Severity::Warn))]), mono::WARN);
    assert_eq!(c(&[with(Status::Busy, Some(Severity::Warn)), with(Status::Stopped, Some(Severity::Error))]), mono::ERROR);
    assert_eq!(c(&[with(Status::Stopped, Some(Severity::Error)), with(Status::NeedsYou, None)]), mono::NEEDS);
}

/// Spacing goes first: below 91 columns no padding and single gaps; the
/// blank lines between boxes stay at any width while the list fits the
/// height, and go (all of them, the one before STOWAWAYS too) when it does not.
#[test]
fn spacing_collapses_before_content() {
    use admiralty::frames::{spacing_for, ROOMY, TIGHT};
    assert_eq!(spacing_for(200), ROOMY);
    assert_eq!(spacing_for(91), ROOMY);
    assert_eq!((spacing_for(90).pad, spacing_for(90).gap), (TIGHT.pad, TIGHT.gap));
    assert_eq!((spacing_for(80).pad, spacing_for(80).gap), (TIGHT.pad, TIGHT.gap));
    for w in [56, 55, 50, 30] {
        assert_eq!((spacing_for(w).pad, spacing_for(w).gap), (TIGHT.pad, TIGHT.gap), "{w}: no spacing where columns are cut");
    }
    let mut t = demo();
    let between = |ls: &[String], top: &str| -> bool {
        let y = ls.iter().position(|l| l.contains(top)).unwrap();
        ls[y - 1].trim().is_empty()
    };
    for w in [200u16, 90] {
        let ls = lines(&draw(&mut t.app, w, 80));
        for top in ["ARGOS", "KITS", "STOWAWAYS"] {
            assert!(between(&ls, top), "{w}: a blank line above {top}");
        }
    }
    let ls = lines(&draw(&mut t.app, 80, 24));
    for top in ["ARGOS", "KITS"] {
        assert!(!between(&ls, top), "80x24: no blank above {top}:\n{}", ls.join("\n"));
    }
    assert!(ls.iter().skip(2).take(20).all(|l| !l.trim().is_empty()), "80x24: no blank line at all:\n{}", ls.join("\n"));
}

/// The admiral's heavy box is square: its top corner in the last column, as
/// its sides and bottom.
#[test]
fn the_admiral_box_corners_line_up() {
    let mut t = demo();
    for w in [200u16, 80] {
        let ls = lines(&draw(&mut t.app, w, 60));
        let top = ls.iter().find(|l| l.starts_with("\u{250f}")).unwrap();
        let side = ls.iter().find(|l| l.starts_with('\u{2503}')).unwrap();
        let bottom = ls.iter().find(|l| l.starts_with('\u{2517}')).unwrap();
        let end = |l: &str, c: char| l.chars().position(|x| x == c).unwrap();
        assert_eq!(end(top, '\u{2513}'), w as usize - 1, "{w}: {top}");
        assert_eq!(side.chars().collect::<Vec<_>>().iter().rposition(|x| *x == '\u{2503}').unwrap(), w as usize - 1);
        assert_eq!(end(bottom, '\u{251b}'), w as usize - 1);
    }
}

/// A collapsed squadron or stowaway box is one open line (`╶ … ╴`); the
/// state filter's note sits outside every box.
#[test]
fn collapsed_boxes_and_the_state_filter_note_sit_outside() {
    let mut t = demo();
    goto(&mut t.app, "STOW");
    let id = t.app.current().unwrap().node.id.clone();
    press(&mut t.app, KeyCode::Char(' '));
    let ls = lines(&draw(&mut t.app, 200, 60));
    let s = ls.iter().position(|l| l.contains("STOWAWAYS")).unwrap();
    assert!(!ls.get(s + 1).is_some_and(|l| l.starts_with('\u{2502}')), "{id}: folded, a bar alone: {}", ls[s]);
    // hide busy squadrons: the note says so, below the last box, not in it
    t.app.fleet_states[admiralty::view::FleetState::Busy.index()] = false;
    t.app.rebuild();
    let ls = lines(&draw(&mut t.app, 200, 60));
    let n = ls.iter().position(|l| l.contains("hidden by the state filter")).unwrap_or_else(|| panic!("the note:\n{}", ls.join("\n")));
    {

        assert!(!ls[n].starts_with('\u{2502}') && !ls[n].starts_with('\u{2503}'), "{}", ls[n]);
        let above = ls[..n].iter().rev().find(|l| !l.trim().is_empty()).unwrap();
        assert!(above.starts_with('\u{2570}') || above.starts_with('\u{2517}'), "the box above closes before the note:\n{}", ls.join("\n"));
    }
}

/// The shown fleet's tab carries its state word (in its hue) and its
/// counts (`icon n`, slashes between); under 100 columns only needs-you and
/// errors stay. Squadron bars' counts end in one column.
#[test]
fn the_tab_carries_the_fleets_state_and_counts() {
    let mut t = demo();
    tab(&mut t.app, "FORGE");
    let buf = draw(&mut t.app, 200, 60);
    let ls = lines(&buf);
    let x = ls[0].find("broken").expect(&ls[0]) as u16;
    assert_eq!(buf[(x, 0)].fg, mono::ERROR, "the state word carries the hue (bold, 3.66:1 on dark cyan)");
    assert!(ls[0].contains("^9 / ^4 / *2 / -1 / x2 / o3 / ?1"), "{}", ls[0]);
    tab(&mut t.app, "MAIN");
    let ls = lines(&draw(&mut t.app, 200, 60));
    assert!(ls[0].contains("needs you") && ls[0].contains("!2 / *4 / -2 / x1"), "needs-you first: {}", ls[0]);
    let end = |l: &str| l.trim_end().chars().count();
    let argos = ls.iter().find(|l| l.contains("ARGOS") && l.contains("comms")).unwrap();
    let kits = ls.iter().find(|l| l.contains("KITS") && l.contains("comms")).unwrap();
    assert_eq!(end(argos), end(kits), "{argos}\n{kits}");
    let ls = lines(&draw(&mut t.app, 90, 40));
    assert!(ls[0].contains("!2") && !ls[0].contains("*4"), "narrow: {}", ls[0]);
    tab(&mut t.app, "FORGE");
    let ls = lines(&draw(&mut t.app, 90, 40));
    assert!(ls[0].contains("^9") && !ls[0].contains("^4"), "narrow: errors only: {}", ls[0]);
}

#[test]
fn columns_line_up_across_fleet_squadron_and_stowaway_frames() {
    let mut t = demo();
    let ls = lines(&draw(&mut t.app, 200, 60));
    let header = col_of(&ls, "STATUS", "STATUS");
    let admiral = col_of(&ls, "ADMIRAL   ", "- idle");
    let ship = col_of(&ls, "BOOM", "- idle");
    tab(&mut t.app, "FORGE");
    let forge = lines(&draw(&mut t.app, 200, 60));
    let stow = col_of(&forge, "scratch-run", "* live");
    let cast = forge.iter().find(|l| l.contains("CAST") && l.contains("- idle")).unwrap();
    let forge_ship = cast[..cast.find("- idle").unwrap()].chars().count();
    assert_eq!(header, admiral);
    assert_eq!(admiral, ship);
    assert_eq!(ship, stow);
    assert_eq!(ship, forge_ship);
    // no KIND column; the kind icon sits right before the name, and no
    // issue mark before it (ASCII: ship `S`, issue `^`)
    assert!(!ls.iter().any(|l| l.contains("KIND")), "{}", ls.join("\n"));
    let boom = ls.iter().find(|l| l.contains("BOOM") && l.contains("- idle")).unwrap();
    assert!(boom.contains("S [MAIN.ARGOS]-BOOM") || boom.contains("S BOOM"), "{boom}");
    assert!(!boom.contains("ship "), "{boom}");
    for l in &ls {
        if let Some(x) = l.find("S [") {
            assert!(!l[..x].contains('^'), "no issue mark before the name: {l}");
        }
    }
}

/// The shown fleet's tab is one strip: its name on cyan, then its state and
/// counts on dark cyan, every cell up to where the next tab starts, in text
/// that reads there; the other tabs keep the plain background.
#[test]
fn the_shown_tab_counts_are_a_dark_cyan_strip_up_to_the_next_tab() {
    use admiralty::theme::contrast;
    for nerd in [false, true] {
        let mut t = demo();
        if nerd {
            t.app.icons = &admiralty::theme::NERD;
            t.app.rebuild();
        }
        for (fleet, next) in [("MAIN", " 2 FORGE"), ("FORGE", " 3 LAB")] {
            tab(&mut t.app, fleet);
            for w in [120u16, 200] {
                let buf = draw(&mut t.app, w, 40);
                let cells: Vec<_> = (0..w).map(|x| buf[(x, 0)].clone()).collect();
                let line: String = cells.iter().map(|c| c.symbol().to_string()).collect();
                let name_end = cells.iter().rposition(|c| c.bg == mono::ACCENT).expect(&line) + 1;
                let next_at = cells.iter().map(|c| c.symbol().to_string()).collect::<Vec<_>>().windows(next.chars().count()).position(|win| win.concat() == next).expect(&line);
                assert!(next_at > name_end + 2, "{fleet} {w}: counts sit between: {line:?}");
                for (x, c) in cells.iter().enumerate().take(next_at).skip(name_end) {
                    assert_eq!(c.bg, mono::ACCENT_DIM, "{fleet} {w} nerd={nerd}: cell {x} {:?} in {line:?}", c.symbol());
                    let need = admiralty::theme::min_contrast(c.modifier.contains(ratatui::style::Modifier::BOLD));
                    assert!(contrast(c.fg, mono::ACCENT_DIM).unwrap() >= need, "{fleet} {w}: cell {x} {:?} fg {:?}", c.symbol(), c.fg);
                }
                assert!(cells[next_at..].iter().all(|c| c.bg != mono::ACCENT_DIM && c.bg != mono::ACCENT), "{fleet} {w}: the next tabs stay plain: {line:?}");
            }
        }
    }
}

/// A band's counts: a Nerd Font icon a terminal may draw two cells wide
/// has a blank cell after it, so it never covers its number, at any width
/// (the squadron bars, the fleet band, the tabs).
#[test]
fn band_count_icons_never_touch_their_numbers() {
    use admiralty::backend::risky;
    let mut t = demo();
    t.app.icons = &admiralty::theme::NERD;
    t.app.rebuild();
    let mut seen = 0;
    for fleet in ["MAIN", "FORGE"] {
        tab(&mut t.app, fleet);
        for w in [60u16, 90, 91, 120, 200] {
            let buf = draw(&mut t.app, w, 60);
            for y in 0..buf.area.height {
                let row: Vec<String> = (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect();
                for x in 0..row.len().saturating_sub(1) {
                    if risky(&row[x]) && row[x + 1].chars().all(|c| c.is_ascii_digit()) && !row[x + 1].is_empty() {
                        panic!("{fleet} {w} cols, line {y}: {:?} touches {:?}\n{}", row[x], row[x + 1], row.concat());
                    }
                    if risky(&row[x]) && row[x + 1] == " " && row.get(x + 2).is_some_and(|c| c.chars().all(|c| c.is_ascii_digit()) && !c.is_empty()) {
                        seen += 1;
                    }
                }
            }
        }
    }
    assert!(seen > 0, "some count was drawn with Nerd Font icons");
}

#[test]
fn column_widths_grow_to_their_maxima_and_keep_detail_on_narrow_screens() {
    use admiralty::frames::{col_widths, COL_MAX, COL_MIN};
    let narrow = col_widths(96);
    assert!(narrow[3] >= 20, "DETAIL keeps room at 100 columns: {narrow:?}");
    let wide = col_widths(400);
    assert_eq!(wide, COL_MAX);
    let tiny = col_widths(10);
    assert_eq!(tiny, COL_MIN);
}

#[test]
fn several_fleets_in_config_order_and_an_elsewhere_fleet_is_folded() {
    let mut t = demo();
    // a tab per fleet, in config order; each tab shows its own fleet only
    assert_eq!(t.app.tabs(), ["MAIN", "FORGE", "LAB"]);
    let bands = |t: &TestApp| -> Vec<String> { t.app.rows.iter().filter(|r| r.node.kind == Kind::FleetBand).map(|r| r.node.name.clone()).collect() };
    assert_eq!(bands(&t), ["[MAIN]"]);
    tab(&mut t.app, "LAB");
    assert_eq!(bands(&t), ["[LAB]"]);
    let lab = t.app.rows.iter().find(|r| r.node.name == "[LAB]").unwrap();
    assert!(!lab.expanded, "a fleet that does not run here starts folded");
    assert!(!t.app.rows.iter().any(|r| r.node.id == "M:LAB/OPTICS/LENS"));
    let ls = lines(&draw(&mut t.app, 200, 60));
    assert!(ls[0].contains("elsewhere"), "its tab says where: {}", ls[0]);
    assert!(ls.iter().any(|l| l.starts_with("\u{257a}\u{2501} \u{25b8} [LAB] ADMIRAL ")), "folded: one line, named");
}

#[test]
fn renders_at_narrow_and_wide_sizes() {
    for (w, h) in [(100, 30), (200, 50), (60, 20), (300, 80)] {
        let mut t = demo();
        keys(&mut t.app, "w");
        let s = screen(&mut t.app, w, h);
        assert!(s.contains("ADMIRALTY"), "{w}x{h}");
        assert!(s.contains("NAME"), "{w}x{h}");
        assert!(s.contains("detail"), "{w}x{h}");
    }
}

#[test]
fn the_detail_panel_floats_over_the_list_on_w_and_follows_the_cursor() {
    let mut t = demo();
    // closed at start: the list takes the whole width
    assert!(!t.app.detail_open);
    let ls = lines(&draw(&mut t.app, 200, 50));
    assert!(!ls.iter().any(|l| l.contains("w/esc close")), "no detail panel until `w`");
    let band = ls.iter().find(|l| l.contains("ADMIRAL ")).unwrap();
    assert!(band.trim_end().chars().count() > 190, "the list spans the window: {band}");
    // `w` floats it over the right part; the list keeps its rows
    keys(&mut t.app, "w");
    let ls = lines(&draw(&mut t.app, 200, 50));
    let y = ls.iter().position(|l| l.contains(" detail ") && l.contains("w/esc close")).expect("the panel's title");
    assert!(y <= 2, "it starts at the top of the body");
    assert!(ls[y].find(" detail ").unwrap() > 100, "on the right: {}", ls[y]);
    assert!(ls.iter().any(|l| l.contains("ADMIRAL ")), "the list is still there");
    // it follows the cursor row
    goto(&mut t.app, "M:MAIN/ARGOS/BOOM");
    assert!(screen(&mut t.app, 200, 50).contains("[MAIN.ARGOS]-BOOM"), "the detail panel names it in full");
    goto(&mut t.app, "M:MAIN/KITS/PIPELINE");
    let s = screen(&mut t.app, 200, 50);
    assert!(s.contains("NEEDS YOU") || s.contains("PIPELINE"), "{s}");
    // esc closes it; `w` toggles; nothing advertises the old split keys
    press(&mut t.app, KeyCode::Esc);
    assert!(!t.app.detail_open);
    keys(&mut t.app, "ww");
    assert!(!t.app.detail_open);
    let s = screen(&mut t.app, 200, 50);
    assert!(s.lines().last().unwrap().contains("w detail") && !s.contains("v/s"), "key bar: {}", s.lines().last().unwrap());
    assert!(admiralty::app::HELP.iter().any(|(k, d)| *k == "w" && d.contains("detail")));
    assert!(!admiralty::app::HELP.iter().any(|(k, _)| k.contains("v / s")));
    // `s` and `v` no longer do anything on the list
    let before = t.app.rows.len();
    keys(&mut t.app, "sv");
    assert!(t.app.modal.is_none() && !t.app.detail_open && t.app.rows.len() == before);
}

#[test]
fn a_saved_state_with_the_old_split_setting_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("ui.json");
    std::fs::write(
        &state,
        r#"{"expanded": {"F:MAIN": false}, "filter": "", "fleet_states": [true, true, true, true, true, true, true], "flagged_only": true, "show_stowaways": true, "split": "horizontal", "tab": "FORGE"}"#,
    )
    .unwrap();
    let ctx = admiralty::actions::Ctx { env: fixtures::demo_env(), mux: Box::new(TestMux::default()), live: false, exit_after_jump: false };
    let app = admiralty::app::App::new(ctx, Some(state), true);
    assert!(app.flagged_only, "the rest of the saved state came back");
    assert_eq!(app.tab.as_deref(), Some("FORGE"));
    assert!(!app.detail_open, "the detail panel starts closed whatever was saved");
}

// ------------------------------------------------------------------ keys

#[test]
fn vim_motion_and_folding() {
    let mut t = demo();
    let n = t.app.rows.len();
    keys(&mut t.app, "G");
    assert_eq!(t.app.cursor, n - 1);
    keys(&mut t.app, "gg");
    assert_eq!(t.app.cursor, 0);
    keys(&mut t.app, "jj");
    assert_eq!(t.app.cursor, 2);
    keys(&mut t.app, "k");
    assert_eq!(t.app.cursor, 1);
    ctrl(&mut t.app, 'd');
    assert!(t.app.cursor > 1);
    ctrl(&mut t.app, 'u');
    // h on a fleet band folds it; l unfolds
    goto(&mut t.app, "F:MAIN");
    let before = t.app.rows.len();
    keys(&mut t.app, "h");
    assert!(t.app.rows.len() < before);
    keys(&mut t.app, "l");
    assert_eq!(t.app.rows.len(), before);
    // h on a member goes to its parent band
    goto(&mut t.app, "M:MAIN/ARGOS/BOOM");
    keys(&mut t.app, "h");
    assert_eq!(t.app.current().unwrap().node.id, "SQ:MAIN/ARGOS");
    // space toggles
    press(&mut t.app, KeyCode::Char(' '));
    assert!(!t.app.rows.iter().any(|r| r.node.id == "M:MAIN/ARGOS/BOOM"));
    press(&mut t.app, KeyCode::Char(' '));
    assert!(t.app.rows.iter().any(|r| r.node.id == "M:MAIN/ARGOS/BOOM"));
}

#[test]
fn filter_next_match_and_pane_steps() {
    let mut t = demo();
    keys(&mut t.app, "/anvil");
    press(&mut t.app, KeyCode::Enter);
    assert!(t.app.rows.iter().any(|r| r.node.id == "M:FORGE/IRON/ANVIL"));
    assert!(!t.app.rows.iter().any(|r| r.node.id == "M:MAIN/ARGOS/BOLT"));
    keys(&mut t.app, "n");
    assert_eq!(t.app.current().unwrap().node.id, "M:FORGE/IRON/ANVIL");
    press(&mut t.app, KeyCode::Esc);
    assert!(t.app.rows.iter().any(|r| r.node.id == "M:MAIN/ARGOS/BOLT"));
    // J / K: rows with a pane
    goto(&mut t.app, "F:MAIN");
    keys(&mut t.app, "J");
    assert_eq!(t.app.current().unwrap().node.id, "M:MAIN");
    keys(&mut t.app, "J");
    assert_eq!(t.app.current().unwrap().node.id, "M:MAIN/ARGOS");
    keys(&mut t.app, "K");
    assert_eq!(t.app.current().unwrap().node.id, "M:MAIN");
}

#[test]
fn flagged_only_stowaways_and_fleet_state_filter() {
    let mut t = demo();
    keys(&mut t.app, "F");
    let ids = |t: &TestApp| -> Vec<String> { t.app.rows.iter().filter(|r| matches!(r.node.kind, Kind::Member(_))).map(|r| r.node.id.clone()).collect() };
    assert!(ids(&t).contains(&"M:MAIN/KITS/PIPELINE".to_string()), "needs you is flagged");
    assert!(!ids(&t).contains(&"M:MAIN/ARGOS/BOOM".to_string()), "a clean idle ship is not");
    tab(&mut t.app, "FORGE");
    assert!(ids(&t).contains(&"M:FORGE/IRON/ANVIL".to_string()), "an issue is flagged");
    tab(&mut t.app, "MAIN");
    keys(&mut t.app, "F");
    assert!(t.app.rows.iter().any(|r| r.node.id == "STOW"));
    keys(&mut t.app, "I");
    assert!(!t.app.rows.iter().any(|r| r.node.id == "STOW"));
    keys(&mut t.app, "I");
    // f: hide broken fleets
    keys(&mut t.app, "f");
    assert!(matches!(t.app.modal, Some(Modal::FleetFilter { .. })));
    keys(&mut t.app, "j "); // second state = broken
    press(&mut t.app, KeyCode::Esc);
    assert!(!t.app.rows.iter().any(|r| r.node.id == "F:FORGE"));
    let s = screen(&mut t.app, 200, 50);
    assert!(s.contains("filtered: no broken (f clears)"), "the status bar says what the filter hides:\n{}", s.lines().rev().nth(1).unwrap());
}

#[test]
fn enter_on_a_pane_row_focuses_it_and_exit_after_jump_quits() {
    let mut t = fixture_app(fixtures::demo(), true);
    goto(&mut t.app, "M:MAIN/ARGOS");
    let e = press(&mut t.app, KeyCode::Enter);
    assert!(e.is_none());
    assert_eq!(t.mux.calls.lock().unwrap().as_slice(), ["focus p2"]);
    t.app.ctx.exit_after_jump = true;
    let e = press(&mut t.app, KeyCode::Enter);
    assert!(matches!(e, Some(Effect::Quit)));
    // a member row with no pane: `nimitz start <id> -p` plans one (confirm first)
    goto(&mut t.app, "M:MAIN/ARGOS/BOOM");
    assert!(press(&mut t.app, KeyCode::Enter).is_none());
    let Some(Modal::Confirm(p)) = t.app.modal.take() else { panic!("{}", toast(&t.app)) };
    assert_eq!(p.command, "nimitz start MAIN/ARGOS/BOOM -p");
}

#[test]
fn y_yanks_and_q_quits() {
    let mut t = demo();
    goto(&mut t.app, "M:MAIN/ARGOS/BOLT");
    keys(&mut t.app, "y");
    assert!(toast(&t.app).contains("ac33ac33-0000"));
    assert!(matches!(press(&mut t.app, KeyCode::Char('q')), Some(Effect::Quit)));
}

// ------------------------------------------------------------------ modals

#[test]
fn help_has_a_legend_line_for_each_icon() {
    let mut t = demo();
    keys(&mut t.app, "?");
    let s = screen(&mut t.app, 200, 120);
    for what in ["errors (orange) and warnings (yellow)", "needs you: waiting", "busy", "parked (idle an hour", "stowaway: a session", "pane dot: idle", "Band counts read"] {
        assert!(s.contains(what), "legend has {what}");
    }
}

#[test]
fn the_footer_keeps_help_first_at_every_width() {
    for w in [40u16, 60, 80, 120, 200] {
        let mut t = demo();
        let ls = lines(&draw(&mut t.app, w, 24));
        let foot = &ls[23];
        assert!(foot.trim_start().starts_with("? help"), "{w}: {foot}");
        assert!(foot.contains("q/esc quit"), "{w}: {foot}");
    }
}

#[test]
fn help_lists_every_key() {
    let mut t = demo();
    keys(&mut t.app, "?");
    let s = screen(&mut t.app, 200, 50);
    for k in ["gg / G", "ctrl-d / ctrl-u", "J / K", "c / e / D", "f / F"] {
        assert!(s.contains(k), "help shows {k}");
    }
    for k in ["w  ", "floating detail panel"] {
        assert!(s.contains(k), "help shows {k}");
    }
    press(&mut t.app, KeyCode::Esc);
    assert!(t.app.modal.is_none());
}

#[test]
fn issues_view_lists_every_issue_code_with_its_fix() {
    let mut t = demo();
    keys(&mut t.app, "i");
    let s = screen(&mut t.app, 220, 70);
    for code in [
        "config.invalid",
        "config.layout",
        "folder.missing",
        "folder.untrusted",
        "name.drift",
        "name.taken",
        "labels.missing",
        "labels.mismatch",
        "dup.live",
        "copy.rows",
        "lineage.broken",
        "held",
        "ship.elsewhere",
        "cc.version",
        "stowaway",
    ] {
        assert!(s.contains(code), "issues view shows {code}");
    }
    assert!(s.contains("nimitz relaunch FORGE/IRON/ANVIL"));
    assert!(s.contains("ERROR"));
    // worst first; enter goes to the row
    let issues = t.app.issues();
    assert_eq!(issues[0].severity, nimitz_core::Severity::Error);
    press(&mut t.app, KeyCode::Enter);
    assert!(t.app.modal.is_none());
    assert_eq!(t.app.current().unwrap().node.member.as_ref(), issues[0].id.as_ref());
}

#[test]
fn broken_shapes_are_marked_on_their_rows_and_in_the_detail_pane() {
    let mut t = demo();
    tab(&mut t.app, "FORGE");
    let buf = draw(&mut t.app, 200, 60);
    let ls = lines(&buf);
    for (row, code) in [
        ("ANVIL-2", "dup.live"),
        ("ANVIL-2", "name.drift"),
        ("COMMODORE ", "labels.missing"),
        ("HAMMER", "copy.rows"),
        ("HAMMER", "lineage.broken"),
        ("ADMIRAL   ", "held"),
    ] {
        let l = ls.iter().find(|l| l.contains(row)).unwrap_or_else(|| panic!("{row}"));
        assert!(l.contains(code), "{row} shows {code}: {l}");
        let y = ls.iter().position(|x| x == l).unwrap() as u16;
        // the issue mark is DETAIL's first word, after the name; nothing before it
        let x = col_of(&ls, row, "^ ") as u16;
        assert!(x > col_of(&ls, row, row.trim()) as u16, "{row}: no mark before the name: {l}");
        let c = buf[(x, y)].fg;
        assert!(c == mono::ERROR || c == mono::WARN, "{row} mark is coloured by severity");
    }
    goto(&mut t.app, "M:FORGE/IRON/ANVIL");
    keys(&mut t.app, "w");
    let s = screen(&mut t.app, 200, 60);
    assert!(s.contains("WHY THIS STATUS"));
    assert!(s.contains("ISSUES (2)"));
    assert!(s.contains("fix  nimitz stop cc33dupe"));
    assert!(s.contains("expected [FORGE.IRON]-ANVIL"));
    // top bar counts errors
    assert!(s.contains("9 errors"), "top bar");
}

#[test]
fn pane_dots_at_row_start_in_herdr_colours() {
    let mut t = demo();
    for (fleet, row, dot, color) in [
        // the pane dot reuses the status hues: idle and done neutral, working busy
        ("MAIN", "ADMIRAL   ", "\u{25cb}", mono::FG),
        ("MAIN", "COMMODORE ", "\u{25cf}", mono::BUSY),
        ("FORGE", "ADMIRAL   ", "\u{25cf}", mono::MUTED),
        ("FORGE", "scratch-run", "\u{00b7}", mono::FAINT),
    ] {
        tab(&mut t.app, fleet);
        let buf = draw(&mut t.app, 200, 60);
        let ls = lines(&buf);
        let y = ls.iter().position(|l| l.contains(row)).unwrap();
        assert_eq!(buf[(2, y as u16)].symbol(), dot, "{row}");
        assert_eq!(buf[(2, y as u16)].fg, color, "{row}");
    }
    tab(&mut t.app, "MAIN");
    let buf = draw(&mut t.app, 200, 60);
    let ls = lines(&buf);
    // KITS commodore's pane is blocked: needs you, and the dot takes the needs-you hue
    let kits_bar = ls.iter().position(|l| l.contains("KITS") && l.contains("comms")).unwrap();
    let y = kits_bar + 1 + ls[kits_bar + 1..].iter().position(|l| l.contains("COMMODORE ")).unwrap();
    assert_eq!(buf[(2, y as u16)].fg, mono::NEEDS);
    assert!(ls[y].contains("needs you"));
    let boom = ls.iter().find(|l| l.contains("BOOM")).unwrap();
    assert!(boom.starts_with("\u{2502}   "), "no pane: no dot: {boom}");
}

#[test]
fn a_blocked_pane_turns_a_live_member_into_needs_you_with_the_reason() {
    let mut b = fixtures::demo();
    b.panes.iter_mut().find(|p| p.pane.pane_id == "p1").unwrap().state = "blocked".into();
    let t = fixture_app(b, false);
    let m = actions::find_member(&t.app.bundle.as_ref().unwrap().snap, &Id("MAIN".into())).unwrap();
    assert_eq!(m.status, Status::NeedsYou);
    assert!(m.reasons[0].contains("blocked"));
}

#[test]
fn needs_you_list_and_jump() {
    let mut t = demo();
    keys(&mut t.app, "!");
    let s = screen(&mut t.app, 200, 50);
    assert!(s.contains("question: ship the flaky test fix?"));
    assert!(s.contains("permission prompt"));
    keys(&mut t.app, "j");
    press(&mut t.app, KeyCode::Enter);
    assert_eq!(t.app.current().unwrap().node.id, "M:MAIN/KITS/PIPELINE");
}

#[test]
fn start_shows_a_confirm_dialog_with_the_command_and_core_plan() {
    let mut t = demo();
    goto(&mut t.app, "M:MAIN/KITS/DOCS");
    keys(&mut t.app, "a");
    let Some(Modal::Confirm(p)) = &t.app.modal else { panic!("confirm: {}", toast(&t.app)) };
    assert_eq!(p.command, "nimitz start MAIN/KITS/DOCS");
    assert_eq!(p.act, Act::Start);
    let s = screen(&mut t.app, 200, 50);
    assert!(s.contains("nimitz start MAIN/KITS/DOCS"));
    assert!(s.contains("STEPS") || s.contains("REFUSED"));
    // demo data: y runs nothing
    keys(&mut t.app, "y");
    assert!(toast(&t.app).contains("demo data, nothing ran") || t.app.modal.is_some());
}

#[test]
fn action_menu_greys_out_what_does_not_apply_with_the_reason() {
    let mut t = demo();
    goto(&mut t.app, "M:FORGE/IRON/HAMMER");
    keys(&mut t.app, "x");
    let Some(Modal::Menu { items, .. }) = &t.app.modal else { panic!() };
    let sweep = items.iter().find(|i| i.key == 'w').unwrap();
    assert!(sweep.enabled.is_ok(), "hammer has stale rows to sweep");
    let stop = items.iter().find(|i| i.key == 's').unwrap();
    assert_eq!(stop.enabled, Err("not running".to_string()));
    let s = screen(&mut t.app, 200, 50);
    assert!(s.contains("adopt is for stowaways"));
    keys(&mut t.app, "w");
    assert!(matches!(t.app.modal, Some(Modal::Confirm(_))));
}

#[test]
fn message_goes_direct_to_leads_and_through_the_commodore_for_ships() {
    let mut t = fixture_app(fixtures::demo(), true);
    goto(&mut t.app, "M:MAIN/ARGOS/BOLT");
    let p = t.app.message_plan().unwrap();
    assert!(p.relay);
    assert_eq!(p.via, "[MAIN.ARGOS]-COMMODORE");
    keys(&mut t.app, "m");
    keys(&mut t.app, "ship it");
    press(&mut t.app, KeyCode::Enter);
    let calls = t.mux.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], "type p2 Relay to [MAIN.ARGOS]-BOLT from the user: ship it", "the CLI's relay text");
    goto(&mut t.app, "M:MAIN");
    let p = t.app.message_plan().unwrap();
    assert!(!p.relay);
    assert_eq!(p.pane.pane_id, "p1");
    // a stopped ship's commodore is live, but the ship is not
    goto(&mut t.app, "M:MAIN/KITS/DOCS");
    assert!(t.app.message_plan().unwrap_err().contains("not running"));
    goto(&mut t.app, "SW:ee11ee11-0000-4000-8000-000000000000");
    assert!(t.app.message_plan().unwrap_err().contains("stowaway"));
}

#[test]
fn open_attaches_a_background_session_in_a_new_pane() {
    let mut t = fixture_app(fixtures::demo(), true);
    goto(&mut t.app, "M:MAIN/ARGOS/BOLT");
    // `A` with no pane: `nimitz start <id> -p` plans it (its tab, a split
    // beside its commodore, a new tab) and runs it; Admiralty opens nothing itself
    keys(&mut t.app, "A");
    let Some(Modal::Confirm(p)) = t.app.modal.take() else { panic!("{}", toast(&t.app)) };
    assert_eq!(p.command, "nimitz start MAIN/ARGOS/BOLT -p");
    assert!(p.panes.is_empty(), "the CLI makes the pane");
    assert!(t.mux.calls.lock().unwrap().is_empty(), "nothing opened by Admiralty");
    // a lead is a job too (0.4.0): `A` attaches to it in its own tab
    goto(&mut t.app, "M:MAIN/ARGOS");
    assert!(t.app.available(Act::Open).is_ok());
    // a process with no job (an old interactive lead) is not attached; it lives in its terminal
    goto(&mut t.app, "SW:ab99ab99-0000-4000-8000-000000000000");
    assert!(t.app.available(Act::Open).unwrap_err().contains("no job"));
}

// ------------------------------------------------------------------ monitor-only

#[test]
fn no_multiplexer_means_monitor_only_with_the_reason_shown() {
    let mut t = fixture_app(fixtures::demo_monitor_only(), false);
    let s = screen(&mut t.app, 200, 50);
    assert!(s.contains("MONITOR-ONLY"));
    // a ship needs no pane: its start opens the confirm dialog
    goto(&mut t.app, "M:MAIN/KITS/DOCS");
    keys(&mut t.app, "a");
    assert!(matches!(t.app.modal, Some(Modal::Confirm(_))), "{}", toast(&t.app));
    press(&mut t.app, KeyCode::Esc);
    // a commodore is a job too (0.4.0): no pane to open, so no multiplexer needed
    goto(&mut t.app, "M:FORGE/MOLD");
    keys(&mut t.app, "a");
    assert!(matches!(t.app.modal, Some(Modal::Confirm(_))), "{}", toast(&t.app));
    press(&mut t.app, KeyCode::Esc);
    // open, message and jump need one
    goto(&mut t.app, "M:MAIN/KITS/DOCS");
    keys(&mut t.app, "A");
    assert!(toast(&t.app).contains("monitor-only"));
    keys(&mut t.app, "m");
    assert!(toast(&t.app).contains("monitor-only"));
    goto(&mut t.app, "M:MAIN");
    press(&mut t.app, KeyCode::Enter);
    assert!(toast(&t.app).contains("monitor-only"));
    let s = screen(&mut t.app, 200, 50);
    assert!(s.contains("monitor-only: no multiplexer here"), "the detail pane says why");
    // stop, sweep, rm and config edits need no multiplexer
    assert!(actions::available(Default::default(), &t.app.bundle.as_ref().unwrap().snap, &Target::Member(Id("FORGE/IRON/HAMMER".into())), Act::Sweep).is_ok());
}

// ------------------------------------------------------------------ forms

#[test]
fn new_fleet_form_validates_inline_and_shows_the_command() {
    let mut t = demo();
    keys(&mut t.app, "c");
    keys(&mut t.app, "f");
    let Some(Modal::Form(_)) = &t.app.modal else { panic!() };
    keys(&mut t.app, "lab2");
    let Some(Modal::Form(f)) = &t.app.modal else { panic!() };
    assert!(f.fields[0].error.as_deref().unwrap().contains("UPPERCASE"));
    assert!(!f.ok());
    let s = screen(&mut t.app, 200, 50);
    assert!(s.contains("must be UPPERCASE"));
    assert!(s.contains("nimitz fleet new lab2 --machine west-mac"));
    ctrl(&mut t.app, 'u');
    keys(&mut t.app, "LAB2");
    let Some(Modal::Form(f)) = &t.app.modal else { panic!() };
    assert!(f.fields[0].error.is_none());
    assert!(f.ok(), "{:?}", f.problems);
    assert_eq!(f.command, "nimitz fleet new LAB2 --machine west-mac");
    assert!(f.plan.steps.iter().any(|s| s.cmd[0] == "write" && s.cmd[1].ends_with("fleet.toml")));
    ctrl(&mut t.app, 's');
    let Some(Modal::Confirm(p)) = &t.app.modal else { panic!() };
    assert!(p.config);
    press(&mut t.app, KeyCode::Esc);
}

#[test]
fn new_ship_form_needs_a_prompt_and_reports_what_core_refuses() {
    let env = fixtures::demo_env();
    let snap = fixtures::demo_snapshot();
    let mut f = Form::new_ship("MAIN", "ARGOS", "west-mac");
    f.set("ship", "NEWSHIP");
    f.validate(&env, &snap);
    assert!(f.fields.iter().find(|x| x.key == "prompt").unwrap().error.is_some());
    f.set("prompt", "You build the importer.");
    f.set("dir", "relative/path");
    f.validate(&env, &snap);
    assert!(f.fields.iter().find(|x| x.key == "dir").unwrap().error.is_some());
    f.set("dir", "/work/importer");
    f.set("mode", "auto");
    f.validate(&env, &snap);
    // the demo env has no fleet MAIN on disk: core refuses, and the form says so
    assert!(!f.problems.is_empty());
    assert_eq!(f.command, "nimitz ship new MAIN/ARGOS/NEWSHIP --dir /work/importer --machine west-mac --prompt 'You build the importer.' --mode auto");
}

#[test]
fn forms_open_where_the_cursor_is() {
    let mut t = demo();
    goto(&mut t.app, "M:MAIN/ARGOS/BOLT");
    let menu = t.app.create_menu();
    assert!(menu.iter().any(|i| i.label == "new squadron in MAIN" && i.enabled.is_ok()));
    assert!(menu.iter().any(|i| i.label == "new ship in MAIN/ARGOS" && i.enabled.is_ok()));
    assert!(menu.iter().any(|i| i.label == "edit ship MAIN/ARGOS/BOLT" && i.enabled.is_ok()));
    goto(&mut t.app, "STOW");
    let menu = t.app.create_menu();
    assert!(menu.iter().any(|i| i.key == 'p' && i.enabled.is_err()));
}

#[test]
fn adopt_opens_a_form_prefilled_from_the_issue_fix() {
    let mut t = demo();
    goto(&mut t.app, "SW:ee11ee11-0000-4000-8000-000000000000");
    keys(&mut t.app, "x");
    keys(&mut t.app, "d");
    let Some(Modal::Form(f)) = &t.app.modal else { panic!("{}", toast(&t.app)) };
    assert_eq!(f.get("id"), "FORGE/IRON/<ship>");
    assert!(f.command.starts_with("nimitz adopt ee11ee11"));
}

#[test]
fn ascii_icons_have_no_nerd_glyphs() {
    let mut t = demo();
    let s = screen(&mut t.app, 200, 60);
    assert!(!s.chars().any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c) || c >= '\u{f0000}'), "ASCII mode has no private-use glyphs");
}

#[test]
fn a_legacy_session_has_its_own_marker_and_adopt_is_prefilled() {
    let mut t = demo();
    let buf = draw(&mut t.app, 200, 70);
    let ls = lines(&buf);
    let y = ls.iter().position(|l| l.contains("[KITS]-LEAD")).unwrap();
    let x = col_of(&ls, "[KITS]-LEAD", "L legacy");
    assert!(x > col_of(&ls, "[KITS]-LEAD", "[KITS]-LEAD"), "the marker is in DETAIL, not before the name: {}", ls[y]);
    assert_eq!(buf[(x as u16, y as u16)].fg, mono::WARN);
    assert!(ls[y].contains("legacy"));
    goto(&mut t.app, "SW:ab99ab99-0000-4000-8000-000000000000");
    assert_eq!(t.app.legacy_adopt(), Some(("ab99ab99".into(), "MAIN/KITS".into())));
    keys(&mut t.app, "x");
    let Some(Modal::Menu { items, .. }) = &t.app.modal else { panic!() };
    let it = items.iter().find(|i| i.key == 'd').unwrap();
    assert_eq!(it.label, "adopt this legacy session as MAIN/KITS");
    assert!(it.enabled.is_ok());
    keys(&mut t.app, "d");
    let Some(Modal::Form(f)) = &t.app.modal else { panic!("{}", toast(&t.app)) };
    assert_eq!(f.get("id"), "MAIN/KITS");
    assert_eq!(f.command, "nimitz adopt ab99ab99 MAIN/KITS");
}

#[test]
fn layout_and_machine_entries_are_where_they_belong() {
    let mut t = demo();
    goto(&mut t.app, "SQ:MAIN/ARGOS");
    assert!(t.app.create_menu().iter().all(|i| i.key != 't'), "no tender entry");
    goto(&mut t.app, "F:MAIN");
    let menu = t.app.action_menu();
    for k in ['L', 'S', 'M', 'w'] {
        assert!(menu.iter().any(|i| i.key == k), "fleet band menu has {k}");
    }
    // the demo's mux has layout; demo data has no nimitz binary path guarantee, so only check monitor-only refuses
    let mut m = fixture_app(fixtures::demo_monitor_only(), false);
    goto(&mut m.app, "F:MAIN");
    assert!(m.app.available(Act::Layout).unwrap_err().contains("monitor-only"));
}

#[test]
fn sweep_from_a_band_is_scoped_to_that_band() {
    let mut t = demo();
    goto(&mut t.app, "SQ:FORGE/IRON");
    keys(&mut t.app, "x");
    keys(&mut t.app, "w");
    let Some(Modal::Confirm(p)) = &t.app.modal else { panic!("{}", toast(&t.app)) };
    assert_eq!(p.command, "nimitz sweep FORGE/IRON");
    press(&mut t.app, KeyCode::Esc);
    goto(&mut t.app, "F:FORGE");
    keys(&mut t.app, "x");
    keys(&mut t.app, "w");
    let Some(Modal::Confirm(p)) = &t.app.modal else { panic!() };
    assert_eq!(p.command, "nimitz sweep FORGE");
}

// ------------------------------------------------------------------ 80x24 (BUG-36)

#[test]
fn at_80x24_the_essentials_stay_on_screen() {
    let mut t = fixture_app(fixtures::demo_monitor_only(), false);
    goto(&mut t.app, "M:FORGE/IRON/ANVIL");
    let ls = lines(&draw(&mut t.app, 80, 24));
    let status = &ls[ls.len() - 2];
    assert!(status.contains("MONITOR-ONLY"), "the status bar, above the key bar, keeps the mux state: {status}");
    assert!(ls[23].contains("? help") && ls[23].contains("q/esc quit"), "footer keeps help and quit: {}", ls[23]);
    let row = ls.iter().find(|l| l.contains("ANVIL-2")).unwrap_or_else(|| panic!("{}", ls.join("\n")));
    assert!(row.contains("name.drift"), "DETAIL keeps the issue code: {row}");
    // the floating panel, opened on the same small screen
    keys(&mut t.app, "w");
    let ls = lines(&draw(&mut t.app, 80, 24));
    let s = ls.join("\n");
    assert!(s.contains("WHY THIS STATUS"), "a short detail pane still shows the reasons:\n{s}");
    // and ctrl-e scrolls it to the fix
    for _ in 0..12 {
        ctrl(&mut t.app, 'e');
    }
    let s = screen(&mut t.app, 80, 24);
    assert!(s.contains("fix  nimitz"), "the fix is reachable by scrolling:\n{s}");
}

#[test]
fn at_80x24_the_issues_view_wraps_and_help_scrolls() {
    let mut t = demo();
    keys(&mut t.app, "i");
    let s = screen(&mut t.app, 80, 24);
    assert!(s.contains("expected [FORGE.IRON]-ANVIL") || s.contains("[FORGE.IRON]-ANVIL-2,"), "messages wrap, not cut:\n{s}");
    press(&mut t.app, KeyCode::Esc);
    keys(&mut t.app, "?");
    let first = screen(&mut t.app, 80, 24);
    assert!(first.contains("j for more"), "help says it scrolls:\n{first}");
    for _ in 0..80 {
        keys(&mut t.app, "j");
    }
    let last = screen(&mut t.app, 80, 24);
    assert!(last.contains("100 columns"), "help scrolls to its end:\n{last}");
}

#[test]
fn wrapped_text_keeps_its_indent() {
    use admiralty::frames::wrap;
    use ratatui::text::Span;
    let ls = wrap(vec![Span::raw("  warn legacy      "), Span::raw("a long message that has to wrap onto a second line here")], 40, 19);
    assert!(ls.len() >= 2);
    for l in &ls[1..] {
        let t: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(t.starts_with(&" ".repeat(19)) && !t[19..].starts_with(' '), "{t:?}");
    }
}

#[test]
fn a_ship_cannot_take_the_commodores_address() {
    let mut f = Form::new_ship("MAIN", "ARGOS", "west-mac");
    f.set("ship", "COMMODORE");
    f.set("prompt", "x");
    f.validate(&fixtures::demo_env(), &fixtures::demo_snapshot());
    assert!(f.fields.iter().find(|x| x.key == "ship").unwrap().error.as_deref().unwrap_or("").contains("address"));
}

#[test]
fn bug43_messages_are_sanitised_in_preview_command_and_send_and_empty_ones_refused() {
    let mut t = fixture_app(fixtures::demo(), true);
    goto(&mut t.app, "M:MAIN");
    let plan = t.app.message_plan().unwrap();
    let raw = "\u{1b}[31mhello\u{1b}[0m\nthere\u{7}\u{1b}]0;title\u{7}";
    t.app.modal = Some(Modal::Message { plan: plan.clone(), text: raw.into() });
    let s = screen(&mut t.app, 200, 50);
    assert!(s.contains("> hello there"), "preview shows what will be sent:\n{s}");
    assert!(s.contains("nimitz signal MAIN 'hello there'"), "and so does the command");
    press(&mut t.app, KeyCode::Enter);
    assert_eq!(t.mux.calls.lock().unwrap().as_slice(), ["type p1 hello there"]);
    // nothing left after cleaning: refused, the box stays open
    t.app.modal = Some(Modal::Message { plan, text: "\u{1b}[2J\n\t \u{7}".into() });
    press(&mut t.app, KeyCode::Enter);
    assert!(matches!(t.app.modal, Some(Modal::Message { .. })));
    assert!(toast(&t.app).contains("empty"));
    assert_eq!(t.mux.calls.lock().unwrap().len(), 1, "nothing more was typed");
}

#[test]
fn the_header_shortens_counts_instead_of_cutting_words() {
    for w in [200u16, 120, 100, 80, 60, 50] {
        let mut t = demo();
        let top = status_bar(&mut t.app, w);
        assert!(!top.contains("warni ") && !top.trim_end().ends_with("warni"), "{w}: {top}");
        let words: Vec<&str> = top.split_whitespace().collect();
        let last = words.last().copied().unwrap_or("");
        assert!(
            ["warnings", "warn", "(i)", "range", "unsupported", "untested", "CC!"].iter().any(|x| last.ends_with(x)) || last.ends_with(")") || last.ends_with('w') || last.ends_with('e') || last.ends_with('!'),
            "{w}: the header ends on a whole piece, not a cut word: {top:?}"
        );
    }
    let mut t = demo();
    let top80 = status_bar(&mut t.app, 80);
    assert!(top80.contains("7 warn") || top80.contains("7w"), "80 columns keep the warning count: {top80}");
}

#[test]
fn a_filter_is_always_marked_in_the_header_even_when_saved_and_narrow() {
    // a filter saved by the last run comes back on launch
    let mut t = demo();
    keys(&mut t.app, "/anvil");
    press(&mut t.app, KeyCode::Enter);
    keys(&mut t.app, "F");
    let state = t.dir.path().join("ui.json");
    let ctx = admiralty::actions::Ctx { env: fixtures::demo_env(), mux: Box::new(TestMux::default()), live: false, exit_after_jump: false };
    let mut again = admiralty::app::App::new(ctx, Some(state), true);
    again.fixed_now = Some(fixtures::NOW);
    again.apply(fixtures::demo());
    for w in [200u16, 120, 80, 60, 50] {
        let top = status_bar(&mut again, w);
        assert!(top.contains("filtered:") || top.contains("FILTERED"), "{w}: no filter marker: {top}");
        assert!(top.contains("esc") && top.contains('F'), "{w}: the marker says how to clear it: {top}");
    }
    let top = status_bar(&mut again, 200);
    assert!(top.contains("filtered: \"anvil\", flagged (esc/F clears)"), "{top}");
}

#[test]
fn bug45_a_missing_or_absurd_age_shows_a_question_mark() {
    let mut b = fixtures::demo();
    for f in &mut b.snap.fleets {
        if let Some(s) = f.admiral.session.as_mut() {
            s.started_at = None;
            s.status_since = None;
        }
        for sq in &mut f.squadrons {
            if let Some(s) = sq.commodore.session.as_mut() {
                s.status_since = Some(fixtures::NOW + 1.0e9); // in the future: not believable
                s.started_at = None;
            }
        }
    }
    let mut t = fixture_app(b, false);
    let ls = lines(&draw(&mut t.app, 200, 60));
    for name in ["ADMIRAL   ", "COMMODORE "] {
        let l = ls.iter().find(|l| l.contains(name)).unwrap();
        let age = &l[l.find(name).unwrap()..];
        assert!(age.contains(" ? "), "{name}: AGE shows ?: {l}");
    }
    // no session at all: the cell stays empty
    tab(&mut t.app, "FORGE");
    let ls = lines(&draw(&mut t.app, 200, 60));
    let l = ls.iter().find(|l| l.contains("QUENCH")).unwrap();
    assert!(!l.contains(" ? "), "{l}");
}

// ------------------------------------------------------------------ fleet tabs

#[test]
fn a_tab_per_fleet_and_tab_keys_switch_them() {
    let mut t = demo();
    assert_eq!(t.app.tabs(), ["MAIN", "FORGE", "LAB"], "every stowaway of the demo sits in a fleet's folders");
    assert_eq!(t.app.tab.as_deref(), Some("MAIN"), "the first tab shows first");
    let ls = lines(&draw(&mut t.app, 200, 60));
    assert!(ls[0].contains("1 MAIN") && ls[0].contains("2 FORGE") && ls[0].contains("3 LAB"), "tab bar on top: {}", ls[0]);
    assert!(ls[0].contains("!2"), "MAIN's tab says two members need you: {}", ls[0]);
    assert!(ls[ls.len() - 2].contains("ADMIRALTY"), "the status bar sits right above the key bar: {}", ls[ls.len() - 2]);
    press(&mut t.app, KeyCode::Tab);
    assert_eq!(t.app.tab.as_deref(), Some("FORGE"));
    assert_eq!(t.app.current().unwrap().node.id, "F:FORGE", "the cursor starts at the tab's top");
    press(&mut t.app, KeyCode::Tab);
    press(&mut t.app, KeyCode::Tab);
    assert_eq!(t.app.tab.as_deref(), Some("MAIN"), "wraps");
    press(&mut t.app, KeyCode::BackTab);
    assert_eq!(t.app.tab.as_deref(), Some("LAB"));
    keys(&mut t.app, "2");
    assert_eq!(t.app.tab.as_deref(), Some("FORGE"));
    keys(&mut t.app, "9");
    assert_eq!(t.app.tab.as_deref(), Some("FORGE"), "no tab 9: nothing happens");
    // a jump to a row on another tab (needs-you list, issue list) shows that tab
    assert!(t.app.goto_id("M:MAIN/KITS/PIPELINE"));
    assert_eq!(t.app.tab.as_deref(), Some("MAIN"));
}

#[test]
fn stowaways_show_on_their_fleets_tab_and_the_rest_on_unassigned() {
    let mut b = fixtures::demo();
    let mut loose = b.snap.stowaways[0].clone();
    loose.session_id = "dd44dd44-0000-4000-8000-000000000000".into();
    loose.name = "loose-run".into();
    loose.cwd = "/home/u/.claude-nimitz/scratch".into();
    b.snap.stowaways.push(loose);
    let mut t = fixture_app(b, false);
    assert_eq!(t.app.tabs(), ["MAIN", "FORGE", "LAB", admiralty::view::UNASSIGNED]);
    let stow = |t: &TestApp| -> Vec<String> { t.app.rows.iter().filter(|r| r.node.kind == Kind::Stowaway).map(|r| r.node.name.clone()).collect() };
    assert_eq!(stow(&t), ["one-off probe", "[KITS]-LEAD"], "MAIN: the ones in MAIN's folders");
    tab(&mut t.app, "FORGE");
    assert_eq!(stow(&t), ["scratch-run"]);
    tab(&mut t.app, admiralty::view::UNASSIGNED);
    assert_eq!(stow(&t), ["loose-run"], "under the nimitz home, in no fleet's folders");
    assert!(!t.app.rows.iter().any(|r| r.node.kind == Kind::FleetBand), "no fleet on the unassigned tab");
    let s = screen(&mut t.app, 200, 60);
    assert!(s.lines().next().unwrap().contains("4 unassigned"), "{}", s.lines().next().unwrap());
    assert!(s.contains("in no fleet's folders"), "the band says what the list is");
}

#[test]
fn a_name_search_spans_every_fleet() {
    let mut t = demo();
    keys(&mut t.app, "/anvil");
    press(&mut t.app, KeyCode::Enter);
    assert_eq!(t.app.tab.as_deref(), Some("MAIN"));
    assert!(t.app.rows.iter().any(|r| r.node.id == "M:FORGE/IRON/ANVIL"), "found on another fleet's tab");
    let s = screen(&mut t.app, 200, 60);
    assert!(s.lines().next().unwrap().contains("search: every fleet"), "{}", s.lines().next().unwrap());
    press(&mut t.app, KeyCode::Esc);
    assert!(!t.app.rows.iter().any(|r| r.node.id == "M:FORGE/IRON/ANVIL"), "cleared: the tab again");
}

#[test]
fn the_state_filter_hides_squadrons_on_a_fleet_tab() {
    // MAIN's tab holds squadrons in different states; `f` must hide the
    // squadrons whose state is unticked, not the whole fleet (or nothing)
    let mut t = demo();
    let snap = t.app.bundle.as_ref().unwrap().snap.clone();
    let main = snap.fleets.iter().find(|f| f.id.0 == "MAIN").unwrap();
    let states: Vec<(String, admiralty::view::FleetState)> =
        main.squadrons.iter().map(|sq| (format!("SQ:{}", sq.id.0), admiralty::view::FleetState::of_squadron(main, sq))).collect();
    let shown = |t: &TestApp| -> Vec<String> { t.app.rows.iter().filter(|r| r.node.kind == Kind::SquadronBand).map(|r| r.node.id.clone()).collect() };
    assert_eq!(shown(&t).len(), states.len(), "every squadron at first");
    // untick the state of the first squadron
    let (gone, state) = states[0].clone();
    let at = admiralty::view::FleetState::ALL.iter().position(|s| *s == state).unwrap();
    keys(&mut t.app, "f");
    for _ in 0..at {
        keys(&mut t.app, "j");
    }
    keys(&mut t.app, " ");
    press(&mut t.app, KeyCode::Esc);
    let now = shown(&t);
    assert!(!now.contains(&gone), "{gone} ({state:?}) is hidden: {now:?}");
    for (id, s) in &states {
        assert_eq!(now.contains(id), *s != state, "{id} ({s:?})");
    }
    assert!(t.app.rows.iter().any(|r| r.node.id == "F:MAIN"), "the fleet itself stays on its tab");
    assert!(t.app.rows.iter().any(|r| r.node.name.contains("hidden by the state filter")), "a note says what is hidden");
}

// ------------------------------------------------------------------ crew window

fn crew_of(id: &str, parent: Option<&str>, label: &str, running: bool, age_s: f64) -> nimitz_core::Crew {
    nimitz_core::Crew { agent_id: id.into(), parent_id: parent.map(String::from), label: label.into(), agent_type: Some("general-purpose".into()), teammate: false, running, last_write: fixtures::NOW - age_s, workflow: None, workflow_name: None, phase: None }
}

/// The demo's BOLT, with crew of every age: running (sa-1, its child sa-2),
/// finished 10 min ago, finished 40 min ago (sa-3), finished 3 h ago, and a
/// 5 h old crew whose nested child still runs.
fn crew_world() -> (admiralty::data::Bundle, String) {
    let mut b = fixtures::demo();
    let mut id = String::new();
    for f in &mut b.snap.fleets {
        for sq in &mut f.squadrons {
            for m in &mut sq.ships {
                if m.crew.iter().any(|c| c.agent_id == "sa-1") {
                    m.crew.push(crew_of("recent", None, "recent review", false, 600.0));
                    m.crew.push(crew_of("old-a", None, "old audit", false, 3.0 * 3600.0));
                    m.crew.push(crew_of("old-b", None, "old parent", false, 5.0 * 3600.0));
                    m.crew.push(crew_of("old-b-kid", Some("old-b"), "kid still running", true, 10.0));
                    id = m.id.0.clone();
                }
            }
        }
    }
    assert!(!id.is_empty());
    (b, id)
}

fn crew_rows(app: &admiralty::app::App, id: &str) -> Vec<String> {
    app.rows.iter().filter(|r| r.node.id.starts_with(&format!("C:{id}:")) || r.node.id == format!("CF:{id}")).map(|r| r.node.name.clone()).collect()
}

/// Running crew and crew finished within 30 min show; older finished crew
/// fold into one `+N finished` row; a running child keeps its old parent in
/// view; the member's "N crew running" counts every running crew.
#[test]
fn old_finished_crew_fold_into_one_row_and_running_children_keep_their_parent() {
    let (b, id) = crew_world();
    let mut t = fixture_app(b, false);
    goto(&mut t.app, &format!("M:{id}"));
    let rows = crew_rows(&t.app, &id);
    assert_eq!(rows, ["fix layout bug", "old parent", "recent review", "+2 finished"], "{rows:?}");
    let parent = t.app.rows.iter().find(|r| r.node.id == format!("C:{id}:old-b")).unwrap();
    assert_eq!(parent.node.children.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["kid still running"], "nested crew follow their parent");
    let member = t.app.rows.iter().find(|r| r.node.id == format!("M:{id}")).unwrap();
    assert!(member.node.detail.iter().any(|s| s.content.contains("3 crew running")), "{:?}", member.node.detail);
    // z on the member opens the fold; z on a crew row inside it closes it again
    press(&mut t.app, KeyCode::Char('z'));
    let rows = crew_rows(&t.app, &id);
    assert!(rows.contains(&"old audit".to_string()) && rows.contains(&"summarize probe logs".to_string()), "{rows:?}");
    goto(&mut t.app, &format!("C:{id}:old-a"));
    press(&mut t.app, KeyCode::Char('z'));
    assert!(!crew_rows(&t.app, &id).contains(&"old audit".to_string()));
    assert_eq!(t.app.current().unwrap().node.id, format!("CF:{id}"), "the cursor moves to the fold row");
    // space on the fold row works too
    press(&mut t.app, KeyCode::Char(' '));
    assert!(crew_rows(&t.app, &id).contains(&"old audit".to_string()));
}

/// The fold's state is kept per member in the saved UI state.
#[test]
fn an_open_crew_fold_is_saved_and_loads_again() {
    let (b, id) = crew_world();
    let mut t = fixture_app(b.clone(), false);
    goto(&mut t.app, &format!("M:{id}"));
    press(&mut t.app, KeyCode::Char('z'));
    t.app.persist();
    let ctx = admiralty::actions::Ctx { env: fixtures::demo_env(), mux: Box::new(TestMux::default()), live: false, exit_after_jump: false };
    let mut again = admiralty::app::App::new(ctx, Some(t.dir.path().join("ui.json")), true);
    again.fixed_now = Some(fixtures::NOW);
    again.apply(b);
    assert!(crew_rows(&again, &id).contains(&"old audit".to_string()), "{:?}", crew_rows(&again, &id));
}

/// A state file a newer Admiralty wrote (its `format`), or one that does not
/// read, is left alone: not read, and never written over.
#[test]
fn a_newer_or_unreadable_ui_state_is_never_written_over() {
    let (b, id) = crew_world();
    let newer = r#"{"format": 2, "expanded": {}, "filter": "x", "fleet_states": [], "flagged_only": false, "show_stowaways": true, "future": 1}"#;
    for raw in [newer, "{not json"] {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ui.json");
        std::fs::write(&p, raw).unwrap();
        let ctx = admiralty::actions::Ctx { env: fixtures::demo_env(), mux: Box::new(TestMux::default()), live: false, exit_after_jump: false };
        let mut app = admiralty::app::App::new(ctx, Some(p.clone()), true);
        app.fixed_now = Some(fixtures::NOW);
        app.apply(b.clone());
        goto(&mut app, &format!("M:{id}"));
        press(&mut app, KeyCode::Char('z'));
        app.persist();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), raw);
    }
}

/// A member with no crew older than the window has no fold: `z` says so and
/// changes nothing (the member stays as it was).
#[test]
fn z_on_a_member_with_no_old_crew_says_so() {
    let mut t = demo();
    let b = t.app.bundle.clone().unwrap();
    let id = b.snap.fleets.iter().flat_map(admiralty::view::fleet_members).find(|m| !m.crew.is_empty() && admiralty::view::folded_crew(&m.crew, fixtures::NOW, "") == 0).map(|m| m.id.0.clone()).expect("a member with only recent crew");
    goto(&mut t.app, &format!("M:{id}"));
    let before = t.app.expanded.clone();
    press(&mut t.app, KeyCode::Char('z'));
    assert!(toast(&t.app).contains("no crew that finished over 30 min ago"), "{}", toast(&t.app));
    assert_eq!(t.app.expanded, before, "nothing opened or saved");
}

/// `z` on a collapsed member whose fold was open opens both again (it never
/// says the member has no old crew); ctrl-z is not `z`.
#[test]
fn z_on_a_collapsed_member_opens_it_with_its_fold() {
    let (b, id) = crew_world();
    let mut t = fixture_app(b, false);
    goto(&mut t.app, &format!("M:{id}"));
    press(&mut t.app, KeyCode::Char('z'));
    assert!(crew_rows(&t.app, &id).contains(&"old audit".to_string()));
    goto(&mut t.app, &format!("M:{id}"));
    press(&mut t.app, KeyCode::Char('h'));
    assert!(crew_rows(&t.app, &id).is_empty(), "collapsed");
    press(&mut t.app, KeyCode::Char('z'));
    assert!(!toast(&t.app).contains("no crew"), "{}", toast(&t.app));
    assert!(crew_rows(&t.app, &id).contains(&"old audit".to_string()), "{:?}", crew_rows(&t.app, &id));
    ctrl(&mut t.app, 'z');
    assert!(crew_rows(&t.app, &id).contains(&"old audit".to_string()), "ctrl-z does not toggle the fold");
}

/// A name filter that matches an old crew shows that crew outside the fold;
/// a crew whose last write is unknown folds, and the fold's age says `?`.
#[test]
fn a_filter_shows_the_old_crew_it_matches_and_unknown_ages_fold() {
    let (mut b, id) = crew_world();
    for f in &mut b.snap.fleets {
        for sq in &mut f.squadrons {
            for m in &mut sq.ships {
                if m.id.0 == id {
                    for c in &mut m.crew {
                        if c.agent_id == "sa-3" || c.agent_id == "old-a" {
                            c.last_write = 0.0;
                        }
                    }
                }
            }
        }
    }
    let mut t = fixture_app(b, false);
    goto(&mut t.app, &format!("M:{id}"));
    let fold = t.app.rows.iter().find(|r| r.node.id == format!("CF:{id}")).expect("the fold");
    assert_eq!((fold.node.name.as_str(), fold.node.age.as_str()), ("+2 finished", "?"));
    keys(&mut t.app, "/old audit");
    press(&mut t.app, KeyCode::Enter);
    goto(&mut t.app, &format!("M:{id}"));
    let rows = crew_rows(&t.app, &id);
    assert!(rows.contains(&"old audit".to_string()) && rows.contains(&"+1 finished".to_string()), "{rows:?}");
}

/// esc closes what is open, one thing at a time (a dialog, the detail
/// panel, the filter), and quits only when nothing is.
#[test]
fn esc_closes_what_is_open_then_quits() {
    let mut t = demo();
    keys(&mut t.app, "?");
    assert!(t.app.modal.is_some());
    assert!(press(&mut t.app, KeyCode::Esc).is_none() && t.app.modal.is_none(), "esc closes help");
    keys(&mut t.app, "w");
    assert!(t.app.detail_open);
    assert!(press(&mut t.app, KeyCode::Esc).is_none() && !t.app.detail_open, "esc closes the detail panel");
    keys(&mut t.app, "/kits");
    assert!(press(&mut t.app, KeyCode::Esc).is_none(), "esc while typing the filter ends it");
    keys(&mut t.app, "/kits");
    press(&mut t.app, KeyCode::Enter);
    assert!(!t.app.filter.is_empty());
    assert!(press(&mut t.app, KeyCode::Esc).is_none() && t.app.filter.is_empty(), "esc clears the filter");
    assert!(matches!(press(&mut t.app, KeyCode::Esc), Some(Effect::Quit)), "with nothing open, esc quits");
}

/// While an action runs, esc does not quit (q still does), and the running
/// line says so; in a dialog, menu, form or message, esc closes it and
/// never quits.
#[test]
fn esc_never_quits_mid_action_or_from_a_dialog() {
    let mut t = demo();
    t.app.busy = Some("sweep FORGE".into());
    assert!(press(&mut t.app, KeyCode::Esc).is_none(), "busy: no quit");
    let foot = lines(&draw(&mut t.app, 120, 30)).pop().unwrap();
    assert!(foot.contains("esc waits") && foot.contains("sweep FORGE"), "{foot}");
    t.app.busy = None;
    for (k, what) in [('x', "the actions menu"), ('c', "the create menu"), ('m', "a message"), ('!', "needs you"), ('i', "issues"), ('?', "help")] {
        goto(&mut t.app, "M:MAIN/ARGOS");
        press(&mut t.app, KeyCode::Char(k));
        assert!(t.app.modal.is_some(), "{k} opens {what}");
        assert!(press(&mut t.app, KeyCode::Esc).is_none(), "{k}: esc closes, never quits");
        assert!(t.app.modal.is_none(), "{k}: closed");
    }
    // a form (c, then s: a new ship) and a confirm (a: start, top-down)
    goto(&mut t.app, "M:MAIN/ARGOS");
    keys(&mut t.app, "cs");
    assert!(matches!(t.app.modal, Some(Modal::Form(_))), "a form: {:?}", t.app.modal.as_ref().map(std::mem::discriminant));
    assert!(press(&mut t.app, KeyCode::Esc).is_none() && t.app.modal.is_none(), "esc cancels the form");
    goto(&mut t.app, "M:MAIN/KITS/DOCS");
    keys(&mut t.app, "a");
    assert!(matches!(t.app.modal, Some(Modal::Confirm(_))), "a confirm");
    assert!(press(&mut t.app, KeyCode::Esc).is_none() && t.app.modal.is_none(), "esc cancels the confirm");
}

/// A fleet whose config does not load: its error, in full, at the top of
/// the admiral's box (under its title), every line of it together.
#[test]
fn a_broken_fleets_error_stays_together_under_its_band() {
    let mut b = fixtures::demo();
    let f = &mut b.snap.fleets[0];
    let id = f.id.clone();
    f.issues.push(nimitz_core::Issue { code: "config.invalid".into(), severity: nimitz_core::Severity::Error, message: format!("{}/fleet.toml:3: expected `=`\nline two of the parser", id.0.to_lowercase()), fix: Some(format!("nimitz check {}", id.0)), id: Some(id.clone()) });
    let mut t = fixture_app(b, false);
    tab(&mut t.app, &id.0);
    let ls = lines(&draw(&mut t.app, 120, 60));
    let top = ls.iter().position(|l| l.contains("ADMIRAL ")).unwrap();
    let first = ls.iter().position(|l| l.contains("fleet.toml:3")).unwrap_or_else(|| panic!("the error:\n{}", ls.join("\n")));
    let adm = ls.iter().position(|l| l.starts_with('\u{2503}') && l.contains("ADMIRAL   ")).unwrap();
    assert_eq!(first, top + 1, "right under the title:\n{}", ls.join("\n"));
    assert!(ls[first..adm].iter().all(|l| l.starts_with('\u{2503}') && l.chars().skip(1).any(|c| !c.is_whitespace() && c != '\u{2503}')), "the error's lines together, in the box:\n{}", ls[top..=adm].join("\n"));
}

/// With every squadron hidden by the state filter the note sits below the
/// admiral's box, not in it; with the admiral filtered out its box says so.
#[test]
fn the_admirals_box_with_everything_or_the_admiral_filtered() {
    let mut t = demo();
    for i in 0..t.app.fleet_states.len() {
        t.app.fleet_states[i] = false;
    }
    t.app.rebuild();
    let ls = lines(&draw(&mut t.app, 200, 60));
    let n = ls.iter().position(|l| l.contains("hidden by the state filter")).unwrap_or_else(|| panic!("{}", ls.join("\n")));
    assert!(!ls[n].starts_with('\u{2503}'), "outside the box: {}", ls[n]);
    let above = ls[..n].iter().rev().find(|l| !l.trim().is_empty()).unwrap();
    assert!(above.starts_with('\u{2517}'), "the admiral's box closed above it:\n{}", ls.join("\n"));
    // F: flagged only; MAIN's admiral has no issue and does not need you
    let mut t = demo();
    keys(&mut t.app, "F");
    let ls = lines(&draw(&mut t.app, 200, 60));
    assert!(!ls.iter().any(|l| l.starts_with('\u{2503}') && l.contains("ADMIRAL   ")), "F hides MAIN's admiral:\n{}", ls.join("\n"));
    let top = ls.iter().position(|l| l.contains("[MAIN] ADMIRAL")).unwrap();
    assert!(ls[top + 1].contains("the admiral is hidden by the filters"), "{}", ls[top + 1]);
}

/// Inside its fleet's tab and its squadron's box a row names only its own
/// part; a search across fleets, the detail panel and a session running
/// under another name keep the full name.
#[test]
fn rows_name_only_their_own_part_inside_their_fleet_and_squadron() {
    let mut t = demo();
    let ls = lines(&draw(&mut t.app, 200, 60));
    for full in ["[MAIN]-ADMIRAL", "[MAIN.ARGOS]-COMMODORE", "[MAIN.ARGOS]-BOOM", "[MAIN.KITS]-PIPELINE"] {
        assert!(!ls.iter().any(|l| l.contains(full)), "no {full} in the rows:\n{}", ls.join("\n"));
    }
    for short in ["ADMIRAL   ", "COMMODORE ", "BOOM ", "PIPELINE "] {
        assert!(ls.iter().any(|l| l.contains(short)), "{short}");
    }
    // at 80 columns the names fit (no ellipsis on a member name)
    let ls = lines(&draw(&mut t.app, 80, 40));
    for name in ["COMMODORE", "BOLT", "BOOM", "INGEST", "DOCS", "PIPELINE"] {
        let l = ls.iter().find(|l| l.contains(name)).unwrap_or_else(|| panic!("{name}:\n{}", ls.join("\n")));
        assert!(!l.contains(&format!("{name}\u{2026}")), "{l}");
    }
    // a search across fleets: full names
    keys(&mut t.app, "/boom");
    press(&mut t.app, KeyCode::Enter);
    let ls = lines(&draw(&mut t.app, 200, 60));
    assert!(ls.iter().any(|l| l.contains("[MAIN.ARGOS]-BOOM")), "{}", ls.join("\n"));
    // at 80 columns a search cuts the full name from the left: the own part stays
    let ls = lines(&draw(&mut t.app, 80, 40));
    assert!(ls.iter().any(|l| l.contains("\u{2026}") && l.contains("]-BOOM")), "cut from the left:\n{}", ls.join("\n"));
    // a squadron-only match: full names, and n steps through them
    press(&mut t.app, KeyCode::Esc);
    keys(&mut t.app, "/argos");
    press(&mut t.app, KeyCode::Enter);
    let ls = lines(&draw(&mut t.app, 200, 60));
    assert!(ls.iter().any(|l| l.contains("[MAIN.ARGOS]-BOLT")), "{}", ls.join("\n"));
    keys(&mut t.app, "n");
    let first = t.app.current().unwrap().node.name.clone();
    keys(&mut t.app, "n");
    let second = t.app.current().unwrap().node.name.clone();
    assert!(first.starts_with("[MAIN.ARGOS]-") && second.starts_with("[MAIN.ARGOS]-") && first != second, "{first} {second}");
    // a drift under the member's own prefix shows its own part, drift and all
    press(&mut t.app, KeyCode::Esc);
    tab(&mut t.app, "FORGE");
    let ls = lines(&draw(&mut t.app, 80, 60));
    assert!(ls.iter().any(|l| l.contains("ANVIL-2") && !l.contains("[FORGE.IRON]")), "{}", ls.join("\n"));
}

/// A session named after a sibling (ANVIL's session called `[F.SQ]-HAMMER`)
/// keeps its full name: never a second HAMMER row that acts on ANVIL.
#[test]
fn a_session_named_after_a_sibling_keeps_its_full_name() {
    let mut b = fixtures::demo();
    for f in &mut b.snap.fleets {
        for sq in &mut f.squadrons {
            for m in sq.ships.iter_mut().filter(|m| m.id.0 == "MAIN/ARGOS/BOOM") {
                if let Some(s) = m.session.as_mut() {
                    s.name = "[MAIN.ARGOS]-BOLT".into();
                }
            }
        }
    }
    let mut t = fixture_app(b, false);
    let ls = lines(&draw(&mut t.app, 200, 60));
    assert_eq!(ls.iter().filter(|l| l.contains("[MAIN.ARGOS]-BOLT")).count(), 1, "{}", ls.join("\n"));
    let boom = t.app.rows.iter().find(|r| r.node.id == "M:MAIN/ARGOS/BOOM").unwrap();
    assert_eq!(boom.node.name, "[MAIN.ARGOS]-BOLT");
    // a name that only begins with the member's own (a sibling BOOMX could have it)
    let mut b = fixtures::demo();
    for f in &mut b.snap.fleets {
        for sq in &mut f.squadrons {
            for m in sq.ships.iter_mut().filter(|m| m.id.0 == "MAIN/ARGOS/BOOM") {
                if let Some(s) = m.session.as_mut() {
                    s.name = "[MAIN.ARGOS]-BOOMX".into();
                }
            }
        }
    }
    let t = fixture_app(b, false);
    let boom = t.app.rows.iter().find(|r| r.node.id == "M:MAIN/ARGOS/BOOM").unwrap();
    assert_eq!(boom.node.name, "[MAIN.ARGOS]-BOOMX", "no dash after its own name: full");
}

// ------------------------------------------------------------------ status motion

/// In the live loop the busy glyph spins, a crew's spinner is dimmer, the
/// needs-you chip breathes; nothing else on the screen moves, and no column
/// shifts. With the motion off (or outside the live loop) every frame is the same.
#[test]
fn status_motion_changes_only_the_status_cell() {
    let mut t = demo();
    let still = lines(&draw(&mut t.app, 200, 60));
    t.app.anim_live = true;
    let mut seen_spin = std::collections::HashSet::new();
    let mut fills = std::collections::HashSet::new();
    for f in 0..12u64 {
        t.app.anim_frame = f;
        let buf = draw(&mut t.app, 200, 60);
        let ls = lines(&buf);
        assert_eq!(ls.len(), still.len());
        for (a, b) in ls.iter().zip(still.iter()) {
            assert_eq!(a.chars().count(), b.chars().count(), "no column moves:\n{a}\n{b}");
        }
        let bolt = ls.iter().position(|l| l.contains("BOLT")).unwrap();
        let x = still[bolt].chars().position(|c| c == '*').unwrap();
        let glyph = ls[bolt].chars().nth(x).unwrap();
        assert!("|/=\\".contains(glyph), "busy spins (the icon set's frames): {}", ls[bolt]);
        seen_spin.insert(glyph);
        let kits = ls.iter().position(|l| l.contains("PIPELINE")).unwrap();
        let cx = ls[kits].find("needs you").unwrap();
        let cx = ls[kits][..cx].chars().count() as u16;
        fills.insert(format!("{:?}", buf[(cx, kits as u16)].bg));
        // everything but the status cells is the still picture
        for (y, (a, b)) in ls.iter().zip(still.iter()).enumerate() {
            if y != bolt && !a.contains("needs you") && !a.contains("busy") && !a.contains("running") {
                assert_eq!(a, b, "line {y} moved");
            }
        }
    }
    assert_eq!(seen_spin.len(), 4, "a spinner: {seen_spin:?}");
    assert!(fills.len() >= 3, "the needs-you chip breathes: {fills:?}");
    // off: `v`, saved; every frame the still picture
    keys(&mut t.app, "v");
    assert!(!t.app.anim && t.app.frame().is_none());
    t.app.anim_frame = 7;
    let off = lines(&draw(&mut t.app, 200, 60));
    assert_eq!(off[..off.len() - 1], still[..still.len() - 1], "(the footer has the toast)");
    t.app.persist();
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(t.dir.path().join("ui.json")).unwrap()).unwrap();
    assert_eq!(saved["anim"], false);
    // outside the live loop (tests, dump) nothing moves even when on
    keys(&mut t.app, "v");
    t.app.anim_live = false;
    t.app.anim_frame = 3;
    let off = lines(&draw(&mut t.app, 200, 60));
    assert_eq!(off[..off.len() - 1], still[..still.len() - 1]);
}

/// A squadron whose status gets worse pulses its bar once, then holds its colour.
#[test]
fn a_squadron_that_gets_worse_pulses_its_bar_once() {
    let mut b = fixtures::demo();
    let mut t = fixture_app(b.clone(), false);
    t.app.anim_live = true;
    t.app.anim_frame = 100;
    // ARGOS: a member now needs you
    for f in &mut b.snap.fleets {
        for sq in &mut f.squadrons {
            if sq.id.0 == "MAIN/ARGOS" {
                sq.ships[0].status = Status::NeedsYou;
            }
        }
    }
    t.app.apply(b);
    let row = t.app.rows.iter().find(|r| r.node.id == "SQ:MAIN/ARGOS").unwrap();
    assert_eq!(row.node.anim, admiralty::view::Anim::Flash(100));
    let fill = |t: &mut TestApp, f: u64| {
        t.app.anim_frame = f;
        let buf = draw(&mut t.app, 200, 60);
        let ls = lines(&buf);
        let y = ls.iter().position(|l| l.contains("ARGOS") && l.contains("comms")).unwrap();
        buf[(2, y as u16)].bg
    };
    assert_ne!(fill(&mut t, 105), mono::NEEDS, "mid-pulse: lighter");
    assert_eq!(fill(&mut t, 111), mono::NEEDS, "then it holds");
    // KITS did not change: no pulse
    assert!(t.app.rows.iter().find(|r| r.node.id == "SQ:MAIN/KITS").is_some_and(|r| r.node.anim == admiralty::view::Anim::None));
}

/// The pulse only for a squadron that gets a warning, an error or needs you
/// (idle to busy is just work), and never from the quick first look to the
/// first full one.
#[test]
fn a_bar_pulses_only_for_a_real_worsening() {
    let full = fixtures::demo();
    let mut quick = full.clone();
    quick.partial = true;
    for f in &mut quick.snap.fleets {
        for sq in &mut f.squadrons {
            for m in std::iter::once(&mut sq.commodore).chain(sq.ships.iter_mut()) {
                m.issues.clear();
            }
        }
    }
    let mut t = fixture_app(quick, false);
    t.app.anim_live = true;
    t.app.apply(full.clone());
    tab(&mut t.app, "FORGE");
    assert!(t.app.rows.iter().all(|r| !matches!(r.node.anim, admiralty::view::Anim::Flash(_))), "no pulse at start-up");
    // idle to busy on a quiet squadron (LAB/OPTICS, rank 0): no pulse
    let mut busy = full.clone();
    for f in &mut busy.snap.fleets {
        for sq in f.squadrons.iter_mut().filter(|q| q.id.0 == "LAB/OPTICS") {
            assert_eq!(admiralty::view::squadron_rank_of(sq), 0, "a quiet squadron");
            sq.ships[0].status = Status::Busy;
            sq.ships[0].issues.clear();
            assert_eq!(admiralty::view::squadron_rank_of(sq), 1, "now busy");
        }
    }
    t.app.apply(busy);
    tab(&mut t.app, "LAB");
    keys(&mut t.app, "l");
    assert!(t.app.rows.iter().all(|r| !matches!(r.node.anim, admiralty::view::Anim::Flash(_))), "work is no pulse");
}

/// Motion pauses under a dialog; with only a shell wait on screen the loop
/// redraws every fourth frame; `v` says so when the environment keeps it off.
#[test]
fn motion_steps_pause_and_the_env_switch() {
    let mut t = demo();
    t.app.anim_live = true;
    assert_eq!(t.app.motion_step(), Some(1));
    keys(&mut t.app, "?");
    assert_eq!(t.app.motion_step(), None, "a dialog: still");
    press(&mut t.app, KeyCode::Esc);
    for r in t.app.rows.iter_mut() {
        r.node.anim = if r.node.anim == admiralty::view::Anim::None { admiralty::view::Anim::None } else { admiralty::view::Anim::Dots };
    }
    assert_eq!(t.app.motion_step(), Some(4), "only a shell wait: every fourth frame");
    t.app.anim_env_off = true;
    t.app.anim = false;
    keys(&mut t.app, "v");
    assert!(toast(&t.app).contains("keeps it still"), "{}", toast(&t.app));
}

/// The dim glyph colour passes 3:1 (a non-text glyph) on the background and on
/// the cursor row; the word next to it keeps its own colour.
#[test]
fn the_dim_glyph_passes_contrast_and_the_word_keeps_its_colour() {
    use admiralty::theme::anim::BUSY_DIM;
    let lum = |c: ratatui::style::Color| {
        let ratatui::style::Color::Rgb(r, g, b) = c else { panic!() };
        let f = |v: u8| {
            let v = v as f64 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
    };
    let ratio = |a, b| {
        let (x, y): (f64, f64) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    };
    assert!(ratio(BUSY_DIM, mono::BG) >= 3.0 && ratio(BUSY_DIM, mono::SEL) >= 3.0);
    let mut t = demo();
    t.app.anim_live = true;
    t.app.anim_frame = 2;
    let buf = draw(&mut t.app, 200, 60);
    let ls = lines(&buf);
    let y = ls.iter().position(|l| l.contains("fix layout bug")).unwrap();
    let x = ls[y].find("running").unwrap();
    let x = ls[y][..x].chars().count() as u16;
    assert_eq!(buf[(x, y as u16)].fg, mono::BUSY, "the word keeps its colour");
    assert_eq!(buf[(x - 2, y as u16)].fg, BUSY_DIM, "the glyph is the dim one");
}

// ------------------------------------------------------------------ Herdr overlay (inset)

/// Herdr's plugin overlay sizes the terminal to the whole tab, then draws a
/// border inside it: the last 2 rows and 2 columns are never shown. With the
/// inset `watch.sh` sets, the status bar and the key bar sit above that edge,
/// and no row or box reaches into it.
#[test]
fn in_herdrs_overlay_the_status_and_key_bars_stay_in_view() {
    for (w, h) in [(160u16, 44u16), (120, 30), (80, 24), (80, 12)] {
        let mut t = demo();
        t.app.inset = (2, 2);
        let ls = lines(&draw(&mut t.app, w, h));
        let (rows, cols) = (h as usize - 2, w as usize - 2);
        assert!(ls[rows - 2].contains("ADMIRALTY"), "{w}x{h}: the status bar in view:\n{}", ls.join("\n"));
        assert!(ls[rows - 1].contains("? help"), "{w}x{h}: the key bar in view: {}", ls[rows - 1]);
        assert!(ls[rows..].iter().all(|l| l.trim().is_empty()), "{w}x{h}: nothing under the border:\n{}", ls[rows..].join("\n"));
        assert!(ls.iter().all(|l| l.chars().skip(cols).all(|c| c == ' ')), "{w}x{h}: nothing under the right border");
    }
}

// ------------------------------------------------------------------ m, message

/// `m` is on the key bar at 80 columns, right after the pane jump: the two
/// ways to reach a lead.
#[test]
fn the_key_bar_keeps_m_at_80_columns() {
    let mut t = demo();
    for w in [80u16, 100, 200] {
        let ls = lines(&draw(&mut t.app, w, 24));
        assert!(ls[23].contains("m message"), "{w}: {}", ls[23]);
    }
}

/// `m` on a lead row opens the message dialog and types into the lead's pane;
/// on a ship it is relayed by the commodore.
#[test]
fn m_on_a_lead_row_sends_into_its_pane() {
    let mut t = fixture_app(fixtures::demo(), true);
    goto(&mut t.app, "M:MAIN/ARGOS");
    press(&mut t.app, KeyCode::Char('m'));
    assert!(matches!(t.app.modal, Some(Modal::Message { .. })), "the dialog: {}", toast(&t.app));
    keys(&mut t.app, "hello lead");
    press(&mut t.app, KeyCode::Enter);
    let calls = t.mux.calls.lock().unwrap().clone();
    assert!(calls.iter().any(|c| c.starts_with("type ") && c.contains("hello lead")), "{calls:?}");
    assert!(toast(&t.app).starts_with("Sent to"), "{}", toast(&t.app));
}

/// Where `m` cannot send, it says why and what to do.
#[test]
fn m_refusals_name_the_fix() {
    let mut t = fixture_app(fixtures::demo(), true);
    // a stopped ship whose commodore runs, but the ship does not: relay refused
    goto(&mut t.app, "M:MAIN/KITS/DOCS");
    press(&mut t.app, KeyCode::Char('m'));
    let why = toast(&t.app);
    assert!(why.contains("not running") && why.contains("a starts it"), "{why}");
    // a stowaway
    t.app.show_stowaways = true;
    t.app.rebuild();
    let i = t.app.rows.iter().position(|r| r.node.kind == Kind::Stowaway).expect("a stowaway row");
    t.app.cursor = i;
    press(&mut t.app, KeyCode::Char('m'));
    let why = toast(&t.app);
    assert!(why.contains("adopt") && why.contains("x then d"), "{why}");
}

/// The message box shows exactly what was typed, trailing and repeated spaces
/// included, the moment each key lands; the text sent is the same, minus
/// control characters and escape sequences.
#[test]
fn the_message_box_shows_and_sends_exactly_what_was_typed() {
    let mut t = fixture_app(fixtures::demo(), true);
    goto(&mut t.app, "M:MAIN/ARGOS");
    press(&mut t.app, KeyCode::Char('m'));
    // one key, one frame: the space is on screen before anything follows it
    keys(&mut t.app, "a ");
    let s = screen(&mut t.app, 120, 30);
    let line = s.lines().find(|l| l.contains("> a")).unwrap_or_else(|| panic!("{s}"));
    assert!(line.contains("> a \u{2588}"), "the trailing space shows at once: {line}");
    keys(&mut t.app, " b ");
    let s = screen(&mut t.app, 120, 30);
    assert!(s.contains("> a  b \u{2588}"), "repeated spaces stay:\n{s}");
    press(&mut t.app, KeyCode::Enter);
    let calls = t.mux.calls.lock().unwrap().clone();
    assert!(calls.iter().any(|c| c.ends_with(" a  b ")), "sent as typed: {calls:?}");
}

// ------------------------------------------------------------------ review follow-ups

/// A paste lands whole in the `m` box: its newlines become spaces (a newline
/// would submit the prompt early), nothing is sent, and nothing leaks into
/// the list as keys.
#[test]
fn a_multi_line_paste_stays_in_the_message_box() {
    let mut t = fixture_app(fixtures::demo(), true);
    goto(&mut t.app, "M:MAIN/ARGOS");
    press(&mut t.app, KeyCode::Char('m'));
    let cursor = t.app.cursor;
    assert!(t.app.on_paste("first line\nsecond line\r\nthird axd").is_none());
    let Some(Modal::Message { text, .. }) = &t.app.modal else { panic!("the box is still open: {:?}", t.app.modal.is_some()) };
    assert_eq!(text, "first line second line third axd");
    assert_eq!(t.app.cursor, cursor, "no key reached the list");
    assert!(t.mux.calls.lock().unwrap().iter().all(|c| !c.starts_with("type ")), "nothing sent");
    // in the list, with nothing open: a paste is no keystrokes either
    press(&mut t.app, KeyCode::Esc);
    let rows = t.app.rows.len();
    assert!(t.app.on_paste("ax\nd").is_none());
    assert!(t.app.modal.is_none() && t.app.rows.len() == rows && t.app.cursor == cursor);
    // in the filter: one line of text
    keys(&mut t.app, "/");
    t.app.on_paste("bs\nim");
    assert_eq!(t.app.filter, "bs im");
    // a lone CR (some terminals send a pasted line break as one) is a break too
    press(&mut t.app, KeyCode::Esc);
    press(&mut t.app, KeyCode::Esc);
    goto(&mut t.app, "M:MAIN/ARGOS");
    press(&mut t.app, KeyCode::Char('m'));
    t.app.on_paste("one\rtwo");
    let Some(Modal::Message { text, .. }) = &t.app.modal else { panic!("the box is open") };
    assert_eq!(text, "one two");
}

/// A paste into a form: a one-line field takes it as one line, a long field
/// keeps its lines (CRLF and a lone CR become \n), a fixed or choice field
/// takes nothing, and the form is checked as after a key.
#[test]
fn a_paste_into_a_form_keeps_long_lines_and_is_validated() {
    use admiralty::forms::Form;
    let mut f = Form::new_squadron("MAIN");
    f.focus = 1;
    f.paste("OPS\nTWO");
    assert_eq!(f.fields[1].value, "OPS TWO");
    f.focus = 3;
    f.paste("line one\r\nline two\rline three\nend");
    assert_eq!(f.fields[3].value, "line one\nline two\nline three\nend");
    f.focus = 0;
    f.paste("X");
    assert_eq!(f.fields[0].value, "MAIN", "a fixed field takes nothing");

    let mut t = demo();
    keys(&mut t.app, "c");
    keys(&mut t.app, "f");
    t.app.on_paste("lab2");
    let Some(Modal::Form(f)) = &t.app.modal else { panic!() };
    assert_eq!(f.fields[0].value, "lab2");
    assert!(f.fields[0].error.as_deref().is_some_and(|e| e.contains("UPPERCASE")), "checked without a key");
}

/// A dialog that is not for text ignores a paste: a pasted "y" never confirms.
#[test]
fn a_paste_into_a_confirm_dialog_does_nothing() {
    let mut t = demo();
    goto(&mut t.app, "M:MAIN/KITS/DOCS");
    keys(&mut t.app, "a");
    assert!(matches!(t.app.modal, Some(Modal::Confirm(_))));
    assert!(t.app.on_paste("y\n").is_none());
    assert!(matches!(t.app.modal, Some(Modal::Confirm(_))), "still asking");
    assert!(t.mux.calls.lock().unwrap().iter().all(|c| !c.starts_with("type ")));
}

/// $EDITOR inside Herdr's overlay sees the terminal minus the inset, so its
/// last lines (vim's command line, nano's help) are in view.
#[test]
fn the_editor_sees_the_terminal_minus_the_inset() {
    use admiralty::run::editor_size;
    assert_eq!(editor_size((44, 134), (2, 2)), (42, 132));
    assert_eq!(editor_size((44, 134), (0, 0)), (44, 134));
    assert_eq!(editor_size((2, 3), (2, 2)), (1, 1), "never below one cell");
}

/// The animation tick runs only while an animated cell is on screen: rows
/// scrolled off, or a screen with no moving row in view, keep it idle.
#[test]
fn the_tick_runs_only_for_animated_cells_in_view() {
    let mut t = demo();
    t.app.anim_live = true;
    draw(&mut t.app, 200, 60);
    assert_eq!(t.app.motion_step(), Some(1), "spinners in view");
    // a screen so short that only the admiral's box shows: its crew spins
    // (in view); scroll to a quiet stretch with nothing moving
    tab(&mut t.app, "MAIN");
    keys(&mut t.app, "G");
    draw(&mut t.app, 200, 7);
    let shown: Vec<String> = lines(&draw(&mut t.app, 200, 7));
    // the list's lines only (not the tab bar, the header, the status and key bars)
    let list = &shown[2..shown.len() - 2];
    let moving_in_view = list.iter().any(|l| l.contains("busy") || l.contains("running") || l.contains("needs you"));
    assert_eq!(t.app.motion_step().is_some(), moving_in_view, "tick iff a moving cell shows:\n{}", shown.join("\n"));
    assert!(!moving_in_view, "this view shows nothing moving (the stowaways):\n{}", shown.join("\n"));
    assert_eq!(t.app.motion_step(), None, "spinners scrolled off: no tick");
}

// ------------------------------------------------------------------ aides

/// The demo with MAIN's aide `@KB` changed by `f`.
fn with_kb(f: impl FnOnce(&mut nimitz_core::Member)) -> TestApp {
    let mut b = fixtures::demo();
    f(b.snap.fleets.iter_mut().flat_map(|fl| fl.aides.iter_mut()).find(|m| m.id.0 == "MAIN/@KB").expect("the demo aide"));
    fixture_app(b, false)
}

/// An aide's row sits in its admiral's box, right after the admiral (and
/// its crew): name, state word, then its queue; at 80 columns the queue
/// keeps its dead count and its pending count, whole, before the rest.
#[test]
fn an_aide_row_sits_in_the_admirals_box_with_its_queue() {
    let mut t = demo();
    let ls = lines(&draw(&mut t.app, 120, 60));
    let adm = ls.iter().position(|l| l.contains("ADMIRAL   ")).unwrap();
    let kb = ls.iter().position(|l| l.contains("@KB")).expect("the aide's row");
    assert!(kb > adm && ls[kb].starts_with('\u{2503}') && ls[kb + 1].starts_with('\u{2517}'), "in the admiral's heavy box, last:\n{}", ls[adm - 1..=kb + 1].join("\n"));
    assert!(ls[kb].contains("A @KB") && ls[kb].contains("* busy"), "its own icon (A) and state: {}", ls[kb]);
    assert!(ls[kb].contains("\u{b7} 3 pending \u{b7} 1 claimed \u{b7} 1 dead"), "at 120 the age goes first: {}", ls[kb]);
    let wide = lines(&draw(&mut t.app, 200, 60));
    assert!(wide.iter().any(|l| l.contains("@KB") && l.contains("\u{b7} 3 pending \u{b7} 1 claimed \u{b7} oldest 4m \u{b7} 1 dead")), "the whole queue, in `Status::line` order, when it fits");
    let ls80 = lines(&draw(&mut t.app, 80, 60));
    let row = ls80.iter().find(|l| l.contains("@KB")).unwrap();
    assert!(row.contains("\u{b7} 3 pending \u{b7} 1 dead") && !row.contains("oldest") && !row.contains("claimed") && !row.contains('\u{2026}'), "dead and pending first, whole parts: {row}");
    // a narrow cell that fits nothing whole still shows what it can
    let r = admiralty::frames::drop_spans(&[ratatui::text::Span::raw("\u{b7} 1 dead "), ratatui::text::Span::raw("\u{b7} 3 pending ")], &[1, 2], 4);
    assert_eq!(r.iter().map(|s| s.content.to_string()).collect::<String>().chars().count(), 4);
}

/// Its status colour follows its queue: a dead letter is the needs-you chip
/// (pulsing), a backlog the warning yellow, an item claimed the busy green
/// (spinning); else its own state's colour.
#[test]
fn an_aide_row_takes_its_colour_from_its_queue() {
    use admiralty::view::{Anim, QueueMood, BACKLOG_AGE_S, BACKLOG_PENDING};
    let cell = |t: &mut TestApp| {
        let buf = draw(&mut t.app, 120, 60);
        let ls = lines(&buf);
        let y = ls.iter().position(|l| l.contains("@KB")).unwrap();
        let node = &t.app.rows.iter().find(|r| r.node.id == "M:MAIN/@KB").unwrap().node;
        let (word, anim) = (node.status.0.clone(), node.anim);
        // the word's first letter (after the glyph and a space)
        let x = (col_of(&ls, "@KB", &word) + 2) as u16;
        (buf[(x, y as u16)].fg, buf[(x, y as u16)].bg, anim)
    };
    // the demo: a dead letter
    let mut t = demo();
    let (fg, bg, anim) = cell(&mut t);
    assert_eq!((fg, bg, anim), (mono::BG, mono::NEEDS, Anim::Pulse), "dead: the pink chip, pulsing");
    let mood = |t: &TestApp| t.app.bundle.as_ref().unwrap().snap.fleets[0].aides[0].inbox.as_ref().and_then(admiralty::view::queue_mood);
    assert_eq!(mood(&t), Some(QueueMood::Dead));
    // a backlog: past either threshold
    for (pending, oldest) in [(BACKLOG_PENDING + 1, 60.0), (2, BACKLOG_AGE_S + 1.0)] {
        let mut t = with_kb(|m| {
            let q = m.inbox.as_mut().unwrap();
            q.dead.clear();
            q.pending = pending;
            q.oldest_age_s = Some(oldest);
        });
        assert_eq!(mood(&t), Some(QueueMood::Backlog));
        let (fg, _, anim) = cell(&mut t);
        assert_eq!((fg, anim), (mono::WARN, Anim::None), "{pending} pending, oldest {oldest}s");
    }
    // working: an item claimed by it
    let mut t = with_kb(|m| m.inbox.as_mut().unwrap().dead.clear());
    assert_eq!(mood(&t), Some(QueueMood::Working));
    assert_eq!(cell(&mut t).0, mono::BUSY);
    assert_eq!(cell(&mut t).2, Anim::Spin);
    // a claim or a backlog on an aide that is not running: its own state says it
    for backlog in [false, true] {
        let mut t = with_kb(|m| {
            m.status = Status::Stopped;
            if let Some(s) = m.session.as_mut() {
                s.pid = None;
            }
            let q = m.inbox.as_mut().unwrap();
            q.dead.clear();
            if backlog {
                q.pending = BACKLOG_PENDING + 5;
            }
        });
        let (fg, _, anim) = cell(&mut t);
        assert_eq!((fg, anim), (mono::MUTED, Anim::None), "stopped, backlog {backlog}");
    }
    // a dead letter shows even then
    let mut t = with_kb(|m| {
        m.status = Status::Stopped;
        if let Some(s) = m.session.as_mut() {
            s.pid = None;
        }
    });
    assert_eq!(cell(&mut t).1, mono::NEEDS);
    // nothing to say: its own state (idle)
    let mut t = with_kb(|m| {
        m.status = Status::Idle;
        let q = m.inbox.as_mut().unwrap();
        q.dead.clear();
        q.claimed.clear();
        q.pending = 0;
        q.oldest_age_s = None;
    });
    assert_eq!(mood(&t), None);
    let (fg, bg, anim) = cell(&mut t);
    assert_eq!((fg, anim), (mono::FG, Anim::None));
    assert_ne!(bg, mono::NEEDS);
    assert!(screen(&mut t.app, 120, 60).contains("\u{b7} empty"));
}

/// `w` on an aide: its queue in full (pending, the claim and its lease,
/// done, the dead letters with their reasons, the senders' limits, the
/// newest items with a reply's result), and retry and drop offered only
/// while there is a dead letter.
#[test]
fn an_aides_detail_shows_its_queue_and_offers_retry_and_drop_for_a_dead_letter() {
    for w in [80u16, 120] {
        let mut t = demo();
        goto(&mut t.app, "M:MAIN/@KB");
        keys(&mut t.app, "w");
        let s = screen(&mut t.app, w, 120);
        for want in ["INBOX", "pending     3", "ends in 7m", "done        12  kept 7d (keep_done)", "DEAD LETTERS (1)", "merge conflict", "SENDERS", "7/60 per 1h", "frees in 30m", "RECENT", "result  Leases are 15", "x T retry its oldest dead letter", "x X drop its oldest dead letter"] {
            assert!(s.contains(want), "{w}: {want}\n{s}");
        }
        if w == 120 {
            assert!(s.contains("|  inbox  |"), "its own picture:\n{s}");
        }
        // no dead letter: no retry, no drop
        let mut t = with_kb(|m| m.inbox.as_mut().unwrap().dead.clear());
        goto(&mut t.app, "M:MAIN/@KB");
        keys(&mut t.app, "w");
        let s = screen(&mut t.app, w, 120);
        assert!(s.contains("INBOX") && !s.contains("DEAD LETTERS") && !s.contains("x T") && !s.contains("x X"), "{w}:\n{s}");
    }
    // the menu runs `nimitz inbox retry <aide> <id>` through the confirm step
    let mut t = demo();
    goto(&mut t.app, "M:MAIN/@KB");
    keys(&mut t.app, "xT");
    let Some(admiralty::app::Modal::Confirm(p)) = t.app.modal.take() else { panic!("a confirm: {}", toast(&t.app)) };
    assert_eq!((p.act, p.command.as_str()), (Act::InboxRetry, "nimitz inbox retry MAIN/@KB 37"));
    assert_eq!(p.plan.steps[0].cmd, ["nimitz", "inbox", "retry", "MAIN/@KB", "37"]);
    keys(&mut t.app, "xX");
    let Some(admiralty::app::Modal::Confirm(p)) = t.app.modal.take() else { panic!("a confirm: {}", toast(&t.app)) };
    assert_eq!(p.command, "nimitz inbox drop MAIN/@KB 37");
    // not on a ship
    goto(&mut t.app, "M:MAIN/ARGOS/BOLT");
    assert!(t.app.available(Act::InboxRetry).is_err());
}

/// The help legend has the aide's line.
#[test]
fn the_legend_names_the_aide() {
    let mut t = demo();
    keys(&mut t.app, "?");
    assert!(screen(&mut t.app, 200, 120).contains("aide: always on, works its inbox"));
}

/// `admiralty dump`: the list, or (`--height`, `--detail`) the whole screen
/// with a row selected and its detail panel open; `--ansi` with its colours.
#[test]
fn dump_renders_the_list_or_the_screen_plain_or_in_colour() {
    let run = |args: &[&str]| {
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_admiralty")).args(args).env("ADMIRALTY_ICONS", "ascii").output().unwrap();
        assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let list = run(&["dump", "--demo", "--width", "80"]);
    assert!(list.contains("@KB") && !list.contains('\u{1b}'), "{list}");
    let colour = run(&["dump", "--demo", "--width", "80", "--ansi"]);
    assert!(colour.contains("\u{1b}[0;38;2;") && colour.contains("@KB"), "{colour}");
    for w in ["80", "120"] {
        let d = run(&["dump", "--demo", "--width", w, "--height", "70", "--select", "MAIN/@KB", "--detail"]);
        assert_eq!(d.lines().count(), 70);
        assert!(d.contains("detail") && d.contains("INBOX") && d.contains("DEAD LETTERS (1)"), "{w}:\n{d}");
        let a = run(&["dump", "--demo", "--width", w, "--height", "70", "--select", "MAIN/@KB", "--detail", "--ansi"]);
        assert!(a.contains("\u{1b}[") && a.contains("INBOX"));
    }
}
