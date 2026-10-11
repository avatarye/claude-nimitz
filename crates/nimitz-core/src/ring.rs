//! The doorbell (aide.md section 5.1): a peer line through an aide's own
//! messaging socket that it has work. Three sources ring: a post (at once,
//! coalesced), the piggyback (`nimitz __ring`: work waiting while the aide
//! sits idle), and nothing else; the SessionStart context and the aide's own
//! 15-minute cron are the other two wakes and need no ring.
//!
//! `<queue>/ring.json` keeps the last ring and the last time the aide found
//! its inbox empty (a `claim` with nothing to hand out): a post rings unless
//! a ring under a minute old has not been drained since, so a burst of posts
//! rings once.
use crate::env::Env;
use crate::queue::Queue;
use crate::types::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A post rings again only after this long, or after the aide drained.
pub const COALESCE_S: f64 = 60.0;
/// The piggyback rings when the oldest pending item is this old, the aide is
/// live and idle, and no ring went out this long ago (`ring_after`, 2 min).
pub const RING_AFTER_S: f64 = 120.0;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RingState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ring: Option<f64>,
    /// the last `claim` that found the inbox empty
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drained_at: Option<f64>,
}

fn path(q: &Queue) -> std::path::PathBuf {
    q.dir.join("ring.json")
}

pub fn load(q: &Queue) -> RingState {
    std::fs::read(path(q)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(q: &Queue, s: &RingState) {
    // best effort: a lost write costs at most one extra ring
    if q.dir.is_dir() {
        if let Ok(t) = serde_json::to_string(s) {
            let _ = crate::config::atomic_write(&path(q), &t);
        }
    }
}

/// The aide found its inbox empty (`claim` exit 3): the next post rings at once.
pub fn drained(q: &Queue, now: f64) {
    let mut s = load(q);
    s.drained_at = Some(now);
    save(q, &s);
}

/// The aide's live session on this machine: (session id, pid, status), by
/// the registry's job and the session file that runs it.
pub fn aide_session(env: &Env, aide: &Id) -> Option<(String, i64, String)> {
    let job = crate::registry::load(env).identities.get(&aide.0)?.job.clone()?;
    for e in std::fs::read_dir(env.claude_home.join("sessions")).into_iter().flatten().flatten() {
        let Some(v) = std::fs::read(e.path()).ok().and_then(|b| crate::evidence::first_json(&b)) else { continue };
        let pid = v.get("pid").and_then(Value::as_i64).unwrap_or(0);
        if v.get("jobId").and_then(Value::as_str) == Some(job.as_str()) && pid > 1 && crate::evidence::pid_alive(pid) {
            let sid = v.get("sessionId").and_then(Value::as_str).unwrap_or_default().to_string();
            let status = if v.get("waitingFor").is_some_and(|w| !w.is_null()) { "waiting".to_string() } else { v.get("status").and_then(Value::as_str).unwrap_or("?").to_string() };
            return Some((sid, pid, status));
        }
    }
    None
}

/// Why it rings.
pub enum Why<'a> {
    /// a new item (its type, its sender as shown)
    Post { kind: &'a str, from: &'a str },
    /// the piggyback: work waits while the aide is idle
    Waiting { pending: usize, oldest_s: f64 },
}

/// Ring the aide if the rules say so: a post unless a ring under
/// `COALESCE_S` old has not been drained since; the piggyback only while the
/// aide is idle, its oldest item past `RING_AFTER_S` and no ring within it.
/// `Ok(None)` when it holds back (and why is not worth a word); `Err` when it
/// should ring but cannot reach the aide.
pub fn ring(env: &Env, q: &Queue, why: Why, now: f64) -> Result<Option<String>, String> {
    let st = load(q);
    let since = st.last_ring.map(|t| now - t);
    let drained_since = match (st.drained_at, st.last_ring) {
        (Some(d), Some(r)) => d >= r,
        (_, None) => true,
        (None, Some(_)) => false,
    };
    let aide = q.aide.clone();
    let (sid, pid, status) = aide_session(env, &aide).ok_or_else(|| format!("{aide} is not live here: it wakes at its start"))?;
    let text = match why {
        Why::Post { kind, from } => {
            if since.is_some_and(|s| s < COALESCE_S) && !drained_since {
                return Ok(None);
            }
            format!("[nimitz inbox] {aide}: 1 new item ({kind}, from {from}). Drain your inbox.")
        }
        Why::Waiting { pending, oldest_s } => {
            if pending == 0 || oldest_s < RING_AFTER_S || since.is_some_and(|s| s < RING_AFTER_S) || status != "idle" {
                return Ok(None);
            }
            format!("[nimitz inbox] {aide}: {pending} item(s) waiting, the oldest {}m. Drain your inbox.", (oldest_s / 60.0).floor() as i64)
        }
    };
    if !crate::inbox::format_ok(crate::inbox::receiver_version(env, Some(pid), None).as_deref()) {
        return Err(format!("{aide}'s Claude Code predates the message line"));
    }
    let socks = crate::inbox::candidates(env, &sid, pid);
    if socks.is_empty() {
        return Err(format!("no messaging socket for {aide}"));
    }
    crate::inbox::send(env, &socks, pid, &text).map_err(|e| format!("{aide}: {e}"))?;
    save(q, &RingState { last_ring: Some(now), ..st });
    Ok(Some(format!("rang {aide}")))
}

/// The piggyback's other duty (aide.md section 13, from S1): a done request
/// whose asker stopped waiting (`ask --wait` killed or timed out) and was
/// never tried gets one push to its live session.
pub fn redeliver(env: &Env, q: &Queue, now: f64) -> Vec<String> {
    let mut out = vec![];
    for (_, item) in q.list(Some(crate::queue::State::Done)) {
        let Some(r) = item.reply_to.as_ref() else { continue };
        let stopped_waiting = r.waiting_until.is_some_and(|w| w <= now);
        if stopped_waiting && r.delivered_at.is_none() && r.delivery.is_none() {
            if let Ok(note) = crate::queue::deliver(env, q, &item, now) {
                out.push(format!("{} item {}: {note}", q.aide, item.id));
            }
        }
    }
    out
}

/// One piggyback pass over every aide this machine runs: ring the idle ones
/// with work waiting, push answers nobody waited for. Never fails.
pub fn piggyback(env: &Env, now: f64) -> Vec<String> {
    let mut out = vec![];
    let (fleets, _) = crate::config::load_all(env);
    for f in &fleets {
        for a in &f.config.aide {
            if f.machine.as_ref().is_none_or(|m| !a.dirs.contains_key(m)) {
                continue; // another machine runs it
            }
            let id = Id::aide(&Id::admiral(&f.config.fleet), &a.name);
            let Ok(q) = Queue::of(env, &id) else { continue };
            if !q.dir.is_dir() {
                continue; // never ran here: nothing waits
            }
            let st = q.peek_status(now, false);
            match ring(env, &q, Why::Waiting { pending: st.pending, oldest_s: st.oldest_age_s.unwrap_or(0.0) }, now) {
                Ok(Some(n)) => out.push(n),
                Ok(None) | Err(_) => {}
            }
            out.extend(redeliver(env, &q, now));
        }
    }
    out
}

/// The piggyback runs at most this often per machine.
pub const PIGGYBACK_EVERY_S: u64 = 30;

/// Is a piggyback due (its stamp older than `PIGGYBACK_EVERY_S`)? Takes the
/// turn: the stamp is touched, so a burst of hooks starts one pass.
pub fn piggyback_due(env: &Env) -> bool {
    let stamp = env.state.join("ring.stamp");
    let fresh = std::fs::metadata(&stamp).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|e| e.as_secs() < PIGGYBACK_EVERY_S);
    if fresh || !env.state.is_dir() {
        return false;
    }
    std::fs::write(&stamp, b"").is_ok()
}
