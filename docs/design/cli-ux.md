# CLI UX for 0.7.3: Perforce paths, four command families, muster, no authority

Status: agreed plan, not built. To be built as 0.7.3, with no backward
compatibility: old forms are removed outright. Code references are to
`dev` at 4df9b06 (0.7.1).

## Decisions for the user (most important first, with a recommendation)

1. **Muster after a Herdr-only closure**: relaunch every live member (turns
   in flight dropped), or only workers on an older Claude Code?
   Recommended: older only; `restart //...` is the explicit "relaunch all".
2. **A member stopping or removing itself** (`stop`, `rm`, `ship rm`, `fleet
   rm`, or a `...` that matches it): run last, detached, after the report
   (section 4), or refuse it (a non-authority `self.end` refusal with the
   fix to run from a shell)? Recommended: detached last, one path for every
   verb that ends or replaces the caller.
3. **"Parked"**: see 6.1. Recommended: `status` says `down` for a member the
   machine took down while active and `parked` only for one the daemon
   retired or that was not active; `...` and muster wake `down`, skip
   `parked`; `--parked` adds those.
4. **`signal --type`** for your own voice (6.4, 6.5)? Recommended: yes, the user's
   `signal` types into a shown tab (today's behaviour), `--inbox` forces the
   inbox; a member's `signal` always uses the inbox.
5. **Self-relaunch idle-wait timeout**: cut the turn, or give up?
   Recommended: give up after 10 minutes, log it, and say so at the
   member's next prompt; never cut a turn the caller is still in.
6. **kb-guard and `--fleet`**: `kb_cmd` would allow `--fleet OTHER`, while
   kb-guard pins the KB agents to the member's own fleet (kb_guard.rs:207).
   Recommended: kb-guard stays pinned (the plan keeps it as is); a member's
   own read-only `kb show/grep --fleet OTHER` is allowed.
7. **`close` and the leftover flags**. Recommended: `close <path>` closes
   the panes that show those members through a new nimitz-mux
   `close_pane` (there is none today), never a pane running anything but
   the member's attach; jobs untouched. `--here` and `--rename` go to
   `layout` and `muster`; `--force` (a second tab beside a taken one,
   `tab.dup`) goes to `open` and `layout`; `--ships` to `layout`. With no
   multiplexer `start`/`restart` need none (they make no panes); `muster`
   runs its job steps and says the pane steps were skipped; `open`,
   `close`, `layout` refuse (`mux.none`).
8. **The risk of no checks** (5.3): a ship can run `fleet rm`, `machine rm`,
   `migrate --shared` or `trust`, guarded only by "plan first" and Claude
   Code's permission prompt (none in a bypass mode). Recommended: accept,
   as asked; the ship rules say ships change no fleet shape, and every one
   of those commands keeps its plan and `--yes`.

## 1. Paths

- A path names one member: `HOME` is the admiral, `HOME/GITHUB` the
  commodore, `HOME/GITHUB/SHIP` a ship. The same in every verb.
- `HOME/...` is everything under it, the lead included; `//...` is every
  fleet. `...` is only ever the last segment of a path, never a bare word:
  oh-my-zsh makes `.....` and longer (and, without the user's override, `...`)
  global aliases that expand standalone words, but never part of a word.
  `print -r -- HOME/... //...` passes both through zsh unchanged (checked).
  MSYS / Git Bash rewrites a leading `//` to `/`, so `/...` is accepted as
  `//...`. `//...` never includes the ADMIRALTY workspace (R-35,
  restore.rs:104, 691).
- No `*`. `--only` and `--all` go.
- `...` leaves parked ships alone; `--parked` includes them. Leads in the
  match are always brought up.
- Several paths per command; a member matched twice counts once. An unknown
  path or an empty match is a refusal that lists what is there.
- One exception (0.7.4): `check F` covers everything under F, as `check F/...`
  does (bug 24: a read-only check never hides a ship's errors).

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
- Self-relaunch, self-stop, self-removal: removing `may_start` also removes
  its `target != id` guard (sessions.rs:432), so a member's `stop`, `rm`,
  `ship rm`, `fleet rm` or `stop HOME/...` would end it mid-command. Every
  verb that ends or replaces the caller takes one path (decision 2): when
  the caller is a member inside the target, it is planned last and run after everything else is done and the report is
  printed, by a detached `nimitz __relaunch-self <id> <job>` (setsid and a
  double fork on Unix, `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` on
  Windows; output to `NIMITZ_STATE/relaunch-self.log`). It replaces
  `start.self` and `restore.self` (restore.rs:612-623, 1214-1230,
  1388-1398). The helper waits for the caller's turn to end (its session
  status idle, with a timeout) so the caller can read the report, then
  runs the caller's own step (relaunch by exact id and reattach its pane;
  or stop; or stop then rm). Tests (an e2e with real Claude Code for the
  first three):
  - the report reaches the Bash tool before the stop;
  - the helper survives `claude stop` of the caller's job (the Bash tool's
    process tree, a process-group or cgroup kill by the daemon);
  - the relaunch resumes the same session id (no copy, no `history.lost`),
    and a helper killed between stop and resume loses no member (the next
    `start` wakes it by its exact id);
  - the idle-wait timeout (decision 5);
  - the lock: a second run while a helper is pending starts no second one;
  - `--plan` spawns nothing;
  - the outcome is appended to the next SessionStart context;
  - Herdr is found through `member_mux_vars` from the detached helper;
  - the Windows detach flags.

## 5. No authority checks, open messaging

### 5.1 Every member may run every command

nimitz refuses nothing on authority: every member may run every command on
any member or fleet. The caller's identity (its labels) stays, for three
uses: self-relaunch ordering (section 4), kb sender ids, and the default
fleet of `kb`, `settings` and `harness` (`kb_cmd::fleet_of`,
crates/nimitz/src/kb_cmd.rs:15-26, keeps its default and loses its refusal
at :21). Two non-authority uses of `Caller::User` also stay: which Herdr a
command talks to (main.rs:46-51) and that only your own command records a
pane env (crates/nimitz-core/src/active.rs:179).

Partial or mismatched labels (`sessions::UNVERIFIED`, sessions.rs:70-95)
print one `[WARN]` on stderr and run as you; kb-guard keeps reading
`UNVERIFIED` as it does (kb_guard.rs:207), unchanged.

Checks to remove (code at 4df9b06; "refusal" means the whole branch goes):

| Where | What |
| --- | --- |
| crates/nimitz-core/src/sessions.rs:429-439 `may_start`, :446-457 `may_config`, :465-545 `may_run` / `may_run_in` | the authority table itself; delete all three |
| sessions.rs:581-590 (`may_touch_row`), 1066-1067, 1118, 1127-1128 (`plan_start_with`), 1165 (`plan_up`, removed with the verb) | `may_start` refusals |
| sessions.rs:1313-1314 (`plan_stop`), 1432-1433 (`plan_relaunch`), 1547-1548 (`plan_rm`), 1788-1789, 1803-1804 (`plan_adopt`) | `may_start` refusals, the stowaway-fleet check |
| sessions.rs:1400, 1405, 1415, 1587, 1602 | "anything else is the user's" codes: a session that names nothing is `session.unknown` for everyone |
| sessions.rs:1714-1749 (`plan_sweep`) | the per-role scope and its refusals (a member's `sweep` with no path still defaults to its own scope) |
| sessions.rs:1083-1090 (`plan_start_with`) | the chain partition: a member's own chain's errors turned into warnings; goes with the caller distinction |
| sessions.rs:1665 | a ship never stops a live copy: goes |
| sessions.rs:1842-1850 | fleet removal is the user's |
| sessions.rs:1027 `trust_first`; crates/nimitz-core/src/trust.rs:238 `for_plan` | the trust step for the user only: every caller gets it |
| crates/nimitz-core/src/restore.rs:551-561 (`plan`), 581-582, 604-615 (`plan_start`) | caller scoping and refusals; the caller is still found for self-relaunch |
| crates/nimitz-core/src/layout.rs:146-160 | layout scoping |
| crates/nimitz-core/src/fleet_settings.rs:148-151 | `kb.*` is the user's |
| crates/nimitz/src/run.rs:67-80 `fixes_for` | "ask the user to run" rewriting: delete, every fix is shown as is |
| crates/nimitz/src/crud.rs:15-25 `authority`, :248, :381-384 | config and migrate refusals |
| crates/nimitz/src/harness_cmd.rs:19-35 | the admiral's-or-yours check and the own-folder check |
| crates/nimitz/src/kb_cmd.rs:21, 143-149 | other fleet's kb, `kb clone` |
| crates/nimitz/src/setup.rs:141-142, 221-222, 287-290 | `init`, `trust`, `doctor --fix` |
| crates/nimitz/src/sandbox_cmd.rs:14-15 | `sandbox` |
| crates/nimitz/src/members.rs:333-341, 526-528, 649-651 | a member's `signal` sent to SendMessage, `open`, `attach` (the `attach.nested` check at 653-667 stays: it is not authority) |

Also: the comments at claude.rs:157-159 and main.rs:222 (the authority
test), the plugin.json:4 description, and the `attach.nested` fix text
(members.rs:657: `start -p` becomes `open`). `notify.rs:43` reads the
identity to address a needs-you notice: identity, not authority; it stays.

The `start.self` / `restore.self` skips are not authority and are replaced
by the self-relaunch (section 4).

### 5.2 Any member may message any member

- Remove the PreToolUse SendMessage decision (hook.rs:116-140),
  `may_signal` and `signal_table` (hook.rs:405-475). No signal is blocked.
- Remove the squadron `comms` setting: `--comms` (crud.rs:37-43, 109-116;
  args.rs:7), its validation (config.rs:323-326, 1175-1184), `Comms` in
  `plan_squadron_new/set` (config.rs:677, 707, 1156), `Squadron.comms`
  (types.rs:481) and its fill in snapshot.rs:344, 372, 409 (it is in the
  status JSON too), migrate.rs:238, check.rs:468, and Admiralty's display
  and forms (crates/admiralty/src/detail.rs:199-203, 255; view.rs:726-730,
  811; forms.rs:85-99, 145, 217, 310, 340, 390; fixtures.rs). Nothing else
  needs it. One catch: `SquadronConfig` is `deny_unknown_fields`
  (types.rs:189-203) and every squadron.toml written so far holds `comms`,
  so the field stays parsed and ignored (`#[serde(default,
  skip_serializing)]`; `Comms::default()` is open): a dropped field would
  refuse every existing squadron. Older nimitz on another machine reads the missing field as its
  default.
- What the hook still does for SendMessage: the PreToolUse and PostToolUse
  lines in the event log (hook.rs:10-11; the `PreToolUse` matcher list in
  setup.rs:24). Nothing reads those lines but the hook's tests; the
  needs-you notice keys on AskUserQuestion, PermissionRequest and
  Notification (notify.rs:136-160), never on SendMessage. So the SendMessage
  matcher could go too, saving a process per message; kept by the plan.
- The plugin's hooks.json description ("rule SendMessage by the chain of
  command") changes.

### 5.3 Risk

With no check, any member, a ship included, can run `fleet rm`, `machine
rm`, `migrate --shared` or `trust` on a broad folder. What stands between
them and the change is the rules' "plan first, show the user" and Claude
Code's own permission prompt for the Bash call (none under a bypass
permission mode). `trust::too_broad` (trust.rs:235, 262) stays: it is a
safety refusal for everyone, not authority.

## 6. What looks wrong or hard

1. **"Parked" means two things.** `Status::Parked` covers a member the
   daemon retired and one the machine took down while active
   (snapshot.rs:673-694). The restore engine already wakes the second kind:
   `wanted = always_on || all || (in_set && not stopped)` (restore.rs:1056),
   and the active set keeps a taken-down member in the set
   (active.rs:222-229). So the plan's "`...` leaves parked ships alone" is
   today's rule if "parked" means the engine's meaning, and wrong if it
   means the status word: after a reboot every ship reads `parked`. The
   CLI must use one meaning in `status` and in `...` (decision 3).
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
   the member's way. (No authority check is involved any more.)
4. **`signal`, inbox only, loses the user's voice.** Text typed into a tab
   is the user's own input; an inbox post is marked "a request, not the
   user's approval" (members.rs:377). A user who signals a lead to approve
   something no longer can. Keep `signal --type` for the user, or say so in
   the rules.
5. Inbox-only `signal` also changes: the ship relay through its
   commodore (members.rs:347-360) goes, as anyone may message anyone; the
   inbox body ("relayed by nimitz from a `nimitz signal` run with no member
   labels", members.rs:377) must name the sender when a member sends it;
   a target that is down is refused with `nimitz start <path>` as the fix
   (today the same), never woken by a signal.
6. **Members and Herdr.** `Ctx::mux()` already routes a member through
   `member_mux_vars` (main.rs:46-51). The gaps: the Herdr identity is
   recorded only by your own command (active.rs:179), so a member's
   `layout` or muster after a Herdr restart may target a stale server
   until you run a command; and nimitz-mux has no close-pane primitive, so
   `close` is new code (decision 7). A member on another machine cannot
   reach your Herdr at all.

## 7. Build order

1. Delete the authority and comms checks (section 5); tests follow.
2. Path parser and `RestoreOpts` (section 1).
3. Verb split and removals (section 2), with the three bugs (section 8).
4. Busy relaunch (section 4, first bullet).
5. The self-end helper, with its e2e (section 4).
6. `muster` (section 3).
7. Rules, skills, README and Admiralty (section 9).

## 8. Bugs fixed in 0.7.3

- `nimitz status` takes no argument, yet five fixes say `nimitz status
  <id>` (members.rs:50, 604, 608, 694; crates/nimitz/src/restore.rs:312,
  335): `status [<path>...]`.
- `<cmd> --help` is not supported (`nimitz stop --help` says "missing id
  or session"): every command prints its own usage, as does `nimitz help
  <cmd>`.
- The five verbs deprecated in 0.5.0 are still in 0.7.1: removed (section 2).

## 9. Text and tests that change

- Rules (crates/nimitz-core/rules/): common.md:11-13 (a lead is attached
  with `open`), 24-30 (start, restart, muster, paths), 37-38 (Signals:
  "any member may message any member"); every role's "may run / never"
  list goes (admiral.md:5-15, commodore.md:5-15, ship.md:12-15, and the
  0.4.0 wording in rules.rs:35); roles stay as a description of the work
  (the admiral for the fleet, the commodore for its squadron's ships, ships
  for long work), and "plan first, show the user" stays for changes unless
  the harness rule's standing approval covers them. Bump `rules::VERSION`
  (rules.rs:25, now 8).
- Skills: plugin/skills/fleet/SKILL.md (23-28, the authority table 54-69
  goes, the task table 105-114; its description's "who may start or
  signal whom"), admiral/SKILL.md:27-31, 51-55,
  commodore/SKILL.md:23, 31, 40, 52.
- Admiralty: crates/admiralty/src/actions.rs:460-468 builds `start <id>
  [--only] [-f|-p]`; it becomes `start`, `restart`, `open` by path, and the
  `R` key runs `muster F/...`; `layout::plan` (actions.rs:406) stays.
- USAGE (main.rs:63-135); README.md:48-60 and the reference section.
- Authority and comms tests that go: sessions.rs:2807-2925 (the role
  table), hook.rs:493-780 (signal and comms decisions), config.rs:1563,
  1873-1883 (comms values), crud.rs:533-537, fleet_settings.rs:273; the
  role-table test cli.rs:388-456 (`every_command_goes_by_the_role_table`)
  becomes "every command runs for every role"; authority asserts in
  relaunch_table.rs (14 hits), cli.rs (28), restore.rs (12), Admiralty's
  ui.rs, sandbox.rs, cli_drift.rs (4-7 each), adversarial_round1.rs (5),
  formats.rs (5, comms in fixtures), e2e.rs (2).
- Tests: old-form hits to move to paths and the new verbs: about 179 in
  crates/nimitz/tests/restore.rs, 36-46 in cli.rs, 12-15 in e2e.rs, plus
  upgrade.rs and adversarial_round1.rs; the largest single cost of 0.7.3; new
  tests for path matching, `--parked`, muster, busy relaunch and the
  self-relaunch helper (the last needs real Claude Code: an e2e).
- CHANGELOG 0.7.3 "Upgrading": every old form and its replacement, since
  nothing prints a pointer.
