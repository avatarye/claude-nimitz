# admiralty-watch

A terminal view of every Claude Code fleet on this machine, run next to the
`[ADMIRALTY]` hub. Shows a live tree, fleet → agent → sub-agent → task, plus a
detail pane for the selected row. It updates by itself: a background thread
re-reads every source every 2 seconds and the screen redraws as each snapshot
lands (and once a second so ages tick). Written in Rust (ratatui); about 0.4% of
one core and 7 MB while running.

Plan and design decisions: `~/Dropbox/claude-orchestrators/admiralty/plans/plan-fleet-status-tui.md`.

## Run

```
cargo install --path .                 # -> ~/.cargo/bin/admiralty-watch
admiralty-watch                        # the TUI, this machine
admiralty-watch dump --others          # the same tree as plain text
admiralty-watch --interval 5           # slower refresh
```

After changing the code, run `cargo install --path .` again.

## Keys (vim style; `?` shows them in the app)

| Key | Does |
| --- | --- |
| `j` / `k` | down / up |
| `h` / `l` | fold, or go to parent / unfold, or go to first child |
| `gg` / `G` | top / bottom |
| `ctrl-d` / `ctrl-u` | half page down / up |
| `space` | expand / collapse |
| `/` | filter by agent, fleet or sub-agent name (enter keeps it, esc clears) |
| `n` / `N` | next / previous agent matching the filter |
| `!` | everything waiting on you; enter jumps to the agent |
| `enter` | on an agent: jump to its Herdr pane; elsewhere: expand / collapse |
| `a` | activate the selected agent: resume its last session, or cold-launch from FLEET_CONFIG |
| `m` | message a live lead or agent: a box opens, type, `enter` sends (details below) |
| `A` | attach to a live background agent in a terminal (tmux: new pane; otherwise this one; confirms) |
| `J` / `K` | next / previous row hosted in a live Herdr pane |
| `x` | delete a stopped session that is not a fleet agent: removes its background job rows with `claude rm` (confirms) |
| `y` | yank the selected session id |
| `I` | show sessions that belong to no fleet |
| `f` | filter fleets by state: needs you, busy, idle, no lead, inactive, elsewhere, unreadable (space toggles, `a` all) |
| `F` | only agents with a flag or a question |
| `v` / `s` | split: tree beside detail / tree above detail (default: beside when the window is 150+ columns wide) |
| `R` | refresh now |
| `?` / `q` | help / quit |

The tree is a list of frames. `[ADMIRALTY]` sits on top in a gold double-lined
frame (always drawn; its sub-agents and tasks inside, `enter` to its Herdr pane).
Then one frame per fleet, then OTHER when `I` is on. Each frame's title bar is a
tinted band across the whole width: name and notes (no lead, off-config) on the
left, the agent counts on the right as `icon n / icon n`: bell = needs you,
bolt = busy, moon = idle, stop = stopped, circle = not running (zero counts left
out). Tree lines start inside the frame, so nothing connects the fleets. A folded
or empty fleet is just its title bar. The cursor lands on titles too: `h`/`l`/
`space` fold and unfold, and the detail pane shows the fleet.

Columns, shared by every frame under one fixed header: NAME, STATUS, AGE, KIND
(`admiralty`, `fleet`, `lead`, `agent`, `sub-agent`, `task`), DETAIL. NAME
starts with Herdr's status dot when a Herdr pane hosts that session, then the
tree indentation; fleet names show as `[TAG]`. Each column grows with the window
from a minimum up to a maximum width.

Colours are Monokai. A fleet's frame and title band take its state colour:
pink = someone needs you, green = lead live and an agent busy, cyan = lead live
and all idle, orange = no live lead but something else live, grey = inactive
(nothing live), dim grey = elsewhere (not on this machine), purple =
FLEET_CONFIG.json unreadable. `f` hides or shows fleets by these same states; the
top bar lists the states shown while it is on.

Folds, the `/` filter, the `f` fleet filter, `F`, `I` and the split stick: they
are saved to `~/.local/state/admiralty-watch/ui.json` on every change and read at
start (`ADMIRALTY_WATCH_STATE` moves it; the sandbox keeps its own).

The detail pane opens with a large ASCII picture of the selected row's type, in
the row's colour, matching its glyph: anchor = `[ADMIRALTY]`, ship = fleet, crown =
lead, robot = agent, two figures = teammate, small bot = sub-agent, a checkbox per
task status, box = finished sub-agents, clock = stale sessions, `?` = OTHER. It
is left out when the pane is too narrow.

Each row's type has a Nerd Font icon (JetBrainsMono Nerd Font). For a terminal
without one, set `ADMIRALTY_WATCH_ICONS=ascii`.

`a` and `A` always show the exact command and wait for `y`. Every
action, Herdr jumps included, is logged to `~/.local/state/admiralty-watch/actions.log`.
Leads are never launched from here. An interactive session (a lead) can only be
reached through its Herdr pane: Claude Code has no command to message it from
outside.

## Herdr status dots

Rows hosted in a Herdr pane start with the same dot Herdr's sidebar shows
(Herdr 0.9.3, `status_icon` / `status_color`): yellow filled = working, red
filled = blocked, teal filled = done (finished, not yet viewed), green empty =
idle, grey `·` = unknown. Herdr `blocked` means the session sits at an approval or
question prompt, so admiralty-watch counts it as **needs you**; for interactive
leads this is the only such signal.

## Stopped leads

A lead is an interactive session: it never gets a job row, and its
`sessions/<pid>.json` goes away with the process. What it leaves is its
transcript. For each fleet on this machine without a live lead, the newest
transcript in that fleet dir's project (`~/.claude/projects/<dir with every
non-alphanumeric as ->/`) whose name (`custom-title.json`, else a `customTitle`
record) starts with `[TAG]-LEAD` becomes a stopped lead row. Its age is when it
was last active. Only you start a lead (`ccl` in the fleet dir); `a` refuses.

## Deleting sessions (`x`)

`x` removes a session's background job rows (its current one and its stale
ones) with `claude rm <id>`, after a confirm dialog showing the commands. Only
**stopped sessions that are not fleet agents** qualify: background sessions under
a lead and OTHER sessions. Declared fleet agents, leads, the hub, anything live,
and sessions that left only a transcript are refused with the reason. `rm` runs
through the native Claude Code binary (newest `~/.localtools/tools/claude-code/*/claude`;
`ADMIRALTY_WATCH_NATIVE_CLAUDE_BIN` overrides): on an ACME box the `claude` on PATH
swallows `rm` as a chat prompt and changes nothing. The result is checked by
re-reading the job rows, not by the exit code. Transcripts are kept.

## Messages (`m`)

`m` on a live row opens a message box. `enter` sends, `esc` cancels, `ctrl-u`
clears. Delivery is `herdr agent prompt <pane> <text>`, so it needs Herdr:

- a fleet lead, or `[ADMIRALTY]`: the text is typed into its own Herdr pane;
- an agent: the text goes to its fleet lead's pane, wrapped as "Message from the
  user for [FLEET]-AGENT. Relay it ... with SendMessage", and the lead relays it.

Herdr refuses a prompt while the target is blocked on an approval; the reason
shows in the footer. Stopped agents, agents whose lead is down, and leads with
no Herdr pane are refused up front. Each message is logged with the actions.

## Herdr

Inside Herdr (`HERDR_ENV=1`), each refresh also reads `herdr agent list`
(read-only, about 30 ms). Panes match sessions by the Claude session id Herdr
reports, with the pane title plus cwd as a fallback. Agents with a pane show a
status dot at the start of the row. `enter` runs `herdr workspace focus`, `tab focus`, then
`agent focus` on that pane. It only moves focus.

`--exit-after-jump` (or `ADMIRALTY_WATCH_EXIT_AFTER_JUMP=1`) quits the TUI once a
jump lands, so a Herdr popup running it gets out of the way. A failed jump, or an
agent with no pane, leaves it up. The
[herdr-admiralty-watch](../herdr-admiralty-watch) plugin sets it.

## What the tree shows

| Glyph | Status |
| --- | --- |
| `!` | live, and its job has a question for you |
| `*` | live, busy |
| `-` | live, idle |
| `x` | stopped: has a job row, no process; `a` resumes it |
| `o` | declared in FLEET_CONFIG, never run here; `a` cold-launches it |

Flags: `off-config` (cwd differs from the config), `DUPLICATE LIVE`, `name variant`
(lead name with a suffix), `not in config`, `untagged name`, `left a question`
(stopped while waiting on you).

What counts as a fleet agent: only the lead and the agents declared in the
fleet's `FLEET_CONFIG.json` (the list `FLEET.md` describes). Any other session
that belongs to the fleet, by `[TAG]-` name or by running in a fleet dir (e.g. a
background helper the lead started), is shown under the lead as a sub-agent marked
"background session"; if the fleet has no lead row, a placeholder lead holds them.
Their questions still count as "needs you". Finished sub-agents are listed for an
hour after their last write; the detail pane gives the all-time total.

Rules: a session is **live only if its row has a `pid`**; `state` goes stale.
The lead is the session named `[TAG]-LEAD...`, never guessed from its directory.
One row per name is shown; older rows fold into `+N stale`. A sub-agent is
"running" if its transcript was written in the last 90 s.

## Data sources

Session rows come from the files `claude agents --json --all` itself reads:
`~/.claude/sessions/<pid>.json` (live processes, liveness by `kill(pid, 0)`) and
`~/.claude/jobs/<id>/state.json` (background job rows). Starting the Node CLI
every 2 s cost about 0.6 s of CPU per refresh; reading the files costs
milliseconds. A test checks both give the same tree. If those files are missing
(a future Claude Code layout), it falls back to the CLI; `ADMIRALTY_WATCH_SOURCE=cli`
forces the CLI.

Claude Code sometimes rewrites a session file in place without truncating it,
leaving bytes of an older, longer version after the JSON object; `claude agents
--json` then drops that live session. admiralty-watch reads only the first JSON
value of each file, so such a session stays live. A live session whose file says
`status: waiting` (Claude Code 2.1.286+, with `waitingFor`, e.g. "permission
prompt") counts as **needs you**.

Detail comes from `jobs/<id>/timeline.jsonl`, `projects/*/<session>/subagents/*`
and `tasks/<team>/*.json`. All of these are internal to Claude Code and can
change in any release; every reader degrades to empty instead of crashing.

## Sandbox

A fictional world with a fake `claude` and a fake `herdr` (this same binary in a
hidden mode) that record calls and never start or focus anything real. Live
sandbox sessions record pid 1, so the real liveness check works unchanged. It has 5 fleets covering every status and
flag, plus 4 Herdr panes.

```
admiralty-watch sandbox create /tmp/aw-sb
admiralty-watch --sandbox /tmp/aw-sb
admiralty-watch sandbox tick /tmp/aw-sb      # second pane: keeps it changing
```

`calls.log` and `herdr_calls.log` in the sandbox show what the fakes received.

## Tests

```
cargo test      # 29 tests: model rules, file-vs-CLI parity, actions, Herdr, keys, rendering
```

Each test builds its own sandbox in the temp dir, so they run in parallel.

## Human test checklist

Sandbox first (safe), then live.

1. `sandbox create`, open it, and run `tick` in a second pane. SCOUT should flip
   between busy and idle, and its sub-agents should stay "running".
2. `!` → FORGE listed as live, ARCHIVE as stopped with a question; enter jumps.
3. On ARCHIVE press `a`: the dialog says *Resume*, with the newest session id. `y`
   → ARCHIVE goes live, `+3 stale`.
4. On BEACON press `a`: *Cold-launch*, permission `auto`. On NOPROMPT and GHOST,
   `a` refuses with a reason. On any LEAD, `a` refuses.
5. On ALPHA's lead press `m`, type, `enter`: `herdr_calls.log` gets
   `agent prompt w2:p2 <text>`. On SCOUT, `m` says it goes through [ALPHA]-LEAD,
   and the logged text is wrapped for relay. `J`/`K` hop between rows with a pane;
   `enter` on one logs the three focus calls. `A` on SCOUT attaches (tmux: new pane).
6. `/`, `n`/`N`, `I`, `F`, `y` behave as above; the cursor stays put across refreshes.
7. Live, inside Herdr: `admiralty-watch`. Leads show the pane icon; `enter` on one
   should switch you to its workspace. Compare the tree against `ListAgents` in
   the hub, then try `a` on a stopped agent you actually want back.
