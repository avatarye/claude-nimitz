//! TUI tests on fixtures: frames, alignment, several fleets, widths, splits,
//! keys, modals, forms, monitor-only mode, issue visibility, pane dots.

mod common;

use admiralty::actions::{self, Act, Target};
use admiralty::app::{Effect, Modal, Split};
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
    let buf = draw(&mut t.app, 200, 60);
    let ls = lines(&buf);
    for tag in ["[MAIN]", "[FORGE]", "[LAB]"] {
        let y = ls.iter().position(|l| l.contains(tag) && l.starts_with('\u{2554}')).unwrap_or_else(|| panic!("{tag} band"));
        let gold = if tag == "[LAB]" { mono::GOLD_DIM } else { mono::GOLD };
        assert_eq!(buf[(0, y as u16)].fg, gold, "{tag} frame is gold (dimmed when it runs elsewhere)");
    }
    // squadron frames open one cell in, inside the fleet's double line
    let argos = ls.iter().find(|l| l.contains("ARGOS") && l.contains("comms")).unwrap();
    assert!(argos.starts_with("\u{2551}\u{256d}"), "{argos}");
    // and close before the fleet does
    assert!(ls.iter().any(|l| l.starts_with("\u{2551}\u{2570}")));
    assert!(ls.iter().any(|l| l.starts_with('\u{255a}')));
    // stowaways: their own pink frame at top level
    let y = ls.iter().position(|l| l.contains("STOWAWAYS")).unwrap();
    assert!(ls[y].starts_with('\u{256d}'));
    assert_eq!(buf[(0, y as u16)].fg, mono::PINK);
}

#[test]
fn title_band_is_tinted_by_state_with_counts_on_the_right() {
    let mut t = demo();
    keys(&mut t.app, "j"); // move the cursor off the first band so its tint shows
    let buf = draw(&mut t.app, 200, 60);
    let ls = lines(&buf);
    let y = ls.iter().position(|l| l.contains("[FORGE]") && l.contains("broken")).unwrap() as u16;
    let x = (ls[y as usize].find("broken").unwrap()) as u16; // ASCII before it
    assert_eq!(buf[(x - 2, y)].bg, mono::T_PURPLE, "broken fleet band is purple-tinted");
    // right side: `icon n`, slashes between, right-aligned as a group
    let band = &ls[y as usize];
    assert!(band.contains("^10 / ^3 / *2 / -1 / x2 / o3 / ?1"), "{band}");
    let main = ls.iter().find(|l| l.contains("[MAIN]") && l.contains("needs you")).unwrap();
    assert!(main.contains("!2 / *3 / -2 / x1"), "needs-you first: {main}");
}

#[test]
fn band_counts_are_a_right_aligned_group_and_narrow_bands_keep_the_key_ones() {
    let mut t = demo();
    let ls = lines(&draw(&mut t.app, 200, 60));
    // every band's group ends at the same column (nested bands too)
    let end = |has: &str| {
        let l = ls.iter().find(|l| l.contains(has)).unwrap();
        let c: Vec<char> = l.chars().take(130).collect();
        let corner = c.iter().rposition(|ch| "\u{2557}\u{256e}".contains(*ch)).unwrap();
        c[..corner].iter().rposition(|ch| *ch != ' ').unwrap()
    };
    assert_eq!(end("[MAIN]   needs you"), end("[FORGE]   broken"));
    assert_eq!(end("[MAIN]   needs you"), end("ARGOS   busy"));
    // under 100 columns: needs-you and errors only
    let ls = lines(&draw(&mut t.app, 90, 40));
    let main = ls.iter().find(|l| l.contains("[MAIN]")).unwrap();
    assert!(main.contains("!2") && !main.contains("*3"), "narrow: {main}");
    let forge = ls.iter().find(|l| l.contains("[FORGE]")).unwrap();
    assert!(forge.contains("^10") && !forge.contains("^3"), "narrow: errors only: {forge}");
}

#[test]
fn columns_line_up_across_fleet_squadron_and_stowaway_frames() {
    let mut t = demo();
    let ls = lines(&draw(&mut t.app, 200, 60));
    let header = col_of(&ls, "STATUS", "STATUS");
    let admiral = col_of(&ls, "[MAIN]-ADMIRAL", "- idle");
    let ship = col_of(&ls, "[MAIN.ARGOS]-BOOM", "- idle");
    let stow = col_of(&ls, "scratch-run", "* live");
    let forge_ship = col_of(&ls, "[FORGE.MOLD]-CAST", "- idle");
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
    let bands: Vec<String> = t.app.rows.iter().filter(|r| r.node.kind == Kind::FleetBand).map(|r| r.node.name.clone()).collect();
    assert_eq!(bands, ["[MAIN]", "[FORGE]", "[LAB]"]);
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
        let s = screen(&mut t.app, w, h);
        assert!(s.contains("ADMIRALTY"), "{w}x{h}");
        assert!(s.contains("NAME"), "{w}x{h}");
        assert!(s.contains("detail"), "{w}x{h}");
    }
}

#[test]
fn split_is_auto_by_width_and_v_s_switch_it_and_it_sticks() {
    let mut t = demo();
    let ls = lines(&draw(&mut t.app, 200, 50));
    let y_wide = ls.iter().position(|l| l.contains(" detail ")).unwrap();
    assert!(y_wide <= 2, "wide: detail beside the list");
    let ls = lines(&draw(&mut t.app, 100, 40));
    let y_narrow = ls.iter().position(|l| l.contains(" detail ")).unwrap();
    assert!(y_narrow > 10, "narrow: detail below the list");
    keys(&mut t.app, "v");
    assert_eq!(t.app.split, Split::Vertical);
    let ls = lines(&draw(&mut t.app, 100, 40));
    assert!(ls.iter().position(|l| l.contains(" detail ")).unwrap() <= 2);
    keys(&mut t.app, "s");
    assert_eq!(t.app.split, Split::Horizontal);
    // folds, filters and split are saved and come back
    keys(&mut t.app, "F");
    let state = t.dir.path().join("ui.json");
    let saved = std::fs::read_to_string(&state).unwrap();
    assert!(saved.contains("horizontal") && saved.contains("\"flagged_only\": true"), "{saved}");
    let ctx = admiralty::actions::Ctx { env: fixtures::demo_env(), mux: Box::new(TestMux::default()), live: false, exit_after_jump: false };
    let again = admiralty::app::App::new(ctx, Some(state), true);
    assert_eq!(again.split, Split::Horizontal);
    assert!(again.flagged_only);
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
    let ids: Vec<String> = t.app.rows.iter().filter(|r| matches!(r.node.kind, Kind::Member(_))).map(|r| r.node.id.clone()).collect();
    assert!(ids.contains(&"M:MAIN/KITS/PIPELINE".to_string()), "needs you is flagged");
    assert!(ids.contains(&"M:FORGE/IRON/ANVIL".to_string()), "an issue is flagged");
    assert!(!ids.contains(&"M:MAIN/ARGOS/BOOM".to_string()), "a clean idle ship is not");
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
    assert!(s.contains("filtered: no broken (f clears)"), "top bar says what the filter hides:\n{}", s.lines().next().unwrap());
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
    for what in ["errors (red) and warnings", "needs you: waiting", "busy", "stowaway: a session", "pane dot: idle", "Band counts read"] {
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
    for k in ["gg / G", "ctrl-d / ctrl-u", "J / K", "c / e / D", "v / s", "f / F"] {
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
        assert!(c == mono::PINK || c == mono::ORANGE, "{row} mark is coloured by severity");
    }
    goto(&mut t.app, "M:FORGE/IRON/ANVIL");
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
    let buf = draw(&mut t.app, 200, 60);
    let ls = lines(&buf);
    for (row, dot, color) in [
        ("[MAIN]-ADMIRAL", "\u{25cb}", mono::GREEN),
        ("[MAIN.ARGOS]-COMMODORE", "\u{25cf}", mono::YELLOW),
        ("[FORGE]-ADMIRAL", "\u{25cf}", mono::CYAN),
        ("scratch-run", "\u{00b7}", mono::COMMENT),
    ] {
        let y = ls.iter().position(|l| l.contains(row)).unwrap();
        assert_eq!(buf[(2, y as u16)].symbol(), dot, "{row}");
        assert_eq!(buf[(2, y as u16)].fg, color, "{row}");
    }
    // KITS commodore's pane is blocked: needs you, and the dot is red
    let y = ls.iter().position(|l| l.contains("[MAIN.KITS]-COMMODORE")).unwrap();
    assert_eq!(buf[(2, y as u16)].fg, mono::PINK);
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
    assert!(calls[0].starts_with("open ") && calls[0].contains("attach") && calls[0].contains("workspace: \"MAIN.ARGOS\"") && calls[0].contains("label: \"BOLT\""), "{calls:?}");
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
    assert_eq!(buf[(x as u16, y as u16)].fg, mono::ORANGE);
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
    assert!(ls[0].contains("MONITOR-ONLY"), "header keeps the mux state: {}", ls[0]);
    assert!(ls[23].contains("? help") && ls[23].contains("q quit"), "footer keeps help and quit: {}", ls[23]);
    let row = ls.iter().find(|l| l.contains("[FORGE.IRON]-AN") && l.contains("ship")).unwrap_or_else(|| panic!("{}", ls.join("\n")));
    assert!(row.contains("name.drift"), "DETAIL keeps the issue code: {row}");
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
        let top = lines(&draw(&mut t.app, w, 24))[0].clone();
        assert!(!top.contains("warni ") && !top.trim_end().ends_with("warni"), "{w}: {top}");
        let words: Vec<&str> = top.split_whitespace().collect();
        let last = words.last().copied().unwrap_or("");
        assert!(
            ["warnings", "warn", "(i)", "range", "unsupported", "CC!"].iter().any(|x| last.ends_with(x)) || last.ends_with('w') || last.ends_with('e') || last.ends_with('!'),
            "{w}: the header ends on a whole piece, not a cut word: {top:?}"
        );
    }
    let mut t = demo();
    let top80 = lines(&draw(&mut t.app, 80, 24))[0].clone();
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
        let top = lines(&draw(&mut again, w, 24))[0].clone();
        assert!(top.contains("filtered:") || top.contains("FILTERED"), "{w}: no filter marker: {top}");
        assert!(top.contains("esc") && top.contains('F'), "{w}: the marker says how to clear it: {top}");
    }
    let top = lines(&draw(&mut again, 200, 24))[0].clone();
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
    let l = ls.iter().find(|l| l.contains("[FORGE.IRON]-QUENCH")).unwrap();
    assert!(!l.contains(" ? "), "{l}");
}
