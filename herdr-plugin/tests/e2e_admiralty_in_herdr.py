"""Admiralty inside a private, real Herdr server. Never touches the running Herdr.

    ADMIRALTY_WATCH_BIN=target/release/admiralty uv run --no-project python herdr-plugin/tests/e2e_admiralty_in_herdr.py

A Herdr client+server in a pseudo-terminal with every HERDR_* unset and
XDG_CONFIG_HOME / XDG_STATE_HOME in a fresh temp dir (the socket path is checked
before anything is stopped). Its panes run /bin/sh. In it:

- a stand-in `claude` (a tiny C program, so Herdr detects it as a claude agent)
  runs in the sandbox admiral's folder, titled [MAIN]-ADMIRAL, and reports the
  admiral's session id; it logs every line typed into it;
- Admiralty runs in a second tab against the sandbox (fake claude), with the real
  Herdr as its multiplexer.

Checks: the admiral row shows Herdr's pane dot; `m` types to the admiral's pane;
`A` on a live ship opens it in workspace MAIN.ARGOS, tab CLEAN; enter jumps to
the admiral's pane. Needs clang (Xcode command line tools).
"""

import fcntl, glob, json, os, pty, select, shutil, signal, struct, subprocess, sys, tempfile, termios, time

TMP = tempfile.mkdtemp(prefix="adm-herdr-")
SOCK = os.path.join(TMP, "config", "herdr", "herdr.sock")
SB = os.path.join(TMP, "sb")
BIN = os.environ.get("ADMIRALTY_WATCH_BIN", "admiralty")
ISO = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_") and not k.startswith("NIMITZ_")}
ISO.update(XDG_CONFIG_HOME=os.path.join(TMP, "config"), XDG_STATE_HOME=os.path.join(TMP, "state"),
           TERM="xterm-256color", SHELL="/bin/sh")
FAILS = []
FAKE_C = r"""
#include <stdio.h>
#include <stdlib.h>
int main(void) {
    const char *title = getenv("FAKE_TITLE"), *log = getenv("FAKE_LOG");
    if (title) { printf("\033]0;%s\007", title); }
    printf("fake claude: %s\n", title ? title : ""); fflush(stdout);
    char line[4096];
    while (fgets(line, sizeof line, stdin)) {
        FILE *f = fopen(log ? log : "/dev/null", "a");
        if (f) { fputs(line, f); fclose(f); }
    }
    return 0;
}
"""


def h(*args):
    r = subprocess.run(["herdr", *args], env=ISO, capture_output=True, text=True, cwd=TMP)
    return r.returncode, (r.stdout.strip() or r.stderr.strip())


def hj(*args):
    rc, out = h(*args)
    assert rc == 0, (args, out)
    return json.loads(out)["result"]


def check(label, cond, detail):
    print(("[OK] " if cond else "[ERROR] ") + label + ("" if cond else f": {detail}"))
    if not cond:
        FAILS.append(label)


def wait(pred, t=8.0):
    end = time.time() + t
    while time.time() < end:
        v = pred()
        if v:
            return v
        time.sleep(0.2)
    return pred()


def admiral():
    for p in glob.glob(os.path.join(SB, "claude_home", "sessions", "*.json")):
        try:
            d = json.load(open(p))
        except ValueError:
            continue
        if d.get("name") == "[MAIN]-ADMIRAL":
            return d["sessionId"], d["cwd"]
    raise AssertionError("no [MAIN]-ADMIRAL in the sandbox")


def keys(pane, *ks):
    for k in ks:
        rc, out = h("pane", "send-keys", pane, k)
        assert rc == 0, out
        time.sleep(0.4)


def text(pane, s):
    rc, out = h("pane", "send-text", pane, s)
    assert rc == 0, out
    time.sleep(0.4)


def screen(pane):
    return h("pane", "read", pane)[1]


def screen_with(pane, needle, t=8.0):
    """The pane's text once it contains `needle`; raises if it never does."""
    end = time.time() + t
    while True:
        s = screen(pane)
        if needle in s:
            return s
        if time.time() >= end:
            raise AssertionError(f"{needle!r} never showed in pane {pane}: {s[-400:]}")
        time.sleep(0.2)


def main():
    if not shutil.which("clang"):
        print("[WARN] no clang: skipped")
        return 0
    os.makedirs(os.path.join(TMP, "config", "herdr"))
    open(os.path.join(TMP, "config", "herdr", "config.toml"), "w").write("onboarding = false\n")
    os.makedirs(os.path.join(TMP, "bin"))
    open(os.path.join(TMP, "fake.c"), "w").write(FAKE_C)
    subprocess.run(["clang", "-o", os.path.join(TMP, "bin", "claude"), os.path.join(TMP, "fake.c")], check=True)
    subprocess.run([BIN, "sandbox", "create", SB], check=True, capture_output=True)
    sid, cwd = admiral()
    typed = os.path.join(TMP, "typed.log")

    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(TMP)
        os.execvpe("herdr", ["herdr"], ISO)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 200, 0, 0))

    def pump(t):
        end = time.time() + t
        while time.time() < end:
            if select.select([fd], [], [], 0.05)[0]:
                try:
                    os.read(fd, 65536)
                except OSError:
                    return

    try:
        pump(4)
        _, st = h("status")
        assert SOCK in st and "running" in st, st
        print(f"[OK] private server at {SOCK}")
        # the admiral's pane: the stand-in claude in its folder, with its session id
        h("pane", "run", "w1:p1", f"cd '{cwd}' && FAKE_TITLE='[MAIN]-ADMIRAL' FAKE_LOG='{typed}' '{TMP}/bin/claude'")
        pump(2)
        h("pane", "report-agent-session", "w1:p1", "--source", "e2e", "--agent", "claude", "--agent-session-id", sid)
        h("pane", "report-agent", "w1:p1", "--source", "e2e", "--agent", "claude", "--state", "idle")
        pump(1)
        agents = hj("agent", "list")["agents"]
        check("herdr sees the admiral's pane as a claude agent", any(a["pane_id"] == "w1:p1" for a in agents), agents)

        # Admiralty in a second tab, with this Herdr as its multiplexer
        before = {p["pane_id"] for p in hj("pane", "list")["panes"]}
        h("tab", "create")
        pump(1)
        apane = [p["pane_id"] for p in hj("pane", "list")["panes"] if p["pane_id"] not in before][0]
        h("pane", "run", apane, f"NIMITZ_MUX=herdr ADMIRALTY_NO_CLIPBOARD=1 '{BIN}' --sandbox '{SB}'")
        pump(1)
        shown = screen_with(apane, "[MAIN]-ADMIRAL")
        row = next((l for l in shown.splitlines() if "[MAIN]-ADMIRAL" in l and "admiral" in l), "")
        check("the admiral row starts with Herdr's idle dot", row[2:4].startswith("○"), row)
        check("monitor-only is off inside Herdr", "MONITOR-ONLY" not in shown and "mux herdr" in shown, shown.splitlines()[0])

        # m: typed into the admiral's own pane
        keys(apane, "J", "m")
        text(apane, "hello admiral")
        keys(apane, "Enter")
        got = wait(lambda: os.path.exists(typed) and "hello admiral" in open(typed).read())
        check("m types the message into the admiral's pane", bool(got), open(typed).read() if os.path.exists(typed) else "(nothing typed)")

        # enter: focus jumps to the admiral's pane
        keys(apane, "Enter")
        focused = wait(lambda: [p["pane_id"] for p in hj("pane", "list")["panes"] if p.get("focused")] == ["w1:p1"])
        check("enter on the admiral focuses its pane", bool(focused), [p["pane_id"] for p in hj("pane", "list")["panes"] if p.get("focused")])

        # A on a live ship: a new tab in workspace MAIN.ARGOS, labelled CLEAN
        h("tab", "focus", "w1:t2")
        pump(1)
        keys(apane, "/")
        text(apane, "CLEAN")
        keys(apane, "Enter", "n", "A")
        dialog = wait(lambda: "nimitz open MAIN/ARGOS/CLEAN" in screen(apane))
        check("A shows the open plan", bool(dialog), screen(apane)[-800:])
        keys(apane, "y")
        ws = wait(lambda: [w for w in hj("workspace", "list")["workspaces"] if w.get("label") == "MAIN.ARGOS"])
        check("A opens workspace MAIN.ARGOS", bool(ws), hj("workspace", "list")["workspaces"])
        if ws:
            tabs = hj("tab", "list", "--workspace", ws[0]["workspace_id"])["tabs"]
            check("with a tab CLEAN", any(t.get("label") == "CLEAN" for t in tabs), tabs)
    finally:
        _, st = h("status")
        if SOCK in st:  # only ever our private server
            h("server", "stop")
        try:
            os.kill(pid, signal.SIGTERM)
        except (ProcessLookupError, UnboundLocalError):
            pass
        time.sleep(0.5)
        shutil.rmtree(TMP, ignore_errors=True)
    print("[OK] all checks passed" if not FAILS else f"[ERROR] {len(FAILS)} check(s) failed")
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
