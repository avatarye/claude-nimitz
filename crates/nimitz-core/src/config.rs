//! Fleet and squadron config: load, validate, save, CRUD (contracts.md section 3). Owner: core-sessions (was core-config).
use crate::env::Env;
use crate::types::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn issue(code: &str, severity: Severity, message: String, fix: Option<String>, id: Option<Id>) -> Issue {
    Issue { code: code.into(), severity, message, fix, id }
}

/// `[A-Z0-9_]+` (fleet, squadron and ship names).
pub fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// Machine labels: lowercase, `[a-z0-9._-]+`.
pub fn valid_label(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_'))
}

/// Ship names that would take another role's address (`[F.SQ]-COMMODORE`).
pub const RESERVED_SHIP_NAMES: [&str; 2] = ["COMMODORE", "ADMIRAL"];

/// Is `name` reserved for a role (COMMODORE, ADMIRAL)?
pub fn reserved_ship_name(name: &str) -> bool {
    RESERVED_SHIP_NAMES.contains(&name)
}

/// Parse a TOML file; the error is one line: `path:line:col: message`.
fn read_toml<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T, String> {
    let raw = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    toml::from_str(&raw).map_err(|e: toml::de::Error| {
        let at = e.span().map(|sp| {
            let before = &raw[..sp.start.min(raw.len())];
            let line = before.matches('\n').count() + 1;
            let col = before.len() - before.rfind('\n').map(|i| i + 1).unwrap_or(0) + 1;
            format!(":{line}:{col}")
        });
        format!("{}{}: {}", p.display(), at.unwrap_or_default(), e.message().trim())
    })
}

/// This machine's label in the fleet's `machines` table, if listed.
fn machine_label(env: &Env, c: &FleetConfig) -> Option<String> {
    c.machines
        .iter()
        .find(|(_, m)| env.matches_hostname(&m.hostname))
        .map(|(label, _)| label.clone())
}

/// Every fleet under NIMITZ_HOME, with config errors as issues (never panics).
/// A squadron whose squadron.toml does not parse is left out of its fleet (and
/// reported); the rest of the fleet still loads.
pub fn load_all(env: &Env) -> (Vec<FleetOnDisk>, Vec<Issue>) {
    let mut fleets = vec![];
    let mut issues = vec![];
    let entries = match std::fs::read_dir(&env.home) {
        Ok(e) => e,
        Err(_) => return (fleets, issues), // no NIMITZ_HOME yet: nothing to load
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("fleet.toml").is_file())
        .collect();
    dirs.sort();
    for dir in dirs {
        match load_fleet_dir(env, &dir) {
            Ok((f, broken)) => {
                issues.extend(broken);
                fleets.push(f);
            }
            Err(i) => issues.push(i),
        }
    }
    (fleets, issues)
}

/// Squadron folders with a squadron.toml, sorted.
fn squadron_dirs(fleet_dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(fleet_dir.join("squadrons"))
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir() && p.join("squadron.toml").is_file()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// The fleet plus one `config.invalid` issue per squadron.toml that does not parse.
fn load_fleet_dir(env: &Env, dir: &Path) -> Result<(FleetOnDisk, Vec<Issue>), Issue> {
    let config: FleetConfig = read_toml(&dir.join("fleet.toml")).map_err(|e| {
        issue(
            "config.invalid",
            Severity::Error,
            format!("fleet.toml does not parse: {e}"),
            Some(format!("fix {} by hand, then nimitz check", dir.join("fleet.toml").display())),
            // no identity: the fleet's members are unknown, so this blocks every start
            None,
        )
    })?;
    let mut squadrons = vec![];
    let mut broken = vec![];
    for sd in squadron_dirs(dir) {
        match read_toml::<SquadronConfig>(&sd.join("squadron.toml")) {
            Ok(sq) => squadrons.push((sd, sq)),
            Err(e) => broken.push(unreadable_issue(&config.fleet, &sd, &e)),
        }
    }
    let machine = machine_label(env, &config);
    Ok((FleetOnDisk { dir: dir.to_path_buf(), config, squadrons, machine }, broken))
}

fn unreadable_issue(fleet: &str, sd: &Path, err: &str) -> Issue {
    let sq = sd.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default();
    issue(
        "config.invalid",
        Severity::Error,
        format!("squadron.toml does not parse: {err} (squadron {fleet}/{sq} and its ships are unreadable; the rest of the fleet still loads)"),
        Some(format!("fix {} by hand, then nimitz check {fleet}/{sq}", sd.join("squadron.toml").display())),
        Some(Id::commodore(fleet, &sq)),
    )
}

/// Squadrons of `f` left out because their squadron.toml does not parse: the
/// squadron id (from the folder name), its folder, and the `config.invalid` issue.
/// Their live members stay visible as members whose config is unreadable.
pub fn unreadable_squadrons(f: &FleetOnDisk) -> Vec<(Id, PathBuf, Issue)> {
    squadron_dirs(&f.dir)
        .into_iter()
        .filter(|sd| !f.squadrons.iter().any(|(d, _)| d == sd))
        .filter_map(|sd| {
            let err = read_toml::<SquadronConfig>(&sd.join("squadron.toml")).err()?;
            let i = unreadable_issue(&f.config.fleet, &sd, &err);
            Some((i.id.clone()?, sd, i))
        })
        .collect()
}

/// The fleet as far as it loads: a squadron whose squadron.toml does not parse
/// is left out (see `unreadable_squadrons`). Err only when fleet.toml itself is bad.
pub fn load_fleet(env: &Env, fleet: &str) -> Result<FleetOnDisk, Issue> {
    let dir = env.fleet_dir(fleet);
    if !dir.join("fleet.toml").is_file() {
        return Err(issue(
            "config.missing",
            Severity::Error,
            format!("fleet {fleet}: no fleet.toml in {}", dir.display()),
            Some(format!("nimitz fleet new {fleet}")),
            Some(Id::admiral(fleet)),
        ));
    }
    load_fleet_dir(env, &dir).map(|(f, _)| f)
}

/// The fleet, for a config change: refuses while any of its squadron.toml files
/// does not parse (a rewrite could drop or overwrite what is in it).
fn load_fleet_for_write(env: &Env, fleet: &str) -> Result<FleetOnDisk, Issue> {
    let f = load_fleet(env, fleet)?;
    match unreadable_squadrons(&f).into_iter().next() {
        Some((_, _, i)) => Err(i),
        None => Ok(f),
    }
}

/// `CLAUDE.md`, `CLAUDE.local.md` and `.claude/CLAUDE.md` in `dir`: every
/// session below it would inherit them.
fn inherited_md(dir: &Path) -> Vec<PathBuf> {
    ["CLAUDE.md", "CLAUDE.local.md", ".claude/CLAUDE.md"].iter().map(|n| dir.join(n)).filter(|p| p.is_file()).collect()
}

fn layout_md_issue(p: &Path, id: Option<Id>) -> Issue {
    issue(
        "config.layout",
        Severity::Error,
        format!("{} would be inherited by every session below {}", p.display(), p.parent().map(|d| d.display().to_string()).unwrap_or_default()),
        Some(format!("rm {}", p.display())),
        id,
    )
}

/// `config.layout` for NIMITZ_HOME itself (an instructions file there reaches every fleet).
pub fn home_layout(env: &Env) -> Vec<Issue> {
    inherited_md(&env.home).iter().map(|p| layout_md_issue(p, None)).collect()
}

/// Schema and layout rules; returns issues (config.invalid, config.layout, ...).
pub fn validate(env: &Env, f: &FleetOnDisk) -> Vec<Issue> {
    let mut out = vec![];
    let c = &f.config;
    let fid = Id::admiral(&c.fleet);
    let inv = |msg: String, id: Option<Id>| issue("config.invalid", Severity::Error, msg, None, id);

    if !valid_name(&c.fleet) {
        out.push(inv(format!("fleet name {:?} is not [A-Z0-9_]+", c.fleet), Some(fid.clone())));
    }
    if f.dir.file_name().map(|n| n.to_string_lossy().to_string()) != Some(c.fleet.to_lowercase()) {
        out.push(inv(
            format!("fleet {:?} lives in {} (folder must be the lowercased fleet name)", c.fleet, f.dir.display()),
            Some(fid.clone()),
        ));
    }
    for (label, m) in &c.machines {
        if !valid_label(label) {
            out.push(inv(format!("machine label {label:?} is not [a-z0-9._-]+"), Some(fid.clone())));
        }
        if m.hostname.is_empty() {
            out.push(inv(format!("machine {label:?} has an empty hostname"), Some(fid.clone())));
        }
    }
    // Instructions files every session below would inherit: in the fleet folder,
    // squadrons/, ships/ and ships/<SQ>/ (each with its .claude/).
    let mut md_dirs = vec![f.dir.clone(), f.dir.join("squadrons"), f.dir.join("ships")];
    if let Ok(rd) = std::fs::read_dir(f.dir.join("ships")) {
        let mut v: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir() && p.file_name() != Some(".claude".as_ref())).collect();
        v.sort();
        md_dirs.extend(v);
    }
    for d in &md_dirs {
        out.extend(inherited_md(d).iter().map(|p| layout_md_issue(p, Some(fid.clone()))));
    }

    let mut sq_names: Vec<&str> = vec![];
    for (sd, sq) in &f.squadrons {
        let sqid = Id::commodore(&c.fleet, &sq.squadron);
        if !valid_name(&sq.squadron) {
            out.push(inv(format!("squadron name {:?} is not [A-Z0-9_]+", sq.squadron), Some(sqid.clone())));
        }
        if sq_names.contains(&sq.squadron.as_str()) {
            out.push(inv(format!("squadron {:?} is defined twice", sq.squadron), Some(sqid.clone())));
        }
        sq_names.push(&sq.squadron);
        if sd.file_name().map(|n| n.to_string_lossy().to_string()) != Some(sq.squadron.to_lowercase()) {
            out.push(inv(
                format!("squadron {:?} lives in {} (folder must be the lowercased name)", sq.squadron, sd.display()),
                Some(sqid.clone()),
            ));
        }
        out.extend(crate::migrate::tender_config_issue(sd, sq, &sqid));
        if let Comms::Mode(m) = &sq.comms {
            if m != "open" && m != "commodore-only" {
                out.push(inv(
                    format!("squadron {}: comms {m:?} is not open|commodore-only|{{allow}}", sq.squadron),
                    Some(sqid.clone()),
                ));
            }
        }
        let ship_names: Vec<&str> = sq.ships.iter().map(|s| s.name.as_str()).collect();
        for (i, ship) in sq.ships.iter().enumerate() {
            let shid = Id::ship(&c.fleet, &sq.squadron, &ship.name);
            if !valid_name(&ship.name) {
                out.push(inv(format!("ship name {:?} is not [A-Z0-9_]+", ship.name), Some(shid.clone())));
            }
            if reserved_ship_name(&ship.name) {
                out.push(inv(format!("ship name {:?} is reserved: {shid} would answer to a role's address", ship.name), Some(shid.clone())));
            }
            if ship_names[..i].contains(&ship.name.as_str()) {
                out.push(inv(format!("ship {:?} is defined twice in {}", ship.name, sq.squadron), Some(shid.clone())));
            }
            if ship.prompt.is_none() && ship.prompt_file.is_none() {
                out.push(issue(
                    "config.invalid",
                    Severity::Warn,
                    format!("ship {shid} has no prompt and no prompt_file (cold launch will refuse)"),
                    Some(format!("nimitz ship set {shid}")),
                    Some(shid.clone()),
                ));
            }
            for d in &ship.depends_on {
                if !ship_names.contains(&d.as_str()) {
                    out.push(inv(format!("ship {shid} depends on unknown ship {d:?}"), Some(shid.clone())));
                }
            }
            for label in ship.dirs.keys().chain(ship.permission_mode.keys()) {
                if !c.machines.contains_key(label) {
                    out.push(inv(
                        format!("ship {shid}: machine label {label:?} is not in fleet.toml [machines]"),
                        Some(shid.clone()),
                    ));
                }
            }
        }
    }

    // Layout: a session folder inside another session folder (on this machine),
    // judged by `nesting` (contracts.md section 7).
    let mut folders: Vec<(Id, PathBuf, bool)> = vec![(fid.clone(), f.dir.join("admiral"), true)];
    for (sd, sq) in &f.squadrons {
        folders.push((Id::commodore(&c.fleet, &sq.squadron), sd.clone(), true));
        for ship in &sq.ships {
            let shid = Id::ship(&c.fleet, &sq.squadron, &ship.name);
            if let Some(d) = member_dir(env, f, &shid) {
                let managed = f.machine.as_ref().and_then(|m| ship.dirs.get(m)).map(|v| v == "managed").unwrap_or(false);
                folders.push((shid, d, managed));
            }
        }
    }
    // Compare real paths: a symlinked folder is the folder it points to.
    let folders: Vec<(Id, PathBuf, bool)> = folders.into_iter().map(|(id, p, m)| (id, crate::env::canon(&p), m)).collect();
    for (i, (a, pa, _)) in folders.iter().enumerate() {
        for (b, pb, _) in &folders[i + 1..] {
            if a != b && pa == pb {
                out.push(issue(
                    "config.layout",
                    Severity::Error,
                    format!("{a} and {b} share the folder {}: one folder holds one member", pa.display()),
                    Some(format!("nimitz ship set {b} --dir <another folder>")),
                    Some(b.clone()),
                ));
            }
        }
    }
    for (a, pa, a_managed) in &folders {
        for (b, pb, _) in &folders {
            if a != b && pa != pb && pb.starts_with(pa) {
                if let Some(sev) = nesting(pa, *a_managed) {
                    out.push(issue(
                        "config.layout",
                        sev,
                        format!("{b}'s folder {} is inside {a}'s folder {} ({})", pb.display(), pa.display(), nesting_why(*a_managed)),
                        None,
                        Some(b.clone()),
                    ));
                }
            }
        }
    }
    out
}

/// How bad is a member folder inside `outer` (contracts.md section 7)? An
/// error when `outer` is nimitz-managed (admiral, commodore, managed ship): its
/// CLAUDE.md is a role's instructions. A warning when `outer` is a user project
/// folder with a CLAUDE.md (the inner member inherits it). Nothing otherwise: a
/// ship whose folder is the home folder is fine.
pub fn nesting(outer: &Path, outer_managed: bool) -> Option<Severity> {
    if outer_managed {
        Some(Severity::Error)
    } else if outer.join("CLAUDE.md").is_file() {
        Some(Severity::Warn)
    } else {
        None
    }
}

fn nesting_why(outer_managed: bool) -> &'static str {
    if outer_managed {
        "a nimitz-managed folder: its role's CLAUDE.md would apply"
    } else {
        "that folder's CLAUDE.md would apply to it too"
    }
}

fn find_ship<'a>(f: &'a FleetOnDisk, id: &Id) -> Option<(&'a PathBuf, &'a SquadronConfig, &'a ShipDef)> {
    let p = id.parts();
    let (sq_name, ship_name) = (*p.get(1)?, *p.get(2)?);
    let (sd, sq) = f.squadrons.iter().find(|(_, s)| s.squadron == sq_name)?;
    let ship = sq.ships.iter().find(|s| s.name == ship_name)?;
    Some((sd, sq, ship))
}

fn expand_dir(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")).join(rest),
        None => PathBuf::from(p),
    }
}

/// The folder of a member on this machine (resolves "managed"); None if not deployed here.
pub fn member_dir(_env: &Env, f: &FleetOnDisk, id: &Id) -> Option<PathBuf> {
    if id.fleet() != f.config.fleet {
        return None; // ids of another fleet never resolve here
    }
    match id.role() {
        Role::Admiral => Some(f.dir.join("admiral")),
        Role::Commodore => {
            let sq = id.squadron()?;
            f.squadrons.iter().find(|(_, s)| s.squadron == sq).map(|(sd, _)| sd.clone())
        }
        Role::Ship => {
            let label = f.machine.as_ref()?;
            let (_, sq, ship) = find_ship(f, id)?;
            match ship.dirs.get(label)?.as_str() {
                "managed" => {
                    Some(f.dir.join("ships").join(sq.squadron.to_lowercase()).join(ship.name.to_lowercase()))
                }
                // the real path when it exists: Claude Code records (and trusts) that one
                d => Some(expand_dir(d)).map(|p| p.canonicalize().unwrap_or(p)),
            }
        }
    }
}

/// The launch prompt for a ship (prompt, or prompt_file relative to the squadron folder).
pub fn ship_prompt(f: &FleetOnDisk, id: &Id) -> Option<String> {
    let (sd, _, ship) = find_ship(f, id)?;
    if let Some(p) = &ship.prompt {
        return Some(p.clone());
    }
    let pf = ship.prompt_file.as_ref()?;
    let path = if Path::new(pf).is_absolute() { PathBuf::from(pf) } else { sd.join(pf) };
    std::fs::read_to_string(path).ok()
}

/// Permission mode for a member on this machine. Empty when nothing sets one:
/// then no `--permission-mode` is passed and Claude Code's own default (or the
/// user's settings) applies, as it did for the old fleet's leads and ships.
pub fn permission_mode(f: &FleetOnDisk, id: &Id) -> String {
    let fallback = String::new();
    let squadron_mode = || {
        id.squadron()
            .and_then(|sq| f.squadrons.iter().find(|(_, s)| s.squadron == sq))
            .and_then(|(_, s)| s.permission_mode.clone())
    };
    match id.role() {
        Role::Admiral => f.config.admiral.permission_mode.clone().unwrap_or(fallback),
        Role::Commodore => squadron_mode().unwrap_or(fallback),
        Role::Ship => find_ship(f, id)
            .and_then(|(_, _, ship)| f.machine.as_ref().and_then(|m| ship.permission_mode.get(m).cloned()))
            .or_else(squadron_mode)
            .unwrap_or(fallback),
    }
}

// ------------------------------------------------------------------ CRUD plans
//
// Steps use cmd ["mkdir", <dir>] | ["write", <path>, <contents>] | ["archive", <path>]
// and run only through `apply_config_plan`.

pub(crate) fn step_mkdir(id: Option<Id>, dir: &Path, why: &str) -> Step {
    Step { id, cmd: vec!["mkdir".into(), dir.to_string_lossy().into_owned()], cwd: None, why: why.into() }
}
pub(crate) fn step_write(id: Option<Id>, path: &Path, contents: String, why: &str) -> Step {
    Step { id, cmd: vec!["write".into(), path.to_string_lossy().into_owned(), contents], cwd: None, why: why.into() }
}
fn step_archive(id: Option<Id>, path: &Path, why: &str) -> Step {
    Step { id, cmd: vec!["archive".into(), path.to_string_lossy().into_owned()], cwd: None, why: why.into() }
}

fn refuse(plan: &mut Plan, code: &str, msg: String, fix: Option<String>, id: Option<Id>) {
    plan.refusals.push(issue(code, Severity::Error, msg, fix, id));
}

pub(crate) fn toml_out<T: serde::Serialize>(v: &T) -> String {
    toml::to_string_pretty(v).unwrap_or_default()
}

/// An admiral's CLAUDE.md: who it is; its rules come from `rules` at session start.
pub(crate) fn admiral_claude_md(fleet: &str) -> String {
    crate::rules::brief(&Id::admiral(fleet))
}

/// A commodore's CLAUDE.md: who it is; its rules come from `rules` at session start.
pub(crate) fn commodore_claude_md(fleet: &str, sq: &str) -> String {
    crate::rules::brief(&Id::commodore(fleet, sq))
}

pub(crate) fn squadron_md(fleet: &str, sq: &str, brief: &str) -> String {
    let body = if brief.is_empty() { "(no brief yet)" } else { brief };
    format!("# Squadron {sq} ({fleet})\n\n{body}\n")
}

pub fn plan_fleet_new(env: &Env, fleet: &str, machine_label: &str) -> Plan {
    let mut plan = Plan { summary: format!("create fleet {fleet}"), ..Default::default() };
    let id = Id::admiral(fleet);
    if !valid_name(fleet) {
        refuse(&mut plan, "config.invalid", format!("fleet name {fleet:?} is not [A-Z0-9_]+"), None, Some(id.clone()));
    }
    if !valid_label(machine_label) {
        refuse(&mut plan, "config.invalid", format!("machine label {machine_label:?} is not [a-z0-9._-]+"), None, Some(id.clone()));
    }
    let dir = env.fleet_dir(fleet);
    if dir.join("fleet.toml").is_file() {
        refuse(&mut plan, "config.exists", format!("fleet {fleet} already exists at {}", dir.display()), None, Some(id.clone()));
    }
    if !plan.ok() {
        return plan;
    }
    let config = FleetConfig {
        fleet: fleet.into(),
        machines: [(
            machine_label.to_string(),
            MachineDef { hostname: env.hostname.clone(), os: Some(std::env::consts::OS.into()) },
        )]
        .into(),
        admiral: AdmiralDef::default(),
    };
    plan.steps = vec![
        step_mkdir(Some(id.clone()), &dir, "fleet folder"),
        step_mkdir(Some(id.clone()), &dir.join("admiral"), "admiral's folder"),
        step_mkdir(Some(id.clone()), &dir.join("squadrons"), "squadron folders live here"),
        step_write(Some(id.clone()), &dir.join("fleet.toml"), toml_out(&config), "fleet settings and machine labels"),
        step_write(Some(id.clone()), &dir.join("admiral/CLAUDE.md"), admiral_claude_md(fleet), "admiral brief"),
    ];
    plan
}

pub fn plan_squadron_new(env: &Env, fleet: &str, sq: &str, brief: &str, comms: Comms) -> Plan {
    let mut plan = Plan { summary: format!("create squadron {fleet}/{sq}"), ..Default::default() };
    let id = Id::commodore(fleet, sq);
    if !valid_name(sq) {
        refuse(&mut plan, "config.invalid", format!("squadron name {sq:?} is not [A-Z0-9_]+"), None, Some(id.clone()));
        return plan;
    }
    let f = match load_fleet_for_write(env, fleet) {
        Ok(f) => f,
        Err(i) => {
            plan.refusals.push(i);
            return plan;
        }
    };
    if f.squadrons.iter().any(|(_, s)| s.squadron == sq) {
        refuse(&mut plan, "config.exists", format!("squadron {sq} already exists in {fleet}"), None, Some(id.clone()));
        return plan;
    }
    let sd = f.dir.join("squadrons").join(sq.to_lowercase());
    let config = SquadronConfig {
        squadron: sq.into(),
        brief: Some("SQUADRON.md".into()),
        comms,
        permission_mode: None,
        ships: vec![],
        tenders: None,
    };
    plan.steps = vec![
        step_mkdir(Some(id.clone()), &sd, "commodore's folder"),
        step_write(Some(id.clone()), &sd.join("squadron.toml"), toml_out(&config), "squadron settings"),
        step_write(Some(id.clone()), &sd.join("SQUADRON.md"), squadron_md(fleet, sq, brief), "the brief"),
        step_write(Some(id.clone()), &sd.join("CLAUDE.md"), commodore_claude_md(fleet, sq), "commodore brief"),
    ];
    plan
}

/// A ship folder value as stored in squadron.toml: `managed`, an absolute path
/// or `~/...`. Refused: empty, `<machine label>=<folder>` (the machine goes in
/// its own field), and relative paths (a front end makes them absolute first).
pub fn normalize_ship_dir(f: &FleetOnDisk, d: &str) -> Result<String, String> {
    let d = d.trim();
    if d.is_empty() {
        return Err("the folder is empty (use managed, an absolute path or ~/...)".into());
    }
    if let Some((label, rest)) = d.split_once('=') {
        if f.config.machines.contains_key(label) {
            return Err(format!("{d:?} puts the machine in the folder; use --dir {rest} --machine {label}"));
        }
    }
    if d == "managed" || d.starts_with('/') || d.starts_with("~/") {
        let t = d.trim_end_matches('/');
        return Ok(if t.is_empty() { "/".into() } else { t.to_string() });
    }
    Err(format!("folder {d:?} is relative; give an absolute path, ~/..., or managed"))
}

/// Shared tail of ship new/set: check folders, rewrite squadron.toml, make managed folders.
fn plan_ship_write(f: &FleetOnDisk, sq_dir: &Path, sq: SquadronConfig, id: &Id, ship: &ShipDef, plan: &mut Plan) {
    for (label, d) in &ship.dirs {
        if let Err(why) = normalize_ship_dir(f, d) {
            refuse(plan, "config.invalid", format!("{id} on {label}: {why}"), None, Some(id.clone()));
            continue;
        }
        if !f.config.machines.contains_key(label) {
            refuse(
                plan,
                "config.missing",
                format!("machine label {label:?} is not in {}'s fleet.toml [machines]", f.config.fleet),
                Some(format!("nimitz machine add {} {label}", f.config.fleet)),
                Some(id.clone()),
            );
        }
        if d != "managed" {
            // A session folder inside another session folder, both ways, judged by `nesting`.
            // Real paths on both sides: a symlink to a folder is that folder.
            let dir = crate::env::canon(&expand_dir(d));
            let mut others: Vec<(PathBuf, bool)> = vec![(f.dir.join("admiral"), true), (f.dir.join("squadrons"), true)];
            for (osd, osq) in &f.squadrons {
                others.push((osd.clone(), true));
                for oship in &osq.ships {
                    if oship.name != ship.name || osq.squadron != sq.squadron {
                        match oship.dirs.get(label).map(String::as_str) {
                            Some("managed") => others.push((f.dir.join("ships").join(osq.squadron.to_lowercase()).join(oship.name.to_lowercase()), true)),
                            Some(od) => others.push((expand_dir(od), false)),
                            None => {}
                        }
                    }
                }
            }
            let _ = sq_dir; // the squadron dir is already in `others` via f.squadrons
            let others: Vec<(PathBuf, bool)> = others.into_iter().map(|(p, m)| (crate::env::canon(&p), m)).collect();
            let sevs: Vec<Severity> = others
                .iter()
                .filter_map(|(p, managed)| {
                    if &dir == p {
                        Some(Severity::Error) // two members in one folder
                    } else if dir.starts_with(p) {
                        nesting(p, *managed)
                    } else if p.starts_with(&dir) {
                        nesting(&dir, false) // the new folder is a user project folder
                    } else {
                        None
                    }
                })
                .collect();
            let worst = if sevs.contains(&Severity::Error) { Some(Severity::Error) } else { sevs.first().copied() };
            match worst {
                Some(Severity::Error) => refuse(
                    plan,
                    "config.layout",
                    format!("{id}: folder {} nests with a nimitz-managed folder (or another member's folder)", dir.display()),
                    None,
                    Some(id.clone()),
                ),
                Some(sev) => plan.warnings.push(issue(
                    "config.layout",
                    sev,
                    format!("{id}: folder {} nests with a project folder that has a CLAUDE.md", dir.display()),
                    None,
                    Some(id.clone()),
                )),
                None => {}
            }
        }
    }
    if !plan.ok() {
        return;
    }
    if ship.dirs.values().any(|d| d == "managed") {
        let managed = f.dir.join("ships").join(sq.squadron.to_lowercase()).join(ship.name.to_lowercase());
        plan.steps.push(step_mkdir(Some(id.clone()), &managed, "managed ship folder"));
    }
    plan.steps.push(step_write(Some(id.clone()), &sq_dir.join("squadron.toml"), toml_out(&sq), "squadron settings"));
}

pub fn plan_ship_new(env: &Env, fleet: &str, sq: &str, ship: ShipDef) -> Plan {
    let mut plan = Plan { summary: format!("add ship {fleet}/{sq}/{}", ship.name), ..Default::default() };
    let id = Id::ship(fleet, sq, &ship.name);
    if !valid_name(&ship.name) {
        refuse(&mut plan, "config.invalid", format!("ship name {:?} is not [A-Z0-9_]+", ship.name), None, Some(id.clone()));
        return plan;
    }
    if reserved_ship_name(&ship.name) {
        refuse(
            &mut plan,
            "config.invalid",
            format!("ship name {:?} is reserved: {id} would answer to [{fleet}.{sq}]-{}, a role's address", ship.name, ship.name),
            Some(format!("nimitz ship new {fleet}/{sq}/<another name>")),
            Some(id.clone()),
        );
        return plan;
    }
    let f = match load_fleet_for_write(env, fleet) {
        Ok(f) => f,
        Err(i) => {
            plan.refusals.push(i);
            return plan;
        }
    };
    let Some((sd, sqc)) = f.squadrons.iter().find(|(_, s)| s.squadron == sq) else {
        refuse(
            &mut plan,
            "config.missing",
            format!("no squadron {sq} in {fleet}"),
            Some(format!("nimitz squadron new {fleet}/{sq}")),
            Some(id.clone()),
        );
        return plan;
    };
    if sqc.ships.iter().any(|s| s.name == ship.name) {
        refuse(&mut plan, "config.exists", format!("ship {id} already exists"), None, Some(id.clone()));
        return plan;
    }
    let mut new_sq = sqc.clone();
    new_sq.ships.push(ship.clone());
    let sd = sd.clone();
    plan_ship_write(&f, &sd, new_sq, &id, &ship, &mut plan);
    plan
}

pub fn plan_ship_set(env: &Env, id: &Id, ship: ShipDef) -> Plan {
    let mut plan = Plan { summary: format!("update ship {id}"), ..Default::default() };
    if id.role() != Role::Ship {
        refuse(&mut plan, "config.invalid", format!("{id} is not a ship id"), None, Some(id.clone()));
        return plan;
    }
    if reserved_ship_name(&ship.name) {
        refuse(&mut plan, "config.invalid", format!("ship name {:?} is reserved for a role's address", ship.name), Some(format!("nimitz ship rm {id}")), Some(id.clone()));
        return plan;
    }
    if ship.name != id.parts()[2] {
        refuse(&mut plan, "config.invalid", format!("ship def names {:?}, id names {id} (no renames)", ship.name), None, Some(id.clone()));
        return plan;
    }
    let f = match load_fleet_for_write(env, id.fleet()) {
        Ok(f) => f,
        Err(i) => {
            plan.refusals.push(i);
            return plan;
        }
    };
    let sq_name = id.squadron().unwrap_or_default().to_string();
    let Some((sd, sqc)) = f.squadrons.iter().find(|(_, s)| s.squadron == sq_name) else {
        refuse(&mut plan, "config.missing", format!("no squadron {sq_name} in {}", id.fleet()), None, Some(id.clone()));
        return plan;
    };
    let Some(pos) = sqc.ships.iter().position(|s| s.name == ship.name) else {
        refuse(&mut plan, "config.missing", format!("no ship {id}"), Some(format!("nimitz ship new {id}")), Some(id.clone()));
        return plan;
    };
    let mut new_sq = sqc.clone();
    new_sq.ships[pos] = ship.clone();
    let sd = sd.clone();
    plan_ship_write(&f, &sd, new_sq, id, &ship, &mut plan);
    plan
}

pub fn plan_rm(env: &Env, id: &Id) -> Plan {
    let mut plan = Plan { summary: format!("remove {id} from the config"), ..Default::default() };
    let f = match load_fleet_for_write(env, id.fleet()) {
        Ok(f) => f,
        Err(i) => {
            plan.refusals.push(i);
            return plan;
        }
    };
    match id.role() {
        Role::Admiral => {
            // Delete the fleet: only with no squadrons left; the config is archived.
            if !f.squadrons.is_empty() {
                refuse(
                    &mut plan,
                    "config.nonempty",
                    format!("fleet {} still has {} squadron(s)", id.fleet(), f.squadrons.len()),
                    Some(format!("nimitz squadron rm {}/<SQ>", id.fleet())),
                    Some(id.clone()),
                );
            } else {
                plan.steps.push(step_archive(Some(id.clone()), &f.dir.join("fleet.toml"), "archive the fleet config"));
            }
        }
        Role::Commodore => {
            let sq_name = id.squadron().unwrap_or_default().to_string();
            match f.squadrons.iter().find(|(_, s)| s.squadron == sq_name) {
                None => refuse(&mut plan, "config.missing", format!("no squadron {id}"), None, Some(id.clone())),
                Some((sd, sqc)) if !sqc.ships.is_empty() => refuse(
                    &mut plan,
                    "config.nonempty",
                    format!("squadron {id} still has {} ship(s) at {}", sqc.ships.len(), sd.display()),
                    Some(format!("nimitz ship rm {id}/<SHIP>")),
                    Some(id.clone()),
                ),
                Some((sd, _)) => {
                    plan.steps.push(step_archive(Some(id.clone()), &sd.join("squadron.toml"), "archive the squadron config"))
                }
            }
        }
        Role::Ship => {
            let sq_name = id.squadron().unwrap_or_default().to_string();
            let ship_name = id.parts()[2].to_string();
            match f.squadrons.iter().find(|(_, s)| s.squadron == sq_name) {
                None => refuse(&mut plan, "config.missing", format!("no squadron {}/{sq_name}", id.fleet()), None, Some(id.clone())),
                Some((sd, sqc)) => {
                    if !sqc.ships.iter().any(|s| s.name == ship_name) {
                        refuse(&mut plan, "config.missing", format!("no ship {id}"), None, Some(id.clone()));
                    } else {
                        let mut new_sq = sqc.clone();
                        new_sq.ships.retain(|s| s.name != ship_name);
                        plan.steps.push(step_write(
                            Some(id.clone()),
                            &sd.join("squadron.toml"),
                            toml_out(&new_sq),
                            "squadron settings without the ship",
                        ));
                    }
                }
            }
        }
    }
    plan
}

/// `nimitz machine add <F> <label>` with no `--hostname`: see `plan_machine_add_on`.
pub fn plan_machine_add(env: &Env, fleet: &str, label: &str, dirs: Vec<(Id, String)>) -> Plan {
    plan_machine_add_on(env, fleet, label, None, dirs)
}

/// `nimitz machine add <F> <label> [--hostname H]`. With no hostname the label
/// names this machine, but only when no other label of the fleet already does
/// (a second label for this host would make two machines of one); a label for
/// another machine needs `--hostname`.
pub fn plan_machine_add_on(env: &Env, fleet: &str, label: &str, hostname: Option<&str>, dirs: Vec<(Id, String)>) -> Plan {
    let mut plan = Plan { summary: format!("add machine {label} to {fleet}"), ..Default::default() };
    let fid = Id::admiral(fleet);
    if !valid_label(label) {
        refuse(&mut plan, "config.invalid", format!("machine label {label:?} is not [a-z0-9._-]+"), None, Some(fid.clone()));
        return plan;
    }
    let f = match load_fleet_for_write(env, fleet) {
        Ok(f) => f,
        Err(i) => {
            plan.refusals.push(i);
            return plan;
        }
    };
    if f.config.machines.contains_key(label) {
        refuse(&mut plan, "config.exists", format!("machine {label} is already in {fleet}"), None, Some(fid.clone()));
        return plan;
    }
    let host = match hostname.map(str::trim) {
        Some("") => {
            refuse(&mut plan, "config.invalid", "--hostname is empty".into(), Some(format!("nimitz machine add {fleet} {label} --hostname <H>")), Some(fid.clone()));
            return plan;
        }
        Some(h) => h.to_lowercase(),
        None => env.hostname.clone(),
    };
    let same_host = |h: &str| {
        let short = |x: &str| x.split('.').next().unwrap_or("").to_string();
        h.eq_ignore_ascii_case(&host) || short(&h.to_lowercase()) == short(&host)
    };
    if let Some((other, _)) = f.config.machines.iter().find(|(_, m)| same_host(&m.hostname)) {
        let why = if hostname.is_none() {
            format!("no --hostname given, and this machine ({host}) is already {fleet}'s machine {other:?}")
        } else {
            format!("host {host} is already {fleet}'s machine {other:?}")
        };
        refuse(
            &mut plan,
            "machine.host",
            format!("machine {label}: {why}; one host has one label"),
            Some(format!("nimitz machine add {fleet} {label} --hostname <the other machine's hostname>")),
            Some(fid.clone()),
        );
        return plan;
    }
    for (id, _) in &dirs {
        if id.fleet() != fleet || id.role() != Role::Ship {
            refuse(&mut plan, "config.invalid", format!("{id} is not a ship of fleet {fleet}"), None, Some(id.clone()));
        }
    }
    let mut config = f.config.clone();
    config.machines.insert(
        label.to_string(),
        // the os is known only for this machine
        MachineDef { hostname: host.clone(), os: (host == env.hostname).then(|| std::env::consts::OS.into()) },
    );
    plan.steps.push(step_write(Some(fid.clone()), &f.dir.join("fleet.toml"), toml_out(&config), "fleet settings with the new machine"));

    // Per-ship folders for the new machine, grouped per squadron.
    for (sd, sqc) in &f.squadrons {
        let mut new_sq = sqc.clone();
        let mut touched = false;
        for (id, dir) in &dirs {
            if id.squadron() != Some(sqc.squadron.as_str()) {
                continue;
            }
            let ship_name = id.parts().get(2).map(|s| s.to_string()).unwrap_or_default();
            match new_sq.ships.iter_mut().find(|s| s.name == ship_name) {
                Some(s) => {
                    s.dirs.insert(label.to_string(), dir.clone());
                    touched = true;
                }
                None => refuse(&mut plan, "config.missing", format!("no ship {id}"), None, Some(id.clone())),
            }
        }
        if touched {
            plan.steps.push(step_write(
                Some(Id::commodore(fleet, &sqc.squadron)),
                &sd.join("squadron.toml"),
                toml_out(&new_sq),
                "ship folders for the new machine",
            ));
        }
    }
    plan
}

// ------------------------------------------------------------------ legacy names

/// `<fleet>/legacy.toml`: old fleet-skill session names and the identity each
/// became (written by `nimitz migrate fleet-skill`):
///
/// ```toml
/// [names]
/// "[GENERAL]-HOME" = "MAIN/GENERAL/HOME"
/// "[GENERAL]-LEAD" = "MAIN/GENERAL"
/// ```
pub fn legacy_path(f: &FleetOnDisk) -> PathBuf {
    f.dir.join("legacy.toml")
}

/// The fleet's legacy map (empty when there is none or it does not parse).
pub fn legacy_map(f: &FleetOnDisk) -> BTreeMap<String, Id> {
    let raw = std::fs::read_to_string(legacy_path(f)).unwrap_or_default();
    let v: toml::Value = toml::from_str(&raw).unwrap_or(toml::Value::Table(Default::default()));
    v.get("names")
        .and_then(|n| n.as_table())
        .map(|t| {
            t.iter()
                .filter_map(|(k, v)| Some((k.clone(), Id(v.as_str()?.to_string()))))
                .filter(|(_, id)| id.fleet() == f.config.fleet)
                .collect()
        })
        .unwrap_or_default()
}

/// The legacy map as file contents.
pub fn legacy_toml(map: &BTreeMap<String, Id>) -> String {
    let mut out = String::from("# Old fleet-skill session names and the nimitz identity each became.\n# Written by `nimitz migrate fleet-skill`; a live session with one of these\n# names belongs to the old fleet (issue `legacy`).\n[names]\n");
    for (k, id) in map {
        out.push_str(&format!("{} = {}\n", toml::Value::String(k.clone()), toml::Value::String(id.0.clone())));
    }
    out
}

/// Claude Code's permission modes (`--permission-mode`).
pub const PERMISSION_MODES: [&str; 6] = ["default", "acceptEdits", "plan", "auto", "dontAsk", "bypassPermissions"];

/// A mode to set: `Some("")` clears it, an unknown mode refuses.
fn mode_change(plan: &mut Plan, mode: Option<&str>, id: &Id) -> Option<Option<String>> {
    let m = mode?;
    if m.is_empty() {
        return Some(None);
    }
    if !PERMISSION_MODES.contains(&m) {
        refuse(plan, "config.invalid", format!("permission mode {m:?} is not one of {}", PERMISSION_MODES.join(", ")), None, Some(id.clone()));
        return None;
    }
    Some(Some(m.to_string()))
}

/// `nimitz fleet set <F>`: the admiral's permission mode (`Some("")` clears it).
/// It takes effect at the admiral's next start.
pub fn plan_fleet_set(env: &Env, fleet: &str, admiral_mode: Option<&str>) -> Plan {
    let fid = Id::admiral(fleet);
    let mut plan = Plan { summary: format!("update fleet {fleet}"), ..Default::default() };
    let f = match load_fleet_for_write(env, fleet) {
        Ok(f) => f,
        Err(i) => {
            plan.refusals.push(i);
            return plan;
        }
    };
    let mut c = f.config.clone();
    if let Some(m) = mode_change(&mut plan, admiral_mode, &fid) {
        c.admiral.permission_mode = m;
    }
    if plan.ok() && c != f.config {
        plan.steps.push(step_write(Some(fid), &f.dir.join("fleet.toml"), toml_out(&c), "fleet settings"));
    }
    plan
}

/// `nimitz squadron set <F/SQ>`: brief text (into the brief file), `comms`, and
/// the default ship mode (`Some("")` clears it). Ships pick changes up at their
/// next relaunch; the comms rule applies to the next signal.
pub fn plan_squadron_set(env: &Env, id: &Id, brief: Option<&str>, comms: Option<Comms>, mode: Option<&str>) -> Plan {
    let mut plan = Plan { summary: format!("update squadron {id}"), ..Default::default() };
    if id.role() != Role::Commodore {
        refuse(&mut plan, "config.invalid", format!("{id} is not a squadron id (F/SQ)"), None, Some(id.clone()));
        return plan;
    }
    let (fleet, sq) = (id.fleet().to_string(), id.squadron().unwrap_or_default().to_string());
    let f = match load_fleet_for_write(env, &fleet) {
        Ok(f) => f,
        Err(i) => {
            plan.refusals.push(i);
            return plan;
        }
    };
    let Some((dir, s)) = f.squadrons.iter().find(|(_, s)| s.squadron == sq) else {
        refuse(&mut plan, "config.missing", format!("no squadron {id}"), Some(format!("nimitz squadron new {id}")), Some(id.clone()));
        return plan;
    };
    let mut c = s.clone();
    if let Some(cm) = comms {
        if let Comms::Allow { allow } = &cm {
            for a in allow {
                let ok = Id(a.clone()).role() == Role::Ship && Id(a.clone()).parts().iter().all(|p| valid_name(p));
                if !ok {
                    refuse(&mut plan, "config.invalid", format!("comms allow entry {a:?} is not a ship id F/SQ/SHIP"), None, Some(id.clone()));
                }
            }
        }
        c.comms = cm;
    }
    if let Some(m) = mode_change(&mut plan, mode, id) {
        c.permission_mode = m;
    }
    if let Some(b) = brief {
        let file = c.brief.clone().unwrap_or_else(|| "SQUADRON.md".into());
        c.brief = Some(file.clone());
        plan.steps.push(step_write(Some(id.clone()), &dir.join(file), squadron_md(&fleet, &sq, b), "the brief"));
    }
    if &c != s {
        plan.steps.insert(0, step_write(Some(id.clone()), &dir.join("squadron.toml"), toml_out(&c), "squadron settings"));
    }
    if !plan.ok() {
        plan.steps.clear();
    }
    plan
}

/// `nimitz machine rm <F> <label>`: the label leaves `fleet.toml` and every
/// ship's folders and modes. Folders and transcripts on disk stay.
pub fn plan_machine_rm(env: &Env, fleet: &str, label: &str) -> Plan {
    plan_machine_rm_on(env, None, fleet, label)
}

/// `plan_machine_rm`, with the snapshot: removing this machine's label while
/// members of the fleet are live here warns and lists them.
pub fn plan_machine_rm_live(env: &Env, snap: &Snapshot, fleet: &str, label: &str) -> Plan {
    plan_machine_rm_on(env, Some(snap), fleet, label)
}

fn plan_machine_rm_on(env: &Env, snap: Option<&Snapshot>, fleet: &str, label: &str) -> Plan {
    let fid = Id::admiral(fleet);
    let mut plan = Plan { summary: format!("remove machine {label} from {fleet}"), ..Default::default() };
    let f = match load_fleet_for_write(env, fleet) {
        Ok(f) => f,
        Err(i) => {
            plan.refusals.push(i);
            return plan;
        }
    };
    let Some(m) = f.config.machines.get(label) else {
        refuse(&mut plan, "config.missing", format!("machine {label} is not in {fleet}"), None, Some(fid));
        return plan;
    };
    if env.matches_hostname(&m.hostname) {
        let live: Vec<String> = snap
            .and_then(|s| s.fleets.iter().find(|fl| fl.id == fid))
            .map(|fl| {
                let mut v = vec![&fl.admiral];
                for sq in &fl.squadrons {
                    v.push(&sq.commodore);
                    v.extend(sq.ships.iter());
                }
                v.into_iter().filter(|m| m.session.as_ref().is_some_and(|s| s.pid.is_some())).map(|m| m.id.0.clone()).collect()
            })
            .unwrap_or_default();
        let (code, message) = if live.is_empty() {
            ("config.pending", format!("{label} is this machine: {fleet}'s ships stop running here after their next stop"))
        } else {
            (
                "machine.live",
                format!("{label} is this machine and {} member(s) of {fleet} are live here: {}; ships lose their folder here at their next stop", live.len(), live.join(", ")),
            )
        };
        plan.warnings.push(issue(code, Severity::Warn, message, None, Some(fid.clone())));
    }
    let mut c = f.config.clone();
    c.machines.remove(label);
    plan.steps.push(step_write(Some(fid), &f.dir.join("fleet.toml"), toml_out(&c), "fleet settings without the machine"));
    for (dir, s) in &f.squadrons {
        let mut ns = s.clone();
        for ship in &mut ns.ships {
            ship.dirs.remove(label);
            ship.permission_mode.remove(label);
        }
        if &ns != s {
            plan.steps.push(step_write(Some(Id::commodore(fleet, &s.squadron)), &dir.join("squadron.toml"), toml_out(&ns), "ship folders without the machine"));
        }
    }
    plan
}

/// Run a config plan: mkdir / atomic write / archive, then verify by re-reading.
pub fn apply_config_plan(_env: &Env, plan: &Plan) -> Outcome {
    let mut out = Outcome::default();
    if !plan.ok() {
        out.issues = plan.refusals.clone();
        return out;
    }
    for step in &plan.steps {
        let (code, msg) = run_step(step);
        let failed = code != 0;
        out.ran.push((step.clone(), code, msg.clone()));
        if failed {
            out.issues.push(issue("config.write", Severity::Error, format!("step failed: {msg}"), None, step.id.clone()));
            return out; // stop at the first failure; what ran is in `ran`
        }
    }
    // Verify: every write reads back, every mkdir exists, every archive is gone.
    out.verified = plan.steps.iter().all(|s| match s.cmd.first().map(|c| c.as_str()) {
        Some("mkdir") => Path::new(&s.cmd[1]).is_dir(),
        Some("write") => std::fs::read_to_string(&s.cmd[1]).map(|got| got == s.cmd[2]).unwrap_or(false),
        Some("archive") => !Path::new(&s.cmd[1]).exists(),
        _ => false,
    });
    if !out.verified {
        out.issues.push(issue("config.write", Severity::Error, "plan applied but re-read does not match".into(), None, None));
    }
    out
}

fn run_step(step: &Step) -> (i32, String) {
    match (step.cmd.first().map(|c| c.as_str()), step.cmd.get(1)) {
        (Some("mkdir"), Some(dir)) => match std::fs::create_dir_all(dir) {
            Ok(()) => (0, format!("[OK] mkdir {dir}")),
            Err(e) => (1, format!("[ERROR] mkdir {dir}: {e}")),
        },
        (Some("write"), Some(path)) => {
            let Some(contents) = step.cmd.get(2) else {
                return (1, format!("[ERROR] write {path}: no contents in the step"));
            };
            match atomic_write(Path::new(path), contents) {
                Ok(()) => (0, format!("[OK] write {path}")),
                Err(e) => (1, format!("[ERROR] write {path}: {e}")),
            }
        }
        (Some("archive"), Some(path)) => {
            let to = format!("{path}.archived");
            match std::fs::rename(path, &to) {
                Ok(()) => (0, format!("[OK] archive {path} -> {to}")),
                Err(e) => (1, format!("[ERROR] archive {path}: {e}")),
            }
        }
        _ => (1, format!("[ERROR] unknown step {:?}", step.cmd)),
    }
}

/// Temp file in the target's folder, then rename (same device, atomic).
pub(crate) fn atomic_write(path: &Path, contents: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        std::process::id()
    ));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A sandboxed Env rooted in a fresh temp dir; keep the TempDir alive.
    pub fn test_env() -> (tempfile::TempDir, Env) {
        let td = tempfile::tempdir().unwrap();
        let base = td.path().to_path_buf();
        let v: HashMap<String, String> = [
            ("HOME", base.join("home").to_string_lossy().into_owned()),
            ("NIMITZ_HOME", base.join("nimitz").to_string_lossy().into_owned()),
            ("NIMITZ_STATE", base.join("state").to_string_lossy().into_owned()),
            ("NIMITZ_CLAUDE_HOME", base.join("cchome").to_string_lossy().into_owned()),
            ("NIMITZ_CLAUDE_BIN", "/usr/bin/false".into()),
            ("NIMITZ_NATIVE_CLAUDE_BIN", "/usr/bin/false".into()),
            ("NIMITZ_HOSTNAME", "test-mac".into()),
        ]
        .map(|(k, v)| (k.to_string(), v))
        .into();
        (td, Env::from_vars(&v))
    }

    /// Apply a plan and assert it ran clean.
    pub fn apply_ok(env: &Env, plan: &Plan) {
        assert!(plan.ok(), "refused: {:?}", plan.refusals);
        let out = apply_config_plan(env, plan);
        assert!(out.verified, "not verified: {:?}", out.issues);
    }

    /// MAIN fleet with squadron ARGOS and ship ARGOS (managed folder here).
    pub fn make_fleet(env: &Env) {
        apply_ok(env, &plan_fleet_new(env, "MAIN", "test-mac"));
        apply_ok(env, &plan_squadron_new(env, "MAIN", "ARGOS", "Build the asset generator.", Comms::default()));
        let ship = ShipDef {
            name: "ARGOS".into(),
            prompt: Some("You are ARGOS.".into()),
            dirs: [("test-mac".to_string(), "managed".to_string())].into(),
            ..Default::default()
        };
        apply_ok(env, &plan_ship_new(env, "MAIN", "ARGOS", ship));
    }

    #[test]
    fn fleet_new_then_load() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let (fleets, issues) = load_all(&env);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(fleets.len(), 1);
        let f = &fleets[0];
        assert_eq!(f.config.fleet, "MAIN");
        assert_eq!(f.machine.as_deref(), Some("test-mac"));
        assert_eq!(f.squadrons.len(), 1);
        assert_eq!(f.squadrons[0].1.ships.len(), 1);
        assert!(validate(&env, f).is_empty(), "{:?}", validate(&env, f));
        // No CLAUDE.md in <fleet>/, but one in admiral/ and in the squadron folder.
        assert!(!f.dir.join("CLAUDE.md").exists());
        assert!(f.dir.join("admiral/CLAUDE.md").is_file());
        assert!(f.squadrons[0].0.join("CLAUDE.md").is_file());
        assert!(f.squadrons[0].0.join("SQUADRON.md").is_file());
    }

    #[test]
    fn load_errors_are_issues_not_panics() {
        let (_td, env) = test_env();
        let dir = env.fleet_dir("BAD");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fleet.toml"), "fleet = ").unwrap();
        let (fleets, issues) = load_all(&env);
        assert!(fleets.is_empty());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "config.invalid");
        // And an empty NIMITZ_HOME is fine too.
        let (_td2, env2) = test_env();
        let (f2, i2) = load_all(&env2);
        assert!(f2.is_empty() && i2.is_empty());
    }

    #[test]
    fn validate_catches_bad_names_dups_and_labels() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let mut f = load_fleet(&env, "MAIN").unwrap();
        f.config.machines.get_mut("test-mac").unwrap().hostname = String::new();
        f.squadrons[0].1.ships.push(ShipDef { name: "lower".into(), prompt: Some("p".into()), ..Default::default() });
        f.squadrons[0].1.ships.push(ShipDef {
            name: "ARGOS".into(),
            prompt: Some("dup".into()),
            dirs: [("no-such-machine".to_string(), "managed".to_string())].into(),
            ..Default::default()
        });
        f.squadrons[0].1.comms = Comms::Mode("sideways".into());
        let issues = validate(&env, &f);
        assert!(issues.iter().all(|i| i.code == "config.invalid"), "{issues:?}");
        let msgs = format!("{issues:?}");
        for want in ["empty hostname", "not [A-Z0-9_]+", "defined twice", "no-such-machine", "sideways"] {
            assert!(msgs.contains(want), "missing {want:?} in {msgs}");
        }
    }

    #[test]
    fn a_broken_squadron_does_not_hide_the_fleet() {
        let (_td, env) = test_env();
        make_fleet(&env);
        apply_ok(&env, &plan_squadron_new(&env, "MAIN", "BOREAL", "", Comms::default()));
        let bad = env.fleet_dir("MAIN").join("squadrons/argos/squadron.toml");
        std::fs::write(&bad, std::fs::read_to_string(&bad).unwrap() + "\nname = \"oops\n").unwrap();
        let (fleets, issues) = load_all(&env);
        assert_eq!(fleets.len(), 1, "the fleet still loads");
        assert_eq!(fleets[0].squadrons.iter().map(|(_, s)| s.squadron.as_str()).collect::<Vec<_>>(), ["BOREAL"]);
        assert_eq!(issues.len(), 1, "{issues:?}");
        let i = &issues[0];
        assert_eq!((i.code.as_str(), i.id.clone()), ("config.invalid", Some(Id::commodore("MAIN", "ARGOS"))));
        let line = std::fs::read_to_string(&bad).unwrap().lines().count();
        assert!(i.message.contains(&format!("squadron.toml:{line}:")), "file and line: {}", i.message);
        let un = unreadable_squadrons(&fleets[0]);
        assert_eq!(un.len(), 1);
        assert_eq!(un[0].0, Id::commodore("MAIN", "ARGOS"));
        // writes refuse while a squadron file is broken (a rewrite could lose it)
        assert!(!plan_squadron_new(&env, "MAIN", "ARGOS", "", Comms::default()).ok());
        assert!(!plan_machine_add_on(&env, "MAIN", "other", Some("other-host"), vec![]).ok());
        assert!(load_fleet(&env, "MAIN").is_ok());
    }

    #[test]
    fn two_members_in_one_folder_and_symlinks_are_one_folder() {
        let (td, env) = test_env();
        make_fleet(&env);
        let real = td.path().join("proj");
        std::fs::create_dir_all(&real).unwrap();
        let link = td.path().join("proj-link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut f = load_fleet(&env, "MAIN").unwrap();
        for (name, d) in [("A", &real), ("B", &link)] {
            f.squadrons[0].1.ships.push(ShipDef { name: name.into(), prompt: Some("p".into()), dirs: [("test-mac".to_string(), d.to_string_lossy().into_owned())].into(), ..Default::default() });
        }
        let issues = validate(&env, &f);
        assert!(issues.iter().any(|i| i.code == "config.layout" && i.severity == Severity::Error && i.message.contains("share the folder")), "{issues:?}");
        // and plan_ship_new refuses the same folder through a symlink
        let ship = ShipDef { name: "C".into(), prompt: Some("p".into()), dirs: [("test-mac".to_string(), link.to_string_lossy().into_owned())].into(), ..Default::default() };
        apply_ok(&env, &plan_ship_new(&env, "MAIN", "ARGOS", ShipDef { name: "A".into(), ..f.squadrons[0].1.ships[1].clone() }));
        assert!(plan_ship_new(&env, "MAIN", "ARGOS", ship).refusals.iter().any(|i| i.code == "config.layout"));
        // member_dir gives the real path
        assert_eq!(member_dir(&env, &f, &Id::ship("MAIN", "ARGOS", "B")), Some(real.canonicalize().unwrap()));
    }

    #[test]
    fn every_inherited_instructions_file_is_a_layout_error() {
        for rel in ["CLAUDE.md", "squadrons/CLAUDE.md", "ships/CLAUDE.md", ".claude/CLAUDE.md", "ships/argos/CLAUDE.local.md"] {
            let (_td, env) = test_env();
            make_fleet(&env);
            let p = env.fleet_dir("MAIN").join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "x").unwrap();
            let issues = validate(&env, &load_fleet(&env, "MAIN").unwrap());
            assert!(issues.iter().any(|i| i.code == "config.layout" && i.message.contains(&p.display().to_string())), "{rel}: {issues:?}");
        }
        let (_td, env) = test_env();
        make_fleet(&env);
        assert!(home_layout(&env).is_empty());
        std::fs::write(env.home.join("CLAUDE.md"), "x").unwrap();
        assert_eq!(home_layout(&env)[0].code, "config.layout");
    }

    #[test]
    fn ship_dirs_are_managed_absolute_or_home_relative() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let f = load_fleet(&env, "MAIN").unwrap();
        assert_eq!(normalize_ship_dir(&f, "managed").as_deref(), Ok("managed"));
        assert_eq!(normalize_ship_dir(&f, "/w/x/").as_deref(), Ok("/w/x"));
        assert_eq!(normalize_ship_dir(&f, "~/x").as_deref(), Ok("~/x"));
        for bad in ["", "  ", "test-mac=managed", "rel/dir", "./x"] {
            assert!(normalize_ship_dir(&f, bad).is_err(), "{bad:?}");
        }
        assert!(normalize_ship_dir(&f, "test-mac=managed").unwrap_err().contains("--machine test-mac"));
        let ship = ShipDef { name: "S".into(), prompt: Some("p".into()), dirs: [("test-mac".to_string(), "test-mac=managed".to_string())].into(), ..Default::default() };
        let p = plan_ship_new(&env, "MAIN", "ARGOS", ship);
        assert!(p.refusals.iter().any(|i| i.code == "config.invalid") && p.steps.is_empty(), "{:?}", p.refusals);
    }

    #[test]
    fn reserved_ship_names_are_refused() {
        let (_td, env) = test_env();
        make_fleet(&env);
        for name in ["COMMODORE", "ADMIRAL"] {
            let ship = ShipDef { name: name.into(), prompt: Some("p".into()), dirs: [("test-mac".to_string(), "managed".to_string())].into(), ..Default::default() };
            assert!(!plan_ship_new(&env, "MAIN", "ARGOS", ship.clone()).ok(), "{name}");
            assert!(!plan_ship_set(&env, &Id::ship("MAIN", "ARGOS", name), ship.clone()).ok(), "{name}");
            let mut f = load_fleet(&env, "MAIN").unwrap();
            f.squadrons[0].1.ships.push(ship);
            assert!(validate(&env, &f).iter().any(|i| i.message.contains("reserved")), "{name}");
        }
        assert!(!reserved_ship_name("COMMODORES"));
    }

    #[test]
    fn validate_warns_on_missing_prompt() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let mut f = load_fleet(&env, "MAIN").unwrap();
        f.squadrons[0].1.ships[0].prompt = None;
        let issues = validate(&env, &f);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].severity, Severity::Warn);
        assert!(issues[0].message.contains("no prompt"));
    }

    #[test]
    fn validate_layout_rules() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let f = load_fleet(&env, "MAIN").unwrap();
        std::fs::write(f.dir.join("CLAUDE.md"), "x").unwrap();
        let issues = validate(&env, &f);
        assert!(issues.iter().any(|i| i.code == "config.layout"), "{issues:?}");
        std::fs::remove_file(f.dir.join("CLAUDE.md")).unwrap();
        // A ship folder inside the admiral's folder.
        let mut f2 = load_fleet(&env, "MAIN").unwrap();
        f2.squadrons[0].1.ships[0]
            .dirs
            .insert("test-mac".into(), f2.dir.join("admiral/sub").to_string_lossy().into_owned());
        let issues = validate(&env, &f2);
        assert!(issues.iter().any(|i| i.code == "config.layout" && i.severity == Severity::Error), "{issues:?}");
    }

    #[test]
    fn a_home_folder_ship_may_hold_other_ships_folders() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let home = env.home.parent().unwrap().join("home");
        let proj = home.join("Projects/thing");
        std::fs::create_dir_all(&proj).unwrap();
        for (name, dir) in [("HOME", &home), ("THING", &proj)] {
            let ship = ShipDef { name: name.into(), prompt: Some("p".into()), dirs: [("test-mac".to_string(), dir.to_string_lossy().into_owned())].into(), ..Default::default() };
            let plan = plan_ship_new(&env, "MAIN", "ARGOS", ship);
            assert!(plan.ok(), "{name}: {:?}", plan.refusals);
            assert!(plan.warnings.iter().all(|i| i.code != "config.layout"), "{:?}", plan.warnings);
            apply_ok(&env, &plan);
        }
        let layout = |env: &Env| -> Vec<Issue> { validate(env, &load_fleet(env, "MAIN").unwrap()).into_iter().filter(|i| i.code == "config.layout").collect() };
        assert!(layout(&env).is_empty(), "a home-folder ship is fine: {:?}", layout(&env));
        // a CLAUDE.md in the outer project folder: a warning, never a refusal
        std::fs::write(home.join("CLAUDE.md"), "mine").unwrap();
        let issues = layout(&env);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].severity, Severity::Warn);
        assert_eq!(issues[0].id, Some(Id::ship("MAIN", "ARGOS", "THING")));
    }

    #[test]
    fn member_dir_resolves_each_role() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let f = load_fleet(&env, "MAIN").unwrap();
        assert_eq!(member_dir(&env, &f, &Id::admiral("MAIN")), Some(f.dir.join("admiral")));
        assert_eq!(member_dir(&env, &f, &Id::commodore("MAIN", "ARGOS")), Some(f.dir.join("squadrons/argos")));
        assert_eq!(member_dir(&env, &f, &Id::ship("MAIN", "ARGOS", "ARGOS")), Some(f.dir.join("ships/argos/argos")));
        assert_eq!(member_dir(&env, &f, &Id::ship("MAIN", "ARGOS", "GHOST")), None);
        assert_eq!(member_dir(&env, &f, &Id::ship("OTHER", "ARGOS", "ARGOS")), None);
        // A ship with no dir for this machine does not run here.
        let mut f2 = f.clone();
        f2.squadrons[0].1.ships[0].dirs.clear();
        assert_eq!(member_dir(&env, &f2, &Id::ship("MAIN", "ARGOS", "ARGOS")), None);
        // Explicit folder.
        let mut f3 = f.clone();
        f3.squadrons[0].1.ships[0].dirs.insert("test-mac".into(), "/work/argos".into());
        assert_eq!(member_dir(&env, &f3, &Id::ship("MAIN", "ARGOS", "ARGOS")), Some(PathBuf::from("/work/argos")));
    }

    #[test]
    fn ship_prompt_inline_and_file() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let f = load_fleet(&env, "MAIN").unwrap();
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        assert_eq!(ship_prompt(&f, &id), Some("You are ARGOS.".into()));
        let mut f2 = f.clone();
        f2.squadrons[0].1.ships[0].prompt = None;
        f2.squadrons[0].1.ships[0].prompt_file = Some("prompts/argos.md".into());
        assert_eq!(ship_prompt(&f2, &id), None); // file missing
        std::fs::create_dir_all(f.squadrons[0].0.join("prompts")).unwrap();
        std::fs::write(f.squadrons[0].0.join("prompts/argos.md"), "From file.").unwrap();
        assert_eq!(ship_prompt(&f2, &id), Some("From file.".into()));
    }

    #[test]
    fn permission_mode_order() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let mut f = load_fleet(&env, "MAIN").unwrap();
        let ship = Id::ship("MAIN", "ARGOS", "ARGOS");
        assert_eq!(permission_mode(&f, &ship), "");
        f.squadrons[0].1.permission_mode = Some("bypassPermissions".into());
        assert_eq!(permission_mode(&f, &ship), "bypassPermissions");
        assert_eq!(permission_mode(&f, &Id::commodore("MAIN", "ARGOS")), "bypassPermissions");
        f.squadrons[0].1.ships[0].permission_mode.insert("test-mac".into(), "plan".into());
        assert_eq!(permission_mode(&f, &ship), "plan");
        assert_eq!(permission_mode(&f, &Id::admiral("MAIN")), "");
        f.config.admiral.permission_mode = Some("auto".into());
        assert_eq!(permission_mode(&f, &Id::admiral("MAIN")), "auto");
    }

    #[test]
    fn crud_refusals() {
        let (_td, env) = test_env();
        make_fleet(&env);
        assert!(!plan_fleet_new(&env, "MAIN", "test-mac").ok()); // exists
        assert!(!plan_fleet_new(&env, "bad name", "test-mac").ok());
        assert!(!plan_squadron_new(&env, "MAIN", "ARGOS", "", Comms::default()).ok()); // exists
        assert!(!plan_squadron_new(&env, "NOPE", "X", "", Comms::default()).ok()); // no fleet
        let dup = ShipDef { name: "ARGOS".into(), prompt: Some("p".into()), ..Default::default() };
        assert!(!plan_ship_new(&env, "MAIN", "ARGOS", dup).ok());
        let nested = ShipDef {
            name: "NESTED".into(),
            prompt: Some("p".into()),
            dirs: [(
                "test-mac".to_string(),
                env.fleet_dir("MAIN").join("admiral/x").to_string_lossy().into_owned(),
            )]
            .into(),
            ..Default::default()
        };
        let p = plan_ship_new(&env, "MAIN", "ARGOS", nested);
        assert!(p.refusals.iter().any(|i| i.code == "config.layout"), "{:?}", p.refusals);
        assert!(!plan_ship_set(
            &env,
            &Id::ship("MAIN", "ARGOS", "GHOST"),
            ShipDef { name: "GHOST".into(), ..Default::default() }
        )
        .ok());
        // A refused plan applies nothing.
        let out = apply_config_plan(&env, &p);
        assert!(out.ran.is_empty() && !out.verified && !out.issues.is_empty());
    }

    #[test]
    fn ship_set_and_rm_rewrite_the_squadron() {
        let (_td, env) = test_env();
        make_fleet(&env);
        let id = Id::ship("MAIN", "ARGOS", "ARGOS");
        let mut def = load_fleet(&env, "MAIN").unwrap().squadrons[0].1.ships[0].clone();
        def.prompt = Some("New prompt.".into());
        apply_ok(&env, &plan_ship_set(&env, &id, def));
        let f = load_fleet(&env, "MAIN").unwrap();
        assert_eq!(ship_prompt(&f, &id), Some("New prompt.".into()));
        apply_ok(&env, &plan_rm(&env, &id));
        let f = load_fleet(&env, "MAIN").unwrap();
        assert!(f.squadrons[0].1.ships.is_empty());
        assert!(validate(&env, &f).is_empty());
    }

    #[test]
    fn rm_order_is_enforced() {
        let (_td, env) = test_env();
        make_fleet(&env);
        // Fleet and squadron refuse while children exist.
        assert!(!plan_rm(&env, &Id::admiral("MAIN")).ok());
        assert!(!plan_rm(&env, &Id::commodore("MAIN", "ARGOS")).ok());
        apply_ok(&env, &plan_rm(&env, &Id::ship("MAIN", "ARGOS", "ARGOS")));
        apply_ok(&env, &plan_rm(&env, &Id::commodore("MAIN", "ARGOS")));
        apply_ok(&env, &plan_rm(&env, &Id::admiral("MAIN")));
        let (fleets, issues) = load_all(&env);
        assert!(fleets.is_empty() && issues.is_empty());
        // Archived, not deleted.
        assert!(env.fleet_dir("MAIN").join("fleet.toml.archived").is_file());
    }

    #[test]
    fn machine_add_updates_fleet_and_ships() {
        let (_td, env) = test_env();
        make_fleet(&env);
        // no --hostname: this host already is test-mac, so a second label refuses
        let p = plan_machine_add(&env, "MAIN", "home-server", vec![]);
        assert!(p.refusals.iter().any(|i| i.code == "machine.host" && i.message.contains("test-mac")), "{:?}", p.refusals);
        assert!(!plan_machine_add_on(&env, "MAIN", "home-server", Some("TEST-MAC.local"), vec![]).ok());
        let plan = plan_machine_add_on(&env, "MAIN", "home-server", Some("u0box"), vec![(Id::ship("MAIN", "ARGOS", "ARGOS"), "managed".into())]);
        apply_ok(&env, &plan);
        let f = load_fleet(&env, "MAIN").unwrap();
        assert_eq!(f.config.machines["home-server"].hostname, "u0box");
        assert_eq!(f.squadrons[0].1.ships[0].dirs.get("home-server").map(|s| s.as_str()), Some("managed"));
        assert!(validate(&env, &f).is_empty());
        assert!(!plan_machine_add(&env, "MAIN", "home-server", vec![]).ok()); // dup label
        assert!(!plan_machine_add(&env, "MAIN", "other", vec![(Id::ship("MAIN", "ARGOS", "GHOST"), "managed".into())]).ok());
    }

    #[test]
    fn fleet_set_squadron_set_machine_rm() {
        let (_td, env) = test_env();
        make_fleet(&env);
        // fleet set: the admiral's mode; "" clears; an unknown mode refuses; no change, no step
        apply_ok(&env, &plan_fleet_set(&env, "MAIN", Some("auto")));
        assert_eq!(load_fleet(&env, "MAIN").unwrap().config.admiral.permission_mode.as_deref(), Some("auto"));
        assert!(plan_fleet_set(&env, "MAIN", Some("auto")).steps.is_empty());
        assert!(!plan_fleet_set(&env, "MAIN", Some("yolo")).ok());
        apply_ok(&env, &plan_fleet_set(&env, "MAIN", Some("")));
        assert_eq!(load_fleet(&env, "MAIN").unwrap().config.admiral.permission_mode, None);
        assert!(!plan_fleet_set(&env, "NOPE", Some("auto")).ok());

        // squadron set: comms, mode, brief
        let sq = Id::commodore("MAIN", "ARGOS");
        let plan = plan_squadron_set(&env, &sq, Some("New orders."), Some(Comms::Mode("commodore-only".into())), Some("plan"));
        apply_ok(&env, &plan);
        let f = load_fleet(&env, "MAIN").unwrap();
        let (dir, s) = &f.squadrons[0];
        assert_eq!(s.comms, Comms::Mode("commodore-only".into()));
        assert_eq!(s.permission_mode.as_deref(), Some("plan"));
        assert!(std::fs::read_to_string(dir.join(s.brief.as_deref().unwrap())).unwrap().contains("New orders."));
        assert!(validate(&env, &f).is_empty());
        let bad = plan_squadron_set(&env, &sq, None, Some(Comms::Allow { allow: vec!["not a ship".into()] }), None);
        assert!(!bad.ok() && bad.steps.is_empty());
        assert!(!plan_squadron_set(&env, &Id::commodore("MAIN", "GHOST"), None, None, Some("plan")).ok());
        assert!(!plan_squadron_set(&env, &Id::admiral("MAIN"), None, None, None).ok());

        // machine rm: the label leaves fleet.toml and every ship's dirs; this machine warns
        apply_ok(&env, &plan_machine_add_on(&env, "MAIN", "home-server", Some("u0box"), vec![(Id::ship("MAIN", "ARGOS", "ARGOS"), "managed".into())]));
        let plan = plan_machine_rm(&env, "MAIN", "home-server");
        assert!(plan.warnings.is_empty(), "another machine: {:?}", plan.warnings);
        apply_ok(&env, &plan);
        let f = load_fleet(&env, "MAIN").unwrap();
        assert!(!f.config.machines.contains_key("home-server"));
        assert!(f.squadrons[0].1.ships[0].dirs.keys().all(|k| k != "home-server"));
        assert!(!plan_machine_rm(&env, "MAIN", "home-server").ok());
        assert!(plan_machine_rm(&env, "MAIN", "test-mac").warnings.iter().any(|i| i.code == "config.pending"));
        // with live members here, the warning names them
        let mut snap = crate::check::tests::snap_for(&env, None);
        snap.fleets[0].admiral.session = Some(crate::check::tests::live_session("[MAIN]-ADMIRAL"));
        let w = plan_machine_rm_live(&env, &snap, "MAIN", "test-mac").warnings;
        assert!(w.iter().any(|i| i.code == "machine.live" && i.message.contains("MAIN")), "{w:?}");
    }
}
