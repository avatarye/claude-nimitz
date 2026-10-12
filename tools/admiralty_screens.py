"""Render docs/screens/admiralty-*.png from Admiralty's own drawing.

Each screen runs the real TUI (`admiralty --demo`, `--demo-monitor` or
`--sandbox`) in a private, detached tmux server of a fixed size, sends the
keys that open the view, captures the pane's cells with their 24-bit colours
(`tmux capture-pane -e`), and draws that cell grid to a PNG with a Nerd Font
in a plain window frame. Nothing reads the real nimitz state: NIMITZ_HOME,
NIMITZ_STATE and ADMIRALTY_STATE point at a temp dir, sandboxes are made
fresh under /tmp/adm-sb (a fixed path, so the screens that show it never
change; the run stops when it exists, and removes only the one it made).

    cargo build -p admiralty
    uv run --with pillow tools/admiralty_screens.py [--bin target/debug/admiralty]
        [--font DIR_OR_REGULAR_TTF] [--out docs/screens] [--only NAME ...]

Needs tmux. The font: --font, else $ADMIRALTY_SCREENS_FONT, else the first
JetBrainsMono Nerd Font found under ~/.local/share/fonts, ~/.fonts,
/usr/share/fonts, /usr/local/share/fonts, ~/Library/Fonts.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unicodedata
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

REPO = Path(__file__).resolve().parent.parent
BG = (0x27, 0x28, 0x22)
FG = (0xE6, 0xE6, 0xDC)
FRAME_BG = (0x1E, 0x1F, 0x1A)
SB = "/tmp/adm-sb"  # the sandbox path shown in the sandbox screens

# name: (cols, rows, source, keys). A key is literal text, or <Name> for a tmux
# key name (<Enter>, <Escape>, <Tab>, <C-e>), or <sleep:S>.
SCREENS: dict[str, tuple[int, int, str, list[str]]] = {
    "admiralty-wide": (200, 50, "demo", ["w"]),
    "admiralty-80x24": (80, 24, "demo-monitor", ["2", "/ANVIL", "<Enter>", "n", "<Escape>", "w"]),
    "admiralty-narrow": (100, 30, "demo", []),
    "admiralty-help": (200, 50, "demo", ["?"]),
    "admiralty-issues-view": (200, 50, "demo", ["i", "j", "j"]),
    "admiralty-issue-detail": (200, 50, "demo", ["2", "/HAMMER", "<Enter>", "n", "<Escape>", "w"]),
    "admiralty-confirm": (200, 50, "demo", ["/DOCS", "<Enter>", "n", "<Escape>", "a"]),
    "admiralty-actions-menu": (200, 50, "demo", ["/BOLT", "<Enter>", "n", "<Escape>", "x"]),
    "admiralty-fleet-menu": (200, 50, "demo", ["gg", "x"]),
    "admiralty-form": (200, 50, "demo", ["c", "s", "OPS", "<Tab>", "Release builds and their notes"]),
    "admiralty-legacy": (200, 50, "demo", ["G", "w"]),
    "admiralty-broken-config": (200, 50, "sandbox-broken", ["3", "j", "j", "j", "j", "j", "j", "j", "w"]),
    "admiralty-sandbox-monitor-only": (200, 50, "sandbox", ["3", "/NOLABEL", "<Enter>", "n", "<Escape>", "w"]),
}


# --- the font ---------------------------------------------------------------

def find_font(arg: str | None) -> tuple[Path, Path]:
    """The regular and bold faces of a JetBrainsMono Nerd Font."""
    cands: list[Path] = []
    given = arg or os.environ.get("ADMIRALTY_SCREENS_FONT")
    if given:
        p = Path(given).expanduser()
        cands = [p] if p.is_file() else sorted(p.rglob("JetBrainsMono*Nerd*.ttf")) or sorted(p.rglob("*.ttf"))
    else:
        for d in ["~/.local/share/fonts", "~/.fonts", "/usr/share/fonts", "/usr/local/share/fonts", "~/Library/Fonts"]:
            d = Path(d).expanduser()
            if d.is_dir():
                cands += sorted(d.rglob("JetBrainsMono*Nerd*.ttf")) + sorted(d.rglob("JetBrainsMonoNerd*.ttf"))
    regs = [c for c in cands if c.name.endswith("-Regular.ttf")] or cands
    if not regs:
        sys.exit("[ERROR] no Nerd Font found; pass --font DIR (any Nerd Font folder) or a .ttf")
    reg = regs[0]
    bold = reg.with_name(reg.name.replace("-Regular", "-Bold"))
    return reg, bold if bold.is_file() else reg


# --- running Admiralty in tmux ----------------------------------------------

def tmux(sock: str, *args: str, check: bool = True) -> str:
    r = subprocess.run(["tmux", "-L", sock, "-f", "/dev/null", *args], capture_output=True, text=True)
    if check and r.returncode != 0:
        raise RuntimeError(f"tmux {' '.join(args)}: {r.stderr.strip()}")
    return r.stdout


def make_sandbox(bin_: Path, env: dict[str, str], broken: bool) -> None:
    subprocess.run([str(bin_), "sandbox", "create", SB], env=env, check=True, capture_output=True)
    if broken:
        toml = next(Path(SB).rglob("squadrons/argos/squadron.toml"))
        with toml.open("a") as f:
            f.write('\n[[ships]]\nname = "LOOSE\n')  # an unterminated string


def capture(bin_: Path, name: str, cols: int, rows: int, source: str, keys: list[str], tmp: Path) -> str:
    env = {
        "HOME": str(tmp / "home"),
        "PATH": f"{bin_.parent}:/usr/bin:/bin",  # its nimitz, for the version on the status row
        "TERM": "xterm-256color",
        "COLORTERM": "truecolor",
        "LANG": "C.UTF-8",
        "NIMITZ_HOME": str(tmp / "nimitz"),
        "NIMITZ_STATE": str(tmp / "state"),
        "ADMIRALTY_STATE": str(tmp / "state" / f"{name}.json"),
        "NIMITZ_NO_ANIM": "1",
        "NIMITZ_MUX": "none",
    }
    for d in ("home", "nimitz", "state"):
        (tmp / d).mkdir(parents=True, exist_ok=True)
    if source.startswith("sandbox"):
        argv = [str(bin_), "--sandbox", SB]
    else:
        argv = [str(bin_), f"--{source}"]
    sock = f"admiralty-screens-{os.getpid()}"
    envs = [f"{k}={v}" for k, v in env.items()]
    made = False
    try:
        if source.startswith("sandbox"):
            # never removes what it did not make: another run's, or anything else there
            try:
                os.mkdir(SB)
            except FileExistsError:
                sys.exit(f"[ERROR] {SB} exists (another run, or not this script's); remove it by hand, then run again")
            made = True
            make_sandbox(bin_, env, broken=source == "sandbox-broken")
        tmux(sock, "new-session", "-d", "-s", "s", "-x", str(cols), "-y", str(rows), "env", "-i", *envs, *argv)
        tmux(sock, "set-option", "-g", "status", "off")
        tmux(sock, "set-option", "-g", "window-size", "manual")
        tmux(sock, "resize-window", "-t", "s", "-x", str(cols), "-y", str(rows))
        time.sleep(2.0)
        for k in keys:
            if k.startswith("<sleep:"):
                time.sleep(float(k[7:-1]))
                continue
            if k.startswith("<") and k.endswith(">"):
                tmux(sock, "send-keys", "-t", "s", k[1:-1])
            else:
                tmux(sock, "send-keys", "-t", "s", "-l", k)
            time.sleep(0.35)
        time.sleep(0.8)
        return tmux(sock, "capture-pane", "-t", "s", "-p", "-e", "-N")
    finally:
        tmux(sock, "kill-server", check=False)
        if source.startswith("sandbox") and made:
            shutil.rmtree(SB, ignore_errors=True)


# --- ANSI to cells -----------------------------------------------------------

SGR = re.compile(r"\x1b\[([0-9;:]*)m")
OTHER = re.compile(r"\x1b(\[[0-9;?]*[A-Za-z]|\][^\x07]*\x07|.)")
PALETTE16 = [(0, 0, 0), (205, 49, 49), (13, 188, 121), (229, 229, 16), (36, 114, 200), (188, 63, 188), (17, 168, 205), (229, 229, 229),
             (102, 102, 102), (241, 76, 76), (35, 209, 139), (245, 245, 67), (59, 142, 234), (214, 112, 214), (41, 184, 219), (255, 255, 255)]


def c256(n: int) -> tuple[int, int, int]:
    if n < 16:
        return PALETTE16[n]
    if n < 232:
        n -= 16
        v = [0, 95, 135, 175, 215, 255]
        return (v[n // 36], v[(n // 6) % 6], v[n % 6])
    g = 8 + 10 * (n - 232)
    return (g, g, g)


class Cell:
    __slots__ = ("ch", "fg", "bg", "bold", "dim", "ul", "wide")

    def __init__(self, ch: str, fg, bg, bold: bool, dim: bool, ul: bool, wide: bool = False):
        self.ch, self.fg, self.bg, self.bold, self.dim, self.ul, self.wide = ch, fg, bg, bold, dim, ul, wide


def parse(text: str, cols: int, rows: int) -> list[list[Cell]]:
    grid = [[Cell(" ", None, None, False, False, False) for _ in range(cols)] for _ in range(rows)]
    for y, line in enumerate(text.split("\n")[:rows]):
        fg = bg = None
        bold = dim = ul = rev = False
        x, i = 0, 0
        while i < len(line) and x < cols:
            m = SGR.match(line, i)
            if m:
                ps = [int(p) if p else 0 for p in re.split("[;:]", m.group(1) or "0")]
                j = 0
                while j < len(ps):
                    p = ps[j]
                    if p == 0:
                        fg = bg = None
                        bold = dim = ul = rev = False
                    elif p == 1:
                        bold = True
                    elif p == 2:
                        dim = True
                    elif p == 22:
                        bold = dim = False
                    elif p == 4:
                        ul = True
                    elif p == 24:
                        ul = False
                    elif p == 7:
                        rev = True
                    elif p == 27:
                        rev = False
                    elif p in (38, 48) and j + 1 < len(ps):
                        if ps[j + 1] == 2 and j + 4 < len(ps):
                            col = tuple(ps[j + 2:j + 5])
                            j += 4
                        elif ps[j + 1] == 5 and j + 2 < len(ps):
                            col = c256(ps[j + 2])
                            j += 2
                        else:
                            col = None
                        if p == 38:
                            fg = col
                        else:
                            bg = col
                    elif p == 39:
                        fg = None
                    elif p == 49:
                        bg = None
                    elif 30 <= p <= 37:
                        fg = PALETTE16[p - 30]
                    elif 90 <= p <= 97:
                        fg = PALETTE16[p - 82]
                    elif 40 <= p <= 47:
                        bg = PALETTE16[p - 40]
                    elif 100 <= p <= 107:
                        bg = PALETTE16[p - 92]
                    j += 1
                i = m.end()
                continue
            m = OTHER.match(line, i) if line[i] == "\x1b" else None
            if m:
                i = m.end()
                continue
            ch = line[i]
            i += 1
            w = 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1
            f, b = (bg, fg) if rev else (fg, bg)
            grid[y][x] = Cell(ch, f, b, bold, dim, ul, w == 2)
            x += w
    return grid


# --- cells to PNG ------------------------------------------------------------

# box drawing: (up, down, left, right), 1 light, 2 heavy, 3 double
BOX: dict[str, tuple[int, int, int, int]] = {}


def _box() -> None:
    spec = {
        "─": (0, 0, 1, 1), "━": (0, 0, 2, 2), "│": (1, 1, 0, 0), "┃": (2, 2, 0, 0),
        "┌": (0, 1, 0, 1), "┐": (0, 1, 1, 0), "└": (1, 0, 0, 1), "┘": (1, 0, 1, 0),
        "┏": (0, 2, 0, 2), "┓": (0, 2, 2, 0), "┗": (2, 0, 0, 2), "┛": (2, 0, 2, 0),
        "├": (1, 1, 0, 1), "┤": (1, 1, 1, 0), "┬": (0, 1, 1, 1), "┴": (1, 0, 1, 1), "┼": (1, 1, 1, 1),
        "┣": (2, 2, 0, 2), "┫": (2, 2, 2, 0), "┳": (0, 2, 2, 2), "┻": (2, 0, 2, 2), "╋": (2, 2, 2, 2),
        "═": (0, 0, 3, 3), "║": (3, 3, 0, 0), "╔": (0, 3, 0, 3), "╗": (0, 3, 3, 0), "╚": (3, 0, 0, 3), "╝": (3, 0, 3, 0),
        "╴": (0, 0, 1, 0), "╶": (0, 0, 0, 1), "╵": (1, 0, 0, 0), "╷": (0, 1, 0, 0),
        "╸": (0, 0, 2, 0), "╺": (0, 0, 0, 2), "╹": (2, 0, 0, 0), "╻": (0, 2, 0, 0),
    }
    BOX.update(spec)


_box()
ROUND = {"╭": (0, 1, 0, 1), "╮": (0, 1, 1, 0), "╰": (1, 0, 0, 1), "╯": (1, 0, 1, 0)}
BLOCKS = {"█": (0, 0, 1, 1), "▀": (0, 0, 1, 0.5), "▄": (0, 0.5, 1, 1), "▌": (0, 0, 0.5, 1), "▐": (0.5, 0, 1, 1)}


def draw_box(d: ImageDraw.ImageDraw, ch: str, x: int, y: int, cw: int, chh: int, col) -> bool:
    lw = {1: 2, 2: 4, 3: 2}
    cx, cy = x + cw // 2, y + chh // 2
    if ch in BLOCKS:
        a, b, c, e = BLOCKS[ch]
        d.rectangle([x + a * cw, y + b * chh, x + c * cw - 1, y + e * chh - 1], fill=col)
        return True
    if ch in ROUND:
        u, dn, l, r = ROUND[ch]
        rad = min(cw, chh) // 2
        w = lw[1]
        # straight parts up to the arc, then a quarter circle
        if u:
            d.rectangle([cx - w // 2, y, cx - w // 2 + w - 1, cy - rad], fill=col)
        if dn:
            d.rectangle([cx - w // 2, cy + rad, cx - w // 2 + w - 1, y + chh], fill=col)
        if l:
            d.rectangle([x, cy - w // 2, cx - rad, cy - w // 2 + w - 1], fill=col)
        if r:
            d.rectangle([cx + rad, cy - w // 2, x + cw, cy - w // 2 + w - 1], fill=col)
        ox = cx + (rad if r else -rad)
        oy = cy + (rad if dn else -rad)
        start = {(0, 1, 0, 1): 180, (0, 1, 1, 0): 270, (1, 0, 0, 1): 90, (1, 0, 1, 0): 0}[(u, dn, l, r)]
        d.arc([ox - rad - w // 2, oy - rad - w // 2, ox + rad + w // 2 - 1, oy + rad + w // 2 - 1], start, start + 90, fill=col, width=w)
        return True
    if ch not in BOX:
        return False
    u, dn, l, r = BOX[ch]
    for weight, rect in ((u, (0, y, 0, cy)), (dn, (0, cy, 0, y + chh)), (l, (x, cy, cx, 0)), (r, (cx, cy, x + cw, 0))):
        if not weight:
            continue
        w = lw[weight]
        offs = [-2, 2] if weight == 3 else [0]
        for o in offs:
            if rect[0] == 0:  # vertical
                d.rectangle([cx + o - w // 2, rect[1], cx + o - w // 2 + w - 1, rect[3]], fill=col)
            else:
                d.rectangle([rect[0], cy + o - w // 2, rect[2], cy + o - w // 2 + w - 1], fill=col)
    return True


def mix(a, b, t: float):
    return tuple(int(a[i] * (1 - t) + b[i] * t) for i in range(3))


def render(grid: list[list[Cell]], reg: Path, bold: Path, out: Path, size: int = 20) -> None:
    fr, fb = ImageFont.truetype(str(reg), size), ImageFont.truetype(str(bold), size)
    cw = round(fr.getlength("M"))
    asc, desc = fr.getmetrics()
    chh = round(size * 1.22)
    rows, cols = len(grid), len(grid[0])
    pad, title = 10, 40
    W, H = cols * cw + 2 * pad, rows * chh + title + pad
    img = Image.new("RGB", (W, H), (0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([0, 0, W - 1, H - 1], radius=14, fill=FRAME_BG, outline=(70, 70, 66), width=2)
    for i, c in enumerate([(255, 95, 86), (255, 189, 46), (39, 201, 63)]):
        cx = 22 + i * 22
        d.ellipse([cx - 7, title // 2 - 7, cx + 7, title // 2 + 7], fill=c)
    tw = fr.getlength("admiralty")
    d.text(((W - tw) / 2, (title - chh) / 2 + (chh - asc - desc) / 2), "admiralty", font=fr, fill=(200, 200, 192))
    ox, oy = pad, title
    d.rectangle([ox, oy, ox + cols * cw - 1, oy + rows * chh - 1], fill=BG)
    base = (chh - asc - desc) // 2
    for y, line in enumerate(grid):
        for x, c in enumerate(line):
            if c.bg:
                d.rectangle([ox + x * cw, oy + y * chh, ox + (x + (2 if c.wide else 1)) * cw - 1, oy + (y + 1) * chh - 1], fill=c.bg)
    for y, line in enumerate(grid):
        for x, c in enumerate(line):
            if c.ch == " ":
                if c.ul:
                    col = c.fg or FG
                    d.rectangle([ox + x * cw, oy + y * chh + base + asc + 1, ox + (x + 1) * cw - 1, oy + y * chh + base + asc + 2], fill=col)
                continue
            col = c.fg or FG
            if c.dim:
                col = mix(col, c.bg or BG, 0.45)
            px, py = ox + x * cw, oy + y * chh
            if draw_box(d, c.ch, px, py, cw, chh, col):
                continue
            font = fb if c.bold else fr
            gw = font.getlength(c.ch)
            span = cw * (2 if c.wide else 1)
            # a Nerd Font "Propo" icon may be wider than a cell: a smaller size
            # of it, centred in the cell (the shape keeps its proportions)
            if gw > span * 1.05:
                small = ImageFont.truetype(font.path, max(8, int(size * span / gw)))
                l, t, r, b = small.getbbox(c.ch)
                d.text((px + (span - (r - l)) / 2 - l, py + (chh - (b - t)) / 2 - t), c.ch, font=small, fill=col)
            else:
                d.text((px + (span - gw) / 2, py + base), c.ch, font=font, fill=col)
            if c.ul:
                d.rectangle([px, py + base + asc + 1, px + span - 1, py + base + asc + 2], fill=col)
    img.save(out, optimize=True)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", default=str(REPO / "target" / "debug" / "admiralty"))
    ap.add_argument("--font")
    ap.add_argument("--out", default=str(REPO / "docs" / "screens"))
    ap.add_argument("--only", nargs="*")
    ap.add_argument("--keep-text", action="store_true", help="also write each capture as NAME.ansi")
    a = ap.parse_args()
    bin_ = Path(a.bin)
    if not bin_.is_file():
        sys.exit(f"[ERROR] no {bin_}; build it first: cargo build -p admiralty")
    if not shutil.which("tmux"):
        sys.exit("[ERROR] tmux is not installed")
    reg, bold = find_font(a.font)
    print(f"[OK] font {reg.name} / {bold.name}")
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="admiralty-screens-") as t:
        for name, (cols, rows, source, keys) in SCREENS.items():
            if a.only and name not in a.only:
                continue
            text = capture(bin_, name, cols, rows, source, keys, Path(t))
            if a.keep_text:
                (out / f"{name}.ansi").write_text(text)
            render(parse(text, cols, rows), reg, bold, out / f"{name}.png")
            print(f"[OK] {name}.png  {cols}x{rows}  {source}  keys {' '.join(keys) or '-'}")


if __name__ == "__main__":
    main()
