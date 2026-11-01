# Fleet-wide aides: posting across machines, and config without lockstep

Status: design for the user (2026-10-12), via the commodore: "the aide design
is flawed for a multi-machine fleet". Two parts: (1) any machine of a fleet
posts to or asks an aide that runs on another; (2) a nimitz never rejects a
fleet for entries it does not need, so machines need not move in lockstep.

## 1. Posting across machines

The aide's queue stays where it is, durable and local to the machine that
runs the aide (one writer, one lock). What is new is a way in from the
other machines, through the synced data root (`H` = `~/.claude-nimitz`, in
Dropbox), and a way back for answers.

### 1.1 The outbox (one writer per folder)

`H/<fleet>/relay/<from machine>/out/<aide %2F>/<envelope>.json`

- Written only by the machine it is named for (no two machines ever write
  one file, so Dropbox never makes a conflicted copy of it). A file is
  written whole (temp name, then rename), never changed after.
- `<envelope>` = `<from machine>-<16 random bytes, hex>`: unique, never
  reused. The file holds the post as the poster's nimitz made it: the
  type, the payload (same limits as a local post), the key, the sender (as
  `sender_of` judged it on that machine), `posted_at`, and for a request
  the reply token's sha256 and `waiting_until`.
- `nimitz inbox post` and `ask` for an aide that runs on another machine
  write this file instead of refusing (`inbox.elsewhere` goes away). The
  policy, the type, the size and the poster's own rate limit are checked
  where the post is made, and again where it is taken in (1.3).

### 1.2 Taking it in, exactly once

The aide's machine drains every `relay/*/out/<aide>/` into its local queue:

- **When:** at every piggyback pass on that machine (any member's hook,
  `status`, `check`: at most every 30 s), at every `inbox claim` of the aide
  (so its 15-minute cron and every doorbell drain first), and at the
  aide's session start.
- **Once:** under the queue lock, an envelope already in the queue's
  `relay.json` ledger is skipped; otherwise it is posted with the key
  `relay:<envelope>` (the queue's own key dedupe catches a crash between
  the post and the ledger) and the ledger records it. Dropbox redelivering
  a file, or a `(conflicted copy)` of one, never makes a second item: a
  file whose name or envelope does not match is ignored and reported.
- **Ack:** the aide's machine writes, in its own folder,
  `H/<fleet>/relay/<aide machine>/acks/<from machine>.json`: each envelope
  it took in, with its local item id and state. The poster's machine
  removes its own outbox files once acked (or after the aide's
  `keep_done`, 7 days, whatever happened), and `inbox status` there shows
  "queued on home-server as item 17".
- **The doorbell:** the drain rings the aide (as a local post does).

### 1.3 Who posted: trust

The threat: a synced file that says it comes from a member it does not.

- What a file proves: only which machine's folder it is in, and that only
  as far as the data root is trusted. Whoever can write the data root (any
  of the user's machines, or the Dropbox account) can already rewrite
  fleet.toml, the lead briefs and the KB itself. So nimitz does not
  pretend a signature would make the data root safe from its own owner.
- What nimitz checks where the post is taken in:
  - the folder's machine is a machine of the fleet's `[machines]`, and the
    envelope names it;
  - a **member** sender is accepted only if that member may run on that
    machine (a ship or an aide whose `dirs` name it; a lead of the fleet);
    otherwise it is taken in as `process (unverified) via <machine>`;
  - the aide's **policy** is judged again on the carried sender (a
    `parent` or `fleet` policy refuses an unverified one), and the
    **rate limit** applies per sender as today, plus a cap per machine (a
    machine cannot get round the limits by naming new senders);
  - provenance in the item says `via <machine>`, and @KB's commit
    trailer `from:` carries it.
- **Recommended:** this, for 0.6.1. **Alternative (the user's call):**
  sign each envelope (Ed25519; each machine's key made on first use in its
  local state, its public key published in the data root and pinned by the
  aide's machine the first time it is seen). That stops one machine from
  posting as another, but adds a crypto dependency, and the same data root
  already holds what such a machine could rewrite anyway.

### 1.4 Answers back

- At `done` of a request whose asker is on another machine, the aide's
  machine writes the result to
  `H/<fleet>/relay/<aide machine>/replies/<asker machine>/<envelope>.json`.
- The asker's machine picks it up: `ask --wait` polls that file as it
  polls its local item today; the piggyback on that machine pushes it once
  to the asker if it is live (the late delivery of S3); `nimitz inbox
  result <aide> <envelope>` reads it with the token or as the asker.
- **Readable only by the asker:** as today, nimitz shows a result only to
  the asker or the token's holder; the file itself is readable by the
  user's own machines, as the local queue's files are by the user's own
  processes. A reply file is removed by the asker's machine once
  delivered, else after `keep_done`.

### 1.5 How long a remote `ask --wait` takes

Dropbox carries a file between two online machines in about 2 to 10 s.
Then the aide's machine must drain: within 30 s when anything runs there
(its piggyback), at worst at the aide's 15-minute cron. Then the answer
(as long as the aide takes), and the same trip back. Realistic: 30 s to
2 min on a busy aide machine; up to about 15 min on an idle one. The
default wait for a remote `ask` is 5 minutes (capped at 10, as today);
past it, `ask` prints the handle, and the answer arrives later by
`inbox result` or as a pushed message.

### 1.6 Housekeeping, Windows

- The outbox, the acks and the replies follow the aide's `keep_done`; each
  machine prunes only its own folders.
- Every write: temp file in the same folder, then `os::replace` (its
  bounded retry for Windows sharing violations); names ASCII; a Dropbox
  `(conflicted copy)` or a temp file is never read as an item.

## 2. Config without lockstep

### 2.1 Today's problem

nimitz parses fleet.toml strictly (`deny_unknown_fields`) and validates the
whole fleet. An entry or a field one machine does not know makes the whole
fleet unreadable there: hence the `min_nimitz` stamps, and 0.6.0's rule
that a snapshot reader waits for 0.6 on every machine.

### 2.2 The fix

- **Each aide in its own file:** `H/<fleet>/aides/<NAME>.toml`, one aide,
  its own `format` and `min_nimitz`. `aide new` writes there; the
  `[[aide]]` tables already in fleet.toml are still read (and `aide set`
  moves one into its own file, with the user's `--yes`).
- **A nimitz skips what is not for it, with a warning, never the fleet:**
  an aide file it cannot read (a newer format, a field it does not know)
  is skipped (`aide.skipped`: Info when the aide runs elsewhere, Error when
  it would run here); an unknown field elsewhere in a fleet file is a
  Warn, not a refusal. An aide that runs on another machine is only parsed
  for what posting needs (its name, its machine, its inbox types and
  policy); the rest is validated where it runs.
- **`min_nimitz` only for meaning here:** an aide file's `min_nimitz` is
  what a machine needs to run it or post to it; fleet.toml's stamp stays
  what fleet.toml itself needs (0.5.0 while it still holds `[[aide]]`,
  none once its aides have moved to their own files).
- **@REF:** in its own file, a reader on home-server needs 0.6.1 there
  only. The 0.6.0 rule (every machine on 0.6 before the first reader) goes.

### 2.3 Older machines

- 0.5.x and 0.6.0 read only fleet.toml: they do not see aides in
  `aides/*.toml` at all. So moving @REF (and @KB) out of fleet.toml frees
  them: their fleet loads, and they simply cannot post to those aides
  (they would have to update to 0.6.1 for that).
- A fleet.toml with `snapshot = true` in it still stops 0.5 (it cannot be
  changed retroactively). 0.6.1 never writes one there; a fleet that has
  one now (none live: HOME's @REF does not exist yet) moves it with `aide
  set`.
- 0.4.x reads fleet.toml format 1 again once no `[[aide]]` is left in it.

## 3. Formats (the maintenance test will ask)

New artifacts: `aide-file` (shared, v1), `relay` (shared: out, acks,
replies, v1), `relay-ledger` (local, v1). fleet.toml does not change
format. A migration step moves nothing by itself: moving an aide into its
own file is `aide set --own-file` (shared, the user's), planned like any
change.

## 4. Tests

- Unit: envelope write and read; the drain once (redelivered file, a
  conflicted copy, a crash between post and ledger); the trust checks
  (member on its machine, member elsewhere, unknown machine); the acks and
  pruning; aide files skipped (newer format, unknown field, another
  machine's aide).
- Two-machine sandbox: two NIMITZ_STATE folders and machine labels on one
  data root; a post on b reaches the aide on a; an `ask` from b is
  answered back on b; policy and rate across machines.
- Real-CC e2e: the same with two NIMITZ_STATE folders on one host and a
  real aide on "a": a note from "b" committed once; an `ask` from "b"
  answered.
- A 0.6.0 binary (as e2e_97 uses 0.4.0) on the same data root: its fleet
  still loads with @REF in its own file.

## 5. Decisions for the user

1. Trust: machine-scoped checks (recommended), or signed envelopes too.
2. The version: 0.6.1 as asked, or 0.7.0 (new shared formats and a
   behaviour change in posting; a minor bump says so). Recommended: 0.7.0.
3. HOME/@KB: move it from fleet.toml into `aides/KB.toml` at the release
   (then HOME's fleet.toml is format 1 again, readable by every machine).
