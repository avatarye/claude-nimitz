# claude-nimitz

A fleet system for Claude Code, built on the `claude` CLI only, so it works with
a Claude subscription login and with Bedrock.

- `nimitz`: the command that builds and runs fleets, squadrons and ships.
- **Admiralty**: the TUI where you watch and control every fleet.
- A Claude Code plugin: the fleet skills and the hooks that report state and
  tell a commodore at once when one of its ships waits on you.

Status: in testing (Claude Code 2.1.288 - 2.1.294).

## Install for testing

Needs Rust (cargo) and Claude Code. Herdr is needed to run admirals and
commodores; status, config and ships work without it.

```sh
git clone -b dev https://github.com/avatarye/claude-nimitz && cd claude-nimitz
cargo build --release
cargo install --path crates/nimitz       # nimitz: the CLI and the hook binary
cargo install --path crates/admiralty    # admiralty: the TUI
claude plugin marketplace add "$PWD" && claude plugin install nimitz@claude-nimitz
nimitz init --label <this-machine>       # data root and machine label
nimitz doctor                            # hooks, trust, Claude Code versions; --fix sets crossSessionInbound
```

`nimitz init` asks you to run `claude` once in the data root
(`~/.claude-nimitz`) to accept the trust prompt. Run `nimitz doctor` again until
it says `[OK] doctor: ready`. To try nimitz without touching your setup, use a
sandbox: `nimitz sandbox create /tmp/nz --herdr && eval "$(nimitz sandbox env /tmp/nz)"`.

## Documents

| Where | What |
| --- | --- |
| `docs/design/plan-v3.md` | The plan: decisions, vocabulary, setup and CRUD, adapters, compatibility, phases. |
| `docs/design/cc-control-model.md` | How `nimitz` controls Claude Code sessions along the chain of command. |
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

`nimitz start [<target>] [-f] [-p] [--fresh] [--all] [--only] [--jobs-only]` is the one
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
- `--only` acts on the target's own member alone: `F` and `F/SQ` name a whole
  fleet or squadron, so `nimitz start F --only -f` relaunches the admiral and
  nothing else. Every fix for a lead that nimitz prints uses it.
- `--fresh` starts a new conversation and drops the history (after the
  confirm).
- `--force` is separate and unchanged: a cold launch over a broken lineage or
  an untested Claude Code. Never use it unless you name it.

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
| admiral | every command inside its own fleet, on any member but itself: start (`-f`, `--all`), stop, rm, sweep, adopt, fleet set, squadron new/set/rm, ship new/set/rm; not `start -p` (yours alone) |
| commodore | inside its own squadron: start (`start F/SQ` covers its ships), stop, rm, sweep, adopt for its ships, not `start -p`; squadron set; ship new/set/rm |
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
