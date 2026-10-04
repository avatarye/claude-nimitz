//! Shared presentation: words, glyphs, ages, fleet summaries, and the plain-text `dump`.

use crate::model::{Agent, Fleet, Snapshot, Status, SubAgent};

pub fn glyph(s: Status) -> &'static str {
    match s {
        Status::NeedsYou => "!",
        Status::Busy => "*",
        Status::Idle => "-",
        Status::Stopped => "x",
        Status::NotRunning => "o",
    }
}

pub fn word(s: Status) -> &'static str {
    match s {
        Status::NeedsYou => "needs you",
        Status::Busy => "busy",
        Status::Idle => "idle",
        Status::Stopped => "stopped",
        Status::NotRunning => "not running",
    }
}

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

pub fn agent_age(a: &Agent, now: f64) -> String {
    if let Some(t) = a.status_since {
        return age(now - t);
    }
    match a.primary.as_ref().and_then(|p| p.started_at) {
        Some(t) => age(now - t),
        None => String::new(),
    }
}

pub fn fleet_summary(f: &Fleet) -> String {
    if let Some(e) = &f.error {
        return e.clone();
    }
    if !f.on_machine() {
        return "not on this machine".into();
    }
    let mut parts = vec![format!("{} live", f.live_count())];
    if f.busy_count() > 0 {
        parts.push(format!("{} busy", f.busy_count()));
    }
    if f.need_count() > 0 {
        parts.push(format!("{} need you", f.need_count()));
    }
    if !f.lead().map(|l| l.live()).unwrap_or(false) {
        parts.push("no lead".into());
    }
    let stopped = f.agents.iter().filter(|a| a.status == Status::Stopped).count();
    if stopped > 0 {
        parts.push(format!("{stopped} stopped"));
    }
    let off = f.agents.iter().filter(|a| a.flags.contains(&crate::model::Flag::OffConfig)).count();
    if off > 0 {
        parts.push(format!("off-config {off}"));
    }
    parts.join("   ")
}

pub fn short_model(m: Option<&str>) -> String {
    m.unwrap_or("").replace("claude-", "").split('[').next().unwrap_or("").to_string()
}

fn sub_line(s: &SubAgent, now: f64) -> String {
    let r = &s.rec;
    let kind = if r.teammate { "teammate".to_string() } else { r.agent_type.clone().unwrap_or("agent".into()) };
    let state = if s.running { "running" } else { "finished" };
    format!(
        "{} {}  [{kind}] {state} {} {}",
        if s.running { ">" } else { "." },
        r.label,
        short_model(r.model.as_deref()),
        age(now - r.last_write)
    )
    .trim_end()
    .to_string()
}

/// Plain-text tree; same model as the TUI. Used for scripting and tests.
pub fn dump(snap: &Snapshot, show_others: bool, show_finished: bool) -> String {
    let now = snap.taken_at;
    let mut out = vec![format!(
        "admiralty-watch  {}  fleets {}  live {}  need-you {}  hub {}  source {}",
        snap.machine_hint,
        snap.fleets.len(),
        snap.live_count(),
        snap.need_count(),
        if snap.hub.is_some() { "live" } else { "not running" },
        snap.source
    )];
    out.extend(snap.errors.iter().map(|e| format!("[ERROR] {e}")));
    for f in &snap.fleets {
        out.push(format!("{}  ({})", f.tag, fleet_summary(f)));
        for a in &f.agents {
            agent_lines(&mut out, a, now, show_finished);
        }
    }
    if show_others && !snap.others.is_empty() {
        out.push(format!("OTHER SESSIONS ({})", snap.others.len()));
        for a in &snap.others {
            agent_lines(&mut out, a, now, show_finished);
        }
    }
    out.join("\n")
}

fn agent_lines(out: &mut Vec<String>, a: &Agent, now: f64, show_finished: bool) {
    let flags: String = a.flags.iter().map(|f| format!("  [{}]", f.text())).collect();
    let sid = a.primary.as_ref().map(|p| p.short_id.clone().unwrap_or(p.session_id.chars().take(8).collect())).unwrap_or_default();
    out.push(
        format!(
            "  {} {}{}  {} {}  {sid}{flags}",
            glyph(a.status),
            if a.is_lead { "LEAD " } else { "" },
            a.name,
            word(a.status),
            agent_age(a, now)
        )
        .trim_end()
        .to_string(),
    );
    if !a.stale.is_empty() {
        out.push(format!("      +{} stale", a.stale.len()));
    }
    if let Some(n) = &a.job.needs {
        out.push(format!("      needs: {}", n.chars().take(100).collect::<String>()));
    }
    fn walk(out: &mut Vec<String>, nodes: &[SubAgent], depth: usize, now: f64, show_finished: bool) {
        let pad = "  ".repeat(depth);
        let done = nodes.iter().filter(|s| !s.running).count();
        for s in nodes.iter().filter(|s| s.running || show_finished) {
            out.push(format!("      {pad}{}", sub_line(s, now)));
            walk(out, &s.children, depth + 1, now, show_finished);
        }
        if done > 0 && !show_finished {
            out.push(format!("      {pad}+ {done} finished"));
        }
    }
    for h in &a.helpers {
        let mut sub = vec![];
        agent_lines(&mut sub, h, now, show_finished);
        out.extend(sub.into_iter().map(|l| format!("    {l}  (background session)")));
    }
    walk(out, &a.subagents, 0, now, show_finished);
    for t in &a.tasks {
        let owner = t.owner.as_ref().map(|o| format!(" ({o})")).unwrap_or_default();
        out.push(format!("      task {} [{}] {}{owner}", t.id, t.status, t.subject.chars().take(70).collect::<String>()));
    }
}
