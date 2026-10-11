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
  `[machines]`; an absolute folder, or `~/...`). `nimitz kb path` is this
  machine's, and only while it is a clone of `kb.remote` (its `origin`): the
  settings never point nimitz at another repo. A label with a dot is set as
  JSON: `set kb.dirs '{"my.host": "~/kb"}'`.
- A member reads its own fleet's settings; the fleet's admiral and the user
  change them (`set`, planned: `--plan`, then `--yes`; a value is JSON, else
  text; `null` removes the key). `kb.*` is the user's alone: it names the
  repo every member commits to and pushes from.

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
| `grep <term> [<prefix>] [--at <sha>]` | any | literal, any case, at one commit (binary files skipped); the prefix a folder or a file; `path:line:text`, at most 400 lines |
| `hash <path>` | any | sha256 of the file's body after its closing frontmatter `---` line (the whole file without frontmatter; a BOM dropped, CRLF read as LF; an empty body is no note): a note's id |
| `commit --title T --paths A,B --pages X,Y --note <sha256> --from <sender>` | any | under the KB's lock, refused while a rebase or merge is under way or no branch is checked out: a note this clone committed before prints `already committed <sha>` (once per clone: two machines that ingest one note before they sync both commit it); else exactly those paths (literal files; never `agent/`, a CLAUDE.md, the top-level docs, `.git*`, `.claude`, `.mcp.json`, an existing `raw/` file), `ingest(auto): T` with `pages:` and the trailers `note:` and `from:`; then a sync (`--no-sync`: none), whose failure leaves the commit |
| `restore --paths A,B` | any | under the lock: undo a failed ingest; what was there is kept under `NIMITZ_STATE/kb-restore/<fleet>/` |
| `sync` | any | under the lock: `pull --rebase --no-autostash` (a rebase that stops is undone, nothing pushed; uncommitted edits to tracked files stop it), then `push`; a branch with no upstream is said, never an OK |
| `clone [--plan\|--yes]` | you | `kb.remote` into `kb.dirs.<this machine>` |

The lock is `NIMITZ_STATE/locks/kb-<hash of the clone's path>.lock`: one
commit or sync at a time on this machine, whatever fleet or session asks.
Fetch and push run the user's own transport: ssh in batch mode with a 20 s
connect timeout (never a prompt nobody would answer, never a hang that
holds the lock), unless the user set an ssh command (`GIT_SSH*`,
`core.sshCommand`), and an https transfer that stalls (under 1000 B/s for
60 s; git 2.31 or later) ends. `sync` pulls and pushes only `origin` and the branch's own branch there, by
name (no `pushRemote` or `pushDefault`), origin being the clone of
`kb.remote`: a branch that tracks another remote, an `origin` URL or push
URL apart, a URL rewrite in the clone's own config (`url.*.insteadOf`,
`pushInsteadOf`), or a merge ref that is no branch, is refused. (A setup
that fetches over https and pushes over ssh by `pushInsteadOf` is refused
too: use one URL.) Of the caller's `GIT_*`, only the transport, TLS, proxy
and identity ones stay. A name
another file system reads as another (a trailing dot or space, `:`, `~`, an
invisible character) is off limits.

## 3. Who writes it

The `kb-scribe` and `kb-ref` subagents (P3) run in any member's session and
call only these commands; a plugin-level PreToolUse hook holds them to it.
