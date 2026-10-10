# Format versions and chained migrations

Status: built for 0.5.0; section 8 lists where the build differs (the user,
2026-10-10: "add and maintain an auto chained migration function, starting
from 0.4 -> 0.5").

## 1. The rule

Every file nimitz owns and reads back has a **format version**. A change to
what a file holds bumps its version and registers one **step** from the old
version to the new. A test fails when a version moves without its step and
its fixture, or when a type's shape moves without a version.

## 2. The artifacts (`nimitz_core::formats::ARTIFACTS`)

| Artifact | Where | Scope | Version kept in | 0.4.0 | 0.5.0 |
| --- | --- | --- | --- | --- | --- |
| `fleet` | `H/<fleet>/fleet.toml` | shared | `format = N` (absent = 1) | 1 | 2 when it holds `[[aide]]` (else 1) |
| `squadron` | `H/<fleet>/squadrons/<sq>/squadron.toml` | shared | `format = N` (absent = 1) | 1 | 1 |
| `legacy` | `H/<fleet>/legacy.toml` | shared | `format = N` (absent = 1) | 1 | 1 |
| `brief` | lead `CLAUDE.md` | shared | the rules version marker | (text) | (unchanged mechanism: `rules::briefs`) |
| `ship-lock` | `.nimitz-ship.lock` | shared | `format` key (absent = 1) | 1 | 1 |
| `registry` | `S/registry.json` | local | `version` | 2 | 3 (role `aide`, unknown records kept) |
| `active` | `S/active.json` | local | `version` | 1 | 1 (now checked) |
| `rules-delivered` | `S/rules-delivered.json` | local | manifest | 1 | 1 |
| `events` | `S/events.jsonl` | local | manifest | 1 | 1 |
| `restore` | `S/restore.json` | local | manifest | 1 | 1 |
| `pending` | `S/pending/*.json` | local | manifest | 1 | 1 |
| `inbox-records` | `S/inbox/*.json` | local | manifest | 1 | 1 |
| `needs` | `S/needs/*.json` | local | manifest | 1 | 2 (`item`, dead-letter keys) |
| `own-pins` | `S/pins.json` | local | manifest | 1 | 1 |
| `queues` | `S/queues/<aide>/` | local | `queue.json` `format` | (none) | 1 |
| `admiralty-ui` | `S/admiralty-ui.json` | local | `format` key (absent = 1) | 1 | 1 |
| CC-owned | `~/.claude.json` trust, `C/jobs/pins.json`, settings | Claude Code's | pinned formats (doctor) | n/a | n/a |

The **manifest** is `S/formats.json`: `{nimitz: "<version that last ran
here>", artifacts: {<name>: <version>}}`, for local artifacts that carry no
version of their own (many small files, or a format nimitz shares with tests).
A missing manifest entry is version 1.

## 3. Steps

`formats::Step { artifact, from, to, scope, summary, apply: fn(&[u8]) -> Result<Vec<u8>> }`
(or, for a folder artifact, `fn(&Path) -> Result<()>` over its files). A step
is **pure** (bytes in, bytes out; no clock, no env), **idempotent** (a file
already at `to` is left alone: the runner checks the version first), and
tested on a fixture of its `from` format (`crates/nimitz-core/tests/fixtures/formats/<artifact>/v<N>.*`,
from a real 0.4.0 install for N = the 0.4.0 version).

0.4 -> 0.5 steps:

- `registry` 2 -> 3: `version: 3` and `min_nimitz: "0.5.0"`; the records as
  they are (0.5 reads `aide` and keeps unknown roles; the step changes no
  record). A 0.4.x binary cannot read a registry holding an aide either way
  (section 5).
- `needs` 1 -> 2: nothing to rewrite (the new `item` field is optional); the
  manifest records 2.
- `queues`: born at 1 (no step).
- `fleet` 1 -> 2: **shared**, never automatic: `format = 2` and
  `min_nimitz = "0.5.0"`, needed only when a fleet holds `[[aide]]`
  (section 4).

## 4. Automatic, local; explicit, shared

- **Local (automatic).** Every nimitz run except `hook` and `__notify` first
  compares `S/formats.json`'s `nimitz` with its own version: equal, nothing
  more (one small read). Otherwise it runs the pending **local** steps: for
  each file, a backup `<file>.pre-<this version>` (once), the step, an atomic
  write, a line in `S/migrations.log` (JSONL: time, artifact, file, from, to,
  ok or error). Then the manifest. One line on stderr:
  `[OK] nimitz 0.5.0: migrated registry 2->3, needs 1->2 (backups *.pre-0.5.0; nimitz migrate --plan)`.
  A crash mid-chain resumes: each step stamps its version as it writes, so
  the next run starts where it stopped; the manifest's `nimitz` is set only
  when every local step is done.
  The hook never migrates (it is on Claude Code's hot path), and it reads
  older formats as it does today.
- **Shared (explicit).** A shared step affects every machine that reads the
  data root (Dropbox). It never runs by itself. `nimitz migrate --plan` lists
  it marked "affects every machine: each one needs nimitz >= X first";
  `nimitz migrate --shared --yes` runs it (the user's alone). `doctor` and
  `check` say when one is due (`format.shared` Warn).
- `nimitz migrate --plan` / `--yes`: every pending step, local and shared;
  `--yes` runs the local ones (and the shared ones with `--shared`).
  `migrate fleet-skill` stays, as `nimitz migrate fleet-skill` (an import,
  one-off), listed under the same command.
- `doctor`: pending local steps (Info: "the next nimitz run applies them"),
  a failed step (Error, the log line, the backup to restore), pending
  shared steps (Warn).

## 5. The cross-machine guard

- A shared file (fleet.toml, squadron.toml, legacy.toml, the ship lock) may
  carry `format` and `min_nimitz`. nimitz >= 0.5.0 reads them **before** the
  strict parse (a lenient read of those two keys): a `min_nimitz` above its
  own version is refused with `config.too_new`:
  "<file> needs nimitz >= X (this machine runs Y): update nimitz here". It is
  never parsed further and never rewritten.
- A local file with a `version` above what this nimitz knows is refused the
  same way (`registry`, `active`); never rewritten, never "recovered" as
  corrupt.
- nimitz writes a shared file at the lowest format that holds its content:
  a fleet with no aides stays format 1 (0.4.x machines can still read it);
  adding `[[aide]]` makes it format 2, and `config::validate` asks for the
  stamp (`format.shared`: `nimitz migrate --shared --yes`).
- **Machines in the fleet.** Each nimitz >= 0.5.0 records itself in the shared
  root: `H/<fleet>/machines/<label>.json` `{nimitz: "0.5.0", seen: <time>}`
  (written only when it changes). `doctor` warns when the fleet's format
  needs a newer nimitz than a listed machine last reported, or when a machine
  of `fleet.toml [machines]` has no record (it runs 0.4.x or has not run
  0.5): "machine X: no nimitz >= 0.5.0 seen; update it before adding an aide".
- **0.4.x machines** predate the guard: 0.4.x refuses a fleet.toml that holds
  `[[aide]]` (its parser rejects unknown fields: the fleet stops loading
  there), and reads a registry that holds an aide as corrupt (it renames it
  and rebuilds it from its backup and the events, losing lineage). So:
  **update every machine to 0.5.0 before adding an aide, and never downgrade
  after one ran** (the README, the CHANGELOG, and doctor's machine warning).

## 6. Maintained

- `formats::ARTIFACTS` holds each artifact's current version and its steps.
- Test `every_format_change_has_a_step_and_a_fixture`: for every artifact and
  every version N below current there is a step N -> N+1 and a fixture
  `vN`; the chain from every fixture reaches the current version and the
  result parses with the current types.
- Test `formats_shapes_are_pinned`: a populated example of each owned type
  (`FleetConfig`, `SquadronConfig`, `Registry`, `ActiveSet`, `Notice`,
  `Item`, `Delivery`, `Saved`, ...) serializes to a golden file named for its
  artifact and current version. A field added or renamed changes the golden,
  and the failure says: bump the artifact's version, add the step, add the
  fixture, regenerate the golden (`NIMITZ_BLESS=1`).
- contracts.md and the README's maintainer section say the same in five
  lines.

## 7. Tests

- Unit: each step on its fixture; idempotence (a step on its own output);
  the runner (backup once, atomic write, log, resume after a crash between
  two steps, nothing to do on the second run); the guard (`min_nimitz` above,
  equal, absent; a registry `version` above).
- A migrate-then-run test: the sandbox state copied from a real 0.4.0 install
  (`tests/fixtures/formats/install-0.4.0/`), then `status`, `check`, `start
  --plan` on the new binary.
- Real-CC e2e (`NIMITZ_E2E_OLD_BIN` = a 0.4.0 build): the old binary creates
  a fleet and a ship and launches it; the new binary migrates (the summary
  line), then `status` shows the ship live with the same session and lineage,
  and `start -f` relaunches it.

## 8. As built (0.5.0, branch feat/migrations)

Where the build differs from, or adds to, the sections above:

- **Version.** The workspace is 0.5.0 (`formats::NIMITZ`): the steps stamp
  `min_nimitz = "0.5.0"`, which a 0.4.0-numbered build would refuse itself.
- **The registry's v1.** `ARTIFACTS` gives each artifact an `oldest` version
  the pure chain starts from; the registry's is 2. A 0.3.x (v1) file is
  brought to 2 by its `lift` (`registry::lift_v1`: the load that has always
  read it, saved; `registry.json.v1` kept), then 2 -> 3 runs. A load never
  moves a file past the shape it read: `migrate::registry_v2` stamps 2, and
  a writer keeps the version it read (a v2 registry the hook writes stays v2
  for the runner, with its backup). A fresh registry is 3 with `min_nimitz`.
- **Which commands migrate.** All but `hook`, `__notify`, `sandbox`,
  `migrate` (it lists and runs the steps itself, so `--plan` still sees the
  local ones) and the hidden `__*` helpers. A sandbox is made at this
  nimitz's formats (its manifest and machine record are written by `create`).
- **A newer nimitz ran here.** The runner never lowers the manifest's
  `nimitz`, so an older nimitz checks on every run; a local file too new for
  it (`registry`, `active`, `queues`, `admiralty-ui`, or a manifest entry
  above what it knows) stops every command but `doctor` with
  `[ERROR] <file> needs nimitz >= X (this machine runs Y): update nimitz here`.
- **The guard** also refuses a `format` above the artifact's current one
  when there is no `min_nimitz` ("is format N, newer than this nimitz reads").
  It covers fleet.toml and squadron.toml (`config.too_new` on the fleet's
  admiral or the squadron's commodore: only that fleet or squadron is
  unreadable here), legacy.toml (its map is not read; `config.too_new` in
  `validate`), the registry (`load_checked`, `save`, `recover_locked`: never
  moved aside; `health` no longer calls it corrupt) and active.json (not
  read, not written over). The ship lock keeps its lenient read (no guard).
- **Codes.** `config.too_new` (Error), `format.shared` (Warn, from
  `config::validate`; fix `nimitz migrate --shared --yes`), `format.failed`
  (Error: a step the log says failed and nothing did since; its log line and
  backup), `format.machine` (Warn, doctor only). `check` gets
  `formats::issues` (local files too new, failed steps).
- **Machine records** are written by every migrating command (a lenient read
  of each fleet.toml's `[machines]`; a write only when the version changed).
  The warning's version is the fleet's `min_nimitz`, or 0.5.0 (the first
  nimitz that records itself); its tail is "update it before adding an aide",
  or, for a fleet that holds one, "update it: it cannot load this fleet's
  [[aide]]".
- **migrate.** `nimitz migrate --plan` lists every step, `--json` gives
  `{nimitz, due, too_new}`; with neither `--plan` nor `--yes` it asks for
  `--yes`. Members are refused before anything runs (both forms).
- **Fixtures and shapes.** `fleet/v1.toml` (the 0.4.0 install's) and
  `fleet/v1.aide.toml` (a dev-era fleet with an aide, unstamped),
  `registry/v2.json` (the 0.4.0 install's), `needs/v1.json` (written by the
  0.4.0 binary's hook in a sandbox). The pinned shapes are
  `tests/fixtures/formats/shapes/<artifact>.v<N>.{json,toml}`; Admiralty's
  `Saved` is pinned by `crates/admiralty/tests/formats.rs` into the same
  folder. The shapes catch a field added, renamed or dropped that the
  example serializes; a new optional field skipped when empty needs the
  example to set it.
- **Tests.** Unit: `formats::tests` (each step on its fixture, idempotence,
  the runner's backup, log, resume and second run, the guard), the registry
  and active guards, `config`'s too-new fleet and squadron. CLI:
  `crates/nimitz/tests/upgrade.rs` (the 0.4.0 install under this build:
  `status`, `check`, `start --plan`; the hook never migrates; a newer
  nimitz's state; the shared step). Real Claude Code: `e2e_97` in
  `crates/nimitz/tests/e2e.rs` (`NIMITZ_E2E=1 NIMITZ_E2E_OLD_BIN=<0.4.0 build>`).
