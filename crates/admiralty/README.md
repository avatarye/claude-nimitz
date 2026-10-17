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

- Two status rows at the bottom, no key row. The ADMIRALTY row: fleets, live
  sessions, how many need you, error and warning counts, the multiplexer (or a
  MONITOR-ONLY badge), and `? keys` at its right (`q` quit is in the `?`
  panel). The row under it: `Nimitz: v0.7.5` (`Nimitz: vY (admiralty vX,
  mismatch)` as a warning when it is not Admiralty's own, `Nimitz: …` until
  `nimitz --version` answers, `Nimitz: ?` when it does not within 5 s),
  `Claude Code: v2.1.296 (supported)` (out of range: `(unsupported)` as an
  error, `(newer than tested, ...)` as a warning; `Claude Code: not found`), `Machine: home-server`, and the local date and time flush right
  (`2026-10-14 15:42`). When no refresh has landed for three intervals,
  `stale 12s` (a warning) sits before the clock; otherwise the refresh is not
  shown. At 80 columns and below both shorten (`v0.7.5  CC v2.1.296
  home-server ... 10-14 15:42`; narrower still, `15:42`).
- The row under the fleet tabs is blank (no column names).
- A tab bar on the top line: one tab per fleet (`1 HOME  2 MAIN ...`, with
  `!n` where members need you), then `unassigned` when a stowaway sits under the
  nimitz home in no fleet's folders. A tab shows only its fleet. `tab` /
  `shift-tab` step through them, `1`-`9` pick one; a jump to a row on another
  tab (`!`, `i`) shows that tab; a `/` search spans every fleet.
- Colours are theme A, "Monokai refined" (`docs/design/admiralty-ui`): five
  hues with one meaning each (pink needs you, the only filled chip; orange
  error; yellow warning and legacy; green busy; cyan focus, key hints and fix
  commands) and four neutral steps (faint not started, muted ages and paths,
  body text and idle, bright live names). Squadron and stowaway title bars are
  filled with their status colour, their text dark; on the cursor the text is
  underlined and the bar's ends turn cyan. On the cursor row, faint and muted
  text is lifted.
- A fleet has no line of its own: its tab carries its state word and its
  `icon n / icon n` status and issue counts (info issues are not counted;
  under 100 columns only needs-you and errors). The other tabs show how many
  need you.
- The admiral sits in a heavy box (`┏━ ▾ ADMIRAL ━┓`, a brighter title) with
  its crew; that top line is the fleet's row (folding it folds the fleet, `x`
  on it has the fleet's actions), and a fleet whose config does not load
  shows its error in full under it.
- A box per squadron under a title bar filled with the squadron's status, the
  worst of its commodore and ships, in this order: pink needs you, orange
  error (an issue at Error, or a config that does not load), yellow warning
  (an issue at Warn), green busy, else neutral (idle, stopped, parked, not
  started; info issues do not count), a muted grey that still reads as a bar.
  The bar reads `▾ <flag> ARGOS   <status glyph> busy   <ship icon> 6  ...counts`:
  the same icons as the rows (the status glyph of the squadron's state: needs
  you, an issue mark for broken, busy, idle, stopped for no commodore, off for
  inactive; then the ship icon and the number of ships; `ADMIRALTY_ICONS=ascii`:
  `* busy   S 6`). The text and the icons on the bar are dark (`INK`, at least
  5.9:1 on every fill). The sides
  stay neutral. Ships are rows; crew, tasks and extra job rows sit under their
  member with tree guides. Crew show while running and for 30 minutes after
  they finish (a nested crew follows its parent; a running one keeps its
  parent in view); older finished crew fold into one `+N finished` row per
  member, which `z` opens and closes, kept in the saved
  state.
- Status motion (in the live loop only): a busy member's status glyph spins (a
  pie that fills; `| / = \` in ASCII), one waiting on a background shell turns
  an hourglass at a quarter of the speed, a running crew's glyph spins dimmer,
  the needs-you chip breathes once a second, and a squadron whose status gets
  worse (a warning, an error, needs you) pulses its bar once; parked has its
  own glyph. It runs on its own 100 ms tick only while something moves and no
  dialog is open, never refreshes or calls nimitz, and never moves a column.
  `v` turns it off (saved); `NIMITZ_NO_ANIM=1` or `NO_COLOR` keep it off;
  `admiralty dump` and the tests are always still. `cargo run --example
  anim_frames -- <dir> <cols> <rows> <frames>` writes the frames' cells as
  JSON, for a preview GIF drawn with a fixed-grid font.
- Spacing: a blank line between the boxes, a column of padding inside each,
  two spaces between columns. It is the first thing to go: below 103 columns
  (where it would cost NAME or DETAIL their room) the padding and gaps go, and
  the blank lines go whenever the list does not fit the height.
- A STOWAWAYS box for unlabelled sessions in fleet scope, on the tab of
  the fleet that holds them (a member's folder, else the fleet's own folder,
  else the fleet its `legacy` issue names; `unassigned` otherwise). A session of
  the old fleet-skill fleets (issue `legacy`) has its own yellow history marker,
  and `x` then `d` adopts it as the identity its fix names.
- Counts read `icon n` (`!2 / *3 / -1`: needs you, busy, idle ...; triangles
  are errors and warnings, orange and yellow on the tab, told apart by their
  shape on a filled bar), right-aligned as a group; under 100 columns only
  needs-you and errors stay. A Nerd Font icon has a blank cell before its
  number, so a terminal that draws it two cells wide does not cover the
  number. `?` has a legend for every icon.
- Columns NAME / STATUS / AGE / DETAIL line up across all boxes (the kind is in
  row's detail, under `x`). NAME starts with the pane dot (green working, pink
  blocked, grey done, plain idle, faint unknown), the tree guides, the fold
  marker and the kind icon (admiral, commodore, ship, crew, teammate, stowaway,
  copy; a task's status); no issue mark there. DETAIL starts with the issue
  glyph, coloured by severity, and the issue codes; then what a member needing
  you waits for, or `N crew running`; else its topic, the one line Claude Code
  keeps on what the job is doing (a finished job's result); an `unknown`
  member, its first reason. The reasons for a status are in the row's detail
  (under `x`).
- Crew: a member's subagents under it. The agents a Workflow run started sit
  under one row per run (the workflow's name, else its `wf_` run id; live /
  total; the newest agent's phase), open while one of them runs. A run, like a
  crew, folds into `+N finished` once all its agents finished over 30 min ago.
- The row's detail is under its actions: `x` opens the actions menu with a
  panel below it (the list keeps the full width otherwise). A large picture of
  the row's kind, status, WHY THIS STATUS (core's reasons), ISSUES with their
  `nimitz` fix commands and the session. The panel takes what room the menu
  leaves; a long one scrolls (`ctrl-e` / `ctrl-y` a line, `PgDn` / `PgUp`
  five), and its title says so.

## Keys

![finder](../../docs/screens/admiralty-finder.png)

`space` opens the finder: every member of every fleet, one line each in tree
order (its path `MAIN/ARGOS/SIGNAL`, its expected name `[MAIN.ARGOS]-SIGNAL`,
its status word and its topic), then the stowaways by short id and name.
Matching is fzf's (the `nucleo-matcher` crate, built in): space-separated terms
must all match (`argos busy`), a letter at a word start scores higher, and a
term that spells the path's word initials in order ranks first (`mas`:
`MAIN/ARGOS/SIGNAL`; `hgn`: `HOME/GITHUB/NIMITZ_DEV`). `up/down` or
`ctrl-n/ctrl-p` move, the matched letters are lit, and on a wide screen the
selected one's detail shows beside the list. `enter` puts the cursor on its row
(its fleet's tab, opening folds and clearing what hides it), so `enter` again
jumps to its pane; `esc` leaves the cursor and the filter as they were.

`tab/shift-tab 1-9` fleet tabs, `j/k h/l gg/G ctrl-d/u`, `z` fold / unfold (on
a member with crew that finished over 30 min ago: its `+N finished`), `/` filter
(every fleet), `n/N`, `space` the finder, `enter` jump to the pane,
`J/K` rows with a pane, `!` waiting on you, `i` every issue (enter goes to the
row, `y` yanks the fix), `a` start (top-down), `u` up a fleet, `A` attach a background session (ship, stowaway) in a
new Herdr tab named `F.SQ.SHIP` (the same as `nimitz open <id>`), `x` actions (every action of the row: start, open, stop, restart, sweep, rm, adopt, remove from config, message;
on a fleet band also up, `layout F/... [--ships]`, `R` muster and removing a machine; a closed one says why; sweep is
scoped to the row: member, squadron or fleet; the row's detail sits below the menu),
`m` message (typed into the member's own pane, a ship's too), `c` create fleet / squadron / ship, `e` edit ship, `D` remove
from config, `y` yank the session id, `I` stowaways, `f` filter squadrons by state (a note says how many are hidden), `F`
flagged only, `R` refresh, `?` the keys panel (`?` or `esc` closes it), `q` quit. `esc` closes what is open, one
thing at a time (a dialog, menu or form, the `/` name filter), and
with nothing open quits (not while an action runs: `q` quits then).

Every action and form shows the exact `nimitz` command and core's plan steps
(refusals with their fixes) and waits for `y`. Forms validate as you type
(names with core's rules, the rest by asking core for the plan); `ctrl-e` edits
a prompt or brief in `$EDITOR`.

Every plan comes from core: `sessions::plan_*` (removal is `plan_retire`: a
ship is stopped, its job rows removed, then the config), `config::plan_*`, and
`layout::plan` for `layout F/... [--ships]` (tabs whose session already has a pane are
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

A squadron whose config does not load stays on screen with an orange bar
and the error in full (file:line and the parser's words) above its members; a
fleet's, in full at the top of its admiral's box.

At 80x24 the ADMIRALTY row keeps the mux state and `? keys`, the row under it
keeps the versions and the clock, DETAIL keeps room for issue codes, the detail under `x` drops its picture to
show the reasons first (ctrl-e / ctrl-y, PgDn / PgUp scroll it), and help and the issues view
wrap and scroll. The first frame comes from the session files alone; `claude
agents` follows right after, and keys typed meanwhile are kept.

## Screens

`docs/screens/`: `admiralty-wide`, `admiralty-issue-detail`,
`admiralty-issues-view`, `admiralty-confirm`, `admiralty-actions-menu`,
`admiralty-form`, `admiralty-fleet-menu`,
`admiralty-legacy`, `admiralty-broken-config`, `admiralty-sandbox-monitor-only`,
`admiralty-narrow`, `admiralty-80x24`, `admiralty-help`.

They are generated, never hand-taken:

```
cargo build -p admiralty -p nimitz
uv run --with pillow tools/admiralty_screens.py [--font DIR] [--only admiralty-help ...]
```

The script runs the built TUI (`--demo`, `--demo-monitor`, or a fresh
`admiralty sandbox create /tmp/adm-sb`, its ARGOS squadron.toml broken for
`admiralty-broken-config`) in a private, detached tmux server at a fixed size
(200x50; 100x30 narrow; 80x24), sends the keys that open each view, captures
the pane's cells with their 24-bit colours (`tmux capture-pane -e`) and draws
them with a JetBrainsMono Nerd Font (box drawing drawn as lines, so frames
have no gaps). NIMITZ_HOME and NIMITZ_STATE point at a temp dir; nothing reads
the real state. Each screen's size, source and keys are in its `SCREENS`
table; change a key sequence there when a view moves.
The committed screens were drawn with JetBrainsMono Nerd Font Propo (Regular
and Bold; not in this repo). A JetBrainsMono Nerd Font is found under the usual
font folders; else `--font DIR` (or `ADMIRALTY_SCREENS_FONT`) takes any Nerd
Font folder (its `-Regular.ttf`, and the `-Bold.ttf` beside it) or one .ttf. With
none the script refuses rather than draw without the icons.
