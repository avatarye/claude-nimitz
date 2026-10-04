//! Import fleets made by the old fleet skill (FLEET_CONFIG.json) as squadrons of a nimitz fleet. Owner: core-config.
//!
//! Old shape (JSON): { fleet, instructions: "FLEET.md", machines: {label: {hostname, os, model?}},
//! agents: [{name, prompt, dirs: {label: path}, permission_mode: {}, bg_isolation: {}, depends_on: []}] }.
//! Each old fleet becomes one squadron of the target fleet; its agents become ships;
//! its machines merge into fleet.toml. The old dirs are read-only; every judgement
//! call lands in the report.
use crate::config::{self, step_mkdir, step_write, toml_out, valid_name};
use crate::env::Env;
use crate::types::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|x| x.to_string()).filter(|x| !x.is_empty())
}

/// Upcase and squeeze into [A-Z0-9_]+; says so in the report when it had to change.
fn upname(raw: &str, what: &str, report: &mut Vec<String>) -> String {
    let up = raw.to_uppercase();
    let clean: String = up.chars().map(|c| if c.is_ascii_uppercase() || c.is_ascii_digit() { c } else { '_' }).collect();
    if clean != raw {
        report.push(format!("[WARN] {what} {raw:?} renamed to {clean:?} (names are [A-Z0-9_]+, uppercase)"));
    }
    clean
}

struct Merge {
    machines: BTreeMap<String, MachineDef>,
    /// hostname -> the label every squadron must use for it
    by_hostname: BTreeMap<String, String>,
}

impl Merge {
    /// Merge one old fleet's machine (label, hostname, os); returns the label to use.
    fn add(&mut self, old_fleet: &str, label: &str, hostname: &str, os: Option<String>, report: &mut Vec<String>) -> String {
        let host = hostname.to_lowercase();
        if let Some(canon) = self.by_hostname.get(&host) {
            if canon != label {
                report.push(format!(
                    "[WARN] {old_fleet}: machine {label:?} is the same host as {canon:?} ({host}); its ships use {canon:?}"
                ));
            }
            return canon.clone();
        }
        if let Some(taken) = self.machines.get(label) {
            // Same label, different host: the second fleet's label gets a suffix.
            let renamed = format!("{label}-{}", old_fleet.to_lowercase());
            report.push(format!(
                "[WARN] {old_fleet}: label {label:?} already names host {} ; this fleet's {host} becomes {renamed:?}",
                taken.hostname
            ));
            self.machines.insert(renamed.clone(), MachineDef { hostname: host.clone(), os });
            self.by_hostname.insert(host, renamed.clone());
            return renamed;
        }
        self.machines.insert(label.to_string(), MachineDef { hostname: host.clone(), os });
        self.by_hostname.insert(host, label.to_string());
        label.to_string()
    }
}

/// One old agent -> one ShipDef, with every mapping decision reported.
fn ship_from_agent(env: &Env, old_fleet: &str, a: &Value, merge: &Merge, label_map: &BTreeMap<String, String>, report: &mut Vec<String>) -> Option<ShipDef> {
    let raw_name = s(a, "name")?;
    let name = upname(&raw_name, &format!("{old_fleet}: agent"), report);
    let prompt = s(a, "prompt");
    if prompt.is_none() {
        report.push(format!("[WARN] {old_fleet}/{name}: no prompt in the old config; a cold launch will refuse until one is set"));
    }
    let mut dirs = BTreeMap::new();
    if let Some(d) = a.get("dirs").and_then(|d| d.as_object()) {
        for (label, path) in d {
            let Some(path) = path.as_str().filter(|p| !p.is_empty()) else { continue };
            let label = label_map.get(label).cloned().unwrap_or_else(|| label.clone());
            // Only this machine's folders can be checked here.
            let on_this_machine = merge.machines.get(&label).map(|m| env.matches_hostname(&m.hostname)).unwrap_or(false);
            if on_this_machine && !Path::new(path).is_dir() {
                report.push(format!("[WARN] {old_fleet}/{name}: dir {path} ({label}) does not exist on this machine"));
            }
            dirs.insert(label, path.to_string());
        }
    }
    let mut permission_mode = BTreeMap::new();
    match a.get("permission_mode") {
        Some(Value::Object(m)) => {
            for (label, mode) in m {
                if let Some(mode) = mode.as_str() {
                    let label = label_map.get(label).cloned().unwrap_or_else(|| label.clone());
                    permission_mode.insert(label, mode.to_string());
                }
            }
        }
        Some(Value::String(mode)) if !mode.is_empty() => {
            // One mode for every machine the ship runs on.
            for label in dirs.keys() {
                permission_mode.insert(label.clone(), mode.clone());
            }
            report.push(format!("[OK] {old_fleet}/{name}: single permission_mode {mode:?} applied to every machine"));
        }
        _ => {}
    }
    match a.get("bg_isolation") {
        None | Some(Value::Null) => {}
        Some(Value::Object(m)) if m.is_empty() => {}
        Some(other) => report.push(format!(
            "[WARN] {old_fleet}/{name}: bg_isolation {other} dropped; nimitz always launches with bgIsolation none"
        )),
    }
    let depends_on = a
        .get("depends_on")
        .and_then(|d| d.as_array())
        .map(|d| d.iter().filter_map(|x| x.as_str()).map(|x| x.to_uppercase()).collect())
        .unwrap_or_default();
    Some(ShipDef { name, prompt, prompt_file: None, depends_on, dirs, permission_mode })
}

/// Read each old fleet dir and plan the new layout under NIMITZ_HOME/<into>/.
/// Never touches the old dirs. Returns a Plan of file writes and a report.
pub fn plan_from_fleet_skill(env: &Env, old_dirs: &[PathBuf], into: &str) -> (Plan, Vec<String>) {
    let mut plan = Plan { summary: format!("import {} fleet-skill fleet(s) into {into}", old_dirs.len()), ..Default::default() };
    let mut report = vec![];
    let refuse = |plan: &mut Plan, msg: String| {
        plan.refusals.push(Issue { code: "config.invalid".into(), severity: Severity::Error, message: msg, fix: None, id: None })
    };
    if !valid_name(into) {
        refuse(&mut plan, format!("target fleet name {into:?} is not [A-Z0-9_]+"));
        return (plan, report);
    }
    let fid = Id::admiral(into);
    let fleet_dir = env.fleet_dir(into);
    let existing = if fleet_dir.join("fleet.toml").is_file() {
        match config::load_fleet(env, into) {
            Ok(f) => Some(f),
            Err(i) => {
                plan.refusals.push(i);
                return (plan, report);
            }
        }
    } else {
        None
    };
    let mut merge = Merge {
        machines: existing.as_ref().map(|f| f.config.machines.clone()).unwrap_or_default(),
        by_hostname: Default::default(),
    };
    for (label, m) in &merge.machines.clone() {
        merge.by_hostname.insert(m.hostname.to_lowercase(), label.clone());
    }
    let mut taken: Vec<String> = existing
        .as_ref()
        .map(|f| f.squadrons.iter().map(|(_, s)| s.squadron.clone()).collect())
        .unwrap_or_default();
    let mut squads: Vec<(SquadronConfig, Option<String>)> = vec![]; // config + FLEET.md body

    for dir in old_dirs {
        let cfg_path = dir.join("FLEET_CONFIG.json");
        let v: Value = match std::fs::read_to_string(&cfg_path).map_err(|e| e.to_string()).and_then(|r| serde_json::from_str(&r).map_err(|e| e.to_string())) {
            Ok(v) => v,
            Err(e) => {
                refuse(&mut plan, format!("{}: {e}", cfg_path.display()));
                continue;
            }
        };
        let Some(old_name) = s(&v, "fleet") else {
            refuse(&mut plan, format!("{}: no \"fleet\" field", cfg_path.display()));
            continue;
        };
        let sq_name = upname(&old_name, "old fleet", &mut report);
        if taken.contains(&sq_name) {
            refuse(&mut plan, format!("squadron {sq_name} already exists in {into} (old fleet {})", dir.display()));
            continue;
        }
        taken.push(sq_name.clone());

        // Machines: merge, remembering what each old label is called now.
        let mut label_map = BTreeMap::new();
        let mut lists_this_machine = false;
        if let Some(ms) = v.get("machines").and_then(|m| m.as_object()) {
            for (label, m) in ms {
                let Some(hostname) = s(m, "hostname") else {
                    report.push(format!("[WARN] {old_name}: machine {label:?} has no hostname; skipped"));
                    continue;
                };
                lists_this_machine |= env.matches_hostname(&hostname);
                let canon = merge.add(&old_name, label, &hostname, s(m, "os"), &mut report);
                label_map.insert(label.clone(), canon);
            }
        }
        if !lists_this_machine {
            report.push(format!("[OK] {old_name}: does not list this machine; migrated anyway (other machines need it)"));
        }

        // Agents -> ships; an agent named LEAD is the old interactive lead, whose
        // place the squadron's commodore takes.
        let mut ships: Vec<ShipDef> = vec![];
        for a in v.get("agents").and_then(|a| a.as_array()).map(|a| a.as_slice()).unwrap_or_default() {
            let Some(ship) = ship_from_agent(env, &old_name, a, &merge, &label_map, &mut report) else {
                report.push(format!("[WARN] {old_name}: an agent with no name was skipped"));
                continue;
            };
            if ship.name == "LEAD" {
                report.push(format!("[OK] {old_name}: agent LEAD is replaced by [{into}.{sq_name}]-COMMODORE, not migrated as a ship"));
                continue;
            }
            if ships.iter().any(|x| x.name == ship.name) {
                report.push(format!("[WARN] {old_name}: duplicate agent {:?} skipped", ship.name));
                continue;
            }
            ships.push(ship);
        }
        for ship in &ships {
            for d in &ship.depends_on {
                if !ships.iter().any(|x| &x.name == d) {
                    report.push(format!("[WARN] {old_name}/{}: depends_on {d:?} names no migrated ship", ship.name));
                }
            }
        }

        // FLEET.md (or whatever "instructions" names) becomes SQUADRON.md.
        let brief_file = s(&v, "instructions").unwrap_or("FLEET.md".into());
        let fleet_md = std::fs::read_to_string(dir.join(&brief_file)).ok();
        if fleet_md.is_none() {
            report.push(format!("[WARN] {old_name}: no {brief_file} to copy; SQUADRON.md starts from the template"));
        }
        report.push(format!("[OK] {old_name}: becomes squadron {into}/{sq_name} with {} ship(s)", ships.len()));
        squads.push((
            SquadronConfig {
                squadron: sq_name,
                brief: Some("SQUADRON.md".into()),
                comms: Comms::default(),
                permission_mode: None,
                ships,
            },
            fleet_md,
        ));
    }
    if !plan.ok() {
        return (plan, report);
    }

    // Steps: the fleet shell first (when new), then fleet.toml, then each squadron.
    if existing.is_none() {
        plan.steps.push(step_mkdir(Some(fid.clone()), &fleet_dir, "fleet folder"));
        plan.steps.push(step_mkdir(Some(fid.clone()), &fleet_dir.join("admiral"), "admiral's folder"));
        plan.steps.push(step_write(
            Some(fid.clone()),
            &fleet_dir.join("admiral/CLAUDE.md"),
            config::admiral_claude_md(into),
            "admiral brief",
        ));
    }
    let fleet_config = FleetConfig {
        fleet: into.into(),
        machines: merge.machines,
        admiral: existing.as_ref().map(|f| f.config.admiral.clone()).unwrap_or_default(),
    };
    plan.steps.push(step_write(Some(fid.clone()), &fleet_dir.join("fleet.toml"), toml_out(&fleet_config), "merged machine labels"));
    for (sq, fleet_md) in squads {
        let sqid = Id::commodore(into, &sq.squadron);
        let sd = fleet_dir.join("squadrons").join(sq.squadron.to_lowercase());
        let brief = fleet_md.unwrap_or_else(|| config::squadron_md(into, &sq.squadron, ""));
        plan.steps.push(step_mkdir(Some(sqid.clone()), &sd, "commodore's folder"));
        plan.steps.push(step_write(Some(sqid.clone()), &sd.join("squadron.toml"), toml_out(&sq), "squadron settings"));
        plan.steps.push(step_write(Some(sqid.clone()), &sd.join("SQUADRON.md"), brief, "the old FLEET.md as the brief"));
        plan.steps.push(step_write(Some(sqid.clone()), &sd.join("CLAUDE.md"), config::commodore_claude_md(into, &sq.squadron), "commodore brief"));
    }
    (plan, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::{apply_ok, test_env};

    fn old_fleet(dir: &Path, json: &str, fleet_md: Option<&str>) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("FLEET_CONFIG.json"), json).unwrap();
        if let Some(md) = fleet_md {
            std::fs::write(dir.join("FLEET.md"), md).unwrap();
        }
    }

    const BOLT: &str = r#"{
      "fleet": "BOLT", "instructions": "FLEET.md",
      "machines": {
        "west-mac": {"hostname": "test-mac", "os": "darwin", "model": "Mac15,6"},
        "ubuntu-box": {"hostname": "box01.corp.example.com", "os": "linux"}
      },
      "agents": [
        {"name": "BOLT", "prompt": "You are BOLT.", "dirs": {"west-mac": "/ws/Acme"},
         "permission_mode": {}, "bg_isolation": {}, "depends_on": []},
        {"name": "BOOM", "prompt": "You are BOOM.",
         "dirs": {"west-mac": "/ws/AcmeOrchestrator", "ubuntu-box": "/home/x/AcmeOrchestrator"},
         "permission_mode": {"west-mac": "acceptEdits"}, "bg_isolation": {}, "depends_on": ["bolt"]}
      ]
    }"#;

    // Same host as west-mac under a different label, plus a lead and oddities.
    const BRIM: &str = r#"{
      "fleet": "BRIM",
      "machines": {"macbook": {"hostname": "TEST-MAC", "os": "darwin"}},
      "agents": [
        {"name": "LEAD", "prompt": "You are the lead."},
        {"name": "brim", "dirs": {"macbook": "/ws/Brim"}, "bg_isolation": {"mode": "worktree"},
         "permission_mode": "bypassPermissions"}
      ]
    }"#;

    #[test]
    fn migrates_two_old_fleets_into_one() {
        let (td, env) = test_env();
        let a = td.path().join("old/bolt-fleet");
        let b = td.path().join("old/brim-fleet");
        old_fleet(&a, BOLT, Some("# BOLT fleet\nThe brief.\n"));
        old_fleet(&b, BRIM, None);
        let (plan, report) = plan_from_fleet_skill(&env, &[a, b], "MAIN");
        apply_ok(&env, &plan);
        let f = config::load_fleet(&env, "MAIN").unwrap();
        // Machines merged; the duplicate hostname keeps the first label.
        assert_eq!(f.config.machines.len(), 2, "{:?}", f.config.machines);
        assert_eq!(f.config.machines["west-mac"].hostname, "test-mac");
        assert!(f.config.machines.contains_key("ubuntu-box"));
        assert_eq!(f.machine.as_deref(), Some("west-mac"));
        // Two squadrons, ships upcased, labels remapped, LEAD dropped.
        assert_eq!(f.squadrons.len(), 2);
        let bolt = &f.squadrons.iter().find(|(_, s)| s.squadron == "BOLT").unwrap().1;
        let brim = &f.squadrons.iter().find(|(_, s)| s.squadron == "BRIM").unwrap().1;
        assert_eq!(bolt.ships.len(), 2);
        assert_eq!(bolt.ships[1].depends_on, ["BOLT"]);
        assert_eq!(bolt.ships[1].permission_mode["west-mac"], "acceptEdits");
        assert_eq!(brim.ships.len(), 1);
        assert_eq!(brim.ships[0].name, "BRIM");
        assert_eq!(brim.ships[0].dirs.get("west-mac").map(|s| s.as_str()), Some("/ws/Brim"));
        assert_eq!(brim.ships[0].permission_mode["west-mac"], "bypassPermissions");
        // FLEET.md copied; the other squadron gets the template.
        let bolt_dir = &f.squadrons.iter().find(|(_, s)| s.squadron == "BOLT").unwrap().0;
        assert!(std::fs::read_to_string(bolt_dir.join("SQUADRON.md")).unwrap().contains("The brief."));
        // The merged config validates, modulo the missing-prompt warning and the
        // old dirs that do not exist in this sandbox.
        let issues = config::validate(&env, &f);
        assert!(issues.iter().all(|i| i.severity == Severity::Warn), "{issues:?}");
        // Report calls out the decisions.
        let r = report.join("\n");
        for want in ["macbook", "LEAD", "bg_isolation", "no prompt", "does not exist"] {
            assert!(r.contains(want), "missing {want:?} in report:\n{r}");
        }
    }

    #[test]
    fn fleet_without_this_machine_still_migrates() {
        let (td, env) = test_env();
        let a = td.path().join("old/remote-fleet");
        old_fleet(
            &a,
            r#"{"fleet": "GX", "machines": {"home": {"hostname": "elsewhere"}},
                "agents": [{"name": "GX", "prompt": "p", "dirs": {"home": "/x"}}]}"#,
            None,
        );
        let (plan, report) = plan_from_fleet_skill(&env, &[a], "MAIN");
        apply_ok(&env, &plan);
        let f = config::load_fleet(&env, "MAIN").unwrap();
        assert_eq!(f.machine, None);
        assert_eq!(f.squadrons[0].1.ships.len(), 1);
        assert!(report.join("\n").contains("does not list this machine"));
    }

    #[test]
    fn second_import_merges_into_the_existing_fleet() {
        let (td, env) = test_env();
        let a = td.path().join("old/bolt-fleet");
        old_fleet(&a, BOLT, None);
        let (plan, _) = plan_from_fleet_skill(&env, &[a.clone()], "MAIN");
        apply_ok(&env, &plan);
        // The same squadron again: refused, nothing applied.
        let (plan2, _) = plan_from_fleet_skill(&env, &[a], "MAIN");
        assert!(!plan2.ok());
        // A different one: merged alongside.
        let b = td.path().join("old/brim-fleet");
        old_fleet(&b, BRIM, None);
        let (plan3, _) = plan_from_fleet_skill(&env, &[b], "MAIN");
        apply_ok(&env, &plan3);
        let f = config::load_fleet(&env, "MAIN").unwrap();
        assert_eq!(f.squadrons.len(), 2);
        assert!(f.config.machines.contains_key("ubuntu-box")); // kept from the first import
    }

    #[test]
    fn bad_input_refuses_without_panicking() {
        let (td, env) = test_env();
        let (plan, _) = plan_from_fleet_skill(&env, &[td.path().join("nope")], "MAIN");
        assert!(!plan.ok());
        let a = td.path().join("old/bad-fleet");
        old_fleet(&a, "{not json", None);
        let (plan, _) = plan_from_fleet_skill(&env, &[a.clone()], "MAIN");
        assert!(!plan.ok());
        old_fleet(&a, r#"{"machines": {}}"#, None);
        let (plan, _) = plan_from_fleet_skill(&env, &[a], "MAIN");
        assert!(!plan.ok());
        let (plan, _) = plan_from_fleet_skill(&env, &[], "bad name");
        assert!(!plan.ok());
    }

    /// Dry-run against copies of the real old fleets, when they exist on this
    /// machine. Prints the migration report; run with `--nocapture` to read it.
    #[test]
    fn report_for_the_real_old_fleets() {
        let real = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join("Dropbox/claude-orchestrators");
        if !real.is_dir() {
            return; // not this machine; nothing to dry-run
        }
        let (td, env) = test_env();
        let mut copies = vec![];
        let mut names: Vec<_> = std::fs::read_dir(&real)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.join("FLEET_CONFIG.json").is_file())
            .collect();
        names.sort();
        for src in names {
            let dst = td.path().join("old").join(src.file_name().unwrap());
            std::fs::create_dir_all(&dst).unwrap();
            std::fs::copy(src.join("FLEET_CONFIG.json"), dst.join("FLEET_CONFIG.json")).unwrap();
            if src.join("FLEET.md").is_file() {
                std::fs::copy(src.join("FLEET.md"), dst.join("FLEET.md")).unwrap();
            }
            copies.push(dst);
        }
        let (plan, report) = plan_from_fleet_skill(&env, &copies, "MAIN");
        println!("== migration report for {} old fleet(s) ==", copies.len());
        for line in &report {
            println!("{line}");
        }
        for r in &plan.refusals {
            println!("[REFUSED] {}", r.message);
        }
        apply_ok(&env, &plan);
        let f = config::load_fleet(&env, "MAIN").unwrap();
        assert_eq!(f.squadrons.len(), copies.len());
        let issues = config::validate(&env, &f);
        assert!(issues.iter().all(|i| i.severity == Severity::Warn), "{issues:?}");
    }
}
