# claude-nimitz

A fleet system for Claude Code, built on the `claude` CLI only, so it works with
a Claude subscription login and with Bedrock.

- `nimitz`: the command that builds and runs fleets, squadrons and ships.
- **Admiralty**: the TUI where you watch and control every fleet.
- A Claude Code plugin: the fleet skills and the hooks that report state and
  tell a commodore at once when one of its ships waits on you.

Version 0.7.14 ([CHANGELOG](CHANGELOG.md): read "Upgrading" first). Status: in testing (Claude Code 2.1.288 - 2.1.310).

A fleet has one **admiral**, squadrons each led by a **commodore**, and
**ships** that do the long work in one folder. Every member is a Claude Code
background job; you look at one in a Herdr pane and detach with ctrl-z.

## Install

Needs Rust (cargo) and Claude Code; Herdr for panes (status, config and ships
work without it).

```sh
git clone -b dev https://github.com/avatarye/claude-nimitz && cd claude-nimitz
cargo install --path crates/nimitz       # nimitz: the CLI and the hook binary
cargo install --path crates/admiralty    # admiralty: the TUI
claude plugin marketplace add "$PWD" && claude plugin install nimitz@claude-nimitz
nimitz init --label <this-machine>       # data root and machine label
nimitz doctor                            # run again until: [OK] doctor: ready
```

In Herdr: `herdr plugin link <repo>/herdr-plugin` and bind a key to
`admiralty.toggle` to open Admiralty as an overlay. To try it all without
touching your setup: `nimitz sandbox create /tmp/nz --herdr && eval "$(nimitz sandbox env /tmp/nz)"`.

Upgrading: install both binaries again, then `claude plugin marketplace update
claude-nimitz && claude plugin update nimitz@claude-nimitz` (sessions load a
cached copy of the plugin), then `nimitz doctor`. The first nimitz command
after an update migrates this machine's files by itself; the CHANGELOG's
"Upgrading" says what else a release needs.

## Most used

| When | Run |
| --- | --- |
| After a reboot, a Claude Code update or a Herdr closure | `nimitz muster` |
| Relaunch members (same conversation, a new process) | `nimitz restart F/SQ/SHIP` |
| Talk to a member | `nimitz open F/SQ/SHIP` (its pane, focused; ctrl-z detaches) |
| See everything | `nimitz status`, or `admiralty` |
| Stop members | `nimitz stop F/SQ/...` |

The rest is the leads' work: an admiral or commodore runs fleet management
itself (`--plan`, `--yes`, then a report) and asks you first only for `fleet
rm`, `squadron rm`, `machine rm`, a folder's trust, `migrate --shared`,
`--force`, `--fresh` (or anything else that drops a conversation) and
`settings set kb.*`. Every command that changes something shows its plan
first: `--plan` only looks, `--yes` runs without the question, `-v` shows the
commands. `nimitz help <command>` (or `nimitz <command> --help`) prints one
command's usage.

## Paths

A path names one member, in every verb: `F` the admiral, `F/SQ` the commodore,
`F/SQ/SHIP` a ship. `F/...` and `F/SQ/...` name everything under one, the lead
included; `//...` names every fleet (never ADMIRALTY; `/...` works too, as
MSYS turns `//` into `/`). A command takes several paths; a member matched
twice counts once; a path that matches nothing is refused. No `*`.

## Agents run these (the command reference)

Four families: fleet shape changes config only, session control changes jobs
and never panes, pane control changes panes only, and `muster` does both as
one plan. Design: [docs/design/cli-ux.md](docs/design/cli-ux.md).

### Session control (jobs)

| Command | What it does |
| --- | --- |
| `start <path>... [--fresh] [--parked] [--force]` | Bring members up, the jobs only: what is down is woken by its exact id, top-down; a member a path names, and a lead under a `...`, gets a first launch when it never ran; a worker on an older Claude Code is relaunched. `--fresh`: a named member's new conversation (drops its history); `--parked`: the parked and stopped ships a `...` covers too; `--force`: a cold launch over a broken lineage or an untested Claude Code |
| `restart <path>... [--parked]` | Relaunch the live members (same id and history, ships first) and wake the ones that are down; a turn in flight is dropped, and the plan says so |
| `stop <path\|session>...` | Stop members (under a `...`: the live ones, ships first), or one session by its id |
| `rm <path\|session>...` | Remove job rows, never transcripts |
| `sweep [<path>...]` | Drop stale rows and copies of members |
| `adopt <session> <F\|F/SQ\|F/SQ/SHIP>` | Make a stowaway (a session in a fleet folder with no labels) one member's session (an exact path; a `...` is refused) |
| `signal <path> <text> [--inbox\|--type] [--from <label>]`, `signal session:<short-or-name> <text>` | Yours (a plain terminal): typed into the member's tab, your own input; `--inbox`, a member's signal, or one from a Claude Code session or with `--from`: posted to its inbox, naming the verified sender (a member's id, else the Claude Code session above it, else an unidentified process; a non-member is `...; not a fleet member, unverified`); their `--type`: typed into the tab, led by `[nimitz signal from <who>]`, through the same pane checks, never into one waiting on the user. `--from` (non-members only, a short plain ASCII label) is a claim: `says it is "<label>"`, unverified. `session:`: any live local Claude Code session, member or not, by its inbox, never typed. One path: a member, or `F/...` / `F/SQ/...` for every live member under it but the sender (never `//...`: a turn in every fleet at once); all or nothing, behind one gate (any refusal sends none). A member named exactly that is down is refused, never woken; under a `...` it is skipped |

### Pane control (Herdr)

| Command | What it does |
| --- | --- |
| `open <path>... [--tab] [--force]` | A pane for each member, focused: its pane, its tab, a split beside its commodore, or a new tab. `--tab`: always its own tab (`F.SQ.SHIP`, `F.ADMIRAL`, `F.SQ.COMMODORE`); `--force`: a second tab beside a taken one. One that is down is woken first |
| `attach <F\|F/SQ\|F/SQ/SHIP>` | Attach this terminal (`claude attach <its exact job>`); one that is down is woken first; ctrl-z detaches; never from inside a Claude Code session. One terminal, one member: a `...` is refused |
| `close <path>...` | Close the panes that show those members (only a pane that runs its attach); the jobs run on |
| `layout <path>... [--ships] [--here] [--rename] [--force]` | The leads' tabs (`--ships`: each live ship's too); `--here`: in this Herdr session; `--rename`: give a tab that shows a member under another name its full name |

With no multiplexer, `open`, `close` and `layout` refuse (`mux.none`).

### Muster

`muster [<path>...] [--parked]` (no path: `//...`; `--here`, `--rename`,
`--force` as for `layout`) is the one command after a reboot, a Claude Code
update or a Herdr closure. It tells the three apart, member by member:

- Claude Code updated: a worker on an older version is relaunched (same id
  and history), then reattached. `restart //...` relaunches every one instead.
- Herdr closed or crashed: the jobs are alive; their tabs are made again and
  attached.
- Reboot: the leads and the ships live at the last look are woken by their
  exact session id (`--parked`: the rest too), then attached.

With no multiplexer it runs the job steps and skips the pane steps. Run twice,
the second run does nothing. What a plan looks like:

```text
muster DEMO/...  ·  Claude Code updated to 2.1.289  ·  5 to relaunch  ·  3 panes to reattach

             job       pane       why
DEMO
  ADMIRAL    relaunch  reattach   on 2.1.288 · pane w2:p1
DEMO · OPS
  COMMODORE  relaunch  reattach   on 2.1.288 · pane w3:p1
  BUILD      relaunch  reattach   on 2.1.288 · pane w3:p2
  DOCS       relaunch  ·          on 2.1.288; busy: its turn in flight is dropped
  TEST       relaunch  ·          on 2.1.288

--yes runs these 15 steps; -v shows the commands
```

A command that would stop, relaunch or remove the member that runs it runs
that step last: a detached `nimitz __self-end` waits for the member's turn to
end (up to 10 minutes), runs it, logs to `NIMITZ_STATE/relaunch-self.log` and
hands the outcome to the member's next session start.

### Fleet shape (config only)

| Command | What it does |
| --- | --- |
| `fleet new\|set\|rm <F>` | A fleet (new: `--machine L`; set: `--admiral-mode M`) |
| `squadron new\|set\|rm <F/SQ>` | A squadron (`--brief T --mode M --effort E`: `--mode` and `--effort` are its ships' defaults) |
| `ship new\|set\|rm <F/SQ/SHIP>` | A ship (`--dir PATH\|managed --machine L --prompt T\|--prompt-file P --mode M --effort E --depends A,B`). `--effort low\|medium\|high\|xhigh\|max` is passed as `claude --effort` on every launch and relaunch (a wake keeps it); `''` clears it. A live ship takes a new one at `nimitz restart <id>`. A squadron.toml that sets an effort needs nimitz 0.7.12 on every machine: an older one refuses that squadron (`config.too_new`) and every config change to its fleet, while the fleet's other squadrons still run there; the plan that sets the first effort warns (`format.raise`) |
| `machine add\|rm <F> <label>` | Run a fleet on another machine too (add: `--hostname H`, `--dir F/SQ/SHIP=PATH`) |
| `settings get [<key>]` · `settings set <key> <value>` | The fleet's `settings.json` (`--fleet F`), e.g. `kb.remote`, `kb.dirs.<machine>` |
| `harness list\|add\|rm\|push` | What every member of one fleet gets beyond nimitz's role rules: rules, agents, skills, settings ([docs/design/harness.md](docs/design/harness.md)) |
| `kb path\|status\|head\|show\|grep\|hash\|commit\|restore\|sync\|clone` | The fleet's knowledge base, a git repo every machine clones; members use it through the `nimitz:kb-scribe` and `nimitz:kb-ref` subagents ([docs/design/kb.md](docs/design/kb.md)) |
| `migrate --plan\|--yes [--shared]` | The format steps due here; `--shared` runs the ones that affect every machine: update each one first ([docs/design/migrations.md](docs/design/migrations.md)) |

### Setup and diagnosis

| Command | What it does |
| --- | --- |
| `init [--label L]` | Data root, machine label, Claude Code's trust of the data root, doctor |
| `doctor [--fix]` | The install: hooks, plugin, trust, Claude Code version, formats |
| `status [<path>...] [--json]` | The fleets, their members and issues; a live member's line ends in its topic, the one line Claude Code keeps on what the job is doing (`-- Inspecting ...`, cut to a terminal's width, never in a pipe; `topic` in the JSON). `down`: taken down while active (a `...` start wakes it); `parked`: retired by the daemon or not active (`--parked` wakes it) |
| `check [<path>...]` | The problems of the members the paths cover, their parents and everything under them, each with its one fix (none: everything; `//...`: every fleet but ADMIRALTY). `check F` and `check F/...` are the same: a check never hides a ship's errors |
| `trust <folder>` | Claude Code's trust for the folder's git root (it lets the folder's `.claude` settings, hooks and MCP servers run) |
| `rules [<path>...]` | The role rules nimitz hands each member at session start (none: the caller's own) |
| `sandbox create\|tick\|env <dir>` | A fake world for tests and demos |

Removed in 0.7.3: `restore` (use `muster`), `relaunch` (`restart`), `up`
(`start F/... --parked`), `layout herdr` (`layout`), `fleet settings`
(`settings`), and `start -f`, `-p`, `--tab`, `--only`, `--all`, `--jobs-only`.

## Admiralty

Keys: `j`/`k` move, `tab` next fleet, `space` find a member in every fleet
(fuzzy, as fzf: `mas` finds `MAIN/ARGOS/SIGNAL`, `argos busy` the busy ones of
ARGOS; `enter` puts the cursor on it, `ctrl-o` jumps to its pane; `!` and `i`
the same), `z` fold / unfold, `enter` jump to a pane, `a` start, `A` a
pane for it, `x` more actions with the row's detail below them (stop,
restart, sweep, rm, adopt; on a fleet: `R` muster), `u` up a fleet with its parked ships, `m` message, `!` what waits on you, `i`
every issue with its fix, `c`/`e`/`D` create, edit, remove, `?` all keys (`?`
again closes them), `q` quit (in the `?` panel). The two status rows at the
bottom show the fleets' counts and `? keys`, then `Nimitz: v0.7.5  Claude Code:
v2.1.296 (supported)  Machine: home-server` (a mismatch or an out-of-range
Claude Code as a warning) and the local date and time at the right, after
`stale 12s` when no refresh has landed for three intervals. A member's DETAIL
cell shows what its job is doing (its topic) when nothing else needs saying.
Every row reads the same way: NAME says who (a member, a crew's agent type or
its own name, a task's `#id` and owner), DETAIL what it is doing, STATUS in the
member glyphs (a crew is busy, idle, done or failed). NAME, STATUS and AGE keep
fixed widths; DETAIL takes the rest of a wide or fullscreen terminal.

## Two rules

- Never start, resume or stop a member with `claude` yourself (`--resume`,
  `--bg`, `-c`, `stop`, `rm`): it makes copies and stowaways nimitz cannot
  manage. `nimitz start` does it right.
- Plan, then run: read the plan (`--plan`), then run it (`--yes`). Every
  member may run every command; the leads run routine fleet management
  themselves and wait for your OK only for `fleet rm`, `squadron rm`,
  `machine rm`, a folder's trust, `migrate --shared`, `--force`, `--fresh`
  (or anything else that drops a conversation) and `settings set kb.*`.

## When something is wrong

- `nimitz doctor` for the install, `nimitz check [<path>...]` for a fleet: every
  problem comes with its one fix line.
- Common fixes nimitz prints:
  - a drifted name or missing labels: `nimitz restart <id>`;
  - `history.lost` (its transcript is gone): `nimitz start <id> --fresh`;
  - `folder.untrusted`: `nimitz trust <folder>` (`ship new` and `start` plan it for you);
  - `tab.taken` (its tab runs something else): ctrl-z or quit it there, then `nimitz open <id>`;
  - copies or stale rows: `nimitz sweep <id>`;
  - `session.unreadable` (a start answers `started a copy ... cannot parse
    the session file ~/.claude/sessions/<pid>.json`): a live session's status
    file is broken, and Claude Code then resumes nothing on the machine.
    Remove what follows its first JSON object (the fix names the byte), or
    restart the session that owns it. A dead process's broken file is
    harmless (`nimitz doctor` lists it as info);
  - a stowaway: `nimitz adopt <session> <id>`, or stop and `rm` it;
  - `tab.resumed` (Herdr reopened a conversation in a pane): `/exit` in that pane, then `nimitz start <id>`.
- Never `/exit` inside an attach: it opens the agent view; ctrl-z detaches.
- Doctor says Claude Code is above the tested range: launches still go ahead;
  `tools/cc-validate` validates the new version (see For maintainers).

## For maintainers

- A change to a file nimitz owns: bump its version in `formats::ARTIFACTS`, add
  the step and a fixture, then `NIMITZ_BLESS=1 cargo test --test formats`
  ([docs/design/migrations.md](docs/design/migrations.md)).
- Unit suite: `cargo test --workspace`. Real Claude Code e2e (opt-in, cheap
  model, every test fleet under `~/.cache/nimitz-e2e`):
  `NIMITZ_E2E=1 NIMITZ_E2E_LIVE_FLEETS=HOME,MAIN cargo test -p nimitz --test e2e -- --test-threads=3`.
- After a Claude Code update, `tools/cc-validate` (`--plan` first) validates
  it in one command, in a fresh worktree from dev: build, unit suite, full
  e2e, a hook-payload probe and a diff of the Claude Code internals nimitz
  relies on, with the live fleets checked before and after. On a pass it
  bumps the tested range, commits, merges into dev, **pushes origin dev** and
  installs; on a fail it changes nothing and writes
  `~/.cache/nimitz-ci/<version>/report.md`. Exit 0 pass, 1 failed, 2 checkout
  not ready. Its own tests: `python3 tools/test_cc_validate.py`.

## Documents

| Where | What |
| --- | --- |
| `CHANGELOG.md` | What each release changed; how to upgrade. |
| `docs/design/cli-ux.md` | 0.7.3: paths, the four command families, muster, no authority checks. |
| `docs/design/cc-control-model.md` | How `nimitz` controls Claude Code sessions: jobs, identity, pins, parking. |
| `docs/design/harness.md` | A fleet's harness: rules, agents, skills, settings. |
| `docs/design/kb.md` | The knowledge base and its two subagents. |
| `docs/design/migrations.md` | Format versions of every file nimitz owns, the chained migrations, the cross-machine guard. |
| `docs/design/rules-size.md` | The role rules and how they reach a member. |
| `docs/design/plan-v3.md` | The plan: decisions, vocabulary, setup and CRUD, adapters, compatibility, phases. |
| `docs/design/aide.md` | Aides (0.5 to 0.6; removed in 0.7.0). |
| `docs/probes/` | P0 probes: requirements, findings per Claude Code build. |

Note: "Claude" is an Anthropic trademark. This name is for private use; check
Anthropic's trademark guidelines before any public release.
