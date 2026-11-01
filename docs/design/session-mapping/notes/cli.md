# Area `cli`: by-hand `claude` CLI actions vs the nimitz identity -> session map

Tested on Claude Code **2.1.292** (installed; nimitz warns `cc.version`, tested up to 2.1.290) and nimitz 0.3.1.
World: `$MAP=~/.cache/nimitz-e2e/nimitz-e2e-map-cli-647776`, fleet `NIMITZ_E2E_CLI`, ship `NIMITZ_E2E_CLI/SQ/S1`
in `$MAP/ship1`, launched by `nz start` as job/session **9b737eb1** (`9b737eb1-d120-4aea-9483-3ff89aa69000`),
name `[NIMITZ_E2E_CLI.SQ]-S1`. The project settings ask for haiku, but the org restricts haiku, so every turn
ran on claude-opus-5-5. Only about 4 tiny turns were sent.

Key fact for everything below: the job's `~/.claude/jobs/<id>/state.json` carries `respawnFlags` (the original
`--name` and `--settings` with the labels), `sessionId` (the original id) and `resumeSessionId` (the id the job
continues now). Claude Code uses `respawnFlags` for every wake (stop/start, respawn, attach, daemon crash
restart). This is why most out-of-band wakes keep the labels.

## 1. Case tables

### Case 1: lifecycle commands on the ship

| case | expected | observed | verdict |
| --- | --- | --- | --- |
| 1a `claude stop 9b737eb1` | ship stopped, same identity, `nz start` resumes it | CC: "stopped 9b737eb1"; row `state done`, pid null; SessionEnd `reason other` with labels. nz: `Stopped` ("job row ... no live pid", "SessionEnd event, reason other"), no issues. `nz start` ran a native `--bg --resume <id>` with no flags: same id, SessionStart `source resume` with labels, lineage +`resume`. | OK |
| 1b `claude kill 9b737eb1` | same as stop | `kill` is an alias of `stop` (`stop\|kill <id>` in `--help`). Output and evidence are the same as 1a. | OK |
| 1c `claude rm 9b737eb1` while stopped | row gone, transcript kept, `nz start` resumes with history | CC: job dir deleted, transcript kept. nz: `Stopped` "no job row; its session's transcript is there (...jsonl)", no issues. `nz start` planned `--bg --resume 9b737eb1 --name ... --settings ...` ("its job row is gone; Claude Code keeps the id"). Same id, history kept, labels and name back, lineage +`relaunch`. | OK |
| 1d `claude respawn 9b737eb1` (own id; live, and again while stopped) | same id, labels kept | Live: "respawned 9b737eb1", new pid, same id, SessionStart `resume` with labels (from `respawnFlags`). Stopped: it wakes the session the same way. nz: `Idle`, no issues. The registry gets no lineage line, which is harmless because the id is the same. | OK |
| 1e-i `claude attach 9b737eb1`, then `/exit` | detaching never stops the bg session | `/exit` does **not** end the session: it drops to **agent view**, which lists *every* session on the machine (the live fleet too). While agent view was open, CC pre-spawned a **spare** bg process (`claude bg-spare`, session file `spare: true`, sessionId 0e73561a, `cwd` = the attach cwd `$MAP`, not in `claude agents`). Two ctrl-c presses closed agent view, and the spare went away. Ship pid unchanged. The text `/exit` was written into the ship's transcript as a user line. nz: `Idle`, no stowaway (spare not shown). | OK (note the spare and the agent-view exposure) |
| 1e-ii attach, then ctrl-c x2 / ctrl-z / kill the terminal | session keeps running | Two ctrl-c presses at the prompt: nothing (session stays attached). ctrl-z: `attach` exits 0 to the shell, session live. tmux `kill-session` while attached: session live, no events. nz: `Idle`. | OK |
| 1e-iii `claude attach` on a **stopped** session | wakes it as itself | "Waking session 9b737eb1...": same id, SessionStart `resume` with labels, new pid. nz `Idle`, no issues. | OK |
| 1f-i `claude --bg --resume <sid>`, no flags, row exists, **stopped** | native resume | "woke session 9b737eb1 with its saved options (--name, --settings)". Same id, labels. | OK |
| 1f-i' the same, **live** | refuse, or attach | "already running in the background, so this started a copy as **4a13c3dc**". The copy has a new id, **the same name** in its session file (none in its job `state.json`), **no labels**, `respawnFlags: []`, SessionStart `source fork`. nz: copy = **STOWAWAY**; S1 gets `name.taken` (Error, "fix: nimitz stop 4a13c3dc"); `nz start` and `nz relaunch` refuse with name.taken. `nz sweep S1` = "nothing to do" (it does not drop an unlabelled copy). After `nz stop 4a13c3dc` the stopped copy stays a stowaway until `nz rm 4a13c3dc`. | GAP |
| 1f-ii `--bg --resume <sid> --name X-CLI-MAP` while stopped (row exists) | (user intent: rename) | "keeps its own saved options, so the flags you passed started a copy as **ea341099**". The copy is named X-CLI-MAP, has no labels, `source fork`, and **has the conversation history** (attach showed the earlier "reply ok" turn and the `/exit` line). nz: S1 `Stopped`, copy = stowaway. `nz start S1` resumes 9b737eb1 cleanly. The copy stays until `nz rm`. | GAP (the copy is left behind; history is copied too, against cc-control-model §6 "that copy has no conversation history") |
| 1f-ii' `--bg --resume <sid> --permission-mode default` while **live** | | Copy **51c5cdc7**, **same name**, no labels: `name.taken` Error on S1 plus a stowaway, as in 1f-i'. | GAP |
| 1g `kill -9 <worker pid>` (crash, no SessionEnd) | nz shows it stopped; `nz start` recovers it | CC: the job `state.json` gets `state crashed`; the session file stays (dead pid). **The daemon restarts the crashed session by itself about 15 s later** (same id, labels, SessionStart `resume`). Meanwhile nz: `Stopped` with reasons "state crashed" and a **stale** "SessionEnd event, reason other" (from an earlier pid). | OK on its own |
| 1g-race `kill -9`, then `nz start` within about 10 s | one ship | **BREAK.** `nz start` ran the native `--bg --resume 9b737eb1`. CC still counted the session as running (the daemon was restarting it), so it **started a copy ed8fea31** (same name, no labels, `fork`). Then the daemon restarted 9b737eb1 (pid 722798, `resume`, labels). nimitz printed **`[OK] ... backgrounded · ed8fea31`**, bound the daemon's SessionStart for 9b737eb1 as its own (lineage +`resume`), and reported success. Status then: `name.taken` Error plus a stowaway ed8fea31 with the identity's name. | BREAK |

### Case 6: other commands and flags that create, end, rename or fork a session

All of `claude --help` and the subcommand `--help` pages were read. Session-relevant items:
`--bg`, `-r/--resume [id|name]`, `-c/--continue`, `--fork-session`, `--session-id`, `-n/--name`,
`--no-session-persistence` (only with -p), `--from-pr`, `--teleport`, `--cloud`, `--remote-control`, `-w/--worktree`,
the subcommands `stop|kill`, `rm`, `respawn [id]|--all`, `attach <id|name>`, `logs`, `purge`, and `agents` (agent
view: ctrl+x deletes a row). In-session: `/rename`, `/clear`, `/exit`.

| case | expected | observed | verdict |
| --- | --- | --- | --- |
| 6a `--bg --session-id <uuid>` | | "warning: --bg manages the session id; ignoring --session-id" (probe 1 still holds on 2.1.292). Made a fresh unlabelled stowaway 1e271213 in the ship folder. `nz adopt 1e271213 S1` while S1 was live: refused `dup.live`. | OK (as designed) |
| 6b `--bg --resume <sid> --fork-session` while stopped | a copy | Copy c5086109, **same name**, no labels, `fork`. With S1 stopped, `nz start S1` **refuses** with `name.taken` (fix: `nimitz stop c5086109`). `nz adopt c5086109 S1 --plan` would bind it and warn `labels.missing`. | GAP (a by-hand fork blocks the ship's own start) |
| 6c `--bg -c` (continue) in the ship folder | | "started a copy of that conversation as 37e6289f": same name, no labels, `fork`. The same as 6b. | GAP |
| 6d `--bg --resume "[NIMITZ_E2E_CLI.SQ]-S1"` (resume **by name**) while stopped | | "started a copy of that conversation as 816d25aa". **Name** = its short id, no labels: a stowaway. A resume by name never continues the identity. | GAP (low) |
| 6e `claude -p --resume <sid> "reply ok"` while **stopped** | | Runs in the foreground with the **same id** and appends a turn to the ship's transcript. Hook lines have **no labels** (SessionStart `resume`). nz: `labels.missing` **Error** ("resumed with flags, or legacy"; fix relaunch), although the row's `respawnFlags` still hold the labels. `nz start` then said "will repair", ran `claude rm` + resume with every flag: same id, history kept. | OK (recovers), wording GAP (low) |
| 6f `claude -p --resume <sid>` while **live** | refuse | "Error: That session is running in the background (9b737eb1) ... Add --fork-session". But the refused process **still fired a SessionEnd hook for 9b737eb1** (pid 747469, no labels). nz status was unaffected (the session file wins). | OK, stray-event note |
| 6g interactive `claude --resume <sid>` (TUI, foreground) while stopped | | Same id, `kind interactive`, no labels. **Its name became `[HOME.GITHUB]-NIMITZ_DEV`**, the live parent session's name, inherited from the env the harness leaks (see the Gaps section, G0). The transcript got a `custom-title`/`agent-name` line "[HOME.GITHUB]-NIMITZ_DEV". nz: `Idle` with `name.drift` + `labels.missing` Errors. After `/exit` (SessionEnd `prompt_input_exit`): `Stopped`, `labels.missing` only (the name drift in the transcript is not seen). `nz start` = rm + resume with flags: repaired, and a new custom-title was written. | OK for nz; **harness hazard** |
| 6h `/rename RENAMED-CLI-MAP` in an attached ship | | The name changes live. **`--name` is removed from `respawnFlags`**; the name now sits in `state.json` with `nameSource user`. nz live: `name.drift` Error (fix relaunch). **Once stopped: no issue at all** (status shows the name RENAMED-CLI-MAP but does not flag it). `nz start` did a native resume and only *afterwards* said `name.drift`. `nz relaunch` (stop, rm, resume with flags) repaired it: same id, right name. | GAP |
| 6i `/clear` in an attached ship | the identity follows the new conversation | **BREAK.** In the same process (same pid, same job 9b737eb1) the session id changes to **533ea11e**. SessionEnd `reason clear` and SessionStart `source clear` both carry the labels. The job row's `sessionId` stays 9b737eb1; `resumeSessionId` becomes 533ea11e; the session file shows 533ea11e. nimitz: the registry is **not** rebound (current stays 9b737eb1). Status says **`Stopped`** ("job row 9b737eb1 with no live pid") while the ship is live and idle, and lists the live session as an `extra_row` (`copy.rows` Warn, fix sweep). Every path is then refused: `nz start` (dup.live, fix "nimitz stop 9b737eb1 # or nimitz adopt 533ea11e"); `nz adopt 533ea11e S1` ("not a stowaway", session.unknown); `nz stop S1` ("is not live", session.none); `nz stop 9b737eb1` ("not a live background session"); `nz relaunch` (dup.live); `nz sweep` (skips the live session). `nz check S1`: **"no errors"**. Only a by-hand `claude stop 9b737eb1` unlocked it. Then `nz start` (native resume) woke the job, CC continued **533ea11e**, and nimitz said `lineage.broken` ("expected 9b737eb1 but bound 533ea11e (a copy; flags on the resume?)"). Only `claude rm 9b737eb1` + `nz start` got a clean ship, back on the **pre-clear** conversation 9b737eb1; the post-clear session 533ea11e was orphaned. | BREAK |
| 6j `claude purge <ship dir>` | | Not run: `--dry-run` shows it also edits `~/.claude.json` (the projects entry) and `~/.claude/history.jsonl`, which the safety rules forbid. Simulated instead (row removed, transcript moved away): nz `Stopped`, "registry knows it, but no row and no transcript", `lineage.broken` Error with fix `nimitz start S1 --force # cold launch; lineage starts over`; `nz start` refuses without `--force`. | OK (handled) |
| not run | | `--from-pr`, `--teleport`, `--cloud`, `--remote-control` (cloud or PR sessions, not local bg jobs); `-w/--worktree` (ship1 is not a git repo); `--no-session-persistence` (only with -p, makes nothing resumable); `respawn --all` and agent-view ctrl+x (would touch the live fleet). | n/a |

## 2. Gaps

**G0 (harness; high, safety).** `mapenv.sh` `cl` and `clean` remove `NIMITZ_*`/`HERDR_*`/`CLAUDE_CODE_MESSAGING_SOCKET`
but **not** `CLAUDE_CODE_SESSION_ID` (= the parent 9aacf927), `CLAUDE_CODE_CHILD_SESSION=1`, `CLAUDE_CODE_MESSAGING_TOKEN`,
`CLAUDE_CODE_EXECPATH` (still 2.1.290) and others. An **interactive** `claude --resume` started from this env took the
live parent's name `[HOME.GITHUB]-NIMITZ_DEV` (session file `name`, plus a custom-title line in the test transcript).
For about 40 s a test session was addressable under a live ship's name. I ended it at once, and `nz start` wrote the right
name back. I did not narrow down which variable does it. Background launches and print mode did not show it
(`--name` was given, or no name was set). Repro: from inside a fleet session, run `env -u NIMITZ_ID ... claude --resume <own stopped sid>`
in tmux and read `~/.claude/sessions/<pid>.json` `name`. Fix: drive interactive claude with `env -i HOME PATH TERM ...`
(what I did afterwards), or unset every `CLAUDE_CODE_*` in `clean`. The brief's example `cl claude agents --json` is
also wrong: `cl` already runs `claude`, so it runs `claude claude agents ...`, which is a **print-mode prompt in the
current cwd** (see Cleanup). Use `cl agents --json --all`.

**G1 /clear changes the session id under a live job (BREAK, high).** Trigger: `/clear` in an attached ship, or a
ship that runs `/clear` itself. Effect: nimitz shows a live ship as Stopped. `start`, `stop`, `relaunch`, `adopt` and `sweep`
all refuse, and `check` says no errors; the suggested fixes contradict each other. Recovery needs a by-hand `claude stop` +
`claude rm` and loses the post-clear conversation. Evidence: events `SessionEnd 9b737eb1 reason clear` and then
`SessionStart 533ea11e source clear` with `NIMITZ_ID=.../S1` from the same pid 763676; `state.json`
`resumeSessionId=533ea11e`. Repro: `nz start S`, `claude attach <job>`, type `/clear`, press ctrl-z, run `nz status`.

**G2 crash and `nz start` race makes a copy, and nimitz reports OK (BREAK, med-high).** Trigger: a worker dies
(kill -9, OOM, a crash) and someone runs `nz start` (or the commodore revives the ship) before the daemon's own restart, which
came about 10-15 s later here. Effect: an unlabelled copy with the identity's name (`name.taken`), and the start output says OK.
Evidence: `[OK] ... backgrounded · ed8fea31`, where the expected short id is 9b737eb1; CC's note "already running ... started a copy as
ed8fea31" was ignored; lineage +`resume` 9b737eb1 came from the daemon's restart. Repro: `kill -9 <pid>; sleep 1; nz start S --yes`.

**G3 by-hand resumes and forks leave copies (GAP, med).** `--resume` with flags while the row exists, `--resume`
while live, `--fork-session`, `-c`, and resume by name all make an unlabelled copy (`source fork`). In most of them the copy
**keeps the identity's name** in its session file, so it blocks `nz start`/`nz relaunch` (`name.taken`), even when the
ship itself is stopped (6b). `nz sweep` ignores these copies; the user has to `nz stop` + `nz rm` them by id. On 2.1.292 the copy also carries the
conversation history (seen in attach), so it is a real fork, not an "empty copy".

**G4 /rename survives a native resume (GAP, med-low).** `/rename` strips `--name` from `respawnFlags`. While the ship
is stopped, nz shows no issue. `nz start` does a native resume (wrong name), *then* reports `name.drift`. It should
treat a stopped row whose `state.json.name` differs from the identity's name like `labels.missing`: rm + resume with flags.

**G5 misleading evidence text (low).** (a) A `Stopped` reason quotes the last SessionEnd for the session id even when it came from an
earlier pid (1g showed "SessionEnd reason other" for a crash). (b) A refused `claude -p --resume` on a live session
still emits a SessionEnd with no labels. (c) `labels.missing` says "resumed with flags" after a `-p --resume` (no flags);
`respawnFlags` still had the labels, so a native resume would have been enough. (d) `claude agents --json` `state`
(`blocked`) differs from `state.json` `state` (`working`); nimitz shows the latter.

**G6 agent view side effects (low, safety note).** `/exit` inside `claude attach` opens agent view, which lists and offers to
act on every session on the machine (ctrl+x deletes, enter opens), and starts a `bg-spare` process in the attach cwd.

## 3. Notes for the design

- **Rebind on `SessionStart source=clear` (and `compact`).** The hook line has the labels and the pid. If the
  pid is the identity's live pid, or the `session_id` equals the job's `resumeSessionId`, append to the lineage with
  `via clear` and move `current`. Also match rows by **jobId** (session file `jobId`, `state.json`
  `daemonShort`), not only by session id: one job can carry several session ids over time. `state.json.sessionId`
  is the *first* id; `resumeSessionId` is the one that matters.
- **Launch output is evidence.** After any `--bg` step, parse the printed short id and the CC notes
  ("started a copy as X", "the flags you passed started a copy"). If the short id is not the expected job, fail the step
  (and offer `nz stop X && nz rm X`). That alone closes G2.
- **Crash window.** A job `state.json` `state: crashed`, or a session file whose pid is dead while the row is still
  `working`/`running`, means the daemon is about to restart it. `nz start` should wait (about 20 s) for the daemon's
  SessionStart, or ask for `--force`, rather than resume now.
- **Name source.** Read `state.json.name`/`nameSource` and `respawnFlags`. `--name` missing from `respawnFlags`, or
  `nameSource user` with a different name, means a `/rename`. Plan rm + resume with flags at start time (G4).
  `respawnFlags` also lets you confirm the labels before calling something `labels.missing` (G5c).
- **Copies.** A `SessionStart source=fork` in the identity's cwd with the identity's name and no labels is almost
  certainly a by-hand copy of that identity. `nz sweep <id>` could list it (stop + rm, with confirmation) instead of
  "nothing to do". Ignore session files with `spare: true` (agent view pre-spawns); nimitz already does, in effect.
- **Harness.** Strip every `CLAUDE_CODE_*` (or use `env -i`) for any interactive claude a test drives.

## 4. Cleanup

- Job ids I created, all **stopped and `claude rm`'d / `nz rm`'d**: 9b737eb1 (ship; final `claude rm` before
  removing the world), copies 4a13c3dc, ea341099, 51c5cdc7, ed8fea31, c5086109, 37e6289f, 816d25aa, stowaway 1e271213.
  Spare 0e73561a went away when agent view closed. Interactive pid 749465 exited (`/exit`). `ls ~/.claude/jobs/<id>`:
  none left.
- tmux server `map-cli-647776` killed. `$MAP` and all ~60 empty `nimitz-e2e-map-cli-*` folders that each re-source of
  `mapenv.sh` created (it does `mkdir` with a new `$$` every time) were removed with `command rm -rf`.
- Transcript folders left for the user:
  - `~/.claude/projects/-home-user--cache-nimitz-e2e-nimitz-e2e-map-cli-647776-ship1/` (9b737eb1, 533ea11e,
    1e271213 + memory/). The 9b737eb1 transcript has one stale `custom-title "[HOME.GITHUB]-NIMITZ_DEV"` line (G0), superseded by a later correct one; deleting the folder removes it.
  - **Mistake:** `~/.claude/projects/-home-user-Github-claude-nimitz/11111111-2222-4333-8444-555555555555.jsonl`,
    an accidental print-mode session caused by `cl claude help agents` (= `claude claude help agents`) in the repo cwd. It
    ran one turn (prompt "claude") and exited. It also wrote 4 **unlabelled** hook lines (SessionStart/UserPromptSubmit/Stop/SessionEnd,
    pid 657362) to the live `~/.local/state/nimitz/events.jsonl`. It is harmless (ended, no labels), but you can delete it.
- Nothing in the repo was modified.
