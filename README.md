# claude-nimitz

A fleet system for Claude Code, built on the `claude` CLI only, so it works with
a Claude subscription login and with Bedrock.

- `nimitz`: the command that builds and runs fleets, squadrons and ships.
- **Admiralty**: the TUI where you watch and control every fleet.
- A Claude Code plugin: the fleet skills and the hooks that report state and
  tell a commodore at once when one of its ships waits on you.

Version 0.7.3 ([CHANGELOG](CHANGELOG.md): read "Upgrading" first). Status: in testing (Claude Code 2.1.288 - 2.1.296).

## User manual

### What it is

nimitz runs fleets of Claude Code sessions: a fleet has one **admiral**, squadrons each led by a **commodore**, and **ships** that do
the long work in one folder. Every member is a Claude Code background job; you
look at one in a Herdr pane and detach with ctrl-z.

### Install and first setup

Needs Rust (cargo) and Claude Code; Herdr for panes (status, config and ships
work without it).

```sh
git clone -b dev https://github.com/avatarye/claude-nimitz && cd claude-nimitz
cargo install --path crates/nimitz       # nimitz: the CLI and the hook binary
cargo install --path crates/admiralty    # admiralty: the TUI
claude plugin marketplace add "$PWD" && claude plugin install nimitz@claude-nimitz
nimitz init --label <this-machine>       # data root and machine label
nimitz doctor                            # run again until: [OK] doctor: ready
```

`nimitz init` also trusts `~/.claude-nimitz` for Claude Code (no trust prompt to
accept by hand). In Herdr: `herdr plugin link <repo>/herdr-plugin` and bind a key
to `admiralty.toggle` to open Admiralty as an overlay. To try it all without
touching your setup: `nimitz sandbox create /tmp/nz --herdr && eval "$(nimitz sandbox env /tmp/nz)"`.

### Daily use

Every command that changes something shows its plan first: add `--plan` to
only look, `--yes` to run without the question.

| You want to | Run |
| --- | --- |
| Bring a fleet (or a squadron, a ship, everything) up | `nimitz start F/...` · `nimitz start F/SQ/...` · `nimitz start F/SQ/SHIP` · `nimitz start //...` (`F` alone is the admiral) |
| Bring everything back after a Claude Code update, a Herdr closure or a reboot | `nimitz muster` (it tells the three apart, member by member) |
| Also wake the ships that were parked | `nimitz start F/... --parked` · `nimitz muster --parked` |
| Restart members (same conversation, a new process) | `nimitz restart F/SQ/SHIP` · `nimitz restart F` (the admiral alone) · `nimitz restart //...` |
| Open a member's pane and focus it | `nimitz open F/SQ/SHIP` |
| Open it in its own tab `F.SQ.SHIP`, never a split | `nimitz open F/SQ/SHIP --tab` |
| Close the panes that show members | `nimitz close F/SQ/...` (the jobs run on) |
| Lay out the leads' tabs | `nimitz layout F/...` (`--ships`: the live ships too) |
| Attach to a member in this terminal | `nimitz attach F/SQ/SHIP` (a member that is down wakes first; ctrl-z detaches) |
| Start a member's conversation over (drops its history) | `nimitz start F/SQ/SHIP --fresh` |
| Stop members | `nimitz stop F/SQ/SHIP` · `nimitz stop F/...` |
| See everything | `nimitz status [<path>...]`, or `admiralty` (keys below) |
| Message a member | `nimitz signal F/SQ/SHIP "text"` (typed into its tab; `--inbox`: its inbox), or `m` in Admiralty |
| Answer a member that waits on you (`[nimitz needs-you]`) | `nimitz open <id>`, then answer in its pane |
| A command's own usage | `nimitz <command> --help`, `nimitz help <command>` |
| Detach from a member's pane | ctrl-z (never `/exit`: inside an attach it opens the agent view) |

What a plan looks like (`nimitz muster DEMO/... --plan` after a Claude Code update):

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

Admiralty keys: `j`/`k` move, `tab` next fleet, `enter` jump to a pane, `a`
start, `A` a pane for it, `x` more actions (stop, restart, sweep, rm, adopt; on
a fleet: `R` muster, `u` up with the parked ships),
`m` message, `!` what waits on you, `i` every issue with its fix, `c`/`e`/`D`
create, edit, remove, `?` all keys, `q` quit.

### Fleets, squadrons and ships

| You want to | Run |
| --- | --- |
| A new fleet on this machine | `nimitz fleet new F --machine <label>` |
| A new squadron | `nimitz squadron new F/SQ --brief "what it is for"` |
| A new ship | `nimitz ship new F/SQ/SHIP --dir <folder>\|managed --prompt "what it does"` |
| Change a ship | `nimitz ship set F/SQ/SHIP --prompt "..." --mode <permission mode>` |
| Remove a ship, a squadron, a fleet | `nimitz ship rm F/SQ/SHIP` · `nimitz squadron rm F/SQ` · `nimitz fleet rm F` |
| Run a fleet on another machine too | `nimitz machine add F <label> --hostname <host>` |

### The knowledge base

A fleet's knowledge base is a git repo every machine of the fleet clones.
Its place is in the fleet's settings (`<NIMITZ_HOME>/<fleet>/settings.json`,
beside fleet.toml but with its own format, so machines need not update in
lockstep):

| You want to | Run |
| --- | --- |
| Name the fleet's KB | `nimitz settings set kb.remote <git url> --fleet F` (`--plan`, then `--yes`) |
| Say where it is on a machine | `nimitz settings set kb.dirs.<machine label> <folder> --fleet F` |
| Clone it on this machine | `nimitz kb clone --fleet F --yes` |
| Read the settings | `nimitz settings get [kb.remote] --fleet F` |
| Push and pull by hand | `nimitz kb sync --fleet F` |

`nimitz kb path|status|head|show|grep|hash|commit|restore|sync` is the KB's
git, run by nimitz (no hooks, literal paths, one commit per note under a
lock, then a sync). Every member writes and reads it through the plugin's
subagents `nimitz:kb-scribe` (a note, one commit) and `nimitz:kb-ref` (a
question, a cited answer), held to the KB by the kb-guard hook;
docs/design/kb.md.

### A fleet's harness

What every member of one fleet gets beyond nimitz's role rules lives in
`<NIMITZ_HOME>/<fleet>/harness/` (docs/design/harness.md):

| You want to | Run |
| --- | --- |
| A rule every member follows | `nimitz harness add rule style.md --fleet F` (`--plan`, then `--yes`): live at each member's next prompt |
| A subagent or a skill for the fleet | `nimitz harness add agent reviewer.md --fleet F` · `nimitz harness add skill ./deploy --fleet F`: at the next launch (`fleet-f:reviewer`), or `/reload-plugins` |
| Settings for the fleet, a squadron or one ship | `nimitz harness add settings perms.json --for F/SQ --fleet F`: at the next launch |
| See it, take something out | `nimitz harness list --fleet F` · `nimitz harness rm rule style --fleet F` |
| Bring the running members up to date | `nimitz harness push --fleet F`, then `nimitz restart F/... --plan` |

The fleet's admiral may change the harness too; a harness never sets hooks or
other settings that run programs.

### Aides (gone in 0.7.0)

nimitz 0.5 and 0.6 had aides (`F/@NAME`): always-on members with a durable
inbox. 0.7.0 removes them. To upgrade a fleet that has one: stop each aide
with 0.6 first (`nimitz stop F/@NAME`; one left running is `aide.retired` in
`nimitz check`, stopped by its short id), install 0.7.0 on every machine, then
run `nimitz migrate --plan` and `nimitz migrate --shared --yes` once. That
drops the `[[aide]]` tables and their format stamp from fleet.toml, so every
nimitz reads it again. Each machine's first 0.7 command moves its aides'
queues, snapshots and notices to `NIMITZ_STATE/archive/aides-0.6/` (nothing
is deleted) and drops their records from the registry. Until then a fleet
with `[[aide]]` still loads, and `check` warns (`aide.legacy`).

### Two rules

- Never start, resume or stop a member with `claude` yourself: a resume of a
  live job makes a copy, `claude --resume` holds the conversation where nimitz
  cannot wake it, a session without nimitz's labels is a stowaway, and a job
  started from a shell takes the wrong environment. `nimitz start` does it right.
- Plan first: read the plan (`--plan`), then run it (`--yes`). Never `--force`
  unless you mean it.

### When something is wrong

- `nimitz doctor`: the install (hooks, plugin, trust, Claude Code version).
- `nimitz check [<id>]`: the fleet's problems, each with its one fix line.
- A folder Claude Code does not trust (`folder.untrusted`): `nimitz trust <folder>`
  writes Claude Code's trust for its git root (or the folder outside git), as
  the trust prompt would. `ship new`, `ship set --dir` and `start` plan it as a
  step for you. It lets that folder's own `.claude` settings, hooks and MCP
  servers run, which every plan that trusts says.
- Common fixes nimitz prints:
  - a drifted name or missing labels: `nimitz restart <id>`;
  - `history.lost` (its transcript is gone): `nimitz start <id> --fresh`;
  - `tab.taken` (its tab runs something else): ctrl-z or quit it there, then `nimitz open <id>`;
  - copies or stale rows: `nimitz sweep <id>`;
  - a stowaway (a session in a fleet folder with no labels): `nimitz adopt <session> <id>`, or stop and `rm` it;
  - `tab.resumed` (Herdr reopened a conversation in a pane): `/exit` in that pane, then `nimitz start <id>`.
- After an update, the first nimitz command migrates this machine's files by
  itself (one `[OK] nimitz X: migrated ...` line; the old file kept as
  `<file>.pre-X`); `nimitz migrate --plan` lists what is due, a shared step
  included (it affects every machine: update them all first).
- A lead shows no pane, or a member shows in another's pane (Herdr's hook
  in a background worker claimed the pane the Claude Code daemon was started
  from): after upgrading to 0.5.0, run `nimitz restart <F>/...` once. A job launched
  before 0.5.0 keeps the daemon's `HERDR_PANE_ID` until a relaunch gives it
  the new settings; a plain wake reuses its old ones.
- `-v` on any plan shows the commands it runs.
- `nimitz doctor` says Claude Code is above the tested range ("tested up to
  ..."): launches still go ahead. To validate the new version and raise the
  range, run `tools/cc-validate` in the repo (see For maintainers).

### For maintainers

- A change to what a file nimitz owns holds (docs/design/migrations.md): bump
  its version in `formats::ARTIFACTS`, add the step from the old version (a
  pure bytes-to-bytes function; shared files only run on `migrate --shared`),
  add a fixture `crates/nimitz-core/tests/fixtures/formats/<artifact>/vN.*` as
  the old version wrote it, then `NIMITZ_BLESS=1 cargo test --test formats`.
- Unit suite: `cargo test --workspace`. Real Claude Code e2e (opt-in, cheap
  model, every test fleet under `~/.cache/nimitz-e2e`):
  `NIMITZ_E2E=1 NIMITZ_E2E_LIVE_FLEETS=HOME,MAIN cargo test -p nimitz --test e2e -- --test-threads=3`.
- After a Claude Code update, `tools/cc-validate` (stdlib Python; you run it, as
  you, on this machine) does the whole validation in one command:
  - `--plan` shows what it will do; `--force` validates a version already tested.
  - Installed `claude --version` equal to `docs/compat.toml`'s `max`: "already
    tested", exit 0.
  - The main checkout must be clean, on dev and level with origin/dev, or it
    stops with the fix (exit 2) and runs nothing.
  - It snapshots the live fleets (`nimitz status --json`), then in a fresh
    worktree from dev (`~/.cache/nimitz-ci/wt-<version>`): build, unit suite,
    the full e2e, a hook-payload probe (two throwaway background jobs) with
    doctor's pinned-format check, and a static diff of the Claude Code
    internals nimitz relies on (retire line, pins lock, copy text, peer frame,
    the agents re-exec, the messaging socket) against the previous tested
    binary in `~/.local/share/claude/versions/`. Then the live fleets again: any
    change to a member's session or pid fails the run.
  - On a pass: bumps `max`, the README range and the pinned tests; commits;
    merges into dev (`--no-ff`); **pushes origin dev**; installs nimitz and
    admiralty; updates the plugin if `plugin/` changed; checks `nimitz doctor`.
  - On a fail: changes nothing and writes `~/.cache/nimitz-ci/<version>/report.md`
    (failing stages and tests; the logs, `internals.diff`, `payloads.jsonl` and
    the guard snapshots next to it).
  - Each stage prints one line with ✓ or ✗ and its time. Exit 0: a pass, or
    already tested; 1: a stage failed (or another run holds the lock); 2: the
    checkout is not ready; 130: interrupted (cleaned up, nothing merged).
  - The guard explains one change by itself: the daemon retiring an idle
    member (same session, no process). A member you restart during the run
    fails it; run it again.
  - It installs where `nimitz` already is (`~/.local/bin` or `~/.cargo/bin`).
  - Its own tests (fakes; no git, cargo or claude): `python3 tools/test_cc_validate.py`.
  - A unit test that fails for a reason outside nimitz on this machine is
    listed in `KNOWN_FAILURES` at the top of the script.

The sections below are the reference: paths and verbs, muster, identity and
messaging.

## Documents

| Where | What |
| --- | --- |
| `CHANGELOG.md` | What each release changed; how to upgrade. |
| `docs/design/aide.md` | Aides (0.5 to 0.6; superseded in 0.7.0): the durable inbox, request and reply. |
| `docs/design/migrations.md` | Format versions and chained migrations. |
| `docs/design/plan-v3.md` | The plan: decisions, vocabulary, setup and CRUD, adapters, compatibility, phases. |
| `docs/design/cc-control-model.md` | How `nimitz` controls Claude Code sessions. |
| `docs/design/cli-ux.md` | 0.7.3: paths, the four command families, muster, no authority checks. |
| `docs/design/migrations.md` | Format versions of every file nimitz owns, the chained migrations, the cross-machine guard. |
| `docs/design/plan-v2.md` | Previous plan (kept for history). |
| `docs/design/review-identity-and-status.md` | How Claude Code, the old fleet skill and the old watch identify agents; the gaps. |
| `docs/probes/` | P0 probes: requirements, findings per Claude Code build, comparison. |
| `probes/p0_probe.py` | The probe script. |

Vocabulary: a **fleet** has one **admiral**; a fleet has **squadrons**, each
commanded by a **commodore**; a squadron has **ships**; sessions carry **crew**
(sub-agents). Admiralty is the office where the admirals work. (The tender
role of 0.3.x, one job for a commodore, was removed in 0.4.0: one-off work is
a subagent in the lead's session, or a ship.)

Every member, leads too, is a Claude Code **background job** (`claude --bg`,
0.4.0). You talk to a member in its Herdr pane: `nimitz open <id>` runs
`claude attach <job>` there; detach with ctrl-z. Do not type `/exit` inside an
attach: it opens the agent view of the whole machine (`nimitz stop <id>` is
how a member stops). A lead's job is pinned (`~/.claude/jobs/pins.json`), so
the daemon never idle-retires it; a ship parks after an idle hour and
`nimitz start` wakes it by its exact session id, never `-c`. A member whose
transcript is gone is `history.lost`: `nimitz start <id> --fresh` is the cold
launch that starts over. A 0.3.x lead that still runs interactive is ended
with `/exit` in its own terminal, then `nimitz start` brings the same
conversation back as a job.

**Paths** (0.7.3). A path names one member: `F` the admiral, `F/SQ` the
commodore, `F/SQ/SHIP` a ship, in every verb. `F/...` and `F/SQ/...` name
everything under one, the lead included; `//...` names every fleet (never the
ADMIRALTY workspace; MSYS turns a leading `//` into `/`, so `/...` works too).
A command takes several paths; a member matched twice counts once; a path that
matches nothing is refused, with what is there. No `*`.

**Four families.** Fleet shape: `fleet`, `squadron`, `ship` new|set|rm,
`machine add|rm`, `settings get|set`, `harness`: the config only. Session
control: `start`, `restart`, `stop`, `rm`, `sweep`, `adopt`, `signal`: jobs,
never panes. Pane control: `open`, `attach`, `close`, `layout`: panes (`open`
and `attach` wake a member that is down first). And `muster`, both as one plan.

- `nimitz start <path>... [--fresh] [--parked] [--force]` wakes what is down
  by its exact id, top-down, and relaunches a worker on an older Claude Code
  (same id and history). A member a path names exactly gets a first launch
  when it never ran; a `...` never cold-launches. `--parked` also wakes the
  parked and stopped ships a `...` covers. `--fresh` starts a named member's
  new conversation (after the confirm). `--force` is a cold launch over a
  broken lineage or an untested Claude Code: never unless you name it. It
  makes no pane; a pane whose attach its relaunch ends gets the new job's
  attach.
- `nimitz restart <path>...` relaunches every live member it covers, current
  or not (same id and history), ships first, then commodores, then admirals; a
  turn in flight is dropped, and the plan says so (`busy: its turn in flight is
  dropped`, `waiting: its question is dropped`).
- `nimitz stop <path|session>...` stops members (under a `...` the live ones,
  ships first) or one session by its id; `rm` removes job rows, never
  transcripts; `sweep [<path>...]` drops stale rows and copies.
- `nimitz open <id>... [--tab] [--force]` gives a member a pane and focuses
  it: a pane already showing it; its own tab (`F.SQ.SHIP`); a split in its
  commodore's tab; a new tab in workspace `F.SQ`. A lead gets its own tab
  (`F.ADMIRAL` in `F`, `F.SQ.COMMODORE` in `F.SQ`). `--tab` always uses its own
  tab, reused or created, and leaves a pane that shows it elsewhere as it is
  (`tab.elsewhere`); `--force` opens a second tab beside a taken one.
- `nimitz close <path>...` closes the panes that show those members, only a
  pane whose foreground is the member's `claude attach <job>`; the jobs run on.
- `nimitz layout <path>... [--ships] [--here] [--rename] [--force]` lays out
  the Herdr tabs only.
- With no multiplexer `start` and `restart` need none; `open`, `close` and
  `layout` refuse (`mux.none`); `muster` runs its job steps and says the pane
  steps were skipped.

**Muster.** After a Claude Code update, a Herdr closure or crash, or a reboot,
one command brings the fleets back: `nimitz muster [<path>...]` (no path:
`//...`), `--plan` first, then `--yes`. It tells the three apart per member,
and the plan says what it found and why it acts (`on 2.1.294 -> relaunch`,
`its pane is gone -> reattach`, `parked -> wake`):

- Claude Code updated: a live member whose worker runs an older Claude Code
  than the installed one is relaunched, same id and history, then its pane
  gets the new job's attach; ships first, then commodores, then admirals.
  `nimitz restart //...` relaunches every member instead.
- Herdr closed or crashed: the jobs are alive, only the panes are gone; their
  tabs are made again (or their shell panes reused) and attached.
- Reboot: the leads (pinned) and the ships that were live at the last look
  (`NIMITZ_STATE/active.json`) are woken by their exact session id, then
  attached; `--parked` also wakes the rest (never a first launch).
- `status` tells them apart: `down` (taken down while active: a `...` start
  or muster wakes it) and `parked` (the daemon retired it, or it was not
  active).

A command that would stop, relaunch or remove the member that runs it (its own
`restart`, `stop`, a muster after an update, `ship rm` of itself) runs that step
last: the rest runs, the report is printed, and a detached `nimitz __self-end`
waits for the member's turn to end (up to 10 minutes, then it gives up and
says so), runs the step, logs to `NIMITZ_STATE/relaunch-self.log` and hands the
outcome to the member's next session start.

`nimitz attach <id>` attaches to a member in the terminal you run it from: it
becomes `claude attach <job>` with the member's exact job (its short id, never
a name: `claude attach` matches names by part). A member that is down is woken
first, by start's own plan (`--plan`, `--yes`); a live member attaches at
once. ctrl-z detaches; ← opens the agent view. It is refused from inside a
Claude Code session (an attach in an attach).

Muster, open and layout read what Herdr has (`herdr api snapshot`, each pane's
foreground), reuse a tab found by the session it shows or by its full name,
create only what is missing, type `claude attach <job>` only into a pane that
is a plain shell, never make a second tab or a copy, and name every member they
cannot bring up with one fix. Run twice, the second run does nothing.
Admiralty: `R` on a fleet runs `nimitz muster F/...`; `a`, `r`, `A` or enter on
a member row with no pane run `nimitz start`, `restart` or `open`.

Every planning command prints the same plan. First a headline: what happened,
then the counts per action. Then one line per member, grouped by fleet and
squadron, in columns: the name, the job verb (`wake`, `relaunch`, `launch`,
`fresh`, `skip`, `refuse`, `nothing`), the pane verb (`reattach`, `open pane`,
`focus`) and why. Then "needs you": the skips, refusals and warnings, each
with one fix. It ends with the prompt `Run N steps? [y/N]`. The raw `claude`
and `herdr` command lines are hidden; `-v` shows them, with each member's full
detail. `--json` is unchanged. Colour shows only on a terminal and `NO_COLOR`
turns it off. The run prints the same lines with `✓`, `✗` or `–` and one
result line.

Removed in 0.7.3, with no pointer: `restore` (use `muster`), `relaunch`
(`restart`), `up` (`start F/... --parked`), `layout herdr` (`layout`), `fleet
settings` (`settings`), and `start -f`, `-p`, `--tab`, `--only`, `--all`,
`--jobs-only` (`restart`, `open`, `open --tab`, an exact path, `--parked`, and
`start` itself, which makes no pane).

Never start, resume or stop a member with `claude` yourself (`claude
--resume`, `claude --bg`, `claude -c`, `claude stop`, `claude rm`): a resume of
a live job starts a copy under the member's name; an interactive resume holds
the conversation in a process with no job, which nimitz cannot wake until you
`/exit` there; a session started without nimitz's labels is a stowaway; and a
job started from your shell takes the daemon's environment instead of the
member's labels. `nimitz start` does it right.

Identity (0.4.0): a member is anchored to its Claude Code **job** (the short
id you see in `claude agents`), which is stable across stop, kill, respawn,
idle retire, a native resume, `/clear`, `/branch`, a crash restart and an
upgrade. Only the **session id** (the conversation) moves; the registry keeps
the conversation now (`session`) and every one the member had (`lineage`). The
SessionStart hook binds job and session under the registry lock; `status`
joins job rows with live sessions by job id, shows `parked` for a job the
daemon retired or that crashed (not an error; `nimitz start` wakes it) and
`pinned` for a job in `jobs/pins.json`.

Who may run what: every member may run every command on any member or fleet
(0.7.3); nimitz refuses nothing on authority. A member's launch allows
`Bash(nimitz *)`, and what stands before a change is its plan: `--plan` first,
the user agrees, then `--yes`. The caller's labels still say who runs a command
(its own end goes last, its kb notes name it, `kb`, `settings` and `harness`
default to its fleet); labels that are partial or disagree get one `[WARN]`.

Messages: any member may message any member (`SendMessage` to its name; the
hook logs it and judges nothing). `nimitz signal <id> <text>` from you types
into the member's tab, your own input; `--inbox`, and any member's signal,
post to its inbox as a request that names the sender. A member that is down is
refused, never woken by a signal.

The rules each role gets at session start are compiled into `nimitz` and
versioned; a new `nimitz` hands a running member the changes at its next
prompt (`nimitz rules updated vN→vM`), with no restart.

Note: "Claude" is an Anthropic trademark. This name is for private use; check
Anthropic's trademark guidelines before any public release.
