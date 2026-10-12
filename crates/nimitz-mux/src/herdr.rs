//! Herdr adapter (`herdr agent list/focus/prompt`, `pane run/split/list/process-info`,
//! `workspace/tab create|list|focus`, `tab rename`). Owner: mux agent.
//!
//! Every operation shells out to the herdr CLI and parses its JSON reply;
//! every id comes from that JSON, never from guessing. Errors (JSON
//! `{"error":{code,message}}` on stderr, exit 1) are surfaced verbatim.
use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::{Caps, LayoutPlan, Mux, MuxError, Pane, PaneRef, Place, Shape, ShapePane, ShapeTab, ShapeWorkspace};

pub struct Herdr {
    pub bin: String,
    /// `--session <name>` on every call (`NIMITZ_HERDR_SESSION`), else Herdr's own resolution
    pub session: Option<String>,
    /// `HERDR_SOCKET_PATH` for every call (`NIMITZ_HERDR_SOCKET`): the
    /// default server a user's own command recorded with no session name
    pub socket: Option<String>,
    /// the Claude home whose job rows and session files back a pane's claim
    /// (the caller's env, as every other read of it)
    pub claude_home: std::path::PathBuf,
}

/// Loose label matching, the herdr-fleet skill's rules in their order:
/// strip a trailing `-FLEET`, **then** delete every `[` and `]`, then upcase.
/// The order matters: bracket-first leaves `[ARGOS]-FLEET` as `ARGOS]`.
pub fn normalize_label(label: &str) -> String {
    let t = label.trim();
    // "-FLEET" is 6 ASCII bytes; if the uppercased tail matches, the slice is char-safe
    let t = if t.len() >= 6 && t.is_char_boundary(t.len() - 6) && t[t.len() - 6..].eq_ignore_ascii_case("-FLEET") {
        &t[..t.len() - 6]
    } else {
        t
    };
    t.chars().filter(|c| !matches!(c, '[' | ']')).collect::<String>().to_uppercase()
}

/// POSIX shell quoting: safe words stay bare, the rest go in single quotes
/// (`'` becomes `'\''`). Works in sh, bash and zsh.
pub fn shell_join(cmd: &[String]) -> String {
    cmd.iter()
        .map(|a| {
            let safe = !a.is_empty()
                && a.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
            if safe { a.clone() } else { format!("'{}'", a.replace('\'', "'\\''")) }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// How a shell reads a typed line, by its program name (`-zsh`,
/// `C:\...\PowerShell.exe` included). None for a shell nimitz does not quote
/// for (nu) and for none known.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Quoting {
    Posix,
    Cmd,
    Pwsh,
}

fn quoting(shell: Option<&str>) -> Option<Quoting> {
    let name = nimitz_core::restore::program_name(shell?).trim_start_matches('-').to_ascii_lowercase();
    match name.as_str() {
        "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" | "tcsh" | "csh" => Some(Quoting::Posix),
        "cmd" => Some(Quoting::Cmd),
        "pwsh" | "powershell" => Some(Quoting::Pwsh),
        _ => None,
    }
}

/// Does nimitz know how `shell` quotes (`shell_line` quotes for it)?
pub fn known_shell(shell: Option<&str>) -> bool {
    quoting(shell).is_some()
}

/// cmd.exe: one word in double quotes, read back by the program's own
/// command-line parser (the C runtime's rules): `"` doubled, a run of `\`
/// doubled where a `"` follows it, and `%` (which cmd expands even inside
/// quotes) typed outside the quotes as `^%`.
fn cmd_quote(a: &str) -> String {
    let mut out = String::from("\"");
    let mut bs = 0;
    for c in a.chars() {
        match c {
            '\\' => {
                bs += 1;
                out.push('\\');
                continue;
            }
            '"' => {
                out.push_str(&"\\".repeat(bs));
                out.push_str("\"\"");
            }
            '%' => {
                out.push_str(&"\\".repeat(bs));
                out.push_str("\"^%\"");
            }
            c => out.push(c),
        }
        bs = 0;
    }
    out.push_str(&"\\".repeat(bs));
    out.push('"');
    out
}

/// PowerShell: one word in single quotes (literal), `'` doubled (and the
/// typographic single quotes PowerShell also reads as quotes).
fn pwsh_quote(a: &str) -> String {
    let mut out = String::from("'");
    for c in a.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// The line that runs `argv` in a pane whose shell is `shell` (its program
/// name or path, from `pane process-info`):
/// - sh, bash, zsh and the other POSIX shells (Git Bash too): `shell_join`;
/// - cmd: `"C:\x\claude.exe" attach <job>`, the program always quoted, the
///   rest where needed (`cmd_quote`);
/// - pwsh, powershell: `& 'C:\x\claude.exe' attach <job>` (the call operator:
///   a quoted first word is a string to PowerShell, not a command);
/// - an unknown shell: cmd's quoting (`bare_claude` first makes the program
///   a bare `claude` when PATH finds the same file, a word every shell runs).
pub fn shell_line(shell: Option<&str>, argv: &[String]) -> String {
    match quoting(shell) {
        Some(Quoting::Posix) => shell_join(argv),
        Some(Quoting::Pwsh) => {
            let safe = |a: &str| !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:\\".contains(c));
            let words = argv.iter().enumerate().map(|(i, a)| if i > 0 && safe(a) { a.clone() } else { pwsh_quote(a) });
            std::iter::once("&".to_string()).chain(words).collect::<Vec<_>>().join(" ")
        }
        Some(Quoting::Cmd) | None => {
            let safe = |a: &str| !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:\\@+=,".contains(c));
            argv.iter()
                .enumerate()
                // a bare `claude` (`bare_claude`) stays bare; any other program is quoted
                .map(|(i, a)| if (i > 0 || a == "claude") && safe(a) { a.clone() } else { cmd_quote(a) })
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
}

/// `argv` with its program as the bare word `claude` when that is the
/// program PATH gives `claude` (`found`, `env::which("claude")`): the same
/// file by path (case and separators aside) or by its real path. A line for
/// a shell whose quoting is unknown then starts with a word every shell runs.
pub fn bare_claude(argv: &[String], found: Option<&str>) -> Vec<String> {
    let (Some(prog), Some(found)) = (argv.first(), found) else { return argv.to_vec() };
    if !nimitz_core::restore::program_name(prog).eq_ignore_ascii_case("claude") {
        return argv.to_vec();
    }
    let norm = |p: &str| p.replace('/', "\\").to_lowercase();
    let same = norm(prog) == norm(found) || matches!((std::fs::canonicalize(prog), std::fs::canonicalize(found)), (Ok(a), Ok(b)) if a == b);
    if !same {
        return argv.to_vec();
    }
    std::iter::once("claude".to_string()).chain(argv[1..].iter().cloned()).collect()
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

// ------------------------------------------------------------------ reply parsing (pure)
//
// Every reply nimitz reads, as a function of the JSON alone, so the committed
// samples of a real Herdr (`tests/fixtures/herdr-0.9.3/`) test them. Reply
// shapes come from the API schema (protocol 22, `herdr api schema`):
// `{"id", "result": {"type": "<variant>", ...}}`.

/// `agent list` (`agent_list`: `result.agents[]` of AgentInfo), and the
/// snapshot's `result.snapshot.agents[]`: the panes that show an agent.
pub fn parse_panes(v: &Value) -> Vec<Pane> {
    agents_in(v.pointer("/result/agents").unwrap_or(&Value::Null))
}

/// AgentInfo rows -> panes (`pane_id`, `tab_id`, `workspace_id`,
/// `agent_session.value`, `terminal_title_stripped`, `agent_status`,
/// `foreground_cwd` else `cwd`).
pub fn agents_in(list: &Value) -> Vec<Pane> {
    let Some(list) = list.as_array() else { return vec![] };
    list.iter()
        .filter(|a| !s(a, "pane_id").is_empty())
        .map(|a| Pane {
            pane: PaneRef {
                pane_id: s(a, "pane_id"),
                tab_id: Some(s(a, "tab_id")).filter(|x| !x.is_empty()),
                workspace_id: Some(s(a, "workspace_id")).filter(|x| !x.is_empty()),
            },
            session_id: a.pointer("/agent_session/value").and_then(Value::as_str).map(String::from),
            title: s(a, "terminal_title_stripped"),
            state: s(a, "agent_status"),
            cwd: {
                let f = s(a, "foreground_cwd");
                if f.is_empty() { s(a, "cwd") } else { f }
            },
            inferred: false,
        })
        .collect()
}

/// WorkspaceInfo rows (`workspace list`'s `result.workspaces[]`, the
/// snapshot's `result.snapshot.workspaces[]`) -> (workspace_id, label, active_tab_id).
pub fn workspaces_in(list: &Value) -> Vec<(String, String, String)> {
    list.as_array().map(|a| a.iter().map(|w| (s(w, "workspace_id"), s(w, "label"), s(w, "active_tab_id"))).filter(|(id, _, _)| !id.is_empty()).collect()).unwrap_or_default()
}

/// TabInfo rows (`tab list`'s `result.tabs[]`, the snapshot's `result.snapshot.tabs[]`) -> (tab_id, label).
pub fn tabs_in(list: &Value) -> Vec<(String, String)> {
    list.as_array().map(|a| a.iter().map(|t| (s(t, "tab_id"), s(t, "label"))).filter(|(id, _)| !id.is_empty()).collect()).unwrap_or_default()
}

/// PaneInfo rows (`pane list`'s `result.panes[]`, the snapshot's
/// `result.snapshot.panes[]`) -> pane refs, in pane id order.
pub fn pane_rows_in(list: &Value) -> Vec<PaneRef> {
    let mut v: Vec<PaneRef> = list
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|p| !s(p, "pane_id").is_empty())
                .map(|p| PaneRef { pane_id: s(p, "pane_id"), tab_id: Some(s(p, "tab_id")).filter(|x| !x.is_empty()), workspace_id: Some(s(p, "workspace_id")).filter(|x| !x.is_empty()) })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| a.pane_id.cmp(&b.pane_id));
    v
}

/// `api snapshot` (`session.snapshot`: `result.snapshot` is a SessionSnapshot
/// with `version`, `protocol`, `workspaces[]`, `tabs[]`, `panes[]`,
/// `agents[]`, `layouts[]`) -> the shape, foregrounds unread (`None`). The
/// `panes[]` rows carry no agent fields (herdr 0.9.3); `agents[]` lists the
/// panes that show an agent, with `agent_session.value` the Claude session
/// id: both are joined by `pane_id`. A reply with no `snapshot` is an error.
pub fn parse_snapshot(v: &Value) -> Result<Shape, String> {
    let snap = v.pointer("/result/snapshot").filter(|s| s.is_object()).ok_or_else(|| format!("no snapshot in the api snapshot reply: {}", v.get("result").map(|r| r.to_string()).unwrap_or_default()))?;
    let workspaces = workspaces_in(&snap["workspaces"]).into_iter().map(|(id, label, active_tab_id)| ShapeWorkspace { id, label, active_tab_id }).collect();
    let tabs = snap["tabs"]
        .as_array()
        .map(|a| a.iter().filter(|t| !s(t, "tab_id").is_empty()).map(|t| ShapeTab { id: s(t, "tab_id"), workspace_id: s(t, "workspace_id"), label: s(t, "label") }).collect())
        .unwrap_or_default();
    let agents = agents_in(&snap["agents"]);
    let agent_kind: std::collections::HashMap<String, String> = snap["agents"].as_array().map(|a| a.iter().map(|x| (s(x, "pane_id"), s(x, "agent"))).collect()).unwrap_or_default();
    let mut panes: Vec<ShapePane> = snap["panes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|p| !s(p, "pane_id").is_empty())
                .map(|p| {
                    let id = s(p, "pane_id");
                    let agent = agents.iter().find(|a| a.pane.pane_id == id);
                    ShapePane {
                        pane: PaneRef { pane_id: id.clone(), tab_id: Some(s(p, "tab_id")).filter(|x| !x.is_empty()), workspace_id: Some(s(p, "workspace_id")).filter(|x| !x.is_empty()) },
                        cwd: {
                            let f = s(p, "foreground_cwd");
                            if f.is_empty() { s(p, "cwd") } else { f }
                        },
                        title: s(p, "terminal_title_stripped"),
                        agent: agent_kind.get(&id).filter(|k| !k.is_empty()).cloned(),
                        session_id: agent.and_then(|a| a.session_id.clone()),
                        claimed: agent.and_then(|a| a.session_id.clone()),
                        state: agent.map(|a| a.state.clone()).unwrap_or_else(|| s(p, "agent_status")),
                        foreground: None,
                        foreground_pid: None,
                        inferred: false,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    // an agent whose pane the panes list lacks (never seen; kept for safety)
    for a in agents {
        if !panes.iter().any(|p| p.pane.pane_id == a.pane.pane_id) {
            panes.push(ShapePane { pane: a.pane, cwd: a.cwd, title: a.title, agent: Some("claude".into()), claimed: a.session_id.clone(), session_id: a.session_id, state: a.state, foreground: None, foreground_pid: None, inferred: false });
        }
    }
    panes.sort_by(|a, b| a.pane.pane_id.cmp(&b.pane.pane_id));
    Ok(Shape { version: s(snap, "version"), protocol: snap.get("protocol").and_then(Value::as_u64).unwrap_or(0), workspaces, tabs, panes })
}

/// `pane process-info` (`pane_process_info`: `result.process_info` is a
/// PaneProcessInfo with `foreground_processes[]` of `{pid, name, argv0,
/// argv[], cmdline, cwd}`; Herdr 0.9.3 nests it so, the flat
/// `result.foreground_processes` is read as a fallback). The argv of every
/// foreground process, in order (`argv`, else `cmdline` split, else `name`).
/// `Err` when the reply carries no list: nothing is typed into a pane whose
/// foreground is unknown.
pub fn parse_process_info(v: &Value) -> Result<Vec<Vec<String>>, String> {
    let list = v
        .pointer("/result/process_info/foreground_processes")
        .or_else(|| v.pointer("/result/foreground_processes"))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("no foreground_processes in the pane process-info reply: {}", v.get("result").map(|r| r.to_string()).unwrap_or_default()))?;
    Ok(list
        .iter()
        .map(|p| {
            let mut argv: Vec<String> = p.get("argv").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            if argv.is_empty() {
                argv = s(p, "cmdline").split_whitespace().map(String::from).collect();
            }
            if argv.is_empty() {
                argv = vec![s(p, "name")].into_iter().filter(|x| !x.is_empty()).collect();
            }
            argv
        })
        .collect())
}

/// What runs in a pane, from its foreground processes: the first that is not
/// a shell; empty when only the shell is there (an idle shell).
pub fn foreground_of(procs: &[Vec<String>]) -> Vec<String> {
    procs.iter().find(|argv| !crate::is_shell(argv)).cloned().unwrap_or_default()
}

/// The pids of the foreground processes, in the order of `parse_process_info`
/// (`None` where the reply names none).
pub fn process_pids(v: &Value) -> Vec<Option<i64>> {
    v.pointer("/result/process_info/foreground_processes")
        .or_else(|| v.pointer("/result/foreground_processes"))
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|p| p.get("pid").and_then(Value::as_i64)).collect())
        .unwrap_or_default()
}

/// `workspace create` (`workspace_created`, ResponseResult: `result.workspace`
/// a WorkspaceInfo, `result.tab` the root TabInfo, `result.root_pane` its
/// PaneInfo, all required) -> (workspace_id, tab_id, pane_id). A reply that
/// names no tab or pane (an older or a stricter Herdr) gives None for them,
/// and the adapter derives them from `active_tab_id` and `pane list`.
pub fn created_workspace(v: &Value) -> Result<(String, Option<String>, Option<String>), String> {
    let r = v.get("result").cloned().unwrap_or(Value::Null);
    let ws = r.pointer("/workspace/workspace_id").and_then(Value::as_str).unwrap_or("").to_string();
    if ws.is_empty() {
        return Err(format!("herdr workspace create: no workspace_id in reply {r}"));
    }
    let tab = r.pointer("/tab/tab_id").or_else(|| r.pointer("/workspace/active_tab_id")).and_then(Value::as_str).filter(|x| !x.is_empty()).map(String::from);
    let pane = r.pointer("/root_pane/pane_id").and_then(Value::as_str).filter(|x| !x.is_empty()).map(String::from);
    Ok((ws, tab, pane))
}

/// `tab create` (`tab_created`, ResponseResult: `result.tab` a TabInfo and
/// `result.root_pane` its PaneInfo, both required) -> (tab_id, pane_id). A
/// reply that names no pane gives None, and the adapter derives it from
/// `pane list`.
pub fn created_tab(v: &Value) -> Result<(String, Option<String>), String> {
    let r = v.get("result").cloned().unwrap_or(Value::Null);
    let tab = r.pointer("/tab/tab_id").and_then(Value::as_str).unwrap_or("").to_string();
    if tab.is_empty() {
        return Err(format!("herdr tab create: no tab_id in reply {r}"));
    }
    let pane = r.pointer("/root_pane/pane_id").and_then(Value::as_str).filter(|x| !x.is_empty()).map(String::from);
    Ok((tab, pane))
}

/// `pane split` (`pane_info`: `result.pane` is a PaneInfo; `pane_created` is
/// the event's tag, not the reply's) -> (pane_id, tab_id).
pub fn created_pane(v: &Value) -> Result<(String, String), String> {
    let p = v.pointer("/result/pane").cloned().unwrap_or(Value::Null);
    let pane = s(&p, "pane_id");
    if pane.is_empty() {
        return Err(format!("herdr pane split: no pane_id in reply {p}"));
    }
    Ok((pane, s(&p, "tab_id")))
}

impl Herdr {
    /// The argv of a call: `--session <name>` first when one is set.
    pub fn argv(&self, args: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = vec![];
        if let Some(s) = &self.session {
            v.extend(["--session".to_string(), s.clone()]);
        }
        v.extend(args.iter().map(|a| a.to_string()));
        v
    }

    /// Every foreground process of a pane: argv and pid (`pane process-info`).
    fn foreground_procs(&self, pane_id: &str) -> Result<Vec<(Vec<String>, Option<i64>)>, MuxError> {
        let v = self.run(&["pane", "process-info", "--pane", pane_id])?;
        let procs = parse_process_info(&v).map_err(MuxError::Failed)?;
        let pids = process_pids(&v);
        Ok(procs.into_iter().enumerate().map(|(i, a)| (a, pids.get(i).copied().flatten())).collect())
    }

    /// The session a pane shows, and whether it is read from the attach
    /// alone (`Pane.inferred`). Herdr's claim stands only when the pane's
    /// foreground backs it (`evidence::claim_backing`; one only a `claude
    /// attach <job>` backs is inferred, as the attach alone is): Herdr's own hook names
    /// the pane by the process's inherited `HERDR_PANE_ID`, which a
    /// background job brings from whoever launched it (or the daemon's), so
    /// a ship's hook can claim its commodore's pane. A claim that is not
    /// backed is ignored, and the pane shows what runs in it, as a pane with
    /// no claim does (`session_of_unclaimed`): the job a `claude attach <job>`
    /// attaches to (inferred: an attach client never runs Herdr's hook, so
    /// Herdr never names the session of the very pane nimitz opened for a
    /// lead), else a held `claude --resume <sid>`, else the session the
    /// foreground pid's session file names. An unreadable foreground backs
    /// nothing and shows nothing.
    fn shown(&self, pane_id: &str, claim: Option<&str>, procs: Option<&[(Vec<String>, Option<i64>)]>) -> (Option<String>, bool) {
        let home = &self.claude_home;
        let read;
        let procs = match procs {
            Some(p) => p,
            None => {
                read = self.foreground_procs(pane_id).unwrap_or_default();
                &read[..]
            }
        };
        // a claim the attach alone backs is inferred too: under that argv an
        // older Claude Code may run the agent view (R-24)
        if let Some(claim) = claim {
            match nimitz_core::evidence::claim_backing(home, procs, claim) {
                Some(nimitz_core::evidence::Backing::Process) => return (Some(claim.to_string()), false),
                Some(nimitz_core::evidence::Backing::Attach) => return (Some(claim.to_string()), true),
                None => {}
            }
        }
        // the attach alone: shown, typed into only while it runs exactly the attach (`Pane.inferred`)
        if let Some(sid) = nimitz_core::evidence::session_of_attach(home, procs) {
            return (Some(sid), true);
        }
        (nimitz_core::evidence::session_of_unclaimed(home, procs), false)
    }

    fn run(&self, args: &[&str]) -> Result<Value, MuxError> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(self.argv(args));
        // a named server (a session name or a recorded socket) is the only
        // one spoken to: the child herdr inherits this process's env, so
        // whatever HERDR_* came with it (a pane's, or the daemon's for a
        // member) goes, and only the recorded socket is set
        if self.session.is_some() || self.socket.is_some() {
            for k in ["HERDR_SESSION", "HERDR_SOCKET_PATH", "HERDR_CLIENT_SOCKET_PATH"] {
                cmd.env_remove(k);
            }
        }
        if let Some(sock) = &self.socket {
            cmd.env("HERDR_SOCKET_PATH", sock);
        }
        let out = cmd.output()
            .map_err(|e| MuxError::Failed(format!("{}: {e}", self.bin)))?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() {
            return Ok(serde_json::from_str(&stdout).unwrap_or(Value::Null));
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = [stderr.as_str(), stdout.as_str()]
            .iter()
            .filter_map(|t| serde_json::from_str::<Value>(t).ok())
            .find_map(|v| {
                let e = v.get("error")?.clone();
                Some(format!("herdr {}: {} ({})", args.join(" "), s(&e, "message"), s(&e, "code")))
            })
            .unwrap_or_else(|| {
                format!("herdr {}: {}", args.join(" "), if stderr.is_empty() { stdout } else { stderr })
            });
        Err(MuxError::Failed(msg))
    }

    /// `herdr workspace list` -> [(workspace_id, label, active_tab_id)]
    fn workspaces(&self) -> Result<Vec<(String, String, String)>, MuxError> {
        let v = self.run(&["workspace", "list"])?;
        Ok(workspaces_in(v.pointer("/result/workspaces").unwrap_or(&Value::Null)))
    }

    /// `herdr tab list --workspace <id>` -> [(tab_id, label)]
    fn tabs(&self, workspace_id: &str) -> Result<Vec<(String, String)>, MuxError> {
        let v = self.run(&["tab", "list", "--workspace", workspace_id])?;
        Ok(tabs_in(v.pointer("/result/tabs").unwrap_or(&Value::Null)))
    }

    /// The panes of a tab, in id order (`herdr pane list`).
    fn tab_panes(&self, workspace_id: &str, tab_id: &str) -> Result<Vec<PaneRef>, MuxError> {
        let v = self.run(&["pane", "list"])?;
        Ok(pane_rows_in(v.pointer("/result/panes").unwrap_or(&Value::Null))
            .into_iter()
            .filter(|p| p.tab_id.as_deref() == Some(tab_id))
            .map(|p| PaneRef { workspace_id: Some(workspace_id.to_string()), ..p })
            .collect())
    }

    /// The first pane of a tab just made, when the reply named no
    /// `root_pane` (a fallback; the schema requires it): `pane list`.
    fn first_pane(&self, workspace_id: &str, tab_id: &str) -> Result<String, MuxError> {
        self.tab_panes(workspace_id, tab_id)?.into_iter().next().map(|p| p.pane_id).ok_or_else(|| MuxError::Failed(format!("herdr pane list: no pane in the new tab {tab_id}")))
    }

    /// The pane of a tab to attach in: its first shell pane (a tab can carry
    /// a split running Yazi or another program: never that one), else its
    /// first pane (then `tab_state` reports what runs there).
    fn pane_in_tab(&self, workspace_id: &str, tab_id: &str) -> Result<PaneRef, MuxError> {
        let panes = self.tab_panes(workspace_id, tab_id)?;
        let shell = panes.iter().find(|p| self.foreground(p).map(|fg| crate::is_shell(&fg)).unwrap_or(false)).cloned();
        shell.or_else(|| panes.first().cloned()).ok_or_else(|| MuxError::Failed(format!("herdr pane list: no pane in tab {tab_id}")))
    }

    /// The tab labelled `label` in workspace `workspace`, both loosely matched.
    fn find_tab(&self, workspace: &str, label: &str) -> Result<Option<(String, String)>, MuxError> {
        let (want_ws, want) = (normalize_label(workspace), normalize_label(label));
        let Some((ws_id, _, _)) = self.workspaces()?.into_iter().find(|(_, l, _)| normalize_label(l) == want_ws) else { return Ok(None) };
        Ok(self.tabs(&ws_id)?.into_iter().find(|(_, l)| normalize_label(l) == want).map(|(tab_id, _)| (ws_id, tab_id)))
    }

    /// `workspace create --label <label> --cwd <cwd> --no-focus`
    /// -> (workspace_id, root tab_id, root pane_id), all from the reply
    /// (`workspace`, `tab`, `root_pane`); `active_tab_id` and `pane list`
    /// stand in only when a reply names no tab or pane.
    fn workspace_create(&self, label: &str, cwd: &str) -> Result<(String, String, String), MuxError> {
        let v = self.run(&["workspace", "create", "--label", label, "--cwd", cwd, "--no-focus"])?;
        let (ws, tab, pane) = created_workspace(&v).map_err(MuxError::Failed)?;
        let tab = match tab {
            Some(t) => t,
            None => self.tabs(&ws)?.into_iter().next().map(|(id, _)| id).ok_or_else(|| MuxError::Failed(format!("herdr tab list: no tab in the new workspace {ws}")))?,
        };
        let pane = match pane {
            Some(p) => p,
            None => self.first_pane(&ws, &tab)?,
        };
        Ok((ws, tab, pane))
    }

    /// `tab create --workspace <id> --label <label> --cwd <cwd> --no-focus`
    /// -> (tab_id, root pane_id), both from the reply (`tab`, `root_pane`);
    /// `pane list` stands in only when a reply names no pane.
    fn tab_create(&self, workspace_id: &str, label: &str, cwd: &str) -> Result<(String, String), MuxError> {
        let v = self.run(&["tab", "create", "--workspace", workspace_id, "--label", label, "--cwd", cwd, "--no-focus"])?;
        let (tab, pane) = created_tab(&v).map_err(MuxError::Failed)?;
        let pane = match pane {
            Some(p) => p,
            None => self.first_pane(workspace_id, &tab)?,
        };
        Ok((tab, pane))
    }

    /// `herdr pane run` joins its arguments with spaces and types the line
    /// into the pane's shell, so the command goes over as one quoted line:
    /// POSIX quoting on Unix; on Windows quoted for the shell the pane runs
    /// (`shell_line`).
    fn pane_run(&self, pane_id: &str, cmd: &[String]) -> Result<(), MuxError> {
        #[cfg(windows)]
        let line = {
            let shell = self.pane_shell(pane_id);
            if known_shell(shell.as_deref()) {
                shell_line(shell.as_deref(), cmd)
            } else {
                shell_line(None, &bare_claude(cmd, nimitz_core::env::which("claude").as_deref()))
            }
        };
        #[cfg(not(windows))]
        let line = shell_join(cmd);
        self.run(&["pane", "run", pane_id, &line]).map(|_| ())
    }

    /// The program of the shell in a pane's foreground (`pane process-info`).
    /// A pane just made may not show its shell yet: asked for up to a second
    /// (an unreadable foreground is not asked again).
    #[cfg(windows)]
    fn pane_shell(&self, pane_id: &str) -> Option<String> {
        for _ in 0..10 {
            let procs = self.foreground_procs(pane_id).ok()?;
            if let Some(p) = procs.iter().find(|(argv, _)| !argv.is_empty() && crate::is_shell(argv)).and_then(|(argv, _)| argv.first()) {
                return Some(p.clone());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        None
    }

    /// The workspace whose label loosely matches, else a new one; `fresh_tab`
    /// is the new workspace's root tab (unlabelled, reusable), None on a match.
    #[allow(clippy::type_complexity)]
    fn find_or_create_workspace(
        &self,
        label: &str,
        cwd: &str,
    ) -> Result<(String, Option<(String, String)>), MuxError> {
        let want = normalize_label(label);
        if let Some((id, _, _)) = self.workspaces()?.into_iter().find(|(_, l, _)| normalize_label(l) == want) {
            return Ok((id, None));
        }
        let (ws, tab, pane) = self.workspace_create(label, cwd)?;
        Ok((ws, Some((tab, pane))))
    }
}

impl Mux for Herdr {
    fn name(&self) -> &'static str {
        "herdr"
    }

    fn tab(&self, place: &Place) -> Option<PaneRef> {
        // the tab nimitz names for it (`F.ADMIRAL`, `F.SQ.SHIP`), in its workspace
        let Place::Tab { workspace, label } = place else { return None };
        let (ws_id, tab_id) = self.find_tab(workspace, label).ok()??;
        self.pane_in_tab(&ws_id, &tab_id).ok()
    }

    /// Every foreground process of the pane, with its pid (`pane process-info`).
    fn foreground_procs(&self, pane: &PaneRef) -> Result<Vec<(Vec<String>, Option<i64>)>, MuxError> {
        Herdr::foreground_procs(self, &pane.pane_id)
    }

    /// `herdr pane process-info --pane <id>`: the foreground process group of
    /// the pane's pty (`result.process_info.foreground_processes[].argv`). The
    /// first process that is not a shell is what runs; none means an idle
    /// shell; a reply with no list is an error (never "idle shell").
    fn foreground(&self, pane: &PaneRef) -> Result<Vec<String>, MuxError> {
        let v = self.run(&["pane", "process-info", "--pane", &pane.pane_id])?;
        let procs = parse_process_info(&v).map_err(MuxError::Failed)?;
        Ok(foreground_of(&procs))
    }

    fn run_in(&self, pane: &PaneRef, cmd: &[String]) -> Result<(), MuxError> {
        self.pane_run(&pane.pane_id, cmd)
    }

    /// `herdr api snapshot` (the whole shape in one reply), then `pane
    /// process-info` for every pane: a pane whose foreground cannot be read
    /// stays `None` (a restore never types into it).
    fn shape(&self, foregrounds: bool) -> Result<Shape, MuxError> {
        let v = self.run(&["api", "snapshot"])?;
        let mut shape = parse_snapshot(&v).map_err(MuxError::Failed)?;
        for p in shape.panes.iter_mut() {
            // a pane that claims a session, or shows an agent Herdr names no
            // session for (an attach), is read in any case (a claim must be
            // backed; an attach names its job); every pane when the
            // foregrounds were asked for
            if !foregrounds && p.session_id.is_none() && p.agent.is_none() {
                continue;
            }
            // one process-info per pane: what runs (the first non-shell) and its pid
            let procs = self.foreground_procs(&p.pane.pane_id).ok();
            p.claimed = p.session_id.clone();
            if p.session_id.is_some() || p.agent.is_some() {
                (p.session_id, p.inferred) = self.shown(&p.pane.pane_id, p.claimed.as_deref(), Some(procs.as_deref().unwrap_or(&[])));
            }
            if foregrounds {
                if let Some(procs) = procs {
                    let at = procs.iter().position(|(argv, _)| !crate::is_shell(argv));
                    p.foreground = Some(at.map(|i| procs[i].0.clone()).unwrap_or_default());
                    p.foreground_pid = at.and_then(|i| procs[i].1);
                }
            }
        }
        Ok(shape)
    }

    fn rename_tab(&self, tab_id: &str, label: &str) -> Result<(), MuxError> {
        self.run(&["tab", "rename", tab_id, label]).map(|_| ())
    }

    fn close_pane(&self, pane: &PaneRef) -> Result<(), MuxError> {
        self.run(&["pane", "close", &pane.pane_id]).map(|_| ())
    }

    fn caps(&self) -> Caps {
        Caps { open: true, focus: true, type_text: true, layout: true }
    }

    /// Agent panes Herdr knows about; empty on any error (same as not inside Herdr).
    fn panes(&self) -> Vec<Pane> {
        let mut panes = self.run(&["agent", "list"]).map(|v| parse_panes(&v)).unwrap_or_default();
        for p in panes.iter_mut() {
            let claim = p.session_id.clone();
            (p.session_id, p.inferred) = self.shown(&p.pane.pane_id, claim.as_deref(), None);
        }
        panes
    }

    fn open(&self, cmd: &[String], cwd: &Path, place: Place) -> Result<PaneRef, MuxError> {
        let cwd = cwd.to_string_lossy().into_owned();
        let (pane, tab, ws) = match place {
            Place::Workspace { label } => {
                let (ws, tab, pane) = self.workspace_create(&label, &cwd)?;
                (pane, tab, ws)
            }
            Place::Tab { workspace, label } => {
                let (ws, fresh_tab) = self.find_or_create_workspace(&workspace, &cwd)?;
                let (tab, pane) = match fresh_tab {
                    // a new workspace already has an unlabelled root tab: claim it
                    Some((tab, pane)) => {
                        self.run(&["tab", "rename", &tab, &label])?;
                        (tab, pane)
                    }
                    None => self.tab_create(&ws, &label, &cwd)?,
                };
                (pane, tab, ws)
            }
            Place::Split { next_to } => {
                let v = self.run(&["pane", "split", &next_to.pane_id, "--direction", "right", "--cwd", &cwd, "--no-focus"])?;
                let (pane, tab) = created_pane(&v).map_err(MuxError::Failed)?;
                let tab = if tab.is_empty() { next_to.tab_id.clone().unwrap_or_default() } else { tab };
                (pane, tab, next_to.workspace_id.clone().unwrap_or_default())
            }
        };
        self.pane_run(&pane, cmd)?;
        Ok(PaneRef {
            pane_id: pane,
            tab_id: Some(tab).filter(|x| !x.is_empty()),
            workspace_id: Some(ws).filter(|x| !x.is_empty()),
        })
    }

    /// Workspace focus, then tab focus, then agent focus — so the pane is
    /// actually in front of the user, not just focused inside a hidden tab.
    fn focus(&self, pane: &PaneRef) -> Result<(), MuxError> {
        if let Some(ws) = pane.workspace_id.as_deref().filter(|x| !x.is_empty()) {
            self.run(&["workspace", "focus", ws])?;
        }
        if let Some(tab) = pane.tab_id.as_deref().filter(|x| !x.is_empty()) {
            self.run(&["tab", "focus", tab])?;
        }
        self.run(&["agent", "focus", &pane.pane_id]).map(|_| ())
    }

    /// `herdr agent prompt`: herdr itself rejects a blocked pane
    /// (`agent_blocked`) before sending anything; that error is surfaced.
    fn type_text(&self, pane: &PaneRef, text: &str) -> Result<(), MuxError> {
        // never type control characters into a pane (callers sanitise too; this is the floor)
        let mut text = crate::strip_controls(text);
        if text.trim().is_empty() {
            return Err(MuxError::Failed("nothing to type after removing control characters".into()));
        }
        // `herdr agent prompt` reads a text that starts with `-` as an option
        // (`unknown option: --wait now`) and takes no `--`: a leading space
        if text.starts_with('-') {
            text.insert(0, ' ');
        }
        self.run(&["agent", "prompt", &pane.pane_id, &text]).map(|_| ())
    }

    /// One workspace per entry, tabs with commands. Idempotent: an existing
    /// workspace/tab is reused by loose label match (`normalize_label`); its
    /// command runs again only in a pane that is a plain shell (a tab Herdr
    /// restored, an attach client that ended), never into a pane that runs
    /// something (R2).
    fn layout(&self, plan: &LayoutPlan) -> Result<Vec<PaneRef>, MuxError> {
        let mut out = vec![];
        for (ws_label, tabs) in &plan.workspaces {
            let first_cwd = tabs.first().map(|(_, _, c)| c.to_string_lossy().into_owned()).unwrap_or_else(|| ".".into());
            let (ws, mut fresh_tab) = self.find_or_create_workspace(ws_label, &first_cwd)?;
            let existing = self.tabs(&ws)?;
            let reuse = |tab_id: &str, cmd: &[String]| -> Result<PaneRef, MuxError> {
                let pane = self.pane_in_tab(&ws, tab_id)?;
                if self.foreground(&pane).map(|fg| crate::is_shell(&fg)).unwrap_or(false) {
                    self.pane_run(&pane.pane_id, cmd)?;
                }
                Ok(pane)
            };
            for (tab_label, cmd, cwd) in tabs {
                let want = normalize_label(tab_label);
                if let Some((tab_id, _)) = existing.iter().find(|(_, l)| normalize_label(l) == want) {
                    out.push(reuse(tab_id, cmd)?);
                    continue;
                }
                // a tab from before full names (`COMMODORE` for `F.SQ.COMMODORE`):
                // reuse it, and give it its full name
                let short = tab_label.strip_prefix(&format!("{ws_label}.")).map(normalize_label);
                if let Some((tab_id, _)) = existing.iter().find(|(_, l)| short.as_deref() == Some(normalize_label(l).as_str())) {
                    let _ = self.run(&["tab", "rename", tab_id, tab_label]);
                    out.push(reuse(tab_id, cmd)?);
                    continue;
                }
                let (tab, pane) = match fresh_tab.take() {
                    Some((tab, pane)) => {
                        self.run(&["tab", "rename", &tab, tab_label])?;
                        (tab, pane)
                    }
                    None => self.tab_create(&ws, tab_label, &cwd.to_string_lossy())?,
                };
                self.pane_run(&pane, cmd)?;
                out.push(PaneRef { pane_id: pane, tab_id: Some(tab), workspace_id: Some(ws.clone()) });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{bare_claude, known_shell, normalize_label, shell_join, shell_line};

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn cmd_gets_double_quotes() {
        let exe = r"C:\Program Files\Claude\claude.exe";
        assert_eq!(shell_line(Some("cmd"), &v(&[exe, "attach", "9aacf927"])), r#""C:\Program Files\Claude\claude.exe" attach 9aacf927"#);
        // the program is quoted even with no space; the path and case of the shell do not matter
        assert_eq!(shell_line(Some(r"C:\Windows\System32\CMD.EXE"), &v(&[r"C:\x\claude.exe", "attach", "j"])), r#""C:\x\claude.exe" attach j"#);
        // `"` doubled, `\` before a `"` doubled, `%` outside the quotes as `^%`, `'` as is
        assert_eq!(
            shell_line(Some("cmd"), &v(&[r"C:\it's here\claude.exe", "--name", r#"say "hi""#, r"C:\dir\", r"C:\my dir\", "50%", "[LUX]-LEAD", ""])),
            r#""C:\it's here\claude.exe" --name "say ""hi""" C:\dir\ "C:\my dir\\" "50"^%"" "[LUX]-LEAD" """#
        );
        assert_eq!(shell_line(Some("cmd"), &v(&["x", r#"a\"b"#])), r#""x" "a\\""b""#);
    }

    #[test]
    fn powershell_gets_the_call_operator_and_single_quotes() {
        let exe = r"C:\Program Files\Claude\claude.exe";
        for sh in ["pwsh", "powershell", r"C:\Windows\System32\WindowsPowerShell\v1.0\PowerShell.exe", "pwsh.exe"] {
            assert_eq!(shell_line(Some(sh), &v(&[exe, "attach", "9aacf927"])), r"& 'C:\Program Files\Claude\claude.exe' attach 9aacf927", "{sh}");
        }
        assert_eq!(
            shell_line(Some("pwsh"), &v(&[r"C:\it's\claude.exe", "--name", "[LUX]-LEAD", "say \"hi\" $x", "it\u{2019}s", ""])),
            "& 'C:\\it''s\\claude.exe' --name '[LUX]-LEAD' 'say \"hi\" $x' 'it\u{2019}\u{2019}s' ''"
        );
    }

    #[test]
    fn posix_shells_keep_posix_quoting() {
        let argv = v(&["/opt/my apps/claude", "attach", "it's"]);
        for sh in ["bash", "-zsh", "sh", "/usr/bin/fish", r"C:\Program Files\Git\usr\bin\bash.exe", "dash", "ksh"] {
            assert_eq!(shell_line(Some(sh), &argv), shell_join(&argv), "{sh}");
            assert!(known_shell(Some(sh)), "{sh}");
        }
        assert_eq!(shell_line(Some("bash"), &argv), "'/opt/my apps/claude' attach 'it'\\''s'");
        // Git Bash reads a Windows path in single quotes as it is
        assert_eq!(shell_line(Some("bash"), &v(&[r"C:\Program Files\claude.exe", "attach", "j"])), r"'C:\Program Files\claude.exe' attach j");
    }

    #[test]
    fn an_unknown_shell_gets_cmd_quoting_and_a_bare_claude_when_path_finds_it() {
        for sh in [None, Some("nu"), Some("yazi")] {
            assert!(!known_shell(sh));
            assert_eq!(shell_line(sh, &v(&[r"C:\a b\claude.exe", "attach", "j"])), r#""C:\a b\claude.exe" attach j"#);
            assert_eq!(shell_line(sh, &v(&["claude", "attach", "j"])), "claude attach j");
        }
        let argv = v(&[r"C:\Users\me\.local\bin\claude.exe", "attach", "j"]);
        // the same file (case and separators aside): the bare word
        assert_eq!(bare_claude(&argv, Some("c:/users/me/.local/bin/claude.exe")), v(&["claude", "attach", "j"]));
        assert_eq!(shell_line(None, &bare_claude(&argv, Some(r"C:\Users\me\.local\bin\claude.exe"))), "claude attach j");
        // another file, none on PATH, or not claude: as given
        assert_eq!(bare_claude(&argv, Some(r"C:\other\claude.exe")), argv);
        assert_eq!(bare_claude(&argv, None), argv);
        let other = v(&[r"C:\x\yazi.exe", "C:\\"]);
        assert_eq!(bare_claude(&other, Some(r"C:\x\yazi.exe")), other);
        // the same file by its real path (a link)
        #[cfg(unix)]
        {
            let td = std::env::temp_dir().join(format!("nimitz-bare-{}", std::process::id()));
            std::fs::create_dir_all(&td).unwrap();
            let real = td.join("claude");
            std::fs::write(&real, "").unwrap();
            let link = td.join("link-claude");
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(&real, &link).unwrap();
            let got = bare_claude(&v(&[real.to_str().unwrap(), "attach", "j"]), link.to_str());
            std::fs::remove_dir_all(&td).unwrap();
            assert_eq!(got, v(&["claude", "attach", "j"]));
        }
    }

    #[test]
    fn shell_join_quotes_only_what_needs_it() {
        let v = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(shell_join(&v(&["ccl"])), "ccl");
        assert_eq!(shell_join(&v(&["sh", "-c", "sleep 30"])), "sh -c 'sleep 30'");
        assert_eq!(shell_join(&v(&["tail", "-f", "/a/x.log"])), "tail -f /a/x.log");
        assert_eq!(shell_join(&v(&["echo", "it's", ""])), "echo 'it'\\''s' ''");
        assert_eq!(shell_join(&v(&["claude", "--name", "[LUX]-LEAD"])), "claude --name '[LUX]-LEAD'");
    }

    #[test]
    fn normalize_order_is_fleet_then_brackets_then_upcase() {
        assert_eq!(normalize_label("[ARGOS]-FLEET"), "ARGOS");
        assert_eq!(normalize_label("[argos]-fleet"), "ARGOS");
        assert_eq!(normalize_label("admiralty"), "ADMIRALTY");
        assert_eq!(normalize_label("[ADMIRALTY]"), "ADMIRALTY");
        assert_eq!(normalize_label("LUX"), "LUX");
        assert_eq!(normalize_label("[LUX]-LEAD"), "LUX-LEAD");
        assert_eq!(normalize_label("Yazi"), "YAZI");
        assert_eq!(normalize_label(" [BOLT]-FLEET "), "BOLT");
        // brackets deleted everywhere, not trimmed from the ends
        assert_eq!(normalize_label("a[b]c"), "ABC");
    }
}
