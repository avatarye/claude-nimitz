"""P0.6-P0.8 probes for claude-nimitz.

P0.6  Do env labels from --settings reach processes the Bash tool starts?
P0.7  What does a PreToolUse hook see for SendMessage, and can it block one?
P0.8  Does `claude -c` (interactive) keep the same session id across a restart?

Throwaway sessions named nimitz-probe2-*, under $NIMITZ_PROBE_DIR (default
~/.cache/nimitz-probe); all stopped and removed at the end. P0.8 drives an
interactive session in a private tmux server (`tmux -L nimitz-probe`).
Writes results2.json in that folder.
"""
import json, os, shutil, subprocess, time
from pathlib import Path

BASE = Path(os.environ.get("NIMITZ_PROBE_DIR", os.path.expanduser("~/.cache/nimitz-probe")))
BASE.mkdir(parents=True, exist_ok=True)
EV = BASE / "events2.jsonl"
HOOK = BASE / "hook2.py"
NATIVE = os.environ.get("NIMITZ_NATIVE_CLAUDE_BIN") or shutil.which("claude") or "claude"
EVENTS = ["SessionStart", "Stop", "SessionEnd", "PreToolUse", "PostToolUse"]

HOOK.write_text('''import json, os, sys
ev = sys.argv[1]
try:
    data = json.load(sys.stdin)
except Exception:
    data = {}
ti = data.get("tool_input")
rec = {"event": ev, "label": os.environ.get("NIMITZ_PROBE"), "session_id": data.get("session_id"),
       "source": data.get("source"), "tool_name": data.get("tool_name"),
       "tool_input": json.dumps(ti)[:600] if ti is not None else None, "keys": sorted(data)}
with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "events2.jsonl"), "a") as f:
    f.write(json.dumps(rec) + "\\n")
if ev == "PreToolUse" and os.environ.get("NIMITZ_DENY_SEND") == "1" and "send" in str(data.get("tool_name", "")).lower():
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny",
                                             "permissionDecisionReason": "nimitz probe: comms rule forbids this signal"}}))
''')


def settings(label, extra_env=None):
    env = {"NIMITZ_PROBE": label, "NIMITZ_ROLE": "ship", "NIMITZ_ID": f"PROBE/{label}"}
    env.update(extra_env or {})
    hooks = {e: [{"matcher": "*", "hooks": [{"type": "command", "command": f"python3 {HOOK} {e}"}]}] if e in ("PreToolUse", "PostToolUse")
             else [{"hooks": [{"type": "command", "command": f"python3 {HOOK} {e}"}]}] for e in EVENTS}
    return json.dumps({"env": env, "worktree": {"bgIsolation": "none"}, "hooks": hooks})


def run(args, cwd=None, timeout=90):
    p = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    return p.returncode, (p.stdout + p.stderr).strip()


def rows():
    rc, out = run([NATIVE, "agents", "--json", "--all"])
    try:
        return [r for r in json.loads(out) if "nimitz-probe2" in str(r.get("name", "")) or str(r.get("cwd", "")).startswith(str(BASE))]
    except ValueError:
        return []


def events(label=None):
    if not EV.exists():
        return []
    out = [json.loads(l) for l in EV.read_text().splitlines() if l.strip()]
    return [e for e in out if label is None or e["label"] == label]


def wait_for(pred, secs=120):
    t = time.time()
    while time.time() - t < secs:
        v = pred()
        if v:
            return v
        time.sleep(3)
    return None


def stopped(label):
    return [e for e in events(label) if e["event"] == "Stop"]


def bg(name, label, cwd, prompt, mode="bypassPermissions", extra_env=None):
    cwd.mkdir(parents=True, exist_ok=True)
    rc, out = run(["claude", "--bg", "--name", name, "--permission-mode", mode,
                   "--settings", settings(label, extra_env), prompt], cwd=str(cwd))
    return {"rc": rc, "out": out[-300:]}


def main():
    EV.unlink(missing_ok=True)
    res = {"native": NATIVE, "version": run([NATIVE, "--version"])[1], "path_version": run(["claude", "--version"])[1]}

    # P0.6 labels reach the Bash tool's processes
    d6 = BASE / "p6"
    envfile = d6 / "env.txt"
    envfile.unlink(missing_ok=True)
    l6 = bg("nimitz-probe2-env", "p6", d6,
            f"Use the Bash tool to run exactly this command: env | grep NIMITZ_ | sort > {envfile} ; then reply with the single word DONE.")
    wait_for(lambda: stopped("p6"))
    time.sleep(2)
    res["p06_labels_in_bash"] = {
        "launch": l6,
        "bash_tool_used": [e["tool_input"] for e in events("p6") if e["event"] == "PreToolUse" and e["tool_name"] == "Bash"],
        "env_file": envfile.read_text().splitlines() if envfile.exists() else None,
    }
    res["p06_labels_in_bash"]["labels_reached_bash"] = bool(res["p06_labels_in_bash"]["env_file"]) and any(
        l.startswith("NIMITZ_ROLE=ship") for l in res["p06_labels_in_bash"]["env_file"])

    # P0.7 SendMessage in PreToolUse; then deny
    target = bg("nimitz-probe2-target", "p7t", BASE / "p7t", "Reply with the single word READY.")
    wait_for(lambda: stopped("p7t"))
    prompt7 = ("Use your SendMessage tool to send the text 'ping from probe' to the session named "
               "nimitz-probe2-target. Then reply with the single word DONE, or with the word FAILED and the reason.")
    l7a = bg("nimitz-probe2-sender", "p7a", BASE / "p7a", prompt7)
    wait_for(lambda: stopped("p7a"))
    l7b = bg("nimitz-probe2-sender-deny", "p7b", BASE / "p7b", prompt7, extra_env={"NIMITZ_DENY_SEND": "1"})
    wait_for(lambda: stopped("p7b"))
    time.sleep(2)

    def tool_calls(label):
        return [{"event": e["event"], "tool_name": e["tool_name"], "tool_input": e["tool_input"], "keys": e["keys"]}
                for e in events(label) if e["event"] in ("PreToolUse", "PostToolUse")]

    res["p07_sendmessage"] = {"target": target, "allow_run": l7a, "deny_run": l7b,
                              "allow_tool_calls": tool_calls("p7a"), "deny_tool_calls": tool_calls("p7b")}
    send_names = {c["tool_name"] for c in tool_calls("p7a") if "send" in str(c["tool_name"]).lower()}
    res["p07_sendmessage"]["send_tool_names"] = sorted(send_names)
    res["p07_sendmessage"]["deny_blocked"] = (
        any(c["event"] == "PreToolUse" and c["tool_name"] in send_names for c in tool_calls("p7b"))
        and not any(c["event"] == "PostToolUse" and c["tool_name"] in send_names for c in tool_calls("p7b")))

    # P0.8 interactive `claude -c` across a restart (private tmux server)
    d8 = BASE / "p8"
    d8.mkdir(parents=True, exist_ok=True)
    tmux = ["tmux", "-L", "nimitz-probe"]
    s8 = settings("p8").replace("'", "'\\''")
    p8 = {"available": bool(shutil.which("tmux"))}
    if p8["available"]:
        run(tmux + ["kill-server"])
        for step, flag in (("first", ""), ("second", "-c")):
            cmd = f"cd {d8} && claude {flag} --name nimitz-probe2-interactive --settings '{s8}'"
            run(tmux + ["new-session", "-d", "-s", f"p8{step}", "-x", "160", "-y", "40", cmd])
            ok = wait_for(lambda: [e for e in events("p8") if e["event"] == "SessionStart" and e.get("_step") is None
                                   and len([x for x in events("p8") if x["event"] == "SessionStart"]) >= (1 if step == "first" else 2)], secs=60)
            time.sleep(4)
            run(tmux + ["send-keys", "-t", f"p8{step}", "Reply with the single word OK.", "Enter"])
            wait_for(lambda: len(stopped("p8")) >= (1 if step == "first" else 2), secs=90)
            p8[f"{step}_screen_tail"] = run(tmux + ["capture-pane", "-p", "-t", f"p8{step}"])[1][-500:]
            run(tmux + ["send-keys", "-t", f"p8{step}", "/exit", "Enter"])
            wait_for(lambda: len([e for e in events("p8") if e["event"] == "SessionEnd"]) >= (1 if step == "first" else 2), secs=30)
            run(tmux + ["kill-session", "-t", f"p8{step}"])
            p8[f"{step}_started"] = bool(ok)
        run(tmux + ["kill-server"])
        starts = [(e["session_id"], e["source"]) for e in events("p8") if e["event"] == "SessionStart"]
        p8["session_starts"] = starts
        p8["same_id_after_c"] = len(starts) >= 2 and starts[0][0] == starts[-1][0]
    res["p08_interactive_continue"] = p8

    # cleanup
    for r in rows():
        if r.get("id"):
            run([NATIVE, "stop", r["id"]])
    time.sleep(3)
    for r in rows():
        if r.get("id"):
            run([NATIVE, "rm", r["id"]])
    time.sleep(2)
    res["cleanup"] = {"rows_left": [{k: r.get(k) for k in ("id", "name", "pid", "kind")} for r in rows()]}
    (BASE / "results2.json").write_text(json.dumps(res, indent=1))
    print(json.dumps(res, indent=1)[:9000])


if __name__ == "__main__":
    main()
