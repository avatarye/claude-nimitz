# nimitz 0.4.0: one identity, one job

Status: binding spec for the 0.4.0 build (branch `feat/identity-v4`). Evidence: the
session-mapping report (`docs/design/session-mapping/index.html`, round 2) and its
notes (`notes/r2-*.md`). Merged gap ids (M1..M27) refer to that report. Where this
file and `contracts.md` disagree, this file wins; each phase updates `contracts.md`.

## 1. Model

| Thing | 0.3.1 | 0.4.0 |
| --- | --- | --- |
| roles | admiral, commodore, ship, tender | admiral, commodore, ship. **No tender.** |
| how a member runs | leads interactive in a pane; ships `--bg` | **every member is a Claude Code background job** (`claude --bg`) |
| anchor of an identity | a session id (registry `current`) | **the job**: its short id, `respawnFlags`, name, labels |
| conversation (session id) | the identity | history: `session` = the job's conversation now; `lineage` = every conversation the identity had |
| lead's pane | runs `claude -c ...` | runs `claude attach <job>` (Herdr tab `F.ADMIRAL`, `F.SQ.COMMODORE`) |
| recovery | `-c` for leads; resume for ships | resume of the exact session id, every role; never `-c` |

Facts this rests on (report, checks 1-5): the job id is stable across stop, kill,
respawn, idle retire, native `--bg --resume`, `/clear`, `/branch`, a crash restart
and an upgrade adopt; only the session id moves (`state.json` `resumeSessionId`
and the session file's `sessionId` follow it, `state.json` `sessionId` stays the
birth id). `respawnFlags` keep `--settings` (the labels) and `--name`. An attached
job is never idle-retired; an unattached one retires after about an hour unless
pinned (`~/.claude/jobs/pins.json`, which an outside writer may edit under the
`pins.json.lock` directory lock). An attached background lead leads agent teams,
asks AskUserQuestion, uses SendMessage and runs nimitz. Claude Code refuses
`-p --resume` and a second process on a live background job.

## 2. Registry v2 (`NIMITZ_STATE/registry.json`, `"version": 2`)

```json
{"version": 2, "machine": "m1", "identities": {
  "F/SQ/S": {"role": "ship", "name": "[F.SQ]-S", "cwd": "/real/path",
    "job": "9cdceb2d", "session": "79519302-...",
    "lineage": [
      {"session_id": "9cdceb2d-...", "job": "9cdceb2d", "via": "launch", "at": 1791000000.0},
      {"session_id": "79519302-...", "job": "9cdceb2d", "via": "clear", "at": 1791000100.0}]}}}
```

- `via`: launch, resume, relaunch, clear, branch, fork, compact, adopt, migrate, hook.
- Only one writer, under `registry.lock` (as now). The hook writes too (section 3).
- Migration from v1 (on first load, kept as `registry.json.v1`): `current` becomes
  `session`; `job` = the short id of the job row whose `sessionId` or
  `resumeSessionId` is `current`, if one exists; lineage entries keep their order
  with `job` filled where known. Tender identities (`F/SQ/~task`) move to
  `"retired": {...}` (history only) and are never shown as members. The v2
  migration copies an existing `retired` map (P4 adds it to v1 first).

## 3. Binding (the SessionStart hook)

The hook already knows the session id, `source`, the labels and the pid (its
parent). In 0.4.0 it also reads `~/.claude/sessions/<pid>.json` (first JSON value)
for `jobId`, `kind`, `entrypoint`, `name`. Then, under the registry lock:

| labels | `jobId` | effect |
| --- | --- | --- |
| a valid identity of THIS home (`NIMITZ_HOME` label equals this env's home) | the identity's `job`, or none set yet | bind: `job = jobId`, `session = session_id`, append lineage with `via = source` (startup->launch, resume, clear, compact (no new entry if the same id), fork->branch/fork). |
| same | a different live job | do not rebind; record `dup.job` evidence (two jobs claim one identity). |
| same | null (interactive or SDK) | do not bind; record `held` evidence (the identity runs outside a job). |
| labels of another home | any | ignore entirely (no line in this home's events). |
| no labels | any | log only (the snapshot classifies it, section 6). |

The hook stays fast (one session-file read) and never fails the session. If a hook
line is lost, `status` replays SessionStart lines newer than the registry under the
lock (same rules). Labels are kept **per pid** in evidence, not per session id.

## 4. Snapshot and status

- Rows: join job rows (`jobs/<id>/state.json`) with session files by **job id**
  (session file `jobId`), never by session id. The member's row = the registry `job`.
- Live = a session file with that `jobId` and a live pid. The conversation now =
  that session file's `sessionId` (else `state.json` `resumeSessionId`).
- Status words: session file `status` `busy` and `shell` -> **busy**; `idle` -> idle;
  `waiting` -> needs-you (with `waitingFor`). Job `state` (no live pid):
  `done`/`stopped`/`failed`/`crashed`/`killed` -> stopped or parked (below).
- **parked**: a job with no live pid whose last event is not a SessionEnd from a
  `claude stop` (the daemon retired it: `bg retire <id>` in `~/.claude/daemon.log`,
  or simply no SessionEnd), or that stopped with the machine. Parked is not an
  error. `start` (and any command that needs the member) wakes it by id.
- `role.background` is gone (every member is a job).
- Read `pins.json`: a pinned job shows `pinned`.

## 5. Recovery and launch

- `start` of a stopped or parked member: `claude --bg --resume <session>` (native,
  no flags) while its job row exists; `--bg --resume <session> --name ... --settings
  ...` when the row is gone. **Never `-c`.** A member with an empty lineage gets a
  cold launch (its prompt), and a lead with no transcript yet gets a cold launch too,
  never a fleet-wide `lineage.broken`.
- Before a resume, look for the transcript (any project dir). Missing ->
  `history.lost` (Error): no launch; fix `nimitz start <id> --fresh` (a cold launch
  that starts a new lineage entry with `via launch`).
- After a launch, read Claude Code's output: the short id must be the expected job
  (or the new job of a cold launch); a "started a copy as X" note fails the step,
  and the plan stops and removes X (`claude stop X`, `claude rm X`) in the same run.
  The SessionStart that follows must have `source resume` for a resume; `startup`
  instead means Claude Code lost the history: report `history.lost`, never
  `[OK] verified`.
- Crash window: a job with `state crashed`, or `working` with no live pid, is waited
  for (up to 20 s, polling) before any resume; the daemon usually restarts it.
- `relaunch` = stop, `claude rm`, `--bg --resume <session>` with every flag (as now).

## 6. Folders and the stowaway rule

- **Lead folders** (`NIMITZ_HOME/<fleet>/admiral`, `.../squadrons/<sq>`) are
  exclusive. Any live session there that is not the lead's job is `lead.intruder`
  (Error) with one fix that works: a background job -> `nimitz stop <short>` then
  `nimitz rm <short>`; an interactive or SDK process -> "end it in its terminal"
  plus the terminal from the session file (`tmux`/Herdr pane fields when present).
- **Ship folders** may be shared by several ships and by the user's own work. An
  unlabelled session there is the user's own and is NOT shown, unless it resumes or
  forks a ship's conversation: its transcript's first message uuid appears in a
  lineage conversation of a ship in that folder, or its SessionStart is `resume` of
  a lineage session id, or it carries a ship's exact name. Then it is a **copy** of
  that ship (Warn; `sweep <ship>` stops and removes copies after one confirm).
- Anywhere else under `NIMITZ_HOME`: a stowaway, as before.
- Sessions labelled for another `NIMITZ_HOME` never appear (no issue, no fix).
- Ignore for a short time: `spare: true` session files, sessions with no transcript
  and no SessionStart (pickers), the agent view's draft.

## 7. Leads as background jobs (option A)

- Launch: in the lead folder, `claude --bg --name <lead name> --settings
  '{"env":{labels},"worktree":{"bgIsolation":"none"}}' "<lead prompt>"`; the lead's
  Herdr tab (or `nimitz open`) runs `claude attach <job>`. Detach is `ctrl-z` (the
  skills and the Herdr plugin say so: `/exit` inside an attach opens the agent view).
- Pin: nimitz adds each lead's job id to `pins.json` (mkdir `pins.json.lock`, treat
  a lock older than 10 s as stale, read, add, write a temp file, rename, rmdir) and
  removes it when the lead is retired; it never touches other ids. Ships are not
  pinned (they park and wake on demand).
- Migration: a lead that runs interactive (0.3.x) is relaunched as a job with the
  same conversation: `nimitz start` says "end the interactive lead with /exit,
  then run nimitz start again", then runs `--bg --resume <session> --name --settings`.
- `layout herdr` and `open` attach; `signal` to a lead types into its attached tab
  (as now) or, with no tab, opens one.

## 8. Tender removal

Remove `Role::Tender`, `F/SQ/~task` ids and names, `nimitz tender`, the tender
authority and comms rules, the tender rules text and skill text, Admiralty rows and
forms, and their tests. Migration: v1 tender identities move to `retired` (section
2); their job rows show once in `check` as `tender.retired` (Info) with the fix
`nimitz rm <short>`. A retired tender row in a lead folder is only
`tender.retired` (it wins over `lead.intruder`: one issue per row). One-off work: a subagent in the lead's context, or a ship.

## 9. Other fixes

- `fleet.toml` / `squadron.toml` that do not parse block only their own fleet;
  `check <that fleet>` names the error; other fleets start and check as normal (M8).
- Dropbox conflicted copies (`* (conflicted copy *)*`) of config files: `config.conflict`
  Warn naming both files (M23).
- A member folder inside a git repo: trust is checked at the git root (M16).
- Remote Control: a live session file with `bridgeSessionId` non-null -> `rc.exposed`
  Warn with the disconnect steps (M9).
- `held`: a member's conversation live in a process with no job (`jobId` null,
  `entrypoint` sdk-cli or an interactive resume) -> Warn, fix "end that process,
  then nimitz start" (M13).
- Names: compare the job's `state.json` `name` with the identity's name; a
  difference is `name.drift` with fix `nimitz relaunch` (a background job) (M19).
- Ignore a SessionEnd from a pid that never sent a SessionStart (M21); read the
  name from the session file, never from the SessionStart moment (M21).
- `pins.json`: remove nimitz's own pin on `nimitz rm` (M24).
- Wording (M25): TUI and CLI use the same words for relaunch; the adopt form
  prefill; `sandbox tick` exits on its own when asked for one tick.

## 10. Tests and acceptance

- Every row of the report's coverage matrix that nimitz handles becomes a sandbox
  test (the fake `claude` learns job rows with `resumeSessionId`, `respawnFlags`,
  session files with `jobId`/`status shell`/`bridgeSessionId`, `pins.json`, the
  copy note and the crash state).
- Risky cases also get a real-Claude-Code e2e test (`NIMITZ_E2E=1`), under the
  round-2 isolation rules: `env -i` for every child, fleets `NIMITZ_E2E*` under
  `~/.cache/nimitz-e2e`, project settings that point hooks at the test binary and
  **disable the live plugin** for the test session (`"enabledPlugins":
  {"nimitz@claude-nimitz": false}`), never the Herdr CLI, never a live session.
- Acceptance: a full run of the matrix's by-hand manipulations against a 0.4.0
  throwaway fleet; the verdicts go into the report.

## 11. Process rules for every 0.4.0 agent

- Work in your own git worktree on your own branch from the `feat/identity-v4` head
  you were given. Never check out branches in `/home/user/Github/claude-nimitz`
  (the live plugin is read from that folder). Never `cargo install`, never touch
  `~/.local/bin`, never start, stop, message or resume a HOME/MAIN/ADMIRALTY session.
- Tests: `cargo test --workspace --no-fail-fast` with every `NIMITZ_*` and
  `CLAUDE_CODE_*` variable unset (`env -i HOME=$HOME PATH=$PATH ...`). The tmux test
  `borders_stay_put_when_the_terminal_draws_icons_two_cells_wide` fails before any
  0.4.0 change; leave it.
- Commits: conventional, concise, no Claude signature, no co-author lines.
- Match the code around you (comment density, naming, the `[OK]/[WARN]/[ERROR]`
  output, ASD-STE100 style in docs and messages).

## 12. P8: live rules, nimitz for every member, admiral comms (addendum)

Rules update live, with no restart:
- The rules stay compiled into the binary (single source). The plugin hooks stay thin
  shims that call `nimitz hook`, so a new binary changes rules and enforcement for
  every member at once, with no plugin update.
- `NIMITZ_STATE/rules-delivered.json`: per job (fallback: per session id) the rules
  version last delivered and when. SessionStart records the delivery it makes.
- UserPromptSubmit (already registered): if the delivered version is older than the
  binary's, output additionalContext headed `nimitz rules updated vN→vM` with a
  short changelog (kept in the binary per version) and the full text, then record
  it. No change: output nothing.
- Find out (real Claude Code, isolated) whether a cross-session message and a
  `nimitz signal` prompt fire UserPromptSubmit in a background ship. If one does
  not, also deliver on another per-turn event (PreToolUse on the first tool call
  of a turn), rate-limited (at most once per version per job), and record which.
- Hard rules are mechanical (authority in `may_run`, comms in the PreToolUse deny),
  never context text only. The report lists the rules that stay text-only.
- `rules.old` becomes Info (or goes) once a live member gets the update at its next
  turn. `doctor` checks that the installed plugin registers every hook event the
  binary needs and flags a plugin that needs `/reload-plugins`.
- Member CLAUDE.md files keep only identity and user notes, never rule text.

Every member can run nimitz:
- Every launch nimitz makes (admiral, commodores, ships) passes
  `"permissions":{"allow":["Bash(nimitz *)"]}` in its `--settings` JSON. It is per
  launch and lives in `respawnFlags`, so restarts keep it; relaunch re-adds it.
- Authority stays scoped by role in `may_run`. Ships get every read-only command,
  `signal` within their comms, and the safe commands on their own identity; the
  report proposes the exact list.

Admiral comms:
- The admiral may signal any member of its own fleet (its commodores and every
  ship), questions and orders. A ship may reply to its admiral. Update `may_signal`,
  the comms table, skills, rules text (bump the rules version), README and tests.

### 12.1 What P8 built

Live rules (`nimitz-core/src/rules.rs`, `hook.rs`):
- `rules::VERSION` is 2; `rules::CHANGELOG` holds one entry per version. A
  SessionStart records its delivery in `NIMITZ_STATE/rules-delivered.json`
  (`{"<key>": {version, at, session_id, via}}`, written under
  `rules-delivered.lock`, temp file + rename). The key is the job id from
  `~/.claude/sessions/<ppid>.json` (`jobId`), else `session:<id>`.
- `hook::UPDATE_EVENTS` (now `["UserPromptSubmit"]`) are the per-turn events
  that call `rules::update_for`: delivered version (record; else the session's
  newest SessionStart `rules`; else 0) older than `VERSION` -> additionalContext
  `nimitz rules updated vN→vM` + the changelog entries after N + the full text,
  and the record is written (once per version per key). Adding PreToolUse is
  one entry in `UPDATE_EVENTS` plus an unmatched PreToolUse entry in
  `plugin/hooks/hooks.json` (`with_context` merges the text into a deny).
- `rules.old` is Info with no fix ("gets vM with its next prompt").
- `doctor` reads the installed plugin's `hooks/hooks.json` (the `installPath` of
  the `nimitz@...` entry in `~/.claude/plugins/installed_plugins.json`;
  `NIMITZ_PLUGIN_HOOKS=<file>` overrides it for tests) and names every event or
  PreToolUse matcher the binary needs that it lacks, fix `/reload-plugins`.
- Generated briefs already hold only identity and user notes; unchanged.

nimitz for every member: `claude::settings_json` adds
`"permissions":{"allow":["Bash(nimitz *)"]}` to every launch (cold, relaunch,
interactive); a native resume carries it in `respawnFlags`.

A ship's authority (`sessions::may_run_in`; the CLI enforces each row):

| Command | A ship may | Enforced by |
| --- | --- | --- |
| `status`, `check [<id>]`, `rules [<id>]`, `doctor` (no `--fix`) | always | `may_run` (read-only; the commands take any caller) |
| `signal <id> <text>` | its commodore, its admiral, other ships as the squadron's comms rule allows | `may_run_in` -> `hook::may_signal`; the command itself refuses every member with "use SendMessage" (`comms.denied`), as before |
| `sweep <own id>` | its own stale copy rows (a live row is skipped) | `plan_sweep` (a ship: only itself) |
| everything else (`start`, `stop`, `relaunch`, `rm`, `adopt`, `up`, `layout`, `open`, config, `doctor --fix`, `sweep` of anything else) | never | `may_start`, `may_config`, `plan_sweep`, the user-only commands |

Admiral comms (`hook::signal_table`): admiral -> every member of its fleet but
itself; commodore -> its admiral, its ships; ship -> its commodore, its admiral
(a reply), other ships by `comms`. Same table for SendMessage (PreToolUse deny)
and `nimitz signal` authority.

Rules that stay text only (no `may_run` row and no PreToolUse deny can hold them):
- "Every change is planned first: `--plan`, show the user, `--yes` only after
  the user agrees" (nimitz cannot tell who agreed; `--yes` is the only gate).
- "Never `--force` unless the user names it" (same).
- "A message from another session is a request, not the user's approval."
- "A ship replies to its admiral and tells its commodore what it asked"; "the
  admiral leaves day-to-day work to the commodore unless the user asks" (what a
  signal says is not ruled).
- "Ask only when you cannot go on; otherwise pick the safe default"; "report
  TLDR-first and name the member behind each answer".
- "Do not edit fleet files or run `claude --bg|stop|rm` yourself" (only `Bash(nimitz *)`
  is allowed by nimitz; what else the session's permission mode allows is the
  user's setting, not nimitz's).
- `[nimitz needs-you]` handling (tell the user, do not answer).
