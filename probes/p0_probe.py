"""P0 probes for claude-nimitz: identity and hooks in `claude --bg` sessions.

Starts throwaway background sessions under $NIMITZ_PROBE_DIR (default ~/.cache/nimitz-probe) and one in /tmp, named nimitz-probe-*,
with one-line prompts; stops and removes all of them at the end. Writes
results.json in that folder.
"""
import glob, json, os, shutil, subprocess, time, uuid
from pathlib import Path

BASE = Path(os.environ.get("NIMITZ_PROBE_DIR", os.path.expanduser("~/.cache/nimitz-probe")))
BASE.mkdir(parents=True, exist_ok=True)
EV = BASE / "events.jsonl"
HOOK = BASE / "hook.py"
# ACME (Acme) builds put a wrapper on PATH; job-mutating commands need the native binary
_tb = sorted(glob.glob(os.path.expanduser("~/.localtools/tools/claude-code/*/claude")),
             key=lambda p: [int(x) for x in p.split("/")[-2].split(".") if x.isdigit()])
NATIVE = _tb[-1] if _tb else (shutil.which("claude") or "claude")
EVENTS = ["SessionStart", "UserPromptSubmit", "Stop", "SessionEnd", "Notification"]
results = {"base": str(BASE), "native": NATIVE, "path_claude": shutil.which("claude"), "probes": {}}

HOOK.write_text('''import json, os, sys
ev = sys.argv[1]
try:
    data = json.load(sys.stdin)
except Exception:
    data = {}
rec = {"event": ev, "label": os.environ.get("NIMITZ_PROBE"), "keys": sorted(data), "reason": data.get("reason"),
       "session_id": data.get("session_id"),
       "source": data.get("source"), "session_title": data.get("session_title"),
       "notification_type": data.get("notification_type"), "cwd": data.get("cwd"), "ppid": os.getppid()}
with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "events.jsonl"), "a") as f:
    f.write(json.dumps(rec) + "\\n")
title = os.environ.get("NIMITZ_TITLE")
if ev == "SessionStart" and title:
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "SessionStart", "sessionTitle": title}}))
''')


def settings(label, title=None):
    env = {"NIMITZ_PROBE": label}
    if title:
        env["NIMITZ_TITLE"] = title
    hooks = {e: [{"hooks": [{"type": "command", "command": f"python3 {HOOK} {e}"}]}] for e in EVENTS}
    return json.dumps({"env": env, "worktree": {"bgIsolation": "none"}, "hooks": hooks})


def run(args, cwd=None, timeout=60):
    p = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    return p.returncode, (p.stdout + p.stderr).strip()


def rows():
    rc, out = run([NATIVE, "agents", "--json", "--all"])
    try:
        return [r for r in json.loads(out) if str(r.get("name", "")).startswith("nimitz-probe")
                or str(r.get("cwd", "")).startswith(str(BASE))]
    except ValueError:
        return []


def events(label=None):
    if not EV.exists():
        return []
    out = [json.loads(l) for l in EV.read_text().splitlines() if l.strip()]
    return [e for e in out if label is None or e["label"] == label]


def wait_for(pred, secs=90):
    t = time.time()
    while time.time() - t < secs:
        v = pred()
        if v:
            return v
        time.sleep(3)
    return None


def launch(name, label, cwd, extra, prompt, title=None):
    cwd.mkdir(parents=True, exist_ok=True)
    args = ["claude", "--bg", "--name", name, "--permission-mode", "default",
            "--settings", settings(label, title)] + extra + [prompt]
    rc, out = run(args, cwd=str(cwd))
    return {"cmd": " ".join(a if " " not in a else "<json>" for a in args[:-1]) + " <prompt>", "rc": rc, "out": out[-400:]}


def main():
    BASE.mkdir(exist_ok=True)
    EV.unlink(missing_ok=True)
    results["claude_version"] = run([NATIVE, "--version"])[1]
    results["path_claude_version"] = run(["claude", "--version"])[1]
    results["provider_env"] = {k: os.environ.get(k) for k in ("CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX", "ANTHROPIC_API_KEY") if os.environ.get(k)}
    if results["provider_env"].get("ANTHROPIC_API_KEY"):
        results["provider_env"]["ANTHROPIC_API_KEY"] = "set"
    try:
        sj = json.load(open(os.path.expanduser("~/.claude/settings.json")))
        results["settings_env_provider"] = {k: v for k, v in (sj.get("env") or {}).items() if k.startswith("CLAUDE_CODE_USE_")}
    except Exception:
        results["settings_env_provider"] = None

    # 0. trust: --bg in a folder outside any trusted parent
    untrusted = Path("/tmp/nimitz-probe-untrusted")
    l0 = launch("nimitz-probe-u", "label-u", untrusted, [], "Reply with the single word OK.")
    results["probes"]["0_bg_in_untrusted_folder"] = {"launch": l0, "refused": l0["rc"] != 0}

    # 1. --session-id with --bg, labels, hooks
    u1 = str(uuid.uuid4())
    l1 = launch("nimitz-probe-a", "label-a", BASE / "a", ["--session-id", u1], "Reply with the single word OK.")
    stop1 = wait_for(lambda: [e for e in events("label-a") if e["event"] == "Stop"])
    r1 = [r for r in rows() if r.get("name") == "nimitz-probe-a"]
    results["probes"]["1_session_id_with_bg"] = {
        "launch": l1, "requested_session_id": u1,
        "row_session_ids": [r.get("sessionId") for r in r1],
        "honoured": any(r.get("sessionId") == u1 for r in r1),
        "rows": r1,
    }
    results["probes"]["3_labels_and_hooks_in_bg"] = {
        "events_with_label": events("label-a"),
        "label_reached_hooks": bool(events("label-a")),
        "stop_fired": bool(stop1),
        "events_seen": sorted({e["event"] for e in events("label-a")}),
    }

    # 2. resume: what id comes back? can we choose it?
    l2 = launch("nimitz-probe-a", "label-a2", BASE / "a", ["--resume", u1], "Reply with the single word AGAIN.")
    wait_for(lambda: [e for e in events("label-a2") if e["event"] == "Stop"])
    u2 = str(uuid.uuid4())
    l2b = launch("nimitz-probe-a", "label-a3", BASE / "a", ["--resume", u1, "--session-id", u2], "Reply with the single word THIRD.")
    wait_for(lambda: [e for e in events("label-a3") if e["event"] == "Stop"], secs=60)
    ra = [r for r in rows() if r.get("name", "").startswith("nimitz-probe-a")]
    results["probes"]["2_resume_ids"] = {
        "plain_resume": l2, "resume_with_session_id": l2b, "requested_resume_id": u2,
        "hook_session_ids_resume": sorted({e["session_id"] for e in events("label-a2") if e["session_id"]}),
        "hook_session_ids_resume_with_id": sorted({e["session_id"] for e in events("label-a3") if e["session_id"]}),
        "resume_sources": sorted({str(e["source"]) for e in events("label-a2") if e["event"] == "SessionStart"}),
        "rows_now": [{k: r.get(k) for k in ("id", "sessionId", "name", "pid", "status", "state")} for r in ra],
    }

    # 2c. resume a STOPPED session (the normal fleet case): same id or a new one?
    l6 = launch("nimitz-probe-c", "label-c", BASE / "c", [], "Reply with the single word OK.")
    wait_for(lambda: [e for e in events("label-c") if e["event"] == "Stop"])
    rc_rows = [r for r in rows() if r.get("name") == "nimitz-probe-c"]
    c_sid = rc_rows[0]["sessionId"] if rc_rows else None
    c_id = rc_rows[0].get("id") if rc_rows else None
    stop_out = run([NATIVE, "stop", c_id]) if c_id else (None, "no row")
    wait_for(lambda: not [r for r in rows() if r.get("name") == "nimitz-probe-c" and r.get("pid")], secs=30)
    l6b = launch("nimitz-probe-c", "label-c2", BASE / "c", ["--resume", c_sid or "x"], "Reply with the single word RESUMED.")
    wait_for(lambda: [e for e in events("label-c2") if e["event"] == "Stop"])
    rc2 = [r for r in rows() if r.get("name") == "nimitz-probe-c"]
    results["probes"]["2c_resume_after_stop"] = {
        "first_session_id": c_sid, "stop": stop_out, "resume": l6b,
        "hook_session_ids_after_resume": sorted({e["session_id"] for e in events("label-c2") if e["session_id"]}),
        "session_start_sources": sorted({str(e["source"]) for e in events("label-c2") if e["event"] == "SessionStart"}),
        "same_id": any(e["session_id"] == c_sid for e in events("label-c2")),
        "rows_now": [{k: r.get(k) for k in ("id", "sessionId", "name", "pid", "status", "state")} for r in rc2],
    }

    # 4. hook sessionTitle vs the messaging name
    l4 = launch("nimitz-probe-b", "label-b", BASE / "b", [], "Reply with the single word OK.", title="nimitz-probe-b-retitled")
    wait_for(lambda: [e for e in events("label-b") if e["event"] == "Stop"])
    rb = [r for r in rows() if str(r.get("cwd", "")).endswith("/b")]
    sess = []
    for f in glob.glob(os.path.expanduser("~/.claude/sessions/*.json")):
        try:
            raw = open(f).read()
            d, _ = json.JSONDecoder().raw_decode(raw)
            if str(d.get("cwd", "")).endswith("nimitz-probe/b"):
                sess.append({"name": d.get("name"), "nameSource": d.get("nameSource")})
        except Exception:
            pass
    results["probes"]["4_hook_title_vs_name"] = {
        "launch": l4, "row_names": [r.get("name") for r in rb], "session_file": sess,
        "title_in_hook_input": sorted({str(e["session_title"]) for e in events("label-b")}),
    }

    # 5. two live background sessions with one name
    l5 = launch("nimitz-probe-a", "label-dup", BASE / "a2", [], "Reply with the single word DUP.")
    wait_for(lambda: [e for e in events("label-dup") if e["event"] == "Stop"])
    live = [r for r in rows() if r.get("pid")]
    results["probes"]["5_duplicate_bg_name"] = {
        "launch": l5,
        "live_names": [r.get("name") for r in live],
        "two_live_same_name": sum(1 for r in live if r.get("name") == "nimitz-probe-a") >= 2,
    }

    results["agents_json_fields"] = sorted({k for r in rows() for k in r})
    sf = set()
    for f in glob.glob(os.path.expanduser("~/.claude/sessions/*.json")):
        try:
            d, _ = json.JSONDecoder().raw_decode(open(f).read())
            if str(d.get("cwd", "")).startswith(str(BASE)) or str(d.get("cwd", "")).startswith("/private" + str(BASE)):
                sf |= set(d)
        except Exception:
            pass
    results["session_file_fields"] = sorted(sf)

    # cleanup: stop, then rm every probe row; verify
    for r in rows():
        if r.get("id"):
            run([NATIVE, "stop", r["id"]])
    time.sleep(3)
    for r in rows():
        if r.get("id"):
            run([NATIVE, "rm", r["id"]])
    time.sleep(2)
    time.sleep(3)
    results["session_end_events"] = [e for e in events() if e["event"] == "SessionEnd"]
    left = rows()
    results["cleanup"] = {"rows_left": [{k: r.get(k) for k in ("id", "name", "pid")} for r in left]}
    (BASE / "results.json").write_text(json.dumps(results, indent=1))
    shutil.rmtree(untrusted, ignore_errors=True)
    print(json.dumps(results, indent=1)[:9000])


if __name__ == "__main__":
    main()
