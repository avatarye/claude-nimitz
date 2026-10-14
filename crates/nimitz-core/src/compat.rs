//! Supported Claude Code range (compat.toml). Owner: core-config.
use crate::env::Env;

#[derive(Clone, Debug, PartialEq)]
pub struct Compat {
    pub min: String,
    pub max: String,
    /// pre-release versions tested and supported (`prerelease = [...]` in compat.toml)
    pub prerelease: Vec<String>,
}

/// docs/compat.toml, compiled into this build.
const COMPAT_TOML: &str = include_str!("../../../docs/compat.toml");

/// The range compiled into this build.
pub fn supported() -> Compat {
    let v: toml::Value = toml::from_str(COMPAT_TOML).expect("docs/compat.toml is part of the build and must parse");
    let get = |k: &str| v.get(k).and_then(|x| x.as_str()).expect("docs/compat.toml needs min and max").to_string();
    let prerelease = v
        .get("prerelease")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).map(str::to_string).collect())
        .unwrap_or_default();
    Compat { min: get("min"), max: get("max"), prerelease }
}

/// The version inside a `--version` line, tolerant of both builds:
/// `2.1.288 (Claude Code)` and `claude 2.1.288.1000 (ACME wrapper ...)`.
/// A pre-release suffix is kept (`2.1.288-beta`, `2.1.288-rc.1`); build
/// metadata (`+build`) is dropped, as it carries no order.
pub fn parse_version(raw: &str) -> Option<String> {
    raw.split_whitespace().find_map(|t| {
        let t = t.trim_start_matches(|c: char| !c.is_ascii_digit());
        let t = t.split('+').next().unwrap_or("");
        let (core, pre) = match t.split_once('-') {
            Some((c, p)) => (c, Some(p.trim_end_matches(|c: char| !c.is_ascii_alphanumeric()))),
            None => (t.trim_end_matches(|c: char| !c.is_ascii_digit()), None),
        };
        let numeric = core.contains('.') && core.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        let pre_ok = pre.is_none_or(|p| !p.is_empty() && p.split('.').all(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_alphanumeric())));
        (numeric && pre_ok).then(|| match pre {
            Some(p) => format!("{core}-{p}"),
            None => core.to_string(),
        })
    })
}

/// Installed version (native binary `--version`), e.g. "2.1.288".
pub fn installed(env: &Env) -> Option<String> {
    let mut c = std::process::Command::new(&env.native_claude_bin);
    c.arg("--version");
    crate::claude::scrub_child_env(&mut c);
    let out = c.output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

fn nums(v: &str) -> Vec<u64> {
    v.split('-').next().unwrap_or("").split('.').filter_map(|p| p.parse().ok()).collect()
}

/// Is `v` at or past `min` (dotted, componentwise)?
pub fn at_least(v: &str, min: &str) -> bool {
    cmp(v, min) != std::cmp::Ordering::Less
}

/// Is `v` older than `than` (dotted, componentwise)?
pub fn older(v: &str, than: &str) -> bool {
    cmp(v, than) == std::cmp::Ordering::Less
}

/// The Claude Code version in an executable's path: the official
/// installer's `.../claude/versions/<v>[/...]` or a localtools install's
/// `.../claude-code/<v>/claude` (`<v>` may carry a fourth, build component:
/// `2.1.294.1000`; `at_least` compares it componentwise). A binary replaced
/// since the process started reads `<path> (deleted)` on Linux; the version
/// in its path is still the one it runs.
pub fn claude_version_in(path: &str) -> Option<String> {
    let path = path.strip_suffix(" (deleted)").unwrap_or(path);
    // a Windows path: `\` separators, `claude.exe`
    let unix = path.replace('\\', "/");
    let path = unix.strip_suffix(".exe").unwrap_or(&unix);
    let first = |rest: &str| rest.split('/').next().filter(|v| v.starts_with(|c: char| c.is_ascii_digit())).map(String::from);
    if let Some(v) = path.split("/claude/versions/").nth(1).and_then(first) {
        return Some(v);
    }
    let (_, rest) = path.rsplit_once("/claude-code/")?;
    let (v, bin) = rest.split_once('/')?;
    (bin == "claude").then(|| first(v)).flatten()
}

/// The executable process `pid` runs (`os::exe_of`).
pub fn exe_of(pid: i64) -> Option<String> {
    crate::os::exe_of(pid)
}

/// The Claude Code version a process runs: from its argv0, else from the
/// executable it runs (the `~/.local/bin/claude` launcher is a link into
/// `versions/`, and a background worker's argv0 is a bare `claude`).
pub fn process_version(argv0: &str, pid: Option<i64>) -> Option<String> {
    claude_version_in(argv0).or_else(|| pid.and_then(exe_of).and_then(|e| claude_version_in(&e)))
}

/// Compare dotted versions componentwise, missing components = 0.
fn cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (a, b) = (nums(a), nums(b));
    let n = a.len().max(b.len());
    for i in 0..n {
        let o = a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0));
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    std::cmp::Ordering::Equal
}

/// -1 below, 0 inside, 1 above the range. The max is inclusive at patch level:
/// `2.1.289.77` is inside a max of `2.1.289`. A pre-release (`2.1.288-beta`) is
/// inside only when compat.toml lists it; otherwise it sits just below its
/// release (-1 when that release is the min or below, else 1: untested).
pub fn position(c: &Compat, version: &str) -> i32 {
    if version.contains('-') {
        if c.prerelease.iter().any(|p| p == version) {
            return 0;
        }
        return if cmp(version, &c.min) != std::cmp::Ordering::Greater { -1 } else { 1 };
    }
    if cmp(version, &c.min) == std::cmp::Ordering::Less {
        return -1;
    }
    // Inside when <= max after truncating to the max's precision.
    let max_parts = c.max.split('.').count();
    let truncated: Vec<&str> = version.split('.').take(max_parts).collect();
    match cmp(&truncated.join("."), &c.max) {
        std::cmp::Ordering::Greater => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::collections::HashMap;

    #[test]
    fn supported_matches_compat_toml() {
        let c = supported();
        assert_eq!(c.min, "2.1.288");
        assert_eq!(c.max, "2.1.300");
    }

    #[test]
    fn a_replaced_binary_still_names_its_version() {
        assert_eq!(claude_version_in("/home/u/.local/share/claude/versions/2.1.283 (deleted)").as_deref(), Some("2.1.283"));
        assert!(older("2.1.294", "2.1.295") && older("2.1.294.1000", "2.1.295") && !older("2.1.295", "2.1.295") && !older("2.1.295.1", "2.1.295"));
    }

    #[test]
    fn parses_both_version_shapes() {
        assert_eq!(parse_version("2.1.288 (Claude Code)").as_deref(), Some("2.1.288"));
        assert_eq!(parse_version("claude 2.1.288.1000 (ACME wrapper, Bedrock)").as_deref(), Some("2.1.288.1000"));
        assert_eq!(parse_version("  2.1.289\n").as_deref(), Some("2.1.289"));
        assert_eq!(parse_version("2.1.288-beta (Claude Code)").as_deref(), Some("2.1.288-beta"));
        assert_eq!(parse_version("claude 2.1.288-rc.1+build.5").as_deref(), Some("2.1.288-rc.1"));
        assert_eq!(parse_version("v2.1.289,").as_deref(), Some("2.1.289"));
        assert_eq!(parse_version("2.1.288-"), None);
        assert_eq!(parse_version("no version here"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn position_in_range() {
        let mut c = Compat { min: "2.1.288".into(), max: "2.1.289".into(), prerelease: vec![] };
        assert_eq!(position(&c, "2.1.288-beta"), -1, "a pre-release is not its release");
        assert_eq!(position(&c, "2.1.289-rc.1"), 1, "untested pre-release inside the range: outside");
        c.prerelease.push("2.1.289-rc.1".into());
        assert_eq!(position(&c, "2.1.289-rc.1"), 0, "listed in compat.toml");
        assert_eq!(position(&c, "2.1.287"), -1);
        assert_eq!(position(&c, "2.0.999"), -1);
        assert_eq!(position(&c, "2.1.288"), 0);
        assert_eq!(position(&c, "2.1.288.1000"), 0);
        assert_eq!(position(&c, "2.1.289"), 0);
        assert_eq!(position(&c, "2.1.289.77"), 0); // patch of the max is inside
        assert_eq!(position(&c, "2.1.290"), 1);
        assert_eq!(position(&c, "2.2.0"), 1);
        assert_eq!(position(&c, "3.0.0"), 1);
    }

    #[cfg(unix)]
    #[test]
    fn installed_reads_the_native_binary() {
        let td = tempfile::tempdir().unwrap();
        let bin = td.path().join("claude");
        std::fs::write(&bin, "#!/bin/sh\necho 'claude 2.1.288.1000 (ACME wrapper)'\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let v: HashMap<String, String> = [
            ("HOME".to_string(), td.path().to_string_lossy().into_owned()),
            ("NIMITZ_NATIVE_CLAUDE_BIN".to_string(), bin.to_string_lossy().into_owned()),
            ("NIMITZ_HOSTNAME".to_string(), "m".to_string()),
        ]
        .into();
        let env = Env::from_vars(&v);
        assert_eq!(installed(&env).as_deref(), Some("2.1.288.1000"));
        // A binary that fails is None, not a panic.
        let v2: HashMap<String, String> = [
            ("HOME".to_string(), td.path().to_string_lossy().into_owned()),
            ("NIMITZ_NATIVE_CLAUDE_BIN".to_string(), "/usr/bin/false".to_string()),
            ("NIMITZ_HOSTNAME".to_string(), "m".to_string()),
        ]
        .into();
        assert_eq!(installed(&Env::from_vars(&v2)), None);
    }
}
