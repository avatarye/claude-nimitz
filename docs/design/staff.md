# Staff: always-on members with a durable inbox

Status: design for the user's review (2026-10-10). Nothing is built. Branch
`design/staff`, off dev 947d360.

## 1. Summary

A **staff** member is a Claude Code background job attached to a parent
member. It is pinned, always on, and it works through a **durable FIFO inbox
that nimitz owns**. Anyone allowed by the inbox's policy posts typed items. The
staff claims them one at a time, does the work and marks each one done; a
crash, reboot, `/clear` or relaunch loses nothing.

The first one is `HOME/@KB`, the admiral's notes keeper. It turns
`wiki.note.v1` items into pages of a new git repo,
`/mnt/data/shared/nimitz-knowledge-base` (a Lodestar instance), fully
automatically, with one git commit per item.

Decisions already made by the user (from the commodore, 2026-10-10):

| Topic | Decision |
| --- | --- |
| Shape | A member attached to a parent, id `<parent>/@NAME`; admiral parents only for now, any lead or ship later |
| Liveness | Pinned, always on, woken by its parent's `start` |
| Config | `[[staff]]` |
| Model | Sonnet (checked against `availableModels`) |
| Inbox | Durable FIFO owned by nimitz; one file per item; pending -> claimed (lease) -> done, or dead after N tries; an expired lease returns an item to pending |
| Inbox commands | `nimitz inbox post <staff> --type ...`; the staff runs claim, done and fail; anyone runs list and status |
| Types | An allowlist with versioned schemas; `wiki.note.v1` only for now |
| Backpressure | A depth cap, per-sender limits, an open-inbox policy so non-member sessions (the user's mod) can post |
| Loop | A doorbell on post (the messaging socket), the queue at session start, a fallback timer; FIFO, one at a time, until empty; dead letters raise a needs-you |
| First staff | Notes keeper on the new repo (not the Obsidian vault), set up from Lodestar by MYCC_DEV; no human gate; provenance `[auto]` plus the source session; every ingest a git commit |

## 2. Identity

| | Admiral staff (phase 1) | Commodore staff (later) | Ship staff (later) |
| --- | --- | --- | --- |
| Id | `HOME/@KB` | `HOME/GITHUB/@X` | `HOME/GITHUB/NIMITZ_DEV/@X` |
| Session name | `[HOME]-@KB` | `[HOME.GITHUB]-@X` | `[HOME.GITHUB.NIMITZ_DEV]-@X` |
| Herdr tab | `HOME.@KB` | `HOME.GITHUB.@X` | `HOME.GITHUB.NIMITZ_DEV.@X` |
| Labels | `NIMITZ_ROLE=staff`, `NIMITZ_ID`, `NIMITZ_FLEET`, `NIMITZ_PARENT=HOME` | `+ NIMITZ_SQUADRON` | `+ NIMITZ_SQUADRON` |

- `Id::role()` today counts parts (1 admiral, 2 commodore, 3+ ship). A last
  part that starts with `@` makes it `Role::Staff`, whatever the count, and
  `Id::parent()` drops that part. `@` is not valid in fleet, squadron or ship
  names today (`valid_name`), so no existing id changes meaning.
- `NIMITZ_PARENT` already exists as a label (`claude::labels`), so the parent
  is carried, not derived.
- The name is upper case like ship names (`@KB`, not `@kb`).

## 3. Config

In the parent's file: `fleet.toml` for an admiral's staff (`squadron.toml`
or `[[ships.staff]]` later).

```toml
[[staff]]
name = "KB"                      # id HOME/@KB
brief = "staff/kb/BRIEF.md"      # its CLAUDE.md, written by nimitz like a lead brief
model = "sonnet"                 # checked against availableModels
permission_mode = "acceptEdits"

[staff.dirs]                     # one machine runs it (a staff never runs twice)
home-server = "/mnt/data/shared/nimitz-knowledge-base"

[staff.inbox]
types = ["wiki.note.v1"]         # the allowlist; each type is a schema nimitz ships
policy = "open"                  # open | fleet | parent
depth_cap = 500                  # pending items; a post past it is refused
per_sender = "60/h"              # a sender's posts per window
lease = "15m"                    # a claim's lifetime; renewed by `inbox heartbeat`
max_tries = 3                    # then dead-letter
keep_done = "30d"                # done items kept for audit, then pruned
```

- `staff new HOME/@KB --dir ... --model sonnet` / `staff set` / `staff rm`
  behave like `ship new|set|rm` (plan, `--yes`, `may_config`).
- The folder runs one machine's job only. A second machine with the fleet has
  no `dirs` entry for the staff, so it never starts it. That avoids two
  keepers committing to one Dropbox repo.

## 4. The inbox

### 4.1 Where it lives

`NIMITZ_STATE/inbox/<staff id, / as %2F>/` (`~/.local/state/nimitz`): on
this machine and **never in Dropbox**. Sync would turn every rename into a
conflict risk, and the queue belongs to the one machine that runs the staff.

(The existing `nimitz-core/src/inbox.rs` is the *doorbell transport*:
delivery of one line into a session's messaging socket. The queue is a new
module, `queue.rs`. The CLI word stays `inbox`, as the user chose.)

```
inbox/HOME%2F@KB/
  queue.lock          dedicated lock file (std File::lock: flock / LockFileEx)
  seq                 the next sequence number
  pending/000000000042.json
  claimed/000000000041.json
  done/000000000040.json
  dead/000000000017.json
  senders.json        per-sender counters for the rate window
```

### 4.2 An item

```json
{
  "id": "000000000042", "type": "wiki.note.v1", "key": "optional sender dedupe key",
  "posted_at": 1791700000.1,
  "sender": {"kind": "member|session|process", "id": "HOME/GITHUB/NIMITZ_DEV",
             "session": "9aacf927-...", "cwd": "/home/...", "verified": true},
  "payload": { ... the type's schema ... },
  "tries": 1,
  "claim": {"by": "<staff session id>", "at": 1791700100.0, "until": 1791701000.0},
  "history": [{"at": ..., "event": "claimed|failed|expired|retried", "note": "..."}],
  "result": {"commit": "a1b2c3d", "pages": ["wiki/..."]}
}
```

- **FIFO:** ids come from `seq` under the lock, zero-padded, so a directory
  listing is the order. A time-based name would sort wrong after a clock jump.
- **The sender:**
  - `member`: the caller's labels, checked as every nimitz command checks them.
  - `session`: a non-member Claude Code session, found by walking up the
    caller's processes to a Claude Code pid and its session file
    (`evidence::ancestors`, as the hook does).
  - `process`: anything else, with `--from <text>`, marked `verified: false`.
  - Provenance in the KB is built from this record, never from the payload.
- **Dedupe:** a `key` that matches a pending, claimed or done item (within
  `keep_done`) is not queued again; `post` prints the existing id.

### 4.3 Operations (each one under `queue.lock`; every write is a temp file plus rename)

| Command | Who | What |
| --- | --- | --- |
| `inbox post <staff> --type T (--file F \| --json S \| stdin) [--key K] [--from TEXT]` | by the policy (4.5) | validate the type and schema, size cap, depth cap, rate; write `pending/<seq>`; ring the doorbell (4.4); print the id |
| `inbox claim <staff>` | the staff only | first expire stale leases; take the oldest pending, `tries += 1`, set the lease, rename it to `claimed/`; print the item JSON, or nothing (exit 3) when empty |
| `inbox heartbeat <staff> <id>` | the staff | extend the lease (long items) |
| `inbox done <staff> <id> [--result JSON]` | the staff | `claimed/` -> `done/` with the result (e.g. the commit) |
| `inbox fail <staff> <id> --reason TEXT [--permanent]` | the staff | back to `pending/` with the reason in history; at `max_tries`, or with `--permanent`, -> `dead/` and a needs-you |
| `inbox list <staff> [--state ...]` / `inbox status [<staff>]` | anyone (read-only) | depths, the oldest age, the claimed item and its lease, dead items with their reason |
| `inbox retry <staff> <id>` / `inbox drop <staff> <id>` | the user, the parent | dead -> pending (tries reset) / dead or pending -> removed (kept in `dead/dropped/`) |

- A **lease expiry** is applied by every queue operation and by `check`: a
  claimed item past `until` goes back to pending ("expired" in history). Its
  `tries` already counted the claim, so a staff that crashes on an item
  dead-letters it after `max_tries`, not forever.
- **At least once, made exactly once:** a crash after the work but before
  `done` re-delivers the item. Each type's processing is idempotent by the
  item id (`wiki.note.v1`: section 6.3).
- **Exit codes:** 0 OK; 1 an error; 2 a usage error; 3 an empty queue (claim);
  4 refused by backpressure (full, rate, type, size), with the reason. A
  poster can tell "try later" from "never".

### 4.4 Types and schemas

`nimitz-core/src/queue/types.rs` is a registry of `(name, version, validator,
max_bytes)`. A type is versioned in its name (`wiki.note.v1`), so a v2 runs
next to v1 while posters move over.

`wiki.note.v1`:

```json
{
  "title": "optional, <= 200 chars",
  "body": "required markdown, <= 64 KiB",
  "domain": "optional, one of the KB's domains (llm-agents, rover-sim, tech, research, misc)",
  "tags": ["optional", "<= 20"],
  "links": ["optional URLs the note cites; not fetched in v1"],
  "source": {"label": "optional free text from the poster, e.g. 'claude-nimitz design talk'"}
}
```

The validator is hand-written in Rust: no JSON Schema dependency, and the
rules stay few.

### 4.5 Backpressure and the open policy

- `policy = "open"`: any process of this user on this machine may post. The
  queue folder is mode 0700 under the user's state dir, so this means "this
  user", not "anyone on the box". `fleet`: members of the staff's fleet only.
  `parent`: its parent only.
- **The depth cap** (pending plus claimed) refuses a post with exit 4,
  `inbox.full`; `check` shows it as a Warn. Nothing is dropped silently.
- **Per sender:** the sender key is the member id, the session id or the
  process's `--from`, with its window counter in `senders.json`.
- **A non-member session (the user's mod)** posts with plain
  `nimitz inbox post HOME/@KB --type wiki.note.v1 --file note.json`.
  - Its sender record is `session`, found by ancestry, with no labels needed.
  - A member posting is not a signal: the comms table does not apply. The
    inbox policy does, because posting asks the staff for work but does not
    talk to it.

## 5. The staff's loop

### 5.1 Three ways to wake (any one is enough; all three together close each other's gaps)

1. **Doorbell on post:**
   - `post` delivers one peer line to the staff session through its messaging
     socket (`inbox::send`, the transport `nimitz signal` already uses on Unix
     and, on windows-port, through the named pipe):
     `[nimitz inbox] HOME/@KB: 1 new item (wiki.note.v1, from HOME/GITHUB/NIMITZ_DEV). Drain your inbox.`
   - Idle: a new turn. Busy: it queues to the end of the turn.
   - Coalesced: no second ring while the last ring is under a minute old and
     the staff has not drained since (`ring.json`). A burst of 50 posts rings
     once.
2. **At session start:**
   - The SessionStart hook (startup, resume, clear, compact) adds the staff's
     rules and the inbox state as context: "3 pending, 1 claimed by an older
     session (its lease ends 12:04)".
   - So a relaunch, a `/clear`, a compact or a crash restart goes back to work
     on its own.
3. **Fallback timer** (no nimitz daemon; two cheap sources):
   - (a) **Piggyback.** Every nimitz run that already reads state (the hooks
     of every member, `status`, `check`) also checks: a pending item older
     than `ring_after` (2 min), the staff live and idle, no ring within
     `ring_after` -> ring. On a busy machine that is every few seconds.
   - (b) **The staff's own** session cron (CronCreate, every 15 min: "drain
     your inbox"). It is session-scoped, so the SessionStart context re-asks
     for it after a relaunch.

### 5.2 One pass

Text in the staff's rules (rules v5), held by `may_run`:

```
loop:
  item = nimitz inbox claim HOME/@KB        # exit 3: empty -> stop, wait for the next ring
  process item (its type's procedure, 6.3) # heartbeat if it runs past half the lease
  ok   -> nimitz inbox done HOME/@KB <id> --result '{"commit": "<sha>"}'
  fail -> nimitz inbox fail HOME/@KB <id> --reason "<one line>"   [--permanent when retrying cannot help]
```

- One at a time, FIFO, until empty. There is no parallel claim: one staff,
  one lease at a time (`claim` refuses a second while the staff holds one).
- **Context:** the conversation is disposable. The queue and git hold the
  state, so auto-compact is harmless, and `nimitz start HOME/@KB --fresh` is
  always safe (it runs between items).
- **Payloads are data, not instructions:**
  - the rules say so;
  - the permissions hold it (6.2);
  - every change lands in one commit that can be reverted.

### 5.3 Dead letters and other needs-yous

- A dead letter writes a needs-you notice for the **parent**: `[nimitz needs-you]
  HOME/@KB: item 17 (wiki.note.v1, from <sender>) failed 3 times: <reason>`.
  - That is the same `notify` path that today tells a commodore that its ship
    waits.
  - The admiral tells the user; the fix is `nimitz inbox retry|drop HOME/@KB 17`.
- A permission prompt or AskUserQuestion in the staff also notifies the parent.
  The staff runs unattended, as a ship does.

## 6. The first staff: `HOME/@KB`

### 6.1 The repo

- `/mnt/data/shared/nimitz-knowledge-base`: a new git repo, initialised from
  Lodestar (Folio v2: `raw/`, `wiki/`, `agent/`; one concept, one page).
  MYCC_DEV sets it up.
- It is **not** the Obsidian vault
  (`/mnt/data/shared/Notes/knowledge-base`), which stays as it is.
- **The gap:** Lodestar's `ingest` skill "confirms 3-7 key ideas before
  writing", which is a human gate. The staff needs an **auto mode** of ingest:
  - no confirmation;
  - provenance `[auto]` plus the sender (member or session, the session name,
    the item id);
  - gaps marked, never filled.
  This lives in the KB repo (`agent/skills/ingest-auto.md`), MYCC_DEV's part,
  and the staff brief points at it.
- **Dropbox and git:**
  - `.git` inside Dropbox works for one writer.
  - The staff is the only writer that runs unattended. The user may edit by
    hand, so the staff runs `git status --porcelain` first and treats foreign
    changes (section 8).

### 6.2 Its launch

- **Command:**
  `claude --bg --name "[HOME]-@KB" --model sonnet --settings '<labels + permissions>' "<staff prompt>"`
  in the repo folder, then pinned in `pins.json` like a lead.
- **The model:** before the launch, `availableModels` (managed settings, else
  the user's) must allow `sonnet`. Claude Code silently swaps a model that is
  not allowed for the account default (seen in the e2e work), so nimitz
  refuses instead: `staff.model` Error, fix "add sonnet to availableModels, or
  set model in [[staff]]". Today's list allows it (`opus`, `sonnet`, `fable`).
- **Permissions** (in `--settings`, so `respawnFlags` keep them):
  - `Bash(nimitz *)`, as every member has;
  - `Bash(git -C <repo> *)` and `Bash(git *)` in its folder;
  - Edit and Write within the repo (`acceptEdits` in its folder).
  - Nothing else is pre-allowed: a payload that talks it into `curl` or a write
    outside the repo hits a prompt, and the prompt becomes a needs-you, not an
    action.

### 6.3 `wiki.note.v1`, idempotent

1. `git log --grep "inbox-item: <id>"`. If a commit has it, `done` with that
   commit (an earlier pass did the work and crashed before `done`).
2. Run the auto-ingest procedure on the payload: search existing pages, then
   update or create, then cross-link.
3. `git add -A && git commit -m "ingest(auto): <title>" -m "inbox-item: <id>" -m "from: <sender>"`.
4. `inbox done ... --result {"commit": "<sha>", "pages": [...]}`.

Rollback is `git revert <sha>`; `inbox list --state done` maps items to commits.

## 7. Authority and comms

New rows for the role table (plan-v4 section 12.1, the
`every_command_goes_by_the_role_table` test):

| Command | admiral `F` | commodore | ship | staff `F/@S` | user |
| --- | --- | --- | --- | --- | --- |
| `staff new\|set\|rm F/@S` | its fleet | no | no | no | yes |
| `start\|stop F/@S` | its fleet (its own staff) | no | no | no (never itself) | yes |
| `inbox post <staff>` | by the inbox policy | by policy | by policy | by policy (not its own) | yes |
| `inbox claim\|heartbeat\|done\|fail <staff>` | no | no | no | **its own only** | yes (repairs) |
| `inbox retry\|drop <staff>` | its own staff | no | no | no | yes |
| `inbox list\|status` | yes | yes | yes | yes | yes |
| `status`, `check`, `rules`, `doctor` | yes | yes | yes | yes | yes |

Comms (`hook::signal_table`, and the PreToolUse deny for SendMessage):

- parent <-> staff, both ways (orders and reports);
- staff -> nobody else;
- the admiral's chain above it: none extra (an admiral is the top).
- A commodore staff later: its commodore, and that commodore's admiral by reply.

## 8. Failure modes

| What happens | Effect | What saves it |
| --- | --- | --- |
| Crash mid-item | Item claimed, no `done` | The lease expires -> pending; the git-log check (6.3) prevents a double commit |
| Reboot | The staff is down; the queue is on disk | `nimitz start HOME` (or `start` with no target) wakes it: it is pinned and named in the plan; check says `staff.down` |
| The daemon retires it | It shouldn't: it is pinned | `check` `staff.unpinned` re-pins on `start` |
| `/clear`, compact, relaunch, `--fresh` | Conversation lost | SessionStart context re-arms the loop; state is in the queue and git |
| Claude Code update | Old binary | `start` relaunches it like a worker, after its parent, only while it holds no claim (otherwise "busy, after this item") |
| The doorbell misses (no socket record, peer creds mismatch) | No wake | Piggyback timer; `check` `inbox.stuck` (oldest pending > 10 min while the staff is idle) |
| A poison item | Fails every time | `max_tries` -> dead -> needs-you to the parent; FIFO continues past it |
| Flood | Queue grows | Depth cap (exit 4 for the poster), per-sender rate; `check` `inbox.full` |
| A hostile payload | Tries to act | Data-only rules, the narrow permission allowlist (prompt -> needs-you), a reversible commit |
| The user edits the repo by hand while it runs | Dirty tree | The staff commits only the files it changed (`git add <paths>`), never `-A` when the tree had foreign changes at claim time; a conflicting edit -> `fail` with "repo dirty: <files>" |
| Dropbox conflicted copy in `.git` | A broken repo | `git fsck` failing -> `fail --permanent` and a needs-you; nimitz `check` names `config.conflict` already for fleet files, extended to the staff's repo |
| Two machines with the fleet | Two keepers | `dirs` names one machine; another machine refuses `start` of a staff with no folder there |
| The staff is busy for long | Posts wait | The lease is per item; `heartbeat` for a long one; posters see depth in `inbox status` |

## 9. How it shows

- **`nimitz status`:** a staff row under its parent, indented like a squadron's
  ships.
  ```
  HOME
    ADMIRAL      Idle   f29c13aa  ...
    @KB          Busy   7a1c02e9  inbox 3 pending · 1 claimed (lease 11m) · 0 dead · oldest 2m
  HOME · GITHUB
    COMMODORE    ...
  ```
- **`--json`:** a member with `role: "staff"`, `parent`, and
  `inbox: {pending, claimed, dead, oldest_age_s, lease_until}`.
- **`nimitz check`:**
  - `staff.down` (Error, fix `nimitz start HOME/@KB`);
  - `staff.unpinned`;
  - `staff.model`;
  - `inbox.dead` (Warn, fix retry or drop);
  - `inbox.stuck`;
  - `inbox.full`.
- **Admiralty:**
  - The staff row sits under its parent with a staff glyph and the depth badge
    (`3` or `3!` when dead > 0). Enter shows the inbox (items, sender, age,
    tries, the last reason).
  - Keys: `a` attaches (`start HOME/@KB -p`), `r` retries and `x` drops a dead
    item (the user's).
- **The layout-B plan view:** the staff row under its parent in `start` and
  `stop` plans, with its own `why` ("pinned staff; inbox 3 pending").

## 10. Start, stop, check

- **`start HOME` (or `start`):** the admiral, then its staff (top-down: a
  staff after its parent), then squadrons. A staff is woken by its exact id,
  like every member.
  - `start HOME/@KB` alone: its parent must be up (the same rule as a ship's
    commodore).
  - `-p` attaches its tab, `-f` relaunches, `--fresh` drops its conversation
    (safe: section 5.2).
- **`stop HOME`:** stops the staff before its parent. `stop HOME/@KB` stops it
  alone. A claimed item stays claimed and comes back with the lease.
- **`relaunch` on a Claude Code update:** like a worker. It waits while the
  staff holds a claim, then runs between items.
- **`check`:** the codes in 9. `doctor`: the queue folder is writable and owned
  by the user, and the type registry matches the config's `types`.

## 11. Generalising to other parents

The same code with a different parent:

- **ids:** the `@` rule;
- **config:** `[[staff]]` in `squadron.toml` for a commodore, `[[ships.staff]]`
  for a ship;
- **labels:** `NIMITZ_PARENT`;
- **comms:** parent <-> staff;
- **authority:** whoever may configure the parent may configure its staff.

The only question per parent kind is liveness. A ship's staff is still
"always on", which wakes a parked ship's helper; the config can say
`always_on = false` for staff that only wakes on a ring (the doorbell then
starts it: `post` runs `start <staff>` when the poster may, else it queues and
`check` shows it).

## 12. Windows (reconcile with `origin/windows-port`)

From the port's commits (7dd6984 .. 0145dfc):

- **Paths:** the defaults are unchanged (`NIMITZ_STATE` = `~/.local/state/nimitz`,
  `~` = `HOME` else `USERPROFILE`), so the queue is at
  `%USERPROFILE%\.local\state\nimitz\inbox\HOME%2F@KB\`.
  - `%2F` and `@` are valid in Windows file names.
  - Staff names are compared case-insensitively on disk (`os::name_on_disk`),
    and the id stays upper case.
- **The doorbell:**
  - On Windows the messaging socket is a named pipe with a random name, read
    from `messagingSocketPath`.
  - `inbox::send` (port) checks `GetNamedPipeServerProcessId`, sends the auth
    line with `peerToken` from `sessions/<pid>.<hash>.key`, then peeks for a
    refusal.
  - The doorbell uses that `send` unchanged. Nothing new per OS.
- **Locks:** the port moved every `flock` to std `File::lock` (`flock` /
  `LockFileEx`) on **dedicated lock files only** (`LockFileEx` is mandatory).
  `queue.lock` follows that rule: a separate empty file, never an item.
- **Atomic rename:**
  - Today every writer uses temp + `std::fs::rename` (`MoveFileExW(REPLACE_EXISTING)`).
    On Windows that fails while another process holds the target open
    without `FILE_SHARE_DELETE` (an antivirus scan, a reader, a Dropbox
    indexer).
  - The queue adds a small `os::replace` with a bounded retry (5 x 50 ms) on
    sharing violations, used by its writes and its state moves.
  - Readers open items with `FILE_SHARE_DELETE` (std's default open does not
    set it; `os::open_shared`). Shared with the port's `os.rs`.
- **State moves** (pending -> claimed -> done) are renames within one folder
  tree on one volume. Each is atomic on NTFS as on ext4, and the queue lock
  makes the move and the `seq` update one step.
- **Liveness:** "the staff is live and idle" for the piggyback timer uses the
  port's `os::alive` (`OpenProcess` + `GetExitCodeProcess`, ACCESS_DENIED =
  alive), and the session file's status.
- **The KB repo:** on Windows the staff is not started (`dirs` names
  `home-server` only). A Windows machine can still **post**: the queue is per
  machine, so a post on the Windows box queues for a Windows staff that does
  not exist. `post` refuses when this machine has no folder for the staff
  ("HOME/@KB runs on home-server"). A cross-machine relay is out of scope (an
  open question).
- **Merge order:** windows-port lands first (it touches `inbox.rs`, `os.rs`,
  the locks); staff builds on its `os` module. Until then the staff branch
  keeps every OS call behind `os::` names so the rebase is mechanical.

## 13. Build plan

| Phase | What | Tests |
| --- | --- | --- |
| S1 | `queue.rs` and the CLI: post, claim, heartbeat, done, fail, list, status, retry, drop; types registry with `wiki.note.v1`; locks, leases, dead letters, depth and rate limits, sender detection | unit (FIFO under concurrency, lease expiry, at-least-once, caps, rate), sandbox CLI, the role table |
| S2 | The staff member: `@` ids, `Role::Staff`, `[[staff]]` config and `staff new\|set\|rm`, labels, launch with the model check, pin, `start`/`stop`/`check`/`status`/`restore` placement; authority and comms rows; rules v5 (staff text and the loop) | sandbox (plans, top-down order, refusals), every role table test, a fake-claude e2e of the loop |
| S3 | The doorbell (ring and coalesce), the SessionStart context, the piggyback timer, needs-you for dead letters and prompts; Admiralty rows and the inbox panel | sandbox (rings, coalescing, piggyback), notify tests, Admiralty snapshots |
| S4 | `HOME/@KB` live: the repo and `ingest-auto` (MYCC_DEV), the brief, the git-commit idempotency | real-CC e2e on a throwaway repo: post -> commit; kill mid-item -> one commit only; `/clear` -> drains again |
| S5 | Other parents (`always_on`, ring-to-start) | sandbox |

Each phase is its own branch and review; nothing merges into dev until the
user says (branch policy, 2026-10-10).

## 14. Open questions for the user

1. **The policy for `HOME/@KB`:** `open` as chosen. Should a member sender also
   be rate-limited, or only non-members?
2. **Retention:** keep done items 30 days? (Git keeps the content anyway; the
   queue keeps the sender and the timing.)
3. **The fallback timer:** the piggyback alone (no in-session cron), or both?
   The cron costs a turn every 15 min while idle (cheap on Sonnet, but not
   zero).
4. **Cross-machine posting** (the Windows box posting to the server's KB): out
   of scope now; a later relay through the synced data root?
5. **Hand edits in the KB repo:** the staff refuses an item while the tree is
   dirty with foreign changes (safe), or commits only its own paths and carries
   on (the default above)?
6. **The mod's posting path:** does the mod call `nimitz inbox post` itself
   (Bash), or does it need a tiny `nimitz-post` wrapper skill?
