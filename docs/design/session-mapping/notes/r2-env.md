# Area `env`: environment events for background members (ship, tender) and interactive leads

Tested 2026-10-07 on Claude Code **2.1.292** (installed) and nimitz **0.3.1**. World `$MAP=~/.cache/nimitz-e2e/nimitz-e2e-r2-env`,
`NIMITZ_HOME=$MAP/nimitz`, `NIMITZ_STATE=$MAP/state` (machine label m1), fake herdr (`$MAP/herdr/world.json`), private tmux `r2-env`.
Fleets: **E2E4** (SQ: ships S1 `$MAP/S1`, S2, S3, S4; admiral + commodore run by hand in tmux with the exact command the fake
pane recorded), **E2E41** (two machines m1/m2), **E2E42** (Dropbox conflicts). Model: project settings ask for haiku, the org
runs Opus 5.5; prompts were "reply ok" only.

Ids I created (all mine): ships f7dfa14c (S1, pinned), a13d7f76 (S2), 0b335f49 (S3), bfe4d6ab (E2E41/SQ/S1); copies made by
Claude Code during tests: f85025f6 (raced resume of S3, stopped+rm'd), spare a8b624ce (agent-view draft, ended with the view);
leads: admiral f8ed81a7 (pid 991107), commodore c4e686ff (pids 991117 then 1088240), commodore cold relaunch 5ae1e57f (L2).
Not mine but visible in this ~/.claude: b97ae728 (E2E1, another explorer) and the user's live fleet (never touched).

## 1. COVERAGE

| id | path | role | via | Claude Code does | nimitz shows | recovery | verdict | evidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| E1 R1:5c-3 | idle retire, unpinned ship (idle + unattached 61 min) | ship | env | daemon.log 01:47:03Z `bg retire a13d7f76: settled, idle 61m` then `bg settled a13d7f76 (done)`; process gone, session file gone; **no SessionEnd** (last hook lines: Stop 00:46:20Z, Notification 00:47:20Z); state.json untouched (`done`, `updatedAt` still 00:46:20Z, flags kept); `agents --json`: state done, pid null. | `stopped a13d7f76 job row a13d7f76 with no live pid (state done)`; check "no errors"; no "retired" reason (indistinguishable from a kill or a `claude stop`). | `nz start`: `--bg --resume a13d7f76`, daemon `bg claimed-spare a13d7f76 (fleet)`, SessionStart `resume` pid 1479241, same id, flags kept, lineage +`resume`. | OK / GAP (low, same as R1: nimitz cannot say "retired") | the plan also re-lists the leads' launch steps (they were dead, E12/E24), which is right |
| E2 | idle retire, **pinned** ship (Ctrl+T in `claude agents --cwd $MAP`; the filtered view showed only my two rows, cursor row verified by the highlight escape) | ship | env | Pinned at 00:48Z (`~/.claude/jobs/pins.json` = `["f7dfa14c"]`; nothing in state.json). At 01:47:28Z, 61 min idle and 25 s after S2's retire, S1 is **still alive** (pid 787324, session file present), and still alive at 01:50:24Z (64 min idle) when I stopped it myself (daemon: `bg settled f7dfa14c (killed)` 01:50:35Z, from my `claude stop`); no `retire` line for it ever. Docs: pin "keep[s] its process running while idle"; only a low-memory sweep stops pinned sessions. | `idle f7dfa14c session file: status idle`; nimitz does not read `pins.json`, so it cannot tell a pinned ship (never retires) from a plain one. | none needed | OK (pin works) / GAP (low: no pin signal) | daemon.log has no `retire f7dfa14c` |
| E3 R1:5c-1 | worker crash: `kill -9 <worker pid>`, `nz start` **3 s later** (G8 race) | ship | env | state.json stays `done` ("user confirmation received"); session file still present at t+2 s; daemon logs `bg settled 0b335f49 (done)` only **10 s** after the kill; the pty host did **not** restart the worker (R1 saw a restart; there the job was mid-work). The `--bg --resume 0b335f49` issued inside that 10 s window made a **copy f85025f6**: `source=fork`, no labels, `respawnFlags: []`, name `[E2E4.SQ]-S3`, "idle — send a prompt to start". No SessionEnd for the kill. | status t+3 s: `stopped` "job row with no live pid (state done)". `nz start`: "backgrounded f85025f6" then `lineage.broken` "no SessionStart bound it within 15s". Afterwards: `name.taken` ERROR + stowaway f85025f6; registry current still 0b335f49. | `nz stop f85025f6`, `nz rm f85025f6`, `nz start E2E4/SQ/S3` -> same id, SessionStart `resume` with labels. 3 commands, and nimitz's own fix hint (`nimitz stop f85025f6`) is the right first step. | GAP (med) | changed vs R1: no auto-restart; the race window is ~10 s after a kill; env-G1 |
| E4 | worker crash: `kill -9` worker **and** pty host, `nz start` after 30 s | ship | env | `bg settled (done)` at t+10 s; session file gone; no SessionEnd; no restart. | `stopped` "job row with no live pid (state done)". | `nz start` -> `--bg --resume 0b335f49`, same id, SessionStart `resume` with labels, job dir unchanged. | OK | pids 1028137/1028043 killed 00:50:15Z; events: SessionStart resume pid 1030426 |
| E5 R1:5b | auto-update (not installed; read-only) | ship/all | env | daemon.log 00:02:02Z: `binary ... 2.1.290 -> 2.1.292 - self-restarting for upgrade`, `shutting down (cause=upgrade, live_workers=2)`, new daemon pid 637403, `bg adopt: adopted=2 respawned=0 dead=0`, `post-takeover prewarm burst - respawned 0/2 stale workers, 2 refused`. Live session files: 19 x 2.1.292, **12 x 2.1.289** (the user's interactive leads), 3 x 2.1.283, **1 bg x 2.1.290** (adopted worker 9aacf927, state.json `cliVersion 2.1.290`). Job ids, session ids, names and `respawnFlags` of the adopted jobs unchanged. | `cc.version` WARN "newer than tested". No per-session "old binary" flag. | none needed | OK / GAP (low, unchanged from R1) | ~/.claude/daemon.log lines 393-403; sessions/3798787.json version 2.1.290 |
| E6 | reboot, simulated: `kill -9` of the workers **and** pty hosts of S2, S3 and E2E41/S1 at once (01:48:52Z); the leads were already dead | ship | env | No SessionEnd for any of them; session files removed by the daemon. **The outcome depends on state.json `state`**: S2 and E2E41/S1 (`done`) -> `bg settled ... (done)` 10 s later, stay stopped; S3 (`working`, detail "(idle — send a prompt to start)", i.e. resumed earlier without a prompt) -> **restarted by the daemon at 01:49:01Z**, 9 s after the kill, SessionStart `resume` with labels, same id, no daemon.log line. What cannot be simulated: the daemon itself dying and coming back (forbidden: it hosts the live fleet); the docs say a session found dead within 48 h "shows as failed. Attach or reply to it and it restarts", past 48 h "stopped with `ended while the background service was off`" (and `Enter` asks before resuming). | S2/E2E41-S1: `stopped - job row ... (state done)`, no reason; S3: `idle` (never seen as stopped). `nz up E2E4 --plan`: leads' launch steps + `--bg --resume a13d7f76` for S2 + cold launch of S4; S3 not listed (live). | `nz up E2E4 --yes` (leads need their panes/`--force`, env-G3) or `nz start` per ship: same ids. | OK (ships) / NOT SIMULABLE (daemon) | pids 1479241/1479203, 1119671/1119658, 1062678/1062667; S3 new pid 1580578 lstart 01:49:01 |
| E7 R1:5a-1 | transcript moved aside while the ship is live, then stop/start | ship | env | Live: nothing. `claude stop` writes a **7-line stub** (cost-state, custom-title, agent-name...). `nz start` (`--bg --resume 0b335f49`): the stub is renamed `0b335f49...orphaned-1791334556216-ed0541d9.jsonl`, SessionStart **`startup`** with the same id, **and the job's original prompt (`intent` "reply ok") is run again** (UserPromptSubmit + Stop at 00:55:57Z, one paid Opus turn). | Live: `idle`, check "no errors". Stopped: `stopped`, no issue. After start: `idle`, "[OK]". No history.lost. | none offered; history is gone | GAP (high) | changed vs R1: the prompt re-run is new (R1 recorded only `source=startup`); env-G2 |
| E8 R1:5a-2 | transcript removed while stopped, job row present | ship | env | Same as E7: `startup`, same id, prompt re-run (00:56:03-06Z). | `stopped` no issue; check "no errors"; start "[OK]". | none | GAP (high) | same as R1; env-G2 |
| E9 R1:5a-3 | no job row (`claude rm 0b335f49` by hand) and no transcript | ship | env | job dir deleted. | `stopped - registry knows it, but no row and no transcript`; `lineage.broken` ERROR, fix `nimitz start E2E4/SQ/S3 --force # cold launch; lineage starts over`; `start` refuses; `relaunch`: session.none. | clear refusal + explicit cold-launch path | OK | note: `nz rm E2E4/SQ/S3` on the identity only drops stale copy rows ("has no stale rows (copy.rows)"); the job row itself can only be removed with `claude rm` |
| E10 R1:5a-4 | transcript in another project dir (`...-S3-moved/`), job row present | ship | env | `--resume` finds it across projects: SessionStart `resume`, history kept, **keeps appending to the moved file** (270 kB there, nothing in the ship's own project dir). | `stopped` before, `idle` after; no mention of the odd location. | `nz start` works | OK (same as R1) | 5a-4 ran with the row present; R1 ran it without a row: both resume |
| E11 | no job row, transcript present in the ship's dir (R1 cli 1c) | ship | env | row re-created under the same job id 0b335f49 by `--bg --resume <id> --name ... --settings ...`; `respawnFlags` back. | `stopped - no job row; its session's transcript is there`; start plan "resume ... with every flag (its job row is gone; Claude Code keeps the id)". | `nz start` -> same id, `resume` | OK | job dir re-created with the same id (ADDENDUM) |
| E12 | lead's pane closed for real (tmux window killed = SIGHUP) + pane gone from herdr | commodore | env | SessionEnd **`other`**, session file removed, pid dead. The /exit-less close leaves the transcript intact. | `stopped - no session row; a transcript in its folder (continue with -c)`; check "no errors"; `open`: "is not running (mux.nopane)" fix `nimitz start`; `start --plan`: `claude -c --name ... --settings ...` "continue the interactive session in a pane"; `relaunch --plan`: same command. | `nz start E2E4/SQ --force` (see E14) + the command in a pane: SessionStart `resume` c4e686ff, history kept. | OK (with --force) | events pid 991117: SessionEnd other 00:54:51Z; then SessionStart resume pid 1088240 |
| E13 | pane "restored" by herdr with the old session id, process dead (fake world) | commodore | env | n/a | identical to E12 (the dead id in the pane is ignored). | same | OK | world.json pane w2:p1 agent_session c4e686ff, no process |
| E14 | pane restored as a plain shell in the lead's folder (no agent); and in general any `nz start` of a lead after its pane went away | commodore | env | n/a | `nz start`: "**already opening in pane w2:p2; it has not bound yet (trust prompt?)**" and runs nothing; repeats forever. Cause: `$MAP/state/pending/E2E4%2FSQ.json` (`{"at":..,"pane_id":"w2:p2",..}`) is never cleared, not even when the pane is gone from `herdr pane list`. Only `--force` opens a new pane (the stale record is then overwritten). | `nz start <lead> --force` | GAP (med) | env-G3 |
| E15 | lead by hand / via the fake pane: registry never binds (R1:3.0) | admiral, commodore | nimitz | SessionStart startup with labels. | status finds the lead by labels + pane (`idle c4e686ff ... pane w2:p1`), but `registry.json` has **no entry** for E2E4 or E2E4/SQ at any point (even after the fake pane reported the session id and `status` ran many times). `lineage.pending` says "the registry binds it later": it does not. | none; all later lead decisions run on labels only | GAP (med; R1 result unchanged) | registry identities = ships only |
| E16 | lead transcript moved aside while live, then `/exit` | commodore | env | `/exit` writes an **8-line stub** under the old id. `claude -c` on it: "**No conversation found to continue**", claude exits at once. | `stopped - a transcript in its folder (continue with -c)`; check "no errors"; `nz start --force --yes` "opened in pane"; the pane's command dies; status unchanged; no history.lost. | none; the lead cannot be started until the stub is removed (then E17) | GAP (high) | env-G4 |
| E17 | lead transcript moved to another project dir (`...-squadrons-sq-moved/`) | commodore | env | `claude --name ... --settings ...` (cold): **new session 5ae1e57f**, SessionStart `startup`; the moved conversation is never found (`-c` only looks in the cwd's project). | `not-started - no session, no lineage`; start plan = first launch, no warning. | none; silent new identity session | GAP (high; R1 inferred, now run) | env-G4 |
| E18 | two machines on one NIMITZ_HOME (Dropbox): m1 real, m2 posed (`NIMITZ_STATE=$MAP/state2`, `NIMITZ_HOSTNAME=otherbox`, `nz init --label m2`, `nz machine add E2E41 m2 --hostname otherbox --dir E2E41/SQ/S1=$MAP/m2-s1`) | ship | nimitz | S1 launched on m1 in `$MAP/m1-s1` (bfe4d6ab). | m1: `E2E41 (runs here as m1)`, S1 busy/idle. m2: `E2E41 (runs here as m2)`, S1 `not-started` **plus** `labels.mismatch` ERROR ("carries E2E41/SQ/S1's labels but runs in folder .../m1-s1, not .../m2-s1", fix `nimitz stop bfe4d6ab`) and `name.taken` ERROR; `m2 start --plan` plans the leads then refuses S1 with both errors; `m2 stop E2E41/SQ/S1`: "not live (session.none)". m2's registry stays empty. For E2E4 (m1-only fleet) m2 says "(does not run on this machine)" yet lists its ships as idle and its **labelled leads as "stowaways (no labels)"**. | per machine: own registry, own sessions; nothing to recover | OK on a real second box (the cross-machine errors only arise because both labels share one ~/.claude here); GAP (low) for the wording | env-G5 (what breaks for real: config writes from both sides -> E19) |
| E19 | Dropbox conflict: `squadron (conflicted copy 2026-10-07).toml` and `fleet (conflicted copy ...).toml` beside the real files | all | env | n/a | **Silently ignored**: status, check, doctor and start never mention them; the ship S1X declared only in the conflicted copy does not exist. | none; the user must notice the file | GAP (low) | env-G6 |
| E20 | Dropbox half-written `fleet.toml` (first 20 bytes) in fleet E2E42 | all | env | n/a | status: E2E42 **vanishes from the list**; one `[ERROR] fleet.toml does not parse ... (config.invalid)` line at the bottom. `nz check E2E42`: "**no errors** (1 other finding(s))". `nz check E2E4` (healthy fleet): ERROR config.invalid. **`nz start E2E4/SQ/S3` (healthy fleet) is refused** with the E2E42 parse error; so is every start/check on m2. `nz fleet set E2E42`: the same parse error with "fix by hand". | fix the file by hand | BREAK (high: one broken file blocks every fleet) | env-G7 |
| E21 | `disableAgentView: true` in the ship folder's project settings (new path from docs) | ship | nimitz | `claude --bg` exits 1: "'--bg' is disabled by the 'disableAgentView' setting." | `nz start`: "[ERROR] claude --bg (exit 1) -> '--bg' is disabled by the 'disableAgentView' setting." + "step failed"; S4 stays not-started; no fix hint. | remove the setting | OK (clear refusal) | S4 `$MAP/S4/.claude/settings.json` |
| E22 | private Herdr server (e2e_isolated.py pattern) for a real pane close | commodore | env | NOT RUN (unsafe) | - | - | NOT RUN (unsafe) | A server did start on `$MAP/hx/config/herdr/herdr.sock`, but the `herdr` 0.9.3 CLI run with the same private XDG env resolved **the user's** socket `~/.config/herdr/sessions/NIMITZ/herdr.sock` (`herdr status`, `herdr pane list` showed the live fleet's panes). nimitz through that CLI would have acted on the user's server; aborted, only my client window and my server pid were ended. The pane-close semantics were taken from E12 (SIGHUP) and the fake herdr instead. |
| E24 R1:5c-2 | `kill -9` of the admiral's interactive pid (lead crash) | admiral | env | session file removed, **no SessionEnd**; no transcript existed (the admiral never had a turn). | `not-started - no session, no lineage`; `start --plan` = cold first launch; but `start --yes` hits env-G3 ("already opening in pane w1:p1", pending record from 00:46). | `nz start E2E4 --force` | OK / GAP (G3) | pid 991107 killed 01:04:43Z |
| E25 | tender: `nz tender E2E4/SQ t1 "reply ok"`, then `kill -9` of its worker **mid-turn** (2 s after start) | tender | env | job fa6d99a2 `[E2E4.SQ]-COMMODORE/t1`, labels `NIMITZ_ID=E2E4/SQ/~t1`. After the kill the daemon **restarted the worker by itself 12 s later** (pid 1180833 -> 1185722), same id, SessionStart `startup` + the prompt submitted again (the transcript had no turn yet); no daemon.log line for the restart; no SessionEnd. The tender then finished its job (see the row below). | `busy` during the restart, then the final state. | none needed | N/A (tender role to be removed; kept as evidence that a bg job killed **mid-turn** is auto-restarted by the daemon under the same id, while a *done* job is only settled, E3/E4; the same daemon path applies to a ship) | events 01:04:48Z and 01:05:00Z |
| E26 | tender finishes while its commodore is dead (E12/E24 had ended the leads) | tender | env | the tender did the job (Stop 01:05:10Z), tried to report to `[E2E4.SQ]-COMMODORE`, failed, and parked on a question: state.json `blocked`, detail "job-done report failed; COMMODORE [E2E4.SQ] unreachable"; Notification hook 01:06:10Z. | `needs-you fa6d99a2 job row left a question: retry when [E2E4.SQ]-COMMODORE is reachable, or confirm alternate endpoint`; registry ~t1 bound (`launch`). | not run | N/A (tender role to be removed; recorded only because it ran before the update) | job fa6d99a2 |
| E23 | sleep/resume, login/logout, `claude auth logout`, low-memory stop, `cleanupPeriodDays` | all | env | docs only (see NEW PATHS) | - | - | NOT RUN (unsafe or not simulable) | |

## 2. NEW PATHS

- **Pin (Ctrl+T) is a daemon fact, not only UI**: docs: "Press Ctrl+T to pin a session to the top and keep its process running while idle"; stored in `~/.claude/jobs/pins.json` as a list of short ids. nimitz does not read it (a pinned ship never retires; see E2). `claude agents --cwd <path>` filters the view to one tree: the safe way to act on a test row.
- **Low memory** (docs, agent-view): "When memory stays low, the supervisor also stops idle sessions ... and stops idle pinned sessions too"; a failed start shows `possibly low memory - free some up and retry`. Not simulable safely.
- **Shutdown** (docs): "Shutting down or restarting your machine stops running background sessions ... Within 48 hours, the session shows as failed. Attach or reply to it and it restarts ... Past 48 hours ... shows as stopped with `ended while the background service was off`". **Sleep**: "Sleep alone doesn't stop sessions". The daemon's own idle exit (`cause=idle_exit`) happens only with `live_workers=0`.
- **Daemon restart with workers**: `claude daemon stop --any --keep-workers` (docs) keeps sessions; forbidden here. Every daemon start with live workers logs `bg adopt: adopted=N`; ids unchanged (E5).
- **`cleanupPeriodDays`** (settings-reference): default unset, "keeps transcripts indefinitely"; when cleanup removed a stopped session's transcript "Claude Code refuses to open the row: the message says there is nothing to resume ... `claude respawn <id>` runs its original prompt again". nimitz has no check for it; a cleaned transcript is E8/E9.
- **`disableAgentView` / `CLAUDE_CODE_DISABLE_AGENT_VIEW`**: kills `--bg` (E21). A user- or managed-level value would stop every ship on the machine.
- **SessionEnd reasons** (hooks doc): clear, resume, logout, prompt_input_exit, other. Observed: SIGHUP of an interactive -> `other`; `claude stop` -> `other`; kill -9 and idle retire -> **no SessionEnd**.
- **`←`/`/background` of a lead and `/fork`**: not in my area (R1 inside 3.5).

## 3. GAPS

**env-G1 (med) `nz start` within ~10 s of a worker death makes an unlabelled copy.** Trigger: the worker of a done ship is
killed (-9, OOM, reboot-like); nimitz sees "job row with no live pid" at once, but the daemon settles the job only ~10 s
later, and a `--bg --resume <id>` sent before that starts a copy (`source=fork`, no labels, `respawnFlags []`, same
name). What breaks: `name.taken` + stowaway, `lineage.broken` at start. Evidence: E3 (f85025f6 from 0b335f49). Repro:
`nz start F/SQ/S1 --yes; P=$(cl agents --json --all | jq -r '.[]|select(.name=="[F.SQ]-S1")|.pid'); kill -9 $P; sleep 3;
nz start F/SQ/S1 --yes` -> "backgrounded <new id>" + lineage.broken; `grep "bg settled" ~/.claude/daemon.log`. Fix idea:
when the session file is gone but state.json is not yet `settled`/updated, wait for the daemon (`bg settled <id>` or
state.json `updatedAt` > kill time) before resuming, or refuse with "worker just died; retry in 10 s".

**env-G2 (high) a lost transcript starts the ship fresh under the same id AND re-runs its launch prompt; nimitz says
[OK].** Trigger: transcript deleted/moved (cleanupPeriodDays, a sync tool, a by-hand mv) while the ship is stopped or live.
What breaks: `--bg --resume <id>` -> SessionStart `startup`, the stub is renamed `<sid>.orphaned-*.jsonl`, the job's
`intent` is submitted again (a paid turn that redoes the first task from scratch), history is gone; status/check "no
errors", start "[OK] verified". Evidence: E7/E8 events 00:55:57Z and 00:56:03Z (SessionStart startup, UserPromptSubmit,
Stop). Repro: `nz stop F/SQ/S1 --yes; mv ~/.claude/projects/<dir>/<sid>.jsonl /tmp/; nz start F/SQ/S1 --yes; tail -3
$NIMITZ_STATE/events.jsonl`. Fix idea: before a resume, require a transcript with at least one `user` line (a 7-line
stub = lost); after a resume, SessionStart `source=startup` on a known id = raise `history.lost` and point at the
`.orphaned-*` file; never let the daemon replay `intent` (relaunch with an empty prompt is not possible with `--resume`,
so at least say it).

**env-G3 (med) a stale pending-pane record blocks every later `nz start` of a lead.** Trigger: a lead opened by nimitz
in a herdr pane; the pane/process ends (close, SIGHUP, crash, /exit) and the pane id is gone from `herdr pane list` or
holds a plain shell. What breaks: `nz start <lead>` answers "already opening in pane <id>; it has not bound yet (trust
prompt?)" and launches nothing, forever; only `--force` works and the hint is buried in a WARN. Evidence: E14,
`$MAP/state/pending/E2E4%2FSQ.json` kept for an hour across many `status` runs. Repro: `nz start F/SQ --yes` (fake herdr),
remove that pane from world.json (or in a real herdr close the pane), `nz start F/SQ --yes` -> "already opening".
Fix idea: drop the pending record when the pane is not listed, has no agent, or is older than the bind timeout.

**env-G4 (high) a lead with a missing/moved transcript is either unstartable (stub) or silently restarted as a new
conversation.** Trigger: `/exit` after the transcript was removed leaves an 8-line stub, `claude -c` says "No
conversation found to continue" and exits (E16); a transcript in another project dir is invisible to `-c`, nimitz says
"not-started" and cold-launches a new session id (E17). What breaks: no `history.lost`, and for E17 a new identity
session with no warning. Repro E16: lead live in tmux, `mv <proj>/<sid>.jsonl /tmp/`, `/exit`, `nz start F/SQ --force
--yes`, run the pane command. Repro E17: `mv <proj>/<sid>.jsonl <proj>-moved/`, `nz start F/SQ --force --yes`.
Fix idea: use `--resume <registry id>` for leads (works across project dirs, like the ship path) once leads are bound
(env-G5); treat a transcript with no `user` line as absent.

**env-G5 (med) leads are never bound in the registry (R1 3.0 still true), even when nimitz itself opened the pane and
the pane later reports the session id.** Evidence: E15: registry identities = ships only after 1 h and ~40 `status`
runs; `lineage.pending` promises a later bind. What breaks: every lead decision (relaunch = `-c`, lineage, "which
conversation") runs on labels and the cwd's newest transcript. Repro: `nz start F/SQ --yes` with the fake herdr, run
the recorded command in a terminal, `nz status`, `cat $NIMITZ_STATE/registry.json`. Fix idea: bind in `status` when a
labelled live session for the id is found (SessionStart with labels in events.jsonl is enough).

**env-G6 (low) Dropbox "(conflicted copy ...)" config files are ignored without a word.** Evidence: E19. Repro: `cp
squadron.toml "squadron (conflicted copy 2026-10-07).toml"; nz check F`. Fix idea: a `config.conflict` WARN naming the
file.

**env-G7 (high) one unparsable fleet.toml blocks `start`/`check` of every other fleet, hides the broken fleet from
status, and `nz check <broken fleet>` says "no errors".** Trigger: a half-synced or half-written `fleet.toml` (Dropbox
mid-sync, an editor crash). Evidence: E20. Repro: `head -c 20 $NIMITZ_HOME/f2/fleet.toml > /tmp/x; cp /tmp/x
$NIMITZ_HOME/f2/fleet.toml; nz start F/SQ/S1 --plan` (F healthy) -> "[ERROR] fleet.toml does not parse ... f2 ...";
`nz check F2` -> "[OK] check F2: no errors (1 other finding(s))". Fix idea: scope the parse error to its fleet (list it
as "E2E42 (config.invalid)"), let other fleets start, and make `check <that fleet>` report the ERROR.

**env-G8 (low) cross-machine view wording.** From a label that does not run a fleet, its labelled leads are listed as
"stowaways (no labels)" and its ships as idle rows (E18); a ship launched on the other label raises `labels.mismatch`
+ `name.taken` with the fix `nimitz stop <it>` (stopping a ship that belongs to the other machine). Only reachable when
two labels share one ~/.claude (my simulation) or a mislabelled machine; low.

## 4. ADDENDUM: is the JOB id (`~/.claude/jobs/<id>`) stable?

Short answer: **yes in every case I could run or read**: the job id, the session id (`sessionId` = `resumeSessionId` =
`daemonShort`) and `respawnFlags` (`--name <name> --settings <labels>`) are unchanged across a worker crash (settled or
auto-restarted), the daemon's own restart (idle exit and upgrade), an upgrade adopt, a `claude rm` + nimitz resume (the
dir is re-created under the same id with the flags re-supplied by nimitz), and an idle retire + `nz start` (see the
retire rows). The only id changes seen were **copies** made by a raced resume (env-G1) and by `/clear`/`←` (R1, not
re-run here): those are new jobs, the old job keeps its id.

| event | before (job, sid, flags) | after (job, sid, flags) | source |
| --- | --- | --- | --- |
| worker `kill -9`, job done, `nz start` after 30 s (E4) | 0b335f49 / 0b335f49 / `--name [E2E4.SQ]-S3 --settings {labels}` | 0b335f49 / 0b335f49 / same flags, new pid 1030426, SessionStart `resume` | state.json read before and after; daemon `bg settled 0b335f49 (done)` then `bg claimed-spare 0b335f49 (fleet)` |
| worker `kill -9` mid-turn, daemon auto-restart (E25) | fa6d99a2 / fa6d99a2 / `--name [E2E4.SQ]-COMMODORE/t1 --settings {labels}` | same job and sid, flags unchanged, new pid 1185722 (no daemon.log line) | state.json + events.jsonl |
| `claude rm` + `nz start` (E11) | dir deleted | dir re-created as 0b335f49 with `respawnFlags` from nimitz's `--name/--settings` | `ls ~/.claude/jobs/0b335f49` before/after |
| raced resume within 10 s of a kill (E3) | 0b335f49 | **new job f85025f6** with `respawnFlags []`, name copied; the old job 0b335f49 keeps its id, sid and flags and is resumed normally once the copy is removed | state.json of both |
| upgrade adopt 2.1.290 -> 2.1.292 (E5, read-only, the user's live workers) | jobs 9aacf927 and 0abc1234, sids equal to the job ids, flags `--name [HOME.GITHUB]-NIMITZ_DEV --settings {labels}` / `--name [MAIN.LUX]-DEV --settings {labels}` | daemon.log `bg adopt: adopted=2 respawned=0 dead=0`; the same job dirs and sids; session files still `version 2.1.290` / `2.1.289` (the worker binaries were not replaced), `createdAt 2026-10-05T03:14:08Z` kept | ~/.claude/daemon.log lines 393-403, jobs/9aacf927/state.json, sessions/3798787.json |
| daemon's own restart (read-only history) | the only restarts in the log are `cause=idle_exit` with `live_workers=0` (nothing to adopt) and `cause=upgrade` with live workers (adopted, above); a crash/`daemon stop` with workers was not made (forbidden) | docs: "Sessions survive that version mismatch intact"; `claude daemon stop --any --keep-workers` keeps sessions | daemon.log lines 245-400 |
| idle retire + `nz start` (E1) | a13d7f76 / a13d7f76 / `--name [E2E4.SQ]-S2 --settings {labels}` | same job dir, same sid, flags unchanged (state.json not even rewritten by the retire), SessionStart `resume`; `bg claimed-spare a13d7f76 (fleet)` | state.json before 00:54Z and after 01:47Z; daemon.log 01:47:03Z/01:47:29Z |
| pinned ship, 72 min idle (E2) | f7dfa14c / f7dfa14c / `--name [E2E4.SQ]-S1 ...` | unchanged, process never stopped | pins.json, session file 787324.json |

Side facts for Q1: a job id is also the key of `~/.claude/jobs/pins.json` (pin survives everything above) and of
`~/.claude/daemon/roster.json`. `state.json.cliVersion` records the binary the job was last started with (a cheap
"old binary" signal nimitz could show after an upgrade).

**env-G9 (low) `claude rm` leaves the pin behind.** `~/.claude/jobs/pins.json` still lists `f7dfa14c` after `claude stop`
+ `claude rm` of that job (I could not unpin it first: the filtered agent view's cursor landed on S2, so I did not press
Ctrl+T, BRIEF rule 5). nimitz could warn on a pinned id with no job row. Evidence: pins.json read after the rm.

**Hazard (not a nimitz gap): a "private" herdr is not private on this box (E22).** `herdr` 0.9.3 CLI calls made with
`XDG_CONFIG_HOME`/`XDG_STATE_HOME` pointing at a temp dir and all `HERDR_*` unset still resolved
`~/.config/herdr/sessions/NIMITZ/herdr.sock` (the user's session) for `status`/`pane list`, while the server I had
started listened on the temp socket. `herdr-plugin/tests/e2e_isolated.py` assumes the CLI follows XDG; with a named
session in `~/.config/herdr/sessions/` it does not. Anyone running that test here would drive the user's panes.

## 5. CLEANUP and livediff

- `livediff` ran after every phase and at the end: always `[OK] live fleet unchanged` (the snapshot is kept as
  `$R2/env-live-before.json`). One `[INFO]` state change never appeared; no live session changed id, pid or kind.
- Jobs stopped and removed with `claude stop` + `claude rm`: f7dfa14c (S1), a13d7f76 (S2), 0b335f49 (S3), bfe4d6ab
  (E2E41/SQ/S1), fa6d99a2 (tender), f85025f6 (raced copy, removed during E3). `claude agents --json --all` lists no
  `[E2E4...]` row. The agent-view spare a8b624ce ended with the view (`bg settled a8b624ce (killed)`).
- Interactive sessions: admiral f8ed81a7 (killed in E24), commodore c4e686ff (SIGHUP in E12, `/exit` in E16),
  5ae1e57f (`/exit`); no process with `E2E4` or `r2-env` in its command line remains.
- Private tmux server `r2-env` killed; my private herdr server (pid 1151735) terminated by pid, the user's herdr servers
  (418777, 2857518) untouched; `$MAP` removed with `command rm -rf` (including the `aside/` copies of the test
  transcripts and the m2 state).
- Left for the user to remove (transcripts; the `-moved` one was made by E17):
  - `~/.claude/projects/-home-user--cache-nimitz-e2e-nimitz-e2e-r2-env-S1/`
  - `~/.claude/projects/-home-user--cache-nimitz-e2e-nimitz-e2e-r2-env-S2/`
  - `~/.claude/projects/-home-user--cache-nimitz-e2e-nimitz-e2e-r2-env-S3/`
  - `~/.claude/projects/-home-user--cache-nimitz-e2e-nimitz-e2e-r2-env-m1-s1/`
  - `~/.claude/projects/-home-user--cache-nimitz-e2e-nimitz-e2e-r2-env-nimitz-e2e4-squadrons-sq/`
  - `~/.claude/projects/-home-user--cache-nimitz-e2e-nimitz-e2e-r2-env-nimitz-e2e4-squadrons-sq-moved/`
  - the stale pin `f7dfa14c` in `~/.claude/jobs/pins.json` (env-G9; I did not edit that file).
- Not mine, seen in this ~/.claude and left alone: jobs b97ae728 (E2E1), E2E2/E2E3/E2E81 sessions, pins 031658b2 and
  36fef5ef.
- Cost: about 9 paid Opus turns (3 ship cold launches, 1 commodore turn, 2 prompt re-runs in E7/E8, the tender's 2
  runs, 1 E2E41 launch).
