#![allow(dead_code)]
//! Test helpers: an App over fixtures or a sandbox, a recording multiplexer,
//! key presses, and the screen as text.

use std::path::Path;
use std::sync::{Arc, Mutex};

use admiralty::actions::Ctx;
use admiralty::app::{App, Effect};
use admiralty::data::Bundle;
use admiralty::fixtures;
use nimitz_mux::{Caps, LayoutPlan, Mux, MuxError, Pane, PaneRef, Place};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

/// A multiplexer that records every call and always succeeds.
#[derive(Clone, Default)]
pub struct TestMux {
    pub calls: Arc<Mutex<Vec<String>>>,
}

impl Mux for TestMux {
    fn name(&self) -> &'static str {
        "test"
    }
    fn caps(&self) -> Caps {
        Caps { open: true, focus: true, type_text: true, layout: true }
    }
    fn panes(&self) -> Vec<Pane> {
        vec![]
    }
    fn open(&self, cmd: &[String], cwd: &Path, place: Place) -> Result<PaneRef, MuxError> {
        self.calls.lock().unwrap().push(format!("open {:?} in {} at {:?}", cmd, cwd.display(), place));
        Ok(PaneRef { pane_id: "new".into(), tab_id: None, workspace_id: None })
    }
    fn focus(&self, pane: &PaneRef) -> Result<(), MuxError> {
        self.calls.lock().unwrap().push(format!("focus {}", pane.pane_id));
        Ok(())
    }
    fn type_text(&self, pane: &PaneRef, text: &str) -> Result<(), MuxError> {
        self.calls.lock().unwrap().push(format!("type {} {text}", pane.pane_id));
        Ok(())
    }
    fn layout(&self, plan: &LayoutPlan) -> Result<Vec<PaneRef>, MuxError> {
        let mut out = vec![];
        for (ws, tabs) in &plan.workspaces {
            for (tab, _, _) in tabs {
                self.calls.lock().unwrap().push(format!("layout {ws}/{tab}"));
                out.push(PaneRef { pane_id: format!("{ws}:{tab}"), tab_id: None, workspace_id: None });
            }
        }
        Ok(out)
    }
}

pub struct TestApp {
    pub app: App,
    pub dir: tempfile::TempDir,
    pub mux: TestMux,
}

/// An App over a fixture bundle (ASCII icons, frozen clock, state in a temp dir).
pub fn fixture_app(b: Bundle, live: bool) -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    let mux = TestMux::default();
    let ctx = Ctx { env: fixtures::demo_env(), mux: Box::new(mux.clone()), live, exit_after_jump: false };
    let mut app = App::new(ctx, Some(dir.path().join("ui.json")), true);
    app.fixed_now = Some(fixtures::NOW);
    app.apply(b);
    TestApp { app, dir, mux }
}

pub fn demo() -> TestApp {
    fixture_app(fixtures::demo(), false)
}

pub fn draw(app: &mut App, w: u16, h: u16) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| admiralty::render::draw(app, f)).unwrap();
    t.backend().buffer().clone()
}

pub fn lines(buf: &Buffer) -> Vec<String> {
    let a = buf.area;
    (0..a.height).map(|y| (0..a.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect()
}

pub fn screen(app: &mut App, w: u16, h: u16) -> String {
    lines(&draw(app, w, h)).join("\n")
}

pub fn press(app: &mut App, code: KeyCode) -> Option<Effect> {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE))
}

pub fn ctrl(app: &mut App, c: char) -> Option<Effect> {
    app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

pub fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        press(app, KeyCode::Char(c));
    }
}

/// Put the cursor on the row with this id.
pub fn goto(app: &mut App, id: &str) {
    assert!(app.goto_id(id), "no row {id}; rows: {:?}", app.rows.iter().map(|r| r.node.id.clone()).collect::<Vec<_>>());
}

pub fn toast(app: &App) -> String {
    app.toast.as_ref().map(|t| t.msg.clone()).unwrap_or_default()
}

/// The column (in cells) where `needle` starts on the first line containing `line_has`.
pub fn col_of(lines: &[String], line_has: &str, needle: &str) -> usize {
    let l = lines.iter().find(|l| l.contains(line_has)).unwrap_or_else(|| panic!("no line with {line_has}"));
    let byte = l.find(needle).unwrap_or_else(|| panic!("no {needle} in {l}"));
    l[..byte].chars().count()
}
