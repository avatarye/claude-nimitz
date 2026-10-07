# Admiralty for Herdr

A [Herdr](https://herdr.dev) plugin that calls up `admiralty`
([claude-nimitz](../README.md), `crates/admiralty`), the live fleet tree, over
whatever you are doing. Press the key to open it; press it again, quit it with
`q`, or jump to an agent with `enter`, and it goes away.

Needs Herdr 0.9 and `admiralty` on `PATH` or in `~/.cargo/bin`
(`cargo install --path crates/admiralty`; it has `--exit-after-jump`).

The plugin id is `admiralty` (until nimitz 0.3.0 it was `admiralty-watch`; see
Install for the switch).

## Leads in their tabs

Every fleet member is a Claude Code background job (nimitz 0.4.0). A lead's
tab (`F.ADMIRAL`, `F.SQ.COMMODORE`; `nimitz restore <F>` or `nimitz open
<id>` makes it) runs `claude attach <job>`. Detach with **ctrl-z**: the job
keeps running and the tab drops back to its shell. Do not type `/exit` inside
an attach: that opens Claude Code's agent view of the whole machine, not the
member's end (`nimitz stop <id>` ends a member). `enter` in the watch jumps
to a lead's tab; `A` attaches a job that has no tab yet; `x R` on a fleet
band restores the fleet after a Herdr restart, a closed tab or a reboot
(`nimitz restore <F>`: tabs found by their full label or by the session
they show, attach typed only into a plain shell, jobs resumed by exact id).

## How it works

One action, `admiralty.toggle`, runs `bin/toggle.sh`:

| State when you press the key | Result |
| --- | --- |
| watch not open | opens in this tab as an overlay, focused |
| watch open in this tab | closes; focus goes back to the pane you were in |
| watch open in another tab | closes there, opens here |

The overlay runs `bin/watch.sh`, which runs `admiralty` with
`ADMIRALTY_EXIT_AFTER_JUMP=1`. The overlay is a Herdr plugin pane, so it closes
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
the label check (`Admiralty`), so the toggle never closes a pane it did not open.

## Install

    herdr plugin link <claude-nimitz repo>/herdr-plugin
    herdr plugin list          # admiralty ... enabled

Linked as `admiralty-watch` before? Unlink that first so two copies never run
(`herdr plugin unlink admiralty-watch`), link this one, and change the
keybinding's `command` from `admiralty.toggle` to `admiralty.toggle`.

Then add this to `~/.config/herdr/config.toml`. `alt+f` (f for fleet) is a direct
key, one press with no prefix; it is free in Herdr's defaults and in the current
config. `prefix+f` also works if you would rather keep it behind the prefix:

```toml
[[keys.command]]
key = "alt+f"
type = "plugin_action"
command = "admiralty.toggle"
description = "admiralty fleet tree"
```

Check and load it:

    herdr config check
    herdr server reload-config

The action also shows in Herdr's action list, and `herdr plugin action invoke
toggle --plugin admiralty` runs it from a shell.

### Options

`bin/watch.sh` reads these from the Herdr server's environment. Extra arguments
can also go in a file, read when `ADMIRALTY_ARGS` is unset, with no restart:
`$(herdr plugin config-dir admiralty)/args`.

| Variable | Default | |
| --- | --- | --- |
| `ADMIRALTY_BIN` | `admiralty` on PATH, then `~/.cargo/bin` | binary to run |
| `ADMIRALTY_ARGS` | the `args` file, else none | extra arguments, e.g. `--sandbox /tmp/aw-sb` |
| `ADMIRALTY_EXIT_AFTER_JUMP` | `1` | `0` keeps the watch up after a jump |

The `ADMIRALTY_WATCH_*` names of 0.3.0 still work when the new ones are unset.

## Uninstall

Remove the `[[keys.command]]` block, then:

    herdr plugin unlink admiralty
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
   `d=$(herdr plugin config-dir admiralty); mkdir -p "$d"; echo "--sandbox /tmp/aw-sb" > "$d/args"`.
   Delete that file before step 6.
2. In any pane, `ctrl+g f`: the watch covers the tab, focused.
3. `ctrl+g f` again: it closes and you are back in the pane you came from.
4. Open it, press `q`: it closes.
5. Open it in one tab, switch tabs, `ctrl+g f`: it leaves the old tab and opens in
   this one.
6. Live: open it, select a lead with a pane icon, `enter`: Herdr switches to that
   lead's workspace and pane and the watch is gone.
7. Open it, `enter` on an agent with no pane: it stays up and says why.
