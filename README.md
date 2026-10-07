# claude-nimitz

A fleet system for Claude Code, built on the `claude` CLI only, so it works with
a Claude subscription login and with Bedrock.

- `nimitz`: the command that builds and runs fleets, squadrons and ships.
- **Admiralty**: the TUI where you watch and control every fleet.
- A Claude Code plugin: the fleet skills and the hooks that report state and
  tell a commodore at once when one of its ships waits on you.

Status: in testing (Claude Code 2.1.288 - 2.1.289).

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
0.4.0). You talk to a lead in its Herdr tab: `nimitz open <id>` (or `nimitz
layout herdr <F>`) runs `claude attach <job>` there; detach with ctrl-z. Do not
type `/exit` inside an attach: it opens the agent view of the whole machine
(`nimitz stop <id>` is how a member stops). A lead's job is pinned
(`~/.claude/jobs/pins.json`), so the daemon never idle-retires it; a ship parks
after an idle hour and `nimitz start` wakes it by its exact session id, never
`-c`. A member whose transcript is gone is `history.lost`: `nimitz start <id>
--fresh` is the cold launch that starts over. A 0.3.x lead that still runs
interactive is ended with `/exit` in its own terminal, then `nimitz start`
brings the same conversation back as a job.

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
| admiral | every command inside its own fleet, on any member but itself: start, stop, relaunch, rm, sweep, adopt, up, layout herdr, fleet set, squadron new/set/rm, ship new/set/rm |
| commodore | inside its own squadron: start, stop, relaunch, rm, sweep, adopt for its ships; squadron set; ship new/set/rm |
| ship | the read-only commands (`status`, `check`, `rules`, `doctor`); `signal` to a member the chain allows; `sweep` of itself (its own stale rows) |

`init`, `fleet new`, `fleet rm`, `machine add|rm` and `migrate` stay yours. More
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
