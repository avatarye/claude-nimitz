//! Status motion preview: draw the demo fleet frame by frame, exactly as the
//! live loop would, and write each frame's cells as JSON (for a GIF made with
//! a fixed-grid font, so the preview never depends on a viewer's fonts).
//! `cargo run --example anim_frames -- <out dir> <cols> <rows> <frames>`
use admiralty::actions::Ctx;
use admiralty::app::App;
use admiralty::fixtures;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use ratatui::Terminal;
use std::io::Write;

fn hex(c: Color, default: &str) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => default.to_string(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, cols, rows, frames): (std::path::PathBuf, u16, u16, u64) = (args[0].clone().into(), args[1].parse().unwrap(), args[2].parse().unwrap(), args[3].parse().unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut b = fixtures::demo();
    // one member waits on a background shell: the quieter dots
    for f in &mut b.snap.fleets {
        for sq in &mut f.squadrons {
            for m in sq.ships.iter_mut().filter(|m| m.id.0 == "MAIN/ARGOS/INGEST") {
                if let Some(s) = m.session.as_mut() {
                    s.status = Some("shell".into());
                }
            }
        }
    }
    let ctx = Ctx { env: fixtures::demo_env(), mux: Box::new(nimitz_mux::none::NoMux), live: false, exit_after_jump: false };
    let mut app = App::new(ctx, None, false);
    app.fixed_now = Some(fixtures::NOW);
    app.apply(b.clone());
    app.anim_live = true;
    let mut term = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    for f in 0..frames {
        app.anim_frame = f;
        // a third of the way in, ARGOS's status gets worse: its bar pulses once
        if f == frames / 3 {
            let mut worse = b.clone();
            for fl in &mut worse.snap.fleets {
                for sq in fl.squadrons.iter_mut().filter(|q| q.id.0 == "MAIN/ARGOS") {
                    sq.ships[2].issues.push(nimitz_core::Issue { code: "x".into(), severity: nimitz_core::Severity::Error, message: "m".into(), fix: None, id: None });
                }
            }
            app.apply(worse);
        }
        term.draw(|fr| admiralty::render::draw(&mut app, fr)).unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = std::fs::File::create(dir.join(format!("frame-{f:03}.json"))).unwrap();
        let cells: Vec<Vec<(String, String, String, bool)>> = (0..rows)
            .map(|y| {
                (0..cols)
                    .map(|x| {
                        let c = &buf[(x, y)];
                        (c.symbol().to_string(), hex(c.fg, "#e6e6dc"), hex(c.bg, "#272822"), c.modifier.contains(ratatui::style::Modifier::BOLD))
                    })
                    .collect()
            })
            .collect();
        writeln!(out, "{}", serde_json::to_string(&cells).unwrap()).unwrap();
    }
}
