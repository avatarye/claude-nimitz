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

Closing the overlay also clears a zoom the user had on the pane under it.

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
and then closing the overlay leaves `zoomed: false` on that tab. Zooming the
pane while the overlay is open does not survive the close either.

## Expected

- The overlay's terminal starts at the inner size of the box Herdr draws for
  it, whatever pane had the focus (and the same for `zoomed` and `tab`
  placements: the size of what is drawn).
- Closing an overlay leaves the zoom of the panes under it as it was.

## Workaround (claude-nimitz 0.7.9)

The Admiralty overlay (`admiralty herdr watch`) runs `herdr pane zoom
$HERDR_PANE_ID --off` and `--on` before it draws; the toggle notes the zoom of
the pane under the overlay and turns it back on once the overlay is gone
(`crates/admiralty/src/herdr_plugin.rs`). Both go away when Herdr fixes this.
