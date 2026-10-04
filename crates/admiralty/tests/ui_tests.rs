mod common;

use admiralty::herdr;
use admiralty::ui::{col_widths, mono, table_max_width, App, Effect, Modal, Split};
use common::*;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

fn app(sb: &Sb) -> App {
    let mut a = App::new(sb.env.clone(), false);
    a.apply(snap(sb), herdr::list_panes(sb.env.herdr_bin.as_deref()));
    a
}

fn key(a: &mut App, c: char) -> Option<Effect> {
    a.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}
fn code(a: &mut App, c: KeyCode) -> Option<Effect> {
    a.on_key(KeyEvent::new(c, KeyModifiers::NONE))
}
/// ids of the fleet and OTHER frames (the hub frame is always first)
fn top_ids(a: &App) -> Vec<String> {
    a.rows.iter().filter(|r| r.depth == 0 && (r.node.id.starts_with("f:") || r.node.id == "others")).map(|r| r.node.id.clone()).collect()
}
fn render(a: &mut App) -> String {
    let mut t = Terminal::new(TestBackend::new(200, 50)).unwrap();
    t.draw(|f| a.draw(f)).unwrap();
    let buf = t.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn tree_header_and_columns_render() {
    let sb = sandbox();
    let mut a = app(&sb);
    assert_eq!(a.rows[0].node.id, "hub");
    assert_eq!(a.rows[0].depth, 0);
    assert_eq!(top_ids(&a), ["f:ALPHA", "f:BRAVO", "f:CHARLIE", "f:DELTA", "f:ECHO"]);
    let screen = render(&mut a);
    assert!(!screen.lines().any(|l| l.contains(" ID ")), "ID column is gone");
    assert!(screen.contains("1 need you") && screen.contains("hub live"));
    assert!(screen.contains("NAME") && screen.contains("STATUS") && screen.contains("DETAIL"));
    // each fleet is a frame: its name and status on the top border, rows inside
    let title = screen.lines().find(|l| l.contains("[ALPHA]") && l.starts_with('╭')).unwrap();
    assert!(title.contains('╮'), "the title bar spans the frame");
    // counts on the right (declared agents + lead): needs-you 1 / busy 2 / not running 2
    let nerd = admiralty::ui::NERD;
    for (icon, n) in [(nerd.s_needs, 1), (nerd.s_busy, 2), (nerd.s_off, 2)] {
        assert!(title.contains(&format!("{icon} {n}")), "{icon} {n} in {title}");
    }
    // the hub is a double-lined frame, always boxed
    assert!(screen.lines().any(|l| l.starts_with('╔') && l.contains("[ADMIRALTY]")));
    assert!(screen.lines().any(|l| l.starts_with('╚')));
    let col = |l: &str, needle: &str| l[..l.find(needle).unwrap()].chars().count();
    let header = screen.lines().find(|l| l.contains("STATUS")).unwrap();
    let lead = screen.lines().find(|l| l.contains("[ALPHA]-LEAD") && l.contains("* busy")).unwrap();
    let twin = screen.lines().find(|l| l.contains("[BRAVO]-TWIN") && l.contains("* busy")).unwrap();
    assert!(lead.starts_with('│') && twin.starts_with('│'));
    // STATUS lines up with its header in every frame
    assert_eq!(col(header, "STATUS"), col(lead, "* busy"));
    assert_eq!(col(lead, "* busy"), col(twin, "* busy"));
    // folded or empty sections are a single rule line, no box
    let charlie = screen.lines().find(|l| l.contains("[CHARLIE]")).unwrap();
    assert!(!charlie.starts_with('╭') && !charlie.starts_with('│'));
}

#[test]
fn vim_motions() {
    let sb = sandbox();
    let mut a = app(&sb);
    key(&mut a, 'j');
    key(&mut a, 'j');
    assert_eq!(a.cursor, 2);
    key(&mut a, 'k');
    assert_eq!(a.cursor, 1);
    key(&mut a, 'G');
    assert_eq!(a.cursor, a.rows.len() - 1);
    key(&mut a, 'g');
    key(&mut a, 'g');
    assert_eq!(a.current_id().unwrap(), "hub");
    key(&mut a, 'j');
    assert_eq!(a.current_id().unwrap(), "f:ALPHA");
    key(&mut a, 'h');
    assert!(!a.current().unwrap().expanded);
    key(&mut a, 'l');
    assert!(a.current().unwrap().expanded);
    key(&mut a, 'l');
    assert_eq!(a.current_id().unwrap(), "a:ALPHA/LEAD");
    key(&mut a, 'h'); // fold LEAD
    key(&mut a, 'h'); // to parent
    assert_eq!(a.current_id().unwrap(), "f:ALPHA");
}

#[test]
fn toggles_filter_and_n() {
    let sb = sandbox();
    let mut a = app(&sb);
    key(&mut a, 'I');
    assert_eq!(top_ids(&a).last().unwrap(), "others");
    key(&mut a, 'F');
    assert!(!top_ids(&a).contains(&"f:CHARLIE".to_string()) && top_ids(&a).contains(&"f:ALPHA".to_string()));
    key(&mut a, 'F');
    key(&mut a, 'I');
    key(&mut a, '/');
    for c in "lead".chars() {
        key(&mut a, c);
    }
    code(&mut a, KeyCode::Enter);
    key(&mut a, 'n');
    let first = a.current_id().unwrap();
    key(&mut a, 'n');
    let second = a.current_id().unwrap();
    let mut both = vec![first.clone(), second];
    both.sort();
    assert_eq!(both, ["a:ALPHA/LEAD", "a:BRAVO/LEAD"]);
    key(&mut a, 'N');
    assert_eq!(a.current_id().unwrap(), first);
    code(&mut a, KeyCode::Esc);
    assert!(a.filter.is_empty());
}

#[test]
fn needs_modal_jumps_to_agent() {
    let sb = sandbox();
    let mut a = app(&sb);
    key(&mut a, '!');
    let Some(Modal::Needs { items, .. }) = &a.modal else { panic!("no needs modal") };
    assert_eq!(items.len(), 2); // FORGE live, ARCHIVE stopped
    code(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none());
    assert_eq!(a.current_id().unwrap(), "a:ALPHA/[ALPHA]-FORGE");
}

#[test]
fn activate_confirms_then_runs() {
    let sb = sandbox();
    let mut a = app(&sb);
    a.goto_id("a:ALPHA/[ALPHA]-BEACON");
    assert!(key(&mut a, 'a').is_none());
    assert!(matches!(a.modal, Some(Modal::Confirm(_))));
    assert!(key(&mut a, 'n').is_none());
    assert!(a.modal.is_none()); // cancelled
    key(&mut a, 'a');
    let Some(Effect::RunDetached(plan)) = key(&mut a, 'y') else { panic!("expected a detached run") };
    a.run_detached(&plan);
    a.apply(snap(&sb), vec![]);
    assert!(a.snap.as_ref().unwrap().agent("ALPHA/[ALPHA]-BEACON").unwrap().live());
}

#[test]
fn activate_on_lead_refused_without_dialog() {
    let sb = sandbox();
    let mut a = app(&sb);
    a.goto_id("a:ALPHA/LEAD");
    key(&mut a, 'a');
    assert!(a.modal.is_none());
}

#[test]
fn enter_jumps_to_herdr_pane() {
    let sb = sandbox();
    let mut a = app(&sb);
    a.goto_id("a:ALPHA/LEAD");
    code(&mut a, KeyCode::Enter);
    let calls: Vec<Vec<String>> = lines(&sb.base.join("herdr_calls.log"))
        .iter()
        .map(|v| v["args"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect())
        .filter(|c: &Vec<String>| c.get(1).map(|s| s == "focus").unwrap_or(false))
        .collect();
    assert_eq!(calls, [vec!["workspace", "focus", "w2"], vec!["tab", "focus", "w2:t1"], vec!["agent", "focus", "w2:p2"]]);
    assert_eq!(lines(&sb.env.action_log).last().unwrap()["action"], "jump");
}

fn prompts(sb: &Sb) -> Vec<Vec<String>> {
    lines(&sb.base.join("herdr_calls.log"))
        .iter()
        .map(|v| v["args"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect::<Vec<_>>())
        .filter(|c| c.get(1).map(|s| s == "prompt").unwrap_or(false))
        .collect()
}

fn type_text(a: &mut App, t: &str) {
    for c in t.chars() {
        key(a, c);
    }
}

#[test]
fn m_on_a_lead_types_into_its_pane() {
    let sb = sandbox();
    let mut a = app(&sb);
    a.goto_id("a:ALPHA/LEAD");
    key(&mut a, 'm');
    assert!(matches!(a.modal, Some(Modal::Message { .. })));
    assert!(render(&mut a).contains("Message [ALPHA]-LEAD"));
    type_text(&mut a, "status please");
    code(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none());
    assert_eq!(prompts(&sb), [vec!["agent", "prompt", "w2:p2", "status please"]]);
    assert_eq!(lines(&sb.env.action_log).last().unwrap()["action"], "message");
    // no attach, no focus change
    assert!(!lines(&sb.base.join("calls.log")).iter().any(|c| c["args"][0] == "attach"));
}

#[test]
fn m_on_an_agent_goes_through_its_lead() {
    let sb = sandbox();
    let mut a = app(&sb);
    a.goto_id("a:ALPHA/[ALPHA]-SCOUT"); // live, no pane of its own
    key(&mut a, 'm');
    let Some(Modal::Message { plan, .. }) = &a.modal else { panic!("no message modal") };
    assert!(plan.relay && plan.via == "[ALPHA]-LEAD");
    type_text(&mut a, "pause sector 8");
    code(&mut a, KeyCode::Enter);
    let p = prompts(&sb);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0][2], "w2:p2"); // the lead's pane
    assert!(p[0][3].contains("for [ALPHA]-SCOUT") && p[0][3].contains("SendMessage") && p[0][3].ends_with("pause sector 8"));
}

#[test]
fn m_refusals_and_cancel() {
    let sb = sandbox();
    let mut a = app(&sb);
    a.goto_id("a:ALPHA/[ALPHA]-ARCHIVE"); // stopped
    key(&mut a, 'm');
    assert!(a.modal.is_none());
    a.goto_id("a:DELTA/[DELTA]-DRIFT"); // live, but DELTA has no lead
    key(&mut a, 'm');
    assert!(a.modal.is_none());
    a.goto_id("a:BRAVO/LEAD");
    key(&mut a, 'm');
    type_text(&mut a, "never mind");
    code(&mut a, KeyCode::Esc);
    assert!(a.modal.is_none());
    code(&mut a, KeyCode::Enter); // enter on the row jumps, it does not send
    assert!(prompts(&sb).is_empty());
}

#[test]
fn shift_j_k_step_through_rows_with_a_pane() {
    let sb = sandbox();
    let mut a = app(&sb);
    // sandbox panes: the hub, ALPHA lead, FORGE, BRAVO lead
    assert_eq!(a.current_id().unwrap(), "hub");
    let mut seen = vec![];
    for _ in 0..4 {
        key(&mut a, 'J');
        seen.push(a.current_id().unwrap());
    }
    assert_eq!(seen, ["a:ALPHA/LEAD", "a:ALPHA/[ALPHA]-FORGE", "a:BRAVO/LEAD", "hub"]);
    key(&mut a, 'K');
    assert_eq!(a.current_id().unwrap(), "a:BRAVO/LEAD");
}

#[test]
fn exit_after_jump_quits_only_when_the_jump_lands() {
    let sb = sandbox();
    let mut a = app(&sb);
    a.goto_id("a:ALPHA/LEAD");
    assert!(code(&mut a, KeyCode::Enter).is_none(), "off by default: the TUI stays up");
    a.env.exit_after_jump = true;
    assert!(matches!(code(&mut a, KeyCode::Enter), Some(Effect::Quit)));
    a.goto_id("a:ALPHA/[ALPHA]-FORGE");
    assert!(matches!(code(&mut a, KeyCode::Enter), Some(Effect::Quit)));
    // m opens the message box instead of jumping, so it never quits
    assert!(key(&mut a, 'm').is_none());
    // no pane to jump to: stay up and say why
    a.goto_id("a:ALPHA/[ALPHA]-SCOUT");
    assert!(code(&mut a, KeyCode::Enter).is_none());
}

#[test]
fn cursor_survives_refresh() {
    let sb = sandbox();
    let mut a = app(&sb);
    a.goto_id("a:DELTA/[DELTA]-DRIFT");
    a.apply(snap(&sb), vec![]);
    assert_eq!(a.current_id().unwrap(), "a:DELTA/[DELTA]-DRIFT");
}

#[test]
fn help_modal() {
    let sb = sandbox();
    let mut a = app(&sb);
    key(&mut a, '?');
    assert!(matches!(a.modal, Some(Modal::Help)));
    assert!(render(&mut a).contains("Keys (vim style)"));
    code(&mut a, KeyCode::Esc);
    assert!(a.modal.is_none());
}


#[test]
fn hub_is_an_actionable_root() {
    let sb = sandbox();
    let mut a = app(&sb);
    assert_eq!(a.current_id().unwrap(), "hub");
    assert_eq!(a.selected_agent().unwrap().name, "[ADMIRALTY]");
    assert_eq!(a.rows[0].node.kind_text, "admiralty");
    // activate refuses the hub; enter jumps to its Herdr pane
    key(&mut a, 'a');
    assert!(a.modal.is_none());
    code(&mut a, KeyCode::Enter);
    let last = lines(&sb.base.join("herdr_calls.log")).last().cloned().unwrap();
    assert_eq!(last["args"], serde_json::json!(["agent", "focus", "w1:p1"]));
}

#[test]
fn kinds_are_lead_agent_sub_agent() {
    let sb = sandbox();
    let mut a = app(&sb);
    a.goto_id("a:ALPHA/LEAD");
    key(&mut a, 'l'); // ensure open
    let kind = |a: &App, id: &str| a.rows.iter().find(|r| r.node.id == id).map(|r| r.node.kind_text.clone()).unwrap();
    assert_eq!(kind(&a, "f:ALPHA"), "fleet");
    assert_eq!(kind(&a, "a:ALPHA/LEAD"), "lead");
    assert_eq!(kind(&a, "a:ALPHA/[ALPHA]-SCOUT"), "agent");
    assert_eq!(kind(&a, "a:ALPHA/[ALPHA]-BEACON"), "agent");
    assert_eq!(kind(&a, "s:ALPHA/LEAD:aplanner-11"), "sub-agent"); // a teammate
    assert_eq!(kind(&a, "s:ALPHA/[ALPHA]-SCOUT:ab1"), "sub-agent");
}

#[test]
fn fleet_rows_tinted_by_status() {
    let sb = sandbox();
    let a = app(&sb);
    let bg = |id: &str| a.rows.iter().find(|r| r.node.id == id).and_then(|r| r.node.row_bg);
    assert_eq!(bg("f:ALPHA"), Some(mono::T_PINK)); // someone needs you
    assert_eq!(bg("f:BRAVO"), Some(mono::T_GREEN)); // lead live, an agent busy
    assert_eq!(bg("f:CHARLIE"), Some(mono::T_DIM)); // elsewhere
    assert_eq!(bg("f:DELTA"), Some(mono::T_ORANGE)); // no lead
    assert_eq!(bg("f:ECHO"), Some(mono::T_PURPLE)); // unreadable
    assert_eq!(bg("a:ALPHA/LEAD"), None);
    assert_eq!(bg("hub"), Some(mono::T_GOLD));
}

#[test]
fn fleet_with_nothing_live_is_grey_inactive() {
    use admiralty::ui::FleetState;
    let sb = sandbox();
    let mut s = snap(&sb);
    let delta = s.fleets.iter_mut().find(|f| f.tag == "DELTA").unwrap();
    assert_eq!(FleetState::of(delta), FleetState::NoLead); // DRIFT is live
    for a in &mut delta.agents {
        if let Some(p) = &mut a.primary {
            p.pid = None;
        }
    }
    assert_eq!(FleetState::of(delta), FleetState::Inactive);
    assert_eq!(FleetState::Inactive.color(), mono::COMMENT);
    assert_eq!(FleetState::Inactive.tint(), mono::T_GREY);
}

#[test]
fn columns_grow_with_width_up_to_a_max() {
    let narrow = col_widths(60);
    let wide = col_widths(300);
    assert!(narrow[0] < wide[0]);
    assert_eq!(wide, [58, 14, 5, 10, 64]); // every column capped at its maximum
    assert!(col_widths(table_max_width() - 2).iter().zip(wide.iter()).all(|(x, y)| x == y));
    let mid = col_widths(110);
    assert!(mid.iter().sum::<u16>() + 5 <= 110);
}

#[test]
fn split_modes() {
    let sb = sandbox();
    let mut a = app(&sb);
    key(&mut a, 'v');
    assert_eq!(a.split, Split::Vertical);
    let v = render(&mut a);
    // side by side: the detail title is on the same line as the table's top border
    assert!(v.lines().nth(1).unwrap().contains(" detail "));
    key(&mut a, 's');
    assert_eq!(a.split, Split::Horizontal);
    let h = render(&mut a);
    assert!(!h.lines().nth(1).unwrap().contains(" detail "));
    assert!(h.lines().skip(2).any(|l| l.contains(" detail ")));
}


#[test]
fn pane_icon_leads_the_row_and_fleets_are_bracketed() {
    let sb = sandbox();
    let mut a = app(&sb);
    let screen = render(&mut a);
    let pane = admiralty::ui::NERD.pane;
    // rows with a Herdr pane start (after the border and cursor bar) with Herdr's
    // status dot and then the pane icon; ALPHA's lead pane is idle: green empty dot
    let lead = screen.lines().find(|l| l.contains("[ALPHA]-LEAD") && l.starts_with('│')).unwrap();
    let gutter: Vec<String> = lead.chars().skip(1).take(2).map(|c| c.to_string()).collect();
    assert_eq!(gutter, ["\u{25cb}", " "]);
    assert!(!screen.contains(pane), "the dot replaced the pane icon");
    // the hub frame's title carries its Herdr dot: working, filled
    let hub = screen.lines().find(|l| l.starts_with('╔') && l.contains("[ADMIRALTY]")).unwrap();
    assert!(hub.contains("\u{25cf}"));
    let scout = screen.lines().find(|l| l.contains("[ALPHA]-SCOUT")).unwrap();
    assert!(!scout.contains("\u{25cb}") && !scout.contains("\u{25cf}"));
    assert!(screen.contains("[BRAVO]") && screen.contains("[CHARLIE]"));
}


#[test]
fn fleet_state_filter() {
    let sb = sandbox();
    let mut a = app(&sb);
    key(&mut a, 'f');
    assert!(matches!(a.modal, Some(Modal::FleetFilter { .. })));
    // untick "elsewhere" (index 5) and "unreadable" (index 6)
    for _ in 0..5 {
        key(&mut a, 'j');
    }
    key(&mut a, ' ');
    key(&mut a, 'j');
    key(&mut a, ' ');
    code(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none());
    assert_eq!(top_ids(&a), ["f:ALPHA", "f:BRAVO", "f:DELTA"]);
    let screen = render(&mut a);
    assert!(screen.contains("fleets: needs you, busy, idle, no lead, inactive"));
    // only "no lead"
    key(&mut a, 'f');
    for i in 0..7 {
        if i != 3 && a.fleet_states[i] {
            for _ in 0..7 { key(&mut a, 'k'); }
            for _ in 0..i { key(&mut a, 'j'); }
            key(&mut a, ' ');
        }
    }
    code(&mut a, KeyCode::Esc);
    assert_eq!(top_ids(&a), ["f:DELTA"]);
    key(&mut a, 'f');
    key(&mut a, 'a');
    code(&mut a, KeyCode::Esc);
    assert_eq!(top_ids(&a).len(), 5);
}

#[test]
fn top_bar_has_no_host_id() {
    let sb = sandbox();
    let mut a = app(&sb);
    let screen = render(&mut a);
    assert!(!screen.lines().next().unwrap().contains("sandbox-host"));
}


#[test]
fn herdr_dots_and_blocked_means_needs_you() {
    use admiralty::herdr::dot;
    assert_eq!((dot("working"), dot("blocked"), dot("done"), dot("idle"), dot("unknown")),
               ("\u{25cf}", "\u{25cf}", "\u{25cf}", "\u{25cb}", "\u{00b7}"));
    let sb = sandbox();
    let mut panes = herdr::list_panes(sb.env.herdr_bin.as_deref());
    for p in &mut panes {
        if p.title.starts_with("[BRAVO]-LEAD") {
            p.status = "blocked".into();
        }
    }
    let mut a = App::new(sb.env.clone(), false);
    a.apply(snap(&sb), panes);
    let s = a.snap.as_ref().unwrap();
    let lead = s.agent("BRAVO/LEAD").unwrap();
    assert_eq!(lead.status, admiralty::model::Status::NeedsYou);
    assert!(lead.job.needs.as_deref().unwrap().contains("Herdr reports blocked"));
    assert_eq!(s.need_count(), 2);
}


#[test]
fn frames_scroll_to_keep_the_cursor_visible() {
    let sb = sandbox();
    let mut a = app(&sb);
    let small = |a: &mut App| {
        let mut t = Terminal::new(TestBackend::new(120, 12)).unwrap();
        t.draw(|f| a.draw(f)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    };
    a.split = Split::Horizontal;
    key(&mut a, 'G');
    let last = a.current().unwrap().node.name.clone();
    assert!(small(&mut a).contains(&last), "bottom row scrolled into view");
    key(&mut a, 'g');
    key(&mut a, 'g');
    assert!(small(&mut a).contains("[ADMIRALTY]"), "back at the top");
}

#[test]
fn folds_filters_and_split_stick_across_restarts() {
    let sb = sandbox();
    let mut a = app(&sb);
    a.goto_id("f:ALPHA");
    key(&mut a, 'h'); // fold ALPHA
    key(&mut a, 'F'); // flagged only
    key(&mut a, 's'); // stacked split
    key(&mut a, 'f'); // fleet filter: untick "elsewhere"
    for _ in 0..5 {
        key(&mut a, 'j');
    }
    key(&mut a, ' ');
    code(&mut a, KeyCode::Esc);
    key(&mut a, '/');
    for c in "bra".chars() {
        key(&mut a, c);
    }
    code(&mut a, KeyCode::Enter);
    assert!(sb.env.state_path.exists());

    let b = app(&sb); // a fresh start reads it back
    assert_eq!(b.expanded.get("f:ALPHA"), Some(&false));
    assert!(b.flagged_only);
    assert_eq!(b.split, Split::Horizontal);
    assert!(!b.fleet_states[5] && b.fleet_states.iter().filter(|x| !**x).count() == 1);
    assert_eq!(b.filter, "bra");
}

#[test]
fn detail_pane_big_icon_matches_the_row_glyph() {
    let sb = sandbox();
    let mut a = app(&sb);
    let first = |a: &App| a.detail_with_art(80).first().map(|l| l.to_string()).unwrap_or_default();
    let art = |a: &App| a.detail_with_art(80).iter().take(5).map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
    assert!(art(&a).contains("_O_"), "hub: anchor");
    assert!(first(&a).contains("[ADMIRALTY]"), "name sits beside the art");
    a.goto_id("f:ALPHA");
    assert!(art(&a).contains("\\_______/"), "fleet: ship");
    a.goto_id("a:ALPHA/LEAD");
    assert!(art(&a).contains("/\\ / \\ /\\"), "lead: crown");
    a.goto_id("a:ALPHA/[ALPHA]-SCOUT");
    assert!(art(&a).contains("[o  o]"), "agent: robot");
    a.goto_id("a:ALPHA/LEAD");
    key(&mut a, 'l'); // open the lead
    a.goto_id("s:ALPHA/LEAD:aplanner-11");
    assert!(art(&a).contains("/|\\  /|\\"), "teammate: two figures");
    a.goto_id("t:ALPHA/LEAD:1");
    assert!(art(&a).contains("V"), "completed task: tick");
    // a narrow pane drops the art rather than squeezing the text
    a.goto_id("f:ALPHA");
    assert!(!a.detail_with_art(20).iter().any(|l| l.to_string().contains("\\_______/")));
}

#[test]
fn background_sessions_sit_under_the_lead_and_old_finished_are_hidden() {
    let sb = sandbox();
    let mut a = app(&sb);
    a.goto_id("a:ALPHA/LEAD");
    key(&mut a, 'l'); // open the lead
    let lead = a.rows.iter().position(|r| r.node.id == "a:ALPHA/LEAD").unwrap();
    let rogue = a.rows.iter().position(|r| r.node.id == "a:ALPHA/[ALPHA]-ROGUE").unwrap();
    assert!(rogue > lead && a.rows[rogue].depth == a.rows[lead].depth + 1);
    assert_eq!(a.rows[rogue].node.kind_text, "sub-agent");
    assert!(a.rows[rogue].node.detail[0].content.contains("background session"));
    // sandbox: one root sub-agent finished within the hour (reviewer, 50 min ago), six older ones not listed
    let fin = a.rows.iter().find(|r| r.node.id == "fin:ALPHA/LEAD:root").unwrap();
    assert_eq!(fin.node.name, "1 finished in the last hour");
    // m on it routes through the lead, like any agent
    a.goto_id("a:ALPHA/[ALPHA]-ROGUE");
    let plan = a.message_plan().unwrap();
    assert!(plan.ok && plan.relay && plan.via == "[ALPHA]-LEAD");
}

#[test]
fn x_confirms_then_deletes_and_refuses_fleet_agents() {
    let sb = sandbox();
    let mut a = app(&sb);
    a.goto_id("a:ALPHA/[ALPHA]-ARCHIVE");
    key(&mut a, 'x');
    assert!(a.modal.is_none(), "fleet agent: refused, no dialog");
    a.goto_id("a:DELTA/LEAD");
    key(&mut a, 'l');
    a.goto_id("a:DELTA/quick look at delta logs");
    key(&mut a, 'x');
    assert!(matches!(a.modal, Some(Modal::Confirm(_))));
    let Some(Effect::RunDetached(plan)) = key(&mut a, 'y') else { panic!("expected a run") };
    a.run_detached(&plan);
    a.apply(snap(&sb), vec![]);
    assert!(a.snap.as_ref().unwrap().agent("DELTA/quick look at delta logs").is_none());
}
