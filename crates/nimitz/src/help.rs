//! The usage: `nimitz help` (every command, most used first, one summary
//! each), `nimitz help <cmd>` and `nimitz <cmd> --help` (one command, every
//! form in full). Wrapped to 80 columns; coloured by the view's rule
//! (`view::colour_on`: a terminal, no `NO_COLOR`, no `TERM=dumb`;
//! `NIMITZ_COLOR=1` forces it): headers, command names, arguments and flags
//! each in their own colour.
use crate::view::{colour_on, Paint, ACCENT, BUSY, FG, STRONG, WARN};

/// Every line fits in this many columns.
const WIDTH: usize = 80;

pub struct Usage {
    pub name: &'static str,
    /// in the "Most used" section
    most: bool,
    /// the synopsis `nimitz help` shows
    head: &'static str,
    /// one line on what it does (`nimitz help`, and above the forms)
    short: &'static str,
    /// each form: its synopsis and what it does (`help <cmd>`); none: the head
    forms: &'static [(&'static str, &'static str)],
}

const fn u(name: &'static str, most: bool, head: &'static str, short: &'static str, forms: &'static [(&'static str, &'static str)]) -> Usage {
    Usage { name, most, head, short, forms }
}

/// Each command's usage, in the order `nimitz help` prints it.
pub const USAGES: &[Usage] = &[
    // ---------------------------------------------------------------- most used
    u("status", true, "status [<path>...] [--json]", "the fleets, their members and issues (paths: only those members)", &[(
        "status [<path>...] [--json]",
        "down: taken down while active (a ... start wakes it); parked: retired by the daemon or not active (--parked wakes it). A live member's line ends in its topic, cut to the terminal's width (never in a pipe).",
    )]),
    u("start", true, "start <path>... [--fresh] [--parked] [--force]", "bring members up, the jobs only (no pane)", &[(
        "start <path>... [--fresh] [--parked] [--force]",
        "What is down is woken by its exact id, top-down; a member a path names, and a lead under a ..., gets a first launch when it never ran; a worker on an older Claude Code is relaunched. --fresh: a named member's new conversation; --parked: the parked and stopped ships a ... covers too; --force: a cold launch over a broken lineage or an untested Claude Code.",
    )]),
    u("stop", true, "stop <path|session>...", "stop members, or one session by its id", &[(
        "stop <path|session>...",
        "Under a ...: the live ones, ships first.",
    )]),
    u("restart", true, "restart <path>... [--parked]", "relaunch the live members and wake the ones that are down", &[(
        "restart <path>... [--parked]",
        "Same id and history, every flag again, ships first; a turn in flight is dropped. A pane whose attach the relaunch ends is reattached.",
    )]),
    u("muster", true, "muster [<path>...] [--parked]", "the one command after a reboot, a Claude Code update or a Herdr closure", &[(
        "muster [<path>...] [--parked] [--here] [--rename] [--force]",
        "No path: //.... One plan: workers on an older Claude Code relaunched (restart //... relaunches every one), down leads and the active set's down ships woken (--parked: the rest too), every member's pane reattached (its own, else its lead tab); --here, --rename, --force as for layout. With no multiplexer: the jobs, the pane steps skipped.",
    )]),
    u("open", true, "open <path>... [--tab] [--force]", "a pane for each member, focused", &[(
        "open <path>... [--tab] [--force]",
        "Its pane, its tab, a split beside its commodore, or a new tab; --tab: always its own tab (F.SQ.SHIP, F.ADMIRAL, F.SQ.COMMODORE), reused or created; --force: a second tab beside a taken one. One that is down is woken first.",
    )]),
    u("signal", true, "signal <path> <text> [--inbox|--type] [--from <label>]", "send a member a message: typed into its tab, or posted to its inbox", &[
        (
            "signal <path> <text> [--inbox|--type] [--from <label>]",
            "Yours (a plain terminal): typed into the member's tab (opened first when it has none), your own input. --inbox, a member's signal, or one from a Claude Code session or with --from: posted to its inbox as a request that names the sender. Their --type: typed into the tab, led by [nimitz signal from <who>], with the same pane checks, never into one waiting on the user.",
        ),
        (
            "signal session:<short-or-name> <text> [--from <label>]",
            "Any live local Claude Code session, member or not, by its inbox (never typed); Claude Code may hold or refuse it.",
        ),
        (
            "",
            "The sender is what nimitz verifies: a member's id, else the Claude Code session above it (its own session file), else an unidentified process; a non-member's header adds \"; not a fleet member, unverified\". --from <label> (non-members only, a short plain ASCII label) is a claim, shown quoted (says it is \"<label>\"), refused when it mimics a member, nimitz or the user. One path: a member, or F/... or F/SQ/... for every live member under it but the sender (never //...), all or nothing, one gate. A member named exactly that is down is refused, never woken; under a ... it is skipped.",
        ),
    ]),
    u("squadron", true, "squadron new|set|rm <F/SQ> [--brief T] [--mode M] [--effort E]", "a squadron's config", &[(
        "squadron new|set|rm <F/SQ> [--brief T] [--mode M] [--effort E]",
        "--mode and --effort are its ships' defaults; a ship's own wins.",
    )]),
    u("ship", true, "ship new|set|rm <F/SQ/SHIP> [--dir PATH|managed] [--prompt T] ...", "a ship's config", &[(
        "ship new|set|rm <F/SQ/SHIP> [--dir PATH|managed] [--machine L] [--prompt T|--prompt-file P] [--mode M] [--depends A,B] [--effort low|medium|high|xhigh|max]",
        "--effort '' clears it; a live ship takes a change at its restart.",
    )]),
    // ---------------------------------------------------------------- less used
    u("fleet", false, "fleet new|set|rm <F>", "a fleet's config", &[("fleet new|set|rm <F> [--machine L] [--admiral-mode M]", "new: --machine L; set: --admiral-mode M.")]),
    u("machine", false, "machine add|rm <F> <label>", "run a fleet on another machine too, or stop doing so", &[(
        "machine add|rm <F> <label> [--hostname H] [--dir F/SQ/SHIP=PATH]",
        "add: --hostname H (another machine), --dir F/SQ/SHIP=PATH (repeat); rm warns about live members when <label> is this machine.",
    )]),
    u("settings", false, "settings get|set [<key>] [<value>]", "the fleet's settings.json", &[
        ("settings get [<key>]", "A dotted key: kb.remote."),
        ("settings set <key> <value>", "JSON or text; null removes it (--plan|--yes)."),
        ("", "--fleet F; a member's default is its own fleet."),
    ]),
    u("harness", false, "harness list|add|rm|push", "what every member of a fleet gets beyond the role rules", &[
        ("harness list", "The fleet's harness: its rules, agents, skills and settings (a member: its own)."),
        (
            "harness add rule|agent|skill|settings <file|folder> [--name N] [--for F/SQ|F/SQ/SHIP [--member]]",
            "Into the fleet's harness (--plan|--yes); settings: model, outputStyle, effortLevel, permissions, env only.",
        ),
        ("harness rm rule|agent|skill <name>", "Out of it (kept as .archived)."),
        ("harness rm settings [--for ID [--member]]", "The settings, out of it."),
        ("harness push", "Make this machine's harness plugin again; say what is live and what needs a restart."),
    ]),
    u("kb", false, "kb path|status|head|show|grep|hash|commit|restore|sync|clone", "the fleet's knowledge base on this machine", &[
        ("kb path|status|head", "Where it is (settings kb.dirs.<this machine>), its state, its HEAD."),
        ("kb show <path> | kb grep <term> [<prefix>]", "Read one commit (HEAD, or --at <sha>); grep is literal, any case."),
        ("kb hash <path>", "sha256 of a file's body after its frontmatter (a note's id)."),
        (
            "kb commit --title T --paths A,B --pages X,Y --note <sha256> --from \"<kind> <id>\"",
            "Commit exactly those paths, once per note, then sync (--no-sync: not).",
        ),
        ("kb restore --paths A,B", "Undo a failed ingest (what was there is kept under NIMITZ_STATE/kb-restore)."),
        ("kb sync", "pull --rebase, then push."),
        ("kb clone", "Clone kb.remote into this machine's folder (--plan|--yes)."),
    ]),
    u("check", false, "check [<path>...]", "the problems of the members the paths cover, each with its fix", &[(
        "check [<path>...]",
        "Their parents and everything under them too (none: everything; //...: every fleet but ADMIRALTY).",
    )]),
    u("rules", false, "rules [<path>...]", "the role rules nimitz hands each member at session start", &[("rules [<path>...]", "None: the caller's own.")]),
    u("doctor", false, "doctor [--fix]", "check this machine's install", &[(
        "doctor [--fix]",
        "Hooks, the plugin's cached copy, crossSessionInbound, native claude, trust, versions, formats, role briefs, the format steps due, the machines of each fleet.",
    )]),
    u("init", false, "init [--label L]", "data root, machine label, the data root's trust, doctor", &[(
        "init [--label L]",
        "Makes the data root, sets this machine's label, has Claude Code trust the data root, then runs doctor.",
    )]),
    u("trust", false, "trust <folder>", "Claude Code's trust for a folder's git root (or the folder)", &[(
        "trust <folder>",
        "It lets the folder's .claude settings, hooks and MCP servers run.",
    )]),
    u("migrate", false, "migrate --plan|--yes [--shared]", "the format steps due on this machine", &[
        (
            "migrate --plan|--yes [--shared]",
            "Local steps run by themselves with the first command after an update; --shared runs the shared ones (fleet.toml), which affect every machine: update each one first (docs/design/migrations.md).",
        ),
        ("migrate fleet-skill <dir...> --into <F>", "Import old fleet-skill fleets."),
    ]),
    u("sweep", false, "sweep [<path>...]", "drop stale rows and copies of members", &[("sweep [<path>...]", "None: a member's own, else every one.")]),
    u("adopt", false, "adopt <session> <F|F/SQ|F/SQ/SHIP>", "make a stowaway one member's session", &[(
        "adopt <session> <F|F/SQ|F/SQ/SHIP>",
        "One exact path, never a ....",
    )]),
    u("rm", false, "rm <path|session>...", "remove job rows (never transcripts)", &[(
        "rm <path|session>...",
        "A member's stale rows, or one row by its id.",
    )]),
    u("attach", false, "attach <F|F/SQ|F/SQ/SHIP>", "attach this terminal to one member", &[(
        "attach <F|F/SQ|F/SQ/SHIP>",
        "claude attach <its exact job>; one that is down is woken first (--plan, --yes); ctrl-z detaches; never from inside a Claude Code session; one exact path, never a ....",
    )]),
    u("close", false, "close <path>...", "close the panes that show those members; the jobs run on", &[(
        "close <path>...",
        "Only a pane that runs its attach.",
    )]),
    u("layout", false, "layout <path>... [--ships] [--here] [--rename] [--force]", "the Herdr tabs only: one attaching each lead", &[(
        "layout <path>... [--ships] [--here] [--rename] [--force]",
        "One tab attaching each lead the paths cover (--ships: and each live ship); --here: in this Herdr session; --rename: give a tab that shows a member under another name its full name; --force: a second tab beside a taken one.",
    )]),
    u("sandbox", false, "sandbox create|tick|env <dir>", "a fake world for tests and demos", &[("sandbox create|tick|env <dir>", "tick --once: one tick, then exit.")]),
    u("hook", false, "hook <Event>", "the Claude Code hook (reads stdin, always exits 0)", &[]),
];

const HEAD: &str = "nimitz <command> [args] [--json] [--plan] [--yes] [--force] [-v]
nimitz --version
nimitz help [<command>]   (or nimitz <command> --help)";

const FLAGS: &str = "--plan prints a plan and stops; --yes runs it without asking; -v shows a plan's details and commands; --json for scripts. Colour on a terminal only: NO_COLOR (any value) or TERM=dumb turns it off, NIMITZ_COLOR=1 forces it.";

const PATHS: &str = "F (an admiral), F/SQ (a commodore), F/SQ/SHIP (a ship), each one member; F/... and F/SQ/... everything under one, the lead included; //... every fleet (never ADMIRALTY). Several per command; a member matched twice counts once.";

/// Is `cmd` a command with a usage?
pub fn known(cmd: &str) -> bool {
    USAGES.iter().any(|u| u.name == cmd)
}

/// The usage of every command, or of `cmd` alone.
pub fn usage(cmd: Option<&str>) -> String {
    let k = Paint(colour_on());
    let mut out: Vec<String> = vec![];
    let header = |out: &mut Vec<String>, s: &str| out.push(k.c(s, ACCENT, true));
    match cmd {
        None => {
            header(&mut out, "Usage");
            for l in HEAD.lines() {
                out.extend(synopsis(&k, l));
            }
            for (title, most) in [("Most used", true), ("Less used", false)] {
                out.push(String::new());
                header(&mut out, title);
                for u in USAGES.iter().filter(|u| u.most == most) {
                    out.extend(synopsis(&k, u.head));
                    out.extend(wrap(u.short, 6).into_iter().map(|l| k.c(&l, FG, false)));
                }
            }
            out.push(String::new());
            header(&mut out, "Flags");
            out.extend(wrap(FLAGS, 2));
            out.push(String::new());
            out.push(k.c("nimitz help <command> shows one command in full.", FG, false));
        }
        Some(c) => {
            let Some(u) = USAGES.iter().find(|u| u.name == c) else { return String::new() };
            out.extend(wrap_units(std::iter::once(k.c(&format!("{c}:"), STRONG, true)).zip([c.len() + 1]).chain(words(u.short)), 0, 2));
            out.push(String::new());
            header(&mut out, "Usage");
            if u.forms.is_empty() {
                out.extend(synopsis(&k, u.head));
            }
            for (syn, desc) in u.forms {
                if !syn.is_empty() {
                    out.extend(synopsis(&k, syn));
                }
                out.extend(wrap(desc, 6));
            }
        }
    }
    out.push(String::new());
    header(&mut out, "Paths");
    out.extend(wrap(PATHS, 2));
    out.join("\n")
}

/// `text` wrapped at word boundaries to `WIDTH`, every line indented by `indent`.
fn wrap(text: &str, indent: usize) -> Vec<String> {
    wrap_units(words(text), indent, indent)
}

/// Each word of `text`, as it prints and its width.
fn words(text: &str) -> impl Iterator<Item = (String, usize)> + '_ {
    text.split_whitespace().map(|w| (w.to_string(), w.chars().count()))
}

/// Units (as they print, their visible width) joined by spaces into lines of
/// at most `WIDTH` columns: the first indented by `first`, the rest by `rest`.
fn wrap_units(units: impl IntoIterator<Item = (String, usize)>, first: usize, rest: usize) -> Vec<String> {
    let mut lines: Vec<String> = vec![];
    let (mut cur, mut used, mut empty) = (" ".repeat(first), first, true);
    for (u, n) in units {
        if !empty && used + 1 + n > WIDTH {
            lines.push(std::mem::replace(&mut cur, " ".repeat(rest)));
            (used, empty) = (rest, true);
        }
        if !empty {
            cur.push(' ');
            used += 1;
        }
        cur.push_str(&u);
        used += n;
        empty = false;
    }
    if !empty {
        lines.push(cur);
    }
    lines
}

/// A synopsis, wrapped (its later lines indented further, never inside a
/// `[...]` or a quote), each word in its colour: the command and its
/// sub-verbs, the arguments, the flags.
fn synopsis(k: &Paint, syn: &str) -> Vec<String> {
    let mut units: Vec<(String, usize)> = vec![];
    let (mut unit, mut plain, mut depth, mut quoted) = (vec![], 0, 0i32, false);
    let mut prev_flag = false;
    for (i, w) in syn.split_whitespace().enumerate() {
        let flag = w.trim_start_matches(['[', '(']).starts_with('-');
        let verb = !flag && !prev_flag && w.chars().all(|c| c.is_ascii_lowercase() || c == '|' || c == '-');
        let colour = if flag {
            WARN
        } else if i == 0 || verb {
            STRONG
        } else {
            BUSY
        };
        prev_flag = flag && !w.ends_with(']') && !w.contains('|');
        unit.push(k.c(w, colour, i == 0));
        plain += w.chars().count() + usize::from(unit.len() > 1);
        depth += w.matches('[').count() as i32 - w.matches(']').count() as i32;
        quoted ^= w.matches('"').count() % 2 == 1;
        // a flag keeps its value on its line
        if depth <= 0 && !quoted && !prev_flag {
            units.push((unit.join(" "), plain));
            (unit, plain) = (vec![], 0);
        }
    }
    if !unit.is_empty() {
        units.push((unit.join(" "), plain));
    }
    wrap_units(units, 2, 6)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `s` without its colour escapes (`ESC [ ... m`).
    fn plain(s: &str) -> String {
        let mut out = String::new();
        let mut esc = false;
        for c in s.chars() {
            match (esc, c) {
                (false, '\x1b') => esc = true,
                (true, 'm') => esc = false,
                (true, _) => {}
                (false, c) => out.push(c),
            }
        }
        out
    }

    #[test]
    fn every_line_fits_in_80_columns() {
        let all = std::iter::once(usage(None)).chain(USAGES.iter().map(|u| usage(Some(u.name))));
        for text in all {
            for l in plain(&text).lines() {
                assert!(l.chars().count() <= WIDTH, "{} columns: {l}", l.chars().count());
            }
        }
    }

    #[test]
    fn wrapping_keeps_every_word_and_its_indent() {
        let text = "one two three ".repeat(20);
        let lines = wrap(&text, 6);
        assert!(lines.len() > 1 && lines.iter().all(|l| l.starts_with("      ") && l.len() <= WIDTH));
        assert_eq!(lines.join(" ").split_whitespace().count(), 60);
    }
}
