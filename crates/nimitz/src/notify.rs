//! Needs-you delivery: type a blocked ship's notice into its
//! commodore's pane (the tab attached to the commodore's job), the way
//! `nimitz signal` reaches a lead.
//!
//! The hook records the notice (`nimitz_core::notify`) and starts
//! `nimitz __notify <id>` detached, in its own session, so the member never
//! waits. Delivery retries while the commodore's pane is busy with a dialog of
//! its own (typing would answer it) or not found, for `NIMITZ_NOTIFY_RETRY`
//! seconds (default 600), and writes the outcome into the notice: `nimitz check`
//! reports one that failed.
use nimitz_core::env::Env;
use nimitz_core::notify::{self, Notice};
use nimitz_core::sessions::caller_from_env;
use nimitz_core::{Caller, Id, Role};
use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(5);

/// Start the delivery and return at once. It inherits the hook's env (the
/// member's labels and its multiplexer variables).
pub fn spawn_delivery(env: &Env, n: &Notice) {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return notify::set_delivery(env, n, "failed", Some(format!("could not find the nimitz binary: {e}"))),
    };
    let mut c = Command::new(exe);
    c.arg("__notify").arg(&n.id).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // its own session: Claude Code may kill the hook's process group when the hook returns
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    if let Err(e) = c.spawn() {
        notify::set_delivery(env, n, "failed", Some(format!("could not start the delivery: {e}")));
    }
}

/// `nimitz __notify <id>`: only the member itself (its labels) delivers its notice.
pub fn deliver_cmd(id: &str) -> i32 {
    let vars: HashMap<String, String> = std::env::vars().collect();
    match caller_from_env(&vars) {
        Caller::Member { id: me, role: Role::Ship } if me.0 == id => {}
        _ => return 2,
    }
    let env = Env::from_vars(&vars);
    let Some(n) = notify::read(&env, id) else { return 0 };
    let retry: f64 = vars.get("NIMITZ_NOTIFY_RETRY").and_then(|v| v.parse().ok()).unwrap_or(600.0);
    let (delivery, note) = deliver(&env, &vars, &n, Duration::from_secs_f64(retry.max(0.0)));
    notify::set_delivery(&env, &n, delivery, Some(note));
    0
}

/// The commodore's live session id: its own row in the snapshot (the job
/// the registry names, or the process that holds its conversation), never a
/// copy and never a stray session that only carries its name.
fn commodore_session(env: &Env, cid: &Id) -> Option<String> {
    let snap = nimitz_core::snapshot::snapshot(env, &mut nimitz_core::snapshot::Cache::default());
    let m = snap.fleets.iter().flat_map(|f| f.squadrons.iter()).map(|sq| &sq.commodore).find(|m| &m.id == cid)?;
    m.session.as_ref().filter(|s| s.pid.is_some() && s.copy.is_none()).map(|s| s.session_id.clone())
}

/// The multiplexer a delivery types into. An explicit setting (`NIMITZ_MUX`
/// other than auto, `NIMITZ_HERDR_SESSION`) is the member's env as given.
/// Else the delivery never goes by inherited `HERDR_*` alone: a
/// daemon-respawned member inherits the daemon's env, the user's live
/// Herdr, in whatever world its `NIMITZ_HOME` names (acc-D F6). The server
/// is the one the user's own nimitz commands recorded in `active.json`
/// (`active::herdr_identity`: a member's command records none), named as
/// the `--session` of every call; with nothing recorded, no multiplexer.
fn delivery_mux(env: &Env, vars: &HashMap<String, String>) -> Box<dyn nimitz_mux::Mux> {
    nimitz_mux::detect(&nimitz_core::active::member_mux_vars(env, vars))
}

fn deliver(env: &Env, vars: &HashMap<String, String>, n: &Notice, retry: Duration) -> (&'static str, String) {
    let id = Id(n.id.clone());
    let cid = Id::commodore(id.fleet(), id.squadron().unwrap_or_default());
    let mux = delivery_mux(env, vars);
    if !mux.caps().type_text {
        return ("failed", format!("no multiplexer in the member's env can type into {cid}'s pane ({}); only an explicit NIMITZ_MUX, or the Herdr server a nimitz command of the user recorded in active.json (by its session name), is used; fix: run nimitz status inside Herdr once", mux.name()));
    }
    let text = n.text();
    let deadline = Instant::now() + retry;
    loop {
        let why = match commodore_session(env, &cid) {
            None => format!("{cid} is not running"),
            Some(sid) => match mux.panes().into_iter().find(|p| p.session_id.as_deref() == Some(sid.as_str())) {
                None => format!("{cid} is not live in a {} pane", mux.name()),
                // a dialog of its own: typed text would answer it
                Some(p) if p.state == "blocked" => format!("{cid}'s pane {} is blocked on a dialog of its own", p.pane.pane_id),
                // its attach pane, read from the foreground alone: the agent
                // view looks the same, and typed text would dispatch a session
                Some(p) if p.inferred => format!("{cid}'s pane {} runs its attach, but Herdr names no session there, so nimitz cannot tell it from the agent view (R-24); nothing is typed", p.pane.pane_id),
                Some(p) => match mux.type_text(&p.pane, &text) {
                    Ok(()) => return ("delivered", format!("typed into {cid}'s pane {}", p.pane.pane_id)),
                    Err(e) => format!("typing into {cid}'s pane {} failed: {e}", p.pane.pane_id),
                },
            },
        };
        // answered or replaced meanwhile: nothing left to deliver
        if !notify::read(env, &n.id).is_some_and(|c| notify::is_same(&c, n)) {
            return ("delivered", "the member moved on before delivery".into());
        }
        if Instant::now() + POLL > deadline {
            return ("failed", why);
        }
        std::thread::sleep(POLL);
    }
}
