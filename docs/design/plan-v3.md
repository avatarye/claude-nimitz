# Plan v3: claude-nimitz

Date: 2026-10-04. Status: v3.1 (review answers applied). Replaces `plan-v2.md`.
Build contracts: `contracts.md`.
Related: `cc-control-model.md` (how `nimitz` controls Claude Code sessions),
`docs/probes/` (the facts, on two builds).

## 0. Context

claude-nimitz is one product that builds and runs fleets of Claude Code agents.
It drives only the `claude` command, so it works with a Claude subscription and
with Bedrock. Phase P0 is done: probes on the ACME build (2.1.288, Bedrock) and
on the public build (2.1.288/2.1.289, subscription) confirmed every fact the
design needs. This plan adds what v2 did not have: setup and CRUD, optional
multiplexers, and compatibility with Claude Code versions.

## 1. Decisions

| Topic | Decision |
| --- | --- |
| Product | One repository `claude-nimitz`: core, `nimitz` command, `admiralty` TUI, Claude Code plugin, Herdr plugin. |
| Commands | `nimitz` (CLI, also used by the plugin), `admiralty` (the TUI, its own command). |
| Distribution | A Claude Code plugin from the repository (skills and hooks together). |
| Fleets | Several per machine. Each has one admiral. Today's hub becomes `[MAIN]-ADMIRAL` in `~/.claude-nimitz/main/admiral/`. |
| Names (addresses) | `[FLEET]-ADMIRAL`, `[FLEET.SQUADRON]-COMMODORE`, `[FLEET.SQUADRON]-SHIP`, tenders `[FLEET.SQUADRON]-COMMODORE/<task>`. |
| Data root | `~/.claude-nimitz` (`NIMITZ_HOME`). Yours: a Dropbox symlink. Others: a normal folder; sync is theirs. |
| Machines | One fleet shape; each machine runs its own set of ships, in its own folders. |
| Ship folders in the data root | Shared by all machines. A ship runs on one machine at a time; `nimitz` refuses a second. |
| Old fleets | Relaunched through `nimitz` once launch works (phase P3). |
| Signals | Ships talk to each other directly. Each squadron has a `comms` rule; a hook enforces it. |
| Multiplexers | Optional. The core never depends on one. Herdr is the first adapter; others can be added. |

## 2. Words

| Word | Meaning |
| --- | --- |
| **Admiralty** | The TUI: the office where the admirals work. |
| **fleet** | A top-level group with one **admiral** (interactive). |
| **squadron** | A group of ships under one **commodore** (interactive; only you start it). |
| **ship** | A background session that a squadron declares. |
| **tender** | A background session a commodore starts for one job, with a parent label. |
| **crew** | A sub-agent or teammate inside a session. |
| **stowaway** | A session in a fleet's scope with no labels. |
| **signal** | A message between sessions (`SendMessage`). |
| **identity** | `nimitz`'s record of one admiral, commodore, ship or tender. It never changes. |
| **labels** | Env values `nimitz` gives a session: `NIMITZ_FLEET`, `NIMITZ_SQUADRON`, `NIMITZ_ROLE`, `NIMITZ_ID`, `NIMITZ_PARENT`. |
| **adapter** | The code that lets `nimitz` use one multiplexer (Herdr, tmux, ...), or none. |

## 3. Facts from P0 (both builds)

| Fact | Effect on the design |
| --- | --- |
| `--bg` ignores `--session-id`; `--resume <unknown id>` never runs. | `nimitz` cannot choose ids. The SessionStart hook binds identity to the session id through the labels. |
| Labels in `--settings` env reach every hook and the Bash tool's processes. | Binding works; `nimitz` can see which role calls it. |
| `--bg --resume <id>` with **no flags**, after a stop, keeps the id, name, mode and labels. Any flag makes a copy **while its job row exists**, and the copy has **no history**; with the row removed (`claude rm`), `--resume <id>` with flags keeps the id and the history and takes the new flags (adversarial tester, 2026-10-04). | Resume = native binary, no flags, from the ship's folder. A ship keeps one id for life. New labels or name: stop, `rm` the row, then resume with every flag (same id). |
| On ACME, `claude` on PATH is a wrapper that adds `--settings`. | Resume, `stop` and `rm` use the native binary. Cold launches use the PATH `claude` (it carries Bedrock settings). |
| Resume of a live session makes a copy. | Never resume a live ship. |
| Two live `--bg` sessions can have one name. | `nimitz` checks names itself. |
| A hook's `sessionTitle` does not change the name. | A drifted name needs a relaunch. |
| `--bg` refuses an untrusted folder; home-folder trust may not cover children. | `nimitz init` gets `NIMITZ_HOME` trusted on its own. |
| PreToolUse sees `SendMessage` (`tool_input.to`) and a `deny` blocks it. | The comms rule is enforced by a hook. |
| Interactive `claude -c` keeps the session id. | A commodore and an admiral keep one id. |
| Field sets change between versions (`scratchpad_dir` appeared in 2.1.289). | `nimitz` reads only the fields it needs; `doctor` checks those. |

## 4. Folder layout

```
~/.claude-nimitz/                      NIMITZ_HOME
  <fleet>/                             no CLAUDE.md here
    fleet.toml                         fleet settings, machine labels
    admiral/                           admiral's folder
    squadrons/<SQ>/                    commodore's folder: squadron.toml, CLAUDE.md, SQUADRON.md
    ships/<SQ>/<SHIP>/                 ships with no project folder
~/.local/state/nimitz/                 per machine, never synced: registry, events
```

No session folder is inside another one (Claude Code loads `CLAUDE.md` from
every parent folder).

## 5. Setup and CRUD

### 5.1 Principle: the config is the truth; starting is a reconcile

Creating, changing or deleting a fleet, squadron or ship only changes the config
(`fleet.toml`, `squadron.toml`), after `nimitz` checks it. Running sessions
change only through **`nimitz up`** and the single-session commands. `nimitz up`
first shows a plan (what it will start, resume or stop) and runs it after you
agree. Admiralty shows the same difference all the time ("drift").

### 5.2 First run

1. Install the plugin and the `nimitz` and `admiralty` binaries.
2. `nimitz init`: create `NIMITZ_HOME`; register this machine's label; ask you
   to trust `NIMITZ_HOME` (run `claude` there once); install the hooks; set
   `crossSessionInbound`; find the native `claude`; run `nimitz doctor`.
3. `admiralty`: an empty screen with "`n` new fleet".

### 5.3 Operations

| Thing | Create | Update | Delete |
| --- | --- | --- | --- |
| fleet | `nimitz fleet new` | `nimitz fleet set` | only with nothing live; the config is archived |
| squadron | `nimitz squadron new` (brief → `SQUADRON.md`, `comms` rule) | brief, `comms` | after its ships are retired; transcripts kept |
| ship | `nimitz ship new` (project folder or managed folder, prompt in `$EDITOR`, permission mode) | prompt, folder, mode (take effect at the next relaunch) | stop, remove job rows, remove from config; transcripts kept |
| machine | `nimitz machine add` (label, then a folder per ship or skip) | folders | remove the label from every config |
| stowaway | `nimitz adopt` (make it a ship or a tender) | — | `nimitz rm` |

Single-session commands: `nimitz admiral`, `commodore`, `launch`, `resume`,
`tender`, `stop`, `rm`, `sweep`, `signal`, `open`, `doctor`, `hook`
(`cc-control-model.md`).

### 5.4 Where you do it

| Front end | How |
| --- | --- |
| `nimitz` | Every operation; for scripts and docs. |
| Admiralty | A form for each create and update; `$EDITOR` for prompts; deletes confirm with the exact effects. Each form shows the `nimitz` command it runs. |
| The skill | You ask in words ("add a ship for package X to ARGOS"); Claude runs `nimitz ... ` with flags. The skill has no rules of its own. |

### 5.5 Who may run what (widened in 0.3.1)

| Caller | May run |
| --- | --- |
| you | everything |
| admiral | inside its own fleet, on any member but itself: start, stop, relaunch, rm, sweep, tender, adopt; `up` and `layout herdr` for its fleet; `fleet set`, `squadron new/set/rm`, `ship new/set/rm` |
| commodore | inside its own squadron: start, stop, relaunch, rm, sweep, tender, adopt for its ships and tenders; `squadron set`; `ship new/set/rm` |
| ship, tender | nothing |

Yours alone: `init`, `fleet new`, `fleet rm`, `machine add|rm`, `migrate`, and
a session named by short id that is no member's (a stowaway outside the
admiral's fleet, a stray). A commodore never removes its own squadron. The
plan step stays: `--plan`, then `--yes`. Core: `sessions::may_start`,
`may_config`, `may_run` (which fixes a member may run itself).

`nimitz` reads the caller's labels from its env (fact: labels reach the Bash
tool's processes).

## 6. Multiplexers: optional, through adapters

### 6.1 The adapter interface

The core uses one interface. An adapter implements it for one multiplexer.

| Operation | Meaning |
| --- | --- |
| `detect()` | Am I running inside this multiplexer? |
| `panes()` | List panes, with the Claude session each one hosts (if known) and its state. |
| `open(command, where)` | Run a command in a new pane, tab or workspace. |
| `focus(pane)` | Show a pane to the user. |
| `type(pane, text)` | Type into an interactive session (optional capability). |
| `layout(plan)` | Make workspaces, tabs and panes for fleets (optional capability). |

Each operation that an adapter does not have is reported, not guessed. With no
adapter (plain shell), `open` means: Admiralty suspends itself, runs the session
in this terminal, and comes back when it ends.

### 6.2 Adapters

| Adapter | Status | Notes |
| --- | --- | --- |
| none (plain shell) | P5 | Always there. |
| Herdr | P5 | `herdr agent list/focus/prompt`, `pane run`, `workspace/tab create`. |
| tmux | later | `split-window`, `select-pane`, `display-popup`; no typing. |
| others (zellij, wezterm, kitty) | later | Same interface. |

Choice: the innermost multiplexer from the env (`HERDR_ENV`, `TMUX`, ...), or
`NIMITZ_MUX=<name>`.

### 6.3 Layout is a separate command

`nimitz layout herdr [<fleet>]` maps fleets onto the Herdr session it runs in:
one workspace per squadron, a tab with its commodore, and the panes the squadron
asks for. `nimitz up` never makes a layout. A layout command exists only for an
adapter that has the `layout` capability.

## 7. Signals

| From | May signal |
| --- | --- |
| admiral | its commodores (questions and orders; questions only before 0.3.1) |
| commodore | its admiral, its ships, its tenders |
| ship | its commodore; other ships as the squadron's `comms` rule allows |
| tender | its commodore |

`comms` in `squadron.toml`: `open`, `commodore-only`, or an allow list (can name
ships of other squadrons). The plugin's PreToolUse hook reads the sender's labels
and `tool_input.to`, and denies a signal that the rule forbids.

You are above the chain: type to an admiral or a commodore in its pane (or in
this terminal); reach a ship through its commodore or by attaching to it.

## 8. Compatibility with Claude Code versions

### 8.1 Supported range

Each `nimitz` release has a supported range of Claude Code versions, in
`compat.toml` (for example `2.1.281 - 2.1.289`). `nimitz doctor` and every
`nimitz` launch check the installed version:

- inside the range: run;
- above the range (newer than any version tested): warn, "tested up to X",
  and run, launches too (since 0.3.2: a nightly Claude Code update must not
  stop a fleet). `max` in `compat.toml` is the highest version tested;
- below the range, an unknown version, or a pre-release `compat.toml` does
  not list: refuse launches (read-only commands still work; `--force`
  overrides).

`docs/pinned-formats.md` lists the fields `nimitz` reads in each file and hook.
`doctor` checks only those fields, so a new field (such as `scratchpad_dir`) does
not fail it.

### 8.2 Tests in three tiers

| Tier | When | Where | Login | Checks |
| --- | --- | --- | --- | --- |
| 1. offline | every push | GitHub runner | none | unit and sandbox tests (fake `claude`, fake `herdr`); stored file formats per version |
| 2. live, public build | nightly and on each new Claude Code release | GitHub runner, or a self-hosted runner at home | `CLAUDE_CODE_OAUTH_TOKEN` from `claude setup-token` (one year; documented for CI) | P0 probes and a smoke test on a matrix of versions, cheapest model, `CLAUDE_CONFIG_DIR` set to a new temp folder |
| 3. ACME | each new ACME version | other machine, by hand | Bedrock | the same probes; findings committed |

A scheduled job finds each new Claude Code release. Pass: a pull request widens
the range. Fail: an issue with the difference; the range stays until a patch.

Unknown: whether `--bg` works on a GitHub-hosted runner (the daemon may need a
persistent user session). The first tier-2 run answers it; the fallback is the
self-hosted runner.

## 9. Repository

```
claude-nimitz/
  crates/nimitz-core/      registry, evidence, rules, actions, compat
  crates/nimitz-mux/       adapter interface + adapters (none, herdr, later tmux)
  crates/nimitz/           CLI and hook binary
  crates/admiralty/        TUI
  plugin/                  skills (fleet, admiral), hooks/hooks.json
  herdr-plugin/            alt+f overlay
  probes/                  P0 scripts
  docs/                    design, probes, pinned-formats.md, compat.toml
  .github/workflows/       tier 1 and tier 2
```

## 10. Phases

| Phase | Result | Days |
| --- | --- | --- |
| P0 | Probes on both builds. **Done.** | — |
| P1 | `nimitz-core` read-only: registry, evidence, `nimitz status/hook/doctor`; `admiralty` (from admiralty-watch) on it, several fleets. Tier 1 CI. | 3 |
| P2 | Config CRUD: `nimitz init`, `fleet/squadron/ship/machine new|set|rm`, validation; Admiralty forms. | 3 |
| P3 | Sessions: `admiral/commodore/launch/resume/tender/stop/rm/sweep`; `nimitz up` (plan, then apply); `compat.toml` checks. Relaunch your fleets. | 3 |
| P4 | Signals and plugin: comms rules and the PreToolUse hook; `nimitz signal`; the plugin's skills. | 2 |
| P5 | Adapters: none and Herdr; `nimitz open`, `nimitz layout herdr`; Admiralty start/open; stowaway list; status reasons. | 3 |
| P6 | Package and CI tier 2: plugin install on both machines, version-watch job, public README. | 2 |

## 11. Risks

1. Claude Code changes every day or two (2.1.288 to 2.1.289 during P0). The
   range, `pinned-formats.md`, `doctor` and tier-2 CI make each change visible.
2. The ACME wrapper can change what it adds. Tier 3 checks each ACME version.
3. Hooks can lose events in background sessions. State does not depend on them.
4. `--bg` on GitHub-hosted runners is unknown. Fallback: self-hosted runner.

## 12. Questions

```ask
id: comms_default
type: choice
question: Q1. Default comms rule for a new squadron?
options:
  - "open: any ship of the squadron may signal any other (recommended)"
  - "commodore-only: ships signal only their commodore"
```

```ask
id: up_plain
type: choice
question: Q2. `nimitz up` in a plain shell (no multiplexer) cannot open the admiral and commodores as panes. What should it do?
options:
  - "Start the ships; list the commodores with the command to start each (recommended)"
  - "Refuse; ask for a multiplexer"
```

```ask
id: range_floor
type: text
question: Q3. Lowest Claude Code version to support at first (2.1.281 added the trust check that --bg needs; recommended floor 2.1.281)
placeholder: 2.1.281
```

```ask
id: ci_runner
type: choice
question: Q4. Tier-2 CI: where should it run first?
options:
  - "GitHub-hosted runner with a setup-token secret; self-hosted only if --bg fails there (recommended)"
  - "Self-hosted runner on the home machine from the start"
  - "No tier 2 yet; probes by hand"
```

```ask
id: user_to_ship
type: choice
question: Q5. When you message a ship from Admiralty, the default?
options:
  - "Relay through its commodore"
  - "Attach to the ship in a pane, so I type myself"
  - "Ask me each time"
```

```ask
id: start_p1
type: confirm
question: Q6. Start phase P1 (read-only core and Admiralty on it) after this review?
```

```ask
id: notes
type: text
question: Anything else to change?
```

## 13. v3.1: decisions from the v3 review

| Topic | Decision |
| --- | --- |
| Multiplexer | **Required to run a fleet**: starting or opening an admiral or a commodore, `nimitz up`, `open`, `layout`. Not required for ships (daemon), `status`, CRUD, `check`, `doctor`, hooks, CI. Admiralty runs anywhere; with no mux it is monitor-only and says why. Adapter interface kept; only Herdr is built now. |
| Restart order | Top-down, always: admiral, then commodore, then the target ship. `nimitz start <id>` brings up each missing parent first. |
| Integrity | `nimitz check` runs before every start and refuses on an error, with a fix command (`contracts.md` section 7). It finds manual changes: renames, moved folders, edited configs, copies, missing labels. |
| Relaunch | Every session shape has one action (`contracts.md` section 8, the relaunch state table); each row is a sandbox test. |
| comms default | `open`. |
| `up` in a plain shell | Refuse; ask for a multiplexer. |
| Supported Claude Code | Floor 2.1.288. |
| CI tier 2 | Self-hosted runner on the home machine from the start. |
| You to a ship | Relay through its commodore by default. |
| Branches | `docs`: documents. `dev`: code. `main`: releases. |
