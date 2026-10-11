---
name: fleet
description: Run claude-nimitz fleets with the nimitz command. Use when the user asks to create, change or delete a fleet, squadron, ship or machine; to start, stop, relaunch or remove a session; to bring a fleet up or back after an update, a Herdr closure or a reboot; to sweep copy rows; to adopt a stowaway; to signal a member; or asks who may start or signal whom.
---

# Fleet: run fleets with `nimitz`

`nimitz` holds every rule. This skill tells you which command to run. Do not
edit fleet files by hand. Do not run `claude --bg`, `claude stop` or
`claude rm` yourself for a fleet member.

## Words

| Word | Meaning | Id | Name (address) |
| --- | --- | --- | --- |
| admiral | Commands one fleet. A background job, attached in its Herdr tab. | `F` | `[F]-ADMIRAL` |
| commodore | Commands one squadron. A background job, attached in its Herdr tab. | `F/SQ` | `[F.SQ]-COMMODORE` |
| ship | Long-term work in one folder. A background job that parks and wakes on demand. | `F/SQ/SHIP` | `[F.SQ]-SHIP` |
| aide | Always on, attached to the admiral, with a durable inbox (`nimitz inbox post <aide>`, `nimitz ask <aide>`): a notes keeper (`@KB`) or a read-only reference desk (`@REF`). | `F/@NAME` | `[F]-@NAME` |

One-off work is a subagent in the lead's own session, or a ship. Every member
is a Claude Code background job (`claude --bg`); a lead's job is pinned, so it
is never idle-retired. You talk to a lead in its tab: `nimitz start <id> -p` runs
`claude attach <job>` there (`-p --tab`: always in its own tab), and `nimitz
attach <id>` attaches the user's current terminal (both the user's alone);
detach with ctrl-z. Never type `/exit` inside an
attach: it opens the agent view of the whole machine instead of ending the
member (`nimitz stop <id>` stops one).

## Rules

1. Look first. Run `nimitz status` (or `nimitz status --json`) before you act.
2. Plan first. Every command that changes something takes `--plan`. Run it
   with `--plan`, show the plan to the user, and run it with `--yes` only
   after the user agrees.
3. Never use `--force` unless the user asks for it by name.
4. The config is the truth. CRUD commands change only the config. Running
   sessions change only through `start`, `stop`, `rm`, `sweep` and
   `adopt`.
5. Start the target's own member. `nimitz start F` names the admiral,
   `F/SQ` the commodore, `F/SQ/SHIP` the ship; a fleet- or squadron-wide start
   never cold-launches the others. Do not start the parents yourself.
6. A refusal is an answer. When `nimitz` prints `[ERROR]` with a `fix:`, show
   the fix to the user. Do not work around the refusal.
7. Never resume, stop or remove a session that is not in the fleet.
8. Recovery is by exact id. `nimitz start` wakes a stopped or parked member
   with `claude --bg --resume <its session>`, never `-c`. A member whose
   transcript is gone is `history.lost`: nothing starts until you run
   `nimitz start <id> --fresh` (a cold launch, a new lineage entry). A lead
   that still runs as a 0.3.x interactive session is ended with `/exit` in
   its own terminal, then `nimitz start <id>` brings the same conversation
   back as a job.

## Who may start what

| Caller | May start |
| --- | --- |
| the user (no labels) | everything |
| admiral | every command inside its own fleet: `start` (`-f`, `--all`; also for the whole fleet), `stop`, `rm`, `sweep`, `adopt` for any member but itself; `fleet set`, `squadron new/set/rm`, `ship new/set/rm` in its fleet |
| commodore | inside its own squadron: `start` (`start F/SQ` covers its ships), `stop`, `rm`, `sweep`, `adopt` for its ships; `squadron set` for its squadron; `ship new/set/rm` for its ships |
| ship | the read-only commands: `status`, `check`, `rules`, `doctor` (no `--fix`); `signal` to a member it may signal (below); `sweep` of itself (its own stale rows) |

`nimitz` reads the caller from the session's labels. It refuses what the table
does not allow. Every member may run `nimitz` (its launch allows `Bash(nimitz *)`). Always the user's alone: `init`, `fleet new`, `fleet rm`,
`machine add`, `machine rm`, `migrate`, and any session named only by its short
id that is no member's (a stowaway outside the admiral's fleet, a stray). A
commodore never removes its own squadron; nobody stops or relaunches itself
(`start <self> --only -f` is refused with the fix; a fleet- or squadron-wide start, `-f` or not, skips its caller with a
`start.self` line naming the command the user runs from their own shell). `start -p` opens a
pane in the user's terminal: it is the user's alone, and members get an authority refusal.
More authority does not skip the plan: `--plan` first, `--yes` only after the
user agrees, `--force` only when the user names it.

## Who may signal whom

Sessions signal with `SendMessage` to a name. The plugin's hook blocks a signal
that the chain does not allow.

| From | May signal |
| --- | --- |
| admiral | every member of its own fleet, commodores and ships (questions and orders) |
| commodore | its admiral, its ships |
| ship | its commodore; its admiral (a reply); other ships as the squadron's `comms` rule allows |

`comms` in `squadron.toml`: `open` (default), `commodore-only`, or an allow list.
The user reaches a ship through its commodore: `nimitz signal F/SQ/SHIP <text>`
types the text into the commodore's pane.

## Commands

| Task | Command |
| --- | --- |
| First run on a machine | `nimitz init --label <label>`, then `nimitz doctor` |
| Check the setup | `nimitz doctor` (`--fix` sets `crossSessionInbound` and replaces role briefs an older nimitz wrote) |
| A role's rules | `nimitz rules <id>` (what every member gets at session start; a new nimitz hands a running member the changes at its next prompt) |
| See everything | `nimitz status` |
| Find problems | `nimitz check [<id>]` |
| New fleet | `nimitz fleet new <F> [--machine <label>]` |
| New squadron | `nimitz squadron new <F/SQ> --brief "<text>" [--comms open\|commodore-only\|allow:F/SQ/SHIP,...]` |
| New ship | `nimitz ship new <F/SQ/SHIP> --dir <folder\|managed> [--prompt "<text>"] [--mode <permission mode>] [--depends A,B]` (no prompt flag on a terminal: `$EDITOR` opens) |
| Change a ship | `nimitz ship set <F/SQ/SHIP> [same flags]` (takes effect at its next relaunch) |
| Retire a ship | `nimitz ship rm <F/SQ/SHIP>` (stops it, removes job rows, keeps transcripts) |
| Delete a squadron or fleet | `nimitz squadron rm <F/SQ>`, `nimitz fleet rm <F>` (only when empty and not live) |
| Add or remove a machine | `nimitz machine add <F> <label> [--dir F/SQ/SHIP=<folder>]`, `nimitz machine rm <F> <label>` |
| Bring a member, squadron or fleet up and ready | `nimitz start [<target>]`: a ship `F/SQ/SHIP`, a squadron `F/SQ` (commodore and ships), a fleet `F`, or nothing (every fleet here). `nimitz stop` is its opposite. No flags: idempotent, per member what is needed. Down, parked or after a reboot: woken by its exact id. Live on an older Claude Code: relaunched (same id and history). Live and current: nothing. Never run: a first launch, but only for the target's own member. Leads always get their tab attached. A member that is busy, waiting on you or the caller is left alone with a fix line |
| Bring fleets back after a Claude Code update, a Herdr closure or crash, a reboot | `nimitz start [<F>]` (no `F`: every fleet here). `--all` also wakes parked ships (never a first launch); `--jobs-only` over ssh, no tabs. Never a second tab, never a copy |
| Relaunch live, current members too | `nimitz start <id> -f` (`--relaunch`; a member that had a pane gets it reattached). `--force` is separate: a cold launch over a broken lineage or an untested Claude Code, only when the user names it |
| Start one member | `nimitz start <id>` (wakes it by its exact id; `--fresh` is a new conversation that drops the history, after `history.lost`). A lead alone: `nimitz start F --only` (F and F/SQ otherwise mean the whole fleet or squadron); `--only -f` relaunches just that lead |
| Stop one member | `nimitz stop <id>`; one copy of two: `nimitz stop <short id>` |
| Remove stale rows and copies of a member | `nimitz sweep [<id>]` (a commodore: `nimitz sweep <F/SQ>`), or one row: `nimitz rm <short id>`. A background copy is stopped and removed; a copy with no job (interactive, SDK) is ended in its own terminal |
| A session with no labels (a stowaway) | `nimitz adopt <session> <id>`, or `nimitz rm <session>`; a live one with no job: end it in its terminal |
| Show a member (the user's alone) | `nimitz start <id> -p`: a pane already showing it; else its own tab (`F.SQ.SHIP`); else a split in its commodore's tab; else a new tab in workspace `F.SQ`. A lead: its own tab (`F.ADMIRAL` in workspace `F`, `F.SQ.COMMODORE` in `F.SQ`). Detach with ctrl-z. No multiplexer: `start` brings up the jobs only and `-p` is refused |
| Old names (deprecated for one release; each prints a pointer) | `restore [<F>]` is `start [<F>]` (`--restart` is `-f`); `relaunch <id>` is `start <id> -f`; `open <id>` is `start <id> -p` (a lead: `start <id> --only -f|-p`); `up <F>` is `start <F> --all` (and `start <id>` for a first launch); `layout herdr [<F>]` is `start [<F>]` |
| Read a plan | A headline (what happened, then counts per action); one line per member in columns (name, job verb `wake`/`relaunch`/`launch`/`fresh`/`skip`/`refuse`/`nothing`, pane verb `reattach`/`open pane`/`focus`, why); "needs you" (skips, refusals, warnings, each with one fix); then `Run N steps? [y/N]`. Same for every planning command. `-v` shows the raw `claude` and `herdr` lines; `--json` is unchanged; `NO_COLOR` turns colour off |
| Start, resume or stop a member with `claude` | never: a resume of a live job is a copy; `claude --resume` holds the conversation with no job; no labels is a stowaway; a job started from a shell takes the daemon's env. Use `nimitz start` |
| Move a fleet-skill fleet | `nimitz migrate fleet-skill <old dir>... --into <F>` (plan), then add `--yes` |

## Status words

`not-started`, `stopped`, `parked`, `idle`, `busy`, `needs-you`, `unknown`.
`reasons` says why. `parked` is a job with no process that nobody stopped
(the daemon retired it, it crashed, or the machine went down): not an error,
`nimitz start` wakes it (a crashed job gets the daemon about 20 s to restart
it first). `unknown` means the evidence does not decide; do not guess.

## Issues to know

`held`: a process with no job runs the member's conversation (an SDK host, a
by-hand `claude --resume`, a 0.3.x interactive lead); end it there, then
`nimitz start`. `lead.intruder`: a live session in a lead's folder that is not
the lead's job; the fix names it (stop then rm a job; end a process in its
terminal). `history.lost`: see rule 8. `name.drift`: a `/rename`; `nimitz
start <id> -f` restores the name.
