# How nimitz controls Claude Code sessions along the chain of command

Date: 2026-10-03. Status: draft for review. Sources: the Claude Code docs
(code.claude.com, read 2026-10-02) and the P0 probes on the ACME build 2.1.288
(`docs/probes/findings-acme-2.1.288.md`). A fact marked **(unverified)** needs a
probe before we build on it.

## 0. Context

Claude Code manages sessions in its own way. It chooses session ids, it can
rename sessions, and it keeps records after a session stops. A fleet needs the
opposite: fixed roles, fixed identities, and a clear chain of command.

This document shows how `nimitz` uses only the `claude` command to get that
control. Section 2 maps each role to a kind of Claude Code session. Sections 3
to 6 show how `nimitz` starts, finds, commands and repairs each one.

## 1. Words

**Roles (the chain of command)**

| Word | Meaning |
| --- | --- |
| **admiral** | Commands one fleet. Asks commodores. Starts nothing. |
| **commodore** | Commands one squadron. Revives one ship when it needs it; starts tenders. |
| **ship** | Does the squadron's long-term work in one folder. |
| **tender** | Does one job for a commodore, then stops. |
| **crew** | A sub-agent or teammate inside one session. |
| **you** | The only one who starts an admiral or a commodore. |

**Claude Code terms**

| Word | Meaning |
| --- | --- |
| **interactive session** | A session in a terminal that you type into. |
| **background session** | A session that the Claude Code daemon keeps running with no terminal (`claude --bg`). |
| **attach** | Show a background session in a terminal. It stays a background session. |
| **session id** | Claude Code's id for one conversation. Claude Code chooses it. |
| **job row** | Claude Code's record of a background session. It stays after the session stops. |
| **session file** | `~/.claude/sessions/<pid>.json`. It exists while the process runs. |
| **name** | The session's address for messages (`--name`). Claude Code can change it. |
| **labels** | Env values that `nimitz` gives a session at start: `NIMITZ_FLEET`, `NIMITZ_SQUADRON`, `NIMITZ_ROLE`, `NIMITZ_ID`, `NIMITZ_PARENT`. |
| **registry** | `nimitz`'s own record of each identity and its session ids, per machine. |

## 2. Each role and its kind of session

| Role | Kind of session | Folder | Name (address) | Started by |
| --- | --- | --- | --- | --- |
| admiral | interactive, foreground | `~/.claude-nimitz/<fleet>/admiral/` | `[FLEET]-ADMIRAL` | you: `nimitz admiral <fleet>` |
| commodore | interactive, foreground | `~/.claude-nimitz/<fleet>/squadrons/<SQ>/` | `[FLEET.SQ]-COMMODORE` | you: `nimitz commodore <fleet> <SQ>` |
| ship | background | its project folder, or `ships/<SQ>/<SHIP>/` | `[FLEET.SQ]-SHIP` | you or its commodore: `nimitz launch` |
| tender | background | the folder of its job | `[FLEET.SQ]-COMMODORE/<task>` | its commodore: `nimitz tender` |
| crew | inside a session | the session's folder | none | the session (Agent tool, teammates) |

Why each kind:

1. **Admiral and commodore are interactive.** You talk to them, and only an
   interactive session can lead agent teams (docs: agent-teams).
2. **Ships and tenders are background sessions.** They run when no terminal is
   open. You can attach to one; when you detach, it stays a background session
   (docs: agent-view, "Detaching never stops a background session").
3. **An admiral or commodore must not go to the background.** In an interactive
   session, `←` on an empty prompt sends the session to the background (docs:
   agent-view). `nimitz` refuses to start one any other way, and Admiralty marks
   one that became a background session as an error.

## 3. Start: how `nimitz` gives each session its identity

### 3.1 What `nimitz` passes to `claude`

**Ship (cold start):**

```
cd <ship folder> && claude --bg \
  --name "[FLEET.SQ]-SHIP" \
  --permission-mode <from squadron.toml> \
  --settings '{"env":{"NIMITZ_FLEET":"F","NIMITZ_SQUADRON":"SQ","NIMITZ_ROLE":"ship",
               "NIMITZ_ID":"F/SQ/SHIP"},"worktree":{"bgIsolation":"none"}}' \
  "<launch prompt from squadron.toml>"
```

**Commodore:**

```
cd ~/.claude-nimitz/<fleet>/squadrons/<SQ> && claude -c \
  --name "[FLEET.SQ]-COMMODORE" \
  --settings '{"env":{"NIMITZ_FLEET":"F","NIMITZ_SQUADRON":"SQ","NIMITZ_ROLE":"commodore",
               "NIMITZ_ID":"F/SQ"}}'
```

The admiral is the same, with `NIMITZ_ROLE` `admiral`. A tender is a ship with
`NIMITZ_ROLE` `tender` and `NIMITZ_PARENT` set to its commodore.

### 3.2 How the label becomes a binding

`claude --bg` ignores `--session-id` (probe 1). So `nimitz` cannot choose the
session id. Instead:

1. `nimitz` starts the session with its labels.
2. The session starts. Claude Code runs the plugin's **SessionStart** hook.
3. The hook gets the `session_id` from Claude Code and the labels from its env
   (probe 3: labels reach hooks in a background session).
4. The hook writes one line: identity, session id, pid, time.
5. `nimitz` reads the line and records the session id in the registry, at the
   end of that identity's **lineage**.

From then on, every decision uses the session id, never the name.

### 3.3 Checks before each start

| Check | Why |
| --- | --- |
| The folder is trusted (inside `NIMITZ_HOME`, or trusted on its own). | `--bg` refuses an untrusted folder (probe 0). |
| No live session has this identity. | Resume of a live session makes a copy (probe 2). |
| No live session has this name. | Claude Code allows two live background sessions with one name (probe 5). |
| The caller may start this role (section 5). | The chain of command. |

## 4. Find: how `nimitz` knows the state of each session

`nimitz` uses four kinds of evidence, in this order:

| Order | Evidence | Gives |
| --- | --- | --- |
| 1 | session file + `kill(pid, 0)` | live or not; `busy`, `idle` or `waiting`; what it waits for |
| 2 | `claude agents --json`, every ~10 s | the documented check; job state; rows of stopped sessions |
| 3 | hook lines | binding (section 3.2); crew start and stop; why a session ended |
| 4 | the multiplexer (Herdr first) | which pane shows the session |

**Hooks do not decide busy or idle.** They miss an interrupt and a granted
permission, and they can lose events in a background session (survey: taimux).
The session file shows both.

**A session with no labels is a stowaway.** Admiralty shows it in red, with
"adopt" and "delete".

## 5. Command: the chain of command, enforced

### 5.1 Who may start what

| Caller | May start |
| --- | --- |
| you | admiral, commodore, ship, tender |
| commodore | one ship of its own squadron that it needs now (revive); tenders |
| admiral | nothing |
| ship, tender | nothing |

**How `nimitz` knows the caller.** A commodore starts a ship by running
`nimitz launch` with its Bash tool. That process inherits the commodore's env,
so `nimitz` reads `NIMITZ_ROLE` and `NIMITZ_ID` from it (probe P0.6: the
labels reach the Bash tool's processes). With no labels, the caller is you.

### 5.2 Who may signal whom

Sessions send signals with Claude Code's own `SendMessage`. The plugin's
**PreToolUse** hook sees each `SendMessage` before it goes, with the sender's
labels and the target name, and blocks a signal that the rules forbid (probe P0.7: tool name `SendMessage`,
target in `tool_input.to`; a `deny` decision blocks the call).

| From | May signal |
| --- | --- |
| admiral | its commodores (questions and orders) |
| commodore | its admiral, its ships, its tenders |
| ship | its commodore; other ships, as the squadron's `comms` rule allows |
| tender | its commodore |

The squadron's `comms` rule in `squadron.toml`: `open` (any ship of the
squadron), `commodore-only`, or a list of allowed targets (can include ships of
other squadrons).

### 5.3 You and the chain

You are above the chain. From Admiralty you can:

- **type to an admiral or a commodore**: in its pane (Herdr `agent prompt`), or
  open it in this terminal;
- **reach a ship**: through its commodore, or attach to the ship and type.
  Claude Code has no supported way to type into a background session from
  outside.

## 6. Repair: how `nimitz` keeps sessions aligned

| Event | What `nimitz` does |
| --- | --- |
| A ship stopped and is needed | Resume it with the **native** binary and **no flags**, from its folder: `<native claude> --bg --resume <id>`. Claude Code continues the same session id with its saved name, permission mode and labels (probe 2c1, both builds; SessionStart `source: resume`). Any flag, or the ACME wrapper on PATH, makes a copy instead **while the job row exists**, and that copy has no conversation history. A deliberate relaunch (new labels, name or settings) therefore stops the session, removes its job row with the native `claude rm`, and then runs `--bg --resume <id>` with every flag: Claude Code keeps the same id and its history and takes the new labels and name (found by the adversarial tester on real Claude Code, 2026-10-04). If the `rm` fails, nothing is resumed. |
| A name differs from the identity | Mark it. A hook title does not change the name (probe 4). The fix is a relaunch with `--name`. |
| Two live sessions have one identity | Mark it red. Do not stop either one without you. |
| An admiral or commodore is now a background session | Mark it. You attach to it and keep it in a terminal. |
| A stowaway | List it. You adopt it (add labels at its next start) or delete it. |
| A session file has extra bytes | Read the first JSON value only (seen on ACME 2.1.286+). |
| Claude Code changes a format | `nimitz doctor` compares with `docs/pinned-formats.md` and fails loudly. |

## 7. What is still open

1. **Done: the public build.** Both builds agree once the ACME wrapper's added flags are
   taken into account (`docs/probes/README.md`).
2. **Done on ACME, to run on the public build:** P0.6 (labels reach Bash), P0.7
   (`SendMessage` in PreToolUse; deny blocks), P0.8 (`claude -c` keeps the id).

## 8. Questions

```ask
id: chain_start
type: confirm
question: Section 5.1 - may a commodore start tenders and revive its own ships, and nothing else?
```

```ask
id: chain_signal
type: confirm
question: Section 5.2 - is this signal table right? (admiral to commodores; commodore to admiral, own ships, own tenders; ships by the squadron's comms rule)
```

```ask
id: user_to_ship
type: choice
question: Section 5.3 - when you message a ship from Admiralty, what should happen by default?
options:
  - "Relay through its commodore"
  - "Attach to the ship in a pane, so I type myself"
  - "Ask me each time"
```

```ask
id: probes_next
type: confirm
question: Run probes P0.6 to P0.8 on this machine now, and add them to the home machine's handoff?
```

```ask
id: unclear
type: text
question: Which part is still unclear?
```
