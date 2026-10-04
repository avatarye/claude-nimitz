"""P0.9 probe for claude-nimitz: does a resume with flags keep the history?

Found by the adversarial tester on ACME 2.1.288 (BUG-53):
  A  `--bg --resume <id>` WITH flags while the stopped session's job row exists
     -> a new session id and NO history.
  B  `claude rm <row>` first, then the same resume with flags
     -> the same id, the new name and labels, and the history kept.
This script checks both on the build it runs on.

Throwaway sessions named nimitz-probe9-*, in $NIMITZ_PROBE_DIR/p9 (default
~/.cache/nimitz-probe/p9); all stopped and removed at the end. Touches no other
session. Writes results-p9.json in $NIMITZ_PROBE_DIR.
"""
import glob, json, os, platform, shutil, socket, subprocess, sys, time
from typing import Any
from pathlib import Path

BASE = Path(os.environ.get("NIMITZ_PROBE_DIR", os.path.expanduser("~/.cache/nimitz-probe")))
WORK = BASE / "p9"
WORK.mkdir(parents=True, exist_ok=True)
EV = BASE / "events9.jsonl"
HOOK = BASE / "hook9.py"
CLAUDE_HOME = Path(os.environ.get("CLAUDE_CONFIG_DIR", os.path.expanduser("~/.claude")))
_tb = sorted(glob.glob(os.path.expanduser("~/.localtools/tools/claude-code/*/claude")),
             key=lambda p: [int(x) for x in p.split(os.sep)[-2].split(".") if x.isdigit()])
NATIVE = _tb[-1] if _tb else (shutil.which("claude") or "claude")
LAUNCH = shutil.which("claude") or "claude"
PY = sys.executable
WORD = "PINEAPPLE"
ASK = "What is the code word I told you earlier? Reply with the word only, or NONE if you were told none."

HOOK.write_text('''import json, os, sys
ev = sys.argv[1]
try:
    data = json.load(sys.stdin)
except Exception:
    data = {}
rec = {"event": ev, "label": os.environ.get("NIMITZ_PROBE"), "session_id": data.get("session_id"),
       "source": data.get("source")}
with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "events9.jsonl"), "a") as f:
    f.write(json.dumps(rec) + "\\n")
''')


def settings(label):
    hooks = {e: [{"hooks": [{"type": "command", "command": f'"{PY}" "{HOOK}" {e}'}]}]
             for e in ("SessionStart", "Stop", "SessionEnd")}
    return json.dumps({"env": {"NIMITZ_PROBE": label, "NIMITZ_ID": f"PROBE/{label}", "NIMITZ_ROLE": "ship"},
                       "worktree": {"bgIsolation": "none"}, "hooks": hooks})


def run(args, timeout=120):
    p = subprocess.run(args, cwd=str(WORK), capture_output=True, text=True, timeout=timeout)
    return p.returncode, (p.stdout + p.stderr).strip()


def rows():
    rc, out = run([NATIVE, "agents", "--json", "--all"])
    try:
        allr = json.loads(out)
    except ValueError:
        return []
    return [r for r in allr if str(r.get("name", "")).startswith("nimitz-probe9")
            or os.path.realpath(str(r.get("cwd", ""))) == os.path.realpath(str(WORK))]


def events(label):
    if not EV.exists():
        return []
    out = [json.loads(l) for l in EV.read_text().splitlines() if l.strip()]
    return [e for e in out if e["label"] == label]


def wait_for(pred, secs=180):
    t = time.time()
    while time.time() - t < secs:
        v = pred()
        if v:
            return v
        time.sleep(3)
    return None


def row_of(sid):
    return next((r for r in rows() if r.get("sessionId") == sid), None)


def answer(sid):
    """Last assistant text in the session's transcript."""
    for f in glob.glob(str(CLAUDE_HOME / "projects" / "*" / f"{sid}.jsonl")):
        text = ""
        for line in Path(f).read_text(errors="replace").splitlines():
            try:
                m = json.loads(line)
            except ValueError:
                continue
            if m.get("type") == "assistant":
                for c in (m.get("message", {}).get("content") or []):
                    if isinstance(c, dict) and c.get("type") == "text":
                        text = c.get("text", "")
        return text.strip()[:200]
    return None


def launch(label, extra):
    rc, out = run([LAUNCH, "--bg", "--name", f"nimitz-probe9-{label}", "--permission-mode", "default",
                   "--settings", settings(label)] + extra)
    start = wait_for(lambda: [e for e in events(label) if e["event"] == "SessionStart"])
    sid = start[0]["session_id"] if start else None
    done = wait_for(lambda: [e for e in events(label) if e["event"] == "Stop"])
    return {"rc": rc, "out": out[-300:], "session_id": sid, "stopped_turn": bool(done),
            "labels_reached": bool(start)}


def stop(sid):
    r = row_of(sid)
    if r:
        run([NATIVE, "stop", r["id"]])
        wait_for(lambda: (row_of(sid) or {}).get("state") not in ("running", "idle", "busy"), secs=60)


def main() -> dict[str, Any]:
    EV.unlink(missing_ok=True)
    res: dict[str, Any] = {"host": socket.gethostname(), "os": platform.platform(), "native": NATIVE, "launch": LAUNCH,
           "native_version": run([NATIVE, "--version"])[1], "launch_version": run([LAUNCH, "--version"])[1]}
    try:
        first = launch("orig", [f"Remember this code word: {WORD}. Reply only OK."])
        res["orig"] = first
        sid0 = first["session_id"]
        if not sid0:
            res["error"] = "first session did not start (trust? --bg unsupported?)"
            return res
        stop(sid0)

        # A: resume with flags while the job row exists
        a = launch("A", ["--resume", sid0, ASK])
        a["same_id"] = a["session_id"] == sid0
        a["answer"] = answer(a["session_id"]) if a["session_id"] else None
        a["kept_history"] = bool(a["answer"]) and WORD in a["answer"].upper()
        res["A_flags_row_exists"] = a
        if a["session_id"] and a["session_id"] != sid0:
            stop(a["session_id"])
            r = row_of(a["session_id"])
            if r:
                run([NATIVE, "rm", r["id"]])

        # B: rm the row first, then resume with flags
        r0 = row_of(sid0)
        res["rm_rc"] = run([NATIVE, "rm", r0["id"]])[0] if r0 else "no row"
        b = launch("B", ["--resume", sid0, ASK])
        b["same_id"] = b["session_id"] == sid0
        b["answer"] = answer(b["session_id"]) if b["session_id"] else None
        b["kept_history"] = bool(b["answer"]) and WORD in b["answer"].upper()
        res["B_rm_then_flags"] = b
    finally:
        for r in rows():
            run([NATIVE, "stop", r["id"]])
        time.sleep(5)
        for r in rows():
            run([NATIVE, "rm", r["id"]])
        res["left_over_rows"] = [r.get("id") for r in rows()]
    return res


if __name__ == "__main__":
    out = main()
    (BASE / "results-p9.json").write_text(json.dumps(out, indent=2))
    print(json.dumps(out, indent=2))
    a, b = out.get("A_flags_row_exists", {}), out.get("B_rm_then_flags", {})
    print(f"[{'OK' if not a.get('kept_history') else 'WARN'}] A (flags, row exists): same_id={a.get('same_id')} kept_history={a.get('kept_history')}")
    print(f"[{'OK' if b.get('same_id') and b.get('kept_history') else 'ERROR'}] B (rm, then flags): same_id={b.get('same_id')} kept_history={b.get('kept_history')}")
    print(f"[{'OK' if not out.get('left_over_rows') else 'ERROR'}] left over rows: {out.get('left_over_rows')}")
