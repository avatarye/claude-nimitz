# Herdr: an overlay plugin pane's terminal starts at the focused pane's size

Draft of an upstream bug report for Herdr. Not filed.

## Summary

A plugin pane opened with `placement = "overlay"` is drawn as a box over the
whole tab, but its terminal (the PTY size the process sees) starts at the size
of the pane that had the focus when it opened. The size is right only after
the next terminal resize. A full-screen TUI in the overlay therefore draws at
the focused pane's width inside a tab-wide box, or at the whole tab's size
(2 rows and 2 columns larger than the box) when that pane was zoomed.
`placement = "zoomed"` and `placement = "tab"` start at the same wrong size.

Closing the overlay with `herdr plugin pane close` also clears a zoom the
user had on the pane under it; an overlay that exits after the focus moved
to another pane of its tab leaves that pane zoomed.

## Version

herdr 0.9.3 (protocol 22), Linux. Default config, sidebar shown.

## Steps

1. A plugin with an overlay pane whose command logs its size every second:
   `sh -c 'while true; do stty size >> /tmp/sizes; sleep 1; done'`.
2. A 160x45 terminal; one tab (the tab area is 134x44); split it once
   (`herdr pane split w1:p1 --direction right`), focus the left pane.
3. `herdr plugin pane open --plugin probe --entrypoint ov --focus`.
4. Resize the terminal.

## Observed

| Before opening | Box drawn | `stty size` at open | after a resize |
| --- | --- | --- | --- |
| focused pane is half the tab (67 wide) | whole tab | `44 67` | `42 131` |
| focused pane zoomed, or the only pane | whole tab | `44 134` | `42 131` |

`herdr pane layout` while the overlay is open: the focused pane was split in
two and the new pane zoomed (`zoomed: true`), so the PTY seems to get the
pre-split pane's rect rather than the zoomed rect or the box's inner size.

`herdr pane zoom <overlay> --off` then `--on` resizes the terminal to the
box (`44 67` -> `42 30` -> `42 131`): the zoomed size is right once a zoom
changes.

Zoom: with the pane under it zoomed (`herdr pane zoom w1:p1 --on`), opening
and then closing the overlay with `herdr plugin pane close` leaves `zoomed:
false` on that tab. Zooming the pane while the overlay is open does not
survive the close either.

Zoom is one flag per tab (`pane layout` reports `zoomed` and the tab's
`focused_pane_id`), on the pane with the tab's focus. When the overlay's own
process exits instead (a private server, 2026-11-02; tab `w1:t1` is
`w1:p1 | w1:p2`, `w1:t2` is `w1:p3 | w1:p4`, every pane an agent):

| Tab 1 before | then, overlay open | tab 1 after the exit |
| --- | --- | --- |
| not zoomed | nothing | not zoomed |
| `w1:p1` zoomed | nothing | `w1:p1` zoomed |
| not zoomed | `tab focus w1:t1; agent focus w1:p2` | `w1:p2` zoomed |
| `w1:p1` zoomed | `agent focus w1:p2` | `w1:p2` zoomed |
| `w1:p1` zoomed | `tab focus w1:t2; agent focus w1:p4` | `w1:p1` zoomed, `w1:t2` untouched |

So an exiting overlay that still has its tab's focus gives the tab back its
zoom from before it opened, but one that lost the focus to another pane of
its tab leaves the overlay's zoom on that pane. With `plugin pane close`
instead, the third row ends `w1:p2`, not zoomed.

## Expected

- The overlay's terminal starts at the inner size of the box Herdr draws for
  it, whatever pane had the focus (and the same for `zoomed` and `tab`
  placements: the size of what is drawn).
- Closing an overlay leaves the zoom of the panes under it as it was, by
  either path, whether or not the focus moved within its tab.

## Workaround (claude-nimitz 0.7.9)

The Admiralty overlay (`admiralty herdr watch`) runs `herdr pane zoom
$HERDR_PANE_ID --off` and `--on` before it draws; the toggle notes the zoom of
the pane under the overlay and turns it back on once the overlay is gone;
and the watch, as it quits, turns off the zoom a jump within the overlay's
tab handed to its target, unless that pane was the zoomed one (after 0.7.19,
`settle_zoom` in `crates/admiralty/src/herdr_plugin.rs`). All go away when
Herdr fixes this.
