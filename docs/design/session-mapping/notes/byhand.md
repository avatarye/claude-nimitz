# Area `byhand`: by-hand `claude` in member folders (case 2) and interactive leads (case 4)

Tested on Claude Code **2.1.292** (installed; nimitz warns "newer than tested 2.1.290") and nimitz 0.3.1.
World: `$MAP=~/.cache/nimitz-e2e/nimitz-e2e-map-byhand-649485`, fleet `NIMITZ_E2E_BH`, squadron `SQ`, ship `S1`
(folder `$MAP/ship1`, launched by `nz start`, session **528aba20**). Interactive sessions ran in the private tmux
server `map-byhand-649485` with a clean env (`env -i` + `cl`). The org policy overrides `model: haiku` with Opus,
so each "reply ok" cost about $0.25.

Harness note: `cl` already means `claude`. The brief's `cl claude agents --json --all` therefore runs
`claude claude agents ...` and fails with "unknown option '--json'". The correct form is `cl agents --json --all`.
That listing includes **interactive** sessions too (`id: null`, `kind: interactive`).

## 1. Results

### Case 2: a new session in a ship's folder, by hand

| case | expected | observed | verdict |
| --- | --- | --- | --- |
| 2b-i `claude -c` while the ship is live (the ship's bg transcript is the only one in the folder) | continue nothing, or refuse | "No conversation found to continue". `-c` skips a session that still has a live job row. | OK |
| 2a plain `claude` (interactive), then `/exit` | a stowaway while it is live, gone after exit | Session **b3b04dba**, auto name `ship1-c7`, `kind interactive`, no job row. SessionStart `source startup`, `labels {}`. `nz status` lists it under stowaways, and `nz check S1` gives a WARN `stowaway` with fix "nimitz stop b3b04dba, then nimitz rm". S1 stays Idle and untouched. After `/exit` (SessionEnd `prompt_input_exit`) it disappears from every nimitz view; only the transcript stays. | OK (fix text wrong, see G4) |
| 2b-ii `claude -c` after 2a | continue 2a as a stowaway | Continued **b3b04dba** (same id; Claude Code renamed it `ship1-2a`). Again a stowaway. | OK |
| 2c-live `claude --resume 528aba20` (no labels) while the ship runs in the background | the member, no copy | The CLI re-execs as `claude attach 528aba20`. No new session, no SessionStart, and the prompt went to the bg ship with its labels (UserPromptSubmit pid 637434). The member stays intact. `/exit` from the attach does not stop the ship: it opens the **agent view** (the machine-wide list, including the live fleet), and that view claimed a pre-warmed `bg-spare` process as a new unlabelled bg session **5fdc2aea** (`agent_type: claude`, SessionStart `startup`) in the ship folder. nimitz showed 5fdc2aea as a live bg stowaway until the agent-view client exited (SessionEnd `other`, no job row left). Ctrl-C does not leave the agent view; I killed the client. Killing an attach client does not stop the ship. | OK / GAP (transient phantom stowaway, G5) |
| 2c-stopped `claude --resume 528aba20`, interactive, ship stopped by `nz stop` | the member is flagged, and nimitz can repair it | The **same id** 528aba20 runs `kind interactive`, pid 694901, name `[NIMITZ_E2E_BH.SQ]-S1`, SessionStart `source resume`, `labels {}`. The job row (done) is still there. nimitz: S1 Idle with kind interactive, ERROR `labels.missing` ("resumed with flags, or legacy", which is the wrong cause), INFO `rules.old` (the session started on rules v0 because the hook handed out no rules). Nothing says "a ship is running interactive". The fix `nimitz relaunch S1` **fails**: "S1 is live with no job id; cannot stop it (session.none)". `nz start` only plans the parents. After `/exit`: S1 Stopped plus `labels.missing`. `nz start S1 --yes` then ran `claude rm 528aba20` and `--bg --resume 528aba20` with every flag: the **same id**, labels back, the interactive turns kept in history (6 user lines), and a lineage entry `via relaunch`. | GAP (dead-end fix, G3) / recovery OK |
| 2c' `claude -c` with the ship stopped and its transcript the newest | same as 2c-stopped | `-c` picked **528aba20**, the member's own session, as interactive with no labels. Same outcome as 2c-stopped: `labels.missing`. | GAP (same as G3) |
| 2d `claude --bg "reply ok"` | a stowaway that is easy to remove | Job **4a379db3**, auto name "reply validation", `labels {}`, `source startup`. A live bg stowaway with fix "nimitz stop, then nimitz rm"; `nz stop` and `nz rm` both worked. S1 not affected. | OK |
| 2e `claude --resume 528aba20 --fork-session`, interactive, ship live | a copy row of S1, or at least flagged | New id **5e5666af**, SessionStart `source fork`, `labels {}`, and it **keeps the member's exact name** `[NIMITZ_E2E_BH.SQ]-S1` (nameSource user). Its transcript shares all 42 message uuids with 528aba20, but neither the session file nor the transcript has a forkedFrom field. nimitz lists it as a stowaway, not a copy row, and S1 gets ERROR `name.taken` with fix "nimitz stop 5e5666af", which **fails** with "not a live background session (session.none)". While it lives, two live sessions answer to S1's name. | GAP (G2, G4) |
| 2e-bg `claude --bg --resume 528aba20 --fork-session "reply ok"` | a copy row of S1 | Job **d2607b4e**, `source fork`, auto name "user response acknowledgment" (nameSource auto). The job state.json has no fork field (`respawnFlags []`). nimitz lists it as a stowaway. Its copy-by-fork rule needs an empty name and a `forkedFrom`-like key, and on 2.1.292 neither exists, so the rule never fires. | GAP (G2) |
| 2-extra `claude -c` after the d2607b4e job row was removed | continue the newest stowaway | It continued **d2607b4e** interactively (`source resume`) and showed as a stowaway. So `-c` can pick any transcript in the folder whose job row is gone. | OK |

### Case 4: interactive leads (commodore of `NIMITZ_E2E_BH/SQ`)

`nz start --plan` (first launch) gives `claude --name [NIMITZ_E2E_BH.SQ]-COMMODORE --settings '{"env":{...commodore labels}}'`,
**without `-c`**, although cc-control-model.md §3.1 says `claude -c`. I ran that command through `cl` in tmux (`proj`'d folder).

| case | expected | observed | verdict |
| --- | --- | --- | --- |
| 4-start the exact plan command, by hand | the commodore is live and bound in the registry | Session **14f7b4aa**, interactive, labelled, SessionStart `startup`. `nz status` shows the commodore Idle "found by labels". The registry **never gets the commodore** (identities: only S1), and the pending files stay. `nz start SQ --plan` has no Bind step. The warning "the registry binds it later" is not implemented anywhere (`run.rs:146`). The only binding happens inside `open_step` within `NIMITZ_BIND_TIMEOUT`. | GAP (G1) |
| 4b a second commodore with `claude -c` while the first is live | refused, or flagged as a duplicate | Claude Code **opened the same session id 14f7b4aa in a second process** (pid 757495, `source resume`, `labels {}`). There are two session files and two live rows with one sessionId, and **no copy**. nimitz merges both rows into one session id and takes labels from the newest SessionStart (`{}`). Result: the commodore is reported **Stopped** ("no session row; a transcript in its folder"), the live labelled commodore's id is listed as a **stowaway**, and the commodore gets ERROR `name.taken` with fix "nimitz stop 14f7b4aa" (which fails for an interactive session). There is no `dup.live`. The transcript **branches**: the turns from both processes share parentUuid 68ebcf48, so a later resume keeps only one branch. `nz start SQ` refuses on `name.taken`, so no third copy starts. | **BREAK (G6)** |
| 4b-after the duplicate `/exit`s, the original keeps running | the original is recognised again | It is still wrong: the real labelled commodore (pid 747465) is still shown as a **stowaway with no labels**, the commodore is still Stopped plus `name.taken`, and the suggested fix is to stop and rm the real commodore. The labels come from the newest SessionStart per session id, and there is no registry binding to fall back on. | **BREAK (G6)** |
| 4c plain `claude` in the commodore folder | a stowaway | Session **e04181c2**, name `sq-21`. A stowaway (in NIMITZ_HOME scope), with the fix "nimitz stop" (fails for interactive). `nz adopt e04181c2 SQ --plan` would bind it with no warning that the identity has another live labelled session. | OK (fix text G4) |
| 4a kill the commodore's tmux window (terminal closed) | the commodore is Stopped and resumable to the **same** session | The process died (SIGHUP), SessionEnd `reason other` with labels, the session file was removed, and there is no job row (interactive). nimitz: Stopped "a transcript in its folder (continue with -c)". The recovery plan is `claude -c --name ...COMMODORE --settings ...`, which has no session id. | GAP → BREAK (G7) |
| 4a-recover I made the 4c stowaway's transcript the newest (resume it, "reply ok", `/exit`), then ran nimitz's planned `claude -c --name ... --settings ...` | continue 14f7b4aa | `-c` continued **e04181c2** (the stowaway), renamed it `[NIMITZ_E2E_BH.SQ]-COMMODORE` and gave it the commodore labels. nimitz shows the commodore Idle, `nz check SQ` reports **no errors**, and the real history (14f7b4aa) is orphaned with no trace in the registry. | **BREAK (G7)** |
| admiral | | Not run separately. It uses the same code path as the commodore (`interactive(env,id,mode,true)` = `-c`, the same open_step and binding), so the same results are expected. | not run |

## 2. Gaps

**G1. Interactive leads started outside a nimitz pane never bind (med-high).**
- Trigger: the user runs the lead's command by hand, or the pane's SessionStart arrives after `NIMITZ_BIND_TIMEOUT`, e.g. a trust prompt.
- What breaks: the registry has no `current` or lineage for the lead. Status finds it only through the labels in its newest SessionStart, so any later unlabelled SessionStart on that id (G6) hides it, and recovery cannot use `--resume <id>` (G7).
- Evidence: registry.json after 4-start lists only S1. `run.rs:141-149` promises a later binding that nothing performs (grep shows `lineage.pending` only in run.rs).
- Repro: `nz start F/SQ` with a fake or slow mux, run the planned command by hand, then `cat $STATE/registry.json`.

**G2. A by-hand fork is not linked to the member (low-med).**
- Trigger: `claude --resume <member sid> --fork-session`, interactive or `--bg`.
- What breaks: it is classified as a stowaway, never a copy row. The interactive fork keeps the member's exact name, so `name.taken` fires and SendMessage to that name is ambiguous. The copy-by-fork rule (`snapshot.rs` `by_fork`: empty name plus `forkedFrom`/`parentSessionId`/...) never fires on 2.1.292: bg forks get an auto name, and neither the job state nor the session file records the parent.
- Evidence: 5e5666af and d2607b4e above; job state for d2607b4e has no fork key.
- Repro: `cd ship && claude --bg --resume <sid> --fork-session "x"; nimitz status`.

**G3. An unlabelled interactive resume of a ship (`--resume <id>` or `-c`, ship stopped) leads to a dead-end fix (med).**
- Trigger: the user resumes a stopped ship by hand in a terminal.
- What breaks: the member runs as `kind interactive` with no labels and no rules. The SessionStart hook returns no rules (`rules: null`), so the ship runs without its role brief. `nz check` says to relaunch, but relaunch refuses with "live with no job id; cannot stop it (session.none)". The cause text "resumed with flags, or legacy" is also wrong, and nothing flags "a ship running interactive". Once it exits, `nz start` repairs it cleanly (same id, history kept).
- Repro: `nz stop F/SQ/S; cd ship && claude --resume <sid>`, then `nimitz relaunch F/SQ/S --plan`.

**G4. Wrong fix text for interactive sessions (low-med).**
- Trigger: any stowaway or name holder that is interactive (2a, 2e, 4b, 4c).
- What breaks: the fix "nimitz stop <sid>, then nimitz rm <sid>" fails: `nz stop` of an interactive session gives `session.none`, "not a live background session", and an interactive session has no job row to rm. In 4b the suggested target is the **real commodore**.
- Repro: run `claude` in a member folder, then `nimitz stop <its sid>`.

**G5. Agent view spawns a phantom stowaway in a member folder (low).**
- Trigger: `/exit` from `claude attach` (or `claude --resume <live id>`, which becomes an attach), or `claude agents`, inside a member folder.
- What breaks: the daemon claims a `bg-spare` process as a new unlabelled bg session in that cwd (5fdc2aea, `agent_type: claude`), which shows as a live stowaway until the view closes. A user who follows the stowaway fix at that moment kills the agent view's draft. The view also lists the whole machine's live fleet, and Ctrl-C does not leave it.
- Repro: `cd ship && claude --resume <live ship sid>`, type `/exit`, then run `nimitz status`.

**G6. Two live processes on one interactive session id (high).**
- Trigger: `claude -c` (or `claude --resume <lead sid>`) in a lead's folder while the lead is live.
- What breaks: Claude Code does not copy or refuse; it opens the same id twice. nimitz collapses the rows by session id and takes labels from the newest SessionStart. As a result:
  1. The live lead is shown Stopped and its own session shows as a stowaway, with a fix that targets it.
  2. The state persists after the duplicate exits.
  3. There is no `dup.live`.
  4. The transcript branches (two leaves under one parentUuid), so one branch of work is silently lost at the next resume.
- Repro: start a commodore, then in its folder in another terminal run `claude -c` and `nimitz status`.

**G7. Lead recovery uses `-c`, which picks the newest transcript in the folder, not the identity's session (high).**
- Trigger: a lead stops (terminal closed or `/exit`), and any other conversation in the lead's folder is newer: a by-hand `claude`, a `--resume` of an old one, a fork.
- What breaks: `nz start` (sessions.rs:519 and 548 always use `interactive(.., true)` = `-c`) continues the wrong conversation, renames it to the lead name and gives it the lead labels. nimitz shows it healthy and `nz check` is clean. The real history is orphaned with no record, because of G1.
- Evidence: 4a-recover, where e04181c2 became the COMMODORE.
- Repro: commodore 14f7b4aa stops; `claude` in its folder, one prompt, `/exit`; then run nimitz's planned `claude -c --name ...COMMODORE --settings ...`.

**Side finding (outside this area, worth a look).** A nimitz on a separate NIMITZ_HOME/NIMITZ_STATE reports every labelled session on the machine that it does not declare as `config.invalid`, with the fix "nimitz rm <id>", including the live NIMITZ_DEV session 9aacf927 and MAIN/LUX/DEV 0abc1234. Following it would remove another fleet's live job row. Labels evidently come from job rows (`respawnFlags` `--settings`), which carry no NIMITZ_HOME scope check.

## 3. Notes for the design

- **Bind leads from the SessionStart hook itself**, not in `open_step`. A labelled SessionStart with role admiral or commodore and an empty or different registry `current` should bind (or append lineage), with `via: hook`. This fixes G1 and makes G6 and G7 recoverable.
- **Resume leads by id, not with `-c`**: `claude --resume <registry current> --name ... --settings ...`. On 2.1.292 an interactive `--resume <id>` keeps the id (seen in 2c-stopped and 4b), and the new `--settings` labels arrive in SessionStart. Fall back to `-c` only when the lineage is empty, and then show which transcript it will pick.
- **Detect two processes on one id.** The session files are per pid, and two files with the same `sessionId` and both pids alive should give a `dup.live` ERROR. Never merge them into one row. Labels should be per **pid** (the SessionStart hook line already carries `pid`), not per session id. The labelled pid is the member; the unlabelled pid on the same id is the intruder.
- **"Ship running interactive."** The session file `kind: interactive` (or `jobId: null`) on a bg-role member, combined with SessionStart `labels {}` and `source resume`, identifies a by-hand resume. Give it a distinct code and the fix "exit it in its terminal (session file `tmux` field shows where: `bh:@0.%0`), then nimitz start". The session file carries `tmux` (and probably herdr) pane coordinates, so nimitz can point at the exact terminal.
- **Forks.** SessionStart `source: fork` is a clean signal. The fork's transcript shares message uuids with the parent's, so the first `uuid` of the fork's transcript is found in a lineage transcript. Use that to classify the fork as a copy of the member, or at least name the parent in the stowaway line.
- **The agent-view spare.** SessionStart `agent_type: "claude"` with no job row and a process `claude bg-spare` means a pre-warmed draft. Ignore it, or show it as transient, for about 1 minute.
- **Fix text** should depend on `kind`: interactive means "exit it in its terminal"; bg means stop and rm.
- Doc drift: cc-control-model.md §3.1 shows the commodore start with `claude -c`; the code uses no `-c` on first launch and `-c` on every later start.

## 4. Cleanup

- Removed through `nz stop`/`nz rm`: 4a379db3 and d2607b4e. `cl stop` + `cl rm`: 528aba20. 5fdc2aea (spare) left no job row. All interactive sessions (b3b04dba, 528aba20-interactive, 5e5666af, d2607b4e-interactive, 14f7b4aa x2, e04181c2 x2) exited with `/exit`, or with the window kill for 14f7b4aa's first process. The agent-view client 681717 was killed.
- Verified: `claude agents --json --all` shows no row with a cwd under the world, and no session file refers to it.
- Killed the tmux server `map-byhand-649485`. Removed `$MAP` with `command rm -rf`.
- Transcript folders left for the user:
  - `~/.claude/projects/-home-user--cache-nimitz-e2e-nimitz-e2e-map-byhand-649485-ship1`
  - `~/.claude/projects/-home-user--cache-nimitz-e2e-nimitz-e2e-map-byhand-649485-nimitz-nimitz-e2e-bh-squadrons-sq`
