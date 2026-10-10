# Aides: always-on members with a durable inbox

Status: revised after the user's review (2026-10-10): the answers in section
14 (settled), MYCC_DEV's six changes, and a second aide, `@REF`,
with request and reply (sections 4.6 and 6.4). Nothing is built; S1 starts
after the user signs off on this revision (signed off 2026-10-10; S1 is on
`feat/aide-s1`). Branch `design/aide`, off dev 947d360.

## 1. Summary

An **aide** (naval: the admiral's aide; the role was called "staff" in the
first drafts) is a Claude Code background job attached to a parent member. It is pinned, always on, and it works through a **durable FIFO inbox
that nimitz owns**. Anyone allowed by the inbox's policy posts typed items. The
aide claims them one at a time, does the work and marks each one done; a
crash, reboot, `/clear` or relaunch loses nothing.

The first two share one git repo, `/mnt/data/shared/nimitz-knowledge-base`
(a Lodestar instance):

- `HOME/@KB`, the admiral's notes keeper, the **writer**. It turns
  `wiki.note.v1` items into pages, fully automatically, with one git commit
  per item. Fire and forget: the poster gets an id, no answer.
- `HOME/@REF`, the **reader**. It answers `wiki.query.v1` questions by
  Lodestar's query rules, read-only, from the last commit. Request and reply:
  the answer goes back to the one who asked, and only to them
  (`nimitz ask HOME/@REF "..." --wait 2m` returns it to an agent's Bash
  call).

Decisions already made by the user (from the commodore, 2026-10-10):

| Topic | Decision |
| --- | --- |
| Shape | A member attached to a parent, id `<parent>/@NAME`; admiral parents only for now, any lead or ship later |
| Liveness | Pinned, always on, woken by its parent's `start` |
| Config | `[[aide]]` |
| Model | Sonnet (checked against `availableModels`) |
| Inbox | Durable FIFO owned by nimitz; one file per item; pending -> claimed (lease) -> done, or dead after N tries; an expired lease returns an item to pending |
| Inbox commands | `nimitz inbox post <aide> --type ...`; the aide runs claim, done and fail; anyone runs list and status |
| Types | An allowlist with versioned schemas; `wiki.note.v1` only for now |
| Backpressure | A depth cap, per-sender limits, an open-inbox policy so non-member sessions (the user's mod) can post |
| Loop | A doorbell on post (the messaging socket), the queue at session start, a fallback timer; FIFO, one at a time, until empty; dead letters raise a needs-you |
| First aide | Notes keeper on the new repo (not the Obsidian vault), set up from Lodestar by MYCC_DEV; no human gate; provenance `[auto]` plus the source session; every ingest a git commit |
| Second aide | Retrieval: a read-only librarian on the same repo, Lodestar's query rules, its own `wiki.query.v1` queue |
| Request/reply | A type that declares `reply: true` gets its answer stored with the item and delivered to the requester only; `inbox result`, `nimitz ask --wait` |

## 2. Identity

| | Admiral aide (phase 1) | Commodore aide (later) | Ship aide (later) |
| --- | --- | --- | --- |
| Id | `HOME/@KB`, `HOME/@REF` | `HOME/GITHUB/@X` | `HOME/GITHUB/NIMITZ_DEV/@X` |
| Session name | `[HOME]-@KB` | `[HOME.GITHUB]-@X` | `[HOME.GITHUB.NIMITZ_DEV]-@X` |
| Herdr tab | `HOME.@KB` | `HOME.GITHUB.@X` | `HOME.GITHUB.NIMITZ_DEV.@X` |
| Labels | `NIMITZ_ROLE=aide`, `NIMITZ_ID`, `NIMITZ_FLEET`, `NIMITZ_PARENT=HOME` | `+ NIMITZ_SQUADRON` | `+ NIMITZ_SQUADRON` |

- `Id::role()` today counts parts (1 admiral, 2 commodore, 3+ ship). A last
  part that starts with `@` makes it `Role::Aide`, whatever the count, and
  `Id::parent()` drops that part. `@` is not valid in fleet, squadron or ship
  names today (`valid_name`), so no existing id changes meaning.
- `NIMITZ_PARENT` already exists as a label (`claude::labels`), so the parent
  is carried, not derived.
- The name is upper case like ship names (`@KB`, not `@kb`).

## 3. Config

In the parent's file: `fleet.toml` for an admiral's aide (`squadron.toml`
or `[[ships.aide]]` later).

```toml
[[aide]]
name = "KB"                      # id HOME/@KB
brief = "aides/kb/BRIEF.md"      # its CLAUDE.md, written by nimitz like a lead brief
model = "sonnet"                 # checked against availableModels
permission_mode = "acceptEdits"

[aide.dirs]                     # one machine runs it (an aide never runs twice)
home-server = "/mnt/data/shared/nimitz-knowledge-base"

[aide.inbox]
types = ["wiki.note.v1"]         # the allowlist; each type is a schema nimitz ships
policy = "open"                  # open | fleet | parent
depth_cap = 500                  # pending items; a post past it is refused
per_sender = "60/h"              # a sender's posts per window
lease = "15m"                    # a claim's lifetime; renewed by `inbox heartbeat`
max_tries = 3                    # then dead-letter
keep_done = "7d"                 # done items (and their results) kept, then pruned

[[aide]]
name = "REF"                     # id HOME/@REF
brief = "aides/ref/BRIEF.md"
model = "sonnet"
permission_mode = "default"      # read-only: Edit, Write and NotebookEdit denied in its settings
snapshot = true                  # reads a detached worktree at the repo's last commit (6.4)

[aide.dirs]
home-server = "/mnt/data/shared/nimitz-knowledge-base"   # the repo; it works in the snapshot

[aide.inbox]
types = ["wiki.query.v1"]
policy = "open"
depth_cap = 200
per_sender = "120/h"
lease = "10m"
max_tries = 2
keep_done = "7d"
```

- `aide new HOME/@KB --dir ... --model sonnet` / `aide set` / `aide rm`
  behave like `ship new|set|rm` (plan, `--yes`, `may_config`).
- The folder runs one machine's job only. A second machine with the fleet has
  no `dirs` entry for the aide, so it never starts it. That avoids two
  keepers committing to one Dropbox repo.

## 4. The inbox

### 4.1 Where it lives

`NIMITZ_STATE/queues/<aide id, / as %2F>/` (`~/.local/state/nimitz`): on
this machine and **never in Dropbox**. Sync would turn every rename into a
conflict risk, and the queue belongs to the one machine that runs the aide.

(The existing `nimitz-core/src/inbox.rs` is the *doorbell transport*:
delivery of one line into a session's messaging socket; it already keeps
its socket records in `NIMITZ_STATE/inbox/<session id>.json`, hence
`queues/`. The queue is a new module, `queue.rs`. The CLI word stays
`inbox`, as the user chose.)

```
queues/HOME%2F@KB/
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
  "reply_to": {"token_sha256": "...", "member": "HOME/GITHUB/NIMITZ_DEV",
               "session": "9aacf927-...", "job": "9aacf927", "delivered_at": null},
  "claim": {"by": "<aide session id>", "at": 1791700100.0, "until": 1791701000.0},
  "history": [{"at": ..., "event": "claimed|failed|expired|retried|delivered", "note": "..."}],
  "result": {"commit": "a1b2c3d", "pages": ["wiki/..."]}
}
```

(`reply_to` only for a type with `reply: true`, section 4.6.)

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
  - **The key convention** (MYCC_DEV): a poster sets `key = sha256(body)`, so
    nimitz drops a repeat before it is queued. The KB's own sha256 check of a
    capture is the backstop.

### 4.3 Operations (each one under `queue.lock`; every write is a temp file plus rename)

| Command | Who | What |
| --- | --- | --- |
| `inbox post <aide> --type T (--file F \| --payload JSON \| stdin) [--key K] [--from TEXT]` | by the policy (4.5) | validate the type and schema, size cap, depth cap, rate; write `pending/<seq>`; ring the doorbell (S3); print the id (`--json`: `{id, token, duplicate}`) |
| `inbox claim <aide>` | the aide only | first expire stale leases; take the oldest pending, `tries += 1`, set the lease, rename it to `claimed/`; print the item JSON, or nothing (exit 3) when empty |
| `inbox heartbeat <aide> <id>` | the aide | extend the lease (long items) |
| `inbox done <aide> <id> [--result JSON]` | the aide | `claimed/` -> `done/` with the result (the commit; for a reply type, the answer, 4.6), then deliver it |
| `inbox fail <aide> <id> --reason TEXT [--permanent]` | the aide | back to `pending/` with the reason in history; at `max_tries`, or with `--permanent`, -> `dead/` and a needs-you |
| `inbox list <aide> [--state ...]` / `inbox status [<aide>]` | anyone (read-only) | depths, the oldest age, the claimed item and its lease, dead items with their reason |
| `inbox retry <aide> <id>` / `inbox drop <aide> <id>` | the user, the parent | dead -> pending (tries reset) / dead or pending -> removed (kept in `dead/dropped/`) |
| `inbox result <aide> <id> [--token T] [--wait D]` | **the requester only** (4.6), the user | the item's state and, once done, its result; `--wait` blocks until done or dead |
| `ask <aide> "<question>" [--scope S] [--length short\|medium\|long] [--wait D \| --no-wait]` | by the policy | posts a `wiki.query.v1` and waits (2 minutes unless `--wait`) for the answer and prints it (exit 0), or exits 5 at the timeout with the id and the token to fetch it later; `--no-wait` returns at once (exit 5) |

- A **lease expiry** is applied by every queue operation and by `check`: a
  claimed item past `until` goes back to pending ("expired" in history). Its
  `tries` already counted the claim, so an aide that crashes on an item
  dead-letters it after `max_tries`, not forever.
- **At least once, made exactly once:** a crash after the work but before
  `done` re-delivers the item. Each type's processing is idempotent by the
  item id (`wiki.note.v1`: section 6.3).
- **Exit codes:** 0 OK; 1 an error; 2 a usage error; 3 an empty queue (claim);
  4 refused by backpressure (full, rate, type, size), with the reason; 5 not
  done yet (`result`/`ask` at the timeout); 6 dead (`result`/`ask`: the item
  failed for good, the reason printed). A poster can tell "try later" from
  "never".

### 4.4 Types and schemas

`nimitz-core/src/queue/types.rs` is a registry of `(name, version, validator,
max_bytes, reply)`. A type is versioned in its name (`wiki.note.v1`), so a v2
runs next to v1 while posters move over. `reply: true` makes it a request
(4.6).

`wiki.note.v1` (`reply: false`):

```json
{
  "title": "optional, <= 200 chars",
  "body": "required markdown, <= 64 KiB",
  "kind": "optional: decision | fact | howto | gotcha | question | preference | gap",
  "domain": "optional slug hint ([a-z0-9-]+, <= 40); the KB maps one it does not know to misc",
  "tags": ["optional", "<= 20"],
  "links": ["optional URLs the note cites; not fetched in v1"],
  "source": {"label": "optional free text from the poster, e.g. 'claude-nimitz design talk'"}
}
```

- `kind` comes from the mod's triage; `ingest-auto` uses it to pick the page
  type (MYCC_DEV).
- `domain` is a hint, not a list nimitz keeps: the KB's domains change at its
  scope checkpoint (MYCC_DEV).

`wiki.query.v1` (`reply: true`):

```json
{
  "question": "required, <= 2000 chars",
  "scope": "optional: a domain slug, or a page path prefix under wiki/",
  "length": "optional: short (a paragraph) | medium (default) | long"
}
```

Its result (what `inbox done --result` carries; `inbox result` prints it):

```json
{
  "answer": "markdown, <= 32 KiB; every claim cites its page as [[page]]",
  "citations": ["wiki/llm-agents/....md", "..."],
  "gaps": "what the wiki does not say (Lodestar: gaps are marked, never filled)",
  "confidence": "high | medium | low",
  "snapshot": "the commit it read"
}
```

The validator is hand-written in Rust: no JSON Schema dependency, and the
rules stay few.

### 4.5 Backpressure and the open policy

- `policy = "open"`: any process of this user on this machine may post. The
  queue folder is mode 0700 under the user's state dir, so this means "this
  user", not "anyone on the box". `fleet`: members of the aide's fleet only.
  `parent`: its parent only.
- **The depth cap** (pending plus claimed) refuses a post with exit 4,
  `inbox.full`; `check` shows it as a Warn. Nothing is dropped silently.
- **Per sender:** the sender key is the member id, the session id or the
  process's `--from`, with its window counter in `senders.json`.
- **A non-member session (the user's mod)** posts with plain
  `nimitz inbox post HOME/@KB --type wiki.note.v1 --file note.json`.
  - Its sender record is `session`, found by ancestry, with no labels needed.
  - A member posting is not a signal: the comms table does not apply. The
    inbox policy does, because posting asks the aide for work but does not
    talk to it.
- **Rate limits** apply to **every sender**, members included (the user,
  2026-10-10): `per_sender` per member id, session or `--from` process.

### 4.6 Request and reply (any type with `reply: true`)

One mechanism for both aide: `@KB`'s notes are fire and forget (`reply:
false`), `@REF`'s queries are requests. Nothing else differs: the same
FIFO, lease, tries and dead letters.

**The reply address.** `post` of a reply type records who asked:

- the sender identity (4.2): a member id, else a session id and its job, else
  an unverified process;
- how to reach it: the session's messaging socket, looked up at delivery time
  from its session file (the socket changes on a restart, so it is never
  stored);
- a **reply token**: 128 random bits, printed once by `post` (`--json`:
  `{"id", "token"}`); only its sha256 is stored.

**The answer** is stored with the item: `inbox done --result` writes it into
`done/<id>.json`, durable like the item (crash, reboot, `/clear`, relaunch).

**Delivery, once, to the requester only:**

1. At `done`, nimitz (not the aide: the aide never addresses the
   requester) checks the reply address. A member or session that is live
   gets a peer message through its own messaging socket (`inbox::send`, the
   peer-credential check included):
   `[nimitz reply] HOME/@REF answered your question 000000000042 (wiki.query.v1). <answer, up to 8 KiB; longer: "run nimitz inbox result HOME/@REF 000000000042">`.
   It is a notice: it arrives as a peer message, never as the user's input.
2. `delivered_at` is set, and there is no second delivery.
3. Not live, or no socket (a process sender): nothing is sent. The answer
   waits in `done/` for `inbox result`.

**Who may read a result** (`inbox result`, `ask --wait`):

- the same member (its labels), or the same session or job (by ancestry, as
  at post);
- or anyone holding the reply token (`--token`): that is how a process
  sender, or the same agent after a restart that changed its session, gets
  it;
- and the user. Every other caller gets `inbox.not_yours` with no content,
  not even the question.
- `inbox list` shows other senders' items without payloads or results.

**`nimitz ask`** posts, keeps the token in memory and polls the item file
(every 500 ms; the queue lock is not held between polls) until it is done
(print the answer, exit 0), dead (print the reason, exit 6) or the timeout
(print the id and the token, exit 5). An agent's Bash call gets the answer
synchronously, and nothing else gets it. `--wait` defaults to 2 minutes and
is capped at 10 (a Bash call's own timeout).

**A requester that has gone:** the result stays in `done/` until `keep_done`
(7 days), readable with the token or by the same member; there is no
redelivery to a later session. A member that comes back under the same id
reads it with `inbox result`.

## 5. The aide's loop

### 5.1 Three ways to wake (any one is enough; all three together close each other's gaps)

1. **Doorbell on post:**
   - `post` delivers one peer line to the aide session through its messaging
     socket (`inbox::send`, the transport `nimitz signal` already uses on Unix
     and, on windows-port, through the named pipe):
     `[nimitz inbox] HOME/@KB: 1 new item (wiki.note.v1, from HOME/GITHUB/NIMITZ_DEV). Drain your inbox.`
   - Idle: a new turn. Busy: it queues to the end of the turn.
   - Coalesced: no second ring while the last ring is under a minute old and
     the aide has not drained since (`ring.json`). A burst of 50 posts rings
     once.
2. **At session start:**
   - The SessionStart hook (startup, resume, clear, compact) adds the aide's
     rules and the inbox state as context: "3 pending, 1 claimed by an older
     session (its lease ends 12:04)".
   - So a relaunch, a `/clear`, a compact or a crash restart goes back to work
     on its own.
3. **Fallback timer** (no nimitz daemon; two cheap sources, both kept, the
   user's choice):
   - (a) **Piggyback.** Every nimitz run that already reads state (the hooks
     of every member, `status`, `check`) also checks: a pending item older
     than `ring_after` (2 min), the aide live and idle, no ring within
     `ring_after` -> ring. On a busy machine that is every few seconds.
   - (b) **The aide's own** session cron (CronCreate, every 15 minutes:
     "drain your inbox"), a cheap Sonnet turn while idle. It is
     session-scoped, so the SessionStart context asks for it again after a
     relaunch, a `/clear` or a compact.

### 5.2 One pass

Text in the aide's rules (rules v5), held by `may_run`:

```
loop:
  item = nimitz inbox claim <me>            # exit 3: empty -> stop, wait for the next ring
  process item (its type's procedure: 6.3 a note, 6.4 a query)   # heartbeat past half the lease
  ok   -> nimitz inbox done <me> <id> --result '<the result; for a query, the answer>'
  fail -> nimitz inbox fail <me> <id> --reason "<one line>"   [--permanent when retrying cannot help]
```

For a reply type the aide never contacts the requester: `done` stores the
answer and nimitz delivers it (4.6).

- One at a time, FIFO, until empty. There is no parallel claim: one aide,
  one lease at a time (`claim` refuses a second while the aide holds one).
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
- A permission prompt or AskUserQuestion in the aide also notifies the parent.
  The aide runs unattended, as a ship does.

## 6. The first two aides: `HOME/@KB` and `HOME/@REF`

### 6.1 The repo

- `/mnt/data/shared/nimitz-knowledge-base`: a new git repo, initialised from
  Lodestar (Folio v2: `raw/`, `wiki/`, `agent/`; one concept, one page).
  MYCC_DEV sets it up.
- It is **not** the Obsidian vault
  (`/mnt/data/shared/Notes/knowledge-base`), which stays as it is.
- **The gap:** Lodestar's `ingest` skill "confirms 3-7 key ideas before
  writing", which is a human gate. The aide needs an **auto mode** of ingest:
  - no confirmation;
  - provenance `[auto]` plus the sender (member or session, the session name,
    the item id);
  - gaps marked, never filled.
  This lives in the KB repo (`agent/skills/ingest-auto.md`), MYCC_DEV's part,
  and the aide brief points at it.
- **Provenance through the capture file** (MYCC_DEV): `ingest-auto` writes the
  item to a capture file under `raw/` whose frontmatter carries the sender,
  the session and the item id (mapped from the envelope, 4.2, and the
  payload, 4.4); a page cites it as `[auto: <capture-file>]`. That meets the
  provenance rule above by one level of indirection: page -> capture ->
  sender, session and item.
- **Dropbox and git:**
  - `.git` inside Dropbox works for one writer.
  - The aide is the only writer that runs unattended. The user may edit by
    hand, so the aide commits only the paths it wrote (6.3), whatever else
    is dirty (the user's choice; section 8).

### 6.2 Its launch

- **Command:**
  `claude --bg --name "[HOME]-@KB" --model sonnet --settings '<labels + permissions>' "<aide prompt>"`
  in the repo folder, then pinned in `pins.json` like a lead.
- **The model:** before the launch, `availableModels` (managed settings, else
  the user's) must allow `sonnet`. Claude Code silently swaps a model that is
  not allowed for the account default (seen in the e2e work), so nimitz
  refuses instead: `aide.model` Error, fix "add sonnet to availableModels, or
  set model in [[aide]]". Today's list allows it (`opus`, `sonnet`, `fable`).
- **Permissions** (in `--settings`, so `respawnFlags` keep them):
  - `Bash(nimitz *)`, as every member has;
  - `Bash(git -C <repo> *)` and `Bash(git *)` in its folder;
  - Edit and Write within the repo (`acceptEdits` in its folder).
  - Nothing else is pre-allowed: a payload that talks it into `curl` or a write
    outside the repo hits a prompt, and the prompt becomes a needs-you, not an
    action.

### 6.3 `wiki.note.v1`, idempotent

1. `git log -E --grep '^inbox-item: <id>$'` (anchored: item 42 never matches
   item 420). If a commit has it, `done` with that commit (an earlier pass did
   the work and crashed before `done`).
2. Run the auto-ingest procedure on the payload: the capture file, then search
   existing pages, update or create, cross-link.
3. Stage **the paths it wrote, by name** (never `git add -A`: the user may
   have other changes in the tree), then
   `git commit -m "ingest(auto): <title>" -m "inbox-item: <id>" -m "from: <sender>"`.
4. `inbox done ... --result {"commit": "<sha>", "pages": [...]}`.

Rollback is `git revert <sha>`; `inbox list --state done` maps items to commits.

### 6.4 The second aide: `HOME/@REF`, read-only retrieval

The name: `@REF` (the user's choice): the reference desk, which finds and
cites and never writes. "The librarian" below is its role.

- **Always on, Sonnet, pinned**, like `@KB`; its own queue,
  `queues/HOME%2F@REF/`, type `wiki.query.v1`, the same FIFO, lease and
  dead-letter machinery.
- **It follows Lodestar's query rules** (the KB's `agent/skills/query.md`):
  - navigate by `_index.md`;
  - cite every claim with its page;
  - carry uncertainty into the answer (`confidence`, `gaps`);
  - never write.
- **Read-only, enforced, not only asked:** its settings deny `Edit`, `Write`
  and `NotebookEdit`; it allows `Read`, `Grep`, `Glob` in its snapshot and
  `Bash(nimitz inbox *)`; `permission_mode = "default"`, so anything else
  prompts and the prompt becomes a needs-you.
- **A consistent snapshot, never the working tree mid-write.** `@KB` writes
  files, then commits; a reader of the working tree could see half an ingest.
  So the librarian works in a **detached worktree at the last commit**:
  - `NIMITZ_STATE/aides/HOME%2F@REF/snapshot`, made once with
    `git -C <repo> worktree add --detach <snapshot> HEAD` (outside Dropbox: a
    synced worktree would race the sync);
  - `inbox claim` (nimitz, not the aide) moves it to the repo's current
    `HEAD` before it hands out a query: `git -C <snapshot> checkout --detach
    <HEAD>`. A commit is atomic for a reader (`HEAD` moves after its objects
    are written), so every query reads one whole commit;
  - the answer's `snapshot` names that commit; a citation is reproducible
    with `git show <commit>:<page>`;
  - its job's folder is the snapshot, which `nimitz trust` (the trust
    feature) plans like any member folder: a worktree needs its own trust.
- **Processing a `wiki.query.v1`:** claim, then answer by the query rules
  within `length`, then `inbox done --result {answer, citations, gaps,
  confidence, snapshot}`. A question the wiki cannot answer is still `done`:
  an answer that says so, with `gaps`. Only an error is `fail`.
- **Gaps go to `@KB`** (the user, 2026-10-10). A gap it finds while
  answering (the wiki does not say, or says it two ways) it posts with
  `nimitz inbox post HOME/@KB --type wiki.note.v1`, `kind: "gap"` (nothing
  known yet) or `kind: "question"` (an open question), the query's item id in
  `source.label`. It goes through the inbox like any post: `@KB`'s policy,
  the per-sender rate limit (the librarian is a sender like everyone else),
  dedupe by `key = sha256(body)`. It still never writes the repo itself.

## 7. Authority and comms

New rows for the role table (plan-v4 section 12.1, the
`every_command_goes_by_the_role_table` test):

| Command | admiral `F` | commodore | ship | aide `F/@S` | user |
| --- | --- | --- | --- | --- | --- |
| `aide new\|set\|rm F/@S` | its fleet | no | no | no | yes |
| `start\|stop F/@S` | its fleet (its own aide) | no | no | no (never itself) | yes |
| `inbox post <aide>` | by the inbox policy | by policy | by policy | by policy (not its own) | yes |
| `inbox claim\|heartbeat\|done\|fail <aide>` | no | no | no | **its own only** | yes (repairs) |
| `inbox retry\|drop <aide>` | its own aide | no | no | no | yes |
| `inbox result <aide> <id>` | its own requests only | its own only | its own only | its own only | yes |
| `ask <aide> "<question>"` | by the inbox policy | by policy | by policy | by policy (not its own) | yes |
| `inbox list\|status` | yes (others' payloads and results hidden) | yes (same) | yes (same) | yes (same) | yes |
| `status`, `check`, `rules`, `doctor` | yes | yes | yes | yes | yes |

Comms (`hook::signal_table`, and the PreToolUse deny for SendMessage):

- parent <-> aide, both ways (orders and reports);
- aide -> nobody else;
- the admiral's chain above it: none extra (an admiral is the top).
- A commodore aide later: its commodore, and that commodore's admiral by reply.
- **A reply is not comms.** The answer to a request goes from nimitz, at
  `done`, to the requester (4.6), never from the aide by SendMessage. So a
  librarian needs no comms row for each asker, and a member cannot use an
  aide to reach a session it may not signal: the reply only ever goes back to
  the one who asked.

## 8. Failure modes

| What happens | Effect | What saves it |
| --- | --- | --- |
| Crash mid-item | Item claimed, no `done` | The lease expires -> pending; the git-log check (6.3) prevents a double commit |
| Reboot | The aide is down; the queue is on disk | `nimitz start HOME` (or `start` with no target) wakes it: it is pinned and named in the plan; check says `aide.down` |
| The daemon retires it | It shouldn't: it is pinned | `check` `aide.unpinned` re-pins on `start` |
| `/clear`, compact, relaunch, `--fresh` | Conversation lost | SessionStart context re-arms the loop; state is in the queue and git |
| Claude Code update | Old binary | `start` relaunches it like a worker, after its parent, only while it holds no claim (otherwise "busy, after this item") |
| The doorbell misses (no socket record, peer creds mismatch) | No wake | Piggyback timer; `check` `inbox.stuck` (oldest pending > 10 min while the aide is idle) |
| A poison item | Fails every time | `max_tries` -> dead -> needs-you to the parent; FIFO continues past it |
| Flood | Queue grows | Depth cap (exit 4 for the poster), per-sender rate; `check` `inbox.full` |
| A hostile payload | Tries to act | Data-only rules, the narrow permission allowlist (prompt -> needs-you), a reversible commit |
| The user edits the repo by hand while it runs | Dirty tree | The aide stages and commits only the paths it wrote, by name (6.3), and carries on (the user's choice); an edit to a file it must write -> `fail` with "repo dirty: <files>" |
| Dropbox conflicted copy in `.git` | A broken repo | `git fsck` failing -> `fail --permanent` and a needs-you; nimitz `check` names `config.conflict` already for fleet files, extended to the aide's repo |
| Two machines with the fleet | Two keepers | `dirs` names one machine; another machine refuses `start` of an aide with no folder there |
| The aide is busy for long | Posts wait | The lease is per item; `heartbeat` for a long one; posters see depth in `inbox status` |
| The requester has gone (session ended, relaunched, a script exited) | The answer cannot be delivered | Kept in `done/` until `keep_done` (7 days), readable with the token or by the same member; no redelivery |
| `ask --wait` times out | The agent's call returns without the answer | Exit 5 with the id and the token; `inbox result ... --token T` later |
| The librarian reads while `@KB` writes | A half-written ingest | It reads its snapshot at the last commit (6.4), never the working tree |
| The snapshot worktree is broken or missing | Queries cannot be read | `claim` recreates it (`worktree prune`, `worktree add --detach`); failing that, `fail` and a needs-you |
| Someone asks for another's result | A leak | `inbox.not_yours`, no content: identity or token only (4.6) |
| A long answer | Peer messages are short | Up to 8 KiB inline, else a pointer to `inbox result`; the stored result is capped at 32 KiB |

## 9. How it shows

- **`nimitz status`:** an aide row under its parent, indented like a squadron's
  ships.
  ```
  HOME
    ADMIRAL      Idle   f29c13aa  ...
    @KB          Busy   7a1c02e9  inbox 3 pending · 1 claimed (lease 11m) · 0 dead · oldest 2m
    @REF         Idle   5e0b77c1  inbox empty · snapshot 4f2a9c1 · 2 answers waiting for pickup
  HOME · GITHUB
    COMMODORE    ...
  ```
- **`--json`:** a member with `role: "aide"`, `parent`, and
  `inbox: {pending, claimed, dead, oldest_age_s, lease_until, undelivered}`
  (and `snapshot` for a reader).
- **`nimitz check`:**
  - `aide.down` (Error, fix `nimitz start HOME/@KB`);
  - `aide.unpinned`;
  - `aide.model`;
  - `inbox.dead` (Warn, fix retry or drop);
  - `inbox.stuck`;
  - `inbox.full`.
- **Admiralty** (S2, the user's spec):
  - An aide row under its parent (in the admiral's box) with an inline queue
    summary: `KB  idle  · 3 pending · oldest 4m · 1 dead`.
  - The row's colour by the theme rules: dead letters raise needs-you pink; a
    backlog over a threshold shows warning yellow; processing shows busy green
    with the animation.
  - The detail panel (`w`) shows the queue: pending; claimed (the lease holder
    and its expiry); done in the last 7 days; dead with reasons; the oldest
    item's age; per-sender rate-limit state; the last few items (type, sender,
    age, and the result for a reply type); the actions, retry or drop a dead
    letter (the user's only).
  - Keys: `a` attaches (`start HOME/@KB -p`).
  - Data: `nimitz inbox status --json` (S1), the same summary `status` shows.
- **The layout-B plan view:** the aide row under its parent in `start` and
  `stop` plans, with its own `why` ("pinned aide; inbox 3 pending").

## 10. Start, stop, check

- **`start HOME` (or `start`):** the admiral, then its aide (top-down: an
  aide after its parent), then squadrons. An aide is woken by its exact id,
  like every member.
  - `start HOME/@KB` alone: its parent must be up (the same rule as a ship's
    commodore).
  - `-p` attaches its tab, `-f` relaunches, `--fresh` drops its conversation
    (safe: section 5.2).
- **`stop HOME`:** stops the aide before its parent. `stop HOME/@KB` stops it
  alone. A claimed item stays claimed and comes back with the lease.
- **`relaunch` on a Claude Code update:** like a worker. It waits while the
  aide holds a claim, then runs between items.
- **`check`:** the codes in 9. `doctor`: the queue folder is writable and owned
  by the user, and the type registry matches the config's `types`.

## 11. Generalising to other parents

The same code with a different parent:

- **ids:** the `@` rule;
- **config:** `[[aide]]` in `squadron.toml` for a commodore, `[[ships.aide]]`
  for a ship;
- **labels:** `NIMITZ_PARENT`;
- **comms:** parent <-> aide;
- **authority:** whoever may configure the parent may configure its aide.

The only question per parent kind is liveness. A ship's aide is still
"always on", which wakes a parked ship's helper; the config can say
`always_on = false` for aide that only wakes on a ring (the doorbell then
starts it: `post` runs `start <aide>` when the poster may, else it queues and
`check` shows it).

## 12. Windows (reconcile with `origin/windows-port`)

From the port's commits (7dd6984 .. 0145dfc):

- **Paths:** the defaults are unchanged (`NIMITZ_STATE` = `~/.local/state/nimitz`,
  `~` = `HOME` else `USERPROFILE`), so the queue is at
  `%USERPROFILE%\.local\state\nimitz\inbox\HOME%2F@KB\`.
  - `%2F` and `@` are valid in Windows file names.
  - Aide names are compared case-insensitively on disk (`os::name_on_disk`),
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
- **Liveness:** "the aide is live and idle" for the piggyback timer uses the
  port's `os::alive` (`OpenProcess` + `GetExitCodeProcess`, ACCESS_DENIED =
  alive), and the session file's status.
- **The KB repo:** on Windows the aide is not started (`dirs` names
  `home-server` only). A Windows machine can still **post**: the queue is per
  machine, so a post on the Windows box queues for a Windows aide that does
  not exist. `post` refuses when this machine has no folder for the aide
  ("HOME/@KB runs on home-server"). A cross-machine relay is out of scope
  (the user, 2026-10-10).
- **Request and reply:** delivery uses the same `inbox::send` (the named pipe
  and its auth line on Windows); `ask --wait` polls the item file, so it
  needs no pipe of its own. The reply token is written with the item through
  `os::replace`.
- **The librarian's snapshot** is a `git worktree` under `NIMITZ_STATE`; on
  Windows a checkout can fail while its reader holds a file open, so `claim`
  retries the refresh briefly and, failing that, answers from the snapshot it
  has (its commit named in the answer).
- **Merge order:** windows-port lands first (it touches `inbox.rs`, `os.rs`,
  the locks); aide builds on its `os` module. Until then the aide branch
  keeps every OS call behind `os::` names so the rebase is mechanical.

## 13. Build plan

| Phase | What | Tests |
| --- | --- | --- |
| S1 | `queue.rs` and the CLI: post, claim, heartbeat, done, fail, list, status, retry, drop; types registry with `wiki.note.v1` and `wiki.query.v1`; locks, leases, dead letters, depth and rate limits, sender detection; request and reply: the reply address, tokens, `inbox result`, `nimitz ask --wait`, delivery at `done`; `inbox status --json`, the structured summary Admiralty and `status` read | unit (FIFO under concurrency, lease expiry, at-least-once, caps, rate, who may read a result), sandbox CLI, the role table |
| S2 | The aide: `@` ids, `Role::Aide`, `[[aide]]` config and `aide new\|set\|rm`, labels, launch with the model check, pin, `start`/`stop`/`check`/`status`/`restore` placement; authority and comms rows; rules v5 (aide text and the loop); **Admiralty aide rows and the queue detail panel, `status`'s queue summary** (the user, 2026-10-10: as early as the aide exists) | sandbox (plans, top-down order, refusals), every role table test, a fake-claude e2e of the loop; Admiralty snapshots at 80 and 120 columns, images for the user before merge |
| S3 | The doorbell (ring and coalesce), the SessionStart context (with the 15-minute cron request), the piggyback timer, needs-you for dead letters and prompts | sandbox (rings, coalescing, piggyback), notify tests, Admiralty snapshots |
| S4 | `HOME/@KB` live: the repo and `ingest-auto` (MYCC_DEV), the brief, the git-commit idempotency | real-CC e2e on a throwaway repo: post -> commit; kill mid-item -> one commit only; `/clear` -> drains again |
| S4b | `HOME/@REF` live: the snapshot worktree and its refresh at claim, read-only settings, the query brief (Lodestar's query rules) | real-CC e2e: `ask --wait` returns a cited answer; a second session cannot read it; a commit during a query does not change what it read |
| S5 | Other parents (`always_on`, ring-to-start) | sandbox |

Each phase is its own branch and review; nothing merges into dev until the
user says (branch policy, 2026-10-10).

## 14. Decisions (the user, 2026-10-10): settled

From the md review (verdict "revise the design first"; comment: "include the
design for librarian", done in 4.6 and 6.4) and the follow-up answers:

1. Rate limits: **every sender**, members included.
2. Retention: done items and stored reply results kept **7 days**.
3. The fallback timer: **the piggyback plus an in-session cron** every 15
   minutes.
4. Cross-machine posting: not now.
5. Hand edits in the KB repo: commit only its own paths and carry on.
6. The mod calls `nimitz inbox post` (and `nimitz ask`) itself.
7. The librarian's name: **`@REF`** (`HOME/@REF`).
8. The librarian **posts the gaps it finds to `@KB`**, as `wiki.note.v1`
   items of kind `gap` or `question`, through `nimitz inbox post`,
   rate-limited like every sender (6.4).
9. `ask --wait`: **2 minutes by default, capped at 10**.

No open questions. S1 starts after the user signs off on this revision.
