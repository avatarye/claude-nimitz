# Contracts: interfaces every crate builds against

Status: binding for the P1-P6 build (dev branch). Read with `plan-v3.md` and
`cc-control-model.md`. If a contract must change, change this file first, in the
same commit, and say why.

## 1. Crates

| Crate | Depends on | Owns |
| --- | --- | --- |
| `nimitz-core` | serde, toml, libc | env/paths, config (TOML), registry, evidence readers, snapshot model, rules S1-S9, integrity check, relaunch planner, actions (plan + apply), compat range, sandbox world + fake `claude` |
| `nimitz-mux` | serde_json | the `Mux` trait, the Herdr adapter, the `None` adapter, fake `herdr` for tests |
| `nimitz` | core, mux | CLI (`nimitz ...`), hook binary (`nimitz hook <Event>`), plugin files under `plugin/` |
| `admiralty` | core, mux, ratatui | the TUI (`admiralty`). No rule or Claude Code file parsing of its own: it calls core. |

`nimitz-core` never depends on `nimitz-mux`. A multiplexer matters only to
starting/opening interactive members and to Admiralty's pane actions.

## 2. Paths and overrides (tests MUST use them)

| Var | Default | Meaning |
| --- | --- | --- |
| `NIMITZ_HOME` | `~/.claude-nimitz` | data root (fleets) |
| `NIMITZ_STATE` | `~/.local/state/nimitz` | per-machine registry and events; never synced |
| `NIMITZ_CLAUDE_HOME` | `~/.claude` | Claude Code's home (sessions/, jobs/, projects/) |
| `NIMITZ_CLAUDE_BIN` | `claude` on PATH | cold launches (on ACME the wrapper, which adds Bedrock settings) |
| `NIMITZ_NATIVE_CLAUDE_BIN` | newest `~/.localtools/tools/claude-code/*/claude`, else `claude` on PATH | resume, stop, rm, agents |
| `NIMITZ_HERDR_BIN` | `herdr` on PATH (only when `HERDR_ENV=1`) | Herdr adapter |
| `NIMITZ_HOSTNAME` | `gethostname()` | machine matching |
| `NIMITZ_MUX` | auto | `herdr` or `none` |
| `NIMITZ_BIND_TIMEOUT` | `10` (s) | how long the CLI waits for a pane's session to start and bind; tests set `0` |

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

A ship with no `dirs` entry for this machine does not run here. Ship names
`COMMODORE` and `ADMIRAL` (and anything starting `COMMODORE/`) are reserved: they
would take a role's address. Folders are compared as real paths (symlinks
resolved, as Claude Code records them); one folder holds one member.

A `squadron.toml` that does not parse leaves only that squadron out: the rest of
the fleet loads, the squadron is `config.invalid` (file:line:col), its live
members stay visible as members whose config is unreadable, and config writes to
the fleet refuse until the file is fixed. `machine add <F> <label>` records this
host only when no other label of the fleet names it; otherwise `--hostname H`.

## 4. Identity, names, labels

| Role | Identity id | Name (address) |
| --- | --- | --- |
| admiral | `F` | `[F]-ADMIRAL` |
| commodore | `F/SQ` | `[F.SQ]-COMMODORE` |
| ship | `F/SQ/SHIP` | `[F.SQ]-SHIP` |
| tender | `F/SQ/~task` | `[F.SQ]-COMMODORE/task` |

Labels (in `--settings '{"env":{...}}'`): `NIMITZ_FLEET`, `NIMITZ_SQUADRON`,
`NIMITZ_ROLE` (`admiral|commodore|ship|tender`), `NIMITZ_ID`, `NIMITZ_PARENT`
(tenders), `NIMITZ_HOME`. Every `nimitz` launch sets all of them, plus
`"worktree":{"bgIsolation":"none"}` for background sessions.

## 5. Registry and events (`NIMITZ_STATE`)

`registry.json` (atomic write: temp + rename; `registry.lock` with `flock`; every
save also writes `registry.json.bak`, the last good copy). A `registry.json` that
does not parse is never replaced in silence: it is kept as
`registry.json.corrupt-<ts>` (with a `.txt` note), the registry is rebuilt from
the `.bak` and the SessionStart labels in `events.jsonl`, and `check` reports
`registry.corrupt` until the kept file is removed:

```json
{"version": 1, "machine": "west-mac", "identities": {
  "MAIN/ARGOS/ARGOS": {"role": "ship", "name": "[MAIN.ARGOS]-ARGOS", "cwd": "/...",
    "current": "9e0cad63-...", "lineage": [
      {"session_id": "9e0cad63-...", "bound_at": 1790900000.0, "via": "launch|resume|relaunch|adopt|migrate"}]}}}
```

`events.jsonl` (append, one line per hook call, `O_APPEND`):

```json
{"ts": 1790900000.1, "event": "SessionStart", "session_id": "...", "source": "startup",
 "reason": null, "agent_id": null, "tool_name": null, "to": null, "pid": 12345,
 "labels": {"NIMITZ_ID": "MAIN/ARGOS/ARGOS", "NIMITZ_ROLE": "ship", "...": "..."}}
```

`pid` is the hook's parent (the Claude Code process). The hook never blocks,
never fails the session (always exit 0), and returns in a few ms.

## 6. Evidence and state (core)

Order: (1) session file + `kill(pid,0)` (read the first JSON value only),
(2) `claude agents --json --all` (native binary, every ~10 s, cached),
(3) events (binding, crew, end reason), (4) mux panes (Admiralty only).

Snapshot types (serde-serializable; `nimitz status --json` prints them):

```
Snapshot { taken_at, machine, fleets: [Fleet], stowaways: [Member], issues: [Issue], cc_version, compat }
Fleet { id, admiral: Member, squadrons: [Squadron], issues }
Squadron { id, commodore: Member, ships: [Member], tenders: [Member], comms, issues }
Member { id, role, name, expected_name, cwd, status, reasons: [String], session: Option<SessionInfo>,
         crew: [Crew], tasks: [Task], issues: [Issue], pane: Option<PaneRef> }
Status = NotStarted | Stopped | Idle | Busy | NeedsYou | Unknown
SessionInfo { session_id, short_id, pid, kind, state, status, waiting_for, started_at, status_since, name }
Issue { code, severity: Info|Warn|Error, message, fix: Option<String> }   // fix = a nimitz command
```

`reasons` says why the status is what it is (e.g. "session file: status waiting,
waitingFor permission prompt"). Unknown is allowed; a guess is not.

## 7. Integrity check (`nimitz check`, run before every start)

Each finding is an `Issue` with a stable `code`:

| Code | Condition |
| --- | --- |
| `config.invalid` | TOML does not parse (file:line:col; a bad squadron.toml hides only its squadron) or breaks a schema rule (names, reserved ship names, duplicates, unknown keys) |
| `config.layout` | an inherited instructions file (`CLAUDE.md`, `CLAUDE.local.md`, `.claude/CLAUDE.md`) in `NIMITZ_HOME/`, `<fleet>/`, `<fleet>/squadrons/`, `<fleet>/ships/` or `<fleet>/ships/<sq>/` (Error); two members with one folder (Error); a member folder inside a nimitz-managed folder (admiral, commodore, managed ship) (Error); a member folder inside a user project folder that has a `CLAUDE.md` (Warn). A member folder inside a user folder without one (e.g. a ship at `~`) is fine |
| `folder.missing` / `folder.untrusted` | a member's folder does not exist / is not trusted |
| `name.drift` | live session's name differs from the expected name |
| `name.taken` | the expected name is held by a live session of another identity |
| `labels.missing` | the identity's current session has no labels (e.g. resumed with flags, legacy) |
| `labels.mismatch` | labels name another identity |
| `dup.live` | two live sessions for one identity |
| `copy.rows` | extra job rows of the identity (copies, stale rows) |
| `lineage.broken` | registry's current session id has no job row and no transcript |
| `role.background` | an admiral or commodore is a background session |
| `ship.elsewhere` | a ship's managed folder is in use on another machine (lock file in the folder) |
| `cc.version` | Claude Code outside `compat.toml` |
| `registry.corrupt` | `registry.json` did not parse: kept as `registry.json.corrupt-<ts>` and rebuilt (Error until the kept file is removed), or could not be kept aside |
| `registry.readonly` | the state folder or `registry.json` is not writable (launches would not be recorded) |
| `stowaway` | an unlabelled session under `NIMITZ_HOME` or in a member's folder itself (not its subfolders); live: fix is stop, then `rm`; stopped: `rm` or `adopt` (an inherited `NIMITZ_HOME` alone is not a label) |
| `legacy` | an unlabelled session named in `<fleet>/legacy.toml` (an old fleet-skill session, written by `migrate`), or any live unlabelled session in a member's folder. Warn while live (start/up/relaunch of that member refuse with `legacy`; fix: stop it in the old fleet first, or `nimitz adopt <sid> <id>`; never `rm`), Info once stopped |

Refusal-only codes (in `Plan.refusals`, not from `check`):

| Code | Condition |
| --- | --- |
| `mux.none` | the action needs a multiplexer and there is none |
| `mux.nopane` | the mux cannot open a pane here |
| `mux.failed` | the mux returned an error |
| `comms.denied` | a signal the chain of command or `comms` rule forbids |
| `lineage.pending` | a pane was opened but its session did not bind in time |
| `config.pending` | a config change is waiting for a relaunch |
| `layout.skip` | a member has no place in the layout |
| `authority` | the caller may not do this (plan-v3 5.5); partial or inconsistent `NIMITZ_*` labels make a caller with no authority, never the user |
| `member.unknown` | the id names no member of any fleet config (or a tender's commodore does not exist) |
| `prompt.missing` | a ship's cold launch needs a prompt and the config has none |
| `machine.missing` | the fleet does not run on this machine |
| `session.unknown` | the target names no member's session, no stowaway and no holder of a fleet name |
| `session.none` | the action needs a (live, background) session and there is none |
| `session.live` | the action needs the session stopped first (`rm`, retire) |
| `session.interactive` | an interactive member is ended in its own pane, not by nimitz |
| `session.unreadable` | the session file cannot be read; its process may be live, so no resume or relaunch |
| `lineage.current` | `rm` of an identity's current session (it would break the lineage) |
| `tender.invalid` | a tender's commodore, task name or prompt is wrong |
| `step.failed` | a step of an applied plan failed (or a lock could not be taken); nothing after it ran |
| `config.missing` | the id names no fleet, squadron, ship or machine label in the config |
| `config.exists` | `new`/`add` of something that is already there |
| `config.nonempty` | `rm` of a fleet or squadron that still has squadrons or ships |
| `config.write` | a config step failed, or the re-read does not match |
| `machine.host` | `machine add` would give one host a second label (no `--hostname`, or the hostname is taken) |
| `machine.live` | (warning) `machine rm` of this machine while members of the fleet are live here; lists them |

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
| stopped (job row, no pid), labels right | resume: native binary, **no flags**, from its folder |
| stopped, labels missing (resumed with flags earlier) | relaunch: native resume would keep no labels; cold launch with `--resume <id>` and all flags (copy, `source: fork`), then record the new id and `rm` the old row |
| no row, transcript exists (interactive member) | `claude -c` in its folder with the name and labels |
| no row, but stopped old-fleet sessions of the ship in its folder (names from `legacy.toml`) | continue the newest (job `updatedAt`, else `startedAt`): cold launch `--resume <it>` with all flags (copy, `source: fork`), lineage `via: migrate`; the old rows stay as Info `legacy`, removed only by `sweep`. An interactive member whose old lead ran in another folder gets a first launch (resume works only in the conversation's folder) |
| no row, no transcript | cold launch (ship: prompt from config; tender: refuse) |
| row `failed` / unknown id | mark lineage broken; cold launch only with `--force` |
| copy rows of the identity | after the start, `rm` every row that is not current; verify |
| folder missing or untrusted | refuse with the fix |
| admiral/commodore is a background session | refuse; fix: attach and keep it in a terminal |
| Claude Code version out of range | refuse launches (read-only still works); `--force` overrides |
| session file has extra bytes | read the first JSON value; not an issue |
| a live old-fleet or unlabelled session in the member's folder (`legacy`) | refuse; fix: stop it in the old fleet, or adopt it (then stop + start = the labels-missing row) |

Rules that hold in every row: never resume a live session; never pass flags to
a resume that must keep the id; `stop`/`rm` through the native binary and verify
by re-reading; never touch a session that is not in the identity's lineage.

Authority for `sweep`: the user, any scope; a commodore, its own squadron only
(its tenders and the stale copy rows of its own ships). Admiral, ship, tender: none.
Core owns a plan for every mutating command, including `fleet set`,
`squadron set`, `machine rm`, retire (`sessions::plan_retire`: `ship|squadron|fleet rm`,
session and config steps in one plan, one verified `apply`) and the layout
(`layout::plan` -> `LayoutSpec`, mux-free; the CLI and Admiralty map it onto
`nimitz_mux::LayoutPlan`, dropping attach tabs a pane already shows with `layout::drop_shown`).

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
}
pub fn detect() -> Box<dyn Mux>;                       // HERDR_ENV=1 -> Herdr, else None
```

`None`: `caps` all false; every call returns `MuxError::Unsupported`. Starting an
admiral or a commodore needs `caps.open`; without it `nimitz` refuses with a
clear message. Admiralty runs anywhere; without a mux it is monitor-only.

## 10. CLI surface (`nimitz`)

`init`, `doctor [--fix]`, `status [--json]`, `check [<id>]` (the target, every
member under it, and its parents; the pre-start gate checks the target and its
parents only),
`fleet new|set|rm`, `squadron new|set|rm`, `ship new|set|rm`, `machine add [--hostname H]|rm`,
`up <fleet> [--plan]`, `start <id>`, `stop <id>`, `relaunch <id>`, `rm <id>`,
`sweep [<id>]`, `tender <F/SQ> <task>`, `adopt <session> <id>`, `signal <id> <text>`,
`open <id>`, `layout herdr [<fleet>]`, `migrate fleet-skill <dir...> --into <F>`,
`hook <Event>`, `sandbox create|tick|env <dir>`, and hidden `__fake-claude`,
`__fake-herdr`. Every command exits non-zero on failure and prints `[OK]`, `[INFO]`,
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
