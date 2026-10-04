# Adversarial test report, round 1

Date: 2026-10-04. Tester: Opus agent (tester-opus). Base: dev 090f77a.
Layers: sandbox (`/tmp/nz-sb-*`, fake claude, fake herdr); real Claude Code
ACME 2.1.288.1000 in `~/.cache/nimitz-e2e/nimitz-e2e-adv1`, fleet
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
