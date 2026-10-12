---
name: fleet
description: Run claude-nimitz fleets with the nimitz command. Use when the user asks to create, change or delete a fleet, squadron, ship or machine; to start, stop, relaunch or remove a session; to bring a fleet up or back after an update, a Herdr closure or a reboot; to sweep copy rows; to adopt a stowaway; to signal a member; to open or close a member's pane.
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
| kb-scribe, kb-ref | The plugin's subagents that write (a note, one commit) and read (a question, a cited answer) the fleet's knowledge base, in any member's session. | `nimitz:kb-scribe` | `nimitz:kb-ref` |

A path names one member (`F`, `F/SQ`, `F/SQ/SHIP`), or everything under one
(`F/...`, `F/SQ/...`; `//...` is every fleet). Several paths per command.
`attach`, `adopt`'s target, the config verbs, `--for` and `--fleet` take one
exact name and refuse a `...`; `signal` takes one path (a `...`: its live members).

One-off work is a subagent in the lead's own session, or a ship. Every member
is a Claude Code background job (`claude --bg`); a lead's job is pinned, so it
is never idle-retired. You talk to a member in its pane: `nimitz open <id>`
runs `claude attach <job>` there (`--tab`: always in its own tab), and
`nimitz attach <id>` attaches the user's current terminal; detach with
ctrl-z. Never type `/exit` inside an attach: it opens the agent view of the
whole machine instead of ending the member (`nimitz stop <id>` stops one).

## Rules

1. Look first. Run `nimitz status` (or `nimitz status --json`) before you act.
2. Plan first. Every command that changes something takes `--plan`. Run it
   with `--plan`, show the plan to the user, and run it with `--yes` only
   after the user agrees.
3. Never use `--force` unless the user asks for it by name.
4. Four families. Fleet shape (`fleet`, `squadron`, `ship`, `machine`,
   `settings`, `harness`) changes only the config. Session control
   (`start`, `restart`, `stop`, `rm`, `sweep`, `adopt`, `signal`) changes
   jobs, never panes. Pane control (`open`, `attach`, `close`, `layout`)
   changes panes; `open` wakes first. `muster` does both, as one plan.
5. An exact path is a member alone: `nimitz start F` is the admiral, `F/...`
   the whole fleet. Only a member named exactly gets a first launch; a `...`
   start never cold-launches. Do not start the parents yourself.
6. A refusal is an answer. When `nimitz` prints `[ERROR]` with a `fix:`, show
   the fix to the user. Do not work around the refusal.
7. Never resume, stop or remove a session that is not in the fleet.
8. Recovery is by exact id. `nimitz start` wakes a stopped or parked member
   with `claude --bg --resume <its session>`, never `-c`. A member whose
   transcript is gone is `history.lost`: nothing starts until you run
   `nimitz start <id> --fresh` (a cold launch, a new lineage entry).
9. Every member may run every command on any member, and any member may
   message any member (`SendMessage` to its name, or `nimitz signal`). A
   command that stops, relaunches or removes the member that runs it runs
   that step last: a detached helper waits for the turn to end (up to 10
   minutes), and the outcome comes with the member's next session start.

## Commands

| Task | Command |
| --- | --- |
| First run on a machine | `nimitz init --label <label>`, then `nimitz doctor` |
| Check the setup | `nimitz doctor` (`--fix` sets `crossSessionInbound` and replaces role briefs an older nimitz wrote) |
| A role's rules | `nimitz rules <path>...` (what every member gets at session start; a new nimitz hands a running member the changes at its next prompt) |
| See everything | `nimitz status [<path>...]` |
| Find problems | `nimitz check [<path>...]` (a lead's check covers everything under it) |
| A command's own usage | `nimitz <command> --help`, `nimitz help <command>` |
| New fleet | `nimitz fleet new <F> [--machine <label>]` |
| New squadron | `nimitz squadron new <F/SQ> --brief "<text>" [--mode <permission mode>]` |
| New ship | `nimitz ship new <F/SQ/SHIP> --dir <folder\|managed> [--prompt "<text>"] [--mode <permission mode>] [--depends A,B]` (no prompt flag on a terminal: `$EDITOR` opens) |
| Change a ship | `nimitz ship set <F/SQ/SHIP> [same flags]` (takes effect at its next relaunch) |
| Retire a ship | `nimitz ship rm <F/SQ/SHIP>` (stops it, removes job rows, keeps transcripts) |
| Delete a squadron or fleet | `nimitz squadron rm <F/SQ>`, `nimitz fleet rm <F>` (only when empty and not live) |
| Add or remove a machine | `nimitz machine add <F> <label> [--dir F/SQ/SHIP=<folder>]`, `nimitz machine rm <F> <label>` |
| A fleet's settings | `nimitz settings get [<key>] --fleet <F>`, `nimitz settings set <key> <value> --fleet <F>` |
| Bring members up | `nimitz start <path>...`: the jobs only, no pane. Down or parked: woken by its exact id; live on an older Claude Code: relaunched (same id and history); live and current: nothing; never run and named exactly: a first launch. `--parked` also wakes the parked and stopped ships a `...` covers; `--fresh` a named member's new conversation |
| Relaunch live members too | `nimitz restart <path>...` (same id and history; a pane its attach showed is reattached; a turn in flight is dropped, and the plan says so). `--force` is separate: a cold launch over a broken lineage or an untested Claude Code, only when the user names it |
| After a reboot, a Claude Code update, a Herdr closure or crash | `nimitz muster [<path>...]` (no path: every fleet here): workers on an older Claude Code relaunched, down leads and the ships live at the last look woken (`--parked`: the rest), every pane reattached in its own pane, else its lead's tab. One plan. No multiplexer: the jobs, the pane steps skipped |
| Stop members | `nimitz stop <path>...` (under a `...`, the live ones, ships first); one copy of two: `nimitz stop <short id>` |
| Remove stale rows and copies | `nimitz sweep [<path>...]` (none: a member's own, else everything), or one row: `nimitz rm <short id>`. A background copy is stopped and removed; a copy with no job (interactive, SDK) is ended in its own terminal |
| A session with no labels (a stowaway) | `nimitz adopt <session> <F/SQ/SHIP>` (one exact path), or `nimitz rm <session>`; a live one with no job: end it in its terminal |
| Message a member | `SendMessage` to its name; the user's `nimitz signal <path> <text>` types into its tab (its own input), `--inbox` (and any member's signal) posts to its inbox as a request. One path: a member, or `F/...`/`F/SQ/...` for every live one under it (never `//...`). A member named exactly that is down is refused, never woken |
| Show a member | `nimitz open <path>...`: a pane already showing it; else its own tab (`F.SQ.SHIP`); else a split in its commodore's tab; else a new tab. A lead: its own tab (`F.ADMIRAL`, `F.SQ.COMMODORE`). `--tab`: always its own tab; `--force`: a second tab beside a taken one. One that is down is woken first |
| Close a member's panes | `nimitz close <path>...` (only a pane that runs its attach; the job runs on) |
| Lay out the tabs only | `nimitz layout <path>... [--ships] [--here] [--rename] [--force]` |
| Read a plan | A headline (what happened, then counts per action); one line per member in columns (name, job verb `wake`/`relaunch`/`launch`/`fresh`/`skip`/`refuse`/`nothing`, pane verb `reattach`/`open pane`/`focus`, why); "needs you" (skips, refusals, warnings, each with one fix); then `Run N steps? [y/N]`. `-v` shows the raw `claude` and `herdr` lines; `--json` is unchanged; `NO_COLOR` turns colour off |
| Start, resume or stop a member with `claude` | never: a resume of a live job is a copy; `claude --resume` holds the conversation with no job; no labels is a stowaway; a job started from a shell takes the daemon's env. Use `nimitz start` |
| Move a fleet-skill fleet | `nimitz migrate fleet-skill <old dir>... --into <F>` (plan), then add `--yes` |

## Status words

`not-started`, `stopped`, `down`, `parked`, `idle`, `busy`, `needs-you`,
`unknown`. `reasons` says why. `down`: the machine took it down while it was
active (a `...` start or `muster` wakes it). `parked`: the daemon retired it,
or it was not active; `--parked` wakes it. Neither is an error. `unknown`
means the evidence does not decide; do not guess.

## Issues to know

`held`: a process with no job runs the member's conversation (an SDK host, a
by-hand `claude --resume`, a 0.3.x interactive lead); end it there, then
`nimitz start`. `lead.intruder`: a live session in a lead's folder that is not
the lead's job; the fix names it (stop then rm a job; end a process in its
terminal). `history.lost`: see rule 8. `name.drift`: a `/rename`; `nimitz
restart <id>` restores the name.
