# Adversarial test report, round 1

Date: 2026-10-04. Tester: Opus agent (tester-opus). Base: dev 090f77a.
Layers: sandbox (`/tmp/nz-sb-*`, fake claude, fake herdr); real Claude Code
2.1.288.1000 (wrapper build) in `~/.cache/nimitz-e2e/nimitz-e2e-adv1`, fleet
`NIMITZ_E2E_ADV`, model haiku. 17 real sessions started; all stopped and
removed; none left. Scenario G (Admiralty) not run in this round.

Regression tests: `crates/nimitz/tests/adversarial_round1.rs`, one per bug,
`#[ignore = "BUG-nn"]`. Run: `cargo test --release --test adversarial_round1 -- --ignored`.

| Result | Scenarios |
| --- | --- |
| PASS (17) | A1, A4, A6, A7, B1, B2, B6, B8, B9, C5, D1, E1, E3, E4, F1, F2, H2 |
| PARTIAL (11) | A3, A5, B4, B7, C1, C2, C3, C4, C6, D2, H1 |
| FAIL (7) | A2 (BUG-02, 03), B3 and B5 (BUG-11), B10 (BUG-13), B11 (BUG-14), E2 (BUG-22), F3 (BUG-23, 29) |

## Bugs

"real" = also seen on real Claude Code.

| Bug | Severity | Area | Finding |
| --- | --- | --- | --- |
| 01 | unclear message | mux | `NIMITZ_MUX=auto` or an unknown value falls back to none; the fix says "run inside Herdr (HERDR_ENV=1)" while HERDR_ENV=1 is set. |
| 02 | makes it worse | core-sessions | Two `start` of one not-started ship at once make two live sessions; both say [OK]. |
| 03 | makes it worse | core-sessions | Concurrent `start` of a stopped ship resumes a live session; copies reported [OK]. |
| 04 | wrong state | core-config | A symlinked absolute `--dir` (/tmp vs /private/tmp) reads as folder.untrusted; the fix cannot help. |
| 05 | unclear message | cli | `ship set --dir ''` removes the folder silently; check green; start refuses with no fix. |
| 06 | wrong state (fails open) | core-sessions | Partial or inconsistent labels make the caller the user. |
| 07 | cosmetic | all | Wording: "an commodore"; refused stop says "may not start"; authority note as [OK]; scoped sweep says "sweep everything"; bad version "(not found)". |
| 08 | fix does not run | core-sessions | name.taken fix `nimitz stop <short>` refuses when the holder is outside fleet scope. |
| 09 | unclear message | core-sessions | config.invalid used as a catch-all; fixes suggest raw `claude stop`. |
| 10 | makes it worse (real) | core-sessions | `relaunch` ignores name.taken; two live sessions answer to one address. |
| 11 | wrong state (real) | core-sessions | A hand-made copy (`<name>-melodic-mitten`, or no name via wrapper) is only an anonymous stowaway; no copy.rows. |
| 12 | wrong state (real) | core-sessions | No row but a transcript is reported as "no transcript"; start cold-launches and drops history. Real CC keeps the id on `--resume <id>` with flags. |
| 13 | makes it worse | core-sessions | Empty or truncated session file of a live ship reads as stopped; start plans a resume of a live session. |
| 14 | wrong state | core-sessions | Corrupt jobs/<id>/state.json is ignored and the ship is cold-launched. |
| 15 | makes it worse | core-config | One bad squadron.toml hides the whole fleet; live members get `nimitz rm` fixes that do not run. |
| 16 | wrong state | core-config | Two ships with one folder (hand edit) not reported. |
| 17 | wrong state | core-config | CLAUDE.md in NIMITZ_HOME/, squadrons/, ships/ or <fleet>/.claude/ not reported. |
| 18 | makes it worse | registry | Corrupt registry.json replaced silently; no backup, no warning; lineage lost. |
| 19 | unclear message | core-sessions | Adopt as a tender impossible ("add it first"). |
| 20 | wrong state | core-config | `machine add <label>` records this host for any label; `machine rm` of this machine with live members: no warning. |
| 21 | wrong state | core-sessions | Stale .nimitz-ship.lock after an outside stop blocks other machines forever. |
| 22 | wrong state (real) | hook | SendMessage to `uds:/tmp/cc-socks/<pid>.sock` bypasses comms; delivered under commodore-only. |
| 23 | wrong state | core-sessions | SessionStart lost: labelled ship shows as stowaway; labels in the job row (respawnFlags) never read. |
| 24 | wrong state | core-config | `check F` / `check F/SQ` skip members below the target. |
| 25 | unclear message | migrate | Real import reports none of the config.layout errors (fixed later in dev 9883486; re-test). |
| 26 | wrong state | core-config | A ship may be named COMMODORE and take the commodore's address. |
| 27 | cosmetic | cli | Fix `nimitz squadron new MAIN NOPE` is invalid; should be MAIN/NOPE. |
| 29 | wrong state (real) | core-sessions | Labels kept per session id from any old event; a same-id resume with no labels still shows clean. |
| 30 | wrong state (real) | core-sessions | A labelled session in the wrong folder is accepted as the identity; a fake admiral blocked every start. |
| 31 | wrong state (real) | core-sessions | After a folder move + `ship set --dir`, check green but start fails (workspace not trusted); relaunch works. |

## Scenario G (Admiralty), run on dev a0a0d1d

Private tmux (`tmux -L nzadv`), every HERDR_* unset; sandboxes and `--demo`;
sizes 50x12, 80x24, 120x40, 250x60. Tests: `crates/admiralty/tests/adversarial_g.rs`.
Totals with G: 18 PASS, 14 PARTIAL, 8 FAIL.

| Check | Result |
| --- | --- |
| G1 states shown | PARTIAL (35, 38) |
| G2 no mux | PARTIAL (34) |
| G3 sizes, icons | PARTIAL (36, 37) |
| G4 rapid refresh | PASS |
| G5 forms and commands | FAIL (32, 33) |

| Bug | Severity | Finding |
| --- | --- | --- |
| 32 | makes it worse | D (remove from config) on a live ship only rewrites squadron.toml; the ship keeps running and leaves the tree. |
| 33 | shown command is not what runs | The form shows `--dir sandbox=managed`; the CLI stores that as a relative folder. |
| 34 | wrong state | Monitor-only disables start/relaunch of ships, which need no mux. |
| 35 | unclear message | One bad squadron.toml removes the fleet from the screen; header says 0 live while 8 run; parse error cut off. |
| 36 | unclear message | 80x24: DETAIL shrinks to "…", header loses mux state, footer loses help/quit, i view cuts messages, help popup clipped, detail shows no reasons. |
| 37 | cosmetic | Nerd-font issue icons shift rows and popup borders by one cell in tmux. |
| 38 | unclear message | A legacy warning on the commodore shows in every start dialog, also for other ships; wrapped text loses indent. |
| 39 | testability | `--sandbox` forces NIMITZ_MUX=none; a `--herdr` sandbox's fake Herdr goes unused. |
| 40 | cosmetic | Keys pressed during the ~3 s startup "loading" are dropped. |

Also: the form accepts a ship named COMMODORE (BUG-26); the demo's role.background fix says `nimitz open FORGE` while core says `claude attach <short>`.

# Re-test of round 1 and round 2 (dev 45f627f)

Real Claude Code: 2.1.288.1000 (wrapper build), `~/.cache/nimitz-e2e/nimitz-e2e-adv2`, fleet
`NIMITZ_E2E_ADV2`; 22 real sessions, all stopped and removed.

## Re-test of round 1

27 of 31 FIXED. Partly open on 45f627f: 07 (idle resumed ship says "job row left a
question"; `2.1.288-beta` counts as 2.1.288), 15 (fixed later on dev), 30 (fix
`nimitz stop` refuses: BUG-52), 33 (CLI `--dir sandbox=managed`; fixed later in a4fff05).

## Round 2: passed

Prompt injection (quotes, backticks, `$()`, newlines; real); unicode and spaces in
folders (real); 40 ships + 51 sessions (status/check ~50 ms, hook ~6 ms); session
ending right after start (real); nimitz from a ship's Bash tool (authority refused);
odd version strings; clock skew; full disk for NIMITZ_STATE (start refuses, registry
valid); ctrl-c during bind; migration flow (real; history lost, BUG-53).

## Round 2: bugs

| Bug | Severity | Area | Finding |
| --- | --- | --- | --- |
| 42 | unclear message | core-sessions | Legacy warning about a stray in the commodore folder shows in every start; its adopt fix would adopt a background session as the commodore. (passes on dev 8fd6f8d) |
| 43 | wrong state | cli/mux | `nimitz signal` types newlines, ctrl-C and ESC sequences into `herdr agent prompt`. |
| 44 | performance | core-sessions | events.jsonl never rotated; at 200k lines status takes 0.57 s. |
| 45 | cosmetic | core | `2.1.288-rc.1+build` reads as no version; `-beta` counts as release; absurd timestamp age "2073…". |
| 47 | makes it worse | cli/mux | Repeated start while an admiral/commodore pane has not bound opens another pane each time. |
| 48 | wrong state | core-sessions | Symlinked NIMITZ_HOME: transcript and stowaway missed. (passes on dev 8fd6f8d) |
| 49 | low | core | Full disk: the hook leaves a partial line; the next event is glued onto it. |
| 50 | wrong state (real) | core-config | Home-folder trust does not cover a git repo root or worktree; check green, start fails "Workspace not trusted". |
| 51 | cosmetic | cli | A ship running `nimitz start <live ship> --plan` gets "nothing to do" and a user-only fix. |
| 52 | fix does not run (real) | core-sessions | BUG-30's fix `nimitz stop <short>` refuses for a session with a member's labels in the wrong folder. |
| 53 | makes it worse (real), HIGH | core-sessions | relaunch / relabel / adopt+start / migration run `--resume <id>` with flags while the old job row exists: a new id with NO history. Verified fix: stop, `claude rm <row>`, then `--bg --resume <id>` with all flags: same id, new labels, history kept. |
