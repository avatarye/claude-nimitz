//! Merge the raw sources into one snapshot: Fleet -> Agent -> SubAgent / Task.
//!
//! Identity rules (plans/plan-fleet-status-tui.md in the admiralty dir):
//! - A session belongs to a fleet by its `[TAG]-` name prefix, else by its cwd.
//! - The lead is the session whose name starts with `[TAG]-LEAD`; never
//!   inferred from cwd.
//! - A session is live only while it has a pid. `state` goes stale.
//! - One row per name is shown; older rows of the same name fold into `stale`.

use std::collections::{BTreeMap, HashMap};

use crate::env::Env;
use crate::sources::{self as src, Cache, DeclaredAgent, FleetConfig, JobState, SessionRow, SubAgentRec, TaskItem};

pub const HUB_NAME: &str = "[ADMIRALTY]";
/// fleet tag and key of the hub's own row (the tree root)
pub const HUB_TAG: &str = "ADMIRALTY";
pub const HUB_KEY: &str = "HUB";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    NeedsYou,
    Busy,
    Idle,
    Stopped,
    NotRunning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Flag {
    OffConfig,
    DuplicateLive,
    NameVariant,
    Undeclared,
    Untagged,
    LeftQuestion,
}

impl Flag {
    pub fn text(self) -> &'static str {
        match self {
            Flag::OffConfig => "off-config",
            Flag::DuplicateLive => "DUPLICATE LIVE",
            Flag::NameVariant => "name variant",
            Flag::Undeclared => "not in config",
            Flag::Untagged => "untagged name",
            Flag::LeftQuestion => "left a question",
        }
    }
    pub fn help(self) -> &'static str {
        match self {
            Flag::OffConfig => "running in a different directory than FLEET_CONFIG.json says",
            Flag::DuplicateLive => "more than one live session has this name",
            Flag::NameVariant => "lead name carries a suffix; a duplicate may have been live when it started",
            Flag::Undeclared => "not listed in this fleet's FLEET_CONFIG.json",
            Flag::Untagged => "name has no [FLEET]- prefix; matched to the fleet by its directory",
            Flag::LeftQuestion => "stopped while waiting on an answer from you",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SubAgent {
    pub rec: SubAgentRec,
    pub running: bool,
    pub children: Vec<SubAgent>,
}

#[derive(Clone, Debug)]
pub struct Agent {
    pub key: String,
    pub fleet: String,
    pub name: String,
    pub is_lead: bool,
    pub declared: Option<DeclaredAgent>,
    pub primary: Option<SessionRow>,
    pub stale: Vec<SessionRow>,
    pub status: Status,
    pub status_since: Option<f64>,
    pub flags: Vec<Flag>,
    pub job: JobState,
    pub subagents: Vec<SubAgent>,
    pub tasks: Vec<TaskItem>,
    pub expected_dir: Option<String>,
    /// on a lead: the fleet's other sessions (not declared in FLEET_CONFIG.json),
    /// e.g. background helpers the lead started; shown as its sub-agents
    pub helpers: Vec<Agent>,
}

impl Agent {
    pub fn live(&self) -> bool {
        self.primary.as_ref().map(|p| p.live()).unwrap_or(false)
    }
    pub fn all_subagents(&self) -> Vec<&SubAgent> {
        let mut out = vec![];
        let mut stack: Vec<&SubAgent> = self.subagents.iter().collect();
        while let Some(s) = stack.pop() {
            out.push(s);
            stack.extend(s.children.iter());
        }
        out
    }
    pub fn running_subagents(&self) -> usize {
        self.all_subagents().iter().filter(|s| s.running).count()
    }
}

#[derive(Clone, Debug)]
pub struct Fleet {
    pub tag: String,
    pub dir: String,
    pub machine: Option<String>,
    pub agents: Vec<Agent>,
    pub error: Option<String>,
}

impl Fleet {
    pub fn on_machine(&self) -> bool {
        self.machine.is_some()
    }
    pub fn lead(&self) -> Option<&Agent> {
        self.agents.iter().find(|a| a.is_lead)
    }
    pub fn live_count(&self) -> usize {
        self.agents.iter().filter(|a| a.live()).count()
    }
    /// counts helpers too: a question is a question whoever asks it
    pub fn need_count(&self) -> usize {
        self.agents.iter().chain(self.agents.iter().flat_map(|a| a.helpers.iter())).filter(|a| a.status == Status::NeedsYou).count()
    }
    pub fn busy_count(&self) -> usize {
        self.agents.iter().filter(|a| a.status == Status::Busy).count()
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub taken_at: f64,
    pub machine_hint: String,
    pub fleets: Vec<Fleet>,
    pub others: Vec<Agent>,
    pub hub: Option<SessionRow>,
    /// the hub as a full agent row: sub-agents, tasks, pane, actions
    pub hub_agent: Agent,
    pub errors: Vec<String>,
    pub source: String,
}

impl Snapshot {
    /// fleet agents (plus the hub and non-fleet sessions when `include_others`)
    pub fn agents(&self, include_others: bool) -> Vec<&Agent> {
        let mut v: Vec<&Agent> = vec![];
        if include_others {
            v.push(&self.hub_agent);
        }
        for a in self.fleets.iter().flat_map(|f| f.agents.iter()) {
            v.push(a);
            v.extend(a.helpers.iter());
        }
        if include_others {
            v.extend(self.others.iter());
        }
        v
    }
    pub fn agent(&self, key: &str) -> Option<&Agent> {
        self.agents(true).into_iter().find(|a| a.key == key)
    }
    pub fn live_count(&self) -> usize {
        self.fleets.iter().map(|f| f.live_count()).sum()
    }
    pub fn need_count(&self) -> usize {
        self.fleets.iter().map(|f| f.need_count()).sum()
    }
}

// ------------------------------------------------------------------ building

pub fn parse_name(name: &str) -> Option<(String, String)> {
    let n = name.trim();
    let rest = n.strip_prefix('[')?;
    let close = rest.find(']')?;
    let tag = &rest[..close];
    let role = rest[close + 1..].strip_prefix('-')?;
    if tag.is_empty() || role.is_empty() {
        return None;
    }
    Some((tag.to_uppercase(), role.to_string()))
}

fn is_lead_role(role: &str) -> bool {
    let r = role.to_uppercase();
    r == "LEAD" || r.starts_with("LEAD-")
}

fn trim_slash(s: &str) -> &str {
    s.trim_end_matches('/')
}

/// Same sessionId listed twice (a live interactive row and the dead job row it
/// was resumed from): keep the live one.
fn dedupe(rows: Vec<SessionRow>) -> Vec<SessionRow> {
    let mut best: Vec<SessionRow> = vec![];
    for r in rows {
        match best.iter().position(|b| b.session_id == r.session_id) {
            Some(i) => {
                if r.live() && !best[i].live() {
                    best[i] = r;
                }
            }
            None => best.push(r),
        }
    }
    best
}

fn pick_primary(mut rows: Vec<SessionRow>) -> (Option<SessionRow>, Vec<SessionRow>) {
    rows.sort_by(|a, b| {
        (b.live(), b.started_at.unwrap_or(0.0))
            .partial_cmp(&(a.live(), a.started_at.unwrap_or(0.0)))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if rows.is_empty() {
        return (None, rows);
    }
    let first = rows.remove(0);
    (Some(first), rows)
}

fn build_subagents(env: &Env, recs: &[SubAgentRec], session_live: bool, now: f64) -> Vec<SubAgent> {
    // children lists by parent id; roots are those whose parent is unknown
    let ids: std::collections::HashSet<&str> = recs.iter().map(|r| r.agent_id.as_str()).collect();
    let mut kids: HashMap<String, Vec<&SubAgentRec>> = HashMap::new();
    let mut roots: Vec<&SubAgentRec> = vec![];
    for r in recs {
        match r.parent_id.as_deref().filter(|p| ids.contains(p)) {
            Some(p) => kids.entry(p.to_string()).or_default().push(r),
            None => roots.push(r),
        }
    }
    fn make(env: &Env, r: &SubAgentRec, kids: &HashMap<String, Vec<&SubAgentRec>>, live: bool, now: f64, depth: usize) -> SubAgent {
        let mut children: Vec<SubAgent> = if depth > 32 {
            vec![]
        } else {
            kids.get(&r.agent_id).map(|v| v.iter().map(|c| make(env, c, kids, live, now, depth + 1)).collect()).unwrap_or_default()
        };
        order(&mut children);
        let running = (live && now - r.last_write < env.active_window_s) || children.iter().any(|c| c.running);
        SubAgent { rec: r.clone(), running, children }
    }
    fn order(v: &mut [SubAgent]) {
        v.sort_by(|a, b| {
            (!a.running, -a.rec.last_write).partial_cmp(&(!b.running, -b.rec.last_write)).unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    let mut out: Vec<SubAgent> = roots.iter().map(|r| make(env, r, &kids, session_live, now, 0)).collect();
    order(&mut out);
    out
}

#[allow(clippy::too_many_arguments)]
fn make_agent(
    env: &Env,
    cache: &mut Cache,
    fleet: Option<&FleetConfig>,
    tag: &str,
    name: &str,
    is_lead: bool,
    declared: Option<DeclaredAgent>,
    rows: Vec<SessionRow>,
    now: f64,
) -> Agent {
    let rows = dedupe(rows);
    let live_rows = rows.iter().filter(|r| r.live()).count();
    let (primary, stale) = pick_primary(rows);
    let mut flags = vec![];
    let expected = match &declared {
        Some(d) => d.dir.clone(),
        None if is_lead => fleet.map(|f| f.dir.to_string_lossy().into_owned()),
        None => None,
    };
    if fleet.is_some() && declared.is_none() && !is_lead && primary.is_some() {
        flags.push(Flag::Undeclared);
    }
    if let (Some(p), Some(_)) = (&primary, fleet) {
        if parse_name(&p.name).is_none() {
            flags.push(Flag::Untagged);
        }
    }
    if is_lead {
        if let Some(p) = &primary {
            if p.name.to_uppercase() != format!("[{tag}]-LEAD") {
                flags.push(Flag::NameVariant);
            }
        }
    }
    if let (Some(p), Some(e)) = (&primary, &expected) {
        if trim_slash(&p.cwd) != trim_slash(e) {
            flags.push(Flag::OffConfig);
        }
    }
    if live_rows > 1 {
        flags.push(Flag::DuplicateLive);
    }

    let mut job = match &primary {
        Some(p) if p.kind == "background" => src::read_job(env, cache, p.short_id.as_deref()),
        _ => JobState::default(),
    };
    let mut since = None;
    let status = match &primary {
        None => Status::NotRunning,
        Some(p) if !p.live() => {
            if job.needs.is_some() {
                flags.push(Flag::LeftQuestion);
            }
            Status::Stopped
        }
        Some(p) => {
            if p.kind == "interactive" {
                since = p.status_since;
            }
            let busy = p.status.as_deref() == Some("busy");
            if let Some(w) = &p.waiting_for {
                // Claude Code reports the session parked at a prompt
                if job.needs.is_none() {
                    job.needs = Some(format!("Waiting for {w} in its session."));
                }
                Status::NeedsYou
            } else if job.needs.is_some() && !busy {
                Status::NeedsYou
            } else if busy {
                Status::Busy
            } else {
                Status::Idle
            }
        }
    };

    let (mut subs, mut tasks) = (vec![], vec![]);
    if let Some(p) = &primary {
        let recs = src::read_subagents(env, cache, &p.session_id);
        subs = build_subagents(env, &recs, p.live(), now);
        let mut teams: BTreeMap<&str, usize> = BTreeMap::new();
        for r in &recs {
            if let Some(t) = &r.team {
                *teams.entry(t.as_str()).or_default() += 1;
            }
        }
        let team = teams
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(t, _)| t.to_string())
            .unwrap_or_else(|| format!("session-{}", &p.session_id[..p.session_id.len().min(8)]));
        tasks = src::read_tasks(env, cache, &team);
    }

    Agent {
        key: format!("{tag}/{}", if is_lead { "LEAD" } else { name }),
        fleet: tag.to_string(),
        name: primary.as_ref().map(|p| p.name.clone()).unwrap_or(name.to_string()),
        is_lead,
        declared,
        primary,
        stale,
        status,
        status_since: since,
        flags,
        job,
        subagents: subs,
        tasks,
        expected_dir: expected,
        helpers: vec![],
    }
}

pub fn build(env: &Env, cache: &mut Cache, configs: &[FleetConfig], rows: Vec<SessionRow>, errors: Vec<String>, now: f64) -> Snapshot {
    let by_tag: HashMap<&str, &FleetConfig> = configs.iter().map(|c| (c.tag.as_str(), c)).collect();
    let mut dir_owner: HashMap<String, String> = HashMap::new();
    for c in configs {
        dir_owner.insert(trim_slash(&c.dir.to_string_lossy()).to_string(), c.tag.clone());
        for a in &c.agents {
            if let Some(d) = &a.dir {
                dir_owner.entry(trim_slash(d).to_string()).or_insert(c.tag.clone());
            }
        }
    }

    let hub = {
        let mut h: Vec<&SessionRow> = rows.iter().filter(|r| r.name == HUB_NAME && r.live()).collect();
        h.sort_by(|a, b| b.started_at.partial_cmp(&a.started_at).unwrap_or(std::cmp::Ordering::Equal));
        h.first().cloned().cloned()
    };
    let hub_rows: Vec<SessionRow> = rows.iter().filter(|r| r.name == HUB_NAME).cloned().collect();
    let mut hub_agent = make_agent(env, cache, None, HUB_TAG, HUB_NAME, false, None, hub_rows, now);
    hub_agent.key = HUB_KEY.to_string();

    // (tag, group name, is_lead) -> rows ; BTreeMap keeps output stable
    let mut buckets: BTreeMap<(String, String, bool), Vec<SessionRow>> = BTreeMap::new();
    let mut other: BTreeMap<String, Vec<SessionRow>> = BTreeMap::new();
    for r in rows {
        if r.name == HUB_NAME {
            continue;
        }
        let fallback = if r.name.is_empty() { r.session_id.chars().take(8).collect() } else { r.name.clone() };
        match parse_name(&r.name).filter(|(t, _)| by_tag.contains_key(t.as_str())) {
            Some((tag, role)) => {
                let lead = is_lead_role(&role);
                let group = if lead { format!("[{tag}]-LEAD") } else { r.name.to_uppercase() };
                buckets.entry((tag, group, lead)).or_default().push(r);
            }
            None => match dir_owner.get(trim_slash(&r.cwd)) {
                Some(tag) => buckets.entry((tag.clone(), fallback, false)).or_default().push(r),
                None => other.entry(fallback).or_default().push(r),
            },
        }
    }

    // a lead that is not running leaves no session row, only its transcript
    for c in configs.iter().filter(|c| c.machine.is_some() && c.error.is_none()) {
        let key = (c.tag.clone(), format!("[{}]-LEAD", c.tag), true);
        if buckets.get(&key).map(|rs| rs.iter().any(|r| r.live())).unwrap_or(false) {
            continue;
        }
        if let Some(row) = src::last_lead_session(env, cache, &c.tag, &c.dir.to_string_lossy()) {
            let rs = buckets.entry(key).or_default();
            if !rs.iter().any(|r| r.session_id == row.session_id) {
                rs.push(row);
            }
        }
    }

    let mut sorted: Vec<&FleetConfig> = configs.iter().collect();
    sorted.sort_by(|a, b| a.tag.cmp(&b.tag));
    let mut fleets = vec![];
    for c in sorted {
        let mut agents = vec![];
        let mut seen = std::collections::HashSet::new();
        for d in &c.agents {
            if c.machine.is_some() && d.dir.is_none() {
                continue; // declared, but not deployed on this machine
            }
            let key = (c.tag.clone(), d.name.clone(), false);
            seen.insert(key.clone());
            let rs = buckets.get(&key).cloned().unwrap_or_default();
            if c.machine.is_none() && rs.is_empty() {
                continue;
            }
            agents.push(make_agent(env, cache, Some(c), &c.tag, &d.name, false, Some(d.clone()), rs, now));
        }
        // only the lead and declared agents are fleet agents; any other session that
        // belongs to the fleet (tagged, or in a fleet dir) hangs under the lead
        let mut helpers = vec![];
        for (key, rs) in buckets.iter().filter(|(k, _)| k.0 == c.tag && !seen.contains(*k)) {
            let declared = c.agents.iter().find(|d| d.name == key.1).cloned();
            if declared.is_none() && !key.2 {
                let mut h = make_agent(env, cache, Some(c), &c.tag, &key.1, false, None, rs.clone(), now);
                h.flags.retain(|f| !matches!(f, Flag::Undeclared | Flag::Untagged));
                helpers.push(h);
            } else {
                agents.push(make_agent(env, cache, Some(c), &c.tag, &key.1, key.2, declared, rs.clone(), now));
            }
        }
        if !helpers.is_empty() {
            helpers.sort_by(|a, b| a.name.cmp(&b.name));
            if !agents.iter().any(|a| a.is_lead) {
                // no lead row at all: a placeholder lead holds them
                let name = format!("[{}]-LEAD", c.tag);
                agents.push(make_agent(env, cache, Some(c), &c.tag, &name, true, None, vec![], now));
            }
            if let Some(lead) = agents.iter_mut().find(|a| a.is_lead) {
                lead.helpers = helpers;
            }
        }
        agents.sort_by(|a, b| (!a.is_lead, a.name.clone()).cmp(&(!b.is_lead, b.name.clone())));
        fleets.push(Fleet {
            tag: c.tag.clone(),
            dir: c.dir.to_string_lossy().into_owned(),
            machine: c.machine.clone(),
            agents,
            error: c.error.clone(),
        });
    }
    let others = other.into_iter().map(|(n, rs)| make_agent(env, cache, None, "OTHER", &n, false, None, rs, now)).collect();
    Snapshot {
        taken_at: now,
        machine_hint: env.hostnames.first().cloned().unwrap_or_default(),
        fleets,
        others,
        hub,
        hub_agent,
        errors,
        source: String::new(),
    }
}

pub fn snapshot(env: &Env, cache: &mut Cache) -> Snapshot {
    let configs = src::read_fleets(env, cache);
    let (rows, errors, source) = match src::read_sessions(env, cache) {
        Ok((rows, source)) => (rows, vec![], source),
        Err(e) => (vec![], vec![e], "none"),
    };
    let mut s = build(env, cache, &configs, rows, errors, src::now());
    s.source = source.to_string();
    s
}
