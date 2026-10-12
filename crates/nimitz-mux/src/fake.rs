//! Fake `herdr` for tests (contracts.md sections 10, 11). Owner: mux agent.
//!
//! `fake_herdr(base, args)` behaves like the herdr CLI against a world file,
//! `<base>/world.json`, and appends every call to `<base>/calls.jsonl` as
//! `{"args": [...]}` lines. The CLI exposes it as `nimitz __fake-herdr <base> ...`.
//!
//! world.json:
//! ```json
//! {
//!   "workspaces": [{"workspace_id": "w1", "label": "ADMIRALTY", "focused": true, "active_tab_id": "w1:t1"}],
//!   "tabs":       [{"tab_id": "w1:t1", "workspace_id": "w1", "label": "ADMIRALTY", "focused": true}],
//!   "panes":      [{"pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1", "agent": "claude",
//!                   "agent_status": "idle", "terminal_title_stripped": "[ADMIRALTY]",
//!                   "cwd": "/x", "foreground_cwd": "/x",
//!                   "agent_session": {"kind": "id", "value": "<uuid>"},
//!                   "foreground": ["claude"]}],   // argv in the pane's foreground; absent = an idle shell
//!   "next_ws": 2,
//!   "fail": "workspace create",  // optional: any call starting with this errors
//!   "exec": true,                // optional: `pane run` really runs the line (sh -c, in the pane's cwd)
//!   "resume_agents_on_restore": true  // optional: `__restart` runs `claude --resume <sid>` in each agent pane, as Herdr does
//! }
//! ```
//! `__restart` (test only) is a server restart from the snapshot: every pane
//! drops to a plain shell (the pane processes died with the server); with
//! `resume_agents_on_restore` the fake then runs `<claude> --resume <sid>` in
//! each pane that showed an agent (`exec` must be on), as Herdr's
//! `[session] resume_agents_on_restore = true` does, and the pane shows that
//! process when it stayed up (a refused resume leaves a shell).
//! `api snapshot` replies with the whole shape in the exact field set of
//! herdr 0.9.3 (`tests/fixtures/herdr-0.9.3/snapshot.json`).
//! Output shapes copy the real CLI (herdr 0.9): one JSON object on stdout,
//! errors as `{"error":{"code","message"},"id"}` on stderr with exit 1.
//! `pane run` records the line in `ran` and makes it the pane's `foreground`
//! (what `pane process-info` then reports, as the real one reads from the OS).
use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::{json, Value};

/// A small world: one workspace, one tab, two panes (one idle agent, one blocked).
pub fn default_world() -> Value {
    json!({
        "workspaces": [{"workspace_id": "w1", "label": "ADMIRALTY", "focused": true, "active_tab_id": "w1:t1"}],
        "tabs": [{"tab_id": "w1:t1", "workspace_id": "w1", "label": "ADMIRALTY", "focused": true}],
        "panes": [
            {"pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1", "agent": "claude",
             "agent_status": "idle", "terminal_title_stripped": "[ADMIRALTY]",
             "cwd": "/fleet/admiralty", "foreground_cwd": "/fleet/admiralty",
             "agent_session": {"kind": "id", "source": "herdr:claude", "value": "11111111-2222-3333-4444-555555555555"}},
            {"pane_id": "w1:p2", "tab_id": "w1:t1", "workspace_id": "w1", "agent": "claude",
             "agent_status": "blocked", "terminal_title_stripped": "[ARGOS]-LEAD",
             "cwd": "/fleet/argos", "foreground_cwd": "/fleet/argos",
             "agent_session": {"kind": "id", "source": "herdr:claude", "value": "66666666-7777-8888-9999-000000000000"}}
        ],
        "next_ws": 2
    })
}

/// Write `world` (or the default) as `<base>/world.json` and clear the call log.
pub fn write_world(base: &Path, world: Option<&Value>) -> std::io::Result<()> {
    fs::create_dir_all(base)?;
    let w = world.cloned().unwrap_or_else(default_world);
    fs::write(base.join("world.json"), serde_json::to_vec_pretty(&w)?)?;
    let _ = fs::remove_file(base.join("calls.jsonl"));
    Ok(())
}

/// Every call the fake has seen, in order.
pub fn calls(base: &Path) -> Vec<Vec<String>> {
    let Ok(text) = fs::read_to_string(base.join("calls.jsonl")) else { return vec![] };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| {
            v.get("args")?.as_array().map(|a| {
                a.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect()
            })
        })
        .collect()
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn err(cmd: &str, code: &str, message: &str) -> i32 {
    eprintln!("{}", json!({"error": {"code": code, "message": message}, "id": format!("cli:{cmd}")}));
    1
}

fn ok(cmd: &str, result: Value) -> i32 {
    println!("{}", json!({"id": format!("cli:{cmd}"), "result": result}));
    0
}

/// `--label X --cwd Y --no-focus` style options -> (flag -> value; bare flags map to "").
fn opts(rest: &[&str]) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let mut i = 0;
    while i < rest.len() {
        if let Some(name) = rest[i].strip_prefix("--") {
            if matches!(name, "focus" | "no-focus") {
                out.insert(name.to_string(), String::new());
            } else {
                out.insert(name.to_string(), rest.get(i + 1).unwrap_or(&"").to_string());
                i += 1;
            }
        }
        i += 1;
    }
    out
}

pub fn fake_herdr(base: &Path, args: &[String]) -> i32 {
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(base.join("calls.jsonl")) {
        // the server-naming env the call ran with (a test asserts what a
        // member's herdr child really got, not the map nimitz built)
        let env: serde_json::Map<String, Value> = ["HERDR_SESSION", "HERDR_SOCKET_PATH", "HERDR_CLIENT_SOCKET_PATH", "HERDR_ENV"].iter().filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), Value::from(v)))).collect();
        let _ = writeln!(f, "{}", json!({ "args": args, "env": env }));
    }
    let mut world: Value = fs::read(base.join("world.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}));
    if !world.is_object() {
        world = json!({});
    }
    for k in ["workspaces", "tabs", "panes"] {
        if !world[k].is_array() {
            world[k] = json!([]);
        }
    }
    let joined = args.join(" ");
    if let Some(fail) = world.get("fail").and_then(Value::as_str) {
        if !fail.is_empty() && joined.starts_with(fail) {
            return err("fail", "injected_failure", &format!("world.json fail matched `{joined}`"));
        }
    }
    // `--session <name>` (NIMITZ_HERDR_SESSION) is logged with the call and stripped
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let a: Vec<&str> = if a.len() >= 2 && a[0] == "--session" { a[2..].to_vec() } else { a };
    let code = dispatch(&mut world, &a);
    let _ = fs::write(base.join("world.json"), serde_json::to_vec_pretty(&world).unwrap_or_default());
    code
}

fn arr(world: &Value, k: &str) -> Vec<Value> {
    world.get(k).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn find<'a>(items: &'a [Value], key: &str, id: &str) -> Option<&'a Value> {
    items.iter().find(|x| s(x, key) == id)
}

fn new_pane(world: &mut Value, ws: &str, tab: &str, cwd: &str) -> Value {
    let n = arr(world, "panes").iter().filter(|p| s(p, "workspace_id") == ws).count() + 1;
    let pane = json!({
        "pane_id": format!("{ws}:p{n}"), "tab_id": tab, "workspace_id": ws,
        "cwd": cwd, "focused": false
    });
    world["panes"].as_array_mut().expect("panes").push(pane.clone());
    pane
}

fn new_tab(world: &mut Value, ws: &str, label: &str, focused: bool) -> Value {
    let n = arr(world, "tabs").iter().filter(|t| s(t, "workspace_id") == ws).count() + 1;
    let tab = json!({"tab_id": format!("{ws}:t{n}"), "workspace_id": ws, "label": label, "focused": focused, "number": n});
    world["tabs"].as_array_mut().expect("tabs").push(tab.clone());
    tab
}

fn num(v: &Value, k: &str, d: u64) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(d)
}

/// `api snapshot`, field for field as herdr 0.9.3 replies
/// (`tests/fixtures/herdr-0.9.3/snapshot.json`): `panes[]` carry no agent
/// fields; `agents[]` lists the panes that show an agent with their
/// `agent_session`; one layout per tab.
fn snapshot(world: &Value) -> Value {
    let (ws, tabs, panes) = (arr(world, "workspaces"), arr(world, "tabs"), arr(world, "panes"));
    let status = |v: &Value| {
        let st = s(v, "agent_status");
        if st.is_empty() { "unknown".to_string() } else { st }
    };
    let title = |p: &Value| {
        let t = s(p, "terminal_title_stripped");
        if t.is_empty() { "user@host:~".to_string() } else { t }
    };
    let fg_cwd = |p: &Value| {
        let f = s(p, "foreground_cwd");
        if f.is_empty() { s(p, "cwd") } else { f }
    };
    let focused = |v: &Value| v.get("focused").and_then(Value::as_bool).unwrap_or(false);
    let workspaces: Vec<Value> = ws
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let id = s(w, "workspace_id");
            let my_tabs: Vec<&Value> = tabs.iter().filter(|t| s(t, "workspace_id") == id).collect();
            let active = s(w, "active_tab_id");
            let active = if active.is_empty() { my_tabs.first().map(|t| s(t, "tab_id")).unwrap_or_default() } else { active };
            json!({
                "active_tab_id": active,
                "agent_status": status(w), "focused": focused(w),
                "label": s(w, "label"), "number": num(w, "number", i as u64 + 1),
                "pane_count": panes.iter().filter(|p| s(p, "workspace_id") == id).count(),
                "tab_count": my_tabs.len(), "workspace_id": id,
            })
        })
        .collect();
    let tab_rows: Vec<Value> = tabs
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let id = s(t, "tab_id");
            json!({"agent_status": status(t), "focused": focused(t), "label": s(t, "label"),
                   "number": num(t, "number", i as u64 + 1), "pane_count": panes.iter().filter(|p| s(p, "tab_id") == id).count(),
                   "tab_id": id, "workspace_id": s(t, "workspace_id")})
        })
        .collect();
    let pane_rows: Vec<Value> = panes
        .iter()
        .map(|p| {
            json!({"agent_status": status(p), "cwd": s(p, "cwd"), "focused": focused(p),
                   "foreground_cwd": fg_cwd(p),
                   "pane_id": s(p, "pane_id"), "revision": num(p, "revision", 1),
                   "scroll": {"max_offset_from_bottom": 0, "offset_from_bottom": 0, "viewport_rows": 40},
                   "tab_id": s(p, "tab_id"), "terminal_id": format!("term_{}", s(p, "pane_id").replace(':', "")),
                   "terminal_title": title(p), "terminal_title_stripped": title(p), "workspace_id": s(p, "workspace_id")})
        })
        .collect();
    let agents: Vec<Value> = panes
        .iter()
        .filter(|p| !s(p, "agent_status").is_empty())
        .map(|p| {
            let kind = s(p, "agent");
            let kind = if kind.is_empty() { "claude".to_string() } else { kind };
            // AgentSessionInfo: {agent, kind, source, value}; a world may give only the value
            let session = p.get("agent_session").filter(|x| x.is_object()).map(|x| {
                let or = |k: &str, d: &str| { let v = s(x, k); if v.is_empty() { d.to_string() } else { v } };
                json!({"agent": or("agent", &kind), "kind": or("kind", "id"), "source": or("source", "herdr:claude"), "value": s(x, "value")})
            }).unwrap_or(Value::Null);
            json!({"agent": kind,
                   "agent_session": session, "agent_status": status(p),
                   "completion_seq": 0, "cwd": s(p, "cwd"), "focused": focused(p),
                   "foreground_cwd": fg_cwd(p),
                   "pane_id": s(p, "pane_id"), "revision": num(p, "revision", 1), "state_change_seq": 0,
                   "tab_id": s(p, "tab_id"), "terminal_id": format!("term_{}", s(p, "pane_id").replace(':', "")),
                   "terminal_title": title(p), "terminal_title_stripped": title(p), "workspace_id": s(p, "workspace_id")})
        })
        .collect();
    let layouts: Vec<Value> = tabs
        .iter()
        .map(|t| {
            let id = s(t, "tab_id");
            let in_tab: Vec<Value> = panes.iter().filter(|p| s(p, "tab_id") == id).map(|p| json!({"focused": false, "pane_id": s(p, "pane_id"), "rect": {"height": 40, "width": 120, "x": 0, "y": 0}})).collect();
            json!({"area": {"height": 40, "width": 120, "x": 0, "y": 0}, "focused_pane_id": in_tab.first().map(|p| s(p, "pane_id")).unwrap_or_default(),
                   "panes": in_tab, "splits": [], "tab_id": id, "workspace_id": s(t, "workspace_id"), "zoomed": false})
        })
        .collect();
    let focused_ws = ws.iter().find(|w| focused(w)).map(|w| s(w, "workspace_id"));
    let focused_tab = tabs.iter().find(|t| focused(t)).map(|t| s(t, "tab_id"));
    json!({
        "agents": agents, "focused_pane_id": Value::Null, "focused_tab_id": focused_tab, "focused_workspace_id": focused_ws,
        "layouts": layouts, "panes": pane_rows, "protocol": 22, "tabs": tab_rows, "version": "0.9.3", "workspaces": workspaces,
    })
}

fn dispatch(world: &mut Value, a: &[&str]) -> i32 {
    match a {
        ["__restart"] => {
            let exec = world.get("exec").and_then(Value::as_bool).unwrap_or(false);
            let resume = world.get("resume_agents_on_restore").and_then(Value::as_bool).unwrap_or(false);
            for p in world["panes"].as_array_mut().expect("panes") {
                let sid = p.pointer("/agent_session/value").and_then(Value::as_str).map(String::from);
                // the claude that ran there (an attach), else `claude` on PATH
                let bin = p["foreground"].as_array().and_then(|a| a.first()).and_then(Value::as_str).filter(|b| b.ends_with("claude") || b.ends_with("claude.exe")).map(String::from).unwrap_or_else(|| "claude".into());
                let o = p.as_object_mut().expect("pane");
                for k in ["agent_session", "agent_status", "agent", "pid"] {
                    o.remove(k);
                }
                o.insert("foreground".into(), json!(["zsh"]));
                let (Some(sid), true) = (sid, resume && exec) else { continue };
                let cwd = s(p, "cwd");
                let mut c = shell_line(&format!("{bin} --resume {sid}"));
                c.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                if !cwd.is_empty() && Path::new(&cwd).is_dir() {
                    c.current_dir(&cwd);
                }
                if c.status().map(|st| st.success()).unwrap_or(false) {
                    p["foreground"] = json!([bin, "--resume", sid]);
                    p["agent_session"] = json!({"kind": "id", "source": "herdr:claude", "value": sid});
                    p["agent_status"] = json!("idle");
                    p["agent"] = json!("claude");
                    p["pid"] = json!(nimitz_core::sandbox::LIVE_PID);
                }
            }
            ok("__restart", json!({"type": "ok"}))
        }
        ["api", "snapshot"] => {
            let snap = snapshot(world);
            ok("api:snapshot", json!({"snapshot": snap}))
        }
        ["agent", "list"] => {
            let agents: Vec<Value> = arr(world, "panes").into_iter().filter(|p| !s(p, "agent_status").is_empty()).collect();
            ok("agent:list", json!({"type": "agent_list", "agents": agents}))
        }
        ["agent", "focus", target] => {
            if find(&arr(world, "panes"), "pane_id", target).is_none() {
                return err("agent:focus", "agent_not_found", &format!("agent target {target} not found"));
            }
            ok("agent:focus", json!({"type": "pane_focused", "pane_id": target}))
        }
        ["agent", "prompt", target, text, ..] => {
            let panes = arr(world, "panes");
            let Some(p) = find(&panes, "pane_id", target) else {
                return err("agent:prompt", "agent_not_found", &format!("agent target {target} not found"));
            };
            if s(p, "agent_status") == "blocked" {
                return err("agent:prompt", "agent_blocked", &format!("agent {target} is blocked; prompt not sent"));
            }
            let agent = p.clone();
            for q in world["panes"].as_array_mut().expect("panes") {
                if s(q, "pane_id") == *target {
                    if !q["prompts"].is_array() {
                        q["prompts"] = json!([]);
                    }
                    q["prompts"].as_array_mut().expect("prompts").push(json!(text));
                }
            }
            ok("agent:prompt", json!({"type": "agent_prompted", "agent": agent}))
        }
        ["workspace", "list"] => {
            let ws = arr(world, "workspaces");
            ok("workspace:list", json!({"type": "workspace_list", "workspaces": ws}))
        }
        ["workspace", "focus", target] => {
            if find(&arr(world, "workspaces"), "workspace_id", target).is_none() {
                return err("workspace:focus", "workspace_not_found", &format!("workspace {target} not found"));
            }
            ok("workspace:focus", json!({"type": "workspace_focused", "workspace_id": target}))
        }
        ["workspace", "create", rest @ ..] => {
            let o = opts(rest);
            let n = world.get("next_ws").and_then(Value::as_u64).unwrap_or(1);
            world["next_ws"] = json!(n + 1);
            let ws_id = format!("w{n}");
            let focused = o.contains_key("focus");
            let label = o.get("label").cloned().unwrap_or_else(|| n.to_string());
            let cwd = o.get("cwd").cloned().unwrap_or_default();
            let tab = new_tab(world, &ws_id, "1", focused);
            let pane = new_pane(world, &ws_id, &s(&tab, "tab_id"), &cwd);
            let ws = json!({"workspace_id": ws_id, "label": label, "focused": focused,
                            "active_tab_id": s(&tab, "tab_id"), "number": n});
            world["workspaces"].as_array_mut().expect("workspaces").push(ws.clone());
            // the schema's reply (ResponseResult workspace_created): workspace, tab and root_pane
            ok("workspace:create", json!({"type": "workspace_created", "workspace": ws, "tab": tab, "root_pane": pane}))
        }
        ["tab", "list", rest @ ..] => {
            let o = opts(rest);
            let tabs: Vec<Value> = arr(world, "tabs")
                .into_iter()
                .filter(|t| o.get("workspace").map(|w| s(t, "workspace_id") == *w).unwrap_or(true))
                .collect();
            ok("tab:list", json!({"type": "tab_list", "tabs": tabs}))
        }
        ["tab", "focus", target] => {
            if find(&arr(world, "tabs"), "tab_id", target).is_none() {
                return err("tab:focus", "tab_not_found", &format!("tab {target} not found"));
            }
            ok("tab:focus", json!({"type": "tab_focused", "tab_id": target}))
        }
        ["tab", "create", rest @ ..] => {
            let o = opts(rest);
            let Some(ws) = o.get("workspace").cloned() else {
                return err("tab:create", "workspace_not_found", "tab create needs --workspace");
            };
            if find(&arr(world, "workspaces"), "workspace_id", &ws).is_none() {
                return err("tab:create", "workspace_not_found", &format!("workspace {ws} not found"));
            }
            let tab = new_tab(world, &ws, o.get("label").map(String::as_str).unwrap_or(""), o.contains_key("focus"));
            let pane = new_pane(world, &ws, &s(&tab, "tab_id"), o.get("cwd").map(String::as_str).unwrap_or(""));
            // the schema's reply (ResponseResult tab_created): tab and root_pane
            ok("tab:create", json!({"type": "tab_created", "tab": tab, "root_pane": pane}))
        }
        ["tab", "rename", target, label @ ..] if !label.is_empty() => {
            let mut hit = false;
            for t in world["tabs"].as_array_mut().expect("tabs") {
                if s(t, "tab_id") == *target {
                    t["label"] = json!(label.join(" "));
                    hit = true;
                }
            }
            if !hit {
                return err("tab:rename", "tab_not_found", &format!("tab {target} not found"));
            }
            ok("tab:rename", json!({"type": "tab_renamed", "tab_id": target}))
        }
        ["pane", "list"] => {
            let panes = arr(world, "panes");
            ok("pane:list", json!({"type": "pane_list", "panes": panes}))
        }
        ["pane", "process-info", "--pane", target] | ["pane", "process-info", target] => {
            let panes = arr(world, "panes");
            let Some(p) = find(&panes, "pane_id", target) else {
                return err("pane:process-info", "pane_not_found", &format!("pane {target} not found"));
            };
            // the foreground process group: the shell alone when idle (herdr-shape.md, PaneProcessInfo)
            // (a pane that shows an agent and names no foreground runs that
            // session: `claude --resume <its id>`, which backs Herdr's claim)
            let shown = p.pointer("/agent_session/value").and_then(Value::as_str).map(|sid| vec!["claude".to_string(), "--resume".into(), sid.into()]);
            let argv: Vec<String> = p.get("foreground").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).or(shown).unwrap_or_else(|| vec!["zsh".into()]);
            let pid = p.get("pid").and_then(Value::as_i64).unwrap_or(4242);
            let proc_ = json!({"pid": pid, "name": argv.first().cloned().unwrap_or_default(), "argv0": argv.first().cloned().unwrap_or_default(),
                               "argv": argv, "cmdline": argv.join(" "), "cwd": s(p, "cwd")});
            // as herdr 0.9.3 replies (tests/fixtures/herdr-0.9.3/process-info.json): nested under process_info
            ok("pane:process_info", json!({"type": "pane_process_info", "process_info": {"pane_id": target, "shell_pid": 4241, "tty": "/dev/pts/9",
                                           "foreground_process_group_id": 4242, "foreground_processes": [proc_]}}))
        }
        ["pane", "close", target] => {
            let panes = world["panes"].as_array_mut().expect("panes");
            let Some(at) = panes.iter().position(|p| s(p, "pane_id") == *target) else {
                return err("pane:close", "pane_not_found", &format!("pane {target} not found"));
            };
            let tab = s(&panes.remove(at), "tab_id");
            // like herdr: a tab whose last pane closes goes with it
            if !panes.iter().any(|p| s(p, "tab_id") == tab) {
                world["tabs"].as_array_mut().expect("tabs").retain(|t| s(t, "tab_id") != tab);
            }
            ok("pane:close", json!({"type": "pane_closed", "pane_id": target}))
        }
        ["pane", "run", target, cmd @ ..] if !cmd.is_empty() => {
            let exec = world.get("exec").and_then(Value::as_bool).unwrap_or(false);
            let mut hit = None;
            for p in world["panes"].as_array_mut().expect("panes") {
                if s(p, "pane_id") == *target {
                    if !p["ran"].is_array() {
                        p["ran"] = json!([]);
                    }
                    // like herdr: the arguments joined with spaces, as typed into the shell
                    let line = cmd.join(" ");
                    p["ran"].as_array_mut().expect("ran").push(json!(line));
                    // ... which now runs in the pane's foreground (quotes dropped: enough for `claude attach <job>`)
                    p["foreground"] = json!(line.split_whitespace().map(|w| w.trim_matches('\'')).collect::<Vec<_>>());
                    hit = Some((line, s(p, "cwd")));
                }
            }
            let Some((line, cwd)) = hit else {
                return err("pane:run", "pane_not_found", &format!("pane {target} not found"));
            };
            // `"exec": true` (a sandbox): the shell really runs the line in the
            // pane's folder, as Herdr's pty would (a `claude attach <job>` of
            // the fake claude wakes a parked job); its output is dropped
            if exec {
                let mut c = shell_line(&line);
                c.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                if !cwd.is_empty() && Path::new(&cwd).is_dir() {
                    c.current_dir(&cwd);
                }
                let _ = c.status();
            }
            ok("pane:run", json!({"type": "ok"}))
        }
        ["pane", "split", target, rest @ ..] => {
            let o = opts(rest);
            let panes = arr(world, "panes");
            let Some(p) = find(&panes, "pane_id", target) else {
                return err("pane:split", "pane_not_found", &format!("pane {target} not found"));
            };
            // like herdr 0.9: --direction is required
            if !matches!(o.get("direction").map(String::as_str), Some("right" | "down")) {
                return err("pane:split", "usage", "usage: herdr pane split <pane_id> --direction right|down");
            }
            let (ws, tab) = (s(p, "workspace_id"), s(p, "tab_id"));
            let pane = new_pane(world, &ws, &tab, o.get("cwd").map(String::as_str).unwrap_or(""));
            // the schema's reply to a split: pane_info (pane_created is an event tag)
            ok("pane:split", json!({"type": "pane_info", "pane": pane}))
        }
        _ => {
            eprintln!("[fake herdr] unsupported: {a:?}");
            2
        }
    }
}

/// The command a pane's shell would run for `line`: `sh -c` on Unix; on
/// Windows (no `sh` to count on) the line split into words, `'...'` and
/// `"..."` quoting kept, and run directly.
fn shell_line(line: &str) -> std::process::Command {
    #[cfg(unix)]
    {
        let mut c = std::process::Command::new("sh");
        c.arg("-c").arg(line);
        c
    }
    #[cfg(not(unix))]
    {
        let mut words: Vec<String> = vec![];
        let (mut cur, mut quote, mut any) = (String::new(), None::<char>, false);
        for ch in line.chars() {
            match (quote, ch) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), c) => cur.push(c),
                (None, '\'' | '"') => {
                    quote = Some(ch);
                    any = true;
                }
                (None, c) if c.is_whitespace() => {
                    if any || !cur.is_empty() {
                        words.push(std::mem::take(&mut cur));
                    }
                    any = false;
                }
                (None, c) => cur.push(c),
            }
        }
        if any || !cur.is_empty() {
            words.push(cur);
        }
        let mut c = std::process::Command::new(words.first().cloned().unwrap_or_default());
        c.args(words.iter().skip(1));
        c
    }
}
