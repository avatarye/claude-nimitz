//! Config commands: fleet, squadron, ship, machine (new/set/rm/add) and the
//! fleet-skill migration. They change only the config (plan-v3 5.1); running
//! sessions change through `up` and the single-session commands. `ship rm` is
//! the one exception the plan names: stop, remove job rows, then the config.
use crate::members::parse_id;
use crate::run::refusal;
use crate::{out, Ctx};
use nimitz_core::config;
use nimitz_core::{Caller, Comms, FleetOnDisk, Id, Plan, Role, ShipDef};
use std::path::{Path, PathBuf};

/// Who may make this config change. `None`: you alone (a fleet created or
/// removed, machines, migration). `Some(scope)`: you, the scope's admiral, or
/// (for a squadron's settings and its ships) its commodore (`may_config`).
fn authority(ctx: &Ctx, plan: &mut Plan, scope: Option<&Id>) {
    let why = match (scope, &ctx.caller) {
        (_, Caller::User) => return,
        (None, _) => "this change is yours alone (fleets created or removed, machines, migration); ask the user".to_string(),
        (Some(scope), caller) => match nimitz_core::sessions::may_config(caller, scope) {
            Ok(()) => return,
            Err(why) => why,
        },
    };
    // the refusal alone: no other refusals or fixes for a change the caller may not make
    *plan = Plan { summary: plan.summary.clone(), refusals: vec![refusal("authority", why, None, scope.cloned())], ..Default::default() };
}

/// Gate, then apply a config plan.
fn config_plan(ctx: &Ctx, mut plan: Plan, scope: Option<&Id>) -> i32 {
    authority(ctx, &mut plan, scope);
    match ctx.gate(&plan) {
        out::Go::Exit(c) => c,
        out::Go::Run => out::outcome(&plan, &config::apply_config_plan(&ctx.env, &plan)),
    }
}

fn parse_comms(s: &str) -> Result<Comms, String> {
    match s {
        "open" | "commodore-only" => Ok(Comms::Mode(s.into())),
        _ => match s.strip_prefix("allow:") {
            Some(list) => Ok(Comms::Allow { allow: list.split(',').filter(|x| !x.is_empty()).map(str::to_string).collect() }),
            None => Err(format!("--comms {s:?}: use open, commodore-only or allow:F/SQ/SHIP,...")),
        },
    }
}

fn load(ctx: &Ctx, fleet: &str) -> Result<FleetOnDisk, Plan> {
    config::load_fleet(&ctx.env, fleet).map_err(|i| Plan { summary: format!("load fleet {fleet}"), refusals: vec![i], ..Default::default() })
}

fn split<'a>(id: &'a Id, role: Role, what: &str) -> Result<Vec<&'a str>, String> {
    if id.role() != role {
        return Err(format!("{id} is not a {what} id"));
    }
    Ok(id.parts())
}

/// This machine's label: the registry's, else one made from the hostname.
pub fn machine_label(ctx: &Ctx) -> String {
    let r = nimitz_core::registry::load(&ctx.env);
    if !r.machine.is_empty() {
        return r.machine;
    }
    let l: String =
        ctx.env.hostname.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
    l.split('.').next().unwrap_or("this-machine").to_string()
}

// ------------------------------------------------------------------ fleet

pub fn fleet(ctx: &Ctx) -> Result<i32, String> {
    let verb = ctx.args.need(0, "new|set|rm")?;
    let fleet = ctx.args.need(1, "fleet")?;
    ctx.args.at_most(2)?;
    let id = parse_id(fleet)?;
    split(&id, Role::Admiral, "fleet")?;
    let plan = match verb {
        "new" => {
            ctx.args.allow(&["--machine"])?;
            let label = ctx.args.value("--machine").map(str::to_string).unwrap_or_else(|| machine_label(ctx));
            return Ok(config_plan(ctx, config::plan_fleet_new(&ctx.env, fleet, &label), None));
        }
        "set" => {
            ctx.args.allow(&["--admiral-mode"])?;
            config::plan_fleet_set(&ctx.env, fleet, ctx.args.value("--admiral-mode"))
        }
        "rm" => {
            ctx.args.allow(&[])?;
            return Ok(retire(ctx, &id));
        }
        v => return Err(format!("fleet {v}: use new, set or rm")),
    };
    Ok(config_plan(ctx, plan, Some(&id)))
}

// ------------------------------------------------------------------ squadron

pub fn squadron(ctx: &Ctx) -> Result<i32, String> {
    let verb = ctx.args.need(0, "new|set|rm")?;
    let id = parse_id(ctx.args.need(1, "squadron id F/SQ")?)?;
    ctx.args.at_most(2)?;
    let parts = split(&id, Role::Commodore, "squadron")?;
    let (fleet, sq) = (parts[0], parts[1]);
    let plan = match verb {
        "new" => {
            ctx.args.allow(&["--brief", "--comms", "--mode"])?;
            let comms = ctx.args.value("--comms").map(parse_comms).transpose()?.unwrap_or_default();
            config::plan_squadron_new(&ctx.env, fleet, sq, ctx.args.value("--brief").unwrap_or(""), comms, ctx.args.value("--mode"))
        }
        "set" => {
            ctx.args.allow(&["--brief", "--comms", "--mode"])?;
            let comms = ctx.args.value("--comms").map(parse_comms).transpose()?;
            config::plan_squadron_set(&ctx.env, &id, ctx.args.value("--brief"), comms, ctx.args.value("--mode"))
        }
        "rm" => {
            ctx.args.allow(&[])?;
            return Ok(retire(ctx, &id));
        }
        v => return Err(format!("squadron {v}: use new, set or rm")),
    };
    // a new squadron is its admiral's (or yours); a squadron's settings its commodore's too
    let scope = if verb == "new" { Id::admiral(fleet) } else { id.clone() };
    Ok(config_plan(ctx, plan, Some(&scope)))
}

// ------------------------------------------------------------------ ship

const SHIP_FLAGS: &[&str] = &["--dir", "--no-dir", "--machine", "--prompt", "--prompt-file", "--mode", "--depends", "--edit"];

/// `$VISUAL`, else `$EDITOR`, else vi (notepad on Windows).
fn editor(ctx: &Ctx) -> String {
    ["VISUAL", "EDITOR"].iter().find_map(|k| ctx.vars.get(*k).filter(|v| !v.trim().is_empty()).cloned()).unwrap_or_else(|| nimitz_core::os::DEFAULT_EDITOR.into())
}

/// Open `editor` on a temp file that holds `initial`; return the saved text, trimmed.
/// The editor command may carry its own arguments (`code -w`).
pub fn edit_text(editor: &str, initial: &str) -> Result<String, String> {
    let path = std::env::temp_dir().join(format!("nimitz-prompt-{}.md", std::process::id()));
    std::fs::write(&path, initial).map_err(|e| e.to_string())?;
    let status = nimitz_core::os::editor_command(editor, &path).status();
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&path);
    match status {
        Ok(s) if s.success() => Ok(text?.trim().to_string()),
        Ok(s) => Err(format!("{editor} exited with {s}")),
        Err(e) => Err(format!("{editor}: {e}")),
    }
}

/// The prompt from `$EDITOR`: on `ship new` at a terminal with no prompt flag,
/// or on `ship set --edit`. None when the editor is not wanted.
fn prompt_from_editor(ctx: &Ctx, current: Option<&str>, new: bool) -> Result<Option<String>, String> {
    use std::io::IsTerminal;
    let flags = ctx.args.value("--prompt").is_some() || ctx.args.value("--prompt-file").is_some();
    let wanted = ctx.args.flag("--edit") || (new && !flags && std::io::stdin().is_terminal());
    if !wanted {
        return Ok(None);
    }
    if flags {
        return Err("--edit and --prompt/--prompt-file do not mix".into());
    }
    let initial = current.map(str::to_string).unwrap_or_else(|| "You are the ship ... Your job is ...\n".into());
    let text = edit_text(&editor(ctx), &initial)?;
    if text.is_empty() {
        out::warn("the prompt is empty; the ship has no prompt (a cold launch refuses)");
        return Ok(None);
    }
    Ok(Some(text))
}

/// `managed`, `~/...` and absolute folders stay as given; a relative one is made
/// absolute against the current folder (and cleaned of `.` and `..`).
fn absolute_dir(d: &str) -> String {
    if d == "managed" || nimitz_core::config::is_absolute_dir(d) || d.starts_with('~') {
        return d.to_string();
    }
    let joined = std::env::current_dir().unwrap_or_default().join(d);
    let mut clean = PathBuf::new();
    for c in joined.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other),
        }
    }
    clean.to_string_lossy().into_owned()
}

/// Apply the ship flags to a definition, for the machine `label`.
fn edit_ship(ctx: &Ctx, def: &mut ShipDef, label: &str, f: &FleetOnDisk) -> Result<(), String> {
    for l in ctx.args.values("--no-dir") {
        if def.dirs.remove(&l).is_none() {
            return Err(format!("{} has no folder for machine {l:?}", def.name));
        }
        def.permission_mode.remove(&l); // no folder there, so no mode either
    }
    if let Some(d) = ctx.args.value("--dir") {
        // core's normalize_ship_dir is the rule (shared with Admiralty); the CLI only
        // resolves a relative folder against the cwd first. `<label>=<x>` and an
        // empty value go to core as typed, so its message names the right form.
        let raw = d.trim();
        let label_form = raw.split_once('=').is_some_and(|(l, _)| f.config.machines.contains_key(l));
        let input = if raw.is_empty() || label_form { raw.to_string() } else { absolute_dir(raw) };
        let stored = config::normalize_ship_dir(f, &input).map_err(|e| {
            if raw.is_empty() { format!("--dir: {e}; to remove a machine's folder use --no-dir <label>") } else { format!("--dir: {e}") }
        })?;
        out::line(&format!("folder for {label}: {stored}"));
        def.dirs.insert(label.into(), stored);
    }
    if let Some(p) = ctx.args.value("--prompt") {
        def.prompt = Some(p.into());
        def.prompt_file = None;
    }
    if let Some(p) = ctx.args.value("--prompt-file") {
        def.prompt_file = Some(p.into());
        def.prompt = None;
    }
    if let Some(m) = ctx.args.value("--mode") {
        if m.is_empty() {
            def.permission_mode.remove(label);
        } else {
            def.permission_mode.insert(label.into(), m.into());
        }
    }
    if let Some(d) = ctx.args.value("--depends") {
        def.depends_on = d.split(',').filter(|x| !x.is_empty()).map(str::to_string).collect();
    }
    Ok(())
}

pub fn ship(ctx: &Ctx) -> Result<i32, String> {
    let verb = ctx.args.need(0, "new|set|rm")?;
    let id = parse_id(ctx.args.need(1, "ship id F/SQ/SHIP")?)?;
    ctx.args.at_most(2)?;
    let parts = split(&id, Role::Ship, "ship")?;
    let (fleet, sq, name) = (parts[0], parts[1], parts[2]);
    if verb == "rm" {
        ctx.args.allow(&[])?;
        return Ok(retire(ctx, &id));
    }
    ctx.args.allow(SHIP_FLAGS)?;
    // refuse before anything opens an editor
    if nimitz_core::sessions::may_config(&ctx.caller, &id).is_err() {
        return Ok(config_plan(ctx, Plan { summary: format!("ship {verb} {id}"), ..Default::default() }, Some(&id)));
    }
    let f = match load(ctx, fleet) {
        Ok(f) => f,
        Err(p) => return Ok(config_plan(ctx, p, Some(&id))),
    };
    let label = match ctx.args.value("--machine").map(str::to_string).or(f.machine.clone()) {
        Some(l) => l,
        None => return Err(format!("this machine has no label in fleet {fleet}; pass --machine <label> or `nimitz machine add {fleet} <label>`")),
    };
    let plan = match verb {
        "new" => {
            if ctx.args.value("--dir").is_none() {
                return Err("ship new needs --dir <folder|managed>".into());
            }
            let mut def = ShipDef { name: name.into(), ..Default::default() };
            edit_ship(ctx, &mut def, &label, &f)?;
            if let Some(p) = prompt_from_editor(ctx, None, true)? {
                def.prompt = Some(p);
            }
            let dir = def.dirs.get(&label).cloned();
            let mut p = config::plan_ship_new(&ctx.env, fleet, sq, def);
            trust_step(ctx, &mut p, &id, Some(&label), dir.as_deref());
            p
        }
        "set" => {
            let cur = f.squadrons.iter().find(|(_, s)| s.squadron == sq).and_then(|(_, s)| s.ships.iter().find(|x| x.name == name)).cloned();
            match cur {
                None => Plan {
                    summary: format!("update ship {id}"),
                    refusals: vec![refusal("config.invalid", format!("no ship {id}"), Some(format!("nimitz ship new {id} --dir <folder>")), Some(id.clone()))],
                    ..Default::default()
                },
                Some(mut def) => {
                    let before = def.clone();
                    let current = config::ship_prompt(&f, &id);
                    edit_ship(ctx, &mut def, &label, &f)?;
                    if let Some(p) = prompt_from_editor(ctx, current.as_deref(), false)? {
                        def.prompt = Some(p);
                        def.prompt_file = None;
                    }
                    let mut p = config::plan_ship_set(&ctx.env, &id, def.clone());
                    if def == before {
                        p.steps.clear();
                    } else if p.ok() {
                        if def.dirs.get(&label) != before.dirs.get(&label) {
                            trust_step(ctx, &mut p, &id, Some(&label), def.dirs.get(&label).map(String::as_str));
                        }
                        p.warnings.push(nimitz_core::Issue {
                            code: "config.pending".into(),
                            severity: nimitz_core::Severity::Info,
                            message: "the change takes effect at the ship's next relaunch".into(),
                            fix: Some(nimitz_core::sessions::start_cmd(&id, "-f")),
                            id: Some(id.clone()),
                        });
                    }
                    p
                }
            }
        }
        v => return Err(format!("ship {v}: use new, set or rm")),
    };
    Ok(config_plan(ctx, plan, Some(&id)))
}

/// A ship folder Claude Code will not trust: the user's plan trusts it
/// (after the config is written); anyone else's says so, the fix the user's.
/// A managed folder is under the data root (`nimitz init` trusts that).
fn trust_step(ctx: &Ctx, plan: &mut Plan, id: &Id, label: Option<&str>, dir: Option<&str>) {
    let Some(dir) = dir.filter(|d| *d != "managed") else { return };
    if !plan.ok() {
        return;
    }
    // another machine's folder is that machine's trust, not this one's
    let here = config::load_fleet(&ctx.env, id.fleet()).ok().and_then(|f| f.machine);
    if label != here.as_deref() {
        return;
    }
    match nimitz_core::trust::for_plan(&ctx.env, &ctx.caller, Some(id), &config::expand_dir(dir)) {
        Some(Ok(step)) => plan.steps.push(step),
        Some(Err(issue)) => plan.warnings.push(issue),
        None => {}
    }
}

/// Stop the ship, remove its job rows (transcripts stay), then drop it from the config.
/// `ship rm`, `squadron rm`, `fleet rm`: core's retire plan (stop, rm the job
/// rows, then the config), applied and verified in one go.
const AIDE_FLAGS: &[&str] = &["--dir", "--machine", "--model", "--mode", "--brief", "--types", "--policy", "--depth", "--per-sender", "--lease", "--max-tries", "--keep-done"];

/// `nimitz aide new|set|rm F/@NAME` (aide.md section 3): like `ship
/// new|set|rm`. You, or the fleet's admiral. `new` needs `--dir` (its repo,
/// on this machine or `--machine`) and `--types`; the model defaults to
/// sonnet, the mode to acceptEdits.
pub fn aide(ctx: &Ctx) -> Result<i32, String> {
    let verb = ctx.args.need(0, "new|set|rm")?;
    let id = parse_id(ctx.args.need(1, "aide id F/@NAME")?)?;
    ctx.args.at_most(2)?;
    split(&id, Role::Aide, "aide")?;
    if verb == "rm" {
        ctx.args.allow(&[])?;
        return Ok(retire(ctx, &id));
    }
    ctx.args.allow(AIDE_FLAGS)?;
    if nimitz_core::sessions::may_config(&ctx.caller, &id).is_err() {
        return Ok(config_plan(ctx, Plan { summary: format!("aide {verb} {id}"), ..Default::default() }, Some(&id)));
    }
    let f = match load(ctx, id.fleet()) {
        Ok(f) => f,
        Err(p) => return Ok(config_plan(ctx, p, Some(&id))),
    };
    let name = id.parts().last().map(|p| p.trim_start_matches('@').to_string()).unwrap_or_default();
    let (new, mut def) = match verb {
        "new" => {
            if ctx.args.value("--dir").is_none() || ctx.args.value("--types").is_none() {
                return Err("aide new needs --dir <its repo> and --types <type,...>".into());
            }
            (true, nimitz_core::AideDef { name, model: Some("sonnet".into()), permission_mode: Some("acceptEdits".into()), ..Default::default() })
        }
        "set" => match config::find_aide(&f, &id) {
            Some(d) => (false, d.clone()),
            None => (false, nimitz_core::AideDef { name, ..Default::default() }), // core refuses: no aide
        },
        v => return Err(format!("aide {v}: use new, set or rm")),
    };
    let before = def.clone();
    if let Some(d) = ctx.args.value("--dir") {
        let label = match ctx.args.value("--machine").map(str::to_string).or(f.machine.clone()) {
            Some(l) => l,
            None => return Err(format!("this machine has no label in fleet {}; pass --machine <label>", id.fleet())),
        };
        let d = d.trim();
        let dir = if config::is_absolute_dir(d) { d.to_string() } else { absolute_dir(d) };
        def.dirs = [(label, dir)].into(); // one machine runs an aide
    } else if ctx.args.value("--machine").is_some() {
        return Err("--machine goes with --dir (the aide's folder on that machine)".into());
    }
    let opt = |v: &str| if v.is_empty() { None } else { Some(v.to_string()) };
    if let Some(m) = ctx.args.value("--model") {
        def.model = opt(m);
    }
    if let Some(m) = ctx.args.value("--mode") {
        def.permission_mode = opt(m);
    }
    if let Some(b) = ctx.args.value("--brief") {
        def.brief = opt(b);
    }
    if let Some(t) = ctx.args.value("--types") {
        def.inbox.types = t.split(',').map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect();
    }
    if let Some(p) = ctx.args.value("--policy") {
        def.inbox.policy = p.into();
    }
    if let Some(n) = ctx.args.value("--depth") {
        def.inbox.depth_cap = n.parse().map_err(|_| format!("--depth {n:?}: a number of items"))?;
    }
    if let Some(r) = ctx.args.value("--per-sender") {
        def.inbox.per_sender = r.into();
    }
    if let Some(l) = ctx.args.value("--lease") {
        def.inbox.lease = l.into();
    }
    if let Some(n) = ctx.args.value("--max-tries") {
        def.inbox.max_tries = n.parse().map_err(|_| format!("--max-tries {n:?}: a number"))?;
    }
    if let Some(k) = ctx.args.value("--keep-done") {
        def.inbox.keep_done = k.into();
    }
    let dir = def.dirs.iter().next().map(|(l, d)| (l.clone(), d.clone()));
    // `--force` past the machine guard is the user's alone (rules: never unless the user names it)
    let force = ctx.force() && ctx.caller == Caller::User;
    let mut plan = config::plan_aide_write(&ctx.env, &id, def.clone(), new, force);
    // moved to another machine: what runs here (its job, its inbox) stays here
    let (was, now) = (before.dirs.keys().next().cloned(), def.dirs.keys().next().cloned());
    if !new && plan.ok() && was.is_some() && was != now {
        let was = was.unwrap_or_default();
        let here = f.machine.as_deref().is_some_and(|m| before.dirs.contains_key(m));
        if !here {
            if !force {
                plan.refusals.push(refusal("aide.elsewhere", format!("{id} runs on {was}: its job and its inbox are there, and this machine sees neither"), Some(format!("move it from {was} (nimitz aide set {id} --machine <label> --dir <folder> there)")), Some(id.clone())));
            }
        } else {
            let snap = ctx.snap();
            let live = snap.fleets.iter().flat_map(|f| f.members()).find(|m| m.id == id).is_some_and(|m| m.session.iter().chain(m.extra_rows.iter()).any(|s| s.pid.is_some()));
            // its queue as `aide rm` reads it (an inbox config that does not parse: the default limits)
            let limits = nimitz_core::queue::Limits::from_def(&before.inbox).unwrap_or_else(|_| nimitz_core::queue::Limits::from_def(&Default::default()).expect("the default inbox"));
            let st = nimitz_core::queue::Queue::open(&ctx.env, &id, limits).peek_status(nimitz_core::evidence::now(), false);
            let work = st.pending + st.claimed.len();
            if live {
                plan.refusals.push(refusal("session.live", format!("{id} runs here; moved, it would keep running here beside its new machine's"), Some(format!("nimitz stop {id}, then move it")), Some(id.clone())));
            }
            if work > 0 {
                plan.refusals.push(refusal("config.nonempty", format!("{id}'s inbox here still has {work} item(s) to work; the new machine's inbox starts empty"), Some(format!("let it drain (or drop them), then move it")), Some(id.clone())));
            }
        }
    }
    if !new && def == before {
        plan.steps.clear();
    } else if plan.ok() {
        if let Some((label, d)) = dir.filter(|_| new || def.dirs != before.dirs) {
            trust_step(ctx, &mut plan, &id, Some(&label), Some(&d));
        }
        let (code, then) = if new { ("aide.new", format!("then: nimitz start {id}  # launches it, pinned, on {}", def.model.as_deref().unwrap_or("sonnet"))) } else { ("config.pending", "the change takes effect at the aide's next relaunch".to_string()) };
        let fix = (!new).then(|| nimitz_core::sessions::start_cmd(&id, "-f"));
        plan.warnings.push(nimitz_core::Issue { code: code.into(), severity: nimitz_core::Severity::Info, message: then, fix, id: Some(id.clone()) });
    }
    Ok(config_plan(ctx, plan, Some(&id)))
}

fn retire(ctx: &Ctx, id: &Id) -> i32 {
    let plan = nimitz_core::sessions::plan_retire(&ctx.env, &ctx.snap(), &ctx.caller, id);
    match ctx.gate(&plan) {
        out::Go::Exit(c) => c,
        out::Go::Run => out::outcome(&plan, &nimitz_core::sessions::apply(&ctx.env, &plan)),
    }
}

// ------------------------------------------------------------------ machine

pub fn machine(ctx: &Ctx) -> Result<i32, String> {
    let verb = ctx.args.need(0, "add|rm")?;
    let fleet = ctx.args.need(1, "fleet")?;
    let label = ctx.args.need(2, "machine label")?;
    ctx.args.at_most(3)?;
    let plan = match verb {
        "add" => {
            ctx.args.allow(&["--dir", "--hostname"])?;
            let mut dirs = vec![];
            for d in ctx.args.values("--dir") {
                let (sid, path) = d.split_once('=').ok_or(format!("--dir {d:?}: use F/SQ/SHIP=PATH"))?;
                dirs.push((parse_id(sid)?, path.to_string()));
            }
            config::plan_machine_add_on(&ctx.env, fleet, label, ctx.args.value("--hostname"), dirs)
        }
        "rm" => {
            ctx.args.allow(&[])?;
            config::plan_machine_rm_live(&ctx.env, &ctx.snap(), fleet, label)
        }
        v => return Err(format!("machine {v}: use add or rm")),
    };
    Ok(config_plan(ctx, plan, None))
}

// ------------------------------------------------------------------ migrate

/// `nimitz migrate fleet-skill <dir...> --into <F>`: plan by default; `--yes` applies.
/// Steps that would change nothing are dropped, so a second run is a no-op.
/// `nimitz migrate --plan|--yes [--shared]`: every format step due here
/// (docs/design/migrations.md section 4); `--yes` runs the local ones, and
/// with `--shared` the shared ones (they affect every machine that reads the
/// data root). `nimitz migrate fleet-skill <dir...> --into <F>`: the import
/// of old fleet-skill fleets. Both are yours alone, like `init`.
pub fn migrate(ctx: &Ctx) -> Result<i32, String> {
    if ctx.caller != Caller::User {
        let plan = Plan {
            summary: "migrate".into(),
            refusals: vec![refusal("authority", "`nimitz migrate` is yours alone (it rewrites this machine's files, and with --shared the data root of every machine); ask the user".into(), None, None)],
            ..Default::default()
        };
        return Ok(match ctx.gate(&plan) {
            out::Go::Exit(c) => c.max(1),
            out::Go::Run => 1,
        });
    }
    match ctx.args.pos.first().map(String::as_str) {
        Some("fleet-skill") => fleet_skill(ctx),
        Some(other) => Err(format!("migrate {other}: use `migrate --plan|--yes [--shared]` or `migrate fleet-skill <dir...> --into <F>`")),
        None => formats(ctx),
    }
}

/// The format steps due here: listed (`--plan`, `--json`), or run (`--yes`).
fn formats(ctx: &Ctx) -> Result<i32, String> {
    use nimitz_core::formats::{self, Scope};
    ctx.args.allow(&["--shared"])?;
    let shared = ctx.args.flag("--shared");
    let due = formats::due(&ctx.env, formats::ARTIFACTS, None);
    let shown: Vec<&formats::Due> = due.iter().filter(|d| !d.silent).collect();
    let too_new = formats::too_new(&ctx.env);
    if out::json_mode() {
        out::json(&serde_json::json!({"nimitz": formats::NIMITZ, "due": due, "too_new": too_new.iter().map(|t| t.to_string()).collect::<Vec<_>>()}));
    } else {
        print_due(&shown);
        for t in &too_new {
            out::error(&t.to_string());
        }
    }
    let bookkeeping = due.iter().any(|d| d.silent);
    if ctx.plan_only() {
        return Ok(if too_new.is_empty() { 0 } else { 1 });
    }
    let runs = |d: &&formats::Due| d.scope == Scope::Local || shared;
    if !shown.iter().any(runs) && !bookkeeping {
        if shown.iter().any(|d| d.scope == Scope::Shared) && !out::json_mode() {
            out::line("the shared steps run with `nimitz migrate --shared --yes` (each machine needs the nimitz they name first)");
        }
        return Ok(if too_new.is_empty() { 0 } else { 1 });
    }
    if !ctx.yes() {
        out::error(&format!("pass --yes to run {}, or --plan to only list them", if shared { "the local and the shared steps" } else { "the local steps" }));
        return Ok(1);
    }
    let mut ran = formats::run(&ctx.env, formats::ARTIFACTS, Scope::Local);
    if shared {
        let s = formats::run(&ctx.env, formats::ARTIFACTS, Scope::Shared);
        ran.done.extend(s.done);
        ran.failed.extend(s.failed);
        ran.backups.extend(s.backups);
        ran.too_new.extend(s.too_new);
    }
    for d in ran.done.iter().filter(|d| !d.silent) {
        out::ok(&format!("{} {}->{} {}", d.artifact, d.from, d.to, d.file.display()));
    }
    for b in &ran.backups {
        out::line(&format!("       backup: {}", b.display()));
    }
    for (d, e) in &ran.failed {
        out::error(&format!("{} {}->{} {}: {e} (the file is as it was; log: {})", d.artifact, d.from, d.to, d.file.display(), formats::log_path(&ctx.env).display()));
    }
    let left = formats::due(&ctx.env, formats::ARTIFACTS, None).into_iter().filter(|d| !d.silent).count();
    if ran.failed.is_empty() {
        out::ok(&format!("migrate: done{}", if left > 0 { format!(" ({left} shared step(s) left for `nimitz migrate --shared --yes`)") } else { String::new() }));
        Ok(0)
    } else {
        Ok(1)
    }
}

/// The steps due, one line each; a shared one says it affects every machine.
fn print_due(due: &[&nimitz_core::formats::Due]) {
    use nimitz_core::formats::{self, Scope};
    if due.is_empty() {
        out::ok(&format!("nimitz {}: nothing to migrate (every file here is at the format this nimitz writes)", formats::NIMITZ));
        return;
    }
    out::line(&format!("nimitz {}: {} step(s) due", formats::NIMITZ, due.len()));
    for d in due {
        let every = match (d.scope, d.min_nimitz) {
            (Scope::Shared, Some(m)) => format!("; affects every machine: each one needs nimitz >= {m} first"),
            (Scope::Shared, None) => "; affects every machine".into(),
            _ => String::new(),
        };
        out::line(&format!("  {:<6}  {} {}->{}  {}", d.scope.as_str(), d.artifact, d.from, d.to, d.file.display()));
        out::line(&format!("          {}{every}", d.summary));
    }
    let (local, shared) = (due.iter().any(|d| d.scope == Scope::Local), due.iter().any(|d| d.scope == Scope::Shared));
    let hint = match (local, shared) {
        (true, true) => "--yes runs the local steps; --shared --yes the shared ones too",
        (true, false) => "--yes runs them (so does the next nimitz command, by itself)",
        _ => "--shared --yes runs them, once every machine runs the nimitz they name",
    };
    out::line(&format!("nimitz migrate {hint}"));
}

/// `nimitz migrate fleet-skill <dir...> --into <F>`.
fn fleet_skill(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--into"])?;
    let into = ctx.args.value("--into").ok_or("migrate needs --into <F>")?;
    let dirs: Vec<PathBuf> = ctx.args.pos[1..].iter().map(PathBuf::from).collect();
    if dirs.is_empty() {
        return Err("migrate needs at least one old fleet folder".into());
    }
    let (mut plan, report) = nimitz_core::migrate::plan_from_fleet_skill(&ctx.env, &dirs, into);
    for r in &report {
        out::line(r);
    }
    plan.steps.retain(|s| match s.cmd.first().map(String::as_str) {
        Some("write") => std::fs::read_to_string(&s.cmd[1]).ok().as_deref() != s.cmd.get(2).map(String::as_str),
        Some("mkdir") => !Path::new(&s.cmd[1]).is_dir(),
        _ => true,
    });
    Ok(config_plan(ctx, plan, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)] // the stand-in editor is a shell line
    #[test]
    fn the_editor_returns_what_it_saved() {
        // a stand-in editor: append a line to the file it is given
        let got = edit_text("sh -c 'printf \"\\nmore\" >> \"$1\"' ed", "first").unwrap();
        assert_eq!(got, "first\nmore");
        assert!(edit_text("false", "x").is_err());
    }

    #[test]
    fn relative_dirs_become_absolute() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(absolute_dir("./a/../b"), cwd.join("b").to_string_lossy());
        assert_eq!(absolute_dir("managed"), "managed");
        assert_eq!(absolute_dir("/x/y"), "/x/y");
        assert_eq!(absolute_dir("~/p"), "~/p");
    }

    #[test]
    fn comms_forms() {
        assert_eq!(parse_comms("open").unwrap(), Comms::Mode("open".into()));
        assert_eq!(parse_comms("allow:MAIN/A/B,MAIN/C/D").unwrap(), Comms::Allow { allow: vec!["MAIN/A/B".into(), "MAIN/C/D".into()] });
        assert!(parse_comms("closed").is_err());
    }
}
