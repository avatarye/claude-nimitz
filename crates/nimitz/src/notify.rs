//! Needs-you delivery: post a blocked ship's notice into its commodore's
//! (an aide's, and its dead letters', into its parent's) inbox socket (a cross-session message from nimitz, `nimitz_core::inbox`),
//! else type it into the commodore's pane (the tab attached to its job), the
//! way `nimitz signal` reaches a lead.
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
    c.arg("__notify").arg(n.key()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // its own session: Claude Code may kill the hook's process group when the hook returns
    nimitz_core::os::detach(&mut c);
    if let Err(e) = c.spawn() {
        notify::set_delivery(env, n, "failed", Some(format!("could not start the delivery: {e}")));
    }
}

/// Start the aides' piggyback pass (`nimitz __ring`) when one is due (at
/// most every `ring::PIGGYBACK_EVERY_S` per machine) and return at once.
pub fn spawn_piggyback(env: &Env) {
    if !nimitz_core::ring::piggyback_due(env) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let mut c = Command::new(exe);
    c.arg("__ring").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    nimitz_core::os::detach(&mut c);
    let _ = c.spawn();
}

/// `nimitz __ring`: one piggyback pass (`ring::piggyback`: idle aides with
/// work waiting are rung, answers nobody waited for pushed) and each aide's
/// new dead letters told to its parent. Always exit 0.
pub fn piggyback_cmd() -> i32 {
    let vars: HashMap<String, String> = nimitz_core::os::env_vars();
    let env = Env::from_vars(&vars);
    let now = nimitz_core::evidence::now();
    let _ = nimitz_core::ring::piggyback(&env, now);
    let (fleets, _) = nimitz_core::config::load_all(&env);
    for f in &fleets {
        for a in f.config.aide.iter().filter(|a| f.machine.as_ref().is_some_and(|m| a.dirs.contains_key(m))) {
            if let Ok(q) = nimitz_core::queue::Queue::of(&env, &Id::aide(&Id::admiral(&f.config.fleet), &a.name)) {
                if q.dir.is_dir() {
                    dead_letters(&env, &q);
                }
            }
        }
    }
    0
}

/// `nimitz __notify <key>`: only the member itself (its labels) delivers its
/// notice; a dead letter's (`<aide>#dead-<item>`), whoever's command found
/// it dead (the text is nimitz's own, from the queue, never the caller's).
pub fn deliver_cmd(key: &str) -> i32 {
    let vars: HashMap<String, String> = nimitz_core::os::env_vars();
    let dead = key.contains("#dead-");
    match caller_from_env(&vars) {
        _ if dead => {}
        Caller::Member { id: me, role: Role::Ship | Role::Aide } if me.0 == key => {}
        _ => return 2,
    }
    let env = Env::from_vars(&vars);
    let Some(n) = notify::read(&env, key) else { return 0 };
    // once: a notice delivered (or given up on) is never sent again
    if n.delivered_at.is_some() || n.delivery != "pending" {
        return 0;
    }
    let retry: f64 = vars.get("NIMITZ_NOTIFY_RETRY").and_then(|v| v.parse().ok()).unwrap_or(600.0);
    let (delivery, note) = deliver(&env, &vars, &n, Duration::from_secs_f64(retry.max(0.0)));
    notify::set_delivery(&env, &n, delivery, Some(note));
    0
}

/// The lead's live session id: its own row in the snapshot (the job the
/// registry names, or the process that holds its conversation), never a
/// copy and never a stray session that only carries its name.
fn commodore_session(env: &Env, cid: &Id) -> Option<nimitz_core::SessionInfo> {
    let snap = nimitz_core::snapshot::snapshot(env, &mut nimitz_core::snapshot::Cache::default());
    let m = snap.fleets.iter().flat_map(|f| f.members()).find(|m| &m.id == cid)?;
    m.session.as_ref().filter(|s| s.pid.is_some() && s.copy.is_none()).cloned()
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
    let cid = notify::lead_of(&Id(n.id.clone()));
    let mux = delivery_mux(env, vars);
    let text = n.text();
    let deadline = Instant::now() + retry;
    loop {
        let why = match commodore_session(env, &cid) {
            None => format!("{cid} is not running"),
            Some(c) => match inbox_send(env, &cid, &c, &text) {
                // its inbox: no pane, no typing
                Ok(note) => return ("delivered", note),
                Err(no_inbox) if !mux.caps().type_text => format!("{no_inbox}; and no multiplexer in the member's env can type into {cid}'s pane ({}): only an explicit NIMITZ_MUX, or the Herdr server a nimitz command of the user recorded in active.json (by its session name), is used; fix: run nimitz status inside Herdr once", mux.name()),
                Err(no_inbox) => match mux.panes().into_iter().find(|p| p.session_id.as_deref() == Some(c.session_id.as_str())) {
                    None => format!("{no_inbox}; and {cid} is not live in a {} pane", mux.name()),
                    // a dialog of its own: typed text would answer it
                    Some(p) if p.state == "blocked" => format!("{no_inbox}; and {cid}'s pane {} is blocked on a dialog of its own", p.pane.pane_id),
                    // its attach pane, known by the foreground alone: typed into
                    // only while it runs exactly `claude attach <job>`, read just
                    // now (the agent view re-execs as `claude agents`, R-24)
                    Some(p) if p.inferred && !c.short_id.as_deref().is_some_and(|j| nimitz_mux::runs_exact_attach(mux.as_ref(), &p.pane, j)) => {
                        let job = c.short_id.as_deref().unwrap_or("?");
                        let check = c.short_id.as_deref().map(|j| nimitz_mux::attach_check(mux.as_ref(), &p.pane, j)).unwrap_or(nimitz_mux::AttachCheck::Other);
                        format!("{no_inbox}; and {cid}: {}: nothing is typed", check.refusal(&p.pane.pane_id, job))
                    }
                    Some(p) => match mux.type_text(&p.pane, &text) {
                        Ok(()) if p.inferred && nimitz_mux::typed_into_view(mux.as_ref(), &p.pane) => return ("failed", format!("typed into {cid}'s pane {}, but the pane runs the agent view now (an /exit raced the typing): check it for a dispatched session ({no_inbox})", p.pane.pane_id)),
                        Ok(()) => return ("delivered", format!("typed into {cid}'s pane {} ({no_inbox})", p.pane.pane_id)),
                        Err(e) => format!("{no_inbox}; and typing into {cid}'s pane {} failed: {e}", p.pane.pane_id),
                    },
                },
            },
        };
        // answered or replaced meanwhile: nothing left to deliver
        if !notify::read(env, &n.key()).is_some_and(|c| notify::is_same(&c, n)) {
            return ("delivered", "the member moved on before delivery".into());
        }
        if Instant::now() + POLL > deadline {
            return ("failed", why);
        }
        std::thread::sleep(POLL);
    }
}

/// Post `text` into the live session `s` of `id` through its inbox socket
/// (`nimitz_core::inbox`): `Ok(note)` when the line went in, else why not
/// (no socket: the job is stopped or retired, or its Claude Code predates
/// the line nimitz knows).
pub fn inbox_send(env: &Env, id: &Id, s: &nimitz_core::SessionInfo, text: &str) -> Result<String, String> {
    let (socks, pid) = inbox_target(env, id, s)?;
    nimitz_core::inbox::send(env, &socks, pid, text).map(|sock| format!("posted to {id}'s inbox {} (a cross-session message from nimitz)", sock.display()))
}

/// The sockets `id`'s live session `s` may listen on, and its pid (`send`
/// takes only the one its own process listens on), or why there is none.
pub fn inbox_target(env: &Env, id: &Id, s: &nimitz_core::SessionInfo) -> Result<(Vec<std::path::PathBuf>, i64), String> {
    let version = nimitz_core::inbox::receiver_version(env, s.pid, s.short_id.as_deref());
    if !nimitz_core::inbox::format_ok(version.as_deref()) {
        return Err(format!(
            "{id} runs Claude Code {}, older than {} where nimitz verified the inbox line",
            version.as_deref().unwrap_or("(version unknown)"),
            nimitz_core::inbox::FORMAT_SINCE
        ));
    }
    let pid = s.pid.ok_or_else(|| format!("{id} is not live"))?;
    let socks = nimitz_core::inbox::candidates(env, &s.session_id, pid);
    if socks.is_empty() {
        return Err(format!("{id} has no inbox socket (no live process listens for it)"));
    }
    Ok((socks, pid))
}

/// Tell an aide's parent of each dead letter in its inbox not told yet
/// (aide.md 5.3): after any inbox command, since a `fail` at its last try, a
/// `--permanent` one, or a lease that ran out at its last try (housekeeping
/// in any command) may have made one. Read only; the notice is recorded once.
pub fn dead_letters(env: &Env, q: &nimitz_core::queue::Queue) {
    for d in q.peek_status(nimitz_core::evidence::now(), false).dead {
        if let Some(n) = notify::record(env, notify::dead_letter(&q.aide, &d)) {
            spawn_delivery(env, &n);
        }
    }
}
