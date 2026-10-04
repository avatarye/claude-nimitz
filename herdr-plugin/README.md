# herdr-admiralty-watch

A [Herdr](https://herdr.dev) plugin that calls up `admiralty`
([claude-nimitz](../README.md), `crates/admiralty`), the live fleet tree, over
whatever you are doing. Press the key to open it; press it again, quit it with
`q`, or jump to an agent with `enter`, and it goes away.

Needs Herdr 0.9 and `admiralty` on `PATH` or in `~/.cargo/bin`
(`cargo install --path crates/admiralty`; it has `--exit-after-jump`).

The plugin id stays `admiralty-watch` on purpose: an existing install keeps its
`herdr plugin link`, its state dir, and the `admiralty-watch.toggle` keybinding
with no re-linking. Only the binary it runs changed, from the retired
`admiralty-watch` repo to claude-nimitz's `admiralty`.

## How it works

One action, `admiralty-watch.toggle`, runs `bin/toggle.sh`:

| State when you press the key | Result |
| --- | --- |
| watch not open | opens in this tab as an overlay, focused |
| watch open in this tab | closes; focus goes back to the pane you were in |
| watch open in another tab | closes there, opens here |

The overlay runs `bin/watch.sh`, which runs `admiralty` with
`ADMIRALTY_WATCH_EXIT_AFTER_JUMP=1` (the binary keeps the `ADMIRALTY_WATCH_`
env prefix). The overlay is a Herdr plugin pane, so it closes
when that process exits: on `q`, and right after `enter` lands on an agent's pane.
Sending a message with `m` keeps the watch up. A jump that fails, or an agent with no pane, leaves the watch up with
the reason in its status line.

Why an overlay and not a popup: a Herdr popup is session-modal. While it is up,
Herdr's own keys do not fire, so this toggle could never close it, and
`plugin pane open` returns no pane id for a popup, so nothing else can close it
either. An overlay covers the tab like a zoomed pane, has an id, and leaves Herdr's
keys working. It takes the whole tab, not a floating box.

The state dir (`$HERDR_PLUGIN_STATE_DIR/pane_id`) holds the id of the watch pane the
toggle opened. A stale id fails `pane get`; an id that now names another pane fails
the label check (`Admiralty Watch`), so the toggle never closes a pane it did not open.

## Install

    herdr plugin link <claude-nimitz repo>/herdr-plugin
    herdr plugin list          # admiralty-watch ... enabled

Already linked from the old `herdr-admiralty-watch` repo? Re-link once so the
path points here (`herdr plugin unlink admiralty-watch`, then the link above);
the keybinding does not change.

Then add this to `~/.config/herdr/config.toml`. `alt+f` (f for fleet) is a direct
key, one press with no prefix; it is free in Herdr's defaults and in the current
config. `prefix+f` also works if you would rather keep it behind the prefix:

```toml
[[keys.command]]
key = "alt+f"
type = "plugin_action"
command = "admiralty-watch.toggle"
description = "admiralty watch"
```

Check and load it:

    herdr config check
    herdr server reload-config

The action also shows in Herdr's action list, and `herdr plugin action invoke
toggle --plugin admiralty-watch` runs it from a shell.

### Options

`bin/watch.sh` reads these from the Herdr server's environment. Extra arguments
can also go in a file, read when `ADMIRALTY_WATCH_ARGS` is unset, with no restart:
`$(herdr plugin config-dir admiralty-watch)/args`.

| Variable | Default | |
| --- | --- | --- |
| `ADMIRALTY_WATCH_BIN` | `admiralty` on PATH, then `~/.cargo/bin` | binary to run |
| `ADMIRALTY_WATCH_ARGS` | the `args` file, else none | extra arguments, e.g. `--sandbox /tmp/aw-sb` |
| `ADMIRALTY_WATCH_EXIT_AFTER_JUMP` | `1` | `0` keeps the watch up after a jump |

## Uninstall

Remove the `[[keys.command]]` block, then:

    herdr plugin unlink admiralty-watch
    herdr server reload-config

## Tests

    tests/check.sh                                       # offline
    uv run --with pyte python tests/e2e_isolated.py      # private Herdr server

`check.sh` parses the scripts, links the manifest into a throwaway Herdr config
(Herdr's own validation), runs `herdr config check` on the snippet above, and runs
`bin/toggle.sh` against a fake `herdr` (open, close, move between tabs, stale id,
open failure).

`e2e_isolated.py` starts a separate Herdr client and server in a pseudo-terminal
under a temp `XDG_CONFIG_HOME` / `XDG_STATE_HOME`, with every `HERDR_*` variable
unset, so it cannot reach the running Herdr. It links the plugin there, binds
`prefix+f`, runs the watch on an admiralty sandbox, presses real keys, and
checks panes and tabs through that server's API. It stops only that server.

## Human test checklist

1. Sandbox first: `admiralty sandbox create /tmp/aw-sb`, then
   `d=$(herdr plugin config-dir admiralty-watch); mkdir -p "$d"; echo "--sandbox /tmp/aw-sb" > "$d/args"`.
   Delete that file before step 6.
2. In any pane, `ctrl+g f`: the watch covers the tab, focused.
3. `ctrl+g f` again: it closes and you are back in the pane you came from.
4. Open it, press `q`: it closes.
5. Open it in one tab, switch tabs, `ctrl+g f`: it leaves the old tab and opens in
   this one.
6. Live: open it, select a lead with a pane icon, `enter`: Herdr switches to that
   lead's workspace and pane and the watch is gone.
7. Open it, `enter` on an agent with no pane: it stays up and says why.
