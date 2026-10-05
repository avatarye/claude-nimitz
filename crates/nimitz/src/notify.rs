//! Needs-you delivery: type a blocked ship's or tender's notice into its
//! commodore's pane, the way `nimitz signal` reaches an interactive member.
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
        Caller::Member { id: me, role: Role::Ship | Role::Tender } if me.0 == id => {}
        _ => return 2,
    }
    let env = Env::from_vars(&vars);
    let Some(n) = notify::read(&env, id) else { return 0 };
    let retry: f64 = vars.get("NIMITZ_NOTIFY_RETRY").and_then(|v| v.parse().ok()).unwrap_or(600.0);
    let (delivery, note) = deliver(&env, &vars, &n, Duration::from_secs_f64(retry.max(0.0)));
    notify::set_delivery(&env, &n, delivery, Some(note));
    0
}

/// The commodore's live session id: the registry's current binding, else a
/// live interactive session holding the commodore's name.
fn commodore_session(env: &Env, cid: &Id) -> Option<String> {
    let rows = nimitz_core::evidence::rows_from_files(env, &mut Default::default()).unwrap_or_default();
    let live = |sid: &str| rows.iter().any(|r| r.session_id == sid && r.live());
    let reg = nimitz_core::registry::load(env);
    if let Some(cur) = reg.identities.get(&cid.0).and_then(|r| r.current.clone()).filter(|s| live(s)) {
        return Some(cur);
    }
    rows.iter().find(|r| r.live() && r.kind == "interactive" && r.name == cid.name()).map(|r| r.session_id.clone())
}

fn deliver(env: &Env, vars: &HashMap<String, String>, n: &Notice, retry: Duration) -> (&'static str, String) {
    let id = Id(n.id.clone());
    let cid = Id::commodore(id.fleet(), id.squadron().unwrap_or_default());
    let mux = nimitz_mux::detect(vars);
    if !mux.caps().type_text {
        return ("failed", format!("no multiplexer in the member's env can type into {cid}'s pane ({})", mux.name()));
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
                Some(p) => match mux.type_text(&p.pane, &text) {
                    Ok(()) => return ("delivered", format!("typed into {cid}'s pane {}", p.pane.pane_id)),
                    Err(e) => format!("typing into {cid}'s pane {} failed: {e}", p.pane.pane_id),
                },
            },
        };
        // answered or replaced meanwhile: nothing left to deliver
        if notify::read(env, &n.id).map(|c| c.ts) != Some(n.ts) {
            return ("delivered", "the member moved on before delivery".into());
        }
        if Instant::now() + POLL > deadline {
            return ("failed", why);
        }
        std::thread::sleep(POLL);
    }
}
