//! The operating system facts nimitz reads: process parents, liveness and
//! executables, the host name, file identity, writability and detached
//! children. One function per fact, one body per platform; callers stay
//! platform-free. File locks are std's (`File::lock`, `File::try_lock`):
//! `flock` on Unix, `LockFileEx` on Windows, and nimitz only ever locks
//! dedicated `.lock` files, so Windows' mandatory byte-range locks block
//! no reader.
use std::fs::File;
use std::path::Path;
use std::process::Command;

/// The process environment as a map. Windows variable names are
/// case-insensitive, and a process may get `Path` (cmd.exe does) where nimitz
/// looks up `PATH`: there every name is upper-cased (the first of two that
/// differ only in case wins, as in Windows' own lookup).
pub fn env_vars() -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for (k, v) in std::env::vars() {
        let k = if cfg!(windows) { k.to_uppercase() } else { k };
        out.entry(k).or_insert(v);
    }
    out
}

/// The pid of this process's parent.
pub fn parent_pid() -> i64 {
    #[cfg(unix)]
    {
        (unsafe { libc::getppid() }) as i64
    }
    #[cfg(windows)]
    {
        parent_of(std::process::id() as i64).unwrap_or(0)
    }
}

/// A process's parent pid. Linux: `/proc/<pid>/stat` field 4 (after the
/// `(comm)`, which may hold spaces and parentheses). Windows: the process
/// table, and only a parent that started before the child (Windows keeps a
/// dead parent's pid in the child, and reuses pids). None for a dead pid and
/// on other systems.
pub fn parent_of(pid: i64) -> Option<i64> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let rest = &stat[stat.rfind(')')? + 1..];
        rest.split_whitespace().nth(1)?.parse().ok()
    }
    #[cfg(windows)]
    {
        win::parent_of(pid)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = pid;
        None
    }
}

/// The process ids above this one (its parent, its parent's parent, ...);
/// empty where `parent_of` reads nothing.
pub fn ancestors() -> Vec<i64> {
    let mut out = vec![];
    let mut pid = std::process::id() as i64;
    for _ in 0..64 {
        match parent_of(pid) {
            Some(ppid) if ppid > 0 && !out.contains(&ppid) => {
                out.push(ppid);
                pid = ppid;
            }
            _ => break,
        }
    }
    out
}

/// Is `pid` a live process (one this user may not signal counts)?
pub fn pid_alive(pid: i64) -> bool {
    if pid <= 0 || pid > i32::MAX as i64 {
        return false;
    }
    #[cfg(unix)]
    {
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        win::pid_alive(pid)
    }
}

/// When process `pid` started (epoch ms): its creation time on Windows
/// (`GetProcessTimes`). None elsewhere (Unix keeps the bare pid check) and
/// for a pid that cannot be opened.
pub fn process_started_ms(pid: i64) -> Option<i64> {
    #[cfg(windows)]
    {
        win::started_ms(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        None
    }
}

/// When process `pid` started (epoch ms), wherever it can be read: the
/// creation time on Windows; on Linux the boot time (`/proc/stat` `btime`)
/// plus its start ticks. None elsewhere and for a pid that cannot be read.
pub fn process_birth_ms(pid: i64) -> Option<i64> {
    #[cfg(windows)]
    {
        win::started_ms(pid)
    }
    #[cfg(target_os = "linux")]
    {
        let btime: i64 = std::fs::read_to_string("/proc/stat").ok()?.lines().find_map(|l| l.strip_prefix("btime ")).and_then(|v| v.trim().parse().ok())?;
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let hz = if hz > 0 { hz as i64 } else { 100 };
        Some(btime * 1000 + start_ticks(pid)? as i64 * 1000 / hz)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

/// When process `pid` started, in clock ticks since boot (`/proc/<pid>/stat`
/// field 22, what Claude Code writes as a session file's `procStart` on
/// Linux). None elsewhere and for a pid that cannot be read.
pub fn start_ticks(pid: i64) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // the name (field 2) is in parentheses and may hold spaces: count from after it
        stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// How much later than a session file says its process started the process
/// may have been created and still be that process (clock steps, rounding).
const SAME_PROCESS_SLACK_MS: i64 = 2000;

/// A process created at `created_ms` can be the one a record says started
/// at `started_ms`: it was not created after (beyond the slack). Unknown
/// times prove nothing, so they pass.
pub fn started_in_time(created_ms: Option<i64>, started_ms: Option<i64>) -> bool {
    match (created_ms, started_ms) {
        (Some(c), Some(s)) => c <= s + SAME_PROCESS_SLACK_MS,
        _ => true,
    }
}

/// Is `pid` alive and the process a record (a session file's `startedAt` or
/// `procStart`, epoch ms) names? Windows reuses pids quickly: a live pid
/// whose process was created after the record says it started is another
/// process. Where the creation time is not read (Unix, a process this user
/// cannot open, the sandbox's always-alive `LIVE_PID`, whose fake sessions
/// may predate the boot) it is `pid_alive` alone.
pub fn is_same_process(pid: i64, started_ms: Option<i64>) -> bool {
    if !pid_alive(pid) {
        return false;
    }
    pid == crate::sandbox::LIVE_PID || started_in_time(process_started_ms(pid), started_ms)
}

/// The executable process `pid` runs: `/proc/<pid>/exe` on Linux,
/// `proc_pidpath` on macOS, `QueryFullProcessImageNameW` on Windows.
pub fn exe_of(pid: i64) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/exe")).ok().map(|e| e.to_string_lossy().into_owned())
    }
    #[cfg(target_os = "macos")]
    {
        let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let n = unsafe { libc::proc_pidpath(pid as libc::c_int, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as u32) };
        (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
    }
    #[cfg(windows)]
    {
        win::exe_of(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = pid;
        None
    }
}

/// The other running processes whose executable is named one of `names`
/// (`nimitz`, `admiralty`): (pid, executable as the system gives it, with
/// Linux's ` (deleted)` when the file was replaced since the process
/// started). Only this user's processes. Linux only (`/proc`); elsewhere none: macOS and Windows would
/// need a process walk (`proc_listallpids`, a toolhelp snapshot) that
/// nothing else in nimitz needs, so `doctor` does not look there.
pub fn running_named(names: &[&str]) -> Vec<(i64, String)> {
    #[cfg(target_os = "linux")]
    {
        let me = i64::from(std::process::id());
        let Ok(rd) = std::fs::read_dir("/proc") else { return vec![] };
        let mut out: Vec<(i64, String)> = rd
            .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<i64>().ok())
            .filter(|pid| *pid != me && owned_by_me(Path::new(&format!("/proc/{pid}"))))
            .filter_map(|pid| Some((pid, exe_of(pid)?)))
            .filter(|(_, exe)| Path::new(exe.trim_end_matches(DELETED)).file_name().and_then(|n| n.to_str()).is_some_and(|n| names.contains(&n)))
            .collect();
        out.sort();
        out
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = names;
        vec![]
    }
}

/// Does `running_named` look at the processes on this OS (`/proc`)?
pub const SCANS_PROCESSES: bool = cfg!(target_os = "linux");

/// A running nimitz or Admiralty older than the one installed (`stale_exe`).
#[derive(Clone, Debug, PartialEq)]
pub struct Stale {
    pub pid: i64,
    pub name: &'static str,
    /// its executable was replaced or removed after it started
    pub replaced: bool,
    pub why: String,
}

impl Stale {
    /// `pid 176935 (admiralty, executable replaced)`
    pub fn short(&self) -> String {
        format!("pid {} ({}, {})", self.pid, self.name, if self.replaced { "executable replaced" } else { "an older version" })
    }
}

/// The running nimitz and Admiralty processes that run older code than
/// this one (issue #1: an old core reads a newer registry as corrupt and
/// rebuilds it in its old shape): `stale_exe` against the binaries beside
/// this process's own executable and this build's version, each other
/// executable asked its version once (`version_at`). None where
/// `SCANS_PROCESSES` is false.
/// A scan is reused for 10 s: Admiralty runs `check` on every refresh, and
/// a process quit or started since is seen at the next scan. Unit tests scan
/// each time, as they start the processes they look for.
pub fn stale_processes() -> Vec<Stale> {
    #[cfg(test)]
    {
        scan_stale()
    }
    #[cfg(not(test))]
    {
        static SCAN: std::sync::Mutex<Option<(std::time::Instant, Vec<Stale>)>> = std::sync::Mutex::new(None);
        let mut last = SCAN.lock().unwrap_or_else(|e| e.into_inner());
        match last.as_ref() {
            Some((at, found)) if at.elapsed() < std::time::Duration::from_secs(10) => found.clone(),
            _ => {
                let found = scan_stale();
                *last = Some((std::time::Instant::now(), found.clone()));
                found
            }
        }
    }
}

fn scan_stale() -> Vec<Stale> {
    let Ok(me) = std::env::current_exe() else { return vec![] };
    let mut asked: std::collections::HashMap<std::path::PathBuf, Option<String>> = Default::default();
    let mut version_of = |p: &Path| asked.entry(p.to_path_buf()).or_insert_with(|| version_at(p)).clone();
    let mut out = vec![];
    for name in ["nimitz", "admiralty"] {
        let installed = me.with_file_name(exe_name(name));
        for (pid, exe) in running_named(&[name]) {
            if let Some(why) = stale_exe(&exe, &installed, env!("CARGO_PKG_VERSION"), &mut version_of) {
                out.push(Stale { pid, name, replaced: exe.ends_with(DELETED), why });
            }
        }
    }
    out
}

/// The nimitz version of the build at `exe`: `nimitz --version` (`nimitz
/// X.Y.Z`), the nimitz itself or, for an Admiralty (it has no `--version`,
/// and an older one would start its TUI), the nimitz installed beside it.
/// Run with an empty environment and given 2 s; None when it does not answer.
pub fn version_at(exe: &Path) -> Option<String> {
    let probe = if exe.file_stem().is_some_and(|n| n == "nimitz") { exe.to_path_buf() } else { exe.with_file_name(exe_name("nimitz")) };
    // never another user's binary
    if !probe.is_file() || !owned_by_me(&probe) {
        return None;
    }
    let mut child = Command::new(&probe).arg("--version").env_clear().stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
    // the output within the limit, read on a thread: a child of the probe
    // that keeps its stdout open never holds doctor past it
    let mut out = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut out, &mut text);
        let _ = tx.send(text);
    });
    let text = rx.recv_timeout(std::time::Duration::from_secs(2)).ok();
    if !matches!(child.try_wait(), Ok(Some(_))) {
        let _ = child.kill();
    }
    let _ = child.wait();
    text?.trim().strip_prefix("nimitz ").map(|v| v.trim().to_string()).filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()))
}

/// Is `p` (a file, or a `/proc/<pid>` folder: its process) this user's?
/// Elsewhere than Unix: yes (nothing there probes another user's binaries).
fn owned_by_me(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(p).is_ok_and(|m| m.uid() == unsafe { libc::geteuid() })
    }
    #[cfg(not(unix))]
    {
        let _ = p;
        true
    }
}

/// What Linux appends to `/proc/<pid>/exe` once the file is gone.
const DELETED: &str = " (deleted)";

/// Why a process running `exe` (`running_named`) runs older code than
/// this nimitz (`mine`, its version): its file was replaced or removed since
/// it started (`cargo install` renames the new one over it), or it is
/// another file whose version (`version_of`) is lower. None when it runs
/// `installed`, the same version (another build of it, a test worktree's),
/// a newer one, or one that does not say.
pub fn stale_exe(exe: &str, installed: &Path, mine: &str, version_of: &mut dyn FnMut(&Path) -> Option<String>) -> Option<String> {
    if let Some(was) = exe.strip_suffix(DELETED) {
        return Some(format!("its executable {was} was replaced or removed after it started"));
    }
    let p = Path::new(exe);
    let same = match (canonical(p), canonical(installed)) {
        (Ok(a), Ok(b)) => a == b,
        _ => p == installed,
    };
    if same {
        return None;
    }
    let v = version_of(p)?;
    crate::compat::older(&v, mine).then(|| format!("it runs {exe}, nimitz {v}, older than this nimitz {mine}"))
}

/// The machine's host name as the system gives it (any case, maybe with a
/// domain): `gethostname()`; on Windows the DNS host name.
pub fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        unsafe {
            if libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) == 0 {
                let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                String::from_utf8_lossy(&buf[..end]).into_owned()
            } else {
                String::new()
            }
        }
    }
    #[cfg(windows)]
    {
        win::hostname().or_else(|| std::env::var("COMPUTERNAME").ok()).unwrap_or_default()
    }
}

/// What tells one file from another at the same path (a rename over it, a
/// rotation): the inode on Unix, the volume file index on Windows.
pub fn file_id(f: &File) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        f.metadata().map(|m| m.ino()).unwrap_or(0)
    }
    #[cfg(windows)]
    {
        win::file_id(f)
    }
}

/// May this user write `p`? Unix: `access(W_OK)`. Windows has no such
/// answer for a folder (its read-only flag means nothing there): a folder is
/// writable when a probe file can be made in it, a file when it opens for
/// writing.
pub fn writable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        let Ok(c) = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()) else { return false };
        unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
    }
    #[cfg(windows)]
    {
        if p.is_dir() {
            let probe = p.join(format!(".nimitz-write-probe-{}", std::process::id()));
            let ok = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe).is_ok();
            let _ = std::fs::remove_file(&probe);
            ok
        } else {
            std::fs::OpenOptions::new().append(true).open(p).is_ok()
        }
    }
}

/// Start `c` outside the caller's lifetime: its own session on Unix (Claude
/// Code may kill the hook's process group when the hook returns); on Windows
/// with a hidden console of its own, in a new process group, and out of the
/// caller's job when the job allows it.
pub fn detach(c: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            c.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(win::DETACH_FLAGS | if win::may_break_away() { win::CREATE_BREAKAWAY_FROM_JOB } else { 0 });
    }
}

/// Start `c` and let it outlive the caller and anything that ends the
/// caller's process tree (the self-end helper, docs/design/cli-ux.md
/// section 4): on Unix its own session and a double fork (the grandchild
/// runs `c`; the child that forked it is reaped here at once), so a kill of
/// the caller's process group or session never reaches it; on Windows a
/// hidden console of its own (`detach`), a new process group, out of the
/// caller's job when the job allows it.
pub fn spawn_detached(c: &mut Command) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            c.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                match libc::fork() {
                    -1 => Err(std::io::Error::last_os_error()),
                    0 => Ok(()),
                    _ => libc::_exit(0),
                }
            });
        }
        let mut child = c.spawn()?;
        child.wait().map(|_| ())
    }
    #[cfg(windows)]
    {
        detach(c);
        c.spawn().map(|_| ())
    }
}

/// Does a detached child outlive the caller's job object? Always on Unix; on
/// Windows only when the job allows breakaway (else a kill of the job takes it).
pub fn detach_outlives_job() -> bool {
    #[cfg(windows)]
    {
        win::may_break_away()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// Make `p` executable (a script nimitz writes); a no-op on Windows.
pub fn make_executable(p: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755))
    }
    #[cfg(windows)]
    {
        let _ = p;
        Ok(())
    }
}

/// `p` with its last part as the folder spells it: a case-insensitive file
/// system (Windows, macOS by default) opens `bad/` for a folder named
/// `Bad/`. `p` itself when nothing at its parent matches.
pub fn name_on_disk(p: &Path) -> std::path::PathBuf {
    let (Some(parent), Some(name)) = (p.parent(), p.file_name()) else { return p.to_path_buf() };
    let name = name.to_string_lossy();
    let Ok(rd) = std::fs::read_dir(parent) else { return p.to_path_buf() };
    let names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    if names.iter().any(|n| *n == name) {
        return p.to_path_buf();
    }
    names.into_iter().find(|n| n.eq_ignore_ascii_case(&name)).map(|n| parent.join(n)).unwrap_or_else(|| p.to_path_buf())
}

/// `std::fs::canonicalize`, but on Windows in the plain form (`C:\x`, not
/// `\\?\C:\x`): the form users type, other programs take, and nimitz
/// compares paths in.
pub fn canonical(p: impl AsRef<Path>) -> std::io::Result<std::path::PathBuf> {
    let c = std::fs::canonicalize(p)?;
    #[cfg(windows)]
    {
        let s = c.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return Ok(std::path::PathBuf::from(format!(r"\\{rest}")));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            if rest.as_bytes().get(1) == Some(&b':') && rest.len() < 260 {
                return Ok(std::path::PathBuf::from(rest));
            }
        }
    }
    Ok(c)
}

/// The editor nobody chose: vi, notepad on Windows.
pub const DEFAULT_EDITOR: &str = if cfg!(windows) { "notepad" } else { "vi" };

/// `$VISUAL`, else `$EDITOR`, else `DEFAULT_EDITOR`, and the command that
/// opens `path` in it.
pub fn editor(path: &Path) -> (String, Command) {
    let e = ["VISUAL", "EDITOR"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty())).unwrap_or_else(|| DEFAULT_EDITOR.into());
    let c = editor_command(&e, path);
    (e, c)
}

/// `editor` (which may carry its own arguments: `code -w`) on `path`. Unix:
/// through `sh -c`, as a shell would run `$EDITOR`. Windows: split on
/// spaces, the program found on PATH with `PATHEXT` (so `code` is
/// `code.cmd`).
pub fn editor_command(editor: &str, path: &Path) -> Command {
    #[cfg(unix)]
    {
        let mut c = Command::new("sh");
        c.arg("-c").arg(format!("{editor} \"$1\"")).arg("sh").arg(path);
        c
    }
    #[cfg(windows)]
    {
        let mut parts = editor.split_whitespace();
        let prog = parts.next().unwrap_or(DEFAULT_EDITOR);
        let mut c = Command::new(crate::env::which(prog).unwrap_or_else(|| prog.to_string()));
        c.args(parts).arg(path);
        c
    }
}

/// `name` as this system names a program file (`claude` -> `claude.exe` on Windows).
pub fn exe_name(name: &str) -> String {
    let suffix = std::env::consts::EXE_SUFFIX;
    if suffix.is_empty() || name.to_ascii_lowercase().ends_with(suffix) {
        name.to_string()
    } else {
        format!("{name}{suffix}")
    }
}

/// A program at `path` that runs `exe` with `prefix` before its own
/// arguments (the sandbox's fake `claude` and `herdr`). Unix: a `#!/bin/sh`
/// script. Windows runs no such script: `path.exe` is a hard link to (else
/// a copy of) `exe`, and `path.shim.json` holds the prefix, which `exe`
/// puts back in front at start (`shim_args`). Returns the program's path.
pub fn write_shim(path: &Path, exe: &Path, prefix: &[String]) -> std::io::Result<std::path::PathBuf> {
    #[cfg(unix)]
    {
        let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
        let args: Vec<String> = std::iter::once(q(&exe.to_string_lossy())).chain(prefix.iter().map(|a| q(a))).collect();
        std::fs::write(path, format!("#!/bin/sh\nexec {} \"$@\"\n", args.join(" ")))?;
        make_executable(path)?;
        Ok(path.to_path_buf())
    }
    #[cfg(windows)]
    {
        let prog = path.with_extension("exe");
        let _ = std::fs::remove_file(&prog);
        if std::fs::hard_link(exe, &prog).is_err() {
            std::fs::copy(exe, &prog)?;
        }
        std::fs::write(path.with_extension("shim.json"), serde_json::json!({ "args": prefix }).to_string())?;
        Ok(prog)
    }
}

/// The prefix `write_shim` left for this program (`<exe>.shim.json` beside
/// it); empty for a program that is no shim, and always on Unix (the script
/// passes it as arguments).
pub fn shim_args() -> Vec<String> {
    #[cfg(windows)]
    {
        let Ok(me) = std::env::current_exe() else { return vec![] };
        let read = || -> Option<Vec<String>> {
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(me.with_extension("shim.json")).ok()?).ok()?;
            Some(v.get("args")?.as_array()?.iter().filter_map(|a| a.as_str().map(String::from)).collect())
        };
        read().unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        vec![]
    }
}

/// This program's arguments (without argv0), behind a shim's prefix.
pub fn args() -> Vec<String> {
    let mut a = shim_args();
    a.extend(std::env::args().skip(1));
    a
}

/// Fill `buf` from the OS's random source (`/dev/urandom`; Windows:
/// `BCryptGenRandom` with the system's preferred RNG). An error, never a
/// guessable fallback.
pub fn random(buf: &mut [u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Read;
        File::open("/dev/urandom")?.read_exact(buf)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
        let rc = unsafe { BCryptGenRandom(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
        if rc == 0 { Ok(()) } else { Err(std::io::Error::other(format!("BCryptGenRandom failed: {rc:#x}"))) }
    }
}

/// Rename `from` over `to`. On Windows a rename fails while another process
/// holds `to` open without delete sharing (a scanner, an indexer, a reader
/// mid-read): retried briefly (5 x 50 ms) before it gives up.
pub fn replace(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let mut last = None;
        for _ in 0..5 {
            match std::fs::rename(from, to) {
                Ok(()) => return Ok(()),
                // ERROR_ACCESS_DENIED (5), ERROR_SHARING_VIOLATION (32)
                Err(e) if matches!(e.raw_os_error(), Some(5) | Some(32)) => {
                    last = Some(e);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| std::io::Error::other("rename failed")))
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)
    }
}

/// A symbolic link at `link` to `target` (tests; a Windows file link needs
/// developer mode or the privilege, so callers there may skip on error).
pub fn symlink(target: impl AsRef<Path>, link: impl AsRef<Path>) -> std::io::Result<()> {
    let (target, link) = (target.as_ref(), link.as_ref());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        }
    }
}

#[cfg(windows)]
pub(crate) mod win {
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, FILETIME, HANDLE, INVALID_HANDLE_VALUE, STILL_ACTIVE};
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::JobObjects::{IsProcessInJob, JobObjectExtendedLimitInformation, QueryInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_BREAKAWAY_OK};
    use windows_sys::Win32::System::SystemInformation::{ComputerNameDnsHostname, GetComputerNameExW};
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    pub use windows_sys::Win32::System::Threading::CREATE_BREAKAWAY_FROM_JOB;
    /// A hidden console of its own rather than none (`DETACHED_PROCESS`):
    /// every console program the child starts (the herdr CLI) shares it, where
    /// with no console each would open, and flash, a window of its own.
    pub const DETACH_FLAGS: u32 = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;

    /// An open process handle, closed on drop.
    struct Proc(HANDLE);
    impl Drop for Proc {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn open(pid: i64) -> Result<Proc, u32> {
        if pid <= 0 || pid > u32::MAX as i64 {
            return Err(0);
        }
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32) };
        if h.is_null() {
            Err(unsafe { GetLastError() })
        } else {
            Ok(Proc(h))
        }
    }

    fn created(p: &Proc) -> Option<u64> {
        let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut c, mut e, mut k, mut u) = (z, z, z, z);
        (unsafe { GetProcessTimes(p.0, &mut c, &mut e, &mut k, &mut u) } != 0).then(|| ((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64)
    }

    /// The process table's parent of `pid`.
    fn table_parent(pid: i64) -> Option<i64> {
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let snap = Proc(snap);
        let mut e: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = unsafe { Process32FirstW(snap.0, &mut e) } != 0;
        while ok {
            if e.th32ProcessID as i64 == pid {
                return Some(e.th32ParentProcessID as i64);
            }
            ok = unsafe { Process32NextW(snap.0, &mut e) } != 0;
        }
        None
    }

    /// The creation time of `pid` as epoch ms (a FILETIME counts 100 ns
    /// since 1601).
    pub fn started_ms(pid: i64) -> Option<i64> {
        let ft = created(&open(pid).ok()?)?;
        Some((ft / 10_000) as i64 - 11_644_473_600_000)
    }

    pub fn parent_of(pid: i64) -> Option<i64> {
        let ppid = table_parent(pid).filter(|p| *p > 0)?;
        // a parent that started after the child is a reused pid, not the parent
        if let (Ok(child), Ok(parent)) = (open(pid), open(ppid)) {
            if let (Some(c), Some(p)) = (created(&child), created(&parent)) {
                if p > c {
                    return None;
                }
            }
        } else if open(ppid).err().is_some_and(|e| e != ERROR_ACCESS_DENIED) {
            return None; // the parent is gone
        }
        Some(ppid)
    }

    pub fn pid_alive(pid: i64) -> bool {
        match open(pid) {
            Ok(p) => {
                let mut code = 0u32;
                (unsafe { GetExitCodeProcess(p.0, &mut code) } != 0) && code == STILL_ACTIVE as u32
            }
            Err(e) => e == ERROR_ACCESS_DENIED,
        }
    }

    pub fn exe_of(pid: i64) -> Option<String> {
        let p = open(pid).ok()?;
        let mut buf = vec![0u16; 32768];
        let mut n = buf.len() as u32;
        (unsafe { QueryFullProcessImageNameW(p.0, 0, buf.as_mut_ptr(), &mut n) } != 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
    }

    pub fn hostname() -> Option<String> {
        let mut n = 0u32;
        unsafe { GetComputerNameExW(ComputerNameDnsHostname, std::ptr::null_mut(), &mut n) };
        if n == 0 {
            return None;
        }
        let mut buf = vec![0u16; n as usize];
        (unsafe { GetComputerNameExW(ComputerNameDnsHostname, buf.as_mut_ptr(), &mut n) } != 0).then(|| String::from_utf16_lossy(&buf[..n as usize])).filter(|s| !s.is_empty())
    }

    pub fn file_id(f: &File) -> u64 {
        let mut i: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(f.as_raw_handle() as HANDLE, &mut i) } == 0 {
            return 0;
        }
        ((i.nFileIndexHigh as u64) << 32) | i.nFileIndexLow as u64
    }

    /// Outside any job, or in one that lets a child break away. (Asking for
    /// a breakaway the job forbids fails the whole spawn.)
    pub fn may_break_away() -> bool {
        let mut in_job = 0;
        if unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job) } == 0 || in_job == 0 {
            return false; // nothing to break away from
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            QueryInformationJobObject(std::ptr::null_mut(), JobObjectExtendedLimitInformation, &mut info as *mut _ as *mut _, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32, std::ptr::null_mut())
        } != 0;
        ok && info.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_BREAKAWAY_OK != 0
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn this_process_is_alive_and_a_dead_pid_is_not() {
        assert!(pid_alive(std::process::id() as i64));
        assert!(!pid_alive(0));
        assert!(!pid_alive(-3));
    }

    #[test]
    fn a_process_created_after_its_record_says_is_another() {
        assert!(started_in_time(Some(10_000), Some(10_000)));
        assert!(started_in_time(Some(5_000), Some(10_000)), "created before the session started");
        assert!(started_in_time(Some(12_000), Some(10_000)), "within the slack");
        assert!(!started_in_time(Some(12_001), Some(10_000)), "a reused pid");
        assert!(started_in_time(None, Some(10_000)) && started_in_time(Some(1), None), "unknown proves nothing");
        let me = std::process::id() as i64;
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        assert!(is_same_process(me, Some(now_ms)) && is_same_process(me, None));
        assert!(!is_same_process(0, Some(now_ms)) && !is_same_process(-3, None));
        // a record from long before this process: on Windows another process;
        // Unix keeps the bare pid check
        assert_eq!(is_same_process(me, Some(1_000)), !cfg!(windows));
        // the sandbox's always-alive pid is never judged by its creation time
        assert!(is_same_process(crate::sandbox::LIVE_PID, Some(1_000)));
        match process_started_ms(me) {
            Some(c) => assert!(cfg!(windows) && c <= now_ms && c > now_ms - 86_400_000, "{c}"),
            None => assert!(!cfg!(windows)),
        }
    }

    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn the_parent_heads_the_ancestors() {
        let up = ancestors();
        assert_eq!(up.first().copied(), Some(parent_pid()), "{up:?}");
        assert_eq!(parent_of(std::process::id() as i64), Some(parent_pid()));
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn the_executable_of_this_process_is_read() {
        let me = std::env::current_exe().unwrap();
        let got = exe_of(std::process::id() as i64).expect("this process's executable");
        assert_eq!(std::fs::canonicalize(&got).unwrap(), std::fs::canonicalize(&me).unwrap());
    }

    #[test]
    fn a_rename_over_a_file_changes_its_id() {
        let d = std::env::temp_dir().join(format!("nimitz-os-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (a, b) = (d.join("a"), d.join("b"));
        std::fs::write(&a, "1").unwrap();
        std::fs::write(&b, "2").unwrap();
        let before = file_id(&File::open(&a).unwrap());
        assert_ne!(before, 0);
        std::fs::rename(&b, &a).unwrap();
        assert_ne!(file_id(&File::open(&a).unwrap()), before);
        assert!(writable(&d) && writable(&a));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn env_names_are_upper_case_on_windows_only() {
        std::env::set_var("Nimitz_Os_Probe", "1");
        let v = env_vars();
        if cfg!(windows) {
            assert_eq!(v.get("NIMITZ_OS_PROBE").map(String::as_str), Some("1"), "cmd.exe passes `Path`; nimitz looks up `PATH`");
            assert!(v.keys().all(|k| *k == k.to_uppercase()));
        } else {
            assert_eq!(v.get("Nimitz_Os_Probe").map(String::as_str), Some("1"));
        }
        std::env::remove_var("Nimitz_Os_Probe");
    }

    /// Issue #1: an Admiralty or nimitz still running after `cargo install`
    /// replaced its binary runs the old code; `doctor` names it. Another
    /// path of a lower version is old code too; the same version at another
    /// path (a test worktree's build) never is.
    #[test]
    fn a_replaced_or_lower_version_executable_is_stale_and_the_same_version_is_not() {
        let installed = Path::new("/opt/nimitz/bin/nimitz");
        let ask = |v: Option<&'static str>| move |_: &Path| v.map(str::to_string);
        assert_eq!(stale_exe("/opt/nimitz/bin/nimitz", installed, "0.7.6", &mut ask(Some("0.3.1"))), None, "the installed binary");
        assert!(stale_exe("/old/nimitz", installed, "0.7.6", &mut ask(Some("0.3.1"))).is_some_and(|w| w.contains("nimitz 0.3.1, older than this nimitz 0.7.6")));
        assert_eq!(stale_exe("/ci/wt/target/debug/nimitz", installed, "0.7.6", &mut ask(Some("0.7.6"))), None, "the same version elsewhere");
        assert_eq!(stale_exe("/new/nimitz", installed, "0.7.6", &mut ask(Some("0.8.0"))), None, "a newer one");
        assert_eq!(stale_exe("/mute/nimitz", installed, "0.7.6", &mut ask(None)), None, "one that does not say");
        let gone = format!("{} (deleted)", installed.display());
        assert!(stale_exe(&gone, installed, "0.7.6", &mut ask(Some("0.7.6"))).is_some_and(|w| w.contains("replaced or removed")));
    }

    /// `version_at` asks the nimitz itself, or the nimitz beside an Admiralty
    /// (never the Admiralty: an old one would start its TUI).
    #[cfg(unix)]
    #[test]
    fn the_version_comes_from_nimitz_and_an_admiralty_s_nimitz() {
        let d = std::env::temp_dir().join(format!("nimitz-os-version-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (dir, v) in [("same", env!("CARGO_PKG_VERSION")), ("old", "0.3.1")] {
            std::fs::create_dir_all(d.join(dir)).unwrap();
            let n = d.join(dir).join("nimitz");
            std::fs::write(&n, format!("#!/bin/sh\necho nimitz {v}\n")).unwrap();
            make_executable(&n).unwrap();
            std::fs::write(d.join(dir).join("admiralty"), "#!/bin/sh\nsleep 30\n").unwrap();
            make_executable(&d.join(dir).join("admiralty")).unwrap();
        }
        // a script just written may be busy for a moment (another test's fork holds it): ask again
        let version_at = |p: &Path| (0..5).find_map(|_| version_at(p));
        assert_eq!(version_at(&d.join("same").join("nimitz")).as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(version_at(&d.join("old").join("admiralty")).as_deref(), Some("0.3.1"), "its nimitz answers");
        let installed = d.join("installed").join("nimitz");
        let mut ask = |p: &Path| version_at(p);
        for name in ["nimitz", "admiralty"] {
            let same = d.join("same").join(name);
            assert_eq!(stale_exe(&same.to_string_lossy(), &installed, env!("CARGO_PKG_VERSION"), &mut ask), None, "same version, another path: {name}");
            let old = d.join("old").join(name);
            assert!(stale_exe(&old.to_string_lossy(), &installed, env!("CARGO_PKG_VERSION"), &mut ask).is_some(), "{name}");
        }
        assert_eq!(version_at(&d.join("none").join("nimitz")), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The `/proc` walk finds a process by its executable's name, and a
    /// binary removed under it reads ` (deleted)`.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_running_process_whose_binary_was_replaced_is_found() {
        let d = std::env::temp_dir().join(format!("nimitz-os-running-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let exe = d.join("admiralty");
        std::fs::copy("/bin/sleep", &exe).unwrap();
        let mut child = spawn_fresh(&exe, "30");
        std::fs::remove_file(&exe).unwrap();
        let pid = i64::from(child.id());
        let found = running_named(&["admiralty"]).into_iter().find(|(p, _)| *p == pid);
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&d);
        let (_, at) = found.expect("the walk finds the child");
        assert!(at.ends_with(" (deleted)"), "{at}");
        assert!(stale_exe(&at, &exe, "0.7.6", &mut |_: &Path| None).is_some());
        assert!(!running_named(&["admiralty"]).iter().any(|(p, _)| *p == i64::from(std::process::id())));
    }

    /// Start a binary just written: another test's fork may hold it open for
    /// writing a moment (ETXTBSY), so try again.
    #[cfg(target_os = "linux")]
    pub fn spawn_fresh(exe: &Path, arg: &str) -> std::process::Child {
        for _ in 0..50 {
            match Command::new(exe).arg(arg).spawn() {
                Ok(c) => return c,
                Err(e) if e.raw_os_error() == Some(26) => std::thread::sleep(std::time::Duration::from_millis(20)),
                Err(e) => panic!("{}: {e}", exe.display()),
            }
        }
        panic!("{} stays busy", exe.display())
    }

    #[test]
    fn the_host_name_is_known() {
        assert!(!hostname().is_empty());
    }
}
