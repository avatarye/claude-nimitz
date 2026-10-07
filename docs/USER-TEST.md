# User test: claude-nimitz on west-mac

Date: 2026-10-04. Branch: `dev`. For: you, before the first real use.

## 1. What is ready

| Part | State |
| --- | --- |
| `nimitz` (CLI, hook, plugin) | All commands in `nimitz help`. Tested in the sandbox and on real Claude Code (ACME 2.1.288). |
| `admiralty` (TUI) | Fleet and squadron frames, issues view, forms, actions. Screens in `docs/screens/`. |
| Herdr plugin (`herdr-plugin/`) | Plugin id `admiralty` (was `admiralty-watch`); runs `admiralty`. |
| Main fleet | Migrated to `~/.claude-nimitz/main` (Dropbox). 10 squadrons, 29 ships. No session was started or stopped. |
| Tests | 315 unit, sandbox and CLI tests, none skipped; 55 adversarial bugs found, all fixed and re-tested; 4 rounds on real Claude Code, including history checks on every relaunch path. Reports: `docs/test/report-round1.md`. |

## 2. Install

1. `cd ~/Github/claude-nimitz && git checkout dev && git pull` (`main` holds only the plan; code is on `dev`)
2. `cargo install --path crates/nimitz` and `cargo install --path crates/admiralty`
3. Plugin: run the command that `nimitz doctor` prints (marketplace add, then install).
4. `nimitz init --label west-mac`, then `nimitz doctor`. Every line must be `[OK]`.
5. Herdr: `herdr plugin unlink admiralty-watch` (the old id, wherever it was linked
   from), then `herdr plugin link ~/Github/claude-nimitz/herdr-plugin`.
   In `~/.config/herdr/config.toml` change the key's `command` to
   `admiralty.toggle`, then `herdr server reload-config`. `alt+f` opens `admiralty`.

## 3. Look first (read-only)

- `nimitz status` and `admiralty`: see MAIN with its squadrons.
- `nimitz check MAIN`. Expect only these:
  - `legacy` warnings for old leads that still run (today: ARGOS, BRIM, GENERAL);
  - nothing for the user's own sessions in the ARGOS ship folder ("publish plan draft spec", `[ARGOS]-RAGTIME`): they are not shown;
  - a trust warning for CARPENTER/LIB (it has never run there).
- `nimitz up MAIN --plan`: what a full start would do. 19 ships continue their
  newest old session (same history), 4 start fresh, and 10 commodores start fresh
  in their new folders.

## 4. Switch the fleet over (top-down, inside Herdr)

1. Admiral: `nimitz start MAIN`. It opens `[MAIN]-ADMIRAL` in
   `~/.claude-nimitz/main/admiral/`. This replaces `[ADMIRALTY]`; exit the old hub
   when you are content.
2. For each squadron, one at a time:
   1. In the old lead's pane, let it finish, then `/exit`.
   2. Old ships that still run: `nimitz adopt <sid> MAIN/<SQ>/<SHIP>` (the fix in
      `nimitz check` names the sid), then `nimitz relaunch MAIN/<SQ>/<SHIP>`. Same
      session id, history kept, new name and labels.
   3. `nimitz start MAIN/<SQ>`: the commodore. Then its ships start top-down
      (`nimitz start MAIN/<SQ>/<SHIP>`, or `nimitz up MAIN` at the end).
3. `nimitz check MAIN` after each squadron. Fix what it shows before the next one.
4. When all squadrons run: `nimitz sweep MAIN --plan` shows the old stopped rows it
   would remove (their transcripts stay). Run it only if you want them gone.

## 5. Known limits

- Windows: not supported yet (Unix-only calls). The home handoff
  (`docs/probes/HANDOFF-round3.md`) records what fails.
- Public Claude Code 2.1.289: in `compat.toml`, but tested on the home machines only
  after you run the handoff there.
- Do not adopt a hand-made copy of a ship (a copy made with flags and no prompt can
  have no history).

## 6. If something looks wrong

`nimitz check <id>` explains each problem and prints a `nimitz` fix. Every fix was
tested. Plans run only after you confirm (`--plan` shows them first).
