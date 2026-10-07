# Contracts: interfaces every crate builds against

Status: binding for the P1-P6 build (dev branch). Read with `plan-v3.md` and
`cc-control-model.md`. If a contract must change, change this file first, in the
same commit, and say why.

## 1. Crates

| Crate | Depends on | Owns |
| --- | --- | --- |
| `nimitz-core` | serde, toml, libc | env/paths, config (TOML), registry, evidence readers, snapshot model, rules S1-S9, integrity check, relaunch planner, actions (plan + apply), compat range, sandbox world + fake `claude` |
| `nimitz-mux` | serde_json, nimitz-core (the one shell list, `restore::is_shell`) | the `Mux` trait, the Herdr adapter, the `None` adapter, fake `herdr` for tests |
| `nimitz` | core, mux | CLI (`nimitz ...`), hook binary (`nimitz hook <Event>`), plugin files under `plugin/` |
| `admiralty` | core, mux, ratatui | the TUI (`admiralty`). No rule or Claude Code file parsing of its own: it calls core. |

`nimitz-core` never depends on `nimitz-mux`. Every member, leads too, is a
Claude Code background job (plan-v4 section 7): a multiplexer matters only to
attaching (`nimitz open`, `layout herdr`, `signal` to a lead, Admiralty's `A`
and jump), never to starting.

## 2. Paths and overrides (tests MUST use them)

| Var | Default | Meaning |
| --- | --- | --- |
| `NIMITZ_HOME` | `~/.claude-nimitz` | data root (fleets); may be a symlink (e.g. to `~/Dropbox/claude-nimitz`). Paths are shown as written; every comparison with a session's cwd, every trust check and the stowaway scope use real paths (Claude Code records real paths) |
| `NIMITZ_STATE` | `~/.local/state/nimitz` | per-machine registry and events; never synced |
| `NIMITZ_CLAUDE_HOME` | `~/.claude` | Claude Code's home (sessions/, jobs/, projects/) |
| `NIMITZ_CLAUDE_BIN` | `claude` on PATH | cold launches (on ACME the wrapper, which adds Bedrock settings) |
| `NIMITZ_NATIVE_CLAUDE_BIN` | newest `~/.localtools/tools/claude-code/*/claude`, else `claude` on PATH | resume, stop, rm, agents |
| `NIMITZ_HERDR_BIN` | `herdr` on PATH (only when `HERDR_ENV=1`) | Herdr adapter |
| `NIMITZ_HERDR_SESSION` | unset (Herdr's own resolution: `HERDR_SESSION`, else the pane's socket) | the named Herdr session every herdr call gets as `--session`; with `NIMITZ_MUX=herdr` it drives a test server from a plain terminal (the checklist), and it is the Herdr identity `active.json` records |
| `NIMITZ_HOSTNAME` | `gethostname()` | machine matching |
| `NIMITZ_MUX` | auto | `herdr` or `none` |
| `NIMITZ_CRASH_WINDOW` | `20` (s) | how long a start waits for the daemon to restart a crashed job (state `crashed`, or `working` with no live pid) before it resumes the job itself; tests set it low |
| `NIMITZ_HOOK_PID` | `getppid()` | the hook's Claude Code process (test override: the sandbox's fake `claude` names a stable fake pid per job) |
| `NIMITZ_HOOK_SESSION_FILE` | `<claude home>/sessions/<pid>.json` | the session file the SessionStart hook reads (test override) |
| `NIMITZ_PLUGIN_HOOKS` | the `installPath` of the `nimitz@...` entry in `~/.claude/plugins/installed_plugins.json`, plus `hooks/hooks.json` | the installed plugin's hooks file `doctor` checks; tests point it at a file |

## 3. Config files (TOML, in `NIMITZ_HOME/<fleet>/`)

`fleet.toml`:

```toml
fleet = "MAIN"                      # uppercase, [A-Z0-9_]+
[machines.west-mac]                 # label -> hostname
hostname = "3f2c1d0e9b7a"
os = "darwin"
[admiral]
permission_mode = "auto"            # optional
```

`squadrons/<SQ>/squadron.toml`:

```toml
squadron = "ARGOS"                  # uppercase, unique in the fleet
brief = "SQUADRON.md"
comms = "open"                      # "open" | "commodore-only" | { allow = ["MAIN/BOLT/BOOM"] }
permission_mode = "bypassPermissions"   # default for ships

[[ships]]
name = "ARGOS"                      # uppercase
prompt = "You are ..."              # or prompt_file = "prompts/ARGOS.md"
depends_on = []
[ships.dirs]                        # machine label -> folder; "managed" = NIMITZ_HOME/<fleet>/ships/<SQ>/<SHIP>
west-mac = "/Users/.../AcmeAssetGen"
home-server = "managed"
[ships.permission_mode]             # optional per machine
```

A ship with no `dirs` entry for this machine does not run here. A `dirs` value
is `managed`, an absolute path or `~/...` (`config::normalize_ship_dir`; empty,
relative and `<label>=<folder>` values are refused, for every front end). Ship names
`COMMODORE` and `ADMIRAL` (and anything starting `COMMODORE/`) are reserved: they
would take a role's address. Folders are compared as real paths (symlinks
resolved, as Claude Code records them); one folder holds one member.

A `squadron.toml` that does not parse leaves only that squadron out: the rest of
the fleet loads, the squadron is `config.invalid` (file:line:col), its live
members stay visible as members whose config is unreadable, and config writes to
the fleet refuse until the file is fixed. A `fleet.toml` that does not parse
leaves only that fleet out the same way (M8): the issue carries the fleet's
admiral id, so `check <F>` and every command aimed at the fleet refuse with it
(fix `fix <file> line N, then nimitz check <F>`), the fleet still shows in
`status` (from its folder, the registry and the labels) with the error once, and
the other fleets load, check and start as normal. Dropbox conflicted copies
(`<name> (... conflicted copy ...).<ext>`) of `fleet.toml`, `squadron.toml`,
`legacy.toml` or a member's `CLAUDE.md` are `config.conflict` (Warn, both files
named; fix: compare the two files, keep one, delete the other) (M23).
`machine add <F> <label>` records this host only when no other label of the
fleet names it; otherwise `--hostname H`.

## 4. Identity, names, labels

| Role | Identity id | Name (address) |
| --- | --- | --- |
| admiral | `F` | `[F]-ADMIRAL` |
| commodore | `F/SQ` | `[F.SQ]-COMMODORE` |
| ship | `F/SQ/SHIP` | `[F.SQ]-SHIP` |

The tender role of 0.3.x (`F/SQ/~task`, one job for a commodore) was removed
in 0.4.0: a v1 registry's tender identities move to the registry's `retired`
map on load (history only, never members), a job row with tender labels is
`tender.retired` (Info, fix `nimitz rm <short>`; `nimitz stop <short>` first
while it is live).

Labels (in `--settings '{"env":{...}}'`): `NIMITZ_FLEET`, `NIMITZ_SQUADRON`,
`NIMITZ_ROLE` (`admiral|commodore|ship`), `NIMITZ_ID`, `NIMITZ_HOME`. Every
`nimitz` launch is `claude --bg` and sets all of them, plus
`"worktree":{"bgIsolation":"none"}` and the `Bash(nimitz *)` allow
(`claude::settings_json`). A lead's cold launch carries `claude::lead_prompt`
(who it is, what to do first); a ship's the prompt from `squadron.toml`.

## 5. Registry and events (`NIMITZ_STATE`)

`registry.json` (atomic write: temp + rename; `registry.lock` with `flock`; every
save also writes `registry.json.bak`, the last good copy). A `registry.json` that
does not parse is never replaced in silence: it is kept as
`registry.json.corrupt-<ts>` (with a `.txt` note), the registry is rebuilt from
the `.bak` and the SessionStart lines in `events.jsonl` (the same binding rules
as the hook, `registry::apply_event`), and `check` reports `registry.corrupt`
until the kept file is removed. Version 2 (plan-v4 section 2): the anchor of an
identity is its **job** (the Claude Code short id); `session` is the conversation
it runs now; `lineage` every conversation it had:

```json
{"version": 2, "machine": "west-mac", "updated_at": 1791000100.5, "identities": {
  "MAIN/ARGOS/ARGOS": {"role": "ship", "name": "[MAIN.ARGOS]-ARGOS", "cwd": "/...",
    "job": "9cdceb2d", "session": "79519302-...", "lineage": [
      {"session_id": "9cdceb2d-...", "job": "9cdceb2d", "via": "launch", "at": 1791000000.0},
      {"session_id": "79519302-...", "job": "9cdceb2d", "via": "clear", "at": 1791000100.0}]}}}
```

`via`: launch, resume, relaunch, clear, branch, fork, compact, adopt, migrate.
`updated_at` is when the file was last written (`status` replays newer
SessionStart lines, below). A 0.3.x file (`version` 1, `current`, `bound_at`,
no `job`) is migrated on load (`migrate::registry_v2`): `current` becomes
`session`, `job` is filled from the job row whose `state.json` `sessionId` or
`resumeSessionId` is that session (each lineage entry the same way), tender
identities move to `retired`. A read never writes; the next locked `update`
saves the v2 shape and keeps the old file once as `registry.json.v1`.

The registry's `machine` is this machine's label, set by `nimitz init --label`;
it is per machine and outlives every fleet (`registry::machine_label`).

`events.jsonl` (append, one line per hook call, `O_APPEND`):

```json
{"ts": 1790900000.1, "event": "SessionStart", "session_id": "...", "source": "startup",
 "reason": null, "agent_id": null, "tool_name": null, "to": null, "pid": 12345,
 "labels": {"NIMITZ_ID": "MAIN/ARGOS/ARGOS", "NIMITZ_ROLE": "ship", "...": "..."}, "rules": 1,
 "job": "9cdceb2d", "kind": "bg", "entrypoint": "cli", "binding": "bound"}
```

`pid` is the hook's parent (the Claude Code process); when the parent has no
session file but its own parent has one, that one (a hook given as one command
string runs under `sh -c`, so the parent is a shell: `hook::claude_pid`, one
step only; `doctor` warns about such an entry in the installed plugin). The
hook never blocks,
never fails the session (always exit 0), and returns in a few ms. Labels are
kept **per pid**: a second process on a member's conversation (a `-p
--resume`, an SDK host) does not strip the member's own process of its labels
(`evidence::row_labels`).

**Binding (SessionStart).** The hook reads `<claude home>/sessions/<pid>.json`
(the first JSON value) for `jobId`, `kind`, `entrypoint` and `name`, and
writes `job`, `kind`, `entrypoint` and `cwd` in the line. A missing or partial
file binds nothing now: the line says `kind` `unknown`, `binding` `none`, and
`status` replays it, judging the job from the row whose `state.json` runs the
session. Then, for labels that name a valid identity of this home
(`NIMITZ_HOME` absent or equal to this env's home, as written or as a real
path; role and id agree; not a 0.3.x tender), under `registry.lock` (waited
for at most 2 s; a lock it cannot take, or a registry it cannot read or write,
is `binding` `none`), `registry::apply_start` says what happened:

| labels | session file | effect | `binding` |
| --- | --- | --- | --- |
| this home's identity | `jobId` = the identity's job, or none set yet | `job = jobId`, `session = session_id`, lineage appended with `via` from `source` (startup -> launch; resume; clear; compact: no new entry for the same conversation; fork -> branch in the same job, else fork). A wake (resume, relaunch) of the conversation it already runs adds no entry and never overwrites how it was born: after a stop and a start the lineage is still `[launch]` (or `[launch, clear]`) | `bound` (`same` when nothing moved) |
| same | another `jobId` whose job row still exists | nothing rebound: two jobs claim one identity | `dup.job` |
| same | `jobId` null (interactive or SDK) | nothing bound: a process with no job holds the conversation | `held` |
| same | a `cwd` that is not the identity's folder (the registry's `cwd`, else the config) | nothing bound: an impostor (BUG-30, BUG-55) | `elsewhere` |
| labels of another `NIMITZ_HOME` | any | ignored by every reader. The hook itself cannot tell (its env is the session's env, and the `--settings` label wins over a project settings env, so `env.home` is the label): with one `NIMITZ_STATE` shared by two homes it writes its line and binds the identity there. `registry::load` drops an identity whose job row's saved labels name another home (`drop_other_homes`), so no reader uses that record (r3-e2e.md finding 1) | - |
| no labels | any | the line only (the snapshot classifies it) | - |

The name is never taken from the SessionStart moment: the snapshot reads it
from the session file (M21). A SessionEnd from a pid that never sent a
SessionStart (a refused `-p --resume` fires one with no labels) is ignored by
every reader (M21). The hook takes one clock for the line's `ts` and the
lineage `at`, and writes the registry before the line, so a line it bound is
older than `updated_at`. If a hook's registry write is lost (`binding` `none`),
`status` replays SessionStart lines newer than `updated_at` in time order under
the lock with the same rules, the folder rule included (`registry::replay`; a
line the hook judged `dup.job`, `held` or `elsewhere` is not replayed); it
writes only when a line binds something. A replayed line that bound nothing
(judged held) is not probed again: `NIMITZ_STATE/replay.json` keeps the `ts`
of the newest line judged, at least 5 s old (a late hook line, which takes its
`ts` before waiting for the lock, is never hidden behind it), and lines at or
before it are skipped. A registry file that is there but
cannot be read (EACCES, EIO) is never written over: `update` fails, a read
sees a fresh registry. A nimitz launch
binds the started session itself (`sessions::bind_started`, with the job that
runs it and nimitz's own `via`), so an impostor's job never keeps the identity.

Role rules: a labelled session's SessionStart (startup, resume, clear, compact,
fork) gets its role's rules (`nimitz-core/rules/*.md`, compiled in; `nimitz
rules <id>` prints them) as `additionalContext`, and the line records `rules`
(the version, `rules::VERSION`, with one `rules::CHANGELOG` entry per version).
Rules update live: `rules-delivered.json` (`{"<key>": {version, at,
session_id, via}}`, under `rules-delivered.lock`, temp file + rename) records
per job (`jobId` of `~/.claude/sessions/<ppid>.json`; else `session:<id>`)
the version last delivered. On a per-turn event (`hook::UPDATE_EVENTS`:
UserPromptSubmit) of a labelled session whose delivered version (the record,
else its newest SessionStart `rules`, else 0) is older than the binary's, the
hook outputs `additionalContext` `nimitz rules updated vN→vM` + the changelog
after N + the full text, once per version per key; else nothing. No record,
no update: the hook tries `rules-delivered.lock` for 1 s, and when the record
cannot be written (lock held, state folder read-only) it hands nothing out
rather than the same text at every prompt; `check` shows `rules.old` (and
`registry.readonly`) until the record is written. `check`
reports `rules.old` (Info, no fix) for a live member still on older rules, and
`brief.old` for an admiral's or commodore's CLAUDE.md that an older nimitz
generated; `doctor --fix` replaces those and leaves a hand-edited brief alone.
A brief holds only who the member is and the user's notes. `doctor` reads the
installed plugin's `hooks/hooks.json` and names every event (and PreToolUse
matcher) the binary needs that it lacks (fix: `/reload-plugins`).

The chain-of-command signal table (`hook::may_signal`; the PreToolUse deny on
SendMessage and the authority of `nimitz signal`):

| From | May signal |
| --- | --- |
| admiral | every member of its own fleet (commodores and ships), not itself |
| commodore | its admiral, its ships |
| ship | its commodore; its admiral (a reply); other ships as the squadron's `comms` rule allows |

Every launch's `--settings` carries `"permissions":{"allow":["Bash(nimitz *)"]}`
(`claude::settings_json`); authority stays by role in `sessions::may_run_in`:
a ship gets `status`, `check`, `rules`, `doctor` (no `--fix`), `signal` within
the table above, and `sweep` of itself.

`needs/<id with / as %2F>.json`: the needs-you notice of a ship that
blocks on the user (PreToolUse `AskUserQuestion`, `PermissionRequest`, or a
blocking `Notification`): `{ts, id, session_id, kind: question|permission|prompt,
event, tool_use_id, detail, delivery: pending|delivered|failed, delivery_note,
delivered_at}`. The hook writes it and starts `nimitz __notify <id>` detached
(only the member's own labels may run it), which types the notice into the
commodore's pane, retrying `NIMITZ_NOTIFY_RETRY` s (default 600) while that pane
is blocked or missing. The member's next UserPromptSubmit, Stop or SessionEnd
removes the file; `check` reports `notify.failed` for one that failed.

## 6. Evidence and state (core)

Order: (1) session file + `kill(pid,0)` (read the first JSON value only),
(2) `claude agents --json --all` (native binary, every ~10 s, cached),
(3) events (binding, crew, end reason), (4) mux panes (Admiralty only).

Rows (plan-v4 section 4): a job row (`jobs/<id>/state.json`) and a live
session file join by **job id** (the session file's `jobId`), never by session
id. A member's row is the registry's `job`; the conversation it runs now is
the live session file's `sessionId`, else `state.json` `resumeSessionId`, else
its birth `sessionId` (`SessionInfo::birth_id` keeps the birth id when the
conversation moved: `/clear`, `/branch`). A live process with no job
(interactive, SDK) is its own row, even on a job's conversation. The CLI
cross-check joins the same way. Not rows at all (plan-v4 section 6): a
background job of another `NIMITZ_HOME` (its `respawnFlags` labels say so,
`registry::other_home`; the hook writes no line for it either: no issue, no
fix, no row). An interactive or SDK session of another home carries no labels
anywhere nimitz can read (its labels live in that process's env only, and the
hook of this home writes no line), so it cannot be recognised: it is
classified as an unlabelled session (the user's own in a ship folder, a
stowaway elsewhere in scope). Also not rows, for
`snapshot::TRANSIENT_GRACE_S` (120 s) after its session file appeared (the
earliest of the file's birth time, its mtime and `startedAt`), a `spare: true`
session file (the agent view's "new session" or draft) and a picker (a live
process with no job, no transcript and no SessionStart yet); after the grace
time they are classified like any other row. `jobs/pins.json` gives `pinned` (the daemon
never idle-retires a pinned job; `status` says so). A live session file's
`tmux` (or a Herdr pane field) is the row's `terminal`, for the fix that says
where to end a process. nimitz pins each lead's job after it starts and
removes that pin when the lead's row goes (`claude rm` leaves it behind,
M24), under Claude Code's own lock for the file: the directory
`jobs/pins.json.lock` (`mkdir`; Claude Code's proper-lockfile stale time is
5 s, the holder refreshing the mtime every 2.5 s; nimitz takes a lock over
only when it is 10 s old, twice that, so it never takes a live one), read,
add or remove, temp file, rename, `rmdir`. `NIMITZ_STATE/pins.json` lists the ids
nimitz pinned; it never touches another id (`pins::pin`, `pins::unpin`). Ships
are not pinned: they park and wake on demand.

Snapshot types (serde-serializable; `nimitz status --json` prints them):

```
Snapshot { taken_at, machine, fleets: [Fleet], stowaways: [Member], issues: [Issue], cc_version, compat }
Fleet { id, admiral: Member, squadrons: [Squadron], issues }
Squadron { id, commodore: Member, ships: [Member], comms, issues }
Member { id, role, name, expected_name, cwd, status, reasons: [String], session: Option<SessionInfo>,
         crew: [Crew], tasks: [Task], issues: [Issue], pane: Option<PaneRef> }
Status = NotStarted | Stopped | Parked | Idle | Busy | NeedsYou | Unknown
SessionInfo { session_id, short_id, pid, kind, state, status, waiting_for, started_at, status_since, name,
              cwd, labels, birth_id, entrypoint, pinned, terminal }
Issue { code, severity: Info|Warn|Error, message, fix: Option<String> }   // fix = a nimitz command
```

`reasons` says why the status is what it is (e.g. "session file: status waiting,
waitingFor permission prompt"). Unknown is allowed; a guess is not. Session file
`status`: `busy` and `shell` (an idle turn with background work running) are
Busy, `idle` Idle, `waiting` NeedsYou, anything else Unknown. Every job `state`
Claude Code writes (starting, running, resuming, adopted, crashed, working,
blocked, done, stopped, failed, or a newer word) is read as it is: a row with no
live pid is **Stopped** when the last event of the job's own processes (the
SessionStart lines that name the job, and the SessionEnd lines from those
pids; the session id alone decides only for a line an older nimitz wrote) is a
SessionEnd (`claude stop`, `/stop`, an exit) or its state is `failed`, else
**Parked** (the daemon retired it: a `[2026-10-07T01:47:03.070Z] [bg] bg retire
<id>: <why>` line in the Claude home's `daemon.log` newer than the job's last
start, named in the reason; `<why>` is any text after the colon, as seen:
`settled, idle 61m`, `idle-prompt, idle 61m`, `abandoned-stale, idle 48h, worker
2.1.289 (daemon 2.1.292)`; it crashed; or the machine went down: no SessionEnd
fires for those).
Parked is not an error; `start` wakes it by id.
A live row goes by its session file.
Session files of `kind` `daemon` or `daemon-worker` are Claude Code's own
processes: never a row, so never a member or a stowaway.

## 7. Integrity check (`nimitz check`, run before every start)

Each finding is an `Issue` with a stable `code`:

| Code | Condition |
| --- | --- |
| `config.invalid` | TOML does not parse (file:line:col; a bad squadron.toml hides only its squadron, a bad fleet.toml only its fleet, on the fleet's admiral id; fix `fix <file> line N`) or breaks a schema rule (names, reserved ship names, duplicates, unknown keys) |
| `config.conflict` | a Dropbox conflicted copy next to `fleet.toml`, `squadron.toml`, `legacy.toml` or a member's `CLAUDE.md` (Warn; names both files; fix: compare the two files, keep one, delete the other) |
| `config.layout` | an inherited instructions file (`CLAUDE.md`, `CLAUDE.local.md`, `.claude/CLAUDE.md`) in `NIMITZ_HOME/`, `<fleet>/`, `<fleet>/squadrons/`, `<fleet>/ships/` or `<fleet>/ships/<sq>/` (Error); two members with one folder (Error); a member folder inside a nimitz-managed folder (admiral, commodore, managed ship) (Error); a member folder inside a user project folder that has a `CLAUDE.md` (Warn). A member folder inside a user folder without one (e.g. a ship at `~`) is fine |
| `folder.missing` / `folder.untrusted` | a member's folder does not exist / is not trusted. Trusted: inside `NIMITZ_HOME`; an accepted entry on the folder's repository itself (git root or below: Claude Code keys trust on the git root, M16); Claude Code already started a session in that exact folder (transcript birth, job row `createdAt` or session file `startedAt` with that cwd), and, inside a repository, started it since the repository appeared (`.git` birth; a folder that became a git root after its sessions ran is asked again; on a file system with no birth times any run counts, as before); or, outside git, an accepted entry on the folder or a parent (keys as given and as real paths). A git repository or worktree (`.git` folder or file) covered only by a parent, with no session started there since: Warn "may not be trusted" (real Claude Code refuses both). Nothing accepts it: Error. The fix names the git root when there is one; a launch that fails with "Workspace not trusted" carries the same fix (`check::untrusted_fix`). `evidence::folder_trusted` (doctor, `folder.moved`) applies the same git-root cut |
| `name.drift` | the job's name (its `state.json`, or the live session file) differs from the expected name; a stopped renamed job too, since a wake keeps the name (M19); fix `nimitz relaunch` (`start` relabels it as well) |
| `name.taken` | the expected name is held by a live session of another identity |
| `labels.missing` | the identity's current session has no labels (e.g. resumed with flags, legacy). Only a session in its lineage (or its registry current) can be its current session; an unlabelled row outside the lineage never is |
| `labels.mismatch` | labels name another identity |
| `dup.live` | two live sessions for one identity |
| `copy.rows` | extra job rows of the identity that are its own stale rows (labelled, or in its lineage) and not copies (below) |
| `copy` | a copy of a ship's conversation (plan-v4 section 6; Warn, one per copy, `SessionInfo::copy` says how): an unlabelled session in the ship's folder whose transcript shares the root of a lineage conversation (`evidence::transcript_root`: the first uuid and the first message; a `/branch`, `/fork`, `--fork-session` or `--bg -c` copies the transcript), or that resumes a lineage session id that is not the conversation now (a process with no job on the conversation now is `held`, not a copy), or that carries the ship's exact name (or an old fleet name); anywhere, a row named `<expected name>-<word>-<word>` (Claude Code's copy name) or unnamed with a job row recording a fork of a lineage session. A copy is never the current session, never `dup.live`, `held` or `name.taken`. Fix per kind: a background job, `nimitz sweep <id>` (stop, then rm); a process with no job (interactive, SDK), "end it in its terminal" with the `tmux` pane from the session file when there is one |
| `lineage.broken` | the registry's session (the conversation now) has no job row and no transcript |
| `held` | a member's conversation (any role; the one it runs now) runs in a live process with no job (`jobId` null: an SDK host, an interactive or Remote Control `--resume`, a 0.3.x interactive lead); nimitz can neither stop nor resume it there, and `start` of that member refuses until it ends (a resume would start a copy); a held parent is live for the chain (its ships start, the warning rides along) (Warn in `check`; fix: end that process in its host, then `nimitz start`; for a 0.3.x lead: "end the interactive lead with /exit, then run nimitz start <id> again", which brings the same conversation back as a job, lineage `via migrate`) (M13). Such a row is never a copy or a second live session of the member; a process with no job on an older lineage conversation is a `copy` |
| `lead.intruder` | a lead's folder is exclusive: a live session there that is not the lead's own (its job, a held process on its conversation, a live copy) is an intruder (Error; one fix per kind: a background job `nimitz stop <short>` then `nimitz rm <short>`; an interactive or SDK process "end it in its terminal (tmux ...): /exit, or close the host", with the session file's terminal when it says). A retired tender's row stays `tender.retired` only; `check` lists such a row as the lead's intruder, not as a stowaway as well; it rides along a start as a warning (the lead's own job is apart from it) |
| `history.lost` | a resume would run with no history: the member's job row is there but its conversation has no transcript in any project dir (no launch; fix `nimitz start <id> --fresh`: a cold launch that starts a new lineage entry and drops the old rows); or a resume came back with SessionStart `startup` instead of `resume` (Claude Code found no history; the start is never verified) (M5, E8). A lead with no transcript and no row gets a cold launch, never a fleet-wide `lineage.broken` (M11) |
| `pin.failed` | a lead's job could not be pinned (`pins.json.lock` held); the start stands (Warn; fix `nimitz relaunch`) |
| `rc.exposed` | a live session file of the member has a non-null `bridgeSessionId`: Remote Control is connected, and a resume reconnects it (Warn; fix: run `/remote-control` in that session to turn it off, or archive it on claude.ai/code) (M9) |
| `ship.elsewhere` | a ship's managed folder is in use on another machine (lock file in the folder) |
| `cc.version` | Claude Code outside `compat.toml`: newer than `max` (the highest tested) is a Warn and launches go ahead; below `min`, unknown, or a pre-release `prerelease = [...]` does not list is an Error |
| `registry.corrupt` | `registry.json` did not parse: kept as `registry.json.corrupt-<ts>` and rebuilt (Error until the kept file is removed), or could not be kept aside |
| `registry.readonly` | the state folder or `registry.json` is not writable (launches would not be recorded) |
| `stowaway` | an unlabelled session under `NIMITZ_HOME` (outside ship folders), a lead's folder included (where a live one is listed as the lead's `lead.intruder` instead; a stopped one stays a stowaway). A ship's folder may be shared by several ships and by the user's own work: an unlabelled session there is the user's own and is not shown at all, unless it is a `copy` of a ship in that folder (plan-v4 section 6). `check` lists all, `check F` / `check F/SQ` / `check F/SQ/SHIP` those in the target's folders (fleet or squadron nimitz folders, or a member folder under the target). Fix per kind: a live background job, stop, then `rm`; a live process with no job (interactive, SDK), "end it in its terminal" (with its `tmux` pane when the session file names one); stopped: `rm` or `adopt` (an inherited `NIMITZ_HOME` alone is not a label) |
| `legacy` | an unlabelled session named in `<fleet>/legacy.toml` (an old fleet-skill session, written by `migrate`). The name alone decides (another agent in a member's folder is a stowaway). Warn while live (start/up/relaunch of that member refuse with `legacy`; fix: stop it in the old fleet first, or `nimitz adopt <sid> <id>`; never `rm`), Info once stopped |

Refusal-only codes (in `Plan.refusals`, not from `check`):

| Code | Condition |
| --- | --- |
| `mux.none` | the action needs a multiplexer and there is none (attaching, typing; never a start) |
| `mux.nopane` | the mux cannot open a pane here |
| `tab.taken` | the member's tab is open but its pane runs something else (another agent, a copy, the agent view, yazi): nothing is typed into it; fix: ctrl-z or quit it in that pane, then `nimitz open` |
| `mux.failed` | the mux returned an error |
| `comms.denied` | a signal the chain of command or `comms` rule forbids |
| `config.pending` | a config change is waiting for a relaunch |
| `layout.skip` | a member has no place in the layout |
| `authority` | the caller may not do this (plan-v3 5.5); partial or inconsistent `NIMITZ_*` labels make a caller with no authority, never the user |
| `config.missing` | the id names no fleet, squadron, ship or machine label in the config |
| `config.exists` | `new`/`add` of something that is already there |
| `config.nonempty` | `rm` of a fleet or squadron that still has squadrons or ships |
| `config.write` | a config step failed, or the re-read does not match |
| `machine.host` | `machine add` would give one host a second label (no `--hostname`, or the hostname is taken) |
| `machine.live` | (warning) `machine rm` of this machine while members of the fleet are live here; lists them |
| `member.unknown` | the id names no member of any fleet config |
| `prompt.missing` | a ship's cold launch needs a prompt and the config has none |
| `machine.missing` | the fleet does not run on this machine |
| `session.unknown` | the target names no member's session, no stowaway and no holder of a fleet name |
| `session.none` | the action needs a (live, background) session and there is none |
| `session.live` | the action needs the session stopped first (`rm`, retire) |
| `session.interactive` | the member runs in a process with no job (a 0.3.x interactive lead, an SDK host): `stop` and `relaunch` cannot reach it; end it in its own terminal (`/exit`) |
| `session.live` | also `wake_steps` of a live job: a resume of a live job starts a copy |
| `session.unreadable` | the session file cannot be read; its process may be live, so no resume or relaunch |
| `lineage.current` | `rm` of an identity's current session (it would break the lineage) |
| `tender.retired` | (info) a job row of a 0.3.x tender, history only (fix: `nimitz rm <short>`, after `nimitz stop <short>` while live) |
| `step.failed` | a step of an applied plan failed (or a lock could not be taken); nothing after it ran |

Session codes from the snapshot, besides section 7's: `job.unreadable` (a job
row's `state.json` does not parse; start refuses), `folder.moved` (the
session's saved folder is gone or untrusted while the member's folder moved;
fix `nimitz relaunch <id>`), `labels.mismatch` also for a session carrying an
identity's labels in another folder (it is not that identity's session).
Every fix is a `nimitz` command (or "type /exit in its pane"), never a raw `claude`.

## 8. Relaunch state table (core; each row is a sandbox test)

`nimitz start <id>` restarts top-down: the admiral, then the commodore, then the
target. For each member, one action from this table:

| State of the member's current session | Action |
| --- | --- |
| live, labels right, name right | nothing |
| live, name drift or labels missing | refuse; Issue with fix `nimitz relaunch <id>` (needs a stop) |
| two live sessions | refuse; list both; fix `nimitz stop <short>` for the one you drop |
| stopped or parked (job row, no pid), labels right | resume: native binary, **no flags**, from its folder (the job keeps its short id; `resumeSessionId` is the conversation it wakes on). Every role: a lead is a job too, never `-c`. Before it, the transcript must exist (any project dir), else `history.lost`; not judged for a job in the crash window (`crashed`, or `working` with no live pid: killed in its first turn, its transcript may not be on disk yet; `apply` waits for the daemon, and a resume that comes back `startup` reports `history.lost` then). `sessions::wake_steps` builds this row and the next (a restore uses it) |
| stopped, labels missing (resumed with flags earlier), or a drifted name | relabel (BUG-53): `rm` its job row (native; if that fails the plan stops), then cold launch `--resume <id>` with all flags: with no row Claude Code keeps the **same id and its history** and takes the new name and labels. (With the row present the same command starts an empty copy with a new id; `apply` refuses such a step.) Same path for `relaunch`, `folder.moved`, adopt + start |
| no row, but the registry's current (or an interrupted relaunch's) session has a transcript (background member) | resume `--resume <id>` with all flags: Claude Code keeps the id and history. A relaunch writes `NIMITZ_STATE/pending/<id>.json` before its `rm` and clears it after the bind, so a start after a crash between the two says "recovering an interrupted relaunch" |
| no row, transcript exists (a lead that never had a job: a 0.3.x interactive lead that ended) | resume `--resume <id>` with all flags, like the row above, lineage `via: migrate` |
| no row, but stopped old-fleet sessions of the ship in its folder (names from `legacy.toml`) | continue the newest (job `updatedAt`, else `startedAt`): `rm` its row, then cold launch `--resume <it>` with all flags, so the old session id becomes the ship's with its history, lineage `via: migrate`; the other old rows stay as Info `legacy`, removed only by `sweep`. A lead whose old lead ran in another folder gets a first launch (resume works only in the conversation's folder) |
| no row, no transcript | cold launch (ship: prompt from config; lead: `claude::lead_prompt`), `--bg` for every role; also a lead whose registry session has no transcript (never `lineage.broken` for a lead) |
| row `failed` / unknown id | mark lineage broken; cold launch only with `--force` (ships) |
| `--fresh` | a cold launch whatever the rows (the `history.lost` fix): a new lineage entry `via launch`; the member's stopped rows are removed after it |
| a job in the crash window (`state crashed`, or `working` with no live pid) | `apply` waits up to `NIMITZ_CRASH_WINDOW` s for the daemon's restart before any resume or relaunch of it; restarted, its steps are skipped ("the daemon restarted it") |
| a live process with no job on the conversation (`held`) | refuse: end it first (a resume would start a copy, cli-L2) |
| copy rows of the identity | after the start, `rm` every row that is not current; verify |
| folder missing or untrusted | refuse with the fix |
| Claude Code version newer than tested | warn ("tested up to X"); launches go ahead |
| Claude Code version below the floor, unknown, or an unlisted pre-release | refuse launches (read-only still works); `--force` overrides |
| session file has extra bytes | read the first JSON value; not an issue |
| a live old-fleet or unlabelled session in the member's folder (`legacy`) | refuse; fix: stop it in the old fleet, or adopt it (then stop + start = the labels-missing row) |

Rules that hold in every row: never resume a live session; never pass flags to
a resume that must keep the id; `stop`/`rm` through the native binary and verify
by re-reading; never touch a session that is not in the identity's lineage.
After every launch, resume or relaunch `apply` reads Claude Code's output
(`claude::copy_started`, `claude::job_started`): a "started a copy as X" note
fails the step and stops and removes X in the same run (`step.failed`); the
job named must be the one expected (a resume: the row's job; a cold launch:
the new job), checked again against the row that runs the bound session. The
SessionStart that binds a resume must say `source resume`; `startup` is
`history.lost`, never "[OK] verified". After a lead binds, its job is pinned;
after an `rm`, nimitz's own pin of that row goes.

Authority for `sweep`: the user, any scope; an admiral, its own fleet; a
commodore, its own squadron only (the stale rows and copies of its own
ships); a ship, itself only (its own stale rows). `sweep` removes a member's
stale rows (stopped only; a live stale row is skipped with `dup.live`) and its
copies (a live background copy is stopped, then removed, in the same plan; a
copy with no job is named with "end it in its terminal" and left). A ship
sweeping itself never stops a live copy (the user's own fork of its
conversation may be at work): it removes stopped copies only and names the
live one with the fix `nimitz sweep <id>` for the user or its commodore.
Core owns a plan for every mutating command, including `fleet set`,
`squadron set`, `machine rm`, retire (`sessions::plan_retire`: `ship|squadron|fleet rm`,
session and config steps in one plan, one verified `apply`) and the layout
(`layout::plan` -> `LayoutSpec`, mux-free; the CLI and Admiralty map it onto
`nimitz_mux::LayoutPlan`, dropping attach tabs a pane already shows with `layout::drop_shown`).
`layout::attach_tab(env, member)` is the one tab that shows a member: `claude
attach <job>` in its folder, labelled `F.ADMIRAL` / `F.SQ.COMMODORE` /
`F.SQ.SHIP` (a `layout.skip` when the member is not live, or runs with no
job); `layout herdr` (every live lead; `--ships` every live ship), `nimitz
open`, `signal` to a lead with no tab, Admiralty's `A` and a restore build on
it. The CLI's `run::attach_in_tab(mux, env, member)` focuses the pane or tab
that shows it already, else opens it (`nimitz_mux::attach`). Detach is
ctrl-z; `/exit` inside an attach opens the agent view. A wake by attach keeps
the relaunch table's checks (a drifted name or missing labels: `nimitz start`
relabels first; a lost transcript: `history.lost`); the CLI says "the attach
wakes the parked job; check with nimitz status <id> in a moment", never
"[OK] attached", for a wake. `signal` into a tab it just attached waits up
to 10 s (`NIMITZ_SIGNAL_WAIT`) for a pane to show the session before it
types; otherwise it types nothing and says so.

Every Herdr reply nimitz reads is parsed by a pure function over the JSON
(`herdr::parse_process_info`, `foreground_of`, `workspaces_in`, `tabs_in`,
`pane_rows_in`, `agents_in`/`parse_panes`, `created_workspace`,
`created_tab`, `created_pane`), and the tests
`crates/nimitz-mux/tests/mux.rs` `fixture_process_info`,
`fixture_snapshot_lists` and `fixture_create_replies` MUST run them on the
real captures in `crates/nimitz-mux/tests/fixtures/herdr-0.9.3/`
(`process-info.json`: `result.process_info.foreground_processes[]`;
`snapshot.json`: `result.snapshot.{workspaces,tabs,panes,agents}`, the same
rows the list replies carry under `result.{workspaces,tabs,panes,agents}`)
and on the schema's create replies (`ResponseResult`: `workspace_created` =
`result.workspace` WorkspaceInfo + `result.tab` TabInfo + `result.root_pane`
PaneInfo, all required; `tab_created` = `result.tab` + `result.root_pane`;
a split replies `pane_info` = `result.pane`). The new tab's pane is read from
`root_pane`; `active_tab_id` + `pane list` stand in only when a reply names
no tab or pane. The fake Herdr emits these exact shapes. A `process-info`
reply with no list is an error, never an idle shell.

## 9. Mux trait (`nimitz-mux`)

```rust
pub trait Mux {
    fn name(&self) -> &'static str;                    // "herdr" | "none"
    fn caps(&self) -> Caps;                            // { open, focus, type_text, layout }
    fn panes(&self) -> Vec<Pane>;                      // pane_id, tab_id, workspace_id, session_id?, title, cwd, state
    fn open(&self, cmd: &[String], cwd: &Path, place: Place) -> Result<PaneRef, MuxError>;
    fn focus(&self, pane: &PaneRef) -> Result<(), MuxError>;
    fn type_text(&self, pane: &PaneRef, text: &str) -> Result<(), MuxError>;
    fn layout(&self, plan: &LayoutPlan) -> Result<Vec<PaneRef>, MuxError>;
    fn tab(&self, place: &Place) -> Option<PaneRef>;                        // the tab by its full label: its shell pane, never a split running a program
    fn foreground(&self, pane: &PaneRef) -> Result<Vec<String>, MuxError>; // argv in the pane's foreground; empty = an idle shell (herdr pane process-info)
    fn run_in(&self, pane: &PaneRef, cmd: &[String]) -> Result<(), MuxError>; // herdr pane run, only into a shell pane
    fn shape(&self, foregrounds: bool) -> Result<Shape, MuxError>; // the whole shape: herdr api snapshot (+ pane process-info per pane)
    fn rename_tab(&self, tab_id: &str, label: &str) -> Result<(), MuxError>; // herdr tab rename
}
pub fn detect() -> Box<dyn Mux>;                       // HERDR_ENV=1 -> Herdr, else None
pub fn tab_state(mux, cmd, place, session_id) -> Result<TabState, MuxError>;   // Shown | Shell | Taken { pane, running } | Missing
pub fn attach(mux, cmd, cwd, place, session_id) -> Result<Attached, AttachError>; // Focused | Claimed | Opened; Err Taken: nothing typed
```

The tab rule (restore-scenarios.md R2, decision 5): a member's tab is found
by a pane that shows its session (anywhere), else by the pane that already
runs its exact `claude attach <job>` (visible in `pane process-info` while it
runs), else by its full label in its workspace. A found tab's pane is judged
by its foreground: a shell (`is_shell`: sh, bash, zsh, fish, ...) is claimed
(`pane run claude attach <job>`); anything else (another agent, a copy, the
agent view, yazi) is `tab.taken` and never typed into; a foreground nimitz
cannot read is treated the same. A tab with a split that runs another
program is attached in its shell pane. `layout` reruns a tab's command only
into a shell pane (a Herdr restore, an ended attach client). An attach of a
parked job wakes it (cli-07): `layout::attach_tab` sets `wakes`, and a parked
lead with a shell tab needs no resume first (its transcript must exist, else
`history.lost`).

`None`: `caps` all false; every call returns `MuxError::Unsupported`. Nothing
starts in a pane: `up` and `start` need no mux; attaching and typing do, and
without one `nimitz` refuses those with a clear message. Admiralty runs
anywhere; without a mux it is monitor-only for open, jump and type.

## 9.1 Restore, the active set and the Herdr shape (P9)

`Shape` (`nimitz-mux`): `workspaces[] {id, label, active_tab_id}`, `tabs[]
{id, workspace_id, label}`, `panes[] {pane (ids), cwd, title, agent,
session_id, state, foreground}`. The Herdr adapter reads it from `herdr api
snapshot` (`result.snapshot.{workspaces,tabs,panes,agents}`: `panes[]` carry
no agent field, `agents[]` the panes that show one with `agent_session.value`
the Claude session id; joined by `pane_id`) and, with `foregrounds`, one
`pane process-info` per pane: `Some(argv)`, empty for an idle shell, `None`
when it could not be read (never typed into). `herdr::parse_snapshot` is the
pure reader; the test `fixture_snapshot_shape` runs it on the 0.9.3 capture
and on the fake's `api snapshot`, which must carry the same field set.

`NIMITZ_STATE/active.json` (`nimitz_core::active`, version 1): the members
live at the last write, by id: `{job, session, status (busy|idle|needs-you|
unknown), waiting_for, attached, place {workspace_id, workspace, tab_id, tab,
pane_id}}`, plus `machine`, `at`, and `herdr {session, socket}` from the pane
env (`HERDR_SESSION`, `HERDR_SOCKET_PATH`). Written by `status`, `open`,
`layout`, `restore` (the set from the snapshot and the shape: a pane shows a
member when its session is any id of the member's job, or its foreground is
`claude attach <job>`) and by the hook (SessionStart: the member is live,
keeping its place when the job is the same; SessionEnd other than `clear`:
gone), under `registry.lock` (`registry::with_lock`), temp file + rename,
no write when only the clock moved. A member parked without a daemon retire
line (the machine went down, a crash) stays as recorded; a recorded place
whose pane still exists and shows nothing else is kept (Herdr's detection
lags an attach); without a mux the recorded places and Herdr identity stay.

`NIMITZ_STATE/restore.json` (`restore::Progress`): `{run, started, fleets,
total, done, steps}` of the run in progress; a run that died partway is
reported by the next (`restore.interrupted`, Info), which converges; the
file goes when a run reaches the end of its step list, failed members or
not (then the verdict says "finished, N member(s) failed").

The shell list is one, core's: `restore::SHELLS` / `restore::is_shell`
(`sh`, `bash`, `zsh`, `fish`, `dash`, `ksh`, `tcsh`, `csh`, `nu`, by program
name, a login `-zsh` included); `nimitz_mux::is_shell` is a re-export of it
(the mux depends on core; core never on the mux). A pane whose foreground
could not be read (`pane process-info` failed) is `tab.unreadable`, fix
"herdr pane process-info --pane P failed; retry nimitz restore F, or rename
the tab": never typed into, never a second tab.

Herdr identity (R-29): the env is the identity. `NIMITZ_HERDR_SESSION` when
set (it is also the `--session` of every call); else, inside Herdr,
`HERDR_SESSION` and `HERDR_SOCKET_PATH` as the server put them in the pane.
The E22 hazard (the CLI may resolve a named session's socket regardless of
the env) is accepted: the shape nimitz reads comes from the same `herdr`
invocation in the same env as the identity it records, so the two name the
same server by construction; a `--session` changes both together. `herdr
status` is not run (the shape readers are the only Herdr commands added).

The agent view (R-24): a pane whose foreground is the member's exact
`claude attach <job>` and whose `agent_session` is another session is
`tab.agentview`. The limit: when Herdr reports no session for such a pane
(its detection did not run, or the spare session is not yet one), the pane
reads as the member's own attach (Shown): no step, nothing typed, the
member reported as shown. nimitz then does nothing wrong, but it cannot say
"this is the agent view" either; the user sees it in the tab.

`nimitz restore [<F>] [--all] [--force] [--jobs-only] [--here] [--rename]`
(`restore::plan`, mux-free; the CLI runs the steps): authority as `layout`
(you; an admiral its own fleet; `may_run_in` row `restore`). Per fleet of
this machine (an explicit fleet of another machine is `machine.missing`,
fix `machine add` then `up`; one of many is a warning), `check_tree` first
(a `config.invalid` refuses the fleet; config findings ride along). Per
member, in chain order:

| State | Restore |
| --- | --- |
| live, a pane shows its job (any session of the job, or the exact `claude attach <job>` with no other agent in the pane) | nothing (`tab.renamed` / `tab.moved` Info when the label or workspace differ); with `--rename`, a `herdr tab rename <tab> <full label>` step for a tab under another name |
| live, its full-label tab (loosely, `F.ADMIRAL` / `F.SQ.COMMODORE` / `F.SQ.SHIP`) has a shell pane, in its workspace first, else anywhere | `pane run claude attach <job>` there; a legacy short label (`COMMODORE`) or a default label (`1`) whose shell sits in the member's folder is renamed first; other tabs with the name: `tab.dup` Info |
| live, its tab has no shell pane | `tab.taken` Error (fix: ctrl-z / quit X in pane P, then restore); the exact attach with another agent in the pane is `tab.agentview` (fix: close that pane); a foreground that could not be read is `tab.unreadable`; `--force` opens a second tab (`tab.dup` Warn), never for the agent view |
| live, no tab | a new tab in its workspace (the workspace made when missing, its root tab claimed); a ship only when its recorded place says it was attached (`layout --ships`: every live ship) |
| parked or stopped, a lead; a ship in the active set and not stopped; any ship with a row under `--all` | its shell tab's attach wakes it (one step; the SessionStart `resume` is verified, a `startup` is `history.lost`, a copy `lineage.broken`; the crash window first); else `sessions::wake_steps` (resume by exact id, or rm + relaunch for a drifted name or missing labels), then the tab as above. A lost transcript is `history.lost` (no tab). `turn.interrupted` / `prompt.lost` Info from the recorded status |
| parked ship out of the set | counted parked, left (`--all` wakes it) |
| stopped ship out of the set (a SessionEnd) | `stopped` Info with `nimitz start <id>`; `--all` wakes it |
| a live copy, `name.taken`, `labels.mismatch`, `job.unreadable`, a member `check` error (`folder.*`, `ship.elsewhere`) | Error with that fix; nothing runs for it |
| held (a process with no job) | `lead.interactive` (fix `/exit` then restore) / `held`; nothing runs |
| no row, the registry's conversation has a transcript | `--bg --resume <sid>` with every flag (lineage `via migrate` for a lead that never had a job), then its tab (the attach names the new job) and the pin |
| no row, no transcript | `not.started` Info, once per fleet (`nimitz up F`); never a cold launch |

A lead's job that is not pinned gets a `pin` step. Tabs of a fleet's
workspaces named `F.*` for no member are `tab.orphan` Info. The `ADMIRALTY`
workspace is never a target and `ADMIRALTY` is refused as a fleet name.
Outside Herdr: `mux.none` (fix: inside Herdr, or `--jobs-only`: wakes, no
tabs). A set recorded in another Herdr session: `mux.other` (fix: run it
there, or `--here`). Verdict per fleet: `[OK] F: restored N lead(s), M
ship(s); K parked` / `[OK] F: nothing to restore (...)` / `[ERROR] F: N
member(s) not restored`; exit 1 only when a member could not be restored.
After the run `rc.exposed` of the fleet's members is repeated, and the
active set is recorded. `up` = `plan_up` then the shape-only engine;
`layout herdr` = the shape-only engine (plan-v4 section 13.1).

## 10. CLI surface (`nimitz`)

`init`, `doctor [--fix]`, `status [--json]`, `check [<id>]` (the target, every
member under it, and its parents; the pre-start gate checks the target and its
parents only),
`fleet new|set|rm`, `squadron new|set|rm`, `ship new|set|rm`, `machine add [--hostname H]|rm`,
`up <fleet> [--plan]`, `start <id> [--force] [--fresh]`, `stop <id>`, `relaunch <id>`, `rm <id>`,
`sweep [<id>]`, `adopt <session> <id>`, `signal <id> <text>`,
`open <id>`, `restore [<F>] [--all] [--force] [--jobs-only] [--here] [--rename]`,
`layout herdr [<fleet>] [--ships]` (an alias of restore, shape only), `migrate fleet-skill <dir...> --into <F>`,
`hook <Event>`, `sandbox create|tick|env <dir>`, and hidden `__fake-claude`,
`__fake-herdr`. The fake `claude` (`sandbox.rs`) models the job model of the
round-2 report: `--bg`, `--bg --resume` (wake a stopped job; a copy under an
auto-generated name with `--name` ignored while the job's process is alive,
whatever its state; or a new job named by the short resumed id after `rm`),
`-p --resume` (an SDK host, refused on a live
job), `stop|kill`, `rm`, `respawn`, `attach`, `agents`, and the sandbox-only
verbs `__inside <short> clear|branch <name>|resume <id>|rename <name>|fork`,
`__retire <short>`, `__crash <short>`, `__pin|__unpin <short>`, `__end <id>`; two
`world.json` knobs, `copy_on_resume` (a flag-free resume races the daemon's
restart: a copy with the "started a copy as X" note, cli-31) and
`resume_loses_history` (the wake fires SessionStart `startup`, E8). Every command exits non-zero on failure and prints `[OK]`, `[INFO]`,
`[WARN]`, `[ERROR]` lines (no emoji). Mutating commands support `--plan` (print,
do nothing).

## 11. Safety rules for every agent working on this repo

1. Tests use the sandbox (fake `claude`, fake `herdr`) and the env overrides in
   section 2. Never point a test at the real `~/.claude` or `~/.claude-nimitz`.
2. Real Claude Code sessions only in the e2e harness: folders `~/.cache/nimitz-e2e/nimitz-e2e-*/`,
   fleet names starting `NIMITZ_E2E` (so session names are `[NIMITZ_E2E...]-*`),
   stopped and removed at the end, verified. Their transcript folders under
   `~/.claude/projects/` stay; the user removes them.
3. Never stop, resume, message or rename any session you did not start.
4. Never touch the user's Herdr session; Herdr tests use a private server (temp
   `XDG_CONFIG_HOME`/`XDG_STATE_HOME`, all `HERDR_*` unset).
5. Never edit `~/Dropbox/claude-orchestrators/` (the current fleets) or
   `~/.config/herdr/`.
6. Commit on your own feature branch; no Claude signature or co-author lines;
   do not push to `main`.
