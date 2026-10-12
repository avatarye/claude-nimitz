# CLI UX for 0.7.3: Perforce paths, four command families, muster

Status: agreed plan, not built. To be built as 0.7.3, with no backward
compatibility: old forms are removed outright. Code references are to
`dev` at 4df9b06 (0.7.1).

## 1. Paths

- A path names one member: `HOME` is the admiral, `HOME/GITHUB` the
  commodore, `HOME/GITHUB/SHIP` a ship. The same in every verb.
- `HOME/...` is everything under it, the lead included; `//...` is every
  fleet. `...` is only ever the last segment of a path, never a bare word:
  oh-my-zsh makes `.....` and longer (and, without the user's override, `...`)
  global aliases that expand standalone words, but never part of a word.
  `print -r -- HOME/... //...` passes both through zsh unchanged (checked).
- No `*`. `--only` and `--all` go.
- `...` leaves parked ships alone; `--parked` includes them. Leads in the
  match are always brought up.
- Several paths per command; a member matched twice counts once. An unknown
  path or an empty match is a refusal that lists what is there.
- Authority is never widened by a pattern: a match outside the caller's
  scope refuses the whole command, naming the path that fits.

Build map: `restore::in_target` (crates/nimitz-core/src/restore.rs:841-849)
becomes the path matcher; `plan_start` (restore.rs:574-633) takes a set of
paths, and its `target`/`named` split (one named member may be cold-launched,
restore.rs:939-940) becomes "an exact path is named, a `...` match is not";
`members::parse_id` (crates/nimitz/src/members.rs:9-16) grows `parse_paths`;
`sessions::start_cmd` (sessions.rs:424-427) drops its `--only`;
`RestoreOpts` (restore.rs:110-140) loses `all`, `only`, `shape_only`,
`ships`, gains `parked`.

## 2. Four families

| Family | Commands | Touches |
| --- | --- | --- |
| Fleet shape | `fleet`, `squadron`, `ship` new\|set\|rm; `machine add\|rm`; `settings get\|set` (was `fleet settings`); `harness list\|add\|rm\|push` | config only, never launches |
| Session control | `start <path> [--fresh] [--parked]`; `restart <path>` (was `start -f`); `stop`, `rm`, `sweep`, `adopt`; `signal` (inbox only) | jobs, never panes |
| Pane control | `open <path> [--tab]` (was `start -p`); `attach <path>`; `close <path>`; `layout <path>` | panes; `open` wakes first |
| Misc | `status [<path>]`, `check`, `rules`, `doctor`, `init`, `trust`, `migrate`, `kb`, `hook`, `sandbox` | read-only or setup |
| One-shot | `muster [<path>]` (section 3) | both, as one plan |

Removed: `restore`, `relaunch`, `up`, `layout herdr`, the deprecated `open`
alias, `start -p/-f/--tab/--only/--all/--jobs-only` (main.rs:126-131,
238-249; members.rs:177-204, 283-290, 519-629, 772-783;
crates/nimitz/src/restore.rs:96-107). `open` keeps its name with the new
meaning.

Where today's code mixes the families, and what the split changes:

| Today | Where | 0.7.3 |
| --- | --- | --- |
| `start` makes every lead's tab whenever Herdr is there | members.rs:239; restore.rs:1007-1008 | `start` makes no tab; `layout` and `muster` do |
| `-p`, `--tab` on `start`, six usage refusals | members.rs:218-237; restore.rs:581-598 | `open [--tab]` |
| `start -f` reattaches the panes the relaunch cut | restore.rs:1457 | `restart` keeps this: a relaunch must not leave a dead pane; it is continuity, not a new pane |
| `attach` wakes a down member through start's plan | members.rs:673-690 | kept: `attach` and `open` both wake first |
| `signal` types into a lead's tab, opening one if needed | members.rs:347-433 | inbox only (see 6.4) |
| `layout herdr` is the restore engine in `shape_only` mode | restore.rs:117-119; crates/nimitz/src/restore.rs:111-113 | `layout <path>` is that mode, renamed |

## 3. `nimitz muster [<path>]` (working name)

The one command after a reboot, a Claude Code update or a Herdr closure.
Default path `//...`. One plan, `--plan` then `--yes`:

1. every live member is relaunched, older Claude Code or not;
2. leads that are down are woken; parked ships stay parked unless `--parked`;
3. every member that had a pane is reattached in its existing Herdr pane,
   else in its lead tab when the pane is gone.

Engine: `restore::plan` with `restart: true` and the active set
(restore.rs:548-570) already does 1-3; muster is that call under a new
verb, scoped by path.

## 4. No mid-turn skip

- Remove `relaunch.busy` (restore.rs:1428-1436, and the run-time recheck in
  crates/nimitz/src/restore.rs:294-307) and the `needs-you` skip
  (restore.rs:1019-1021); drop both from `SKIPS` (restore.rs:751). A busy
  member is relaunched; its plan line says "busy: its turn in flight is
  dropped"; one waiting on a prompt says "waiting: its question is dropped".
- Self-relaunch: when the caller is a member inside the target, it is
  planned last and run after everything else is done and the report is
  printed, by a detached `nimitz __relaunch-self <id> <job>` (setsid and a
  double fork on Unix, `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` on
  Windows; output to `NIMITZ_STATE/relaunch-self.log`). It replaces
  `start.self` and `restore.self` (restore.rs:612-623, 1214-1230,
  1388-1398). The helper waits for the caller's turn to end (its session
  status idle, with a timeout) so the caller can read the report, then
  relaunches it by exact id and reattaches its pane. To check before
  relying on it:
  - the helper survives `claude stop` of the caller's job (the Bash tool's
    process tree, and any process-group or cgroup kill by the daemon);
  - the relaunch resumes the same session id (no copy, no `history.lost`);
  - a second run while a helper is pending starts no second one (a lock);
  - the caller's next session start reports the outcome (from the log, via
    the SessionStart hook's context).

## 5. No "yours alone"

Any member may run these. Role scoping between members (`may_start`,
`may_config`, sessions.rs:429-462) and the SendMessage chain hook stay.

| Check | Where | Change |
| --- | --- | --- |
| `init` | crates/nimitz/src/setup.rs:141-142 | removed |
| `trust` | setup.rs:221-222; the trust step only for the user in `trust::for_plan` (crates/nimitz-core/src/trust.rs:238) and `sessions::trust_first` (sessions.rs:1027) | removed: a member's start trusts a new member's folder as yours does |
| `migrate` (and `--shared`) | crates/nimitz/src/crud.rs:381-384 | removed |
| `kb clone` | crates/nimitz/src/kb_cmd.rs:143-149 | removed |
| `settings set kb.*` | crates/nimitz-core/src/fleet_settings.rs:148-151; `may_run_in` (sessions.rs:534) | removed |
| `attach` | members.rs:649-651 | removed; but see 6.3 |
| `open` and the pane commands | restore.rs:581-582; members.rs:526-528; `may_run_in` (sessions.rs:500) | removed |
| `may_run_in` rows for `up`, `layout`, `restore` | sessions.rs:519-525 | deleted with the verbs; new rows for `restart`, `open`, `close`, `layout`, `muster`, `settings` |
| Not in the plan, still the user's | `doctor --fix` (setup.rs:287-290), `sandbox` (sandbox_cmd.rs:14), fleet new/rm and machine add/rm (crud.rs:17-18, sessions.rs:1842-1843), `stop <short>` of a session outside the fleet (sessions.rs:1400, 1587) | decide (6.5) |

The authority test runs every command as every role (main.rs:220-223,
plan-v4 section 12.1); its table changes with every row above.

## 6. What looks wrong or hard

1. **"Parked" after a reboot is every ship.** A member with a job row, no
   pid and no SessionEnd reads as parked (crates/nimitz-core/src/
   snapshot.rs:673-694: "retired, crashed or stopped with the machine"). So
   `muster` after a reboot leaves every ship down, the working ones too.
   Today's restore wakes the active set's ships (restore.rs:1012-1015,
   1056). Proposed: for `...` and `muster`, "parked" means "not in the
   active set" (`active.json`), so what was active comes back and
   `--parked` adds the rest.
2. **Muster after a Herdr closure relaunches everything.** A Herdr closure
   leaves every job live and current; step 1 drops every turn in flight
   across `//...` only to reattach panes. Proposed: muster relaunches only
   workers on an older Claude Code (today's rule), and `restart //...` is
   the explicit "relaunch all". If the plan stands, the plan view must lead
   with "N turns in flight will be dropped".
3. **`attach` from a member cannot work.** A member always runs inside
   Claude Code, so `attach` is refused as `attach.nested` (members.rs:653-
   667) whatever the authority, and `claude attach` exec'd in a Bash tool
   has no terminal. Lifting "yours alone" for it changes nothing; `open` is
   the member's way.
4. **`signal`, inbox only, loses the user's voice.** Text typed into a tab
   is the user's own input; an inbox post is marked "a request, not the
   user's approval" (members.rs:377). A user who signals a lead to approve
   something no longer can. Keep `signal --type` for the user, or say so in
   the rules.
5. **The leftover user-only checks** (last row of section 5) need a call.
6. **Members opening panes** run in the daemon's environment; their Herdr
   is found through `active::member_mux_vars` (main.rs:46-51). That works
   for today's lead tabs; `open` and `close` from a member need the same,
   and a member on another machine cannot reach the user's Herdr at all.

## 7. Bugs fixed in 0.7.3

- `nimitz status` takes no argument, yet five fixes say `nimitz status
  <id>` (members.rs:50, 604, 608, 694; crates/nimitz/src/restore.rs:312,
  335): `status [<path>...]`.
- `<cmd> --help` is not supported (`nimitz stop --help` says "missing id
  or session"): every command prints its own usage, as does `nimitz help
  <cmd>`.
- The five verbs deprecated in 0.5.0 are still in 0.7.1: removed (section 2).

## 8. Text and tests that change

- Rules (crates/nimitz-core/rules/): common.md:11-13 (a lead is attached
  with `open`), 24-30 (start, restart, muster, paths); admiral.md:5-11 and
  commodore.md:5-9, 21 (paths, no `-p` reservation, self-relaunch is
  automatic). Bump `rules::VERSION` (rules.rs:25, now 8).
- Skills: plugin/skills/fleet/SKILL.md (23-28, the authority table 54-69,
  the task table 105-114), admiral/SKILL.md:27-31, 51-55,
  commodore/SKILL.md:23, 31, 40, 52.
- Admiralty: crates/admiralty/src/actions.rs:460-468 builds `start <id>
  [--only] [-f|-p]`; it becomes `start`, `restart`, `open` by path, and the
  `R` key runs `muster F/...`; `layout::plan` (actions.rs:406) stays.
- USAGE (main.rs:63-135); README.md:48-60 and the reference section.
- Tests: crates/nimitz/tests/restore.rs (12 `--only`), cli.rs, e2e.rs,
  upgrade.rs, adversarial_round1.rs move to paths and the new verbs; new
  tests for path matching, `--parked`, muster, busy relaunch and the
  self-relaunch helper (the last needs real Claude Code: an e2e).
- CHANGELOG 0.7.3 "Upgrading": every old form and its replacement, since
  nothing prints a pointer.
