//! What a key does to a row: the target it names, whether the action is open
//! here (and why not), the core plan it would run, and running it. Plans come
//! from nimitz-core (`sessions::plan_*`, `config::plan_*`); panes from nimitz-mux.
//! Admiralty adds no rules of its own beyond "no multiplexer: monitor-only".

use std::io::Write;
use std::path::{Path, PathBuf};

use nimitz_core::env::Env;
use nimitz_core::{Caller, Id, Member, Outcome, Plan, Role, Snapshot, Step};
use nimitz_mux::{Caps, LayoutPlan, Mux, Pane, PaneRef, Place, TabState};

/// Everything an action needs from the outside world.
pub struct Ctx {
    pub env: Env,
    pub mux: Box<dyn Mux>,
    /// false: fixture data; plans are shown, nothing runs
    pub live: bool,
    pub exit_after_jump: bool,
}

/// What the selected row is about.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Fleet(Id),
    Squadron(Id),
    Member(Id),
    /// a stowaway, by session id
    Stowaway(String),
    /// an extra job row of an identity
    ExtraRow { owner: Id, session_id: String },
    /// crew, tasks, the stowaway group: nothing to act on
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Start,
    Up,
    Stop,
    Relaunch,
    Sweep,
    Rm,
    Adopt,
    Open,
    ConfigRm,
    /// a create or edit form's config change
    Save,
    /// `nimitz layout herdr F`, run by the CLI
    Layout,
    /// the same with `--ships` (a tab attaching each live ship)
    LayoutShips,
    /// `nimitz restore F`, run by the CLI: the fleet back from the Herdr shape
    Restore,
}

impl Act {
    /// The order the action menu lists them in.
    pub const MENU: [Act; 9] = [Act::Start, Act::Up, Act::Open, Act::Stop, Act::Relaunch, Act::Sweep, Act::Rm, Act::Adopt, Act::ConfigRm];

    pub fn key(self) -> char {
        match self {
            Act::Start => 'a',
            Act::Up => 'u',
            Act::Open => 'A',
            Act::Stop => 's',
            Act::Relaunch => 'r',
            Act::Sweep => 'w',
            Act::Rm => 'x',
            Act::Adopt => 'd',
            Act::ConfigRm => 'D',
            Act::Save => 'c',
            Act::Layout => 'L',
            Act::LayoutShips => 'S',
            Act::Restore => 'R',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Act::Start => "start (top-down: admiral, commodore, then this)",
            Act::Up => "up: start everything this fleet runs here",
            Act::Open => "open: attach in a new pane",
            Act::Stop => "stop",
            Act::Relaunch => "relaunch (same session id and history; labels, name and mode restored)",
            Act::Sweep => "sweep: stop and remove copies, remove stale job rows",
            Act::Rm => "rm job rows (transcripts kept)",
            Act::Adopt => "adopt as a ship",
            Act::ConfigRm => "remove from the config",
            Act::Save => "save the config change",
            Act::Layout => "layout herdr: workspaces and tabs for this fleet",
            Act::LayoutShips => "layout herdr --ships: and a tab attaching each live ship",
            Act::Restore => "restore: bring this fleet back from the Herdr shape (leads always, ships from the last active set)",
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Act::Start => "start",
            Act::Up => "up",
            Act::Open => "open",
            Act::Stop => "stop",
            Act::Relaunch => "relaunch",
            Act::Sweep => "sweep",
            Act::Rm => "rm",
            Act::Adopt => "adopt",
            Act::ConfigRm => "remove",
            Act::Save => "save",
            Act::Layout | Act::LayoutShips => "layout",
            Act::Restore => "restore",
        }
    }

    /// The key that does this from the main list: its own key when the list
    /// binds one (a, u, A, D), else `x` (the actions menu) then its menu key.
    pub fn main_key(self) -> String {
        match self {
            Act::Start | Act::Up | Act::Open | Act::ConfigRm => self.key().to_string(),
            other => format!("x {}", other.key()),
        }
    }

    fn needs_mux(self) -> bool {
        matches!(self, Act::Up | Act::Open | Act::Layout | Act::LayoutShips | Act::Restore)
    }
}

pub const MONITOR_ONLY: &str = "monitor-only: no multiplexer here (run Admiralty inside Herdr to open, jump or type)";

/// A plan ready for the confirm dialog.
#[derive(Clone, Debug)]
pub struct Pending {
    pub act: Act,
    /// the `nimitz` command that does the same
    pub command: String,
    pub plan: Plan,
    /// a config plan (apply with `config::apply_config_plan`)
    pub config: bool,
    /// steps the multiplexer opens as panes (`open`: the attach), with where
    pub panes: Vec<(Step, Place)>,
    /// a layout for the multiplexer (core's `layout::plan`, mapped onto the mux)
    pub layout: Option<LayoutPlan>,
}

/// A `nimitz` process that sees exactly this Env (contracts.md section 2) and,
/// with no NIMITZ_* labels, runs as the user, never as a member (for tests and
/// tools that check Admiralty against the CLI).
pub fn cli_command(env: &Env, bin: &str) -> std::process::Command {
    let mut c = std::process::Command::new(bin);
    for k in ["NIMITZ_FLEET", "NIMITZ_SQUADRON", "NIMITZ_ROLE", "NIMITZ_ID", "NIMITZ_PARENT"] {
        c.env_remove(k);
    }
    let p = |x: &Path| x.to_string_lossy().into_owned();
    c.env("NIMITZ_HOME", p(&env.home))
        .env("NIMITZ_STATE", p(&env.state))
        .env("NIMITZ_CLAUDE_HOME", p(&env.claude_home))
        .env("NIMITZ_CLAUDE_BIN", &env.claude_bin)
        .env("NIMITZ_NATIVE_CLAUDE_BIN", &env.native_claude_bin)
        .env("NIMITZ_HOSTNAME", &env.hostname);
    match &env.mux {
        Some(m) => c.env("NIMITZ_MUX", m),
        None => c.env_remove("NIMITZ_MUX"),
    };
    if let Some(h) = &env.herdr_bin {
        c.env("NIMITZ_HERDR_BIN", h);
    }
    c
}

// ------------------------------------------------------------------ lookups

pub fn find_member<'a>(snap: &'a Snapshot, id: &Id) -> Option<&'a Member> {
    for f in &snap.fleets {
        if &f.admiral.id == id {
            return Some(&f.admiral);
        }
        for sq in &f.squadrons {
            if &sq.commodore.id == id {
                return Some(&sq.commodore);
            }
            if let Some(m) = sq.ships.iter().find(|m| &m.id == id) {
                return Some(m);
            }
        }
    }
    None
}

fn live(m: &Member) -> bool {
    m.session.as_ref().map(|s| s.pid.is_some()).unwrap_or(false)
}

fn short_of(s: &nimitz_core::SessionInfo) -> String {
    s.short_id.clone().unwrap_or_else(|| s.session_id.chars().take(8).collect())
}

/// Shell-quote one argument only when it needs it.
pub fn quote(a: &str) -> String {
    if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+~[]".contains(c)) {
        a.to_string()
    } else {
        format!("'{}'", a.replace('\'', "'\\''"))
    }
}

pub fn shell(cmd: &[String]) -> String {
    cmd.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
}

// ------------------------------------------------------------------ availability

/// Is `act` open for `target`? `Err` carries the reason shown to the user.
pub fn available(caps: Caps, snap: &Snapshot, target: &Target, act: Act) -> Result<(), String> {
    if act.needs_mux() && !caps.open {
        return Err(MONITOR_ONLY.into());
    }
    match (act, target) {
        (Act::Save, _) => Ok(()),
        (Act::Layout | Act::LayoutShips, _) if !caps.layout => Err("the multiplexer here has no layout (Herdr has)".into()),
        (Act::Layout | Act::LayoutShips, Target::Fleet(id)) => match snap.fleets.iter().find(|f| &f.id == id) {
            Some(f) if f.machine.is_some() => Ok(()),
            Some(_) => Err("this fleet does not run on this machine".into()),
            None => Err("not in the snapshot".into()),
        },
        (Act::Layout | Act::LayoutShips, _) => Err("layout is for a fleet (select its band)".into()),
        (Act::Restore, Target::Fleet(id)) => match snap.fleets.iter().find(|f| &f.id == id) {
            Some(f) if f.machine.is_some() => Ok(()),
            Some(_) => Err("this fleet does not run on this machine".into()),
            None => Err("not in the snapshot".into()),
        },
        (Act::Restore, _) => Err("restore is for a fleet (select its band)".into()),
        (_, Target::None) => Err("nothing to act on here".into()),
        (Act::Start, Target::Member(id)) => {
            let m = find_member(snap, id).ok_or("not in the snapshot")?;
            if live(m) && m.issues.is_empty() {
                Err("already running".into())
            } else {
                Ok(())
            }
        }
        (Act::Start, Target::Squadron(id)) => available(caps, snap, &Target::Member(id.clone()), act),
        (Act::Start, Target::Fleet(id)) => available(caps, snap, &Target::Member(id.clone()), act),
        (Act::Start, _) => Err("start names a member".into()),
        (Act::Up, Target::Fleet(id)) => match snap.fleets.iter().find(|f| &f.id == id) {
            Some(f) if f.machine.is_some() => Ok(()),
            Some(_) => Err("this fleet does not run on this machine".into()),
            None => Err("not in the snapshot".into()),
        },
        (Act::Up, _) => Err("up is for a fleet (select its band)".into()),
        (Act::Open, Target::Member(id)) => {
            // a job, live or parked (an attach wakes a parked job, cli-07)
            let m = find_member(snap, id).ok_or("not in the snapshot")?;
            let s = m.session.as_ref().ok_or("no job yet; `a` starts it")?;
            if s.short_id.is_none() {
                return Err(if m.pane.is_some() {
                    "a process with no job: enter jumps to its pane".into()
                } else {
                    "a process with no job (interactive or SDK): it lives in the terminal that started it".into()
                });
            }
            Ok(())
        }
        (Act::Open, Target::Stowaway(sid)) => match snap.stowaways.iter().find(|s| &s.session_id == sid) {
            Some(s) if s.pid.is_some() && s.short_id.is_some() => Ok(()),
            Some(s) if s.pid.is_some() => Err("a process with no job (interactive or SDK): it lives in the terminal that started it".into()),
            _ => Err("not running".into()),
        },
        (Act::Open, _) => Err("open names a running session".into()),
        (Act::Stop, Target::Member(id)) => {
            let m = find_member(snap, id).ok_or("not in the snapshot")?;
            if live(m) || m.extra_rows.iter().any(|r| r.pid.is_some()) {
                Ok(())
            } else {
                Err("not running".into())
            }
        }
        (Act::Stop, _) => Err("stop names a member".into()),
        (Act::Relaunch, Target::Member(id)) => {
            let m = find_member(snap, id).ok_or("not in the snapshot")?;
            if m.session.is_some() {
                Ok(())
            } else {
                Err("no session to relaunch; `a` starts it".into())
            }
        }
        (Act::Relaunch, _) => Err("relaunch names a member".into()),
        (Act::Sweep, Target::Member(id)) => {
            let m = find_member(snap, id).ok_or("not in the snapshot")?;
            if m.extra_rows.is_empty() {
                Err("no stale job rows".into())
            } else {
                Ok(())
            }
        }
        (Act::Sweep, Target::Fleet(_) | Target::Squadron(_)) => Ok(()),
        (Act::Sweep, _) => Err("sweep names a member, a squadron or a fleet".into()),
        (Act::Rm, Target::Stowaway(sid)) => match snap.stowaways.iter().find(|s| &s.session_id == sid) {
            Some(s) if s.pid.is_none() => Ok(()),
            Some(_) => Err("live: stop it first".into()),
            None => Err("not in the snapshot".into()),
        },
        (Act::Rm, Target::ExtraRow { owner, session_id }) => {
            let m = find_member(snap, owner).ok_or("not in the snapshot")?;
            match m.extra_rows.iter().find(|r| &r.session_id == session_id) {
                Some(r) if r.pid.is_none() => Ok(()),
                Some(_) => Err("a live copy: stop it first (see the issue's fix)".into()),
                None => Err("not in the snapshot".into()),
            }
        }
        (Act::Rm, Target::Member(id)) => available(caps, snap, &Target::Member(id.clone()), Act::Sweep),
        (Act::Rm, _) => Err("rm names a job row, a stowaway or a member".into()),
        (Act::Adopt, Target::Stowaway(_)) => Ok(()),
        (Act::Adopt, _) => Err("adopt is for stowaways".into()),
        (Act::ConfigRm, Target::Member(id)) if id.role() == Role::Ship => Ok(()),
        (Act::ConfigRm, Target::Squadron(_) | Target::Fleet(_)) => Ok(()),
        (Act::ConfigRm, _) => Err("remove names a fleet, a squadron or a ship".into()),
    }
}

// ------------------------------------------------------------------ plans

fn member_id(target: &Target) -> Option<Id> {
    match target {
        Target::Member(id) | Target::Squadron(id) | Target::Fleet(id) => Some(id.clone()),
        Target::ExtraRow { owner, .. } => Some(owner.clone()),
        _ => None,
    }
}

/// The pane a member opens in, as `nimitz` places it: workspace `F` or
/// `F.SQ`, tab `F.ADMIRAL`, `F.SQ.COMMODORE`, `F.SQ.SHIP` or `F.SQ.~task`.
pub fn place_for(id: &Id) -> Place {
    Place::Tab { workspace: nimitz_core::layout::workspace_of(id), label: nimitz_core::layout::tab_of(id) }
}

/// A `legacy` warning belongs to the member it names: keep it only in that
/// member's own dialog.
fn only_own_legacy(p: &mut Pending, target: Option<&Id>) {
    p.plan.warnings.retain(|w| w.code != "legacy" || w.id.as_ref() == target);
}

/// A core plan as a pending action: every step is `apply`'s (every member is
/// a background job), so nothing opens a pane.
fn with_panes(act: Act, command: String, plan: Plan) -> Pending {
    Pending { act, command, plan, config: false, panes: vec![], layout: None }
}

/// Build the core plan for `act` on `target`; `caps` are what the loader's
/// multiplexer reported; `adopt_as` names the identity for adopt.
pub fn prepare(ctx: &Ctx, caps: Caps, snap: &Snapshot, target: &Target, act: Act, adopt_as: Option<&Id>) -> Result<Pending, String> {
    available(caps, snap, target, act)?;
    let env = &ctx.env;
    let user = Caller::User;
    let p = match (act, target) {
        (Act::Start, _) => {
            let id = member_id(target).ok_or("no member")?;
            with_panes(act, format!("nimitz start {id}"), nimitz_core::sessions::plan_start(env, snap, &id, &user, false))
        }
        (Act::Up, Target::Fleet(id)) => with_panes(act, format!("nimitz up {id}"), nimitz_core::sessions::plan_up(env, snap, id.fleet(), &user)),
        (Act::Stop, _) => {
            let id = member_id(target).ok_or("no member")?;
            with_panes(act, format!("nimitz stop {id}"), nimitz_core::sessions::plan_stop(env, snap, &id, &user))
        }
        (Act::Relaunch, _) => {
            let id = member_id(target).ok_or("no member")?;
            with_panes(act, format!("nimitz relaunch {id}"), nimitz_core::sessions::plan_relaunch(env, snap, &id, &user))
        }
        (Act::Sweep, _) => {
            let id = member_id(target).ok_or("no member")?;
            with_panes(act, format!("nimitz sweep {id}"), nimitz_core::sessions::plan_sweep(env, snap, Some(&id), &user))
        }
        (Act::Rm, Target::Stowaway(sid)) => {
            let short = snap.stowaways.iter().find(|s| &s.session_id == sid).map(short_of).unwrap_or(sid.clone());
            with_panes(act, format!("nimitz rm {short}"), nimitz_core::sessions::plan_rm(env, snap, &short, &user))
        }
        (Act::Rm, Target::ExtraRow { owner, session_id }) => {
            let short = find_member(snap, owner)
                .and_then(|m| m.extra_rows.iter().find(|r| &r.session_id == session_id))
                .map(short_of)
                .unwrap_or(session_id.clone());
            with_panes(act, format!("nimitz rm {short}"), nimitz_core::sessions::plan_rm(env, snap, &short, &user))
        }
        (Act::Rm, Target::Member(id)) => with_panes(act, format!("nimitz rm {id}"), nimitz_core::sessions::plan_rm(env, snap, &id.0, &user)),
        (Act::Adopt, Target::Stowaway(sid)) => {
            let id = adopt_as.ok_or("adopt needs an identity")?;
            let short = snap.stowaways.iter().find(|s| &s.session_id == sid).map(short_of).unwrap_or(sid.clone());
            with_panes(act, format!("nimitz adopt {short} {id}"), nimitz_core::sessions::plan_adopt(env, snap, sid, id, &user))
        }
        (Act::Open, _) => plan_open(ctx, snap, target)?,
        (Act::ConfigRm, _) => {
            // retire: stop, rm the job rows, then the config (core's plan; `apply` runs all of it)
            let id = member_id(target).ok_or("no member")?;
            let what = match id.role() {
                Role::Admiral => "fleet",
                Role::Commodore => "squadron",
                _ => "ship",
            };
            with_panes(act, format!("nimitz {what} rm {id}"), nimitz_core::sessions::plan_retire(env, snap, &user, &id))
        }
        _ => return Err(format!("{} does not apply here", act.word())),
    };
    let mut p = p;
    if matches!(act, Act::Start | Act::Relaunch | Act::Stop) {
        only_own_legacy(&mut p, member_id(target).as_ref());
    }
    Ok(p)
}

/// `layout herdr F [--ships]`: core's layout for the fleet, minus tabs whose
/// session already has a pane, mapped onto the multiplexer's layout plan.
pub fn prepare_layout(ctx: &Ctx, caps: Caps, snap: &Snapshot, panes: &[Pane], fleet: &Id, ships: bool) -> Result<Pending, String> {
    let act = if ships { Act::LayoutShips } else { Act::Layout };
    available(caps, snap, &Target::Fleet(fleet.clone()), act)?;
    let mut spec = nimitz_core::layout::plan(&ctx.env, snap, Some(fleet.fleet()), ships && caps.open, &Caller::User, false);
    let shown: Vec<String> = panes.iter().filter_map(|p| p.session_id.clone()).collect();
    nimitz_core::layout::drop_shown(&mut spec, &shown);
    let lp = LayoutPlan {
        workspaces: spec
            .workspaces
            .iter()
            .map(|w| (w.label.clone(), w.tabs.iter().map(|t| (t.label.clone(), t.cmd.clone(), PathBuf::from(&t.cwd))).collect()))
            .collect(),
    };
    let mut plan = spec.plan;
    plan.summary = format!("layout herdr {fleet}{}", if ships { " --ships" } else { "" });
    let command = format!("nimitz layout herdr {fleet}{}", if ships { " --ships" } else { "" });
    Ok(Pending { act, command, plan, config: false, panes: vec![], layout: Some(lp) })
}

/// The `nimitz` binary a restore runs: `NIMITZ_BIN`, else the one beside
/// this binary (or beside its folder: `target/debug/deps/` in a test), else
/// `nimitz` on PATH.
pub fn nimitz_bin() -> String {
    if let Some(b) = std::env::var_os("NIMITZ_BIN").filter(|b| !b.is_empty()) {
        return b.to_string_lossy().into_owned();
    }
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.parent().into_iter().chain(exe.parent().and_then(Path::parent)) {
            let p = dir.join("nimitz");
            if p.is_file() {
                return p.to_string_lossy().into_owned();
            }
        }
    }
    "nimitz".into()
}

/// The last JSON value on a CLI's stdout (`--json` prints the plan or the
/// outcome there; the human lines go to stderr).
fn last_json<T: serde::de::DeserializeOwned>(stdout: &str) -> Option<T> {
    let text = stdout.trim();
    let start = text.rfind("\n{").map(|i| i + 1).unwrap_or(0);
    serde_json::from_str(&text[start..]).ok().or_else(|| serde_json::from_str(text).ok())
}

/// `R` on a fleet band: `nimitz restore F --plan --json` gives the plan the
/// confirm dialog shows; `y` then runs `nimitz restore F --yes` (`apply`).
/// The CLI holds the whole engine (the Herdr shape, the wakes, the tabs), so
/// Admiralty says the same as the command line (M25).
pub fn prepare_restore(ctx: &Ctx, caps: Caps, snap: &Snapshot, fleet: &Id) -> Result<Pending, String> {
    available(caps, snap, &Target::Fleet(fleet.clone()), Act::Restore)?;
    let bin = nimitz_bin();
    let out = cli_command(&ctx.env, &bin).args(["restore", fleet.fleet(), "--plan", "--json"]).output().map_err(|e| format!("{bin}: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let plan: Plan = last_json(&stdout).ok_or_else(|| {
        let err = String::from_utf8_lossy(&out.stderr);
        format!("`{bin} restore {} --plan --json` gave no plan: {}", fleet.fleet(), err.lines().last().unwrap_or("no output"))
    })?;
    Ok(Pending { act: Act::Restore, command: format!("nimitz restore {}", fleet.fleet()), plan, config: false, panes: vec![], layout: None })
}

/// Run `nimitz restore F --yes --json` and read its outcome (a run with
/// nothing to restore prints none: then the exit code is the verdict).
fn run_restore(env: &Env, command: &str) -> Outcome {
    let bin = nimitz_bin();
    let args: Vec<&str> = command.split_whitespace().skip(1).collect(); // after `nimitz`
    let out = match cli_command(env, &bin).args(&args).args(["--yes", "--json"]).output() {
        Ok(o) => o,
        Err(e) => {
            return Outcome { ran: vec![], verified: false, issues: vec![nimitz_core::Issue { code: "step.failed".into(), severity: nimitz_core::Severity::Error, message: format!("{bin}: {e}"), fix: None, id: None }] };
        }
    };
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout);
    if let Some(o) = last_json::<Outcome>(&stdout) {
        return o;
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let last = stderr.lines().rev().find(|l| l.starts_with("[OK]") || l.starts_with("[ERROR]")).unwrap_or("").to_string();
    let issues = if code == 0 { vec![] } else { vec![nimitz_core::Issue { code: "step.failed".into(), severity: nimitz_core::Severity::Error, message: format!("{command} exited {code}: {last}"), fix: None, id: None }] };
    Outcome { ran: vec![], verified: code == 0, issues }
}

/// What the tab `A` would attach in shows now (`nimitz_mux::tab_state`): a
/// pane with the session (jump there), a plain shell (claim it: the attach
/// runs there), something else (`tab.taken`: never typed into), or no tab
/// (`Missing`: open one, after the confirm). `Err` with the issue when the
/// attach is refused (`history.lost`, `name.drift`, ...: its fix says what
/// first) or the pane cannot be read into. The attach step itself is `plan_open`'s.
pub fn existing_open(ctx: &Ctx, snap: &Snapshot, target: &Target) -> Result<TabState, nimitz_core::Issue> {
    if let Target::Member(id) = target {
        let m = find_member(snap, id).ok_or_else(|| refusal_issue(id, "not in the snapshot".into()))?;
        nimitz_core::layout::attach_tab(&ctx.env, m)?;
    }
    let Ok(p) = plan_open(ctx, snap, target) else { return Ok(TabState::Missing) };
    let Some((step, place)) = p.panes.first() else { return Ok(TabState::Missing) };
    let sid = match target {
        Target::Member(id) => find_member(snap, id).and_then(|m| m.session.as_ref()).map(|s| s.session_id.clone()),
        Target::Stowaway(sid) => Some(sid.clone()),
        _ => None,
    };
    nimitz_mux::tab_state(ctx.mux.as_ref(), &step.cmd, place, sid.as_deref()).map_err(|e| refusal_issue(&member_id(target).unwrap_or(Id("?".into())), format!("cannot see what its tab runs ({e}); nothing is typed into it")))
}

fn refusal_issue(id: &Id, message: String) -> nimitz_core::Issue {
    nimitz_core::Issue { code: "mux.failed".into(), severity: nimitz_core::Severity::Error, message, fix: None, id: Some(id.clone()) }
}

/// `A` on a tab whose pane is a plain shell: run the attach there and focus it.
pub fn claim_open(ctx: &Ctx, snap: &Snapshot, target: &Target, pane: &PaneRef) -> Result<(), String> {
    let p = plan_open(ctx, snap, target)?;
    let (step, _) = p.panes.first().ok_or("nothing to attach")?;
    ctx.mux.run_in(pane, &step.cmd).map_err(|e| e.to_string())?;
    ctx.mux.focus(pane).map_err(|e| e.to_string())
}

fn plan_open(ctx: &Ctx, snap: &Snapshot, target: &Target) -> Result<Pending, String> {
    let (step, place, name) = match target {
        Target::Member(id) => {
            let m = find_member(snap, id).ok_or("not in the snapshot")?;
            let tab = nimitz_core::layout::attach_tab(&ctx.env, m).map_err(|i| i.message)?;
            let wake = if tab.wakes { "; it wakes the parked job" } else { "" };
            let step = Step { id: Some(id.clone()), cmd: tab.cmd, cwd: Some(tab.cwd), why: format!("attach to {} in tab {}{wake}; detaching (ctrl-z) leaves it running", m.expected_name, tab.label) };
            (step, place_for(id), m.expected_name.clone())
        }
        Target::Stowaway(sid) => {
            let s = snap.stowaways.iter().find(|s| &s.session_id == sid).cloned().ok_or("not in the snapshot")?;
            let short = s.short_id.clone().ok_or("a process with no job: nothing to attach to")?;
            let step = Step {
                id: Some(Id(format!("stowaway {short}"))),
                cmd: nimitz_core::claude::attach(&ctx.env, &short),
                cwd: Some(s.cwd.clone()),
                why: format!("attach to {} in a new pane; detaching leaves it running", s.name),
            };
            (step, Place::Tab { workspace: "STOWAWAYS".into(), label: s.name.clone() }, s.name.clone())
        }
        _ => return Err("open names a running session".into()),
    };
    let plan = Plan { summary: format!("open {name}"), steps: vec![step.clone()], ..Default::default() };
    let command = match target {
        Target::Member(id) => format!("nimitz open {id}"),
        _ => shell(&step.cmd),
    };
    Ok(Pending { act: Act::Open, command, plan, config: false, panes: vec![(step, place)], layout: None })
}

// ------------------------------------------------------------------ running

/// Open the plan's pane steps, or lay out its workspaces, in the multiplexer
/// (main thread: the adapter lives here).
pub fn open_panes(ctx: &Ctx, p: &Pending) -> Result<Vec<PaneRef>, String> {
    if let Some(lp) = &p.layout {
        return ctx.mux.layout(lp).map_err(|e| format!("layout: {e}"));
    }
    let mut out = vec![];
    for (step, place) in &p.panes {
        let cwd = PathBuf::from(step.cwd.clone().unwrap_or_else(|| ".".into()));
        match ctx.mux.open(&step.cmd, &cwd, place.clone()) {
            Ok(r) => out.push(r),
            Err(e) => return Err(format!("{}: {e}", step.id.as_ref().map(|i| i.0.as_str()).unwrap_or("pane"))),
        }
    }
    Ok(out)
}

/// Run the non-pane part of the plan (any thread). Core skips pane steps itself.
pub fn apply(env: &Env, p: &Pending) -> Outcome {
    if p.act == Act::Open || p.layout.is_some() {
        return Outcome { verified: true, ..Default::default() };
    }
    if p.act == Act::Restore {
        return run_restore(env, &p.command);
    }
    if p.config {
        nimitz_core::config::apply_config_plan(env, &p.plan)
    } else {
        nimitz_core::sessions::apply(env, &p.plan)
    }
}

/// One line for the footer: what happened.
pub fn outcome_text(p: &Pending, o: &Outcome) -> (bool, String) {
    let failed = o.ran.iter().find(|(_, rc, _)| *rc != 0);
    if let Some(i) = o.issues.first() {
        let fix = i.fix.as_ref().map(|f| format!("  fix: {f}")).unwrap_or_default();
        return (false, format!("{}: {} [{}] {}{fix}", p.act.word(), p.plan.summary, i.code, i.message));
    }
    if let Some((s, rc, text)) = failed {
        return (false, format!("{}: step failed (exit {rc}): {} {}", p.act.word(), shell(&s.cmd), text.lines().last().unwrap_or("")));
    }
    if let Some(lp) = &p.layout {
        let tabs: usize = lp.workspaces.iter().map(|(_, t)| t.len()).sum();
        return (true, format!("layout: {} ({tabs} tab(s) in {} workspace(s); members bind at their SessionStart)", p.plan.summary, lp.workspaces.len()));
    }
    if p.act == Act::Restore && o.ran.is_empty() {
        return (true, format!("restore: {}: nothing to restore", p.plan.summary.trim_start_matches("restore ")));
    }
    let checked = if o.verified || p.config { "verified" } else { "done" };
    (true, format!("{}: {} ({} steps, {checked})", p.act.word(), p.plan.summary, o.ran.len()))
}

/// Append one line to the action log (never fails the action).
pub fn log(env: &Env, what: &str, detail: &str) {
    let path = env.state.join("admiralty-actions.log");
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let line = serde_json::json!({"ts": nimitz_core::evidence::now(), "what": what, "detail": detail});
        let _ = writeln!(f, "{line}");
    }
}

// ------------------------------------------------------------------ panes, messages

pub fn pane_ref(panes: &[Pane], pane_id: &str) -> Option<PaneRef> {
    panes.iter().find(|p| p.pane.pane_id == pane_id).map(|p| p.pane.clone())
}

/// Where `enter` jumps: the row's pane, if the multiplexer can focus it.
pub fn jump_target(caps: Caps, pane_id: Option<&str>, panes: &[Pane]) -> Result<PaneRef, String> {
    if !caps.focus {
        return Err(MONITOR_ONLY.into());
    }
    let id = pane_id.ok_or("no pane hosts this session")?;
    pane_ref(panes, id).ok_or_else(|| format!("pane {id} is gone"))
}

/// Who `m` reaches and through which pane.
#[derive(Clone, Debug, PartialEq)]
pub struct MessagePlan {
    /// the identity the message is for
    pub id: Id,
    /// the member the message is for
    pub to: String,
    /// whose pane it is typed into
    pub via: String,
    pub pane: PaneRef,
    /// true: typed to the commodore, who relays it with SendMessage
    pub relay: bool,
}

pub fn plan_message(caps: Caps, snap: &Snapshot, panes: &[Pane], target: &Target) -> Result<MessagePlan, String> {
    if !caps.type_text {
        return Err(MONITOR_ONLY.into());
    }
    let Target::Member(id) = target else {
        return Err(match target {
            Target::Stowaway(_) => "a stowaway has no chain of command; adopt it first".into(),
            Target::Fleet(id) | Target::Squadron(id) => return plan_message(caps, snap, panes, &Target::Member(id.clone())),
            _ => "message names a member".into(),
        });
    };
    let m = find_member(snap, id).ok_or("not in the snapshot")?;
    let (via_m, relay) = match m.role {
        Role::Admiral | Role::Commodore => (m, false),
        Role::Ship => {
            let sq = id.parent().ok_or("no squadron")?;
            (find_member(snap, &sq).ok_or("its commodore is not in the snapshot")?, true)
        }
    };
    if !live(via_m) {
        return Err(if relay {
            format!("its commodore {} is not running; relay goes through it", via_m.expected_name)
        } else {
            "not running".into()
        });
    }
    let pane_id = via_m.pane.as_deref().ok_or_else(|| format!("{} has no pane to type into", via_m.expected_name))?;
    let pane = pane_ref(panes, pane_id).ok_or_else(|| format!("pane {pane_id} is gone"))?;
    if relay && !live(m) {
        return Err(format!("{} is not running", m.expected_name));
    }
    Ok(MessagePlan { id: id.clone(), to: m.expected_name.clone(), via: via_m.expected_name.clone(), pane, relay })
}

/// The CLI command that sends the same message (its text as it will be sent).
pub fn signal_command(p: &MessagePlan, text: &str) -> String {
    format!("nimitz signal {} {}", quote(&p.id.0), quote(&nimitz_mux::sanitize_text(text)))
}

/// The text typed into the pane: sanitised (no escape sequences or control
/// characters, newlines as spaces), the same floor the Herdr adapter applies.
pub fn message_text(p: &MessagePlan, text: &str) -> String {
    let text = nimitz_mux::sanitize_text(text);
    if p.relay {
        format!("Relay to {} from the user: {text}", p.to)
    } else {
        text.to_string()
    }
}

/// Write a file atomically (temp + rename).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}
