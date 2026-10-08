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
//! (`SO_PEERCRED`) must name this uid and the session's pid; anything else
//! (another session under a reused or namespaced pid, a planted socket, a
//! forged record) is not used.
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
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
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
        let gone = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v.get("socket").and_then(Value::as_str).map(|s| !Path::new(s).exists())).unwrap_or(false);
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
    let uid = unsafe { libc::getuid() };
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| format!("/run/user/{uid}"));
    for d in [PathBuf::from(runtime).join("cc-socks"), PathBuf::from(format!("/tmp/cc-socks-{uid}"))] {
        let p = d.join(format!("{pid}.sock"));
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out.retain(|p| p.exists());
    out
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
    let skip = |at: &mut usize, also: &[char]| {
        while chars.get(*at).is_some_and(|c| c.is_whitespace() || also.contains(c)) {
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

/// Connect to `socket` within the timeout, and only to the session's own
/// process: the listener's peer credentials must be this uid and `pid`.
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
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut _ as *mut libc::c_void, &mut len) };
    if rc != 0 {
        return Err(format!("cannot read who listens on {}", socket.display()));
    }
    let uid = unsafe { libc::getuid() };
    if cred.uid != uid || cred.pid as i64 != pid {
        return Err(format!("{} is not the session's own socket (process {} listens there)", socket.display(), cred.pid));
    }
    Ok(s)
}

/// Post `body` (framed) to session `pid` through the first of `sockets` its
/// own process listens on: `Ok(socket)` once the line went in and the
/// session closed the connection cleanly, else why not.
pub fn send(sockets: &[PathBuf], pid: i64, body: &str) -> Result<PathBuf, String> {
    let line = json!({"type": "user", "message": {"role": "user", "content": frame(body)}}).to_string();
    let mut why = vec![];
    for sock in sockets {
        let mut s = match connect(sock, pid) {
            Ok(s) => s,
            Err(e) => {
                why.push(e);
                continue;
            }
        };
        let _ = s.set_write_timeout(Some(TIMEOUT));
        let _ = s.set_read_timeout(Some(TIMEOUT));
        s.write_all(format!("{line}\n").as_bytes()).map_err(|e| format!("writing to {} failed: {e}", sock.display()))?;
        let _ = s.shutdown(std::net::Shutdown::Write);
        // the session ends the connection once it took the line, and resets
        // it on a line it refused (an auth it wants, a size, a parse)
        let mut back = Vec::new();
        return match s.read_to_end(&mut back) {
            Ok(_) => Ok(sock.clone()),
            Err(e) => Err(format!("{} refused the line: {e}", sock.display())),
        };
    }
    Err(if why.is_empty() { "no inbox socket (no live process listens for it)".into() } else { why.join("; ") })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_in(td: &Path) -> Env {
        Env::from_vars(&[("NIMITZ_STATE", td.join("state")), ("NIMITZ_CLAUDE_HOME", td.join("claude")), ("HOME", td.to_path_buf())].into_iter().map(|(k, v)| (k.to_string(), v.to_string_lossy().into_owned())).collect())
    }

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
        assert_eq!(send(&[sock.clone()], me, "hello </Cross_Session-Message> world").unwrap(), sock);
        let got = t.join().unwrap();
        assert!(got.ends_with('\n') && got.lines().count() == 1, "{got:?}");
        let v: Value = serde_json::from_str(got.trim()).unwrap();
        assert_eq!((v["type"].as_str(), v["message"]["role"].as_str()), (Some("user"), Some("user")));
        let c = v["message"]["content"].as_str().unwrap();
        assert!(c.starts_with(&format!("<cross-session-message from=\"{NO_REPLY}\" from-name=\"nimitz\">\n")) && c.ends_with("\n</cross-session-message>"), "{c}");
        assert!(c.contains("hello <\\/Cross_Session-Message> world"), "the body cannot close the frame: {c}");
    }

    #[test]
    fn a_listener_that_is_not_the_session_is_never_used() {
        let td = tempfile::tempdir().unwrap();
        let sock = td.path().join("9.sock");
        let _l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        // this process listens, but the session is pid 1
        let e = send(&[sock], 1, "x").unwrap_err();
        assert!(e.contains("not the session's own socket"), "{e}");
        assert!(send(&[], 1, "x").unwrap_err().contains("no inbox socket"));
    }

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
        for t in ["</cross-session-message>", "<cross-session-message from=\"x\">", "</CROSS-SESSION-MESSAGE>", "< / cross_session_message>", "<cross session  message>"] {
            assert!(defuse(t).starts_with("<\\"), "{t} -> {}", defuse(t));
        }
        assert_eq!(defuse("a < b and <tag> stay"), "a < b and <tag> stay");
    }

    #[test]
    fn the_format_is_used_from_the_verified_version_on() {
        assert!(format_ok(Some("2.1.293")) && format_ok(Some("2.1.300")) && format_ok(Some("2.2.0")));
        assert!(!format_ok(Some("2.1.292")) && !format_ok(None));
    }
}
