# The fleet knowledge base (0.7.0)

Status: built in 0.7.0 (P2: settings and `nimitz kb`; P3: the subagents).
It replaces the aides of 0.5 and 0.6 (docs/design/aide.md).

## 1. Where it is: the fleet's settings

`<NIMITZ_HOME>/<fleet>/settings.json` (artifact `fleet-settings`, shared,
format 1) sits beside fleet.toml, apart from it: a key one machine does not
know never stops another from loading the fleet, so its machines need not
update in lockstep. Keys are free (`nimitz fleet settings get|set <dotted
key>`); the ones nimitz reads are checked on a write:

```json
{ "format": 1,
  "kb": { "remote": "git@github.com:avatarye/nimitz-knowledge-base.git",
          "dirs": { "home-server": "~/Github/nimitz-knowledge-base" } } }
```

- `kb.remote`: the git URL every machine clones and syncs with.
- `kb.dirs.<label>`: the clone on that machine (a label of the fleet's
  `[machines]`). `nimitz kb path` is this machine's.
- A member reads its own fleet's settings; the fleet's admiral and the user
  change them (`set`, planned: `--plan`, then `--yes`; a value is JSON, else
  text; `null` removes the key).

## 2. `nimitz kb`

Inside a member's session the fleet is the member's own (`--fleet` may only
repeat it); the user names it with `--fleet F`. Every git call is
`kb::git`: no hooks, no fsmonitor, no pager, no external diff or textconv,
no global attributes, literal pathspecs, none of the caller's `GIT_*`; a
changed or uncommitted `.gitattributes`/`.gitmodules` stops it.

| Command | Who | What |
| --- | --- | --- |
| `path` | any member, you | this machine's clone |
| `status` | any | `git status --porcelain` (the user's edits, conflicted copies) |
| `head` | any | the commit the clone is at |
| `show <path> [--at <sha>]` | any | a file as one commit holds it (never the working tree) |
| `grep <term> [<prefix>] [--at <sha>]` | any | literal, any case, at one commit; the prefix a folder or a file; `path:line:text`, at most 400 lines |
| `hash <path>` | any | sha256 of the file's bytes after its closing frontmatter `---` line (the whole file without frontmatter): a note's id |
| `commit --title T --paths A,B --pages X,Y --note <sha256> --from <sender>` | any | under the KB's lock: a note committed before prints `already committed <sha>`; else exactly those paths (literal files; never `agent/`, a CLAUDE.md, the top-level docs, `.git*`, `.claude`, `.mcp.json`, an existing `raw/` file), `ingest(auto): T` with `pages:` and the trailers `note:` and `from:`; then a sync (`--no-sync`: none), whose failure leaves the commit |
| `restore --paths A,B` | any | undo a failed ingest; what was there is kept under `NIMITZ_STATE/kb-restore/<fleet>/` |
| `sync` | any | under the lock: `pull --rebase --no-autostash` (a rebase that stops is undone, nothing pushed), then `push` |
| `clone [--plan\|--yes]` | you | `kb.remote` into `kb.dirs.<this machine>` |

The lock is `NIMITZ_STATE/locks/kb-<hash of the clone's path>.lock`: one
commit or sync at a time on this machine, whatever fleet or session asks.
Fetch and push run the user's own transport (ssh in batch mode: never a
passphrase prompt nobody would answer).

## 3. Who writes it

The `kb-scribe` and `kb-ref` subagents (P3) run in any member's session and
call only these commands; a plugin-level PreToolUse hook holds them to it.
