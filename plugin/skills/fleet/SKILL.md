---
name: fleet
description: Run claude-nimitz fleets with the nimitz command. Use when the user asks to create, change or delete a fleet, squadron, ship or machine; to start, stop, relaunch or remove a session; to bring a fleet up; to sweep copy rows; to adopt a stowaway; to signal a member; or asks who may start or signal whom.
---

# Fleet: run fleets with `nimitz`

`nimitz` holds every rule. This skill tells you which command to run. Do not
edit fleet files by hand. Do not run `claude --bg`, `claude stop` or
`claude rm` yourself for a fleet member.

## Words

| Word | Meaning | Id | Name (address) |
| --- | --- | --- | --- |
| admiral | Commands one fleet. Interactive. | `F` | `[F]-ADMIRAL` |
| commodore | Commands one squadron. Interactive. | `F/SQ` | `[F.SQ]-COMMODORE` |
| ship | Long-term work in one folder. Background. | `F/SQ/SHIP` | `[F.SQ]-SHIP` |

One-off work is a subagent in the lead's own session, or a ship.

## Rules

1. Look first. Run `nimitz status` (or `nimitz status --json`) before you act.
2. Plan first. Every command that changes something takes `--plan`. Run it
   with `--plan`, show the plan to the user, and run it with `--yes` only
   after the user agrees.
3. Never use `--force` unless the user asks for it by name.
4. The config is the truth. CRUD commands change only the config. Running
   sessions change only through `up`, `start`, `stop`, `relaunch`, `rm`,
   `sweep` and `adopt`.
5. Start top-down. `nimitz start <id>` starts the admiral, then the commodore,
   then the target. Do not start the parents yourself.
6. A refusal is an answer. When `nimitz` prints `[ERROR]` with a `fix:`, show
   the fix to the user. Do not work around the refusal.
7. Never resume, stop or remove a session that is not in the fleet.

## Who may start what

| Caller | May start |
| --- | --- |
| the user (no labels) | everything |
| admiral | every command inside its own fleet: `start`, `stop`, `relaunch`, `rm`, `sweep`, `adopt` for any member but itself; `up` and `layout herdr` for its fleet; `fleet set`, `squadron new/set/rm`, `ship new/set/rm` in its fleet |
| commodore | inside its own squadron: `start`, `stop`, `relaunch`, `rm`, `sweep`, `adopt` for its ships; `squadron set` for its squadron; `ship new/set/rm` for its ships |
| ship | the read-only commands: `status`, `check`, `rules`, `doctor` (no `--fix`); `signal` to a member it may signal (below); `sweep` of itself (its own stale rows) |

`nimitz` reads the caller from the session's labels. It refuses what the table
does not allow. Every member may run `nimitz` (its launch allows `Bash(nimitz *)`). Always the user's alone: `init`, `fleet new`, `fleet rm`,
`machine add`, `machine rm`, `migrate`, and any session named only by its short
id that is no member's (a stowaway outside the admiral's fleet, a stray). A
commodore never removes its own squadron; nobody stops or relaunches itself.
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
| Bring a fleet up | `nimitz up <F>` (needs Herdr) |
| Start one member | `nimitz start <id>` (admiral and commodore open in a Herdr tab) |
| Stop one member | `nimitz stop <id>`; one copy of two: `nimitz stop <short id>` |
| New session id, labels restored | `nimitz relaunch <id>` |
| Remove stale rows | `nimitz sweep [<id>]` (a commodore: `nimitz sweep <F/SQ>`), or one row: `nimitz rm <short id>` |
| A session with no labels | `nimitz adopt <session> <id>`, or `nimitz rm <session>` |
| Show a member | `nimitz open <id>` |
| Herdr workspaces for fleets | `nimitz layout herdr [<F>]` |
| Move a fleet-skill fleet | `nimitz migrate fleet-skill <old dir>... --into <F>` (plan), then add `--yes` |

## Status words

`not-started`, `stopped`, `idle`, `busy`, `needs-you`, `unknown`. `reasons`
says why. `unknown` means the evidence does not decide; do not guess.
