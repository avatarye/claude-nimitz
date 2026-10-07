# Verify `inside` (round 2): area `v-inside`

Claude Code **2.1.292** (`claude --version`), nimitz **0.3.1**, 2026-10-07. World
`$MAP=~/.cache/nimitz-e2e/nimitz-e2e-r2-v-inside` (machine label `r2vi`), private tmux `r2-v-inside`.
Fleets: **E2E8** (pin test: ships PA, PB), **E2E81** (admiral, commodore SQ, ships S1, S2, tender ~t1),
**E2E82** (an admiral started by hand, never bound, for G3). Every repro ran from scratch in this new world. Model:
haiku is turned into Opus 5.5 by the org; about 9 turns in all.

Per the user's update (tender role to be removed), tender-only parts are marked N/A and were not re-run. The tender
~t1 was started once (before the update arrived) and never touched after that.

Ids I created: admiral E2E81 82633697 (fork 04d188a4, /background copy 55db890f); commodore E2E81/SQ f36d09b3 and a
second unbound start (both ended with /exit before any turn), bound 3151de63, `←` forks a328fbb7 (no turn) and
903de92d (one turn), draft spares 0d001bec and 1eef2f4a, /clear id ec198afa; S1 job 54f2e4f3 (session after /clear
ca16744e); S2 job 0a6ce393 (session after /branch b8df9ff1); agent-view spare efa7e9f9; tender c2561749; admiral E2E82
4ab11d35 (fork e549b536); pin-test ships PA 36fef5ef, PB f59e1f5f.

## Gaps

| gap | verdict | your evidence | notes |
| --- | --- | --- | --- |
| inside-G1 `/clear` or `/branch` in an attached ship reads as "stopped + a copy" | **REPRODUCED** (ship part); tender part (3.1d) **N/A (tender role to be removed)** | S1: `/clear` gave SE clear 54f2e4f3 + SS clear **ca16744e**, same pid 1364154; state.json `sessionId` 54f2e4f3, `resumeSessionId` ca16744e. `nz status`: **stopped** "job row 54f2e4f3 with no live pid" + copy.rows naming 54f2e4f3 itself; `nz check`: "no errors". `nz stop E2E81/SQ/S1` session.none, `nz stop 54f2e4f3` session.none, `nz stop ca16744e` session.unknown; `nz start --plan` dup.live with fix "nimitz stop 54f2e4f3 # or nimitz adopt ca16744e..."; `adopt` "not a stowaway"; `sweep --plan` skips (dup.live), fix "nimitz stop 54f2e4f3". After `claude stop 54f2e4f3`: `nz start --yes` ran `--bg --resume 54f2e4f3`, Claude Code gave **SS resume ca16744e** (correct), nimitz said **lineage.broken** "a copy; flags on the resume?" + "not verified", and status stayed stopped. After a second `claude stop`: `nz relaunch --yes` (rm + `--bg --resume 54f2e4f3` with flags) gave **SS resume 54f2e4f3**, "[OK] verified", Idle: the pre-clear conversation; `ca16744e.jsonl` is left orphaned in the s1 transcript folder. S2 `/branch vb1`: SE resume 0a6ce393 + **SS fork b8df9ff1**, same pid; state.json `sessionId` **and** `resumeSessionId` stay 0a6ce393, name "vb1"/nameSource user; status stopped + copy.rows, check no errors. `claude stop 0a6ce393` then `claude attach 0a6ce393` (Claude Code's own wake): **SS resume 0a6ce393**, prompt bar "vb1": Claude Code itself rolls back the branch. | Every fix nimitz prints for the cleared ship (stop by job id, adopt, sweep, start) refuses or loops back to another of them. |
| inside-G2 `←` and `/background` on a lead | **REPRODUCED**, with one condition on the last step | `←` on the bound commodore 3151de63 (one turn earlier): daemon `claimed-spare a328fbb7 (slash)` + `claimed-spare 0d001bec (spare)`; SS fork a328fbb7 (labels) and SS startup 0d001bec (agent_type claude, labels). Status: Idle 3151de63 + **dup.live (3)** + copy.rows; `nz start E2E81/SQ/S1 --plan` refused with the commodore's dup.live. `nz stop a328fbb7` + `kill -TERM 1359725` (host): status **stopped on the fork row a328fbb7 + role.background**, fix "nimitz open E2E81/SQ"; `open` -> mux.nopane, fix "nimitz start E2E81/SQ"; `start` -> role.background, fix "open" (a loop); `start E2E81/SQ/S1` blocked by the same error. `nz relaunch E2E81/SQ` (`claude -c`, run by hand in the pane): **bound 3151de63** (correct) because the fork had no turn and so no transcript. Second run with the conversation continued in the fork (← then Esc, Esc, one turn: UserPromptSubmit came from **903de92d**, the fork, and the prompt bar read "[E2E81.SQ]-COMMODORE (2)"): stop fork + kill host + relaunch: **bound 903de92d**, SS resume 903de92d, under a plan that says "same session id and history". `/background` on the admiral 82633697: **SE prompt_input_exit**, the process exits to the shell, SS fork **55db890f**; status Idle 55db890f + **role.background**, fix "nimitz open E2E81" -> mux.nopane ("runs in a terminal outside herdr"); `relaunch` refuses "type /exit in its pane" (there is no pane). `nz stop 55db890f` + `nz relaunch E2E81` bound 82633697 again; `nz sweep E2E81` removed 55db890f. | The identity's id moves to the fork **only when a turn happened in the fork** (the explorer's 3.5b path); a bare ← then host kill recovers the right id. NEW: after the recovery the bound id 903de92d still has its job row (state done); sweep keeps it (it is current). A later `/clear` in the live commodore (903de92d -> ec198afa) then gave **stopped on job row 903de92d + role.background** for a live interactive commodore: round 1's 3.1b BREAK comes back through this route. |
| inside-G3 a `/fork` copy with the member's labels is taken for the member | **REPRODUCED** for an unbound lead; **PARTIAL** for a bound one | Unbound admiral E2E82 4ab11d35 (started by hand, no registry entry), one turn, `/fork`: SS fork **e549b536** with labels; status **Idle e549b536** + dup.live + **name.drift "[E2E82]-ADMIRAL ⑂"** (fix relaunch, which refuses "type /exit in its pane") + **role.background** + copy.rows that names the **real admiral 4ab11d35** as the extra row; `nz sweep E2E82 --plan` fix: "nimitz stop 4ab11d35-..." (the real admiral). `nz stop e549b536` -> Idle 4ab11d35 + copy.rows; `nz sweep` removed it. Bound admiral E2E81 82633697, one turn, `/fork`: SS fork 04d188a4 (labels, name "[E2E81]-ADMIRAL ⑂", nameSource auto, interactiveLineage, state blocked): status kept **Idle 82633697**, dup.live "stop <short>" + copy.rows only, no name.drift; but dup.live **blocks `nz start E2E81/SQ/S1`**. `nz stop 04d188a4` + `nz sweep E2E81` cleaned it. | Worse than reported in the unbound case: the role.background error and a sweep fix that names the live real admiral. With a registry binding the copy is not taken for the member, but it still blocks every start under the lead until stopped. |
| inside-G4 names drift through rename surfaces nimitz does not read | **PARTIAL** | `/rename E2E81-RENAMED` in attached S1: state.json name + nameSource user; respawnFlags became `[--settings, {...}, --name, E2E81-RENAMED]`; status **name.drift ERROR, fix "nimitz relaunch"**; `/rename [E2E81.SQ]-S1` cleared it (flags `--name [E2E81.SQ]-S1`). Same on S2 (launch flags `--name vb1 --settings ...` -> `--settings ... --name E2E81-R2` -> `--name [E2E81.SQ]-S2`). `/branch vb1` named the session "vb1" and wrote `--name vb1` into respawnFlags. `/rename E2E81-X` then `claude stop`: the stopped row's name is "E2E81-X", status "stopped", `check` "no errors" (drift invisible). | NOT REPRODUCED: "`--name` is dropped from respawnFlags". On 2.1.292 here `/rename` and `/branch` **rewrite** the `--name` value (and move it to the end); renaming back restores it. Reproduced: the fix text is always relaunch; the drift is invisible once stopped. Ctrl+R in the agent view not re-run (agent view rule 5; `/rename` covers the same write). |
| inside-G5 registry/lineage never record `/clear`, `/resume`, `/branch` | **REPRODUCED** | S1 lineage `[54f2e4f3 launch, 54f2e4f3 relaunch]` after /clear to ca16744e; S2 `[0a6ce393 launch]` after /branch to b8df9ff1; commodore current stays **903de92d** after `/clear` to ec198afa (lineage `[3151de63 launch, 3151de63 launch, 903de92d launch]`: relaunches are recorded as `launch`). Recovery by `claude -c` continued the fork 903de92d when the fork had a turn (see G2). | Also seen: on the very first `nz start E2E81/SQ`, the commodore's SessionStart (f36d09b3) came while nimitz was still binding the admiral, so it was never bound (lineage.pending, then nothing); a later `nz start` said "already opening ... focused it" and did not wait; only `nz start --force` bound it. |
| inside-G6 config.invalid fix names live sessions of another NIMITZ_HOME | **REPRODUCED** | First `nz status` in this world: `[WARN] HOME/GITHUB/NIMITZ_DEV: session 9aacf927... config.invalid` fix **`nimitz rm 9aacf927`**, and `MAIN/LUX/DEV` 0abc1234 fix `nimitz rm 0abc1234`. The live job's label (read from jobs/9aacf927/state.json respawnFlags) is `NIMITZ_HOME=/home/user/.claude-nimitz`, not this world's. Same warning for every other test world's sessions (E2E4/*, E2E41/SQ/S1, E2E9/*). | I did not run any of those fixes. |
| inside-G7 transient stowaway from the agent view | **REPRODUCED** | `/exit` in attached S2 -> the machine-wide agent view; daemon `claimed-spare efa7e9f9 (spare)` in `$MAP/s2`; `nz status` while open: `stowaways: efa7e9f9 idle ... /s2` with fix "`nimitz adopt <session> <id>` or `nimitz rm <session>`". Esc, Esc closed the view: `settled efa7e9f9 (killed)`, stowaway gone. | Fix wording is adopt/rm (the explorer wrote stop/rm). I pressed only Esc in the view. |

## Pin by file

Setup: E2E8 ships PA (job **36fef5ef**) and PB (job **f59e1f5f**) started by `nz start` with the prompt "reply ok"; both
answered at 01:18:31 and were left idle and unattached (no attach, no prompt, nothing typed into their rows).
At 01:19:27 I appended only `36fef5ef` to `~/.claude/jobs/pins.json` with an atomic write (read, add, write a temp file in
the same folder in the same 2-space JSON format, `os.replace` over it). Before: `["f7dfa14c","031658b2"]`; after:
`["f7dfa14c","031658b2","36fef5ef"]`.

Results:
- **B retired, A survived.** daemon.log: `[2026-10-07T02:19:03.072Z] [bg] bg retire f59e1f5f: settled, idle 60m` then
  `bg settled f59e1f5f (done)`. No retire line for 36fef5ef. At 02:21:17 PA's pid 1304058 was still alive (etime
  01:04:01), row `state done, status idle`; PB's row `pid null`. Both had the same idle start within 2 s, so the
  same sweep skipped A.
- **The agent view honours the pin at once**: when it opened at 01:29 (G7), `[E2E8.SQ]-PA` was listed under "Pinned".
- **The daemon kept my entry.** pins.json was rewritten by something else at 02:00:00 (mtime changed, not by me), with
  identical content: my id was kept. It was never dropped while A lived.
- Code check (2.1.292 binary, read only): `retireIfSettled` returns `{retired:false, reason:"pinned"}` when the pin
  set has the job's short id; the listing reads pins.json fresh each time (no cache). Claude Code's own pin writer
  takes a lock (`pins.json.lock`, proper-lockfile style, stale 5 s) before its read-modify-write; my writer did not.
  An outside writer that does not take that lock can lose an update if Ctrl+T runs at the same moment (the atomic
  rename keeps the file whole, but one of the two writes wins). Unpin in Claude Code also deletes
  `jobs/<id>/order`; a by-file unpin does not (I never pinned through the view, so there was none).
- Afterwards (02:21) I removed only `36fef5ef`, the same atomic way: pins.json = `["f7dfa14c","031658b2"]` (the two
  ids I did not add are untouched; `031658b2` is the explorer's stale pin of an rm'd job, `f7dfa14c` another area's).

Verdict: a pin written by another program works for idle retirement: nimitz could pin a background lead (or any
member) by writing pins.json, preferably under the same lock.

## Cleanup and livediff

- Interactive leads ended with `/exit` (E2E81 admiral 82633697, commodore ec198afa/903de92d, E2E82 admiral 4ab11d35).
- `claude stop` + `claude rm`: 36fef5ef, f59e1f5f, 54f2e4f3, 0a6ce393, c2561749, 903de92d. Already removed during the
  run by `nz sweep` / `claude rm`: 04d188a4, 55db890f, a328fbb7, e549b536. Spares 0d001bec, 1eef2f4a, efa7e9f9 had no
  job dir left. `cl agents --json --all` lists no `[E2E8*` row afterwards.
- tmux server `r2-v-inside` killed; `$MAP` removed with `command rm -rf`; no process with the world's path remains.
  Hook log and registry kept at `$R2/v-inside-events.jsonl` and `$R2/v-inside-registry.json`.
- pins.json restored to `["f7dfa14c","031658b2"]`.
- Transcript folders left under `~/.claude/projects/`: `-home-user--cache-nimitz-e2e-nimitz-e2e-r2-v-inside-`
  + `nimitz-e2e81-admiral`, `nimitz-e2e81-squadrons-sq`, `nimitz-e2e82-admiral`, `pa`, `pb`, `s1`, `s2`.
- livediff `[OK] live fleet unchanged` after the gap phase (01:37), during the wait (01:41) and at the end (02:22).
  Idle retire seen on non-live test sessions only (a13d7f76, another area's).
