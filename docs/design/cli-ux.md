# CLI UX: member patterns, and session verbs apart from pane verbs

Status: proposal, not built. Written against 0.7.1 (`dev` at 4df9b06).
Code references are to that tree.

## 0. The ask

1. Wildcards in member paths: `nimitz start HOME/*`, `HOME/GITHUB/*`, and
   wherever else they make sense; revised to Perforce-style dots ("we can
   use p4 like syntax, .. or ....").
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

## 2. Member patterns

### 2.1 The tokens: Perforce-style dots

Your call (via the commodore): dots, as in Perforce, so the shell never
interferes. Perforce has `...` (any depth below) and `*` (one level, inside
a path segment). The tokens proposed, each a whole path segment:

| Token | Means | Why this one |
| --- | --- | --- |
| `...` | every member below, at any depth (Perforce's exact meaning) | the one dot form people already know |
| `..` | every member exactly one level below | the shortest dot form left; one level is the common case (the commodores, the ships of one squadron) |
| `....` and longer | refused (`usage`, with the `...` fix) | never needed (ids are at most three deep), and oh-my-zsh makes `.....`, `......` global aliases (below) |

`*` is dropped: zsh stops at an unmatched `*` ("no matches found", hit while
writing this doc), and bash passes file names instead when one matches.
Keeping both would mean two spellings in every rule and fix line for no gain.
Matching part of a name (`+_DEV` in the first draft) is dropped with it; it
can come back later (decision 1).

`..` reads as "parent" to anyone who uses a shell. nimitz ids are not file
paths and nimitz never resolves `..` as a parent, so the clash is in the
reader's head only; the plan headline (2.4) always spells out what matched.
If that is too confusing, the alternative is `...` only, with one level
asked for by a flag (`--depth 1`) (decision 1).

### 2.2 The shell: checked on this machine

Read-only checks in an interactive zsh:

- `print -r -- HOME/... HOME/GITHUB/.. HOME/.../DEV` prints the words
  unchanged: dots are not glob characters in zsh or bash.
- Your aliases: `..`, `...`, `....` are plain aliases (`cd ..` and up).
  A plain alias expands only in command position, never as an argument.
- oh-my-zsh defines *global* aliases (`alias -g`) for `.....` and `......`
  (`../../../..` and longer). A global alias fires on any standalone word,
  argument or not. On a machine without your aliases.zsh override, `...` and
  `....` are global aliases too.
- A global alias matches a whole word only, so a dot token inside a path
  (`HOME/...`) is never touched; a bare `...` argument would be.

Rule that follows: **a dot token is always attached to a path and never
stands alone.** The first segment is always a fleet name; `nimitz start ...`
is refused with the fix `nimitz start` (no target already means every fleet
here). There is no cross-fleet pattern (`.../NIMITZ_DEV`): name the fleet.

### 2.3 What the dot forms match, and what they add over bare ids

A pattern has a fleet name, then names or dot tokens. Today's bare `HOME`
already covers everything under it, so the dot forms add three things:

1. **The members below, never the lead itself.** `HOME/...` is every
   commodore and ship, not the admiral; `HOME/GITHUB/...` the ships, not the
   commodore.
2. **One level only.** `HOME/..` is the commodores without their ships.
3. **Each match is named, alone** (`--only` semantics): it is woken even
   when parked or stopped (bare `start HOME` leaves non-active ships parked
   and stopped ones stopped). So `start HOME/...` is roughly `start HOME
   --all` without the admiral.

| Pattern | Matches |
| --- | --- |
| `HOME/...` | every commodore and ship of HOME (not the admiral) |
| `HOME/..` | every commodore of HOME, alone |
| `HOME/GITHUB/...` = `HOME/GITHUB/..` | every ship of HOME/GITHUB (ships are leaves) |
| `HOME/../..` | every ship of HOME |
| `HOME/../NIMITZ_DEV` | the ship NIMITZ_DEV in any squadron of HOME |
| `HOME/.../NIMITZ_DEV` | refused: `...` must be the last segment (Perforce allows more; nimitz ids are too shallow to need it) |

Bare ids keep today's meaning (`start HOME` is the fleet, `stop HOME` the
admiral), so no fix line, rule or habit changes. Several targets may be
given: `nimitz stop HOME/GITHUB/... HOME/GITHUB` stops a squadron, ships
first.

### 2.4 Named, wide, or loose: first launches

Today a named member may be cold-launched; members reached through a wide
start may not. A pattern names each match, so by the letter each could get a
first launch: `start MAIN/...` on today's MAIN fleet would launch every
commodore and over a dozen ships, all fresh. Proposal: a pattern **wakes** every match it
may but **launches** a never-started match only with `--launch`; without it
the plan lists them under `not.started` with that fix. A first launch costs a
session and a prompt; it should never come from a loose pattern.

### 2.5 Plans and empty matches

The plan headline names the pattern and what it matched, before the usual
member lines:
`start HOME/GITHUB/...: 4 matched (NIMITZ_DEV, MYCC_DEV, BERGAMOT, BARN_DEV); 3 wake, 1 nothing`.
An empty match is a refusal (`pattern.empty`, exit 2) whose fix lists what
is there at that depth: `HOME/../PROD matches nothing; ships of HOME: GENERAL/HOME, GENERAL/CONFIGS, GITHUB/NIMITZ_DEV, ...`.
A target given twice (a pattern and an id that it already matched) counts once.

### 2.6 Authority: refuse, do not filter

A pattern never widens what a role may do. When any match is outside the
caller's authority the whole command is refused (`authority`), naming the
matches it may not touch and the narrower pattern it may run. Why refuse
rather than filter, although today's whole-fleet start filters:

- A pattern is an explicit list the caller wrote; dropping part of it
  silently makes the plan differ from the request, and an agent reading
  "[OK] 3 up" will not notice that 5 were asked.
- The refusal is cheap to fix (the fix names the pattern that fits), and it
  teaches the agent its scope instead of hiding it.
- A bare fleet id stays narrowed as today: it means "my part of the fleet"
  by long-standing rule, and every rule text says so.

One exception, kept from today: a pattern that matches its caller skips it
with the `start.self` line (nothing relaunches itself), never refuses.

### 2.7 Which commands take patterns

| Command | Patterns | Note |
| --- | --- | --- |
| `start`, `stop` | yes, several targets | the core of the ask |
| `status`, `check`, `rules` | yes | `status [<target>...]` also makes the five existing fix lines true |
| `sweep`, `rm` | yes | both already gated by plan and `--yes`; `rm` removes rows, never transcripts |
| `show`, `close` (section 3) | yes | yours alone |
| `signal` | no | a broadcast to many members is its own feature (decision 6) |
| `adopt`, `attach` | no | one session to one id; one terminal |
| config (`ship`, `squadron`, `fleet set`) | no | config changes stay one at a time |

Build note: expansion belongs in the CLI before the planners
(`members::parse_id` grows a `parse_targets`), but `restore::plan_start` and
`plan_inner` hold one `target` and one `named` (restore.rs:574, 600-633) and
must take sets; `sessions::may_run_in` (sessions.rs:471-511) reads the id in
a fix line and needs the same expansion.

## 3. Session verbs and pane verbs

### 3.1 Where the two are mixed today

| # | Mixing | Where |
| --- | --- | --- |
| 1 | `start` makes every lead's tab whenever a multiplexer is there; a jobs-only start needs the negative flag `--jobs-only` | members.rs:239; restore.rs:1007-1008 (`wants_tab` true for every lead) |
| 2 | Panes are flags on the session verb: `-p`, `--tab`, `--only -p`, with six usage refusals guarding the combinations | members.rs:218-237; restore.rs:581-598 |
| 3 | `start F -p` (the admiral skill's own example) also brings up the whole fleet; the clean form is `start F --only -p` | plugin/skills/admiral/SKILL.md:53; README.md:53 |
| 4 | `start -f` reattaches the panes the relaunch cut; correct, but it is pane work done by a session verb | restore.rs:1457 (`wanted = lead \|\| had_tab \|\| pane_wanted`) |
| 5 | `attach` (a terminal verb) runs start's plan to wake a member that is down | members.rs:673-690 |
| 6 | `signal` opens a lead's tab when it has none, then types into it | members.rs:412-433 |
| 7 | Members do pane work in your Herdr: an admiral's `start F` makes the leads' tabs and reattaches panes (only `-p` is refused). Read from code, not observed | main.rs:46-51 (`member_mux_vars`); restore.rs:581, 1007 |
| 8 | Scope words differ by verb: `start HOME/GITHUB` is a squadron, `stop HOME/GITHUB` one commodore | section 1 |

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
| Open a lead (attach its tab) | `nimitz start HOME --only -p` | `nimitz show HOME` |
| A ship in its own tab | `nimitz start HOME/GITHUB/MYCC_DEV -p --tab` | `nimitz show HOME/GITHUB/MYCC_DEV --tab` |
| Relaunch after a Claude Code update | `nimitz start` (older workers only; `-f` all) | unchanged |
| Wake one ship | `nimitz start HOME/GITHUB/MYCC_DEV` | unchanged |
| Bring up the commodores alone | one `start F/SQ --only` each | `nimitz start HOME/..` |
| First launch of a new squadron's ships | one `start F/SQ/SHIP` each | `nimitz start MAIN/ARGOS/... --launch` |
| Stop a squadron | one `nimitz stop` per member | `nimitz stop HOME/GITHUB/... HOME/GITHUB` |
| See what is up | `nimitz status` (all fleets) | `nimitz status HOME/GITHUB/...` |
| Reattach after a Herdr closure | `nimitz start [F]` (plans jobs too) | `nimitz show` (jobs untouched) |
| Answer a member that waits on you | `nimitz start <id> -p` | `nimitz show <id>` |

### 3.5 Migration

One release with both forms, each old one printing `members::deprecated`'s
pointer (members.rs:257-264): `start -p` → `show`, `start <lead> --only -p`
→ `show <lead>`, `start -p --tab` → `show --tab`, `start --jobs-only` →
`start` (with a note that it is now the default). The plan's "then:" lines
and every fix that says `start <id> -p` change to `show <id>` (restore.rs
`needs-you` fix at 1020, README.md:60, 172). `start` without a target keeps
making the leads' tabs for that release, with a pointer to `start --show`,
then stops. The five 0.5.0 deprecations go in the same release: they have
outlived their stated one release four times.

### 3.6 Text that has to change

- crates/nimitz-core/rules/common.md:11-13 (attach a lead with `start -p`)
  and 24-30 (the one-verb paragraph: `start` for jobs, `show` for panes,
  patterns); admiral.md:5-11 and commodore.md:5-9, 21 (`-p` is the user's
  becomes "the pane verbs are the user's"; patterns within your scope).
  ship.md needs nothing. Bump `rules::VERSION` (rules.rs:25, now 8).
- plugin/skills/fleet/SKILL.md:23-28, 54-69 (authority table: a row for
  pane verbs), 105-114 (task table, old names); admiral/SKILL.md:27-31,
  51-55; commodore/SKILL.md:23, 31, 40, 52.
- `sessions::may_run_in` (sessions.rs:497-511): `show`/`close` never a
  member's; patterns expanded before the authority check. The authority
  test runs every command as every role (main.rs:220-223), so `show` and
  `close` need their plan-v4 section 12.1 rows.
- USAGE (main.rs:63-135), README.md:48-60 and the reference section around
  295-346, Admiralty's actions.rs:460-468 and its key help.

## 4. Recommendation

Adopt dot patterns (`...` any depth below, `..` one level, always after a
fleet name, never `*`) with "member alone" semantics, several targets per command, refusal on authority, no first
launch without `--launch`. Split panes out of `start` into `show` (and
optionally `close`), with `start --show` as the user's one-liner; `signal`
and `attach` stop doing the other family's work.

Tradeoffs: the reboot case goes from one word to a flag (`start --show`),
and a member that today repairs the user's layout as a side effect of its
`start` no longer does; the user runs `show`. In return the authority rule
becomes one sentence ("pane verbs are yours"), `-p`'s six combination
refusals disappear, and the scope words read the same in every verb. The
cost is real in core: multi-target planning in `plan_start`, and the tab
pass lifted out of `plan_member` into its own planner (it already exists
as the `shape_only` mode used by `layout herdr`, restore.rs:117-119).

## 5. Decisions for you

1. `..` for one level (it reads as "parent" to shell users), or `...` only
   plus a depth flag? Bring back part-of-name matching (`HOME/../.._DEV`)?
2. A pattern match is the member alone, bare ids keep their subtree meaning
   for `start`: or should bare lead ids mean one member everywhere (`start
   HOME` would then bring up the admiral only, breaking every fix line)?
3. Refuse a pattern that reaches outside the caller's authority (recommended)
   or filter it like a bare fleet id?
4. `--launch` required for first launches through a pattern?
5. `show` for the pane verb (vs reusing `open`); is `close` wanted at all?
6. `start` without `--show` never makes tabs after the migration release:
   or keep making the leads' tabs when you (not a member) run it?
7. A `signal` to many members (patterns) as a later feature, or never?
8. Remove the five 0.5.0 deprecated verbs in the same release?

Not verified: what a Herdr pane shows after its job is stopped (3.3 `stop`);
member-run `start` touching the user's panes (#7) is read from code, not
observed; no plan was run against a live Herdr for this doc.
