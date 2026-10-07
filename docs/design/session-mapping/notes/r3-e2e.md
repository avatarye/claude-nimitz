# Round 3: the 0.4.0 e2e suite against the real Claude Code (P7a)

Claude Code **2.1.292**, nimitz 0.4.0 (branch `v4/p7-e2e`), 2026-10-07. The tests are
`crates/nimitz/tests/e2e.rs`, one per risky case of plan-v4 section 10, gated by
`NIMITZ_E2E=1`. Run as the task asked: every child scrubbed (`env_clear` + HOME, PATH,
TERM, USER, SHELL, LANG), one fleet `NIMITZ_E2E<n>` per test under
`~/.cache/nimitz-e2e/nimitz-e2e-<n>-<pid>/` with private `NIMITZ_HOME` and
`NIMITZ_STATE`, the live plugin off in every member folder's `.claude/settings.json`,
every hook event pointed at the build under test, `--model haiku` on the command line
(the org turns it into Opus 5.5: "Model haiku is restricted by your organization's
settings. Using claude-opus-5-5 instead"), leads attached with `claude attach` in a
private tmux server, never the Herdr CLI. Cleanup is a drop guard; each test ends
with its own live-fleet guard (every fleet-named session that is not `NIMITZ_E2E*`
keeps its id, pid and kind), and `livediff.py` against a fresh baseline printed
`[OK] live fleet unchanged` before and after the whole run.

Command: `NIMITZ_E2E=1 cargo test -p nimitz --test e2e -- --test-threads=4`.

## Verdicts

| # | Case | Verdict | Evidence (final run) |
| --- | --- | --- | --- |
| 1 | launch binds job and session, rules delivered | OK | SessionStart line `binding bound`, `job` = the short id, `rules 2`; registry `job`, `session`, lineage `[launch]`; `rules-delivered.json` keyed by the job, `via SessionStart`; the rules text in the transcript |
| 2 | `/clear` inside the job | OK | typed via attach; SessionStart `clear` with the job, new session id; registry `session` moved, lineage `[launch, clear]`; status follows, no copy rows |
| 3 | stop, then start: resume by exact id | OK | plan = `<native> --bg --resume <sid>` and nothing else; output `backgrounded · <job>`; same job, same session; SessionStart `resume`; lineage stays `[launch]` (a wake adds no entry: `registry::bind_into`) |
| 4 | transcript deleted, then start | OK | refused `history.lost` with the `--fresh` fix, nothing launched, the old row kept; `--fresh` = cold launch, new session and job, lineage `[launch, launch]`, the old row dropped |
| 5 | start of a live member; hand resume of a live job | OK | the plan has no launch step; pid unchanged. The hand `--bg --resume <sid> --model haiku` of the live job: `backgrounded · <new id> (idle — send a prompt to start)`, **no "started a copy" note**; the copy carries the ship's exact name and no labels; status: `copy: name`, issue `copy`; `sweep` stops and removes it |
| 6 | kill -9 of a working member | OK | killed while Busy (its first turn); state.json `crashed`; `start` waited 10.1 s and said `skipped: the daemon restarted job 608c5566 (state was crashed); nothing to resume`; one row, same job, same session, Idle again. Before finding 2 the same start, run 1 s after the kill, refused `history.lost` |
| 7 | a lead is a background job | OK | cold launch `--bg --name [F]-ADMIRAL ...`; `pinned: true`, the id added to `~/.claude/jobs/pins.json`, no `pins.json.lock` left; its first turn ends `NeedsYou` ("job row left a question: give orders ..."), no PermissionRequest; `claude attach` in tmux shows `[F]-ADMIRAL` with a prompt; ctrl-z returns the shell and the job stays live; `stop` + `fleet rm` remove the row and the pin, every other pin stays |
| 8 | rules update at the next prompt, once | OK | record set to v1; two prompts typed via attach; exactly one UserPromptSubmit line with `rules 2`; record `version 2, via UserPromptSubmit`; the transcript holds the header once, as `attachment.type hook_additional_context` |
| 9 | a ship runs nimitz through `Bash(nimitz *)` | OK | mode `default`; `nimitz --version`, `status`, `stop <other> --yes` ran (the PATH wrapper's call log); no PermissionRequest, no needs-you notice, status never NeedsYou; the stop's output in the transcript says `authority`; OTHER untouched |
| 10 | unlabelled session hidden; a fork is a copy | OK | the user's own `--bg` session in the ship folder: not a row of the ship, not a stowaway, no issue names it. `--bg --resume <sid> --fork-session --model haiku` of the stopped ship: new id, the ship's exact name, `copy: name`, issue `copy`; `sweep` removes the fork and leaves the user's own session |
| 11 | a member labelled for another `NIMITZ_HOME` | OK after bug 1 | the session (`--settings` labels with another home) is no row, no stowaway, no issue; the ship is `NotStarted`; `check` names nothing. Its hook wrote 3 lines, all carrying the other home (see finding 1) |
| 12 | v1 registry migration, then a live start | OK | a copy of the live registry in the 0.3.x shape (`version 1`, `current`, `bound_at`, no `job`, plus a tender): a read migrates in memory and writes nothing; `start` resumes by the job of the row that runs `current`; the file is v2 with `job` on the identity and the lineage entry, the tender under `retired`, the old file kept as `registry.json.v1` |

## Findings (Claude Code behaviour that differs from the notes or the spec)

1. **The hook cannot tell "another home" apart.** contracts.md section 5 says labels of
   another `NIMITZ_HOME` get no line. The hook's env *is* the session's env, and the
   `--settings` env wins over the project settings env (the line's labels show the
   other home while the project settings said this one), so `env.home` in the hook
   equals the label and `registry::other_home` is never true there. The hook wrote
   `SessionStart binding bound` and bound the identity in the shared `NIMITZ_STATE`
   registry; the reader then showed the never-started ship as `Stopped` ("its
   session's transcript is there") and a `start` would have resumed the other home's
   conversation with every flag. Fixed on the reader side: `registry::load` strips a
   binding to a job whose row's saved labels (`respawnFlags`) name another home
   (`drop_other_homes`: those lineage entries go, `job` and `session` fall back to
   the newest entry left, an identity with nothing of its own goes; sandbox tests
   `a_job_labelled_for_another_home_never_binds_the_member`,
   `a_foreign_binding_is_stripped_and_this_homes_history_kept`).
   The rule only matters when two homes share one `NIMITZ_STATE`; the line itself
   is harmless (every reader skips rows labelled for another home).
2. **kill -9 in the first turn: no transcript yet.** The job's transcript was not on
   disk when `start` ran 1 s after the kill, so `wake_steps` refused with
   `history.lost` and the `--fresh` fix; the daemon then restarted the job anyway
   (a `--fresh` at that moment would have made a duplicate). Fixed: a row in the
   crash window (`crashed`, or `working` with no live pid) skips the transcript
   check; `apply` waits for the daemon as before and, when the daemon did not
   restart the job, looks for the transcript again: still gone, `history.lost`
   with the `--fresh` fix and no resume (sandbox test
   `a_crashed_job_is_resumed_not_judged_history_lost`). Case 6 re-run after
   0963f12 (guard `[OK]` before and after): killed while Busy, state `crashed`,
   `start` exit 0 after 10.6 s with `skipped: the daemon restarted job 83a52843
   (state was crashed); nothing to resume`, no `history.lost` in its output, Idle
   under the same job and session afterwards.
3. **A flagged resume of a live job prints no copy note** (2.1.292): `claude --bg
   --resume <sid> --model haiku` on a live job answered `backgrounded · <new id>
   (idle — send a prompt to start)` with no "started a copy as" line. The copy took
   the conversation's saved name (`[F.SQ]-SHIP`) and carried no labels. P8 saw an
   auto-generated name when `--name` was passed; without it the name is inherited.
   nimitz needs no note here: the copy is found by name (`copy: name`). The same
   shape for `--fork-session` of a stopped job.
4. **A wake adds no lineage entry.** A resume of the conversation the identity runs
   already keeps its birth entry (`registry::bind_into`): after stop + start the
   lineage is still `[launch]`. contracts.md section 5 ("lineage appended with `via`
   from `source` ... resume") reads as if an entry were added; the code and
   "a wake never overwrites how it was born" are the actual rule. The tests assert the
   code's rule.
5. **The "; pinned" note never showed.** `apply` appended it after Claude Code's
   multi-line launch output, and the CLI prints a step's first line only. Fixed:
   `note` inserts on the first line (`backgrounded · <id> · <name>; pinned`).
6. **A lead's first turn ends `NeedsYou`, not Idle:** the job row's `needs` says "give
   orders (plan squadron/ship, or direct task)" once the admiral has reported and
   asked what to do. Not an error; the test accepts Idle or NeedsYou and checks that
   no PermissionRequest fired.
7. **Hooks must be `command` + `args`.** A hook given as one command string runs
   through `sh -c`, so the hook's parent pid is the shell and
   `sessions/<ppid>.json` does not exist: every SessionStart binds `none`. The
   plugin's hooks.json uses the args form; so must any project-level copy.
   Hardened since: the hook tries the parent's parent once (`/proc/<ppid>/stat`
   field 4) and takes it only when its session file exists (`hook::claude_pid`,
   unit-tested on a fake chain and on the real one), so a string-form copy still
   binds; `doctor` warns about a string-form entry in the installed plugin with
   the reinstall + `/reload-plugins` fix.
8. **`start <ship>` is top-down**: a bare ship test launches the admiral and the
   commodore too (each a tool-using Opus turn). The harness launches both leads by
   hand with their exact labels and names and a one-word prompt; a live, clean lead
   is left alone by `start`.
9. **A third retire reason exists.** Besides `settled, idle 61m` and `idle-prompt, idle
   61m`, the daemon wrote `[2026-10-07T05:14:03.119Z] [bg] bg retire 0abc1234:
   abandoned-stale, idle 48h, worker 2.1.289 (daemon 2.1.292)` (a job whose worker
   was an older build, retired after a daemon upgrade), then `bg settled 0abc1234
   (done)`. `evidence::daemon_retired` takes any text after the colon, so the parked
   reason reads it whole; the unit test
   `daemon_retire_reason_is_any_text_after_the_colon` pins the three shapes.
10. **An interactive `claude --resume <sid>` on a background job's conversation**
    (for P9: Herdr's `resume_agents_on_restore = true` runs it in every agent pane
    after a restart). Claude Code 2.1.292, by hand under the e2e rules: scrubbed env,
    private tmux `nz-e2e-q10`, a labelled `--bg` job (`[NIMITZ_E2EQ.SQ]-SHIP`, job
    9ee5b0fc, one-word prompt, `--model haiku` -> Opus by org policy), guard
    `[OK] live fleet unchanged` before and after, everything stopped, removed and
    deleted at the end. "Wake" = `claude --bg --resume <sid> --model haiku` from a
    shell while the interactive process is alive.

    | Job | Interactive `--resume <sid>` | Session file | Job row | Terminal | Wake meanwhile |
    | --- | --- | --- | --- | --- | --- |
    | (a) live (`done`, pid alive) | **refused, exit 1**; no process starts, nothing re-execs | the job's own only (`kind bg`, `jobId 9ee5b0fc`) | `done`, untouched | `That session is running in the background (9ee5b0fc). Run \`claude attach 9ee5b0fc\` to open it, or \`claude stop 9ee5b0fc\` first to resume it here. Add --fork-session to branch off a copy instead.` | n/a (no interactive process; a wake of a live job is the known copy) |
    | (b) stopped (`claude stop`) | the conversation opens **interactively, with no job**: the `held` case. The job is not woken | `kind interactive`, `jobId null`, `entrypoint cli`, `sessionId` = the job's conversation, the saved name, `status idle` | stays `done` (not `running`); `resumeSessionId` unchanged | the full conversation under the name `[NIMITZ_E2EQ.SQ]-SHIP`, a prompt, "auto mode on" | a **copy**: `note: session 9ee5b0fc is open in another Claude Code process, so this started a copy as d5d9fc3a. The original conversation is unchanged.` New job, new session id, row `name` null (the session file carries the ship's name), no labels |
    | (b') parked (daemon retire) | not run (needs the 60-minute idle retire, or a crash restart); the row shape is the same as (b), so the same outcome is expected | | | | |
    | (c) after `claude rm` | the same as (b): an interactive process holding the conversation, no job row at all | `kind interactive`, `jobId null`, `entrypoint cli`, same `sessionId`, the saved name | none | as (b) | the same copy note (`started a copy as 57f54dd0`), a new job under a new id; the conversation itself is not given a job while the interactive process lives |

    `/exit` in the interactive process ends it cleanly (the session file goes, the
    job row of (b) stays `done`). Round 1's "re-exec as `claude attach`" for a live
    job is gone in 2.1.292: it refuses and names `attach`. For a restore this means
    `resume_agents_on_restore` never wakes a job: a live lead's pane shows the refusal
    (harmless), a stopped or parked member's pane turns the conversation into a `held`
    interactive process, after which nimitz's own wake makes a copy until that
    process ends (`/exit`); and a removed job's conversation is held the same way.

11. **`/branch` (and `/resume`) inside a job: `resumeSessionId` follows `/clear` only.**
    Real Claude Code, e2e cases 13 and 14 (guard `[OK]` before and after). `/branch` in an
    attached ship fires SessionStart `source: fork` with a new session id under the same
    job (the hook binds it `via branch`), and renames the job's saved name to
    `<first prompt> (Branch)`; `state.json` keeps `sessionId` = `resumeSessionId` = the
    pre-branch conversation. After a stop, a wake by the birth id (`claude --bg --resume
    <birth>`, once the daemon settled, finding 12) answers `note: woke session <birth>
    with its saved options` and brings the **pre-branch** conversation back (SessionStart
    `resume` on the birth id, same job): the branch is left behind, and nimitz's hook
    then records the woken one as the member's `session` (the branch stays in the
    lineage). This is a GAP in Claude Code's own wake for a branched job. nimitz never
    takes that path in silence: `wake_steps` compares the registry's `session` (the
    conversation the member ran last) with the row's `resumeSessionId`, and when they
    differ it removes the row and resumes the registry's conversation by id with every
    flag (`rm <job>`, then `--bg --resume <branch> --name ... --settings ...`), with the
    plan line "the job would wake on <old> (resumeSessionId follows /clear only), not on
    <branch> that it ran last (a /branch or /resume): resume that one with every flag".
    (one helper, `sessions::wake_target`, answers every wake path: `wake_steps`,
    `relaunch`, restore's attach-wake; a registry conversation that is not later in the
    lineage than the row's is `lineage.stale`, the row wins.) Case 13 (`/clear`, wake by birth id, then `/branch`, wake): the branch came back
    under its own short id with the ship's name and labels, one row, no issue (the
    relabel also undoes the `(Branch)` rename). Sandbox test
    `branch_then_stop_then_start_resumes_the_branch_with_every_flag`; the identity
    matrix test `inside_3_4d_then_e1_a_branch_then_a_retire_is_parked` now expects it.
12. **A wake right after a stop makes a copy.** The first run of case 14 woke the job by
    its birth id about 1 s after `nimitz stop`'s SessionEnd (the session file was gone,
    `status` showed no pid): Claude Code answered `note: session <job> is already running
    in the background, so this started a copy as <new>` (SessionStart `fork`, no labels;
    the daemon's own bookkeeping had not settled). Once `claude agents` listed the job
    with no pid and a state other than `running` (a few seconds), the same wake woke
    the job. `apply` now waits up to 5 s for that before any wake (`wait_settled`); the
    copy note and its cleanup stay as the backstop.

## Harness notes

- Session `kind` in `status --json` is `background` (the session file's word), not
  the `bg` of `claude agents`.
- A session's status line shows `[F.SQ]-SHIP` and a `❯` prompt after `claude attach`;
  ctrl-z returns the pane to the shell prompt and the job runs on.
- `nimitz --version` is the same string for the installed 0.3.1 and this build; the
  members' `nimitz` is a wrapper on PATH that logs each call before it execs the
  build under test, so case 9 can prove which binary ran.
- Twelve tests, four at a time: 76 s wall clock in the final run (05:56:44Z to
  05:57:59Z, guard to guard); case 6 waits the crash window (10.1 s) and case 7 a
  lead's full turn plus an attach (20 s alone).

## Cleanup

Every test stops and removes its rows, deletes its transcript folders under
`~/.claude/projects/*nimitz-e2e-<n>-<pid>*`, removes the pins it added, kills its
tmux server and removes its folder on success (kept on failure, for evidence). The
live-fleet guard printed `live fleet unchanged` for every test in every run; the
one pre-run `livediff.py` failure (`[MAIN.LUX]-DEV` 0abc1234 losing its pid) was the
daemon's own `bg retire 0abc1234: abandoned-stale, idle 48h` at 05:14:03Z, before the
first test ran; a fresh baseline was taken and held for the whole run.
