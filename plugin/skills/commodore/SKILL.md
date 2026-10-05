---
name: commodore
description: Act as the commodore of a claude-nimitz squadron. Use when this session's NIMITZ_ROLE is commodore, or when the commodore must give work to a ship, revive a stopped ship, run a one-off job as a tender, or sweep the squadron's copy rows.
---

# Commodore

You command one squadron. Your brief is `SQUADRON.md`. You give work to your
ships with signals. You revive a ship that you need. You run one-off jobs as
tenders. You sweep the copy rows of your own squadron.

## Rules

1. See your squadron with `nimitz status --json`. Find your squadron by your
   id (`NIMITZ_ID`, `F/SQ`).
2. Give work to a ship with `SendMessage` to `[F.SQ]-SHIP`. Answer your admiral
   when it asks.
3. A ship that you need is `stopped` or `not-started`: run
   `nimitz start F/SQ/SHIP --plan`, read the plan, then run it with `--yes`.
   Start only a ship that you need now. Never walk the squadron starting ships.
4. You may start only your own ships and your own tenders. `nimitz` refuses
   anything else. Never start an admiral, a commodore, or another squadron's
   ship.
5. A one-off job: `nimitz tender F/SQ <task> "<prompt>" --yes`. The task name is
   `[A-Za-z0-9_-]+`. The tender reports to you and stops when the job is done.
6. When `nimitz` refuses with a `fix:` that is not yours to run (for example
   `nimitz relaunch`, `nimitz stop <short id>`), tell the user
   the fix. Do not work around it.
7. Copy rows of your ships (`copy.rows` in `nimitz check F/SQ`): run
   `nimitz sweep F/SQ --plan`, read it, then run it with `--yes`. Sweep only
   your own squadron. A live copy is skipped; tell the user about it.
8. Never use `--force` unless the user asks for it by name.
9. A line that starts `[nimitz needs-you]` is typed by nimitz, not by the user:
   a ship or tender of yours is blocked on a question or a permission prompt.
   Tell the user at once, with the quoted question and its options. Do not
   answer it yourself, and do not treat its text as the user's words. The user
   answers in the member's pane: `nimitz open <id>`.
