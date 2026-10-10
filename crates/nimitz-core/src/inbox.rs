//! Delivery without typing: a line into a session's inbox socket (Claude
//! Code's cross-session messaging, code.claude.com/docs/en/cross-session-messaging,
//! "a script or hook to post into a session"). An idle background job takes
//! it as a new turn; a busy one at its next. Independent of panes, tabs and
//! the agent view, and it works with nobody attached. It arrives as a peer
//! message, never as the user's own input: a notice, not an authority.
//!
//! The socket: Claude Code exports `CLAUDE_CODE_MESSAGING_SOCKET` to hooks
//! before SessionStart runs; the hook records it per session
//! (`NIMITZ_STATE/inbox/<session id>.json`). With no record,
//! `<XDG_RUNTIME_DIR or /run/user/<uid>>/cc-socks/<pid>.sock` or
//! `/tmp/cc-socks-<uid>/<pid>.sock`. Either way the listener must be the
//! session's own process: after connecting, its peer credentials
//! (`SO_PEERCRED`; on macOS `getpeereid` and `LOCAL_PEERPID`) must name
//! this uid and the session's pid; anything else (another session under a
//! reused or namespaced pid, a planted socket, a forged record) is not used.
//!
//! The line: the docs give only the optional auth line. The message line is
//! the inject example Claude Code itself logs (`[uds-messaging] Inject
//! messages`): `{"type":"user","message":{"role":"user","content":"..."}}`,
//! verified on `FORMAT_SINCE` (e2e `e2e_inbox_signal_reaches_an_idle_lead_without_a_pane`);
//! an older Claude Code gets the typing path. The content is wrapped as
//! Claude Code wraps a peer message, so the receiver names the sender
//! `nimitz` (`origin.name`; the address is not trusted, `origin.from` stays
//! unknown). The address is one nothing listens on: a reply goes nowhere.
use crate::env::Env;
use serde_json::{json, Value};
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The first Claude Code the message line was verified on.
pub const FORMAT_SINCE: &str = "2.1.293";

/// The `from` of the frame: a socket address no process listens on, so a
/// receiver that replies to it reaches nobody.
pub const NO_REPLY: &str = "uds:/nonexistent/nimitz-no-reply.sock";

const TIMEOUT: Duration = Duration::from_secs(5);

fn dir(env: &Env) -> PathBuf {
    env.state.join("inbox")
}

fn file_of(env: &Env, sid: &str) -> Option<PathBuf> {
    // a session id is a uuid; anything else names no file
    (!sid.is_empty() && sid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).then(|| dir(env).join(format!("{sid}.json")))
}

/// SessionStart: session `sid` (process `pid`) listens on `socket`. One file
/// per session, written through a per-process temp file: hooks of members
/// that start together never lose each other's record.
pub fn record(env: &Env, sid: &str, pid: i64, socket: &str) {
    let Some(f) = file_of(env, sid) else { return };
    if socket.is_empty() {
        return;
    }
    let _ = std::fs::create_dir_all(dir(env));
    let tmp = f.with_extension(format!("json.tmp{}", std::process::id()));
    if std::fs::write(&tmp, json!({"socket": socket, "pid": pid, "at": crate::evidence::now()}).to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &f);
    }
    // records of sessions whose socket is gone
    for e in std::fs::read_dir(dir(env)).into_iter().flatten().flatten() {
        let p = e.path();
        if p == f || p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let gone = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| record_gone(&v)).unwrap_or(false);
        if gone {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// The sockets session `sid` (live as `pid`) may listen on, most likely
/// first: its record, then `<pid>.sock` in Claude Code's socket directories.
/// None is trusted yet: `send` checks who listens.
pub fn candidates(env: &Env, sid: &str, pid: i64) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Some(r) = file_of(env, sid).and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
        if r.get("pid").and_then(Value::as_i64) == Some(pid) {
            if let Some(s) = r.get("socket").and_then(Value::as_str) {
                out.push(PathBuf::from(s));
            }
        }
    }
    for p in default_sockets(env, pid) {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out.retain(|p| listens(p));
    out
}

/// Unix: `<pid>.sock` in Claude Code's socket directories.
#[cfg(unix)]
fn default_sockets(_env: &Env, pid: i64) -> Vec<PathBuf> {
    let uid = unsafe { libc::getuid() };
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| format!("/run/user/{uid}"));
    [PathBuf::from(runtime).join("cc-socks"), PathBuf::from(format!("/tmp/cc-socks-{uid}"))].into_iter().map(|d| d.join(format!("{pid}.sock"))).collect()
}

/// Windows: the named pipe the session file names (`messagingSocketPath`;
/// the pipe name is random, nothing derives it from the pid).
#[cfg(windows)]
fn default_sockets(env: &Env, pid: i64) -> Vec<PathBuf> {
    let f = env.claude_home.join("sessions").join(format!("{pid}.json"));
    std::fs::read(f).ok().and_then(|b| crate::evidence::first_json(&b)).and_then(|v| v.get("messagingSocketPath").and_then(Value::as_str).map(PathBuf::from)).into_iter().collect()
}

/// Might something listen at `p`? A socket file exists; a Windows pipe
/// cannot be asked without taking one of its instances, so a pipe path
/// counts (`send` checks who listens).
fn listens(p: &Path) -> bool {
    is_pipe(p) || p.exists()
}

fn is_pipe(p: &Path) -> bool {
    let s = p.to_string_lossy();
    cfg!(windows) && (s.starts_with(r"\\.\pipe\") || s.starts_with("//./pipe/"))
}

/// Is a record's session gone? Its socket file is (Unix); for a pipe, its
/// process is.
fn record_gone(v: &Value) -> Option<bool> {
    let s = v.get("socket").and_then(Value::as_str)?;
    if is_pipe(Path::new(s)) {
        return Some(!v.get("pid").and_then(Value::as_i64).is_some_and(crate::os::pid_alive));
    }
    Some(!Path::new(s).exists())
}

/// The Claude Code version session `pid` runs (its session file), else
/// the job row's `cliVersion`.
pub fn receiver_version(env: &Env, pid: Option<i64>, job: Option<&str>) -> Option<String> {
    let read = |p: PathBuf, k: &str| std::fs::read(p).ok().and_then(|b| crate::evidence::first_json(&b)).and_then(|v| v.get(k).and_then(Value::as_str).map(String::from));
    pid.and_then(|p| read(env.claude_home.join("sessions").join(format!("{p}.json")), "version"))
        .or_else(|| job.and_then(|j| read(env.claude_home.join("jobs").join(j).join("state.json"), "cliVersion")))
}

/// May the line go to a receiver on `version`? Only one at or past `FORMAT_SINCE`.
pub fn format_ok(version: Option<&str>) -> bool {
    version.is_some_and(|v| crate::compat::at_least(v, FORMAT_SINCE))
}

/// Text that cannot open or close the frame, or pass for another frame: any
/// `<` that starts a `cross-session-message` tag (any case, `-`/`_`/spaces
/// between the words, a `/` or not) becomes `<\`, as Claude Code defuses
/// its own.
pub fn defuse(body: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    let at_ci = |at: usize| chars.get(at).map(|c| c.to_ascii_lowercase());
    let word = |at: &mut usize, w: &str| -> bool {
        for c in w.chars() {
            if at_ci(*at) != Some(c) {
                return false;
            }
            *at += 1;
        }
        true
    };
    // invisible formatting characters (zero-width space and joiners, word
    // joiner, BOM) count as spaces: they would hide a tag from the scan
    let invisible = |c: &char| matches!(*c, '\u{200b}'..='\u{200f}' | '\u{2060}'..='\u{2064}' | '\u{feff}' | '\u{00ad}');
    let skip = |at: &mut usize, also: &[char]| {
        while chars.get(*at).is_some_and(|c| c.is_whitespace() || invisible(c) || also.contains(c)) {
            *at += 1;
        }
    };
    let mut out = String::with_capacity(body.len());
    for (i, c) in chars.iter().enumerate() {
        if *c == '<' {
            let mut at = i + 1;
            skip(&mut at, &['/']);
            let tag = word(&mut at, "cross") && {
                skip(&mut at, &['-', '_']);
                word(&mut at, "session")
            } && {
                skip(&mut at, &['-', '_']);
                word(&mut at, "message")
            };
            if tag {
                out.push_str("<\\");
                continue;
            }
        }
        out.push(*c);
    }
    out
}

/// The content of the line: `body` (defused) wrapped as Claude Code wraps a
/// peer message, from `nimitz`, with no reply address.
pub fn frame(body: &str) -> String {
    format!("<cross-session-message from=\"{NO_REPLY}\" from-name=\"nimitz\">\n{}\n</cross-session-message>", defuse(body))
}

/// Post `body` (framed) to session `pid` through the first of `sockets` its
/// own process listens on: `Ok(socket)` once the line went in ("posted":
/// nothing acknowledges more), else why not.
pub fn send(env: &Env, sockets: &[PathBuf], pid: i64, body: &str) -> Result<PathBuf, String> {
    let line = json!({"type": "user", "message": {"role": "user", "content": frame(body)}}).to_string();
    let mut why = vec![];
    for sock in sockets {
        match post(env, sock, pid, &line) {
            Ok(()) => return Ok(sock.clone()),
            Err(Failed::NotThere(e)) => why.push(e),
            Err(Failed::Refused(e)) => return Err(e),
        }
    }
    Err(if why.is_empty() { "no inbox socket (no live process listens for it)".into() } else { why.join("; ") })
}

/// Why one socket took no line: the session's own process does not listen
/// there (try the next), or its listener refused the line.
enum Failed {
    NotThere(String),
    Refused(String),
}

/// Unix: the line, then a half close. The session closes the connection
/// once it read the line; Claude Code sends no acknowledgement, so a line it
/// refused after reading it (an auth it wants, a parse) closes just as
/// cleanly and cannot be told apart. A reset (unread data left, an oversized
/// line) fails.
#[cfg(unix)]
fn post(_env: &Env, sock: &Path, pid: i64, line: &str) -> Result<(), Failed> {
    let mut s = connect(sock, pid).map_err(Failed::NotThere)?;
    let _ = s.set_write_timeout(Some(TIMEOUT));
    let _ = s.set_read_timeout(Some(TIMEOUT));
    s.write_all(format!("{line}\n").as_bytes()).map_err(|e| Failed::Refused(format!("writing to {} failed: {e}", sock.display())))?;
    let _ = s.shutdown(std::net::Shutdown::Write);
    let mut back = Vec::new();
    s.read_to_end(&mut back).map(|_| ()).map_err(|e| Failed::Refused(format!("{} refused the line: {e}", sock.display())))
}

/// Windows: the inbox is a named pipe, and Claude Code there closes any
/// connection whose first line is not `{"type":"auth","token":...}` with the
/// session's peer token (`sessions/<pid>.<hash>.key`, readable by this user
/// only), delivering nothing. The pipe's server process must be the session's
/// `pid`. A pipe cannot half close and the session keeps an authenticated
/// connection open: the line is posted once the session read it
/// (`FlushFileBuffers`) and the pipe is still open a moment later; a refused
/// auth closes it.
#[cfg(windows)]
fn post(env: &Env, sock: &Path, pid: i64, line: &str) -> Result<(), Failed> {
    let token = peer_token(env, pid).ok_or_else(|| Failed::NotThere(format!("no peer token for process {pid} (sessions/{pid}.*.key)")))?;
    let (tx, rx) = std::sync::mpsc::channel();
    let (p, auth, line) = (sock.to_path_buf(), json!({"type": "auth", "token": token}).to_string(), line.to_string());
    std::thread::spawn(move || {
        let _ = tx.send(win::post(&p, pid, &auth, &line));
    });
    match rx.recv_timeout(TIMEOUT) {
        Ok(r) => r,
        Err(_) => Err(Failed::NotThere(format!("posting to {} took over {} s", sock.display(), TIMEOUT.as_secs()))),
    }
}

/// Session `pid`'s peer token: `sessions/<pid>.<hash>.key`, when the key
/// names the process start `sessions/<pid>.json` does (a reused pid is
/// another process).
#[cfg(windows)]
fn peer_token(env: &Env, pid: i64) -> Option<String> {
    let dir = env.claude_home.join("sessions");
    let started = std::fs::read(dir.join(format!("{pid}.json"))).ok().and_then(|b| crate::evidence::first_json(&b)).and_then(|v| v.get("procStart").and_then(Value::as_str).map(String::from));
    let prefix = format!("{pid}.");
    std::fs::read_dir(&dir).ok()?.flatten().find_map(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&prefix) || !name.ends_with(".key") {
            return None;
        }
        let v: Value = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
        if let (Some(k), Some(s)) = (v.get("procStartFt").and_then(Value::as_str), started.as_deref()) {
            if k != s {
                return None;
            }
        }
        v.get("peerToken").and_then(Value::as_str).filter(|t| !t.is_empty()).map(String::from)
    })
}

#[cfg(windows)]
mod win {
    use super::Failed;
    use std::io::Write;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{ERROR_PIPE_BUSY, HANDLE};
    use windows_sys::Win32::System::Pipes::{GetNamedPipeServerProcessId, PeekNamedPipe, WaitNamedPipeW};

    pub(super) fn post(p: &Path, pid: i64, auth: &str, line: &str) -> Result<(), Failed> {
        let at = || p.display().to_string();
        let open = || std::fs::OpenOptions::new().read(true).write(true).open(p);
        let mut f = match open() {
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                let wide: Vec<u16> = p.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
                unsafe { WaitNamedPipeW(wide.as_ptr(), 2000) };
                open()
            }
            r => r,
        }
        .map_err(|e| Failed::NotThere(format!("cannot connect to {}: {e}", at())))?;
        let h = f.as_raw_handle() as HANDLE;
        let mut server = 0u32;
        if unsafe { GetNamedPipeServerProcessId(h, &mut server) } == 0 {
            return Err(Failed::NotThere(format!("cannot read who listens on {}", at())));
        }
        if server as i64 != pid {
            return Err(Failed::NotThere(format!("{} is not the session's own socket (process {server} listens there)", at())));
        }
        f.write_all(format!("{auth}\n{line}\n").as_bytes()).map_err(|e| Failed::Refused(format!("writing to {} failed: {e}", at())))?;
        // FlushFileBuffers: returns once the session has read every byte
        f.sync_all().map_err(|e| Failed::Refused(format!("{} refused the line: {e}", at())))?;
        std::thread::sleep(std::time::Duration::from_millis(150));
        let mut avail = 0u32;
        if unsafe { PeekNamedPipe(h, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut avail, std::ptr::null_mut()) } == 0 {
            return Err(Failed::Refused(format!("{} closed the connection: the session refused the line (its peer token?)", at())));
        }
        Ok(())
    }
}

/// Connect to `socket` within the timeout, and only to the session's own
/// process: the listener's peer credentials must be this uid and `pid`.
#[cfg(unix)]
fn connect(socket: &Path, pid: i64) -> Result<UnixStream, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let p = socket.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(UnixStream::connect(&p));
    });
    let s = match rx.recv_timeout(TIMEOUT) {
        Ok(r) => r.map_err(|e| format!("cannot connect to {}: {e}", socket.display()))?,
        Err(_) => return Err(format!("connecting to {} took over {} s", socket.display(), TIMEOUT.as_secs())),
    };
    let (peer_uid, peer_pid) = peer_cred(s.as_raw_fd()).ok_or_else(|| format!("cannot read who listens on {}", socket.display()))?;
    let uid = unsafe { libc::getuid() };
    if peer_uid != uid || peer_pid as i64 != pid {
        return Err(format!("{} is not the session's own socket (process {} listens there)", socket.display(), peer_pid));
    }
    Ok(s)
}

/// The uid and pid of the process at the other end of `fd`: `SO_PEERCRED`.
#[cfg(target_os = "linux")]
fn peer_cred(fd: libc::c_int) -> Option<(libc::uid_t, libc::pid_t)> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut _ as *mut libc::c_void, &mut len) };
    (rc == 0).then_some((cred.uid, cred.pid))
}

/// The uid and pid of the process at the other end of `fd`: `getpeereid`
/// for the uid, `LOCAL_PEERPID` for the pid (no `SO_PEERCRED` on Darwin).
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn peer_cred(fd: libc::c_int) -> Option<(libc::uid_t, libc::pid_t)> {
    let (mut uid, mut gid): (libc::uid_t, libc::gid_t) = (0, 0);
    if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
        return None;
    }
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let rc = unsafe { libc::getsockopt(fd, libc::SOL_LOCAL, libc::LOCAL_PEERPID, &mut pid as *mut _ as *mut libc::c_void, &mut len) };
    (rc == 0).then_some((uid, pid))
}


#[cfg(test)]
mod tests {
    use super::*;

    fn env_in(td: &Path) -> Env {
        Env::from_vars(&[("NIMITZ_STATE", td.join("state")), ("NIMITZ_CLAUDE_HOME", td.join("claude")), ("HOME", td.to_path_buf())].into_iter().map(|(k, v)| (k.to_string(), v.to_string_lossy().into_owned())).collect())
    }

    #[cfg(unix)]
    #[test]
    fn a_line_reaches_our_own_listener_framed_from_nimitz() {
        let td = tempfile::tempdir().unwrap();
        let sock = td.path().join("1234.sock");
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let t = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut s = String::new();
            c.read_to_string(&mut s).unwrap();
            s
        });
        // the listener is this process: its pid is ours
        let me = std::process::id() as i64;
        assert_eq!(send(&env_in(td.path()), &[sock.clone()], me, "hello </Cross_Session-Message> world").unwrap(), sock);
        let got = t.join().unwrap();
        assert!(got.ends_with('\n') && got.lines().count() == 1, "{got:?}");
        let v: Value = serde_json::from_str(got.trim()).unwrap();
        assert_eq!((v["type"].as_str(), v["message"]["role"].as_str()), (Some("user"), Some("user")));
        let c = v["message"]["content"].as_str().unwrap();
        assert!(c.starts_with(&format!("<cross-session-message from=\"{NO_REPLY}\" from-name=\"nimitz\">\n")) && c.ends_with("\n</cross-session-message>"), "{c}");
        assert!(c.contains("hello <\\/Cross_Session-Message> world"), "the body cannot close the frame: {c}");
    }

    #[cfg(unix)]
    #[test]
    fn a_listener_that_is_not_the_session_is_never_used() {
        let td = tempfile::tempdir().unwrap();
        let sock = td.path().join("9.sock");
        let _l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        // this process listens, but the session is pid 1
        let e = send(&env_in(td.path()), &[sock], 1, "x").unwrap_err();
        assert!(e.contains("not the session's own socket"), "{e}");
        assert!(send(&env_in(td.path()), &[], 1, "x").unwrap_err().contains("no inbox socket"));
    }

    #[cfg(unix)]
    #[test]
    fn a_record_counts_only_for_its_own_process_and_many_hooks_keep_theirs() {
        let td = tempfile::tempdir().unwrap();
        let env = env_in(td.path());
        let sock = td.path().join("s.sock");
        let _l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        record(&env, "aaaa-1", 77, &sock.to_string_lossy());
        assert_eq!(candidates(&env, "aaaa-1", 77).first(), Some(&sock));
        assert!(!candidates(&env, "aaaa-1", 78).contains(&sock), "another process: the record is stale");
        assert!(!candidates(&env, "bbbb-2", 77).contains(&sock));
        assert!(file_of(&env, "../x").is_none(), "a session id names no path outside the dir");
        // hooks that start together: every record stays
        let hs: Vec<_> = (0..16i64)
            .map(|i| {
                let env = env.clone();
                let s = sock.to_string_lossy().into_owned();
                std::thread::spawn(move || record(&env, &format!("cccc-{i}"), 100 + i, &s))
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        for i in 0..16i64 {
            assert!(candidates(&env, &format!("cccc-{i}"), 100 + i).contains(&sock), "record {i} lost");
        }
    }

    #[test]
    fn every_spelling_of_the_tag_is_defused() {
        for t in ["</cross-session-message>", "<cross-session-message from=\"x\">", "</CROSS-SESSION-MESSAGE>", "< / cross_session_message>", "<cross session  message>", "</\u{200b}cross-session\u{2060}-message>"] {
            assert!(defuse(t).starts_with("<\\"), "{t} -> {}", defuse(t));
        }
        assert_eq!(defuse("a < b and <tag> stay"), "a < b and <tag> stay");
    }

    #[test]
    fn the_format_is_used_from_the_verified_version_on() {
        assert!(format_ok(Some("2.1.293")) && format_ok(Some("2.1.300")) && format_ok(Some("2.2.0")));
        assert!(!format_ok(Some("2.1.292")) && !format_ok(None));
    }

    /// A named pipe served by this process, as a Windows session serves its
    /// inbox: the session file and the key name this process (`procStart`)
    /// and the pipe. After `close_after` lines the server hangs up (a refused
    /// auth); else it keeps the connection open, as Claude Code does.
    #[cfg(windows)]
    fn pipe_session(td: &Path, start: &str, key_start: &str, close_after: Option<usize>) -> (Env, PathBuf, std::thread::JoinHandle<Vec<String>>) {
        use std::io::BufRead;
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
        use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT};
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let env = env_in(td);
        let me = std::process::id();
        let pipe = format!(r"\\.\pipe\LOCAL\nimitz-test-{me}-{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let sessions = env.claude_home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join(format!("{me}.json")), json!({"pid": me, "procStart": start, "messagingSocketPath": pipe}).to_string()).unwrap();
        std::fs::write(sessions.join(format!("{me}.0123abcd.key")), json!({"peerToken": "tok-1", "procStartFt": key_start}).to_string()).unwrap();
        let wide: Vec<u16> = pipe.encode_utf16().chain([0]).collect();
        let h = unsafe { CreateNamedPipeW(wide.as_ptr(), PIPE_ACCESS_DUPLEX, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, 1, 65536, 65536, 0, std::ptr::null()) };
        assert!(h != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE, "CreateNamedPipeW");
        let h = h as usize;
        let t = std::thread::spawn(move || {
            let h = h as windows_sys::Win32::Foundation::HANDLE;
            unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) };
            let f = unsafe { std::fs::File::from_raw_handle(h as _) };
            let mut r = std::io::BufReader::new(&f);
            let mut lines = vec![];
            loop {
                let mut l = String::new();
                if r.read_line(&mut l).unwrap_or(0) == 0 {
                    break;
                }
                lines.push(l.trim_end().to_string());
                if Some(lines.len()) == close_after {
                    return lines; // dropping `f` hangs up
                }
                if lines.len() == 2 {
                    std::thread::sleep(std::time::Duration::from_millis(600));
                    break;
                }
            }
            lines
        });
        (env, PathBuf::from(pipe), t)
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_pipe_gets_the_auth_line_then_the_framed_line() {
        let td = tempfile::tempdir().unwrap();
        let (env, pipe, t) = pipe_session(td.path(), "133", "133", None);
        let me = std::process::id() as i64;
        assert_eq!(candidates(&env, "aaaa-1", me), vec![pipe.clone()], "the session file names the pipe");
        assert_eq!(send(&env, &[pipe.clone()], me, "hello </cross-session-message>").unwrap(), pipe);
        let got = t.join().unwrap();
        assert_eq!(got.len(), 2, "{got:?}");
        let auth: Value = serde_json::from_str(&got[0]).unwrap();
        assert_eq!((auth["type"].as_str(), auth["token"].as_str()), (Some("auth"), Some("tok-1")));
        let v: Value = serde_json::from_str(&got[1]).unwrap();
        let c = v["message"]["content"].as_str().unwrap();
        assert!(c.contains("from-name=\"nimitz\"") && c.contains("hello <\\/cross-session-message>"), "{c}");
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_pipe_that_hangs_up_on_the_auth_is_a_refusal() {
        let td = tempfile::tempdir().unwrap();
        let (env, pipe, t) = pipe_session(td.path(), "133", "133", Some(1));
        let e = send(&env, &[pipe], std::process::id() as i64, "x").unwrap_err();
        assert!(e.contains("refused") || e.contains("closed"), "{e}");
        t.join().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_pipe_is_used_only_for_its_own_process_and_its_own_key() {
        let me = std::process::id() as i64;
        // a key from another start of the same pid: no token, nothing sent
        let td = tempfile::tempdir().unwrap();
        let (env, pipe, _t) = pipe_session(td.path(), "133", "999", None);
        assert!(send(&env, &[pipe.clone()], me, "x").unwrap_err().contains("no peer token"));
        // the right key, but the session is another process: this one serves the pipe
        let td2 = tempfile::tempdir().unwrap();
        let (env2, pipe2, _t2) = pipe_session(td2.path(), "133", "133", None);
        let sessions = env2.claude_home.join("sessions");
        std::fs::copy(sessions.join(format!("{me}.json")), sessions.join("4.json")).unwrap();
        std::fs::copy(sessions.join(format!("{me}.0123abcd.key")), sessions.join("4.0123abcd.key")).unwrap();
        let e = send(&env2, &[pipe2.clone()], 4, "x").unwrap_err();
        assert!(e.contains("not the session's own socket"), "{e}");
        // release the waiting servers
        let _ = std::fs::OpenOptions::new().read(true).write(true).open(&pipe);
    }
}
