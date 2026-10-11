//! `nimitz inbox ...` and `nimitz ask`: an aide's durable inbox
//! (`nimitz_core::queue`, docs/design/aide.md section 4).
//!
//! Who may do what (aide.md section 7): post and ask by the inbox's policy;
//! claim, heartbeat, done and fail the aide itself (and the user, for
//! repairs); retry and drop the user or the aide's parent; a result the one
//! who asked (same member, session or job, or the reply token) and the user;
//! list and status anyone, with others' payloads and results hidden.
use crate::out;
use crate::Ctx;
use nimitz_core::evidence::now;
use nimitz_core::queue::{self, Policy, Queue, Refusal, Sender, State};
use nimitz_core::types::Id;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// The default and the cap of `--wait` (aide.md 4.6): the default stays
/// under Claude Code's own 120 s Bash timeout, so an agent's `ask` returns
/// before its shell is killed.
const WAIT_DEFAULT: f64 = 100.0;
const WAIT_MAX: f64 = 600.0;

fn refuse(r: &Refusal) -> i32 {
    if out::json_mode() {
        out::json(&json!({"ok": false, "code": r.code, "message": r.message}));
    } else {
        out::error(&format!("{} ({})", r.message, r.code));
    }
    r.exit
}

fn authority(msg: String) -> Refusal {
    Refusal { code: "authority", message: msg, exit: 1 }
}

fn usage(msg: String) -> Refusal {
    Refusal { code: "usage", message: msg, exit: 2 }
}

fn aide(ctx: &Ctx, i: usize) -> Result<Id, Refusal> {
    let s = ctx.args.need(i, "aide id (F/@NAME)").map_err(usage)?;
    queue::aide_id(s).map_err(usage)
}

fn open(ctx: &Ctx, id: &Id) -> Result<Queue, Refusal> {
    Queue::of(&ctx.env, id).map_err(|e| Refusal { code: "inbox.unknown", message: e, exit: 1 })
}

fn me(ctx: &Ctx) -> Sender {
    queue::sender_of(&ctx.env, &ctx.vars, ctx.args.value("--from"))
}

/// Who holds a lease when `s` claims: its job (it outlives a `/clear` and a
/// crash restart, which change the session), else its session, else its id;
/// the user (None) may finish any item (repairs).
fn holder(s: &Sender) -> Option<String> {
    if is_user(s) {
        return None;
    }
    Some(s.job.clone().or(s.session.clone()).or(s.id.clone()).unwrap_or_default())
}

fn is_user(s: &Sender) -> bool {
    s.kind == "user"
}

fn is(s: &Sender, id: &Id) -> bool {
    s.kind == "member" && s.id.as_deref() == Some(id.0.as_str())
}

/// The aide's own operations: the aide itself, or the user (repairs).
fn the_aide(s: &Sender, aide: &Id, what: &str) -> Result<(), Refusal> {
    if is_user(s) || is(s, aide) {
        return Ok(());
    }
    Err(authority(format!("`nimitz inbox {what}` is {aide}'s own (and the user's); {} may post", s.line())))
}

/// May `s` post to `q`? By the inbox's policy; never the aide to itself.
fn may_post(s: &Sender, q: &Queue) -> Result<(), Refusal> {
    if is(s, &q.aide) {
        return Err(authority(format!("{} does not post to its own inbox", q.aide)));
    }
    let parent = queue::parent_of(&q.aide);
    let ok = is_user(s)
        || match q.limits.policy {
            Policy::Open => true,
            Policy::Fleet => s.kind == "member" && s.id.as_deref().is_some_and(|i| Id(i.to_string()).fleet() == q.aide.fleet()),
            Policy::Parent => is(s, &parent),
        };
    if ok {
        Ok(())
    } else {
        Err(authority(format!("{}'s inbox takes posts from {} only", q.aide, if q.limits.policy == Policy::Fleet { format!("members of fleet {}", q.aide.fleet()) } else { parent.0.clone() })))
    }
}

fn payload(ctx: &Ctx) -> Result<Value, Refusal> {
    let raw = match (ctx.args.value("--payload"), ctx.args.value("--file")) {
        (Some(p), None) => p.to_string(),
        (None, Some(f)) => std::fs::read_to_string(f).map_err(|e| usage(format!("--file {f}: {e}")))?,
        (None, None) => {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s).map_err(|e| usage(format!("stdin: {e}")))?;
            s
        }
        (Some(_), Some(_)) => return Err(usage("give the payload once: --payload JSON or --file F (or stdin)".into())),
    };
    serde_json::from_str(&raw).map_err(|e| Refusal { code: "inbox.schema", message: format!("the payload is not JSON: {e}"), exit: 4 })
}

fn wait_secs(ctx: &Ctx) -> Result<Option<f64>, Refusal> {
    match ctx.args.value("--wait") {
        None => Ok(None),
        Some(w) => queue::duration(w).map(|s| Some(s.min(WAIT_MAX))).map_err(usage),
    }
}

// ------------------------------------------------------------------ commands

pub fn inbox(ctx: &Ctx) -> Result<i32, String> {
    let verb = ctx.args.need(0, "post|claim|heartbeat|done|fail|list|status|retry|drop|result")?;
    let r = match verb {
        "post" => post(ctx),
        "claim" => claim(ctx),
        "heartbeat" => heartbeat(ctx),
        "done" => done(ctx),
        "fail" => fail(ctx),
        "list" => list(ctx),
        "status" => status(ctx),
        "retry" | "drop" => retry_drop(ctx, verb),
        "result" => result(ctx),
        v => return Err(format!("inbox {v}: use post, claim, heartbeat, done, fail, list, status, retry, drop or result")),
    };
    told(ctx, 1);
    Ok(r.unwrap_or_else(|e| refuse(&e)))
}

/// Its parent hears of each new dead letter (whatever command made it).
fn told(ctx: &Ctx, at: usize) {
    if let Some(q) = ctx.args.pos.get(at).and_then(|s| queue::aide_id(s).ok()).and_then(|id| Queue::of(&ctx.env, &id).ok()) {
        crate::notify::dead_letters(&ctx.env, &q);
    }
}

fn post(ctx: &Ctx) -> Result<i32, Refusal> {
    ctx.args.allow(&["--type", "--payload", "--file", "--key", "--from"]).map_err(usage)?;
    ctx.args.at_most(2).map_err(usage)?;
    let id = aide(ctx, 1)?;
    let kind = ctx.args.value("--type").ok_or_else(|| usage("--type is required (wiki.note.v1, wiki.query.v1)".into()))?.to_string();
    let q = open(ctx, &id)?;
    let s = me(ctx);
    may_post(&s, &q)?;
    let p = q.post(&s, &kind, &payload(ctx)?, ctx.args.value("--key"), None, now())?;
    posted(&q, &p);
    if !p.duplicate {
        doorbell(&ctx.env, &q, &kind, &s);
    }
    Ok(0)
}

/// Ring the aide for a new item (coalesced; aide.md 5.1). A ring that cannot
/// reach it is no failure of the post: the item waits, and its start, its
/// cron or the piggyback wakes it.
fn doorbell(env: &nimitz_core::env::Env, q: &Queue, kind: &str, s: &Sender) {
    match nimitz_core::ring::ring(env, q, nimitz_core::ring::Why::Post { kind, from: &nimitz_core::ring::from_line(s) }, now()) {
        Ok(Some(n)) if !out::json_mode() => out::line(&n),
        Err(why) if !out::json_mode() => out::line(&format!("not rung ({why})")),
        _ => {}
    }
}

fn posted(q: &Queue, p: &queue::Posted) {
    if out::json_mode() {
        out::json(p);
        return;
    }
    if p.duplicate {
        out::ok(&format!("{} already holds it: item {} (same key)", q.aide, p.id));
    } else {
        out::ok(&format!("posted item {} to {}", p.id, q.aide));
    }
    if let Some(t) = &p.token {
        out::line(&format!("reply token {t} (printed once; `nimitz inbox result {} {} --token {t}`)", q.aide, p.id));
    }
}

fn claim(ctx: &Ctx) -> Result<i32, Refusal> {
    ctx.args.allow(&[]).map_err(usage)?;
    ctx.args.at_most(2).map_err(usage)?;
    let id = aide(ctx, 1)?;
    let s = me(ctx);
    the_aide(&s, &id, "claim")?;
    let q = open(ctx, &id)?;
    let by = holder(&s).unwrap_or_else(|| "the user".into());
    match q.claim(&by, now())? {
        Some(item) => {
            // always JSON: the aide reads it
            println!("{}", serde_json::to_string_pretty(&item).unwrap_or_default());
            Ok(0)
        }
        None => {
            if out::json_mode() {
                out::json(&json!({"ok": true, "empty": true}));
            } else {
                out::line(&format!("{}'s inbox is empty", q.aide));
            }
            Ok(3)
        }
    }
}

fn item_arg(ctx: &Ctx) -> Result<(Id, String), Refusal> {
    ctx.args.at_most(3).map_err(usage)?;
    let id = aide(ctx, 1)?;
    let item = queue::item_id(ctx.args.need(2, "item id").map_err(usage)?).map_err(usage)?;
    Ok((id, item))
}

fn heartbeat(ctx: &Ctx) -> Result<i32, Refusal> {
    ctx.args.allow(&[]).map_err(usage)?;
    let (id, item) = item_arg(ctx)?;
    let s = me(ctx);
    the_aide(&s, &id, "heartbeat")?;
    let q = open(ctx, &id)?;
    let i = q.heartbeat(&item, holder(&s).as_deref(), now())?;
    out::ok(&format!("item {} leased until {}", i.id, i.claim.map(|c| queue::human(c.until - now())).map(|h| format!("+{h}")).unwrap_or_default()));
    Ok(0)
}

fn done(ctx: &Ctx) -> Result<i32, Refusal> {
    ctx.args.allow(&["--result", "--result-file"]).map_err(usage)?;
    let (id, item) = item_arg(ctx)?;
    let s = me(ctx);
    the_aide(&s, &id, "done")?;
    let q = open(ctx, &id)?;
    let result = match (ctx.args.value("--result"), ctx.args.value("--result-file")) {
        (Some(r), None) => Some(r.to_string()),
        (None, Some(f)) => Some(std::fs::read_to_string(f).map_err(|e| usage(format!("--result-file {f}: {e}")))?),
        (None, None) => None,
        _ => return Err(usage("give the result once: --result JSON or --result-file F".into())),
    };
    let result = result.map(|r| serde_json::from_str::<Value>(&r).map_err(|e| usage(format!("the result is not JSON: {e}")))).transpose()?;
    let t = now();
    let i = q.done(&item, holder(&s).as_deref(), result, t)?;
    let delivery = if i.reply_to.is_some() { Some(queue::deliver(&ctx.env, &q, &i, t)) } else { None };
    if out::json_mode() {
        out::json(&json!({"ok": true, "id": i.id, "delivered": delivery.as_ref().map(|d| d.is_ok()), "delivery": delivery.as_ref().map(|d| match d { Ok(s) | Err(s) => s.clone() })}));
    } else {
        out::ok(&format!("item {} done", i.id));
        match delivery {
            Some(Ok(note)) => out::ok(&format!("answer {note}")),
            Some(Err(why)) => out::line(&format!("answer not delivered: {why}")),
            None => {}
        }
    }
    Ok(0)
}

fn fail(ctx: &Ctx) -> Result<i32, Refusal> {
    ctx.args.allow(&["--reason", "--permanent"]).map_err(usage)?;
    let (id, item) = item_arg(ctx)?;
    let s = me(ctx);
    the_aide(&s, &id, "fail")?;
    let reason = ctx.args.value("--reason").ok_or_else(|| usage("--reason is required (one line)".into()))?;
    let q = open(ctx, &id)?;
    let (i, dead) = q.fail(&item, holder(&s).as_deref(), reason, ctx.args.flag("--permanent"), now())?;
    if dead {
        out::warn(&format!("item {} is a dead letter after {} tr{}: {reason}", i.id, i.tries, if i.tries == 1 { "y" } else { "ies" }));
        out::line(&format!("       fix: nimitz inbox retry {id} {}  # or drop it (the user's or {}'s)", i.id, queue::parent_of(&id)));
    } else {
        out::ok(&format!("item {} back to pending (try {} of {})", i.id, i.tries, q.limits.max_tries));
    }
    Ok(0)
}

fn retry_drop(ctx: &Ctx, verb: &str) -> Result<i32, Refusal> {
    ctx.args.allow(&[]).map_err(usage)?;
    let (id, item) = item_arg(ctx)?;
    let s = me(ctx);
    let parent = queue::parent_of(&id);
    if !(is_user(&s) || is(&s, &parent)) {
        return Err(authority(format!("`nimitz inbox {verb}` is the user's or {parent}'s")));
    }
    let q = open(ctx, &id)?;
    let i = if verb == "retry" { q.retry(&item, now())? } else { q.drop_item(&item, now())? };
    nimitz_core::notify::clear_dead(&ctx.env, &id.0, &i.id);
    out::ok(&format!("item {} {}", i.id, if verb == "retry" { "back to pending" } else { "dropped (kept under dead/dropped/)" }));
    Ok(0)
}

/// The rows of `list`: others' payloads and results never shown.
fn list(ctx: &Ctx) -> Result<i32, Refusal> {
    ctx.args.allow(&["--state"]).map_err(usage)?;
    ctx.args.at_most(2).map_err(usage)?;
    let id = aide(ctx, 1)?;
    let state = match ctx.args.value("--state") {
        Some(s) => Some(State::parse(s).ok_or_else(|| usage(format!("--state {s}: pending, claimed, done or dead")))?),
        None => None,
    };
    let q = open(ctx, &id)?;
    let s = me(ctx);
    let t = now();
    let rows: Vec<Value> = q
        .list(state)
        .into_iter()
        .map(|(st, i)| {
            let mine = queue::may_read(&i, &s, None) || is(&s, &id);
            json!({
                "id": i.id, "state": st.dir(), "type": i.kind, "sender": i.sender.line(), "age_s": (t - i.posted_at).round(),
                "tries": i.tries, "reason": i.reason,
                "payload": if mine { i.payload.clone() } else { Value::Null },
                "result": if mine { i.result.clone().unwrap_or(Value::Null) } else { Value::Null },
            })
        })
        .collect();
    if out::json_mode() {
        out::json(&rows);
        return Ok(0);
    }
    if rows.is_empty() {
        out::line(&format!("{}'s inbox has no {}items", id, state.map(|s| format!("{} ", s.dir())).unwrap_or_default()));
    }
    for r in &rows {
        let reason = r["reason"].as_str().map(|x| format!("  {x}")).unwrap_or_default();
        out::line(&format!("{}  {:<8} {:<14} {:<28} {:>6}{reason}", r["id"].as_str().unwrap_or(""), r["state"].as_str().unwrap_or(""), r["type"].as_str().unwrap_or(""), r["sender"].as_str().unwrap_or(""), queue::human(r["age_s"].as_f64().unwrap_or(0.0))));
    }
    Ok(0)
}

/// Every configured aide on this machine's fleets.
fn all_aides(ctx: &Ctx) -> Vec<Id> {
    let mut out = vec![];
    for f in nimitz_core::config::load_all(&ctx.env).0 {
        for s in &f.config.aide {
            if let Ok(id) = queue::aide_id(&format!("{}/@{}", f.config.fleet, s.name)) {
                out.push(id);
            }
        }
    }
    out
}

fn status(ctx: &Ctx) -> Result<i32, Refusal> {
    ctx.args.allow(&[]).map_err(usage)?;
    ctx.args.at_most(2).map_err(usage)?;
    let ids = match ctx.args.pos.get(1) {
        Some(_) => vec![aide(ctx, 1)?],
        None => all_aides(ctx),
    };
    let results = is_user(&me(ctx));
    let t = now();
    let mut all = vec![];
    for id in &ids {
        // one aide whose queue cannot open is one row with its error, never the whole list
        match open(ctx, id) {
            Ok(q) => all.push(q.status(t, results)),
            Err(e) if ctx.args.pos.get(1).is_none() => all.push(queue::Status { aide: id.0.clone(), error: Some(e.message), ..Default::default() }),
            Err(e) => return Err(e),
        }
    }
    if out::json_mode() {
        if ctx.args.pos.get(1).is_some() {
            out::json(&all[0]);
        } else {
            out::json(&all);
        }
        return Ok(0);
    }
    if all.is_empty() {
        out::line("no aides ([[aide]] in a fleet.toml)");
    }
    for s in &all {
        out::line(&format!("{}  {}", s.aide, s.line()));
        for c in &s.claimed {
            out::line(&format!("  claimed  {}  {} by {}  lease {}", c.id, c.kind, c.by, queue::human(c.until - t)));
        }
        for d in &s.dead {
            out::line(&format!("  dead     {}  {} from {}: {}", d.id, d.kind, d.sender, d.reason));
        }
    }
    Ok(0)
}

/// Poll `item` until it is done or dead, or `wait` runs out.
fn await_item(q: &Queue, item: &str, wait: f64) -> Option<(State, queue::Item)> {
    let end = Instant::now() + Duration::from_secs_f64(wait);
    loop {
        let got = q.get(item);
        if matches!(got, Some((State::Done | State::Dead, _))) || Instant::now() >= end {
            return got;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Print a done item's result (an answer as markdown with its citations) or a
/// dead one's reason; the exit code (0 done, 5 not yet, 6 dead).
fn show(q: &Queue, item: &str, got: Option<(State, queue::Item)>, token: Option<&str>) -> Result<i32, Refusal> {
    let Some((st, i)) = got else {
        // dropped while it waited: as final as a dead letter
        if q.dir.join("dead").join("dropped").join(format!("{item}.json")).exists() {
            return Err(Refusal { code: "inbox.dead", message: format!("item {item} was dropped from {}", q.aide), exit: 6 });
        }
        return Err(Refusal { code: "inbox.unknown", message: format!("no item {item} in {}", q.aide), exit: 1 });
    };
    let code = match st {
        State::Done => 0,
        State::Dead => 6,
        _ => 5,
    };
    if out::json_mode() {
        let mut v = json!({"id": i.id, "state": st.dir(), "result": i.result, "reason": i.reason});
        if st != State::Done && st != State::Dead {
            if let Some(t) = token {
                v["token"] = json!(t); // a process asker's only way back to its answer
            }
        }
        out::json(&v);
        return Ok(code);
    }
    match st {
        State::Done => match i.result.as_ref() {
            Some(r) if r.get("answer").is_some() => {
                out::line(r["answer"].as_str().unwrap_or(""));
                if let Some(c) = r.get("citations").and_then(Value::as_array).filter(|c| !c.is_empty()) {
                    out::line("");
                    out::line(&format!("sources: {}", c.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")));
                }
                if let Some(g) = r.get("gaps").and_then(Value::as_str).filter(|g| !g.is_empty()) {
                    out::line(&format!("gaps: {g}"));
                }
                if let Some(s) = r.get("snapshot").and_then(Value::as_str) {
                    out::line(&format!("(read at commit {s}; confidence {})", r.get("confidence").and_then(Value::as_str).unwrap_or("?")));
                }
            }
            Some(r) => out::line(&serde_json::to_string_pretty(r).unwrap_or_default()),
            None => out::ok(&format!("item {} done", i.id)),
        },
        State::Dead => out::error(&format!("item {} failed for good: {} (inbox.dead)", i.id, i.reason.unwrap_or_default())),
        _ => {
            out::line(&format!("item {} is {} (not answered yet)", i.id, st.dir()));
            let t = token.map(|t| format!(" --token {t}")).unwrap_or_default();
            out::line(&format!("later: nimitz inbox result {} {}{t}", q.aide, i.id));
        }
    }
    Ok(code)
}

fn result(ctx: &Ctx) -> Result<i32, Refusal> {
    ctx.args.allow(&["--token", "--wait"]).map_err(usage)?;
    let (id, item) = item_arg(ctx)?;
    let q = open(ctx, &id)?;
    let token = ctx.args.value("--token");
    let s = me(ctx);
    let Some((_, i)) = q.get(&item) else { return show(&q, &item, None, token) };
    if !queue::may_read(&i, &s, token) {
        return Err(Refusal { code: "inbox.not_yours", message: format!("item {item} is another sender's request"), exit: 1 });
    }
    let got = match wait_secs(ctx)? {
        Some(w) => await_item(&q, &item, w),
        None => q.get(&item),
    };
    show(&q, &item, got, token)
}

/// `nimitz ask <aide> "<question>" [--scope S] [--length L] [--wait D]`: post
/// a `wiki.query.v1`, then (by default) wait for the answer.
pub fn ask(ctx: &Ctx) -> Result<i32, String> {
    let r = (|| {
        ctx.args.allow(&["--scope", "--length", "--wait", "--from", "--no-wait"]).map_err(usage)?;
        ctx.args.at_most(2).map_err(usage)?;
        let id = aide(ctx, 0)?;
        let question = ctx.args.need(1, "the question").map_err(usage)?;
        let mut p = json!({"question": question});
        if let Some(s) = ctx.args.value("--scope") {
            p["scope"] = json!(s);
        }
        if let Some(l) = ctx.args.value("--length") {
            p["length"] = json!(l);
        }
        let q = open(ctx, &id)?;
        let s = me(ctx);
        may_post(&s, &q)?;
        let wait = if ctx.args.flag("--no-wait") { None } else { Some(wait_secs(ctx)?.unwrap_or(WAIT_DEFAULT)) };
        let t = now();
        // while `ask` waits, the answer comes back here, not as a peer message too
        let posted = q.post(&s, "wiki.query.v1", &p, None, wait.map(|w| t + w), t)?;
        // the handle first, on stderr: a caller killed while it waits (a Bash
        // timeout) still has its item and its token
        if let Some(tok) = &posted.token {
            let n = posted.id.trim_start_matches('0');
            eprintln!("asked {} item {n}; token {tok}; later: nimitz inbox result {} {n} --token {tok}", q.aide, q.aide);
        }
        if !posted.duplicate {
            doorbell(&ctx.env, &q, "wiki.query.v1", &s);
        }
        let Some(wait) = wait else {
            posted_line(&q, &posted);
            return Ok(5);
        };
        // until the post's own `waiting_until`, not longer: past it the answer
        // goes as a peer message, and twice would be one too many
        let got = await_item(&q, &posted.id, (t + wait - now()).max(0.0));
        if matches!(got, Some((State::Done, _))) {
            let _ = q.mark_delivered(&posted.id, "returned to `nimitz ask`", true, now());
        }
        show(&q, &posted.id, got, posted.token.as_deref())
    })();
    told(ctx, 0);
    Ok(r.unwrap_or_else(|e| refuse(&e)))
}

fn posted_line(q: &Queue, p: &queue::Posted) {
    posted(q, p);
}
