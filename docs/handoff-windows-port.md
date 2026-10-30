# Handoff: the `windows-port` branch

For the agent that reviews and merges this branch on Ubuntu. Written
2026-10-10 on the Windows machine `home-windows` (Windows 11, Claude Code 2.1.296, Herdr 0.9.3, rustc 1.99).

## TLDR

- nimitz, admiralty and the Herdr plugin build and run on native Windows. The
  branch is 6 commits on `66f88bc` (`main` has not moved since), 56 files.
- On Windows the whole workspace suite passes (over 600 tests; 26 Unix-only
  tests are gated, below). Linux and macOS were **type-checked only**
  (`cargo check --workspace --tests --target x86_64-unknown-linux-gnu` and
  `aarch64-apple-darwin`, no warnings). **No test has run on Unix yet: the
  run below is the first.**
- Four changes alter Unix behaviour on purpose (section "Unix behaviour
  changes"). Each needs a yes or no from you.
- Verified against real Claude Code on Windows: a nimitz inbox line reached a
  live background session through its named pipe and the session replied.

## What to run on Ubuntu

```sh
git fetch origin && git checkout windows-port
cargo test --workspace                      # the first Unix run of this branch
herdr-plugin/tests/check.sh                 # needs admiralty: ADMIRALTY_BIN=target/debug/admiralty
uv run --with pyte python herdr-plugin/tests/e2e_isolated.py   # private Herdr server
NIMITZ_E2E=1 cargo test -p nimitz --test e2e   # opt-in, real Claude Code (as before)
```

Then reinstall (`cargo install --path crates/nimitz`, `--path
crates/admiralty`) and smoke-test the Herdr overlay (`alt+shift+f`): the
plugin's commands changed (below).

## Commits

| Commit | What |
| --- | --- |
| `7dd6984` | Build and run on Windows: the `os` module, std file locks, the named-pipe inbox, the hook's walk to its Claude Code process, PATHEXT-aware `which`, sandbox shims, Unix-only tests gated |
| `c4bcac5` | Trust keys in one form, case-insensitive fleet folders, Herdr config under `%APPDATA%`, test harness fixes; suite green on Windows |
| `11f4895` | Herdr plugin: `admiralty herdr toggle\|watch` replaces `bin/toggle.sh` and `bin/watch.sh` |
| `3c5c4ba` | Detect Herdr from a `cmd.exe` pane (env names are case-insensitive on Windows) |
| `d0f1b20` | Trust agrees with Claude Code on a linked data root and on case |
| `0145dfc` | On Windows the trust fix starts `claude` from PowerShell, spelled exactly |

## Design

All platform code is in one new module, `crates/nimitz-core/src/os.rs`: one
function per fact, one body per platform, callers platform-free. Read it
first.

| Need | Unix (unchanged) | Windows |
| --- | --- | --- |
| parent pid, ancestors | `/proc/<pid>/stat` (Linux) | Toolhelp32 snapshot; a parent that started after the child is a reused pid and is refused |
| pid alive | `kill(pid, 0)` | `OpenProcess` + `GetExitCodeProcess`; access denied counts as alive |
| executable of a pid | `/proc/<pid>/exe`, `proc_pidpath` | `QueryFullProcessImageNameW` |
| host name | `gethostname` | `GetComputerNameExW` (DNS host name) |
| file identity (events rotation) | inode | volume file index |
| writable | `access(W_OK)` | probe file in a folder, open for append on a file |
| detached child (`__notify`) | `setsid` in `pre_exec` | `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP`, plus breakaway when the job allows it |
| file locks | `File::lock` / `try_lock` (std, flock underneath) | the same (LockFileEx); nimitz locks only dedicated `.lock` files |
| env as a map | as is | names upper-cased (`os::env_vars`) |
| editor | `sh -c "$EDITOR \"$1\""` | split on spaces, program found through PATHEXT; default `notepad` |
| sandbox fakes (`claude`, `herdr`) | `#!/bin/sh` script | hard link to the test binary plus `<name>.shim.json` with the prefix arguments (`os::write_shim`, `os::args`) |
| canonical path | `canonicalize` | the same without the `\\?\` prefix (`os::canonical`) |

`flock` calls became std locks on every platform (same semantics on Unix).

### Findings from live probes (Windows, Claude Code 2.1.296)

These drove the code; each was observed, not assumed.

1. **Inbox.** A session's inbox is a named pipe (`\\.\pipe\LOCAL\cc-msg-<hash>`,
   `messagingSocketPath` in `sessions/<pid>.json`). Claude Code closes any
   connection whose first line is not `{"type":"auth","token":...}`. The
   token for another session is `peerToken` in `sessions/<pid>.<hash>.key`
   (its `procStartFt` matches the session file's `procStart`; nimitz checks
   that, against pid reuse). nimitz also checks `GetNamedPipeServerProcessId`
   equals the session's pid. A pipe cannot half close: the line counts as
   posted after `FlushFileBuffers` while the pipe stays open; a refused auth
   closes it. See `inbox.rs` (`post`, `peer_token`, `win::post`) and its
   Windows tests, plus the opt-in `a_real_session_takes_the_line`.
2. **Hook parent.** A hook in the plugin's argv form runs directly under
   `claude.exe`. A hook written as one command string runs as
   `bash.exe` under `bash.exe` under `claude.exe` (Git Bash). `claude_pid_through`
   walks up through shells only (at most 4), so a session started from
   another session's Bash tool still never binds to the outer one. Unix keeps
   the old one-step rule (`is_shell` is `false` there).
3. **Hook jobs.** Claude Code runs hooks in a job object with kill-on-close
   and silent breakaway allowed; a detached child outlived the hook.
4. **Trust.** Claude Code resolves links before looking up trust, looks the
   folder up in its on-disk case, and compares keys **with case**. Its
   trust dialog stores the folder as the shell spelled it, and a `cmd.exe`
   with clink/zoxide hands it `c:\...` however it is typed. So a trust
   accepted there (`c:/Dropbox/...`) never covers a background launch
   (`C:\Dropbox\...`). PowerShell's `Set-Location` keeps the case.
5. **Herdr plugins** run commands as an argv with no shell, and one command per
   action id (two `toggle` actions with disjoint `platforms` are refused:
   `duplicate_plugin_action_id`). A relative extension-less program is run
   as-is (`bin/toggle` does not find `bin/toggle.cmd`).
6. **Environment.** `cmd.exe` passes `PATH` as `Path`; std's `env::vars()`
   keeps that spelling, so an exact `"PATH"` lookup found nothing and nimitz
   fell back to no multiplexer inside a Herdr pane.

## Unix behaviour changes (decide each)

1. **Herdr plugin** (`11f4895`). `herdr-plugin.toml` now runs `admiralty herdr
   toggle` and `admiralty herdr watch` on every platform; `bin/toggle.sh` and
   `bin/watch.sh` are deleted. Their logic is in
   `crates/admiralty/src/herdr_plugin.rs` (unit tested; `check.sh` step 4 runs
   the binary against `tests/fake-herdr.sh`). Lost: the `~/.cargo/bin`
   fallback and `ADMIRALTY_BIN` for the plugin; `admiralty` must be on the
   Herdr server's PATH. Kept: the state-dir pane id, the label check, the
   `args` file, `ADMIRALTY_*` and `ADMIRALTY_WATCH_*` variables.
   Why not per-platform scripts: probe finding 5.
2. **Trust inside the data root** (`d0f1b20`). Folders inside `NIMITZ_HOME`
   used to count as trusted unconditionally. Now they count only when the
   data root's real path is trusted; otherwise `check` reports
   `folder.untrusted` with the data root as the fix. Reason: on Windows this
   shortcut said "trusted" while `claude --bg` refused (finding 4). The
   sandbox's trust file now lists its data root, as `nimitz init` has the
   user do.
3. **Trust ancestors** (`d0f1b20`). A parent as given counts only when it
   holds the real folder (`real.starts_with(canon(parent))`). For
   `/tmp/x` → `/private/tmp/x` nothing changes; a trusted home no longer
   covers `~/.claude-nimitz` when that is a link elsewhere (finding 4).
4. **Trust key form** (`c4bcac5`, `d0f1b20`). Keys and folders are compared
   without trailing separators (Unix: the old exact-or-`/` match, in one
   function `env::trust_key`); on Windows also with `/` for `\`, case kept.

Smaller, Unix-visible but cosmetic: `restore::SHELLS` gains `pwsh`,
`powershell`, `cmd`; `program_name` strips `.exe` and splits on `\`;
`config::is_absolute_dir` accepts `C:\x`, `C:/x`, `~\x`, `\\server` (a fleet
config holds folders for every machine); a mis-cased fleet folder is found
under its name on disk (`os::name_on_disk`, also right on macOS's
case-insensitive default).

## Tests gated to Unix

`crates/nimitz/tests/e2e.rs` (whole file, `#![cfg(unix)]`: its real-Claude
wrappers are `#!/bin/sh`). And, per test, because they need Unix permission
modes, `sh` scripts as fake programs, signals or Unix sockets:

- `nimitz-core`: `compat::installed_reads_the_native_binary`;
  `inbox::{a_line_reaches_our_own_listener_framed_from_nimitz,
  a_listener_that_is_not_the_session_is_never_used,
  a_record_counts_only_for_its_own_process_and_many_hooks_keep_theirs}`;
  `registry::{a_read_only_registry_is_a_clear_error,
  a_registry_that_cannot_be_read_is_an_error_for_update_and_fresh_for_a_read}`;
  `tests/identity_matrix::a_registry_that_cannot_be_read_is_never_written_over_by_the_hook`
- `nimitz`: `crud::the_editor_returns_what_it_saved`; `tests/cli::{a_closed_pipe_never_panics,
  a_notice_and_a_signal_go_to_the_leads_inbox_not_its_pane,
  an_older_claude_code_gets_the_typing_path,
  the_users_nimitz_outside_herdr_uses_the_recorded_server,
  with_no_multiplexer_a_signal_goes_to_the_inbox_as_a_request}`;
  `tests/restore::with_several_herdr_servers_status_reads_only_the_recorded_one`
- `admiralty`: `tests/sandbox::{bug40_the_first_look_is_quick_and_the_full_one_follows,
  with_several_herdr_servers_enter_and_j_k_jump_to_the_attach_pane}`

They still run on Linux. Integration tests also changed shape (no behaviour
change intended): paths built per part (`.join("a").join("b")`),
`os::canonical` for `canonicalize`, `.exe` names, JSON-escaped paths,
tolerant matches (`claude.exe stop`), a larger hook-time budget on Windows.

## Known gaps

1. **Claude Code version of a process on an npm install (Windows).**
   `compat::process_version` reads the version from the executable's path;
   npm's `node_modules/@anthropic-ai/claude-code/bin/claude.exe` has none.
   Workers fall back to their session file's `version`; an attach client's
   version reads as unknown, so restore cannot tell an old attach window.
2. **`e2e.rs` on Windows**: its wrappers need porting (shims, or `.cmd`).
3. **`check.sh` on Windows** would link into the real Herdr (Herdr there keeps
   its config under `%APPDATA%` and ignores `XDG_CONFIG_HOME`); run only the
   unit tests there.
4. **Tested range.** Windows ran Claude Code 2.1.296; `docs/compat.toml` tests
   up to 2.1.294 (a warning, launches go ahead).
5. Production paths still join some parts with `/` (`.claude/CLAUDE.md`
   fixed; others remain). Windows accepts mixed separators; messages may show
   them.

## Setting up a Windows machine

```powershell
cargo install --path crates/nimitz; cargo install --path crates/admiralty
claude plugin marketplace add C:\Projects\Github\claude-nimitz
claude plugin install nimitz@claude-nimitz
New-Item -ItemType SymbolicLink -Path "$env:USERPROFILE\.claude-nimitz" -Target C:\Dropbox\claude-nimitz
nimitz init --label home-windows
# trust the data root spelled exactly (finding 4), accept, /exit:
powershell -NoProfile -Command "Set-Location -LiteralPath 'C:\Dropbox\claude-nimitz'; claude"
herdr plugin link C:\Projects\Github\claude-nimitz\herdr-plugin
nimitz doctor
```

Herdr's config is `%APPDATA%\herdr\config.toml`; the `admiralty.toggle`
binding (`prefix+f`, `alt+shift+f`) is already in the shared key region of all
three per-OS copies in `Configs/{macos,ubuntu,windows}/herdr/config.toml`.
