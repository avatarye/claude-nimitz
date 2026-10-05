//! `nimitz`: the fleet command line and the hook binary (contracts.md section 10).
//!
//! Every rule lives in nimitz-core; this crate parses arguments, prints plans,
//! asks before it changes anything, and opens interactive members in a
//! multiplexer pane (core never depends on the mux).
mod args;
mod crud;
mod members;
mod notify;
mod out;
mod run;
mod sandbox_cmd;
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
    pub fn mux(&self) -> Box<dyn nimitz_mux::Mux> {
        nimitz_mux::detect(&self.vars)
    }
    pub fn snap(&self) -> Snapshot {
        nimitz_core::snapshot::snapshot(&self.env, &mut nimitz_core::snapshot::Cache::default())
    }
    /// The gate, with this command's `--plan` / `--yes`.
    pub fn gate(&self, p: &nimitz_core::Plan) -> out::Go {
        let mut shown = p.clone();
        run::fixes_for(&self.caller, &mut shown);
        out::gate(&shown, self.plan_only(), self.yes())
    }
}

const USAGE: &str = "\
usage: nimitz <command> [args] [--json] [--plan] [--yes] [--force]
       nimitz --version

  init [--label L]                 data root, machine label, trust check, doctor
  doctor [--fix]                   hooks, crossSessionInbound, native claude, versions, formats, role briefs
  rules [<id>]                     the role rules nimitz hands a member at session start
  status [--json]                  the fleets, their members and issues
  check [<id>]                     integrity check of <id>, its parents and everything under it
  fleet new|set|rm <F>             fleet config (new: --machine L; set: --admiral-mode M)
  squadron new|set|rm <F/SQ>       --brief T --comms open|commodore-only|allow:A,B --mode M
  ship new|set|rm <F/SQ/SHIP>      --dir PATH|managed --machine L --prompt T|--prompt-file P --mode M --depends A,B
  machine add|rm <F> <label>       add: --hostname H (another machine), --dir F/SQ/SHIP=PATH (repeat);
                                   rm warns about live members when <label> is this machine
  up <F>                           start every member of a fleet (needs a multiplexer)
  start|stop|relaunch <id>         one member; start brings up missing parents first
  rm <id|session>                  remove job rows (never transcripts)
  sweep [<id>]                     drop stale copy rows
  tender <F/SQ> <task> <prompt>    one background job for a commodore
  adopt <session> <id>             make a stowaway an identity's session
  signal <id> <text>               type to a member (ships: through the commodore)
  open <id>                        show a member in a pane
  layout herdr [<F>]               workspaces and tabs for fleets in Herdr
  migrate fleet-skill <dir...> --into <F>
  hook <Event>                     Claude Code hook (reads stdin, always exits 0)
  sandbox create|tick|env <dir>    a fake world for tests and demos

ids: F (admiral), F/SQ (commodore), F/SQ/SHIP (ship), F/SQ/~task (tender)";

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(dispatch(argv));
}

fn dispatch(argv: Vec<String>) -> i32 {
    let cmd = argv.first().cloned().unwrap_or_default();
    // The hook and the fakes come first and stay lean: no parsing, no snapshot.
    match cmd.as_str() {
        "hook" => return hook(argv.get(1).map(String::as_str).unwrap_or("")),
        "__notify" => return notify::deliver_cmd(argv.get(1).map(String::as_str).unwrap_or("")),
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
            println!("{USAGE}");
            return if cmd.is_empty() { 2 } else { 0 };
        }
        _ => {}
    }
    let args = match Args::parse(&argv[1..]) {
        Ok(a) => a,
        Err(e) => return usage_error(&e),
    };
    out::set_json(args.flag("--json"));
    let vars: HashMap<String, String> = std::env::vars().collect();
    if let Err(e) = nimitz_mux::valid_mux_setting(&vars) {
        return usage_error(&e);
    }
    let ctx = Ctx { env: Env::from_vars(&vars), caller: nimitz_core::sessions::caller_from_env(&vars), vars, args };
    let r = match cmd.as_str() {
        "init" => setup::init(&ctx),
        "doctor" => setup::doctor(&ctx),
        "rules" => setup::rules(&ctx),
        "status" => members::status(&ctx),
        "check" => members::check(&ctx),
        "fleet" => crud::fleet(&ctx),
        "squadron" => crud::squadron(&ctx),
        "ship" => crud::ship(&ctx),
        "machine" => crud::machine(&ctx),
        "migrate" => crud::migrate(&ctx),
        "up" => members::up(&ctx),
        "start" => members::start(&ctx),
        "stop" => members::stop(&ctx),
        "relaunch" => members::relaunch(&ctx),
        "rm" => members::rm(&ctx),
        "sweep" => members::sweep(&ctx),
        "tender" => members::tender(&ctx),
        "adopt" => members::adopt(&ctx),
        "signal" => members::signal(&ctx),
        "open" => members::open(&ctx),
        "layout" => members::layout(&ctx),
        "sandbox" => sandbox_cmd::sandbox(&ctx),
        other => Err(format!("unknown command {other:?}")),
    };
    match r {
        Ok(code) => code,
        Err(e) => usage_error(&e),
    }
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
    let vars: HashMap<String, String> = std::env::vars().collect();
    let env = Env::from_vars(&vars);
    let (out, notice) = nimitz_core::hook::handle_full(&env, event, &stdin, &vars);
    if let Some(n) = notice {
        notify::spawn_delivery(&env, &n);
    }
    if !out.is_empty() {
        println!("{out}");
    }
    0
}
