//! `nimitz restore` against a sandbox world (fake `claude`, fake `herdr`):
//! one test per scenario of docs/design/restore-scenarios.md (R-01..R-35; a
//! test covers several rows when the setup is shared, the ids are in its
//! name). Every run clears the process env first, so nothing here reaches
//! the real `~/.claude`, `~/.claude-nimitz` or the user's Herdr.
//!
//! The sandbox's fake herdr runs a `pane run` line for real (`exec`), so a
//! `claude attach <job>` typed into a shell pane wakes a parked job as Herdr
//! would. Pane ids are stable across the fake's "restarts" (a real Herdr
//! may renumber them: nimitz matches by label and session, never by id).
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const EXE: &str = env!("CARGO_BIN_EXE_nimitz");

struct Sb {
    _td: tempfile::TempDir,
    base: PathBuf,
    vars: BTreeMap<String, String>,
}

struct Run {
    code: i32,
    out: String,
}

fn raw(vars: &BTreeMap<String, String>, args: &[&str], extra: &[(&str, &str)]) -> Run {
    let mut c = Command::new(EXE);
    // the crash window short: a `working` row is waited for 0.2 s, not 20
    c.env_clear().env("PATH", "/usr/bin:/bin").env("NIMITZ_CRASH_WINDOW", "0.2").envs(vars).args(args);
    for (k, v) in extra {
        c.env(k, v);
    }
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let o = c.output().expect("spawn nimitz");
    let out = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    Run { code: o.status.code().unwrap_or(-1), out }
}

impl Sb {
    fn new(herdr: bool) -> Sb {
        let td = tempfile::tempdir().unwrap();
        let base = td.path().join("world");
        let mut args = vec!["sandbox", "create", base.to_str().unwrap()];
        if herdr {
            args.push("--herdr");
        }
        let r = raw(&BTreeMap::new(), &args, &[]);
        assert_eq!(r.code, 0, "{}", r.out);
        let e = raw(&BTreeMap::new(), &["sandbox", "env", base.to_str().unwrap(), "--json"], &[]);
        let vars: BTreeMap<String, String> = serde_json::from_str(e.out.trim()).expect("sandbox env --json");
        let base = base.canonicalize().unwrap();
        Sb { _td: td, base, vars }
    }
    fn run(&self, args: &[&str]) -> Run {
        raw(&self.vars, args, &[])
    }
    fn run_as(&self, id: &str, args: &[&str]) -> Run {
        let role = match id.split('/').count() {
            1 => "admiral",
            2 => "commodore",
            _ => "ship",
        };
        raw(&self.vars, args, &[("NIMITZ_ID", id), ("NIMITZ_ROLE", role)])
    }
    fn ok(&self, args: &[&str]) -> String {
        let r = self.run(args);
        assert_eq!(r.code, 0, "nimitz {args:?}:\n{}", r.out);
        r.out
    }
    fn status(&self) -> Value {
        let r = self.run(&["status", "--json"]);
        assert_eq!(r.code, 0, "{}", r.out);
        let start = r.out.find('{').unwrap();
        let mut de = serde_json::Deserializer::from_str(&r.out[start..]).into_iter::<Value>();
        de.next().unwrap().unwrap()
    }
    fn member(&self, id: &str) -> Value {
        let s = self.status();
        for f in s["fleets"].as_array().unwrap() {
            if f["admiral"]["id"] == id {
                return f["admiral"].clone();
            }
            for sq in f["squadrons"].as_array().unwrap() {
                if sq["commodore"]["id"] == id {
                    return sq["commodore"].clone();
                }
                for m in sq["ships"].as_array().unwrap() {
                    if m["id"] == id {
                        return m.clone();
                    }
                }
            }
        }
        panic!("no member {id}");
    }
    fn live(&self, id: &str) -> bool {
        !self.member(id)["session"]["pid"].is_null()
    }
    fn short(&self, id: &str) -> String {
        self.member(id)["session"]["short_id"].as_str().expect("a job").to_string()
    }
    fn sid(&self, id: &str) -> String {
        self.member(id)["session"]["session_id"].as_str().expect("a session").to_string()
    }
    fn calls(&self) -> String {
        std::fs::read_to_string(self.base.join("herdr/calls.jsonl")).unwrap_or_default()
    }
    /// `pane run` lines so far (the fake herdr's log), the pane id and the line.
    fn runs(&self) -> Vec<(String, String)> {
        self.calls()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|v| {
                let a = v["args"].as_array()?;
                (a.len() >= 4 && a[0] == "pane" && a[1] == "run").then(|| (a[2].as_str().unwrap_or("").to_string(), a[3].as_str().unwrap_or("").to_string()))
            })
            .collect()
    }
    fn n_runs(&self) -> usize {
        self.runs().len()
    }
    fn creates(&self, what: &str) -> usize {
        self.calls().lines().filter(|l| l.contains(&format!("[\"{what}\",\"create\""))).count()
    }
    /// Run the sandbox's fake `claude` (the native binary) in `cwd`.
    fn claude(&self, cwd: &Path, args: &[&str]) -> String {
        let o = Command::new(self.base.join("bin/claude")).args(args).current_dir(cwd).env_clear().env("PATH", "/usr/bin:/bin").envs(&self.vars).output().expect("fake claude");
        format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
    }
    fn cwd_of(&self, id: &str) -> PathBuf {
        PathBuf::from(self.member(id)["cwd"].as_str().expect("a folder"))
    }
    fn world(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.base.join("herdr/world.json")).unwrap()).unwrap()
    }
    fn herdr_world(&self, f: impl FnOnce(&mut Value)) {
        let p = self.base.join("herdr/world.json");
        let mut w: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        f(&mut w);
        std::fs::write(&p, serde_json::to_vec(&w).unwrap()).unwrap();
    }
    fn set_pane(&self, pane: &str, f: impl FnOnce(&mut Value)) {
        let mut f = Some(f);
        self.herdr_world(|w| {
            if let Some(p) = w["panes"].as_array_mut().unwrap().iter_mut().find(|p| p["pane_id"] == pane) {
                (f.take().expect("once"))(p);
            }
        });
        assert!(f.is_none(), "no pane {pane}");
    }
    /// The pane whose foreground runs `claude attach <job>` of the member.
    fn pane_attaching(&self, id: &str) -> String {
        let short = self.short(id);
        let w = self.world();
        w["panes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["foreground"].as_array().is_some_and(|fg| fg.len() == 3 && fg[1] == "attach" && fg[2] == short.as_str()))
            .map(|p| p["pane_id"].as_str().unwrap().to_string())
            .unwrap_or_else(|| panic!("no pane runs {id}'s attach: {w}"))
    }
    /// Herdr sees the agent: the pane's `agent_session` is the member's session.
    fn attach_all(&self) {
        let s = self.status();
        let mut pairs: Vec<(String, String)> = vec![];
        for f in s["fleets"].as_array().unwrap() {
            let mut all = vec![f["admiral"].clone()];
            for sq in f["squadrons"].as_array().unwrap() {
                all.push(sq["commodore"].clone());
                all.extend(sq["ships"].as_array().unwrap().iter().cloned());
            }
            for m in all {
                if let (Some(short), Some(sid)) = (m["session"]["short_id"].as_str(), m["session"]["session_id"].as_str()) {
                    pairs.push((short.to_string(), sid.to_string()));
                }
            }
        }
        self.herdr_world(|w| {
            for p in w["panes"].as_array_mut().unwrap() {
                let Some(fg) = p["foreground"].as_array().cloned() else { continue };
                if fg.len() == 3 && fg[1] == "attach" {
                    if let Some((_, sid)) = pairs.iter().find(|(s, _)| fg[2] == s.as_str()) {
                        p["agent_session"] = json!({"kind": "id", "source": "herdr:claude", "value": sid});
                        p["agent_status"] = json!("idle");
                        p["agent"] = json!("claude");
                    }
                }
            }
        });
    }
    /// The pane drops to its shell (ctrl-z, the attach client died, a Herdr restart).
    fn shell(&self, pane: &str) {
        self.set_pane(pane, |p| {
            p.as_object_mut().unwrap().remove("agent_session");
            p.as_object_mut().unwrap().remove("agent_status");
            p.as_object_mut().unwrap().remove("agent");
            p["foreground"] = json!(["zsh"]);
        });
    }
    /// Run the sandbox's fake `herdr` (its hidden verbs: `__restart`).
    fn herdr(&self, args: &[&str]) -> String {
        let o = Command::new(self.base.join("bin/herdr")).args(args).env_clear().env("PATH", "/usr/bin:/bin").envs(&self.vars).output().expect("fake herdr");
        format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
    }
    /// A Herdr restart from its snapshot: names and folders come back, every
    /// pane a shell; with the world's `resume_agents_on_restore` the fake
    /// then runs `claude --resume <sid>` in each agent pane, as Herdr does.
    fn herdr_restart(&self) {
        self.herdr(&["__restart"]);
    }
    /// Herdr's `[session] resume_agents_on_restore` for the fake server.
    fn resume_on_restore(&self, on: bool) {
        self.herdr_world(|w| w["resume_agents_on_restore"] = json!(on));
    }
    /// The panes where Herdr's resume holds a conversation: `/exit` typed in
    /// each (the fake's `__end`), the pane a shell again.
    fn exit_resumed(&self) -> usize {
        let held: Vec<(String, String, String)> = self.world()["panes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| {
                let fg = p["foreground"].as_array()?;
                let i = fg.iter().position(|a| a == "--resume")?;
                Some((p["pane_id"].as_str()?.to_string(), fg[i + 1].as_str()?.to_string(), p["cwd"].as_str().unwrap_or("/").to_string()))
            })
            .collect();
        for (pane, sid, cwd) in &held {
            self.claude(Path::new(cwd), &["__end", sid]);
            self.shell(pane);
        }
        held.len()
    }
    fn tabs(&self, label: &str) -> Vec<Value> {
        self.world()["tabs"].as_array().unwrap().iter().filter(|t| t["label"] == label).cloned().collect()
    }
    fn workspaces(&self, label: &str) -> Vec<Value> {
        self.world()["workspaces"].as_array().unwrap().iter().filter(|w| w["label"] == label).cloned().collect()
    }
    fn active(&self) -> Option<Value> {
        std::fs::read(self.base.join("state/active.json")).ok().and_then(|b| serde_json::from_slice(&b).ok())
    }
    fn write_active(&self, f: impl FnOnce(&mut Value)) {
        let mut v = self.active().unwrap_or_else(|| json!({"version": 1, "at": 0.0, "members": {}}));
        f(&mut v);
        std::fs::write(self.base.join("state/active.json"), serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    }
    fn session_file(&self, id: &str, f: impl FnOnce(&mut Value)) {
        let p = self.base.join("claude_home/sessions").join(format!("{}.json", self.short(id)));
        let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        f(&mut v);
        std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    }
    fn job_row(&self, short: &str, f: impl FnOnce(&mut Value)) {
        let p = self.base.join("claude_home/jobs").join(short).join("state.json");
        let mut v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        f(&mut v);
        std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    }
    /// The machine went down: every process gone, no SessionEnd for any.
    fn reboot(&self) {
        for e in std::fs::read_dir(self.base.join("claude_home/sessions")).unwrap().flatten() {
            let _ = std::fs::remove_file(e.path());
        }
        self.herdr_restart();
    }
    fn pins(&self) -> Vec<String> {
        std::fs::read(self.base.join("claude_home/jobs/pins.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
}

/// The leads in their tabs, as a first restore leaves them, seen by Herdr.
fn lay(sb: &Sb) {
    sb.ok(&["restore", "MAIN", "--yes"]);
    sb.attach_all();
    sb.ok(&["status"]);
}

/// MISLABEL runs with another identity's labels: its fix is a relaunch, which
/// no restore does; out of the active set, so a scenario's verdict is clean.
fn stop_mislabel(sb: &Sb) {
    let short = sb.short("MAIN/ARGOS/MISLABEL");
    sb.claude(&sb.cwd_of("MAIN/ARGOS/MISLABEL"), &["stop", &short]);
    sb.ok(&["status"]);
}

fn lines_with<'a>(out: &'a str, needle: &str) -> Vec<&'a str> {
    out.lines().filter(|l| l.contains(needle)).collect()
}

// ------------------------------------------------------------------ the scenarios

#[test]
fn r01_left_overnight_leads_attached_ships_parked() {
    let sb = Sb::new(true);
    lay(&sb);
    // an idle, unattached ship retires after an hour; the attached leads never do
    let clean = sb.short("MAIN/ARGOS/CLEAN");
    sb.claude(&sb.cwd_of("MAIN/ARGOS/CLEAN"), &["__retire", &clean]);
    assert_eq!(sb.member("MAIN/ARGOS/CLEAN")["status"], "Parked");
    let before = sb.n_runs();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.out_contains("[OK] MAIN: nothing to restore (") && r.contains("parked"), "{r}");
    assert!(!r.contains("--bg --resume"), "parked ships stay parked:\n{r}");
    assert_eq!(sb.n_runs(), before, "nothing typed into the attached tabs:\n{}", sb.calls());
    assert!(!sb.live("MAIN/ARGOS/CLEAN"));
    // --all wakes them, by exact id
    let sid = sb.sid("MAIN/ARGOS/CLEAN");
    let r = sb.run(&["restore", "MAIN", "--all", "--plan"]);
    assert!(r.out.contains(&format!("--bg --resume {sid}")), "{}", r.out);
    let r = sb.ok(&["restore", "MAIN", "--all", "--yes"]);
    assert!(sb.live("MAIN/ARGOS/CLEAN"), "{r}");
    assert_eq!(sb.sid("MAIN/ARGOS/CLEAN"), sid, "the same conversation");
    assert!(r.contains("[OK] MAIN: restored 0 lead(s)"), "{r}");
}

trait OutExt {
    fn out_contains(&self, s: &str) -> bool;
}
impl OutExt for String {
    fn out_contains(&self, s: &str) -> bool {
        self.contains(s)
    }
}

#[test]
fn r02_r23_lead_detached_then_retired_wakes_by_attach_and_is_repinned() {
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    let job = sb.short("MAIN/ARGOS");
    // ctrl-z: the tab is a shell; the daemon's sweep retired the lead and its pin is lost
    sb.shell(&pane);
    sb.claude(&sb.cwd_of("MAIN/ARGOS"), &["__retire", &job]);
    sb.claude(&sb.cwd_of("MAIN/ARGOS"), &["__unpin", &job]);
    assert_eq!(sb.member("MAIN/ARGOS")["status"], "Parked");
    let r = sb.run(&["restore", "MAIN", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains(&format!("herdr pane run {pane}")) && r.out.contains("the attach wakes the parked job"), "{}", r.out);
    assert!(!r.out.contains("--bg --resume") && !r.out.contains("tab create"), "one step, no resume beside the attach:\n{}", r.out);
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(sb.live("MAIN/ARGOS"), "the attach woke it:\n{r}");
    assert_eq!(sb.short("MAIN/ARGOS"), job, "the same job");
    assert!(r.contains("SessionStart resume") && r.contains("[OK] MAIN: restored 1 lead(s)"), "{r}");
    assert!(sb.pins().contains(&job), "re-pinned: {:?}", sb.pins());
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1, "no second tab");
    assert_eq!(sb.runs().iter().filter(|(p, _)| p == &pane).count(), 2, "the attach typed once more into that pane");
}

#[test]
fn r03_herdr_crashed_sessions_alive_tabs_back_as_shells() {
    let sb = Sb::new(true);
    lay(&sb);
    // a ship the user had opened, and the user's own tab in the squadron's workspace
    sb.ok(&["open", "MAIN/ARGOS/CLEAN"]);
    sb.attach_all();
    sb.herdr_world(|w| {
        let ws = w["workspaces"].as_array().unwrap().iter().find(|x| x["label"] == "MAIN.ARGOS").unwrap()["workspace_id"].clone();
        w["tabs"].as_array_mut().unwrap().push(json!({"tab_id": "w3:t9", "workspace_id": ws, "label": "notes", "focused": false}));
        w["panes"].as_array_mut().unwrap().push(json!({"pane_id": "w3:p9", "tab_id": "w3:t9", "workspace_id": ws, "cwd": "/tmp", "agent_status": "idle",
            "agent_session": {"kind": "id", "value": "99999999-0000-0000-0000-000000000000"}, "foreground": ["claude"]}));
    });
    sb.ok(&["status"]); // the active set: CLEAN attached, its place recorded
    let a = sb.active().unwrap();
    assert_eq!(a["members"]["MAIN/ARGOS/CLEAN"]["attached"], true, "{a}");
    assert_eq!(a["members"]["MAIN/ARGOS/CLEAN"]["place"]["tab"], "MAIN.ARGOS.CLEAN");
    assert_eq!(a["members"]["MAIN/ARGOS/NOLABEL"]["attached"], false);
    // Herdr restarts: names and folders back, every pane a shell (the user's own too)
    sb.herdr_restart();
    let before = sb.n_runs();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    let runs = &sb.runs()[before..];
    assert_eq!(runs.len(), 4, "three leads and the one ship that was attached:\n{r}\n{:?}", runs);
    assert!(runs.iter().all(|(_, l)| l.contains(" attach ")), "{runs:?}");
    assert!(!runs.iter().any(|(p, _)| p == "w3:p9"), "the user's notes tab is never typed into");
    assert!(!r.contains("--bg --resume"), "every job is alive: nothing resumed:\n{r}");
    assert_eq!(sb.creates("workspace"), 3, "the workspaces of the first layout only");
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1);
    assert!(r.contains("[OK] MAIN: restored 3 lead(s), 1 ship(s)"), "{r}");
    // the snapshot was not restored at all: an empty Herdr, one default workspace
    let sb2 = Sb::new(true);
    lay(&sb2);
    sb2.herdr_world(|w| {
        w["workspaces"] = json!([{"workspace_id": "w1", "label": "shell", "focused": true, "active_tab_id": "w1:t1"}]);
        w["tabs"] = json!([{"tab_id": "w1:t1", "workspace_id": "w1", "label": "1", "focused": true}]);
        w["panes"] = json!([{"pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1", "cwd": "/tmp"}]);
    });
    let r = sb2.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("[OK] MAIN: restored 3 lead(s)"), "{r}");
    for ws in ["MAIN", "MAIN.ARGOS", "MAIN.BOREAL"] {
        assert_eq!(sb2.workspaces(ws).len(), 1, "{ws}");
    }
    assert!(sb2.tabs("1").iter().all(|t| t["workspace_id"] == "w1"), "the default workspace's tab is not claimed for a fleet");
}

#[test]
fn r04_r05_a_lead_pane_or_tab_closed_is_made_again_without_a_resume() {
    let sb = Sb::new(true);
    lay(&sb);
    sb.ok(&["open", "MAIN/ARGOS/CLEAN"]);
    sb.attach_all();
    sb.ok(&["status"]);
    // the commodore's tab closed (its only pane killed); CLEAN's tab closed too
    let gone = |label: &str| {
        sb.herdr_world(|w| {
            let tab = w["tabs"].as_array().unwrap().iter().find(|t| t["label"] == label).unwrap()["tab_id"].clone();
            w["tabs"].as_array_mut().unwrap().retain(|t| t["tab_id"] != tab);
            w["panes"].as_array_mut().unwrap().retain(|p| p["tab_id"] != tab);
        });
    };
    gone("MAIN.ARGOS.COMMODORE");
    gone("MAIN.ARGOS.CLEAN");
    let r = sb.run(&["restore", "MAIN", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert_eq!(lines_with(&r.out, "herdr tab create").len(), 2, "{}", r.out);
    assert!(r.out.contains("--label MAIN.ARGOS.COMMODORE") && r.out.contains("--label MAIN.ARGOS.CLEAN"), "{}", r.out);
    assert!(!r.out.contains("--bg --resume"), "the jobs are live: no resume\n{}", r.out);
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert_eq!(sb.workspaces("MAIN.ARGOS").len(), 1, "the tab goes into the workspace that exists");
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1);
    assert_eq!(sb.tabs("MAIN.ARGOS.CLEAN").len(), 1, "{r}");
    // a ship that was never attached (NOLABEL) gets no tab, with --all too
    sb.ok(&["restore", "MAIN", "--all", "--yes"]);
    assert!(sb.tabs("MAIN.ARGOS.NOLABEL").is_empty());
}

#[test]
fn r06_a_workspace_closed_is_made_again_and_its_root_tab_claimed() {
    let sb = Sb::new(true);
    lay(&sb);
    sb.ok(&["open", "MAIN/ARGOS/CLEAN"]);
    sb.attach_all();
    sb.ok(&["status"]);
    sb.herdr_world(|w| {
        let ws = w["workspaces"].as_array().unwrap().iter().find(|x| x["label"] == "MAIN.ARGOS").unwrap()["workspace_id"].clone();
        w["workspaces"].as_array_mut().unwrap().retain(|x| x["workspace_id"] != ws);
        w["tabs"].as_array_mut().unwrap().retain(|t| t["workspace_id"] != ws);
        w["panes"].as_array_mut().unwrap().retain(|p| p["workspace_id"] != ws);
    });
    let before = sb.creates("workspace");
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert_eq!(sb.creates("workspace"), before + 1, "{r}");
    let ws = sb.workspaces("MAIN.ARGOS");
    assert_eq!(ws.len(), 1);
    let in_ws: Vec<String> = sb.world()["tabs"].as_array().unwrap().iter().filter(|t| t["workspace_id"] == ws[0]["workspace_id"]).map(|t| t["label"].as_str().unwrap().to_string()).collect();
    assert_eq!(in_ws, ["MAIN.ARGOS.COMMODORE", "MAIN.ARGOS.CLEAN"], "the root tab claimed, the ship's tab beside it, no Yazi:\n{r}");
}

#[test]
fn r07a_r07b_a_renamed_or_moved_tab_is_found_by_session_and_reused_after_a_restart() {
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    let tab = sb.world()["panes"].as_array().unwrap().iter().find(|p| p["pane_id"] == pane.as_str()).unwrap()["tab_id"].clone();
    // R-07a: renamed to `cmdr`: found by session, a hint, nothing typed
    sb.herdr_world(|w| {
        for t in w["tabs"].as_array_mut().unwrap().iter_mut().filter(|t| t["tab_id"] == tab) {
            t["label"] = json!("cmdr");
        }
    });
    let before = sb.n_runs();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("(tab.renamed)") && r.contains("herdr tab rename") && r.contains("nothing to restore"), "{r}");
    assert_eq!(sb.n_runs(), before);
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 0);
    // --rename: the one step is the rename, through the mux; nothing typed
    let r = sb.ok(&["restore", "MAIN", "--yes", "--rename"]);
    assert!(r.contains("herdr tab rename") && r.contains("MAIN.ARGOS.COMMODORE now"), "{r}");
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1);
    assert_eq!(sb.tabs("cmdr").len(), 0);
    assert_eq!(sb.n_runs(), before);
    let r = sb.ok(&["restore", "MAIN", "--yes", "--rename"]);
    assert!(r.contains("nothing to restore") && !r.contains("tab.renamed"), "{r}");
    // the rename hint again, for the restart below
    sb.herdr_world(|w| {
        for t in w["tabs"].as_array_mut().unwrap().iter_mut().filter(|t| t["tab_id"] == tab) {
            t["label"] = json!("cmdr");
        }
    });
    // after a restart `cmdr` is a shell with no session: a new tab is made, cmdr stays
    sb.herdr_restart();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1, "{r}");
    assert_eq!(sb.tabs("cmdr").len(), 1);
    // R-07b: the commodore's tab dragged into workspace MAIN: found by session; Info
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    let main_ws = sb.workspaces("MAIN")[0]["workspace_id"].clone();
    let tab = sb.world()["panes"].as_array().unwrap().iter().find(|p| p["pane_id"] == pane.as_str()).unwrap()["tab_id"].clone();
    sb.herdr_world(|w| {
        for t in w["tabs"].as_array_mut().unwrap().iter_mut().filter(|t| t["tab_id"] == tab) {
            t["workspace_id"] = main_ws.clone();
        }
        for p in w["panes"].as_array_mut().unwrap().iter_mut().filter(|p| p["tab_id"] == tab) {
            p["workspace_id"] = main_ws.clone();
        }
    });
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("(tab.moved)") && r.contains("nothing to restore"), "{r}");
    // after a restart: the name matches in the wrong workspace, a shell: reused there
    sb.herdr_restart();
    let before = sb.n_runs();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(sb.runs()[before..].iter().any(|(p, l)| p == &pane && l.contains("attach")), "claimed where it is:\n{r}");
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1, "no second tab in MAIN.ARGOS");
    assert!(r.contains("(tab.moved)"), "{r}");
}

#[test]
fn r07c_two_tabs_with_one_name_are_never_both_attached() {
    let sb = Sb::new(true);
    lay(&sb);
    let ws = sb.workspaces("MAIN.ARGOS")[0]["workspace_id"].clone();
    sb.herdr_world(|w| {
        w["tabs"].as_array_mut().unwrap().push(json!({"tab_id": "w3:t8", "workspace_id": ws, "label": "MAIN.ARGOS.COMMODORE", "focused": false}));
        w["panes"].as_array_mut().unwrap().push(json!({"pane_id": "w3:p8", "tab_id": "w3:t8", "workspace_id": ws, "cwd": "/tmp"}));
    });
    let before = sb.n_runs();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("nothing to restore"), "{r}");
    assert_eq!(sb.n_runs(), before, "the attached one is the member's; the shell is left");
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 2, "nimitz closes no tab");
    // the lead retired, both tabs shells, one in another workspace: the one in its workspace is claimed, the other reported
    let pane = sb.pane_attaching("MAIN/ARGOS");
    sb.shell(&pane);
    let main_ws = sb.workspaces("MAIN")[0]["workspace_id"].clone();
    sb.herdr_world(|w| {
        for t in w["tabs"].as_array_mut().unwrap().iter_mut().filter(|t| t["tab_id"] == "w3:t8") {
            t["workspace_id"] = main_ws.clone();
        }
        for p in w["panes"].as_array_mut().unwrap().iter_mut().filter(|p| p["pane_id"] == "w3:p8") {
            p["workspace_id"] = main_ws.clone();
        }
    });
    let job = sb.short("MAIN/ARGOS");
    sb.claude(&sb.cwd_of("MAIN/ARGOS"), &["__retire", &job]);
    let before = sb.n_runs();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    let runs = &sb.runs()[before..];
    assert_eq!(runs.len(), 1, "{r}");
    assert_eq!(runs[0].0, pane, "the tab in MAIN.ARGOS, not the one in MAIN");
    assert!(r.contains("(tab.dup)") && r.contains("close tab w3:t8"), "{r}");
    assert!(sb.live("MAIN/ARGOS"));
}

#[test]
fn r07d_r33_a_split_is_found_by_session_and_a_yazi_only_tab_is_taken() {
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    let (tab, ws) = {
        let w = sb.world();
        let p = w["panes"].as_array().unwrap().iter().find(|p| p["pane_id"] == pane.as_str()).unwrap().clone();
        (p["tab_id"].clone(), p["workspace_id"].clone())
    };
    sb.herdr_world(|w| {
        w["panes"].as_array_mut().unwrap().push(json!({"pane_id": "w3:p7", "tab_id": tab, "workspace_id": ws, "cwd": "/tmp", "foreground": ["yazi"]}));
    });
    let before = sb.n_runs();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("nothing to restore"), "{r}");
    assert_eq!(sb.n_runs(), before);
    // the attach pane died; only the Yazi pane remains: taken, never typed into
    sb.herdr_world(|w| w["panes"].as_array_mut().unwrap().retain(|p| p["pane_id"] != pane.as_str()));
    let job = sb.short("MAIN/ARGOS");
    sb.claude(&sb.cwd_of("MAIN/ARGOS"), &["__retire", &job]);
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(tab.taken)") && r.out.contains("runs `yazi`") && r.out.contains("quit `yazi` in pane w3:p7"), "{}", r.out);
    assert!(r.out.contains("[ERROR] MAIN: 1 member(s) not restored"), "{}", r.out);
    assert_eq!(sb.n_runs(), before, "nothing typed into yazi");
    assert!(!sb.live("MAIN/ARGOS"), "the job stays as it is");
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1);
    // --force: a second tab beside it, said so
    let r = sb.ok(&["restore", "MAIN", "--yes", "--force"]);
    assert!(r.contains("(tab.dup)"), "{r}");
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 2);
    assert!(sb.live("MAIN/ARGOS"));
}

#[test]
fn r08_r19_reboot_resumes_leads_and_the_active_set_leaves_parked_ships() {
    let sb = Sb::new(true);
    lay(&sb);
    stop_mislabel(&sb);
    sb.ok(&["open", "MAIN/ARGOS/CLEAN"]);
    sb.attach_all();
    // CLEAN busy and attached; NOLABEL idle, unattached; STOPPED parked for a day
    sb.session_file("MAIN/ARGOS/CLEAN", |v| v["status"] = json!("busy"));
    sb.ok(&["status"]);
    let a = sb.active().unwrap();
    assert_eq!(a["members"]["MAIN/ARGOS/CLEAN"]["status"], "busy");
    assert!(a["members"].get("MAIN/ARGOS/STOPPED").is_none(), "{a}");
    let clean = sb.short("MAIN/ARGOS/CLEAN");
    let sids: Vec<(String, String)> = ["MAIN", "MAIN/ARGOS", "MAIN/BOREAL", "MAIN/ARGOS/CLEAN", "MAIN/ARGOS/NOLABEL"].iter().map(|id| (id.to_string(), sb.sid(id))).collect();
    sb.reboot();
    sb.job_row(&clean, |j| j["state"] = json!("working")); // mid-turn when the power went
    for id in ["MAIN", "MAIN/ARGOS", "MAIN/ARGOS/CLEAN", "MAIN/ARGOS/NOLABEL"] {
        assert!(!sb.live(id));
    }
    // a status after the loss keeps the set: the members the machine took down stay, with their places
    sb.ok(&["status"]);
    let a = sb.active().unwrap();
    assert_eq!(a["members"]["MAIN/ARGOS/CLEAN"]["place"]["tab"], "MAIN.ARGOS.CLEAN", "{a}");
    assert_eq!(a["members"]["MAIN/ARGOS/CLEAN"]["status"], "busy");
    assert!(a["members"].get("MAIN/ARGOS/NOLABEL").is_some(), "{a}");
    assert!(a["members"].get("MAIN/ARGOS/STOPPED").is_none());
    let r = sb.run(&["restore", "MAIN", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("(turn.interrupted)") && r.out.contains("MAIN/ARGOS/CLEAN was mid-turn"), "{}", r.out);
    assert!(!r.out.contains("STOPPED"), "a ship parked before the loss stays parked:\n{}", r.out);
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    for (id, sid) in &sids {
        assert!(sb.live(id), "{id} back:\n{r}");
        assert_eq!(&sb.sid(id), sid, "{id}: the same conversation (SessionStart resume)");
    }
    assert!(!sb.live("MAIN/ARGOS/STOPPED"));
    assert!(!r.contains("[ERROR]"), "{r}");
    // CLEAN, NOLABEL, and DRIFT and TWIN (live at the loss too) come back; STOPPED does not
    assert!(r.contains("[OK] MAIN: restored 3 lead(s), 4 ship(s)"), "{r}");
    // the leads woke by their tabs' attach, never by a resume beside it (one wake each)
    let lead_resumes = lines_with(&r, "--bg --resume").into_iter().filter(|l| l.starts_with("[OK] MAIN:") || l.starts_with("[OK] MAIN/ARGOS:") || l.starts_with("[OK] MAIN/BOREAL:")).count();
    assert_eq!(lead_resumes, 0, "{r}");
    assert!(sb.active().unwrap()["members"]["MAIN/ARGOS/CLEAN"]["attached"] == true);
}

#[test]
fn r08_with_resume_agents_on_restore_herdr_holds_every_lead_and_nothing_is_woken() {
    // the user's real Herdr config: after a reboot and a Herdr restart, Herdr
    // runs `claude --resume <sid>` in each lead's pane; a stopped or parked
    // job's conversation then runs interactively, with no job (finding 10)
    let sb = Sb::new(true);
    lay(&sb);
    stop_mislabel(&sb);
    let sids: Vec<(String, String)> = ["MAIN", "MAIN/ARGOS", "MAIN/BOREAL"].iter().map(|id| (id.to_string(), sb.sid(id))).collect();
    sb.resume_on_restore(true);
    sb.reboot();
    for (id, sid) in &sids {
        let m = sb.member(id);
        let held = m["extra_rows"].as_array().unwrap().iter().any(|r| r["session_id"] == sid.as_str() && r["short_id"].is_null() && !r["pid"].is_null());
        assert!(held, "{id}: its conversation is held by an interactive process: {m}");
    }
    let before = sb.n_runs();
    let r = sb.run(&["restore", "MAIN", "--plan"]).out; // (--plan exits by the gate; the verdict's exit 1 is --yes's)
    assert_eq!(lines_with(&r, "(tab.resumed)").len(), 3, "{r}");
    for (id, _) in &sids {
        let pane = sb.world()["panes"].as_array().unwrap().iter().find(|p| p["agent_session"]["value"] == sb.sid(id).as_str()).map(|p| p["pane_id"].as_str().unwrap().to_string()).expect("the pane that holds it");
        assert!(r.contains(&format!("is open in pane {pane} (Herdr's resume_agents_on_restore, or a claude --resume by hand)")) && r.contains(&format!("type /exit in pane {pane}, then nimitz restore MAIN")), "{id} in its own pane:\n{r}");
    }
    for (id, sid) in &sids {
        assert!(!r.contains(&format!("--bg --resume {sid}")), "{id}: a wake would make a copy beside the held conversation:\n{r}");
    }
    assert!(!r.contains("herdr pane run"), "never typed into:\n{r}");
    // (the ships of the active set are not held: they wake as in R-08)
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "the leads are stuck: never [OK]\n{}", r.out);
    assert_eq!(sb.n_runs(), before, "nothing typed into any pane");
    assert!(r.out.contains("[ERROR] MAIN: 3 member(s) not restored") && r.out.contains("3 held in a pane"), "{}", r.out);
    for (id, _) in &sids {
        assert!(!sb.live(id), "{id} stays parked");
    }
    // the user types /exit in each pane: the leads come back by their shell tabs' attach, the same conversations
    assert_eq!(sb.exit_resumed(), 3);
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert_eq!(lines_with(&r, "the attach woke it").len(), 3, "{r}");
    for (id, sid) in &sids {
        assert!(sb.live(id), "{id}:\n{r}");
        assert_eq!(&sb.sid(id), sid);
    }
    assert!(!r.contains("[ERROR]"), "{r}");
}

#[test]
fn r02_with_resume_agents_on_restore_a_retired_lead_is_held_not_woken() {
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    let job = sb.short("MAIN/ARGOS");
    let sid = sb.sid("MAIN/ARGOS");
    sb.claude(&sb.cwd_of("MAIN/ARGOS"), &["__retire", &job]);
    sb.resume_on_restore(true);
    sb.herdr_restart();
    // the live leads: Herdr's resume is refused ("running in the background"), their panes are shells
    for id in ["MAIN", "MAIN/BOREAL"] {
        assert!(sb.member(id)["extra_rows"].as_array().unwrap().is_empty(), "{id}: a live job's resume is refused");
    }
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(tab.resumed)") && r.out.contains(&format!("is open in pane {pane}")) && r.out.contains("[ERROR] MAIN: 1 member(s) not restored"), "{}", r.out);
    assert!(!r.out.contains(&format!("--bg --resume {sid}")) && !sb.runs().iter().any(|(p, l)| p == &pane && l.contains("attach") && sb.runs().iter().filter(|(q, _)| q == p).count() > 1), "{}", r.out);
    assert!(!sb.live("MAIN/ARGOS"));
    // the other two leads got their attach again (shells after the refused resume)
    assert_eq!(r.out.lines().filter(|l| l.starts_with("[OK] MAIN") && l.contains("herdr pane run")).count(), 2, "{}", r.out);
    assert_eq!(sb.exit_resumed(), 1);
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(sb.live("MAIN/ARGOS") && sb.sid("MAIN/ARGOS") == sid, "{r}");
}

#[test]
fn a_held_process_is_matched_to_its_pane_by_pid_when_nothing_names_the_session() {
    // a `claude -c` typed by hand in the lead's tab after it was retired: no
    // `--resume <sid>` in the argv, no agent detected; only the pid says which pane
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    let job = sb.short("MAIN/ARGOS");
    let sid = sb.sid("MAIN/ARGOS");
    let dir = sb.cwd_of("MAIN/ARGOS");
    sb.claude(&dir, &["__retire", &job]);
    sb.shell(&pane);
    // the held process: this test's own pid (alive, and not the fake's pid 1)
    let pid = std::process::id() as i64;
    let now = (nimitz_core::evidence::now() * 1000.0) as i64;
    std::fs::write(
        sb.base.join("claude_home/sessions/i-byhand.json"),
        json!({"pid": pid, "sessionId": sid, "cwd": dir.to_string_lossy(), "startedAt": now, "kind": "interactive", "name": "[MAIN.ARGOS]-COMMODORE",
               "jobId": null, "status": "idle", "statusUpdatedAt": now, "bridgeSessionId": null, "entrypoint": "cli", "version": "2.1.288"}).to_string(),
    ).unwrap();
    let bin = sb.base.join("bin/claude").to_string_lossy().into_owned();
    sb.set_pane(&pane, |p| {
        p["foreground"] = json!([bin, "-c"]);
        p["pid"] = json!(pid);
    });
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(tab.resumed)") && r.out.contains(&format!("is open in pane {pane}")), "{}", r.out);
    assert!(!r.out.contains(&format!("--bg --resume {sid}")) && !sb.runs().iter().any(|(p, l)| p == &pane && l.contains("attach") && l.contains(&job) && sb.runs().iter().filter(|(q, _)| q == p).count() > 1), "{}", r.out);
    // another pid in that pane: the held process is somewhere nimitz cannot see
    let other = std::os::unix::process::parent_id() as i64;
    sb.set_pane(&pane, |p| p["pid"] = json!(other));
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(lead.interactive)") && !r.out.contains("(tab.resumed)"), "{}", r.out);
}

#[test]
fn r_doctor_warns_when_herdr_resumes_agents_on_restore() {
    let sb = Sb::new(true);
    let cfg = sb.base.join("herdr-config.toml");
    let path = cfg.to_string_lossy().into_owned();
    let doctor = |sb: &Sb| raw(&sb.vars, &["doctor"], &[("HERDR_CONFIG_PATH", path.as_str())]).out;
    // no file: the default is on
    let out = doctor(&sb);
    assert!(out.contains("[WARN] no Herdr config at") && out.contains(&format!("set resume_agents_on_restore = false under [session] in {path}")), "{out}");
    std::fs::write(&cfg, "onboarding = false\n[session]\nresume_agents_on_restore = true\n").unwrap();
    let out = doctor(&sb);
    assert!(out.contains("resume_agents_on_restore is on (the default)") && out.contains("tab.resumed"), "{out}");
    std::fs::write(&cfg, "onboarding = false\n").unwrap();
    assert!(doctor(&sb).contains("resume_agents_on_restore is on (the default)"));
    std::fs::write(&cfg, "[session]\nresume_agents_on_restore = false\n").unwrap();
    let out = doctor(&sb);
    assert!(out.contains("resume_agents_on_restore is off") && !out.contains("[WARN] Herdr"), "{out}");
    std::fs::write(&cfg, "[session\n").unwrap();
    assert!(doctor(&sb).contains("does not parse"));
    assert_eq!(std::fs::read_to_string(&cfg).unwrap(), "[session\n", "nimitz never edits the Herdr config");
}

#[test]
fn r09_claude_code_updated_overnight_nothing_to_restore() {
    let sb = Sb::new(true);
    lay(&sb);
    let adm = sb.short("MAIN");
    sb.job_row(&adm, |j| j["cliVersion"] = json!("2.1.280"));
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("nothing to restore") && r.contains("(cc.old)") && r.contains("2.1.280 under 2.1.288"), "{r}");
    assert!(!r.contains("relaunch MAIN "), "no relaunch for a version difference:\n{r}");
}

#[test]
fn r10_an_interrupted_restore_converges_on_the_second_run() {
    let sb = Sb::new(true);
    lay(&sb);
    let argos_dir = sb.cwd_of("MAIN/ARGOS").to_string_lossy().into_owned();
    // left behind: workspace MAIN.ARGOS made with its unlabelled root tab and no command;
    // the commodore live (resumed) but its tab not made; the admiral done
    sb.herdr_world(|w| {
        let ws = w["workspaces"].as_array().unwrap().iter().find(|x| x["label"] == "MAIN.ARGOS").unwrap()["workspace_id"].clone();
        w["tabs"].as_array_mut().unwrap().retain(|t| t["workspace_id"] != ws);
        w["panes"].as_array_mut().unwrap().retain(|p| p["workspace_id"] != ws);
        w["tabs"].as_array_mut().unwrap().push(json!({"tab_id": "w3:t1", "workspace_id": ws, "label": "1", "focused": false}));
        w["panes"].as_array_mut().unwrap().push(json!({"pane_id": "w3:p1", "tab_id": "w3:t1", "workspace_id": ws, "cwd": argos_dir}));
    });
    std::fs::write(sb.base.join("state/restore.json"), json!({"run": "ab12", "started": 1.0, "fleets": ["MAIN"], "total": 7, "done": 3, "steps": []}).to_string()).unwrap();
    let before = sb.creates("workspace");
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("(restore.interrupted)") && r.contains("3 of 7 steps done"), "{r}");
    assert_eq!(sb.creates("workspace"), before, "no second MAIN.ARGOS workspace:\n{r}");
    assert!(r.contains("herdr tab rename w3:t1 MAIN.ARGOS.COMMODORE") && r.contains("herdr pane run w3:p1"), "{r}");
    assert!(!r.contains("--bg --resume"), "the live commodore is not resumed again (that makes a copy)\n{r}");
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1);
    assert!(!sb.base.join("state/restore.json").exists(), "a clean finish removes the progress file");
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("nothing to restore") && !r.contains("restore.interrupted"), "{r}");
    // a run that reaches the end with a failed member is finished, not interrupted
    let job = sb.short("MAIN/ARGOS");
    sb.claude(&sb.cwd_of("MAIN/ARGOS"), &["__retire", &job]);
    sb.set_pane("w3:p1", |p| p["foreground"] = json!(["yazi"]));
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(tab.taken)"), "{}", r.out);
    assert!(!sb.base.join("state/restore.json").exists(), "the run reached its end");
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert!(!r.out.contains("restore.interrupted"), "{}", r.out);
}

#[test]
fn r11_r35_only_the_fleet_asked_and_never_the_admiralty_workspace() {
    let sb = Sb::new(true);
    sb.ok(&["start", "BETA", "--yes"]); // BETA's admiral: a job now, no tab yet
    sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(sb.tabs("BETA.ADMIRAL").is_empty(), "restore MAIN touches only MAIN");
    assert_eq!(sb.tabs("MAIN.ADMIRAL").len(), 1);
    let r = sb.ok(&["restore", "--yes"]);
    assert_eq!(sb.tabs("BETA.ADMIRAL").len(), 1, "{r}");
    assert!(r.contains("[OK] BETA: restored 1 lead(s)") && r.contains("[OK] MAIN: nothing to restore"), "{r}");
    let w = sb.world();
    assert_eq!(w["tabs"].as_array().unwrap().iter().filter(|t| t["workspace_id"] == "w1").count(), 1, "the ADMIRALTY workspace keeps its one tab");
    assert!(!sb.runs().iter().any(|(p, _)| p.starts_with("w1:")), "nothing typed into the ADMIRALTY workspace");
    let r = sb.run(&["fleet", "new", "ADMIRALTY", "--plan"]);
    assert_eq!(r.code, 1);
    assert!(r.out.contains("reserved"), "{}", r.out);
}

#[test]
fn r12_r34_run_twice_the_second_run_types_nothing() {
    let sb = Sb::new(true);
    lay(&sb);
    let before = sb.n_runs();
    let tabs = sb.world()["tabs"].as_array().unwrap().len();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("[OK] MAIN: nothing to restore"), "{r}");
    assert_eq!(sb.n_runs(), before, "typing `claude attach` into an attached pane would be a prompt to the lead");
    let r = sb.run(&["restore", "MAIN", "--yes", "--json"]);
    assert_eq!(r.code, 0, "{}", r.out);
    let start = r.out.find("{\n").expect("an Outcome on stdout");
    let o: Value = serde_json::Deserializer::from_str(&r.out[start..]).into_iter::<Value>().next().unwrap().expect("outcome json");
    assert_eq!((o["verified"].as_bool(), o["ran"].as_array().map(Vec::len)), (Some(true), Some(0)), "{o}");
    assert!(!sb.calls().contains("\"agent\",\"prompt\""));
    assert_eq!(sb.world()["tabs"].as_array().unwrap().len(), tabs);
    // the same without Herdr's agent detection: the panes run the attach (R-12, by the exact argv)
    let sb = Sb::new(true);
    sb.ok(&["restore", "MAIN", "--yes"]);
    let before = sb.n_runs();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("nothing to restore"), "{r}");
    assert_eq!(sb.n_runs(), before);
}

#[test]
fn r13_a_tab_that_names_no_member_is_reported_and_left() {
    let sb = Sb::new(true);
    lay(&sb);
    let ws = sb.workspaces("MAIN.ARGOS")[0]["workspace_id"].clone();
    sb.herdr_world(|w| {
        w["tabs"].as_array_mut().unwrap().push(json!({"tab_id": "w3:t8", "workspace_id": ws, "label": "MAIN.ARGOS.S2", "focused": false}));
        w["panes"].as_array_mut().unwrap().push(json!({"pane_id": "w3:p8", "tab_id": "w3:t8", "workspace_id": ws, "cwd": "/tmp"}));
    });
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("(tab.orphan)") && r.contains("MAIN.ARGOS.S2") && r.contains("close it"), "{r}");
    assert_eq!(sb.tabs("MAIN.ARGOS.S2").len(), 1, "nimitz closes no tab");
    assert!(!sb.runs().iter().any(|(p, _)| p == "w3:p8"));
}

#[test]
fn r14_r15_a_tab_showing_another_job_is_foreign_and_the_users_own_session_is_ignored() {
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    let clean_sid = sb.sid("MAIN/ARGOS/CLEAN");
    let clean_job = sb.short("MAIN/ARGOS/CLEAN");
    let bin = sb.base.join("bin/claude").to_string_lossy().into_owned();
    sb.set_pane(&pane, |p| {
        p["agent_session"]["value"] = json!(clean_sid);
        p["foreground"] = json!([bin, "attach", clean_job]);
    });
    let job = sb.short("MAIN/ARGOS");
    sb.claude(&sb.cwd_of("MAIN/ARGOS"), &["__retire", &job]);
    // R-15: the user's own claude in a `scratch` tab of the squadron's workspace
    let ws = sb.workspaces("MAIN.ARGOS")[0]["workspace_id"].clone();
    sb.herdr_world(|w| {
        w["tabs"].as_array_mut().unwrap().push(json!({"tab_id": "w3:t8", "workspace_id": ws, "label": "scratch", "focused": false}));
        w["panes"].as_array_mut().unwrap().push(json!({"pane_id": "w3:p8", "tab_id": "w3:t8", "workspace_id": ws, "cwd": "/tmp", "agent_status": "idle",
            "agent_session": {"kind": "id", "value": "77777777-0000-0000-0000-000000000000"}, "foreground": ["claude"]}));
    });
    let before = sb.n_runs();
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(tab.taken)") && r.out.contains(&format!("ctrl-z in pane {pane} (it shows [MAIN.ARGOS]-CLEAN")), "{}", r.out);
    assert_eq!(sb.n_runs(), before, "nothing typed into the foreign pane or the user's");
    assert!(!sb.live("MAIN/ARGOS"), "the job is left as it is");
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1);
    assert_eq!(sb.tabs("scratch").len(), 1);
    assert!(!r.out.contains("scratch"), "the user's tab is not nimitz's business:\n{}", r.out);
    let r = sb.ok(&["restore", "MAIN", "--yes", "--force"]);
    assert!(r.contains("(tab.dup)"), "{r}");
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 2);
    assert!(sb.live("MAIN/ARGOS"));
}

#[test]
fn r16_a_lead_still_interactive_from_0_3_x_is_refused_then_migrated() {
    let sb = Sb::new(true);
    let dir = sb.base.join("nimitz/beta/admiral");
    let home = sb.vars["NIMITZ_HOME"].clone();
    let settings = json!({"env": {"NIMITZ_FLEET": "BETA", "NIMITZ_ROLE": "admiral", "NIMITZ_ID": "BETA", "NIMITZ_HOME": home}}).to_string();
    let out = sb.claude(&dir, &["-c", "--name", "[BETA]-ADMIRAL", "--settings", &settings]);
    assert!(out.contains("interactive session"), "{out}");
    let m = sb.member("BETA");
    assert!(m["session"]["short_id"].is_null() && !m["session"]["pid"].is_null(), "held: {m}");
    let sid = m["session"]["session_id"].as_str().unwrap().to_string();
    let r = sb.run(&["restore", "BETA", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(lead.interactive)") && r.out.contains("/exit") && r.out.contains("nimitz restore BETA"), "{}", r.out);
    assert!(r.out.contains("[ERROR] BETA: 1 member(s) not restored"), "{}", r.out);
    assert!(sb.tabs("BETA.ADMIRAL").is_empty() && !r.out.contains("--bg --resume"), "no step for it:\n{}", r.out);
    // the user ended it with /exit: the same conversation comes back as a job, attached, pinned
    sb.claude(&dir, &["__end", &sid]);
    let r = sb.ok(&["restore", "BETA", "--yes"]);
    assert!(r.contains(&format!("--bg --resume {sid} --name")), "every flag (no row):\n{r}");
    let m = sb.member("BETA");
    assert!(!m["session"]["pid"].is_null() && !m["session"]["short_id"].is_null(), "{m}");
    assert_eq!(m["session"]["session_id"], sid);
    assert_eq!(sb.tabs("BETA.ADMIRAL").len(), 1);
    assert!(sb.pins().contains(&sb.short("BETA")));
    let reg: Value = serde_json::from_slice(&std::fs::read(sb.base.join("state/registry.json")).unwrap()).unwrap();
    assert_eq!(reg["identities"]["BETA"]["lineage"].as_array().unwrap().last().unwrap()["via"], "migrate");
}

#[test]
fn r17_a_lost_transcript_is_history_lost_and_blocks_only_that_member() {
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    let job = sb.short("MAIN/ARGOS");
    let sid = sb.sid("MAIN/ARGOS");
    sb.shell(&pane);
    sb.claude(&sb.cwd_of("MAIN/ARGOS"), &["__retire", &job]);
    for e in walkdir(&sb.base.join("claude_home/projects")) {
        if e.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&sid)) {
            std::fs::remove_file(&e).unwrap();
        }
    }
    // the admiral's tab closed too: it is made again while the commodore is refused
    sb.herdr_world(|w| {
        let tab = w["tabs"].as_array().unwrap().iter().find(|t| t["label"] == "MAIN.ADMIRAL").unwrap()["tab_id"].clone();
        w["tabs"].as_array_mut().unwrap().retain(|t| t["tab_id"] != tab);
        w["panes"].as_array_mut().unwrap().retain(|p| p["tab_id"] != tab);
    });
    let before = sb.n_runs();
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(history.lost)") && r.out.contains("nimitz start MAIN/ARGOS --fresh"), "{}", r.out);
    assert!(!sb.runs()[before..].iter().any(|(p, _)| p == &pane), "no attach into a job with no history");
    assert!(!sb.live("MAIN/ARGOS"));
    assert_eq!(sb.tabs("MAIN.ADMIRAL").len(), 1, "the rest of the fleet restores:\n{}", r.out);
    assert!(r.out.contains("[ERROR] MAIN: 1 member(s) not restored"), "{}", r.out);
}

#[test]
fn r18_a_fleet_of_another_machine_is_refused_and_pointed_at_up() {
    let sb = Sb::new(true);
    let toml = sb.base.join("nimitz/main/fleet.toml");
    let raw = std::fs::read_to_string(&toml).unwrap();
    std::fs::write(&toml, raw.replace("hostname = \"sandbox-host\"", "hostname = \"far-away-box\"")).unwrap();
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(machine.missing)") && r.out.contains("does not run on this machine") && r.out.contains("nimitz up MAIN"), "{}", r.out);
    assert!(sb.tabs("MAIN.ADMIRAL").is_empty() && !r.out.contains("--bg --resume"));
    // every fleet: that one is skipped with the warning, the others go on
    let r = sb.ok(&["restore", "--yes"]);
    assert!(r.contains("(machine.missing)") && r.contains("[OK] BETA:"), "{r}");
}

#[test]
fn r19_r20_a_busy_ship_is_only_reattached_and_a_waiting_one_is_reported() {
    let sb = Sb::new(true);
    lay(&sb);
    stop_mislabel(&sb);
    sb.ok(&["open", "MAIN/ARGOS/CLEAN"]);
    sb.attach_all();
    sb.session_file("MAIN/ARGOS/CLEAN", |v| v["status"] = json!("busy"));
    sb.session_file("MAIN/ARGOS/NOLABEL", |v| {
        v["status"] = json!("waiting");
        v["waitingFor"] = json!("permission prompt");
    });
    sb.ok(&["status"]);
    sb.herdr_restart(); // Herdr died; the jobs did not
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(!r.contains("--bg --resume"), "nothing resumed:\n{r}");
    assert!(sb.runs().iter().any(|(_, l)| l.contains(&format!("attach {}", sb.short("MAIN/ARGOS/CLEAN")))), "CLEAN reattached:\n{r}");
    assert!(r.contains("(needs-you)") && r.contains("nimitz open MAIN/ARGOS/NOLABEL"), "{r}");
    assert!(!sb.calls().contains("\"agent\",\"prompt\""), "nimitz answers nothing");
    // the machine died instead: the prompt is gone with the process
    sb.reboot();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("(prompt.lost)") && r.contains("MAIN/ARGOS/NOLABEL was waiting on you"), "{r}");
    assert!(sb.live("MAIN/ARGOS/NOLABEL"));
}

#[test]
fn r21_config_changed_on_another_machine_no_cold_launch_and_conflicts_reported() {
    let sb = Sb::new(true);
    lay(&sb);
    sb.ok(&["ship", "new", "MAIN/ARGOS/S9", "--dir", "managed", "--prompt", "You are S9.", "--yes"]);
    let sq = sb.base.join("nimitz/main/squadrons/argos");
    std::fs::copy(sq.join("squadron.toml"), sq.join("squadron (laptop's conflicted copy 2026-10-07).toml")).unwrap();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("(config.conflict)"), "{r}");
    assert!(r.contains("not started") && r.contains("MAIN/ARGOS/S9") && r.contains("nimitz up MAIN"), "{r}");
    assert!(!r.contains("--bg --name"), "a restore never cold-launches:\n{r}");
    assert!(!sb.live("MAIN/ARGOS/S9"));
}

#[test]
fn r22_the_daemon_died_leads_wake_by_attach_and_active_ships_resume() {
    let sb = Sb::new(true);
    lay(&sb);
    stop_mislabel(&sb);
    sb.ok(&["status"]);
    let clean_sid = sb.sid("MAIN/ARGOS/CLEAN");
    // every worker dead, rows kept; the attach clients exited: the tabs are shells
    for e in std::fs::read_dir(sb.base.join("claude_home/sessions")).unwrap().flatten() {
        let _ = std::fs::remove_file(e.path());
    }
    sb.herdr_restart();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    for id in ["MAIN", "MAIN/ARGOS", "MAIN/BOREAL", "MAIN/ARGOS/CLEAN"] {
        assert!(sb.live(id), "{id}:\n{r}");
    }
    assert_eq!(lines_with(&r, "the attach woke it").len(), 3, "the leads by their tabs:\n{r}");
    assert!(r.contains(&format!("--bg --resume {clean_sid}")), "the active ship by a resume:\n{r}");
    assert!(!r.contains("[ERROR]"), "{r}");
}

#[test]
fn r24_a_tab_left_in_the_agent_view_is_never_typed_into() {
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    // `/exit` inside the attach: the client shows the agent view, Herdr sees a spare session
    sb.set_pane(&pane, |p| p["agent_session"]["value"] = json!("55555555-0000-0000-0000-000000000000"));
    let before = sb.n_runs();
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(tab.agentview)") && r.out.contains(&format!("close that pane {pane}")), "{}", r.out);
    assert_eq!(sb.n_runs(), before, "nothing typed into the agent view");
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1, "no second tab, --force or not");
    let r = sb.run(&["restore", "MAIN", "--yes", "--force"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1);
}

/// A ship launched from its commodore's attach pane inherits that pane's
/// HERDR_PANE_ID, and Herdr's own hook then claims the pane for the ship.
/// That is not the agent view: the commodore's attach still runs there, so
/// restore leaves the tab alone (no tab.agentview, nothing typed, one tab).
#[test]
fn r24b_a_ships_claim_on_its_commodores_attach_pane_is_not_the_agent_view() {
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    let ship = sb.sid("MAIN/ARGOS/CLEAN");
    sb.set_pane(&pane, |p| p["agent_session"]["value"] = json!(ship));
    let before = sb.n_runs();
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(!r.out.contains("tab.agentview") && !r.out.contains("close that pane"), "{}", r.out);
    assert_eq!(sb.n_runs(), before, "nothing typed into the working attach");
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1);
}

#[test]
fn r25_a_leads_job_removed_under_its_tab_resumes_with_every_flag_and_repins() {
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    let job = sb.short("MAIN/ARGOS");
    let sid = sb.sid("MAIN/ARGOS");
    let dir = sb.cwd_of("MAIN/ARGOS");
    sb.claude(&dir, &["stop", &job]);
    sb.claude(&dir, &["rm", &job]);
    sb.shell(&pane);
    assert!(sb.member("MAIN/ARGOS")["session"].is_null(), "no row");
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains(&format!("--bg --resume {sid} --name")), "every flag:\n{r}");
    assert!(sb.live("MAIN/ARGOS"));
    assert_eq!(sb.sid("MAIN/ARGOS"), sid, "the same id");
    let new_job = sb.short("MAIN/ARGOS");
    assert!(sb.runs().iter().any(|(p, l)| p == &pane && l.contains(&format!("attach {new_job}"))), "attached in its shell tab, naming the new job:\n{r}\n{:?}", sb.runs());
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1, "no second tab");
    assert!(sb.pins().contains(&new_job), "pinned: {:?}", sb.pins());
    assert!(!r.contains("--bg --name"), "never a cold launch:\n{r}");
}

#[test]
fn r26_a_copy_from_an_earlier_race_blocks_the_ship_until_swept() {
    let sb = Sb::new(true);
    lay(&sb);
    let sid = sb.sid("MAIN/ARGOS/CLEAN");
    let job = sb.short("MAIN/ARGOS/CLEAN");
    let dir = sb.cwd_of("MAIN/ARGOS/CLEAN");
    let out = sb.claude(&dir, &["--bg", "--resume", &sid]);
    assert!(out.contains("started a copy"), "{out}");
    sb.claude(&dir, &["stop", &job]);
    let r = sb.run(&["restore", "MAIN", "--all", "--yes"]);
    assert!(r.out.contains("(name.taken)") && r.out.contains("nimitz sweep MAIN/ARGOS/CLEAN"), "{}", r.out);
    assert!(!r.out.contains(&format!("--bg --resume {sid}")), "never a second copy:\n{}", r.out);
    assert!(!sb.live("MAIN/ARGOS/CLEAN"));
}

#[test]
fn r27_no_active_set_recorded_leads_only_and_it_says_so() {
    let sb = Sb::new(true);
    lay(&sb);
    let clean = sb.short("MAIN/ARGOS/CLEAN");
    sb.claude(&sb.cwd_of("MAIN/ARGOS/CLEAN"), &["__retire", &clean]);
    std::fs::remove_file(sb.base.join("state/active.json")).unwrap();
    sb.herdr_restart();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("no active set recorded") && r.contains("--all"), "{r}");
    assert_eq!(lines_with(&r, "the attach woke it").len(), 0);
    assert_eq!(sb.runs().len(), 6, "three leads at the layout, three now");
    assert!(!sb.live("MAIN/ARGOS/CLEAN"));
    assert!(r.contains("[OK] MAIN: restored 3 lead(s), 0 ship(s)"), "{r}");
    assert!(sb.active().is_some(), "the run records a set");
}

#[test]
fn r28_a_ship_stopped_on_purpose_after_the_last_write_is_left_alone() {
    let sb = Sb::new(true);
    lay(&sb);
    let clean = sb.short("MAIN/ARGOS/CLEAN");
    let sid = sb.sid("MAIN/ARGOS/CLEAN");
    assert!(sb.active().unwrap()["members"].get("MAIN/ARGOS/CLEAN").is_some());
    sb.claude(&sb.cwd_of("MAIN/ARGOS/CLEAN"), &["stop", &clean]);
    // the hook's SessionEnd took it out of the set; a stale set from before says live
    assert!(sb.active().unwrap()["members"].get("MAIN/ARGOS/CLEAN").is_none(), "the hook keeps the set current");
    sb.write_active(|a| a["members"]["MAIN/ARGOS/CLEAN"] = json!({"job": clean, "session": sid, "status": "idle", "attached": false, "place": null}));
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("stopped on purpose") && r.contains("SessionEnd") && r.contains("nimitz start MAIN/ARGOS/CLEAN"), "{r}");
    assert!(!sb.live("MAIN/ARGOS/CLEAN"));
    assert!(r.contains("2 stopped on purpose"), "CLEAN and FAILED:\n{r}");
    // --all is the explicit "everything" switch
    sb.ok(&["restore", "MAIN", "--all", "--yes"]);
    assert!(sb.live("MAIN/ARGOS/CLEAN"));
}

#[test]
fn r29_outside_herdr_or_in_another_session() {
    let sb = Sb::new(false);
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(mux.none)") && r.out.contains("--jobs-only"), "{}", r.out);
    let stopped = sb.sid("MAIN/ARGOS/STOPPED");
    let r = sb.ok(&["restore", "MAIN", "--all", "--jobs-only", "--yes"]);
    assert!(r.contains(&format!("--bg --resume {stopped}")) && !r.contains("herdr"), "{r}");
    assert!(sb.live("MAIN/ARGOS/STOPPED"));
    // the set was recorded in Herdr session NIMITZ; this nimitz runs in `work`
    let sb = Sb::new(true);
    lay(&sb);
    sb.write_active(|a| a["herdr"] = json!({"session": "NIMITZ"}));
    let r = raw(&sb.vars, &["restore", "MAIN", "--yes"], &[("HERDR_SESSION", "work")]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(mux.other)") && r.out.contains("Herdr session NIMITZ") && r.out.contains("--here"), "{}", r.out);
    let r = raw(&sb.vars, &["restore", "MAIN", "--yes", "--here"], &[("HERDR_SESSION", "work")]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert_eq!(sb.active().unwrap()["herdr"]["session"], "work", "re-recorded here");
}

#[test]
fn r30_a_missing_folder_refuses_the_member_with_the_fix() {
    let sb = Sb::new(true);
    lay(&sb);
    let short = sb.short("MAIN/ARGOS/STOPPED");
    let sid = sb.sid("MAIN/ARGOS/STOPPED");
    std::fs::remove_dir_all(sb.cwd_of("MAIN/ARGOS/STOPPED")).unwrap();
    sb.write_active(|a| a["members"]["MAIN/ARGOS/STOPPED"] = json!({"job": short, "session": sid, "status": "idle", "attached": false, "place": null}));
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(folder.missing)") && r.out.contains("MAIN/ARGOS/STOPPED"), "{}", r.out);
    assert!(!r.out.contains(&format!("--bg --resume {sid}")), "no launch:\n{}", r.out);
}

#[test]
fn r31_remote_control_on_a_member_is_warned_after_the_run() {
    let sb = Sb::new(true);
    lay(&sb);
    sb.session_file("MAIN/ARGOS/CLEAN", |v| v["bridgeSessionId"] = json!("bridge-1"));
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("(rc.exposed)") && r.contains("MAIN/ARGOS/CLEAN"), "{r}");
}

#[test]
fn r32_a_session_id_moved_under_the_job_still_matches_the_tab() {
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    let job = sb.short("MAIN/ARGOS");
    let old = sb.sid("MAIN/ARGOS");
    sb.claude(&sb.cwd_of("MAIN/ARGOS"), &["__inside", &job, "clear"]);
    let new = sb.sid("MAIN/ARGOS");
    assert_ne!(old, new);
    assert_eq!(sb.short("MAIN/ARGOS"), job, "the job is the anchor");
    // Herdr still reports the old id for the pane (its detection lags)
    sb.set_pane(&pane, |p| p["agent_session"]["value"] = json!(old));
    let before = sb.n_runs();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("nothing to restore") && !r.contains("tab.taken"), "{r}");
    assert_eq!(sb.n_runs(), before);
    // and the other way: the pane reports the new id while the registry lags behind
    sb.set_pane(&pane, |p| p["agent_session"]["value"] = json!(new));
    let reg_path = sb.base.join("state/registry.json");
    let mut reg: Value = serde_json::from_slice(&std::fs::read(&reg_path).unwrap()).unwrap();
    reg["identities"]["MAIN/ARGOS"]["session"] = json!(old);
    std::fs::write(&reg_path, serde_json::to_vec(&reg).unwrap()).unwrap();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("nothing to restore"), "{r}");
    assert_eq!(sb.n_runs(), before);
}

#[test]
fn r_up_wakes_everything_and_makes_the_leads_tabs() {
    let sb = Sb::new(true);
    let r = sb.ok(&["up", "BETA", "--yes"]);
    assert!(sb.live("BETA"), "{r}");
    assert_eq!(sb.tabs("BETA.ADMIRAL").len(), 1, "the tab pass after the jobs:\n{r}");
    assert!(r.contains("[OK] BETA: restored 1 lead(s)"), "{r}");
    let r = sb.run(&["up", "BETA", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("then: the leads' Herdr tabs"), "{}", r.out);
}

#[test]
fn r_layout_is_the_shape_only_alias() {
    let sb = Sb::new(true);
    let r = sb.ok(&["layout", "herdr", "MAIN", "--yes"]);
    assert!(r.contains("alias of `nimitz restore`"), "{r}");
    assert_eq!(sb.tabs("MAIN.ADMIRAL").len(), 1);
    // a parked lead with no tab: a layout never resumes
    let job = sb.short("MAIN/ARGOS");
    sb.claude(&sb.cwd_of("MAIN/ARGOS"), &["__retire", &job]);
    sb.herdr_world(|w| {
        let tab = w["tabs"].as_array().unwrap().iter().find(|t| t["label"] == "MAIN.ARGOS.COMMODORE").unwrap()["tab_id"].clone();
        w["tabs"].as_array_mut().unwrap().retain(|t| t["tab_id"] != tab);
        w["panes"].as_array_mut().unwrap().retain(|p| p["tab_id"] != tab);
    });
    let r = sb.ok(&["layout", "herdr", "MAIN", "--yes"]);
    assert!(r.contains("(layout.skip)") && r.contains("no tab to wake it from") && !r.contains("--bg --resume"), "{r}");
    assert!(!sb.live("MAIN/ARGOS"));
    // an admiral may lay out (restore) its own fleet, nobody else
    assert_eq!(sb.run_as("MAIN", &["restore", "MAIN", "--plan"]).code, 0);
    assert!(sb.run_as("MAIN/ARGOS", &["restore", "MAIN", "--plan"]).out.contains("(authority)"));
    assert!(sb.run_as("BETA", &["restore", "MAIN", "--plan"]).out.contains("(authority)"));
}

#[test]
fn r_legacy_short_labels_are_renamed_and_claimed_once() {
    // the user's real layout: workspace MAIN with tab ADMIRAL, MAIN.ARGOS with tab COMMODORE
    let sb = Sb::new(true);
    lay(&sb);
    sb.herdr_world(|w| {
        for t in w["tabs"].as_array_mut().unwrap() {
            match t["label"].as_str().unwrap_or("") {
                "MAIN.ADMIRAL" => t["label"] = json!("ADMIRAL"),
                "MAIN.ARGOS.COMMODORE" => t["label"] = json!("COMMODORE"),
                _ => {}
            }
        }
    });
    sb.herdr_restart();
    let before = sb.n_runs();
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(r.contains("herdr tab rename") && r.contains("MAIN.ARGOS.COMMODORE"), "{r}");
    let runs = &sb.runs()[before..];
    assert_eq!(runs.len(), 3, "one attach per lead:\n{r}\n{runs:?}");
    for (label, short) in [("MAIN.ADMIRAL", "ADMIRAL"), ("MAIN.ARGOS.COMMODORE", "COMMODORE")] {
        assert_eq!(sb.tabs(label).len(), 1, "{label}: renamed, not made beside the short one");
        assert_eq!(sb.tabs(short).len(), 0, "{short} is gone");
    }
    assert_eq!(sb.workspaces("MAIN.ARGOS").len(), 1);
    // a legacy tab whose pane is not a shell is taken, as a full-named one would be
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    sb.herdr_world(|w| {
        for t in w["tabs"].as_array_mut().unwrap().iter_mut().filter(|t| t["label"] == "MAIN.ARGOS.COMMODORE") {
            t["label"] = json!("COMMODORE");
        }
    });
    sb.herdr_restart();
    sb.set_pane(&pane, |p| p["foreground"] = json!(["yazi"]));
    let before = sb.n_runs();
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(tab.taken)") && r.out.contains("runs `yazi`"), "{}", r.out);
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 0, "no tab made beside the taken one");
    assert!(!sb.runs()[before..].iter().any(|(p, _)| p == &pane));
}

#[test]
fn r_an_unreadable_foreground_is_never_typed_into_and_has_its_fix() {
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    let job = sb.short("MAIN/ARGOS");
    sb.shell(&pane);
    sb.claude(&sb.cwd_of("MAIN/ARGOS"), &["__retire", &job]);
    sb.herdr_world(|w| w["fail"] = json!("pane process-info"));
    let before = sb.n_runs();
    let r = sb.run(&["restore", "MAIN", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("(tab.unreadable)") && r.out.contains(&format!("herdr pane process-info --pane {pane} failed; retry nimitz restore MAIN, or rename the tab")), "{}", r.out);
    assert_eq!(sb.n_runs(), before, "nothing typed into a pane nimitz cannot see into");
    assert!(!sb.live("MAIN/ARGOS"));
    assert_eq!(sb.tabs("MAIN.ARGOS.COMMODORE").len(), 1);
}

#[test]
fn r_up_that_fails_says_the_tabs_were_not_made() {
    let sb = Sb::new(true);
    // the LAYOUT fleet has a CLAUDE.md in its folder: its up refuses
    let r = sb.run(&["up", "LAYOUT", "--yes"]);
    assert_eq!(r.code, 1, "{}", r.out);
    assert!(r.out.contains("the leads' tabs were not made; nimitz restore LAYOUT makes them once the jobs run"), "{}", r.out);
    assert!(sb.tabs("LAYOUT.ADMIRAL").is_empty());
}

fn walkdir(d: &Path) -> Vec<PathBuf> {
    let mut v = vec![];
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            v.extend(walkdir(&p));
        } else {
            v.push(p);
        }
    }
    v
}

/// A lead that `/branch`ed, then stopped, with a shell tab: the attach would
/// wake it natively on the pre-branch conversation (resumeSessionId follows
/// /clear only, r3-e2e finding 11), so restore wakes it with every flag onto
/// the branch first, then opens the tab (review item 3).
#[test]
fn r_a_branched_lead_with_a_shell_tab_is_woken_with_every_flag_then_opened() {
    let sb = Sb::new(true);
    lay(&sb);
    let pane = sb.pane_attaching("MAIN/ARGOS");
    let job = sb.short("MAIN/ARGOS");
    sb.shell(&pane);
    let out = sb.claude(&sb.cwd_of("MAIN/ARGOS"), &["__inside", &job, "branch", "[MAIN.ARGOS]-COMMODORE"]);
    let branch = out.split_whitespace().last().unwrap().to_string();
    sb.claude(&sb.cwd_of("MAIN/ARGOS"), &["stop", &job]);
    // stopped: the row would wake on the pre-branch conversation; the registry says the branch
    assert_ne!(sb.member("MAIN/ARGOS")["session"]["session_id"], branch.as_str());
    let reg: Value = serde_json::from_slice(&std::fs::read(PathBuf::from(&sb.vars["NIMITZ_STATE"]).join("registry.json")).unwrap()).unwrap();
    assert_eq!(reg["identities"]["MAIN/ARGOS"]["session"], branch.as_str());
    let r = sb.run(&["restore", "MAIN", "--plan"]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains(&format!("rm {job}")) && r.out.contains(&format!("--resume {branch} --name")), "{}", r.out);
    assert!(!r.out.contains("the attach wakes the parked job"), "no native wake by attach:\n{}", r.out);
    let r = sb.ok(&["restore", "MAIN", "--yes"]);
    assert!(sb.live("MAIN/ARGOS"), "{r}");
    assert_eq!(sb.member("MAIN/ARGOS")["session"]["session_id"], branch.as_str(), "the branch is what runs");
}
