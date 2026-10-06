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

#[test]
fn each_fleet_is_a_gold_double_frame_with_squadron_frames_inside() {
    let mut t = demo();
    // one fleet per tab
    for fleet in ["MAIN", "FORGE", "LAB"] {
        tab(&mut t.app, fleet);
        let buf = draw(&mut t.app, 200, 60);
        let ls = lines(&buf);
        let tag = format!("[{fleet}]");
        let y = ls.iter().position(|l| l.contains(&tag) && l.starts_with('\u{2554}')).unwrap_or_else(|| panic!("{tag} band"));
        // theme A: frames are neutral, except one that needs you or does not load
        let frame = match fleet {
            "MAIN" => mono::NEEDS,
            "FORGE" => mono::ERROR,
            _ => mono::LINE,
        };
        assert_eq!(buf[(0, y as u16)].fg, frame, "{tag} frame");
        assert_eq!(ls.iter().filter(|l| l.starts_with('\u{2554}')).count(), 1, "only {tag} on its tab");
    }
    tab(&mut t.app, "MAIN");
    let buf = draw(&mut t.app, 200, 60);
    let ls = lines(&buf);
    // squadron frames open one cell in, inside the fleet's double line
    let argos = ls.iter().find(|l| l.contains("ARGOS") && l.contains("comms")).unwrap();
    assert!(argos.starts_with("\u{2551}\u{256d}"), "{argos}");
    // and close before the fleet does
    assert!(ls.iter().any(|l| l.starts_with("\u{2551}\u{2570}")));
    assert!(ls.iter().any(|l| l.starts_with('\u{255a}')));
    // stowaways: their own frame at top level, neutral like the rest
    let y = ls.iter().position(|l| l.contains("STOWAWAYS")).unwrap();
    assert!(ls[y].starts_with('\u{256d}'));
    assert_eq!(buf[(0, y as u16)].fg, mono::LINE);
}

#[test]
fn title_band_is_tinted_by_state_with_counts_on_the_right() {
    let mut t = demo();
    tab(&mut t.app, "FORGE");
    keys(&mut t.app, "j"); // move the cursor off the first band so its tint shows
    let buf = draw(&mut t.app, 200, 60);
    let ls = lines(&buf);
    let y = ls.iter().position(|l| l.contains("[FORGE]") && l.contains("broken")).unwrap() as u16;
    let x = (ls[y as usize].find("broken").unwrap()) as u16; // ASCII before it
    assert_eq!(buf[(x - 2, y)].bg, mono::BG_HI, "no tinted bands: every band on the raised background");
    assert_eq!(buf[(x, y)].fg, mono::ERROR, "the state word carries the hue");
    // right side: `icon n`, slashes between, right-aligned as a group
    let band = &ls[y as usize];
    assert!(band.contains("^10 / ^3 / *2 / -1 / x2 / o3 / ?1"), "{band}");
    tab(&mut t.app, "MAIN");
    let ls = lines(&draw(&mut t.app, 200, 60));
    let main = ls.iter().find(|l| l.contains("[MAIN]") && l.contains("needs you")).unwrap();
    assert!(main.contains("!2 / *3 / -2 / x1"), "needs-you first: {main}");
}

#[test]
fn band_counts_are_a_right_aligned_group_and_narrow_bands_keep_the_key_ones() {
    let mut t = demo();
    let main_ls = lines(&draw(&mut t.app, 200, 60));
    tab(&mut t.app, "FORGE");
    let forge_ls = lines(&draw(&mut t.app, 200, 60));
    // every band's group ends at the same column (nested bands too, other tabs too)
    let end = |ls: &[String], has: &str| {
        let l = ls.iter().find(|l| l.contains(has)).unwrap();
        let c: Vec<char> = l.chars().collect();
        let corner = c.iter().rposition(|ch| "\u{2557}\u{256e}".contains(*ch)).unwrap();
        c[..corner].iter().rposition(|ch| *ch != ' ').unwrap()
    };
    assert_eq!(end(&main_ls, "[MAIN]   needs you"), end(&forge_ls, "[FORGE]   broken"));
    assert_eq!(end(&main_ls, "[MAIN]   needs you"), end(&main_ls, "ARGOS   busy"));
    // under 100 columns: needs-you and errors only
    let forge_ls = lines(&draw(&mut t.app, 90, 40));
    tab(&mut t.app, "MAIN");
    let ls = lines(&draw(&mut t.app, 90, 40));
    let main = ls.iter().find(|l| l.contains("[MAIN]")).unwrap();
    assert!(main.contains("!2") && !main.contains("*3"), "narrow: {main}");
    let forge = forge_ls.iter().find(|l| l.contains("[FORGE]")).unwrap();
    assert!(forge.contains("^10") && !forge.contains("^3"), "narrow: errors only: {forge}");
}

#[test]
fn columns_line_up_across_fleet_squadron_and_stowaway_frames() {
    let mut t = demo();
    let ls = lines(&draw(&mut t.app, 200, 60));
    let header = col_of(&ls, "STATUS", "STATUS");
    let admiral = col_of(&ls, "[MAIN]-ADMIRAL", "- idle");
    let ship = col_of(&ls, "[MAIN.ARGOS]-BOOM", "- idle");
    tab(&mut t.app, "FORGE");
    let forge = lines(&draw(&mut t.app, 200, 60));
    let stow = col_of(&forge, "scratch-run", "* live");
    let forge_ship = col_of(&forge, "[FORGE.MOLD]-CAST", "- idle");
    assert_eq!(header, admiral);
    assert_eq!(admiral, ship);
    assert_eq!(ship, stow);
    assert_eq!(ship, forge_ship);
    let kind_admiral = col_of(&ls, "[MAIN]-ADMIRAL", "admiral ");
    let kind_ship = col_of(&ls, "[MAIN.ARGOS]-BOOM", "ship ");
    assert_eq!(kind_admiral, kind_ship);
}

#[test]
fn column_widths_grow_to_their_maxima_and_keep_detail_on_narrow_screens() {
    use admiralty::frames::{col_widths, COL_MAX, COL_MIN};
    let narrow = col_widths(96);
    assert!(narrow[4] >= 20, "DETAIL keeps room at 100 columns: {narrow:?}");
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
    assert!(!t.app.rows.iter().any(|r| r.node.name == "[LAB.OPTICS]-LENS"));
    let s = screen(&mut t.app, 200, 60);
    assert!(s.contains("elsewhere   not on this machine"));
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
    let band = ls.iter().find(|l| l.contains("[MAIN]") && l.contains("needs you")).unwrap();
    assert!(band.trim_end().chars().count() > 190, "the list spans the window: {band}");
    // `w` floats it over the right part; the list keeps its rows
    keys(&mut t.app, "w");
    let ls = lines(&draw(&mut t.app, 200, 50));
    let y = ls.iter().position(|l| l.contains(" detail ") && l.contains("w/esc close")).expect("the panel's title");
    assert!(y <= 2, "it starts at the top of the body");
    assert!(ls[y].find(" detail ").unwrap() > 100, "on the right: {}", ls[y]);
    assert!(ls.iter().any(|l| l.contains("[MAIN]") && l.contains("needs you")), "the list is still there");
    // it follows the cursor row
    goto(&mut t.app, "M:MAIN/ARGOS/BOOM");
    assert!(screen(&mut t.app, 200, 50).contains("[MAIN.ARGOS]-BOOM"));
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
    // a row with no pane: says why, does not quit
    goto(&mut t.app, "M:MAIN/ARGOS/BOOM");
    assert!(press(&mut t.app, KeyCode::Enter).is_none());
    assert!(toast(&t.app).contains("no pane hosts this session"), "{}", toast(&t.app));
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
    let s = screen(&mut t.app, 200, 80);
    for what in ["errors (orange) and warnings (yellow)", "needs you: waiting", "busy", "stowaway: a session", "pane dot: idle", "Band counts read"] {
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
        assert!(foot.contains("q quit"), "{w}: {foot}");
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
        "role.background",
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
        ("[FORGE.IRON]-ANVIL-2", "dup.live"),
        ("[FORGE.IRON]-ANVIL-2", "name.drift"),
        ("[FORGE.IRON]-COMMODORE ", "labels.missing"),
        ("[FORGE.IRON]-HAMMER", "copy.rows"),
        ("[FORGE.IRON]-HAMMER", "lineage.broken"),
        ("[FORGE]-ADMIRAL", "role.background"),
    ] {
        let l = ls.iter().find(|l| l.contains(row)).unwrap_or_else(|| panic!("{row}"));
        assert!(l.contains(code), "{row} shows {code}: {l}");
        let y = ls.iter().position(|x| x == l).unwrap() as u16;
        let x = col_of(&ls, row, "^ ") as u16; // the issue mark before the name
        assert!(x < col_of(&ls, row, row.trim()) as u16);
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
    assert!(s.contains("10 errors"), "top bar");
}

#[test]
fn pane_dots_at_row_start_in_herdr_colours() {
    let mut t = demo();
    for (fleet, row, dot, color) in [
        // the pane dot reuses the status hues: idle and done neutral, working busy
        ("MAIN", "[MAIN]-ADMIRAL", "\u{25cb}", mono::FG),
        ("MAIN", "[MAIN.ARGOS]-COMMODORE", "\u{25cf}", mono::BUSY),
        ("FORGE", "[FORGE]-ADMIRAL", "\u{25cf}", mono::MUTED),
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
    let y = ls.iter().position(|l| l.contains("[MAIN.KITS]-COMMODORE")).unwrap();
    assert_eq!(buf[(2, y as u16)].fg, mono::NEEDS);
    assert!(ls[y].contains("needs you"));
    let boom = ls.iter().find(|l| l.contains("[MAIN.ARGOS]-BOOM")).unwrap();
    assert!(boom.starts_with("\u{2551}\u{2502}  "), "no pane: no dot: {boom}");
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
    keys(&mut t.app, "A");
    let Some(Modal::Confirm(p)) = t.app.modal.take() else { panic!("{}", toast(&t.app)) };
    assert_eq!(p.command, "nimitz open MAIN/ARGOS/BOLT");
    assert_eq!(p.panes.len(), 1);
    assert!(p.panes[0].0.cmd.contains(&"attach".to_string()));
    t.app.begin(&p).unwrap();
    let calls = t.mux.calls.lock().unwrap().clone();
    assert!(calls[0].starts_with("open ") && calls[0].contains("attach") && calls[0].contains("workspace: \"MAIN.ARGOS\"") && calls[0].contains("label: \"MAIN.ARGOS.BOLT\""), "{calls:?}");
    // `A` again: its tab MAIN.ARGOS.BOLT is open, so jump there and open nothing
    keys(&mut t.app, "A");
    assert!(t.app.modal.is_none(), "no confirm: nothing would be created");
    let calls = t.mux.calls.lock().unwrap().clone();
    assert_eq!(calls.iter().filter(|c| c.starts_with("open ")).count(), 1, "{calls:?}");
    assert_eq!(calls.last().map(String::as_str), Some("focus new"), "{calls:?}");
    assert!(toast(&t.app).contains("Open already"), "{}", toast(&t.app));
    // an interactive lead is not attached; it lives in its terminal
    goto(&mut t.app, "M:MAIN/ARGOS");
    assert!(t.app.available(Act::Open).unwrap_err().contains("interactive"));
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
    // a commodore does: refused, with the reason
    goto(&mut t.app, "M:FORGE/MOLD");
    keys(&mut t.app, "a");
    assert!(t.app.modal.is_none());
    assert!(toast(&t.app).contains("monitor-only"), "{}", toast(&t.app));
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
    let x = col_of(&ls, "[KITS]-LEAD", "L ");
    assert!(x < col_of(&ls, "[KITS]-LEAD", "[KITS]-LEAD"), "marker before the name: {}", ls[y]);
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
fn tender_layout_and_machine_entries_are_where_they_belong() {
    let mut t = demo();
    goto(&mut t.app, "SQ:MAIN/ARGOS");
    let menu = t.app.create_menu();
    let it = menu.iter().find(|i| i.key == 't').unwrap();
    assert_eq!(it.label, "new tender for [MAIN.ARGOS]-COMMODORE");
    assert!(it.enabled.is_ok());
    goto(&mut t.app, "F:MAIN");
    assert!(t.app.create_menu().iter().find(|i| i.key == 't').unwrap().enabled.is_err());
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
    assert!(ls[23].contains("? help") && ls[23].contains("q quit"), "footer keeps help and quit: {}", ls[23]);
    let row = ls.iter().find(|l| l.contains("[FORGE.IRON]-AN") && l.contains("ship")).unwrap_or_else(|| panic!("{}", ls.join("\n")));
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
    assert!(top80.contains("6 warn") || top80.contains("6w"), "80 columns keep the warning count: {top80}");
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
    for name in ["[MAIN]-ADMIRAL", "[MAIN.ARGOS]-COMMODORE"] {
        let l = ls.iter().find(|l| l.contains(name)).unwrap();
        let age = &l[l.find(name).unwrap()..];
        assert!(age.contains(" ? "), "{name}: AGE shows ?: {l}");
    }
    // no session at all: the cell stays empty
    tab(&mut t.app, "FORGE");
    let ls = lines(&draw(&mut t.app, 200, 60));
    let l = ls.iter().find(|l| l.contains("[FORGE.IRON]-QUENCH")).unwrap();
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
