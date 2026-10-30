# Using nimitz: the detail

The [README](../README.md) gets you started. This page is the detail behind
it: paths, how the commands group, what `muster` does, who runs what,
Admiralty's keys, fixes for common problems, and notes for maintainers. Each
command's full usage is `nimitz help <command>`; the design is in
[docs/design/](design/).

## Paths

A path names one member, in every verb: `F` the admiral, `F/SQ` the
commodore, `F/SQ/SHIP` a ship. `F/...` and `F/SQ/...` name everything under
one, the lead included; `//...` names every fleet (never ADMIRALTY; `/...`
works too, as MSYS turns `//` into `/`). A command takes several paths; a
member matched twice counts once; a path that matches nothing is refused. No
`*`.

## The four command families

Fleet shape (`fleet`, `squadron`, `ship`, `machine`, `settings`, `harness`)
changes config only. Session control (`start`, `restart`, `stop`, `rm`,
`sweep`, `adopt`, `signal`) changes jobs and never panes. Pane control
(`open`, `attach`, `close`, `layout`) changes panes only; with no multiplexer,
`open`, `close` and `layout` refuse (`mux.none`). `muster` does both as one
plan. Design: [docs/design/cli-ux.md](design/cli-ux.md).

Every command that changes something shows its plan first: `--plan` only
looks, `--yes` runs without the question, `-v` shows the commands.

## Muster

`muster [<path>...] [--parked]` (no path: `//...`; `--here`, `--rename`,
`--force` as for `layout`) is the one command after a reboot, a Claude Code
update or a Herdr closure. It tells the three apart, member by member:

- Claude Code updated: a worker on an older version is relaunched (same id
  and history), then reattached. `restart //...` relaunches every one instead.
- Herdr closed or crashed: the jobs are alive; their tabs are made again and
  attached.
- Reboot: the leads and the ships live at the last look are woken by their
  exact session id (`--parked`: the rest too), then attached.

With no multiplexer it runs the job steps and skips the pane steps. Run twice,
the second run does nothing. What a plan looks like:

```text
muster DEMO/...  ·  Claude Code updated to 2.1.289  ·  5 to relaunch  ·  3 panes to reattach

             job       pane       why
DEMO
  ADMIRAL    relaunch  reattach   on 2.1.288 · pane w2:p1
DEMO · OPS
  COMMODORE  relaunch  reattach   on 2.1.288 · pane w3:p1
  BUILD      relaunch  reattach   on 2.1.288 · pane w3:p2
  DOCS       relaunch  ·          on 2.1.288; busy: its turn in flight is dropped
  TEST       relaunch  ·          on 2.1.288

--yes runs these 15 steps; -v shows the commands
```

## A member that ends itself

A command that would stop, relaunch or remove the member that runs it runs
that step last: a detached `nimitz __self-end` waits for the member's turn to
end (up to 10 minutes), runs it, logs to `NIMITZ_STATE/relaunch-self.log` and
hands the outcome to the member's next session start.

## Two rules

- Never start, resume or stop a member with `claude` yourself (`--resume`,
  `--bg`, `-c`, `stop`, `rm`): it makes copies and stowaways nimitz cannot
  manage. `nimitz start` does it right.
- Plan, then run: read the plan (`--plan`), then run it (`--yes`). Every
  member may run every command; the leads run routine fleet management
  themselves and wait for your OK only for `fleet rm`, `squadron rm`,
  `machine rm`, a folder's trust, `migrate --shared`, `--force`, `--fresh`
  (or anything else that drops a conversation) and `settings set kb.*`.

## Notes on single commands

- `status`: a live member's line ends in its topic, the one line Claude Code
  keeps on what the job is doing (`-- Inspecting ...`, cut to a terminal's
  width, never in a pipe; `topic` in the JSON). `down`: taken down while
  active (a `...` start wakes it); `parked`: retired by the daemon or not
  active (`--parked` wakes it).
- `check F` and `check F/...` are the same: a check never hides a ship's
  errors.
- `ship --effort low|medium|high|xhigh|max` is passed as `claude --effort` on
  every launch and relaunch (a wake keeps it); `''` clears it. A live ship
  takes a new one at `nimitz restart <id>`. A squadron.toml that sets an
  effort needs nimitz 0.7.12 on every machine: an older one refuses that
  squadron (`config.too_new`) and every config change to its fleet; the plan
  that sets the first effort warns (`format.raise`).
- `signal`: yours (a plain terminal) is typed into the member's tab as your
  own input; a member's, a Claude Code session's, or one with `--from`, goes
  to its inbox naming the verified sender. `--from` (non-members only) is a
  claim, shown as `says it is "<label>"`. One path, or `F/...` / `F/SQ/...`
  for every live member under it but the sender (never `//...`), all or
  nothing behind one gate.
- `harness`: rules, agents, skills and settings every member of one fleet
  gets ([docs/design/harness.md](design/harness.md)).
- `kb`: the fleet's knowledge base, a git repo every machine clones; members
  use it through the `nimitz:kb-scribe` and `nimitz:kb-ref` subagents
  ([docs/design/kb.md](design/kb.md)).
- Removed in 0.7.3: `restore` (use `muster`), `relaunch` (`restart`), `up`
  (`start F/... --parked`), `layout herdr` (`layout`), `fleet settings`
  (`settings`), and `start -f`, `-p`, `--tab`, `--only`, `--all`,
  `--jobs-only`.

## The provider a machine runs on

`nimitz machine set <label> --provider anthropic|bedrock|vertex` names the API
provider every member on that machine must run on, and `--claude-bin PATH` the
claude a cold launch runs (`NIMITZ_CLAUDE_BIN` still wins; `''` clears either;
the settings live in the shared `machines.toml`).

- For bedrock or vertex, a member's `--settings` env gets the selector
  (`CLAUDE_CODE_USE_BEDROCK=1`, `CLAUDE_CODE_USE_VERTEX=1`): it beats the
  shell's env and the folder's local settings, and the job keeps it for every
  wake. For anthropic nothing is added.
- Before a cold launch (and a relaunch), nimitz asks Claude Code itself which
  provider that exact launch would use (`claude --settings <the launch's>
  auth status --json`, in the member's folder) and refuses a mismatch before
  anything starts (`provider.mismatch`). An answer it cannot read refuses too
  (`provider.unconfirmed`) where a provider is named or a member runs on
  Bedrock; on an optional machine it is a warning and the launch goes ahead.
  A machine with no provider named may run Bedrock members only once it names
  one (`provider.unset`).
- After the launch, the member's SessionStart record (what its worker really
  runs on) is checked again: a provider its selectors give that differs,
  which only managed settings can cause, stops and removes the job (its first
  prompt may have gone out); a model that disagrees (a custom
  `ANTHROPIC_MODEL`, an alias) is a warning (`provider.model`).
- `status` and `check` warn about a live member whose record differs (the
  fix, `stop` then `start --fresh`, drops its conversation: ask first).
  `doctor` shows the provider and lists the claude binaries it finds.

## Colour

Plans and the usage are coloured on a terminal only. `NO_COLOR` (any value)
or `TERM=dumb` turns colour off; `NIMITZ_COLOR=1` (or `always`) forces it in a
pipe or a capture, and `NO_COLOR` still wins.

## Admiralty

Keys: `j`/`k` move, `tab` next fleet, `space` find a member in every fleet
(fuzzy, as fzf: `wwa` finds `WORK/WEB/API`, `web busy` the busy ones of WEB;
`enter` puts the cursor on it, `ctrl-o` jumps to its pane; `!` and `i` the
same; it lists only rows with a pane and leaves the topic out until `ctrl-a`
and `ctrl-t`; `ctrl-s` hides stowaways, `ctrl-f` keeps this fleet, `ctrl-l`
the ships), `z` fold / unfold, `enter` jump to a pane, `a` start, `A` a pane for
it, `x` more actions with the row's detail below them (stop, restart, sweep,
rm, adopt; on a fleet: `R` muster), `u` up a fleet with its parked ships, `m`
message, `!` what waits on you, `i` every issue with its fix, `c`/`e`/`D`
create, edit, remove, `?` all keys (`?` again closes them), `q` quit (in the
`?` panel).

The two status rows at the bottom show the fleets' counts and `? keys`, then
`Nimitz: v0.7.15  Claude Code: v2.1.312 (supported)  Machine: laptop` (a
mismatch or an out-of-range Claude Code as a warning) and the local date and
time at the right, after `stale 12s` when no refresh has landed for three
intervals. Every row reads the same way: NAME says who (a member, a crew's
agent type or its own name, a task's `#id` and owner), DETAIL what it is
doing (a member's topic when nothing else needs saying), STATUS in the member
glyphs (a crew is busy, idle, done or failed). NAME, STATUS and AGE keep fixed
widths; DETAIL takes the rest of a wide or fullscreen terminal.

## When something is wrong

- `nimitz doctor` for the install, `nimitz check [<path>...]` for a fleet:
  every problem comes with its one fix line.
- Common fixes nimitz prints:
  - a drifted name or missing labels: `nimitz restart <id>`;
  - `history.lost` (its transcript is gone): `nimitz start <id> --fresh`;
  - `folder.untrusted`: `nimitz trust <folder>` (`ship new` and `start` plan
    it for you);
  - `tab.taken` (its tab runs something else): ctrl-z or quit it there, then
    `nimitz open <id>`;
  - copies or stale rows: `nimitz sweep <id>`;
  - `session.unreadable` (a start answers `started a copy ... cannot parse
    the session file ~/.claude/sessions/<pid>.json`): a live session's status
    file is broken, and Claude Code then resumes nothing on the machine.
    Remove what follows its first JSON object (the fix names the byte), or
    restart the session that owns it. A dead process's broken file is
    harmless (`nimitz doctor` lists it as info);
  - a stowaway (a session in a fleet folder with no labels): `nimitz adopt
    <session> <id>`, or stop and `rm` it;
  - `tab.resumed` (Herdr reopened a conversation in a pane): `/exit` in that
    pane, then `nimitz start <id>`.
- Never `/exit` inside an attach: it opens the agent view; ctrl-z detaches.
- Doctor says Claude Code is above the tested range: launches still go ahead;
  `tools/cc-validate` validates the new version (see below).

## For maintainers

- A change to a file nimitz owns: bump its version in `formats::ARTIFACTS`,
  add the step and a fixture, then `NIMITZ_BLESS=1 cargo test --test formats`
  ([docs/design/migrations.md](design/migrations.md)).
- Unit suite: `cargo test --workspace`. Real Claude Code e2e (opt-in, cheap
  model, every test fleet under `~/.cache/nimitz-e2e`):
  `NIMITZ_E2E=1 NIMITZ_E2E_LIVE_FLEETS=HOME,WORK cargo test -p nimitz --test e2e -- --test-threads=3`.
- After a Claude Code update, `tools/cc-validate` (`--plan` first) validates
  it in one command, in a fresh worktree from dev: build, unit suite, full
  e2e, a hook-payload probe and a diff of the Claude Code internals nimitz
  relies on, with the live fleets checked before and after. On a pass it
  bumps the tested range, commits, merges into dev, **pushes origin dev** and
  installs; on a fail it changes nothing and writes
  `~/.cache/nimitz-ci/<version>/report.md`. Exit 0 pass, 1 failed, 2 checkout
  not ready. Its own tests: `python3 tools/test_cc_validate.py`.

## Documents

| Where | What |
| --- | --- |
| `CHANGELOG.md` | What each release changed; how to upgrade. |
| `docs/design/cli-ux.md` | 0.7.3: paths, the four command families, muster, no authority checks. |
| `docs/design/cc-control-model.md` | How `nimitz` controls Claude Code sessions: jobs, identity, pins, parking. |
| `docs/design/harness.md` | A fleet's harness: rules, agents, skills, settings. |
| `docs/design/kb.md` | The knowledge base and its two subagents. |
| `docs/design/migrations.md` | Format versions of every file nimitz owns, the chained migrations, the cross-machine guard. |
| `docs/design/rules-size.md` | The role rules and how they reach a member. |
| `docs/design/plan-v3.md` | The plan: decisions, vocabulary, setup and CRUD, adapters, compatibility, phases. |
| `docs/design/aide.md` | Aides (0.5 to 0.6; removed in 0.7.0). |
| `docs/probes/` | P0 probes: requirements, findings per Claude Code build. |
