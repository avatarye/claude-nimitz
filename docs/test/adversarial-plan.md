# Adversarial test plan

The tester plays a careless or curious user. The goal: find every way the shape
of a fleet can break, and check that `nimitz` and Admiralty (a) see it, (b) say
what is wrong in plain words, (c) offer a fix that works, and (d) never make it
worse. Each scenario below gets a result line in a test report:
`PASS | FAIL | PARTIAL`, the commands run, what nimitz said, and for a FAIL the
smallest steps to reproduce.

## Where to test

| Layer | How |
| --- | --- |
| Sandbox | `nimitz sandbox create /tmp/nz-sb`; all commands with the env from `nimitz sandbox env`. Fake `claude` and fake `herdr`. Every scenario first runs here. |
| Real Claude Code | `NIMITZ_HOME`, `NIMITZ_STATE` under `~/.cache/nimitz-e2e/<run>/`; a test fleet `E2E`; sessions named `[E2E.*]-*`; real `claude` (on this machine the ACME wrapper and the native binary). Stop and remove every session you started; check that none is left. Never touch another session. |
| Real Herdr | A private Herdr server only (temp `XDG_CONFIG_HOME`/`XDG_STATE_HOME`, every `HERDR_*` unset), never the user's. |

## Scenarios

### A. Starting and order
1. `nimitz start E2E/SQ/SHIP` with no admiral and no commodore running: must start the admiral, then the commodore, then the ship (or refuse with the reason when no mux).
2. Start a ship twice quickly (two shells at the same time).
3. Start a ship whose folder does not exist; whose folder is not trusted.
4. `nimitz up` outside a multiplexer; inside the private Herdr.
5. Start a commodore from inside a ship (env `NIMITZ_ROLE=ship`): must refuse.
6. A commodore starts a ship of another squadron: must refuse.
7. Claude Code version outside `compat.toml` (fake `--version`): read-only works, launches refuse, `--force` overrides.

### B. Breaking a live session by hand
1. `/rename` a live ship (fake: change its name in the session file and job row) → `name.drift`, fix works.
2. Start a second session with the same name by hand: `claude --bg --name "[E2E.SQ]-SHIP"` → `dup.live` or `name.taken`.
3. Resume a live ship by hand with `claude --bg --resume <id>` (copy) → `copy.rows`, labels missing on the copy.
4. Resume a stopped ship by hand **with** flags (`--name` only) → copy without labels → `labels.missing` / stowaway; relaunch repairs it.
5. Resume through the ACME wrapper with no flags (makes a copy) → detected.
6. `claude stop` a ship by hand, then `nimitz start` it → same session id kept.
7. `claude rm` a ship's current row by hand → `lineage.broken`; start offers a cold launch only with `--force`.
8. Background a commodore (`←` on an empty prompt; fake: kind `background`) → `role.background`.
9. Kill a session process (`kill -9 <pid>`) → session file left behind → not live, no crash.
10. Corrupt a session file (trailing bytes, truncated JSON, missing fields, extra fields) → tolerated.
11. Corrupt `jobs/<id>/state.json`; delete a transcript.

### C. Breaking the config and folders by hand
1. Edit `squadron.toml` with a syntax error; with an unknown key; two ships with one name; a lowercase name.
2. Move a ship's folder; make a ship folder a child of the commodore's folder.
3. Put a `CLAUDE.md` in `<fleet>/`.
4. Delete `registry.json`; corrupt it; make it read-only.
5. Two `nimitz` commands writing the registry at once.
6. A ship in the shared managed folder started on a second "machine" (fake hostname) while live on the first → `ship.elsewhere`.

### D. Stowaways and strays
1. A plain `claude --bg "do x"` in a ship's folder (no labels) → stowaway; `adopt` and `rm` both work.
2. A session named like a fleet member but in a random folder.

### E. Signals
1. A ship sends `SendMessage` to a ship in its squadron with `comms = open` → allowed.
2. With `commodore-only` → denied with the reason; to its commodore → allowed.
3. A ship signals the admiral directly → denied.
4. The admiral signals a ship → denied (admiral asks commodores only).

### F. Hooks
1. A hook gets garbage on stdin, an empty stdin, a huge stdin → exit 0, fast.
2. `NIMITZ_STATE` not writable → exit 0, the session is not harmed.
3. Events arrive out of order; SessionStart lost (delete the line) → binding still recovers from labels in saved options / the next event, or the issue is reported.

### G. Admiralty
1. Every state above shows in Admiralty with a clear marker, a reason, and the fix; nothing is hidden by folds or filters without a hint.
2. No multiplexer → monitor-only, start/open keys disabled with the reason.
3. Small terminal (80x24), large (250x60); light of contrast; ASCII icons.
4. Rapid refresh while sessions change; no flicker, cursor stays.
5. Forms: invalid input is refused with the field marked; the shown `nimitz` command is the one that runs.

### H. Migration
1. Import copies of the real old fleet configs into a temp `NIMITZ_HOME`; check the report and the result with `nimitz check`.
2. Run the migration twice: no duplicates.
