"""End-to-end check in a private Herdr server. Never touches the running Herdr.

    uv run --with pyte python tests/e2e_isolated.py

Starts a Herdr client+server in a pseudo-terminal with HERDR_* unset and
XDG_CONFIG_HOME / XDG_STATE_HOME in a fresh temp dir, links this plugin there,
binds prefix+f to it, and runs the admiralty TUI (claude-nimitz) against its
sandbox (fake claude, fake herdr for the jump). Presses real keys and checks
pane state through the private server's API. Stops only that server, after
checking its socket path. ADMIRALTY_WATCH_BIN picks the binary (default:
`admiralty` on PATH, e.g. target/debug/admiralty for an uninstalled build).
"""

import fcntl, glob, json, os, pty, re, select, shutil, signal, struct, subprocess, sys, tempfile, termios, time

import pyte

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TMP = tempfile.mkdtemp(prefix="haw-e2e-")
SOCK = os.path.join(TMP, "config", "herdr", "herdr.sock")
SANDBOX = os.path.join(TMP, "aw-sb")
ISO = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}
ISO.update(XDG_CONFIG_HOME=os.path.join(TMP, "config"), XDG_STATE_HOME=os.path.join(TMP, "state"),
           TERM="xterm-256color", ADMIRALTY_WATCH_ARGS=f"--sandbox {SANDBOX}")
FAILS = []


def h(*args):
    r = subprocess.run(["herdr", *args], env=ISO, capture_output=True, text=True, cwd=TMP)
    return r.returncode, r.stdout.strip()


def setup():
    os.makedirs(os.path.join(TMP, "config", "herdr"))
    with open(os.path.join(TMP, "config", "herdr", "config.toml"), "w") as f:
        f.write('onboarding = false\n[keys]\nprefix = "ctrl+g"\n\n[[keys.command]]\nkey = "prefix+f"\n'
                'type = "plugin_action"\ncommand = "admiralty-watch.toggle"\ndescription = "admiralty watch"\n')
    subprocess.run([ISO.get("ADMIRALTY_WATCH_BIN", "admiralty"), "sandbox", "create", SANDBOX],
                   check=True, capture_output=True)
    rc, out = h("plugin", "link", ROOT)
    assert rc == 0 and "plugin_linked" in out, out


class Screen(pyte.Screen):  # the client sends queries pyte does not know
    def report_device_status(self, *a, **k): pass
    def report_device_attributes(self, *a, **k): pass
    def select_graphic_rendition(self, *a, **k): pass


SCR = Screen(180, 50)
STREAM = pyte.ByteStream(SCR)
QUERIES = set()


def pump(fd, t):
    end = time.time() + t
    while time.time() < end:
        if select.select([fd], [], [], 0.05)[0]:
            try:
                d = os.read(fd, 65536)
            except OSError:
                return
            QUERIES.update(re.findall(rb"\x1b\[[?>=]?[0-9;]*[cnu]", d))
            try:
                STREAM.feed(d)
            except TypeError:
                pass


def key(fd, ch, ctrl=False, wait=0.3):
    # the client pushes kitty keyboard flags (>31u) and then expects kitty-encoded keys
    kitty = any(q.startswith(b"\x1b[>") and q.endswith(b"u") and q != b"\x1b[>7u" for q in QUERIES)
    code = 13 if ch == "\r" else ord(ch)
    if kitty:
        b = f"\x1b[{code};5u" if ctrl else f"\x1b[{code}u"
        os.write(fd, b.encode())
    else:
        os.write(fd, bytes([code & 0x1F]) if ctrl else ch.encode())
    pump(fd, wait)


def panes():
    return [p["pane_id"] for p in json.loads(h("pane", "list")[1])["result"]["panes"]]


def state():
    hits = glob.glob(os.path.join(TMP, "state", "**", "pane_id"), recursive=True)
    return open(hits[0]).read().strip() if hits else ""


def check(label, cond, detail):
    print(("[OK] " if cond else "[ERROR] ") + label + ("" if cond else f": {detail}"))
    if not cond:
        FAILS.append(label)


def main():
    setup()
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(TMP)
        os.execvpe("herdr", ["herdr"], ISO)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 180, 0, 0))
    try:
        pump(fd, 4)
        rc, st = h("status")
        assert SOCK in st and "running" in st, st
        print(f"[OK] private server at {SOCK}")
        toggle = lambda w=3: (key(fd, "g", ctrl=True), key(fd, "f", wait=w))

        toggle(); p = panes()
        check("prefix+f opens the watch, focused", len(p) == 2 and state() == p[1], (p, state()))
        info = json.loads(h("pane", "get", p[1])[1])["result"]["pane"]
        check("its pane label is the one toggle.sh checks", info.get("label") == "Admiralty Watch", info)
        check("the watch renders", any("admiralty" in l for l in SCR.display), "no header")
        toggle(); check("prefix+f again closes it", panes() == p[:1], panes())
        toggle(); key(fd, "q", wait=3); check("q closes it", len(panes()) == 1, panes())
        toggle(); check("stale id: prefix+f opens a fresh one", len(panes()) == 2, panes())
        h("tab", "create"); pump(fd, 1); h("tab", "focus", "w1:t2"); pump(fd, 1)
        toggle(); tabs = {t["tab_id"]: t["pane_count"] for t in json.loads(h("tab", "list")[1])["result"]["tabs"]}
        check("from another tab: moves to this tab", tabs == {"w1:t1": 1, "w1:t2": 2}, tabs)
        key(fd, "j", wait=0.5); key(fd, "\r", wait=3)
        calls = [json.loads(l)["args"] for l in open(os.path.join(SANDBOX, "herdr_calls.log")) if "focus" in l]
        check("enter on a lead jumps (fake herdr)", calls[-1:] == [["agent", "focus", "w2:p2"]], calls[-3:])
        tabs = {t["tab_id"]: t["pane_count"] for t in json.loads(h("tab", "list")[1])["result"]["tabs"]}
        check("and the watch closes after the jump", tabs == {"w1:t1": 1, "w1:t2": 1}, tabs)
    finally:
        rc, st = h("status")
        if SOCK in st:  # only ever our private server
            h("server", "stop")
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        time.sleep(0.5)
        shutil.rmtree(TMP, ignore_errors=True)
    print("[OK] all e2e checks passed" if not FAILS else f"[ERROR] {len(FAILS)} e2e check(s) failed")
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
