# P0 probes: requirements

## Why

claude-nimitz gives each agent its identity at launch and reads its state from
Claude Code. The design depends on facts about `claude --bg` that the docs do
not state, or that differ between builds. a company's internal build (ACME) wraps
Claude Code and can behave differently from the public build. The probes record
the facts for each build, so the design uses only what all builds do.

## What each probe checks

| Id | Question | Why the design needs it |
| --- | --- | --- |
| 0 | Does `--bg` start in a folder that is not trusted? | `nimitz` creates new ship folders. They must be trusted first. |
| 1 | Does `--bg` accept `--session-id <uuid>`? | If yes, `nimitz` can choose each session id. If no, it learns the id from the SessionStart hook. |
| 2a | Does `--bg --resume <uuid that names no session>` start a session with that id? | A second way `nimitz` might choose ids. |
| 2b | What happens on `--bg --resume <id>` while the session is **live**, with no flags? | `nimitz` must never make a copy of a live ship. |
| 2 | The same, with `--name`, `--permission-mode` and `--settings` passed again. | Same reason; the flags decide copy or continue. |
| 2c | What happens on `--bg --resume <id>` after the session is **stopped**: with no flags (2c1), with `--name` only (2c2), with all flags (2c3)? Same id or a new one? | This is the normal relaunch of a ship. It decides how lineage works. Any flag on resume starts a copy, so each case is probed. |
| 3 | Do env labels in `--settings '{"env":{...}}'` reach hooks in a `--bg` session? Which hooks fire? | Identity binding uses these labels. |
| 4 | Does a SessionStart hook's `sessionTitle` change the name in `claude agents --json` (the messaging address)? | Decides if a hook can repair a drifted name. |
| 5 | Can two live `--bg` sessions have the same name? | `nimitz launch` must check names itself if Claude Code does not. |
| — | Fields in `claude agents --json` and in `~/.claude/sessions/<pid>.json` | Pinned formats for `nimitz doctor`. |

## Rules for a probe run

1. Run on a machine where you may start Claude Code sessions.
2. The probe folder must be inside a trusted folder (default
   `~/.cache/nimitz-probe`). Do not rely on the home folder: on the public build
   accepting trust in `~` was not saved. Run `claude` once in the probe folder and
   accept the prompt. Set `NIMITZ_PROBE_DIR` to change it.
3. The script starts about ten short background sessions named
   `nimitz-probe-*`, each with a one-line prompt and no tools. At the end it
   stops and removes every one of them. `cleanup.rows_left` must be empty.
4. Do not run it while you depend on agent view being quiet: the rows show
   there for about two minutes.

## How to run

```
python3 probes/p0_probe.py
```

The result is `$NIMITZ_PROBE_DIR/results.json` (default
`~/.cache/nimitz-probe/results.json`).

## How to report

1. Copy `results.json` to `docs/probes/results-<build>-<version>-<provider>.json`,
   for example `results-vanilla-2.1.288-subscription.json`. Remove
   `last_assistant_message` fields.
2. Write `docs/probes/findings-<build>-<version>.md` in the same form as
   `findings-acme-2.1.288.md`.
3. Add a row per probe to the comparison table in `docs/probes/README.md`.
4. Commit and push.
