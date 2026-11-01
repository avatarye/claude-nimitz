# Area `nimitz`: every path THROUGH nimitz, per role (round 2)

Tested on Claude Code **2.1.292** (installed; nimitz warns `cc.version`, tested up to 2.1.290), nimitz **0.3.1**, admiralty 0.3.1,
herdr client 0.9.3. World `$MAP=~/.cache/nimitz-e2e/nimitz-e2e-r2-nimitz`, fleet **E2E1** (admiral, squadron SQ, ships S1 S2
managed + S3 at `$MAP/stow` + S4 at `$MAP/stow2`, tenders t1 t2 t3), migration targets E2E11/E2E12 (`--plan` only). Multiplexer:
the fake herdr `nz` wires (`$MAP/herdr-bin`), so a "pane" never runs its command; interactive leads were run by hand in the
private tmux `r2-nimitz` with the exact plan command. Member caller wrappers: `$MAP/as-cmdr.sh`, `as-adm.sh`, `as-ship.sh`
(`env -i ... NIMITZ_ID=E2E1/SQ NIMITZ_ROLE=commodore ... nimitz "$@"`, the way a member's Bash tool calls it). The org turns
haiku into Opus; about 7 paid turns in all (3 tenders, 1 stowaway x3, the commodore answered one tender message).

Session ids made: ships 1f3d0a1c (S1), b9ea8200 (S2), b97ae728 (S3, adopted stowaway), 21f85cd1 (S4, adopted stowaway);
tenders e400991c (t1), 2231320b (t2), 6ce21f6f (t3); copies/stowaways 33e4ed29 (race copy), dc07e172 (stowaway);
interactive dcaf69ca (commodore), 5facc508 and cf98bfd0 (admiral, twice). All stopped and removed at the end (section 4).

## 1. COVERAGE

| id | path | role | via | Claude Code does | nimitz shows | recovery | verdict | evidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| N1 | `nz init` | - | nimitz | nothing (no claude run) | plan writes only `$MAP/state/registry.json` (machine label); doctor: plugin hooks on, crossSessionInbound accept, 27 pinned files ok | n/a | OK | `[OK] doctor: ready`; it never touched `~/.claude/settings.json` (the doctor only plans a write when crossSessionInbound is not accept) |
| N2 | `nz fleet/squadron/ship new` | - | nimitz | nothing | needs `--yes` outside a tty ("not a terminal"); `--mode` is not a squadron flag (USAGE says it is) | n/a | OK (usage text GAP, low) | `[ERROR] unknown flag --mode` for `squadron new ... --mode default` |
| N3 | `nz up E2E1 --yes` (NZ_BIND=0) | admiral+commodore | nimitz | leads: nothing (fake pane); ships: `--bg --name --settings` cold launch, SessionStart `startup` with labels, pid 993141 / 994184 | leads `not-started`, `lineage.pending` "the registry binds it later"; ships bound `via launch` | - | OK for ships; see N6 for leads | registry S1 1f3d0a1c, S2 b9ea8200 `launch`; fake calls `workspace create`, `tab rename`, `pane run <claude --name ... --settings ...>` |
| N4 | `nz start E2E1/SQ/S1 --plan` (cold) | ship | nimitz | - | plan is top-down: admiral pane, commodore pane, then the ship; no Bind step for the leads | - | OK | plan text |
| N5 | `nz start E2E1 --yes --force` with `NZ_BIND=15`, the plan command run by hand in tmux 3 s later | admiral | nimitz+by-hand | interactive session 5facc508, SessionStart `startup` with labels, pid 1007420 | `opened in herdr pane w1:p2; bound 5facc508`, lineage `launch` | - | OK (bind works inside the window; `--force` was needed because `nz up` left a pending file, else `start` only re-focuses the pane) | adm-start.log 00:47:52 -> 00:47:56; registry E2E1 5facc508 |
| N6 R1:4-start | the commodore's exact plan command run by hand (outside any nimitz window) | commodore | by-hand | interactive dcaf69ca, labels, SessionStart `startup`, pid 1000808 | status `idle` "session file" (found by labels); **registry never gets it**; `pending/E2E1%2FSQ.json` stays; `nz start E2E1/SQ --plan` is empty with no message; `nz open E2E1/SQ` = `mux.nopane` | none: nothing binds after the window (unchanged from round 1) | GAP (nimitz-G2) | registry `E2E1/SQ: None` for the whole run |
| N7 | `as-cmdr status` / `as-ship status` | commodore, ship | nimitz | - | the full picture | - | OK | same output as the user's |
| N8 | `as-cmdr up E2E1`, `start E2E1`, `start E2E1/SQ` | commodore | nimitz | - | refused `authority` ("yours or E2E1's admiral's" / "a commodore, its own ships and tenders") | - | OK | plan output |
| N9 | `as-cmdr stop E2E1/SQ/S1 --yes` | commodore->ship | nimitz | `claude stop 1f3d0a1c`; row `done`, SessionEnd `other` with labels; `respawnFlags` keep `--name --settings` | `stopped` "job row ... no live pid (state done)" | - | OK | job state.json; events pid 993141 |
| N10 | `as-cmdr start E2E1/SQ/S1 --yes` (revive) | commodore->ship | nimitz | native `--bg --resume <sid>`, same id, SessionStart `resume` with labels, new pid 1018043 | `idle`; lineage +`resume` | - | OK | `backgrounded · 1f3d0a1c · [E2E1.SQ]-S1` |
| N11 | `as-cmdr relaunch E2E1/SQ/S1 --yes` (live) | commodore->ship | nimitz | stop, `claude rm`, `--bg --resume <sid> --name --settings`: same id, history kept (52-line transcript at the end) | `idle`; lineage +`relaunch` | - | OK | 3 OK steps; registry |
| N12 | `as-cmdr tender E2E1/SQ t1 "reply ok" --yes` | commodore->tender | nimitz | `--bg --name [E2E1.SQ]-COMMODORE/t1 --settings {... NIMITZ_PARENT ...}` in the commodore's folder; e400991c; SessionStart, UserPromptSubmit, PreToolUse, Stop; the job's turn ends (`state done`) but the **process stays live, idle at the prompt** | `idle` "session file: status idle", no issue, for as long as the daemon keeps it (60 min idle retire) | `nz stop ~t1` works; then no way to retire it: `rm ~t1` "no stale rows", `rm e400991c` refused `lineage.current`, `sweep` "nothing to do"; `tender ... t1` again = `dup.live`; TUI `a` on the stopped tender resumes it | GAP (nimitz-G4) | Stop event 00:49:37; status idle pid 1019590 at 00:50:33; plan `why: it stops when the job is done` is false |
| N13 | `as-cmdr sweep`, `sweep E2E1/SQ`, `rm E2E1/SQ/S2` | commodore | nimitz | - | allowed, scoped to its squadron; "no stale rows" | - | OK | plan output |
| N14 | `as-cmdr signal E2E1/SQ/S1 hi` / `signal E2E1 hi` | commodore | nimitz | - | refused `comms.denied` "sessions signal with SendMessage" | - | OK | plan output |
| N15 | `as-adm up E2E1 --plan`, `start E2E1/SQ --plan` | admiral | nimitz | - | allowed (0.3.1 widening); `up` with everything live = "nothing to do"; `start E2E1/SQ` on the live, unbound commodore = empty plan (also with `--force`: no second lead) | - | OK (empty plan has no "already live" line: low) | plan output |
| N16 | `as-adm start E2E1` (itself) | admiral | nimitz | - | refused `authority` | - | OK | |
| N17 | `as-adm stop E2E1/SQ` / `relaunch E2E1/SQ` (live commodore) | admiral | nimitz | - | refused `session.interactive` "type /exit in its pane" (fix marked "ask the user to run") | - | OK | |
| N18 | `as-adm stop E2E1/SQ/S2 --yes`, `start --yes` | admiral->ship | nimitz | stop; native resume, same id, SessionStart `resume` with labels (pid 1043089) | lineage +`resume` | - | OK | |
| N19 | `as-adm tender E2E1/SQ t2 --yes` | admiral->tender | nimitz | 2231320b, as N12 | bound `launch` | - | OK (same lingering as N12) | |
| N20 | `as-ship start/stop/tender/sweep` | ship | nimitz | - | every change refused `authority`; `status`/`check` allowed | - | OK | `[E2E1.SQ]-S2 may not start E2E1/SQ/S1: ...` |
| N21 | `nz stop S1`, `nz start S1` (row exists) | ship | nimitz | as N9/N10 | lineage +`resume` | - | OK | pids 1045509 |
| N22 R1:1c | `nz rm E2E1/SQ/S1` and `nz rm 1f3d0a1c` on the stopped ship | ship | nimitz | nothing | `rm <id>`: "no stale rows"; `rm <sid>`: refused `lineage.current` "removing it breaks the lineage; fix relaunch" | - | OK (the by-hand `claude rm` path of round 1 is not reachable through nimitz) | |
| N23 | `nz relaunch S1 --yes` (live) | ship | nimitz | as N11 | lineage +`relaunch`; `check E2E1` no errors | - | OK | registry S1: launch, resume, relaunch, resume, resume, relaunch |
| N24 | `nz sweep --yes` | - | nimitz | nothing | "nothing to do" | - | OK | |
| N25 | `cl --bg "reply ok"` in S3's folder, then `as-cmdr adopt <sid> E2E1/SQ/S3 --yes` | commodore->ship | by-hand+nimitz | b97ae728 unlabelled, name "reply ok confirmation" | stowaway listed; adopt = registry bind `via adopt`, warn `labels.missing`; then `name.drift` + `labels.missing` errors, `rules.old` info; `nz start S3` refuses with those | `nz relaunch S3 --yes`: stop, rm, resume with flags: same id, labels, name; lineage adopt, relaunch | OK | events SessionStart with `NIMITZ_ID: E2E1/SQ/S3` after the relaunch |
| N26 | `nz adopt <stowaway> E2E1/SQ/S2` (S2 live) | user | nimitz | dc07e172 | refused `dup.live` | `nz stop dc07e172 --yes; nz rm dc07e172 --yes`: row gone | OK | |
| N27 | `nz adopt <labelled sid> ...` | user | nimitz | - | refused `session.unknown` "not a stowaway" (also for a tender id `~t9`) | - | OK | |
| N28 | `nz open E2E1/SQ` / `open E2E1` (fake panes with the leads' session ids) | leads | nimitz | - | `focused E2E1/SQ in pane w2:p1` | - | OK | fake calls `workspace focus`, `tab focus`, `agent focus w2:p1` |
| N29 | `nz open E2E1/SQ/S1 --yes`, `open ~t1` | ship, tender | nimitz | (fake) `claude attach 1f3d0a1c` in a new tab | `attached in pane w2:p2` | - | OK | fake `tab create --label E2E1.SQ.S1`, `pane run ... claude attach 1f3d0a1c` |
| N30 | `nz signal E2E1/SQ hi`, `signal E2E1/SQ/S1 hi`, `signal E2E1 hi` | user | nimitz | - | without a pane: `mux.nopane` fix "nimitz start E2E1/SQ"; with the fake pane: `agent prompt w2:p1 hi-cmdr`; a ship's signal is typed to the commodore: `Relay to [E2E1.SQ]-S1 from the user: hi-ship` | - | OK | fake calls.jsonl |
| N31 | `nz layout herdr E2E1 [--ships] --yes` | - | nimitz | (fake) `claude attach` per ship | leads live: `layout.skip` "`nimitz open` shows it"; `--ships` attached S1 **again** in a second tab (w2:p3 after `open` had made w2:p2); message "members bind at their SessionStart" is wrong for attach panes | - | OK (low: double attach with the fake; a real herdr may report the attach pane's session) | fake calls |
| N32 | `nz migrate fleet-skill <fixtures> --into E2E11/E2E12 --plan` | - | nimitz | - | FREE -> E2E11/FREE 4 ships, 5 legacy names; GX+LUX -> E2E12 with host-merge warnings and `folder.untrusted` errors for the LUX Dropbox folders | - | OK (plan only; `--yes` not run: it would be a config-only write) | plan output |
| N33 | admiralty TUI (`admiralty` in tmux on the test world) | - | nimitz | - | tree, help, `i` issues (S4 name.drift/labels.missing with fixes), status line "+12 outside the config" (other areas' sessions) | - | OK | capture-pane |
| N34 | TUI `enter` on the commodore, `A` on S1 | leads, ship | nimitz | (fake) focus / attach | `agent focus w2:p1`; `A`: "Open already: focused pane w2:p2" | - | OK | fake calls |
| N35 | TUI `x` -> `s` stop S2 -> `y` | ship | nimitz | `claude stop b9ea8200` | confirm shows command, steps, why; result "(1 steps, verified)"; row `x stopped` | - | OK | admiralty-actions.log |
| N36 | TUI `x` -> `r` relaunch S2 -> `y` | ship | nimitz | rm + resume with flags; same id | lineage +`relaunch`; menu text says "relaunch (new session, ...)" while the confirm says "same session id and history" | - | OK (wording GAP, low) | |
| N37 | TUI `a` on the stopped tender t1 | tender | nimitz | native `--bg --resume e400991c`: the tender is live and idle again | `idle` | - | OK as a resume; see N12 | |
| N38 | TUI `x` -> `w` sweep / `x` rm on t1 and S2 | - | nimitz | - | "no stale job rows" (greyed) | - | OK | |
| N39 | TUI `x` -> `d` adopt on the stowaway 21f85cd1 -> identity E2E1/SQ/S4 -> `y` | ship | nimitz | - | the form was prefilled with the word "then" ("not in any fleet config"); after typing the id: confirm with the `labels.missing` warning; bound `via adopt` | `nz relaunch` as N25 | OK (prefill GAP, low) | registry S4 adopt |
| N40 R1:1g-race | `kill -9 <S2 pid>`, 1 s later `nz start E2E1/SQ/S2 --yes` | ship | by-hand+nimitz | nimitz's `--bg --resume b9ea8200` ran while the daemon was restarting the crashed job: Claude Code made a **copy 33e4ed29** with the identity's name and no labels; 6 s later the daemon re-claimed b9ea8200 (`bg claimed-spare b9ea8200 (shell)`) | **`[OK] E2E1/SQ/S2: claude --bg -> backgrounded · 33e4ed29`** and `verified`; 20 s later: `name.taken` Error on S2 + stowaway 33e4ed29 named `[E2E1.SQ]-S2`; lineage +`resume` (the daemon's restart) | `nz stop 33e4ed29 --yes; nz rm 33e4ed29 --yes` by hand; `check S2` clean after | BREAK (nimitz-G1; unchanged from round 1) | daemon.log 00:56:27 `settled (killed)`, 00:56:33 `claimed-spare`; two `cl agents` rows named `[E2E1.SQ]-S2` (33e4ed29 pid 1114163, b9ea8200 pid 1118555) |
| N41 | `/exit` in the commodore (interactive, unbound) | commodore | by-hand | SessionEnd `prompt_input_exit` with labels; transcript dcaf69ca.jsonl stays | `stopped` "no session row; a transcript in its folder (continue with -c)" | `nz start E2E1/SQ --plan` = `claude -c --name --settings` in a pane | OK | events pid 1000808 |
| N42 | `/exit` in the admiral that was bound (5facc508) but **never prompted** | admiral | by-hand | SessionEnd `prompt_input_exit`; **no transcript file at all** (no project folder) | `stopped` "registry knows it, but no row and no transcript", `lineage.broken` Error; **every start under the fleet refuses**: `start E2E1/SQ`, `start E2E1/SQ/~t3`, `up E2E1`, even `start E2E1/SQ/S1 --force` | only `nz start E2E1 --force` (cold launch); with NZ_BIND=20 and the admiral command run by hand it bound cf98bfd0 and the lineage became `[5facc508 launch, cf98bfd0 launch]` (old entry kept); then `start ~t3 --plan` was clean | GAP (nimitz-G3) | `[ERROR] E2E1: E2E1's current session 5facc508 ... has no job row and no transcript (lineage.broken)` on `nz start E2E1/SQ/S1 --force --plan` |
| N43 R1:4a/G3 | `nz start E2E1/SQ` (`claude -c`) after a **newer tender transcript** (t3, 6ce21f6f) appeared in the commodore's folder | commodore | nimitz+by-hand | `claude -c` continued **dcaf69ca (the commodore)**, not the newer tender transcript; SessionStart `resume` dcaf69ca pid 1137135 | status `idle` (by labels); registry still `None` (bound outside the window: the top-down start waited 20 s on the admiral pane first, so the commodore's SessionStart came before that step's `since`) | - | OK for `-c` (a background tender in the same folder does not hijack `-c` on 2.1.292); GAP for the bind (nimitz-G2) | `ls -t` showed 6ce21f6f.jsonl newest; events |
| N44 | `nz ship rm E2E1/SQ/S4 --yes` (live) | ship | nimitz | `claude stop`, `claude rm`, config write | "retire: stop, remove job rows, remove from the config"; row gone; registry keeps the S4 entry (hidden) | - | OK | |
| N45 | `nz fleet rm` / `squadron rm` while members are live | - | nimitz | - | refused `config.nonempty` + `session.live` per live member ("type /exit in its pane" for the lead) | - | OK | |
| N46 | `nz machine rm E2E1 home-server --plan` | - | nimitz | - | plan + WARN listing 8 live members "ships lose their folder here at their next stop" | - | OK | |
| N47 | `nz start 1f3d0a1c` (a session id as target) | - | nimitz | - | usage error "not an id" | - | OK | |
| N48 | `nz sandbox tick $SB` | - | nimitz | - | runs forever (no exit, no output in 60 s); had to `pkill` it | - | N/A (not a session path; note the hang) | |
| N49 R1:6f | `nz status` on this NIMITZ_HOME | - | nimitz | - | every labelled session of other fleets and areas is `config.invalid` with fix `nimitz rm <sid>`: HOME/GITHUB/NIMITZ_DEV 9aacf927, MAIN/LUX/DEV 0abc1234, E2E2..E2E4; `nz doctor` also fails on a foreign job row (`jobs/c49147e6/state.json lacks name`) | - | GAP (nimitz-G7; unchanged) | status output |
| N50 | herdr plugin, private server (`e2e_isolated.py`, patched copy) | - | plugin | - | prefix+f opens the overlay focused, label "Admiralty", renders, toggles closed, `q` closes, stale id reopens, moves between tabs: all OK; the jump check fails because the TUI opens on the first fleet tab (BETA) and the script expects `[MAIN]-ADMIRAL` in `dump` | - | GAP (nimitz-G6, test only) | `[ERROR] enter on the admiral jumps (fake herdr): []` |

## 2. NEW PATHS (not in the list)

- **A tender's end is not a path nimitz has.** A done tender stays a live idle process until the daemon retires it; `stop` works,
  but `rm`/`sweep` never drop its row or registry entry, and the TUI's `a` revives it (N12, N37).
- **`--force` on a child does not unblock a broken parent** (N42): `nz start F/SQ/S1 --force` still refuses on the admiral's
  `lineage.broken`; only `nz start F --force` does.
- **`nz up`'s pending file**: after `nz up` with a slow pane, `nz start <lead>` for 600 s only re-focuses the pane
  (`lineage.pending`, BUG-47) and needs `--force` to open anew (N5).
- **`claude -c` on 2.1.292 skips a newer background transcript in the same folder** (N43): the commodore's `-c` recovery is safe
  against its own tenders. Round 1's G3 (a newer *interactive* transcript) still applies.
- **TUI `D` (remove from config)** on a live ship = `ship rm` retire (stop, rm rows, config). `nz rm <current sid>` is refused.
- **Admiralty `dump`** prints only the current (first) fleet tab; any test that greps another fleet's row in it fails (N50).
- Not run: `nz doctor --fix` (it may write `~/.claude/settings.json`), `nz migrate --yes`, `nz hook` by hand, `sandbox tick`
  as a path, TUI `u`/`c`/`e`/`m` (`m` = `nz signal`, tested), the `claude -c` recovery while an interactive by-hand
  transcript is newer (round 1, area by-hand).

## 3. GAPS

**nimitz-G1 (BREAK, high) crash + `nz start` race makes a named copy and nimitz prints [OK].** Trigger: a ship's worker dies
(kill -9, OOM) and `nz start` (or a commodore's revive) runs before the daemon's own restart (about 6 s here). Breaks: Claude Code
starts a copy with the identity's name and no labels; nimitz prints `[OK] ... backgrounded · <copy id>` and `verified`; afterwards
`name.taken` + a stowaway; the daemon's restart is bound as `resume`. Evidence: N40. Repro (fresh world, ship live):
`P=$(python3 -c "...pid of name [F.SQ]-S1 from ~/.claude/sessions...")`; `kill -9 $P; sleep 1; nz start F/SQ/S1 --yes`; `sleep 20; nz status`.
Fix hint: compare the printed short id with the expected one; wait out a `state crashed` row.

**nimitz-G2 (GAP, high) interactive leads bind only inside `open_step`'s window; nothing binds later.** Trigger: `nz up`/`start`
of a lead whose pane starts after NIMITZ_BIND_TIMEOUT (trust prompt, slow herdr), or a lead started by hand with the plan
command, or a top-down start where an earlier pane step eats the window (N43). Breaks: `lineage.pending` says "the registry binds
it later" but the registry never gets the lead (`E2E1/SQ: None` all run); `status` works by labels, but `relaunch`/`check`
lineage logic has nothing; the pending file then makes `nz start <lead>` a focus-only no-op for 600 s. Evidence: N6, N43.
Repro: `NZ_BIND=0 nz up F --yes`, run the printed commodore command in tmux, `nz status`, `cat $MAP/state/registry.json`.

**nimitz-G3 (GAP, med-high) a lead that exits before any prompt breaks the whole fleet's lineage.** Trigger: `nz up`, the
admiral pane opens and binds, the user closes it (`/exit`, closing the tab) without typing a prompt: Claude Code writes no
transcript. Breaks: `lineage.broken` on the admiral; every `start`/`up` under the fleet refuses, `--force` on the child too;
the only fix is `nz start F --force`. Evidence: N42. Repro: `NZ_BIND=15 nz start F --yes` with the admiral command run by hand,
`/exit` at once, then `nz start F/SQ/S1 --plan`.

**nimitz-G4 (GAP, med) a tender never ends through nimitz.** Trigger: any `nz tender`. Breaks: the process stays live and idle
("it stops when the job is done" is false), nimitz shows `idle` with no issue; after `nz stop` the row and registry entry are
permanent (`rm` = no stale rows / `lineage.current`, `sweep` = nothing); the same task name is `dup.live` while it lingers; the
TUI's `a` revives it. Evidence: N12, N37, N38. Repro: `nz tender F/SQ t1 "reply ok" --yes; sleep 30; nz status; nz stop F/SQ/~t1 --yes; nz rm F/SQ/~t1 --plan; nz sweep --plan`.

**nimitz-G5 (GAP, low) wording.** TUI `x` menu "relaunch (new session, ...)" vs the confirm "same session id and history" (N36);
`layout` "members bind at their SessionStart" for attach panes (N31); `start <live lead> --plan` prints an empty plan with no
reason (N15); the adopt form is prefilled with "then" (N39); USAGE lists `--mode` for `squadron new` (N2); tender plan `why`
(N12).

**nimitz-G6 (GAP, low, tests) `herdr-plugin/tests/e2e_isolated.py` is stale on 0.3.1.** `admiralty dump --sandbox` shows the
first tab (BETA) only, so `assert "[MAIN]-ADMIRAL" in dump` fails before any herdr check; with that patched, the jump check
needs a tab switch and still found no pane row. The overlay/toggle checks pass in a private server. Repro:
`cd herdr-plugin && uv run --with pyte python tests/e2e_isolated.py`.

**nimitz-G7 (GAP, med, safety; R1 6f unchanged) another NIMITZ_HOME's sessions are `config.invalid` with fix `nimitz rm <sid>`.**
`nz status` on the test home listed the live HOME and MAIN sessions (9aacf927, 0abc1234) with that fix; `nz doctor` fails on a
foreign job row. Evidence: N49. Repro: `NIMITZ_HOME=/tmp/x nimitz init --yes; nimitz status`.

## 4. CLEANUP and livediff

- `livediff` before cleanup and after cleanup: `[OK] live fleet unchanged` (26 live entries in `live-before.json`).
- Ended through nimitz: `ship rm` S1 S2 S3 (S4 earlier), `stop` t1 t2 t3 then `claude rm` e400991c 2231320b 6ce21f6f,
  `squadron rm E2E1/SQ`, `fleet rm E2E1` (both archived their toml). Race copy 33e4ed29 and stowaway dc07e172: `nz stop` + `nz rm`.
  Interactive dcaf69ca, 5facc508, cf98bfd0 ended with `/exit`. No job dir, no `claude agents` row and no session file of mine is left.
- tmux server `r2-nimitz` killed; the hung `nimitz sandbox tick` killed; the plugin script's temp dirs removed by itself;
  the private herdr server stopped by the script (user's server untouched, `herdr status` client 0.9.3 only).
- `$MAP` (`~/.cache/nimitz-e2e/nimitz-e2e-r2-nimitz`, incl. the sandbox `sb-test` and the patched script copy) removed.
- Transcript folders left under `~/.claude/projects/` (delete freely):
  `-home-user--cache-nimitz-e2e-nimitz-e2e-r2-nimitz-nimitz-e2e1-ships-sq-s1`, `...-ships-sq-s2`, `...-squadrons-sq`
  (commodore + 3 tenders), `-home-user--cache-nimitz-e2e-nimitz-e2e-r2-nimitz-stow`, `...-stow2`.
- Nothing in the repo was modified.
