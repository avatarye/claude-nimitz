//! Migrate fixture copies of the user's real fleet-skill fleets
//! (tests/fixtures/old-fleets/) into a temp NIMITZ_HOME, then prove the result
//! loads, checks clean, and that a second import plans nothing.
//! Run with `-- --nocapture` to read the migration report.
use nimitz_core::config;
use nimitz_core::env::Env;
use nimitz_core::migrate::plan_from_fleet_skill;
use nimitz_core::{check, Severity, Snapshot};
use std::collections::HashMap;
use std::path::PathBuf;

fn test_env(base: &std::path::Path) -> Env {
    let v: HashMap<String, String> = [
        ("HOME", base.join("home").to_string_lossy().into_owned()),
        ("NIMITZ_HOME", base.join("nimitz").to_string_lossy().into_owned()),
        ("NIMITZ_STATE", base.join("state").to_string_lossy().into_owned()),
        ("NIMITZ_CLAUDE_HOME", base.join("cchome").to_string_lossy().into_owned()),
        ("NIMITZ_CLAUDE_BIN", "/usr/bin/false".into()),
        ("NIMITZ_NATIVE_CLAUDE_BIN", "/usr/bin/false".into()),
        ("NIMITZ_HOSTNAME", "test-mac".into()),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .into();
    Env::from_vars(&v)
}

/// The committed copies of the real old fleets, in name order.
fn fixtures() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("old-fleets");
    let mut out: Vec<_> = std::fs::read_dir(&dir)
        .expect("fixtures/old-fleets missing")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join("FLEET_CONFIG.json").is_file())
        .collect();
    out.sort();
    out
}

/// A snapshot with no live sessions: check() then judges the config on disk.
fn empty_snapshot() -> Snapshot {
    Snapshot {
        taken_at: 0.0,
        machine: None,
        fleets: vec![],
        stowaways: vec![],
        issues: vec![],
        cc_version: None,
        compat_ok: true,
    }
}

#[test]
fn real_old_fleets_migrate_check_clean_and_idempotent() {
    let td = tempfile::tempdir().unwrap();
    let env = test_env(td.path());
    let old = fixtures();
    assert!(!old.is_empty());

    let (plan, report) = plan_from_fleet_skill(&env, &old, "MAIN");
    println!("== migration report for {} old fleet(s) ==", old.len());
    for line in &report {
        println!("{line}");
    }
    assert!(plan.ok(), "refused: {:?}", plan.refusals);
    let out = config::apply_config_plan(&env, &plan);
    assert!(out.verified, "not verified: {:?}", out.issues);

    // The migrated fleet loads whole and validates with warnings at worst.
    let (fleets, issues) = config::load_all(&env);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(fleets.len(), 1);
    let f = &fleets[0];
    assert_eq!(f.config.fleet, "MAIN");
    assert_eq!(f.squadrons.len(), old.len());

    // One canonical label per hostname: the duplicate-label hosts collapsed.
    let labels: Vec<_> = f.config.machines.keys().cloned().collect();
    assert_eq!(labels, ["home-server", "home-ubuntu", "home-windows", "ubuntu-box", "west-mac"]);
    let hostnames: Vec<_> = f.config.machines.values().map(|m| m.hostname.clone()).collect();
    assert_eq!(hostnames.len(), hostnames.iter().collect::<std::collections::BTreeSet<_>>().len());
    // Every ship's dirs use canonical labels only.
    for (_, sq) in &f.squadrons {
        for ship in &sq.ships {
            for label in ship.dirs.keys().chain(ship.permission_mode.keys()) {
                assert!(f.config.machines.contains_key(label), "{}/{}: label {label:?} not canonical", sq.squadron, ship.name);
            }
        }
    }

    // check() over the result: nothing worse than a warning.
    let bad: Vec<_> = check::check(&env, &empty_snapshot(), None)
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    assert!(bad.is_empty(), "{bad:?}");

    // Idempotent: the same import again plans no steps.
    let (plan2, report2) = plan_from_fleet_skill(&env, &old, "MAIN");
    assert!(plan2.ok(), "refused: {:?}", plan2.refusals);
    assert!(plan2.steps.is_empty(), "second import planned: {:?}", plan2.steps);
    assert!(report2.iter().filter(|l| l.contains("no changes")).count() == old.len(), "{report2:?}");
}

/// On the other Mac (the hostname in the fixtures), GENERAL/HOME's folder is the
/// home folder and holds the other ships' folders: that is fine (contracts.md
/// section 7), and the old session names are recorded for the `legacy` rule.
#[test]
fn on_the_west_mac_a_home_folder_ship_is_not_a_layout_error_and_old_names_map() {
    let td = tempfile::tempdir().unwrap();
    let mut env = test_env(td.path());
    env.hostname = "3f2c1d0e9b7a".into();
    let (plan, _) = plan_from_fleet_skill(&env, &fixtures(), "MAIN");
    assert!(plan.ok(), "{:?}", plan.refusals);
    assert!(config::apply_config_plan(&env, &plan).verified);
    let f = config::load_fleet(&env, "MAIN").unwrap();
    let layout_errors: Vec<_> =
        config::validate(&env, &f).into_iter().filter(|i| i.code == "config.layout" && i.severity == Severity::Error).collect();
    assert!(layout_errors.is_empty(), "{layout_errors:?}");

    let legacy = config::legacy_map(&f);
    assert_eq!(legacy.get("[GENERAL]-HOME").map(|i| i.0.as_str()), Some("MAIN/GENERAL/HOME"));
    assert_eq!(legacy.get("[ARGOS]-ARGOS").map(|i| i.0.as_str()), Some("MAIN/ARGOS/ARGOS"));
    assert_eq!(legacy.get("[ARGOS]-LEAD").map(|i| i.0.as_str()), Some("MAIN/ARGOS"));
    // a second import changes nothing, the legacy map included
    let (again, _) = plan_from_fleet_skill(&env, &fixtures(), "MAIN");
    assert!(again.steps.is_empty(), "{:?}", again.steps.iter().map(|s| &s.why).collect::<Vec<_>>());
}
