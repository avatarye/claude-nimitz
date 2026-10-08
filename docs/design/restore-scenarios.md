# Restore scenarios: how a fleet's live state gets lost, and what `nimitz restore` must bring back

Written from the user's side for P9 (plan-v4 section 13). Facts about Claude Code
come from the round-2 notes (`docs/design/session-mapping/notes/r2-*.md`, Claude
Code 2.1.292); facts about Herdr from the adapter (`crates/nimitz-mux/src/herdr.rs`),
the fake world, and the shape of a Herdr session snapshot
(`~/.config/herdr/session-snapshots/*.json`: workspaces with `custom_name`, tabs
with `custom_name`, panes with **cwd only**; no command, no agent session).

## 0. Vocabulary and the baseline facts

**Who I am.** I run two or three fleets at once in one Herdr session (`NIMITZ`).
Workspace `F` holds the tab `F.ADMIRAL`; workspace `F.SQ` holds `F.SQ.COMMODORE`
and, when I opened one, `F.SQ.SHIP`. The `ADMIRALTY` workspace is the hub, not a
fleet. I also keep my own Claude sessions and a Yazi tab in some of those
workspaces. Fleet config lives in Dropbox (`NIMITZ_HOME`), state per machine
(`NIMITZ_STATE`).

**Member states (0.4.0, every member is a job).**

| word | meaning | how nimitz tells |
| --- | --- | --- |
| live-attached | process alive, a terminal attached (never idle-retired) | session file with `jobId` + live pid; a Herdr pane whose `agent_session` is the member's session |
| live-unattached | process alive, nobody attached (ships normally; a lead after ctrl-z) | same, no pane shows it |
| parked | no process; the daemon retired it, it died with the machine, or it crashed and was not restarted; last event is not a SessionEnd from `claude stop` | job row, no live pid, no stop SessionEnd (section 4 of the plan) |
| stopped | no process because someone ran `claude stop` / `nimitz stop` | SessionEnd `other` with labels from the job's pid |
| gone | no job row (`claude rm`, agent-view delete) | registry `job` with no `jobs/<id>/` |
| not-started | never launched, or started over (`--fresh`) | empty lineage |

**Tab states** (what a `F.*` tab's pane can show):

| word | pane content |
| --- | --- |
| attached | `claude attach <job>` of the right member (`agent_session` = member's session) |
| shell | no agent (attach client ended, Herdr restored the tab from a snapshot, a half-made restore) |
| foreign | an agent whose session is not the member's (wrong job, a copy, my own `claude`) |
| agent-view | the machine-wide agent view (`/exit` or `/stop` inside an attach lands there) |
| missing | no tab with that full name in the expected workspace |

**What survives what** (evidence in brackets):

| event | job row + `respawnFlags` | process | session file | transcript | pin | Herdr ws/tab names | Herdr pane processes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| idle retire (~1 h unattached, unpinned) | kept, untouched | gone | gone, **no SessionEnd** | kept | kept | kept | attach client exits to a shell (not tested in Herdr; attach of a stopped job wakes it, cli-07) |
| `claude stop` / `nimitz stop` | kept | gone | gone, SessionEnd `other` | kept | kept | kept | same |
| worker `kill -9` | kept (`crashed` or `done`) | gone; mid-turn jobs are restarted by the daemon in ~10-15 s (E25, cli-30), done jobs are only settled (E3/E4) | gone | kept | kept | kept | same |
| Herdr server killed | kept | **alive** (the daemon is not Herdr's child) | kept | kept | kept | names come back from the snapshot when the server restarts; pane ids may restart | all pane processes gone; restored panes are plain shells in the recorded cwd |
| reboot | kept (`working`/`done`/...; docs: within 48 h shown as `failed`, "attach or reply restarts it"; past 48 h "ended while the background service was off") | gone; **no respawn** until something touches the job | stale files with dead pids may remain | kept | kept | from the snapshot | gone; shells |
| Claude Code auto-update | kept, ids unchanged (E5) | idle workers moved to the new binary; attached/busy untouched | kept | kept | kept | kept | kept |
| `claude rm` | **gone** (pin entry stays, env-G9) | gone | gone | kept | stale | kept | attach client exits |
| transcript deleted/moved (cleanup, Dropbox, `mv`) | kept | - | - | **gone**: a resume runs `startup` and **re-runs the launch prompt** (env-G2) | kept | kept | - |

"Restored panes are plain shells" (the Herdr rows above) holds only with
Herdr's `[session] resume_agents_on_restore` off: at its default (true) the
server itself runs `claude --resume <session>` in every restored agent pane.
Measured (r3-e2e.md finding 10, Claude Code 2.1.292): on a live job that is
refused and the pane is a shell again; on a stopped, parked or removed job
the conversation opens interactively with no job (`held`), and a `--bg
--resume` meanwhile starts a copy. So after a reboot plus a Herdr restart
every lead's pane holds its conversation: restore reports `tab.resumed`
(an Error: fix `/exit` there, then restore) and never wakes a held member; `nimitz
doctor` warns while the setting is on. The checklist turns it off for its
test server and shows the on case in one optional step.

Other facts the scenarios lean on: the job id is stable across every event above
(plan section 1); `claude attach <job>` on a parked job wakes it as itself
(cli-07); ctrl-z detaches to the shell and the job keeps running (3.13, cli-06);
`/exit` in an attach opens the agent view (3.11); killing an attach client leaves
the job running (cli-06); `--bg --resume` of a live job or inside ~10 s of a kill
starts a **copy** with the member's name (cli-09, env-G1); a resume of a job that
once ran Remote Control reconnects it (remote-G1); Herdr pane ids are not stable
across a server restart (`toggle.sh`: "ids can restart with the server"), labels
and cwd are.

**Rules every scenario assumes of `nimitz restore`** (R = rule):

- R1 Reuse before create: a tab is found by its **full name** in its workspace, or by the
  **session** a pane shows, anywhere. Create a workspace or tab only when neither matches.
- R2 Never type into a live agent: `pane run` only into a pane with no agent (shell). An
  attached, foreign or agent-view pane is reported, never written to.
- R3 Resume by exact session id, under plan section 5 (wait out the crash window; read
  the launch output; a "started a copy" note fails the step and the copy is removed).
- R4 Leads always come back (resume + attach). Ships come back only if they were in the
  last-known active set and not explicitly stopped, or with `--all`. Parked is not an error.
- R5 One member, one tab, one job: a duplicate (second tab, second attach client, a copy
  job) is reported with one fix; nimitz closes no tab and kills no interactive process.
- R6 Idempotent: a second run with nothing changed plans zero steps and says so.
- R7 Only this fleet (or `--all` fleets of this machine); never the `ADMIRALTY`
  workspace, never a tab or pane that is not a member's.
- R8 `--plan` first; the plan names every tab it will create, every pane it will run in,
  every job it will resume, and every member it cannot restore with a fix.

## 1. Scenarios

Each row: the story (my words), state before, what is left after the event, the
restored state I expect, what restore must NOT do, what nimitz can know (and what it
must have recorded beforehand), priority (must / should / nice).

### R-01 Left overnight, leads still attached

- Story: "I left the fleet running and went to bed. In the morning the ships are all parked and the leads are still in their tabs."
- Before: admiral and commodores live-attached in `F.ADMIRAL` / `F.SQ.COMMODORE`; ships live-unattached (two of them busy at 23:00); one ship tab `F.SQ.S1` attached.
- Left: leads unchanged (attached is never retired); ships that were idle and unattached retired after ~1 h (row `done`, no SessionEnd, daemon.log `bg retire`); S1 in its attached tab still live; the busy ships finished and then retired.
- Expected after `restore F`: nothing to do for leads and S1; parked ships stay parked ("parked, wakes on demand"); `--all` wakes them. Output: `[OK] F: nothing to restore (3 ships parked)`.
- Must not: wake parked ships without `--all`; re-run `claude attach` in the attached tabs; treat parked as an error.
- nimitz knows: parked vs stopped from the event log (no stop SessionEnd); the attached state from Herdr `agent list`; the active set from `active.json` (written on the last status/layout/open and by the hook's SessionEnd).
- Priority: must (this is the common morning).

### R-02 Left overnight, leads detached (ctrl-z) and then retired or swept

- Story: "I ctrl-z'd out of the commodore to use its tab as a shell, forgot, and went to bed."
- Before: commodore live, then detached; its tab shows a shell. Lead is pinned by nimitz (plan section 7).
- Left: pinned, so normally still live-unattached. Under memory pressure the daemon stops idle pinned sessions too: then parked. Tab: shell in the lead folder.
- Expected: the tab `F.SQ.COMMODORE` is reused (shell pane): `claude attach <job>` typed there; if parked, the attach wakes it (same job, SessionStart `resume`); nimitz verifies the bind. If the pin is missing (someone unpinned, or `pins.json` was lost), restore re-pins.
- Must not: open a second `F.SQ.COMMODORE` tab; `--bg --resume` **and** attach (attach alone wakes it; a resume while the attach is waking races into a copy).
- With `resume_agents_on_restore` on and a Herdr restart in between: Herdr's `claude --resume` holds the parked lead's conversation in that pane (a live lead's is refused, its pane a shell): `tab.resumed`, no wake, until `/exit` there (finding 10).
- nimitz knows: pane has no agent (`agent list` has no row for it, or `pane list` shows it with no agent), the tab name matches; `pins.json` membership.
- Priority: must.

### R-03 Herdr server killed or crashed; sessions alive, panes gone

- Story: "Herdr crashed (or I restarted it for an update). Every pane process is dead but `claude agents` shows the whole fleet still running."
- Before: two fleets, leads attached, three ship tabs attached, my own Claude session in a tab `notes` in workspace `F.SQ`, Yazi tabs.
- Left: daemon and all jobs alive (leads were attached: now live-unattached; the attach clients died with Herdr). Herdr restarted: workspaces and tabs come back **by name** from the snapshot with plain shells in the recorded cwd (pane ids may differ). Or, if the snapshot was not restored: an empty Herdr with one default workspace.
- Expected: for each lead, the tab is found by full name (shell pane) and `claude attach <job>` is run in it; a missing tab/workspace is created. Ship tabs: only those in the active set (they were attached before the loss) get an attach again. My `notes` tab and Yazi tabs are untouched. The leads were unattached for a while: if the hour passed and a lead was not pinned, the attach wakes it.
- Must not: match by stale pane ids from `active.json`; create `F.SQ` twice because the snapshot restored it as `F.SQ` and nimitz looked for `[F.SQ]`-style names (loose match, `normalize_label`); type into a restored shell that someone already reused for their own `claude`.
- nimitz knows: everything, if `active.json` recorded workspace+tab **labels** (not only ids) and which ship tabs were attached; Herdr identity (session name) to detect R-29.
- Priority: must.

### R-04 A lead's shell or pane killed

- Story: "I closed the pane by accident (or `kill`ed the shell, or the attach client got SIGHUP)."
- Before: commodore live-attached in `F.SQ.COMMODORE`.
- Left: commodore live-unattached (an attach client's death never stops the job, cli-06). Tab: gone (pane was the tab's only pane) or a shell (a split pane remains).
- Expected: tab reused or recreated; `claude attach <job>` in it. Job untouched.
- Must not: resume anything; open a tab while a split pane of the same tab still shows the attach (then it is R1 "found by session": focus, nothing else).
- nimitz knows: the job is live (session file + pid); no pane shows its session.
- Priority: must.

### R-05 A Herdr tab closed

- Story: "I closed the `F.SQ.COMMODORE` tab to tidy up and now the commodore is unreachable from the fleet workspace."
- Before: as R-04.
- Left: job live-unattached (pinned, so it stays); no tab.
- Expected: `tab create --label F.SQ.COMMODORE` in workspace `F.SQ`, attach. Same for a closed `F.ADMIRAL` in `F`. A closed ship tab is recreated only if `--all` or the ship was in the active set as attached.
- Must not: put the tab in a different workspace than the one that exists for `F.SQ` (loose-matched), create a new workspace when one matches loosely.
- nimitz knows: `active.json` says the member had a tab; Herdr says it does not.
- Priority: must.

### R-06 A Herdr workspace closed

- Story: "I closed the whole `F.SQ` workspace (commodore tab, S1 tab, Yazi)."
- Before: workspace `F.SQ` with `F.SQ.COMMODORE` attached, `F.SQ.S1` attached, `Yazi`.
- Left: jobs live-unattached; no workspace.
- Expected: `workspace create --label F.SQ --cwd <sq folder>`; its unlabelled root tab is **claimed** (renamed `F.SQ.COMMODORE`, attach run in its root pane); `F.SQ.S1` tab created + attach (it was active); Yazi is not nimitz's: not recreated.
- Must not: leave an unlabelled root tab behind and create the lead tab next to it (today's `layout` claims it; restore must too); recreate ship tabs that were not attached before.
- nimitz knows: as R-05.
- Priority: must.

### R-07 Tabs and panes rearranged by me

Four sub-cases; all "before": leads attached, S1 attached, all names standard.

- R-07a Tab renamed to something else ("I renamed `F.SQ.COMMODORE` to `cmdr`").
  - Left: the pane still shows the commodore's session.
  - Expected: found **by session** (R1); nothing to do. Report Info `tab.renamed` ("`cmdr` shows F/SQ; nimitz looks for F.SQ.COMMODORE"), fix `herdr tab rename <id> F.SQ.COMMODORE`; rename it itself only with `--rename`. After a Herdr restart (R-03) the renamed tab comes back as a shell named `cmdr`: no name match, no session match -> a new `F.SQ.COMMODORE` tab is created and `cmdr` stays a shell: acceptable, reported as `tab.orphan`.
  - Must not: create a second tab while the session is visible somewhere.
  - Priority: must (the by-session match), should (the rename hint).
- R-07b Lead tab moved to another workspace ("I dragged `F.SQ.COMMODORE` into workspace `F` to have both leads side by side").
  - Left: the tab is in `F`, pane attached.
  - Expected: found by session; nothing to do; Info `tab.moved` naming where it is. After R-03 (shell): the name matches in the wrong workspace: reuse it **there** (the user moved it on purpose), do not create one in `F.SQ`. Rule: name match in any workspace wins over "its" workspace when the tab is a shell in the member's folder; otherwise prefer the member's workspace.
  - Must not: move tabs.
  - Priority: should.
- R-07c Two tabs with the same name ("after a Herdr restart and a hasty `nimitz open` I have two `F.SQ.COMMODORE` tabs, one attached, one a shell").
  - Expected: the attached one is the member's; the shell one is reported `tab.dup` with fix "close tab <id> (shell)". Two **attached** tabs for one job (both show the same session; Claude Code allows several attach clients; not measured): no step, Warn `tab.dup` with fix "ctrl-z in tab <id>". Two shells with the name: use the one in the member's workspace, report the other.
  - Must not: close tabs; type into both.
  - Priority: must (never two attaches from nimitz), should (the report).
- R-07d Split panes ("the lead tab has a Yazi pane next to the attach").
  - Expected: the tab is found by session (any pane in it); nothing to do. If the attach pane died and the Yazi pane remains, the tab is a name match with **no shell pane**: `pane split` next to the remaining pane and attach there (today's `pane_in_tab` takes the first pane: that would type `claude attach` into Yazi: wrong).
  - Must not: `pane run` into a pane whose foreground process is not a shell.
  - nimitz knows: `pane list` gives the foreground cwd/title; Herdr marks agent panes; a pane with a non-shell foreground that is not an agent (Yazi) must count as "taken".
  - Priority: must.

### R-08 Reboot: everything gone

- Story: "Power cut / kernel update. Herdr, the daemon, every worker: gone. I open Herdr, my workspaces come back as shells."
- Before: two fleets; F: admiral and commodore attached, S1 attached and busy, S2 idle unattached, S3 parked for a day; G: everything parked except the admiral.
- Left: job rows with their last `state` (`working` for S1 and anything mid-turn, `done`/`stopped` for the rest; docs: shown as `failed` within 48 h), `respawnFlags` intact, transcripts intact, `pins.json` intact, stale `~/.claude/sessions/<pid>.json` files possible (dead pids; **pid reuse** after a reboot is real: pid 1234 may be a browser now). Herdr: names and cwds from the snapshot, shells. The daemon starts on the first `claude` call and adopts nothing (`adopted=0`); nothing respawns on its own.
- Expected (`restore --all` or per fleet): leads: `--bg --resume <session>` (job row exists: native resume) then attach in their tabs; the crash window rule (plan section 5) applies to `working`/`crashed` rows: wait up to 20 s for a daemon restart before resuming; S1 (active, busy at loss) resumed and re-attached, reported `turn.interrupted` Info ("S1 was mid-turn; its last prompt was not finished; re-prompt it"); S2 (active, idle) resumed because it was in the active set; S3 stays parked (it was parked before the loss); G: admiral resumed + attached, ships stay parked. After the resumes every SessionStart must be `resume` (a `startup` means history lost -> `history.lost`, never `[OK]`).
- Must not: trust a session file by pid alone (check the process is a `claude` with that job, or its start time is after the file's); resume a job whose pid is alive; resume ships not in the active set without `--all`; run two resumes of one job because the first took longer than the poll.
- With `resume_agents_on_restore` on (the user's config): Herdr's restart runs `claude --resume` in every lead's pane, so every lead's conversation is held interactively (finding 10); restore reports `tab.resumed` per lead with the `/exit` fix and wakes nothing (a wake would make a copy); after the `/exit`s the next restore wakes them by their shell tabs' attach.
- nimitz knows: `active.json` (members live/attached and their status words at the last write); job rows; event log tail (the last line per job is not a stop SessionEnd: parked). What it cannot know: whether the interrupted turn matters; it reports and leaves the decision to me or the commodore.
- Priority: must.

### R-09 Claude Code updated overnight (upgrade adopt)

- Story: "Claude Code auto-updated at 02:00. In the morning everything still runs."
- Before: leads attached, ships live.
- Left: daemon self-restarted, `bg adopt: adopted=N respawned=0`; idle workers moved to the new binary; attached/busy workers kept on the old binary (`state.json.cliVersion` older than the daemon); Herdr panes untouched; the attach clients stayed connected (E5 saw the user's attached leads survive).
- Expected: `restore` plans nothing: `[OK] nothing to restore`. Info per member still on the old binary ("runs 2.1.290 under daemon 2.1.292; `nimitz relaunch` moves it when idle"). `compat` Warn if the new version is above the tested range; restores still go ahead (as status does today).
- Must not: relaunch members because of the version difference; refuse to restore under an untested-but-newer version.
- nimitz knows: `cliVersion` per row vs `claude --version`; daemon.log `bg adopt` line.
- Priority: should.

### R-10 A partial restore interrupted halfway

- Story: "I ran `nimitz restore --all --yes`, Herdr hung on the third tab and I hit ctrl-c" (or the machine went to sleep, or `herdr tab create` failed with an error mid-list).
- Before: R-08 state (reboot), restore in progress.
- Left: workspace `F.SQ` created with an **unlabelled root tab** and no command; `F.ADMIRAL` created, admiral resumed and attached; the commodore resumed (job live) but its tab not made; a ship whose resume printed "started a copy as X" (the race, if the window was misjudged) with the copy still running; `active.json` unchanged.
- Expected: the second run converges: claims the unlabelled root tab of `F.SQ` for `F.SQ.COMMODORE` (it is a shell in the right folder) and attaches the already-live commodore (no resume); skips the admiral (attached); the copy X is detected (a job with the member's name, no labels, `respawnFlags []`) and the member is reported `name.taken` with fix `nimitz sweep F/SQ/S1` (or restore stops+removes it itself when it created it in this run).
- Must not: create a second `F.SQ` workspace because the root tab is unlabelled and the loose match of `F.SQ` failed; resume the commodore again (live pid: that makes a copy); leave the plan half-applied with `[OK]`.
- nimitz knows: Herdr lists the half-made shape; job rows show what is live. Restore should record its own progress (`restore.json`: run id, steps done) so the report after an interrupt says "3 of 7 done; run again".
- Priority: must.

### R-11 Two fleets in two workspaces

- Story: "F and G each have their workspaces. I only want F back."
- Before: F and G laid out; G's commodore tab is in `G.SQ`.
- Left (after R-03): shells everywhere.
- Expected: `restore F` touches only `F`, `F.SQ*` workspaces and F's jobs; G's tabs and jobs untouched, listed as "not in scope"; `restore --all` does both, F then G, one plan. Fleet names are unique, so tab names never collide across fleets; the `ADMIRALTY` workspace is never a target.
- Must not: match `G.SQ.COMMODORE` for F by a loose rule; resume G's leads when only F was asked.
- Priority: must.

### R-12 Restore run twice in a row

- Story: "I ran it, it worked, I ran it again to be sure."
- Expected: second run: zero steps, `[OK] F: nothing to restore`, exit 0, no new tab, no `claude attach` typed anywhere (typing `claude attach <job>` into an attached pane sends it as a prompt to the lead: a paid turn and nonsense in its context).
- Must not: anything.
- nimitz knows: Herdr `agent list` shows every member's session in a pane.
- Priority: must (this is the acceptance test for R2 and R6).

### R-13 A member removed from the fleet while its tab stays

- Story: "I ran `nimitz ship rm F/SQ/S2` (or `squadron rm`), the job was stopped and removed, but the tab `F.SQ.S2` is still there showing a dead attach / shell."
- Before: S2 attached in its tab.
- Left: no config entry, no job row; tab with a shell (attach client exited when the job was removed) or, if only the config was edited by hand, a live unlabelled-by-config job still attached.
- Expected: restore ignores the tab for the fleet (no member named S2); reports Info `tab.orphan` "`F.SQ.S2` names no member; close it". If a job still runs there with the old labels: it is a stowaway of this home (section 6): reported with `nimitz stop/rm` fix, not touched by restore.
- Must not: reuse that tab for another member; close it; stop the job in it as part of restore.
- Priority: should.

### R-14 A tab attached to the wrong job

- Story: "I typed `claude attach` with the wrong id in the `F.SQ.COMMODORE` tab; it shows the ship S1 (or a copy made by a race, or G's commodore)."
- Before: tab name right, pane session = another job's.
- Left: the same after any loss; the commodore itself is live-unattached or parked.
- Expected: the tab counts as **foreign** (name matches, session does not): never written to. The commodore is restored only if a tab for it can be made without a duplicate name: it cannot, so it is reported `tab.taken` Error with the one fix that works: "ctrl-z in tab <id> (it shows [F.SQ]-S1), then `nimitz restore F`". With `--force`, open a second tab anyway and report `tab.dup`.
- Must not: type into the pane; close it; `claude attach` the commodore in it (it would stack an attach inside an attach? not measured; never do it).
- nimitz knows: the pane's session id vs the member's session.
- Priority: must.

### R-15 A pane showing my own unrelated Claude session

- Story: "I opened a plain `claude` in a tab `scratch` in workspace `F.SQ` for my own notes; and once I opened one in the ship folder itself."
- Before: that session is unlabelled; in a ship folder it is the user's own (section 6), not a copy unless it resumes a ship's conversation.
- Expected: ignored by restore in every case: not a member's session, not a member's tab name. If I named the tab `F.SQ.S1` myself and run my own `claude` there: R-14 rules (foreign, `tab.taken`).
- Must not: type into it, close it, count it as the ship's tab, list it as a stowaway (it is not one unless it is a copy).
- Priority: must.

### R-16 A lead still running interactive from 0.3.x

- Story: "I migrated to 0.4.0 but the admiral tab still runs the old `claude -c --name ... --settings` interactive session."
- Before: `F.ADMIRAL` tab, interactive pid, labelled, no job row; registry v1 migrated: `job` unknown, `session` = its id.
- Left: unchanged, or after R-03: the interactive session died with the pane (SessionEnd `other`), transcript intact, tab a shell.
- Expected: live interactive: restore reports `lead.interactive` with the fix "type /exit in tab F.ADMIRAL, then `nimitz restore F`" and does nothing for it (section 7). Dead (after R-03): `--bg --resume <session> --name --settings` (no job row: every flag), SessionStart `resume`, attach in the tab, pin it. Lineage gets `via migrate`.
- Must not: resume while the interactive is live (cli-L2: a copy with the admiral's name); kill the interactive process; type `/exit` for me (R5).
- nimitz knows: session file `jobId null`, `kind interactive`, labels of this home.
- Priority: must (migration week), then nice.

### R-17 Transcript lost: history cannot come back

- Story: "`cleanupPeriodDays` or Dropbox ate the commodore's transcript (or I `claude purge`d the folder)."
- Before: commodore parked; registry `session` set.
- Left: job row intact, no `<session>.jsonl` in any project dir (or only a 7-9 line stub with no `user` line).
- Expected: no resume (it would run the launch prompt again under the same id and say `startup`): `history.lost` Error with fix `nimitz start F/SQ --fresh`; the tab is still prepared? No: no tab is created for a member that is not restored (keep the shape honest); an existing shell tab is left. The rest of the fleet restores normally; the squadron's ships are not blocked by their lead's lost history (unlike 0.3.1's fleet-wide `lineage.broken`).
- Must not: `[OK] verified` on a `startup`; cold-launch on its own; block other members.
- nimitz knows: transcript lookup across project dirs (plan section 5); stub detection (no `user` line).
- Priority: must.

### R-18 The machine moved: cross-machine fleet

- Story: "NIMITZ_HOME is in Dropbox. Fleet F ran on my server; I open Herdr on the laptop and run `nimitz restore F` there."
- Before: F's `machine` = server; the laptop has its own `~/.claude`, its own `NIMITZ_STATE` (registry without F's jobs, no `active.json` for F).
- Expected: refuse per fleet: "F does not run on this machine (runs as <server>); nothing recorded here" (as `layout` skips today). If I added the laptop with `machine add` and want F here: that is `nimitz up F` (cold launches, new lineage), not restore; restore says so. If the same machine was re-labelled (`init --label` changed): `machine.mismatch` with the fix.
- Must not: resume the server's session ids on the laptop (no transcripts here: a chain of `history.lost`, or worse, a `startup` replay); touch Herdr on the laptop for F.
- nimitz knows: `fleet.machine`, the machine label, the per-machine registry.
- Priority: should.

### R-19 A ship mid-turn when Herdr died (and when the machine died)

- Story: "S1 was editing files when Herdr crashed."
- Herdr only: the daemon and S1 are untouched; S1 finishes its turn. Restore: reattach its tab if it was attached; nothing else. Must not resume (live pid) or interrupt.
- Machine died: row `working`, no pid. Restore: wait the crash window (the daemon may restart it on its own if it has started by then), then resume by id; SessionStart `resume`; report `turn.interrupted` (the turn is not replayed; uncommitted edits in its folder or worktree stay as they are). If the ship used a worktree (`bgIsolation` worktree), say where the worktree is.
- nimitz knows: `active.json` status word at the last write (`busy` + `waitingFor` absent); `state.json.state working` with no pid.
- Priority: must (correct resume), should (the report).

### R-20 A permission prompt or question pending

- Story: "The commodore had an AskUserQuestion open and S2 was waiting on a permission when Herdr died."
- Herdr only: both still wait (a blocked process keeps running, never retires). Restore: reattach the commodore (the question renders again in the attach); S2: Warn `needs-you` with fix `nimitz open F/SQ/S2` (or the commodore's peek-reply). Nothing is answered by nimitz.
- Machine died: the pending prompt is gone with the process; after the resume the session is idle at its last assistant message; the tool was never run. Report `prompt.lost` Info: "S2 was waiting on a permission; re-prompt it".
- Must not: answer prompts; type anything into a `blocked` pane (Herdr itself refuses `agent prompt` on a blocked pane).
- nimitz knows: `active.json` status `needs-you` + `waitingFor`; session file `status waiting` while live.
- Priority: should.

### R-21 Dropbox-synced fleet files changed on another machine

- Story: "On the laptop I added ship S9, renamed S2 to S2X and removed S3 in `squadron.toml`; Dropbox synced it overnight; the server fleet was running the old shape."
- Left: config: S9 (never started here), S2X (job row named `[F.SQ]-S2`, labels `F/SQ/S2`), S3 gone from config (job row + tab `F.SQ.S3` remain). Possibly `squadron (conflicted copy ...).toml` beside it, or a half-written file.
- Expected: restore reads the config first. S9: not-started, not restored (not in the active set; `up` starts it). S2X: no identity `F/SQ/S2X` in the registry; the old S2 job is a stowaway of this home carrying `F/SQ/S2` labels: reported `name.drift`/`labels.mismatch`, with fix `nimitz adopt <job> F/SQ/S2X` then `relaunch`; restore does not launch S2X cold. S3: R-13. Conflicted copy: `config.conflict` Warn (M23). Unparsable `squadron.toml`: that fleet refused with the parse error, other fleets restore (M8).
- Must not: edit config; cold-launch new members; guess that S2X is S2.
- nimitz knows: config vs registry vs job names; file mtimes.
- Priority: should.

### R-22 The daemon stopped by hand or died (Herdr intact)

- Story: "I ran `claude daemon stop` without `--keep-workers` (or the daemon OOMed). Every tab says the session ended; the Herdr shape is intact."
- Left: all workers dead; attach clients exit (each lead tab: a shell, or the agent view if the client dropped into it); rows `working`/`done`; docs: a restarted daemon re-adopts live workers only, dead ones are resumed "the next time the agents view opens" (2.1.196) or on attach.
- Expected: leads resumed by id (crash window first) and attached in their existing tabs; active ships resumed; `--all` for the rest. Attach alone also wakes a job (cli-07): for a lead with a shell tab, prefer **attach-wakes** over resume+attach (one step, no race); verify SessionStart `resume`.
- Must not: resume a job the daemon is already restarting (crash window); type into a tab that shows the agent view (R-24).
- Priority: must.

### R-23 Low-memory sweep stopped pinned leads

- Story: "Memory ran out; the daemon stopped idle sessions, pinned leads included; my attached tabs dropped to a shell."
- Same as R-02 (parked lead, shell tab). Restore: attach in the tab wakes it. The daemon's "possibly low memory" refusal on start is surfaced as the step error, not hidden.
- Priority: should.

### R-24 A lead tab left in the agent view

- Story: "I typed `/exit` in the commodore's attach; the tab now shows the machine-wide agent view with the cursor on some other fleet's row."
- Left: commodore live-unattached; tab pane = agent view, plus a transient spare session in the lead folder (a stowaway while the view is open; the lead folder is exclusive: `lead.intruder` would fire: it must be ignored as `spare: true`, section 6).
- Expected: the tab counts as **agent-view** (not a shell): never written to. Report `tab.agentview` with fix "close that pane (or Esc/ctrl-c until a shell), then `nimitz restore F`"; nothing else for that lead. Same for `/stop` (the job is then stopped on purpose: restore does not wake it; `nimitz start` does).
- Must not: type `claude attach` into the view (it would land in the dispatch input and create a new session).
- nimitz knows: Herdr `agent list` shows the pane with an agent whose session is the spare (`spare: true` in the session file) or no session; the title.
- Priority: must (never type), should (the report).
- Built (fix after 0.4.0): Herdr names no session for an attach pane at all (an attach client runs no Herdr hook: `agent` claude, no `agent_session`), so a lead's own attach pane is read from its foreground: `claude attach <job>` shows that job's session (`evidence::session_of_attach`) for `status`, Admiralty, `open`, `A` and a restore. The agent view with no claim then reads as the attach, the limit contracts.md names; a claim the attach does not back still never shows the attach's member (test `an_attach_pane_herdr_never_claimed_still_shows_its_lead`).

### R-25 A lead's job removed under a live tab

- Story: "I pressed ctrl-x twice on the commodore in the agent view (or `claude rm`), its tab dropped to a shell."
- Left: no job row, transcript kept, pin entry stale, tab shell.
- Expected: resume with every flag (`--bg --resume <session> --name --settings`, section 5: the row is gone), same id, attach in the tab, re-pin. Lineage `via resume`.
- Must not: cold-launch; leave the stale pin as the only pin (re-add is idempotent).
- Priority: must.

### R-26 A copy already exists from an earlier race

- Story: "Yesterday a restore raced the daemon and left `[F.SQ]-S1` twice: the real job and a copy (no labels, `respawnFlags []`)."
- Expected: restore refuses to touch S1 until the copy is gone: `name.taken` Error, fix `nimitz sweep F/SQ/S1` (stops and removes copies after one confirm). If the copy's pane is attached in a tab named `F.SQ.S1`, that tab is `tab.taken` (R-14). A copy that restore itself created in this run is removed in the same run (section 5).
- Must not: attach the copy as the ship; resume S1 while the copy is live (a second copy).
- Priority: must.

### R-27 No active set recorded (first restore since install, or `active.json` lost)

- Story: "`NIMITZ_STATE` was wiped / this is the first 0.4.0 restore after a reboot."
- Expected: leads restored (always); ships: none without `--all`; the output says "no active set recorded; ships stay parked (`--all` wakes every ship with a job row)". Tabs for leads: by name or created.
- Priority: must (graceful), nice (hint text).

### R-28 Active set stale: a member stopped on purpose after the last write

- Story: "I ran `claude stop` on S2 by hand at 23:00 because it was looping; `active.json` from 22:00 says it was live."
- Expected: S2 is **stopped**, not parked (SessionEnd from its pid with labels): restore leaves it, says "S2 stopped by `claude stop` at 23:00; `nimitz start F/SQ/S2` wakes it". `nimitz stop` keeps `active.json` current itself; the hook's SessionEnd (reason `other` from a stop) updates it too.
- Must not: wake a member someone stopped on purpose, even with `--all`? Decision: `--all` wakes parked **and** stopped ships (it is the explicit "everything" switch); the default wakes neither stopped nor out-of-set.
- nimitz knows: events tail per job; `active.json` write time.
- Priority: must.

### R-29 nimitz run in the wrong Herdr session, or outside Herdr

- Story: "I have two Herdr sessions (`NIMITZ` for fleets, another for work). I ran `nimitz restore` from the other one." Or: "I ran it over ssh with no Herdr."
- Left: nothing lost; the shape exists in session `NIMITZ`.
- Expected: outside Herdr (`HERDR_ENV` not 1): refuse `mux.none` with the fix ("run inside Herdr; or `--jobs-only` to resume the jobs without tabs"). In another Herdr session: the recorded Herdr identity (session name / socket) differs from the current: refuse `mux.other` "the fleet's tabs live in Herdr session NIMITZ; run restore there (or `--here` to rebuild the shape in this session)". Note the E22 hazard: the Herdr CLI may resolve a named session's socket regardless of env; nimitz must take the identity from `herdr status`, not from env alone.
- Must not: rebuild every tab in the second session, attaching each lead twice.
- nimitz knows: must record the Herdr identity in `active.json` at every write; must read it from `herdr status` (`session` / socket path) at restore.
- Built (P9): the identity is the env (`NIMITZ_HERDR_SESSION`, passed as `--session`; else the pane's `HERDR_SESSION` / `HERDR_SOCKET_PATH`), not `herdr status`: the shape nimitz reads and the identity it records come from the same `herdr` invocation in the same env, so they name the same server by construction, and the E22 hazard is accepted on that ground. `--jobs-only` and `--here` as above.
- Built (fix after 0.4.0): outside Herdr (no `HERDR_ENV`, nothing explicit) the user's own `nimitz` and Admiralty use the server the user's commands recorded in `active.json` (`active::user_mux_vars`), so a restore from a plain terminal runs in session `NIMITZ`; with nothing recorded it refuses `mux.none` as above; a recorded server that is not reachable refuses `mux.failed` with the fix that names it and `--jobs-only` (cli test `the_users_nimitz_outside_herdr_uses_the_recorded_server`).
- Priority: should.

### R-30 A member's folder gone, moved, or untrusted

- Story: "Dropbox renamed the ship folder (`ship1 (1)`), or I `git init`ed in it, or the mount was not up yet when I restored."
- Left: job row with `cwd` pointing at the old path; transcript keyed by the old cwd; trust keyed at the new git root (`Workspace not trusted`, cli-29).
- Expected: `folder.missing` Error ("cwd ... does not exist; mount it or `nimitz ship set F/SQ/S1 --dir ...`"), no launch; `folder.untrusted` Error with the fix "run `claude` once in <git root> and accept the trust prompt" (checked before launching, from `~/.claude.json`, env.rs `trust_keys`). Moved folder and a new `--dir`: the transcript stays findable by id across project dirs (E10), so a resume works after the config fix; the tab's cwd is the new path.
- Priority: should.

### R-31 Remote Control reconnects on resume

- Story: "Months ago I drove S1 from my phone once. Restore resumed it and it is on claude.ai again."
- Expected: after every resume, read the live session file: `bridgeSessionId` non-null -> `rc.exposed` Warn with the disconnect steps (M9); restore's summary lists it. Nice: `--no-rc` refuses to resume such a member and names it.
- Priority: should (the Warn is already planned), nice (the refusal).

### R-32 A ship's own tab opened by me, then the ship was relaunched (session id changed)

- Story: "`F.SQ.S1` tab was attached; the commodore relaunched S1 (`claude rm` + resume with flags, same id) / S1 `/clear`ed in its attach (new session id under the same job)."
- Left: the attach client follows the job, so the pane keeps showing the job: its `agent_session` now reports the **new** session id (after `/clear`) while the registry `session` may be the old one until the hook line lands.
- Expected: match by **job**, not only by session: Herdr reports the pane's session id; nimitz maps it to a job through the session files (`jobId`) and compares jobs. A pane whose session belongs to the member's job is the member's tab.
- Must not: call the tab foreign because the session id moved.
- nimitz knows: session file `jobId` for every live session (section 4 joins by job id already).
- Priority: must (0.4.0's whole point: identity = job).

### R-33 Herdr restored a tab with a plain shell that someone already reused

- Story: "After the Herdr restart I started `yazi` (or my own `claude`) in what used to be the `F.SQ.COMMODORE` tab before running restore."
- Expected: the tab is a name match whose only pane has a non-shell foreground (or a foreign agent): `tab.taken`; restore creates nothing for the commodore and gives the fix ("quit yazi in tab <id>, or rename the tab, then restore"). With `--force`: a new tab + `tab.dup` Warn.
- nimitz knows: `pane list` foreground title/cwd; `agent list` for agents. A shell is: no agent and a foreground process that is a shell (zsh/bash/fish) — nimitz needs that signal from Herdr (`foreground_cwd` is not enough; the foreground command name is; check what `pane list` gives).
- Priority: must (never type into a taken pane), should (the fix text).

### R-34 Sleep / resume of the laptop, VPN flap

- Story: "I closed the lid; next day everything is as I left it, or the daemon's auth refresh failed overnight."
- Left: docs: sleep alone stops nothing; attach clients survive; a failed credential refresh shows per job in the agent view.
- Expected: restore = R-12 (nothing to do); an auth failure shows as the job's `detail` in Info.
- Priority: nice.

### R-35 The admiralty hub's own workspace

- Story: "I keep the `ADMIRALTY` workspace with the overlay plugin and a `[ADMIRALTY]` session from the old fleet skill."
- Expected: never a restore target; never matched by loose label against any fleet (`normalize_label("[ADMIRALTY]") = ADMIRALTY` only collides with a fleet named ADMIRALTY: refuse that fleet name at `fleet new`).
- Priority: must (one line of exclusion).

## 2. Scenario matrix

| id | scenario | jobs | Herdr shape | restore does | priority |
| --- | --- | --- | --- | --- | --- |
| R-01 | overnight, leads attached | ships parked | intact | nothing (report parked) | must |
| R-02 | overnight, lead detached/retired | lead parked | shell tab | attach in tab (wakes) | must |
| R-03 | Herdr crashed | all live | names restored, shells / empty | attach leads, active ships; create missing | must |
| R-04 | lead pane killed | live | tab gone/shell | attach | must |
| R-05 | tab closed | live | tab missing | create tab, attach | must |
| R-06 | workspace closed | live | ws missing | create ws, claim root tab, attach | must |
| R-07a-d | renamed / moved / duplicate / split | live | odd | match by session/job; report | must/should |
| R-08 | reboot | all dead, rows kept | shells | resume leads + active, attach; crash window; `history.lost` on `startup` | must |
| R-09 | CC upgrade adopt | live | intact | nothing; Info on old binaries | should |
| R-10 | restore interrupted | mixed | half-made | converge; claim unlabelled root tab; remove own copies | must |
| R-11 | two fleets | - | two shapes | scope to the fleet asked | must |
| R-12 | run twice | - | - | zero steps | must |
| R-13 | member removed, tab stays | gone | orphan tab | ignore + Info | should |
| R-14 | tab attached to wrong job | - | foreign | `tab.taken` Error + fix | must |
| R-15 | my own claude in the workspace | - | foreign/unrelated | ignore | must |
| R-16 | 0.3.x interactive lead | no job | attached (interactive) | refuse until /exit; then resume with flags | must (migration) |
| R-17 | transcript lost | parked | any | `history.lost`, no launch, no fleet-wide block | must |
| R-18 | other machine | none here | none | refuse "runs on <m>"; point at `up` | should |
| R-19 | ship mid-turn | live or `working` dead | - | leave / resume after window; `turn.interrupted` | must/should |
| R-20 | prompt pending | blocked or dead | - | reattach lead; `needs-you`; `prompt.lost` | should |
| R-21 | config changed on another machine | mismatch | - | no cold launches; drift/conflict reports | should |
| R-22 | daemon stopped/died | all dead | intact | attach-wakes leads; resume active ships | must |
| R-23 | low-memory sweep | leads parked | shells | attach-wakes | should |
| R-24 | tab in agent view | live | agent-view | never type; report | must |
| R-25 | lead job rm'd | gone | shell | resume with every flag; re-pin | must |
| R-26 | copy from an old race | dup | maybe taken | `name.taken` -> sweep first | must |
| R-27 | no active.json | - | - | leads only; say so | must |
| R-28 | stale active set, stopped on purpose | stopped | - | leave stopped (not `--all`) | must |
| R-29 | wrong Herdr session / no Herdr | live | elsewhere | refuse `mux.other` / `mux.none` | should |
| R-30 | folder gone / untrusted | - | - | `folder.missing` / `folder.untrusted` | should |
| R-31 | Remote Control reconnects | live | - | `rc.exposed` after resume | should |
| R-32 | session id moved under the job | live | attached | match by job | must |
| R-33 | restored shell reused | live | taken | `tab.taken` | must |
| R-34 | sleep | live | intact | nothing | nice |
| R-35 | ADMIRALTY workspace | - | - | never a target | must |

## 3. What nimitz must record before a loss

`NIMITZ_STATE/active.json`, one writer under `registry.lock`, rewritten (not appended) on
`status`, `layout`, `open`, `restore`, `start`, `stop`, `relaunch`, `rm` and on every
hook SessionStart/SessionEnd of a member (the hook already holds the lock to write the
registry; the extra write is one small file):

```json
{"version": 1, "at": 1791000000.0, "machine": "m1",
 "herdr": {"session": "NIMITZ", "socket": "/home/u/.config/herdr/sessions/NIMITZ/herdr.sock", "server_pid": 418777},
 "members": {
   "F": {"job": "9cdceb2d", "session": "...", "status": "idle", "attached": true,
         "place": {"workspace_id": "w3", "workspace": "F", "tab_id": "w3:t1", "tab": "F.ADMIRAL", "pane_id": "w3:p1"}},
   "F/SQ/S1": {"job": "...", "session": "...", "status": "busy", "attached": true, "place": {...}},
   "F/SQ/S2": {"job": "...", "session": "...", "status": "idle", "attached": false, "place": null}}}
```

- Members present = live at the last write (the "active set"). Absent = parked, stopped
  or never started; the event log says which. A member stopped through nimitz is removed
  at once; a `claude stop` by hand is caught by the hook's SessionEnd.
- `status` = the last status word (busy / idle / needs-you + `waitingFor`): R-19, R-20.
- `place` keeps **labels and ids**: ids for a fast path while the server lives, labels for
  after a restart (ids are not stable across a Herdr restart). A ship with `place` is
  one I had opened: R-03 reattaches exactly those.
- `herdr` identity for R-29, read from `herdr status` (never only from env).
- Already recorded elsewhere and needed: registry `job`/`session`/lineage; events.jsonl
  SessionEnd per pid (parked vs stopped); `pins.json` (leads must be pinned);
  `jobs/<id>/state.json` (`state`, `resumeSessionId`, `respawnFlags`, `cliVersion`);
  session files (`jobId`, `status`, `bridgeSessionId`, `spare`); daemon.log `bg retire` /
  `bg settled` / `bg adopt` lines (why a member is parked; the crash window).
- Restore's own progress: `NIMITZ_STATE/restore.json` (run id, started, steps done,
  steps left) so an interrupted run reports "3 of 7 done" and the next run converges
  (R-10). Removed on a clean finish.
- The `pending/<id>.json` records of 0.3.x must not survive into restore (env-G3): a
  pending record is dropped when its pane is gone, has no agent, or is older than the
  bind timeout.

## 4. Decisions for the implementer

1. **One engine.** `restore`, `layout herdr` and `up` are three entry points to one
   plan: `shape(fleet) -> diff(Herdr now, jobs now, active set) -> steps`. Steps are of
   five kinds only: `workspace create`, `tab create|rename` (claim), `pane run <claude
   attach>` into a shell pane, `claude --bg --resume <id>` (section 5), `pin`. Plus
   reports. Nothing else ever runs.
2. **`restore [F] [--all] [--force] [--jobs-only]`** = leads + active set (default);
   `--all` = every member with a job row, parked or stopped, plus their recorded tabs;
   `--force` = open a fresh tab beside a taken one (`tab.dup` Warn); `--jobs-only` = no
   Herdr (resumes only, for ssh). Never a cold launch: a member with an empty lineage is
   reported "not started; `nimitz up F` or `nimitz start <id>`".
3. **`up F`** = `restore F --all` **plus** cold launches of members with an empty lineage,
   in chain order (admiral, commodores, ships). `up` is the only one of the three that
   spends a first prompt. Keep the name; define it in docs as "restore everything and start
   what never ran".
4. **`layout herdr [F] [--ships]`** folds into restore: it is `restore F --jobs-only=false
   --no-resume` (shape only: tabs for leads, attach where the job is live, no resume, no
   launch; `--ships` adds tabs for every live ship). Keep the command as an alias for one
   release with a one-line note; the TabKind::Start path (a tab that runs the start
   command) goes away: in 0.4.0 a tab only ever runs `claude attach`, so there is no
   "pane step" and no bind window for leads (nimitz-G2/G3 disappear by construction).
5. **Matching order for a member's tab**: (a) a pane showing a session of the member's
   **job** (session files map session -> job), anywhere: use it, no step; (b) a tab with
   the full name in the member's workspace whose pane is a shell: claim it; (c) a tab with
   the full name in any workspace, shell: claim it there (R-07b); (d) a legacy short name
   (`COMMODORE`) as today: rename + claim; (e) an unlabelled root tab of a workspace nimitz
   just created: rename + claim; (f) create. A name match whose pane is not a shell is
   `tab.taken` (R-14, R-24, R-33), never (f) unless `--force`.
6. **Attach wakes.** For a lead whose job is parked and whose tab is a shell, one step:
   `pane run claude attach <job>`; verify the SessionStart `resume` within the bind
   timeout. Use `--bg --resume` first only when there is no shell pane to attach from
   yet (then attach after the resume is verified). Never both in a way that overlaps.
7. **Crash window and copies** exactly as section 5: wait for `crashed`/`working`-no-pid
   rows; read the launch output; a copy note fails the step and the copy is stopped and
   removed in the same run; a pre-existing copy is `name.taken` -> `sweep`.
8. **A shell pane** needs a Herdr signal nimitz does not read today: the pane's
   foreground command (not only `foreground_cwd`). Check `herdr pane list` / `pane get`
   fields; if absent, treat "no agent + pane title equals the shell's default title" as
   shell, and anything else as taken. Write the rule down in `contracts.md` section 9.
   Answered (herdr-shape.md): `herdr pane process-info --pane <id>` gives
   `foreground_processes[].argv`, so a shell pane and a `claude attach <job>` pane are
   both readable while live; `herdr api snapshot` gives the whole shape in one call.
9. **Herdr identity**: record and compare (R-29). If `herdr status` has no stable
   identity, use the socket path.
10. **Exit codes and words**: `[OK] F: restored 2 lead(s), 1 ship(s); 3 parked` /
    `[OK] F: nothing to restore` / `[ERROR]` only when a member could not be restored;
    every such member has exactly one fix line, and the fix is a nimitz command or one
    Herdr action I take by hand (close a pane, ctrl-z, /exit).
11. **Admiralty**: a restore action per fleet row (`R`), which runs `nimitz restore F
    --plan`, shows the plan, then `--yes`; the same words as the CLI (M25).
12. **Tests**: every R-id above becomes a fake-Herdr world (`world.json` + fake job rows +
    `active.json`) with the expected call list; R-01/02/04/05/08/12/22 also real
    Claude Code e2e (`NIMITZ_E2E=1`) without Herdr (the fake mux records the attach
    commands; no live server, per section 13); the by-hand checklist
    (`docs/test/restore-checklist.md`) covers R-03, R-07, R-24, R-29, R-33, which need
    a real Herdr.
