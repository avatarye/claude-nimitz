# `nimitz trust`: working notes

Status: built on `feat/trust` (2026-10-10), Opus-reviewed; the "Built" section
says what changed from the plan below. Not merged (branch policy).

## The request (the user, via the commodore)

- `nimitz trust <folder>` sets Claude Code's trust for the folder's git root
  (or the folder itself outside git): `projects["<path>"].hasTrustDialogAccepted
  = true` in the trust file (`<claude_home>.json`, i.e. `~/.claude.json`).
  Atomic write, a backup kept. The format is undocumented: doctor's pinned
  formats check it.
- `ship new`, `ship set --dir` and `start` show "trust <path> (writes
  ~/.claude.json)" as a plan step when the target folder is not trusted, and
  run it on `--yes`. `folder.untrusted`'s fix becomes `nimitz trust <path>`.
- The user's alone: members get an authority refusal (trust lets the repo's own
  `.claude` settings, hooks and MCP servers run); the plan line says so.
- Round-2 facts: a git repo needs trust on its own root; a parent's trust does
  not cover it. Write both the path as given and the real path.
- Tests in the sandbox; one real-CC e2e (an untrusted throwaway repo, then
  `nimitz trust`, then `--bg` starts without the dialog).

## What Claude Code does (2.1.296 binary)

- It writes the trust file under a lock: proper-lockfile on `<file>` with
  `lockfilePath: "<file>.lock"` (a directory made with mkdir; stale after
  ~10 s, refreshed by the holder). Under the lock it re-reads the file, applies
  its change and merges `projects` (`wi(old.projects, new.projects)`), so a
  writer that takes the same lock and writes atomically is safe.
- Its own trust grant (`Fw`): `{...projects[k], hasTrustDialogAccepted: true}`
  on key `oC(dir)` (git root when in a repo, else the folder, normalized). The
  runner writes keys as given, NFC, realpath and realpath NFC.
- A `.claude.json.backup` and timestamped backups exist on its side.

## Plan

1. core `trust.rs`: `target(dir)` (git root found from the path as given and
   from the real path, else the folder), `keys(target)` (as given, real path,
   NFC forms, deduped), `state(env, dir)` (trusted / parent-only for a repo /
   untrusted; `check::trust_issue` moves onto it), `write(env, dir)` (mkdir
   `<file>.lock` with a short retry and the same stale rule; backup to
   `NIMITZ_STATE/backups/claude.json.<ts>`, keep 5; re-read under the lock;
   merge `{...old, hasTrustDialogAccepted: true}` per key; temp file + fsync
   + rename, mode 0600; rmdir the lock), `format_ok(env)` for doctor.
2. Step `["trust", <dir>]`: `Kind::Config` in `sessions::classify`;
   `config::run_step` and its verify learn it (needs `env`).
3. `plan_start_with`: right after `gate`, for the user only, each
   `folder.untrusted` (Error or Warn) of the chain becomes a trust step at the
   front plus an Info warning naming the consequence; a member keeps the
   refusal, its fix `nimitz trust <path>` (user-only, so "ask the user to
   run"). The engine's action text says "trust, then first launch".
4. `crud::ship` new / set with a new `--dir`: the same helper after the config
   plan (it knows the caller).
5. CLI `nimitz trust <folder>`: plan, gate, apply; refusal for members; usage,
   `COMMANDS`, the authority table row (plan-v4 section 12.1, the cli test).
6. `check::untrusted_fix` -> `nimitz trust <path>`; init's warning too.
7. doctor pinned formats: the trust file is an object, `projects` an object of
   objects, `hasTrustDialogAccepted` a bool where present.
8. Tests: core (keys, git root, lock wait and stale, backup rotation, merge
   keeps other fields, atomic), sandbox CLI (user plan/run, member refusal,
   ship new and start show the step), e2e (real CC).

## Reconcile with origin/windows-port

`d0f1b20` and `0145dfc` change the same code: `trust_issue` (inside NIMITZ_HOME
only when the data root is trusted; a case twin is named), `env::trust_key`
(keeps case on Windows), the Windows fix text (PowerShell). Build `trust.rs` on
those rules (rebase onto windows-port first, or port them), never the dev copy.
On Windows: the lock directory, `MoveFileEx` replace for the rename, keys with
`\` and `/` both written as Claude Code compares them.

## Open item

Rewriting `~/.claude.json` through serde_json sorts its keys. serde_json's
`preserve_order` keeps them; enabling it workspace-wide failed one test only
(`setup::tests::a_command_string_hook_is_named`, an order assertion). Decide
then.

## Built (and what the review changed)

- `trust::target` takes the git root of the **real** path (Claude Code
  resolves links first) and keeps the typed form only when it leads to that
  root; inside the data root, outside git, the target is the data root.
- Never trusted (`trust.broad`, for everyone, also in `write`): `/`, the
  user's home or a parent of it, a parent of the data root. Claude Code never
  saves trust for the home folder, and its lookup outside git walks up.
  `init` and `doctor` say so instead of planning it (a dotfiles repo at `~`).
- `start`: only the user's, only the started member's own folder, only on its
  first launch. A wake, a parent's folder and a fleet- or squadron-wide start
  keep the refusal with the fix `nimitz trust <path>, then nimitz start <id>`.
- `ship new|set --dir`: only for this machine's folder (`--machine` another
  machine plans no trust).
- Writes go to every config file nimitz reads trust from that exists
  (`<claude home>/.claude.json`, `<claude home>.json`); with none, the one
  Claude Code would make. Backups per source, the newest five plus `.first`.
- serde_json `preserve_order` is on (the rewrite keeps Claude Code's order;
  `forget` uses `shift_remove`).
- e2e `e2e_trust_lets_a_background_job_start_in_an_untrusted_repo` (#96):
  real Claude Code 2.1.296 refuses `--bg` in a fresh repo ("Workspace not
  trusted"), `nimitz trust` writes it, the same `--bg` starts.

## Left for later

- Admiralty plans with `--plan --json` and runs `--yes`, which plans again: a
  trust step that appears in between (a member's `ship set --dir` in that
  window) is not in the confirm the user saw. Narrowed (only a named first
  launch trusts); closing it needs `--yes` to carry the confirmed plan.
- `nimitz up` (deprecated) still refuses an untrusted folder.
- Windows, at the rebase onto windows-port: keys through `env::trust_key`
  (`/` separators, case as on disk), `canonicalize`'s `\\?\` prefix,
  PowerShell quoting in the fix, `case_exact_hint`.
- NFC key forms (macOS) are not written.
- Claude Code's gated "storage v5" config backend (a versioned update, not the
  lock directory) is not verified to notice nimitz's rename.
