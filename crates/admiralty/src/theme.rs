//! Palette (Monokai), Nerd Font / ASCII icon sets, status words and styles,
//! and the large ASCII art for the detail pane.

use nimitz_core::{Role, Severity, Status};
use ratatui::style::{Color, Modifier, Style};

/// Monokai palette.
pub mod mono {
    use ratatui::style::Color;
    pub const BG: Color = Color::Rgb(0x27, 0x28, 0x22);
    pub const BG_HI: Color = Color::Rgb(0x3e, 0x3d, 0x32); // line highlight
    pub const SEL: Color = Color::Rgb(0x5b, 0x59, 0x4a); // cursor row
    pub const FG: Color = Color::Rgb(0xf8, 0xf8, 0xf2);
    pub const COMMENT: Color = Color::Rgb(0x75, 0x71, 0x5e);
    pub const DIM: Color = Color::Rgb(0x5a, 0x58, 0x4b);
    pub const SOFT: Color = Color::Rgb(0xcf, 0xcf, 0xc2);
    pub const PINK: Color = Color::Rgb(0xf9, 0x26, 0x72);
    pub const ORANGE: Color = Color::Rgb(0xfd, 0x97, 0x1f);
    pub const YELLOW: Color = Color::Rgb(0xe6, 0xdb, 0x74);
    pub const GREEN: Color = Color::Rgb(0xa6, 0xe2, 0x2e);
    pub const CYAN: Color = Color::Rgb(0x66, 0xd9, 0xef);
    pub const PURPLE: Color = Color::Rgb(0xae, 0x81, 0xff);
    // dark tints for title bands
    pub const T_PINK: Color = Color::Rgb(0x55, 0x1e, 0x35);
    pub const T_GREEN: Color = Color::Rgb(0x31, 0x40, 0x16);
    pub const T_CYAN: Color = Color::Rgb(0x1d, 0x3a, 0x42);
    pub const T_ORANGE: Color = Color::Rgb(0x4d, 0x33, 0x12);
    pub const T_GREY: Color = Color::Rgb(0x36, 0x36, 0x2f);
    pub const T_DIM: Color = Color::Rgb(0x2d, 0x2e, 0x28);
    pub const GOLD: Color = Color::Rgb(0xff, 0xc6, 0x3a);
    pub const T_GOLD: Color = Color::Rgb(0x4a, 0x3c, 0x14);
    pub const GOLD_DIM: Color = Color::Rgb(0x8a, 0x72, 0x2e);
    pub const T_PURPLE: Color = Color::Rgb(0x38, 0x2a, 0x55);
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
        Status::NotStarted => "o",
        Status::Unknown => "?",
    }
}

pub fn status_color(s: Status) -> Color {
    match s {
        Status::NeedsYou => mono::PINK,
        Status::Busy => mono::GREEN,
        Status::Idle => mono::YELLOW,
        Status::Stopped => mono::COMMENT,
        Status::NotStarted => mono::DIM,
        Status::Unknown => mono::PURPLE,
    }
}

pub fn status_style(s: Status) -> Style {
    match s {
        Status::NeedsYou => bold(mono::PINK),
        other => st(status_color(other)),
    }
}

pub fn severity_color(s: Severity) -> Color {
    match s {
        Severity::Error => mono::PINK,
        Severity::Warn => mono::ORANGE,
        Severity::Info => mono::CYAN,
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

pub fn pane_color(state: &str) -> Color {
    match state {
        "working" => mono::YELLOW,
        "blocked" => mono::PINK,
        "done" => mono::CYAN,
        "idle" => mono::GREEN,
        _ => mono::COMMENT,
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
    pub tender: &'static str,    // small boat / wrench
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
    pub s_off: &'static str,
    pub s_unknown: &'static str,
}

pub const NERD: Icons = Icons {
    fleet: "\u{f0100}",     // nf-md-anchor
    admiral: "\u{f01a5}",   // nf-md-crown (the lead glyph)
    squadron: "\u{f024}",   // nf-fa-flag
    commodore: "\u{f0a3}",  // nf-fa-certificate (rank rosette)
    ship: "\u{f21a}",       // nf-fa-ship
    tender: "\u{f0ad}",     // nf-fa-wrench
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
    s_off: "\u{f10c}",      // empty circle
    s_unknown: "\u{f059}",  // question circle
};

pub const ASCII: Icons = Icons {
    fleet: "#",
    admiral: "@",
    squadron: ">",
    commodore: "&",
    ship: "S",
    tender: "t",
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
    s_off: "o",
    s_unknown: "?",
};

impl Icons {
    /// Every glyph of the set, for width checks.
    pub fn all(&self) -> Vec<&'static str> {
        vec![
            self.fleet, self.admiral, self.squadron, self.commodore, self.ship, self.tender, self.crew, self.teammate, self.stowaway,
            self.issue, self.legacy, self.finished, self.row, self.completed, self.in_progress, self.pending, self.pane, self.s_needs,
            self.s_busy, self.s_idle, self.s_stopped, self.s_off, self.s_unknown,
        ]
    }

    pub fn status(&self, s: Status) -> &'static str {
        match s {
            Status::NeedsYou => self.s_needs,
            Status::Busy => self.s_busy,
            Status::Idle => self.s_idle,
            Status::Stopped => self.s_stopped,
            Status::NotStarted => self.s_off,
            Status::Unknown => self.s_unknown,
        }
    }

    pub fn role(&self, r: Role) -> &'static str {
        match r {
            Role::Admiral => self.admiral,
            Role::Commodore => self.commodore,
            Role::Ship => self.ship,
            Role::Tender => self.tender,
        }
    }
}

// ------------------------------------------------------------------ detail-pane art

const ART_ANCHOR: [&str; 5] = ["    _O_", "     |", "  \\  |  /", "   '-+-'", "  ~~~~~~~"];
const ART_CROWN: [&str; 5] = ["  .  ^  .", " /\\ / \\ /\\", " \\  V V  /", "  \\_____/", "  [_____]"];
const ART_FLAG: [&str; 5] = ["  |\\______", "  |  >>>  )", "  |______/", "  |", "  |"];
const ART_ROSETTE: [&str; 5] = ["   .-\"\"-.", "  / *  * \\", "  \\  **  /", "   '-..-'", "    |  |"];
const ART_SHIP: [&str; 5] = ["     |\\", "     | \\", "     |__\\", "  ___|_____", "  \\_______/"];
const ART_TENDER: [&str; 5] = ["", "    /\\", "   /__\\", "  _|__|_", "  \\____/"];
const ART_BOT: [&str; 5] = ["", "   .---.", "  ( o o )", "   | = |", "   '---'"];
const ART_TEAM: [&str; 5] = ["", "  o    o", " /|\\  /|\\", " / \\  / \\", ""];
const ART_MASK: [&str; 5] = ["   _____", "  / ? ? \\", "  \\__ __/", "   /   \\", "  (_____)"];
const ART_WARN: [&str; 5] = ["     /\\", "    /  \\", "   / !! \\", "  /  !!  \\", " /________\\"];
const ART_TASK_DONE: [&str; 5] = ["  +------+", "  |    / |", "  | \\ /  |", "  |  V   |", "  +------+"];
const ART_TASK_WIP: [&str; 5] = ["  +------+", "  | //// |", "  | //   |", "  |      |", "  +------+"];
const ART_TASK_TODO: [&str; 5] = ["  +------+", "  |      |", "  |      |", "  |      |", "  +------+"];
const ART_CLOCK: [&str; 5] = ["   .---.", "  / 12  \\", " |   |_  |", "  \\     /", "   '---'"];

/// Which picture the detail pane shows for a row kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Art {
    Fleet,
    Admiral,
    Squadron,
    Commodore,
    Ship,
    Tender,
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
        Art::Tender => &ART_TENDER,
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
