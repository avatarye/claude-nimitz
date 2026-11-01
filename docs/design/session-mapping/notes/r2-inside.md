# Area `inside` (round 2): changes from inside a member session and from the agent view

Claude Code **2.1.292**, nimitz **0.3.1**, 2026-10-07. World `$MAP=~/.cache/nimitz-e2e/nimitz-e2e-r2-inside` (machine
label `r2in`), fleets **E2E3** (admiral + squadron SQ: commodore, ship S1 in `$MAP/ship1`, tender ~t1) and **E2E31**
(SQ: a *background* commodore + tender ~t1, for the addendum). Leads ran in the private tmux `r2-inside` with the exact
`nz start --plan` commands; ship and tender were attached with `claude attach <id>` in tmux. Model: haiku is restricted
by the org, every turn ran on Opus 5.5 (about 14 turns in all). Hook log kept at `$R2/inside-events.jsonl` (147 lines),
registry at `$R2/inside-registry.json`.

Ids I created (all stopped and `claude rm`'d, see CLEANUP): admiral a05a8e1b -> 7d7f30d8 -> a7e2a093 (+ bg copies
6e42c621, 0770ba11); commodore c1c2d31b -> 5fc3215b -> 94daea27 -> 10f37412 (+ fork 738e0de4, draft spare c8cd7003);
ship job 9795fb64 (sessions 9795fb64, dfede68c, 73ce98f1; phantom spares 178c24f4, 071d9f75); tender job bb6fa8bf
(859edf47); E2E31 bg commodore job 031658b2, tender 2bad5d90; agent-view dispatch: not run (see NEW PATHS).

Vocabulary of the evidence columns: `SS`/`SE` = SessionStart/SessionEnd hook line with its `source`/`reason`;
"labels" = the NIMITZ_* env in the hook line; "row" = `claude agents --json --all`; "state.json" = the job file.

## 1. COVERAGE

| id | path | role | via | Claude Code does | nimitz shows | recovery | verdict | evidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| R1:3.0 (unchanged) | start by hand with the plan command | admiral, commodore | by-hand | interactive, labelled, SS startup | status finds both by labels; **registry gets no entry** | n/a | GAP (low) | registry.json listed only S1 and ~t1 after both leads were up |
| R1:3.0b (new) | start by `nz start` with a slow pane: `NZ_BIND=120 nz start E2E3/SQ --yes` while the plan command is run by hand within the window | commodore, admiral | nimitz | same | `bound 94daea27`, lineage `launch`; later `relaunch` bound 7d7f30d8 the same way | binds | OK | start-sq.log `opened in herdr pane w1:p1; bound 94daea27`; relaunch-adm.log `bound 7d7f30d8` |
| R1:3.1 (unchanged) | `/clear` (unbound lead) | admiral, commodore | by-hand | same pid, SE clear + SS clear with labels, name kept; old id with no turn leaves no transcript | status follows the new id by labels | n/a | OK | admiral a05a8e1b -> 7d7f30d8 (pid 984908); commodore c1c2d31b -> 94daea27; admiral folder held only 7d7f30d8.jsonl |
| R1:3.1b (changed) | `/clear` in a commodore nimitz **has bound** | commodore | nimitz | same as 3.1: 94daea27 -> 10f37412 | status **Idle 10f37412** (labels win), `check`: no errors; registry current stays 94daea27, lineage not updated; `relaunch` refuses (live), `signal` says mux.nopane | `/resume <bound id>` restores the bound id | GAP (low) | round 1 saw BREAK only because a stale bg fork row existed (3.5); with a clean registry the live labelled session wins |
| R1:3.1c (unchanged) | `/clear` in an attached ship | ship | nimitz | same job 9795fb64, same pid; SE clear/SS clear with labels; session -> 73ce98f1; state.json `sessionId` stays 9795fb64, `resumeSessionId` = 73ce98f1; row shows 73ce98f1 while live and 9795fb64 once stopped | **Stopped** "job row 9795fb64 with no live pid" + copy.rows; `check` "no errors"; `start` refuses dup.live with fix "nimitz stop 9795fb64 # or nimitz adopt 73ce98f1"; **`adopt` refuses** "not a stowaway"; `nz stop` refuses | `claude stop` by hand, then `nz start` = native resume: CC continues 73ce98f1 (correct) but nimitz says **lineage.broken "a copy; flags on the resume?"** and "not verified"; `nz relaunch` = rm + `--bg --resume 9795fb64` with flags: resumes the **pre-clear** conversation, 73ce98f1 orphaned silently | BREAK | SS resume 73ce98f1 pid 1242692 after `nz start`; SS resume 9795fb64 pid 1263322 after `nz relaunch`; registry lineage [launch, relaunch] all 9795fb64 |
| 3.1d (new) | `/clear` in an attached tender | tender | nimitz | same job bb6fa8bf; bb6fa8bf -> 859edf47; `resumeSessionId` follows | Stopped + copy.rows, check no errors; **`nz stop E2E3/SQ/~t1`, `nz stop bb6fa8bf`, `nz stop 859edf47` all refuse** (session.none / session.unknown) | only `claude stop bb6fa8bf` | BREAK | SE clear bb6fa8bf / SS clear 859edf47 pid 988265 |
| R1:3.2 (unchanged) | `/compact` | commodore | by-hand | same id, SubagentStop then SS compact with labels | unchanged | n/a | OK | c1c2d31b pid 984921; one turn of cost |
| R1:3.3 (unchanged) | `/resume <own other id>` | commodore, admiral, ship | by-hand | same pid; SE resume (old) + SS resume (new) with labels; **the name stored in the resumed transcript is restored** ("Session renamed to: [E2E3.SQ]-COMMODORE") | status follows by labels; a bound lead resumed to its bound id is clean again | the one-line fix for 3.1b and 3.4-branch | OK | commodore 5fc3215b -> c1c2d31b and 10f37412 -> 94daea27; ship dfede68c -> 9795fb64 (Idle again) |
| R1:3.4 (unchanged) | `/rename X` | commodore | by-hand | no hook; session file name, nameSource user, formerNames | name.drift ERROR, fix "relaunch" (refuses for a live interactive) | `/rename <expected>` clears it; nimitz never says so | GAP (low) | E2E3-RENAMED on c1c2d31b |
| 3.4b (new) | `/rename X` in an attached ship, then `/rename` back | ship | nimitz | state.json `name` + `nameSource user`; **`--name` is dropped from `respawnFlags` and never comes back** after renaming back | name.drift while renamed; nothing after renaming back | `nz relaunch` restores the flags | GAP (low) | respawnFlags = [`--settings`, ...] only, after both renames |
| 3.4c (new) | `/branch <name>` | commodore | by-hand | same pid, **new id** 5fc3215b, SE resume (old) + SS **fork** (new) with labels; the branch name becomes the session name; formerNames keeps the stale id | name.drift "b1"; status follows the new id | `/resume <old id>` returns and restores the name | GAP (low, unbound) / same class as 3.1b when bound | SS fork 5fc3215b pid 984921 |
| 3.4d (new) | `/branch <name>` in an attached ship | ship | nimitz | same job; session -> dfede68c, SS fork with labels; **state.json `sessionId` AND `resumeSessionId` stay 9795fb64** (a stop/start would resume the pre-branch conversation), state.json `name` = "sb1" (nameSource user, kept after `/resume` back; the agent view and the stopped row show "sb1") | Stopped + copy.rows (same as 3.1c); `check` "no errors"; after `/resume 9795fb64`: Idle, no drift (session file wins) | `/resume <job session id>` | BREAK (same class as 3.1c) | row sessionId dfede68c while live; `agents` row name "sb1" once stopped |
| R1:3.5 (unchanged) | `←` on an empty prompt | commodore (bound) | by-hand | daemon `claimed-spare c8cd7003 (spare)` + `spawned 738e0de4 (slash)`; 738e0de4 = **fork** job (SS fork, labels, name kept, `interactiveLineage:true`, `respawnFlags` = labels + `--permission-mode auto`); c8cd7003 = labelled draft spare (`spare:true`, SS startup, agent_type claude); interactive session file keeps 94daea27 + `parkedJobId:738e0de4`; the interactive row disappears from `agents --json` | Idle 94daea27 (registry current wins this time) + **dup.live (3)** + copy.rows; **blocks `nz start` of the ship** (parents first) | see 3.5b/3.6 | BREAK | `nz start E2E3/SQ/S1 --yes` refused with the commodore's dup.live |
| R1:3.5b (changed) | return from that agent view | commodore | by-hand | **Esc, Esc** returns to the conversation (docs v2.1.218); the terminal is then an *attach client of the fork*: `/exit` goes back to the agent view, Esc and Ctrl+C twice no longer leave it once the fork row was stopped; `kill -TERM <host pid>` ends it: SE other for 94daea27 and the spare, "Resume this session with claude --resume" printed | after the kill: **Stopped on the stale fork row 738e0de4 + role.background ERROR** (5c-2 class) | `nz relaunch E2E3/SQ` is still offered: `claude -c` continued **738e0de4** (the fork's id, full history) and bound it as current, lineage [94daea27 launch, 738e0de4 launch]; "same session id" in the plan text is false; sweep then "nothing to do" | BREAK (the identity's id changes silently) | SS resume 738e0de4 pid 1244896; registry E2E3/SQ current 738e0de4 |
| R1:3.6 (unchanged) | recovery summary | commodore | nimitz | | | `nz stop <fork>` + end the host + `nz relaunch` works, but the commodore continues under the fork's id | GAP | lineage never gets clear/resume/branch ids |
| 3.7 (new) | `/background` (`/bg`) | admiral | by-hand | **SE prompt_input_exit** for 7d7f30d8, the interactive process exits to the shell; new bg job 0770ba11 (SS fork, labels, name kept, interactiveLineage) | Idle **0770ba11** + **role.background ERROR**, fix "nimitz open E2E3" which fails (mux.nopane); `relaunch` refuses "is live; end it in its pane" | `nz stop 0770ba11`, then `nz relaunch E2E3` (`claude -c`) continued **7d7f30d8** (the admiral folder held only that transcript; a /background copy with no turn writes none), bound; `nz sweep E2E3` removed 0770ba11 and 6e42c621 | BREAK (needs two by-hand steps nimitz does not suggest) | relaunch-adm.log; SS resume 7d7f30d8 pid 1194592 |
| 3.8 (new) | `/fork` (no prompt) | admiral | by-hand | with no turn: "Nothing to fork yet"; after one turn: bg job 6e42c621 named "[E2E3]-ADMIRAL ⑂" (nameSource auto), SS fork **with the labels**, interactiveLineage, state blocked "waiting for a prompt"; the admiral keeps running | **picks the copy as the admiral's session**: Idle 6e42c621 + dup.live + name.drift "[E2E3]-ADMIRAL ⑂", fix "stop <short>"/"relaunch" | `nz stop 6e42c621` -> Idle 7d7f30d8 + copy.rows -> `nz sweep` drops it | GAP (med): a labelled copy is taken for the member until stopped | SS fork 6e42c621 pid 1055676 labels NIMITZ_ID=E2E3 |
| 3.9 (new) | `/subtask <task>` | commodore | by-hand | forked **subagent**: SubagentStart/Stop with agent_type `fork`, agent_id `areply-ok-...`, under the lead's pid and labels; no row, no session file | nothing (correct) | n/a | OK | events: SubagentStart fork 94daea27 pid 984921 |
| 3.10 (new) | agent team: lead spawns teammate `tm1` (CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1 in the member's `.claude/settings.json`) | commodore (interactive) | by-hand | teammate = **in-process subagent**: SubagentStart/Stop with agent_type `tm1`, agent_id `atm1-...`, lead's pid and labels; transcript `projects/<sq>/94daea27/subagents/agent-atm1-*.jsonl` + `.meta.json` (`taskKind: in_process_teammate`, `teamName: session-c1c2d31b`); **team dir `~/.claude/teams/session-<startup id>`** (c1c2d31b, created at session startup, before any teammate, keeps that name across `/clear`; removed at `/exit`), inboxes `team-lead.json`, `tm1.json`; no row, session file or job; no TeammateIdle/TaskCreated lines (the plugin does not hook them) | nothing for crew (correct) | n/a | OK | teams/session-c1c2d31b/config.json members = team-lead only; `ls ~/.claude/teams` had no session-94daea27 |
| 3.11 (new) | `/exit` in an attached ship | ship | nimitz | **detaches into the machine-wide agent view**; the view claims a pre-warmed spare as "new session" in the ship folder (178c24f4, unlabelled, SS startup, agent_type claude) until the view closes (SE other) | the spare is a live **stowaway** with fix "nimitz stop/rm" while the view is open | Esc (twice) / `/exit` in the dispatch input closes it | GAP (low, transient; R1 byhand G5) | daemon `claimed-spare 178c24f4 (spare)`, `settled 178c24f4 (killed)` |
| 3.12 (new) | `←` in an attached ship | ship | nimitz | agent view + phantom spare 071d9f75 as in 3.11; **Esc, Esc goes to the shell**, not back to the ship (docs' "returns to that conversation" is for a session backgrounded with ←) | same as 3.11 | re-attach | GAP (low) | SS startup 071d9f75 / SE other |
| 3.13 (new) | Ctrl+Z in an attached ship | ship | nimitz | back to the shell; no hook line; ship keeps running | unchanged | n/a | OK | row pid 987890 unchanged |
| 3.14 (new) | `/stop` in an attached ship | ship | nimitz | **SE prompt_input_exit** with labels; row pid null; row name becomes the state.json name ("sb1"); the terminal lands in the agent view | Stopped, no reason worth the name; the drift in the row name is not flagged | `claude attach` wakes it (SS resume, new pid 1150081, prompt bar "sb1") | OK / GAP (low: name drift invisible once stopped) | |
| 3.15 (new) | `/rewind` -> "/resume <id> (previous session)" after `/clear` | admiral | by-hand | SE resume (cleared id) + SS resume (pre-clear id), same pid | follows by labels | the second way to undo a `/clear` | OK | a7e2a093 -> 7d7f30d8 pid 1194592 |
| 3.16 (new) | `claude attach` to a finished tender | tender | by-hand | plain attach (no SS); a "done" job keeps its process idle until retire | Idle | n/a | OK | pid 988265 alive 18 min after "done" |
| AV-1 | agent view: pin my row (Ctrl+T) | bg commodore E2E31 | by-hand | row moves to "Pinned"; **pin stored in `~/.claude/jobs/pins.json`** (array of job short ids); nothing in state.json, roster or the session file; `agents --json` has no pinned field; **`claude rm` leaves the id in pins.json** | nothing | n/a | OK (CC: stale pin entry after rm) | pins.json = ["f7dfa14c","031658b2"] before and after rm |
| AV-2 | agent view: rename my row (Ctrl+R) | bg commodore E2E31 | by-hand | prompt is prefilled with the current name (typed text appends); writes state.json `name`/`nameSource user` and the session file; **drops `--name` from respawnFlags** | name.drift ERROR, fix relaunch (refuses: live/role.background) | Ctrl+R again with the exact name (Ctrl+U clears the field) | GAP (low; fix text) | state.json name "[E2E31.SQ]-COMMODOREE2E31-AVRENAMED" |
| AV-3 | agent view: delete my row (Ctrl+X twice) | tender E2E31 | by-hand | job dir removed, transcript kept, SE other | Stopped "no job row; its transcript is there" | `nz start` (resume by id, per round-1 cli 1c) | OK | 2bad5d90 |
| AV-4 | agent view: open my row (Enter, cursor proven by capture-pane -e) | bg commodore E2E31 | by-hand | attaches; no hook line; `/exit` returns to the view | unchanged | n/a | OK | |
| AV-5 | agent view: dispatch a new session | - | by-hand | NOT RUN: typed text through tmux `send-keys` did not land in the dispatch input (it opened a secondary "enter to create" box; safe to abort, so no row was created) | | | NOT RUN (input interception) | |
| AV-6 | agent view: filter `n:<text>` | - | by-hand | works right after `claude agents` opens; the cursor then sits on the first matching row | | | OK (the safe way to select your own row) | |

Q1 (addendum, my part): **the job id is stable across `/clear` and `/branch` inside an attached background session**; only
the session id moves. Ship `/branch`: job 9795fb64, session 9795fb64 -> dfede68c, state.json `sessionId` 9795fb64,
`resumeSessionId` 9795fb64 (did not follow). Ship `/clear`: job 9795fb64, session 9795fb64 -> 73ce98f1,
`resumeSessionId` 73ce98f1 (followed). Tender `/clear`: job bb6fa8bf, session bb6fa8bf -> 859edf47, `resumeSessionId`
859edf47. The `agents --json` row shows the live session id while the process runs and the state.json `sessionId`
once it is stopped.

## 2. NEW PATHS

Slash commands / keys that create, change, end, rename, background or fork a session (from `/help`, the commands and
agent-view docs, 2.1.292), with what I did:
- `/clear` (aliases `/reset`, `/new`, `/clear <name>` labels the old one), `/compact`, `/resume` (`/continue`;
  resuming a *running* bg session now moves your conversation to the background and attaches, since v2.1.285: not run),
  `/rename`, `/branch <name>` (SS fork, same pid), `/fork [prompt]` (bg copy with labels), `/subtask` (forked
  subagent), `/background`/`/bg` (exits the interactive, new bg job), `←` (same as /bg but the host stays as an attach
  client), `/exit`/`/quit` (interactive: SE prompt_input_exit; attached bg: detach to the agent view), `/stop` (attached
  bg: stop + agent view), Ctrl+Z (attached: detach to shell), `/rewind` (`/checkpoint`, `/undo`; its
  "previous session" entry resumes the pre-/clear id), Ctrl+C twice (exit; also the only exit from a ←-view with a
  live return target). Not run: `/teleport` (cloud), `/batch` (worktree subagents), `/btw` (side question, no session
  change by docs), `/loop`, `/remote-control`, `/resume` into a running bg session.
- Agent view keys: Ctrl+T pin (pins.json), Ctrl+R rename, Ctrl+X stop / twice delete, Enter attach, Space peek,
  Ctrl+S grouping, Ctrl+F or `n:` filter, `a:`/`s:`/`o:` filters, `/resume` in the dispatch input (picker of the
  launch dir's past sessions, resumes as bg: not run), `/model` in the dispatch input, `@dir` dispatch, `! cmd` shell
  job, Shift+arrows reorder, Esc (clear input / quit; after ← it returns to the conversation).
- Hazards for nimitz users: `/exit`, `/stop` and `←` from an attached member all drop you into the machine-wide view
  with the cursor on an arbitrary row (the live fleet is listed there). The view claims a labelled-or-not spare as
  "new session" in the member's folder while open.
- **Cross-world hazard (NEW, med)**: `nz status` on this test NIMITZ_HOME printed `config.invalid` for the live fleet's
  labelled sessions (`HOME/GITHUB/NIMITZ_DEV` 9aacf927, `MAIN/LUX/DEV` 0abc1234) with fix **`nimitz rm 9aacf927`**. The
  labels carry `NIMITZ_HOME`; nimitz should scope "no config declares" to sessions whose NIMITZ_HOME label is its own.
- Tender prompt design: a tender started with the prompt "reply ok" used SendMessage to report "ok" to its commodore
  (an extra commodore turn), and later blocked "couldn't reach commodore" once the commodore's session changed.

## 3. GAPS

- **inside-G1 (BREAK, high) /clear or /branch in an attached ship or tender is read as "stopped + a copy"** (R1 gap 1/2,
  unchanged). Trigger: attach, `/clear` (or `/branch x`). Breaks: status Stopped while live; `check` says no errors;
  `nz stop` refuses by id, by session and by identity (3.1d); `adopt` refuses although `nz start` suggests it; `nz start`
  after a by-hand `claude stop` reports lineage.broken on a correct resume; `nz relaunch` resumes the pre-clear
  conversation (silent rollback; /branch leaves `resumeSessionId` at the old id so even Claude Code's own stop/start
  rolls back). Repro: `nz start F/SQ/S1 --yes; tmux: claude attach <id>; /clear; nz status; nz stop F/SQ/S1 --plan;
  claude stop <id>; nz start F/SQ/S1 --yes` (read the SessionStart id vs the error), `claude stop <id>; nz relaunch
  F/SQ/S1 --yes` (SessionStart resume = old id).
- **inside-G2 (BREAK, high) `←` and `/background` on a lead** (R1 gap 3/4). Trigger: `←` on an empty prompt (host stays)
  or `/background` (host exits). Breaks: a labelled fork job plus (for ←) a labelled draft spare; dup.live blocks every
  `start` under the lead; after the host dies, Stopped on the stale fork row + role.background; `relaunch` continues the
  fork's id and calls it "same session id". `open` fails (mux.nopane), the suggested fixes do not repair it. Repro:
  commodore in tmux, `send-keys Left`, `nz status --json`, `nz start F/SQ/S1 --plan`; then `nz stop <fork>`, kill the
  host pid, `nz relaunch F/SQ --yes` with the pane command run by hand, read the bound id.
- **inside-G3 (GAP, med) a `/fork` copy with the member's labels is taken for the member** (3.8). Trigger: `/fork` after
  one turn. Breaks: status reports the copy's id, dup.live and name.drift against the real lead; fix text points at
  `stop <short>`/`relaunch`. Repro: admiral, one turn, `/fork`, `nz status`.
- **inside-G4 (GAP, low) names drift through rename surfaces nimitz does not read** (3.4b, 3.4d, 3.14, AV-2): `/rename`
  or Ctrl+R writes state.json `name` and drops `--name` from `respawnFlags`; `/branch x` names the session "x"; once a
  ship is stopped the row carries the state.json name and nimitz flags nothing; the fix text is always "relaunch"
  although `/rename <expected>` (live interactive) or Ctrl+R (bg) is the one-line fix. Repro: attach, `/rename X`,
  `/rename <expected>`, `cat jobs/<id>/state.json | jq .respawnFlags`.
- **inside-G5 (GAP, low) registry/lineage never record /clear, /resume, /branch** (3.1b, 3.4c); a bound lead stays bound
  to an id it no longer runs; recovery relies on `claude -c`, which continued the fork's transcript in 3.5b.
- **inside-G6 (GAP, med) config.invalid fix names live sessions of another NIMITZ_HOME** (NEW PATHS). Repro: any second
  nimitz world on a machine with a live fleet, `nz status`.
- **inside-G7 (GAP, low) transient stowaway from the agent view** (3.11/3.12): the view's "new session" spare in the
  member folder is listed as a live stowaway with a stop/rm fix while the view is open.
- Claude Code side (for the record): `claude rm` leaves the pinned id in `jobs/pins.json`; Ctrl+R prefill appends.

## 4. ADDENDUM

**Q3. Can an admiral or commodore be a background job?** Yes, in practice: `claude --bg --name [E2E31.SQ]-COMMODORE
--settings '<labels>' 'reply ok'` in its folder gave job **031658b2** (SS startup with the commodore labels), and with
`claude attach 031658b2` in tmux it could:
- lead an agent team: teammate `tm2` spawned and replied (SubagentStart/Stop agent_type `tm2` under pid 1073345,
  labels E2E31/SQ; `~/.claude/teams/session-031658b2/inboxes/team-lead.json`);
- use AskUserQuestion: the question ("proceed?" yes/no) rendered in the attached terminal and Enter answered it;
- use SendMessage: "ok" reached its tender `[E2E31.SQ]-COMMODORE/t1` (UserPromptSubmit on 2bad5d90). A first attempt
  to message a *foreign* ship `[E2E3.SQ]-S1` was refused by nimitz's own PreToolUse chain-of-command hook ("a commodore
  signals its admiral, its ships and its tenders"), not by Claude Code;
- run nimitz: Bash `nimitz status` worked and the session read its own `role.background` error.
What needs a real interactive session (docs, code.claude.com, read 2026-10-07): agent-teams says "Spawning teammates
also requires an interactive session. In non-interactive mode with the -p flag ... Claude doesn't spawn teammates" - an
*attached* bg session counts as interactive (tested). Split-pane teammates need tmux/iTerm2 (not tested). agent-view:
`/install-github-app`, the `/mcp` settings list and MCP auth "work in a background session while a terminal is attached,
and are refused only when nobody is attached" (v2.1.213); AskUserQuestion/permission prompts park the session under
"Needs input" when nobody is attached. `/status` shows `Session kind: background job · attached|unattended`.
nimitz's view: `role.background` ERROR with fix `nimitz open` (fails: mux.nopane); `relaunch` refuses twice
(session.interactive + role.background). Nothing in nimitz can turn it interactive; the docs' way is `claude --resume
<id>` after a stop.
**Pin and idle retire**: docs (agent-view, Ctrl+T): "Pin a session to the top; keeps its process running while idle"
and "Keeps its process running even when idle (normally stops after ~1 hour)". Changelog v2.1.214: a ←/`/background`
session "left idle with nothing running has its process stopped like any other idle session" - so a pinned bg commodore
is the documented way to avoid the 60-min retire (`bg retire <id>: idle-prompt, idle 61m` in daemon.log). I pinned my
row (cursor proven with `capture-pane -e`, after `n:E2E31`); the 60-min wait was not run. **Pin storage:
`~/.claude/jobs/pins.json`** = `["f7dfa14c","031658b2"]` (job short ids; the only file changed under ~/.claude besides
my parent job's own state.json); not in state.json, roster.json, the session file or `agents --json`.
**Q1** (ship/tender): see the end of section 1.

## 5. CLEANUP and livediff

- Stopped + `claude rm`: 031658b2, 9795fb64, bb6fa8bf, 738e0de4 (6e42c621, 0770ba11 by `nz sweep`; 2bad5d90 by
  Ctrl+X); c8cd7003 had no job dir. Interactive leads ended with `/exit`; the ← host was ended with `kill -TERM`
  (its own pid). tmux server `r2-inside` killed; `$MAP` removed with `command rm -rf`. No process with the world's
  path remains. `jobs/pins.json` still lists 031658b2 (a stale pin; I did not edit the file).
- Transcript folders left under `~/.claude/projects/`: `-home-user--cache-nimitz-e2e-nimitz-e2e-r2-inside-nimitz-e2e3-admiral`,
  `...-nimitz-e2e3-squadrons-sq`, `...-nimitz-e2e31-squadrons-sq`, `...-ship1`. Team/task dirs `~/.claude/tasks/session-*`
  for my ids may remain (Claude Code's own retention).
- livediff: `[OK] live fleet unchanged` after the pin phase and at the end. The only live-fleet file touched during the
  pin diff was `jobs/9aacf927/state.json` (my parent job's own periodic update, not an action of mine).
