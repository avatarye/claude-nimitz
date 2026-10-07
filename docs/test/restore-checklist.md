# Restore checklist: real Herdr, by hand

What the fake Herdr cannot show: a server killed, a tab closed, a tab
renamed, a reboot-like loss, and a restore run twice, on the real Herdr
0.9.3 and the real Claude Code. About ten minutes. Every step: the action,
the exact command, the expected result.

Scope: a throwaway fleet `NIMITZ_E2E_RESTORE` under `~/.cache/nimitz-e2e`
(fleet names are `[A-Z0-9_]+`, so the underscore). Its sessions are named
`[NIMITZ_E2E_RESTORE]-ADMIRAL` and `[NIMITZ_E2E_RESTORE.SQ]-COMMODORE`. Nothing
below touches another fleet, your live Herdr tabs, or `~/.claude-nimitz`.

Do the steps in your `NIMITZ` Herdr session (the one with your fleets): the
point is that a restore matches by label and session, never by pane id,
beside tabs that are not its own. Steps 1-5 set up; 6-18 test; 19-21 clean
up. Mark each step PASS or FAIL in the last column.

## Setup

| # | Action | Command | Expected |
| --- | --- | --- | --- |
| 1 | Build this branch's binary; never `cargo install` | `cd <worktree> && cargo build -p nimitz && export N=$PWD/target/debug/nimitz` | `$N --version` prints `nimitz 0.4.0` (not the 0.3.1 on PATH) |
| 2 | A private data root and state dir | `export NIMITZ_HOME=~/.cache/nimitz-e2e/restore/nimitz NIMITZ_STATE=~/.cache/nimitz-e2e/restore/state && mkdir -p $NIMITZ_HOME $NIMITZ_STATE` | both folders exist; `NIMITZ_HOME` is not `~/.claude-nimitz` |
| 3 | Trust the root once (real Claude Code refuses an untrusted folder) | `cd $NIMITZ_HOME && claude` then accept the trust prompt and `/exit` | the prompt said "trust this folder"; the session ended |
| 4 | The fleet: one squadron, no ship | `$N fleet new NIMITZ_E2E_RESTORE --machine here --yes && $N squadron new NIMITZ_E2E_RESTORE/SQ --brief "restore test" --yes` | `[OK]` twice; `$N status` lists the fleet as not-started |
| 5 | Keep the live plugin out of the throwaway sessions: project settings that point the hooks at this binary | in `$NIMITZ_HOME/nimitz_e2e_restore/admiral/.claude/settings.json` and `.../squadrons/sq/.claude/settings.json` write `{"env":{"NIMITZ_STATE":"<the state dir>"},"enabledPlugins":{"nimitz@claude-nimitz":false},"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"<$N>","args":["hook","SessionStart"],"timeout":5}]}],"SessionEnd":[{"hooks":[{"type":"command","command":"<$N>","args":["hook","SessionEnd"],"timeout":5}]}]}}` (paths spelled out, `--model haiku` keeps it cheap: `export NIMITZ_E2E_MODEL=haiku`) | the two files exist; `$N doctor` is not needed for them |

## The fleet up, then a restore that has nothing to do

| # | Action | Command | Expected |
| --- | --- | --- | --- |
| 6 | Bring it up: the two leads as background jobs, then their tabs | `$N up NIMITZ_E2E_RESTORE --yes` | two `claude --bg` launches `[OK]`, then `[OK] NIMITZ_E2E_RESTORE: restored 2 lead(s), 0 ship(s)`; Herdr has workspaces `NIMITZ_E2E_RESTORE` (tab `NIMITZ_E2E_RESTORE.ADMIRAL`) and `NIMITZ_E2E_RESTORE.SQ` (tab `NIMITZ_E2E_RESTORE.SQ.COMMODORE`), each running `claude attach <job>` |
| 7 | Look, and record the active set | `$N status && cat $NIMITZ_STATE/active.json` | both leads `idle` (or `busy`) with a `pane`; `active.json` has both members with `"attached": true` and a `place` whose `tab` is the full label |
| 8 | Run twice (R-12) | `$N restore NIMITZ_E2E_RESTORE --yes` | `[OK] NIMITZ_E2E_RESTORE: nothing to restore`; no new tab; nothing typed into the attached panes (the leads got no prompt: their panes show no new turn) |

## Tab closed, tab renamed, detached

| # | Action | Command | Expected |
| --- | --- | --- | --- |
| 9 | Close the commodore's tab (R-05) | in Herdr: close tab `NIMITZ_E2E_RESTORE.SQ.COMMODORE` (its only pane) | `$N status` still shows the commodore live (an attach client's death never stops the job) |
| 10 | Restore | `$N restore NIMITZ_E2E_RESTORE --plan` then `--yes` | the plan has one step, `herdr tab create --workspace NIMITZ_E2E_RESTORE.SQ --label NIMITZ_E2E_RESTORE.SQ.COMMODORE -- claude attach <job>`, no `--bg --resume`; after `--yes` the tab is back in the `NIMITZ_E2E_RESTORE.SQ` workspace (no second workspace), attached |
| 11 | Rename the admiral's tab (R-07a) | in Herdr: rename tab `NIMITZ_E2E_RESTORE.ADMIRAL` to `boss` | - |
| 12 | Restore | `$N restore NIMITZ_E2E_RESTORE --yes` | `nothing to restore`; `[INFO] ... tab boss (pane ...) shows NIMITZ_E2E_RESTORE; nimitz looks for NIMITZ_E2E_RESTORE.ADMIRAL (tab.renamed)` with the fix `herdr tab rename <id> NIMITZ_E2E_RESTORE.ADMIRAL`; no new tab. Rename it back by hand |
| 13 | Detach the commodore (R-02) | in its tab: `ctrl-z` | the tab drops to a shell; `$N status`: the commodore still live, no `pane` |
| 14 | Restore | `$N restore NIMITZ_E2E_RESTORE --yes` | one step, `herdr pane run <that pane> claude attach <job>`; the same tab shows the attach again; no second `NIMITZ_E2E_RESTORE.SQ.COMMODORE` tab |

## Server killed, reboot-like

| # | Action | Command | Expected |
| --- | --- | --- | --- |
| 15 | Kill the Herdr server (R-03); it restarts from its snapshot | `herdr server stop` (or kill it), start Herdr again as you always do | the workspaces and tabs come back by name, every pane a plain shell; `claude agents` still lists both jobs live |
| 16 | Restore | `$N restore NIMITZ_E2E_RESTORE --yes` | two `herdr pane run ... claude attach <job>` steps, into the restored tabs (found by label), no `tab create`, no `--bg --resume`; `[OK] NIMITZ_E2E_RESTORE: restored 2 lead(s), 0 ship(s)`; your other tabs untouched |
| 17 | Reboot-like loss (R-08): kill the server, stop the throwaway jobs without a `claude stop` hook (the daemon's view of a dead machine) | `herdr server stop`; `kill -9 <pid of each [NIMITZ_E2E_RESTORE...] worker>` (from `claude agents`); start Herdr again | `$N status`: both leads `parked` (no SessionEnd); `active.json` still holds both (a crash keeps a member in the set) |
| 18 | Restore | `$N restore NIMITZ_E2E_RESTORE --plan` then `--yes` | the plan: per lead one `herdr pane run <pane> claude attach <job>` (the attach wakes the parked job) and a `pin`; after `--yes`: `[OK] <id>: the attach woke it; SessionStart resume of <sid> verified` for both, the same session ids as in step 7 (`$N status --json`), `[OK] ... restored 2 lead(s)`; a `startup` instead of `resume` would be `[ERROR] history.lost`, never `[OK]` |

## Cleanup

| # | Action | Command | Expected |
| --- | --- | --- | --- |
| 19 | Retire the fleet (stops and removes both jobs, keeps transcripts) | `$N squadron rm NIMITZ_E2E_RESTORE/SQ --yes && $N fleet rm NIMITZ_E2E_RESTORE --yes` | `[OK]` twice; `claude agents` lists no `[NIMITZ_E2E_RESTORE...]` session; `$N status` lists no fleet |
| 20 | Close the two workspaces by hand | in Herdr: close `NIMITZ_E2E_RESTORE` and `NIMITZ_E2E_RESTORE.SQ` | your other workspaces as before |
| 21 | Remove the throwaway data | `rm -rf ~/.cache/nimitz-e2e/restore` and `unset NIMITZ_HOME NIMITZ_STATE N` | nothing of the test is left; the transcripts under `~/.claude/projects/-home-...-cache-nimitz-e2e-restore-*` stay (remove them yourself) |

Verdicts go into the acceptance report (plan-v4 section 10) as one line
per scenario id: R-02, R-03, R-05, R-07a, R-08, R-12.
