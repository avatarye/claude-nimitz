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
| `NIMITZ_HERDR_BIN` | `herdr` on PATH (when `HERDR_ENV=1`, or `NIMITZ_MUX=herdr`, which outside Herdr the recorded server sets) | Herdr adapter |
| `NIMITZ_HERDR_SESSION` | unset (Herdr's own resolution: `HERDR_SESSION`, else the pane's socket) | the named Herdr session every herdr call gets as `--session`; with `NIMITZ_MUX=herdr` it drives a test server from a plain terminal (the checklist), and it is the Herdr identity `active.json` records |
| `NIMITZ_HERDR_SOCKET` | unset | set by nimitz only (`active::member_mux_vars`): the recorded default Herdr server's socket, passed as `HERDR_SOCKET_PATH` to every herdr call of a member's command; the inherited `HERDR_SOCKET_PATH` never counts there |
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
| aide | `<parent>/@NAME` (`F/@KB`) | `[<parent's parts joined by .>]-@NAME` (`[F]-@KB`) |

An aide (docs/design/aide.md) is an always-on member attached to its parent:
a last part `@NAME` makes the role `aide` whatever the count, `Id::parent`
drops that part, and `Id::squadron` is never an `@` part. Only an admiral's
aides exist (`[[aide]]` in `fleet.toml`); one machine runs each (its `dirs`).

The tender role of 0.3.x (`F/SQ/~task`, one job for a commodore) was removed
in 0.4.0: a v1 registry's tender identities move to the registry's `retired`
map on load (history only, never members), a job row with tender labels is
`tender.retired` (Info, fix `nimitz rm <short>`; `nimitz stop <short>` first
while it is live).

Labels (in `--settings '{"env":{...}}'`): `NIMITZ_FLEET`, `NIMITZ_SQUADRON`,
`NIMITZ_ROLE` (`admiral|commodore|ship|aide`), `NIMITZ_ID`, `NIMITZ_HOME`;
an aide also `NIMITZ_PARENT`, its launch `--model` (`[[aide]] model`, else
`sonnet`), its own permissions and `claude::aide_prompt`. Every
`nimitz` launch is `claude --bg` and sets all of them, plus
`"worktree":{"bgIsolation":"none"}` and the `Bash(nimitz *)` allow
(`claude::settings_json`). A lead's cold launch carries `claude::lead_prompt`
(who it is, what to do first); a ship's the prompt from `squadron.toml`.

## 5. Registry and events (`NIMITZ_STATE`)

**Every `claude` nimitz starts begins without the caller's session state**
(`claude::scrub_child_env`, on launch, wake, relaunch, stop, rm, `agents` and
the version checks; a denylist): nimitz's identity and multiplexer names
(`NIMITZ_ID`, `_ROLE`, `_FLEET`, `_SQUADRON`, `_PARENT`, `_MUX`, `_HERDR_*`,
`_HOOK_*`), every `HERDR_*`, `CLAUDE_PID`, `CLAUDECODE`, `CLAUDE_BG_*`,
`CLAUDE_CODE_RESUME_*` and the per-session `CLAUDE_CODE_ENTRYPOINT`,
`_EXECPATH`, `_MESSAGING_SOCKET`, `_MESSAGING_TOKEN`, `_SESSION_ID`,
`_BRIDGE_SESSION_ID`, `_CHILD_SESSION`, `_SESSION_ATTENDED`, `_SDK_HAS_OAUTH_REFRESH`,
`_SDK_READS_SESSION_STATE`, `_HOST_WORKTREE*`, `CLAUDE_BRIDGE_*`, `CLAUDE_JOB_DIR` and
`CLAUDE_EFFORT` (the launching session's own). Everything else passes: the user's
`CLAUDE_CODE_*` and `ANTHROPIC_*` config (auth, proxy, provider, tuning;
Claude Code copies a job's provider settings from the launching command),
and nimitz's own config. A member's labels go only through `--settings`,
which also carries nimitz's `NIMITZ_STATE`, `NIMITZ_CLAUDE_HOME` and `NIMITZ_HOSTNAME`, so the
member's hook writes where this nimitz reads whatever env the daemon has, and blanks
`HERDR_PANE_ID`, `HERDR_TAB_ID` and `HERDR_WORKSPACE_ID`, so Herdr's own hook in the worker
never claims the pane of a daemon started by hand in a Herdr pane. A daemon that such a call
happens to start keeps a clean environment, whoever ran nimitz. The `claude
attach <job>` typed into a Herdr pane runs in that pane's shell, with the
pane's own env.

**A background job's labels are the ones its launch saved.** Claude Code's
daemon hands its own environment to every job it starts, so a daemon started
from a labelled lead's shell (an interactive 0.3.x lead that ran `claude --bg`)
labels every job launched without labels of its own (the user's `claude --bg`,
a hand copy, the agent view's draft) as that lead. A job's `NIMITZ_*` count
only when its row saved them (`--settings` in `respawnFlags`) and they name the
same id (`evidence::job_backs_labels`); otherwise the session is unlabelled,
in the hook (no binding, no rules, no signal authority, no needs-you), in a
replay, and in the snapshot (`row_labels`: a job row's saved labels decide).
Once backed, the effects use the saved labels, not the env's (an inherited
key the launch did not name stays out). The SessionStart hook waits up to
500 ms for a new job's row; still none, the session is left unlabelled for now
(no rules: its first prompt gets them in full through `rules::update_for`; no
binding), the event line keeps the env's labels, and `status` replays it,
binding only when the row then backs them (so does the launch's own wait,
`wait_for_binding`, and its check). Only SessionStart waits for a row. A spare (`spare: true`) is never
labelled. A process
with no job (interactive, SDK) is labelled by its env, as before.

`registry.json` (atomic write: temp + rename; `registry.lock` with `flock`; every
save also writes `registry.json.bak`, the last good copy). **A registry never
shrinks except by an explicit removal** (none exists in 0.4.0): every write
(`registry::update`, `save`) is refused, the file and the `.bak` untouched, when
its result lacks an identity the file holds (`retired`, where a 0.3.x tender
moves, counts as kept); a write that changes nothing (a hook that judged `held`,
`dup.job`, `elsewhere` or `same`) does not touch the file. A `registry.json` that
is missing while the `.bak` holds identities was lost, not reset: the copy and
the SessionStart lines newer than its `updated_at` rebuild it (a read in
memory; the next write saves it; `check` says so, `registry.rebuilt`, Info,
until then). Resetting the registry means removing both files.
A `registry.json` that
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
      {"session_id": "9cdceb2d-...", "job": "9cdceb2d", "via": "launch", "at": 1791000000.0, "home": "/Users/me/.claude-nimitz"},
      {"session_id": "79519302-...", "job": "9cdceb2d", "via": "clear", "at": 1791000100.0, "home": "/Users/me/.claude-nimitz"}]}}}
```

`home` is the `NIMITZ_HOME` (as written) of the process that bound the entry,
stamped by the registry write (`registry::stamp_home`: every entry a write adds
that the file did not have); an entry an older nimitz wrote has none.

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
| same | another `jobId` whose job row still exists (a row of this home: `registry::job_row_of_this_home`; a row whose saved labels name another home never holds the identity) | nothing rebound: two jobs claim one identity | `dup.job` |
| same | `jobId` null (interactive or SDK) | nothing bound: a process with no job holds the conversation | `held` |
| same | a `cwd` that is not the identity's folder (the registry's `cwd`, else the config) | nothing bound: an impostor (BUG-30, BUG-55) | `elsewhere` |
| labels of another `NIMITZ_HOME` | any | ignored by every reader. The hook itself cannot tell (its env is the session's env, and the `--settings` label wins over a project settings env, so `env.home` is the label): with one `NIMITZ_STATE` shared by two homes it writes its line and binds the identity there. `registry::load` strips such a binding for every reader, in memory only (`drop_other_homes`: the lineage entries of a job of another home go (`foreign_job`: an entry's `home` says whose it is; an entry with none, an older nimitz's, is judged by its job's row, `respawnFlags`, while the row exists), so a foreign binding never comes back into this home's view once its row is gone; `job` and `session` fall back to the newest entry left, an identity with nothing of its own goes), so no reader uses it and this home's own history stays (r3-e2e.md finding 1). A writer works on the file as it is, every home's bindings side by side, and never saves the stripped shape: which home is "other" is the env of the process, and the hook of the other home (its env is that home; its `NIMITZ_STATE` this one, from a project settings env) would strip and save away every record of this home (acc-D F1: a registry left with one identity and a lone `resume`). A record a foreign hook bound has that home's folder in `cwd`: this home's hook takes the folder from the config then (`identity_folder`) | - |
| no labels | any | the line only (the snapshot classifies it) | - |

The name is never taken from the SessionStart moment: the snapshot reads it
from the session file (M21). A SessionEnd from a pid that never sent a
SessionStart (a refused `-p --resume` fires one with no labels) is ignored by
every reader (M21); so is the SessionEnd of a string-form hook whose session
file is already gone when it fires (the chain cannot be checked, so the line
carries the shell's pid). `doctor` warns about the form. The hook takes one clock for the line's `ts` and the
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
(only the member's own labels may run it), which posts the notice into the
commodore's inbox socket, else types it into the commodore's pane, retrying
`NIMITZ_NOTIFY_RETRY` s (default 600) while neither works. The member's next
UserPromptSubmit, Stop or SessionEnd removes the file; `check` reports
`notify.failed` for one that failed, with each path's reason in the note.

The inbox (`nimitz_core::inbox`): Claude Code's cross-session messaging lets a
script post into a session's socket (docs: cross-session-messaging, "a script
or hook to post into a session"); an idle background job takes the line as a
new turn, a busy one at its next, with no pane and nobody attached. It arrives
as a peer message, never as the user's input: Claude Code tells the receiver
it is not from the user and cannot authorize anything. The SessionStart hook
of a member records `CLAUDE_CODE_MESSAGING_SOCKET` (exported before any hook
runs) in `NIMITZ_STATE/inbox/<session id>.json` as `{socket, pid, at}` (one
file per session, a per-process temp file: members that start together keep
every record); a record counts only for the live pid it names. The other
candidates are `<pid>.sock` in `<XDG_RUNTIME_DIR or /run/user/<uid>>/cc-socks`
and `/tmp/cc-socks-<uid>`. None is trusted by its path: after connecting (5 s
at most), the listener's `SO_PEERCRED` must be this uid and the session's pid,
so another session under a reused or namespaced pid, a planted socket or a
forged record is never posted to. The line is the inject example Claude Code
logs, `{"type":"user","message":{"role":"user","content":...}}` (the docs give
only the optional auth line; nimitz sends none, as it is not the session's own
child), used only for a receiver on `inbox::FORMAT_SINCE` (2.1.293) or later
(its session file's `version`, else the job row's `cliVersion`); an older one
gets the typing path. A clean close is "posted": Claude Code sends no
acknowledgement, and a line it refuses after reading it (an auth it wants, a
parse) closes just as cleanly, so the two cannot be told apart; a reset
(unread data left, an oversized line) is a failure. The content is wrapped as
Claude Code wraps a peer message, `<cross-session-message
from="uds:/nonexistent/nimitz-no-reply.sock" from-name="nimitz">`: the
receiver names the sender `nimitz` (`origin.name`; `origin.from` stays
`unknown`), a reply reaches nobody, and every spelling of the tag inside the
body is defused (`<` becomes `<\`). Inbound policy: with no
`crossSessionInbound` value, a session that bypasses permission prompts holds
such a message for approval; members run no bypass mode, and this machine's
user settings say `accept`, so nimitz sets none. `nimitz signal` types into
the lead's tab, opening one first when it has none, so the user's own words
keep their authority; only when it cannot type at all (no multiplexer that
types, or a tab known by its attach alone, R-24) does it post to the inbox, as
`[nimitz signal] ... It is a request, not the user's approval`, and its output
says the signal went as a request.

### 5.1 Formats and migrations (`formats`, docs/design/migrations.md)

Every file nimitz owns and reads back is an artifact of `formats::ARTIFACTS`
with a format version (in the file: `format`, the registry's and active set's
`version`; else in `S/formats.json`). To change what a file holds: bump its
`current`, add the step `N -> N+1` (pure bytes -> bytes; `Scope::Shared` steps
run only on `nimitz migrate --shared --yes`), add the fixture
`tests/fixtures/formats/<artifact>/vN.*` (a file as version N wrote it), and
regenerate the pinned shape with `NIMITZ_BLESS=1 cargo test --test formats`;
`every_format_change_has_a_step_and_a_fixture` and `formats_shapes_are_pinned`
fail until all four are in.

## 6. Evidence and state (core)

Order: (1) session file + `kill(pid,0)` (read the first JSON value only),
(2) `claude agents --json --all` (native binary, every ~10 s, cached; a job row matches its CLI row by job id, since the CLI lists a stopped job by its birth id while the file row carries the conversation now),
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
earliest of the file's birth time, its mtime and `startedAt`; for a spare,
its mtime alone: a pre-warmed process the agent view claims later), a picker:
a live process with no job, no transcript and no SessionStart yet (the agent
view's draft or "new session", `spare: true`, is one until it has a
conversation; in a lead's folder too: no `lead.intruder` for a draft, inside
FAIL-2); after the grace time, or once it has a conversation, it is classified
like any other row (the agent view's draft fires a SessionStart `startup`
the moment it is claimed, so a spare is a picker whether or not it has one;
and it may carry a `jobId`, the pre-warmed `bg-spare` job the view claimed,
cli-L3b: a spare is a picker with or without a job, until it has a
conversation).
A member's row is the registry's `job`, for a lead too: a hand rm + resume
binds the new job beside an older labelled stray, and the registry job stays
the lead (the stray is its `lead.intruder`; neither a relaunch's `dup.live`
check nor `apply`'s "live already" guard counts a lead's intruder, and the
labels-in-folder rule of that guard holds only while the registry names no
job). Only two cases hand a lead to another own live job: the registry has no
job, or the registry job bound itself as a fork (the first lineage entry of
that job says `via branch`: a `/fork` copy's hook binds when the lead had no
job). A `/branch` or `/rename` of the real job (Claude Code names a branch
`<prompt> (Branch)`) changes nothing. Then the earliest-started own live job
carrying the expected name, else the earliest with a `started_at`; never on a
missing or equal `started_at`. `jobs/pins.json` gives `pinned` (the daemon
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
| `copy` | a copy of a ship's conversation (plan-v4 section 6; Warn, one per copy, `SessionInfo::copy` says how): an unlabelled session in the ship's folder whose transcript shares the root of a lineage conversation (`evidence::transcript_root`: the first uuid and the first message; a `/branch`, `/fork`, `--fork-session` or `--bg -c` copies the transcript), or that resumes a lineage session id that is not the conversation now (a process with no job on the conversation now is `held`, not a copy), or that carries the ship's exact name (or an old fleet name), or that is a second **job** on the conversation the member runs now while the member's own job row is there (a wake by an id that is not the job's birth id; for a lead too, told as a copy, never as its session); anywhere, a row named `<expected name>-<word>-<word>` (Claude Code's copy name) or unnamed with a job row recording a fork of a lineage session. A copy is never the current session, never `dup.live`, `held` or `name.taken`. Fix per kind: a background job, `nimitz sweep <id>` (stop, then rm); a process with no job (interactive, SDK), "end it in its terminal" with the `tmux` pane from the session file when there is one |
| `lineage.stale` | the registry names a conversation of a stopped member that is neither the one its job would wake nor a later one of its lineage (Warn; fix `nimitz relaunch <id>`; the wake brings the job's conversation back) |
| `lineage.broken` | a start or wake did not bind what was expected (no SessionStart within the wait, another job answered, the registry could not be written); a `failed` row. No row and no transcript is `history.lost` (above) |
| `held` | a member's conversation (any role; the one it runs now) runs in a live process with no job (`jobId` null: an SDK host, an interactive or Remote Control `--resume`, a 0.3.x interactive lead); nimitz can neither stop nor resume it there, and `start` of that member refuses until it ends (a resume would start a copy); a held parent is live for the chain (its ships start, the warning rides along) (Warn in `check`; fix: end that process in its host, then `nimitz start`; for a 0.3.x lead: "end the interactive lead with /exit, then run nimitz start <id> again", which brings the same conversation back as a job, lineage `via migrate`) (M13). Such a row is never a copy or a second live session of the member; a process with no job on an older lineage conversation is a `copy` |
| `lead.intruder` | a lead's folder is exclusive: a live session there that is not the lead's own (its registry job, a held process on its conversation, a live copy) is an intruder, a `/fork` of the lead or a hand launch with its labels included (never `dup.live` or `copy.rows` for the lead; its own job is never the target, and the ships under it still start) (Error; one issue per row: an intruder is never the lead's `name.taken` as well (cli-L8, K1); one fix per kind: a background job `nimitz stop <short>` then `nimitz rm <short>`; an interactive or SDK process "end it in its terminal (tmux ...): /exit, or close the host", with the session file's terminal when it says). A retired tender's row stays `tender.retired` only; `check` lists such a row as the lead's intruder, not as a stowaway as well; it rides along a start as a warning (the lead's own job is apart from it) |
| `history.lost` | a resume would run with no history: the member's job row is there but its conversation has no transcript in any project dir, or its registry session has no job row and no transcript (acceptance env E9: one word, one flag); a transcript with no message line within the first 4 MB (the stub `claude stop` writes after the file was moved away, env F5; a `/branch` copy hoists many 4 KB snapshot lines above its first message and is still one) counts as none (`evidence::has_message`, the `transcript_root` scanner). No launch; fix `nimitz start <id> --fresh`: a cold launch that starts a new lineage entry and drops the old rows (the `--fresh` plan does not repeat the error). Also a resume that came back with SessionStart `startup` instead of `resume` (Claude Code found no history; the start is never verified) (M5, E8). A lead with no transcript and no row gets a cold launch, never a fleet-wide issue (M11) |
| `aide.down` | an aide whose `dirs` names this machine has no live job (Error; fix `nimitz start <id>`; a start is its fix, so it never refuses one) |
| `aide.unpinned` | an aide's live job is not in `pins.json` (Warn; fix `nimitz start <id>`, which pins it again) |
| `aide.model` | the aide's model is not in `availableModels` (managed settings, else `<claude_home>/settings.json`): Claude Code would run the default instead (Error; fix: add the model to availableModels, or set model in `[[aide]]`) |
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
| `attach.nested` | `nimitz attach` from inside a Claude Code session: a live Claude Code process above it (its session file, whose `procStart` matches the process: a stale file is no session), fix: a plain terminal or `nimitz start <id> -p`; or `CLAUDECODE` set with none above (a terminal started from one inherits it), fix: `env -u CLAUDECODE nimitz attach <id>` |
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
| `session.live` | the action needs the session stopped first (`rm`, retire). `nimitz stop <short>` of a copy plans its `rm` with the stop, as `sweep` does: a never-prompted copy has no name, no labels and no transcript once stopped, and no command would see its row again (cli-N1) |
| `session.interactive` | the member runs in a process with no job (a 0.3.x interactive lead, an SDK host, a `claude --resume` of its conversation by hand: `held`): `stop` and `relaunch` cannot reach it, and never answer `session.none` or `dup.live` with a `nimitz stop <sid>` that cannot work (cli-18); the fix is the held one: end that process in its host, then `nimitz start` |
| `session.live` | also `wake_steps` of a live job: a resume of a live job starts a copy |
| `session.unreadable` | the session file cannot be read; its process may be live, so no resume or relaunch |
| `lineage.current` | `rm` of a member's current session: its row goes with the member, never alone (fix: `nimitz relaunch <id>` replaces it, `nimitz ship|squadron|fleet rm <id>` retires the member) |
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
| stopped or parked (job row, no pid), labels right | resume: native binary, **no flags**, from its folder, **by the job's birth id** (`state.json` `sessionId`; Claude Code 2.1.292 wakes a stopped job only by that id, and any other id of its conversation starts a new unlabelled job named after it, acceptance F1). The job keeps its short id and wakes on `resumeSessionId`, the conversation it runs now (after a `/clear`, a later one); `apply` expects that one back. `resumeSessionId` follows `/clear` only: every wake path (`wake_steps`, `relaunch`, restore's attach-wake) asks one helper, `sessions::wake_target`, which conversation must come back. The registry's `session` LATER in the lineage than the row's (a `/branch` or `/resume` after it): the row goes and that session is resumed with every flag (`rm`, then `--bg --resume <it> --name --settings`; the plan says why; restore wakes first, then opens the tab), never the older conversation in silence (r3-e2e finding 11). The same: a native wake. Any other difference (a lost hook line, a hand edit): the row wins, Claude Code's own truth, with the Warn `lineage.stale` (fix `nimitz relaunch`). The transcript checked for `history.lost` is the target's. Every role: a lead is a job too, never `-c`. Before it, the transcript must exist (any project dir), else `history.lost`; not judged for a job in the crash window (`crashed`, or `working` with no live pid: killed in its first turn, its transcript may not be on disk yet; `apply` waits for the daemon, and a resume that comes back `startup` reports `history.lost` then). `sessions::wake_steps` builds this row and the next (a restore uses it) |
| stopped, labels missing (resumed with flags earlier), or a drifted name | relabel (BUG-53): `rm` its job row (native; if that fails the plan stops), then cold launch `--resume <id>` with all flags: with no row Claude Code keeps the **same id and its history** and takes the new name and labels. (With the row present the same command starts an empty copy with a new id; `apply` refuses such a step.) Same path for `relaunch`, `folder.moved`, adopt + start |
| no row, but the registry's current (or an interrupted relaunch's) session has a transcript (background member) | resume `--resume <id>` with all flags: Claude Code keeps the id and history. A relaunch writes `NIMITZ_STATE/pending/<id>.json` before its `rm` and clears it after the bind, so a start after a crash between the two says "recovering an interrupted relaunch" |
| no row, transcript exists (a lead that never had a job: a 0.3.x interactive lead that ended) | resume `--resume <id>` with all flags, like the row above, lineage `via: migrate` |
| no row, but stopped old-fleet sessions of the ship in its folder (names from `legacy.toml`) | continue the newest (job `updatedAt`, else `startedAt`): `rm` its row, then cold launch `--resume <it>` with all flags, so the old session id becomes the ship's with its history, lineage `via: migrate`; the other old rows stay as Info `legacy`, removed only by `sweep`. A lead whose old lead ran in another folder gets a first launch (resume works only in the conversation's folder) |
| no row, no transcript | cold launch (ship: prompt from config; lead: `claude::lead_prompt`), `--bg` for every role; also a lead whose registry session has no transcript (never `lineage.broken` for a lead) |
| row `failed` / unknown id | mark lineage broken; cold launch only with `--force` (ships) |
| `--fresh` | a cold launch whatever the rows (the `history.lost` fix): a new lineage entry `via launch`; the member's stopped rows are removed after it |
| a job in the crash window (`sessions::in_crash_window`: no live pid, and `state crashed` or `working`, or a parked row (no SessionEnd) changed within `CRASH_RECENT_S` = 30 s: a kill -9 of a `blocked` or `done` job, which the 2.1.292 daemon restarts within about 10 s, env F3) | `apply` waits up to `NIMITZ_CRASH_WINDOW` s for the daemon's restart before any resume or relaunch of it; restarted, its steps are skipped ("the daemon restarted it"); not restarted and the transcript still missing, `history.lost` |
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
on a native wake stops and removes X, waits for the job's row to settle (up to
15 s, then 2 s), and wakes once more when the job is still not live (a wake
right after a stop races the daemon's bookkeeping under load, r3-e2e finding
12); a second copy, or a copy on any other launch, fails the step and stops
and removes X in the same run (`step.failed`); the
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
pub fn detect() -> Box<dyn Mux>;                       // NIMITZ_MUX, else HERDR_ENV=1 -> Herdr, else None (callers pass active::user_mux_vars / member_mux_vars)
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

A pane's session is taken only when its foreground backs Herdr's claim
(`evidence::claim_backed`, applied in the adapter to `panes()` and `shape()`):
a foreground process names the session (`--resume <sid>`), attaches to its
job (`claude attach <job>`), or has a session file (`sessions/<pid>.json`)
that names it. Herdr learns a pane's session from its own SessionStart hook,
which names the pane by the process's inherited `HERDR_PANE_ID`; a background
job inherits the pane id of whoever launched it (or the daemon's), so the
claim alone can put a ship in its commodore's pane. An unbacked claim is
ignored (`ShapePane.claimed` keeps it for the agent-view test, R-24; a
claim that names another member's session is that member's hook, not the
agent view), and the pane shows what its foreground runs, as a pane Herdr
shows an agent in but claims no session for does
(`evidence::session_of_unclaimed`): the job a `claude attach <job>`
attaches to (the job row's `resumeSessionId`, else `sessionId`), else a
foreground pid's session file (an interactive resume a `/clear` moved
on), else the conversation a held `claude --resume <sid>` names. A claim
backed by the attach alone (`evidence::Backing::Attach`: no process names
the session in its argv or its session file) is inferred too: under that
argv an older Claude Code may run the agent view. `status`, Admiralty's dots, enter and `J`/`K`, `start -p`
and a restore's claim and reattach all read the pane so. The root cause
is closed at launch: every member's `--settings` env blanks
`HERDR_PANE_ID`, `HERDR_TAB_ID` and `HERDR_WORKSPACE_ID`
(`claude::HERDR_PANE_VARS`), so a member's worker never claims the pane
its daemon was started from (`HERDR_ENV` and the socket stay: a member's
nimitz takes its server from the record). A job launched before keeps the
daemon's values until `nimitz start <id> -f` relaunches it with the
settings again. A session read from the attach alone is `Pane.inferred`
(`ShapePane.inferred`): shown and focused, and typed into only while every
non-shell process of its foreground is exactly `claude attach <the job>` on
a Claude Code at or past `nimitz_mux::ATTACH_EXEC_SINCE` (2.1.293; the
version from the `claude/versions/<v>` path of argv0 or `/proc/<pid>/exe`),
read again just before typing (`nimitz_mux::runs_exact_attach`). On that
version `/exit` makes the attach client re-exec in place as `claude agents`,
the agent view (same pid; e2e `e2e_agent_view_re_execs_as_claude_agents`), so
the argv tells the live attach from the view. An older or unknown version, a
launcher that does not exec, or anything else in the foreground is
`tab.unverified` on a delivery, `nimitz signal` or Admiralty `m`, and nothing
is typed. A tab `signal` just opened is not typed into (the attach may not be
connected yet): its inbox instead. Admiralty types only into a pane that
shows the session, never one matched by its title alone. The window that
stays: text typed while the user's `/exit` is on its way reaches the view;
nimitz looks again after typing (`typed_into_view`) and says so (a failed
delivery, a `signal` warning, an Admiralty error) so the user can check the
pane. An attach client never runs Herdr's
SessionStart hook, so Herdr never names the session of a lead's own attach
pane (seen live after the 0.4.0 migration: `agent` claude, no
`agent_session`). A pane whose foreground cannot be read
(`pane process-info` fails) shows no session at all. The cost: one `pane
process-info` per agent pane on `status` and on every `panes()` read. The
adapter reads job rows and session files under the Claude home of the env
it was built from (`Herdr.claude_home`), not the process env.

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
set (it is also the `--session` of every call); else, inside Herdr and only
for the user's own command, `HERDR_SESSION` and `HERDR_SOCKET_PATH` as the
server put them in the pane. A member's command (`NIMITZ_*` labels in its
env) records none: its env is its launch env, or the daemon's after a respawn
(the user's live `HERDR_*`, whatever world its `NIMITZ_HOME` names; acc-D
F6), so the identity is unknown and the set keeps the one recorded before.
No member's command and no hook path runs a Herdr command on inherited
`HERDR_*` alone (`active::member_mux_vars`): a member's `status`, `up`,
`layout`, `restore` and a needs-you delivery (`nimitz __notify`) take an
explicit `NIMITZ_MUX`/`NIMITZ_HERDR_SESSION` as given, else the recorded
identity's session name as `--session` (with its socket when recorded), else
the recorded default server's socket (`NIMITZ_HERDR_SOCKET`), else no
multiplexer. The herdr child of such a call gets that server only: its
inherited `HERDR_SESSION`, `HERDR_SOCKET_PATH` and `HERDR_CLIENT_SOCKET_PATH`
are removed, and only the recorded socket is set. Only the user's own command detects
Herdr from its env; outside Herdr (no `HERDR_ENV`, no explicit
`NIMITZ_MUX`/`NIMITZ_HERDR_SESSION`: a terminal, a launcher) the user's own
`nimitz` and Admiralty take the server the user's commands recorded
(`active::user_mux_vars`), else no multiplexer.
The E22 hazard (the CLI may resolve a named session's socket regardless of
the env) is accepted: the shape nimitz reads comes from the same `herdr`
invocation in the same env as the identity it records, so the two name the
same server by construction; a `--session` changes both together. `herdr
status` is not run (the shape readers are the only Herdr commands added).

Herdr's agent resume (finding 10, Claude Code 2.1.292): with `[session]
resume_agents_on_restore = true` (Herdr's default) a restarted server runs
`claude --resume <session>` in every pane that showed an agent. On a live
job it is refused ("That session is running in the background (<short>).
Run `claude attach <short>` ..."), and the pane is a shell again; on a
stopped, parked or removed job the conversation opens interactively with no
job (`kind interactive`, `jobId null`), the `held` case, and a `--bg
--resume` meanwhile starts a copy. `nimitz doctor` reads the Herdr config
(`HERDR_CONFIG_PATH`, else `~/.config/herdr/config.toml`, never written)
and warns while the setting is not `false`, with the fix "set
resume_agents_on_restore = false under [session] in <path>". The shape
carries each pane's `foreground_pid` (`pane process-info`) so a held
process is matched by its session, its `--resume <session>` argv or its pid.

The agent view (R-24): a pane whose foreground is the member's exact
`claude attach <job>` and whose `agent_session` is another session that
names no member is `tab.agentview` (the pane is inferred from the attach;
restore reads the claim for this test alone). On Claude Code 2.1.293 and later a view the attach client
re-exec'd into runs `claude agents`, not the attach: the tab is taken by it,
never the member's. On an older version the view may keep the attach's
argv; the limit then stands: with no session reported for the pane it reads
as the member's own attach (Shown), nothing is typed into it (below
`ATTACH_EXEC_SINCE`), and nimitz cannot say "this is the agent view".

`nimitz restore [<F>] [--all] [--restart] [--force] [--jobs-only] [--here]
[--rename]` (`restore::plan`, mux-free; the CLI runs the steps), the one
command after a Claude Code update, a Herdr closure or crash, or a reboot:
authority you; an admiral its own fleet; a commodore its own ships (no `F`,
or its fleet: only `F/SQ/*` members are planned). Per fleet of this machine
(an explicit fleet of another machine is `machine.missing`, fix `machine
add` then `nimitz start <id>`; one of many is a warning), `check_tree` first
(a `config.invalid` refuses the fleet; config findings ride along). The
plan's first line says what happened (`restore F: Claude Code updated to V
(N live worker(s) on an older one); Herdr closed or restarted, or never laid
out (N live member(s) without their pane); a reboot, or jobs that ended (N
member(s) down to wake)`, or `nothing to bring back`), and `Plan.found`
holds one line per member in scope, `<id>  <what was found> -> <what it
does>` (the never started share one `not.started` line). Per member, in
chain order:

| State | Restore |
| --- | --- |
| live, its worker on an older Claude Code than the installed one (`restore::worker_version`: `versions/<v>` in the process's executable, `(deleted)` stripped; else its session file's `version`), or any live member under `--restart` | `sessions::plan_relaunch` (stop, rm, `--bg --resume <id>` with every flag: same id, same job id, its first 8; `apply` pins a lead's job), then: the pane that runs its exact attach gets a `Reattach` (the old attach ends with its job; the executor waits up to `NIMITZ_REATTACH_WAIT` s, 15, for the shell, then types the new job's attach); a pane that shows it without the attach gets nothing typed (`tab.taken` Warn, fix `nimitz open`); else a lead (or a ship that had a pane) its tab's shell or a new tab. Relaunch groups (`RestoreStep::Relaunch`) run after every other step: ships, then commodores, then admirals. Busy or waiting on you: `relaunch.busy` Info, left running; the executor reads the job's status again just before its relaunch and holds back one that turned busy since the plan (its later steps skipped, not failed). A new worker still older than the installed version: `relaunch.old` Warn. The caller itself (by its labels, else the member whose worker is an ancestor of the restore's process): `restore.self` Warn, fix `nimitz restore F` from the user's own shell (it relaunches and reattaches); a commodore's own restore gives it that line too. A refused `plan_relaunch` (`dup.live`, `name.taken`, ...): its Error |
| live, current worker, a pane shows it by an attach client on an older version (argv0 or the pane's foreground pid) | nothing typed; `attach.old` Info (fix: ctrl-z in pane P, then restore) |
| live, a pane shows its job (any session of the job, or the exact `claude attach <job>` with no other agent in the pane) | nothing (`tab.renamed` / `tab.moved` Info when the label or workspace differ); with `--rename`, a `herdr tab rename <tab> <full label>` step for a tab under another name |
| live, its full-label tab (loosely, `F.ADMIRAL` / `F.SQ.COMMODORE` / `F.SQ.SHIP`) has a shell pane, in its workspace first, else anywhere | `pane run claude attach <job>` there; a legacy short label (`COMMODORE`) or a default label (`1`) whose shell sits in the member's folder is renamed first; other tabs with the name: `tab.dup` Info |
| `-p --tab`, the named member | only its own tab counts: a pane that shows it in another tab is left alone, nothing typed (`tab.elsewhere` Info), its legacy short-label tab in its workspace is its own (renamed); no split beside its commodore, no default-labelled tab claimed; else its full-label tab's shell, else a new tab |
| live, its tab has no shell pane | `tab.taken` Error (fix: ctrl-z / quit X in pane P, then restore); the exact attach with another agent in the pane is `tab.agentview` (fix: close that pane); a foreground that could not be read is `tab.unreadable`; `--force` opens a second tab (`tab.dup` Warn), never for the agent view |
| live, no tab | a new tab in its workspace (the workspace made when missing, its root tab claimed); a ship only when its recorded place says it was attached (`layout --ships`: every live ship) |
| parked or stopped, a lead; a ship in the active set and not stopped; any ship with a row under `--all` | its shell tab's attach wakes it (one step; the SessionStart `resume` is verified, a `startup` is `history.lost`, a copy `lineage.broken`; the crash window first); else `sessions::wake_steps` (resume by exact id, or rm + relaunch for a drifted name or missing labels), then the tab as above. A lost transcript is `history.lost` (no tab). `turn.interrupted` / `prompt.lost` Info from the recorded status |
| parked ship out of the set | counted parked, left (`--all` wakes it) |
| stopped ship out of the set (a SessionEnd) | `stopped` Info with `nimitz start <id>`; `--all` wakes it |
| a live copy, `name.taken`, `labels.mismatch`, `job.unreadable`, a member `check` error (`folder.*`, `ship.elsewhere`) | Error with that fix; nothing runs for it |
| held (a process with no job) in a pane of this shape: the pane shows that session, runs `claude --resume <session>`, or runs the process's pid (Herdr's `[session] resume_agents_on_restore = true` after its restart, finding 10) | `tab.resumed` Error, no step: "its conversation is open in pane P (Herdr's resume_agents_on_restore, or a claude --resume by hand)", fix "type /exit in pane P, then nimitz restore F"; counted as not restored ("N held in a pane" in the verdict, exit 1). A wake meanwhile would start a copy, so a held member is never woken anywhere in a restore |
| held elsewhere (a 0.3.x interactive lead, an SDK host) | `lead.interactive` (fix `/exit` then restore) / `held`; nothing runs |
| no row, the registry's conversation has a transcript | `--bg --resume <sid>` with every flag (lineage `via migrate` for a lead that never had a job), then its tab (the attach names the new job) and the pin |
| no row, the registry's conversation has a transcript, a ship out of the set | counted parked, its line says `--all` resumes it |
| no row, no transcript | `not.started` Info, once per fleet (fix `nimitz start <id>`); never a first launch |

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
`layout herdr` = the shape-only engine (plan-v4 section 13.1); both are
deprecated and print a pointer to restore.

## 10. CLI surface (`nimitz`)

`init`, `doctor [--fix]`, `status [--json]`, `check [<id>]` (the target, every
member under it, and its parents; the pre-start gate checks the target and its
parents only),
`fleet new|set|rm`, `squadron new|set|rm`, `ship new|set|rm`, `machine add [--hostname H]|rm`,
`up <fleet> [--plan]`, `start <id> [--force] [--fresh]`, `stop <id>`, `relaunch <id>`, `rm <id>`,
`sweep [<id>]`, `adopt <session> <id>`, `signal <id> <text>`,
`open <id>`, `restore [<F>] [--all] [--force] [--jobs-only] [--here] [--rename]`,
`layout herdr [<fleet>] [--ships]` (an alias of restore, shape only), `migrate --plan|--yes [--shared]`
(the format steps due; every other command but `hook`, `__notify` and `sandbox` runs the local ones
first, by itself), `migrate fleet-skill <dir...> --into <F>`,
`hook <Event>`, `sandbox create|tick|env <dir>`, and hidden `__fake-claude`,
`__fake-herdr`. A closed pipe (`nimitz status --json | head`) ends the process
quietly with SIGPIPE (exit 141), never a panic. `rm <lead>` with no stale rows
names what removes its own row (`relaunch`, or `fleet|squadron rm`). The fake `claude` (`sandbox.rs`) models the job model of the
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
   stopped and removed at the end, verified. The harness deletes its own
   transcript folders under `~/.claude/projects/` (`*nimitz-e2e-<n>-<pid>*`) too.
3. Never stop, resume, message or rename any session you did not start.
4. Never touch the user's Herdr session; Herdr tests use a private server (temp
   `XDG_CONFIG_HOME`/`XDG_STATE_HOME`, all `HERDR_*` unset).
5. Never edit `~/Dropbox/claude-orchestrators/` (the current fleets) or
   `~/.config/herdr/`.
6. Commit on your own feature branch; no Claude signature or co-author lines;
   do not push to `main`.
