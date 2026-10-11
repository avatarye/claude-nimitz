//! `nimitz harness list|add|rm|push`: a fleet's harness (`nimitz_core::harness`,
//! docs/design/harness.md). Every member lists its own fleet's; the fleet's
//! admiral and the user change it (planned) and push it.
use crate::kb_cmd::fleet_of;
use crate::out;
use crate::run::refusal;
use crate::Ctx;
use nimitz_core::harness::{self, Kind};
use nimitz_core::{Caller, Id, Plan, Role, Step};
use std::path::Path;

pub fn harness(ctx: &Ctx) -> Result<i32, String> {
    let verb = ctx.args.need(0, "list|add|rm|push")?.to_string();
    let fleet = match fleet_of(ctx) {
        Ok(f) => f,
        Err(done) => return done,
    };
    nimitz_core::config::load_fleet(&ctx.env, &fleet).map_err(|i| i.message)?;
    let fid = Id::admiral(&fleet);
    // a change is the admiral's or yours: said before anything is read
    if let Caller::Member { id, role } = &ctx.caller {
        if verb != "list" && *role != Role::Admiral {
            out::issue(&refusal("authority", format!("`nimitz harness {verb}` is the fleet's admiral's or the user's, not {}'s", id.name()), None, Some(fid)));
            return Ok(1);
        }
    }
    match verb.as_str() {
        "list" => {
            ctx.args.allow(&["--fleet"])?;
            ctx.args.at_most(1)?;
            list(ctx, &fleet);
            Ok(0)
        }
        "add" => {
            ctx.args.allow(&["--fleet", "--name", "--for"])?;
            ctx.args.at_most(3)?;
            let kind = Kind::parse(ctx.args.need(1, "rule|agent|skill|settings")?).ok_or("harness add: rule, agent, skill or settings")?;
            let src = Path::new(ctx.args.need(2, "the file (a skill: its folder)")?);
            let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let name = ctx.args.value("--name").map(str::to_string).unwrap_or(stem);
            let scope = ctx.args.value("--for").map(|s| Id(s.to_string()));
            if scope.is_some() && kind != Kind::Settings {
                return Err("--for goes with settings (a squadron's, a member's)".into());
            }
            let files = harness::item_files(kind, &name, src, scope.as_ref(), &fleet)?;
            let root = harness::dir(&ctx.env, &fleet);
            let mut plan = Plan { summary: format!("harness add {} {name} ({fleet})", kind.folder()), ..Default::default() };
            for (rel, text) in files {
                let p = root.join(&rel);
                if std::fs::read_to_string(&p).ok().as_deref() == Some(text.as_str()) {
                    continue;
                }
                let why = format!("the fleet's harness ({}): {}", kind.folder(), live_when(kind));
                plan.steps.push(Step { id: Some(fid.clone()), cmd: vec!["write".into(), p.to_string_lossy().into_owned(), text], cwd: None, why });
            }
            Ok(crate::crud::config_plan(ctx, plan, Some(&fid)))
        }
        "rm" => {
            ctx.args.allow(&["--fleet"])?;
            ctx.args.at_most(3)?;
            let kind = Kind::parse(ctx.args.need(1, "rule|agent|skill|settings")?).ok_or("harness rm: rule, agent, skill or settings")?;
            let what = ctx.args.need(2, "the name (settings: F, F/SQ or F/SQ/SHIP)")?;
            let root = harness::dir(&ctx.env, &fleet);
            let p = match kind {
                Kind::Settings => root.join(harness::settings_rel(Some(&Id(what.to_string())), &fleet)?),
                Kind::Skill => root.join("skills").join(what),
                _ => root.join(kind.folder()).join(format!("{what}.md")),
            };
            let mut plan = Plan { summary: format!("harness rm {} {what} ({fleet})", kind.folder()), ..Default::default() };
            if !p.exists() {
                plan.refusals.push(refusal("harness.missing", format!("{} is not there", p.display()), Some(format!("nimitz harness list --fleet {fleet}")), Some(fid.clone())));
            } else {
                plan.steps.push(Step { id: Some(fid.clone()), cmd: vec!["archive".into(), p.to_string_lossy().into_owned()], cwd: None, why: format!("out of the fleet's harness (kept as .archived): {}", live_when(kind)) });
            }
            Ok(crate::crud::config_plan(ctx, plan, Some(&fid)))
        }
        "push" => {
            ctx.args.allow(&["--fleet"])?;
            ctx.args.at_most(1)?;
            push(ctx, &fleet);
            Ok(0)
        }
        v => Err(format!("harness {v}: use list, add, rm or push")),
    }
}

/// When a change of `kind` reaches the members.
fn live_when(kind: Kind) -> &'static str {
    match kind {
        Kind::Rule => "every member gets it at its next prompt",
        Kind::Agent | Kind::Skill => "a member launched from now on has it; a running one after /reload-plugins in its session, or nimitz start F -f",
        Kind::Settings => "at each member's next launch (nimitz start F -f)",
    }
}

fn list(ctx: &Ctx, fleet: &str) {
    let root = harness::dir(&ctx.env, fleet);
    out::line(&format!("fleet {fleet}'s harness: {}", root.display()));
    for kind in [Kind::Rule, Kind::Agent, Kind::Skill] {
        let n = harness::names(&ctx.env, fleet, kind);
        out::line(&format!("  {:<9} {}", kind.folder(), if n.is_empty() { "-".to_string() } else { n.join(", ") }));
    }
    let mut settings: Vec<String> = vec![];
    let sd = root.join("settings");
    let mut stack = vec![sd.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "json") {
                settings.push(p.strip_prefix(&sd).unwrap_or(&p).to_string_lossy().into_owned());
            }
        }
    }
    settings.sort();
    out::line(&format!("  {:<9} {}", "settings", if settings.is_empty() { "-".to_string() } else { settings.join(", ") }));
    if let Some((body, _)) = harness::rules(&ctx.env, fleet) {
        out::line(&format!("  the rules members get: {} characters (at most {})", body.chars().count(), harness::RULES_MAX));
    }
    for i in harness::issues(&ctx.env, fleet) {
        out::issue(&i);
    }
}

/// Make this machine's plugin again and say what is live where.
fn push(ctx: &Ctx, fleet: &str) {
    match harness::plugin(&ctx.env, fleet) {
        Some(p) => out::ok(&format!("plugin {} at {} (agents and skills)", harness::plugin_name(fleet), p.display())),
        None => out::ok("no agents or skills: no plugin"),
    }
    for i in harness::issues(&ctx.env, fleet) {
        out::issue(&i);
    }
    out::line(&format!("  rules: live, every member gets them at its next prompt"));
    out::line("  agents, skills: in members launched from now on; a running member: /reload-plugins in its session");
    out::line(&format!("  settings: at each member's next launch"));
    out::line(&format!("  to relaunch every live member of the fleet now: nimitz start {fleet} -f --plan  # then --yes (the user's or the admiral's)"));
}
