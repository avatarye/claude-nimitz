# Plan v2: claude-nimitz — a gap-free fleet system on the Claude Code CLI

Date: 2026-10-03. Status: v2, after your answers to v1. Earlier review:
`review-fleet-identity-and-status.md` (gaps A–I, rules S1–S9).

## 0. Context

You run several groups of Claude Code agents. The current fleet skill states its
rules as text, and the watch guesses identity and status from Claude Code's
internal files. That causes the gaps in the earlier review.

**claude-nimitz** replaces both with one product on the `claude` CLI only. Thus
it works with your Claude subscription at home and with Bedrock elsewhere.

## 1. Decisions so far

| Topic | Decision |
| --- | --- |
| Repository | One new repository, `claude-nimitz`: core, command, TUI, Claude Code plugin, Herdr plugin. |
| Command | `nimitz` (global CLI; also used by the plugin's skills and hooks). |
| TUI | **Admiralty**: the office where the admirals work. It shows and controls every fleet on the machine. |
| Distribution | A Claude Code plugin from the repository: skills and hooks together. |
| Helpers (tenders) | Started with `nimitz tender`, with an explicit parent label. |
| Old sessions | Every fleet relaunches through `nimitz` once phase P2 is ready. |
| Probes (P0) | `[GENERAL]-LEAD` runs them, from a script in the repository. |
| Messaging | Ships talk to each other directly. Each squadron has its own rule for it. |
| Data root | `~/.claude-nimitz` (override: `NIMITZ_HOME`). For you: a symlink into Dropbox. For other users: a normal folder; sync is their choice. |
| Machines | One fleet shape for all machines; each machine can run a different set of ships, in different folders. |
| Fleets per machine | Several, each with its own admiral. |

## 2. Words

| Word | Meaning |
| --- | --- |
| **Admiralty** | The TUI. It shows all fleets, admirals, squadrons, ships. |
| **fleet** | A top-level group, with one admiral. A machine can hold several fleets. |
| **admiral** | The interactive session that commands one fleet. It asks commodores; it gives no orders to ships. |
| **squadron** | A group of ships under one commodore (today: ARGOS, BOLT, ...). |
| **commodore** | The interactive session that commands one squadron (today: the lead). Only you start it. It must stay in the foreground. |
| **ship** | A background session that a squadron declares. It has its own process and folder. You can attach to it; it stays a background session. |
| **tender** | A background session that a commodore starts for one job. It has a parent label. It is not a ship. |
| **crew** | A sub-agent or teammate inside a session. It has no process of its own. |
| **stowaway** | A session in a fleet's scope that has no labels. Admiralty shows it in red. |
| **signal** | A message between sessions. |
| **identity** | The registry record of a ship (or commodore, admiral, tender). It never changes. |
| **lineage** | The session ids that one identity used, oldest first. |
| **machine label** | The key for this machine in a squadron config (as today). |

## 3. Folder layout

```
~/.claude-nimitz/                      NIMITZ_HOME (for you: symlink into Dropbox)
  <fleet>/                             no CLAUDE.md here
    fleet.toml                         fleet name, admiral settings, machine labels
    admiral/                           admiral's working folder (its CLAUDE.md)
    squadrons/<SQUADRON>/              commodore's working folder
      squadron.toml                    ships, per-machine folders, comms rule
      CLAUDE.md, SQUADRON.md
    ships/<SQUADRON>/<SHIP>/           folders for ships with no project folder
~/.local/state/nimitz/                 per machine, never synced
  registry.json                        identities, labels, lineage
  events.jsonl                         hook events
```

Rules for the layout:

1. **No folder is inside another session's folder.** Claude Code loads
   `CLAUDE.md` from every parent folder. Thus `admiral/`, `squadrons/` and
   `ships/` are siblings, and `<fleet>/` holds no `CLAUDE.md`.
2. **Per-machine state is outside `NIMITZ_HOME`.** Dropbox must not sync the
   registry or the event log between machines.
3. **A ship's folder per machine is in `squadron.toml`**, as `dirs` is in
   `FLEET_CONFIG.json` today. A ship with no entry for this machine does not
   run here.

## 4. The design

### 4.1 Identity in three layers

| Layer | What | Set by | Can Claude Code change it? |
| --- | --- | --- | --- |
| **Identity** | registry record `FLEET/SQUADRON/SHIP` | `nimitz` | No. |
| **Binding** | our own `--session-id <uuid>` at launch, and labels in `--settings '{"env":{...}}'` | `nimitz` | The id changes on resume; `nimitz` records each new id in the lineage. |
| **Address** | `--name`, which `SendMessage` uses | `nimitz` | Yes (variants, plan titles, renames). So `nimitz` checks it and does not trust it. |

`nimitz launch` refuses a name that a live session holds, because Claude Code
does not check this for background sessions. Admiralty marks a name that no
longer matches its identity.

### 4.2 Evidence for state, in order

| Order | Evidence | Gives |
| --- | --- | --- |
| 1 | `sessions/<pid>.json` + `kill(pid,0)` | live; busy / idle / waiting; the waiting reason |
| 2 | `claude agents --json`, every ~10 s | the documented check; job state |
| 3 | hook events | binding (session id + labels + pid), crew start and stop, end reason, permission mode |
| 4 | Herdr panes | where a session shows; Herdr state |

Hooks do not decide busy or idle: they miss interrupts and granted permissions,
and they can lose events in background sessions (survey: taimux).

### 4.3 Signals

- **Ship to ship:** direct `SendMessage`. Each `squadron.toml` has a `comms`
  rule: `open` (any ship in the squadron), `commodore-only`, or an allow list
  that can include ships of other squadrons. The `nimitz` hook checks each
  `SendMessage` before it goes (PreToolUse) and blocks one that the rule forbids.
- **Admiral to commodore:** questions only, as today.
- **You to a ship, from Admiralty:** Claude Code has no supported way to type
  into a background session from outside. Admiralty offers two ways: relay
  through the commodore, or attach to the ship in a pane and type.

### 4.4 The `nimitz` command

| Command | Does |
| --- | --- |
| `nimitz status [--json]` | The snapshot Admiralty shows. Read-only. |
| `nimitz admiral <FLEET>` | Start the admiral (replaces `ccl` in the hub). |
| `nimitz commodore <FLEET> <SQUADRON>` | Start a commodore (replaces `ccl` in a squadron). Refuses a second live one. Foreground only. |
| `nimitz launch / resume <ship>` | Cold start with a new UUID and labels; or resume the newest session in the lineage. |
| `nimitz tender <SQUADRON> <TASK>` | A commodore starts a tender with a parent label. |
| `nimitz stop / rm / sweep` | Through the native binary; then check that it took effect. |
| `nimitz signal <identity> <text>` | Relay through the commodore, or type into a pane (Herdr). |
| `nimitz open <identity>` | Show the session in a pane, or in this terminal. |
| `nimitz doctor [--fix]` | Hooks, `crossSessionInbound`, native binary, pinned formats, folder rules. |
| `nimitz hook <event>` | Called by the plugin's hooks. Appends one line. Always exits 0. |

### 4.5 Admiralty (the TUI)

- One frame per fleet at the top level, with its admiral; squadrons inside;
  ships, tenders and crew as today.
- Every key calls the same library code as the `nimitz` command.
- Multiplexer adapter, chosen from the innermost multiplexer (`HERDR_ENV`,
  `TMUX`, `ZELLIJ`, `WEZTERM_PANE`), with an override. In a plain shell,
  Admiralty suspends itself, runs the session in this terminal, and comes back.
- Feature order (your ranking): start a commodore or a ship; open any session;
  stowaway list with adopt / delete; a reason for every status; mailbox;
  tokens and cost.

### 4.6 Repository

```
claude-nimitz/
  crates/nimitz-core/     registry, evidence, rules, actions
  crates/nimitz/          the `nimitz` command and hook binary
  crates/admiralty/       the TUI
  plugin/                 Claude Code plugin: skills/fleet, skills/admiral, hooks/hooks.json
  herdr-plugin/           the alt+f overlay
  probes/                 the P0 script for [GENERAL]-LEAD
  docs/pinned-formats.md  every internal Claude Code format we read
```

## 5. Phases

| Phase | Result | Who runs real sessions |
| --- | --- | --- |
| **P0. Probes** (½ day) | Facts: `--session-id` with `--bg`; the id after `--bg --resume`; labels reach hooks in a background session; hooks fire there; `sessionTitle` from a hook resets the address; same on Bedrock and subscription. | `[GENERAL]-LEAD` (you ask it) |
| **P1. Core, read-only** (2 d) | `nimitz-core`, `nimitz status/hook/doctor`; Admiralty moves onto it and shows several fleets. | nobody |
| **P2. Launch** (2 d) | `nimitz admiral/commodore/launch/resume/tender`; registry and lineage; folder layout. | you, then each commodore |
| **P3. Actions + plugin** (2 d) | `stop/rm/sweep/signal/open`; comms rules and the hook that enforces them; the plugin's skills. Relaunch of all fleets. | each commodore |
| **P4. Admiralty control** (3 d) | Multiplexer adapters; start / open from Admiralty; stowaway list; status reasons. | you |
| **P5. Package** (1 d) | Plugin install on both machines; Herdr plugin in the repository; public README for other users. | you |

## 6. Risks

1. Claude Code changes every few days. `docs/pinned-formats.md` and
   `nimitz doctor` show a change at once.
2. Hooks can lose events in background sessions. State does not depend on them.
3. A ship folder inside Dropbox can sync while the same ship runs on two
   machines. Question Q2.
4. Session names are global on one machine. With several fleets, two squadrons
   can have the same name. Question Q1.
5. The admiral may not start sessions. The hub writes code; you and the
   commodores run real sessions.

## 7. Questions

```ask
id: names
type: choice
question: Q1. Session names must be unique on a machine, and you will run several fleets. What form?
options:
  - "[FLEET.SQUADRON]-SHIP, [FLEET.SQUADRON]-COMMODORE, [FLEET]-ADMIRAL (unique; recommended)"
  - "[SQUADRON]-SHIP; squadron names must be unique across all fleets on the machine"
```

```ask
id: shipdirs
type: choice
question: Q2. A ship with no project folder gets a folder in ships/ (synced by Dropbox). If that ship runs on two machines at once, both write the same files. What rule?
options:
  - "One folder per machine: ships/<SQUADRON>/<SHIP>/<machine>/ (recommended)"
  - "One shared folder; a ship may run on one machine at a time (nimitz refuses a second)"
  - "One shared folder; no rule"
```

```ask
id: tui_cmd
type: choice
question: Q3. How do you start Admiralty?
options:
  - "`admiralty` (its own command, built from the same repository)"
  - "`nimitz` with no arguments (one command for everything)"
```

```ask
id: hub
type: choice
question: Q4. Today's hub ([ADMIRALTY] in ~/Dropbox/claude-orchestrators/admiralty) becomes what?
options:
  - "The admiral of one fleet, e.g. [MAIN]-ADMIRAL, in ~/.claude-nimitz/main/admiral/"
  - "Keep it as a separate machine-wide hub above all admirals"
```

```ask
id: notes
type: text
question: Anything else to change?
```
