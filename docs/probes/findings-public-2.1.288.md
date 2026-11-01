# P0 findings: public build 2.1.288, subscription

Machine: home Ubuntu server (`home-server`). Date: 2026-10-03.
`claude --version` on PATH and native: `2.1.288 (Claude Code)`, vanilla build at
`~/.local/bin/claude` (no wrapper). Provider: Claude subscription login (no
`CLAUDE_CODE_USE_*`, no `ANTHROPIC_API_KEY`). Raw data:
`results-public-2.1.288-subscription.json`.

This run uses the **revised** `p0_probe.py` (see "What was done"). Probes 2 and 2c
were rewritten and 2a and 2b are new, because the first script passed flags on
every resume and so only ever tested the copy path. Results that differ from the
wrapper build are in **bold**.

| Id | Result |
| --- | --- |
| 0 | Refused. `--bg` in an untrusted folder exits 1: "Workspace not trusted. Run `claude` in <dir> once and accept the trust prompt, then retry." **The home folder was not a trusted parent here: accepting the trust prompt in `~` was not saved (`~/.claude.json` keeps `hasTrustDialogAccepted: false` for the home folder), so `~/.cache/nimitz-probe` had to be trusted on its own.** |
| 1 | Ignored. "warning: --bg manages the session id; ignoring --session-id". The daemon chooses the id; the job id is its first 8 characters. |
| 2a (new) | **Cannot choose an id this way.** `--bg --resume <uuid that names no session>` returns `backgrounded · <that uuid>`, but the session never runs: only SessionEnd fires (with the requested id), no SessionStart, UserPromptSubmit or Stop, and the row ends `failed` with no pid. The wrapper build's run 1-then-2 sequence hit the same case by accident (its row `d4d4525f` also ended `failed`). |
| 2b (new) | **Copy, unlabelled.** `--bg --resume <id>` of a **live** session with no flags starts a copy with a new id: "session X is already running in the background, so this started a copy as Y". The copy fired **no hooks at all**, so it carries none of the original's `--settings` labels or hooks. Its name was an auto-generated title in run 1 and the original `--name` in run 2. |
| 2 | Copy. `--bg --resume <id>` of a live session **with** flags starts a copy with a new id, SessionStart `source: "fork"`, carrying only the flags passed. The original stays live. **The copy's row stays `done`/idle, not `failed` as on the wrapper build.** |
| 2c1 (new) | **Same id.** After `claude stop`, `--bg --resume <id>` with **no flags** continues the same session: "woke session X with its saved options (--name, --permission-mode, --settings)". Same session id, SessionStart `source: "resume"`, the original labels reach the hooks, no new row. |
| 2c2 (new) | **Copy, unlabelled.** After stop, `--resume <id> --name <same name>` starts a copy with a new id ("the flags you passed started a copy"). The copy fired no hooks: passing `--name` alone drops the saved `--settings`. |
| 2c3 | New id (copy). After stop, `--resume <id>` with all flags (`--name`, `--permission-mode`, `--settings`) starts a copy, SessionStart `source: "fork"`, with the new labels; the old row stays `done`. This is what the wrapper build's run measured as "2c"; the public build agrees. |
| 3 | Yes. The `env` labels from `--settings` reached every hook. Fired: SessionStart (`source: startup`), UserPromptSubmit, Stop; SessionEnd at stop (`reason: other`). No Notification in run 2 (run 1 saw one Notification per session after it sat idle, so it depends on timing). |
| 4 | Title only. The hook's `sessionTitle` reached later hook input as `session_title`; `claude agents --json` and the session file kept the `--name` value (`nameSource: "peer"`). |
| 5 | Allowed. Two live `--bg` sessions with the same name; neither got a variant name. |

Other facts seen (same as the wrapper build):

- A `--bg` session stays live and `idle` after its prompt finishes.
- `claude agents --json` fields: `cwd, id, kind, name, pid, sessionId, startedAt, state, status`.
- Session file fields: `cwd, entrypoint, jobId, kind, messagingSocketPath, name, nameSince, nameSource, peerFeatures, peerProtocol, pid, pidDomain, procStart, sessionId, startedAt, status, statusUpdatedAt, updatedAt, version`.
- `stop` and `rm` worked with the plain `claude` on PATH; `cleanup.rows_left` was empty after both runs.

## Differences from the wrapper build

1. **Home-folder trust.** On the wrapper build the home folder was a trusted parent of the probe
   folder. Here accepting the trust prompt in `~` was not persisted, so the probe
   folder needed its own trust. This may be a property of the public build or of
   this machine's history; it was not tested further.
2. **State of copied rows.** On the wrapper build the copies made by resuming a live session
   ended `failed`; here they stayed `done` and idle with a live pid.

Everything else matches, including the copy notes, which the wrapper build's output already
printed word for word ("Without flags, the same command continues X itself"). The
wrapper build's "2c: new id" was caused by the old probe passing flags, not by the build.

## What this changes in the plan

1. **Still cannot choose a session id** (1, 2a). Conclusion 1 stands, now on firmer
   ground: `--resume <unknown uuid>` looks like it works but the session never runs.
2. **A relaunch can keep the id** (2c1). Plan conclusion 2 ("every relaunch makes a
   new id") is wrong for the flag-free case: `claude --bg --resume <id> [prompt]`,
   from the ship's folder, wakes the same session with its saved name, permission
   mode and settings (labels and hooks). Lineage can stay one id per ship.
3. **Any flag on resume makes a copy, and a partial set of flags loses identity**
   (2c2, 2c3). The copy keeps only the flags passed: `--name` alone gives a copy with
   no labels or hooks. Changing a ship's labels or settings therefore means a full
   relaunch (all flags, new id, old row to remove), never a partial one.
4. **Never resume a live ship** (2, 2b). Even with no flags it makes a copy, and a
   flag-free copy has no labels, so label-based identity cannot see it.
5. **Trust NIMITZ_HOME explicitly** (0). Do not rely on the home folder being a
   trusted parent: `nimitz init` should have the user run `claude` once in
   `NIMITZ_HOME`.

## What was done

1. Cloned the repo; read `P0-requirements.md`, the wrapper build's findings and
   raw JSON, and `p0_probe.py`.
2. Found from the wrapper build's raw output that probes 2 and 2c only tested the copy path:
   the script passed `--name`, `--permission-mode` and `--settings` on every
   resume, and the CLI itself said "the flags you passed started a copy ... Without
   flags, the same command continues X itself". Probe 2 also resumed an id that did
   not exist (probe 1's ignored `--session-id`).
3. Revised `p0_probe.py` (user-approved): added `resume_plain()` (resume with no
   flags), new probe 2a (resume an unknown id), 2b (live resume, no flags), kept 2
   as "live resume with flags", and split 2c into 2c1 (no flags), 2c2 (`--name`
   only) and 2c3 (all flags). Probes 0, 1, 3, 4, 5 are unchanged.
4. Trust: the home folder's trust did not persist; the user trusted
   `~/.cache/nimitz-probe` directly.
5. Run 1 completed and cleaned up, but showed that 2b/2c picked hook events by time
   only, so 2c2 caught a stray event from another probe session. Fixed the filters
   to the probe's own labels, added `ran_a_turn` to 2a and `copy_rows` to 2c, and
   ran again. This file and the JSON are from run 2; run 1 agreed on every fact
   except the timing-dependent Notification.

## P0.6 to P0.8 (`probes/p0_probe2.py`)

Run on 2026-10-04 with **2.1.289** (Claude Code updated itself after round 1; the wrapper
build measured these on 2.1.288). Raw data: `results2-public-2.1.289-subscription.json`.
`cleanup.rows_left` empty; the private tmux server `nimitz-probe` was gone afterwards.

| Id | Result |
| --- | --- |
| P0.6 | Labels reach the Bash tool's processes. A `--bg` session ran `env \| grep NIMITZ_ \| sort` with its Bash tool; the output had `NIMITZ_ID=PROBE/p6`, `NIMITZ_PROBE=p6`, `NIMITZ_ROLE=ship` from `--settings`. |
| P0.7 | PreToolUse sees `SendMessage` and can block it. Tool name `SendMessage`; `tool_input` has `to`, `recipient`, `message`, `content`, `summary`, `type`. The session first loaded the tool with `ToolSearch` (deferred tool). With `permissionDecision: "deny"` PreToolUse fired and PostToolUse did not. |
| P0.8 | `claude -c` keeps the session id for an interactive session across a restart: SessionStart `startup`, then `resume`, same id. |

PreToolUse input fields: `cwd, effort, hook_event_name, permission_mode, prompt_id,
**scratchpad_dir**, session_id, tool_input, tool_name, tool_use_id, transcript_path`.

### Differences from the wrapper build (round 2)

1. **Version:** public 2.1.289 against the wrapper build's 2.1.288.
2. **PreToolUse input has `scratchpad_dir`**, which the wrapper build's field list does not.
   Likely the version, not the build. `nimitz` should not depend on the exact field set.

All three results match the wrapper build.
