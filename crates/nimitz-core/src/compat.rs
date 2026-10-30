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
/// in its path is still the one it runs. An npm install has no version in
/// its path: it is read from the package's `package.json` (`npm_version`).
pub fn claude_version_in(path: &str) -> Option<String> {
    version_in(path, Started::Any)
}

/// When the process a path is read for started (an npm package.json counts
/// only when it is older: `npm_version`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Started {
    /// no process: the path alone (`claude_version_in`)
    Any,
    /// epoch seconds
    At(f64),
    /// a process whose start cannot be read (macOS): no package.json counts
    Unknown,
}

fn version_in(path: &str, started: Started) -> Option<String> {
    match path.strip_suffix(" (deleted)") {
        // replaced under the running process: what is on disk now (an npm
        // package.json) is the new version, not the one it runs
        Some(old) => in_path(old),
        None => in_path(path).or_else(|| npm_version(path, &|p| std::fs::read_to_string(p).ok(), &modified, started)),
    }
}

/// A file's modification time (epoch seconds).
fn modified(p: &str) -> Option<f64> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64())
}

/// An absolute path, Unix (`/...`) or Windows (`C:\...`, `C:/...`, `\\server\...`).
fn absolute(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with(['/', '\\']) || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'/' | b'\\'))
}

/// A Windows-style path (a drive letter or a UNC path), or any path on Windows.
fn windows_path(p: &str) -> bool {
    cfg!(windows) || p.as_bytes().get(1) == Some(&b':') || p.starts_with("\\\\")
}

fn in_path(path: &str) -> Option<String> {
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

/// The npm package an install runs from: `.../node_modules/@anthropic-ai/claude-code`.
const NPM_PACKAGE: &str = "/node_modules/@anthropic-ai/claude-code";

/// An npm install's version, from `<package root>/package.json` (`read`
/// reads a file): the root is found by walking up from a path inside the
/// package (`bin/claude(.exe)`, `cli.js`), from where a link points (the
/// `bin/claude` npm puts on PATH), or beside a shim (`<npm>/claude.cmd`,
/// `claude.ps1`, the POSIX `claude` script). Only that package's own
/// `package.json` counts (its `name`), and only from an absolute path (a
/// relative argv0 would resolve against nimitz's own folder). For a running
/// process, only a package.json older than the process: one written after
/// it started is an `npm update`'s, the NEW version (a process under node is
/// never `(deleted)`, and its argv0's link reaches the new package); a
/// process whose start cannot be read gets none (fail-safe: Unknown).
fn npm_version(path: &str, read: &dyn Fn(&str) -> Option<String>, mtime: &dyn Fn(&str) -> Option<f64>, started: Started) -> Option<String> {
    if !absolute(path) {
        return None;
    }
    let unix = path.replace('\\', "/");
    let mut roots: Vec<String> = vec![];
    let mut inside = |p: &str| {
        if let Some(i) = p.find(&format!("{NPM_PACKAGE}/")).or_else(|| p.ends_with(NPM_PACKAGE).then(|| p.len() - NPM_PACKAGE.len())) {
            roots.push(p[..i + NPM_PACKAGE.len()].to_string());
        }
    };
    inside(&unix);
    if let Ok(real) = std::fs::canonicalize(path) {
        inside(&real.to_string_lossy().replace('\\', "/"));
    }
    // a shim: `claude.cmd`, `claude.ps1` or the extension-less `claude`
    // script (never a `claude.exe` that merely sits beside node_modules)
    if let Some((dir, file)) = unix.rsplit_once('/') {
        let (stem, ext) = file.rsplit_once('.').unwrap_or((file, ""));
        let shim = ext.is_empty() || ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("ps1");
        if shim && stem.eq_ignore_ascii_case("claude") && !dir.is_empty() {
            roots.push(format!("{dir}{NPM_PACKAGE}"));
        }
    }
    roots.iter().find_map(|root| {
        let file = format!("{root}/package.json");
        let pkg: serde_json::Value = serde_json::from_str(&read(&file)?).ok()?;
        let before = match started {
            Started::Any => true,
            Started::At(t) => mtime(&file).is_some_and(|m| m <= t),
            Started::Unknown => false,
        };
        (pkg["name"] == "@anthropic-ai/claude-code" && before).then(|| pkg["version"].as_str().and_then(parse_version)).flatten()
    })
}

/// The executable process `pid` runs (`os::exe_of`).
pub fn exe_of(pid: i64) -> Option<String> {
    crate::os::exe_of(pid)
}

/// The Claude Code version a process runs: from its argv0, else from the
/// executable it runs (the `~/.local/bin/claude` launcher is a link into
/// `versions/`, and a background worker's argv0 is a bare `claude`).
pub fn process_version(argv0: &str, pid: Option<i64>) -> Option<String> {
    let started = match pid {
        None => Started::Any,
        Some(p) => crate::os::process_birth_ms(p).map(|ms| Started::At(ms as f64 / 1000.0)).unwrap_or(Started::Unknown),
    };
    version_in(argv0, started).or_else(|| pid.and_then(exe_of).and_then(|e| version_in(&e, started)))
}

/// `process_version`, else the version this machine's Claude Code reports
/// (`installed`, read once per process), but only when the process runs the
/// very binary nimitz itself resolved for it (`Env::native_claude_bin`):
/// a version still read and compared, never assumed for some other binary.
pub fn process_version_or_machine(argv0: &str, pid: Option<i64>) -> Option<String> {
    static MACHINE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    process_version(argv0, pid).or_else(|| {
        let env = Env::from_process();
        let exe = pid.and_then(exe_of);
        runs_native(argv0, exe.as_deref(), &env.native_claude_bin).then(|| MACHINE.get_or_init(|| installed(&env)).clone()).flatten()
    })
}

/// Does a process (its argv0, the executable it runs) run `native` as it is
/// on disk now? Never once its executable was replaced (`(deleted)`),
/// whatever its argv0 says: nimitz launches with argv0 = the native path,
/// which then names the NEW install.
fn runs_native(argv0: &str, exe: Option<&str>, native: &str) -> bool {
    if exe.is_some_and(|e| e.ends_with(" (deleted)")) {
        return false;
    }
    [Some(argv0), exe].into_iter().flatten().any(|p| same_binary(p, native))
}

/// Do `a` and `b` name one executable: the same path once separators and,
/// for a Windows path, `.exe` and case are set aside, or the same file once
/// links are followed? Never a binary replaced since it started (`(deleted)`).
pub fn same_binary(a: &str, b: &str) -> bool {
    let norm = |p: &str| {
        let windows = windows_path(p);
        let p = p.replace('\\', "/");
        match windows {
            true => p.strip_suffix(".exe").unwrap_or(&p).to_ascii_lowercase(),
            false => p,
        }
    };
    // a replaced binary is not the one on disk now, whatever its path
    let deleted = |p: &str| p.ends_with(" (deleted)");
    if a.is_empty() || b.is_empty() || deleted(a) || deleted(b) || !(a.contains(['/', '\\']) && b.contains(['/', '\\'])) {
        return false;
    }
    norm(a) == norm(b) || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
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
        assert_eq!(c.max, "2.1.312");
    }

    /// An npm install (#13): its version from the package's package.json,
    /// from the package's own files, a link to them or a shim beside it, in
    /// Windows and Unix paths; another package's package.json never counts.
    #[test]
    fn an_npm_install_reads_its_package_json() {
        let pkg = |name: &str, v: &str| format!(r#"{{"name": "{name}", "version": "{v}"}}"#);
        let win = r"C:\Users\u\AppData\Roaming\npm";
        let at = |root: &'static str, json: String| move |p: &str| (p == format!("{root}/node_modules/@anthropic-ai/claude-code/package.json")).then(|| json.clone());
        let read = at("C:/Users/u/AppData/Roaming/npm", pkg("@anthropic-ai/claude-code", "2.1.312"));
        for p in [
            format!(r"{win}\node_modules/@anthropic-ai/claude-code/bin\claude.exe"),
            format!(r"{win}\node_modules\@anthropic-ai\claude-code\cli.js"),
            format!(r"{win}\claude.cmd"),
            format!(r"{win}\claude.ps1"),
            format!(r"{win}\claude"),
        ] {
            assert_eq!(npm_version(&p, &read, &|_| None, Started::Any).as_deref(), Some("2.1.312"), "{p}");
        }
        assert_eq!(npm_version(&format!(r"{win}\other.cmd"), &read, &|_| None, Started::Any), None, "not claude's shim");
        assert_eq!(npm_version(&format!(r"{win}\claude.exe"), &read, &|_| None, Started::Any), None, "a claude.exe beside node_modules is no shim");
        assert_eq!(npm_version(&format!(r"{win}\claude.bat"), &read, &|_| None, Started::Any), None, "nor a .bat");
        let rel = at("npm", pkg("@anthropic-ai/claude-code", "2.1.312"));
        assert_eq!(npm_version("npm/claude.cmd", &rel, &|_| None, Started::Any), None, "a relative path never reads");
        assert_eq!(npm_version("./npm/node_modules/@anthropic-ai/claude-code/cli.js", &rel, &|_| None, Started::Any), None);
        let unix = at("/usr/lib", pkg("@anthropic-ai/claude-code", "2.1.300"));
        assert_eq!(npm_version("/usr/lib/node_modules/@anthropic-ai/claude-code/bin/claude", &unix, &|_| None, Started::Any).as_deref(), Some("2.1.300"));
        assert_eq!(npm_version("/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js", &unix, &|_| None, Started::Any).as_deref(), Some("2.1.300"));
        let other = at("/usr/lib", pkg("claude-code-imposter", "9.9.9"));
        assert_eq!(npm_version("/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js", &other, &|_| None, Started::Any), None, "its own package only");
        assert_eq!(npm_version("/home/u/.local/bin/claude", &unix, &|_| None, Started::Any), None);
        // on disk, through claude_version_in, and the link npm puts on PATH
        let td = tempfile::tempdir().unwrap();
        let root = td.path().join("lib/node_modules/@anthropic-ai/claude-code");
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("package.json"), pkg("@anthropic-ai/claude-code", "2.1.312")).unwrap();
        std::fs::write(root.join("bin/claude.exe"), "").unwrap();
        let exe = root.join("bin/claude.exe");
        assert_eq!(claude_version_in(exe.to_str().unwrap()).as_deref(), Some("2.1.312"));
        // replaced under a running process: the package.json on disk is the
        // new version, not the one it runs
        assert_eq!(claude_version_in(&format!("{} (deleted)", exe.display())), None);
        assert_eq!(claude_version_in("/home/u/.local/share/claude/versions/2.1.283 (deleted)").as_deref(), Some("2.1.283"), "a version in the path still counts");
        #[cfg(unix)]
        {
            std::fs::create_dir_all(td.path().join("bin")).unwrap();
            let link = td.path().join("bin/claude");
            std::os::unix::fs::symlink(&exe, &link).unwrap();
            assert_eq!(claude_version_in(link.to_str().unwrap()).as_deref(), Some("2.1.312"));
            assert!(same_binary(link.to_str().unwrap(), exe.to_str().unwrap()), "a link is its target");
        }
    }

    /// Re-review of #13: a process whose executable was replaced is never
    /// the machine's native binary, even when its argv0 is that path; an
    /// npm package.json newer than the process is an update's, never its own.
    #[test]
    fn a_replaced_or_updated_install_never_names_the_running_version() {
        let native = "/home/u/.local/bin/claude";
        assert!(runs_native(native, Some(native), native));
        assert!(runs_native(native, None, native));
        assert!(!runs_native(native, Some("/home/u/.local/share/claude/versions/2.1.300 (deleted)"), native), "argv0 = native, exe deleted");
        assert!(!runs_native("/opt/other/claude", Some("/opt/other/claude"), native));
        let pkg = r#"{"name": "@anthropic-ai/claude-code", "version": "2.1.312"}"#.to_string();
        let read = move |_: &str| Some(pkg.clone());
        let p = "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js";
        let at = |m: f64| move |_: &str| Some(m);
        assert_eq!(npm_version(p, &read, &at(100.0), Started::At(200.0)).as_deref(), Some("2.1.312"), "older than the process");
        assert_eq!(npm_version(p, &read, &at(300.0), Started::At(200.0)), None, "written after it started: an update's");
        assert_eq!(npm_version(p, &read, &|_| None, Started::At(200.0)), None, "no mtime: not trusted");
        assert_eq!(npm_version(p, &read, &at(100.0), Started::Unknown), None, "no start time: not trusted");
        assert_eq!(npm_version(p, &read, &|_| None, Started::Any).as_deref(), Some("2.1.312"), "a path alone");
    }

    #[test]
    fn the_same_binary_is_told_by_path_or_link() {
        let npm = r"C:\Users\u\AppData\Roaming\npm\node_modules/@anthropic-ai/claude-code/bin\claude.exe";
        assert!(same_binary(npm, "c:/users/u/appdata/roaming/npm/node_modules/@anthropic-ai/claude-code/bin/claude.exe"));
        assert!(!same_binary("/opt/c/claude (deleted)", "/opt/c/claude"), "a replaced binary is another one");
        assert!(!same_binary("/opt/c/claude.exe", "/opt/c/claude"), "a Unix path keeps its .exe");
        assert!(same_binary(r"C:\npm\claude.exe", "c:/npm/claude"));
        assert!(!same_binary("/opt/c/claude", "/opt/C/claude"), "a Unix path keeps its case");
        assert!(!same_binary("/opt/c/claude", "/opt/d/claude"));
        assert!(!same_binary("claude", "claude"), "a bare name is no path");
        assert!(!same_binary("", ""));
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
