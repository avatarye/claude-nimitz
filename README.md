# claude-nimitz

A fleet system for Claude Code, built on the `claude` CLI only, so it works with
a Claude subscription login and with Bedrock.

- `nimitz`: the command that builds and runs fleets, squadrons and ships.
- **Admiralty**: the TUI where you watch and control every fleet.
- A Claude Code plugin: the fleet skills and the hooks that report state and
  tell a commodore at once when one of its ships waits on you.

Version 0.6.0 ([CHANGELOG](CHANGELOG.md): read "Upgrading" first). Status: in testing (Claude Code 2.1.288 - 2.1.296).

## User manual

### What it is

nimitz runs fleets of Claude Code sessions with a chain of command: a fleet
has one **admiral**, squadrons each led by a **commodore**, and **ships** that do
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
| Bring a fleet (or a squadron, a ship, everything) up and ready | `nimitz start F` · `nimitz start F/SQ` · `nimitz start F/SQ/SHIP` · `nimitz start` |
| Do the same after a Claude Code update, a Herdr closure or a reboot | `nimitz start [F]` (it tells the three apart, member by member) |
| Also wake the ships that were parked | `nimitz start F --all` |
| Restart a member (same conversation, a new process) | `nimitz start F/SQ/SHIP -f` |
| Restart one lead alone (`F` and `F/SQ` mean the whole fleet or squadron) | `nimitz start F --only -f` · `nimitz start F/SQ --only -f` |
| Open a member's pane and focus it | `nimitz start F/SQ/SHIP -p` (a lead: `nimitz start F --only -p`) |
| Open it in its own tab `F.SQ.SHIP`, never a split | `nimitz start F/SQ/SHIP -p --tab` |
| Attach to a member in this terminal | `nimitz attach F/SQ/SHIP` (a member that is down wakes first; ctrl-z detaches) |
| Start a member's conversation over (drops its history) | `nimitz start F/SQ/SHIP --fresh` |
| Stop a member | `nimitz stop F/SQ/SHIP` |
| See everything | `nimitz status`, or `admiralty` (keys below) |
| Message a member | `nimitz signal F/SQ/SHIP "text"`, or `m` in Admiralty |
| Answer a member that waits on you (`[nimitz needs-you]`) | `nimitz start <id> -p`, then answer in its pane |
| Detach from a member's pane | ctrl-z (never `/exit`: inside an attach it opens the agent view) |

What a plan looks like (`nimitz start DEMO --plan` after a Claude Code update):

```text
start DEMO  ·  Claude Code updated to 2.1.289  ·  4 to relaunch  ·  3 panes to reattach  ·  1 skipped

             job       pane       why
DEMO
  ADMIRAL    relaunch  reattach   on 2.1.288 · pane w2:p1
DEMO · OPS
  COMMODORE  relaunch  reattach   on 2.1.288 · pane w3:p1
  BUILD      relaunch  reattach   on 2.1.288 · pane w3:p2
  DOCS       skip      ·          on 2.1.288; mid-turn
  TEST       relaunch  ·          on 2.1.288

needs you
  skip    DEMO/OPS/DOCS  is mid-turn: not relaunched now (relaunch.busy)
          fix: nimitz start DEMO/OPS/DOCS -f  # once it is idle

--yes runs these 15 steps; -v shows the commands
```

Admiralty keys: `j`/`k` move, `tab` next fleet, `enter` jump to a pane, `a`
start, `A` a pane for it, `x` more actions (stop, relaunch, sweep, rm, adopt),
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

### Aides

An **aide** is an always-on member attached to a fleet's admiral (`F/@NAME`,
its session `[F]-@NAME`). It works a durable inbox nimitz keeps for it: anyone
the inbox's policy allows posts typed items, the aide claims them one at a
time, does the work and marks each one done; a crash, a reboot or a `/clear`
loses nothing. A dead letter (an item that failed for good) and a question
the aide asks reach its admiral as a `[nimitz needs-you]`.

Add one with `nimitz aide new F/@KB --dir <its repo> --types wiki.note.v1`
(the model defaults to sonnet, the mode to acceptEdits); one machine runs it,
the one its folder is on. It lands in the fleet's `fleet.toml` as:

```toml
[[aide]]
name = "KB"                      # F/@KB
model = "sonnet"                 # must be in your availableModels
permission_mode = "acceptEdits"

[aide.dirs]
home-server = "/path/to/its/repo"

[aide.inbox]
types = ["wiki.note.v1"]         # what it accepts
policy = "open"                  # open | fleet | parent
```

`nimitz start F` brings it up after the admiral (pinned, on its model);
`nimitz stop F` stops it first; `nimitz status` shows its row and its queue.

| You want to | Run |
| --- | --- |
| Add, change or remove an aide | `nimitz aide new F/@KB --dir <its repo> --types wiki.note.v1` · `nimitz aide set F/@KB --depth 200` · `nimitz aide rm F/@KB` (you or its admiral; `nimitz help` for the flags) |
| Give an aide work | `nimitz inbox post F/@KB --type wiki.note.v1 --payload '{"body": "..."}'` |
| Ask an aide and wait for the answer | `nimitz ask F/@REF "question"` (its item and token print at once; `--wait 5m`) |
| See its queue | `nimitz inbox status F/@KB` · `nimitz inbox list F/@KB` |
| Read an answer later | `nimitz inbox result F/@REF <id> [--token T]` |
| Retry or drop a dead letter | `nimitz inbox retry F/@KB <id>` · `nimitz inbox drop F/@KB <id>` |

An aide wakes four ways: a post rings it (a `[nimitz inbox]` line through
its own socket, once per burst until it drains); its session start shows its
inbox; its own 15-minute cron; and the piggyback, which every member's hooks
and `status`/`check` run at most every 30 s: it rings an idle aide whose work
has waited two minutes, returns a lapsed lease's item to the queue (or makes
it a dead letter its admiral hears of), and pushes an answer whose `ask
--wait` gave up. `check` names `inbox.dead`, `inbox.stuck` and `inbox.full`.
A read-only reader (`aide new F/@REF --dir <repo> --types wiki.query.v1
--snapshot --mode default`) works in a snapshot of its repo (a detached
worktree under NIMITZ_STATE, trusted on its own) that each `claim` moves to
the repo's last commit; its answers name that commit, and nimitz posts each
gap of an answer to the fleet's notes keeper. `aide set F/@REF --snapshot`
makes a lost snapshot again. An aide
runs in its own permission mode (default `acceptEdits`; `bypassPermissions`,
`dontAsk` and `auto` are refused: they would skip its rules) and runs no git at
all: `nimitz aide grep <term> [<path>]` and `nimitz aide status` read its
folder, and a note ends in `nimitz inbox done <aide> <item> --commit --title T
--paths A,B --pages A`, which commits exactly those paths with the item's
`inbox-item: <ref>` trailer (a retry finds that commit and makes no second).
It never edits `.git` nor its repo's own rules (CLAUDE.md, agent/ and the files
the KB names), and never pushes. `[[aide]] brief = "<file>"` (in its folder)
follows its rules at every session start.

**Update every machine first.** A nimitz before 0.5.0 that shares the fleet
folder cannot load a fleet.toml holding `[[aide]]`, and reads a registry
that holds an aide as corrupt (it moves it aside); a nimitz 0.5 refuses a
read-only reader (`snapshot = true`). Update every machine to 0.6.0 before you
add an aide, and do not go back below 0.5.0 after one ran:
a 0.4.x on the migrated registry moves it aside when it holds an aide, and
otherwise rewrites it without the `min_nimitz` stamp (an early 0.4.0 build
also without each lineage entry's `home`, which keeps two homes' bindings apart).
`nimitz doctor` names each machine of the fleet that runs too old a nimitz for
what the fleet holds now, and `aide new` refuses a fleet's first aide while a machine has not run 0.5,
and its first reader while one has not run 0.6 (`aide.machines`; `--force` is
yours alone). It sees only the machines `[machines]` lists: a
machine that reads the fleet folder without a label there is not checked. `aide new` stamps fleet.toml (`format = 2`, `min_nimitz =
"0.5.0"`, or `"0.6.0"` while it has a reader): an older nimitz is then
refused with "update nimitz here" instead of misreading the file. `aide rm` of the last aide takes the
stamp off again (format 1, readable by 0.4).

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
  step for you. It is yours alone: it lets that folder's own `.claude` settings,
  hooks and MCP servers run, so a member asks you.
- Common fixes nimitz prints:
  - a drifted name or missing labels: `nimitz start <id> -f` (a lead: `--only -f`);
  - `history.lost` (its transcript is gone): `nimitz start <id> --fresh`;
  - `tab.taken` (its tab runs something else): ctrl-z or quit it there, then `nimitz start <id> -p`;
  - copies or stale rows: `nimitz sweep <id>`;
  - a stowaway (a session in a fleet folder with no labels): `nimitz adopt <session> <id>`, or stop and `rm` it;
  - `tab.resumed` (Herdr reopened a conversation in a pane): `/exit` in that pane, then `nimitz start F`.
- After an update, the first nimitz command migrates this machine's files by
  itself (one `[OK] nimitz X: migrated ...` line; the old file kept as
  `<file>.pre-X`); `nimitz migrate --plan` lists what is due, a shared step
  included (it affects every machine: update them all first).
- A lead shows no pane, or a member shows in another's pane (Herdr's hook
  in a background worker claimed the pane the Claude Code daemon was started
  from): after upgrading to 0.5.0, run `nimitz start <F> -f` (a lead:
  `nimitz start <id> --only -f`) once for each live member. A job launched
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

The sections below are the reference: the restore details, identity,
authority and messaging.

## Documents

| Where | What |
| --- | --- |
| `CHANGELOG.md` | What each release changed; how to upgrade. |
| `docs/design/aide.md` | Aides: the durable inbox, request and reply, the build plan. |
| `docs/design/migrations.md` | Format versions and chained migrations. |
| `docs/design/plan-v3.md` | The plan: decisions, vocabulary, setup and CRUD, adapters, compatibility, phases. |
| `docs/design/cc-control-model.md` | How `nimitz` controls Claude Code sessions along the chain of command. |
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
0.4.0). You talk to a lead in its Herdr tab: `nimitz start <id> -p` (or
`nimitz start <F>`) runs `claude attach <job>` there; detach with ctrl-z. Do not
type `/exit` inside an attach: it opens the agent view of the whole machine
(`nimitz stop <id>` is how a member stops). A lead's job is pinned
(`~/.claude/jobs/pins.json`), so the daemon never idle-retires it; a ship parks
after an idle hour and `nimitz start` wakes it by its exact session id, never
`-c`. A member whose transcript is gone is `history.lost`: `nimitz start <id>
--fresh` is the cold launch that starts over. A 0.3.x lead that still runs
interactive is ended with `/exit` in its own terminal, then `nimitz start`
brings the same conversation back as a job.

After a Claude Code update, a Herdr closure or crash, or a reboot, the same
verb brings the fleets back: `nimitz start [<F>]` (no `F`: every fleet on this
machine), `--plan` first, then `--yes`. It tells the three apart per member,
and the plan says, member by member, what it found and why it acts (`worker on
2.1.294, installed 2.1.295 -> relaunch`, `its pane is gone -> reattach`,
`parked -> wake`):

- Claude Code updated: every live member whose worker runs an older Claude
  Code than the installed one (read from its process) is relaunched, same id
  and history, then its pane gets the new job's attach. Ships first, then
  commodores, then admirals, one at a time; never one mid-turn or waiting on
  you (run start again once it is idle), and never the member that runs the
  command (a fleet-wide start skips its caller with a `start.self` line that
  names the command you run afterwards from your own shell). An attach client
  older than the installed version is reported (ctrl-z there, then start
  again).
- Herdr closed or crashed: the jobs are alive, only the panes are gone; their
  tabs are made again (or their shell panes reused) and attached. Nothing is
  relaunched.
- Reboot: the leads (pinned) and the ships that were live at the last look
  (`NIMITZ_STATE/active.json`) are woken by their exact session id, then
  attached; `--all` also wakes parked ships (never a first launch).
- `-f` (`--relaunch`) relaunches every live member whatever its version (same
  order, same rules). A member that had a pane, lead or ship, gets that pane
  attached again after the relaunch.

`nimitz start [<target>] [-f] [-p [--tab]] [--fresh] [--all] [--only] [--jobs-only]` is the one
verb that brings a member up and ready. The target is a ship `F/SQ/SHIP`, a
squadron `F/SQ` (its commodore and its ships), a fleet `F`, or nothing (every
fleet on this machine). `nimitz stop` is its opposite. With no flags it is
idempotent and does per member what is needed: a member that is down, parked
or lost in a reboot wakes by its exact id; a live member on an older Claude
Code is relaunched (same id and history); a live, current member is left as
it is. A member that never ran gets a first launch only when it is the
target's own member (`F` names the admiral, `F/SQ` the commodore, `F/SQ/SHIP`
the ship), so a fleet- or squadron-wide start never cold-launches the others.
The leads always get their tab attached. A member that is busy, waiting on
you, or the one running the command is left alone, with a fix line.

- `-p` (`--pane`) makes sure the target's member has a pane and focuses it. It
  tries, in order: a pane already showing the member; its own tab
  (`F.SQ.SHIP`); a split in its commodore's tab beside the commodore's pane;
  a new tab in the squadron's workspace `F.SQ` (made if needed). A lead gets
  its own tab: `F.ADMIRAL` in workspace `F`, `F.SQ.COMMODORE` in workspace
  `F.SQ`. `-p` is yours alone, since it opens a pane in your terminal;
  members get an authority refusal. With no multiplexer, `start` brings up
  the jobs only and `-p` is refused.
- `-p --tab` always puts the member in its own tab, named as above (`F.SQ.SHIP`
  in workspace `F.SQ`, a lead's own tab, an aide's `F.@NAME` in `F`), then
  attaches and focuses it. It skips the shown pane and the split: its tab is
  reused when it is there (its shell pane claimed; a tab that runs something
  else is `tab.taken`, as always) and created when it is not. A pane that
  shows the member somewhere else is left as it is, never typed into; the plan
  notes it (`tab.elsewhere`). `--tab` without `-p` is a usage error.
- `--only` acts on the target's own member alone: `F` and `F/SQ` name a whole
  fleet or squadron, so `nimitz start F --only -f` relaunches the admiral and
  nothing else. Every fix for a lead that nimitz prints uses it.
- `--fresh` starts a new conversation and drops the history (after the
  confirm).
- `--force` is separate and unchanged: a cold launch over a broken lineage or
  an untested Claude Code. Never use it unless you name it.

`nimitz attach <id>` attaches to a member in the terminal you run it from: it
becomes `claude attach <job>` with the member's exact job (its short id, never
a name: `claude attach` matches names by part, so `DEV` could be `NIMITZ_DEV`
or `MYCC_DEV`). A member that is parked or stopped is woken first, by start's
own plan: the plan shows, and `--yes` (or the question) runs it; a member
that never ran gets its first launch there (and its folder's trust, when due),
as `start` would; a live member attaches at once. ctrl-z detaches; ← opens the agent view. It is yours alone,
and refused from inside a Claude Code session (an attach in an attach).

Start reads what Herdr has (`herdr api snapshot`, each pane's foreground),
reuses a tab found by the session it shows or by its full name (`F.ADMIRAL`,
`F.SQ.COMMODORE`, `F.SQ.SHIP`), creates only what is missing, types `claude
attach <job>` only into a pane that is a plain shell, never makes a second
tab or a copy, and names every member it cannot bring up with one fix. Run
twice, the second run does nothing. Admiralty: `R` on a fleet, and `a`, `r`,
`A` or enter on a member row with no pane, run `nimitz start` (with `-f` or
`-p` as fits).

Every planning command prints the same plan (start, stop, sweep, rm, adopt,
and the ship, squadron and fleet changes). First a headline: what happened,
then the counts per action. Then one line per member, grouped by fleet and
squadron, in columns: the name, the job verb (`wake`, `relaunch`, `launch`,
`fresh`, `skip`, `refuse`, `nothing`), the pane verb (`reattach`, `open pane`,
`focus`) and why. Then "needs you": the skips, refusals and warnings, each
with one fix. It ends with the prompt `Run N steps? [y/N]`. The raw `claude`
and `herdr` command lines are hidden; `-v` shows them, with each member's full
detail. `--json` is unchanged. Colour shows only on a terminal and `NO_COLOR`
turns it off; the plain text reads the same, because the verbs carry the
meaning. The run prints the same lines with `✓`, `✗` or `–` and one result
line.

Deprecated for one release, each keeps its old behaviour and prints a
one-line pointer: `restore [<F>]` (use `start [<F>]`; `--restart` is `-f`),
`relaunch <id>` (`start <id> -f`), `open <id>` (`start <id> -p`; a lead adds
`--only` to both), `up <F>`
(`start <F> --all`, and `start <id>` for a first launch) and `layout herdr
[<F>]` (`start [<F>]`).

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

Who may run what (`nimitz` reads the caller from the session's labels; no
labels is you):

| Caller | May run |
| --- | --- |
| you | everything |
| admiral | every command inside its own fleet, on any member but itself: start (`-f`, `--all`), stop, rm, sweep, adopt, fleet set, squadron new/set/rm, ship new/set/rm; not `start -p` or `attach` (yours alone) |
| commodore | inside its own squadron: start (`start F/SQ` covers its ships), stop, rm, sweep, adopt for its ships, not `start -p` or `attach`; squadron set; ship new/set/rm |
| ship | the read-only commands (`status`, `check`, `rules`, `doctor`); `signal` to a member the chain allows; `sweep` of itself (its own stale rows) |

`init`, `fleet new`, `fleet rm`, `machine add|rm` and `migrate` stay yours.
Nobody relaunches itself: `start <self> --only -f` is refused with the fix; a
wider start skips its caller. More
authority skips nothing: every change is still `--plan` first, then `--yes`.
Every launch allows `Bash(nimitz *)` in the member's settings; the table above
is what nimitz then lets each role do.

Who may signal whom (`SendMessage`; the plugin's hook blocks the rest):

| From | May signal |
| --- | --- |
| admiral | every member of its own fleet: commodores and ships |
| commodore | its admiral, its ships |
| ship | its commodore; its admiral (a reply); other ships as the squadron's `comms` rule allows |

The rules each role gets at session start are compiled into `nimitz` and
versioned; a new `nimitz` hands a running member the changes at its next
prompt (`nimitz rules updated vN→vM`), with no restart.

Note: "Claude" is an Anthropic trademark. This name is for private use; check
Anthropic's trademark guidelines before any public release.
