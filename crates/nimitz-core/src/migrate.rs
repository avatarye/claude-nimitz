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
        // as every config writer: never over a squadron.toml or legacy.toml
        // that does not read here (a newer nimitz's, or broken)
        match config::load_fleet_for_write(env, into) {
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
    // Within-run duplicates refuse; a squadron already on disk is compared
    // instead (identical -> no steps, different -> refuse), so a re-import of
    // the same old fleets plans nothing the second time.
    let mut taken: Vec<String> = vec![];
    let mut squads: Vec<(SquadronConfig, Option<String>)> = vec![]; // config + FLEET.md body
    // old session names -> identities, so the old fleet's running sessions are recognised
    let mut legacy: BTreeMap<String, Id> = existing.as_ref().map(config::legacy_map).unwrap_or_default();

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
            refuse(&mut plan, format!("two old fleets both become squadron {sq_name} (second: {})", dir.display()));
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
        // the old lead ran as `[OLD]-LEAD` (started with `ccl`), listed as an agent or not
        legacy.insert(format!("[{old_name}]-LEAD"), Id::commodore(into, &sq_name));
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
            let raw = s(a, "name").unwrap_or_default();
            legacy.insert(format!("[{old_name}]-{raw}"), Id::ship(into, &sq_name, &ship.name));
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
        let sq = SquadronConfig {
            squadron: sq_name.clone(),
            brief: Some("SQUADRON.md".into()),
            comms: Comms::default(),
            permission_mode: None,
            ships,
            ..Default::default()
        };
        // Already migrated? Identical means nothing to plan; different refuses.
        if let Some(prev) = existing.as_ref().and_then(|f| f.squadrons.iter().find(|(_, s)| s.squadron == sq_name)) {
            if prev.1 == sq {
                report.push(format!("[OK] {old_name}: already migrated as {into}/{sq_name}; no changes"));
            } else {
                refuse(&mut plan, format!("squadron {sq_name} already exists in {into} with different content (old fleet {})", dir.display()));
            }
            continue;
        }
        report.push(format!("[OK] {old_name}: becomes squadron {into}/{sq_name} with {} ship(s)", sq.ships.len()));
        squads.push((sq, fleet_md));
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
        aide: existing.as_ref().map(|f| f.config.aide.clone()).unwrap_or_default(),
        // the stamp the fleet carries already (a merge never lowers it)
        format: existing.as_ref().and_then(|f| f.config.format),
        min_nimitz: existing.as_ref().and_then(|f| f.config.min_nimitz.clone()),
    };
    if existing.as_ref().map(|f| f.config != fleet_config).unwrap_or(true) {
        plan.steps.push(step_write(Some(fid.clone()), &fleet_dir.join("fleet.toml"), toml_out(&fleet_config), "merged machine labels"));
    }
    let old_legacy = existing.as_ref().map(config::legacy_map).unwrap_or_default();
    if legacy != old_legacy {
        report.push(format!("[OK] {} old session name(s) recorded in {}/legacy.toml", legacy.len(), into.to_lowercase()));
        plan.steps.push(step_write(Some(fid.clone()), &fleet_dir.join("legacy.toml"), config::legacy_toml(&legacy), "old session names -> identities"));
    }
    // What `nimitz check` will say about the result, in the report now.
    check_result(env, &fleet_dir, &fleet_config, existing.as_ref(), &squads, &mut report);
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

/// Validate the fleet as it will be after the import (config.layout and every
/// other config rule, plus folder.missing / folder.untrusted for this machine)
/// and put each issue in the report.
fn check_result(env: &Env, fleet_dir: &Path, config: &FleetConfig, existing: Option<&FleetOnDisk>, squads: &[(SquadronConfig, Option<String>)], report: &mut Vec<String>) {
    let mut squadrons: Vec<(PathBuf, SquadronConfig)> = existing.map(|f| f.squadrons.clone()).unwrap_or_default();
    for (sq, _) in squads {
        squadrons.push((fleet_dir.join("squadrons").join(sq.squadron.to_lowercase()), sq.clone()));
    }
    let machine = config.machines.iter().find(|(_, m)| env.matches_hostname(&m.hostname)).map(|(l, _)| l.clone());
    let f = FleetOnDisk { dir: fleet_dir.to_path_buf(), config: config.clone(), squadrons, machine };
    let mut issues = config::validate(env, &f);
    let mut trust = crate::check::TrustCache::default();
    for (_, sq) in &f.squadrons {
        for ship in &sq.ships {
            let id = Id::ship(&config.fleet, &sq.squadron, &ship.name);
            if let Some(d) = config::member_dir(env, &f, &id) {
                issues.extend(crate::check::folder_issue_in(env, &mut trust, &id, &d.to_string_lossy()));
            }
        }
    }
    if issues.is_empty() {
        report.push("[OK] the result passes `nimitz check` for its config and folders on this machine".to_string());
    }
    for i in issues {
        let tag = match i.severity {
            Severity::Error => "[ERROR]",
            Severity::Warn => "[WARN]",
            Severity::Info => "[INFO]",
        };
        let fix = i.fix.map(|x| format!(" (fix: {x})")).unwrap_or_default();
        report.push(format!("{tag} after import, check will report {}: {}{fix}", i.code, i.message));
    }
}

// ------------------------------------------------------------------ 0.3.x tenders

/// Was `id` a 0.3.x tender (`F/SQ/~task`)? The role was removed in 0.4.0: its
/// identities and job rows are history only (never a member, never started).
pub fn is_tender_id(id: &str) -> bool {
    let p: Vec<&str> = id.split('/').collect();
    p.len() == 3 && p[2].starts_with('~')
}

/// Move every tender identity of a v1 registry to `retired`. Returns how many
/// moved. `registry::load` runs it on every read (in memory, nothing is
/// written); the next registry write saves the moved shape.
pub fn retire_tenders(r: &mut Registry) -> usize {
    let ids: Vec<String> = r.identities.keys().filter(|k| is_tender_id(k)).cloned().collect();
    for id in &ids {
        let Some(rec) = r.identities.remove(id) else { continue };
        match r.retired.get_mut(id) {
            None => {
                r.retired.insert(id.clone(), rec);
            }
            // both maps know it (a rebuild from registry.json.bak and the events
            // can do that): one history, and the newer of the two currents
            Some(old) => {
                for l in rec.lineage {
                    if !old.lineage.iter().any(|x| x.session_id == l.session_id) {
                        old.lineage.push(l);
                    }
                }
                let at = |cur: &Option<String>, lineage: &[LineageEntry]| {
                    cur.as_ref().and_then(|c| lineage.iter().find(|l| &l.session_id == c)).map(|l| l.at).unwrap_or(f64::MIN)
                };
                if rec.session.is_some() && (old.session.is_none() || at(&rec.session, &old.lineage) > at(&old.session, &old.lineage)) {
                    old.session = rec.session;
                    old.job = rec.job.or(old.job.take());
                }
            }
        }
    }
    ids.len()
}

/// Migrate a 0.3.x (v1) registry to v2 in memory (plan-v4 section 2): serde
/// already read `current` as `session` and `bound_at` as `at`; this fills each
/// identity's `job` (and each lineage entry's) from the job rows whose
/// `state.json` `sessionId` or `resumeSessionId` is that session, and stamps
/// the version. `registry::load` runs it on every read of a v1 file (nothing
/// is written); the next `update` saves the v2 shape and keeps the old file as
/// `registry.json.v1`. Returns whether anything changed.
pub fn registry_v2(env: &Env, r: &mut Registry) -> bool {
    if r.version >= 2 {
        return false;
    }
    let jobs = job_rows(env);
    let job_of = |sid: &str| jobs.iter().find(|(_, born, now)| born == sid || now.as_deref() == Some(sid)).map(|(j, _, _)| j.clone());
    for rec in r.identities.values_mut().chain(r.retired.values_mut()) {
        if rec.job.is_none() {
            rec.job = rec.session.as_deref().and_then(job_of);
        }
        for l in rec.lineage.iter_mut() {
            if l.job.is_none() {
                l.job = job_of(&l.session_id);
            }
        }
    }
    // version 2: the chain's next step (`formats`, 2 -> 3) is the runner's,
    // with its backup; a load never moves a file past the shape it read
    r.version = 2;
    true
}

/// Every job row: (short id, `sessionId`, `resumeSessionId`).
fn job_rows(env: &Env) -> Vec<(String, String, Option<String>)> {
    let Ok(rd) = std::fs::read_dir(env.claude_home.join("jobs")) else { return vec![] };
    rd.filter_map(|e| e.ok())
        .filter_map(|e| {
            let d = crate::evidence::first_json(&std::fs::read(e.path().join("state.json")).ok()?)?;
            let born = d.get("sessionId")?.as_str()?.to_string();
            let now = d.get("resumeSessionId").and_then(|v| v.as_str()).map(str::to_string);
            Some((e.file_name().to_string_lossy().into_owned(), born, now))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::{apply_ok, test_env};

    #[test]
    fn a_v1_registry_retires_its_tenders_on_load() {
        let (_td, env) = test_env();
        let ship = Id::ship("MAIN", "ARGOS", "ARGOS");
        crate::registry::update(&env, |r| crate::registry::bind(r, &ship, Role::Ship, &ship.name(), "/w", "sid-1", "launch", 1.0)).unwrap();
        // the 0.3.x shape, word for word: version 1, a `tender` role and a `~task` id
        let raw = std::fs::read_to_string(env.registry_path()).unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        v["version"] = serde_json::json!(1);
        v["identities"]["MAIN/ARGOS/~fixci"] = serde_json::json!({
            "role": "tender", "name": "[MAIN.ARGOS]-COMMODORE/fixci", "cwd": "/sq", "current": "sid-t",
            "lineage": [{"session_id": "sid-t", "bound_at": 2.0, "via": "launch"}]});
        std::fs::write(env.registry_path(), v.to_string()).unwrap();
        let r = crate::registry::load(&env);
        assert_eq!(r.identities.keys().collect::<Vec<_>>(), [&ship.0], "the tender is no member");
        assert_eq!(r.retired["MAIN/ARGOS/~fixci"].session.as_deref(), Some("sid-t"), "its history stays");
        assert_eq!(r.retired["MAIN/ARGOS/~fixci"].lineage.len(), 1);
        // a read writes nothing: the file keeps its 0.3.x shape until the next update
        assert!(std::fs::read_to_string(env.registry_path()).unwrap().contains(r#""role":"tender""#));
        assert_eq!(crate::registry::load(&env), r, "every read moves it the same way");
        assert!(crate::registry::health(&env).is_empty(), "no corrupt copy was made");
        crate::registry::update(&env, |_| ()).unwrap();
        let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(env.registry_path()).unwrap()).unwrap();
        assert_eq!(saved["version"], 2, "the v2 shape, with `retired`");
        assert!(saved["identities"].get("MAIN/ARGOS/~fixci").is_none());
        assert!(saved["retired"]["MAIN/ARGOS/~fixci"].is_object());
        assert!(crate::registry::v1_path(&env).is_file(), "the v1 file is kept once");
        // the update above stamped the file anew; everything else reads back as before
        let mut again = crate::registry::load(&env);
        again.updated_at = r.updated_at;
        assert_eq!(again, r);
        // a registry with no tenders is written without a `retired` key
        let (_td2, env2) = test_env();
        crate::registry::update(&env2, |r| crate::registry::bind(r, &ship, Role::Ship, &ship.name(), "/w", "sid-1", "launch", 1.0)).unwrap();
        assert!(!std::fs::read_to_string(env2.registry_path()).unwrap().contains("retired"));
    }

    #[test]
    fn a_tender_known_to_both_maps_merges_into_one_history() {
        let (_td, env) = test_env();
        let tid = "MAIN/ARGOS/~fixci";
        let rec = |cur: &str, lineage: &[(&str, f64)]| {
            serde_json::json!({"role": "tender", "name": "[MAIN.ARGOS]-COMMODORE/fixci", "cwd": "/sq", "current": cur,
                "lineage": lineage.iter().map(|(s, t)| serde_json::json!({"session_id": s, "bound_at": t, "via": "launch"})).collect::<Vec<_>>()})
        };
        std::fs::create_dir_all(&env.state).unwrap();
        // retired knows the old conversation; identities (as a rebuild leaves it) the newer one
        let v = serde_json::json!({"version": 1, "machine": "test-mac",
            "identities": {tid: rec("sid-2", &[("sid-1", 1.0), ("sid-2", 5.0)])},
            "retired": {tid: rec("sid-1", &[("sid-1", 1.0)])}});
        std::fs::write(env.registry_path(), v.to_string()).unwrap();
        let r = crate::registry::load(&env);
        assert!(r.identities.is_empty());
        let got = &r.retired[tid];
        assert_eq!(got.lineage.iter().map(|l| l.session_id.as_str()).collect::<Vec<_>>(), ["sid-1", "sid-2"]);
        assert_eq!(got.session.as_deref(), Some("sid-2"), "the newer current wins");
        // the other way round: the retired current is the newer one and stays
        let v = serde_json::json!({"version": 1, "machine": "test-mac",
            "identities": {tid: rec("sid-1", &[("sid-1", 1.0)])},
            "retired": {tid: rec("sid-2", &[("sid-2", 5.0)])}});
        std::fs::write(env.registry_path(), v.to_string()).unwrap();
        let got = crate::registry::load(&env).retired[tid].clone();
        assert_eq!(got.session.as_deref(), Some("sid-2"));
        assert_eq!(got.lineage.len(), 2);
    }

    #[test]
    fn a_rebuild_from_events_keeps_tenders_out_of_the_members() {
        let (_td, env) = test_env();
        let ship = Id::ship("MAIN", "ARGOS", "ARGOS");
        crate::registry::update(&env, |r| crate::registry::bind(r, &ship, Role::Ship, &ship.name(), "/w", "sid-1", "launch", 1.0)).unwrap();
        let ev = |sid: &str, id: &str, role: &str, ts: f64| {
            serde_json::json!({"ts": ts, "event": "SessionStart", "session_id": sid, "source": "startup",
                "labels": {"NIMITZ_ID": id, "NIMITZ_ROLE": role}}).to_string()
        };
        // (newer than the last good copy: only those lines are replayed over it)
        let t0 = crate::registry::load(&env).updated_at;
        std::fs::write(env.events_path(), format!("{}\n{}\n", ev("sid-t", "MAIN/ARGOS/~fixci", "tender", t0 + 1.0), ev("sid-2", &ship.0, "ship", t0 + 2.0))).unwrap();
        std::fs::write(env.registry_path(), "{garbage").unwrap();
        let r = crate::registry::load(&env);
        assert_eq!(r.identities.keys().collect::<Vec<_>>(), [&ship.0], "{:?}", r.identities.keys());
        assert_eq!(r.identities[&ship.0].session.as_deref(), Some("sid-2"));
        assert_eq!(r.retired["MAIN/ARGOS/~fixci"].session.as_deref(), Some("sid-t"), "the tender's event is history");
        assert_eq!(crate::registry::load(&env), r, "the saved rebuild reads back the same");
    }

    #[test]
    fn tender_ids_are_three_parts_with_a_tilde() {
        assert!(is_tender_id("F/SQ/~fix-ci"));
        assert!(!is_tender_id("F/SQ/SHIP") && !is_tender_id("F/SQ") && !is_tender_id("F") && !is_tender_id("F/SQ/~x/y"));
    }

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

    /// An import into a fleet whose squadron.toml or legacy.toml a newer
    /// nimitz wrote is refused, as every config writer refuses it.
    #[test]
    fn an_import_never_writes_over_a_file_for_a_newer_nimitz() {
        let (td, env) = test_env();
        let a = td.path().join("old/bolt-fleet");
        old_fleet(&a, BOLT, None);
        let (plan, _) = plan_from_fleet_skill(&env, std::slice::from_ref(&a), "MAIN");
        apply_ok(&env, &plan);
        let f = config::load_fleet(&env, "MAIN").unwrap();
        let legacy = config::legacy_path(&f);
        std::fs::write(&legacy, "min_nimitz = \"99.0.0\"\n[names]\n").unwrap();
        let (plan, _) = plan_from_fleet_skill(&env, std::slice::from_ref(&a), "MAIN");
        assert!(plan.refusals.iter().any(|r| r.code == "config.too_new"), "{plan:?}");
        std::fs::remove_file(&legacy).unwrap();
        let sq = f.squadrons[0].0.join("squadron.toml");
        let future = format!("min_nimitz = \"99.0.0\"\n{}", std::fs::read_to_string(&sq).unwrap());
        std::fs::write(&sq, &future).unwrap();
        let (plan, _) = plan_from_fleet_skill(&env, &[a], "MAIN");
        assert!(plan.refusals.iter().any(|r| r.code == "config.too_new"), "{plan:?}");
        assert_eq!(std::fs::read_to_string(&sq).unwrap(), future);
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
        let (plan, _) = plan_from_fleet_skill(&env, std::slice::from_ref(&a), "MAIN");
        apply_ok(&env, &plan);
        // The same old fleet again: idempotent, nothing planned.
        let (plan2, report2) = plan_from_fleet_skill(&env, std::slice::from_ref(&a), "MAIN");
        assert!(plan2.ok(), "{:?}", plan2.refusals);
        assert!(plan2.steps.is_empty(), "{:?}", plan2.steps);
        assert!(report2.join("\n").contains("no changes"));
        // The same name with different content: refused.
        old_fleet(&a, &BOLT.replace("You are BOLT.", "You are not BOLT."), None);
        let (plan2b, _) = plan_from_fleet_skill(&env, std::slice::from_ref(&a), "MAIN");
        assert!(!plan2b.ok());
        old_fleet(&a, BOLT, None); // restore
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
        let (plan, _) = plan_from_fleet_skill(&env, std::slice::from_ref(&a), "MAIN");
        assert!(!plan.ok());
        old_fleet(&a, r#"{"machines": {}}"#, None);
        let (plan, _) = plan_from_fleet_skill(&env, &[a], "MAIN");
        assert!(!plan.ok());
        let (plan, _) = plan_from_fleet_skill(&env, &[], "bad name");
        assert!(!plan.ok());
        // Two old fleets that would become the same squadron, in one run.
        let c = td.path().join("old/bolt-copy");
        old_fleet(&c, BOLT, None);
        let d = td.path().join("old/bolt-fleet");
        old_fleet(&d, BOLT, None);
        let (plan, _) = plan_from_fleet_skill(&env, &[c, d], "MAIN");
        assert!(!plan.ok());
    }

    // The real old fleets are exercised from fixture copies in
    // tests/migrate_old_fleets.rs (load_all + check + idempotence).

    #[test]
    fn the_report_lists_what_check_will_say() {
        let (td, env) = test_env();
        let home = td.path().join("home");
        let proj = home.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(home.join("CLAUDE.md"), "mine").unwrap();
        let json = format!(
            r#"{{"fleet": "GEN", "machines": {{"m": {{"hostname": "test-mac"}}}}, "agents": [
              {{"name": "HOME", "prompt": "p", "dirs": {{"m": "{}"}}}},
              {{"name": "PROJ", "prompt": "p", "dirs": {{"m": "{}"}}}},
              {{"name": "GONE", "prompt": "p", "dirs": {{"m": "/no/such/folder"}}}}]}}"#,
            home.display().to_string().replace('\\', "\\\\"),
            proj.display().to_string().replace('\\', "\\\\")
        );
        let old = td.path().join("old/gen");
        old_fleet(&old, &json, None);
        let (plan, report) = plan_from_fleet_skill(&env, &[old], "MAIN");
        assert!(plan.ok(), "{:?}", plan.refusals);
        let r = report.join("\n");
        assert!(r.contains("check will report config.layout") && r.contains("MAIN/GEN/PROJ"), "{r}");
        assert!(r.contains("check will report folder.missing"), "{r}");
    }
}
