# Migrating a live fleet from nimitz 0.3.x to 0.4.0

For the user, or an agent that guides the user through it. Read it all
before the first step. Steps marked **(user)** need a person at the keyboard;
an agent never does them itself.

## What changes

| Area | 0.3.x | 0.4.0 |
| --- | --- | --- |
| Identity | keyed by session id | keyed by the Claude Code **job**; the session id is history |
| Leads (admiral, commodore) | interactive sessions in a Herdr pane | background jobs, pinned, attached in their Herdr tab |
| Ships | background jobs | unchanged (must be background) |
| Tenders | a role | removed: one-off work is a subagent or a ship |
| Rules | at session start only | also at the next turn after a nimitz update (`nimitz rules updated vN→vM`) |
| Authority | ships ran nothing | every member runs `nimitz` within its role (`nimitz rules`) |
| Recovery | `nimitz up`, `layout herdr` | one command, `nimitz restore [<F>]`, after a Claude Code update, a Herdr closure or a reboot; `up` and `layout herdr` are deprecated aliases for one release |
| Detach a lead | close the pane | **ctrl-z**; `/exit` inside an attach opens the agent view |

Ships need no migration. Every 0.3.x lead does, once.

## 0. Before you start

1. Install 0.4.0 (skip what is already done):
   - `cd ~/Github/claude-nimitz && git checkout dev && git pull`
   - `cargo install --path crates/nimitz --root ~/.local --force`
   - `cargo install --path crates/admiralty --root ~/.local --force`
   - `claude plugin update nimitz@claude-nimitz`
2. `nimitz --version` prints `nimitz 0.4.0`, and `nimitz doctor` ends with
   `[OK] doctor: ready`.
3. **(user)** Turn off Herdr's agent resume. In `~/.config/herdr/config.toml`:

   ```toml
   [session]
   resume_agents_on_restore = false
   ```

   With it on, every Herdr restart reopens each lead's conversation
   interactively in its pane (`tab.resumed`), and the lead cannot come back
   as a job until someone `/exit`s it there. `nimitz doctor` warns while it
   is on. nimitz never edits this file.
4. Run every `nimitz` command below from a **plain shell**: a terminal that is
   not inside a lead's pane, with no `NIMITZ_*` variables set
   (`env | grep NIMITZ_` prints nothing). 0.4.0 scrubs the env of every
   `claude` it starts, but a plain shell keeps the caller unambiguous: it is
   the user, who may migrate every lead.

## 1. See what needs migrating

    nimitz status

Each 0.3.x lead shows a `[WARN] … runs as a 0.3.x interactive lead (pid …,
no job)`. Then, per fleet:

    nimitz restore <F> --plan

For each 0.3.x lead the plan prints `[ERROR] <id>: <id>'s conversation is open
in pane <pane> … (tab.resumed)` with the fix `type /exit in pane <pane>, then
nimitz restore <F>`. Write down the pane of each lead. Ships show as live or
`parked`; members that never started show as `not.started` and are left out
(that is correct).

## 2. Migrate one fleet

Do one fleet at a time. Do the fleet that holds the session guiding you
**last**, and in that fleet, its own lead last: the `/exit` stops it until
the restore brings it back.

1. **(user)** In each lead's pane from step 1, type `/exit`. Its commodores
   first, then its admiral. The conversation is kept on disk; nothing is lost.
2. `nimitz restore <F> --plan`. Each lead now shows a wake by its exact
   session id, as a pinned background job, plus an attach in its tab
   `F.ADMIRAL` / `F.SQ.COMMODORE` (an old short-named tab is reused and
   renamed). No `[ERROR]` remains for the leads.
3. `nimitz restore <F> --yes`. It ends with the members it woke and verified.
4. `nimitz status`: no `0.3.x interactive lead` warning in this fleet; each
   lead is `idle` or `busy` with a job.

If one lead is still left out, `nimitz start <id> --plan`, then `--yes`.

## 3. Clean up

1. Old tenders: `nimitz check` lists each as `tender.retired`;
   `nimitz rm <short id>` each one.
2. Old fleet-skill sessions (`legacy` stowaways, e.g. `[ROVER]-SILGAP`):
   `nimitz adopt <session> <F/SQ/SHIP>` to make one a ship's history, or
   `nimitz rm <short id>`. Leave them if unsure; they do no harm.
3. **(user, optional)** Restart Claude Code's daemon from a plain shell once no
   0.3.x lead is left. A daemon first started by a `claude --bg` inside a
   labelled 0.3.x lead keeps that lead's `NIMITZ_*` and `HERDR_PANE_ID` in its
   env; check with
   `tr '\0' '\n' < /proc/$(pgrep -f 'claude daemon run' | head -1)/environ | grep -E 'NIMITZ_|HERDR_PANE'`.
   0.4.0 ignores inherited labels, so nothing breaks, but other tools may
   still see them. Find out first what a restart does to running jobs on
   your Claude Code version.

## 4. After the migration

- Open a lead: `nimitz open <id>`, or its Herdr tab. **Detach with ctrl-z.**
  Never `/exit` an attached member: that opens the agent view, and the agent
  rules forbid asking the user to do it.
- After a Claude Code update, a Herdr closure or crash, or a reboot: `nimitz
  restore [<F>] --plan`, then `--yes` (no `F`: every fleet here). The plan
  says per member what it found and what it does: workers on an older Claude
  Code are relaunched (same id and history) and reattached, panes that are
  gone are reattached, members that are down are woken. Leads always come
  back; parked ships wake on demand, or all at once with `--all`. `--restart`
  relaunches every live member. A restore never relaunches the member that
  runs it; it names the `nimitz restore <F>` you run afterwards from your own
  shell (that one relaunches it and reattaches its pane).
- Never start, resume or stop a member with `claude` yourself: a resume of a
  live job starts a copy; an interactive `claude --resume` holds the
  conversation with no job (nimitz cannot wake it until `/exit` there); a
  session without nimitz's labels is a stowaway; a job started from your
  shell takes the daemon's environment, not the member's labels.
- When nimitz updates, every live member gets the new rules at its next turn.
  Run `/reload-plugins` in a session only when `nimitz doctor` says the plugin
  needs it.
- Real-Herdr check of restore, a few minutes: `docs/test/restore-checklist.md`.

## Rollback

Nothing in this migration deletes a conversation. To go back:

1. `cd ~/Github/claude-nimitz && git checkout <0.3.1 commit>` and run the two
   `cargo install` lines again.
2. The 0.4.0 registry migration kept the old file as
   `~/.local/state/nimitz/registry.json.v1`; copy it back over
   `registry.json`.
3. Leads that are now background jobs keep their conversation. How 0.3.x
   shows a lead that is a job is not tested; to be safe, stop the job
   (`claude stop <short id>`) and reopen the lead with 0.3.x's
   `nimitz start <id>`, which resumes it by `-c` in its folder.

## For an agent guiding this

- Do not type `/exit` into a pane, and do not run `claude stop`, `claude rm`
  or `claude --bg` for a member. Ask the user for every **(user)** step and
  wait for their word.
- Run each change with `--plan` first and show the plan; `--yes` only after
  the user agrees. Never `--force` unless the user names it.
- A lead that is being migrated cannot migrate itself: its own `/exit` ends
  it. Hand the last steps to the user.
- On any `[ERROR]` with a `fix:` you may not run, pass the fix to the user
  word for word.
