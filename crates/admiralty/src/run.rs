//! The terminal loop. A loader thread owns the data source and sends a fresh
//! bundle every interval (or at once on `R` and after an action); actions apply
//! on a worker thread. The screen redraws when data or a key arrives, and once a
//! second so ages and toasts stay current; otherwise the loop sleeps in `poll`.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

use crate::actions::Pending;
use crate::app::{App, Effect};
use crate::data::{Bundle, DataSource};

pub type SourceMaker = Box<dyn FnOnce() -> Box<dyn DataSource> + Send>;

type Term = ratatui::Terminal<crate::backend::GlyphSafe<ratatui::backend::CrosstermBackend<std::io::Stdout>>>;

/// Raw mode, the alternate screen and the panic hook from `ratatui::init`, on
/// a backend that keeps positions right whatever the terminal's glyph widths.
fn init_terminal() -> std::io::Result<Term> {
    drop(ratatui::init());
    ratatui::Terminal::new(crate::backend::GlyphSafe(ratatui::backend::CrosstermBackend::new(std::io::stdout())))
}

enum Cmd {
    Refresh,
}

fn spawn_loader(make: SourceMaker, interval: Duration) -> (mpsc::Sender<Cmd>, mpsc::Receiver<Bundle>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut source = make();
        loop {
            let b = source.load();
            let partial = b.partial;
            if tx.send(b).is_err() {
                return;
            }
            if partial {
                continue; // the full snapshot right after the quick first look
            }
            match cmd_rx.recv_timeout(interval) {
                Ok(Cmd::Refresh) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    (cmd_tx, rx)
}

/// Edit `text` in `$EDITOR` (vi when unset); returns the new text.
fn edit_in_editor(text: &str) -> std::io::Result<String> {
    let path = std::env::temp_dir().join(format!("admiralty-edit-{}.md", std::process::id()));
    std::fs::write(&path, text)?;
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
    let status = std::process::Command::new("sh").arg("-c").arg(format!("{editor} \"$1\"")).arg("sh").arg(&path).status()?;
    let out = std::fs::read_to_string(&path)?;
    let _ = std::fs::remove_file(&path);
    if status.success() {
        Ok(out)
    } else {
        Err(std::io::Error::other(format!("{editor} exited with {status}")))
    }
}

/// Carry out one effect; `true` means quit.
fn effect(app: &mut App, e: Effect, terminal: &mut Term, cmd_tx: &mpsc::Sender<Cmd>, done_tx: &mpsc::Sender<(Pending, nimitz_core::Outcome)>) -> bool {
    match e {
        Effect::Quit => return true,
        Effect::Refresh => {
            let _ = cmd_tx.send(Cmd::Refresh);
        }
        Effect::Run(p) => {
            if app.busy.is_some() {
                app.notify("An action is still running; wait for it.", crate::theme::st(crate::theme::mono::WARN));
            } else if app.begin(&p).is_ok() {
                let env = app.ctx.env.clone();
                let tx = done_tx.clone();
                std::thread::spawn(move || {
                    let o = crate::actions::apply(&env, &p);
                    let _ = tx.send((p, o));
                });
            }
        }
        Effect::Editor => {
            if let Some(text) = app.editor_text() {
                ratatui::restore();
                let edited = edit_in_editor(&text);
                match init_terminal() {
                    Ok(t) => *terminal = t,
                    Err(e) => app.notify(format!("terminal: {e}"), crate::theme::st(crate::theme::mono::ERROR)),
                }
                let _ = terminal.clear();
                match edited {
                    Ok(t) => app.editor_done(t),
                    Err(e) => app.notify(format!("$EDITOR: {e}"), crate::theme::st(crate::theme::mono::ERROR)),
                }
            }
        }
    }
    false
}

pub fn run(mut app: App, make: SourceMaker, interval: Duration) -> std::io::Result<()> {
    let (cmd_tx, data_rx) = spawn_loader(make, interval);
    let (done_tx, done_rx) = mpsc::channel::<(Pending, nimitz_core::Outcome)>();
    let mut terminal = init_terminal()?;
    let mut dirty = true;
    let mut last_draw = Instant::now();
    // keys typed before the first data lands, replayed once it does
    let mut early: Vec<KeyEvent> = vec![];
    let result = 'main: loop {
        let mut latest = None;
        while let Ok(b) = data_rx.try_recv() {
            latest = Some(b);
        }
        if let Some(b) = latest {
            app.apply(b);
            dirty = true;
            for k in std::mem::take(&mut early) {
                if let Some(e) = app.on_key(k) {
                    if effect(&mut app, e, &mut terminal, &cmd_tx, &done_tx) {
                        break 'main Ok(());
                    }
                }
            }
        }
        while let Ok((p, o)) = done_rx.try_recv() {
            app.finish(&p, &o);
            let _ = cmd_tx.send(Cmd::Refresh);
            dirty = true;
        }
        if dirty || last_draw.elapsed() >= Duration::from_secs(1) {
            if let Err(e) = terminal.draw(|f| crate::render::draw(&mut app, f)) {
                break Err(e);
            }
            dirty = false;
            last_draw = Instant::now();
        }
        match event::poll(Duration::from_millis(250)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => break Err(e),
        }
        let ev = match event::read() {
            Ok(e) => e,
            Err(e) => break Err(e),
        };
        dirty = true;
        let Event::Key(k) = ev else { continue };
        let ctrl_c = k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL);
        if app.bundle.is_none() && !ctrl_c {
            early.push(k);
            continue;
        }
        if let Some(e) = app.on_key(k) {
            if effect(&mut app, e, &mut terminal, &cmd_tx, &done_tx) {
                break Ok(());
            }
        }
    };
    ratatui::restore();
    result
}
