# CLI UX: Perforce-style member paths, and session verbs apart from pane verbs

Status: WIP, paused (2026-10-12). To be redone from the commodore's agreed
plan. New direction: no backward compatibility (no warning releases or
migration paths, just switch); quick iteration over polish; four command
families (fleet shape, session control, pane control, misc). The migration
sections below (2.6, 3.5) are superseded by that. Earlier: proposal, not built. Written against 0.7.1 (`dev` at 4df9b06).
Code references are to that tree.

## 0. The ask

1. Wildcards in member paths: `nimitz start HOME/*`, `HOME/GITHUB/*`, and
   wherever else they make sense; revised to Perforce's convention exactly
   (a path names one member, `...` is everything under it).
2. Verbatim: "I also want to have a smoother ux with the commands. The main
   actions are 2 types: fleet sessions and pane operation. They are not
   clean. How to have commands that are more user friendly and logically
   clean?"

## 1. What the commands do today (verified in code)

| Fact | Where |
| --- | --- |
| `start` takes one target; a second positional is refused | crates/nimitz/src/members.rs:231 |
| `start F` covers every member of F, `start F/SQ` the commodore and its ships, `start F/SQ/SHIP` the ship | crates/nimitz-core/src/restore.rs:841-849 (`in_target`) |
| Inside that scope: leads always come back; ships only from the active set (or every one with `--all`); stopped ships stay stopped | restore.rs:1012-1015, 1056, 1159 |
| Only the *named* member (the target's own: admiral, commodore or ship) may get a first launch, `--fresh` or `-p`; a fleet- or squadron-wide start never cold-launches the rest | restore.rs:567-573, 939-940 (`not.started`) |
| A lead alone needs `--only`, because a lead's id names its subtree | restore.rs:631-632; sessions.rs:424-427 (`start_cmd`); Admiralty adds it, crates/admiralty/src/actions.rs:462-465 |
| `stop` takes one id and stops that one member: `stop HOME/GITHUB` stops the commodore only, not the squadron | members.rs:266-276; sessions.rs:1311 |
| `status` takes no argument (`nimitz status HOME` errors "unexpected argument"), yet five fix lines say `nimitz status <id>` | members.rs:50, 604, 608, 694; crates/nimitz/src/restore.rs:312, 335 |
| `check [<id>]` and `sweep [<id>]` already take a subtree | members.rs:148-170, 300-306; sessions.rs:1721 |
| `<cmd> --help` is not supported (`nimitz stop --help` says "missing id or session") | members.rs:269 |
| `restore`, `relaunch`, `open`, `up`, `layout herdr` are still in the binary, "gone in the next release" since 0.5.0 | main.rs:126-131, 238-249 |
| Ids are 1-3 names of `[A-Z0-9_]+`; aides (`F/@NAME`) are not ids for these commands | crates/nimitz-core/src/types.rs:97-100, 601 |

Authority (sessions.rs:429-439, `may_start`): you, anything; an admiral, its
fleet but itself; a commodore, its own ships; a ship, nothing. `-p` is yours
alone (restore.rs:581). A whole-fleet target from a member is already
*narrowed*, not refused: a commodore's `start F` means its own ships
(restore.rs:609-612), and a wide start skips the caller with a `start.self`
line.

## 2. Member paths, Perforce style

Your rule (correcting the first two drafts): Perforce's convention exactly,
and today's "a bare lead id covers everything under it" goes.

### 2.1 A path names one member

| Path | Names |
| --- | --- |
| `HOME` | the admiral, only |
| `HOME/GITHUB` | the commodore, only |
| `HOME/GITHUB/NIMITZ_DEV` | that ship |

Every verb reads a path the same way, so `start HOME/GITHUB` and `stop
HOME/GITHUB` finally act on the same member (today they do not: section 1).
`--only` has nothing left to do and goes (2.6).

### 2.2 `...`: everything under, at any depth, the lead included

`HOME/...` is the admiral, every commodore and every ship of HOME;
`HOME/GITHUB/...` the commodore and its ships. It is the old bare `HOME`,
spelled out.

Whether the lead is included: by the letter of Perforce, `//depot/...`
matches paths that start with `//depot/`, and the depot itself is not a
file. But the lead *is* the node itself, and the commodore's reading is the
right one: `//depot/...` is "this directory and everything in it", and what
people mean by `HOME/...` is "the fleet". Including the lead also makes
`start HOME/...` a drop-in for today's `start HOME`, which is what keeps the
migration (2.6) mechanical. Recommended: **included**. "The members below,
not the lead" is the one-level form (2.3) or an explicit list.

`...` follows today's wide-start rules, unchanged: leads always come back,
ships from the active set, `--all` wakes the rest, a never-started member is
listed under `not.started` and launched only with `--launch` (a pattern must
never spin up a dozen fresh sessions by accident; today's fleet-wide start
never cold-launches either, restore.rs:939-940). A **named path** keeps the
named treatment: woken whatever its state, a first launch when it never ran,
`--fresh`, `show`.

### 2.3 One level: `*`, quoted, as a whole segment

Perforce's one-level wildcard is `*`, which zsh globs: unquoted, zsh stops
with "no matches found" before nimitz runs (harmless: nothing happens);
bash passes it literally unless a file matches. The tasks that need one
level and are not covered by `...` plus names:

| Task | With `*` | Without |
| --- | --- | --- |
| The commodores alone | `'HOME/*'` | `HOME/GENERAL HOME/GITHUB` (one name per squadron) |
| A squadron's ships, not its commodore (a commodore's everyday scope) | `'HOME/GITHUB/*'` | a commodore's `HOME/GITHUB/...` includes itself, which is skipped with the `start.self` line anyway |
| Every ship of a fleet | `'HOME/*/*'` | one `F/SQ/...` per squadron, which brings the commodores too |

Recommended: **support `*` as a whole segment, documented quoted**
(`'HOME/*'`), not as part of a name (`*_DEV` waits until it is asked for). A
first release could leave it out: `...` plus explicit names cover reboot,
update, a squadron and one member, and a commodore's skip makes
`HOME/GITHUB/...` do the right thing for its ships. The bash failure mode is
bounded: a file name that bash substitutes (`HOME/README`) is not a member,
so it is refused; only a folder `HOME/GITHUB` in the current directory could
turn into a real member, and that is far-fetched.

No other one-level token: `..` would read as "parent" to every shell user.

### 2.4 The shell (checked on this machine, read-only)

- `print -r -- HOME/... HOME/GITHUB/... ` prints the words unchanged: dots
  are not glob characters in zsh or bash.
- Your `..`, `...`, `....` are plain aliases (`cd ..` and up), which expand
  only in command position, never as arguments.
- oh-my-zsh defines *global* aliases (`alias -g`) for `.....` and
  `......`, which expand on any standalone word; on a machine without your
  aliases.zsh override, `...` and `....` are global too.
- A global alias matches a whole word only, so `HOME/...` is never touched;
  a bare `...` argument would be.

Rule: **`...` is always attached to a path, never a bare argument.** The
first segment is always a fleet name; `nimitz start ...` is refused with the
fix `nimitz start` (no target already means every fleet here, for you; a
member's own scope for a member). `...` is the last segment only;
`HOME/.../X` is refused.

### 2.5 Plans, empty matches, authority

The plan headline names each target and what it matched:
`start HOME/GITHUB/...: 5 matched (GITHUB, NIMITZ_DEV, MYCC_DEV, BERGAMOT, BARN_DEV); 3 wake, 2 nothing`.
An unknown path or an empty match is a refusal (`member.unknown`,
`pattern.empty`) whose fix lists what is there at that depth. Several
targets may be given; a member matched twice counts once.

A pattern never widens what a role may do. When any match is outside the
caller's authority the whole command is refused, naming those matches and
the pattern that fits (a commodore's `start HOME/...` → `start
HOME/GITHUB/...`). Refuse rather than filter, although today's member
`start F` filters to the caller's part:

- A target is an explicit request; dropping part of it silently makes the
  plan differ from it, and an agent reading "[OK] 3 up" will not notice
  that 5 were asked.
- The refusal is cheap (its fix is the pattern that fits) and teaches the
  agent its scope instead of hiding it.

Kept from today: a match that is the caller itself is skipped with the
`start.self` line (nothing relaunches itself), never refused; and a member's
`start` with no target means its own scope (an admiral its fleet, a
commodore its ships).

### 2.6 Migration of the bare path

This changes behaviour: `nimitz start HOME` stops waking the fleet. For one
release (N):

- A bare fleet or squadron path that used to mean "everything under"
  (`start`, `start -f`, `check`, `sweep`; `stop` and `rm` already take one
  member) **keeps acting as before** and prints
  `[WARN] nimitz start HOME means HOME/... in this release; from N+1 it is the admiral alone: use nimitz start HOME/...`.
  `--only` keeps working, means the new reading, and silences the warning.
- Why keep the old action rather than switch with a warning: `start HOME` is
  the reboot command in hands, in Admiralty's `R` key, in fix lines that sit
  in member transcripts and KB notes, and in the rules a running member read
  at its last prompt. Switched, a reboot would bring up the admiral, print
  `[OK]`, and leave the fleet down: a failure that looks like success. The
  old reading is a superset of the new one, so acting on it never leaves a
  member down that you named. The warning names the exact new form, so the
  habit moves within the release.
- N+1: a bare path is one member; `--only` is accepted as a no-op with a
  pointer for one more release, then goes.

What moves to the new forms in release N (all of it, so nothing nimitz says
uses the old reading):

- Fix and hint text: `sessions::start_cmd` (sessions.rs:424-427) drops
  `--only` for leads, so the `start.self` fix becomes `nimitz start HOME -f`
  (admiral.md:9-11 says `nimitz start {FLEET} --only -f`); fleet-wide fixes
  name `F/...` (`tab.resumed` and `lead.interactive` at restore.rs:984-995,
  the `mux.failed` and `mux.none` fixes at crates/nimitz/src/restore.rs:132,
  147; 21 `start_cmd(` call sites in total).
- Admiralty: `actions.rs:460-468` passes the path (`F/...` for a fleet or
  squadron row, the bare id for a member row) and stops adding `--only`.
- Tests: crates/nimitz/tests/restore.rs (12 `--only` uses), cli.rs (2), and
  every `start F` in e2e.rs, upgrade.rs and adversarial_round1.rs move to
  `F/...`; one new test per reading of the bare path in N (old action plus
  warning) and N+1.
- Rules, skills, README: section 3.6.

### 2.7 Which commands take patterns

| Command | Patterns | Note |
| --- | --- | --- |
| `start`, `stop` | yes, several targets | `stop HOME/GITHUB/...` stops a squadron, ships first, commodore last |
| `status`, `check`, `rules` | yes | `status [<path>...]` also makes the five existing `nimitz status <id>` fix lines true |
| `sweep`, `rm` | yes | both gated by plan and `--yes`; `rm` removes rows, never transcripts |
| `show`, `close` (section 3) | yes | yours alone |
| `signal` | no | a broadcast to many members is its own feature (decision 7) |
| `adopt`, `attach` | no | one session to one id; one terminal |
| config (`ship`, `squadron`, `fleet set`) | no | config changes stay one at a time |

Build note: expansion belongs in the CLI before the planners
(`members::parse_id` grows a `parse_targets`), but `restore::plan_start` and
`plan_inner` hold one `target` and one `named` (restore.rs:574, 600-633) and
must take a set of named members plus a set of subtrees;
`sessions::may_run_in` (sessions.rs:471-511) reads the path in a fix line
and needs the same expansion.

## 3. Session verbs and pane verbs

### 3.1 Where the two are mixed today

| # | Mixing | Where |
| --- | --- | --- |
| 1 | `start` makes every lead's tab whenever a multiplexer is there; a jobs-only start needs the negative flag `--jobs-only` | members.rs:239; restore.rs:1007-1008 (`wants_tab` true for every lead) |
| 2 | Panes are flags on the session verb: `-p`, `--tab`, `--only -p`, with six usage refusals guarding the combinations | members.rs:218-237; restore.rs:581-598 |
| 3 | `start F -p` (the admiral skill's own example) also brings up the whole fleet; the clean form today is `start F --only -p` (section 2 makes bare `F` the admiral alone) | plugin/skills/admiral/SKILL.md:53; README.md:53 |
| 4 | `start -f` reattaches the panes the relaunch cut; correct, but it is pane work done by a session verb | restore.rs:1457 (`wanted = lead \|\| had_tab \|\| pane_wanted`) |
| 5 | `attach` (a terminal verb) runs start's plan to wake a member that is down | members.rs:673-690 |
| 6 | `signal` opens a lead's tab when it has none, then types into it | members.rs:412-433 |
| 7 | Members do pane work in your Herdr: an admiral's `start F` makes the leads' tabs and reattaches panes (only `-p` is refused). Read from code, not observed | main.rs:46-51 (`member_mux_vars`); restore.rs:581, 1007 |
| 8 | Scope words differ by verb: `start HOME/GITHUB` is a squadron, `stop HOME/GITHUB` one commodore (fixed by section 2) | section 1 |

The "panes are the user's" rule is therefore enforced for `-p` only.

### 3.2 Why splitting again is not the 0.5.0 churn backwards

0.5.0 deprecated five verbs (dbbaf9c) because each was a different *mix* of
jobs and tabs for a different *scenario* (`restore`: jobs and tabs after an
event; `up`: jobs, first launches and tabs; `layout herdr`: tabs only;
`open`: one tab; `relaunch`: one job). Their scopes overlapped, so the user
had to know which scenario they were in. `start` fixed that by making the
scenario the planner's job. That stays. This proposal splits by *object*
instead: one family changes Claude Code jobs, the other changes Herdr panes.
The axes do not overlap, so no task needs knowledge of which verb "covers"
it; and the planner still tells update, closure and reboot apart.

### 3.3 Proposed verb set

Session verbs (jobs; members may run them by the authority table, patterns
allowed):

| Verb | Does | Panes |
| --- | --- | --- |
| `start <targets> [-f] [--all] [--launch] [--fresh]` | wake, relaunch older workers, `-f` relaunch all, first launch of named members | keeps continuity only: a pane that showed the member before a relaunch is reattached (#4); never makes a tab |
| `stop <targets>` | stop | leaves panes alone (what a pane shows after its job stops is not verified here) |
| `rm`, `sweep`, `adopt` | rows and stowaways | none |
| `signal <id> <text>` | types into the tab a lead has, else posts to its inbox | never opens a tab (#6); with no tab it uses the inbox, which it already can |

Pane verbs (Herdr; yours alone, refused for every member, patterns allowed):

| Verb | Does |
| --- | --- |
| `show [<targets>] [--tab]` | today's `-p` placement: focus its pane, else its own tab, else a split beside its commodore, else a new tab; `--tab` always its own tab. Several matches each get their own tab. No target: the layout, every live lead's tab and the active set's panes (today's leads-tab pass of `start`, and what Herdr closure needs). A parked member's attach wakes it, as `claude attach` itself does (restore.rs:187); a stopped or never-started one is refused with `nimitz start <id>` as the fix |
| `close <targets>` | detach and close the panes showing those members; their jobs keep running (optional, decision 5) |
| `attach <id>` | unchanged: this terminal. A member that is down is refused with the `start` fix instead of woken (#5) |

`start --show` is the one bridge, yours alone: run the session plan, then
`show` the same targets (the reboot one-liner). It is a composition the user
asks for by name, not a default, so a member's `start` never touches panes
beyond continuity (#7).

Names: `show`, not `open`. `open` still prints "deprecated: use `start
-p`" in 0.7.1, and the rules and skills of the last two releases tell
agents so; a name that flips meaning back would contradict text that agents
still read in old transcripts and KB notes. The skill already calls the task
"Show a member" (plugin/skills/fleet/SKILL.md:112). Admiralty's `Act::Open`
maps to `show` (actions.rs:468).

### 3.4 Before and after

| Task | Today | Proposed |
| --- | --- | --- |
| Bring the fleet back after a reboot | `nimitz start` | `nimitz start --show` (or `start`, then `show`) |
| Bring one fleet up | `nimitz start HOME` | `nimitz start HOME/...` |
| Open a lead (attach its tab) | `nimitz start HOME --only -p` | `nimitz show HOME` |
| Relaunch the admiral alone | `nimitz start HOME --only -f` | `nimitz start HOME -f` |
| A ship in its own tab | `nimitz start HOME/GITHUB/MYCC_DEV -p --tab` | `nimitz show HOME/GITHUB/MYCC_DEV --tab` |
| Relaunch after a Claude Code update | `nimitz start` (older workers only; `-f` all) | unchanged |
| Wake one ship | `nimitz start HOME/GITHUB/MYCC_DEV` | unchanged |
| Bring up the commodores alone | one `start F/SQ --only` each | `nimitz start 'HOME/*'` (or name them) |
| First launch of a new squadron's ships | one `start F/SQ/SHIP` each | `nimitz start MAIN/ARGOS/... --launch` |
| Stop a squadron | one `nimitz stop` per member | `nimitz stop HOME/GITHUB/...` |
| See what is up | `nimitz status` (all fleets) | `nimitz status HOME/GITHUB/...` |
| Reattach after a Herdr closure | `nimitz start [F]` (plans jobs too) | `nimitz show` or `nimitz show HOME/...` (jobs untouched) |
| Answer a member that waits on you | `nimitz start <id> -p` | `nimitz show <id>` |

### 3.5 Migration

The same release N as the bare-path change (2.6), so users relearn once.
Both forms work for that release, each old one printing
`members::deprecated`'s pointer (members.rs:257-264): `start -p` → `show`,
`start <lead> --only -p` → `show <lead>`, `start -p --tab` → `show --tab`, `start --jobs-only` →
`start` (with a note that it is now the default). The plan's "then:" lines
and every fix that says `start <id> -p` change to `show <id>` (restore.rs
`needs-you` fix at 1020, README.md:60, 172). `start` without a target keeps
making the leads' tabs for that release, with a pointer to `start --show`,
then stops. The five 0.5.0 deprecations go in the same release: they have
outlived their stated one release four times.

### 3.6 Text that has to change

- crates/nimitz-core/rules/common.md:11-13 (attach a lead with `start -p`)
  and 24-30 (the one-verb paragraph: `start` for jobs, `show` for panes,
  a path is one member, `F/...` everything under); admiral.md:5-11 and commodore.md:5-9, 21 (`-p` is the user's
  becomes "the pane verbs are the user's"; the `start.self` fix without
  `--only`; commodore.md:6 "`start {FLEET}/{SQ}` covers your ships" becomes
  `{FLEET}/{SQ}/...`).
  ship.md needs nothing. Bump `rules::VERSION` (rules.rs:25, now 8).
- plugin/skills/fleet/SKILL.md:23-28, 54-69 (authority table: a row for
  pane verbs), 40-42 ("`start F` names the admiral" now true of every
  verb), 105-114 (task table, old names, every `--only`);
  admiral/SKILL.md:27-31, 51-55; commodore/SKILL.md:23, 31, 40, 52.
- `sessions::may_run_in` (sessions.rs:497-511): `show`/`close` never a
  member's; patterns expanded before the authority check. The authority
  test runs every command as every role (main.rs:220-223), so `show` and
  `close` need their plan-v4 section 12.1 rows.
- USAGE (main.rs:63-135), README.md:48-60 and the reference section around
  295-346, Admiralty's actions.rs:460-468 and its key help.

## 4. Recommendation

Adopt Perforce paths: a path is one member in every verb; `F/...` and
`F/SQ/...` are everything under, the lead included, with today's wide-start
rules; `*` one level as a quoted whole segment; several targets per
command; refusal on authority; no first launch through a pattern without
`--launch`. The bare path keeps its old action with a warning for one
release. Split panes out of `start` into `show` (and
optionally `close`), with `start --show` as the user's one-liner; `signal`
and `attach` stop doing the other family's work.

Tradeoffs: the reboot case goes from one word to a flag (`start --show`),
and a member that today repairs the user's layout as a side effect of its
`start` no longer does; the user runs `show`. In return the authority rule
becomes one sentence ("pane verbs are yours"), `-p`'s six combination
refusals disappear, `--only` disappears, and a path reads the same in
every verb. The bare-path change costs every user and agent one habit
(`start HOME` → `start HOME/...`), spread over a warning release. The
cost is real in core: multi-target planning in `plan_start`, and the tab
pass lifted out of `plan_member` into its own planner (it already exists
as the `shape_only` mode used by `layout herdr`, restore.rs:117-119).

## 5. Decisions for you

1. `HOME/...` includes the admiral (recommended, as the commodore reads
   Perforce) or only the members below?
2. One level: `*` as a quoted whole segment (recommended), or leave it out
   of the first release (`...` plus names)?
3. In release N, a bare fleet or squadron path keeps its old action with a
   warning (recommended), or switches at once with a warning?
4. Refuse a target that reaches outside the caller's authority (recommended)
   or filter it?
5. `--launch` required for first launches through a pattern?
6. `show` for the pane verb (vs reusing `open`); is `close` wanted at all?
7. `start` without `--show` never makes tabs after the migration release,
   or keep making the leads' tabs when you (not a member) run it? And a
   `signal` to many members, later or never?
8. Remove the five 0.5.0 deprecated verbs in the same release N?

Not verified: what a Herdr pane shows after its job is stopped (3.3 `stop`);
member-run `start` touching the user's panes (#7) is read from code, not
observed; no plan was run against a live Herdr for this doc.
