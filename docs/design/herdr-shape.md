# Herdr test isolation — findings (herdr 0.9.3, 2026-10-07)

## Verdict

**Separate test server: YES in principle, NOT exercised on this machine.**

- Herdr has first-class named sessions. Every named session gets its own directory, its own
  two unix sockets and its own log/state files. The vendor skill file says so explicitly:
  *"Never kill the main Herdr process. Use named test sessions for experiments that need an
  isolated server."* (`herdr --skill`, line 213).
- Everything Herdr touches hangs off `$HOME/.config/herdr` (hard-coded `.config/` +
  `home_dir()`, not XDG — which is why private `XDG_*` dirs changed nothing earlier) and
  `$HOME/.local/state/herdr`. `/run/user/<uid>` is NOT used for sockets.
- A test server is therefore reachable only through the socket path a test chooses; with a
  private `HOME` (plus `--session`) the test's CLI resolves a path that does not exist in
  the user's tree, and the user's CLI never looks in the test's tree.
- Phase 2/3 (actually running a server) was NOT done: the mandated precondition — a user +
  mount namespace that physically hides the live sockets — is unavailable here
  (see "Why not run"). Nothing beyond `--help`, `--version`, `--skill`, `--default-config`
  was executed. No Herdr command that opens a socket was run.

## Exact recipe (for when isolation is available, or for the test harness itself)

```
export HOME=$TESTDIR/home            # all herdr state now under $TESTDIR/home/.config/herdr
mkdir -p $HOME
export HERDR_SESSION=nimitz-test     # or pass --session nimitz-test on every command
export HERDR_CONFIG_PATH=$TESTDIR/config.toml   # optional; omit to use defaults
herdr --session nimitz-test server &             # headless server, detached
#  -> sockets: $HOME/.config/herdr/sessions/nimitz-test/herdr.sock (API)
#              $HOME/.config/herdr/sessions/nimitz-test/herdr-client.sock (TUI attach)
#  -> logs:    .../sessions/nimitz-test/herdr-server.log, session.json, session-snapshots/
herdr --session nimitz-test workspace create --label W --cwd /tmp --no-focus
herdr --session nimitz-test tab create --workspace w1 --label T --cwd /tmp --no-focus
herdr --session nimitz-test pane run w1:p1 "sleep 30"
herdr --session nimitz-test api snapshot          # whole shape, one JSON
herdr --session nimitz-test pane process-info --pane w1:p1
herdr --session nimitz-test server stop           # or: herdr session stop nimitz-test
```

Defence in depth for the harness, all cheap:
1. Private `HOME` (the real lever: changes the socket directory).
2. `--session <unique-name>` on every call AND `HERDR_SESSION` in the env — even if a test
   leaks the real `HOME`, the name never collides with `NIMITZ`/`nikitz`/default.
3. Assert before any call that `$HOME/.config/herdr/sessions/<name>/herdr.sock` is absent,
   and after start that it exists and that `$HOME` is not `~`.
4. Unset `HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_WORKSPACE_ID`, `HERDR_TAB_ID`,
   `HERDR_SOCKET_PATH`, `HERDR_CLIENT_SOCKET_PATH` in the test env: these are injected into
   panes by the live server and are what "inherited session and socket context" means in the
   skill — a test launched from inside a live Herdr pane would otherwise inherit NIMITZ.

Residual risk of the recipe without a namespace: it relies on Herdr honouring `HOME`.
Evidence it does: `home directory is not set; cannot locate home directory` string (a
`home_dir()` call, which on Linux reads `$HOME` first), `.config/` and `.local/state/`
relative strings in `src/config/io.rs`, and the live logs showing sockets under
`~/.config/herdr/...`. Not proven by execution.

## Why phase 2/3 was not run (isolation unavailable)

| Route | Result |
|---|---|
| `unshare -r -m` | `write failed /proc/self/uid_map: Operation not permitted` |
| `sysctl kernel.apparmor_restrict_unprivileged_userns` | `= 1` (Ubuntu 24.04 default) |
| `aa-exec -p unprivileged_userns -- unshare -r -m` | `unshare failed: Operation not permitted` (profile denies capabilities, so no mount ns) |
| `bwrap` | not installed |
| `systemd-run --user -p TemporaryFileSystem=/run/user/1000` | exit 226 = EXIT_NAMESPACE |
| `docker` | binary present, socket `root:docker` 660, user groups = `avatarye sudo` only -> permission denied |
| `podman`, `firejail`, `proot` | not installed |
| `sudo -n` | password required |

Any one of these, done once by the user, unblocks a real isolated run:
- `sudo apt install bubblewrap` (Ubuntu ships an AppArmor profile for bwrap that permits userns), or
- `sudo sysctl kernel.apparmor_restrict_unprivileged_userns=0` (session-wide, reversible), or
- `sudo usermod -aG docker avatarye` (herdr is a static-pie binary, so a bare `ubuntu:24.04`
  container with the binary bind-mounted read-only would run it), or
- run on another host (e.g. the Ubuntu box `ssh ubuntu`): unix sockets cannot cross hosts.

## How Herdr resolves things (static evidence)

- Socket names: `herdr.sock` (JSON API) and `herdr-client.sock` (TUI/attach protocol), in
  `session_dir`. Strings: `--sessionrunningsocket_pathsession_dirherdr-client.sock`,
  `..herdr.sock`. Confirmed by live logs:
  `api server listening path=~/.config/herdr/sessions/NIMITZ/herdr.sock` and
  the default session at `~/.config/herdr/herdr.sock`.
- Session selection: `--session <name>` flag or `HERDR_SESSION` env. Name rule:
  ASCII letters, digits, `.`, `_`, `-`; not `.`/`..`; default session cannot be deleted.
- Config: `$HOME/.config/herdr/config.toml`, override `HERDR_CONFIG_PATH`.
- State: `$HOME/.local/state/herdr/{agent-detection,client-shell,plugins}`.
- `HERDR_HOME` exists only in the Windows PowerShell installer string.
- `HERDR_SOCKET_PATH` / `HERDR_CLIENT_SOCKET_PATH`: exported by the server into panes and
  used by the live-handoff path (`src/server/handoff.rs`) and integrations; I found no
  evidence the CLI accepts them as a *target override* — the supported override is
  `--session`. Treat them as inherited context to scrub, not as a knob.
- Headless server: `herdr server` (advanced command; prints "herdr server running; you can
  use any herdr CLI command in another terminal"). Refuses with "herdr server is already
  running (socket busy at ...)" if the socket is live.
- Live state on this box: two servers, `NIMITZ` (pid 2857518, started Oct 5 02:44:37) and
  `nikitz` (pid 418777, Oct 5 06:31:22); one attached client (pid 602310). Config dir is a
  symlink into Dropbox: `~/.config/herdr/config.toml -> /mnt/data/shared/.../herdr/config.toml`.

## Shape-reading commands (for nimitz restore)

All return JSON (`.result...`). API protocol 22; schema embedded in the binary (extracted to
`herdr-api-schema-0.json`; `herdr api schema` prints the same bundle). CLI -> API method:

| CLI | API method | Result |
|---|---|---|
| `herdr api snapshot` | `session.snapshot` | **everything in one call**: `workspaces[]`, `tabs[]`, `panes[]`, `agents[]`, `layouts[]`, focused ids |
| `herdr workspace list` | `workspace.list` | `workspaces[]` of WorkspaceInfo |
| `herdr tab list --workspace w1` | `tab.list` | `tabs[]` of TabInfo |
| `herdr pane list --workspace w1` | `pane.list` | `panes[]` of PaneInfo |
| `herdr pane get --pane w1:p1` | `pane.get` | PaneInfo |
| `herdr pane process-info --pane w1:p1` | `pane.process_info` | PaneProcessInfo |
| `herdr agent list` / `agent get <name|pane>` | `agent.list` / `agent.get` | AgentInfo |
| (no CLI verb seen) | `layout.export` (`tab_id`/`pane_id`) | LayoutDescription: split tree whose leaves carry `command: array|null, cwd, env, label, pane_id` |

Field shapes (from the schema; sample output could not be captured without running a server):

- **WorkspaceInfo**: `workspace_id, label, number, active_tab_id, tab_count, pane_count, focused, agent_status, worktree{repo_root, checkout_path, ...}|null, tokens`
- **TabInfo**: `tab_id, workspace_id, label, number, pane_count, focused, agent_status`
- **PaneInfo**: `pane_id, tab_id, workspace_id, terminal_id, label, title, terminal_title,
  terminal_title_stripped, cwd, foreground_cwd, agent (kind, e.g. "claude"), display_agent,
  agent_status (idle|working|blocked|done|unknown), agent_session{agent, kind: id|path,
  source, value}|null, restore_error, focused, revision, scroll, state_labels, tokens`
- **PaneProcessInfo**: `pane_id, shell_pid, tty, foreground_process_group_id,
  foreground_processes[]{pid, name, argv0, argv[], cmdline, cwd}`

### Can the pane's foreground command (e.g. `claude attach <job>`) be read?

**Yes, two ways:**
1. `herdr pane process-info --pane <id>` -> `foreground_processes[].argv` / `.cmdline` is the
   live foreground process group of the pane's pty, read from the OS, so `claude attach
   9aacf927` is visible verbatim while it runs. This is live-only (gone after a server restart).
2. `agent_session` on PaneInfo / snapshot / persisted `session.json`: for recognised agents
   Herdr's integration reports the native session ref. Real example from the live file
   (read-only): `{"source":"herdr:claude","agent":"claude","kind":"id","value":"f29c13aa-..."}`
   — that is the Claude Code session id, and `[session] resume_agents_on_restore` uses it to
   relaunch the agent after a Herdr restart. The persisted `session.json` keeps per pane only
   `cwd` + `agent_session`; it does NOT persist the shell command line.

Implication for nimitz restore: at save time take `api snapshot` (labels, cwd, agent kind,
agent_session) plus `pane process-info` per pane (argv) — the second is the only place the
literal `claude attach <job>` command lives, and it is only there while the process runs.
`layout.export`'s `command` leaf field is the natural input to `layout.apply` for re-creating
a tab with commands.

## Cleanup confirmation

No test server was started, so nothing to clean. Live state verified identical before and
after: `ps -o pid,lstart,cmd -C herdr` -> pids 418777 (Oct 5 06:31:22), 2857518 (Oct 5
02:44:37), 602310 client (Oct 6 23:56:56) unchanged; socket mtimes in
`~/.config/herdr/sessions/{NIMITZ,nikitz}/` unchanged (Oct 5). Nothing under
`~/.config/herdr` or `~/.local/state/herdr` was written. Only `--help`, `--version`,
`--skill`, `--default-config` and `<group> --help` were executed (the group `--help` output
is the global help; it does not connect).

## Artifacts (this directory)

- `herdr-strings.txt` — `strings -n 6` of the binary
- `herdr-api-schema-0.json` — embedded API schema (protocol 22), `schema-digest.txt`,
  `schema-structs.txt` — method list and struct dumps
- `herdr-skill.md` — `herdr --skill`; `herdr-default-config.toml` — `herdr --default-config`
- `leaf-help.txt` — proof that `herdr <group> <cmd> --help` prints the global help only
