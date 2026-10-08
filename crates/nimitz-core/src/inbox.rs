//! Delivery without typing: a line into a session's inbox socket (Claude
//! Code's cross-session messaging, code.claude.com/docs/en/cross-session-messaging,
//! "a script or hook to post into a session"). An idle background job takes
//! it as a new turn; a busy one at its next. Independent of panes, tabs and
//! the agent view, and it works with nobody attached.
//!
//! The socket: Claude Code exports `CLAUDE_CODE_MESSAGING_SOCKET` to hooks
//! before SessionStart runs; the hook records it here per session
//! (`NIMITZ_STATE/inbox.json`). With no record, `<runtime>/cc-socks/<pid>.sock`.
//!
//! The line: the docs give only the optional auth line. The message line is
//! the inject example Claude Code itself logs (`[uds-messaging] Inject
//! messages`): `{"type":"user","message":{"role":"user","content":"..."}}`,
//! verified on `FORMAT_SINCE` (e2e `e2e_probe_inbox_delivery`); an older
//! Claude Code gets the typing path. The content is wrapped as Claude Code
//! wraps a peer message, so the receiver sees the sender as `nimitz`
//! (`origin.name`; the address is not trusted, `origin.from` stays unknown).
use crate::env::Env;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The first Claude Code the message line was verified on.
pub const FORMAT_SINCE: &str = "2.1.293";

fn path(env: &Env) -> PathBuf {
    env.state.join("inbox.json")
}

fn load(env: &Env) -> serde_json::Map<String, Value> {
    std::fs::read(path(env)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

/// SessionStart: session `sid` (process `pid`) listens on `socket`. Records
/// whose socket is gone are dropped on the way.
pub fn record(env: &Env, sid: &str, pid: i64, socket: &str) {
    if sid.is_empty() || socket.is_empty() {
        return;
    }
    let mut m = load(env);
    m.retain(|_, v| v.get("socket").and_then(Value::as_str).is_some_and(|s| Path::new(s).exists()));
    m.insert(sid.to_string(), json!({"socket": socket, "pid": pid, "at": crate::evidence::now()}));
    let _ = std::fs::create_dir_all(&env.state);
    let tmp = path(env).with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec_pretty(&Value::Object(m)).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, path(env));
    }
}

/// Where session `sid` (live as `pid`) listens: its recorded socket, when
/// recorded for that same process and still there; else `<pid>.sock` in
/// Claude Code's socket directory.
pub fn socket_of(env: &Env, sid: &str, pid: Option<i64>) -> Option<PathBuf> {
    let rec = load(env).get(sid).cloned();
    if let Some(r) = rec {
        let same = pid.is_none() || r.get("pid").and_then(Value::as_i64) == pid;
        if let Some(s) = r.get("socket").and_then(Value::as_str).filter(|s| same && Path::new(s).exists()) {
            return Some(PathBuf::from(s));
        }
    }
    let pid = pid?;
    // the pid alone could be any process now (a sandbox's made-up pid is a
    // live session of the user's): only the process the session file names,
    // by its start time
    if !same_process(env, pid) {
        return None;
    }
    let uid = unsafe { libc::getuid() };
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| format!("/run/user/{uid}"));
    [PathBuf::from(runtime).join("cc-socks"), PathBuf::from(format!("/tmp/cc-socks-{uid}")), PathBuf::from("/tmp/cc-socks")]
        .into_iter()
        .map(|d| d.join(format!("{pid}.sock")))
        .find(|p| p.exists())
}

/// Is `pid` the very process whose session file (`sessions/<pid>.json`)
/// this Claude home holds? It runs in the file's folder (`/proc/<pid>/cwd`)
/// and started no later than the session (`startedAt`; a background job's
/// process is a pre-warmed spare, started earlier).
fn same_process(env: &Env, pid: i64) -> bool {
    let Some(v) = std::fs::read(env.claude_home.join("sessions").join(format!("{pid}.json"))).ok().and_then(|b| crate::evidence::first_json(&b)) else { return false };
    let (Some(ms), Some(cwd)) = (v.get("startedAt").and_then(Value::as_f64), v.get("cwd").and_then(Value::as_str)) else { return false };
    let here = std::fs::read_link(format!("/proc/{pid}/cwd")).ok();
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    here.is_some_and(|h| canon(&h) == canon(Path::new(cwd))) && proc_start(pid).is_some_and(|at| at <= ms / 1000.0 + 10.0)
}

/// When process `pid` started (epoch seconds): `/proc/<pid>/stat` field 22
/// (clock ticks after boot) plus `btime` from `/proc/stat`.
fn proc_start(pid: i64) -> Option<f64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = &stat[stat.rfind(')')? + 2..];
    let ticks: f64 = after.split_whitespace().nth(19)?.parse().ok()?;
    let btime: f64 = std::fs::read_to_string("/proc/stat").ok()?.lines().find_map(|l| l.strip_prefix("btime ")).and_then(|v| v.trim().parse().ok())?;
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    (hz > 0).then(|| btime + ticks / hz as f64)
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

/// The content of the line: `body` wrapped as Claude Code wraps a peer
/// message, from `nimitz`. A closing tag inside the body is defused.
pub fn frame(body: &str) -> String {
    let body = body.replace("</cross-session-message", "</cross-session-message_");
    format!("<cross-session-message from=\"nimitz\" from-name=\"nimitz\">\n{body}\n</cross-session-message>")
}

/// Post `body` (framed) into the session listening on `socket`.
pub fn send(socket: &Path, body: &str) -> Result<(), String> {
    let line = json!({"type": "user", "message": {"role": "user", "content": frame(body)}}).to_string();
    let mut s = std::os::unix::net::UnixStream::connect(socket).map_err(|e| format!("cannot connect to {}: {e}", socket.display()))?;
    let t = Some(Duration::from_secs(5));
    let _ = s.set_write_timeout(t);
    let _ = s.set_read_timeout(t);
    s.write_all(format!("{line}\n").as_bytes()).map_err(|e| format!("writing to {} failed: {e}", socket.display()))?;
    let _ = s.shutdown(std::net::Shutdown::Write);
    // the session closes the connection once it took the line
    let mut back = String::new();
    let _ = s.read_to_string(&mut back);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_reaches_the_socket_framed_from_nimitz() {
        let td = tempfile::tempdir().unwrap();
        let sock = td.path().join("1234.sock");
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let t = std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut s = String::new();
            c.read_to_string(&mut s).unwrap();
            s
        });
        send(&sock, "hello </cross-session-message> world").unwrap();
        let got = t.join().unwrap();
        assert!(got.ends_with('\n') && got.lines().count() == 1, "{got:?}");
        let v: Value = serde_json::from_str(got.trim()).unwrap();
        assert_eq!((v["type"].as_str(), v["message"]["role"].as_str()), (Some("user"), Some("user")));
        let c = v["message"]["content"].as_str().unwrap();
        assert!(c.starts_with("<cross-session-message from=\"nimitz\" from-name=\"nimitz\">\n") && c.ends_with("\n</cross-session-message>"), "{c}");
        assert_eq!(c.matches("</cross-session-message>").count(), 1, "the body cannot close the frame: {c}");
    }

    #[test]
    fn the_recorded_socket_counts_only_for_its_own_process() {
        let td = tempfile::tempdir().unwrap();
        let env = Env::from_vars(&[("NIMITZ_STATE".to_string(), td.path().join("state").to_string_lossy().into_owned())].into_iter().collect());
        let sock = td.path().join("s.sock");
        let _l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        record(&env, "s-1", 77, &sock.to_string_lossy());
        assert_eq!(socket_of(&env, "s-1", Some(77)), Some(sock.clone()));
        assert_eq!(socket_of(&env, "s-1", None), Some(sock.clone()));
        assert_eq!(socket_of(&env, "s-1", Some(78)), None, "another process: the record is stale (and no 78.sock)");
        assert_eq!(socket_of(&env, "s-2", None), None);
        // a record whose socket is gone is dropped at the next write
        drop(_l);
        std::fs::remove_file(&sock).unwrap();
        record(&env, "s-3", 79, "/nonexistent/x.sock");
        assert!(load(&env).get("s-1").is_none());
    }

    #[test]
    fn the_pid_fallback_needs_the_session_files_own_process() {
        let td = tempfile::tempdir().unwrap();
        let ch = td.path().join("claude");
        std::fs::create_dir_all(ch.join("sessions")).unwrap();
        let env = Env::from_vars(&[("NIMITZ_CLAUDE_HOME".to_string(), ch.to_string_lossy().into_owned()), ("NIMITZ_STATE".to_string(), td.path().join("state").to_string_lossy().into_owned())].into_iter().collect());
        let me = std::process::id() as i64;
        let at = proc_start(me).unwrap();
        let cwd = std::env::current_dir().unwrap().to_string_lossy().into_owned();
        let file = |started: f64, cwd: &str| std::fs::write(ch.join("sessions").join(format!("{me}.json")), json!({"pid": me, "startedAt": started * 1000.0, "cwd": cwd}).to_string()).unwrap();
        file(at + 900.0, &cwd);
        assert!(same_process(&env, me), "a spare started before its session");
        file(at - 3600.0, &cwd);
        assert!(!same_process(&env, me), "a session older than the process: another process under a reused pid");
        file(at, "/nonexistent/elsewhere");
        assert!(!same_process(&env, me), "another folder: not this process");
        assert!(!same_process(&env, 1), "no session file");
    }

    #[test]
    fn the_format_is_used_from_the_verified_version_on() {
        assert!(format_ok(Some("2.1.293")) && format_ok(Some("2.1.300")) && format_ok(Some("2.2.0")));
        assert!(!format_ok(Some("2.1.292")) && !format_ok(None));
    }
}
