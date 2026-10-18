//! The finder (`space`, issue #5): its candidates, its ranking, where enter
//! lands, what esc leaves, and the `z` fold that took `space`'s old job.

mod common;

use admiralty::app::Modal;
use admiralty::data::Bundle;
use admiralty::finder::{candidates, Finder};
use admiralty::fixtures;
use common::*;
use nimitz_core::{Id, Status};
use ratatui::crossterm::event::KeyCode;

/// The demo world plus MAIN/ARGOS/SIGNAL and a fleet HOME with
/// HOME/GITHUB/NIMITZ_DEV: the names the issue ranks.
fn world() -> Bundle {
    let mut b = fixtures::demo();
    let argos = b.snap.fleets[0].squadrons.iter_mut().find(|s| s.id.0 == "MAIN/ARGOS").unwrap();
    let mut schema = argos.ships[0].clone();
    schema.id = Id::ship("MAIN", "ARGOS", "SIGNAL");
    schema.expected_name = schema.id.name();
    schema.status = Status::Idle;
    schema.topic = None;
    argos.ships.push(schema);
    let mut home = b.snap.fleets.iter().find(|f| f.id.0 == "LAB").unwrap().clone();
    home.id = Id::admiral("HOME");
    home.admiral.id = home.id.clone();
    home.admiral.expected_name = home.id.name();
    let sq = &mut home.squadrons[0];
    sq.id = Id::commodore("HOME", "GITHUB");
    sq.commodore.id = sq.id.clone();
    sq.commodore.expected_name = sq.id.name();
    sq.ships[0].id = Id::ship("HOME", "GITHUB", "NIMITZ_DEV");
    sq.ships[0].expected_name = sq.ships[0].id.name();
    b.snap.fleets.push(home);
    b
}

fn ranked(f: &mut Finder, q: &str) -> Vec<String> {
    f.clear();
    f.push(q);
    f.hits.iter().map(|h| f.cands[h.cand].row_id.clone()).collect()
}

fn open(app: &mut admiralty::app::App) -> &mut Finder {
    press(app, KeyCode::Char(' '));
    match app.modal.as_mut() {
        Some(Modal::Finder(f)) => f,
        _ => panic!("space opens the finder"),
    }
}

/// One line per member of every fleet, then each tab's stowaways, in the
/// order the list shows them; each line carries the path, the expected
/// name, the status word and the topic.
#[test]
fn every_member_of_every_fleet_one_line_each_in_tree_order() {
    let b = world();
    let cands = candidates(&b.snap);
    let mut want = vec![];
    for tab in admiralty::view::tabs(&b.snap) {
        for f in b.snap.fleets.iter().filter(|f| f.id.0 == tab) {
            want.extend(admiralty::view::fleet_members(f).iter().map(|m| format!("M:{}", m.id.0)));
        }
        want.extend(b.snap.stowaways.iter().filter(|s| admiralty::view::stowaway_tab(&b.snap, s) == tab).map(|s| format!("SW:{}", s.session_id)));
    }
    assert_eq!(cands.iter().map(|c| c.row_id.clone()).collect::<Vec<_>>(), want);
    let members: usize = b.snap.fleets.iter().map(|f| admiralty::view::fleet_members(f).len()).sum();
    assert_eq!(cands.len(), members + b.snap.stowaways.len());
    assert!(cands.iter().all(|c| !c.text.contains('\n')));
    let bolt = cands.iter().find(|c| c.row_id == "M:MAIN/ARGOS/BOLT").unwrap();
    assert!(bolt.text.starts_with("MAIN/ARGOS/BOLT  [MAIN.ARGOS]-BOLT  busy  Fixing the layout"), "{}", bolt.text);
    assert!(cands.iter().any(|c| c.row_id == "M:MAIN/ARGOS/SIGNAL") && cands.iter().any(|c| c.row_id == "M:HOME/GITHUB/NIMITZ_DEV"));
    let stow = cands.iter().find(|c| c.row_id.starts_with("SW:ee11ee11")).unwrap();
    assert!(stow.text.starts_with("ee11ee11  scratch-run  stowaway live"), "{}", stow.text);
}

/// Word starts score higher: `mas` ranks MAIN/ARGOS/SIGNAL first (before
/// BOLT and BOOM, whose `s` is inside a word), `hgn` HOME/GITHUB/NIMITZ_DEV.
#[test]
fn initials_rank_their_member_first() {
    let b = world();
    let mut f = Finder::new(&b.snap);
    let mas = ranked(&mut f, "mas");
    assert_eq!(mas[0], "M:MAIN/ARGOS/SIGNAL", "{mas:?}");
    let at = |id: &str| mas.iter().position(|x| x == id);
    assert!(at("M:MAIN/ARGOS/BOLT").is_none_or(|i| i > 0), "{mas:?}");
    let hgn = ranked(&mut f, "hgn");
    assert_eq!(hgn[0], "M:HOME/GITHUB/NIMITZ_DEV", "{hgn:?}");
    // the matched letters are the initials
    f.clear();
    f.push("mas");
    let h = &f.hits[0];
    let text: Vec<char> = f.cands[h.cand].text.chars().collect();
    assert_eq!(h.idx.iter().map(|i| text[*i as usize]).collect::<String>(), "MAS");
}

/// Every space-separated term must match: `argos busy` leaves the busy
/// members of ARGOS only.
#[test]
fn several_terms_must_all_match() {
    let b = world();
    let mut f = Finder::new(&b.snap);
    let got = ranked(&mut f, "argos busy");
    let want: Vec<String> = b.snap.fleets.iter().flat_map(admiralty::view::fleet_members).filter(|m| m.id.squadron() == Some("ARGOS") && m.status == Status::Busy).map(|m| format!("M:{}", m.id.0)).collect();
    assert!(!want.is_empty());
    let mut sorted = got.clone();
    sorted.sort();
    let mut w = want.clone();
    w.sort();
    assert_eq!(sorted, w, "{got:?}");
    assert!(ranked(&mut f, "argos").len() > got.len(), "one term alone matches more");
    assert!(ranked(&mut f, "argos zzzz").is_empty());
}

/// Enter puts the cursor on the member, on its own tab, through a fold and
/// the `/` filter; enter again then acts on that row.
#[test]
fn enter_lands_on_the_member_switching_tab() {
    let mut t = fixture_app(world(), false);
    tab(&mut t.app, "MAIN");
    // ANVIL's squadron folded on FORGE's tab
    tab(&mut t.app, "FORGE");
    goto(&mut t.app, "SQ:FORGE/IRON");
    press(&mut t.app, KeyCode::Char('z'));
    assert!(!t.app.rows.iter().any(|r| r.node.id == "M:FORGE/IRON/ANVIL"));
    tab(&mut t.app, "MAIN");
    let f = open(&mut t.app);
    f.push("anvil");
    assert_eq!(f.selected().unwrap().row_id, "M:FORGE/IRON/ANVIL");
    press(&mut t.app, KeyCode::Enter);
    assert!(t.app.modal.is_none(), "enter closes the finder");
    assert_eq!(t.app.tab.as_deref(), Some("FORGE"));
    assert_eq!(t.app.current().unwrap().node.id, "M:FORGE/IRON/ANVIL");
    // the other fleet's member, through a `/` filter that hides it
    t.app.filter = "BOLT".into();
    t.app.rebuild();
    let f = open(&mut t.app);
    f.push("hgn");
    press(&mut t.app, KeyCode::Enter);
    assert_eq!(t.app.current().unwrap().node.id, "M:HOME/GITHUB/NIMITZ_DEV");
    assert_eq!(t.app.tab.as_deref(), Some("HOME"));
    // a stowaway, by its short id; ctrl-n / ctrl-p and the arrows move
    let f = open(&mut t.app);
    f.push("ee11");
    assert_eq!(f.hits.len(), 1);
    press(&mut t.app, KeyCode::Down);
    ctrl(&mut t.app, 'p');
    ctrl(&mut t.app, 'n');
    press(&mut t.app, KeyCode::Up);
    press(&mut t.app, KeyCode::Enter);
    assert!(t.app.current().unwrap().node.id.starts_with("SW:ee11ee11"), "{}", t.app.current().unwrap().node.id);
    // enter again: the row's own enter (a demo jump says what it would do)
    press(&mut t.app, KeyCode::Enter);
    assert!(toast(&t.app).contains("would focus pane p5"), "{}", toast(&t.app));
}

/// Esc closes the finder and changes nothing: cursor, tab, filter.
#[test]
fn esc_leaves_cursor_tab_and_filter_unchanged() {
    let mut t = fixture_app(world(), false);
    tab(&mut t.app, "MAIN");
    t.app.filter = "BS".into();
    t.app.rebuild();
    goto(&mut t.app, "M:MAIN/ARGOS/BOOM");
    let (cursor, tab_was, rows) = (t.app.cursor, t.app.tab.clone(), t.app.rows.len());
    let f = open(&mut t.app);
    f.push("anv");
    press(&mut t.app, KeyCode::Down);
    keys(&mut t.app, "il");
    press(&mut t.app, KeyCode::Backspace);
    ctrl(&mut t.app, 'u');
    match &t.app.modal {
        Some(Modal::Finder(f)) => assert!(f.query.is_empty(), "ctrl-u clears: {}", f.query),
        _ => panic!("still open"),
    }
    press(&mut t.app, KeyCode::Esc);
    assert!(t.app.modal.is_none());
    assert_eq!((t.app.cursor, t.app.tab.clone(), t.app.filter.as_str(), t.app.rows.len()), (cursor, tab_was, "BS", rows));
}

/// `space` opens the finder; `z` folds what space folded (a squadron);
/// the `?` panel lists both.
#[test]
fn space_finds_z_folds_and_the_help_says_so() {
    let mut t = demo();
    goto(&mut t.app, "SQ:MAIN/ARGOS");
    let n = t.app.rows.len();
    press(&mut t.app, KeyCode::Char(' '));
    assert!(matches!(t.app.modal, Some(Modal::Finder(_))));
    assert_eq!(t.app.rows.len(), n, "space folds nothing now");
    press(&mut t.app, KeyCode::Esc);
    press(&mut t.app, KeyCode::Char('z'));
    assert!(t.app.rows.len() < n);
    press(&mut t.app, KeyCode::Char('z'));
    assert_eq!(t.app.rows.len(), n);
    let help = admiralty::app::HELP;
    assert!(help.iter().any(|(k, d)| *k == "space" && d.contains("find")), "{help:?}");
    assert!(help.iter().any(|(k, d)| *k == "z" && d.contains("fold / unfold") && d.contains("+N done")), "{help:?}");
    let mut t = demo();
    press(&mut t.app, KeyCode::Char('?'));
    let s = screen(&mut t.app, 200, 80);
    assert!(s.contains("space") && s.contains("find a member"), "{s}");
}

/// The panel: the query, `hits / all`, the matched letters lit, and the
/// selected member's detail beside it on a wide screen; on a narrow one the
/// panel alone. Monitor-only works the same.
#[test]
fn the_panel_shows_the_ranked_list_and_previews_the_selection() {
    for b in [world(), { let mut b = world(); b.panes.clear(); b.mux_caps = Default::default(); b }] {
        let mut t = fixture_app(b, false);
        let f = open(&mut t.app);
        f.push("mas");
        let all = f.cands.len();
        let buf = draw(&mut t.app, 200, 50);
        let ls = lines(&buf);
        let s = ls.join("\n");
        assert!(s.contains("> mas"), "{s}");
        assert!(s.contains(&format!("/ {all}")) && s.contains("enter row") && s.contains("ctrl-o pane"), "{s}");
        let y = ls.iter().position(|l| l.contains("MAIN/ARGOS/SIGNAL  [MAIN.ARGOS]-SIGNAL")).expect("the best line");
        let x = ls[y].find("MAIN/ARGOS/SIGNAL").unwrap();
        let x = ls[y][..x].chars().count() as u16;
        assert_eq!(buf[(x, y as u16)].fg, admiralty::theme::mono::ACCENT, "M is lit");
        assert_ne!(buf[(x + 1, y as u16)].fg, admiralty::theme::mono::ACCENT, "A of MAIN is not");
        assert!(s.contains("identity    MAIN/ARGOS/SIGNAL"), "the preview:\n{s}");
        let s = screen(&mut t.app, 100, 30);
        assert!(s.contains("> mas") && !s.contains("identity    MAIN/ARGOS/SIGNAL"), "{s}");
    }
}

/// A row hidden only by a fold, the state filter or hidden stowaways is
/// shown without clearing the `/` filter that does not hide it (review repro).
#[test]
fn enter_keeps_a_filter_that_does_not_hide_the_row() {
    let mut t = fixture_app(world(), false);
    tab(&mut t.app, "MAIN");
    t.app.filter = "BS".into();
    t.app.rebuild();
    goto(&mut t.app, "SQ:MAIN/ARGOS");
    press(&mut t.app, KeyCode::Char('z'));
    assert!(!t.app.rows.iter().any(|r| r.node.id == "M:MAIN/ARGOS/BOLT"), "folded");
    open(&mut t.app).push("argos/bolt");
    press(&mut t.app, KeyCode::Enter);
    assert_eq!(t.app.current().unwrap().node.id, "M:MAIN/ARGOS/BOLT");
    assert_eq!(t.app.filter, "BS", "the filter stays");
    // the state filter hides ARGOS
    let b = t.app.bundle.clone().unwrap();
    let f = &b.snap.fleets[0];
    let sq = f.squadrons.iter().find(|s| s.id.0 == "MAIN/ARGOS").unwrap();
    let i = admiralty::view::FleetState::of_squadron(f, sq).index();
    t.app.fleet_states[i] = false;
    t.app.rebuild();
    assert!(!t.app.rows.iter().any(|r| r.node.id == "M:MAIN/ARGOS/BOOM"), "hidden by state");
    open(&mut t.app).push("argos/boom");
    press(&mut t.app, KeyCode::Enter);
    assert_eq!(t.app.current().unwrap().node.id, "M:MAIN/ARGOS/BOOM");
    assert!(t.app.fleet_states[i] && t.app.filter == "BS");
    // hidden stowaways (no filter: a search never lists stowaways)
    t.app.filter.clear();
    t.app.flagged_only = true;
    t.app.show_stowaways = false;
    t.app.rebuild();
    open(&mut t.app).push("ee11");
    press(&mut t.app, KeyCode::Enter);
    assert!(t.app.current().unwrap().node.id.starts_with("SW:ee11ee11"));
    assert!(t.app.show_stowaways);
}

/// One letter is no acronym: `a` does not put the stowaway `ab99ab99` (one
/// initial, all of it) above the members; a half typed `!` hides nothing;
/// `!busy` leaves out the busy ones.
#[test]
fn one_letter_and_operators() {
    let b = world();
    let mut f = Finder::new(&b.snap);
    let a = ranked(&mut f, "a");
    assert!(!a[0].starts_with("SW:"), "{a:?}");
    let all = f.cands.len();
    assert_eq!(ranked(&mut f, "!").len(), all, "a lone ! is not yet a term");
    assert_eq!(ranked(&mut f, "argos !").len(), ranked(&mut f, "argos").len());
    let not_busy = ranked(&mut f, "argos !busy");
    assert!(!not_busy.is_empty() && not_busy.iter().all(|id| b.snap.fleets.iter().flat_map(admiralty::view::fleet_members).any(|m| format!("M:{}", m.id.0) == *id && m.status != Status::Busy)), "{not_busy:?}");
}

/// A line with non-ASCII text lights no fzf letters (nucleo counts it in
/// graphemes); the path's initials still light. A long query shows its end;
/// a short screen keeps the panel off the status rows.
#[test]
fn non_ascii_lines_long_queries_and_short_screens() {
    let mut b = world();
    b.snap.fleets[0].admiral.topic = Some("caf\u{e9} \u{1f680} weekly".into());
    let mut f = Finder::new(&b.snap);
    f.push("weekly");
    let h = f.hits.iter().find(|h| f.cands[h.cand].row_id == "M:MAIN").unwrap();
    assert!(h.idx.is_empty(), "{:?}", h.idx);
    let mut t = fixture_app(b, false);
    let fd = open(&mut t.app);
    fd.push(&"x".repeat(300));
    fd.push("END");
    let s = screen(&mut t.app, 200, 50);
    assert!(s.contains("xxEND\u{2588}"), "{s}");
    for h in [6u16, 8, 10] {
        let ls = lines(&draw(&mut t.app, 200, h));
        assert!(ls[h as usize - 2].contains("ADMIRALTY") && ls[h as usize - 1].contains("Nimitz"), "{h} rows:\n{}", ls.join("\n"));
    }
}
