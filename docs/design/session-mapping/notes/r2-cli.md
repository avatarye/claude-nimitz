# Area `cli` (round 2): by-hand `claude` CLI paths without nimitz, for every role

Tested on Claude Code **2.1.292** (installed; nimitz warns `cc.version`, tested up to 2.1.290), nimitz 0.3.1.
World `$MAP=~/.cache/nimitz-e2e/nimitz-e2e-r2-cli`, fleet **E2E2**, squadron SQ. Members:
ship **S1** = job/session `942bf763` (`942bf763-7b7b-4d1e-bd87-34f827a8583b`, folder `$MAP/ship1`, launched by `nz start`),
tender **~t1** = `45c8aa75` (launched by `nz tender E2E2/SQ t1 "reply ok"`, folder = the commodore's folder `$MAP/nimitz/e2e2/squadrons/sq`),
admiral = interactive `31686e97` and commodore = interactive `02d4c732`, both run in the private tmux `r2-cli` with the exact
command `nz start --plan` prints. The org turns haiku into Opus; about 8 turns were sent in total.
Full session uuids: commodore `02d4c732-4f3a-499d-8ddf-b4f49eab7127`, admiral `31686e97-b668-478b-9e39-d26e43880d03`,
tender `45c8aa75-f787-42f7-b284-75239ac787a5`.

Harness notes. (1) The Bash tool resets cwd between calls: one `--bg --resume` ran in the repo cwd by mistake (see Cleanup,
"mistake"). (2) `timeout 60 cl ...` cannot run a shell function; use `scrub timeout 60 claude ...`. (3) zsh does not split
`$var` in `for`: use `${=var}`. (4) `claude --add-dir` is variadic and swallows a prompt that follows it: put the prompt first.
(5) Under `env -i` the round-1 harness hazard G0 (an interactive resume taking the parent's name) did **not** occur (cli-18).

## 1. COVERAGE

| id | path | role | via | Claude Code does | nimitz shows | recovery | verdict | evidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| cli-01 R1:1a (same) | `claude stop <job>` | ship | by-hand | row keeps pid-less, `state.json` state `done` (idle ship) or `stopped` (busy ship); SessionEnd `other` with labels | `stopped` "job row with no live pid" | `nz start` = native `--bg --resume`, same id, labels, lineage +resume | OK | pids 1053118->none; events SessionEnd 942bf763 labels E2E2/SQ/S1 |
| cli-02 R1:1b (same) | `claude kill <job>` | ship | by-hand | alias of stop, identical | same | same | OK | "stopped 942bf763" |
| cli-03 R1:1c (same) | `claude rm <job>` while stopped, then `nz start` | ship | by-hand | job dir gone, transcript kept | `stopped` "no job row; its session's transcript is there" | `nz start` plans `--bg --resume <sid> --name --permission-mode --settings`; same id 942bf763 and history, lineage +relaunch | OK | Q1 table below |
| cli-04 R1:1d (same) | `claude respawn <job>` live and stopped | ship | by-hand | same id, new pid, SessionStart `resume` with labels (from `respawnFlags`) | `idle`, no issue; registry gets no lineage line | n/a | OK | pid 988519->1053118; 1067070->1073252 |
| cli-05 R1:1e-i (changed) | `claude attach`, then `/exit` | ship | by-hand | drops into **agent view** (whole machine). CC claims a `bg-spare` as session 28f3d8a7 (`spare: true`, cwd ship1, SessionStart `startup`, no labels). **Two ctrl-c did not close the view** (round 1: it did); I killed the tmux client. Ship unaffected | the spare is listed as a live **stowaway** 28f3d8a7 (round 1 said not shown; matches byhand G5) | gone when the view closes | GAP (low, byhand G5) | session file 1055780.json spare True; events SessionStart/SessionEnd 28f3d8a7 |
| cli-06 R1:1e-ii (same) | attach, ctrl-z; attach, kill the tmux client | ship | by-hand | ship keeps running, no events | `idle` | n/a | OK | pid unchanged |
| cli-07 R1:1e-iii (same) | `claude attach` on a **stopped** ship | ship | by-hand | wakes it as itself: same id, labels, SessionStart `resume` | `idle`; a previous `labels.missing` (cli-18) cleared because the newest SessionStart is labelled | n/a | OK | pid 1223884 labels E2E2/SQ/S1 |
| cli-08 R1:1f-i (same, + trust) | `--bg --resume <uuid>` no flags, stopped | ship | by-hand | "woke session 942bf763 with its saved options"; same id, labels | `idle` | n/a | OK | note: refused with **"Workspace not trusted"** right after I `git init`ed ship1 (cli-29) |
| cli-09 R1:1f-i' (same) | `--bg --resume <uuid>` no flags, **live** | ship | by-hand | "already running ... started a copy as dc74d940": new id, **same name**, no labels, `respawnFlags []`, SessionStart `fork` | S1 `name.taken` Error (fix `nimitz stop dc74d940`) + stowaway; `nz check` 1 error; `nz sweep S1` nothing to do | `nz stop` + `nz rm` the copy by hand | GAP (R1 G3) | copy dc74d940 pid 1023524 |
| cli-10 R1:1f-ii (same) | `--bg --resume <uuid> --name X-E2E2-CLI`, stopped | ship | by-hand | "keeps its own saved options, so the flags you passed started a copy as 19da599d" named X-E2E2-CLI, `fork`, flags `[--name X-E2E2-CLI]` | stowaway only (no name.taken); S1 `stopped`, check no errors | nz stop/rm the copy | GAP (R1 G3) | 19da599d |
| cli-11 R1:1f-ii' (same) | `--bg --resume <uuid> --permission-mode default`, live | ship | by-hand | copy a771417f, same name, flags `[--permission-mode default]`, `fork` | `name.taken` + stowaway | nz stop/rm | GAP (R1 G3) | a771417f |
| cli-12 R1:6a (same) | `--bg --session-id <uuid>` in ship1 | ship | by-hand | "warning: --bg manages the session id; ignoring --session-id"; fresh unlabelled session c49147e6 named by its short id | stowaway (fix stop+rm works) | nz stop/rm | OK (as designed) | c49147e6 |
| cli-13 R1:6b (same) | `--bg --resume <uuid> --fork-session`, stopped | ship | by-hand | copy 9022551c, **same name**, no labels, `fork`; no "copy" note printed | S1 `name.taken` (blocks `nz start`) + stowaway | nz stop/rm | GAP (R1 G3) | 9022551c |
| cli-13b (new) | the same while **live** | ship | by-hand | copy 900d462d, same name | same | same | GAP | 900d462d |
| cli-14 R1:6c (same) | `--bg -c` in ship1, S1 stopped | ship | by-hand | "started a copy of that conversation as 55a9057c" of the newest transcript (942bf763); same name, `fork` | `name.taken` + stowaway | nz stop/rm | GAP (R1 G3) | 55a9057c |
| cli-15 R1:6d (changed while live) | `--bg --resume "[E2E2.SQ]-S1"` (by name) | ship | by-hand | stopped: copy a85c1931 **named by its short id** (as round 1). **Live: copy 431199ba carries the member's name** (new). CC now prints "pass its full session id (lowercase...) to --resume" | stopped: stowaway only. live: `name.taken` + stowaway | nz stop/rm | GAP (low) | a85c1931, 431199ba |
| cli-16 R1:6e (same) | `-p --resume <uuid> "reply ok"`, stopped | ship | by-hand | foreground turn on the **same id**, transcript appended; hooks have **no labels**. New: `-p --resume <short id>` is refused ("not a UUID and does not match any session title") | `labels.missing` Error ("resumed with flags, or legacy", wrong cause) | `nz start`: "will repair", `claude rm` + resume with every flag: same id, history kept, lineage +relaunch | OK (recovers), wording GAP (R1 G5c) | pid 1127975 SessionStart resume labels None |
| cli-17 R1:6f (same) | `-p --resume <uuid or title>`, live | ship | by-hand | refused: "That session is running in the background (942bf763)... Add --fork-session"; each refused process still fires a **SessionEnd with no labels** for 942bf763 | unaffected | n/a | OK, stray event (R1 G5b) | SessionEnd 942bf763 pid 1010479 labels None |
| cli-18 R1:6g (changed: name) | interactive `claude --resume <uuid>`, stopped | ship | by-hand | same id as `kind interactive`, no job id, **own name `[E2E2.SQ]-S1`** (round-1's parent-name drift was harness leakage, gone under `env -i`); SessionStart `resume`, labels {} | `idle` + `labels.missing` Error, `rules.old` Info; **`nz relaunch` refuses "live with no job id; cannot stop it (session.none)"** (byhand G3). After `/exit`: `stopped` + `labels.missing` | `nz start` after exit = rm + resume with flags (same id). Or `claude attach` (cli-07) | GAP (byhand G3, dead-end fix) | pid 1223040 |
| cli-19 R1:6h (changed) | `/rename RENAMED-E2E2-CLI` in an attached ship | ship | by-hand | name changes live; **`--name RENAMED-E2E2-CLI` is now rewritten inside `respawnFlags`** (round 1: stripped); labels kept | live: `name.drift` Error (fix relaunch). **Stopped: no issue at all**; `nz start --plan` = native resume, which would keep the wrong name (could not run it: the parent was in the cli-L1 state) | `nz relaunch` = rm + resume with flags: same id, right name | GAP (R1 G4) | Q2 table |
| cli-20 R1:6i (same) | attach, `/clear`, ctrl-z | ship | by-hand | same pid and job 942bf763, **session id becomes ec1f4662**; SessionEnd `clear` + SessionStart `clear`, both labelled; `state.json` sessionId 942bf763, resumeSessionId ec1f4662 | **`stopped`** "job row 942bf763 with no live pid" while the ship is live; `copy.rows` Warn; `nz start` refuses `dup.live` (fix "nimitz stop 942bf763 # or adopt ec1f4662"); `nz stop 942bf763` refuses `session.none`; `nz sweep` skips (dup.live); `nz check`: no errors | by-hand `claude stop` -> `nz start` (native resume) wakes job 942bf763 on **ec1f4662** -> `lineage.broken` ("expected 942bf763 but bound ec1f4662"), registry still 942bf763. `claude rm 942bf763` + `nz start` -> clean ship on the **pre-clear** 942bf763; ec1f4662 orphaned (and later picked up by `claude -c`, cli-24) | BREAK (R1 G1) | pid 1073252; registry current 942bf763 |
| cli-21 R1:6j (same) | `claude purge --dry-run $MAP/ship1` | ship | by-hand | would delete the project transcripts dir + memory/, the `~/.claude.json` projects entry (trust, history, MCP) and 5 history.jsonl prompts. **It does not mention the job row or session file**: a purge leaves a live/stopped job whose transcript is gone | (not run) | round 1 simulated: `lineage.broken`, `nz start --force` | OK (handled), not run | dry-run output |
| cli-22 | `claude logs <job>` | ship | by-hand | prints the pty buffer with raw ANSI | n/a | n/a | OK | |
| cli-23 (new) | `-p -c "reply ok"` in ship1 while the only transcript belongs to the live ship | ship | by-hand | `-c` skips the live job's transcript and **starts a new one-shot session 4580aceb** (SessionStart `startup`, no labels, ends at once); its transcript stays in the ship folder and is now the newest, so later `-c` picks it | nothing (ended, no row) | n/a | GAP (low): a print-mode `-c` in a member folder silently makes a new conversation and costs a turn | 4580aceb pid 1009172 |
| cli-24 (new variant of byhand 2b-i) | interactive `claude -c` in ship1 while S1 live and an orphan transcript exists | ship | by-hand | picks the newest transcript that has **no live job** (the post-clear orphan ec1f4662) and runs it interactively with the **member's name** | `name.taken` Error on S1 + stowaway ec1f4662 (fix `nimitz stop` fails for interactive, byhand G4) | `/exit` clears it | GAP (byhand G4/G6 family) | pid 1190922 |
| cli-25 byhand 2c-live (same) | interactive `claude --resume <uuid>` while live | ship | by-hand | re-execs as `claude attach`; no new session, no events | unchanged | ctrl-z | OK | |
| cli-26 (new) | `claude --resume` picker | ship | by-hand | lists only the cwd project's sessions (both rows named `[E2E2.SQ]-S1`: the live bg ship and the orphan). Enter on the live row = "Opening the background session (942bf763)" = attach. Esc cancels clean | unchanged | n/a | OK | capture |
| cli-27 (new) | `--bg --name "[E2E2.SQ]-S1"` in ship1 while S1 live (name collision) | ship | by-hand | **CC allows a duplicate name**: 26b327f7 `[E2E2.SQ]-S1`, flags `[--name ...]`, no labels | `name.taken` + stowaway | nz stop/rm | GAP (CC side: two addressable sessions with one name) | 26b327f7 |
| cli-28 byhand 2a (same) | plain `claude` in ship1 (trust prompt accepted), `/exit` | ship | by-hand | session 808ff1ad `ship1-46`, interactive, no labels | stowaway, fix "nimitz stop <sid>" (wrong for interactive, byhand G4); `nz check S1` no errors | exit | OK (fix text) | 808ff1ad |
| cli-29 (new) | ship folder becomes a **git repo after launch** (`git init` in ship1) | ship | env | trust is cut at the git root: ship1 was trusted only through its ancestor `~/.cache/nimitz-e2e`; after `git init`, a by-hand `--bg --resume` refused **"Workspace not trusted. Run `claude` in ... once and accept the trust prompt"** (while `-p` runs, non-TTY skips trust). The already-running ship was unaffected | nothing (nimitz does not check trust at start) | I accepted the prompt interactively | GAP (med): `nz start`/`relaunch` of a ship in an untrusted git folder will fail the same way (not run: ship1 was trusted afterwards) | message text; `~/.claude.json` ship1 `hasTrustDialogAccepted: false` before |
| cli-30 R1:1g (same) | `kill -9 <worker pid>` | ship | env | `state.json` `crashed`, row keeps `working` with no pid; **the daemon restarts it by itself after about 15 s** (same id, labels, `resume`); daemon.log `bg settled 942bf763 (killed)` then `bg claimed-spare 942bf763 (fleet)` | 3 s after: `stopped` "state crashed"; 20 s after: `idle` | none needed | OK | pids 1223884 -> 1274942 |
| cli-31 R1:1g-race (same) | `kill -9`, then `nz start --yes` after 1 s | ship | env | CC: the resume became a **copy 29a81431** (same name, no labels, `fork`); the daemon then restarted 942bf763 (pid 1278970) | **`[OK] E2E2/SQ/S1: claude --bg -> backgrounded · 29a81431`**, "verified by re-reading", lineage +resume 942bf763 (the daemon's); status then `name.taken` + stowaway 29a81431 | nz stop/rm the copy by hand | BREAK (R1 G2) | events: SessionStart 29a81431 fork, then SessionStart 942bf763 resume |
| cli-T1 | `claude stop <tender>` | tender | by-hand | as cli-01 | `~t1 stopped` "state done" | native `--bg --resume` wakes with saved options | OK | 45c8aa75 pid 990816 -> 1148387 |
| cli-T2 | `claude rm <tender>` then `nz start E2E2/SQ/~t1` | tender | by-hand | row gone | `stopped` "no job row; transcript is there" | `nz start ~t1` resumes with every flag: same id 45c8aa75, labels, lineage +relaunch | OK | |
| cli-T3 | `nz tender E2E2/SQ t1` again while ~t1 live (same task name) | tender | nimitz | n/a | refused `dup.live` | n/a | OK | |
| cli-T4 (new) | `--bg -c` in the commodore folder while the commodore is live and the tender's transcript is newest | tender/commodore | by-hand | copies the **tender's** conversation (a44f0c48 named `[E2E2.SQ]-COMMODORE/t1`); `--bg -c` does not skip a live job's transcript (interactive `-c` does, cli-L3) | `~t1 name.taken` + stowaway | nz stop/rm | GAP | a44f0c48 |
| cli-L1 (new) | `-p --resume <commodore uuid> "reply ok"` while the commodore is **live** | commodore | by-hand | **not refused** (only bg sessions are): a print-mode turn runs on the **same session id** in a second process with no labels; the transcript branches; the live pane shows "← 2 agents" | commodore **`stopped`** "no session row; a transcript in its folder", its own live session listed as a **stowaway** with `name.taken` (fix "nimitz stop <commodore>"); `nz start` of anything under E2E2/SQ then refuses on the parent; **persists** after the print process ends | `/exit` the lead and run the plan command (cli-L3), or `/clear` in it | BREAK (byhand G6 by print mode) | pid 1130805 SessionStart resume labels None |
| cli-L2 (new) | `--bg --resume <admiral uuid>` while the admiral is live | admiral | by-hand | "open in another Claude Code process, so this started a copy as 7d18608f"; a **bg** copy with the admiral's name, no labels | admiral `name.taken` + stowaway | nz stop/rm the copy (works: it is bg) | GAP | 7d18608f |
| cli-L3 byhand 4a-recover (same) | commodore `/exit`, then the exact plan command (`claude -c --name --settings`) | commodore | by-hand | `-c` continued the commodore's own 02d4c732 (interactive `-c` skips the tender's transcript while that job is live); labels back | `idle`, `nz check` no errors; **registry still never binds leads** (byhand G1: identities = S1, ~t1 only) | n/a | OK here; byhand G7 holds when the newest transcript has no live job | SessionStart 02d4c732 resume labels E2E2/SQ |
| cli-L4 byhand 4b (same) | interactive `claude --resume <admiral uuid>` while the admiral is live | admiral | by-hand | **opens the same id in a second process** (two session files, two rows with one sessionId), SessionStart `resume`, no labels | admiral `stopped`, own session a stowaway, `name.taken`; **persists after the duplicate `/exit`s** | `/exit` + plan command (cli-K2) | BREAK (byhand G6) | pids 989881 + 1183907 |
| cli-L6 (new) | `--bg --name "[E2E2.SQ]-COMMODORE"` in the commodore folder | commodore | by-hand | bg session a2b2e960 with the lead's name | `name.taken` + stowaway | nz stop/rm | GAP (as cli-27) | a2b2e960 |
| cli-L7 (new) | interactive `claude --session-id 22222222-...` in the commodore folder | commodore | by-hand | new session with the chosen id, name `sq-ea`, no labels | stowaway while live; gone at `/exit` | exit | OK | 22222222 pid 1244394 |
| cli-L8 byhand 2e (same) | interactive `claude --resume <commodore uuid> --fork-session` while live | commodore | by-hand | fork ccd21641 with the **same name**, `fork`, no labels | `name.taken` + stowaway; fix "nimitz stop <sid>" fails for interactive | `/exit` | GAP (byhand G2/G4) | ccd21641 |
| cli-L9 (new) | commodore `/exit`, plan command (`-c`) while the live tender's transcript is the newest in the folder | commodore | by-hand | interactive `-c` skipped the tender (live job) and continued 02d4c732 | `idle`, no errors | n/a | OK (but see cli-T4: `--bg -c` differs) | |
| cli-L10 (new) | commodore `/exit`, then `claude --resume <uuid> --name [E2E2.SQ]-COMMODORE --settings '{...labels}'` by hand | commodore | by-hand | same id 02d4c732, labels back, name kept | `idle`, `nz check` no errors | n/a | OK: the by-id resume byhand proposed for G7 works on 2.1.292 | SessionStart 02d4c732 resume labels E2E2/SQ pid 1258472 |
| cli-K1 (new) | interactive `claude --name "[E2E2]-ADMIRAL"` in the admiral folder while the admiral is live | admiral | by-hand | duplicate name allowed: fc893142 `[E2E2]-ADMIRAL` | both listed as stowaways (the admiral was already in the cli-L4 state), `name.taken` on fc893142 | `/exit` | GAP (as cli-27) | fc893142 |
| cli-K2 (same as byhand 4a-recover) | admiral `/exit`, then the plan command (`claude -c --name --settings`) | admiral | by-hand | continued 31686e97 with labels | `idle`, `nz check E2E2` no errors (the cli-L4 state is cleared only by this) | n/a | OK | pid 1233484 |
| cli-N1 | `nz rm <id>` right after `nz stop <id>` | stowaway | nimitz | n/a | twice during cli-09/cli-12 the `nz rm` directly after `nz stop` said "names no identity and no job row (session.unknown)" although the row existed; a retry 3 s later worked. A clean repro (2ec0ab08) did **not** reproduce it | retry | GAP (low, intermittent) | copycase output in the log |
| not run | `--from-pr`, `--teleport`, `--cloud`, `--remote-control`, `--desktop`, `--environment` (cloud/PR/desktop sessions); `--no-session-persistence` (-p only, nothing to resume); `respawn --all`, agent-view Enter/ctrl-x/ctrl-r (machine-wide, unsafe); `claude purge` for real; a lead crash (SIGHUP) = byhand 4a | | | | | | | N/A | |

## 2. NEW PATHS (from `--help`, the subcommand help pages, and the runs)

- `-p -c` in a member folder starts a **new** session when the newest transcript belongs to a live job (cli-23); `--bg -c` makes a copy of it instead (cli-T4); interactive `-c` skips it (cli-L9). Three different rules for one flag.
- `-p --resume <live interactive id>` is a silent second process on a lead's id (cli-L1). Only background sessions are protected by the "running in the background" refusal.
- `-p --resume` needs the **full uuid or the title**; a short id is refused (cli-16).
- `--resume <name>` with `--bg` now copies with the member's name when the member is live (cli-15).
- `--name` is never unique on the CC side: `--bg --name` and interactive `--name` both make a second session with a member's name (cli-27, cli-L6, cli-K1).
- `/rename` keeps `--name` in `respawnFlags` (rewritten), so a native resume carries the new name (cli-19).
- `--session-id` works for interactive sessions (cli-L7) and is ignored with `--bg` (cli-12).
- `--resume` picker: Enter on a live bg row attaches; Ctrl+R in it renames (not pressed).
- `claude rm` did **not** remove the worktrees of `-w` jobs (Q4, "when that is safe" did not trigger).
- `claude purge` would delete a member's transcripts while its job row stays (cli-21).
- `--worktree`/`-w`, `--tmux` (needs `-w`), `--bare`, `--safe-mode`, `--restricted`, `--setting-sources`, `--system-prompt-snapshot`: none creates or ends a session by itself beyond what is covered; `--bare`/`--safe-mode` would skip the nimitz hooks (a labelled session with no events: not run).
- Trust: a member folder that becomes its own git root loses the ancestor trust (cli-29).

## 3. GAPS

**cli-G1 (BREAK, high) `-p --resume` on a live lead makes nimitz lose the lead (byhand G6 by print mode).** Trigger: anyone (a script, a commodore skill, a user) runs `claude -p --resume <lead uuid> "..."` while the lead is open. What breaks: the turn runs on the same id with no labels; nimitz takes labels from the newest SessionStart, so the live lead becomes `stopped` + its own session a stowaway with `name.taken`; every `nz start` under that parent refuses; the state persists. Evidence: cli-L1 (pid 1130805, SessionStart 02d4c732 `resume` labels {}). Repro: `nz start E2E2/SQ`, run its command in tmux, then `cd <sq folder> && claude -p --resume <commodore uuid> "reply ok"; nimitz status`.

**cli-G2 (BREAK, med-high) crash + `nz start` race still reports OK (R1 G2 unchanged).** Repro: `pid=$(claude agents --json | jq ...S1 pid); kill -9 $pid; sleep 1; nimitz start E2E2/SQ/S1 --yes` -> `[OK] ... backgrounded · <new id>` with a copy carrying the name. Evidence cli-31 (copy 29a81431).

**cli-G3 (BREAK, high) `/clear` under a live job (R1 G1 unchanged).** Repro: `claude attach <ship>`, `/clear`, ctrl-z, `nimitz status`. Evidence cli-20. New detail: the orphaned post-clear transcript is what `claude -c` picks later (cli-24), with the member's name.

**cli-G4 (GAP, med) every by-hand resume/fork/continue with a live or stopped member makes an unlabelled copy that carries the member's name (R1 G3 unchanged, wider).** Paths: cli-09, 11, 13, 13b, 14, 15-live, L2, L8, T4. `nz sweep` never drops them; the user must `nz stop` + `nz rm` by id; interactive ones cannot be stopped by nimitz (byhand G4). Repro: `cd ship1 && claude --bg --resume <uuid> --fork-session; nimitz status`.

**cli-G5 (GAP, med) name collisions are not prevented by Claude Code; nimitz only reports them (cli-27, L6, K1).** Repro: `cd ship1 && claude --bg --name "[E2E2.SQ]-S1"; nimitz status` (then `nimitz stop <id>; nimitz rm <id>`).

**cli-G6 (GAP, med) a member folder that becomes a git root is untrusted; by-hand `--bg --resume` refuses (cli-29).** `nz start` of such a ship is expected to fail the same way (not run). Repro: `nz start F/SQ/S` in a non-git folder under a trusted ancestor, `git init` there, `nz stop`, then `claude --bg --resume <uuid>` -> "Workspace not trusted". Fix idea: nimitz doctor/start should check `~/.claude.json` `projects[<git root>].hasTrustDialogAccepted` (nimitz-core `env.rs trust_keys` already models this) and say so before launching.

**cli-G7 (GAP, med-low) `/rename` while stopped is invisible and `nz start` would keep the wrong name (R1 G4; changed mechanics).** On 2.1.292 `respawnFlags` keeps `--name <new>`, so `state.json.name != identity name` is the signal. Repro: attach, `/rename X`, ctrl-z, `claude stop <ship>`, `nimitz check F/SQ/S` (no error), `nimitz start F/SQ/S --plan` (native resume).

**cli-G8 (GAP, low) `-p -c` in a member folder starts a hidden new conversation (cli-23).** Repro: `cd ship1 && claude -p -c "reply ok"` while the ship is live; then `ls ~/.claude/projects/<ship1>/`.

**cli-G9 (GAP, low) agent view: ctrl-c no longer closes it; the spare is a transient stowaway (cli-05, byhand G5).**

**cli-G10 (GAP, low) `claude rm` leaves `-w` worktrees and branches behind (Q4-8).** Repro: `cd repo && claude --bg -w; claude stop <id>; claude rm <id>; git worktree list`.

**cli-G11 (GAP, low, intermittent) `nz rm` right after `nz stop` says `session.unknown` with the row present (cli-N1).**

## 4. ADDENDUM

### Q1. Is the job id stable? (ship 942bf763, every step recorded)

| step | job dir / `claude agents` id | `state.json` sessionId | `state.json` resumeSessionId | live session id | pid |
| --- | --- | --- | --- | --- | --- |
| launch by `nz start` | 942bf763 | 942bf763 | 942bf763 | 942bf763 | 988519 |
| `claude respawn` (live) | 942bf763 | same | same | same | 1053118 |
| `claude stop` | 942bf763 (state `done`, no pid) | same | same | none | - |
| native `claude --bg --resume <uuid>` | 942bf763 | same | same | 942bf763 | 1067070 |
| `claude kill` | 942bf763 | same | same | none | - |
| `claude respawn` (stopped) | 942bf763 | same | same | 942bf763 | 1073252 |
| attach + `/clear` | **942bf763** (unchanged) | 942bf763 | **ec1f4662** | **ec1f4662** | 1073252 (same process) |
| `claude stop`, `nz start` (native resume) | 942bf763 | 942bf763 | ec1f4662 | ec1f4662 | 1083882 |
| `claude stop` + `claude rm 942bf763` | dir gone | - | - | none | - |
| by-hand `claude --bg --resume ec1f4662-...` after rm | **ec1f4662** (new dir; flags `[]`, no labels, name kept from the transcript's custom-title) | ec1f4662 | ec1f4662 | ec1f4662 | 1098827 |
| `nz start` after rm (resume 942bf763 with every flag) | **942bf763** again | 942bf763 | 942bf763 | 942bf763 | 1106676 |
| `kill -9`, daemon restart | 942bf763 | same | same | same | 1274942 |

Answer: the job id is stable across stop, kill, respawn, native `--bg --resume`, `/clear` and a crash restart. After `claude rm` a resume creates a **new job whose id is the short form of the resumed session id** (`ec1f4662` for the post-clear session; `942bf763` again when nimitz resumes the original), with empty `respawnFlags`. So "same short id" after rm is a coincidence of resuming the original id, not a kept row.

### Q2. Does `respawnFlags` keep `--settings` (labels) and `--name`?

| step | respawnFlags |
| --- | --- |
| launch, stop, kill, respawn, native resume, `/clear`, crash restart | `[--name [E2E2.SQ]-S1, --permission-mode default, --settings {labels}]` unchanged through every step |
| `/rename RENAMED-E2E2-CLI` | `[--permission-mode default, --settings {labels}, --name RENAMED-E2E2-CLI]`: labels kept, **`--name` rewritten** (round 1 saw it stripped) |
| `claude rm` + by-hand resume | `[]` (no labels, no name); `--resume <uuid> --permission-mode default` while a row exists: the copy gets only `[--permission-mode default]` |
| `nz relaunch` / `nz start` after rm | full flags again (nimitz passes them) |
| `-p --resume` | flags of the row untouched (the print process runs without them) |

### Q4. `--worktree` and `bgIsolation` (test repo `$MAP/repo`, `git init` + one commit, trusted by hand)

- `claude --bg` with **no** `worktree.bgIsolation` setting: the job runs in the **repo root** on `main` (row cwd = root, no worktree; job 59ad0789). So on this machine the default is no isolation (no `worktree` key in `~/.claude/settings.json`).
- `claude --bg -w`: a worktree `<repo>/.claude/worktrees/<random-name>` on branch `worktree-<random-name>`, marked **locked**; the row's cwd is the worktree, `state.json.cwd` stays the **repo root** (job 4540f99d). With `--settings '{"worktree":{"bgIsolation":"none"}}'` **and** `-w`, the worktree is still made (the flag wins; 3bfc0033).
- `-c` from the worktree folder: `--bg -c` found that job's conversation and made a copy there (2afbf0a3, later `state failed`, no prompt had been sent). `-c` from the root: copy of a root job (415374e2, also `failed`). `--bg --resume <worktree job uuid>` from the root while live: copy **in the root**, not in the worktree (6427a0a4). All three copies ended `failed` because their source had no transcript yet (no prompt): a copy of an idle job fails.
- A by-hand interactive `claude` in the root while the `-w` job lived: both fine (row `repo-5d` interactive in root, jobs untouched; `git status` in root only `?? .claude/`). A ship in its own worktree and a by-hand session in the root do not break each other (no turns were run, so no file conflicts were exercised).
- `claude stop` + `claude rm` on all six jobs: rows removed, but **both worktrees and their branches stayed** (`git worktree list` still shows them locked). I removed them in cleanup with `git worktree remove --force`.
- Transcripts for a worktree session: none were written (no prompt); the root `-p` run (Q5) wrote under `-...-r2-cli-repo-sub`.

### Q5. What Claude Code keys by folder

| thing | key | evidence |
| --- | --- | --- |
| transcripts | the **exact cwd** (project dir `-home-...-r2-cli-repo-sub` for a run in `repo/sub`; `-...-repo` for the root), not the git root. A resumed session keeps writing to its original project dir even when resumed from another cwd (ec1f4662 resumed from the repo cwd still wrote under `...-ship1`) | `~/.claude/projects/` listing |
| `-c` | the exact cwd's project dir, newest transcript first; interactive `-c` skips transcripts whose job is live, `--bg -c` and `-p -c` do not | cli-14, 23, 24, L3, L9, T4 |
| trust | `~/.claude.json projects[<path>].hasTrustDialogAccepted`, looked up along ancestors but **cut at the git root** | cli-29; env.rs `trust_keys` |
| settings | `.claude/settings.json` of the cwd project (ship1's `proj` env reached the hooks; the repo-cwd mistake wrote to the **live** `~/.local/state/nimitz/events.jsonl` because that repo's settings have no NIMITZ_STATE) | events files |
| CLAUDE.md | from `repo/sub`: **MARKER-A** (`repo/CLAUDE.md`, git-root ancestor) and **MARKER-B** (`repo/sub/CLAUDE.md`, cwd) loaded; **MARKER-C in `--add-dir $MAP/addir/CLAUDE.md` did not load** (one `-p` turn, the model listed A and B only) | Q5-1 output |
| auto memory | `memory/` under the project dir of the **git root** (`-...-r2-cli-repo/memory` exists, `-...-repo-sub` has only the jsonl); non-git folders get their own (`ship1/memory`, `sq/memory`, `admiral/memory`) | listing |

## 5. CLEANUP and livediff

- livediff after every phase and at the end: `[OK] live fleet unchanged` every time (no `[ERROR]`, no idle-retire notes for live sessions).
- Jobs I created, all stopped and `claude rm`'d / `nz rm`'d: ship 942bf763, tender 45c8aa75, copies dc74d940, 431199ba, c49147e6, 26b327f7, a771417f, 900d462d, 19da599d, 9022551c, a85c1931, 55a9057c, ec1f4662 (twice), 7d18608f, a44f0c48, a2b2e960, 2ec0ab08, 29a81431; repo jobs 59ad0789, 4540f99d, 3bfc0033, 2afbf0a3, 415374e2, 6427a0a4. Spare 28f3d8a7 went away with the agent view. `claude agents --json --all` shows no row and no session file under `$MAP`; `~/.claude/jobs/` has none of these dirs.
- Interactive sessions ended with `/exit` (admiral 31686e97, commodore 02d4c732, 808ff1ad, bc1b8026, 93f3acca, fc893142, ccd21641, 22222222, the ec1f4662 and 942bf763 interactive runs); tmux server `r2-cli` killed.
- The two `-w` worktrees were still **locked** ("lock reason: claude session <name> (pid ...)") after `claude rm`; `git worktree remove --force` refused (needs `-f -f`); they went with `command rm -rf $MAP`. `$MAP` removed.
- Transcript folders left under `~/.claude/projects/` (delete freely):
  `-home-user--cache-nimitz-e2e-nimitz-e2e-r2-cli-ship1`, `...-r2-cli-nimitz-e2e2-admiral`, `...-r2-cli-nimitz-e2e2-squadrons-sq`,
  `...-r2-cli-repo`, `...-r2-cli-repo-sub`.
- Left in place on purpose (not mine to edit): `~/.claude.json` `projects[...]` entries for `$MAP/ship1`, `$MAP/repo`, the admiral and sq folders (trust accepted by hand for ship1 and repo); `~/.claude/history.jsonl` prompt lines for those folders.
- **Mistake:** one `claude --bg --resume ec1f4662-...` ran with cwd `/home/user/Github/claude-nimitz` (the Bash tool resets cwd). It made job `ec1f4662` in the repo cwd for about 40 s, labelled E2E2/SQ/S1 (nimitz flagged `labels.mismatch` + `name.taken`, a correct detection), stopped and removed at once; livediff stayed OK. Side effects: 2 **unlabelled** hook lines (SessionStart `resume`, SessionEnd `other`, pid 1086130, session ec1f4662) in the live `~/.local/state/nimitz/events.jsonl`; a `custom-title.json` under the ship1 project dir. No file in the repo was touched (`git status` clean).
- Nothing in `/home/user/Github/claude-nimitz` was modified.
