//! Paths and overrides (contracts.md section 2). Owner: core-config.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Env {
    pub home: PathBuf,
    pub state: PathBuf,
    pub claude_home: PathBuf,
    pub claude_bin: String,
    pub native_claude_bin: String,
    pub herdr_bin: Option<String>,
    pub hostname: String,
    pub mux: Option<String>,
    /// how long a start waits for the daemon to restart a crashed job before
    /// it resumes the job itself (`NIMITZ_CRASH_WINDOW`, seconds; default 20)
    pub crash_window_s: f64,
}

/// `$HOME` from the given vars first (tests), then the process env; on
/// Windows `USERPROFILE` when there is no `HOME` (Claude Code's hooks there
/// may run without one).
fn home_dir(v: &HashMap<String, String>) -> PathBuf {
    let keys: &[&str] = if cfg!(windows) { &["HOME", "USERPROFILE"] } else { &["HOME"] };
    keys.iter()
        .find_map(|k| v.get(*k).filter(|s| !s.is_empty()).map(PathBuf::from))
        .or_else(|| keys.iter().find_map(|k| std::env::var_os(k).filter(|s| !s.is_empty()).map(PathBuf::from)))
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// This process's home: `HOME`, on Windows `USERPROFILE` when there is none.
pub fn home() -> PathBuf {
    home_dir(&HashMap::new())
}

/// `p` with a leading `~/` (on Windows also `~\`) or a lone `~` as `home`.
pub fn expand(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\").filter(|_| cfg!(windows))) {
        Some(rest) => home.join(rest),
        None if p == "~" => home.to_path_buf(),
        None => PathBuf::from(p),
    }
}

/// The real path of `p` (symlinks resolved, as Claude Code records folders):
/// the longest existing prefix canonicalized, the rest appended as given.
pub fn canon(p: &Path) -> PathBuf {
    let mut rest = vec![];
    let mut cur = p;
    loop {
        if let Ok(real) = crate::os::canonical(&cur) {
            return rest.iter().rev().fold(real, |acc: PathBuf, c| acc.join(c));
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                cur = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// The git repository (or worktree) root holding `dir`: the nearest folder,
/// walking up the real path, with a `.git` folder or file.
pub fn git_root(dir: &Path) -> Option<PathBuf> {
    canon(dir).ancestors().find(|a| a.join(".git").exists()).map(Path::to_path_buf)
}

/// A trust-file key (or a folder) in the one form keys are compared in:
/// without a trailing separator; on Windows also with `/` for `\` (Claude
/// Code writes `C:\x` and `C:/x` there). Case is kept: Claude Code 2.1.296
/// on Windows looks a folder up in its on-disk case and compares keys
/// case-sensitively, so a trust accepted from a shell that typed `c:\...`
/// (key `c:/...`) does not cover `C:\...` for a background launch.
pub fn trust_key(k: &str) -> String {
    if cfg!(windows) {
        let k = k.replace('\\', "/");
        let t = k.trim_end_matches('/');
        // a drive root keeps its separator
        if t.len() == 2 && t.ends_with(':') { format!("{t}/") } else { t.to_string() }
    } else {
        let t = k.trim_end_matches('/');
        if t.is_empty() { "/".into() } else { t.to_string() }
    }
}

/// `dir` and its parents, as given and as real paths, without duplicates: the
/// trust-file keys that may cover `dir`. A parent as given counts only when
/// it holds the real folder too: Claude Code resolves links first, so a
/// trusted home does not cover `~/.claude-nimitz` when that is a link into
/// Dropbox.
pub fn trust_ancestors(dir: &Path) -> Vec<PathBuf> {
    let real = canon(dir);
    let mut keys: Vec<PathBuf> = vec![];
    let given = dir.ancestors().filter(|a| *a == dir || real.starts_with(canon(a)));
    for a in given.chain(real.ancestors()) {
        if a.as_os_str().is_empty() {
            continue;
        }
        let t = a.to_string_lossy().trim_end_matches('/').to_string();
        let a = if t.is_empty() { PathBuf::from("/") } else { PathBuf::from(t) };
        if !keys.contains(&a) {
            keys.push(a);
        }
    }
    keys
}

/// `trust_ancestors`, cut at the git root when `dir` is in a repository: the
/// keys that are the repository's own trust (the root or below it).
pub fn trust_keys(dir: &Path) -> Vec<PathBuf> {
    match git_root(dir) {
        Some(root) => trust_ancestors(dir).into_iter().filter(|a| canon(a).starts_with(&root)).collect(),
        None => trust_ancestors(dir),
    }
}

/// A git worktree root: `.git` is a file (`gitdir: ...`), not a folder.
pub fn is_git_worktree(root: &Path) -> bool {
    root.join(".git").is_file()
}

/// When `root` became a repository (epoch seconds): the birth time of its
/// `.git` (folder or worktree file). None when the file system keeps no birth
/// time: "cannot tell", never a guess from a later write.
pub fn git_birth(root: &Path) -> Option<f64> {
    std::fs::metadata(root.join(".git")).ok()?.created().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs_f64())
}

/// First `name` on PATH that is a file. On Windows only a file Windows can
/// start counts: `name` with one of `PATHEXT`'s extensions (an npm install
/// puts a POSIX `claude` script beside `claude.cmd`), and an npm shim is
/// followed to the program it starts (`claude.cmd` -> its `claude.exe`).
pub fn which(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    let found = std::env::split_paths(&path).find_map(|d| candidates(&d, name).into_iter().find(|p| p.is_file()))?;
    Some(npm_target(&found).unwrap_or(found).to_string_lossy().into_owned())
}

#[cfg(not(windows))]
fn candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    vec![dir.join(name)]
}

#[cfg(windows)]
fn candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    if Path::new(name).extension().is_some() {
        return vec![dir.join(name)];
    }
    let exts = std::env::var("PATHEXT").ok().filter(|e| !e.is_empty()).unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
    exts.split(';').filter(|e| !e.is_empty()).map(|e| dir.join(format!("{name}{}", e.to_lowercase()))).collect()
}

/// The `claude.exe` an npm `claude.cmd` shim starts, when it is there.
fn npm_target(found: &Path) -> Option<PathBuf> {
    let ext = found.extension()?.to_string_lossy().to_lowercase();
    if ext != "cmd" && ext != "bat" {
        return None;
    }
    let stem = found.file_stem()?.to_string_lossy().into_owned();
    let exe = found.parent()?.join("node_modules/@anthropic-ai/claude-code/bin").join(format!("{stem}.exe"));
    (stem == "claude" && exe.is_file()).then_some(exe)
}

/// Newest `~/.localtools/tools/claude-code/<version>/claude`, else None (ACME).
fn localtools_claude(home: &Path) -> Option<String> {
    let dir = home.join(".localtools/tools/claude-code");
    let mut vs: Vec<(Vec<u64>, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join("claude").is_file())
        .map(|p| {
            let v = p
                .file_name()
                .map(|n| n.to_string_lossy().split('.').filter_map(|x| x.parse().ok()).collect())
                .unwrap_or_default();
            (v, p.join("claude"))
        })
        .collect();
    vs.sort();
    vs.pop().map(|(_, p)| p.to_string_lossy().into_owned())
}

/// The system host name (`os::hostname`), lowercased, without the domain part.
fn local_hostname() -> String {
    crate::os::hostname().to_lowercase().split('.').next().unwrap_or("").to_string()
}

impl Env {
    /// Build from variables (the process env in normal use; a map in tests).
    pub fn from_vars(v: &HashMap<String, String>) -> Env {
        let get = |k: &str| v.get(k).filter(|s| !s.is_empty()).cloned();
        let home = home_dir(v);
        let herdr_bin = if v.get("HERDR_ENV").map(|s| s == "1").unwrap_or(false) {
            get("NIMITZ_HERDR_BIN").or_else(|| which("herdr"))
        } else {
            None
        };
        let nimitz_home = expand(&get("NIMITZ_HOME").unwrap_or("~/.claude-nimitz".into()), &home);
        let state = expand(&get("NIMITZ_STATE").unwrap_or("~/.local/state/nimitz".into()), &home);
        let hostname = get("NIMITZ_HOSTNAME").map(|h| h.to_lowercase()).unwrap_or_else(local_hostname);
        Env {
            claude_home: expand(&get("NIMITZ_CLAUDE_HOME").unwrap_or("~/.claude".into()), &home),
            // the cold-launch binary: NIMITZ_CLAUDE_BIN, else this machine's
            // `claude_bin` (machines.toml, #12), else PATH's
            claude_bin: get("NIMITZ_CLAUDE_BIN")
                .or_else(|| crate::provider::claude_bin_for(&nimitz_home, &state, &hostname))
                .or_else(|| which("claude"))
                .unwrap_or("claude".into()),
            home: nimitz_home,
            state,
            native_claude_bin: get("NIMITZ_NATIVE_CLAUDE_BIN")
                .or_else(|| localtools_claude(&home))
                .or_else(|| which("claude"))
                .unwrap_or("claude".into()),
            herdr_bin,
            hostname,
            mux: get("NIMITZ_MUX"),
            crash_window_s: get("NIMITZ_CRASH_WINDOW").and_then(|v| v.parse().ok()).unwrap_or(20.0),
        }
    }

    pub fn from_process() -> Env {
        Env::from_vars(&crate::os::env_vars())
    }

    /// Does `hostname` (full or short, any case) name this machine?
    pub fn matches_hostname(&self, hostname: &str) -> bool {
        let h = hostname.to_lowercase();
        let short = h.split('.').next().unwrap_or("");
        !h.is_empty() && (h == self.hostname || short == self.hostname)
    }

    /// `NIMITZ_STATE/registry.json`.
    pub fn registry_path(&self) -> PathBuf {
        self.state.join("registry.json")
    }
    /// `NIMITZ_STATE/registry.lock`.
    pub fn registry_lock_path(&self) -> PathBuf {
        self.state.join("registry.lock")
    }
    /// `NIMITZ_STATE/events.jsonl`.
    pub fn events_path(&self) -> PathBuf {
        self.state.join("events.jsonl")
    }
    /// `NIMITZ_HOME/<fleet>/` (fleet names are uppercase; folders lowercase).
    pub fn fleet_dir(&self, fleet: &str) -> PathBuf {
        self.home.join(fleet.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn defaults_resolve_under_home() {
        let e = Env::from_vars(&vars(&[("HOME", "/tmp/fakehome"), ("NIMITZ_HOSTNAME", "west-mac")]));
        assert_eq!(e.home, PathBuf::from("/tmp/fakehome/.claude-nimitz"));
        assert_eq!(e.state, PathBuf::from("/tmp/fakehome/.local/state/nimitz"));
        assert_eq!(e.claude_home, PathBuf::from("/tmp/fakehome/.claude"));
        assert_eq!(e.hostname, "west-mac");
        assert_eq!(e.mux, None);
    }

    #[test]
    fn overrides_win() {
        let e = Env::from_vars(&vars(&[
            ("HOME", "/tmp/fakehome"),
            ("NIMITZ_HOME", "/data/nimitz"),
            ("NIMITZ_STATE", "/data/state"),
            ("NIMITZ_CLAUDE_HOME", "~/cchome"),
            ("NIMITZ_CLAUDE_BIN", "/bin/fake-claude"),
            ("NIMITZ_NATIVE_CLAUDE_BIN", "/bin/native-claude"),
            ("NIMITZ_HOSTNAME", "Box.Example.COM"),
            ("NIMITZ_MUX", "none"),
        ]));
        assert_eq!(e.home, PathBuf::from("/data/nimitz"));
        assert_eq!(e.state, PathBuf::from("/data/state"));
        assert_eq!(e.claude_home, PathBuf::from("/tmp/fakehome/cchome"));
        assert_eq!(e.claude_bin, "/bin/fake-claude");
        assert_eq!(e.native_claude_bin, "/bin/native-claude");
        assert_eq!(e.hostname, "box.example.com");
        assert_eq!(e.mux, Some("none".into()));
    }

    #[test]
    fn home_falls_back_to_userprofile_on_windows_and_tilde_expands() {
        let h = home_dir(&vars(&[("USERPROFILE", r"C:\Users\me")]));
        if cfg!(windows) {
            assert_eq!(h, PathBuf::from(r"C:\Users\me"));
        } else {
            assert_ne!(h, PathBuf::from(r"C:\Users\me"), "USERPROFILE is Windows' only");
        }
        let home = Path::new("/h");
        assert_eq!(expand("~/proj", home), PathBuf::from("/h/proj"));
        assert_eq!(expand("~", home), PathBuf::from("/h"));
        assert_eq!(expand("/srv/x", home), PathBuf::from("/srv/x"));
        let back = expand(r"~\proj", home);
        assert_eq!(back, if cfg!(windows) { home.join("proj") } else { PathBuf::from(r"~\proj") });
    }

    #[test]
    fn empty_var_means_unset() {
        let e = Env::from_vars(&vars(&[("HOME", "/tmp/h"), ("NIMITZ_HOME", ""), ("NIMITZ_HOSTNAME", "m")]));
        assert_eq!(e.home, PathBuf::from("/tmp/h/.claude-nimitz"));
    }

    #[test]
    fn herdr_bin_only_inside_herdr() {
        let outside =
            Env::from_vars(&vars(&[("HOME", "/tmp/h"), ("NIMITZ_HERDR_BIN", "/bin/h"), ("NIMITZ_HOSTNAME", "m")]));
        assert_eq!(outside.herdr_bin, None);
        let inside = Env::from_vars(&vars(&[
            ("HOME", "/tmp/h"),
            ("HERDR_ENV", "1"),
            ("NIMITZ_HERDR_BIN", "/bin/h"),
            ("NIMITZ_HOSTNAME", "m"),
        ]));
        assert_eq!(inside.herdr_bin, Some("/bin/h".into()));
    }

    #[test]
    fn hostname_matching() {
        let e = Env::from_vars(&vars(&[("HOME", "/tmp/h"), ("NIMITZ_HOSTNAME", "west-mac")]));
        assert!(e.matches_hostname("west-mac"));
        assert!(e.matches_hostname("WEST-MAC"));
        assert!(e.matches_hostname("west-mac.local"));
        assert!(!e.matches_hostname("home-server"));
        assert!(!e.matches_hostname(""));
    }

    #[test]
    fn hostname_falls_back_to_gethostname() {
        let e = Env::from_vars(&vars(&[("HOME", "/tmp/h")]));
        assert!(!e.hostname.is_empty());
        assert!(!e.hostname.contains('.'));
        assert_eq!(e.hostname, e.hostname.to_lowercase());
    }

    #[test]
    fn canon_resolves_symlinks_and_keeps_missing_tails() {
        let td = tempfile::tempdir().unwrap();
        let real = crate::os::canonical(&td.path()).unwrap().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = td.path().join("link");
        crate::os::symlink(&real, &link).unwrap();
        assert_eq!(canon(&link), real);
        assert_eq!(canon(&link.join("not/yet")), real.join("not/yet"));
    }

    #[test]
    fn trust_inside_a_git_repo_stops_at_its_root() {
        let td = tempfile::tempdir().unwrap();
        let base = crate::os::canonical(&td.path()).unwrap();
        let repo = base.join("home/repo");
        let wt = base.join("home/.cache/wt");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: /elsewhere\n").unwrap();
        assert_eq!(git_root(&repo.join("sub")), Some(repo.clone()));
        assert_eq!(trust_keys(&repo.join("sub")), vec![repo.join("sub"), repo.clone()]);
        assert_eq!(trust_keys(&wt), vec![wt.clone()], "a worktree (.git file) is a root too");
        let plain = base.join("home/plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(trust_keys(&plain).contains(&base.join("home")), "outside git every parent counts");
    }

    #[test]
    fn state_paths_and_fleet_dir() {
        let e = Env::from_vars(&vars(&[("HOME", "/tmp/h"), ("NIMITZ_STATE", "/s"), ("NIMITZ_HOSTNAME", "m")]));
        assert_eq!(e.registry_path(), PathBuf::from("/s/registry.json"));
        assert_eq!(e.registry_lock_path(), PathBuf::from("/s/registry.lock"));
        assert_eq!(e.events_path(), PathBuf::from("/s/events.jsonl"));
        assert_eq!(e.fleet_dir("MAIN"), PathBuf::from("/tmp/h/.claude-nimitz/main"));
    }
}
