# Rules size: the 10,000-character hook limit, and shorter role rules

Status: proposal (0.8), not built. Claude Code 2.1.296.

Today a member's rules are one hook's `additionalContext`: its role's rules,
then its fleet's harness rules. `CONTEXT_MAX` (9,800, rules.rs) keeps the
whole under Claude Code's 10,000-character limit, and `RULES_MAX` (4,000,
harness.rs) holds the harness's share. The user asks to raise or remove the
harness cap, and to shorten nimitz's own role rules.

## 1. What Claude Code does with a hook's context

Read from the 2.1.296 binary, then checked with a probe run (below).

- **The limit is per hook, not per event.** Each hook command's
  `additionalContext` goes through one function (`dce`, threshold
  `vAs = 1e4`). Plain stdout and `systemMessage` go through it too. The
  10,000 is a constant: no setting or environment variable changes it.
- **Above the limit, the text is written to disk and the model gets a
  2,000-character preview.** The whole text goes to the session's
  `tool-results/` folder. In its place the model sees `<persisted-output>
  Output too large (11.7KB). Full output saved to: <path> Preview (first 2KB):
  ...</persisted-output>`. The rest reaches the model only if it reads the
  file. If the write fails, the text is cut at 10,000 with a marker.
- **Several hook entries on one event each get their own 10,000.** A command
  hook's entries are not added up. One SessionStart had four hooks: three of
  9,500 characters and one of 12,000. The three came in whole (28,500
  characters), and the 12,000 one as a preview. Two UserPromptSubmit hooks of
  9,500 each both came in whole. The model quoted the end marker of every
  whole entry. (The newer managed-hook path, `urt`, has its own budget of
  100,000 per entry and 200,000 per dispatch. nimitz does not use it.)
- **Entries arrive in completion order, not the order in hooks.json.** The
  hooks run in parallel, and the probe's SessionStart entries arrived as B,
  A, BIG, C. A text split across hooks must say which part each piece is.
- Probe: four SessionStart and two UserPromptSubmit command hooks in a
  `--settings` file, each printing a JSON `additionalContext` of N
  characters ending in a codeword. The run was `claude -p --model haiku`,
  which listed every codeword it could see. The session's transcript
  attachments confirmed the sizes and the order.

## 2. The routes, against what the rules need

The rules need two properties: (a) they are in context on every turn without
the model choosing to read them, and (b) a live member gets a change at its
next prompt.

| Route | (a) | (b) | Ceiling | Notes |
| --- | --- | --- | --- | --- |
| One hook (today) | yes | yes | 10,000 in all | role and harness share it |
| **Several hook entries** | yes | yes | 10,000 per entry | entries arrive unordered; one more process per entry per event |
| CLAUDE.md `@`-import | yes | no | none | read when the session starts. Ships get no nimitz-written CLAUDE.md (their folder is the user's repo), so this reaches leads only |
| `--append-system-prompt-file` | yes | no | none | read at launch. A change needs `nimitz start -f`, which drops a turn in flight. Not checked: whether a job resumed by the daemon keeps the flag |
| Fleet output style | yes | no | none | one per session, so it collides with the user's own `outputStyle`. Unless it sets `keep-coding-instructions`, it replaces Claude Code's coding instructions |
| A pointer to a file | no | yes | none | the model may not read the file. Fits reference material, not rules |
| A harness skill (exists, P4) | no | yes | none | loaded on demand. The right home for long procedures |

Only hooks hold both properties. Removing the cap with one hook gives (a) to
the first 2,000 characters only, so it fails the moment the text passes
10,000.

## 3. The cost

- **Tokens.** The rules sit in the conversation once per delivery: at session
  start, after each `/clear` and `/compact` (SessionStart fires again), and
  once per update. An old copy stays until the next compaction. Every request
  sends them, mostly as cached input after the first turn. At about 4
  characters a token, today's ~6,000 characters are ~1,500 tokens a member.
  A 30,000-character harness would be ~7,500, under 4% of a 200,000-token
  window. That is small in money and window.
- **Adherence costs more.** Each added rule competes with the others and with
  the task. Long instruction sets are followed less reliably. That is widely
  reported; it is not measured here. A harness rule should earn its place,
  and procedures belong in harness skills.
- **Updates.** Today any change, to the role rules or the harness, sends the
  whole text again. The update's "What changed" list from v0 is 3,400
  characters, most of it about aides, which no longer exist.

## 4. Recommendation: raise the cap, don't remove it

1. **Deliver the rules over several hook entries.** Each entry is its own
   text with its own delivery record.
   - Add entries to SessionStart and UserPromptSubmit in hooks.json:
     `nimitz hook rules role`, `nimitz hook rules harness-1`, `harness-2` and
     `harness-3`.
   - `role` carries the role's rules, as today minus the harness.
   - `harness-N` carries the fleet's harness rules, split at `###` section
     boundaries into parts of at most 9,800 characters. Each part is headed
     "Fleet HOME's own rules, part N of M", because parts arrive in any order.
   - A part with nothing to say exits at once with no output.
   - The SessionStart hook keeps binding the session and stops carrying rules.
   - On UserPromptSubmit, each part checks its own record. A harness edit then
     resends only the harness, and a role-rules change only the role rules.
   - `rules-delivered.json` keeps a hash per part. An old record holds one
     `harness` hash; it counts as no parts delivered, which costs one resend
     after the upgrade.
2. **Set `RULES_MAX` to 29,000** (three parts). `harness add` and `harness
   push` refuse anything over it, as they do today for 4,000.
   - **Add a soft warning at 6,000:** `harness list` and `add` print what
     every member pays each turn.
   - A fourth part costs nothing to add later. A cap that is high but real
     keeps one runaway file from taking up every member's context.
3. **Old binaries.** The 0.7.1 binary exits 0 silently on an unknown hook
   name. I checked this for `nimitz hook SessionStartHarness`. That makes the
   new entries harmless while the plugin, which runs live from the main
   checkout, is ahead of the installed binary. To check before merging: does
   0.7.1 log an `events.jsonl` line for an unknown hook name? 0.6.0 did for
   kb-guard.
4. **Trim the update's changelog.** Show at most the last three entries after
   the member's version, then "(older changes: superseded)". A member on v8
   then gets one line.
5. **Not chosen:**
   - removing the cap (anything past 10,000 becomes a preview);
   - `--append-system-prompt-file`, an output style and CLAUDE.md imports,
     which fail (b), and the CLAUDE.md route does not reach ships at all.

## 5. Shorter role rules (v9)

The draft is in `docs/design/rules-v9-draft/`: `common.md`, `admiral.md`,
`commodore.md` and `ship.md`, plus `rules.diff` against today's
`crates/nimitz-core/rules/`. Sizes are counted as rendered for HOME, without
the harness:

| Role | Now | Draft | Cut |
| --- | ---: | ---: | ---: |
| admiral | 4,479 | 3,241 | 27% |
| commodore | 4,603 | 3,491 | 24% |
| ship | 4,381 | 3,244 | 25% |

With HOME's harness (1,567) added, members get about 6,000 today and about
4,800 with the draft.

**Every behaviour stays. Dropped outright:**

- **The deprecated verbs** (`restore`, `relaunch`, `open`, `up`, `layout
  herdr`). That is history. Each verb prints its replacement when run, and
  all are gone next release.
- **The four reasons for never using `claude`** (a copy, a held
  conversation, a stowaway, the daemon's environment). One clause is
  enough: "it makes copies and stowaways nimitz cannot manage".
- **The signal matrix in the common part.** Each role's section already says
  whom it may signal. The commodore's gains "You signal your admiral and your
  ships", the one fact only the matrix held.
- **Duplicates:**
  - the admiral's second "leave day-to-day work to the commodore" bullet;
  - the commodore's "a one-off job is a subagent", which the common part
    says;
  - the ship's "nimitz refuses every other change", which the common part
    says;
  - the ship's "who tells the user".
- **Implementation detail:** "wakes what is down by its exact id", and "(you
  alone, relaunched)" after the `start.self` fix.

**Reworded with the same meaning.** The commodore's "take orders from your
admiral as you would the user's: plan first, run a change once the user has
agreed" now reads "...as you would the user's, but a change still runs only
once the user has agreed to its plan". The old wording could be read as
letting an admiral's order stand in for the user's agreement; the new wording
rules that out.

**Possible further cut, not in the draft.** The `kb-scribe` JSON shape (~350
characters) could move into the agent's own description, which Claude Code
already lists in the Agent tool every turn. The rules would keep only "keep
notes with `nimitz:kb-scribe`, ask `nimitz:kb-ref`; never touch the KB
yourself".

**When built:**
- rules v9, with the changelog line "the rules are shorter; nothing a member
  does changes";
- one update to every live member;
- tests that pin today's wording move to the new wording (rules.rs pins
  about a dozen phrases; the draft keeps all of them).
