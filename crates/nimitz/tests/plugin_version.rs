//! The installed plugin is cached by its version
//! (`~/.claude/plugins/cache/claude-nimitz/nimitz/<version>`), so a change
//! under `plugin/` (a skill, the hooks) reaches no installed plugin until
//! `plugin/.claude-plugin/plugin.json`'s `version` is bumped and the user runs
//! `claude plugin update`. This test fails when anything under `plugin/`
//! changed after the commit that last bumped the version: in a later commit,
//! or in the working tree without a bump there. Skipped outside a git
//! checkout or in a shallow clone (no history to read).
use std::path::{Path, PathBuf};
use std::process::Command;

const MANIFEST: &str = "plugin/.claude-plugin/plugin.json";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn git(args: &[&str]) -> Option<String> {
    let o = Command::new("git").current_dir(root()).args(args).env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// The manifest's `version` in `rev` (`None`: the working tree).
fn version_line(rev: Option<&str>) -> Option<String> {
    let text = match rev {
        Some(r) => git(&["show", &format!("{r}:{MANIFEST}")])?,
        None => std::fs::read_to_string(root().join(MANIFEST)).ok()?,
    };
    let line = text.lines().find(|l| l.trim_start().starts_with("\"version\""))?;
    Some(line.split_once(':')?.1.trim().trim_end_matches(',').trim_matches('"').to_string())
}

#[test]
fn a_change_under_plugin_comes_with_a_version_bump() {
    if git(&["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        println!("[OK] skipped: not a git checkout");
        return;
    }
    if git(&["rev-parse", "--is-shallow-repository"]).as_deref() == Some("true") {
        println!("[OK] skipped: a shallow clone has no history to read");
        return;
    }
    // the last commit whose manifest changed its `version` line
    let bump = git(&["log", "-1", "--format=%H", "-G", "\"version\"", "--", MANIFEST]).filter(|h| !h.is_empty()).expect("a commit that set the plugin version");
    let later = git(&["log", "--format=%h %s", &format!("{bump}..HEAD"), "--", "plugin/"]).unwrap_or_default();
    assert!(
        later.is_empty(),
        "plugin/ changed after the last version bump ({}), so no installed plugin gets it; bump `version` in {MANIFEST} (then `claude plugin update nimitz@claude-nimitz`):\n{later}",
        &bump[..8]
    );
    let dirty = git(&["status", "--porcelain", "--", "plugin/"]).unwrap_or_default();
    if !dirty.is_empty() {
        assert_ne!(
            version_line(None),
            version_line(Some("HEAD")),
            "plugin/ has changes and {MANIFEST} keeps version {}; bump it with them:\n{dirty}",
            version_line(None).unwrap_or_default()
        );
    }
}
