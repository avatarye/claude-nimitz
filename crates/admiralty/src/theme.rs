//! Palette (Monokai), Nerd Font / ASCII icon sets, status words and styles,
//! and the large ASCII art for the detail pane.

use nimitz_core::{Role, Severity, Status};
use ratatui::style::{Color, Modifier, Style};

/// Theme A, "Monokai refined" (docs/design/admiralty-ui, direction 2): warm
/// olive neutrals and Monokai's own hues, each hue with one meaning. Five hues:
/// NEEDS (needs you, the only filled chip), ERROR, WARN, BUSY, and ACCENT (focus:
/// the active tab, key hints, fix commands). Everything else is neutral, with
/// four brightness steps: FAINT, MUTED, FG, STRONG. No tinted bands; frames are
/// LINE, except a squadron or fleet that needs you (NEEDS) or does not load (ERROR).
/// Text tokens are at least 3.8:1 on BG and lifted on the cursor row.
pub mod mono {
    use ratatui::style::Color;
    /// screen background
    pub const BG: Color = Color::Rgb(0x27, 0x28, 0x22);
    /// raised: band rows, status and key bars
    pub const BG_HI: Color = Color::Rgb(0x2f, 0x30, 0x2a);
    /// the cursor row
    pub const SEL: Color = Color::Rgb(0x4a, 0x49, 0x39);
    /// frames and tree guides
    pub const LINE: Color = Color::Rgb(0x4e, 0x4d, 0x42);
    /// text on a filled bar (a squadron's status): contrast against the fills
    /// NEEDS 5.88, ERROR 7.85, WARN 10.02, BUSY 11.05, MUTED 6.93 (WCAG AA is 4.5)
    pub const INK: Color = Color::Rgb(0x1b, 0x1c, 0x17);
    /// not started, kind, inert detail, separators (4.1:1)
    pub const FAINT: Color = Color::Rgb(0x8a, 0x86, 0x72);
    /// ages, headers, paths, info issues, key labels (6.0:1)
    pub const MUTED: Color = Color::Rgb(0xaa, 0xa5, 0x8f);
    /// body text, idle sessions (11.8:1)
    pub const FG: Color = Color::Rgb(0xe6, 0xe6, 0xdc);
    /// names of live sessions, titles (13.9:1)
    pub const STRONG: Color = Color::Rgb(0xf8, 0xf8, 0xf2);
    /// needs you: text, and the only filled chip on screen (5.1:1)
    pub const NEEDS: Color = Color::Rgb(0xff, 0x5c, 0x93);
    /// errors, a config that does not load (6.8:1)
    pub const ERROR: Color = Color::Rgb(0xfd, 0x97, 0x1f);
    /// warnings, no admiral, legacy, stowaways (8.7:1)
    pub const WARN: Color = Color::Rgb(0xd8, 0xc7, 0x66);
    /// busy, pane working, crew running (9.6:1)
    pub const BUSY: Color = Color::Rgb(0xa6, 0xe2, 0x2e);
    /// focus: active tab, key hints, fix commands (9.0:1)
    pub const ACCENT: Color = Color::Rgb(0x66, 0xd9, 0xef);
    /// the shown fleet's tab after its name (state word, counts): ACCENT's
    /// hue at 40% brightness, so name and counts read as one strip
    pub const ACCENT_DIM: Color = Color::Rgb(0x29, 0x57, 0x60);
    // older names, kept for the call sites that mean the same step
    pub const COMMENT: Color = MUTED;
    pub const SOFT: Color = MUTED;
    pub const DIM: Color = FAINT;
}

pub fn st(c: Color) -> Style {
    Style::new().fg(c)
}
pub fn bold(c: Color) -> Style {
    Style::new().fg(c).add_modifier(Modifier::BOLD)
}

// ------------------------------------------------------------------ status

pub fn status_word(s: Status) -> &'static str {
    match s {
        Status::NeedsYou => "needs you",
        Status::Busy => "busy",
        Status::Idle => "idle",
        Status::Stopped => "stopped",
        Status::Parked => "parked",
        Status::NotStarted => "not started",
        Status::Unknown => "unknown",
    }
}

pub fn status_glyph(s: Status) -> &'static str {
    match s {
        Status::NeedsYou => "!",
        Status::Busy => "*",
        Status::Idle => "-",
        Status::Stopped => "x",
        Status::Parked => "z",
        Status::NotStarted => "o",
        Status::Unknown => "?",
    }
}

pub fn status_color(s: Status) -> Color {
    match s {
        Status::NeedsYou => mono::NEEDS,
        Status::Busy => mono::BUSY,
        Status::Idle => mono::FG,
        Status::Stopped => mono::MUTED,
        Status::Parked => mono::MUTED,
        Status::NotStarted => mono::FAINT,
        Status::Unknown => mono::MUTED,
    }
}

/// Needs-you is the one filled chip; the rest is coloured text.
pub fn status_style(s: Status) -> Style {
    match s {
        Status::NeedsYou => chip(mono::NEEDS),
        other => st(status_color(other)),
    }
}

/// WCAG contrast ratio of two RGB colours (1 to 21); None for a non-RGB one.
pub fn contrast(a: Color, b: Color) -> Option<f64> {
    fn lum(c: Color) -> Option<f64> {
        let Color::Rgb(r, g, b) = c else { return None };
        let f = |v: u8| {
            let c = v as f64 / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        Some(0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b))
    }
    let (x, y) = (lum(a)?, lum(b)?);
    Some((x.max(y) + 0.05) / (x.min(y) + 0.05))
}

/// The contrast text needs on its background: WCAG AA, 4.5:1, or 3:1 for
/// bold text (AA's large-text step).
pub fn min_contrast(bold: bool) -> f64 {
    if bold { 3.0 } else { 4.5 }
}

/// `c` on `bg` when it reads there (`min_contrast`), else `FG`.
pub fn readable_on(c: Color, bg: Color, bold: bool) -> Color {
    if contrast(c, bg).is_some_and(|r| r >= min_contrast(bold)) { c } else { mono::FG }
}

/// Dark text on a filled background, bold.
pub fn chip(c: Color) -> Style {
    Style::new().fg(mono::BG).bg(c).add_modifier(Modifier::BOLD)
}

pub fn severity_color(s: Severity) -> Color {
    match s {
        Severity::Error => mono::ERROR,
        Severity::Warn => mono::WARN,
        Severity::Info => mono::MUTED,
    }
}

pub fn severity_word(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warn => "warn",
        Severity::Info => "info",
    }
}

/// The multiplexer's pane-state dot (Herdr sidebar: working|blocked|done|idle|unknown).
pub fn pane_dot(state: &str) -> &'static str {
    match state {
        "working" | "blocked" | "done" => "\u{25cf}", // filled
        "idle" => "\u{25cb}",                         // empty
        _ => "\u{00b7}",                              // unknown
    }
}

/// The pane dot reuses the status hues: working is busy, blocked needs you,
/// idle and done stay neutral.
pub fn pane_color(state: &str) -> Color {
    match state {
        "working" => mono::BUSY,
        "blocked" => mono::NEEDS,
        "done" => mono::MUTED,
        "idle" => mono::FG,
        _ => mono::FAINT,
    }
}

// ------------------------------------------------------------------ icons

/// Nerd Font glyphs (JetBrainsMono Nerd Font, verified codepoints);
/// `ADMIRALTY_ICONS=ascii` swaps them out.
pub struct Icons {
    pub fleet: &'static str,     // anchor
    pub admiral: &'static str,   // star / rank
    pub squadron: &'static str,  // flag
    pub commodore: &'static str, // rank badge
    pub ship: &'static str,      // ferry
    pub aide: &'static str,      // inbox tray: an always-on member with a queue
    pub crew: &'static str,      // robot
    pub teammate: &'static str,  // people
    pub stowaway: &'static str,  // incognito
    pub issue: &'static str,     // warning triangle
    pub legacy: &'static str,    // history: a session of the old fleet-skill fleets
    pub finished: &'static str,  // archive
    pub row: &'static str,       // dash / stale row
    pub completed: &'static str,
    pub in_progress: &'static str,
    pub pending: &'static str,
    pub pane: &'static str, // terminal
    pub s_needs: &'static str,
    pub s_busy: &'static str,
    pub s_idle: &'static str,
    pub s_stopped: &'static str,
    /// parked: retired by the daemon after an idle hour, wakes on a prompt
    pub s_parked: &'static str,
    pub s_off: &'static str,
    pub s_unknown: &'static str,
    /// busy, frame by frame (the status motion)
    pub spin: &'static [&'static str],
    /// busy waiting on a background shell, frame by frame (slower)
    pub wait: &'static [&'static str],
}

pub const NERD: Icons = Icons {
    fleet: "\u{f0100}",     // nf-md-anchor
    admiral: "\u{f01a5}",   // nf-md-crown (the lead glyph)
    squadron: "\u{f024}",   // nf-fa-flag
    commodore: "\u{f0a3}",  // nf-fa-certificate (rank rosette)
    ship: "\u{f21a}",       // nf-fa-ship
    aide: "\u{f01c}",       // nf-fa-inbox
    crew: "\u{f06a9}",      // nf-md-robot-outline
    teammate: "\u{f0c0}",   // nf-fa-users
    stowaway: "\u{f21b}",   // nf-fa-user-secret
    issue: "\u{f071}",      // nf-fa-warning
    legacy: "\u{f02da}",    // nf-md-history
    finished: "\u{f187}",   // nf-fa-archive
    row: "\u{f28d}",        // nf-fa-stop-circle-o
    completed: "\u{f058}",  // nf-fa-check-circle
    in_progress: "\u{f110}",// nf-fa-spinner
    pending: "\u{f10c}",    // nf-fa-circle-o
    pane: "\u{f120}",       // nf-fa-terminal
    s_needs: "\u{f0f3}",    // bell
    s_busy: "\u{f0e7}",     // bolt
    s_idle: "\u{f186}",     // moon
    s_stopped: "\u{f28d}",  // stop circle
    s_parked: "\u{f04b2}",  // nf-md-sleep
    s_off: "\u{f10c}",      // empty circle
    s_unknown: "\u{f059}",  // question circle
    // nf-md-circle-slice-1..8: a pie that fills; nf-md-timer-sand-empty, -sand, -full
    spin: &["\u{f0a9e}", "\u{f0a9f}", "\u{f0aa0}", "\u{f0aa1}", "\u{f0aa2}", "\u{f0aa3}", "\u{f0aa4}", "\u{f0aa5}"],
    wait: &["\u{f06ad}", "\u{f051f}", "\u{f078c}", "\u{f051f}"],
};

pub const ASCII: Icons = Icons {
    fleet: "#",
    admiral: "@",
    squadron: ">",
    commodore: "&",
    ship: "S",
    aide: "A",
    crew: "s",
    teammate: "T",
    stowaway: "?",
    issue: "^",
    legacy: "L",
    finished: "+",
    row: "-",
    completed: "v",
    in_progress: "~",
    pending: ".",
    pane: "P",
    s_needs: "!",
    s_busy: "*",
    s_idle: "-",
    s_stopped: "x",
    s_parked: "z",
    s_off: "o",
    s_unknown: "?",
    // no other status's glyph (`-` idle, `o` not started, `.` finished)
    spin: &["|", "/", "=", "\\"],
    wait: &[",", ";", ":", ";"],
};

impl Icons {
    /// Every glyph of the set, for width checks.
    pub fn all(&self) -> Vec<&'static str> {
        vec![
            self.fleet, self.admiral, self.squadron, self.commodore, self.ship, self.aide, self.crew, self.teammate, self.stowaway,
            self.issue, self.legacy, self.finished, self.row, self.completed, self.in_progress, self.pending, self.pane, self.s_needs,
            self.s_busy, self.s_idle, self.s_stopped, self.s_parked, self.s_off, self.s_unknown,
        ]
        .into_iter()
        .chain(self.spin.iter().copied())
        .chain(self.wait.iter().copied())
        .collect()
    }

    pub fn status(&self, s: Status) -> &'static str {
        match s {
            Status::NeedsYou => self.s_needs,
            Status::Busy => self.s_busy,
            Status::Idle => self.s_idle,
            Status::Stopped => self.s_stopped,
            Status::Parked => self.s_parked,
            Status::NotStarted => self.s_off,
            Status::Unknown => self.s_unknown,
        }
    }

    pub fn role(&self, r: Role) -> &'static str {
        match r {
            Role::Admiral => self.admiral,
            Role::Commodore => self.commodore,
            Role::Ship => self.ship,
            Role::Aide => self.aide,
        }
    }
}

// ------------------------------------------------------------------ detail-pane art

const ART_ANCHOR: [&str; 5] = ["    _O_", "     |", "  \\  |  /", "   '-+-'", "  ~~~~~~~"];
const ART_CROWN: [&str; 5] = ["  .  ^  .", " /\\ / \\ /\\", " \\  V V  /", "  \\_____/", "  [_____]"];
const ART_FLAG: [&str; 5] = ["  |\\______", "  |  >>>  )", "  |______/", "  |", "  |"];
const ART_ROSETTE: [&str; 5] = ["   .-\"\"-.", "  / *  * \\", "  \\  **  /", "   '-..-'", "    |  |"];
const ART_SHIP: [&str; 5] = ["     |\\", "     | \\", "     |__\\", "  ___|_____", "  \\_______/"];
const ART_BOT: [&str; 5] = ["", "   .---.", "  ( o o )", "   | = |", "   '---'"];
const ART_TEAM: [&str; 5] = ["", "  o    o", " /|\\  /|\\", " / \\  / \\", ""];
const ART_MASK: [&str; 5] = ["   _____", "  / ? ? \\", "  \\__ __/", "   /   \\", "  (_____)"];
const ART_WARN: [&str; 5] = ["     /\\", "    /  \\", "   / !! \\", "  /  !!  \\", " /________\\"];
const ART_TASK_DONE: [&str; 5] = ["  +------+", "  |    / |", "  | \\ /  |", "  |  V   |", "  +------+"];
const ART_TASK_WIP: [&str; 5] = ["  +------+", "  | //// |", "  | //   |", "  |      |", "  +------+"];
const ART_TASK_TODO: [&str; 5] = ["  +------+", "  |      |", "  |      |", "  |      |", "  +------+"];
const ART_INBOX: [&str; 5] = ["  _________", " |\\       /|", " | \\_____/ |", " |  inbox  |", " |_________|"];
const ART_CLOCK: [&str; 5] = ["   .---.", "  / 12  \\", " |   |_  |", "  \\     /", "   '---'"];

/// Which picture the detail pane shows for a row kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Art {
    Fleet,
    Admiral,
    Squadron,
    Commodore,
    Ship,
    Aide,
    Crew,
    Teammate,
    Stowaway,
    Issue,
    TaskDone,
    TaskWip,
    TaskTodo,
    StaleRow,
}

pub fn art(a: Art) -> &'static [&'static str; 5] {
    match a {
        Art::Fleet => &ART_ANCHOR,
        Art::Admiral => &ART_CROWN,
        Art::Squadron => &ART_FLAG,
        Art::Commodore => &ART_ROSETTE,
        Art::Ship => &ART_SHIP,
        Art::Aide => &ART_INBOX,
        Art::Crew => &ART_BOT,
        Art::Teammate => &ART_TEAM,
        Art::Stowaway => &ART_MASK,
        Art::Issue => &ART_WARN,
        Art::TaskDone => &ART_TASK_DONE,
        Art::TaskWip => &ART_TASK_WIP,
        Art::TaskTodo => &ART_TASK_TODO,
        Art::StaleRow => &ART_CLOCK,
    }
}

// ------------------------------------------------------------------ ages

pub fn age(seconds: f64) -> String {
    if seconds.is_nan() || seconds < 0.0 {
        return String::new();
    }
    let s = seconds as i64;
    for (unit, n) in [("d", 86400), ("h", 3600), ("m", 60)] {
        if s >= n {
            return format!("{}{unit}", s / n);
        }
    }
    format!("{s}s")
}

/// The motion a row's status cell (or a bar) takes in the live loop, one
/// frame every `anim::TICK`. Static everywhere else (tests, `dump`).
pub mod anim {
    use ratatui::style::Color;

    /// one animation frame
    pub const TICK_MS: u64 = 100;
    /// the glyph frames are the icon set's (`Icons::spin`, `Icons::wait`):
    /// a Nerd Font has them where a braille block may be missing
    /// the dim green of a crew's spinner glyph (running crew, a shell wait:
    /// quieter than a member's); only the glyph, the word keeps its colour.
    /// A glyph needs 3:1: 5.45 on BG, 3.34 on the cursor row's SEL
    pub const BUSY_DIM: Color = Color::Rgb(0x80, 0xaa, 0x2a);
    /// the bright end of the needs-you pulse (the chip's dark text, mono::BG,
    /// keeps 7.7:1 on it, 5.1:1 on NEEDS)
    pub const NEEDS_HI: Color = Color::Rgb(0xff, 0x9e, 0xc0);

    /// 0..=1..=0 over `period` frames (a triangle wave)
    pub fn tri(frame: u64, period: u64) -> f64 {
        let p = (frame % period) as f64 / period as f64;
        1.0 - (2.0 * p - 1.0).abs()
    }

    /// `a` blended toward `b` by `t` (0..=1)
    pub fn mix(a: Color, b: Color, t: f64) -> Color {
        match (a, b) {
            (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
                let m = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
                Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
            }
            _ => a,
        }
    }
}
