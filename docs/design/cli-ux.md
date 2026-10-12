# CLI UX: member patterns, and session verbs apart from pane verbs

Status: proposal, not built. Written against 0.7.1 (`dev` at 4df9b06).
Code references are to that tree.

## 0. The ask

1. Wildcards in member paths: `nimitz start HOME/*`, `HOME/GITHUB/*`, and
   wherever else they make sense.
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

### 2.1 The wildcard character: `+`, and quoted `*`

zsh stops at an unmatched `*` ("no matches found", seen while writing this
doc); bash passes it literally unless a file matches, in which case it
silently passes file names. Options weighed:

| Syntax | No quotes in zsh and bash | Verdict |
| --- | --- | --- |
| `*` | no | accepted when quoted, never documented as the main form |
| `%` | yes, but `%1` is a job spec at the start of some shells' words, and reads as "percent" | no |
| `@` | taken: aides are `F/@NAME` | no |
| `HOME/GITHUB/` (trailing slash) | yes | one meaning only ("the children"), cannot do `+_DEV` or `HOME/+/DEV` |
| `--ships` / `--leads` flags | yes | only two shapes; grows a flag per need |
| **`+`** | yes: no glob meaning in zsh (even with `extendedglob`) or bash (extglob needs `+(…)`) | **recommended** |

`+` is not a name character, so a pattern can never be mistaken for an id.

### 2.2 What `+` matches

A pattern has one to three segments, like an id. Each segment is a name, or
a name with `+` standing for any run of name characters (never across `/`).
A pattern matches **members at exactly its depth**; it is never a subtree.

| Pattern | Matches |
| --- | --- |
| `HOME/+` | every commodore of HOME (the commodores alone, not their ships) |
| `HOME/GITHUB/+` | every ship of HOME/GITHUB |
| `HOME/+/+` | every ship of HOME |
| `HOME/+/+_DEV` | ships of HOME whose name ends in `_DEV` |
| `HOME/+/HOME` | the ship called HOME in any squadron of HOME |
| `+` | every admiral on this machine's fleets |
| `+/+/NIMITZ_DEV` | that ship name in any fleet |

Every matched member is the member *alone* (`--only` semantics): `start
HOME/+` brings up the commodores without their ships. The bare-id forms keep
today's meaning (`start HOME` is the fleet, `stop HOME` the admiral), so no
fix line, rule or habit changes. Several targets may be given:
`nimitz stop HOME/GITHUB/+ HOME/GITHUB` stops a squadron, ships first.

### 2.3 Named, wide, or loose: first launches

Today a named member may be cold-launched; members reached through a wide
start may not. A pattern names each match, so by the letter each could get a
first launch: `start MAIN/+/+` on today's MAIN fleet would launch 14 fresh
ships. Proposal: a pattern **wakes** every match it may (parked and stopped
ones too: naming them is the ask, like `--all` for that set) but
**launches** a never-started match only with `--launch`; without it the plan
lists them under `not.started` with that fix. A first launch costs a session
and a prompt; it should never come from a loose pattern.

### 2.4 Plans and empty matches

The plan headline names the pattern and what it matched, before the usual
member lines:
`start HOME/GITHUB/+: 4 matched (NIMITZ_DEV, MYCC_DEV, BERGAMOT, BARN_DEV); 3 wake, 1 nothing`.
An empty match is a refusal (`pattern.empty`, exit 2) whose fix lists what
is there at that depth: `HOME/+/+_PROD matches nothing; ships of HOME: GENERAL/HOME, GENERAL/CONFIGS, GITHUB/NIMITZ_DEV, ...`.
A target given twice (a pattern and an id that it already matched) counts once.

### 2.5 Authority: refuse, do not filter

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

### 2.6 Which commands take patterns

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
| Bring up the commodores alone | one `start F/SQ --only` each | `nimitz start HOME/+` |
| First launch of a new squadron's ships | one `start F/SQ/SHIP` each | `nimitz start MAIN/ARGOS/+ --launch` |
| Stop a squadron | one `nimitz stop` per member | `nimitz stop HOME/GITHUB/+ HOME/GITHUB` |
| See what is up | `nimitz status` (all fleets) | `nimitz status HOME/GITHUB/+` |
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

Adopt `+` patterns (quoted `*` too) at exact depth with "member alone"
semantics, several targets per command, refusal on authority, no first
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

1. `+` as the unquoted wildcard (vs a trailing slash or flags)?
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
