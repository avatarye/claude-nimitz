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
            ctx.args.allow(&["--brief", "--comms"])?;
            let comms = ctx.args.value("--comms").map(parse_comms).transpose()?.unwrap_or_default();
            config::plan_squadron_new(&ctx.env, fleet, sq, ctx.args.value("--brief").unwrap_or(""), comms)
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

/// `$VISUAL`, else `$EDITOR`, else vi.
fn editor(ctx: &Ctx) -> String {
    ["VISUAL", "EDITOR"].iter().find_map(|k| ctx.vars.get(*k).filter(|v| !v.trim().is_empty()).cloned()).unwrap_or_else(|| "vi".into())
}

/// Open `editor` on a temp file that holds `initial`; return the saved text, trimmed.
/// The editor command may carry its own arguments (`code -w`).
pub fn edit_text(editor: &str, initial: &str) -> Result<String, String> {
    let path = std::env::temp_dir().join(format!("nimitz-prompt-{}.md", std::process::id()));
    std::fs::write(&path, initial).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("sh").arg("-c").arg(format!("{editor} \"$1\"")).arg("sh").arg(&path).status();
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
    if d == "managed" || d.starts_with('/') || d.starts_with('~') {
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
    Ok(config_plan(ctx, plan, Some(&id)))
}

/// Stop the ship, remove its job rows (transcripts stay), then drop it from the config.
/// `ship rm`, `squadron rm`, `fleet rm`: core's retire plan (stop, rm the job
/// rows, then the config), applied and verified in one go.
fn retire(ctx: &Ctx, id: &Id) -> i32 {
    let plan = nimitz_core::sessions::plan_retire(&ctx.env, &ctx.snap(), &ctx.caller, id);
    match ctx.gate(&plan) {
        out::Go::Exit(c) => c,
        out::Go::Run => out::outcome(&nimitz_core::sessions::apply(&ctx.env, &plan)),
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
    Ok(config_plan(ctx, plan, None))
}

#[cfg(test)]
mod tests {
    use super::*;

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
