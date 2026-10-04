//! Supported Claude Code range (compat.toml). Owner: core-config.
use crate::env::Env;

#[derive(Clone, Debug, PartialEq)]
pub struct Compat {
    pub min: String,
    pub max: String,
}

/// docs/compat.toml, compiled into this build.
const COMPAT_TOML: &str = include_str!("../../../docs/compat.toml");

/// The range compiled into this build.
pub fn supported() -> Compat {
    let v: toml::Value = toml::from_str(COMPAT_TOML).expect("docs/compat.toml is part of the build and must parse");
    let get = |k: &str| v.get(k).and_then(|x| x.as_str()).expect("docs/compat.toml needs min and max").to_string();
    Compat { min: get("min"), max: get("max") }
}

/// The version inside a `--version` line, tolerant of both builds:
/// `2.1.288 (Claude Code)` and `claude 2.1.288.1000 (ACME wrapper ...)`.
pub fn parse_version(raw: &str) -> Option<String> {
    raw.split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_ascii_digit() && c != '.'))
        .find(|t| {
            let mut parts = t.split('.');
            t.contains('.') && parts.all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(|t| t.to_string())
}

/// Installed version (native binary `--version`), e.g. "2.1.288".
pub fn installed(env: &Env) -> Option<String> {
    let out = std::process::Command::new(&env.native_claude_bin).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

fn nums(v: &str) -> Vec<u64> {
    v.split('.').filter_map(|p| p.parse().ok()).collect()
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
/// `2.1.289.77` is inside a max of `2.1.289`.
pub fn position(c: &Compat, version: &str) -> i32 {
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
    use std::collections::HashMap;

    #[test]
    fn supported_matches_compat_toml() {
        let c = supported();
        assert_eq!(c.min, "2.1.288");
        assert_eq!(c.max, "2.1.289");
    }

    #[test]
    fn parses_both_version_shapes() {
        assert_eq!(parse_version("2.1.288 (Claude Code)").as_deref(), Some("2.1.288"));
        assert_eq!(parse_version("claude 2.1.288.1000 (ACME wrapper, Bedrock)").as_deref(), Some("2.1.288.1000"));
        assert_eq!(parse_version("  2.1.289\n").as_deref(), Some("2.1.289"));
        assert_eq!(parse_version("no version here"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn position_in_range() {
        let c = Compat { min: "2.1.288".into(), max: "2.1.289".into() };
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
