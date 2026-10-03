# Review: how Claude Code, the fleet skill and admiralty-watch identify agents

Date: 2026-10-02. Scope: this machine. Sources: the fleet skill
(`~/.claude/skills/fleet/SKILL.md`, `references/operations.md`,
`references/admiralty.md`), the admiralty-watch code (`src/sources.rs`,
`src/model.rs`), and the files on this machine, read on 2026-10-01/02.

## 0. Why this document

You see statuses in admiralty-watch that you do not recognize. The cause is
that three layers each keep their own state, with their own words:

1. **Claude Code** keeps files about sessions and processes.
2. **The fleet skill** gives rules about names, directories and launches.
3. **admiralty-watch** reads layer 1 and applies the rules of layer 2.

Some rules in layer 2 are missing or are not strict. Where a rule is missing,
admiralty-watch had to guess. This document shows each layer, then each gap,
then a proposal for strict rules. Section 6 has the questions for you.

## 1. Words used in this document (the glossary)

Each word has one meaning in this document.

| Word | Meaning |
| --- | --- |
| **session** | One Claude Code conversation. Its identity is a **session id** (a UUID). |
| **transcript** | The file that stores a session: `~/.claude/projects/<dir>/<session id>.jsonl`. It stays after the process stops. |
| **title** | The name of a session, stored with the transcript (`custom-title.json`). `--name` sets it. |
| **process** | A running `claude` program that hosts one session. It has a **pid**. |
| **session file** | `~/.claude/sessions/<pid>.json`. A process writes it while it runs. |
| **job row** | `~/.claude/jobs/<job id>/state.json`. Claude Code writes it for each **background** session. It stays after the process stops. |
| **live** | A session whose process runs now (its pid exists). |
| **dead** | A session with no process. Its transcript and job row can still exist. |
| **fleet** | A directory with `FLEET_CONFIG.json` and `FLEET.md`. |
| **lead** | The interactive session named `[FLEET]-LEAD`, started by you with `ccl`. |
| **fleet agent** | A background session whose name and directory match one entry in `FLEET_CONFIG.json` `agents[]` for this machine. |
| **helper** | Any other session that belongs to a fleet. It is not a fleet agent. |
| **sub-agent** | A worker inside a session (Agent tool or in-process teammate). It has no process of its own. |
| **stale row** | An older job row with the same name as a newer one. |
| **stray** | A session in a fleet directory, or with a fleet tag, that no rule explains. |

## 2. Layer 1: what Claude Code keeps (verified on this machine)

```
~/.claude/
  projects/<dir as dashes>/
      <session id>.jsonl            transcript (stays)
      <session id>/custom-title.json title, e.g. "[ARGOS]-LEAD"
      <session id>/subagents/agent-<id>.meta.json + .jsonl   sub-agents
  sessions/<pid>.json               session file (only while the process runs)
  jobs/<job id>/state.json          job row (background only; stays)
  jobs/<job id>/timeline.jsonl      job history
  tasks/<team>/<n>.json             task list items
```

Facts. Each one was seen on this machine or is stated in the fleet skill.

1. **A session file exists only while its process runs.** It holds `pid`,
   `sessionId`, `cwd`, `kind` (`interactive` or `bg`), `name`, `status`.
2. **`status` in a session file has three values:** `busy` (in a turn), `idle`
   (waits for input), `waiting` (stopped at a prompt; `waitingFor` gives the
   prompt, e.g. "permission prompt"). `waiting` appeared in Claude Code 2.1.286.
3. **A job row stays after the process stops.** Its `state` field holds the
   last value (`blocked`, `done`, `failed`, `stopped`, ...). This value goes
   stale. It does not tell you if the session is live.
4. **A job row can hold `needs`:** the question the agent asked you. It can stay
   after the agent stops.
5. **`claude agents --json --all`** is job rows plus session files. It gives a
   `pid` only for live sessions.
6. **`claude -c` continues the newest session in a directory with the same
   session id.** Seen: the ARGOS lead kept session `0def5678` when its pid
   changed from 27315 to 39801.
7. **`claude --bg --resume <id>` makes a new session id and a new job row.** The
   new transcript holds a copy of the old one. The old job row stays. (Fleet
   skill, "What --resume actually does".) Thus each relaunch adds one stale row.
8. **`claude rm <job id>` removes a job row.** The transcript stays.
9. **A sub-agent has no status field.** Only the time of its last write tells you
   if it still works.
10. **Claude Code sometimes writes a session file badly.** It rewrites the file
    without truncation, so old bytes stay after the JSON. Seen on the ARGOS lead
    (`sessions/39801.json`). `claude agents --json` then drops that live session.
11. **An interactive session has no job row.** When a lead stops, only its
    transcript stays.

## 3. Layer 2: what the fleet skill says

The rules that the skill has now:

- **Identity is (directory, full name) on one machine.** Full name is
  `[FLEET]-<AGENT>`, uppercase.
- **Fleet agents are the entries in `FLEET_CONFIG.json` `agents[]`** that have a
  `dirs[<this machine>]` entry.
- **The lead is not in `agents[]`.** Only you start it, with `ccl`.
- **Live means the row has a pid.** `state: blocked` is not death.
- **One live session per name.** Relaunch = resume the newest row, then `rm`
  the older rows, then check that each (name, directory) has exactly one row.
- **Always pass `--name`.** A session without a name drops out of the fleet.
- **`status` operation:** three groups: in the config and running; in the
  config but not here; running but not in the config ("offer save").
- **`save` operation:** adds each live `[FLEET]-<AGENT>` background session to
  the config.

## 4. Layer 3: what admiralty-watch does now

| Step | Rule in the code |
| --- | --- |
| Find sessions | Read job rows and session files directly (same data as `claude agents --json --all`). Ignore bytes after the first JSON value (fact 10). |
| Live | The session file exists and `kill(pid, 0)` succeeds. |
| Fleet of a session | Its name has a `[TAG]-` prefix of a known fleet, else its `cwd` is a fleet directory or a declared agent directory. |
| Lead | Name is `[TAG]-LEAD` or `[TAG]-LEAD-...`. If no live lead, the newest transcript with that title (fact 11). |
| Fleet agent | Name matches an `agents[]` entry. |
| Helper | Any other session of the fleet. Shown under the lead as "background session". |
| One row per name | The newest live row is shown. Older rows fold into "+N stale". |
| Status of a fleet agent or helper | `not running` (no row) → `stopped` (rows, no live process) → `needs you` (live and: `status: waiting`, or `needs` in the job row, or Herdr says `blocked`) → `busy` (`status: busy`) → `idle`. |
| Sub-agent | `running` if written in the last 90 s (or a child runs), else `finished`. Finished ones show for one hour. |

## 5. The gaps (where the rules are missing or not strict)

### Gap A. The skill does not define "helper".
**What you see:** `[ARGOS]-RAGTIME` and "publish plan draft spec" under ARGOS.
**Cause:** The ARGOS lead started `[ARGOS]-RAGTIME` with `claude --bg --name`.
Its launch prompt says "Report to [ARGOS]-LEAD". The name has the fleet tag, but
the agent is not in `agents[]`. The skill has no word for this.
**Risk:** The skill's `status` operation says "running but not in the config →
offer save". `save` then adds the helper as a fleet agent. This is the opposite
of your model.

### Gap B. The skill allows a session with no name in a fleet directory.
**What you see:** "publish plan draft spec". Claude Code made this title from the
first prompt, because the launch had no `--name`.
**Cause:** "Always pass `--name`" is a rule for the lead's launches. Nothing
stops other launches (for example from the agent view TUI) in a fleet directory.
Nothing records who started it.

### Gap C. "Off-config" is a different identity, but the watch shows it as the same agent.
**What you see:** `[GX]-BOGS` live in `AcmeWS`, but the config says
`AcmeFleet`.
**Cause:** The skill says identity is (directory, name). By that rule, this is a
second agent with a borrowed name. The watch groups by name only and adds a flag.

### Gap D. Stale rows grow between relaunches.
**What you see:** `[ARGOS]-ARGOS  +3 stale`.
**Cause:** Each resume adds a row (fact 7). The skill removes old rows only in
`restore` and in the one-agent revive. A revive that fails between "launch" and
"rm" leaves a row. Nothing finds these rows at other times.

### Gap E. The skill's liveness check can miss a live session.
**Cause:** The skill checks for a `pid` in `claude agents --json`. Fact 10 makes
that list drop a live session.
**Risk:** The lead can "revive" an agent that is live. That gives two live
sessions with one name, which the skill forbids.

### Gap F. "Waiting for you" has three sources, and the skill names none of them.
The sources are `status: waiting` in the session file, `needs` in the job row,
and Herdr `blocked`. The skill says only that `blocked` is not death. A lead
cannot tell "idle and healthy" from "stopped at a question" with the skill's
words.

### Gap G. The lead name can drift.
**What you see:** `[BRIM]-LEAD-wobbly-acorn`. Also, the session file of the ARGOS
lead says `[ARGOS]-LEAD` and its title says `[ARGOS]-LEAD-serene-scroll`.
**Cause:** Claude Code adds a suffix when the name is in use. `ccl` refuses a
duplicate, but a manual `/rename` or a second start can still make one. Two
places store the name (session file and title), and they can differ.

### Gap H. A lead with no transcript has no row.
**What you see:** CARPENTER and OAK show no lead.
**Cause:** No session in those directories has the title `[TAG]-LEAD`. The leads
ran under other names, or never ran here.

### Gap I. Sub-agent status is a guess.
A sub-agent has no status (fact 9). The watch uses "written in the last 90 s".
A sub-agent that thinks for a long time without writing shows `finished` until
its next write. No rule can fix this; the watch can only label it as a guess.

## 6. Proposal: strict rules

Each rule has an id. Rules S* go in the fleet skill. Rules W* go in
admiralty-watch. The questions below are the decisions that only you can make.

**S1. Three kinds of session in a fleet, and only three.** A session in a fleet
is the lead, a fleet agent, or a helper. A fleet agent is an `agents[]` entry
with this machine's directory, and its name **and** directory match. Everything
else is a helper.

**S2. Helper names.** The lead names each helper so that the owner shows in the
name. The form is a question for you (Q1). The name `[FLEET]-<AGENT>` is only for
fleet agents.

**S3. `save` and `status` never promote a helper.** "Running but not in the
config" becomes "helpers" in `status`. `save` adds a session only when you name
it.

**S4. Every launch in a fleet directory has `--name`.** A session with no name is
a stray. The lead reports strays. It does not adopt them.

**S5. Liveness has two checks.** A pid in `claude agents --json`, **or** a session
file with that session id whose pid runs. Do not revive a session that passes
either check.

**S6. Waiting for you.** A live session waits for you when its session file says
`status: waiting`, or its job row has `needs`, or Herdr says `blocked`. The lead
reports this as a blocker. It does not answer for you.

**S7. Stale rows.** After each resume, `rm` the older rows and check one row per
(name, directory). When a check fails, report it.

**S8. Wrong directory.** A session with a fleet agent's name in another directory
is a stray, not that agent. The lead reports it.

**S9. Lead name.** The lead's name is exactly `[FLEET]-LEAD` in the session file
and in the title. A suffix is an error to report.

**W1.** The watch shows only what S1 to S9 define: lead, fleet agents, helpers
under the lead, strays (with a red flag), sub-agents.
**W2.** The watch shows the reason for each status in the detail pane, for
example "needs you: job row has a question", "needs you: Herdr blocked".
**W3.** The watch marks the sub-agent status as estimated (Gap I).
**W4.** The watch counts strays and stale rows in the fleet title, so that
cleanup work is visible.

```ask
id: helper_name
type: choice
question: Q1. How should a lead name the helpers it starts (S2)?
options:
  - "[FLEET]-LEAD/<task>  (owner and task in the name)"
  - "[FLEET]-H-<TASK>  (H for helper)"
  - "Do not start helpers as background sessions: use Agent-tool sub-agents only"
  - "Keep [FLEET]-<NAME>; anything not in agents[] is a helper (today's watch rule)"
```

```ask
id: offconfig
type: choice
question: Q2. A session with a fleet agent's name in the wrong directory (Gap C, S8). How should the watch show it?
options:
  - "As a stray under the lead, with a red flag (recommended: matches the skill's identity rule)"
  - "As the fleet agent, with an off-config flag (today)"
```

```ask
id: unnamed
type: choice
question: Q3. A session with no name in a fleet directory (Gap B, S4)?
options:
  - "Stray: show it under the lead with a red flag (recommended)"
  - "Helper: show it under the lead like a named helper (today)"
  - "Hide it; show it only in OTHER"
```

```ask
id: liveness
type: confirm
question: Q4. Change the fleet skill so that liveness has two checks (S5)? Without it, a lead can revive a live agent and make a duplicate (Gap E).
```

```ask
id: skill_owner
type: choice
question: Q5. Who changes the fleet skill?
options:
  - "Me, through /mycc (the skill lives in the my_claude store)"
  - "[GENERAL]-CONFIGS (its fleet owns the configs)"
  - "Write the change as a diff first; I decide after"
```

```ask
id: other_gaps
type: text
question: Anything here still unclear, or a status you saw that this document does not explain?
```
