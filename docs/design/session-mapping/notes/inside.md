# Area `inside`: Part A cases 3 (inside a member session) and 5 (environment)

Tested on Claude Code **2.1.292** (installed; auto-updated from 2.1.290) and nimitz **0.3.1**, 2026-10-07.
World: `$MAP=~/.cache/nimitz-e2e/nimitz-e2e-map-inside-649964`, fleet `NIMITZ_E2E_IN`, squadron `SQ`, ship `S1`
(folder `$MAP/ship1`). Commodore run by hand in a private tmux (`tmux -L map-inside-649964`) with the exact
`nz start NIMITZ_E2E_IN/SQ --plan` command. Model: project settings ask for haiku, but the org restricts it, so every
turn ran on Opus 5.5 (the banner says so). Prompts were kept to "reply ok".

Note on the brief: `cl claude agents --json --all` runs `claude claude agents ...` (it fails with "unknown option
--json"). The right form is `cl agents --json --all` and `cl attach <id>`.

Ids I created:
- Commodore (interactive pid 655686, then 719848, 731094, 783990): 94097938 (startup), ca0aed5b (/clear),
  2c6384a4 (/clear), ca0aed5b again (/resume), aa26510f (agent-view draft spare), 7e1875a4 (fork made by `←`),
  e73621b0 (/clear after the relaunch).
- Ship: job 9cdceb2d, session 9cdceb2d; 79519302 (/clear while attached).

## 1. Results

### Case 3: inside a member session

| case | expected (what the user wants) | observed (Claude Code facts + nimitz facts) | verdict |
| --- | --- | --- | --- |
| 3.0 commodore started by hand with the exact plan command | The identity binds; the registry records it | Live, labelled, `nz status` finds it by labels (Idle, session 94097938). **The registry gets no entry.** Only `nz start/relaunch` binds an interactive member (`bind_started`); a hook line alone never writes the registry. | GAP (low) |
| 3.1 `/clear` in the interactive commodore | Same identity, new session followed | Same pid. SessionEnd `reason=clear` (old id), SessionStart `source=clear` with the labels kept. The new id is ca0aed5b (later 2c6384a4, then e73621b0). The session file `sessionId` updates; the name is kept. nimitz status follows the new id by labels. Registry: no update. `/clear` before any turn leaves **no transcript** for the old id (94097938 never got a .jsonl). | OK while unbound; see 3.1b |
| 3.1b `/clear` in a commodore that nimitz **has bound** (registry current = 7e1875a4, plus that id's stale bg row from 3.5) | Status follows the live session | Live interactive e73621b0. nimitz keeps registry current 7e1875a4, so status = **Stopped / background / role.background ERROR**, reason "SessionEnd event, reason clear". The live commodore is listed as an "extra job row (copy)". `start` refuses (role.background) and tells you to "nimitz stop e73621b0" (the real commodore) "or adopt". `adopt` refuses ("not a stowaway"). `rm 7e1875a4` refuses (lineage.current). **`relaunch --plan` offers a second `claude -c` while the commodore is live** (a duplicate). Recovery: `/exit`, then `nz relaunch`. `-c` continues the newest transcript (e73621b0) and binds it. | BREAK |
| 3.1c `/clear` inside an **attached background ship** | Same identity, new session followed | Same job 9cdceb2d, same pid 650262. SessionStart `source=clear` with labels; new id 79519302. Session file and `agents --json` say `sessionId=79519302`. jobs/9cdceb2d/state.json keeps `sessionId=9cdceb2d` and sets `resumeSessionId=79519302`. nimitz: S1 **Stopped** ("job row 9cdceb2d with no live pid", "reason clear") while it runs. The live session counts as an extra copy row named "9cdceb2d". `stop` says "not live". `relaunch` refuses with dup.live and "nimitz stop 9cdceb2d" (the real ship). `adopt` refuses. After `claude stop`, the row's `sessionId` goes **back to 9cdceb2d** in `agents --json`. `nz start` runs `--bg --resume 9cdceb2d`: Claude Code continues 79519302 (correct), but nimitz reports **lineage.broken "a copy; flags on the resume?"** (wrong) and stays Stopped. `nz relaunch` (rm, then resume with flags) **resumes the pre-clear 9cdceb2d conversation**: everything after /clear is orphaned, with no issue reported. | BREAK |
| 3.2 `/compact` (commodore) | Same id | Same id ca0aed5b and pid. SubagentStop, then SessionStart `source=compact` with labels. No PreCompact/PostCompact lines (the plugin does not hook them). nimitz is unaffected. Not run on the ship; same mechanism is expected. | OK |
| 3.3 `/resume <id>` to another conversation (commodore; my 2c6384a4 -> ca0aed5b) | Same identity, now on the resumed session | Same pid. SessionEnd `reason=resume` (2c6384a4), SessionStart `source=resume` (ca0aed5b), labels kept, name kept. Status follows by labels. Registry: no update (same class as 3.1b once bound). `/resume <ship id>` from the commodore folder: "Session ... was not found" (`/resume` only looks in the cwd's project), so a cross-identity resume needs two identities in one folder. | OK unbound / GAP bound |
| 3.4 `/rename NEWNAME` (commodore) | Drift flagged; easy fix | No hook fires. Session file `name=NEWNAME`, `nameSource=user`, and `formerNames:[{name:"[..]-COMMODORE", sessionId:"94097938"}]`: the **stale pre-/clear id**. The name survives `/clear` and `/resume`. nimitz: `name.drift` ERROR, fix "nimitz relaunch", but relaunch refuses "end it in its pane first". `/rename [NIMITZ_E2E_IN.SQ]-COMMODORE` clears it at once; nimitz never suggests that. On the attached ship, `/rename SHIPNEW` also writes `jobs/<id>/state.json name` (it survives respawn), while nimitz thought the ship was stopped (3.1c), so no drift was shown. `nz relaunch` restored the name. | GAP (low) |
| 3.5 `←` on an empty prompt (interactive commodore) | Refused, or nimitz shows "moved to background" with the same identity | Claude Code opens the **agent view of the whole machine** (the live fleet is listed and reorders live). Daemon: `bg claimed-spare aa26510f (spare)` and `bg spawned 7e1875a4 (slash)`. **7e1875a4 is a fork**: `--session-id 7e1875a4 --fork-session --resume <ca0aed5b.jsonl>` with the labels as `respawnFlags`, `interactiveLineage:true`, name kept, SessionStart `source=fork`. **aa26510f** is a spare claimed in the commodore folder **with the commodore's labels** (SessionStart startup, NIMITZ_ID=NIMITZ_E2E_IN/SQ, session file `kind:bg, spare:true`, name "aa26510f"): it is the agent view's "new session" draft. The interactive process 655686 stays alive as the agent-view host. Its session file keeps `sessionId=ca0aed5b, kind=interactive` plus `parkedJobId:7e1875a4` (stale). nimitz picks **aa26510f as the commodore's session** (Idle, name.drift "aa26510f"), dup.live (3 live), role.background, and copy.rows that list the interactive ca0aed5b as a "job row". Suggested fixes are wrong: `nimitz stop ca0aed5b...` on an interactive; `relaunch` asks for `/exit` in a pane; `open` says "runs in a terminal outside herdr". | BREAK |
| 3.5b Return from the agent view (Enter on the row) | Back to the interactive session | The terminal is **attached to the bg fork**: the next prompt's UserPromptSubmit came from 7e1875a4 (pid 696047), not from 655686. The interactive session file still says ca0aed5b / idle. `/exit` only detaches back to the agent view. Ctrl+C twice ends the host: SessionEnd `reason=other` for ca0aed5b and the draft aa26510f. The fork 7e1875a4 **keeps running** as a bg session named `[..]-COMMODORE` with labels. nimitz: role.background. `start` refuses even after `nz stop` (role.background on a stopped row). `relaunch` = `claude -c`. It continued 7e1875a4 **interactively with the same id** (history kept), but left job row 7e1875a4 (done) behind. That stale row caused 3.1b and 5c. | BREAK |
| 3.6 Recovery summary | One command repairs | `/exit` + `nz relaunch <commodore>` (with the pane bound) recovers a commodore every time. `nz sweep` then drops the stale fork row once the registry has moved on. Lineage entries are all `via=launch` even for relaunches of interactive members. The lineage never records clear/resume/fork ids (ship S1 lineage = 6 entries, all 9cdceb2d). | GAP (low) |

### Case 5: environment

| case | expected | observed | verdict |
| --- | --- | --- | --- |
| 5a-1 transcript deleted **while the ship is live** (moved aside), then stop/start | Missing history flagged | While live: nothing flagged. On stop, Claude Code writes a **7-line metadata-only stub** (cost-state, custom-title, agent-name, mode, ...; no messages) under the same name. nimitz counts that as "transcript present". On `nz start` (`--bg --resume 9cdceb2d`), Claude Code renames the stub to `9cdceb2d-...orphaned-<ts>-<rand>.jsonl` and starts fresh with **the same id**: SessionStart `source=startup`. nimitz: "[OK] verified", Idle, no issue. History is lost silently. | GAP (high) |
| 5a-2 transcript deleted while the ship is **stopped, job row present** | Flagged before start | status/check: Stopped, **no issue** (the transcript is only checked when there is no row). `nz start`: SessionStart `source=startup`, same id, "[OK] verified". Silent loss. | GAP (high) |
| 5a-3 transcript deleted and **no job row** | Clear error | `lineage.broken`: "current session ... has no job row and no transcript", fix `nimitz start <id> --force` (cold launch). `relaunch`: session.none, use start. Clear, but the older `79519302.jsonl` in the same folder is not offered. | OK |
| 5a-4 transcript **moved** to another project dir (`...-ship1-moved/`), no job row | Found or flagged | nimitz finds it by id in any project dir ("transcript is there (.../ship1-moved/...)") and plans `--bg --resume <id>` with every flag. Claude Code `--resume <id>` also searches across projects: SessionStart `source=resume`, history loaded (the old turn is visible in `claude logs`), and it **keeps appending to the moved file**. An interactive member (`claude -c`) only sees its cwd's project, so a moved commodore transcript would not be continued (inferred; not run). | OK (bg) / GAP inferred (interactive) |
| 5b Claude Code auto-update (not installed; read from daemon.log) | Live workers keep their identity | `binary at ~/.local/bin/claude changed (2.1.290 -> 2.1.292) - self-restarting for upgrade` / `shutting down (cause=upgrade, ... live_workers=2)` / new daemon `bg adopt: adopted=2 respawned=0 dead=0` / `post-takeover prewarm burst - respawned 0/2 stale workers, 2 refused`. Adopted workers keep running the **old binary**: the live worker 9aacf927 has session file `version: 2.1.290` under daemon 2.1.292. Ids, names and labels are unchanged. Simulated respawn (`claude respawn 9cdceb2d`, mine): SessionEnd `other`, then SessionStart with **the same id, labels and name**. The worker command is `--session-id <id> --name ... --settings <labels>`. No registry change is needed. nimitz shows a compat WARN "newer than tested". It shows no per-session "old binary" flag, although the session file `version` gives that. | OK / GAP (low) |
| 5c-1 kill -9 of my **worker** pid (simulated crash/reboot) | Stopped, then resumable | state.json `state=crashed`, session file removed (`agents --json` still said `working` for a while). The daemon's pty host **restarted the worker by itself about 11 s later** with `--session-id 9cdceb2d --name ... --settings <labels>`: same id, labels and name, SessionStart `source=startup`. The source was startup because the transcript had been deleted in 5a, so whether a restart with a transcript says resume is untested. daemon.log has **no line** for this restart. Killing the pty host too (pid 780114 + worker): the daemon re-created the host with the same spare socket and restarted the worker again. **A real reboot cannot be simulated** without stopping the daemon (forbidden: it hosts the live fleet). | OK (identity kept) / NOT SIMULABLE (reboot) |
| 5c-2 kill -9 of my **interactive commodore** pid | Stopped; `start` brings it back | Session file removed, **no SessionEnd** line. nimitz then fell back to the stale bg fork row 7e1875a4 (from 3.5): Stopped + role.background ERROR. That error **blocks `nz up NIMITZ_E2E_IN`, `nz start` of the commodore and `nz start` of the ship S1** (start brings parents up first), even with `--force`. `sweep` does nothing; `rm 7e1875a4` refuses (lineage.current) although the registry current is e73621b0. Only `nz relaunch NIMITZ_E2E_IN/SQ` works (`-c` continued e73621b0, same id). | BREAK |
| 5c-3 idle retire and idle exit (from the log; 60 min wait not run) | Stopped cleanly; same id on next start | Log kinds: `bg retire <id>: idle-prompt, idle 61m`, `settled, idle 61m / 8h`, `stale-spare, idle 61m`. Then `[supervisor] idle 5s with no clients - exiting` (`cause=idle_exit`). In the live events file, the retire of 9aacf927 (2026-10-06 02:18, idle-prompt) left **no SessionEnd line**. The next `nimitz start` resumed the same id (`claimed-spare 9aacf927 (fleet)`, SessionStart resume). nimitz shows Stopped with no reason. | OK / GAP (low) |

## 2. Gaps

1. **Registry current is never moved by a /clear, /resume or fork; nimitz trusts the stale current over a live labelled
   session in the same folder** (3.1b, 3.1c). Severity **high**: a live ship or commodore shows as Stopped, the
   suggested fix stops the real session, and `relaunch` offers a duplicate interactive.
   Evidence: 3.1c status `Stopped ['job row 9cdceb2d with no live pid', 'SessionEnd event, reason clear']` while pid
   650262 was live as 79519302; `nz relaunch NIMITZ_E2E_IN/SQ --plan` offered `claude -c` while e73621b0 was live.
   Repro: `nz start F/SQ/S1 --yes`; `cl attach <short>`; type `/clear`; `nz status`.
2. **Job row identity is keyed by sessionId, not jobId** (3.1c). state.json `sessionId` stays at the launch id;
   `resumeSessionId` and the session file carry the live id; `agents --json` shows the new id while running and the old
   one once stopped. Severity **high**: nimitz splits one job into a "stopped current" and a "live copy" with the same
   short id, and a later `relaunch` resumes the **pre-clear** conversation (silent rollback).
   Repro: as 1, then `cl stop <short>; nz relaunch F/SQ/S1 --yes` and see SessionStart `resume` for the old id.
3. **`←` turns an interactive member into a forked background session plus a labelled draft spare** (3.5, 3.5b).
   Severity **high**: new id (`source=fork`), stale interactive session file (`parkedJobId`), a labelled spare
   (`spare:true`) that nimitz picks as the member's main session, and a stale bg row that later blocks `up`/`start`
   for the whole squadron (5c-2). The wrong fixes (`open`, `relaunch` "end it in its pane", `stop <interactive id>`)
   do not repair it. Repro: interactive commodore in tmux, `tmux send-keys Left`, then `nz status --json`.
4. **A stale bg row of a commodore blocks every start under it** (5c-2): `role.background` is an Error on a
   *stopped* row; `start`, `up` and `start --force` of the ship all refuse; `sweep` is empty; `rm` refuses as
   "current" although the registry current differs. Severity **high** after a reboot. Repro: 3.5, return,
   `nz relaunch` the commodore, then kill its pid; `nz up F --plan`.
5. **Transcript loss is silent** (5a-1, 5a-2). Claude Code starts fresh under the same id (`source=startup`) and
   sets aside a stub as `*.orphaned-*.jsonl`; nimitz says "[OK] verified". Severity **high** for long-lived ships.
   Repro: stop a ship, `mv` its `<sid>.jsonl` aside, `nz start`, read the SessionStart `source`.
6. **Wrong diagnosis on a correct resume after /clear** (3.1c): `lineage.broken "... a copy; flags on the resume?"`
   when Claude Code continued `resumeSessionId`. Severity med (misleads repair).
7. **A commodore run by hand is never bound** (3.0), and lineage records only nimitz launches (`via=launch` for
   relaunches, duplicate entries per resume, no clear/resume/fork ids) (3.6). Severity low/med: lineage cannot answer
   "which conversations did this identity have".
8. **Name drift fix points to an impossible action** (3.4): `relaunch` of a live interactive refuses; the one-line
   fix `/rename <expected>` is not suggested. `formerNames[].sessionId` is stale after /clear. Severity low.
9. **No SessionEnd on idle retire or kill -9** (5c-2, 5c-3). nimitz cannot tell retired, crashed or killed apart, and it
   shows a stale `reason` from an older SessionEnd (e.g. "reason clear" for a crashed commodore). Severity low.
10. **Upgrade leaves workers on the old binary** (5b), with no per-session flag in nimitz. Severity low.
11. **`relaunch` of an interactive uses `claude -c`** (newest transcript in the cwd), not the registry id. Any newer
    conversation in the commodore folder (a tender, a by-hand `claude`, or the fork) wins. Inferred; severity med.

## 3. Notes for the design (signals nimitz can use)

- **SessionStart `source`** is the key signal. `clear` / `resume` / `compact` / `fork` with the same labels and the
  same pid (or the same `jobId`) means "same identity, new session": append it to the lineage with `via=<source>` and
  move `current` (auto-bind from the hook, or on the next `status` under the registry lock). Pair it with SessionEnd
  `reason` (`clear`, `resume`, `prompt_input_exit`, `other`).
- **`source=startup` on a member nimitz just resumed** means the conversation was missing: raise
  `history.lost` (and look for `<sid>.orphaned-*.jsonl` in the project dir as evidence).
- **Join bg rows by `jobId` / daemon short id**, not sessionId. The live id is session file `sessionId` or state.json
  `resumeSessionId`; state.json `sessionId` is only the launch id. Resume a stopped job by `resumeSessionId`.
- **Session file fields**: `parkedJobId` on an interactive = moved to background (identity now = that job);
  `spare:true` = a draft, never a member's session (exclude it, or flag a labelled spare as a would-be impostor);
  `kind` interactive vs bg; `version` vs the daemon version = running on an old binary; `formerNames` = rename
  history (do not trust its `sessionId`).
- **job state.json `interactiveLineage:true`** marks a bg job forked from an interactive (`←`). Offer "close the
  fork and continue it interactively with `--resume <fork id>`", and remove the fork row in the same plan.
- **Prefer a live labelled session in the member's folder over a dead registry current**; demote the dead
  current to history instead of raising role.background or dup.live. Never make a stopped row's role.background an
  Error that blocks parents' children.
- **For interactive relaunch use `--resume <current>`** (registry id) instead of `-c`, once the registry follows
  /clear and /resume.
- **The name**: suggest `/rename <expected>` for a live interactive; for bg, the relaunch already restores it.
- **Retire/crash**: no SessionEnd arrives; use state.json `state` (`crashed`, `done`, `stopped`) for the reason.

## 4. Cleanup

- Stopped and removed: job 9cdceb2d (`claude stop` + `claude rm`), job row 7e1875a4 (`claude rm`). aa26510f's job dir
  was already gone (the draft ended with the agent-view host). The interactive commodore was ended with `/exit`.
  No process with NIMITZ_E2E_IN remains. The pty host did not restart the worker after `claude stop`.
- tmux server `map-inside-649964` killed. `$MAP` and the 54 empty extra `nimitz-e2e-map-inside-*` folders (one per
  sourcing) were removed with `command rm -rf`.
- Env exposure (coordinator's safety update): my interactive commodore runs were started in a tmux server launched
  with `env -i HOME PATH TERM USER SHELL LANG`, so no CLAUDE_CODE_* reached them. `cl` (old version) ran only
  non-interactive `agents`, `stop`, `rm` and `respawn <my id>`, and `nz` launched ships through `claude --bg`. Their
  names were always `[NIMITZ_E2E_IN.SQ]-*`. I checked afterwards: the parent's session file name is still
  `[HOME.GITHUB]-NIMITZ_DEV` with no `formerNames`.
- Transcript folders left for the user to remove:
  - `~/.claude/projects/-home-user--cache-nimitz-e2e-nimitz-e2e-map-inside-649964-nimitz-nimitz-e2e-in-squadrons-sq/`
  - `~/.claude/projects/-home-user--cache-nimitz-e2e-nimitz-e2e-map-inside-649964-ship1/`
  - `~/.claude/projects/-home-user--cache-nimitz-e2e-nimitz-e2e-map-inside-649964-ship1-moved/` (made by case 5a-4)
- Safety note: the agent view (`←`) lists the user's live sessions and reorders them live. One Enter press there landed
  on my own row (checked afterwards), but this is a hazard for any future probe of `←`.
