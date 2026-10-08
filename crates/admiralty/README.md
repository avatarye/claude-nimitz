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
admiralty --exit-after-jump        # quit once a pane jump lands (ADMIRALTY_EXIT_AFTER_JUMP=1; the Herdr plugin sets it)
```

`ADMIRALTY_ICONS=ascii` for a terminal without a Nerd Font. Folds, filters and
the fleet tab are saved to `$NIMITZ_STATE/admiralty-ui.json` (`ADMIRALTY_STATE`
moves it). Actions are logged to `$NIMITZ_STATE/admiralty-actions.log`.

## Layout

- Status bar (the ADMIRALTY line, at the bottom right above the key bar):
  fleets, live sessions, how many need you, error and warning counts, the
  multiplexer (or a MONITOR-ONLY badge), a Claude Code version warning.
- A tab bar on the top line: one tab per fleet (`1 HOME  2 MAIN ...`, with
  `!n` where members need you), then `unassigned` when a stowaway sits under the
  nimitz home in no fleet's folders. A tab shows only its fleet. `tab` /
  `shift-tab` step through them, `1`-`9` pick one; a jump to a row on another
  tab (`!`, `i`) shows that tab; a `/` search spans every fleet.
- Colours are theme A, "Monokai refined" (`docs/design/admiralty-ui`): five
  hues with one meaning each (pink needs you, the only filled chip; orange
  error; yellow warning and legacy; green busy; cyan focus, key hints and fix
  commands) and four neutral steps (faint not started, muted ages and paths,
  body text and idle, bright live names). Bands sit on one raised background,
  never tinted. On the cursor row and band, faint and muted text is lifted.
- A fleet is one band line (no frame: each fleet has its own tab), its state
  word on it, with `icon n / icon n` status and issue counts on the right
  (info issues are not counted).
- The admiral sits in a heavy box (`┏━ ADMIRAL ━┓`, a brighter title) with its
  crew.
- A rounded box per squadron, its title and comms rule in the top border. The
  top border takes the squadron's status, the worst of its commodore and
  ships, in this order: pink needs you, orange error (an issue at Error, or a
  config that does not load), yellow warning (an issue at Warn), green busy,
  else neutral (idle, stopped, parked, not started; info issues do not count).
  The sides stay neutral. Ships are rows; crew, tasks and extra job rows sit
  under their member with tree guides. Crew show while running and for 30
  minutes after they finish (a nested crew follows its parent; a running one
  keeps its parent in view); older finished crew fold into one `+N finished`
  row per member, which `z` (or space on it) opens and closes, kept in the
  saved state.
- Spacing: a blank line between the boxes, a column of padding inside each,
  two spaces between columns. It is the first thing to go: below 103 columns
  (where it would cost NAME or DETAIL their room) the padding and gaps go, and
  the blank lines go whenever the list does not fit the height.
- A STOWAWAYS box for unlabelled sessions in fleet scope, on the tab of
  the fleet that holds them (a member's folder, else the fleet's own folder,
  else the fleet its `legacy` issue names; `unassigned` otherwise). A session of
  the old fleet-skill fleets (issue `legacy`) has its own yellow history marker,
  and `x` then `d` adopts it as the identity its fix names.
- Band counts read `icon n` (`!2 / *3 / -1`: needs you, busy, idle ...; orange and yellow
  triangles are errors and warnings), right-aligned as a group; under 100 columns only
  needs-you and errors stay. `?` has a legend for every icon.
- Columns NAME / STATUS / AGE / KIND / DETAIL line up across all boxes. NAME
  starts with the pane dot (green working, pink blocked, grey done, plain
  idle, faint unknown) and an issue mark coloured by severity; DETAIL lists the
  issue codes.
- Detail panel (`w` opens and closes it, `esc` closes it; closed at start, so
  the list takes the full width): it floats over the right of the list and
  follows the cursor row. A large
  picture of the row's kind, status, WHY THIS STATUS (core's reasons), ISSUES
  with their `nimitz` fix commands, the session, and what each key does here or
  why it will not.

## Keys

`tab/shift-tab 1-9` fleet tabs, `j/k h/l gg/G ctrl-d/u space`, `/` filter
(every fleet), `n/N`, `enter` jump to the pane,
`J/K` rows with a pane, `!` waiting on you, `i` every issue (enter goes to the
row, `y` yanks the fix), `a` start (top-down), `u` up a fleet, `A` attach a background session (ship, stowaway) in a
new Herdr tab named `F.SQ.SHIP` (the same as `nimitz open <id>`), `x` actions (stop, relaunch, sweep, rm, adopt, remove from config;
on a fleet band also `layout herdr [--ships]` and removing a machine; sweep is
scoped to the row: member, squadron or fleet),
`m` message (admiral and commodore: typed into their pane; ships: relayed by
their commodore), `c` create fleet / squadron / ship, `e` edit ship, `D` remove
from config, `y` yank the session id, `w` the detail panel (`esc` closes it), `I` stowaways, `f` filter squadrons by state (a note says how many are hidden), `F`
flagged only, `R` refresh, `?` help, `q` quit. `esc` closes what is open, one
thing at a time (a dialog or form, the detail panel, the `/` name filter), and
with nothing open quits (not while an action runs: `q` quits then).

Every action and form shows the exact `nimitz` command and core's plan steps
(refusals with their fixes) and waits for `y`. Forms validate as you type
(names with core's rules, the rest by asking core for the plan); `ctrl-e` edits
a prompt or brief in `$EDITOR`.

Every plan comes from core: `sessions::plan_*` (removal is `plan_retire`: a
ship is stopped, its job rows removed, then the config), `config::plan_*`, and
`layout::plan` for `layout herdr` (tabs whose session already has a pane are
dropped, the rest go to the multiplexer's `layout`).

`tests/cli_drift.rs` runs every command Admiralty shows through the real CLI
with `--plan --json` in a sandbox and compares the plans, so the two cannot
drift. `herdr-plugin/tests/e2e_admiralty_in_herdr.py` runs Admiralty in a
private Herdr server (pane dots, `m` into an admiral's pane, `A`, enter).

Inside a Herdr pane Admiralty uses that pane's Herdr server; started outside
Herdr (a terminal, a launcher) it uses the server the user's nimitz commands
recorded in `active.json`, so it sees the same panes as inside.

With no multiplexer Admiralty is monitor-only: what needs a pane is closed and
says why (starting an admiral or commodore, `up`, open, attach, jump, typing a
message); ships start, relaunch and stop as usual, and so do sweep,
rm, adopt and config changes.

A squadron whose config does not load stays on screen with an orange top border
and the error in full (file:line and the parser's words) above its members; a
fleet's, in full under its band.

At 80x24 the header keeps the mux state, the footer keeps `? help` and `q/esc
quit`, DETAIL keeps room for issue codes, the detail pane drops its picture to
show the reasons first (ctrl-e / ctrl-y scroll it), and help and the issues view
wrap and scroll. The first frame comes from the session files alone; `claude
agents` follows right after, and keys typed meanwhile are kept.

## Screens

`docs/screens/`: `admiralty-wide`, `admiralty-issue-detail`,
`admiralty-issues-view`, `admiralty-confirm`, `admiralty-actions-menu`,
`admiralty-form`, `admiralty-fleet-menu`,
`admiralty-legacy`, `admiralty-broken-config`, `admiralty-sandbox-monitor-only`,
`admiralty-narrow`, `admiralty-80x24`, `admiralty-help`.
