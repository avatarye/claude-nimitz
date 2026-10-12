# Changelog

## 0.7.4 (unreleased)

### Changed

- **Paths everywhere a set makes sense.** `check` and `rules` take several
  paths (`check MAIN/...`, `rules HOME/GITHUB/...`; `rules` heads each member
  with `== ID ==`). `check F` still covers everything under F, so an exact
  lead never reads clean while its ships have errors. `signal F/...` (or
  `F/SQ/...`) sends to every live member under it, one plan; down members are
  listed and skipped; `//...` is refused (a turn in every fleet at once).
- Commands that act on one member (`attach`, `adopt`'s target, `fleet`,
  `squadron` and `ship` new|set|rm, `machine`, `harness --for`, `--fleet`)
  refuse a pattern, saying why and naming the exact form.
- Admiralty's screenshots (`docs/screens/`) show the 0.7.3 layout, made by
  `tools/admiralty_screens.py` (see crates/admiralty/README.md); the design
  page has a "UI as shipped" section.

### Fixed

- A start refused for an untrusted data root names that folder, once, with
  `nimitz trust <root>` as the fix (it named a commodore's folder, twice).
- The `start` usage wraps like its neighbours.

### Upgrading

- Install both crates and update the plugin (`claude plugin marketplace
  update claude-nimitz && claude plugin update nimitz@claude-nimitz`, then
  `/reload-plugins`). No format or rules change.

## 0.7.3 (2026-10-12)

The commands, cleaned up: Perforce paths, four families, one `muster`, no
authority checks. No old form is kept. Read **Upgrading** first.

### Changed

- **A path names one member.** `HOME` is the admiral, `HOME/GITHUB` the
  commodore, `HOME/GITHUB/SHIP` a ship. `HOME/...` is everything under HOME,
  the admiral included; `//...` (or `/...`) is every fleet, the ADMIRALTY
  workspace never. A command takes several paths. `...` leaves parked ships
  alone; `--parked` adds them. A member a path names, and a lead under a
  `...`, gets a first launch when it never ran. `--only` and `--all` are
  gone.
- **Four families.**
  - Fleet shape (configuration, nothing launched): `fleet`, `squadron`,
    `ship` new|set|rm; `machine add|rm`; `settings get|set` (was `fleet
    settings`); `harness`.
  - Session control (jobs, never panes): `start [--fresh] [--parked]`,
    `restart` (was `start -f`), `stop`, `rm`, `sweep`, `adopt`, `signal`.
  - Pane control: `open [--tab]` (was `start -p`; wakes the member first),
    `attach`, `close` (closes the member's pane; the member keeps running),
    `layout` (the leads' tabs; `--here`, `--rename`, `--force`, `--ships`).
  - Misc: `status [<path>...]`, `check`, `rules`, `doctor`, `init`, `trust`,
    `migrate`, `kb`, `hook`, `sandbox`.
- **`nimitz muster [<path>...]`**, the one command after a reboot, a Claude
  Code update or a Herdr closure (default `//...`): relaunches workers on an
  older Claude Code, wakes the leads and the ships that were active, and
  puts every interactive member back in its Herdr pane (or its lead's tab).
  One plan.
- **A busy member is relaunched.** The plan says "busy: its turn in flight is
  dropped" (or "waiting: its question is dropped"); `relaunch.busy` and the
  needs-you skip are gone.
- **A member that relaunches, stops or removes itself** does it last: its
  own step runs in a detached `nimitz __self-end` after the command's report,
  once its turn ends (10 minutes at most), and the outcome comes at its next
  session start. `start.self` and `restore.self` are gone.
- **No authority checks.** Every member may run every command on any member
  or fleet; nothing is "the user's alone". The member's identity stays (its
  own self-end, its kb sender id, its default fleet); partial labels warn.
  kb-guard is unchanged.
- **Any member may message any member.** The SendMessage chain-of-command
  hook and the squadron `comms` setting are gone (`comms` in an existing
  squadron.toml is read and ignored). The user's `signal` still types into a
  lead's tab (`--inbox` for the inbox); a member's goes to the inbox, naming
  its sender.
- **Rules v10**: no "may run" lists or signal tables; roles describe the work.
- **doctor** warns when Claude Code's cached copy of the plugin
  (`~/.claude/plugins/cache/claude-nimitz/nimitz/<version>/`) is not this
  binary's version, and its fix for a missing hook is the plugin update.

- **Admiralty** shows the nimitz version in its status bar (both, flagged,
  when admiralty's and the binary's differ); `m` types into the member's
  own pane, a ship's too; `x` then `R` runs `muster`, and `u` runs `start
  F/... --parked`.
- **README**: a "Most used" section and one reference line per command.

### Fixed

- Fix lines that said `nimitz status <id>` (status took no id; it now takes
  paths).
- `nimitz <cmd> --help` and `nimitz help <cmd>` print the command's usage.

### Removed

- `restore`, `relaunch`, `up`, `layout herdr`, the old `open` alias,
  `start -p/-f/--tab/--only/--all/--jobs-only`, `fleet settings`,
  `squadron --comms`.

### Upgrading

- Install both crates, then update the plugin: `claude plugin marketplace
  update claude-nimitz && claude plugin update nimitz@claude-nimitz`, then
  `/reload-plugins` in each live session (sessions load Claude Code's cached
  copy, not the repo). `nimitz doctor` says when they differ.
- Old forms and what replaces them:

  | Before | 0.7.3 |
  | --- | --- |
  | `nimitz start F` (the whole fleet) | `nimitz start F/...` |
  | `nimitz start F --all` | `nimitz start F/... --parked` |
  | `nimitz start F --only` | `nimitz start F` |
  | `nimitz start <id> -f` | `nimitz restart <id>` |
  | `nimitz start <id> -p` | `nimitz open <id>` |
  | `nimitz start <id> -p --tab` | `nimitz open <id> --tab` |
  | `nimitz restore`, `nimitz up F` | `nimitz muster`, `nimitz start F/... --parked` |
  | `nimitz relaunch <id>` | `nimitz restart <id>` |
  | `nimitz layout herdr` | `nimitz layout //...` |
  | `nimitz fleet settings get/set` | `nimitz settings get/set` |
  | `nimitz stop` of a squadron with its ships | `nimitz stop F/SQ/...` |

- Live members get rules v10 at their next prompt, and a fleet's harness
  rules once more as "changed" (their opening and closing lines are new).

## 0.7.2 (2026-10-12)

### Changed

- **A fleet's own rules (its harness) may hold 9,500 characters, up from
  4,000.** They now come to every member in a hook context of their own
  (`nimitz hook rules harness`, a new entry in the plugin's hooks.json),
  since Claude Code gives each hook entry 10,000 characters whole and only a
  2,000-character preview above. At every session start, and at the next
  prompt when they change, once; when they are taken away, one line says so.
  `harness list` and `harness add` warn above 6,000: every member carries
  them every turn.
- **Rules v9: shorter, with the same behaviour** (about a quarter
  shorter for each role). `nimitz start` is in the leads' rules only (a ship
  may not run it); the deprecated verbs and the history are gone. A rules
  update lists the last three changes.
- docs/design/rules-size.md: what Claude Code does with a hook's context
  (per hook, not per event; a preview and a file above 10,000).

### Upgrading

- Install over 0.7.1 (`cargo install` both crates) and update the plugin
  together: the new hooks.json entry needs the 0.7.2 binary (0.7.1 gives it
  no answer and logs a line for each call). Live members get rules v9 at
  their next prompt, and a fleet's harness rules once more as "changed" (their
  heading now says they add to the role rules, which moves their hash).

## 0.7.1 (2026-10-12)

### Fixed

- **The kb-scribe and kb-ref reports never reached their caller in auto
  mode.** There Claude Code has a subagent report through its
  `SubagentHandback` tool, and the kb-guard denied every tool outside each
  agent's own, that one too: the note was committed, the question answered,
  and neither result arrived. The guard now gives no answer for
  `SubagentHandback`, which delivers a subagent's report, and for two
  harmless tools of Claude Code's own, `TodoWrite` and `ToolSearch`: Claude
  Code decides as for any session. Every other tool outside the agents' own
  is still denied, the task tools too. e2e #98 runs its ship in auto mode
  and checks that both reports arrive.

### Upgrading

- Install over 0.7.0 (`cargo install` both crates, then the plugin); no
  file changes format, the rules stay v8.

## 0.7.0 (2026-10-12)

Aides are gone. Read **Upgrading** first.

### Upgrading

- **Stop every aide with 0.6 before you install 0.7.0** (`nimitz stop
  F/@NAME`). An aide job still running afterwards (pinned, so the daemon
  never retires it) is `aide.retired` in `nimitz check`, with its fix:
  `nimitz stop <short>`, then `nimitz rm <short>` (yours).
- On each machine, the first 0.7 command archives the aides' state (their
  queues, snapshots, doorbell files and needs-you notices) under
  `NIMITZ_STATE/archive/aides-0.6/`, nothing deleted, and drops the aides'
  records from the registry (the registry stays version 3: no downgrade
  barrier). A snapshot reader's folder was a git worktree of the KB: its repo
  lists it as prunable afterwards (`git worktree prune` there).
- Once, from any machine: `nimitz migrate --plan`, then `nimitz migrate
  --shared --yes`. It drops each fleet's `[[aide]]` tables and the
  `format = 2` / `min_nimitz` stamp they needed, so every nimitz (0.4 too)
  reads fleet.toml again; the rest of the file, comments included, stays as
  it was, with a backup beside it. Until then a fleet with `[[aide]]` still
  loads, and `check` warns (`aide.legacy`).
- Rules move to **v7** (live members get the change at their next prompt).

### New

- **A fleet's settings**: `<NIMITZ_HOME>/<fleet>/settings.json`, its own
  format beside fleet.toml (no lockstep). `nimitz fleet settings get [<key>]`
  (every member, its own fleet) and `set <key> <value>` (the admiral, you;
  planned). `kb.remote` and `kb.dirs.<machine>` name the fleet's knowledge base.
- **`nimitz kb path|status|head|show|grep|hash|commit|restore|sync|clone`**:
  the knowledge base's git, run by nimitz. Reads at one commit; `commit` takes
  exactly the named paths, once per note (`note:` and `from:` trailers),
  under a lock, then syncs (pull --rebase, push); `clone` is yours.
  docs/design/kb.md.
- **The `kb-scribe` and `kb-ref` subagents** ship in the plugin, for every
  fleet member (`nimitz:kb-scribe`: a note as JSON, one commit each, on
  Sonnet; `nimitz:kb-ref`: a question, a cited answer read at one commit),
  with their procedures as the skills `kb-ingest` and `kb-query`. The
  **kb-guard** (`nimitz hook kb-guard`, a plugin PreToolUse hook on every
  tool) holds them to `nimitz kb` and the KB's folder by the call's
  `agent_type`, in any permission mode, and allows what they may do so a
  background member never stops on a prompt. Rules v7 tell members how to use
  them. `nimitz doctor` checks the guard's entry.

- **A fleet's harness** (`<NIMITZ_HOME>/<fleet>/harness/`): rules after the
  role rules (live: every member gets a change at its next prompt), agents and
  skills (a plugin nimitz makes on each machine, `fleet-<fleet>`, given to
  every launch with `--plugin-dir`, which a wake keeps), and settings merged
  fleet, squadron, member under nimitz's own `--settings` (never hooks or
  other programs, never nimitz's env). `nimitz harness list|add|rm|push`
  (the admiral and you change it; every member lists its own).
  docs/design/harness.md. `rules-delivered` moves to format 2 (by itself:
  nothing to rewrite).

### Removed

- The aide role (`F/@NAME`), `nimitz aide new|set|rm|grep|status`, `nimitz
  inbox ...`, `nimitz ask`, the doorbell and the piggyback (`__ring`), the
  snapshot reader, the aide checks (`aide.*`, `inbox.*`), and Admiralty's
  aide rows and queue panel.

## 0.6.0 (2026-10-11)

The aide system, complete: `@KB` keeps notes in a git knowledge base, `@REF`
answers from it read-only. Read **Upgrading** first.

### Upgrading

- From 0.5.x nothing migrates (no file format changed). Rules move to **v6**
  (live members get the change at their next prompt). Update the plugin
  (`claude plugin update nimitz@claude-nimitz`).
- **Before a fleet gets its first aide, update every machine of the fleet to
  0.6.0** and run any nimitz command there: a nimitz before 0.5 cannot load a
  fleet.toml that holds `[[aide]]`, and 0.5 refuses a read-only reader
  (`snapshot = true`). `aide new` refuses a fleet's first aide until each
  machine of `[machines]` has run 0.5, and its first reader until each has run
  0.6 (`aide.machines`: the refusal names them; `nimitz doctor` names a machine
  below 0.6 once the fleet has a reader); a fleet with a reader is
  stamped `min_nimitz = "0.6.0"`, so an older nimitz says "update nimitz here".
- To give HOME its two aides (once every machine is on 0.6.0):
  `nimitz aide new HOME/@KB --dir /mnt/data/shared/nimitz-knowledge-base
  --types wiki.note.v1 --plan`, then `--yes`; `nimitz aide new HOME/@REF
  --dir /mnt/data/shared/nimitz-knowledge-base --types wiki.query.v1
  --snapshot --mode default --plan`, then `--yes`; then `nimitz start HOME`.
  docs/test/aide-checklist.md walks a throwaway fleet through it first.

### New

- **`nimitz aide new|set|rm F/@NAME`**: an aide is added, changed and
  removed like a ship (plan, `--yes`; you or the fleet's admiral), with its
  folder's trust planned, its model checked against `availableModels`, the
  modes that skip its rules refused and its inbox set by flags. The first aide
  stamps fleet.toml format 2 and waits until every machine of the fleet has
  run nimitz >= 0.5 (`aide.machines`); removing the last one makes the fleet
  readable by 0.4 again. `aide rm` stops and unpins a live aide, and refuses
  while its inbox holds work.
- **The aide's loop (S3)**: a post or an ask rings the aide at once through
  its own messaging socket (`[nimitz inbox]`, coalesced: a burst rings once
  until it drains); its session start (a launch, a relaunch, a `/clear`, a
  compact) shows its inbox and asks for its 15-minute cron; the piggyback
  (`nimitz __ring`, at most every 30 s from any member's hook, `status` or
  `check`) rings an idle aide whose work has waited two minutes, returns a
  lapsed lease to the queue or dead-letters it (its admiral hears), and pushes
  an answer whose `ask --wait` gave up. `check` names `inbox.dead`,
  `inbox.stuck` and `inbox.full`. Rules v6. A lease is enforced by the
  piggyback too (every 30 s on a busy machine): an aide that works an item
  past its lease without `inbox heartbeat` loses it, and on its last try the
  item is a dead letter.
- **The KB's aide (S4)**: an aide runs no git (every git command denied);
  `nimitz aide grep <term> [<path>]` and `nimitz aide status` read its folder,
  and `nimitz inbox done <aide> <item> --commit --title --paths --pages`
  commits exactly the named paths with the item's `inbox-item: <ref>` trailer,
  once (a retry finds the commit; `claim` says so), paths taken literally,
  file by file; `inbox fail --restore <paths>` puts back what a failed pass
  wrote. Edits are denied on the repo's own rules, its brief, and at any depth
  on `.git`, `.gitattributes`, `.gitmodules`, `.gitignore` and CLAUDE.md; a
  changed attributes file stops nimitz's git until the user looks. `[[aide]] brief` follows the rules at each session start.
  `inbox list` shows each item's ref. Tested on real Claude Code with a copy of
  the KB: one commit per note through a kill, a `/clear` and a relaunch; a
  hostile note refused.
- **The reference desk (S4b)**: `aide new F/@REF --snapshot` makes a
  read-only reader that works in a snapshot of its repo (a detached worktree
  under NIMITZ_STATE, trusted on its own), moved to the repo's last commit at
  each claim; it edits nothing and runs no git; its answers name the commit
  they read, and nimitz posts each gap of an answer to the notes keeper (one
  note per gap text). `aide set --snapshot` makes a lost snapshot again;
  `aide rm` unregisters it. A post or an ask on a machine that does not run
  the aide is refused (`inbox.elsewhere`). Tested on real Claude Code with
  both aides on a copy of the KB.
- `machine rm` warns of an aide that runs on the machine removed
  (`machine.aide`); `check` flags an aide with folders on more than one
  machine (one machine runs an aide; a hand-edited fleet.toml could say two).

### Known limits

- Aides on Windows were never run end to end (the `//c/...` permission
  rules, `NUL`, the named-pipe doorbell are built and checked by `cargo
  check`, not used).
- No relay between machines: an aide's inbox is on the machine that runs it,
  and a post elsewhere is refused (`inbox.elsewhere`).
- A reader's snapshot is a worktree of the repo, so each claim's checkout
  touches that worktree's files under the repo's `.git` (in Dropbox: a little
  sync traffic).
- The 15-minute cron, a reboot, the Admiralty queue panel and the other items
  of docs/test/aide-checklist.md are checked by hand.

## 0.5.1 (2026-10-11)

### Upgrading

- From 0.5.0: nothing migrates (no file format changed); update the plugin
  (`claude plugin update nimitz@claude-nimitz`) for the skills' new lines.
  From 0.4: read 0.5.0's **Upgrading** below first.

### New

- **`nimitz start <id> -p --tab`**: the member always in its own tab
  (`F.SQ.SHIP` in workspace `F.SQ`, a lead's own tab), reused or created, then
  attached and focused; a pane that shows it elsewhere is left alone
  (`tab.elsewhere`), its 0.3.0 short-label tab is renamed. Plain `-p` keeps
  its order. Yours alone.
- **`nimitz attach <id>`**: this terminal attached to the member's exact job
  (`claude attach <short id>`, never a name), woken first by start's plan when
  it is down. ctrl-z detaches, ← opens the agent view. Yours alone; refused
  inside a Claude Code session (`attach.nested`). On Windows, ctrl-c and
  ctrl-break go to the attach while nimitz waits for it.
- The fleet, admiral and commodore skills name `nimitz attach` and
  `start -p --tab`.

### Fixed

- A session file left by a crash whose pid another process now has is no
  longer read as the Claude Code session above a command (Linux and Windows:
  its `procStart` must match the process): an inbox post's sender is no
  longer named after that dead session.

### Known limits

- macOS: the stale session file check is not made (nimitz does not read a
  process's start there), so a file left by a crash at a reused pid can still
  refuse `nimitz attach` (`attach.nested`) and name an inbox post's sender
  after it, as in 0.5.0; deleting that `~/.claude/sessions/<pid>.json` gets
  past it.

## 0.5.0

Read **Upgrading** first: 0.5.0 migrates this machine's files on its first
run, and a fleet that gets an aide needs every machine on 0.5.0.

### Upgrading

- **Update every machine that shares the fleet folder before you add an
  aide, and never go back below 0.5.0 after one ran.** A nimitz before 0.5.0
  cannot load a fleet.toml that holds `[[aide]]` (its parser rejects unknown
  fields), and reads a registry that holds an aide as corrupt (it moves it
  aside and rebuilds it). A 0.4.x on a migrated (v3) registry without aides
  reads it, but drops `min_nimitz` when it writes it back (a 0.4.0 built
  before 5f4cc2a also drops the lineage `home` stamps).
- **The first nimitz command migrates this machine's files** (never the
  hook): one line, `[OK] nimitz 0.5.0: migrated ...`, a backup of each file
  as `<file>.pre-0.5.0`, a log in `NIMITZ_STATE/migrations.log`. `nimitz
  migrate --plan` lists what is due; a step that affects every machine (a
  shared fleet file) runs only with `nimitz migrate --shared --yes`.
- **After installing, relaunch each live member once:** `nimitz start <F> -f`
  (a lead: `nimitz start <id> --only -f`), `--plan` first. A job launched
  before 0.5.0 keeps the daemon's inherited `HERDR_PANE_ID` until a relaunch
  gives it the new settings (a plain wake reuses its old ones); until then a
  pane may show the wrong member or none.
- **Herdr:** the Admiralty plugin's commands are now `admiralty herdr
  toggle|watch` (the old `bin/toggle.sh` and `bin/watch.sh` stay as shims for
  a Herdr with the old manifest cached). `herdr plugin link
  <repo>/herdr-plugin` again, or restart Herdr, to use the new manifest.
- Rules move to **v5** (live members get the change at their next prompt).

### New

- **`nimitz start`**: one verb brings any member up and ready: a ship, a
  squadron, a fleet, or every fleet here; wakes what is down by its exact
  id, relaunches workers on an older Claude Code, attaches the leads' tabs.
  `-f` relaunches the live ones too, `-p` gives a member a pane, `--only` one
  lead without its fleet, `--fresh` a new conversation, `--all` parked ships
  too. `restore`, `relaunch`, `open`, `up` and `layout herdr` are deprecated
  (each prints its `start`).
- **The plan view (layout B)** for every planning command: a headline, one
  row per member (job, pane, why), what needs you with its fix, `-v` for the
  commands.
- **Aides**: an always-on member attached to the admiral (`F/@NAME`,
  `[F]-@NAME`), with a **durable inbox** nimitz owns: FIFO, leases, dead
  letters, backpressure (types, size, depth, a per-sender rate), and
  **request and reply** (the answer goes to the one who asked, and only to
  them). `nimitz inbox post|claim|heartbeat|done|fail|list|status|retry|drop|result`,
  `nimitz ask <aide> "<question>" [--wait]`. `[[aide]]` in fleet.toml;
  `start`/`stop`/`check`/`status` handle aides; Admiralty shows an aide's row
  with its queue and a queue panel (retry and drop of a dead letter);
  needs-you notices for an aide's prompts and its dead letters go to the
  admiral. Design: docs/design/aide.md.
- **`nimitz trust <folder>`**: Claude Code's trust for a folder's git root,
  written under Claude Code's own lock, with a backup; `ship new`, `ship set
  --dir`, a named first `start`, `init` and `doctor --fix` plan it. The
  user's alone; never the home folder or a parent of it.
- **Windows**: nimitz and Admiralty build and run natively (an `os` module,
  `File::lock`, named-pipe inbox, the Herdr plugin through `admiralty herdr`).
- **Format versions and chained migrations** (docs/design/migrations.md):
  every file nimitz owns has a format version; a new nimitz migrates this
  machine by itself and refuses a file written for a newer one ("update
  nimitz to >= X"); a test fails when a format changes without its step.
- **`tools/cc-validate`**: one command validates a new Claude Code (build,
  unit suite, the real-CC e2e, hook payloads, pinned formats, an internals
  diff) and, on a pass, raises the tested range.

### Fixed

- A Herdr pane claim that what runs in the pane does not back is ignored,
  and the pane shows what does run there (the HOME admiral's pane went
  missing); an attach-only claim is typed into only after the exact-attach
  check; members' launches no longer inherit the daemon's Herdr pane.
- Tested up to Claude Code 2.1.296.

### Decisions pending the user's word

- An aide never pushes its repo (git only as `git -C <its folder>` with the
  subcommands an ingest needs; push, remote, fetch, pull, clone, config,
  submodule denied), and never runs in `bypassPermissions`, `dontAsk` or
  `auto` (default `acceptEdits`).
- A read-only snapshot aide (`snapshot = true`) is refused until it exists.

### Not in 0.5.0

- Aides: the doorbell (S3: an aide wakes on its 15-minute cron, a message or
  a start), `@KB` and `@REF` live (S4, S4b: the knowledge-base ingest and the
  read-only reference desk), aides of other parents and `aide new|set|rm`
  (S5). Edit `[[aide]]` by hand.

### Known limits

- Windows: never run on a Windows machine end to end: `start`, `open` and
  restore tabs; whether Herdr reports a pane's shell; `procStart`; an npm
  install's version (attach panes are not typed into when it is unknown);
  paths of 260+ characters; the e2e is not ported.
- A real-Herdr restore is checked by hand (docs/test/restore-checklist.md).
