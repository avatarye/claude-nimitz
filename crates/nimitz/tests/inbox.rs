//! `nimitz inbox` and `nimitz ask` in a sandbox (docs/design/aide.md S1):
//! the aide's loop by hand, who may do what, request and reply.
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
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
    // (a needs-you delivery gives up at once: no process outlives the test by minutes)
    let o = Command::new(EXE).env_clear().env("PATH", "/usr/bin:/bin").env("NIMITZ_NOTIFY_RETRY", "0").envs(vars).envs(extra.iter().copied()).args(args).stdin(Stdio::null()).output().unwrap();
    Run { code: o.status.code().unwrap_or(-1), out: format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)) }
}

const KB: &[(&str, &str)] = &[("NIMITZ_ROLE", "aide"), ("NIMITZ_ID", "MAIN/@KB"), ("NIMITZ_FLEET", "MAIN")];
const REF: &[(&str, &str)] = &[("NIMITZ_ROLE", "aide"), ("NIMITZ_ID", "MAIN/@REF"), ("NIMITZ_FLEET", "MAIN")];
const SHIP: &[(&str, &str)] = &[("NIMITZ_ROLE", "ship"), ("NIMITZ_ID", "MAIN/ARGOS/CLEAN"), ("NIMITZ_FLEET", "MAIN"), ("NIMITZ_SQUADRON", "ARGOS")];
const ADMIRAL: &[(&str, &str)] = &[("NIMITZ_ROLE", "admiral"), ("NIMITZ_ID", "MAIN"), ("NIMITZ_FLEET", "MAIN")];

impl Sb {
    /// The sandbox world, with two aides in fleet MAIN: KB (notes,
    /// rate 3 a minute) and REF (queries).
    fn new() -> Sb {
        let td = tempfile::tempdir().unwrap();
        let base = td.path().join("world");
        let r = raw(&BTreeMap::new(), &["sandbox", "create", base.to_str().unwrap()], &[]);
        assert_eq!(r.code, 0, "{}", r.out);
        let e = raw(&BTreeMap::new(), &["sandbox", "env", base.to_str().unwrap(), "--json"], &[]);
        let vars: BTreeMap<String, String> = serde_json::from_str(e.out.trim()).unwrap();
        let sb = Sb { _td: td, base: base.canonicalize().unwrap(), vars };
        let f = sb.fleet_toml();
        let mut t = std::fs::read_to_string(&f).unwrap();
        t.push_str("\n[[aide]]\nname = \"KB\"\nmodel = \"sonnet\"\n[aide.inbox]\ntypes = [\"wiki.note.v1\"]\nper_sender = \"3/m\"\nmax_tries = 2\n");
        t.push_str("\n[[aide]]\nname = \"REF\"\nmodel = \"sonnet\"\n[aide.inbox]\ntypes = [\"wiki.query.v1\"]\n");
        std::fs::write(&f, t).unwrap();
        sb
    }
    fn fleet_toml(&self) -> PathBuf {
        PathBuf::from(&self.vars["NIMITZ_HOME"]).join("main/fleet.toml")
    }
    fn run(&self, args: &[&str]) -> Run {
        raw(&self.vars, args, &[])
    }
    fn run_as(&self, who: &[(&str, &str)], args: &[&str]) -> Run {
        raw(&self.vars, args, who)
    }
    /// A Claude Code session above the caller (a non-member session: the
    /// user's mod): its session file under the fake pid `pid`.
    fn session(&self, pid: i64, sid: &str, job: &str) -> Vec<(String, String)> {
        let d = PathBuf::from(&self.vars["NIMITZ_CLAUDE_HOME"]).join("sessions");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(format!("{pid}.json")), json!({"pid": pid, "sessionId": sid, "jobId": job, "cwd": "/tmp", "status": "idle", "kind": "background"}).to_string()).unwrap();
        vec![("NIMITZ_TEST_ANCESTORS".into(), pid.to_string())]
    }
    fn json(&self, who: &[(&str, &str)], args: &[&str]) -> Value {
        let mut a: Vec<&str> = args.to_vec();
        a.push("--json");
        let r = self.run_as(who, &a);
        serde_json::from_str(r.out.trim()).unwrap_or_else(|e| panic!("{args:?}: {e}\n{}", r.out))
    }
}

fn pairs(v: &[(String, String)]) -> Vec<(&str, &str)> {
    v.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect()
}

#[test]
fn the_aide_loop_by_hand_post_claim_done_and_status() {
    let sb = Sb::new();
    let p = sb.json(SHIP, &["inbox", "post", "MAIN/@KB", "--type", "wiki.note.v1", "--payload", r#"{"body": "leases are 15m", "kind": "fact"}"#]);
    let id = p["id"].as_str().unwrap().to_string();
    assert!(p.get("token").is_none(), "a note is fire and forget: {p}");
    let st = sb.json(&[], &["inbox", "status", "MAIN/@KB"]);
    assert_eq!(st["pending"], 1, "{st}");
    assert_eq!(st["recent"][0]["sender"], "MAIN/ARGOS/CLEAN");
    // the aide claims: the item as JSON
    let c = sb.run_as(KB, &["inbox", "claim", "MAIN/@KB"]);
    assert_eq!(c.code, 0, "{}", c.out);
    let item: Value = serde_json::from_str(&c.out).unwrap();
    assert_eq!((item["id"].as_str(), item["tries"].as_u64()), (Some(id.as_str()), Some(1)));
    assert_eq!(item["sender"]["kind"], "member");
    let d = sb.run_as(KB, &["inbox", "done", "MAIN/@KB", &id, "--result", r#"{"commit": "abc1234", "pages": ["wiki/lease.md"]}"#]);
    assert_eq!(d.code, 0, "{}", d.out);
    // empty: exit 3
    assert_eq!(sb.run_as(KB, &["inbox", "claim", "MAIN/@KB"]).code, 3);
    let st = sb.json(&[], &["inbox", "status", "MAIN/@KB"]);
    assert_eq!((st["pending"].as_u64(), st["done"].as_u64()), (Some(0), Some(1)), "{st}");
    // status with no aide named: every aide, and the line form
    let all = sb.run(&["inbox", "status"]);
    assert!(all.out.contains("MAIN/@KB  empty") && all.out.contains("MAIN/@REF  empty"), "{}", all.out);
}

#[test]
fn who_may_do_what() {
    let sb = Sb::new();
    let post = |who: &[(&str, &str)]| sb.run_as(who, &["inbox", "post", "MAIN/@KB", "--type", "wiki.note.v1", "--payload", r#"{"body": "x"}"#]);
    assert_eq!(post(SHIP).code, 0, "open policy: a ship posts");
    let own = post(KB);
    assert!(own.code != 0 && own.out.contains("(authority)"), "an aide does not post to itself: {}", own.out);
    let c = sb.run_as(SHIP, &["inbox", "claim", "MAIN/@KB"]);
    assert!(c.code != 0 && c.out.contains("(authority)"), "{}", c.out);
    let other_aide = sb.run_as(REF, &["inbox", "claim", "MAIN/@KB"]);
    assert!(other_aide.out.contains("(authority)"), "{}", other_aide.out);
    // a dead letter: retry and drop are the user's or the parent's (the admiral)
    let id = serde_json::from_str::<Value>(&sb.run_as(KB, &["inbox", "claim", "MAIN/@KB"]).out).unwrap()["id"].as_str().unwrap().to_string();
    let f = sb.run_as(KB, &["inbox", "fail", "MAIN/@KB", &id, "--reason", "the payload names no page", "--permanent"]);
    assert!(f.out.contains("dead letter") && f.out.contains(&format!("nimitz inbox retry MAIN/@KB {id}")), "{}", f.out);
    let st = sb.json(&[], &["inbox", "status", "MAIN/@KB"]);
    assert_eq!(st["dead"][0]["reason"], "the payload names no page", "{st}");
    assert!(sb.run_as(SHIP, &["inbox", "retry", "MAIN/@KB", &id]).out.contains("(authority)"));
    assert!(sb.run_as(KB, &["inbox", "retry", "MAIN/@KB", &id]).out.contains("(authority)"));
    assert_eq!(sb.run_as(ADMIRAL, &["inbox", "retry", "MAIN/@KB", &id]).code, 0);
    assert_eq!(sb.json(&[], &["inbox", "status", "MAIN/@KB"])["pending"], 1);
    assert_eq!(sb.run(&["inbox", "drop", "MAIN/@KB", &id]).code, 0, "the user drops");
    // unknown aide, bad ids
    assert!(sb.run(&["inbox", "status", "MAIN/@NOPE"]).out.contains("no aide MAIN/@NOPE"));
    assert_eq!(sb.run(&["inbox", "status", "MAIN/KB"]).code, 2);
}

#[test]
fn backpressure_exits_4_with_the_reason() {
    let sb = Sb::new();
    for i in 0..3 {
        let r = sb.run_as(SHIP, &["inbox", "post", "MAIN/@KB", "--type", "wiki.note.v1", "--payload", &format!(r#"{{"body": "n{i}"}}"#)]);
        assert_eq!(r.code, 0, "{}", r.out);
    }
    // every sender is rate-limited, members too
    let r = sb.run_as(SHIP, &["inbox", "post", "MAIN/@KB", "--type", "wiki.note.v1", "--payload", r#"{"body": "n3"}"#]);
    assert_eq!(r.code, 4, "{}", r.out);
    assert!(r.out.contains("(inbox.rate)"), "{}", r.out);
    let t = sb.run_as(SHIP, &["inbox", "post", "MAIN/@KB", "--type", "wiki.query.v1", "--payload", r#"{"question": "q"}"#]);
    assert!(t.code == 4 && t.out.contains("(inbox.type)"), "{}", t.out);
    let s = sb.run(&["inbox", "post", "MAIN/@KB", "--type", "wiki.note.v1", "--payload", r#"{"body": "x", "domain": "Not A Slug"}"#]);
    assert!(s.code == 4 && s.out.contains("(inbox.schema)"), "{}", s.out);
    // the senders' window shows in status
    let st = sb.json(&[], &["inbox", "status", "MAIN/@KB"]);
    assert_eq!(st["senders"][0]["used"], 3, "{st}");
    assert_eq!(st["senders"][0]["limit"], 3);
}

#[test]
fn only_the_asker_reads_the_answer_and_the_token_opens_it() {
    let sb = Sb::new();
    let mod_session = sb.session(901, "s-mod", "jmod0001");
    let asker = pairs(&mod_session);
    let p = sb.json(&asker, &["inbox", "post", "MAIN/@REF", "--type", "wiki.query.v1", "--payload", r#"{"question": "what is a lease?", "length": "short"}"#]);
    let (id, token) = (p["id"].as_str().unwrap().to_string(), p["token"].as_str().unwrap().to_string());
    // the aide answers
    let c = sb.run_as(REF, &["inbox", "claim", "MAIN/@REF"]);
    let item: Value = serde_json::from_str(&c.out).unwrap();
    assert_eq!(item["sender"]["kind"], "session", "{item}");
    assert_eq!(item["sender"]["session"], "s-mod");
    let answer = json!({"answer": "A lease is a claim's lifetime [[queue]].", "citations": ["wiki/queue.md"], "gaps": "", "confidence": "high", "snapshot": "4f2a9c1"});
    let no_answer = sb.run_as(REF, &["inbox", "done", "MAIN/@REF", &id]);
    assert!(no_answer.out.contains("(inbox.result)"), "{}", no_answer.out);
    let d = sb.run_as(REF, &["inbox", "done", "MAIN/@REF", &id, "--result", &answer.to_string()]);
    assert_eq!(d.code, 0, "{}", d.out);
    assert!(d.out.contains("answer not delivered: the asker is not live"), "{}", d.out);
    // the asker reads it; a stranger session, a ship and the aide's list do not
    let r = sb.run_as(&asker, &["inbox", "result", "MAIN/@REF", &id]);
    assert_eq!(r.code, 0, "{}", r.out);
    assert!(r.out.contains("A lease is a claim's lifetime") && r.out.contains("sources: wiki/queue.md") && r.out.contains("4f2a9c1"), "{}", r.out);
    let stranger_session = sb.session(902, "s-other", "jother01");
    let stranger = pairs(&stranger_session);
    let s = sb.run_as(&stranger, &["inbox", "result", "MAIN/@REF", &id]);
    assert!(s.code != 0 && s.out.contains("(inbox.not_yours)") && !s.out.contains("lease"), "{}", s.out);
    assert!(sb.run_as(SHIP, &["inbox", "result", "MAIN/@REF", &id]).out.contains("(inbox.not_yours)"));
    let l = sb.json(&stranger, &["inbox", "list", "MAIN/@REF"]);
    assert!(l[0]["payload"].is_null() && l[0]["result"].is_null(), "others' payloads and results hidden: {l}");
    // the token opens it for anyone holding it; the same job after a /clear too
    let t = sb.run_as(&stranger, &["inbox", "result", "MAIN/@REF", &id, "--token", &token]);
    assert_eq!(t.code, 0, "{}", t.out);
    let cleared_session = sb.session(903, "s-mod-after-clear", "jmod0001");
    let cleared = pairs(&cleared_session);
    assert_eq!(sb.run_as(&cleared, &["inbox", "result", "MAIN/@REF", &id]).code, 0);
    // the user sees results in status; others do not
    assert!(!sb.json(&[], &["inbox", "status", "MAIN/@REF"])["recent"][0]["result"].is_null());
    assert!(sb.json(SHIP, &["inbox", "status", "MAIN/@REF"])["recent"][0].get("result").is_none());
    assert_eq!(sb.json(&[], &["inbox", "status", "MAIN/@REF"])["undelivered"], 1);
}

#[test]
fn ask_waits_for_the_answer_or_exits_5_with_the_token() {
    let sb = Sb::new();
    // nobody answers: exit 5, the id and the token to fetch it later
    let r = sb.run(&["ask", "MAIN/@REF", "what is a lease?", "--wait", "1s"]);
    assert_eq!(r.code, 5, "{}", r.out);
    assert!(r.out.contains("not answered yet") && r.out.contains("--token "), "{}", r.out);
    // an aide that answers while `ask` waits: the answer comes back to the caller
    let vars = sb.vars.clone();
    let asker = std::thread::spawn(move || raw(&vars, &["ask", "MAIN/@REF", "how long is a lease?", "--wait", "20s", "--length", "short"], &[]));
    let mut answered = false;
    for _ in 0..80 {
        let c = sb.run_as(REF, &["inbox", "claim", "MAIN/@REF"]);
        if c.code == 0 {
            let item: Value = serde_json::from_str(&c.out).unwrap();
            let id = item["id"].as_str().unwrap();
            let q = item["payload"]["question"].as_str().unwrap();
            let ans = if q.starts_with("how long") { "15 minutes [[lease]]." } else { "(the first question, answered late)" };
            sb.run_as(REF, &["inbox", "done", "MAIN/@REF", id, "--result", &json!({"answer": ans, "citations": ["wiki/lease.md"]}).to_string()]);
            if q.starts_with("how long") {
                answered = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(answered);
    let out = asker.join().unwrap();
    assert_eq!(out.code, 0, "{}", out.out);
    assert!(out.out.contains("15 minutes [[lease]].") && out.out.contains("sources: wiki/lease.md"), "{}", out.out);
    // --wait is capped at 10 minutes; a bad duration is a usage error
    assert_eq!(sb.run(&["ask", "MAIN/@REF", "q", "--wait", "soon"]).code, 2);
}

#[test]
fn a_crashed_aide_gives_the_item_back_and_a_poison_item_dead_letters() {
    let sb = Sb::new();
    // a lease of 1 s, two tries
    let f = sb.fleet_toml();
    let t = std::fs::read_to_string(&f).unwrap().replace("max_tries = 2\n", "max_tries = 2\nlease = \"1s\"\n");
    std::fs::write(&f, t).unwrap();
    sb.run(&["inbox", "post", "MAIN/@KB", "--type", "wiki.note.v1", "--payload", r#"{"body": "poison"}"#]);
    let first: Value = serde_json::from_str(&sb.run_as(KB, &["inbox", "claim", "MAIN/@KB"]).out).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1200));
    let again: Value = serde_json::from_str(&sb.run_as(KB, &["inbox", "claim", "MAIN/@KB"]).out).unwrap();
    assert_eq!((again["id"].clone(), again["tries"].as_u64()), (first["id"].clone(), Some(2)), "the lease ran out: the same item, try 2");
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert_eq!(sb.run_as(KB, &["inbox", "claim", "MAIN/@KB"]).code, 3, "dead after two tries, the queue is empty");
    let st = sb.json(&[], &["inbox", "status", "MAIN/@KB"]);
    assert!(st["dead"][0]["reason"].as_str().unwrap().contains("lease ran out"), "{st}");
    // its parent is told (aide.md 5.3): the needs-you notice of the dead letter,
    // delivered (or, here, with no admiral running, failed and in check)
    let id = st["dead"][0]["id"].as_str().unwrap().to_string();
    let n: Value = serde_json::from_slice(&std::fs::read(PathBuf::from(&sb.vars["NIMITZ_STATE"]).join("needs").join(format!("MAIN%2F@KB#dead-{id}.json"))).unwrap()).unwrap();
    assert_eq!((n["kind"].as_str(), n["id"].as_str()), (Some("dead"), Some("MAIN/@KB")));
    assert!(n["detail"].as_str().unwrap().starts_with(&format!("item {} (wiki.note.v1, from the user) failed 2 times; its last reason, quoted: <<its lease ran out", id.trim_start_matches('0'))), "{n}");
    let check = (0..50).find_map(|_| {
        let c = sb.run(&["check"]);
        std::thread::sleep(std::time::Duration::from_millis(100));
        c.out.contains("notify.failed").then_some(c.out)
    });
    let check = check.expect("the delivery to an admiral that is not running fails, and check says so");
    assert!(check.contains("MAIN/@KB has a dead letter and MAIN was not told") && check.contains(&format!("nimitz inbox retry MAIN/@KB {}", id.trim_start_matches('0'))), "{check}");
    // told once: a replay of its delivery sends nothing (it was given up on)
    let path = PathBuf::from(&sb.vars["NIMITZ_STATE"]).join("needs").join(format!("MAIN%2F@KB#dead-{id}.json"));
    let before = std::fs::read(&path).unwrap();
    let r = raw(&sb.vars, &["__notify", &format!("MAIN/@KB#dead-{id}")], SHIP);
    assert_eq!(r.code, 0);
    assert_eq!(std::fs::read(&path).unwrap(), before, "a delivered or failed notice is not delivered again");
    // a retry clears it
    assert_eq!(sb.run(&["inbox", "retry", "MAIN/@KB", &id]).code, 0);
    assert!(!sb.run(&["check"]).out.contains("notify.failed"));
    let _ = &sb.base;
}

#[test]
fn ask_json_keeps_the_token_short_ids_work_and_a_broken_aide_is_one_row() {
    let sb = Sb::new();
    // a timed-out ask in JSON still hands back the token
    let r = sb.run(&["ask", "MAIN/@REF", "q?", "--wait", "1s", "--json"]);
    assert_eq!(r.code, 5, "{}", r.out);
    // (its handle went to stderr: `asked ...`; stdout is the JSON alone)
    assert!(r.out.contains("asked MAIN/@REF item 1; token "), "{}", r.out);
    let v: Value = serde_json::Deserializer::from_str(&r.out[r.out.find('{').unwrap()..]).into_iter::<Value>().next().unwrap().unwrap();
    assert!(v["token"].as_str().is_some_and(|t| t.len() == 32), "{v}");
    let id = v["id"].as_str().unwrap().trim_start_matches('0').to_string();
    // the short id the fix lines use works; a path never does
    assert_eq!(sb.run(&["inbox", "result", "MAIN/@REF", &id, "--token", v["token"].as_str().unwrap()]).code, 5);
    let t = sb.run(&["inbox", "drop", "MAIN/@REF", "../../MAIN%2F@KB/pending/000000000001"]);
    assert_eq!(t.code, 2, "{}", t.out);
    // one aide's bad config: its row says so, the others still show
    let f = sb.fleet_toml();
    let s = std::fs::read_to_string(&f).unwrap().replace("types = [\"wiki.query.v1\"]", "types = [\"wiki.query.v1\"]\npolicy = \"anyone\"");
    std::fs::write(&f, s).unwrap();
    let all = sb.json(&[], &["inbox", "status"]);
    let rows = all.as_array().unwrap();
    assert_eq!(rows.len(), 2, "{all}");
    assert!(rows.iter().any(|r| r["aide"] == "MAIN/@REF" && r["error"].as_str().is_some_and(|e| e.contains("anyone"))), "{all}");
    assert!(rows.iter().any(|r| r["aide"] == "MAIN/@KB" && r["error"].is_null()), "{all}");
}

#[test]
fn the_fleet_and_parent_policies() {
    let sb = Sb::new();
    let f = sb.fleet_toml();
    let s = std::fs::read_to_string(&f).unwrap().replace("types = [\"wiki.note.v1\"]", "types = [\"wiki.note.v1\"]\npolicy = \"parent\"");
    std::fs::write(&f, s).unwrap();
    let post = |who: &[(&str, &str)]| sb.run_as(who, &["inbox", "post", "MAIN/@KB", "--type", "wiki.note.v1", "--payload", r#"{"body": "x"}"#]);
    assert!(post(SHIP).out.contains("(authority)"), "parent policy: a ship may not");
    assert_eq!(post(ADMIRAL).code, 0, "the parent may");
    assert_eq!(post(&[]).code, 0, "the user may");
    let s = std::fs::read_to_string(&f).unwrap().replace("policy = \"parent\"", "policy = \"fleet\"");
    std::fs::write(&f, s).unwrap();
    assert_eq!(post(SHIP).code, 0, "fleet policy: a member of MAIN may");
    let other = [("NIMITZ_ROLE", "ship"), ("NIMITZ_ID", "BETA/X/Y"), ("NIMITZ_FLEET", "BETA"), ("NIMITZ_SQUADRON", "X")];
    assert!(post(&other).out.contains("(authority)"), "another fleet's member may not");
}

/// `ask` hands out its handle on stderr right after the post, so a caller
/// killed while it waits (a Bash timeout) can still pick the answer up.
#[test]
fn ask_prints_its_item_and_token_to_stderr_at_once() {
    let sb = Sb::new();
    let o = Command::new(EXE).env_clear().env("PATH", "/usr/bin:/bin").env("NIMITZ_NOTIFY_RETRY", "0").envs(&sb.vars).args(["ask", "MAIN/@REF", "how long is a lease?", "--wait", "1s"]).stdin(Stdio::null()).output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    let line = err.lines().find(|l| l.starts_with("asked MAIN/@REF item 1; token ")).unwrap_or_else(|| panic!("{err}"));
    let tok = line.split("token ").nth(1).unwrap().split(';').next().unwrap().to_string();
    assert!(line.ends_with(&format!("; later: nimitz inbox result MAIN/@REF 1 --token {tok}")), "{line}");
    assert_eq!(o.status.code(), Some(5), "not answered within the wait");
    // the token opens it later
    let r = sb.run(&["inbox", "result", "MAIN/@REF", "1", "--token", &tok]);
    assert!(!r.out.contains("not_yours"), "{}", r.out);
}
