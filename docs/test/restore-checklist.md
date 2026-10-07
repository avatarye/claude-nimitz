# Restore checklist: real Herdr, by hand

**Never stop your own Herdr server for this.** Every step below runs against
a separate, named Herdr session (`nimitz-e2e`: its own server, its own
sockets under `~/.config/herdr/sessions/nimitz-e2e/`), and "server killed"
means `herdr --session nimitz-e2e server stop`, nothing else. Your live
session (`NIMITZ`), its tabs and your own panes are never touched.

What the fake Herdr cannot show: a server killed, a tab closed, a tab
renamed, a reboot-like loss, and a restore run twice, on the real Herdr
0.9.3 and the real Claude Code. About ten minutes. Every step: the action,
the exact command, the expected result.

Scope: a throwaway fleet `NIMITZ_E2E_RESTORE` under `~/.cache/nimitz-e2e`
(fleet names are `[A-Z0-9_]+`, so the underscore). Its sessions are named
`[NIMITZ_E2E_RESTORE]-ADMIRAL` and `[NIMITZ_E2E_RESTORE.SQ]-COMMODORE`. Nothing
below touches another fleet or `~/.claude-nimitz`.

Run every command from a **plain terminal** (not a Herdr pane), or prefix
each with `env -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH
-u HERDR_PANE_ID -u HERDR_WORKSPACE_ID -u HERDR_TAB_ID`, so a live pane's
socket is never inherited. nimitz speaks to the test server only through
`NIMITZ_HERDR_SESSION=nimitz-e2e`, which its Herdr adapter passes as
`--session nimitz-e2e` on every `herdr` call (`contracts.md` section 2);
`NIMITZ_MUX=herdr` turns the adapter on outside a Herdr pane.

Herdr's default `[session] resume_agents_on_restore = true` makes the server
itself run `claude --resume <session>` in every restored agent pane after a
restart, so the panes would not be plain shells. The test server gets a
private config with that setting off (step 2a); the live server's config is
not touched.

Steps 1-6 set up; 7-19 test; 20-23 clean up. Mark each step PASS or FAIL.

## Setup

| # | Action | Command | Expected |
| --- | --- | --- | --- |
| 1 | Build this branch's binary; never `cargo install` | `cd <worktree> && cargo build -p nimitz && export N=$PWD/target/debug/nimitz` | `$N --version` prints `nimitz 0.4.0` (not the 0.3.1 on PATH); `$N help \| grep restore` prints the restore line |
| 2a | A private Herdr config for the test server: no agent resume on restore | `mkdir -p ~/.cache/nimitz-e2e/restore && printf 'onboarding = false\n[session]\nresume_agents_on_restore = false\n' > ~/.cache/nimitz-e2e/restore/herdr.toml && export HERDR_CONFIG_PATH=~/.cache/nimitz-e2e/restore/herdr.toml && herdr config check` | `config check` passes; `~/.config/herdr/config.toml` (the live one) is unchanged |
| 2 | The test Herdr session: a headless server of its own, on that config | `herdr --session nimitz-e2e server &` | `ls ~/.config/herdr/sessions/nimitz-e2e/` shows `herdr.sock`; `herdr --session nimitz-e2e status` says running; `herdr status` (your live session) is as before |
| 3 | A private data root and state dir, and the test session for every nimitz call | `export NIMITZ_HOME=~/.cache/nimitz-e2e/restore/nimitz NIMITZ_STATE=~/.cache/nimitz-e2e/restore/state NIMITZ_MUX=herdr NIMITZ_HERDR_SESSION=nimitz-e2e && mkdir -p $NIMITZ_HOME $NIMITZ_STATE` | both folders exist; `NIMITZ_HOME` is not `~/.claude-nimitz` |
| 4 | Trust the root once (real Claude Code refuses an untrusted folder) | `cd $NIMITZ_HOME && claude` then accept the trust prompt and `/exit` | the prompt said "trust this folder"; the session ended |
| 5 | The fleet: one squadron, no ship | `$N fleet new NIMITZ_E2E_RESTORE --machine here --yes && $N squadron new NIMITZ_E2E_RESTORE/SQ --brief "restore test" --yes` | `[OK]` twice; `$N status` lists the fleet as not-started |
| 6 | Keep the live plugin out of the throwaway sessions: project settings that point the hooks at this binary | in `$NIMITZ_HOME/nimitz_e2e_restore/admiral/.claude/settings.json` and `.../squadrons/sq/.claude/settings.json` write `{"env":{"NIMITZ_STATE":"<the state dir>"},"enabledPlugins":{"nimitz@claude-nimitz":false},"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"<$N>","args":["hook","SessionStart"],"timeout":5}]}],"SessionEnd":[{"hooks":[{"type":"command","command":"<$N>","args":["hook","SessionEnd"],"timeout":5}]}]}}` (paths spelled out); `export NIMITZ_E2E_MODEL=haiku` keeps it cheap | the two files exist |

To look at the test session's tabs at any step: `herdr --session nimitz-e2e`
in another plain terminal (detach with Herdr's own detach key), or
`herdr --session nimitz-e2e api snapshot` for the shape as JSON.

## The fleet up, then a restore that has nothing to do

| # | Action | Command | Expected |
| --- | --- | --- | --- |
| 7 | Bring it up: the two leads as background jobs, then their tabs | `$N up NIMITZ_E2E_RESTORE --yes` | two `claude --bg` launches `[OK]`, then `[OK] NIMITZ_E2E_RESTORE: restored 2 lead(s), 0 ship(s)`; the test session has workspaces `NIMITZ_E2E_RESTORE` (tab `NIMITZ_E2E_RESTORE.ADMIRAL`) and `NIMITZ_E2E_RESTORE.SQ` (tab `NIMITZ_E2E_RESTORE.SQ.COMMODORE`), each running `claude attach <job>`; your live session has none of them |
| 8 | Look, and record the active set | `$N status && cat $NIMITZ_STATE/active.json` | both leads `idle` (or `busy`) with a `pane`; `active.json` has both members with `"attached": true`, a `place` whose `tab` is the full label, and `"herdr": {"session": "nimitz-e2e"}` |
| 9 | Run twice (R-12) | `$N restore NIMITZ_E2E_RESTORE --yes` | `[OK] NIMITZ_E2E_RESTORE: nothing to restore`; no new tab; nothing typed into the attached panes (the leads got no prompt: their panes show no new turn) |

## Tab closed, tab renamed, detached

| # | Action | Command | Expected |
| --- | --- | --- | --- |
| 10 | Close the commodore's tab (R-05) | `herdr --session nimitz-e2e tab list --workspace <id of NIMITZ_E2E_RESTORE.SQ>` for the tab id, then close that tab in the test session's UI (or `herdr --session nimitz-e2e tab close <id>` if your Herdr has it) | `$N status` still shows the commodore live (an attach client's death never stops the job) |
| 11 | Restore | `$N restore NIMITZ_E2E_RESTORE --plan` then `--yes` | the plan has one step, `herdr tab create --workspace NIMITZ_E2E_RESTORE.SQ --label NIMITZ_E2E_RESTORE.SQ.COMMODORE -- claude attach <job>`, no `--bg --resume`; after `--yes` the tab is back in the `NIMITZ_E2E_RESTORE.SQ` workspace (no second workspace), attached |
| 12 | Rename the admiral's tab (R-07a) | `herdr --session nimitz-e2e tab rename <id of NIMITZ_E2E_RESTORE.ADMIRAL> boss` | - |
| 13 | Restore | `$N restore NIMITZ_E2E_RESTORE --yes` | `nothing to restore`; `[INFO] ... tab boss (pane ...) shows NIMITZ_E2E_RESTORE; nimitz looks for NIMITZ_E2E_RESTORE.ADMIRAL (tab.renamed)` with the fix `herdr tab rename <id> NIMITZ_E2E_RESTORE.ADMIRAL  # or nimitz restore NIMITZ_E2E_RESTORE --rename`; no new tab. Then `$N restore NIMITZ_E2E_RESTORE --yes --rename`: one `herdr tab rename` step, the tab is `NIMITZ_E2E_RESTORE.ADMIRAL` again |
| 14 | Detach the commodore (R-02) | in the test session's UI, in its tab: `ctrl-z` | the tab drops to a shell; `$N status`: the commodore still live, no `pane` |
| 15 | Restore | `$N restore NIMITZ_E2E_RESTORE --yes` | one step, `herdr pane run <that pane> claude attach <job>`; the same tab shows the attach again; no second `NIMITZ_E2E_RESTORE.SQ.COMMODORE` tab |

## Server killed, reboot-like

| # | Action | Command | Expected |
| --- | --- | --- | --- |
| 16 | Kill the test server (R-03); start it again (same `HERDR_CONFIG_PATH`), it restores from its snapshot | `herdr --session nimitz-e2e server stop`, then `herdr --session nimitz-e2e server &` | the test session's workspaces and tabs come back by name; with `resume_agents_on_restore = false` every pane is a plain shell (`herdr --session nimitz-e2e pane process-info --pane <id>` lists only the shell; `herdr --session nimitz-e2e api snapshot` has an empty `agents` list); `claude agents` still lists both jobs live; `herdr status` (your live session) is as before |
| 17 | Restore | `$N restore NIMITZ_E2E_RESTORE --yes` | two `herdr pane run ... claude attach <job>` steps, into the restored tabs (found by label, their panes shells), no `tab create`, no `--bg --resume`; `[OK] NIMITZ_E2E_RESTORE: restored 2 lead(s), 0 ship(s)`; the snapshot's `agents` list has both panes again |
| 18 | Reboot-like loss (R-08): stop the test server and kill the throwaway workers without a `claude stop` hook (the daemon's view of a dead machine) | `herdr --session nimitz-e2e server stop`; `kill -9 <pid of each [NIMITZ_E2E_RESTORE...] worker>` (from `claude agents`); `herdr --session nimitz-e2e server &` | the tabs are back, every pane a plain shell (the setting is off: Herdr resumes nothing itself); `$N status`: both leads `parked` (no SessionEnd); `active.json` still holds both (a crash keeps a member in the set) |
| 19 | Restore | `$N restore NIMITZ_E2E_RESTORE --plan` then `--yes` | the plan: per lead one `herdr pane run <pane> claude attach <job>` into the shell pane (the attach wakes the parked job); a `pin` step only for a lead whose job is not in `~/.claude/jobs/pins.json` (after step 7 both are pinned, so normally none); after `--yes`: `[OK] <id>: the attach woke it; SessionStart resume of <sid> verified` for both, the same session ids as in step 8 (`$N status --json`), `[OK] ... restored 2 lead(s)`; a `startup` instead of `resume` would be `[ERROR] history.lost`, never `[OK]` |

## Optional: the setting on (your real config)

| # | Action | Command | Expected |
| --- | --- | --- | --- |
| 19a | The reboot-like loss again, with Herdr's default on: edit the test config, stop the server, kill the workers, start the server | `sed -i 's/resume_agents_on_restore = false/resume_agents_on_restore = true/' $HERDR_CONFIG_PATH; herdr --session nimitz-e2e server stop; kill -9 <each throwaway worker pid>; herdr --session nimitz-e2e server &` | each lead's pane now runs `claude --resume <sid>` (the conversation, interactive, no job: `claude agents` shows no live pid for the jobs; `$N status` shows each lead parked with a held interactive row) |
| 19b | Restore | `$N restore NIMITZ_E2E_RESTORE --yes` | no step; per lead `[ERROR] ... is open in pane <P> (Herdr's resume_agents_on_restore, or a claude --resume by hand) ... (tab.resumed)` with the fix `type /exit in pane <P>, then nimitz restore NIMITZ_E2E_RESTORE`; `[ERROR] NIMITZ_E2E_RESTORE: 2 member(s) not restored ... 2 held in a pane`, exit 1; no `--bg --resume` ran (a wake would make a copy) |
| 19c | Do the fix | `/exit` in each of the two panes, then `$N restore NIMITZ_E2E_RESTORE --yes` | one attach per lead: a `herdr pane run ... claude attach <job>` when the pane survived the `/exit` as a shell, a `herdr tab create ...` when the pane closed with it; either way `the attach woke it; SessionStart resume` for both and the same session ids as in step 8; `$N doctor` shows `[WARN] Herdr ...: resume_agents_on_restore is on` with the fix line. Put the setting back to `false` in `$HERDR_CONFIG_PATH` before step 20 |

## Cleanup

| # | Action | Command | Expected |
| --- | --- | --- | --- |
| 20 | Retire the fleet (stops and removes both jobs, keeps transcripts) | `$N squadron rm NIMITZ_E2E_RESTORE/SQ --yes && $N fleet rm NIMITZ_E2E_RESTORE --yes` | `[OK]` twice; `claude agents` lists no `[NIMITZ_E2E_RESTORE...]` session; `$N status` lists no fleet |
| 21 | Stop the test server | `herdr --session nimitz-e2e server stop` | `herdr --session nimitz-e2e status` says not running; `herdr status` (your live session) is as before |
| 22 | Remove the test session's files, its config and the throwaway data | `rm -rf ~/.config/herdr/sessions/nimitz-e2e ~/.cache/nimitz-e2e/restore && unset HERDR_CONFIG_PATH NIMITZ_HOME NIMITZ_STATE NIMITZ_MUX NIMITZ_HERDR_SESSION NIMITZ_E2E_MODEL N` | nothing of the test is left; the transcripts under `~/.claude/projects/-home-...-cache-nimitz-e2e-restore-*` stay (remove them yourself) |
| 23 | Your own Herdr | `herdr status` | running, the same pid as before step 2 |

Verdicts go into the acceptance report (plan-v4 section 10) as one line
per scenario id: R-02, R-03, R-05, R-07a, R-08, R-12.
