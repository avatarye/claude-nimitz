//! Config commands: fleet, squadron, ship, machine (new/set/rm/add) and the
//! fleet-skill migration. They change only the config (plan-v3 5.1); running
//! sessions change through `up` and the single-session commands. `ship rm` is
//! the one exception the plan names: stop, remove job rows, then the config.
use crate::members::parse_id;
use crate::run::refusal;
use crate::{out, Ctx};
use nimitz_core::config;
use nimitz_core::{FleetOnDisk, Id, Plan, Role, ShipDef};
use std::path::{Path, PathBuf};

/// Gate, then apply a config plan.
pub(crate) fn config_plan(ctx: &Ctx, plan: Plan) -> i32 {
    match ctx.gate(&plan) {
        out::Go::Exit(c) => c,
        out::Go::Run => out::outcome(&plan, &config::apply_config_plan(&ctx.env, &plan)),
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
            return Ok(config_plan(ctx, config::plan_fleet_new(&ctx.env, fleet, &label)));
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
    Ok(config_plan(ctx, plan))
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
            ctx.args.allow(&["--brief", "--mode"])?;
            config::plan_squadron_new(&ctx.env, fleet, sq, ctx.args.value("--brief").unwrap_or(""), ctx.args.value("--mode"))
        }
        "set" => {
            ctx.args.allow(&["--brief", "--mode"])?;
            config::plan_squadron_set(&ctx.env, &id, ctx.args.value("--brief"), ctx.args.value("--mode"))
        }
        "rm" => {
            ctx.args.allow(&[])?;
            return Ok(retire(ctx, &id));
        }
        v => return Err(format!("squadron {v}: use new, set or rm")),
    };
    Ok(config_plan(ctx, plan))
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
                            fix: Some(nimitz_core::sessions::restart_cmd(&id)),
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

/// A ship folder Claude Code will not trust: the plan trusts it (after the
/// config is written). A managed folder is under the data root (`nimitz init` trusts that).
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
    match nimitz_core::trust::for_plan(&ctx.env, Some(id), &config::expand_dir(dir)) {
        Some(Ok(step)) => plan.steps.push(step),
        Some(Err(issue)) => plan.warnings.push(issue),
        None => {}
    }
}

/// Stop the ship, remove its job rows (transcripts stay), then drop it from the config.
/// `ship rm`, `squadron rm`, `fleet rm`: core's retire plan (stop, rm the job
/// rows, then the config), applied and verified in one go.
/// A retire that takes its caller (`ship rm` of itself, `fleet rm` of its
/// fleet) runs whole, last, detached, once the caller's turn ends (`self_end`).
fn retire(ctx: &Ctx, id: &Id) -> i32 {
    let plan = nimitz_core::sessions::plan_retire(&ctx.env, &ctx.snap(), &ctx.caller, id);
    let snap = ctx.snap();
    let me = crate::self_end::me(ctx, &snap).filter(|m| nimitz_core::sessions::inside(m, id));
    let verb = match id.role() {
        Role::Admiral => "fleet",
        Role::Commodore => "squadron",
        Role::Ship => "ship",
    };
    match ctx.gate(&plan) {
        out::Go::Exit(c) => c.max(match (&me, ctx.plan_only() && plan.ok()) {
            (Some(m), true) => crate::self_end::defer(ctx, m, &[verb.to_string(), "rm".into(), id.0.clone()]),
            _ => 0,
        }),
        out::Go::Run => match &me {
            Some(m) => crate::self_end::defer(ctx, m, &[verb.to_string(), "rm".into(), id.0.clone()]),
            None => {
                let code = out::outcome(&plan, &nimitz_core::sessions::apply(&ctx.env, &plan));
                // a removed member's self-end notes go with it
                if code == 0 {
                    nimitz_core::self_end::forget_under(&ctx.env, id);
                }
                code
            }
        },
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
    Ok(config_plan(ctx, plan))
}

// ------------------------------------------------------------------ migrate

/// `nimitz migrate fleet-skill <dir...> --into <F>`: plan by default; `--yes` applies.
/// Steps that would change nothing are dropped, so a second run is a no-op.
/// `nimitz migrate --plan|--yes [--shared]`: every format step due here
/// (docs/design/migrations.md section 4); `--yes` runs the local ones, and
/// with `--shared` the shared ones (they affect every machine that reads the
/// data root). `nimitz migrate fleet-skill <dir...> --into <F>`: the import
/// of old fleet-skill fleets.
pub fn migrate(ctx: &Ctx) -> Result<i32, String> {
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
        out::ok(&format!("{} {}", d.label(), d.file.display()));
    }
    for b in &ran.backups {
        out::line(&format!("       backup: {}", b.display()));
    }
    for (d, e) in &ran.failed {
        out::error(&format!("{} {}: {e} (the file is as it was; log: {})", d.label(), d.file.display(), formats::log_path(&ctx.env).display()));
    }
    let left = formats::due(&ctx.env, formats::ARTIFACTS, None).into_iter().filter(|d| !d.silent).collect::<Vec<_>>();
    let (local, shared) = (left.iter().filter(|d| d.scope == Scope::Local).count(), left.iter().filter(|d| d.scope == Scope::Shared).count());
    if ran.failed.is_empty() {
        let mut more = vec![];
        if shared > 0 {
            more.push(format!("{shared} shared step(s) left for `nimitz migrate --shared --yes`"));
        }
        if local > 0 {
            more.push(format!("{local} local step(s) still due: run it again"));
        }
        out::ok(&format!("migrate: done{}", if more.is_empty() { String::new() } else { format!(" ({})", more.join("; ")) }));
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
        out::line(&format!("  {:<6}  {}  {}", d.scope.as_str(), d.label(), d.file.display()));
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
    Ok(config_plan(ctx, plan))
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
}
