//! `nimitz`: the fleet command line and the hook binary (contracts.md section 10).
//!
//! Every rule lives in nimitz-core; this crate parses arguments, prints plans,
//! asks before it changes anything, and attaches to members in multiplexer
//! tabs (core never depends on the mux).
mod args;
mod crud;
mod harness_cmd;
mod help;
mod kb_cmd;
mod members;
mod notify;
mod out;
mod restore;
mod run;
mod view;
mod sandbox_cmd;
mod self_end;
mod setup;

use args::Args;
use nimitz_core::env::Env;
use nimitz_core::{Caller, Snapshot};
use std::collections::HashMap;
use std::io::Read;

/// What every command gets: the env, the caller and the parsed arguments.
pub struct Ctx {
    pub env: Env,
    pub vars: HashMap<String, String>,
    pub caller: Caller,
    pub args: Args,
}

impl Ctx {
    pub fn plan_only(&self) -> bool {
        self.args.flag("--plan")
    }
    pub fn yes(&self) -> bool {
        self.args.flag("--yes")
    }
    pub fn force(&self) -> bool {
        self.args.flag("--force")
    }
    /// The user's own command detects the multiplexer from its env; a
    /// member's goes by `active::member_mux_vars` (its env may be the
    /// daemon's, with the user's live `HERDR_*`).
    pub fn mux(&self) -> Box<dyn nimitz_mux::Mux> {
        match self.caller {
            nimitz_core::Caller::User => nimitz_mux::detect(&nimitz_core::active::user_mux_vars(&self.env, &self.vars)),
            _ => nimitz_mux::detect(&nimitz_core::active::member_mux_vars(&self.env, &self.vars)),
        }
    }
    pub fn snap(&self) -> Snapshot {
        nimitz_core::snapshot::snapshot(&self.env, &mut nimitz_core::snapshot::Cache::default())
    }
    /// The gate, with this command's `--plan` / `--yes`.
    pub fn gate(&self, p: &nimitz_core::Plan) -> out::Go {
        out::gate(p, self.plan_only(), self.yes())
    }
}

fn main() {
    let argv: Vec<String> = nimitz_core::os::args();
    // a closed pipe (`nimitz status --json | head`) ends a command quietly, as
    // every command-line tool does (SIGPIPE, exit 141), never with a panic.
    // Not a hook (it always exits 0, whatever Claude Code does with its
    // pipe) nor the internal fakes: there the write error is ignored instead
    #[cfg(unix)]
    let quiet_pipe = !matches!(argv.first().map(String::as_str), Some("hook" | "__notify" | "__self-end" | "__fake-claude" | "__fake-herdr"));
    #[cfg(unix)]
    if quiet_pipe {
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
    }
    std::process::exit(dispatch(argv));
}

fn dispatch(argv: Vec<String>) -> i32 {
    let cmd = argv.first().cloned().unwrap_or_default();
    // `<cmd> --help`: its own usage, before anything reads its arguments (the
    // hook's stdin included): `--help` first after the verb, or its only flag
    // (never inside `signal`'s text)
    let flags: Vec<&String> = argv.iter().skip(1).filter(|a| a.starts_with("--")).collect();
    let asks = argv.get(1).is_some_and(|a| a == "--help") || (cmd != "signal" && flags.len() == 1 && flags[0] == "--help");
    if asks && help::known(&cmd) {
        println!("{}", help::usage(Some(&cmd)));
        return 0;
    }
    // The hook and the fakes come first and stay lean: no parsing, no snapshot.
    match cmd.as_str() {
        "hook" => return hook(argv.get(1).map(String::as_str).unwrap_or("")),
        "__notify" => return notify::deliver_cmd(argv.get(1).map(String::as_str).unwrap_or("")),
        "__self-end" => return self_end::helper(&argv[1..]),
        "__fake-claude" => {
            return match argv.get(1) {
                Some(base) => nimitz_core::sandbox::fake_claude(std::path::Path::new(base), &argv[2..]),
                None => 2,
            }
        }
        "__fake-herdr" => {
            return match argv.get(1) {
                Some(base) => nimitz_mux::fake::fake_herdr(std::path::Path::new(base), &argv[2..]),
                None => 2,
            }
        }
        "--version" | "-V" | "version" => {
            println!("nimitz {}", env!("CARGO_PKG_VERSION"));
            return 0;
        }
        "" | "help" | "--help" | "-h" => {
            let of = argv.get(1).map(String::as_str).filter(|_| cmd == "help");
            if let Some(c) = of.filter(|c| !help::known(c)) {
                return usage_error(&format!("no command {c:?}"));
            }
            println!("{}", help::usage(of));
            return if cmd.is_empty() { 2 } else { 0 };
        }
        _ => {}
    }
    // `-v` is `--verbose` for every command but `signal` (whose text it may be)
    // (never after `--`, never a flag's value: `--prompt -v` keeps its text)
    let mut rest: Vec<String> = vec![];
    let mut literal = false;
    for (i, a) in argv[1..].iter().enumerate() {
        let after_value_flag = i > 0 && args::takes_value(&argv[i]);
        literal |= a == "--";
        rest.push(if a == "-v" && cmd != "signal" && !literal && !after_value_flag { "--verbose".to_string() } else { a.clone() });
    }
    let args = match Args::parse(&rest) {
        Ok(a) => a,
        Err(e) => return usage_error(&e),
    };
    out::set_json(args.flag("--json"));
    out::set_verbose(args.flag("--verbose") || std::env::var("NIMITZ_VERBOSE").is_ok_and(|v| v == "1"));
    let vars: HashMap<String, String> = nimitz_core::os::env_vars();
    if let Err(e) = nimitz_mux::valid_mux_setting(&vars) {
        return usage_error(&e);
    }
    // partial or mismatched labels: said once; the command runs, as a caller
    // that is no member by name (its own member, for a self-end, is found by
    // its process; its signal goes to the inbox)
    let caller = nimitz_core::sessions::caller_from_env(&vars);
    if matches!(&caller, Caller::Member { id, .. } if id.0 == nimitz_core::sessions::UNVERIFIED) {
        eprintln!("[WARN] this process's NIMITZ_ID / NIMITZ_ROLE labels are partial or disagree; nimitz runs it with no member's name");
    }
    let ctx = Ctx { env: Env::from_vars(&vars), caller, vars, args };
    // a registry too new for this nimitz stops a command at its first read,
    // never reads as an empty one (doctor reports it instead)
    nimitz_core::registry::set_strict(cmd != "doctor");
    if migrates(&cmd) {
        if let Some(code) = auto_migrate(&ctx.env, &cmd) {
            return code;
        }
    }
    let r = match COMMANDS.iter().find(|(name, _)| *name == cmd) {
        Some((_, run)) => run(&ctx),
        None => Err(format!("unknown command {cmd:?}")),
    };
    match r {
        Ok(code) => code,
        Err(e) => usage_error(&e),
    }
}

/// Every command that takes a `Ctx` (parsed args, the caller from the
/// labels), in usage order (docs/design/cli-ux.md section 2: fleet shape,
/// session control, pane control, the rest). The hidden `__commands` prints
/// the names, so the role test runs every one of them as every role (none is
/// refused on authority, 0.7.3), and the usage test finds each one's usage.
const COMMANDS: &[(&str, fn(&Ctx) -> Result<i32, String>)] = &[
    ("init", setup::init),
    ("trust", setup::trust),
    ("doctor", setup::doctor),
    ("rules", setup::rules),
    ("status", members::status),
    ("check", members::check),
    ("fleet", crud::fleet),
    ("squadron", crud::squadron),
    ("ship", crud::ship),
    ("machine", crud::machine),
    ("settings", kb_cmd::settings),
    ("harness", harness_cmd::harness),
    ("kb", kb_cmd::kb),
    ("migrate", crud::migrate),
    ("start", members::start),
    ("restart", members::restart),
    ("stop", members::stop),
    ("rm", members::rm),
    ("sweep", members::sweep),
    ("adopt", members::adopt),
    ("signal", members::signal),
    ("open", members::open),
    ("attach", members::attach),
    ("close", members::close),
    ("layout", members::layout),
    ("sandbox", sandbox_cmd::sandbox),
    ("muster", restore::muster),
    ("__commands", |_| {
        for (name, _) in COMMANDS {
            if !name.starts_with("__") {
                println!("{name}");
            }
        }
        Ok(0)
    }),
];

/// Does `cmd` migrate this machine's files first? Not the hook (Claude
/// Code's hot path; it reads the older formats as they are) nor the
/// delivery it starts, not `sandbox` (it makes a world, maybe from a shell
/// whose env points at the real state), not `migrate` (it shows and runs the
/// steps itself), and not the hidden helpers.
fn migrates(cmd: &str) -> bool {
    !matches!(cmd, "hook" | "__notify" | "sandbox" | "migrate") && !cmd.starts_with("__")
}

/// The automatic local migration (docs/design/migrations.md section 4): the
/// manifest says this nimitz ran here already, and that is all (one small
/// read); else the pending local steps run, and one line on stderr says what
/// moved. Then this machine's record in its fleets. A file a newer nimitz
/// wrote here stops every command but `doctor` (which reports it).
fn auto_migrate(env: &Env, cmd: &str) -> Option<i32> {
    use nimitz_core::formats;
    let ran = formats::migrate_local(env);
    if let Some(line) = ran.line() {
        eprintln!("{line}");
    }
    if !ran.too_new.is_empty() && cmd != "doctor" {
        for t in &ran.too_new {
            eprintln!("[ERROR] {t}");
        }
        eprintln!("[ERROR] nimitz {} stops: a newer nimitz ran on this machine, and its files are not this one's to read or write (never downgrade after it ran); `nimitz doctor` lists them", formats::NIMITZ);
        return Some(1);
    }
    // on record in the fleets only once this machine's files are this nimitz's
    if ran.failed.is_empty() && ran.too_new.is_empty() && formats::up_to_date(env) {
        formats::record_machine(env);
    }
    None
}

fn usage_error(e: &str) -> i32 {
    out::error(e);
    eprintln!("run `nimitz help` for usage");
    2
}

/// `nimitz hook <Event>`: read stdin, log the event, print a decision if any.
/// A needs-you notice goes to a detached delivery process, so the session never
/// waits for it. Always exit 0: a hook must never fail the session.
fn hook(event: &str) -> i32 {
    let mut stdin = String::new();
    let _ = std::io::stdin().read_to_string(&mut stdin);
    let vars: HashMap<String, String> = nimitz_core::os::env_vars();
    let env = Env::from_vars(&vars);
    // the KB agents' guard (on every tool call): an answer for them, nothing else
    if event == "kb-guard" {
        let out = nimitz_core::kb_guard::hook(&env, &stdin, &vars);
        if !out.is_empty() {
            use std::io::Write;
            let _ = writeln!(std::io::stdout(), "{out}");
        }
        return 0;
    }
    let (out, notice) = nimitz_core::hook::handle_full(&env, event, &stdin, &vars);
    if let Some(n) = notice {
        notify::spawn_delivery(&env, &n);
    }
    if !out.is_empty() {
        // a pipe Claude Code already closed is no failure of the hook
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), "{out}");
    }
    0
}
