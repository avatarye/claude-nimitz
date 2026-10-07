//! Fleet restore from the Herdr shape (plan-v4 section 13,
//! restore-scenarios.md). One engine for `restore`, `up` (its tab pass) and
//! `layout herdr`: `plan` diffs the shape the mux reports (workspaces, tabs,
//! panes, what each pane shows and runs), the jobs the snapshot knows and
//! the last-known active set (`active.json`) into steps of five kinds only:
//! a wake (`sessions::wake_steps`, run by `sessions::apply`), a claim (`pane
//! run claude attach <job>` into a shell pane, with a rename when the tab
//! had a short or default label), an open (a new tab, its workspace made
//! when missing), a pin. Plus reports. Core never talks to the mux: the CLI
//! and Admiralty hand it the shape and run the steps.
//!
//! Rules (restore-scenarios.md R1-R8): reuse before create; never type into a
//! pane that runs anything but a shell; resume by exact id; leads always come
//! back, ships from the active set unless `--all`; one member, one tab, one
//! job; idempotent; only the fleets asked for, never the ADMIRALTY workspace.
use crate::env::Env;
use crate::layout::{legacy_tab_of, tab_of, workspace_of};
use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ------------------------------------------------------------------ the shape, as core sees it

/// One pane of the multiplexer's shape (the CLI maps `nimitz_mux::Shape` onto
/// this): where it is, its folder, the session it shows, what runs in it
/// (`Some(argv)`: empty for an idle shell; `None`: unread, never typed into).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShapePane {
    pub pane_id: String,
    pub tab_id: String,
    pub workspace_id: String,
    pub cwd: String,
    pub session_id: Option<String>,
    pub foreground: Option<Vec<String>>,
    /// the pid of what runs in the foreground, when the mux read it
    pub foreground_pid: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    /// (workspace id, label)
    pub workspaces: Vec<(String, String)>,
    /// (tab id, workspace id, label)
    pub tabs: Vec<(String, String, String)>,
    pub panes: Vec<ShapePane>,
}

impl Shape {
    fn workspace_label(&self, ws_id: &str) -> String {
        self.workspaces.iter().find(|(id, _)| id == ws_id).map(|(_, l)| l.clone()).unwrap_or_default()
    }
    fn tab_label(&self, tab_id: &str) -> String {
        self.tabs.iter().find(|(id, _, _)| id == tab_id).map(|(_, _, l)| l.clone()).unwrap_or_default()
    }
    fn panes_of(&self, tab_id: &str) -> Vec<&ShapePane> {
        let mut v: Vec<&ShapePane> = self.panes.iter().filter(|p| p.tab_id == tab_id).collect();
        v.sort_by(|a, b| a.pane_id.cmp(&b.pane_id));
        v
    }
}

/// Loose label matching, as the Herdr adapter does it (`normalize_label`):
/// a trailing `-FLEET` off, brackets out, upper case.
pub fn norm_label(label: &str) -> String {
    let t = label.trim();
    let t = if t.len() >= 6 && t.is_char_boundary(t.len() - 6) && t[t.len() - 6..].eq_ignore_ascii_case("-FLEET") { &t[..t.len() - 6] } else { t };
    t.chars().filter(|c| !matches!(c, '[' | ']')).collect::<String>().to_uppercase()
}

/// The shells: a pane whose foreground is one of these alone is at its
/// prompt. The one list; `nimitz_mux::is_shell` is this function.
pub const SHELLS: [&str; 9] = ["sh", "bash", "zsh", "fish", "dash", "ksh", "tcsh", "csh", "nu"];

/// A shell at its prompt: nothing in the foreground but the shell itself
/// (by its program name, a login `-zsh` included).
pub fn is_shell(argv: &[String]) -> bool {
    match argv.first() {
        None => true,
        Some(p) => SHELLS.contains(&program_name(p).trim_start_matches('-')),
    }
}

/// What `shell_pane_of` says of a pane whose foreground could not be read.
const UNREADABLE: &str = "a foreground nimitz cannot read";

fn program_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// The Herdr workspace that is never a target (R-35).
pub const ADMIRALTY_WORKSPACE: &str = "ADMIRALTY";

// ------------------------------------------------------------------ options, steps, the plan

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RestoreOpts {
    /// every member with a job row, parked or stopped, not only the active set
    pub all: bool,
    /// open a fresh tab beside a taken one (`tab.dup`)
    pub force: bool,
    /// no multiplexer: wake the jobs, make no tab (over ssh)
    pub jobs_only: bool,
    /// the shape only: tabs for the leads, attach where a job is live, no
    /// resume of a parked job unless its tab's attach wakes it (`layout herdr`, `up`'s tab pass)
    pub shape_only: bool,
    /// with `shape_only`: a tab for every live ship too (`layout herdr --ships`)
    pub ships: bool,
    /// rebuild the shape in this Herdr session although the set was recorded in another
    pub here: bool,
    /// rename a tab that shows a member under another name (R-07a)
    pub rename: bool,
}

/// Where a step runs in the mux.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PaneAt {
    pub pane_id: String,
    pub tab_id: String,
    pub workspace_id: String,
}

/// One step of a restore, in the order they run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RestoreStep {
    /// the member's claude steps (a resume, or rm + relaunch), one `apply`
    Wake { id: Id, steps: Vec<Step> },
    /// `pane run <cmd>` into a shell pane of its tab; `rename` gives the tab
    /// its full label first; `wakes`: the attach wakes the parked job, and
    /// the SessionStart `resume` of `session` is waited for
    Claim { id: Id, pane: PaneAt, rename: Option<(String, String)>, cmd: Vec<String>, cwd: String, session: String, wakes: bool },
    /// a new tab `label` in `workspace` (made when missing) running `cmd`
    Open { id: Id, workspace: String, label: String, cmd: Vec<String>, cwd: String, session: String, wakes: bool },
    /// pin a lead's job (a lost or missing pin, R-02)
    Pin { id: Id, job: String },
    /// give the tab that shows the member its full label (`--rename`, R-07a)
    Rename { id: Id, tab_id: String, label: String },
}

impl RestoreStep {
    pub fn id(&self) -> &Id {
        match self {
            RestoreStep::Wake { id, .. } | RestoreStep::Claim { id, .. } | RestoreStep::Open { id, .. } | RestoreStep::Pin { id, .. } | RestoreStep::Rename { id, .. } => id,
        }
    }
    /// How the step reads in a plan (the `herdr` lines are what the mux runs).
    pub fn display(&self) -> Vec<Step> {
        let why_wake = |w: bool| if w { "; the attach wakes the parked job" } else { "" };
        match self {
            RestoreStep::Wake { steps, .. } => steps.clone(),
            RestoreStep::Claim { id, pane, rename, cmd, cwd, wakes, .. } => {
                let mut v = vec![];
                if let Some((tab, label)) = rename {
                    v.push(Step { id: Some(id.clone()), cmd: vec!["herdr".into(), "tab".into(), "rename".into(), tab.clone(), label.clone()], cwd: None, why: format!("give the tab its full name {label}") });
                }
                let mut c = vec!["herdr".into(), "pane".into(), "run".into(), pane.pane_id.clone()];
                c.extend(cmd.iter().cloned());
                v.push(Step { id: Some(id.clone()), cmd: c, cwd: Some(cwd.clone()), why: format!("its tab is open and pane {} is a plain shell: attach there{}; detaching (ctrl-z) leaves it running", pane.pane_id, why_wake(*wakes)) });
                v
            }
            RestoreStep::Open { id, workspace, label, cmd, cwd, wakes, .. } => {
                let mut c = vec!["herdr".into(), "tab".into(), "create".into(), "--workspace".into(), workspace.clone(), "--label".into(), label.clone(), "--".into()];
                c.extend(cmd.iter().cloned());
                vec![Step { id: Some(id.clone()), cmd: c, cwd: Some(cwd.clone()), why: format!("tab {label} in workspace {workspace}: attach{}; detaching (ctrl-z) leaves it running", why_wake(*wakes)) }]
            }
            RestoreStep::Pin { id, job } => vec![Step { id: Some(id.clone()), cmd: vec!["pin".into(), job.clone()], cwd: None, why: "a lead stays up: pin its job".into() }],
            RestoreStep::Rename { id, tab_id, label } => vec![Step { id: Some(id.clone()), cmd: vec!["herdr".into(), "tab".into(), "rename".into(), tab_id.clone(), label.clone()], cwd: None, why: format!("the tab shows {id} under another name; --rename gives it its full name") }],
        }
    }
}

/// What one fleet's restore comes to.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FleetReport {
    pub fleet: String,
    /// leads and ships with a step
    pub leads: usize,
    pub ships: usize,
    /// ships left parked (they wake on demand)
    pub parked: usize,
    /// members stopped on purpose, left alone
    pub stopped: usize,
    /// members with no job and no history
    pub not_started: usize,
    /// members that could not be restored (an Error each)
    pub failed: usize,
    /// members whose conversation a Herdr pane holds interactively (`tab.resumed`)
    pub held: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RestorePlan {
    /// the steps as the gate shows them (`RestoreStep::display`, flattened),
    /// refusals (nothing runs) and reports
    pub plan: Plan,
    pub steps: Vec<RestoreStep>,
    pub fleets: Vec<FleetReport>,
    /// no active set was recorded (R-27)
    pub no_active_set: bool,
}

// ------------------------------------------------------------------ progress of a run (restore.json)

/// `NIMITZ_STATE/restore.json`: a run's progress, so a run that died partway
/// is reported by the next ("N of M done") and converges (R-10). Removed when
/// the run reaches the end of its step list, failed members or not.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub run: String,
    pub started: f64,
    pub fleets: Vec<String>,
    pub total: usize,
    pub done: usize,
    /// one line per step, the done ones first
    pub steps: Vec<String>,
}

pub fn progress_path(env: &Env) -> std::path::PathBuf {
    env.state.join("restore.json")
}

pub fn progress_load(env: &Env) -> Option<Progress> {
    let v = crate::evidence::first_json(&std::fs::read(progress_path(env)).ok()?)?;
    serde_json::from_value(v).ok()
}

pub fn progress_write(env: &Env, p: &Progress) -> std::io::Result<()> {
    crate::config::atomic_write(&progress_path(env), &serde_json::to_string_pretty(p).map_err(std::io::Error::other)?)
}

pub fn progress_clear(env: &Env) {
    let _ = std::fs::remove_file(progress_path(env));
}

// ------------------------------------------------------------------ the planner

fn issue(code: &str, severity: Severity, message: String, fix: Option<String>, id: Option<Id>) -> Issue {
    Issue { code: code.into(), severity, message, fix, id }
}

/// What a member's tab shows now (restore-scenarios.md decision 5).
#[derive(Clone, Debug, PartialEq)]
enum TabMatch {
    /// a pane shows the member's job (any of its sessions) or runs its exact attach
    Shown { pane: PaneAt, tab_label: String, ws_label: String },
    /// a tab for it with a shell pane: claim it (`rename` when its label is short or default)
    Shell { pane: PaneAt, rename: Option<(String, String)>, ws_label: String },
    /// its tab is there but no pane of it is a shell: never typed into
    Taken { pane: PaneAt, running: String, agent_view: bool },
    Missing,
}

/// The planner's read-only inputs.
struct Ctx<'a> {
    env: &'a Env,
    snap: &'a Snapshot,
    shape: &'a Shape,
    active: Option<&'a crate::active::ActiveSet>,
    opts: RestoreOpts,
    /// session id -> job (short id), from every job row (a pane reports a
    /// session; the member is its job, R-32)
    job_of: HashMap<String, String>,
    /// (session id -> member id) for every member's own sessions
    member_of: HashMap<String, Id>,
}

impl Ctx<'_> {
    /// Is this pane the member's: it shows one of the member's job's sessions
    /// (or its conversation now), or runs its exact `claude attach <job>`
    /// with no other agent in it?
    fn pane_is(&self, p: &ShapePane, m: &Member, cmd: &[String]) -> Option<bool> {
        let s = m.session.as_ref()?;
        if let Some(sid) = &p.session_id {
            let mine = sid == &s.session_id || s.birth_id.as_deref() == Some(sid) || (s.short_id.is_some() && self.job_of.get(sid) == s.short_id.as_ref());
            if mine {
                return Some(true);
            }
        }
        let fg = p.foreground.as_deref()?;
        let same = cmd.len() > 1 && fg.len() == cmd.len() && fg[1..] == cmd[1..] && program_name(&fg[0]) == program_name(&cmd[0]);
        if !same {
            return Some(false);
        }
        // the attach of the member's job, but the pane shows another agent:
        // the agent view (`/exit` inside the attach, R-24) or a copy
        Some(p.session_id.is_none())
    }

    /// The pane that holds `h`'s conversation interactively: it shows that
    /// session, or runs `claude --resume <session>`, or runs `h`'s pid.
    fn held_pane(&self, h: &SessionInfo) -> Option<&ShapePane> {
        // the session first (Herdr's detection, or the `--resume <sid>` argv);
        // the pid only when no pane names the session
        let by_session = self.shape.panes.iter().find(|p| {
            p.session_id.as_deref() == Some(h.session_id.as_str()) || p.foreground.as_deref().is_some_and(|fg| fg.windows(2).any(|w| w[0] == "--resume" && w[1] == h.session_id))
        });
        by_session.or_else(|| self.shape.panes.iter().find(|p| h.pid.is_some() && p.foreground_pid.is_some() && p.foreground_pid == h.pid && p.session_id.is_none()))
    }

    /// The pane of a tab to attach in: its first shell pane; else what its
    /// first pane runs (`Err`: nothing is typed).
    fn shell_pane_of(&self, tab_id: &str) -> Result<&ShapePane, (PaneAt, String)> {
        let panes = self.shape.panes_of(tab_id);
        if let Some(p) = panes.iter().find(|p| p.session_id.is_none() && p.foreground.as_deref().is_some_and(is_shell)) {
            return Ok(p);
        }
        let Some(p) = panes.first() else { return Err((PaneAt::default(), "no pane".into())) };
        let running = match (&p.session_id, &p.foreground) {
            (Some(sid), _) => {
                let who = self.member_of.get(sid).map(|id| format!("{} ({})", id.name(), id)).unwrap_or_else(|| format!("session {}", &sid[..sid.len().min(8)]));
                format!("an agent: {who}")
            }
            (None, Some(fg)) => fg.join(" "),
            (None, None) => UNREADABLE.into(),
        };
        Err((at(p), running))
    }

    /// The member's tab (decision 5): (a) a pane that shows its job, anywhere;
    /// (b) its full name in its workspace, a shell; (c) its full name anywhere,
    /// a shell; (d) its legacy short name in its workspace; (e) a default-named
    /// tab in its workspace whose shell sits in the member's folder; (f) none.
    /// A name match whose pane is not a shell is taken.
    fn find_tab(&self, m: &Member, cmd: &[String], extra: &mut Vec<Issue>) -> TabMatch {
        let id = &m.id;
        let want_tab = norm_label(&tab_of(id));
        let want_ws = norm_label(&workspace_of(id));
        // (a) by session or by the exact attach
        let shown: Vec<&ShapePane> = self.shape.panes.iter().filter(|p| self.pane_is(p, m, cmd) == Some(true)).collect();
        if let Some(p) = shown.first() {
            for other in shown.iter().skip(1) {
                extra.push(issue("tab.dup", Severity::Warn, format!("{id} shows in pane {} and in pane {} (two attach clients)", p.pane_id, other.pane_id), Some(format!("ctrl-z in pane {} (tab {})", other.pane_id, self.shape.tab_label(&other.tab_id))), Some(id.clone())));
            }
            return TabMatch::Shown { pane: at(p), tab_label: self.shape.tab_label(&p.tab_id), ws_label: self.shape.workspace_label(&p.workspace_id) };
        }
        let in_ws = |ws_id: &str| norm_label(&self.shape.workspace_label(ws_id)) == want_ws;
        let named: Vec<&(String, String, String)> = self.shape.tabs.iter().filter(|(_, _, l)| norm_label(l) == want_tab).collect();
        // (b) then (c): the member's workspace first
        let mut ordered: Vec<&(String, String, String)> = named.iter().filter(|(_, ws, _)| in_ws(ws)).copied().collect();
        ordered.extend(named.iter().filter(|(_, ws, _)| !in_ws(ws)).copied());
        let mut taken: Option<TabMatch> = None;
        for (tab_id, ws_id, _) in &ordered {
            match self.shell_pane_of(tab_id) {
                Ok(p) => {
                    for (other, _, _) in ordered.iter().filter(|(t, _, _)| t != tab_id) {
                        let what = self.shell_pane_of(other).map(|_| "a shell").unwrap_or("not a shell");
                        extra.push(issue("tab.dup", Severity::Info, format!("a second tab {} ({other}) is {what}", tab_of(id)), Some(format!("close tab {other}")), Some(id.clone())));
                    }
                    if !in_ws(ws_id) {
                        extra.push(issue("tab.moved", Severity::Info, format!("{id}'s tab {} is in workspace {}, not {}; reused there", tab_of(id), self.shape.workspace_label(ws_id), workspace_of(id)), None, Some(id.clone())));
                    }
                    return TabMatch::Shell { pane: at(p), rename: None, ws_label: self.shape.workspace_label(ws_id) };
                }
                Err((pane, running)) if taken.is_none() => {
                    let agent_view = pane.pane_id.is_empty() || self.shape.panes.iter().any(|p| p.pane_id == pane.pane_id && p.foreground.as_deref().is_some_and(|fg| fg.len() == cmd.len() && cmd.len() > 1 && fg[1..] == cmd[1..]));
                    taken = Some(TabMatch::Taken { pane, running, agent_view });
                }
                Err(_) => {}
            }
        }
        if let Some(t) = taken {
            return t;
        }
        // (d) the short label of 0.3.0 in the member's workspace: a shell is
        // claimed and renamed; anything else is taken, as in (b)
        let legacy = norm_label(&legacy_tab_of(id));
        for (tab_id, ws_id, _) in self.shape.tabs.iter().filter(|(_, ws, l)| in_ws(ws) && norm_label(l) == legacy) {
            match self.shell_pane_of(tab_id) {
                Ok(p) => return TabMatch::Shell { pane: at(p), rename: Some((tab_id.clone(), tab_of(id))), ws_label: self.shape.workspace_label(ws_id) },
                Err((pane, running)) => {
                    let agent_view = self.shape.panes.iter().any(|p| p.pane_id == pane.pane_id && p.foreground.as_deref().is_some_and(|fg| fg.len() == cmd.len() && cmd.len() > 1 && fg[1..] == cmd[1..]));
                    return TabMatch::Taken { pane, running, agent_view };
                }
            }
        }
        // (e) a default-named tab (`1`) of the workspace, a shell in the member's folder
        let folder = m.cwd.as_deref().map(crate::snapshot::norm_dir);
        for (tab_id, ws_id, label) in self.shape.tabs.iter().filter(|(_, ws, l)| in_ws(ws) && (l.is_empty() || l.chars().all(|c| c.is_ascii_digit()))) {
            let _ = label;
            if let Ok(p) = self.shell_pane_of(tab_id) {
                if folder.as_deref() == Some(crate::snapshot::norm_dir(&p.cwd).as_str()) {
                    return TabMatch::Shell { pane: at(p), rename: Some((tab_id.clone(), tab_of(id))), ws_label: self.shape.workspace_label(ws_id) };
                }
            }
        }
        TabMatch::Missing
    }
}

fn at(p: &ShapePane) -> PaneAt {
    PaneAt { pane_id: p.pane_id.clone(), tab_id: p.tab_id.clone(), workspace_id: p.workspace_id.clone() }
}

fn live(m: &Member) -> bool {
    m.session.as_ref().map(|s| s.pid.is_some()).unwrap_or(false)
}

/// Why a member is woken although it was not clean at the loss (R-08, R-19, R-20).
fn interrupted_note(active: Option<&crate::active::ActiveMember>, id: &Id) -> Option<Issue> {
    let a = active?;
    match a.status.as_str() {
        "busy" => Some(issue("turn.interrupted", Severity::Info, format!("{id} was mid-turn at the loss; its last prompt was not finished (re-prompt it)"), None, Some(id.clone()))),
        "needs-you" => Some(issue("prompt.lost", Severity::Info, format!("{id} was waiting on you at the loss ({}); the prompt is gone with the process (re-prompt it)", a.waiting_for.as_deref().unwrap_or("a question")), None, Some(id.clone()))),
        _ => None,
    }
}

/// Plan the restore of `fleet` (None: every fleet that runs here). You restore
/// any fleet; an admiral its own. `herdr`: the Herdr identity of this process
/// (compared with the recorded one, R-29); `has_mux`: a multiplexer is there.
#[allow(clippy::too_many_arguments)]
pub fn plan(env: &Env, snap: &Snapshot, shape: &Shape, active: Option<&crate::active::ActiveSet>, herdr: Option<&crate::active::HerdrIdentity>, has_mux: bool, fleet: Option<&str>, opts: RestoreOpts, caller: &Caller) -> RestorePlan {
    let verb = if opts.shape_only { "layout herdr" } else { "restore" };
    let mut rp = RestorePlan { plan: Plan { summary: format!("{verb} {}", fleet.unwrap_or("(every fleet here)")), ..Default::default() }, ..Default::default() };
    let fleet = match (caller, fleet) {
        (Caller::User, f) => f,
        (Caller::Member { id, role: Role::Admiral }, None) => Some(id.fleet()),
        (Caller::Member { id, role: Role::Admiral }, Some(f)) if f == id.fleet() => Some(f),
        (Caller::Member { id, .. }, _) => {
            rp.plan.refusals.push(issue("authority", Severity::Error, format!("`nimitz {verb}` brings back whole fleets: yours, or an admiral's for its own fleet; not {}'s", id.name()), None, None));
            return rp;
        }
    };
    if !has_mux && !opts.jobs_only {
        rp.plan.refusals.push(issue("mux.none", Severity::Error, format!("`nimitz {verb}` rebuilds Herdr tabs; there is no multiplexer here"), Some("run it inside Herdr; or `nimitz restore --jobs-only` to wake the jobs without tabs".into()), None));
        return rp;
    }
    // the set was recorded in another Herdr session: its tabs live there (R-29)
    if let (Some(rec), Some(now)) = (active.and_then(|a| a.herdr.as_ref()), herdr) {
        let differs = (rec.session.is_some() && now.session.is_some() && rec.session != now.session) || (rec.socket.is_some() && now.socket.is_some() && rec.socket != now.socket);
        if differs && !opts.here && !opts.jobs_only {
            rp.plan.refusals.push(issue(
                "mux.other",
                Severity::Error,
                format!("the fleet's tabs live in Herdr session {}; this is {}", rec.session.clone().or(rec.socket.clone()).unwrap_or_default(), now.session.clone().or(now.socket.clone()).unwrap_or_else(|| "another session".into())),
                Some(format!("run nimitz {verb} there; or `--here` to rebuild the shape in this session")),
                None,
            ));
            return rp;
        }
    }
    if let Some(p) = progress_load(env) {
        rp.plan.warnings.push(issue("restore.interrupted", Severity::Info, format!("an earlier restore (run {}, {} of {} steps done) was interrupted; this run converges on what it left", p.run, p.done, p.total), None, None));
    }
    rp.no_active_set = active.is_none();
    let rows = crate::evidence::rows_from_files(env, &mut crate::evidence::FileCache::default()).unwrap_or_default();
    let mut job_of = HashMap::new();
    for r in rows.iter().filter(|r| r.short_id.is_some()) {
        for sid in r.session_ids() {
            job_of.insert(sid.to_string(), r.short_id.clone().unwrap_or_default());
        }
    }
    let mut member_of = HashMap::new();
    for f in &snap.fleets {
        for m in fleet_members(f) {
            for s in m.session.iter().chain(m.extra_rows.iter()) {
                member_of.insert(s.session_id.clone(), m.id.clone());
            }
        }
    }
    let ctx = Ctx { env, snap, shape, active, opts, job_of, member_of };
    let explicit = fleet.is_some();
    for f in snap.fleets.iter().filter(|f| fleet.map(|x| x == f.id.fleet()).unwrap_or(true)) {
        if f.id.fleet() == ADMIRALTY_WORKSPACE {
            continue; // never a target (R-35)
        }
        plan_fleet(&ctx, f, explicit, &mut rp);
    }
    if explicit && !snap.fleets.iter().any(|f| Some(f.id.fleet()) == fleet) {
        rp.plan.refusals.push(issue("member.unknown", Severity::Error, format!("no fleet {} in the config", fleet.unwrap_or_default()), None, None));
    }
    if rp.no_active_set && !opts.shape_only {
        rp.plan.warnings.push(issue("active.none", Severity::Info, "no active set recorded (first restore, or NIMITZ_STATE wiped): leads come back; ships stay parked (`--all` wakes every ship with a job row)".into(), None, None));
    }
    rp.plan.steps = rp.steps.iter().flat_map(RestoreStep::display).collect();
    dedup(&mut rp.plan);
    rp
}

pub fn fleet_members(f: &Fleet) -> Vec<&Member> {
    std::iter::once(&f.admiral).chain(f.squadrons.iter().flat_map(|q| std::iter::once(&q.commodore).chain(q.ships.iter()))).collect()
}

fn dedup(plan: &mut Plan) {
    let key = |i: &Issue| (i.code.clone(), i.id.clone(), i.message.clone());
    let mut seen = std::collections::HashSet::new();
    plan.refusals.retain(|i| seen.insert(key(i)));
    let mut seen = std::collections::HashSet::new();
    plan.warnings.retain(|i| seen.insert(key(i)));
}

/// Issues the relaunch table (a wake) fixes itself: not a reason to skip a member.
const TABLE_HANDLED: [&str; 4] = ["name.drift", "labels.missing", "copy.rows", "lead.intruder"];

fn plan_fleet(ctx: &Ctx, f: &Fleet, explicit: bool, rp: &mut RestorePlan) {
    let (env, snap) = (ctx.env, ctx.snap);
    let fleet = f.id.fleet();
    let mut rep = FleetReport { fleet: fleet.to_string(), ..Default::default() };
    // a fleet of another machine: nothing is recorded here (R-18)
    if f.machine.is_none() {
        let labels: Vec<String> = crate::config::load_fleet(env, fleet).map(|fod| fod.config.machines.keys().cloned().collect()).unwrap_or_default();
        let msg = format!("fleet {fleet} does not run on this machine (its machines: {}); nothing is recorded here", if labels.is_empty() { "none".into() } else { labels.join(", ") });
        let fix = Some(format!("nimitz machine add {fleet} <label>, then nimitz up {fleet}  # a first start here, new lineage"));
        if explicit {
            rp.plan.refusals.push(issue("machine.missing", Severity::Error, msg, fix, None));
        } else {
            rp.plan.warnings.push(issue("machine.missing", Severity::Warn, msg, fix, None));
        }
        rp.fleets.push(rep);
        return;
    }
    // the pre-start gate over the whole fleet: a config error refuses it (M8);
    // warnings ride along; a member's own errors are judged where it is woken
    let gate = crate::check::check_tree(env, snap, Some(&f.id));
    if let Some(i) = gate.iter().find(|i| i.code == "config.invalid" && i.severity == Severity::Error) {
        if explicit {
            rp.plan.refusals.push(i.clone());
        } else {
            rp.plan.warnings.push(i.clone());
        }
        rp.fleets.push(rep);
        return;
    }
    // the config's findings ride along (a conflicted copy, a layout problem);
    // a member's own findings are `status`'s to show, and a restore judges
    // them where it wakes the member
    for i in gate.iter().filter(|i| i.id.is_none() || i.code.starts_with("config.")).filter(|i| i.severity != Severity::Error || i.id.is_none()) {
        rp.plan.warnings.push(i.clone());
    }
    let before = rep.not_started;
    let mut never: Vec<String> = vec![];
    for m in fleet_members(f) {
        let n = rep.not_started;
        plan_member(ctx, m, &mut rep, rp);
        if rep.not_started > n {
            never.push(m.id.0.clone());
        }
    }
    // members with no job and no history, once per fleet: a restore never cold-launches
    if rep.not_started > before && !ctx.opts.shape_only {
        rp.plan.warnings.push(issue("not.started", Severity::Info, format!("not started (no job, no history), so not restored: {}", never.join(", ")), Some(format!("nimitz up {fleet}  # starts them; or nimitz start <id>")), None));
    }
    // tabs named for members this fleet does not have (R-13)
    let ws_of_fleet = |label: &str| {
        let n = norm_label(label);
        n == fleet || n.starts_with(&format!("{fleet}."))
    };
    let known: Vec<String> = fleet_members(f).iter().flat_map(|m| [norm_label(&tab_of(&m.id)), norm_label(&legacy_tab_of(&m.id))]).collect();
    for (tab_id, ws_id, label) in &ctx.shape.tabs {
        let n = norm_label(label);
        if ws_of_fleet(&ctx.shape.workspace_label(ws_id)) && n.starts_with(&format!("{fleet}.")) && !known.contains(&n) {
            rp.plan.warnings.push(issue("tab.orphan", Severity::Info, format!("tab {label} ({tab_id}) names no member of {fleet}; close it"), None, None));
        }
    }
    rp.fleets.push(rep);
}

/// One member: should it be back, and how (the job, then its tab).
fn plan_member(ctx: &Ctx, m: &Member, rep: &mut FleetReport, rp: &mut RestorePlan) {
    let (env, opts) = (ctx.env, ctx.opts);
    let id = &m.id;
    let lead = m.role.lead();
    let active = ctx.active.and_then(|a| a.members.get(&id.0));
    let had_tab = active.map(|a| a.attached).unwrap_or(false);
    let fail = |rp: &mut RestorePlan, rep: &mut FleetReport, i: Issue| {
        rep.failed += 1;
        rp.plan.warnings.push(Issue { severity: Severity::Error, ..i });
    };
    // a process with no job runs it (a 0.3.x interactive lead, an SDK host, a
    // `claude --resume` Herdr ran after its restart): nothing to attach to,
    // and never a wake, which would start a copy beside it (R-16, finding 10)
    if let Some(h) = crate::sessions::held_of(m) {
        if let Some(p) = ctx.held_pane(h) {
            rep.held += 1;
            fail(rp, rep, issue(
                "tab.resumed",
                Severity::Error,
                format!("{id}'s conversation is open in pane {} (Herdr's resume_agents_on_restore, or a claude --resume by hand), interactively, with no job; a wake would start a copy beside it, so nothing runs for it", p.pane_id),
                Some(format!("type /exit in pane {}, then nimitz restore {}  # with Herdr's resume_agents_on_restore on, every restart does this again: nimitz doctor", p.pane_id, id.fleet())),
                Some(id.clone()),
            ));
            return;
        }
        let mut i = crate::sessions::held_issue(m, h);
        if lead && !opts.shape_only {
            i.code = "lead.interactive".into();
            i.fix = Some(format!("type /exit in its pane (tab {}), then nimitz restore {}", tab_of(id), id.fleet()));
        }
        fail(rp, rep, Issue { severity: Severity::Error, ..i });
        return;
    }
    let cwd = m.cwd.clone();
    // the tab the member should have, and whether it should have one at all
    let wants_tab = match (lead, opts.shape_only, opts.ships) {
        (true, _, _) => true,
        (false, true, ships) => ships && live(m),
        (false, false, _) => had_tab,
    };
    // which members come back without a tab's attach: leads always; ships in
    // the active set, or every one with a row under --all (R4); never a cold launch
    let in_set = active.is_some();
    match m.session.as_ref() {
        // live: nothing to wake; its tab, when it should have one
        Some(s) if s.pid.is_some() => {
            if let Some(v) = old_binary(env, s, ctx.snap) {
                rp.plan.warnings.push(v.with_id(id));
            }
            if m.status == Status::NeedsYou && !lead && !opts.shape_only {
                rp.plan.warnings.push(issue("needs-you", Severity::Warn, format!("{id} waits on you ({}); nimitz answers nothing", s.waiting_for.as_deref().unwrap_or("a question")), Some(format!("nimitz open {id}")), Some(id.clone())));
            }
            if !wants_tab {
                return; // a live ship gets a tab only when it had one, or with `--ships`
            }
            tab_step(ctx, m, false, rep, rp);
        }
        // no process: a job row to wake, or nothing to wake
        Some(s) => {
            if s.status.as_deref() == Some("unreadable") {
                fail(rp, rep, issue("session.unreadable", Severity::Error, format!("{id}'s session file cannot be read; its process may be live"), Some(format!("nimitz status  # read it again in a moment")), Some(id.clone())));
                return;
            }
            let wanted = lead || opts.all || (in_set && m.status != Status::Stopped);
            if opts.shape_only && !lead {
                rp.plan.warnings.push(issue("layout.skip", Severity::Warn, format!("{id} is not live; no tab (`nimitz start {id}` first)"), None, Some(id.clone())));
                return;
            }
            if !wanted {
                if m.status == Status::Stopped {
                    rep.stopped += 1;
                    let why = m.reasons.iter().find(|r| r.starts_with("SessionEnd")).cloned().unwrap_or_else(|| "stopped".into());
                    rp.plan.warnings.push(issue("stopped", Severity::Info, format!("{id} was stopped on purpose ({why}); left alone"), Some(format!("nimitz start {id}  # wakes it; `nimitz restore --all` wakes every stopped and parked ship")), Some(id.clone())));
                } else {
                    rep.parked += 1;
                }
                return;
            }
            // a live copy or an impostor holds its name: never a second one (R-26)
            if let Some(r) = m.extra_rows.iter().find(|r| r.pid.is_some()) {
                fail(rp, rep, issue("name.taken", Severity::Error, format!("{id} is stopped but a copy {} ({}) is live; nothing is resumed beside it", r.short_id.as_deref().unwrap_or(&r.session_id), r.name), Some(format!("nimitz sweep {id}  # stops and removes the copy; then nimitz restore {}", id.fleet())), Some(id.clone())));
                return;
            }
            if let Some(e) = m.issues.iter().find(|i| i.severity == Severity::Error && ["job.unreadable", "labels.mismatch", "name.taken"].contains(&i.code.as_str())) {
                fail(rp, rep, e.clone());
                return;
            }
            if lead && opts.shape_only {
                // the attach wakes a parked lead with a shell tab; no resume otherwise
                tab_step(ctx, m, true, rep, rp);
                return;
            }
            // a wake needs the folder (there, trusted, not another machine's): the
            // member's own check, as `nimitz start` runs it
            if let Some(i) = crate::check::check(env, ctx.snap, Some(id)).into_iter().find(|i| i.id.as_ref() == Some(id) && i.severity == Severity::Error && !TABLE_HANDLED.contains(&i.code.as_str())) {
                fail(rp, rep, i);
                return;
            }
            if let Some(n) = interrupted_note(active, id) {
                rp.plan.warnings.push(n);
            }
            // its tab first: a shell tab's attach wakes it in one step (decision 6)
            let cmd = s.short_id.as_ref().map(|j| crate::claude::attach(env, j)).unwrap_or_default();
            let mut extra = vec![];
            let found = if opts.jobs_only { TabMatch::Missing } else { ctx.find_tab(m, &cmd, &mut extra) };
            rp.plan.warnings.extend(extra);
            let needs_relabel = crate::sessions::session_issues(env, m).iter().any(|i| matches!(i.code.as_str(), "name.drift" | "labels.missing"));
            match found {
                TabMatch::Shown { pane, .. } => {
                    // a pane runs its attach already: Claude Code is waking it
                    rp.plan.warnings.push(issue("attaching", Severity::Info, format!("{id}'s tab (pane {}) runs its attach already; it wakes the job", pane.pane_id), None, Some(id.clone())));
                }
                // (an attach wakes natively, on the row's conversation: after a
                // /branch or /resume the registry's later conversation needs the
                // wake-with-every-flag first, finding 11)
                TabMatch::Shell { pane, rename, .. } if wants_tab && !needs_relabel && !crate::sessions::wake_target(env, id, s).relabel => match crate::layout::attach_tab(env, m) {
                    Ok(tab) => {
                        count(rep, lead);
                        rp.steps.push(RestoreStep::Claim { id: id.clone(), pane, rename, cmd: tab.cmd, cwd: tab.cwd, session: s.session_id.clone(), wakes: true });
                        pin_step(env, m, rp);
                    }
                    Err(i) => fail(rp, rep, Issue { severity: Severity::Error, ..i }),
                },
                TabMatch::Taken { pane, running, agent_view } => {
                    let (code, fix) = taken_fix(id, &pane, &running, agent_view);
                    if opts.force && !agent_view {
                        rp.plan.warnings.push(issue("tab.dup", Severity::Warn, format!("{id}'s tab is taken by `{running}` (pane {}); a second tab is opened beside it (--force)", pane.pane_id), None, Some(id.clone())));
                    } else {
                        fail(rp, rep, issue(code, Severity::Error, format!("{id}'s tab is open but pane {} runs `{running}`, not a shell; nothing is typed into it and the job stays as it is", pane.pane_id), Some(fix), Some(id.clone())));
                        return;
                    }
                    wake_then_open(ctx, m, &cwd, wants_tab, rep, rp);
                }
                _ => wake_then_open(ctx, m, &cwd, wants_tab, rep, rp),
            }
        }
        // no row: its conversation from the transcript (the row was removed,
        // R-25; a 0.3.x lead that ended, R-16), else not started
        None => {
            if opts.shape_only {
                rp.plan.warnings.push(issue("layout.skip", Severity::Warn, format!("{id} has no job; no tab (`nimitz start {id}` first)"), None, Some(id.clone())));
                return;
            }
            let rec = crate::registry::load(env).identities.remove(&id.0);
            let current = rec.as_ref().and_then(|r| r.session.clone());
            let has_log = |sid: &str| crate::evidence::transcript(env, &mut crate::evidence::FileCache::default(), cwd.as_deref(), sid).is_some();
            let wanted = lead || opts.all || in_set;
            match (current.filter(|sid| has_log(sid)), cwd.as_deref()) {
                (Some(sid), Some(cwd)) if wanted => {
                    if let Some(i) = crate::check::check(env, ctx.snap, Some(id)).into_iter().find(|i| i.id.as_ref() == Some(id) && i.severity == Severity::Error && !TABLE_HANDLED.contains(&i.code.as_str())) {
                        fail(rp, rep, i);
                        return;
                    }
                    let fod = match crate::config::load_fleet(env, id.fleet()) {
                        Ok(f) => f,
                        Err(i) => {
                            fail(rp, rep, i);
                            return;
                        }
                    };
                    let mode = crate::config::permission_mode(&fod, id);
                    let never_a_job = lead && rec.as_ref().and_then(|r| r.job.clone()).is_none();
                    let why = if never_a_job {
                        format!("{}: resume {} as a background job with every flag (same id and history)", crate::sessions::MIGRATE_LEAD_WHY, &sid[..sid.len().min(8)])
                    } else {
                        format!("resume {} from its transcript with every flag (its job row is gone; Claude Code keeps the id)", &sid[..sid.len().min(8)])
                    };
                    let step = Step { id: Some(id.clone()), cmd: crate::claude::relaunch_bg(env, id, &mode, &sid), cwd: Some(cwd.to_string()), why };
                    count(rep, lead);
                    rp.steps.push(RestoreStep::Wake { id: id.clone(), steps: vec![step] });
                    if wants_tab && !opts.jobs_only {
                        // the job is not there yet: the attach names it once it is
                        let cmd = vec![env.native_claude_bin.clone(), "attach".into(), NEW_JOB.into()];
                        tab_after_wake(ctx, m, cmd, cwd, &sid, rep, rp);
                    }
                }
                _ => rep.not_started += 1, // reported once per fleet (`plan_fleet`)
            }
        }
    }
}

fn count(rep: &mut FleetReport, lead: bool) {
    if lead {
        rep.leads += 1;
    } else {
        rep.ships += 1;
    }
}

/// The `tab.taken` family: the one fix that works, by what runs there.
fn taken_fix(id: &Id, pane: &PaneAt, running: &str, agent_view: bool) -> (&'static str, String) {
    let fleet = id.fleet();
    if running == UNREADABLE {
        return ("tab.unreadable", format!("herdr pane process-info --pane {} failed; retry nimitz restore {fleet}, or rename the tab", pane.pane_id));
    }
    if agent_view {
        return ("tab.agentview", format!("close that pane {} (or Esc / ctrl-c until a shell), then nimitz restore {fleet}", pane.pane_id));
    }
    if running.starts_with("an agent") {
        return ("tab.taken", format!("ctrl-z in pane {} (it shows {}), then nimitz restore {fleet}", pane.pane_id, running.trim_start_matches("an agent: ")));
    }
    ("tab.taken", format!("quit `{running}` in pane {} (or rename the tab), then nimitz restore {fleet}", pane.pane_id))
}

/// A parked or stopped member with no shell tab to wake it from: wake it by
/// its exact id (`wake_steps`), then its tab (a plain attach).
fn wake_then_open(ctx: &Ctx, m: &Member, cwd: &Option<String>, wants_tab: bool, rep: &mut FleetReport, rp: &mut RestorePlan) {
    let (env, id, lead) = (ctx.env, &m.id, m.role.lead());
    let Some(cwd) = cwd.as_deref() else {
        rep.failed += 1;
        rp.plan.warnings.push(issue("folder.missing", Severity::Error, format!("{id} has no folder on this machine"), Some(format!("nimitz ship set {id} --dir <folder|managed>")), Some(id.clone())));
        return;
    };
    let fod = match crate::config::load_fleet(env, id.fleet()) {
        Ok(f) => f,
        Err(i) => {
            rep.failed += 1;
            rp.plan.warnings.push(i);
            return;
        }
    };
    let mode = crate::config::permission_mode(&fod, id);
    match crate::sessions::wake_steps(env, m, &mode, cwd) {
        Ok(steps) => {
            count(rep, lead);
            let s = m.session.as_ref().expect("a row");
            rp.steps.push(RestoreStep::Wake { id: id.clone(), steps });
            if wants_tab && !ctx.opts.jobs_only {
                let cmd = s.short_id.as_ref().map(|j| crate::claude::attach(env, j)).unwrap_or_default();
                tab_after_wake(ctx, m, cmd, cwd, &s.session_id, rep, rp);
            }
            pin_step(env, m, rp);
        }
        Err(i) => {
            rep.failed += 1;
            rp.plan.warnings.push(Issue { severity: Severity::Error, ..i });
        }
    }
}

/// The job id an attach names once a wake that makes a new job (its row was
/// gone) has run; the executor fills it in.
pub const NEW_JOB: &str = "<the new job>";

/// The tab of a member woken by a `Wake` step just before: a plain attach
/// (no second wake) in its shell tab, a new tab, or a report when taken.
fn tab_after_wake(ctx: &Ctx, m: &Member, cmd: Vec<String>, cwd: &str, session: &str, rep: &mut FleetReport, rp: &mut RestorePlan) {
    let id = &m.id;
    let mut extra = vec![];
    let found = ctx.find_tab(m, &cmd, &mut extra);
    rp.plan.warnings.extend(extra);
    match found {
        TabMatch::Shown { .. } => {}
        TabMatch::Shell { pane, rename, .. } => rp.steps.push(RestoreStep::Claim { id: id.clone(), pane, rename, cmd, cwd: cwd.to_string(), session: session.to_string(), wakes: false }),
        TabMatch::Taken { pane, running, agent_view } => {
            let (code, fix) = taken_fix(id, &pane, &running, agent_view);
            if ctx.opts.force && !agent_view {
                rp.plan.warnings.push(issue("tab.dup", Severity::Warn, format!("{id}'s tab is taken by `{running}` (pane {}); a second tab is opened beside it (--force)", pane.pane_id), None, Some(id.clone())));
                rp.steps.push(RestoreStep::Open { id: id.clone(), workspace: workspace_of(id), label: tab_of(id), cmd, cwd: cwd.to_string(), session: session.to_string(), wakes: false });
            } else {
                rep.failed += 1;
                rp.plan.warnings.push(issue(code, Severity::Error, format!("{id}'s tab is open but pane {} runs `{running}`, not a shell; the job wakes, nothing is typed into the pane", pane.pane_id), Some(fix), Some(id.clone())));
            }
        }
        TabMatch::Missing => rp.steps.push(RestoreStep::Open { id: id.clone(), workspace: workspace_of(id), label: tab_of(id), cmd, cwd: cwd.to_string(), session: session.to_string(), wakes: false }),
    }
}

/// The tab of a live member (or, `wakes`, a parked lead in a shape-only pass):
/// found by session (nothing), claimed (its shell pane), opened (new), or taken.
fn tab_step(ctx: &Ctx, m: &Member, wakes: bool, rep: &mut FleetReport, rp: &mut RestorePlan) {
    let (env, opts, id, lead) = (ctx.env, ctx.opts, &m.id, m.role.lead());
    if opts.jobs_only {
        return;
    }
    let tab = match crate::layout::attach_tab(env, m) {
        Ok(t) => t,
        Err(i) => {
            rep.failed += 1;
            rp.plan.warnings.push(Issue { severity: Severity::Error, ..i });
            return;
        }
    };
    let mut extra = vec![];
    let found = ctx.find_tab(m, &tab.cmd, &mut extra);
    rp.plan.warnings.extend(extra);
    match found {
        TabMatch::Shown { pane, tab_label, ws_label } => {
            if norm_label(&tab_label) != norm_label(&tab_of(id)) {
                if opts.rename {
                    count(rep, lead);
                    rp.steps.push(RestoreStep::Rename { id: id.clone(), tab_id: pane.tab_id.clone(), label: tab_of(id) });
                } else {
                    rp.plan.warnings.push(issue("tab.renamed", Severity::Info, format!("tab {tab_label} (pane {}) shows {id}; nimitz looks for {}", pane.pane_id, tab_of(id)), Some(format!("herdr tab rename {} {}  # or nimitz restore {} --rename", pane.tab_id, tab_of(id), id.fleet())), Some(id.clone())));
                }
            } else if norm_label(&ws_label) != norm_label(&workspace_of(id)) {
                rp.plan.warnings.push(issue("tab.moved", Severity::Info, format!("{id}'s tab is in workspace {ws_label}, not {}; left there", workspace_of(id)), None, Some(id.clone())));
            }
            if opts.shape_only {
                rp.plan.warnings.push(issue("layout.skip", Severity::Warn, format!("{id} already has a pane; `nimitz open {id}` shows it"), None, Some(id.clone())));
            }
        }
        TabMatch::Shell { pane, rename, .. } => {
            count(rep, lead);
            rp.steps.push(RestoreStep::Claim { id: id.clone(), pane, rename, cmd: tab.cmd, cwd: tab.cwd, session: tab.session_id.clone().unwrap_or_default(), wakes: wakes && tab.wakes });
            if wakes && tab.wakes {
                pin_step(env, m, rp);
            }
        }
        TabMatch::Taken { pane, running, agent_view } => {
            let (code, fix) = taken_fix(id, &pane, &running, agent_view);
            if opts.force && !agent_view {
                rp.plan.warnings.push(issue("tab.dup", Severity::Warn, format!("{id}'s tab is taken by `{running}` (pane {}); a second tab is opened beside it (--force)", pane.pane_id), None, Some(id.clone())));
                count(rep, lead);
                rp.steps.push(RestoreStep::Open { id: id.clone(), workspace: workspace_of(id), label: tab_of(id), cmd: tab.cmd, cwd: tab.cwd, session: tab.session_id.clone().unwrap_or_default(), wakes: wakes && tab.wakes });
            } else {
                rep.failed += 1;
                rp.plan.warnings.push(issue(code, Severity::Error, format!("{id}'s tab is open but pane {} runs `{running}`, not a shell; nothing is typed into it", pane.pane_id), Some(fix), Some(id.clone())));
            }
        }
        TabMatch::Missing => {
            if wakes && tab.wakes && opts.shape_only {
                // a layout never resumes: the tab is made only for a live lead (`nimitz start` first)
                rp.plan.warnings.push(issue("layout.skip", Severity::Warn, format!("{id} is not live and has no tab to wake it from; no tab (`nimitz start {id}` or `nimitz restore {}` first)", id.fleet()), None, Some(id.clone())));
                return;
            }
            count(rep, lead);
            rp.steps.push(RestoreStep::Open { id: id.clone(), workspace: workspace_of(id), label: tab_of(id), cmd: tab.cmd, cwd: tab.cwd, session: tab.session_id.clone().unwrap_or_default(), wakes: wakes && tab.wakes });
        }
    }
}

/// A lead's job must be pinned (plan-v4 section 7): a missing pin is re-added
/// after the lead is back (R-02, R-25).
fn pin_step(env: &Env, m: &Member, rp: &mut RestorePlan) {
    if !m.role.lead() {
        return;
    }
    if let Some(job) = m.session.as_ref().and_then(|s| s.short_id.clone()) {
        if !crate::pins::is_pinned(env, &job) {
            rp.steps.push(RestoreStep::Pin { id: m.id.clone(), job });
        }
    }
}

struct OldBinary(String, String);
impl OldBinary {
    fn with_id(self, id: &Id) -> Issue {
        issue("cc.old", Severity::Info, format!("{id} runs Claude Code {} under {}; `nimitz relaunch {id}` moves it when idle", self.0, self.1), None, Some(id.clone()))
    }
}

/// A live job still on the binary of before an upgrade adopt (R-09): its row's
/// `cliVersion` against the installed version.
fn old_binary(env: &Env, s: &SessionInfo, snap: &Snapshot) -> Option<OldBinary> {
    let short = s.short_id.as_ref()?;
    let v = crate::evidence::first_json(&std::fs::read(env.claude_home.join("jobs").join(short).join("state.json")).ok()?)?;
    let row = v.get("cliVersion").and_then(|x| x.as_str())?.to_string();
    let now = snap.cc_version.clone()?;
    (row != now).then_some(OldBinary(row, now))
}

/// The run id of a restore: its start time, short.
pub fn run_id() -> String {
    format!("{:x}", (crate::evidence::now() * 1000.0) as u64 % 0xffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_shells() {
        assert_eq!(norm_label("[F.SQ]-fleet"), "F.SQ");
        assert_eq!(norm_label(" admiralty "), "ADMIRALTY");
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_shell(&v(&["-zsh"])) && is_shell(&v(&["/bin/bash"])) && is_shell(&[]));
        assert!(!is_shell(&v(&["claude", "attach", "x"])) && !is_shell(&v(&["yazi"])));
    }

    #[test]
    fn progress_round_trip() {
        let (_td, env) = crate::config::tests::test_env();
        assert!(progress_load(&env).is_none());
        let p = Progress { run: "ab12".into(), started: 1.0, fleets: vec!["F".into()], total: 7, done: 3, steps: vec!["x".into()] };
        progress_write(&env, &p).unwrap();
        assert_eq!(progress_load(&env), Some(p));
        progress_clear(&env);
        assert!(progress_load(&env).is_none());
    }

    #[test]
    fn a_step_displays_as_what_runs() {
        let id = Id::commodore("F", "SQ");
        let claim = RestoreStep::Claim { id: id.clone(), pane: PaneAt { pane_id: "w3:p1".into(), tab_id: "w3:t1".into(), workspace_id: "w3".into() }, rename: Some(("w3:t1".into(), "F.SQ.COMMODORE".into())), cmd: vec!["claude".into(), "attach".into(), "j1".into()], cwd: "/sq".into(), session: "s".into(), wakes: true };
        let d = claim.display();
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].cmd.join(" "), "herdr tab rename w3:t1 F.SQ.COMMODORE");
        assert_eq!(d[1].cmd.join(" "), "herdr pane run w3:p1 claude attach j1");
        assert!(d[1].why.contains("wakes the parked job"));
        let open = RestoreStep::Open { id: id.clone(), workspace: "F.SQ".into(), label: "F.SQ.COMMODORE".into(), cmd: vec!["claude".into(), "attach".into(), "j1".into()], cwd: "/sq".into(), session: "s".into(), wakes: false };
        assert!(open.display()[0].why.starts_with("tab F.SQ.COMMODORE in workspace F.SQ: attach"));
        assert_eq!(RestoreStep::Pin { id, job: "j1".into() }.display()[0].cmd, ["pin", "j1"]);
    }
}
