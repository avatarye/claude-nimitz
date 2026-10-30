"""Wrapper-build follow-up: launch through the PATH wrapper, then stop and resume through the
native binary with no flags. Writes $NIMITZ_PROBE_DIR/native-resume.json."""
import json, os, sys, time
sys.path.insert(0, "probes")
import p0_probe as p
D = p.BASE / "nr"
out = {}
l = p.launch("nimitz-probe-nr", "label-nr", D, [], "Reply with the single word OK.")
p.wait_for(lambda: [e for e in p.events("label-nr") if e["event"] == "Stop"])
r = [x for x in p.rows() if x.get("name") == "nimitz-probe-nr"]
sid, jid = r[0]["sessionId"], r[0]["id"]
out["first"] = sid
out["stop"] = p.run([p.NATIVE, "stop", jid])
p.wait_for(lambda: not [x for x in p.rows() if x.get("name") == "nimitz-probe-nr" and x.get("pid")], secs=30)
rc, txt = p.run([p.NATIVE, "--bg", "--resume", sid, "Reply with the single word NATIVE."], cwd=str(D))
out["native_resume"] = {"rc": rc, "out": txt[-400:]}
p.wait_for(lambda: [e for e in p.events("label-nr") if e["event"] == "Stop" and e["session_id"]], secs=90)
time.sleep(5)
ev = p.events("label-nr")
out["hook_ids"] = sorted({e["session_id"] for e in ev if e["session_id"]})
out["starts"] = [(e["session_id"][:8], e["source"]) for e in ev if e["event"] == "SessionStart"]
out["rows"] = [{k: x.get(k) for k in ("id", "sessionId", "name", "pid", "status", "state")} for x in p.rows() if "nimitz-probe-nr" in (x.get("name") or "") or str(x.get("cwd","")).endswith("/nr")]
for x in p.rows():
    if x.get("id"): p.run([p.NATIVE, "stop", x["id"]])
time.sleep(3)
for x in p.rows():
    if x.get("id"): p.run([p.NATIVE, "rm", x["id"]])
time.sleep(2)
out["left"] = [x.get("id") for x in p.rows()]
json.dump(out, open(p.BASE / "native-resume.json", "w"), indent=1)
print(json.dumps(out, indent=1))
