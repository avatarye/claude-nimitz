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
    // right side: `icon n / icon n`
    let band = &ls[y as usize];
    assert!(band.contains("^ 13 / * 2 / - 1"), "{band}");
    let main = ls.iter().find(|l| l.contains("[MAIN]") && l.contains("needs you")).unwrap();
    assert!(main.contains("! 2 / * 3 / - 2 / x 1"), "{main}");
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
    assert!(s.contains("fleets: needs you, busy"), "top bar lists the states shown");
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
    assert!(calls[0].starts_with("type p2 Message from the user for [MAIN.ARGOS]-BOLT"), "{calls:?}");
    assert!(calls[0].ends_with("ship it"));
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
    assert!(calls[0].starts_with("open ") && calls[0].contains("attach") && calls[0].contains("workspace: \"MAIN\""), "{calls:?}");
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
    goto(&mut t.app, "M:MAIN/KITS/DOCS");
    keys(&mut t.app, "a");
    assert!(t.app.modal.is_none());
    assert!(toast(&t.app).contains("monitor-only"), "{}", toast(&t.app));
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
