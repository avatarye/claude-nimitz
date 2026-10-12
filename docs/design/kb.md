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
no global attributes, literal pathspecs; of the caller's `GIT_*` only the
transport, TLS, proxy and identity ones and where the user's global and
system config are (`GIT_CONFIG_GLOBAL`, `_SYSTEM`, `_NOSYSTEM`) stay; a
changed or uncommitted `.gitattributes`/`.gitmodules` stops it. It needs
git 2.20 or later (`sync` reads `git config --worktree`).

| Command | Who | What |
| --- | --- | --- |
| `path` | any member, you | this machine's clone |
| `status` | any | `git status --porcelain` (the user's edits, conflicted copies); refused while a rebase or merge is under way or no branch is checked out |
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
URL apart, a URL rewrite in the clone's own config, a file it includes or
its worktree config (`url.*.insteadOf`, `pushInsteadOf`), or a merge ref
that is no branch, is refused; the user's global rewrites stay theirs. The
clone's git config is the user's, as git trusts it (a hand that can write
it can already run code through `core.sshCommand`): these checks keep a
stray setting from moving the notes, not a hostile one. A name
another file system reads as another (a trailing dot or space, `:`, `~`, an
invisible character) is off limits.

## 3. Who writes it: the plugin's subagents

`plugin/agents/kb-scribe.md` and `kb-ref.md` (Sonnet), with their procedures
preloaded as the skills `kb-ingest` and `kb-query` (`user-invocable: false`).
Claude Code namespaces them: a member runs `nimitz:kb-scribe` with a note's
JSON (its `sender` the member) and `nimitz:kb-ref` with a question; the
rules (v7) say so. kb-ref reads one commit: `kb head`, then `--at <sha>` on
every `show` and `grep`.

Plugin subagents ignore a `hooks:` in their frontmatter, so the limits are
a plugin-level PreToolUse hook on every tool, `nimitz hook kb-guard`
(`nimitz_core::kb_guard`), keyed on the payload's `agent_type`
(`nimitz:kb-scribe`, `nimitz:kb-ref`; any other caller gets no answer, after
a string test that keeps it cheap):

- kb-scribe: Bash only one `nimitz kb path|status|grep|hash|commit|restore`
  (no `;`, `&`, `|`, redirects, unquoted globs or comments; `$`, backticks
  and `\` only inside single quotes; no `--fleet`); Read in the KB but `.git`; Edit and Write in the KB, never what
  `kb::off_limits` names or an existing `raw/` file.
- kb-ref: Bash only one `nimitz kb path|status|head|show|grep`; Read in the
  KB but `.git`; nothing else.
- Paths are judged real (links followed as far as they exist). The KB is the
  session's fleet's (its labels judged as `caller_from_env` judges them);
  with none, only `nimitz kb path`
  passes, and `nimitz kb path` says why, so the agent stops.

It matches only the plugin's own names: a project's `.claude/agents/kb-scribe.md`
is not the plugin's agent and gets no answer. `--fleet` is denied on the line
(the KB is the session's fleet's, by labels judged as `caller_from_env`
judges them), and a write through a link that leads out, or nowhere yet, is
denied.

Claude Code's own tools for a subagent's work with its caller
(`SubagentHandback`, which delivers its result in auto mode whatever its
`tools:` say, `TodoWrite`, `ToolSearch`) get no answer: Claude Code decides
as for any session. Never an explicit allow: only auto mode's review may
allow `SubagentHandback`, and Claude Code turns a hook's allow into a deny.
The task tools stay denied (they write the session's task list). Every other tool than each agent's own is denied. (0.7.0 denied
`SubagentHandback` too, so the agents' reports never reached their callers;
fixed in 0.7.1.)

**It fails open if it cannot run.** A hook Claude Code cannot run (nimitz
missing from the session's PATH, a crash, past its 5 s) is no answer, so the
agents fall back to the session's own permission mode: in `default` or
`acceptEdits` a write outside the project still asks, but in
`bypassPermissions` or `auto` nothing stops them. `nimitz doctor` checks the
hook's entry; keep nimitz on every member's PATH.

What an agent may do the guard *allows*: a member in `default` mode in the
background writes the KB outside its own folder without a prompt (verified on
Claude Code 2.1.296, e2e #98). What it may not do is denied with the reason,
whatever the mode.
