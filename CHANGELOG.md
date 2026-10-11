# Changelog

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
  inside a Claude Code session (`attach.nested`).

### Fixed

- A session file left by a crash whose pid another process now has is no
  longer read as the Claude Code session above a command (Linux and Windows:
  its `procStart` must match the process).
- Windows: while `nimitz attach` waits, ctrl-c and ctrl-break go to the
  attach, not to nimitz (which used to exit and leave the attach running).
- The fleet, admiral and commodore skills name `nimitz attach` and
  `start -p --tab`.

### Known limits

- macOS: the stale session file check is not made (nimitz does not read a
  process's start there), so a file left by a crash at a reused pid can still
  refuse `nimitz attach` (`attach.nested`); a plain terminal, or deleting that
  `~/.claude/sessions/<pid>.json`, gets past it.

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
