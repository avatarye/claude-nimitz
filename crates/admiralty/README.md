# admiralty

The TUI where you watch and control every claude-nimitz fleet on this machine.
It reads `nimitz_core::snapshot::snapshot` (plus core's `check` and session
issues) and the multiplexer's panes (`nimitz-mux`); every action is a core plan
(`sessions::plan_*`, `config::plan_*`) shown in full before it runs. It parses
no Claude Code files itself.

![overview](../../docs/screens/admiralty-wide.png)

## Run

```
admiralty                          # this machine (NIMITZ_* env, contracts.md section 2)
admiralty --sandbox DIR            # a fake world: `admiralty sandbox create DIR` first
admiralty --demo                   # fixture data with a multiplexer; nothing runs
admiralty --demo-monitor           # fixture data, no multiplexer
admiralty dump [--width N] [...]   # the list as plain text
admiralty --exit-after-jump        # quit once a pane jump lands (ADMIRALTY_WATCH_EXIT_AFTER_JUMP=1; the Herdr plugin sets it)
```

`ADMIRALTY_ICONS=ascii` for a terminal without a Nerd Font. Folds, filters and
the split are saved to `$NIMITZ_STATE/admiralty-ui.json` (`ADMIRALTY_STATE`
moves it). Actions are logged to `$NIMITZ_STATE/admiralty-actions.log`.

## Layout

- Top bar: fleets, live sessions, how many need you, error and warning counts,
  the multiplexer (or a MONITOR-ONLY badge), a Claude Code version warning.
- One gold double-line frame per fleet. Its title band is tinted by the fleet's
  state (pink needs you, purple broken, green busy, cyan idle, orange no
  admiral, grey inactive, dim elsewhere), with `icon n / icon n` status and issue
  counts on the right. The admiral is the first row inside.
- A rounded frame per squadron inside its fleet, tinted the same way, with its
  comms rule; ships and tenders are rows; crew, tasks and extra job rows sit
  under their member with tree guides.
- A pink STOWAWAYS frame for unlabelled sessions in fleet scope.
- Columns NAME / STATUS / AGE / KIND / DETAIL line up across all frames. NAME
  starts with the pane dot (yellow working, red blocked, teal done, green
  idle, grey unknown) and an issue mark coloured by severity; DETAIL lists the
  issue codes.
- Detail pane (beside when 150+ columns, below otherwise; `v` / `s`): a large
  picture of the row's kind, status, WHY THIS STATUS (core's reasons), ISSUES
  with their `nimitz` fix commands, the session, and what each key does here or
  why it will not.

## Keys

`j/k h/l gg/G ctrl-d/u space`, `/` filter, `n/N`, `enter` jump to the pane,
`J/K` rows with a pane, `!` waiting on you, `i` every issue (enter goes to the
row, `y` yanks the fix), `a` start (top-down), `u` up a fleet, `A` attach in a
new pane, `x` actions (stop, relaunch, sweep, rm, adopt, remove from config),
`m` message (admiral and commodore: typed into their pane; ships: relayed by
their commodore), `c` create fleet / squadron / ship, `e` edit ship, `D` remove
from config, `y` yank the session id, `I` stowaways, `f` fleet-state filter, `F`
flagged only, `R` refresh, `?` help, `q` quit.

Every action and form shows the exact `nimitz` command and core's plan steps
(refusals with their fixes) and waits for `y`. Forms validate as you type
(names with core's rules, the rest by asking core for the plan); `ctrl-e` edits
a prompt or brief in `$EDITOR`.

With no multiplexer Admiralty is monitor-only: start, open, jump and message
are closed and say why; stop, sweep, rm, adopt and config changes still work.

## Screens

`docs/screens/`: `admiralty-wide`, `admiralty-issue-detail`,
`admiralty-issues-view`, `admiralty-confirm`, `admiralty-actions-menu`,
`admiralty-form`, `admiralty-sandbox-monitor-only`, `admiralty-narrow`.
