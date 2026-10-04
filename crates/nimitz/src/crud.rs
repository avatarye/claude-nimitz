//! Config commands: fleet, squadron, ship, machine (new/set/rm/add) and the
//! fleet-skill migration. They change only the config (plan-v3 5.1); running
//! sessions change through `up` and the single-session commands. `ship rm` is
//! the one exception the plan names: stop, remove job rows, then the config.
use crate::members::{member, parse_id};
use crate::run::refusal;
use crate::{out, Ctx};
use nimitz_core::config;
use nimitz_core::{Caller, Comms, FleetOnDisk, Id, Plan, Role, ShipDef, Step};
use std::path::{Path, PathBuf};

fn write_step(id: &Id, path: &Path, contents: String, why: &str) -> Step {
    Step { id: Some(id.clone()), cmd: vec!["write".into(), path.to_string_lossy().into_owned(), contents], cwd: None, why: why.into() }
}

fn toml_of<T: serde::Serialize>(v: &T) -> Result<String, String> {
    toml::to_string_pretty(v).map_err(|e| e.to_string())
}

/// Config changes are yours: a session's own labels never edit the fleet.
fn user_only(ctx: &Ctx, plan: &mut Plan) {
    if ctx.caller != Caller::User {
        plan.refusals.insert(0, refusal("config.invalid", "config changes are yours alone; ask the user".into(), None, None));
    }
}

/// Gate, then apply a config plan.
fn config_plan(ctx: &Ctx, mut plan: Plan) -> i32 {
    user_only(ctx, &mut plan);
    match ctx.gate(&plan) {
        out::Go::Exit(c) => c,
        out::Go::Run => out::outcome(&config::apply_config_plan(&ctx.env, &plan)),
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
            config::plan_fleet_new(&ctx.env, fleet, &label)
        }
        "set" => {
            ctx.args.allow(&["--admiral-mode"])?;
            match load(ctx, fleet) {
                Err(p) => p,
                Ok(f) => {
                    let mut c = f.config.clone();
                    if let Some(m) = ctx.args.value("--admiral-mode") {
                        c.admiral.permission_mode = if m.is_empty() { None } else { Some(m.into()) };
                    }
                    let mut p = Plan { summary: format!("update fleet {fleet}"), ..Default::default() };
                    if c != f.config {
                        p.steps.push(write_step(&id, &f.dir.join("fleet.toml"), toml_of(&c)?, "fleet settings"));
                    }
                    p
                }
            }
        }
        "rm" => {
            ctx.args.allow(&[])?;
            let mut p = config::plan_rm(&ctx.env, &id);
            if let Some(m) = member(&ctx.snap(), &id).filter(|m| m.session.as_ref().is_some_and(|s| s.pid.is_some())) {
                p.refusals.push(refusal("config.invalid", format!("{} is live; a fleet is deleted only with nothing live", m.id), Some("type /exit in its pane".into()), Some(id.clone())));
            }
            p
        }
        v => return Err(format!("fleet {v}: use new, set or rm")),
    };
    Ok(config_plan(ctx, plan))
}

// ------------------------------------------------------------------ squadron

fn squadron_md(fleet: &str, sq: &str, brief: &str) -> String {
    let body = if brief.is_empty() { "(no brief yet)" } else { brief };
    format!("# Squadron {sq} ({fleet})\n\n{body}\n")
}

pub fn squadron(ctx: &Ctx) -> Result<i32, String> {
    let verb = ctx.args.need(0, "new|set|rm")?;
    let id = parse_id(ctx.args.need(1, "squadron id F/SQ")?)?;
    ctx.args.at_most(2)?;
    let parts = split(&id, Role::Commodore, "squadron")?;
    let (fleet, sq) = (parts[0], parts[1]);
    let plan = match verb {
        "new" => {
            ctx.args.allow(&["--brief", "--comms"])?;
            let comms = ctx.args.value("--comms").map(parse_comms).transpose()?.unwrap_or_default();
            config::plan_squadron_new(&ctx.env, fleet, sq, ctx.args.value("--brief").unwrap_or(""), comms)
        }
        "set" => {
            ctx.args.allow(&["--brief", "--comms", "--mode"])?;
            match load(ctx, fleet) {
                Err(p) => p,
                Ok(f) => {
                    let mut p = Plan { summary: format!("update squadron {id}"), ..Default::default() };
                    match f.squadrons.iter().find(|(_, s)| s.squadron == sq) {
                        None => p.refusals.push(refusal("config.invalid", format!("no squadron {id}"), Some(format!("nimitz squadron new {id}")), Some(id.clone()))),
                        Some((dir, s)) => {
                            let mut c = s.clone();
                            if let Some(v) = ctx.args.value("--comms") {
                                c.comms = parse_comms(v)?;
                            }
                            if let Some(m) = ctx.args.value("--mode") {
                                c.permission_mode = if m.is_empty() { None } else { Some(m.into()) };
                            }
                            if let Some(b) = ctx.args.value("--brief") {
                                let file = c.brief.clone().unwrap_or_else(|| "SQUADRON.md".into());
                                c.brief = Some(file.clone());
                                p.steps.push(write_step(&id, &dir.join(file), squadron_md(fleet, sq, b), "the brief"));
                            }
                            if &c != s {
                                p.steps.insert(0, write_step(&id, &dir.join("squadron.toml"), toml_of(&c)?, "squadron settings"));
                            }
                        }
                    }
                    p
                }
            }
        }
        "rm" => {
            ctx.args.allow(&[])?;
            let mut p = config::plan_rm(&ctx.env, &id);
            let snap = ctx.snap();
            let live: Vec<String> = crate::members::members(&snap)
                .into_iter()
                .filter(|m| m.id == id || m.id.0.starts_with(&format!("{id}/")))
                .filter(|m| m.session.as_ref().is_some_and(|s| s.pid.is_some()))
                .map(|m| m.id.0.clone())
                .collect();
            if !live.is_empty() {
                p.refusals.push(refusal("config.invalid", format!("still live: {}", live.join(", ")), None, Some(id.clone())));
            }
            p
        }
        v => return Err(format!("squadron {v}: use new, set or rm")),
    };
    Ok(config_plan(ctx, plan))
}

// ------------------------------------------------------------------ ship

const SHIP_FLAGS: &[&str] = &["--dir", "--machine", "--prompt", "--prompt-file", "--mode", "--depends"];

/// Apply the ship flags to a definition, for the machine `label`.
fn edit_ship(ctx: &Ctx, def: &mut ShipDef, label: &str) {
    if let Some(d) = ctx.args.value("--dir") {
        if d.is_empty() {
            def.dirs.remove(label);
        } else {
            let d = if d == "managed" || d.starts_with('/') || d.starts_with('~') {
                d.to_string()
            } else {
                std::env::current_dir().map(|c| c.join(d).to_string_lossy().into_owned()).unwrap_or(d.into())
            };
            def.dirs.insert(label.into(), d);
        }
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
}

pub fn ship(ctx: &Ctx) -> Result<i32, String> {
    let verb = ctx.args.need(0, "new|set|rm")?;
    let id = parse_id(ctx.args.need(1, "ship id F/SQ/SHIP")?)?;
    ctx.args.at_most(2)?;
    let parts = split(&id, Role::Ship, "ship")?;
    let (fleet, sq, name) = (parts[0], parts[1], parts[2]);
    if verb == "rm" {
        ctx.args.allow(&[])?;
        return Ok(ship_rm(ctx, &id));
    }
    ctx.args.allow(SHIP_FLAGS)?;
    let f = match load(ctx, fleet) {
        Ok(f) => f,
        Err(p) => return Ok(config_plan(ctx, p)),
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
            edit_ship(ctx, &mut def, &label);
            config::plan_ship_new(&ctx.env, fleet, sq, def)
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
                    edit_ship(ctx, &mut def, &label);
                    let mut p = config::plan_ship_set(&ctx.env, &id, def.clone());
                    if def == before {
                        p.steps.clear();
                    } else if p.ok() {
                        p.warnings.push(nimitz_core::Issue {
                            code: "config.pending".into(),
                            severity: nimitz_core::Severity::Info,
                            message: "the change takes effect at the ship's next relaunch".into(),
                            fix: Some(format!("nimitz relaunch {id}")),
                            id: Some(id.clone()),
                        });
                    }
                    p
                }
            }
        }
        v => return Err(format!("ship {v}: use new, set or rm")),
    };
    Ok(config_plan(ctx, plan))
}

/// Stop the ship, remove its job rows (transcripts stay), then drop it from the config.
fn ship_rm(ctx: &Ctx, id: &Id) -> i32 {
    let snap = ctx.snap();
    let cfg = config::plan_rm(&ctx.env, id);
    let mut sess = Plan { summary: format!("retire {id}"), ..Default::default() };
    user_only(ctx, &mut sess);
    if let Some(m) = member(&snap, id) {
        for r in m.session.iter().chain(m.extra_rows.iter()) {
            let Some(short) = &r.short_id else { continue };
            if r.pid.is_some() {
                sess.steps.push(Step { id: Some(id.clone()), cmd: nimitz_core::claude::stop(&ctx.env, short), cwd: None, why: "stop it first".into() });
            }
            sess.steps.push(Step { id: Some(id.clone()), cmd: nimitz_core::claude::rm(&ctx.env, short), cwd: None, why: "remove the job row; the transcript stays".into() });
        }
    }
    let mut shown = sess.clone();
    shown.summary = format!("retire {id}: stop, remove job rows, remove from the config");
    shown.steps.extend(cfg.steps.iter().cloned());
    shown.refusals.extend(cfg.refusals.iter().cloned());
    match ctx.gate(&shown) {
        out::Go::Exit(c) => c,
        out::Go::Run => {
            if !sess.steps.is_empty() {
                let o = nimitz_core::sessions::apply(&ctx.env, &sess);
                if out::outcome(&o) != 0 {
                    out::error("the config is unchanged because the sessions part failed");
                    return 1;
                }
            }
            out::outcome(&config::apply_config_plan(&ctx.env, &cfg))
        }
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
            ctx.args.allow(&["--dir"])?;
            let mut dirs = vec![];
            for d in ctx.args.values("--dir") {
                let (sid, path) = d.split_once('=').ok_or(format!("--dir {d:?}: use F/SQ/SHIP=PATH"))?;
                dirs.push((parse_id(sid)?, path.to_string()));
            }
            config::plan_machine_add(&ctx.env, fleet, label, dirs)
        }
        "rm" => {
            ctx.args.allow(&[])?;
            match load(ctx, fleet) {
                Err(p) => p,
                Ok(f) => plan_machine_rm(&f, fleet, label)?,
            }
        }
        v => return Err(format!("machine {v}: use add or rm")),
    };
    Ok(config_plan(ctx, plan))
}

/// Remove a label from fleet.toml and from every ship's folders and modes.
fn plan_machine_rm(f: &FleetOnDisk, fleet: &str, label: &str) -> Result<Plan, String> {
    let fid = Id::admiral(fleet);
    let mut p = Plan { summary: format!("remove machine {label} from {fleet}"), ..Default::default() };
    if !f.config.machines.contains_key(label) {
        p.refusals.push(refusal("config.invalid", format!("machine {label} is not in {fleet}"), None, Some(fid)));
        return Ok(p);
    }
    let mut c = f.config.clone();
    c.machines.remove(label);
    p.steps.push(write_step(&fid, &f.dir.join("fleet.toml"), toml_of(&c)?, "fleet settings without the machine"));
    for (dir, s) in &f.squadrons {
        let mut ns = s.clone();
        for ship in &mut ns.ships {
            ship.dirs.remove(label);
            ship.permission_mode.remove(label);
        }
        if &ns != s {
            p.steps.push(write_step(&Id::commodore(fleet, &s.squadron), &dir.join("squadron.toml"), toml_of(&ns)?, "ship folders without the machine"));
        }
    }
    Ok(p)
}

// ------------------------------------------------------------------ migrate

/// `nimitz migrate fleet-skill <dir...> --into <F>`: plan by default; `--yes` applies.
/// Steps that would change nothing are dropped, so a second run is a no-op.
pub fn migrate(ctx: &Ctx) -> Result<i32, String> {
    ctx.args.allow(&["--into"])?;
    if ctx.args.need(0, "source (fleet-skill)")? != "fleet-skill" {
        return Err("only `migrate fleet-skill` exists".into());
    }
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
    Ok(config_plan(ctx, plan))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comms_forms() {
        assert_eq!(parse_comms("open").unwrap(), Comms::Mode("open".into()));
        assert_eq!(parse_comms("allow:MAIN/A/B,MAIN/C/D").unwrap(), Comms::Allow { allow: vec!["MAIN/A/B".into(), "MAIN/C/D".into()] });
        assert!(parse_comms("closed").is_err());
    }
}
