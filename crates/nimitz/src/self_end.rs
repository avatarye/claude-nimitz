//! The self-end helper (docs/design/cli-ux.md section 4): a member's command
//! that would end or replace the member itself (its own relaunch, its stop,
//! a retire that takes it) runs everything else, prints its report, and
//! leaves that one step to `nimitz __self-end`, detached (`os::spawn_detached`:
//! it survives a `claude stop` of the caller's job). The helper waits for the
//! caller's turn to end (its job idle or gone; it gives up after
//! `self_end::WAIT_S`, never cutting a turn), runs the step as a plain
//! `nimitz <args> --yes` with no member labels (the caller is no longer above
//! it), logs to `NIMITZ_STATE/relaunch-self.log` and leaves the outcome for
//! the member's next SessionStart. One helper per member (its lock).
use crate::{out, Ctx};
use nimitz_core::env::Env;
use nimitz_core::self_end;
use nimitz_core::Id;
use std::collections::HashMap;
use std::process::{Command, Stdio};

/// The member that runs this command (`sessions::me_of`: its labels, else
/// its process), when one does.
pub fn me(ctx: &Ctx, snap: &nimitz_core::Snapshot) -> Option<Id> {
    nimitz_core::sessions::me_of(snap, &ctx.caller)
}

/// Leave `nimitz <args>` to the helper, for `id` (the caller): said under
/// `--plan`; else the lock, then the detached helper. Returns the exit code.
pub fn defer(ctx: &Ctx, id: &Id, args: &[String]) -> i32 {
    let shown = format!("nimitz {}", args.join(" "));
    if ctx.plan_only() {
        out::line(&format!("then, after this command and once {id}'s turn ends: {shown} (a detached helper)"));
        return 0;
    }
    if let Err(e) = self_end::claim(&ctx.env, id) {
        out::error(&format!("{shown} is not left to a helper: {e}"));
        return 1;
    }
    // the job whose turn the helper waits for
    let snap = ctx.snap();
    let job = snap.fleets.iter().flat_map(|f| f.members()).find(|m| &m.id == id).and_then(|m| m.session.as_ref()).and_then(|s| s.short_id.clone()).unwrap_or_default();
    match spawn(ctx, id, &job, args) {
        Ok(()) => {
            self_end::log(&ctx.env, &format!("{id}: {shown} left to a helper (waits for job {job})"));
            if !nimitz_core::os::detach_outlives_job() {
                self_end::log(&ctx.env, &format!("{id}: this job object allows no breakaway: the helper may end with the caller's job"));
            }
            out::line(&format!("[INFO] {id}: `{shown}` runs after this command, once its turn ends (a detached helper; log {}); the outcome comes with its next session start", self_end::log_path(&ctx.env).display()));
            0
        }
        Err(e) => {
            self_end::release(&ctx.env, id);
            out::error(&format!("could not start the helper for `{shown}`: {e}"));
            1
        }
    }
}

fn spawn(ctx: &Ctx, id: &Id, job: &str, args: &[String]) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(self_end::log_path(&ctx.env))?;
    // the member's Herdr (its recorded server), and nothing of the member's
    // session: no labels (the helper's nimitz runs as yours, so it relaunches
    // or stops the member), no Claude Code session, no pane of its own
    let mut vars = nimitz_core::active::member_mux_vars(&ctx.env, &ctx.vars);
    vars.retain(|k, _| !matches!(k.as_str(), "NIMITZ_ID" | "NIMITZ_ROLE" | "NIMITZ_FLEET" | "NIMITZ_SQUADRON" | "NIMITZ_PARENT" | "CLAUDECODE" | "CLAUDE_CODE_SESSION_ID") && !k.starts_with("HERDR_"));
    let mut c = Command::new(exe);
    c.arg("__self-end").arg(&id.0).arg(job).arg("--").args(args);
    let _ = std::fs::create_dir_all(&ctx.env.state);
    c.env_clear().envs(&vars).current_dir(&ctx.env.state).stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log);
    nimitz_core::os::spawn_detached(&mut c)
}

/// `nimitz __self-end <id> <job> -- <args>...`: the helper itself.
pub fn helper(argv: &[String]) -> i32 {
    let vars: HashMap<String, String> = nimitz_core::os::env_vars();
    let env = Env::from_vars(&vars);
    let (Some(id), Some(job), Some("--")) = (argv.first(), argv.get(1), argv.get(2).map(String::as_str)) else { return 2 };
    let id = Id(id.clone());
    let args = &argv[3..];
    self_end::hold(&env, &id, std::process::id());
    let code = run(&env, &vars, &id, job, args);
    self_end::release(&env, &id);
    code
}

fn run(env: &Env, vars: &HashMap<String, String>, id: &Id, job: &str, args: &[String]) -> i32 {
    let shown = format!("nimitz {}", args.join(" "));
    let wait: f64 = vars.get("NIMITZ_SELF_END_WAIT").and_then(|v| v.parse().ok()).unwrap_or(self_end::WAIT_S);
    let verb = args.first().map(String::as_str).unwrap_or_default();
    // a retire (`ship rm`, ...) removes the member: no note for a later member of that id
    let removes = args.get(1).is_some_and(|a| a == "rm");
    if let Err(why) = wait_turn(env, id, job, wait) {
        let msg = format!("`{shown}` did not run: {why}");
        self_end::log(env, &format!("{id}: gave up: {msg}"));
        self_end::write_note(env, id, &msg);
        return 1;
    }
    self_end::log(env, &format!("{id}: its turn ended; runs {shown}"));
    // said before it runs: a relaunch's own SessionStart comes during the run
    if verb == "restart" {
        let running = format!("`{shown}`, which you ran, ran after your turn ended; this session start is its relaunch (log {})", self_end::log_path(env).display());
        self_end::write_note(env, id, &running);
    }
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            self_end::write_note(env, id, &format!("`{shown}` did not run: no nimitz binary ({e})"));
            return 1;
        }
    };
    let o = Command::new(exe).args(args).arg("--yes").stdin(Stdio::null()).output();
    let (code, text) = match o {
        Ok(o) => (o.status.code().unwrap_or(1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))),
        Err(e) => (1, e.to_string()),
    };
    for l in text.lines() {
        self_end::log(env, &format!("{id}: | {l}"));
    }
    let last: String = text.lines().rev().find(|l| l.starts_with("[OK]") || l.starts_with("[ERROR]")).unwrap_or("").chars().take(300).collect();
    let verdict = if code == 0 { "ran".to_string() } else { format!("failed (exit {code})") };
    let note = format!("`{shown}`, which you ran, {verdict} after your turn ended: {last} (log {})", self_end::log_path(env).display());
    self_end::log(env, &format!("{id}: {verdict}"));
    // a session start took the relaunch's note already: a failure is still
    // said at the next one; a stop's outcome waits for its next start; a
    // removed member's notes go with it
    if removes && code == 0 {
        if let Some(target) = args.get(2) {
            self_end::forget_under(env, &Id(target.clone()));
        }
    } else if code != 0 || verb != "restart" || self_end::has_note(env, id) {
        self_end::write_note(env, id, &note);
    }
    code
}

/// Wait until `job` (the caller's, when the command ran) is idle or no
/// longer live, up to `secs`. `Err` with why it gives up: the turn did not
/// end in time, or the member runs another job now (relaunched meanwhile:
/// never a second relaunch). No job: nothing to wait for.
fn wait_turn(env: &Env, id: &Id, job: &str, secs: f64) -> Result<(), String> {
    if job.is_empty() {
        return Ok(());
    }
    let until = std::time::Instant::now() + std::time::Duration::from_secs_f64(secs.max(0.0));
    loop {
        let now = nimitz_core::registry::load(env).identities.get(&id.0).and_then(|r| r.job.clone());
        if let Some(now) = now.filter(|j| j != job) {
            return Err(format!("{id} was already relaunched (its job is {now} now, not {job})"));
        }
        match nimitz_core::sessions::find_row(env, job) {
            Some(r) if r.live() && r.status.as_deref() != Some("idle") => {}
            _ => return Ok(()),
        }
        if std::time::Instant::now() >= until {
            return Err(format!("{id}'s turn did not end within {} min; run it again from a shell", (secs / 60.0).ceil()));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}
