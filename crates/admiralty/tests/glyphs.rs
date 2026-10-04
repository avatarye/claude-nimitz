//! BUG-37: every glyph Admiralty draws must be one cell wide, both by
//! unicode-width (what the layout counts) and in tmux (what the screen does),
//! or rows and popup borders shift.

use admiralty::theme::{ASCII, NERD};
use unicode_width::UnicodeWidthStr;

/// The fixed glyphs outside the icon sets: fold markers, pane dots, keys, bars.
const OTHERS: [&str; 14] = ["\u{25be}", "\u{25b8}", "\u{25cf}", "\u{25cb}", "\u{00b7}", "\u{23ce}", "\u{2588}", "\u{2039}", "\u{203a}", "\u{2717}", "\u{2026}", "\u{2191}", "\u{2193}", "\u{2014}"];

#[test]
fn every_glyph_is_one_cell_by_unicode_width() {
    for g in NERD.all().into_iter().chain(ASCII.all()).chain(OTHERS) {
        assert_eq!(g.width(), 1, "{:?} (U+{:04X}) is not one cell", g, g.chars().next().unwrap() as u32);
    }
}

/// Render the demo in a private tmux server and check every line fills exactly
/// the window: a glyph tmux draws two cells wide would leave a line one short.
#[test]
fn rows_and_borders_line_up_in_tmux() {
    if std::process::Command::new("tmux").arg("-V").output().is_err() {
        eprintln!("[WARN] no tmux: skipped");
        return;
    }
    let sock = format!("adm-glyphs-{}", std::process::id());
    let t = |args: &[&str]| std::process::Command::new("tmux").arg("-L").arg(&sock).args(args).output().unwrap();
    let bin = env!("CARGO_BIN_EXE_admiralty");
    for (w, h, keys) in [(200, 50, ""), (200, 50, "i"), (200, 50, "x"), (120, 40, "?"), (80, 24, "")] {
        t(&["kill-server"]);
        let cmd = format!("ADMIRALTY_NO_CLIPBOARD=1 TERM=xterm-256color '{bin}' --demo");
        let out = t(&["new-session", "-d", "-x", &w.to_string(), "-y", &h.to_string(), &cmd]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let cap = || String::from_utf8_lossy(&t(&["capture-pane", "-p", "-N"]).stdout).into_owned();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !cap().contains("ADMIRALTY") && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        for k in keys.chars() {
            t(&["send-keys", &k.to_string()]);
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        let cap = cap();
        t(&["kill-server"]);
        let lines: Vec<&str> = cap.lines().collect();
        assert!(lines.len() >= h as usize - 1, "{w}x{h} {keys:?}: captured {} lines", lines.len());
        for (i, l) in lines.iter().enumerate() {
            assert_eq!(l.chars().count(), w as usize, "{w}x{h} {keys:?} line {i} is not {w} cells in tmux: {l:?}");
        }
    }
}

/// A terminal may draw Nerd-font glyphs two cells wide (tmux's
/// `codepoint-widths` makes tmux do just that). Borders must still land in the
/// same columns as with one-cell glyphs: `GlyphSafe` repositions after each.
#[test]
fn borders_stay_put_when_the_terminal_draws_icons_two_cells_wide() {
    if std::process::Command::new("tmux").arg("-V").output().is_err() {
        eprintln!("[WARN] no tmux: skipped");
        return;
    }
    let bin = env!("CARGO_BIN_EXE_admiralty");
    let wide = |c: char| (0xE000..=0xF8FF).contains(&(c as u32)) || c as u32 >= 0xF0000;
    let run = |two: bool| -> Vec<String> {
        let sock = format!("adm-wide-{}-{two}", std::process::id());
        let t = |args: &[&str]| std::process::Command::new("tmux").arg("-L").arg(&sock).args(args).output().unwrap();
        t(&["kill-server"]);
        let cmd = format!("sleep 0.5; ADMIRALTY_NO_CLIPBOARD=1 TERM=xterm-256color exec '{bin}' --demo");
        assert!(t(&["-f", "/dev/null", "new-session", "-d", "-x", "200", "-y", "50", &cmd]).status.success());
        if two {
            t(&["set", "-s", "codepoint-widths", "U+E000-U+F8FF=2,U+F0000-U+FFFFD=2"]);
        }
        let cap = || String::from_utf8_lossy(&t(&["capture-pane", "-p", "-N"]).stdout).into_owned();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !cap().contains("ADMIRALTY") && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        for k in ["j"; 15].iter().chain(["x"].iter()) {
            t(&["send-keys", k]);
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        let out = cap();
        t(&["kill-server"]);
        out.lines().map(String::from).collect()
    };
    let columns = |lines: &[String], two: bool| -> std::collections::BTreeSet<usize> {
        let mut set = std::collections::BTreeSet::new();
        for l in lines {
            let mut x = 0;
            for c in l.chars() {
                if "\u{2551}\u{2502}\u{2503}\u{2557}\u{256e}\u{2513}\u{255d}\u{256f}\u{251b}".contains(c) {
                    set.insert(x);
                }
                x += if two && wide(c) { 2 } else { 1 };
            }
        }
        set
    };
    let normal = columns(&run(false), false);
    let doubled = columns(&run(true), true);
    assert_eq!(doubled, normal, "with two-cell icons some borders moved");
}
