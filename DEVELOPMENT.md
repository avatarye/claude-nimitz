# DEVELOPMENT

For agents and people changing claude-nimitz. It describes version 0.7.18, and
it was checked against the code at that commit. The [README](README.md) is for
users; this file is how the system works.

The repository restarted from one commit at 0.7.18, so older commits and issue
numbers mentioned in the docs and the CHANGELOG aren't in this history.

**Order of trust: the code, then this file, then `docs/design/`.** The design
notes are history and several sections are stale; section 20 lists the known
drift. Module and function names below are the stable reference. They are
written as `module::function`, so grep for them.

Example names in this repo are invented: fleet `MAIN`, squadron `ARGOS`, ship
`S1`. Keep it that way (section 19).

## 1. Repo map

| Path | What it is |
| --- | --- |
| `crates/nimitz-core` | Every rule: config, registry, evidence, plans, execution, hooks, formats. Depends on no multiplexer |
| `crates/nimitz-mux` | The `Mux` trait, the Herdr adapter, `NoMux`, a fake `herdr` binary for tests |
| `crates/nimitz` | The `nimitz` binary: parses, prints plans, asks, attaches. Holds no rules |
| `crates/admiralty` | The `admiralty` TUI (ratatui) |
| `plugin/` | The Claude Code plugin: hooks, skills, two KB subagents |
| `.claude-plugin/marketplace.json` | Marketplace `claude-nimitz`; plugin id `nimitz@claude-nimitz` |
| `herdr-plugin/` | Herdr plugin `admiralty`: the overlay toggle |
| `docs/compat.toml` | Tested Claude Code range, compiled in with `include_str!` |
| `docs/usage.md` | Long-form user manual |
| `docs/design/` | Design notes and contracts |
| `docs/probes/`, `probes/` | P0 probes of Claude Code behaviour, with results per build |
| `docs/img/` | Logo and generated Admiralty screens |
| `tools/cc-validate` | Validates a new Claude Code release end to end |
| `tools/admiralty_screens.py` | Draws `docs/img/admiralty-*.png` from the real TUI |

Binaries: `nimitz`, `admiralty`, and `nimitz-sandbox`
(`crates/nimitz-core/src/bin/nimitz-sandbox.rs`), a stand-in used by core's own
tests. Workspace version lives in `[workspace.package]` in `Cargo.toml`.

## 2. Architecture

```text
             you / a member running nimitz
                 |                   |
          nimitz (CLI)          admiralty (TUI)
                 |  \             /  |
                 |   nimitz-core     |      rules, plans, evidence, execution
                 |        |          |
              nimitz-mux  |      nimitz-mux  ---> herdr (subprocess, panes)
                          |
          +---------------+----------------+
          |               |                |
   config files      state files     Claude Code files
   NIMITZ_HOME       NIMITZ_STATE    NIMITZ_CLAUDE_HOME
          ^               ^                ^
          |               |                |
          +--- claude CLI (--bg jobs) -----+
                     |
             member sessions --> plugin hooks --> `nimitz hook <Event>` --> state
```

Hard rules:

- **Rules live in core.** `crates/nimitz` parses and prints. Admiralty reads core
  and parses no Claude Code files.
- **Core never touches a multiplexer.** The CLI and Admiralty supply a
  `nimitz_mux::Shape` and run the pane steps.
- **`claude.rs` is the only place that knows Claude Code's flags.**
- **Plans never mutate.** Build a `Plan`, show it, then execute it. A refusal
  means no step runs.
- **No authority table since 0.7.3.** Every member may run every command.
  `caller` is ignored by the plans, except `plan_sweep` for its default scope.
  The rules text tells members what to leave to the user; nothing enforces it.

Concepts:

- A **member** is an admiral (`F`), commodore (`F/SQ`) or ship (`F/SQ/SHIP`).
- A **job** is a Claude Code background session (`claude --bg`), owned by the
  Claude Code daemon. It outlives any terminal. nimitz adds nothing to the
  process; it launches, labels and reads.
- A **pane** is a Herdr pane running `claude attach <job>`. Panes are views.
- **Crew** are a member's subagents, read from its transcript folder.
- A **stowaway** is an unlabelled session in fleet scope. A **copy** is a second
  session carrying a member's identity. Both are reported, never trusted.

## 3. Identity

- `types::Id(String)`, role by part count: 1 admiral, 2 commodore, 3 ship.
- **Names** are `[A-Z0-9_]+` (`config::valid_name`, no Windows device names).
- **Paths** (`paths.rs`): `F`, `F/SQ`, `F/SQ/SHIP`, `F/...`, `F/SQ/...`, `//...`.
  `/...` also works, because MSYS rewrites `//`. `//...` never covers the
  `ADMIRALTY` workspace. An empty match is `member.unknown`.
- **Session name** (`Id::name`): `[F]-ADMIRAL`, `[F.SQ]-COMMODORE`, `[F.SQ]-SHIP`.
  This is the `SendMessage` address. `sessions::id_from_name` parses it back.
- **Tab name** (`layout::tab_of`): `F.ADMIRAL`, `F.SQ.COMMODORE`, `F.SQ.SHIP`.
- **Labels** (`claude::labels_for`): `NIMITZ_FLEET`, `NIMITZ_SQUADRON` (not on an
  admiral), `NIMITZ_ROLE`, `NIMITZ_ID`, `NIMITZ_HOME`. A session is labelled when
  it has `NIMITZ_ID` or `NIMITZ_ROLE`; an inherited `NIMITZ_HOME` alone is no
  label.

Labels reach a worker only through `--settings` env. The daemon passes its own
env to every job, so a job's labels count only if its own launch saved them
(`evidence::job_saved_labels`, `job_backs_labels`).

Binding a session to a member:

1. The SessionStart hook calls `registry::identity_of`, then `bind_start`, then
   `registry::try_update` and `registry::apply_start`, with a 2 s lock budget.
2. A lost write is recovered: `status` replays event lines (`registry::replay`).
3. `sessions::bind_started` waits up to 15 s for the binding after a launch, then
   checks the session id, `registry::bind_job`, `claim_folder` and `verify_live`.

Row ownership in `snapshot.rs`, strongest first: registry job, registry session
(birth id included), labels in the right folder, lineage, retired session,
retired job. Never by name alone, except the ship-folder copy rule (section 9).

Caller: `sessions::caller_from_env`. No ID or ROLE is `Caller::User`. Partial,
disagreeing or malformed labels make `sessions::UNVERIFIED`, and the CLI warns
and runs it as the user.

## 4. Command reference

Help is authoritative for syntax: `nimitz help`, `nimitz help <command>`.
`COMMANDS` in `crates/nimitz/src/main.rs` is the dispatch table; `USAGES` in
`help.rs` is the text. Each command declares its own flags with `Args::allow`.

### Global flags

| Flag | Effect |
| --- | --- |
| `--json` | stdout is JSON only; human lines go to stderr (section 6) |
| `--plan` | Print the plan and stop |
| `--yes` | Run without the question |
| `--force` | Used by `start`, `restart`, `open`, `layout`, `muster`. `attach --force` is a usage error; others ignore it |
| `-v`, `--verbose`, `NIMITZ_VERBOSE=1` | Show found lines, numbered commands, per-step lines |
| `--help` | After the verb; `signal` only in first place |

The verb is always first: `nimitz --json status` is an unknown command. Colour
is on for a terminal only; `NO_COLOR` or `TERM=dumb` turns it off, and
`NIMITZ_COLOR=1|always` forces it (`NO_COLOR` still wins).

`VALUE_FLAGS` in `args.rs` is one global list, so a value flag on the wrong
command says "needs a value", not "unknown flag". Single-dash words are
positionals; `-v` is rewritten before parsing, except in `signal`.

### Paths

`F`, `F/SQ`, `F/SQ/SHIP`, `F/...`, `F/SQ/...`, `//...`. Several per command; a
member matched twice counts once. A path that matches nothing is refused. No `*`.

### Fleet shape (config)

```text
fleet new <F> [--machine L]
fleet set <F> [--admiral-mode M]
fleet rm <F>
squadron new|set <F/SQ> [--brief T] [--mode M] [--effort E]
squadron rm <F/SQ>
ship new <F/SQ/SHIP> --dir PATH|managed [--machine L] [--prompt T|--prompt-file P]
         [--mode M] [--depends A,B] [--effort low|medium|high|xhigh|max]
ship set <F/SQ/SHIP> [same flags] [--no-dir <label>] [--edit]
ship rm <F/SQ/SHIP>
machine add <F> <label> [--hostname H] [--dir F/SQ/SHIP=PATH]...
machine rm <F> <label>
machine set <label> [--claude-bin PATH] [--native-bin PATH] [--provider anthropic|bedrock|vertex]
settings get [<key>] [--fleet F]
settings set <key> <value> [--fleet F]
harness list [--fleet F]
harness add rule|agent|skill|settings <file|folder> [--name N] [--for F/SQ|F/SQ/SHIP [--member]]
harness rm rule|agent|skill <name>
harness rm settings [--for ID [--member]]
harness push
```

Notes: `ship new` needs `--dir`. With no prompt flag at a terminal it opens
`$VISUAL`, `$EDITOR`, else vi or notepad. `--effort ''` and `--mode ''` clear.
`--no-dir` and `--edit` are accepted but absent from help. `machine set` writes
the shared `machines.toml`, and `''` clears. `settings set` takes JSON or text;
`null` removes the key. `fleet|squadron|ship rm` are the exception to
"config only": they stop members and remove rows first (`crud::retire`).

### Session control (jobs)

```text
start <path>... [--fresh] [--parked] [--force]
restart <path>... [--parked] [--force]
stop <path|session>...
rm <path|session>...
sweep [<path>...]
adopt <session> <F|F/SQ|F/SQ/SHIP>
signal <path> <text> [--inbox|--type] [--from <label>]
signal session:<short-or-name> <text> [--from <label>]
```

`start`: a `...` path never cold-launches a ship, so a first launch needs the
ship named. `HOME/... HOME/WEB/API` is the idiom. `--parked` also wakes parked
and stopped ships a `...` covers. `--fresh` starts a new conversation.

`signal`: the user's own is typed into the member's tab, opening one if needed.
A member's, a Claude Code session's, or one with `--from` goes to the inbox and
names the verified sender. `--from` is a claim, shown as such. `--inbox` with
`--type` is a usage error; so are `session:` with `--type`, `--from` from a
member, and `//...`. Several members are all or nothing behind one gate.

### Pane control

```text
open <path>... [--tab] [--force]
attach <F|F/SQ|F/SQ/SHIP>
close <path>...
layout <path>... [--ships] [--here] [--rename] [--force]
```

`open` and `attach` wake a down job first. `close` closes only panes running the
exact attach, and never asks. `attach` execs on Unix and spawns on Windows.

### One plan over both

```text
muster [<path>...] [--parked] [--here] [--rename] [--force]
```

No path means `//...`. With no multiplexer it does the jobs and skips the panes.

### Inspection and setup

```text
status [<path>...]          the members, their state, their issues
check [<path>...]           every problem with its fix; exit 1 on an Error
rules [<path>...]           the role rules; no path needs a member caller
doctor [--fix]              this machine's install
init [--label L]            data root, machine label, trust, then doctor
trust <folder>              Claude Code's trust for a git root or folder
migrate --plan|--yes [--shared]
migrate fleet-skill <dir...> --into <F>
sandbox create <dir> [--herdr]
sandbox env <dir>
sandbox tick <dir> [--once]
```

`status` is not pure: it writes `active.json` and may replay the registry.

### Knowledge base

```text
kb path|status|head [--fleet F]
kb show <path> [--at <sha>]
kb grep <term> [<prefix>] [--at <sha>]
kb hash <path>
kb commit --title T --paths A,B --pages X,Y --note <sha256> --from "<kind> <id>" [--no-sync]
kb restore --paths A,B
kb sync
kb clone
```

### Which family changes what

Fleet shape changes config. Session control changes jobs. Pane control changes
panes. `muster` does both. The boundaries are not absolute: `open` and `attach`
wake jobs, `restart` reattaches a pane its relaunch cut, a user's `signal` can
open a tab, and `status`, `open`, `layout` and every restore write `active.json`.

### Gates

| Command | Gate | Executor |
| --- | --- | --- |
| fleet, squadron, ship, machine, settings set, harness add/rm, trust, migrate fleet-skill | `ctx.gate` | `crud::config_plan` |
| fleet/squadron/ship rm | `ctx.gate` in `crud::retire` | `sessions::apply` |
| stop, rm, sweep, adopt | `run::session_plan` | `sessions::apply` |
| start, restart, muster, layout, open, attach's wake | `restore::run_engine` | `restore::execute` |
| signal | `ctx.gate`, then `deliver` | |
| close | `out::gate(.., yes=true)` | never asks |
| init, doctor --fix | `ctx.gate` | inline |
| migrate | own `--plan`/`--yes` | no `Plan` |
| kb clone | own `--plan`/`--yes` | |
| **harness push, kb commit/restore/sync, sandbox create/tick** | **none** | **`--plan` is accepted and the command still runs** |

### Hidden commands

Dispatched before parsing, in `main.rs`:

- `hook <Event>`: always exits 0. `hook kb-guard` and `hook rules [harness]` are
  special cases; the word `harness` is never read.
- `__notify <key>`: delivers a needs-you notice. Only a ship whose `NIMITZ_ID`
  equals the key; else exit 2.
- `__self-end <id> <job> -- <args>...`: runs a member's own ending after its turn.
- `__fake-claude <base> ...`, `__fake-herdr <base> ...`: test stand-ins.
- `__commands`: prints `COMMANDS` minus `__*`; tests use it.

## 5. How a change flows

`main::dispatch`, in order:

1. `--help`, then the lean commands with no parsing or snapshot (`hook`, `__*`,
   version, help).
2. `-v` to `--verbose`, `Args::parse`, `out::set_json`, `out::set_verbose`.
3. `nimitz_mux::valid_mux_setting`: a bad `NIMITZ_MUX` is a usage error.
4. `sessions::caller_from_env`, build `Ctx`, `registry::set_strict(cmd != "doctor")`.
5. `auto_migrate`, unless `hook`, `__*`, `sandbox` or `migrate`. A file a newer
   nimitz wrote stops everything but `doctor`.
6. Find in `COMMANDS`. `Ok(code)` is a handled result, refusal included;
   `Err(text)` is a usage error, exit 2.

A command then runs:

1. `ctx.args.allow(...)`, parse paths, `ctx.snap()`.
2. Core builds a `Plan`.
3. `out::gate(plan, plan_only, yes)`:
   - `--plan`: print or JSON; exit 0, or 1 with refusals.
   - Refusals: print, exit 1.
   - No steps: `[OK] <summary>: nothing to do`, exit 0.
   - Print the plan. `--yes` runs.
   - stdin not a terminal: `[ERROR] not a terminal: pass --yes ...`, exit 1.
   - Prompt `Run N steps? [y/N]`.
4. Execute: `config::apply_config_plan`, `sessions::apply` or `restore::execute`.
5. `out::outcome`: `[OK] verified by re-reading` or `[ERROR] not verified`.

**Self-end.** A command that would stop, relaunch or remove the member running it
leaves that step to a detached `nimitz __self-end`. It waits for the turn to end
(600 s, `NIMITZ_SELF_END_WAIT`), runs the command with no member labels, logs to
`$NIMITZ_STATE/relaunch-self.log`, and leaves a note for the next SessionStart.

## 6. JSON API

`serde_json::to_string_pretty`. Enums use Rust names: `Severity` is
`Info|Warn|Error`; `Status` is
`NotStarted|Stopped|Parked|Idle|Busy|NeedsYou|Unknown`; `role` is lowercase.
Human words differ (`not-started`, `needs-you`), and **`down` exists only in
text**: it is `Parked` plus membership of the active set.

| Command | stdout with `--json` |
| --- | --- |
| `status` | `Snapshot`, exit 0 |
| `status <unknown path>` | **empty**, issue on stderr, exit 1 |
| `check` | `[Issue]`, most severe first |
| any plan with `--plan` | `Plan` |
| refused plan, no `--plan` | `Plan`, human view on stderr, exit 1 |
| a run through `config_plan`, `session_plan`, `run_engine` | `Outcome` |
| `sandbox env` | `{ "VAR": "value" }` |
| `migrate` | `{"nimitz": "...", "due": [...], "too_new": [...]}` |
| empty plan, no `--plan` | **nothing** |
| `doctor`, `harness list`, `trust` ok lines, usage errors | **nothing** |
| `rules`, `kb *`, `settings get` | plain text; `--json` is ignored |

Types, in `crates/nimitz-core/src/types.rs`:

```text
Snapshot { taken_at, machine, fleets[], stowaways[], issues[], cc_version, compat_ok }
Fleet    { id, dir, machine, admiral, squadrons[], issues[] }
Squadron { id, commodore, ships[], issues[] }
Member   { id, role, expected_name, cwd, status, reasons[], topic, session,
           extra_rows[], crew[], tasks[], issues[], pane, effort? }
SessionInfo { session_id, short_id, pid, kind, state, status, waiting_for,
              started_at, status_since, name, cwd, labels, pinned,
              birth_id?, entrypoint?, copy?, terminal? }
Issue    { code, severity, message, fix, id }
Plan     { summary, steps[], refusals[], warnings[], found[] }
Step     { id, cmd[], cwd, why }
Found    { id, found, action, job, pane, place }
Outcome  { ran: [[Step, exit_code, output]...], verified, issues[] }
```

A member is live when `session.pid` is non-null. With paths, `status` filters
`fleets`, but `stowaways` and top-level `issues` are not filtered.

`steps[].cmd[0]` is a program, or an internal verb: `write`, `mkdir`, `archive`,
`bind`, `trust`, `pin`, `label`, `inbox`, `type`, `herdr ...`.

`found` appears only on restore-engine plans. `found[].job` is one of `relaunch`,
`wake`, `launch`, `fresh`, `skip`, `refuse`, `nothing`; `pane` is `reattach`,
`open pane`, `focus` or empty.

**Trap:** on a restore-engine plan a member can be refused with `refusals: []`.
`start <missing-folder-ship> --plan --json` puts the Error in `warnings`, sets
`found[].job = "refuse"`, and exits 1. Test the exit code or `found[].job`.

### Errors and exit codes

`out::issue` prints:

```text
[ERROR] NOPE: NOPE matches no member; the fleets are BETA, MAIN (member.unknown)
       fix: nimitz status
```

Plan refusals print through `view.rs` with `refuse`, not `[ERROR]`. `Issue.fix` is
usually a `nimitz` command but can be free text. Human lines go to stdout unless
`--json`; the y/N prompt and the labels warning always go to stderr.

| Exit | When |
| --- | --- |
| 0 | OK, verified, ok `--plan`, nothing to do, help |
| 1 | Refused, not verified, cancelled, non-terminal without `--yes`, `check` errors, `doctor` problems, a member failed in a restore, unknown path in status or check, kb failure, file too new |
| 2 | Usage error, bad path, bad `NIMITZ_MUX`, bare `nimitz` |
| 141 | Closed pipe on Unix |
| 0 always | `hook` |

## 7. Core library API

Everything in `crates/nimitz-core/src`. `lib.rs` re-exports `types::*`.

| Module | Owns | Key API |
| --- | --- | --- |
| `types` | Data types, `REGISTRY_VERSION` 3 | `Id`, `Role`, `Plan`, `Step`, `Outcome`, `Snapshot`, `Member`, `Issue`, `Caller` |
| `paths` | Path grammar | `Path`, `parse`, `parse_all`, `covered`, `resolve` |
| `env` | Environment | `Env::{from_process,from_vars,registry_path,events_path,fleet_dir}`, `canon`, `git_root`, `which` |
| `config` | TOML config and CRUD plans | `load_all`, `load_fleet`, `validate`, `member_dir`, `ship_prompt`, `plan_*`, `apply_config_plan`, `valid_name`, `EFFORT_LEVELS`, `PERMISSION_MODES` |
| `registry` | `registry.json` | `load`, `update`, `try_update`, `save`, `bind`, `bind_job`, `apply_start`, `replay`, `identity_of`, `health` |
| `evidence` | Read-only Claude Code readers | `rows_from_files`, `events`, `transcript`, `read_subagents`, `read_tasks`, `read_job`, `FileCache`, `SessionRow` |
| `snapshot` | The picture | `snapshot(env, &mut Cache)`, `stowaway_fleet` |
| `sessions` | Identity actions and execution | `plan_start`, `plan_start_with`, `plan_stop`, `plan_stop_session`, `plan_relaunch`, `plan_rm`, `plan_sweep`, `plan_adopt`, `plan_retire`, `apply`, `session_issues`, `caller_from_env` |
| `restore` | The restore engine | `plan`, `plan_start`, `RestorePlan`, `RestoreStep`, `RestoreOpts` |
| `layout` | Tab naming and plans | `workspace_of`, `tab_of`, `attach_tab`, `plan`, `drop_shown` |
| `claude` | Claude Code argv | `launch_bg`, `relaunch_bg`, `resume_bg`, `stop`, `rm`, `attach`, `agents`, `run`, `settings_json`, `labels_for`, `scrub_child_env` |
| `check` | Problems | `check`, `check_tree`, `folder_issue` |
| `compat` | Tested range | `supported`, `installed`, `position`, `older` |
| `formats` | Migrations | `ARTIFACTS`, `CLEANUPS`, `run`, `migrate_local`, `due`, `too_new`, `guard` |
| `migrate` | Fleet-skill import, 0.3.x helpers. **Not the migration runner** | `plan_from_fleet_skill` |
| `hook` | The hook handler | `handle`, `handle_full`, `compact_events` |
| `rules` | Role rules | `VERSION`, `CHANGELOG`, `text`, `update_for`, `harness_for`, `brief` |
| `harness` | Per-fleet additions | `settings_for`, `merge`, `plugin`, `rules`, `issues` |
| `fleet_settings` | `settings.json` | `load`, `get`, `set`, `plan_set`, `kb` |
| `kb`, `kb_guard` | Knowledge base | `head`, `grep`, `show`, `commit`, `sync`, `clone`, `restore`; `hook`, `decide` |
| `notify` | Needs-you notices | `from_hook`, `record`, `clear`, `issues`, `lead_of` |
| `inbox` | Messages to a job | `record`, `candidates`, `send`, `frame`, `defuse` |
| `peers` | Non-member senders | `cc_caller`, `claim`, `neutralize`, `resolve` |
| `provider` | Provider checks | `plan_machine_set`, `check_launch`, `check_started`, `forced_selector`, `delivery_gate`, `issues` |
| `trust` | Claude Code's trust file | `target`, `too_broad`, `step`, `fix` |
| `active` | `active.json` | `note_start`, `note_end`, `user_mux_vars`, `member_mux_vars` |
| `pins`, `self_end` | Pins, deferred self-ending | `pin`, `unpin`; `claim`, `hold`, `release`, `write_note`, `take_note` |
| `os` | Platform layer | `pid_alive`, `is_same_process`, `spawn_detached`, `replace`, `hostname` |
| `sandbox` | Fake world | `create`, `env`, `env_for`, `fake_claude` |

### The plan pattern

`Plan` is plain data. `ok()` means no refusals. Steps are argv, and their kind is
read back from the argv by `sessions::classify`:

- `["bind",..]`, `["mkdir",_]`, `["write",_,_]`, `["archive",_]`, `["trust",_]`: config.
- `[_, "stop", s]`, `[_, "rm", s]`: stop, rm.
- no `--bg`: other, refused at run time.
- `--resume` as the last pair and no `--settings`: resume (wake).
- `--resume` otherwise: relaunch. Anything else: launch.

Two executors: `sessions::apply` runs everything, and
`config::apply_config_plan` runs config-only plans and verifies by re-reading.
`archive` renames to `<path>.archived`, then `.archived-2`, and so on.

### `sessions::apply`, step by step

1. Refuse if `!plan.ok()`.
2. Take per-identity locks `$NIMITZ_STATE/locks/<id>.lock`, sorted, held for the
   whole apply.
3. For ids that waited: skip if now live with the correct name.
4. `wait_settled` (5 s) before wakes, and wait out the crash window. Skip jobs the
   daemon restarted.
5. Check transcripts for wakes (`history.lost`).
6. Provider preflight for every launch, relaunch and resume, before any step.
7. Run steps, re-checking liveness before each launch or resume. A failed step
   stops the plan. Rm before Relaunch writes `pending/<id>.json`.
8. Copy on wake: stop and rm the copy, then retry once after 15 s + 2 s, unless
   `copy_cause` shows it was not a race.
9. `bind_started`, `provider::check_started`, job id check; pin leads.
10. Stop: `release_folder`, must not be live, wait 10 s for the worker to exit.
    Rm: the row must be gone, then unpin.
11. `verified` means no Error issues and all steps ran; `verify_live` for each.

## 8. Launching a member

`claude.rs` holds every flag. Two binaries:

- `env.claude_bin`: `NIMITZ_CLAUDE_BIN`, machines.toml `claude_bin`, PATH. Used
  for cold launch and relaunch.
- `env.native_claude_bin`: `NIMITZ_NATIVE_CLAUDE_BIN`, machine
  `native_claude_bin`, machine `claude_bin`, PATH. Used for wake, stop, rm,
  attach, agents and `--version`. A resume through a wrapper makes a copy.

| Action | Argv |
| --- | --- |
| Cold launch | `<claude> --bg --name <name> [--permission-mode M] [--effort E] --settings <json> [--plugin-dir P] [prompt]` |
| Relaunch | `<claude> --bg --resume <sid> --name <name> [--permission-mode M] [--effort E] --settings <json> [--plugin-dir P]` |
| Wake | `<native> --bg --resume <sid>`, no flags; keeps short id, name, mode and labels |
| Stop, rm, attach | `<native> stop\|rm\|attach <short>` |
| Agents | `<native> agents --json --all` |

**BUG-53:** `--resume <id>` with flags while the row exists makes an empty copy.
So a relaunch is stop, rm, then relaunch. `no_fork` asserts this, and `apply`
refuses at run time.

`--settings` JSON (`settings_json`): harness settings first, with nimitz's own
merged over them:

- `env`: the labels, `NIMITZ_STATE`, `NIMITZ_CLAUDE_HOME`, `NIMITZ_HOSTNAME`,
  blanked `HERDR_PANE_ID`, `HERDR_TAB_ID`, `HERDR_WORKSPACE_ID`, and the machine's
  forced provider selector.
- `permissions.allow`: `["Bash(nimitz *)"]`.
- `worktree.bgIsolation`: `"none"`.

`claude::run` runs steps with null stdin after `scrub_child_env`. Scrubbed
prefixes include `HERDR_`, `CLAUDE_BG_`, `CLAUDE_CODE_RESUME_`, `CLAUDE_BRIDGE_`.

### `sessions::start_member`, the relaunch table

Checks in order: unknown member; config load; no folder (`folder.missing`); legacy
refusal; `job.unreadable`, `labels.mismatch`, `name.taken`; `session.unreadable`;
`held`; live (Error refuses, else nothing); stopped with a live copy (`dup.live`);
session errors other than `name.drift`, `labels.missing`, `copy.rows`; `--fresh`.

Then by row:

| State | Action |
| --- | --- |
| Row `failed` | Refuse `lineage.broken` unless `--force`; forced: cold launch, then rm the row |
| Row stopped or parked | `wake_steps` |
| No row, `history.lost` or `lineage.broken` | Refuse unless `--force` |
| No row, pending relaunch or registry session with a transcript | `relaunch_bg` from the transcript |
| No row, ship with legacy rows | Relabel onto the newest |
| No row | Cold launch |

Afterwards, each non-live `extra_rows` row is removed.

`wake_steps`: `wake_target` chooses the conversation. A transcript is required
unless inside the crash window (30 s). Labels and name right: native
`resume_bg(birth)`. Otherwise relabel: `[stop if live], rm, relaunch_bg`.

## 9. Status and evidence

`snapshot::snapshot` is **not pure**: it replays the registry and clears this
machine's stale ship lock.

Evidence, in order: session files plus pid liveness (`os::is_same_process` guards
against pid reuse), `claude agents --json --all` every 10 s, `events.jsonl`, and
the multiplexer (never in core). `claude --version` every 60 s.

`evidence::rows_from_files` joins `jobs/<id>/state.json` with
`sessions/<pid>.json` by `jobId`. Session id: live `sessionId`, then
`resumeSessionId`, then birth `sessionId`.

Status, from `snapshot::build_member`:

| Condition | Status |
| --- | --- |
| Unreadable and not live | `Unknown` |
| Live, session `status: waiting` | `NeedsYou` |
| Live, `busy` or `shell` | `Busy` |
| Live, job row `needs` not an idle hint | `NeedsYou` |
| Live, `idle` | `Idle` |
| Job row, no pid, state `failed` | `Stopped` |
| Job row, no pid, last own event a `SessionEnd` from a pid with its own SessionStart | `Stopped` |
| Job row, no pid, otherwise | `Parked` |
| No row, known to the registry | `Stopped` |
| No row, unknown | `NotStarted` |

Text mode says `down` for `Parked` in the active set. `Member.reasons` says why.
Crew run while their log was written in the last 90 s and not failed.

Stowaways and copies:

- Stowaway: unclaimed, unlabelled, under a fleet folder or `NIMITZ_HOME`, in a
  member's exact folder, or named by a legacy map.
- Ship folders may be shared. An unclaimed, unlabelled row in one is a copy if it
  shares the transcript root, resumes a lineage id, has the ship's exact or
  legacy name, has Claude Code's copy name `<name>-<word>-<word>`, forks from the
  lineage with an empty name, or is a second job on the current conversation.
  Otherwise it's the user's own, and hidden.
- Lead folders are exclusive: any other live row is `lead.intruder`.
- `held`: a live process, no job, not a copy.

### Issue codes

`E` Error, `W` Warn, `I` Info. The authority is the source; grep for the code.

- **Sessions:** `name.drift` E, `name.taken` E, `labels.missing` E/W,
  `labels.mismatch` E/W, `dup.live` E/W, `copy` W, `copy.rows` W/I, `held` W/E,
  `folder.moved` E, `ship.elsewhere` E, `lineage.stale` W, `history.lost` E,
  `lineage.broken` E, `lineage.current` E, `member.unknown` E, `folder.missing` E,
  `prompt.missing` E, `session.unreadable` E/W, `session.none` E,
  `session.live` E/I, `session.interactive` E, `session.unknown` E,
  `step.failed` E, `pin.failed` W.
- **Snapshot:** `job.unreadable` E, `lead.intruder` E, `rc.exposed` W,
  `tender.retired` I, `aide.retired` W, `legacy` W/I.
- **Check:** `folder.untrusted` E/W, `cc.version` E/W, `stowaway` W.
- **Config:** `config.invalid` E/W, `config.layout` E/W, `config.conflict` W,
  `config.missing` E, `config.exists` E, `config.nonempty` E, `config.write` E,
  `config.pending` W, `machine.live` W, `machine.host` E, `prompt.file` W,
  `aide.legacy` W, `format.shared` W, `format.raise` W.
- **Formats:** `config.too_new` E, `format.failed` E, `format.machine` W.
- **Registry:** `registry.rebuilt` I, `registry.corrupt` E/W, `registry.unknown` W,
  `registry.readonly` E.
- **Rules, notify, harness, settings:** `brief.old` W, `rules.old` I,
  `notify.failed` W, `harness.invalid` W, `settings.unreadable`, `settings.key`,
  `settings.invalid`.
- **Provider:** `provider.unreadable` E, `provider.unconfirmed` W/E,
  `provider.mismatch` E/W, `provider.unset` E, `provider.unrecorded` W/E,
  `provider.model` W, `provider.unstopped` E, `provider.wake` W/E,
  `machine.unknown`.
- **Restore:** `mux.none` I/E, `mux.other` E, `restore.interrupted` I, `active.none` I,
  `machine.missing` E/W, `not.started` I, `tab.orphan` I, `tab.resumed` E,
  `lead.interactive` E, `stopped` I, `attaching` I, `tab.dup` W/I, `tab.moved` I,
  `tab.elsewhere` I, `tab.renamed` I, `tab.taken` E, `tab.agentview` E,
  `tab.unreadable` E, `turn.interrupted` I, `prompt.lost` I, `folder.trust` I,
  `self.end` I, `attach.old` I, `layout.skip` W, `usage` E.
- **CLI:** `attach.nested`, `harness.missing`, `inbox.none`, `mux.failed`,
  `signal.member`, `signal.self`, `signal.waiting`, `tab.unverified`, `trust.broad`,
  `effort.pending`, `focus.partial`, `pane.none`, `relaunch.old`.

## 10. The restore engine

`restore.rs` plans `muster`, `start <path>`, `restart`, `open` and `layout`. Core
plans and never talks to a multiplexer; `crates/nimitz/src/restore.rs` and
Admiralty supply the `Shape` and run the steps. Entry points are `restore::plan`
(`muster`, or `layout` with `shape_only`) and `restore::plan_start`.

Rules of the module: reuse before create; never type into a non-shell pane; resume
by exact id; leads always come back, and ships from the active set unless
`--parked`; one member, one tab, one job; idempotent; only the fleets asked for.

`RestoreStep`: `Wake` (through `sessions::apply`), `Claim` (`pane run <cmd>` into a
shell pane, optional rename), `Open`, `Pin`, `Rename`, `Relaunch`, `Split`,
`Focus`, `Reattach`.

`plan_member`, the decision table:

- **Held:** a pane holding it is `tab.resumed`; else `lead.interactive`,
  `session.interactive` or `held`. Never woken.
- **Named by exact path:** `plan_named`, through `plan_start_with`; a first launch
  is allowed. An unnamed `...` match never cold-launches a ship, but `plan_start`
  promotes never-ran leads to named.
- **Live:** a worker older than the installed Claude Code, or `--restart`, is
  relaunched; relaunches run last, ships then commodores then admirals. Self:
  deferred to `__self-end`. Otherwise repin, and add a tab step if wanted.
- **Job row, no pid:** wanted if `always_on`, `--parked`, or in the active set and
  not stopped. Refused beside a live copy, or on `TABLE_HANDLED` exceptions. Then
  by `find_tab`: shown attaches, shell claims, taken refuses, none opens.
- **No row:** with a transcript and wanted, relaunch from the transcript;
  otherwise `not.started`.

`wants_tab`: a lead always; a ship only if it had a tab
(`active.members[id].attached`), or with `--ships` when live.

`find_tab` order: a pane showing its job anywhere; full name in the workspace with
a shell; full name anywhere; legacy short name in the workspace; a default-named
tab in the workspace with a shell in the member's folder; none.

Progress goes to `restore.json` and is removed at the end. An interrupted run is
reported as `restore.interrupted`.

**Pins.** `pins::pin` writes `<claude home>/jobs/pins.json` under Claude Code's own
lock directory. nimitz's own ids are mirrored in `$NIMITZ_STATE/pins.json`, and
only those are ever removed. Leads are pinned; ships may park.

## 11. Hooks, rules and messages

### Hook events

All entries in `plugin/hooks/hooks.json` use `type: command`, `command: "nimitz"`
in argv form, and a 5 s timeout. `HOOK_EVENTS` in `setup.rs` lists what `doctor`
requires, and a test pins `hooks.json` to it.

| Event (matcher) | Calls | Effect |
| --- | --- | --- |
| SessionStart | `hook SessionStart` | Non-member: provider record. Member: inbox socket, provider record, rules delivery, `additionalContext`, binding, `active::note_start` |
| SessionStart, UserPromptSubmit | `hook rules harness` | Harness rules, as a separate entry for its own 10,000-character budget |
| SessionEnd | `hook SessionEnd` | Clears the notice; `note_end` unless reason is `clear` |
| UserPromptSubmit | `hook UserPromptSubmit` | Clears the notice; rules update once per version per job |
| Stop | `hook Stop` | Clears the notice |
| Notification | `hook Notification` | Ship: needs-you notice on `permission_prompt`, `elicitation_*` or no type |
| SubagentStart, SubagentStop | same name | Logs the agent |
| PreToolUse (SendMessage) | `hook PreToolUse` | Logged with `to`, never judged |
| PreToolUse (AskUserQuestion) | `hook PreToolUse` | Ship: question notice |
| PreToolUse (`*`) | `hook kb-guard` | Guards the KB subagents |
| PermissionRequest | `hook PermissionRequest` | Ship: permission notice |

`hook::handle_full` runs under `catch_unwind` and never fails. Pid resolution walks
through shells to the process with `sessions/<pid>.json` (`claude_pid`). A
`spare: true` session file clears labels. Each logged line goes to `events.jsonl`,
compacted above 4 MiB, keeping each session's newest start and end plus the last
2000 lines.

### Rules

- `rules/common.md` plus the role's files, compiled in with `include_str!`:
  `leads.md` + `admiral.md`, `leads.md` + `commodore.md`, or `ship.md`.
  Placeholders: `{VERSION} {ID} {NAME} {ROLE} {WHERE} {FLEET} {SQ} {PARENT_NAME} {PARENT}`.
- `rules::VERSION` is 12. A change needs a bump and one `CHANGELOG` entry in
  `rules.rs`. Members get `nimitz rules updated vN→vM` at their next prompt, with
  the last 3 entries and the full rules.
- Delivery key: `jobId`, else `session:<id>`. Recorded in `rules-delivered.json`.
- Context limit `CONTEXT_MAX` 9800 characters.
- Briefs: `rules::brief`, with `BriefState` Current, Old, Custom or Missing.
  `doctor --fix` replaces Old only.

### Notices, inbox and signals

- Needs-you: `notify::from_hook`, ships only. `__notify` delivers to the commodore,
  detached: inbox first, typed into the pane as a fallback, retrying for 600 s
  (`NIMITZ_NOTIFY_RETRY`). Record state is `pending`, `delivered` or `failed`.
- `inbox::send` posts a `user` message framed as `<cross-session-message ...>`
  from `nimitz`. The receiver must be at least 2.1.293, else the CLI types. Unix
  checks peer credentials; Windows uses the pipe with the session's key file.
- `defuse` neutralises frame tags in a body. `peers.rs` handles non-member senders,
  and trusts only labels or a session file found by ancestry.

### Provider gate

`machine set --provider` makes a machine gated.

- **Preflight:** `<bin> --settings <S> auth status --json` in the member's folder,
  20 s. A different answer is `provider.mismatch`; none is `provider.unconfirmed`.
- **Selector:** `CLAUDE_CODE_USE_BEDROCK=1` or `_VERTEX=1` forced into `--settings`.
- **After a start:** `check_started` reads this start's SessionStart record. A wake
  gets 10 s and refuses; a cold launch gets 3 s and warns.
- **Wakes** whose saved flags don't select the provider become relaunches.
- **Deliveries** wait for the record (`delivery_gate`).

`docs/usage.md` lists what is checked and the known gaps.

### Trust

`trust.rs` writes `projects["<key>"].hasTrustDialogAccepted` in `~/.claude.json`
and the Claude home's `.claude.json`, under `<file>.lock`, with a backup in
`$NIMITZ_STATE/backups` (newest 5). `too_broad` refuses the filesystem root, home
or a parent of it, and a parent of the data root. Inside `NIMITZ_HOME`, a folder is
trusted only when the data root itself is trusted.

### Compat

`docs/compat.toml` has `min`, `max` and an optional `prerelease` list, compiled in,
so a change needs a rebuild. Below `min`, unknown, or an unlisted pre-release:
`cc.version` Error, and `--force` overrides. Above `max`: a warning only.

### Harness

`$NIMITZ_HOME/<fleet>/harness/`: `rules/`, `agents/`, `skills/`,
`settings/{fleet.json,squadrons/,members/}`. Settings are limited to `ALLOWED`
(`model`, `outputStyle`, `effortLevel`, `permissions`, `env`); env only `FLEET_*`;
a deny or ask covering `nimitz` is dropped; files are at most 64 KiB, no links.
Merged fleet, squadron, member, under nimitz's own. A generated plugin
`fleet-<fleet>` lives at `$NIMITZ_STATE/harness/<fleet>/plugin`, passed with
`--plugin-dir`.

### Knowledge base

A git repository per fleet, set by `kb.remote` and `kb.dirs` in the fleet's
`settings.json`. git runs with program-running options off. Commits are
`ingest(auto): <title>` with `note:` and `from:` trailers. `sync` is `pull --rebase`
then `push`. `OFF_LIMITS` protects agent files. `hook kb-guard` answers only for
`nimitz:kb-scribe` (Bash verbs `path|status|grep|hash|commit|restore`) and
`nimitz:kb-ref` (`path|status|head|show|grep`). It's a hook because plugin
subagents ignore `hooks:` in frontmatter.

## 12. On-disk state

`H` is `NIMITZ_HOME` (default `~/.claude-nimitz`, shared, may be a synced folder).
`S` is `NIMITZ_STATE` (default `~/.local/state/nimitz`, per machine, on Windows
too). `C` is `NIMITZ_CLAUDE_HOME` (default `~/.claude`).

### Shared

```text
H/machines.toml                                   claude_bin, native_claude_bin, provider per label
H/<fleet>/fleet.toml                              strict: format, min_nimitz, fleet, [machines.<label>], [admiral]
H/<fleet>/admiral/CLAUDE.md                       brief
H/<fleet>/squadrons/<sq>/squadron.toml            format, min_nimitz, squadron, brief, permission_mode, effort, [[ships]]
H/<fleet>/squadrons/<sq>/{SQUADRON.md,CLAUDE.md}
H/<fleet>/squadrons/<sq>/prompts/<SHIP>.{md,txt}
H/<fleet>/ships/<sq>/<ship>/                      managed ship folder, with .nimitz-ship.lock
H/<fleet>/settings.json                           fleet settings
H/<fleet>/legacy.toml                             old name -> id
H/<fleet>/machines/<label>.json                   last nimitz version seen
H/<fleet>/harness/                                section 11
```

`[[ships]]` has `name`, `prompt`, `prompt_file`, `depends_on`, `dirs{label}`,
`permission_mode{label}`, `effort`.

### Per machine

```text
S/registry.json        identities: role, name, cwd, job, session, lineage[]; retired
S/registry.lock        also guards active.json
S/registry.json.bak    every write
S/replay.json          replay watermark
S/events.jsonl         one Event per line; only the hook appends
S/active.json          last known active set, Herdr identity
S/rules-delivered.json
S/needs/<id>.json      notices
S/inbox/<session>.json socket records
S/pending/<id>.json    relaunch in progress
S/provider/sessions/<session>.json
S/pins.json            nimitz's own pins
S/restore.json         progress of a restore run
S/locks/               per-identity and KB locks
S/self-end/            locks and notes
S/relaunch-self.log
S/backups/             trust backups
S/harness/<fleet>/plugin/
S/archive/             cleanups
S/formats.json         migration manifest
S/migrations.log
S/admiralty-ui.json    Admiralty's sticky state
S/admiralty-actions.log
```

Lineage `via` values: `launch`, `resume`, `relaunch`, `clear`, `branch`, `fork`,
`compact`, `adopt`, `migrate`, `hook`. Unknown fields in the registry are preserved.

### Claude Code's files

Read tolerantly: `C/sessions/<pid>.json`, `C/jobs/<id>/state.json`,
`C/projects/<dir>/<sid>.jsonl` with `subagents` and `workflows`, `C/tasks/<team>`,
`C/daemon.log`, `C/settings.json`, managed settings. Written only:
`C/jobs/pins.json` and the trust entries, under Claude Code's own locks.

### Formats and migrations

`formats::ARTIFACTS` is the table. `formats.rs` is the runner.

| Artifact | Scope | Current | Oldest |
| --- | --- | --- | --- |
| fleet | shared | 2 | 1 |
| squadron | shared | 2 | 1 |
| fleet-settings | shared | 1 | 1 |
| legacy | shared | 1 | 1 |
| brief | shared | 1 | 1 |
| ship-lock | shared | 1 | 1 |
| machines | shared | 1 | 1 |
| registry | local | 3 | 2 |
| active | local | 1 | 1 |
| rules-delivered | local | 2 | 1 |
| events | local | 1 | 1 |
| restore | local | 1 | 1 |
| pending | local | 1 | 1 |
| inbox-records | local | 1 | 1 |
| needs | local | 2 | 1 |
| own-pins | local | 1 | 1 |
| admiralty-ui | local | 2 | 1 |
| provider-records | local | 1 | 1 |

- **Local runs are automatic:** `main::auto_migrate` calls
  `formats::migrate_local` unless `up_to_date`.
- **Shared runs** need `nimitz migrate --shared --yes`.
- **Per file:** guard, lift if below `oldest`, one step at a time re-reading the
  file, backup `<file>.pre-<version>` once, atomic write, a line in
  `migrations.log`.
- **Guard:** `format` above current, `min_nimitz` above this nimitz, or a format
  below 1 is `TooNew`: never read further, never rewritten.

### Locks

All are file locks on dedicated `.lock` files, or Claude Code's lock directories.

| Lock | Guards |
| --- | --- |
| `S/registry.lock` | Registry and active set; hook budget 2 s, active 1 s |
| `S/locks/<id>.lock` | Per-identity, held for a whole `apply` |
| `S/locks/kb-<hash>.lock` | KB commit and sync |
| `S/events.lock` | Shared for appends, exclusive for compaction |
| `S/rules-delivered.lock` | 1 s budget; no record means no update |
| `S/migrate.lock` | One migration at a time |
| `C/jobs/pins.json.lock`, `<config>.lock` | Directory locks shared with Claude Code |
| `<ship folder>/.nimitz-ship.lock` | Cross-machine marker, not a file lock |

All writes are temp plus rename. `registry::refuse_shrink` refuses a write that
drops an identity. Never truncate a lock file.

## 13. Environment variables

Empty means unset. On Windows names are upper-cased.

| Variable | Effect |
| --- | --- |
| `NIMITZ_HOME`, `NIMITZ_STATE`, `NIMITZ_CLAUDE_HOME` | The three roots |
| `NIMITZ_CLAUDE_BIN`, `NIMITZ_NATIVE_CLAUDE_BIN` | The two claude binaries |
| `NIMITZ_HOSTNAME` | Machine name, lower-cased |
| `NIMITZ_ID`, `NIMITZ_ROLE`, `NIMITZ_FLEET`, `NIMITZ_SQUADRON` | Member labels |
| `NIMITZ_MUX` | `auto`, `herdr` or `none`; anything else is a usage error |
| `HERDR_ENV` | `1` means inside Herdr |
| `NIMITZ_HERDR_BIN`, `NIMITZ_HERDR_SESSION`, `NIMITZ_HERDR_SOCKET` | Which Herdr |
| `NIMITZ_BIN` | Admiralty's nimitz binary |
| `NIMITZ_COLOR`, `NO_COLOR`, `NIMITZ_COLUMNS`, `NIMITZ_VERBOSE` | Output |
| `NIMITZ_NO_ANIM` | Admiralty motion off |
| `NIMITZ_CRASH_WINDOW` (20) | Seconds after a crash to wait |
| `NIMITZ_PREFLIGHT_TIMEOUT` (20) | Provider preflight seconds |
| `NIMITZ_SIGNAL_WAIT`, `NIMITZ_NOTIFY_RETRY` (600), `NIMITZ_SELF_END_WAIT` (600), `NIMITZ_REATTACH_WAIT`, `NIMITZ_FOCUS_RETRY_MS` | Waits |
| `ADMIRALTY_ICONS=ascii`, `ADMIRALTY_STATE`, `ADMIRALTY_EXIT_AFTER_JUMP`, `ADMIRALTY_INSET`, `ADMIRALTY_ARGS`, `ADMIRALTY_NO_CLIPBOARD`, `ADMIRALTY_MOUSE=0` | Admiralty |

Test only: `NIMITZ_E2E*`, `NIMITZ_HERDR_E2E`, `NIMITZ_BLESS`, `NIMITZ_HOOK_PID`,
`NIMITZ_HOOK_SESSION_FILE`, `NIMITZ_PLUGIN_HOOKS`, `NIMITZ_TEST_ANCESTORS`,
`NIMITZ_ATTACH_DRY`, `NIMITZ_INBOX_PID`, `NIMITZ_OS_PROBE`.

`CLAUDE_CONFIG_DIR` is not read. Use `NIMITZ_CLAUDE_HOME`.

## 14. Multiplexer

`crates/nimitz-mux/src/lib.rs`:

```rust
trait Mux {
    // required
    fn name(&self) -> &'static str;
    fn caps(&self) -> Caps;                    // open, focus, type_text, layout
    fn panes(&self) -> Vec<Pane>;
    fn open(&self, cmd: &[String], cwd: &Path, place: Place) -> Result<PaneRef, MuxError>;
    fn focus(&self, pane: &PaneRef) -> Result<(), MuxError>;
    fn type_text(&self, pane: &PaneRef, text: &str) -> Result<(), MuxError>;
    fn layout(&self, plan: &LayoutPlan) -> Result<Vec<PaneRef>, MuxError>;
    // defaulted
    fn focus_now(&self, pane: &PaneRef) -> Result<(), MuxError>;
    fn tab(&self, place: &Place) -> Option<PaneRef>;
    fn foreground(&self, pane: &PaneRef) -> Result<Vec<String>, MuxError>;
    fn foreground_procs(&self, pane: &PaneRef) -> Result<Vec<(Vec<String>, Option<i64>)>, MuxError>;
    fn run_in(&self, pane: &PaneRef, cmd: &[String]) -> Result<(), MuxError>;
    fn shape(&self, foregrounds: bool) -> Result<Shape, MuxError>;
    fn rename_tab(&self, tab_id: &str, label: &str) -> Result<(), MuxError>;
    fn close_pane(&self, pane: &PaneRef) -> Result<(), MuxError>;
}
```

`MuxError` is `Unsupported(&str)`, `Failed(String)` or `Unfocused(String)`: the tab
is in front but the agent focus failed. Adapters: `Herdr`, `NoMux`, and test-only
`TestMux`. **`fake.rs` is not an adapter.** It stands in for the `herdr` binary,
and the real adapter reaches it through a subprocess, which tests the argv and
JSON parsing. Its world is `<base>/world.json`, its call log `<base>/calls.jsonl`.

`detect(vars)` is pure in `vars`:

1. `NIMITZ_MUX=none`: `NoMux`.
2. `NIMITZ_MUX=herdr`: `Herdr`.
3. `auto`, empty or unset: `Herdr` only if `HERDR_ENV=1` and a binary resolves.

Callers never pass the raw env. They pass `active::user_mux_vars` or
`member_mux_vars`, so a member, or Admiralty started from a plain terminal, uses
the Herdr server recorded in `active.json`.

The adapter calls `herdr` as a subprocess for every operation: `agent list|focus|prompt`,
`api snapshot`, `pane process-info|list|get|run|split|close`,
`workspace list|create|focus`, `tab list|create|rename|focus`. It parses JSON from
stdout, and pure parsers are tested on `tests/fixtures/herdr-0.9.3`. **There is no
call timeout**, so a hung `herdr` hangs its caller. `panes()` turns any error into
an empty list. Each session claim is re-judged by the pane's foreground process,
because Herdr's hook names a pane by an inherited `HERDR_PANE_ID`.

`mux.none` is an issue code: a refusal for `open`, `layout`, `close` and a `signal`
that must type; Info for `muster`, which does the jobs. `start`, `restart` and
`stop` never need a multiplexer.

## 15. Admiralty

Pipeline: `snapshot` + panes, to the `view` node tree, to flat rows, to `frames`
lines. `app` holds state and keys, `render` draws, `run` is the terminal loop.

| Module | Role |
| --- | --- |
| `main` | Args, env, `dump`, `sandbox`, `herdr` subcommands |
| `run` | Terminal loop, loader thread, workers, `$EDITOR` |
| `app` | `App` state, every key, `Modal`, `Effect`, `Saved`, `HELP`. No terminal |
| `mouse` | `Hits` (what the last frame put where), `App::on_mouse`. No terminal |
| `actions` | `Ctx`, `Target`, `Act`, `Pending`, availability, plans, CLI calls, `apply`, log |
| `data` | `Bundle`, `DataSource`, `CoreSource`, `FixtureSource`, `merge_check` |
| `view` | Snapshot to nodes and rows, `SHIP_INDENT`. Pure |
| `frames` | Rows to styled lines, widths. Pure |
| `render` | `draw`, tab line, status rows, modals |
| `detail` | The row's detail panel |
| `finder` | The `space` finder's candidates and matching (nucleo-matcher); `render` draws it |
| `forms` | Create and edit forms, validated through core plans |
| `theme` | Palette, status words, `Icons`, art |
| `backend` | `GlyphSafe` backend: positions never depend on the terminal's width table |
| `fixtures` | Demo world |
| `herdr_plugin` | `herdr toggle|watch|rezoom` |

Modes: `admiralty`, `--demo`, `--demo-monitor`, `--fixture FILE`, `--sandbox DIR`,
`--interval SECS`, `--exit-after-jump`, `dump [--width N] [--ansi] [--height N]
[--select ID] [--detail]`, `sandbox create|env`, `herdr toggle|watch|rezoom`.

### Popups

Every popup goes through `render::panel`: one of three widths (`PANEL_SMALL` 50,
`PANEL_STD` 100, `PANEL_WIDE` 160), the height of its content, centred in the
body, stopping at `PANEL_MARGIN`. Past that it scrolls, with `border_bar` on its
right border. `modal_block` is the one frame, `scrolled` draws a body with its
keys always under it, and `wrapped` makes rows exact, so nothing is cut off. The finder has a fixed height (`FINDER_HEIGHT`) so it doesn't jump as its
results change. `split` gives the wide popups a list and the detail beside it,
from 120 columns. The message panel reads `evidence::last_output` once, when it
opens. `App.panel_rows` holds what the last draw could scroll.

### Mouse

`draw` starts a fresh `App.hits` (`mouse::Hits`) and records what is
hit-testable as it draws: the tabs' columns, the list's lines and spacing, each
popup's rect, each entry's screen rows, the actions menu's detail. `on_mouse`
reads an event against that frame only (it ignores one from before any draw or
from another popup), inside `area`, so the inset is never hit. Everything
routes to the keys' code: a double click is `enter`, a popup's wheel is up and
down (ctrl-e / ctrl-y over the menu's detail), a click outside a popup is `esc`
where that loses nothing (`outside_closes`: not a confirm, a form, or a message
with text). A click on an entry sets the `sel` the arrows set; on a fleet
filter's `[x]` it is `space`. New code paths: the list's wheel moves `scroll`,
the cursor kept on a row in view (`scroll_list`); a form's wheel is
`panel_scroll`, PgDn's code in smaller steps. The fold marker's column is
`frames::fold_x`, next to the code that draws it; a test checks the glyph is
there at every width. Nothing on the confirm panel is hit-testable. The loop
drops pointer moves, releases and drags before they wake a draw, and every
other event is followed by a draw, so a click always meets the frame on screen. On Unix `run::set_mouse` asks for clicks, drags and the wheel only (1000, 1002,
SGR 1006), never bare pointer moves, and a terminal that refuses capture leaves
Admiralty on the keyboard. A click with more input already waiting
(`App.mouse_burst`) makes no double click: it was queued while main was busy.

Capture: `init_terminal(app.mouse)` turns it on after the terminal is set up;
`release_input` turns it and bracketed paste off in `leave_terminal` (quit,
errors, before `$EDITOR`) and in the panic hook. `M` flips it
(`Effect::Mouse`); `ADMIRALTY_MOUSE=0` starts it off. Not done: dragging a
scrollbar, clicking a form's field, a kill by signal (raw mode is not
restored then either).

### Loop

A **loader** thread owns the `DataSource`: load, send, wait the interval (default
2 s, floor 0.5 s), or loop at once on a partial look. A **worker** thread per
action runs `actions::apply`. **Main** draws, reads keys and carries out `Effect`s
(`Quit`, `Refresh`, `Run(Pending)`, `Editor`, `Mouse`). `prepare_start`, focus and typing
block main.

The first load uses a throwaway env with no claude binary: it's a fast partial
look with no `claude agents`, `check` or `session_issues`. `compat_ok` is forced
true, so `Claude Code: not found (supported)` can flash on the first frame, and
`admiralty dump` on live or sandbox data renders only this partial look. Tests of
`dump` use `--demo`.

`App::apply` adds panes with `pane_for` and makes one status override: a blocked
pane on a live session becomes `NeedsYou`.

### Actions: Admiralty does both

| Action | Plan from | Runs through |
| --- | --- | --- |
| Start, restart, up, muster, open (live) | `nimitz ... --plan --json` | `nimitz ... --yes --json` |
| Stop, sweep, rm, adopt, remove | `sessions::plan_*` | `sessions::apply` |
| Forms | `config::plan_*` | `config::apply_config_plan` |
| Layout | `layout::plan` + `drop_shown` | `mux.layout` |
| Message | `plan_message` | `mux.type_text` |
| Jump, `A` on a shown pane | none | `mux.focus_now` |

`cli_command` runs the CLI as the user: member labels removed, the roots set.
`nimitz_bin` uses `NIMITZ_BIN`, then a sibling binary, then PATH. On Windows the
sibling test misses `nimitz.exe`, so it falls through to PATH.

`m` needs a live member whose pane's session id matches. It runs
`provider::delivery_gate`, requires `attach_check` to be `Exact`, types, then checks
`typed_into_view`. It displays `nimitz signal` but doesn't run it.

### Keys

From `app::handle_key`. Ctrl is ignored on the main list unless named.

| Key | Action |
| --- | --- |
| `q`, `ctrl-c` | Quit |
| `esc` | Clear the filter, else quit |
| `tab`, `shift-tab`, `1`-`9` | Fleet tabs |
| `j`/`k`, arrows | Move |
| `h`/`l`, left/right | Fold or parent; unfold or child |
| `gg`, `G`, Home, End | Top, bottom |
| `ctrl-d`/`ctrl-u`, PgDn/PgUp | Half page |
| `J`/`K` | Rows with a pane |
| `space` | Finder |
| `enter` | Row with a pane or no children: jump. Else fold |
| `z` | Fold; on old crew, `+N done` |
| `/`, `n`, `N` | Filter, next, previous |
| `!`, `i`, `?` | Needs list, issues, help |
| `I`, `f`, `F` | Stowaways, state filter, flagged only |
| `R`, `y`, `v` | Refresh, yank session id, motion |
| `M` | Mouse capture on / off |
| `a`, `u`, `A`, `D` | Start, up, open, remove |
| `x` | Actions menu: `a u A s r w x d D m`; on a fleet `R L S M` |
| `c` | Create menu: `f s p e` |
| `e` | Edit ship, squadron or fleet |
| `m` | Message |

Confirm: `y` or `enter` runs if the plan is ok; `n`, `esc`, `q` close. Finder:
`enter` lands, `ctrl-o` lands and jumps, `ctrl-p`/`ctrl-n` move, toggles `ctrl-a`
pane only, `ctrl-t` topic, `ctrl-s` stowaways, `ctrl-f` this fleet, `ctrl-l` ships.
`!` and `i` take `ctrl-o` too. Forms: `ctrl-s` submit, `ctrl-e` `$EDITOR`.

### Layout limits

Columns NAME, STATUS, AGE, DETAIL: minimums 12, 12, 4, 0, maximums 29, 14, 5, none.
Padding goes below 71 columns, computed in `frames::spacing_for`. Under 100 columns,
counts show only needs-you and errors. Rows cut below 33 columns. Status rows
shorten by fit, not at a fixed width.

A squadron's ships sit two columns in from their commodore
(`view::SHIP_INDENT`), and their crew and notes follow them.

The finder (`render::finder_areas`, `draw_finder`) is one group in the middle of
the view, filling it but for a margin of 3 columns and 1 row (`FINDER_MARGIN`).
From 120 columns the detail panel sits beside the list at the same height, and the
list takes 58% of the group, at least 72 columns. Below 120 there's no detail. The
list has columns like the main view: NAME (a member's path, a stowaway's name) as
wide as its longest entry, RANK, STATUS and, with 12 columns or more left, TOPIC. A
member's expected name and a stowaway's short id still match but aren't shown. The
detail panel has the popups' thick frame.

### Theme

"Monokai refined". Five hues, one meaning each: `NEEDS #ff5c93`, `ERROR #fd971f`,
`WARN #d8c766`, `BUSY #a6e22e`, `ACCENT #66d9ef`. Background `BG #272822`. Motion
runs on a 100 ms tick only while something moves and no dialog is open.

## 16. Plugins

### Claude Code plugin (`plugin/`)

Installed machine-wide, not per member, so every session gets the hooks and skills.
Sessions load Claude Code's cached copy, never the repo, so a change needs a
marketplace update. `nimitz` must be on PATH, because hooks call it by name.

- Skills: `admiral`, `commodore`, `fleet` trigger by description; `kb-ingest` and
  `kb-query` are `user-invocable: false`, loaded by the agents.
- Agents: `kb-ref` (read-only, returns answer, citations, gaps, confidence, commit)
  and `kb-scribe` (one commit per note).
- Role rules are not skills. They arrive as `additionalContext`.
- `plugin/**/*.md` must stay LF (`.gitattributes`): Claude Code's frontmatter
  fallback skips CRLF lines.

**A change under `plugin/` needs a version bump in
`plugin/.claude-plugin/plugin.json`**, or `tests/plugin_version.rs` fails.

### Herdr plugin (`herdr-plugin/`)

Manifest `herdr-plugin.toml`: id `admiralty`, action `toggle`, overlay pane `watch`,
both the `admiralty` binary (`admiralty herdr toggle|watch`), because Herdr runs an
argv with no shell and a script could not serve Windows. Overlay, not popup: a
popup is session-modal and returns no pane id. `bin/*.sh` are compatibility shims.

`herdr_plugin::toggle_with`: open, close, or move between tabs. `pane_id` in the
plugin state dir holds the overlay; a stale id fails `pane get`, and a reused id
fails the `Admiralty` label check. Watch zooms its own pane off and on so Herdr
sizes the terminal to the box, and the covered pane's zoom is restored afterwards
(`docs/design/herdr-overlay-size.md`).

The `[[keys.command]]` block in `herdr-plugin/README.md` is test input for
`tests/check.sh`.

## 17. Testing

```sh
cargo test --workspace                 # unit and integration
cargo test -p nimitz --test cli        # one target
cargo test --test formats              # both crates' format guards
NIMITZ_E2E=1 NIMITZ_E2E_LIVE_FLEETS=<fleets> cargo test -p nimitz --test e2e -- --test-threads=3
python3 tools/test_cc_validate.py      # Unix only: imports fcntl
```

| Target | Covers |
| --- | --- |
| `nimitz/tests/cli.rs` | The CLI end to end, help coverage, roles, JSON |
| `nimitz/tests/restore.rs` | Restore scenarios R-01 onward |
| `nimitz/tests/adversarial_round1.rs` | One regression per bug |
| `nimitz/tests/upgrade.rs` | A 0.4.0 install fixture under this build |
| `nimitz/tests/plugin_version.rs` | `plugin/` changes bump `plugin.json` |
| `nimitz/tests/e2e.rs` | Real Claude Code, opt-in, needs tmux |
| `nimitz-core/tests/relaunch_table.rs` | The relaunch table |
| `nimitz-core/tests/identity_matrix.rs`, `snapshot_evidence.rs`, `stowaway_rule.rs` | Identity and evidence |
| `nimitz-core/tests/formats.rs` | Each format change has a step, fixture and shape |
| `nimitz-core/tests/plugin_frontmatter.rs` | Strict YAML in LF and CRLF |
| `nimitz-core/tests/migrate_old_fleets.rs` | Synthetic legacy fleets |
| `nimitz-mux/tests/mux.rs` | Custom harness; its binary doubles as fake herdr, and a name filter isn't honoured |
| `admiralty/tests/ui.rs`, `finder.rs` | Frames and keys on fixtures |
| `admiralty/tests/sandbox.rs` | Real core path through the App |
| `admiralty/tests/cli_drift.rs` | Commands Admiralty shows against the CLI. Open, up, signal and layout are parse-only |
| `admiralty/tests/glyphs.rs`, `formats.rs`, `adversarial_g.rs` | Glyph widths, `Saved` shape, regressions |

Herdr plugin: `tests/check.sh` (offline, not on Windows),
`uv run --with pyte python tests/e2e_isolated.py`,
`cargo test -p admiralty herdr_plugin`. `NIMITZ_HERDR_E2E=1` runs the mux test
against a private Herdr server.

Other test switches: `NIMITZ_E2E_MODEL`, `NIMITZ_E2E_OLD_BIN`, `NIMITZ_E2E_OLD06_BIN`,
`NIMITZ_E2E_KB`, `NIMITZ_E2E_PROBE`. `NIMITZ_BLESS=1` regenerates pinned shapes, only
after the version, step and fixture are in.

### Sandbox

```sh
nimitz sandbox create <dir> [--herdr]
eval "$(nimitz sandbox env <dir>)"
admiralty --sandbox <dir>
```

A fake `claude`, and fleets `MAIN`, `BETA`, `LAYOUT` seeded with every broken state.
`create` wipes `<dir>` if it's already a sandbox and refuses any other non-empty
folder. `sandbox env` prints `HOME`, the `NIMITZ_*` roots, the fake binaries,
`NIMITZ_HOSTNAME` and `NIMITZ_MUX=none`; with `--herdr`, also `NIMITZ_HERDR_BIN`,
`HERDR_ENV=1` and `NIMITZ_MUX=herdr`.

**Trap, from inside a member session:** `sandbox env` unsets nothing, so
`NIMITZ_ID`, `NIMITZ_ROLE`, `NIMITZ_FLEET`, `NIMITZ_SQUADRON` and `HERDR_*` stay, and
you remain a member. Run with `env -i` and re-add what you need. Never run real
fleet commands while testing.

## 18. Tools

### `tools/cc-validate`

`tools/cc-validate [--plan] [--force]`, Unix only. Validates a new Claude Code in a
fresh worktree from `dev`:

1. Compare `claude --version` with compat's `max`; not newer: done.
2. The checkout must be clean, on `dev`, level with `origin/dev`.
3. Snapshot live fleets; create worktree `ci/cc-<v>`.
4. Build, unit suite, full e2e.
5. Hook-payload probe with two real jobs, and `doctor`'s pinned formats.
6. Diff six markers of Claude Code internals against the tested binary.
7. Compare live fleets again.
8. On a pass with a newer version: bump, commit, merge into `dev`,
   **push origin dev**, install, update the plugin, run `doctor`.

A failure before the merge changes nothing; one after it leaves `dev` merged and
pushed. Exit 0 pass, 1 failed, 2 checkout not ready, 130 ctrl-c. Report:
`~/.cache/nimitz-ci/<version>/report.md`.

`bump` rewrites exactly four places, and each regex must match exactly once:

| File | Line |
| --- | --- |
| `docs/compat.toml` | `max = "<v>"` |
| `README.md` | `Status: in testing (Claude Code <min> - <max>)` |
| `crates/nimitz-core/src/compat.rs` | `assert_eq!(c.max, "<v>");` |
| `crates/nimitz-core/src/check.rs` | `snap.cc_version = Some("<v+1>".into());` |

**Keep that README text literal, on one occurrence, with `<max>` equal to compat's.**

### `tools/admiralty_screens.py`

```sh
cargo build -p admiralty
uv run --with pillow tools/admiralty_screens.py [--font DIR] [--only NAME ...]
```

Runs the real TUI in a private tmux server with `--demo`, `--demo-monitor` or a fresh
sandbox at `/tmp/adm-sb`, captures the cells with 24-bit colour, and draws them with a
JetBrainsMono Nerd Font. Needs tmux, and refuses without the font. Each screen's size,
source and keys are in its `SCREENS` table. Screens are generated, never hand-taken.

## 19. Conventions

- **Generic names only.** Example fleets, ships, machines and paths in sources, tests,
  docs and screens are invented. Nothing enforces this, so check your own diff.
- **Commits:** conventional, plain sentence: `fix(provider): ...`, `test(os): ...`,
  `docs(changelog): ...`. No Claude signature or co-author lines.
- **Branches:** work on your own branch; don't push to main.
- **Console output:** `[OK]`, `[INFO]`, `[WARN]`, `[ERROR]`, no emoji.
- **Comments and docs:** concise, plain words.

### Release

There's no release script. Convention from earlier releases:

1. `chore(release): X.Y.Z` touches `Cargo.toml`, `Cargo.lock`, the `Version` in
   `README.md`, `herdr-plugin/herdr-plugin.toml`, `plugin/.claude-plugin/plugin.json`.
2. `docs(changelog): X.Y.Z` adds `## X.Y.Z (unreleased)`.
3. `merge: X.Y.Z, fixes #N: <what>`, with `--no-ff`.
4. `chore(release): X.Y.Z (CHANGELOG date)` sets the date.
5. Annotated tag `vX.Y.Z` on that date commit.

Tested-range bumps are separate, made by cc-validate on `dev`, so `dev` can carry a
newer range than `main`. CHANGELOG entries go newest first, with `### Added`,
`### Changed`, `### Fixed`, and `### Upgrading` last when a release needs one (0.7.18
has none). Bullets open with a bold sentence.

Nothing compares `herdr-plugin.toml` or the README's `Version` with the workspace.

### Recipes

**Add a command.** Add it to `COMMANDS` and `USAGES`, call `allow` with its flags, add
value flags to `VALUE_FLAGS`, add `crud` or `members` code with rules in core. Tests
`every_command_has_its_own_help` and `every_command_runs_for_every_role` will fail
until it's complete.

**Change a file nimitz owns.** Bump `current` in `formats::ARTIFACTS`, add the `Step`
and a fixture, then `NIMITZ_BLESS=1 cargo test --test formats`. See
`docs/design/migrations.md`.

**Change the role rules.** Edit `crates/nimitz-core/rules/`, bump `rules::VERSION`,
add a `CHANGELOG` entry in `rules.rs`, and stay within `CONTEXT_MAX`.

**Change `plugin/`.** Bump `plugin.json`. Keep LF.

**Add a hook event.** Add it to `hooks.json` and `HOOK_EVENTS` in `setup.rs`, and
handle it in `hook.rs`. Probes in `tools/cc-validate` check payload fields.

**Add an Admiralty action.** Add the `Act`, its key and availability, build the plan
in `actions`, add it to `HELP`, and add a `cli_drift` case if it shows a command.

**Touch Claude Code behaviour.** Everything goes through `claude.rs`. Check
`docs/probes/README.md` first, then re-run e2e.

## 20. Known drift and traps

### Stale documents

- `docs/design/contracts.md`:
  - Section 10, the CLI contract, predates 0.7.3. It lists `up`, `relaunch`, `restore`
    and `layout herdr`. Trust `help` and `COMMANDS`.
  - It says `nimitz relaunch` in several places; the command is `restart`.
  - "Mutating commands support `--plan`" is false (section 4 gates).
  - `authority` and `comms.denied` no longer exist. `comms` is parsed and ignored.
  - The sweep authority table no longer applies.
  - Section 9's `Mux` trait is missing methods. `--jobs-only` is gone.
- `docs/design/cli-ux.md`: `signal` isn't inbox only.
- `docs/design/trust-notes.md`: `nimitz trust` isn't user-only since 0.7.3.
- `docs/usage.md`: "never panes" and "panes only" are too strong (section 4), and
  cc-validate's failure and plugin wording is loose (section 18).
- `crates/admiralty/README.md`:
  - "Every plan comes from core" is false for start, restart, open, up and muster.
  - Monitor-only doesn't close starting a lead.
  - `e` also edits squadrons and fleets.
  - `cli_drift` doesn't compare every plan.
- `herdr-plugin/README.md`: names the removed `nimitz restore`; use
  `nimitz muster F/...`. Its `admiralty-watch` migration line has a typo. Its
  checklist uses `ctrl+g f`, while Install binds `alt+f`.
- Skills use `not-started`, `down`, `parked`, while `status --json` gives `NotStarted`
  and `Parked`.

### Stale code

- `sessions.rs`: the `lineage_broken` check is always false, because `lineage.broken`
  never reaches `Member.issues`. The live trigger is row state `failed`.
- `hook.rs` still handles PostToolUse; the plugin registers none.
- `provider.rs`: the `Record` doc names a per-member path that doesn't exist.
- `rules.rs`: "a ship may not run start" isn't enforced.
- The adversarial test headers mention `#[ignore]`; nothing there is ignored.
- `herdr-plugin/tests/e2e_admiralty_in_herdr.py` expects tab `CLEAN`, while the code
  names it `MAIN.ARGOS.CLEAN`. Read from the code, not run.

### Traps

- `snapshot()` and `status` write state.
- `--plan` isn't honoured by `harness push`, `kb commit|restore|sync` or
  `sandbox create|tick`.
- A refused restore plan can have `refusals: []` (section 6).
- `--json` can leave stdout empty (section 6).
- `doctor` is the only command that runs past a too-new file.
- `harness add`, without `--fleet`, is a usage error for a non-member.
- `Issue.fix` isn't always a command.
- `admiralty dump` on live data is the partial look (section 15).
- `/exit` inside an attach opens the agent view. `ctrl-z` detaches.
- Never start, resume or stop a member with `claude` directly: it makes copies.
