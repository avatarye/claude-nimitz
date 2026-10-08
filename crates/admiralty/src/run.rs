//! The terminal loop. A loader thread owns the data source and sends a fresh
//! bundle every interval (or at once on `R` and after an action); actions apply
//! on a worker thread. The screen redraws when data or a key arrives, and once a
//! second so ages and toasts stay current; otherwise the loop sleeps in `poll`.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyModifiers};

use crate::actions::Pending;
use crate::app::{App, Effect};
use crate::data::{Bundle, DataSource};

pub type SourceMaker = Box<dyn FnOnce() -> Box<dyn DataSource> + Send>;

type Term = ratatui::Terminal<crate::backend::GlyphSafe<ratatui::backend::CrosstermBackend<std::io::Stdout>>>;

/// Raw mode, the alternate screen and the panic hook from `ratatui::init`, on
/// a backend that keeps positions right whatever the terminal's glyph widths.
fn init_terminal() -> std::io::Result<Term> {
    drop(ratatui::init());
    // a panic restores the terminal through ratatui's hook; this one turns
    // bracketed paste off first (installed once: init runs again after $EDITOR)
    static PASTE_HOOK: std::sync::Once = std::sync::Once::new();
    PASTE_HOOK.call_once(|| {
        let next = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = ratatui::crossterm::execute!(std::io::stdout(), ratatui::crossterm::event::DisableBracketedPaste);
            next(info);
        }));
    });
    // a paste arrives whole (Event::Paste), never as keystrokes
    let _ = ratatui::crossterm::execute!(std::io::stdout(), ratatui::crossterm::event::EnableBracketedPaste);
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

/// Leave the terminal: bracketed paste off, then ratatui's own restore.
fn leave_terminal() {
    let _ = ratatui::crossterm::execute!(std::io::stdout(), ratatui::crossterm::event::DisableBracketedPaste);
    ratatui::restore();
}

/// The terminal size `$EDITOR` gets: the real one minus the inset (rows,
/// cols) that Herdr's overlay hides, at least one cell.
pub fn editor_size(real: (u16, u16), inset: (u16, u16)) -> (u16, u16) {
    (real.0.saturating_sub(inset.0).max(1), real.1.saturating_sub(inset.1).max(1))
}

/// Run `f` with the terminal's size shrunk by `inset` (TIOCSWINSZ on stdout),
/// then put the size back: an editor in Herdr's overlay keeps its last lines
/// in view. A size the terminal set while the editor ran (a resize) is kept.
fn with_inset_size<T>(inset: (u16, u16), f: impl FnOnce() -> T) -> T {
    use std::os::unix::io::AsRawFd;
    if inset == (0, 0) {
        return f();
    }
    let fd = std::io::stdout().as_raw_fd();
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } != 0 {
        return f();
    }
    let real = ws;
    let (rows, cols) = editor_size((ws.ws_row, ws.ws_col), inset);
    ws.ws_row = rows;
    ws.ws_col = cols;
    unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &ws) };
    let out = f();
    let mut now: libc::winsize = unsafe { std::mem::zeroed() };
    let resized = unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut now) } == 0 && (now.ws_row, now.ws_col) != (rows, cols);
    if !resized {
        unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &real) };
    }
    out
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
                leave_terminal();
                let edited = with_inset_size(app.inset, || edit_in_editor(&text));
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
    // status motion: its own tick, never a refresh or a nimitz call; only the
    // cells that change are written (ratatui sends the diff)
    app.anim_env_off = std::env::var("NIMITZ_NO_ANIM").is_ok_and(|v| v == "1") || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    app.anim_live = !app.anim_env_off;
    let started = Instant::now();
    let tick = Duration::from_millis(crate::theme::anim::TICK_MS);
    // keys and pastes before the first data lands, replayed in order once it does
    let mut early: Vec<Event> = vec![];
    let result = 'main: loop {
        // the frame first: a refresh below stamps a bar's pulse with this frame
        let frame = (started.elapsed().as_millis() / tick.as_millis()) as u64;
        if let Some(step) = app.motion_step() {
            if frame / step != app.anim_frame / step {
                dirty = true;
            }
        }
        app.anim_frame = frame;
        let mut latest = None;
        while let Ok(b) = data_rx.try_recv() {
            latest = Some(b);
        }
        if let Some(b) = latest {
            app.apply(b);
            dirty = true;
            for ev in std::mem::take(&mut early) {
                let out = match ev {
                    Event::Paste(text) => app.on_paste(&text),
                    Event::Key(k) => app.on_key(k),
                    _ => None,
                };
                if let Some(e) = out {
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
        // a frame's worth of waiting while something moves, else the usual
        // until the next frame that changes something, else the usual
        let wait = match app.motion_step() {
            Some(step) => {
                let period = tick.as_millis() * step as u128;
                Duration::from_millis((period - started.elapsed().as_millis() % period) as u64).max(Duration::from_millis(10))
            }
            None => Duration::from_millis(250),
        };
        match event::poll(wait) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => break Err(e),
        }
        let ev = match event::read() {
            Ok(e) => e,
            Err(e) => break Err(e),
        };
        dirty = true;
        if matches!(ev, Event::Paste(_)) && app.bundle.is_none() {
            early.push(ev);
            continue;
        }
        if let Event::Paste(text) = &ev {
            if let Some(e) = app.on_paste(text) {
                if effect(&mut app, e, &mut terminal, &cmd_tx, &done_tx) {
                    break Ok(());
                }
            }
            continue;
        }
        let Event::Key(k) = ev else { continue };
        let ctrl_c = k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL);
        if app.bundle.is_none() && !ctrl_c {
            early.push(Event::Key(k));
            continue;
        }
        if let Some(e) = app.on_key(k) {
            if effect(&mut app, e, &mut terminal, &cmd_tx, &done_tx) {
                break Ok(());
            }
        }
    };
    leave_terminal();
    result
}
