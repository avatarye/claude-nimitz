# claude-nimitz

Run a fleet of Claude Code background sessions as a team: an **admiral**,
squadrons led by **commodores**, and **ships** that do the long work, each in
its own folder.

`nimitz` is the command; **Admiralty** is the TUI that watches every fleet.
Only the `claude` CLI runs underneath, so a subscription login or Bedrock
both work.

Version 0.7.16 ([CHANGELOG](CHANGELOG.md): read "Upgrading" first). Status: in testing (Claude Code 2.1.288 - 2.1.314).

## Install

Needs Rust (cargo) and Claude Code; Herdr for panes (status, config and ships
work without it).

```sh
git clone -b dev https://github.com/avatarye/claude-nimitz && cd claude-nimitz
cargo install --path crates/nimitz       # the CLI and the hook binary
cargo install --path crates/admiralty    # the TUI
claude plugin marketplace add "$PWD" && claude plugin install nimitz@claude-nimitz
nimitz init --label laptop               # data root and this machine's name
nimitz doctor                            # run again until: [OK] doctor: ready
```

In Herdr, `herdr plugin link <repo>/herdr-plugin` and bind a key to
`admiralty.toggle` to open Admiralty as an overlay.

To upgrade: install both binaries again, run `claude plugin marketplace update
claude-nimitz && claude plugin update nimitz@claude-nimitz`, then `nimitz
doctor`. The CHANGELOG's "Upgrading" says what else a release needs.

## Your first fleet

```sh
nimitz fleet new HOME --yes                               # a fleet and its admiral
nimitz squadron new HOME/WEB --brief "the web app" --yes  # a squadron and its commodore
nimitz ship new HOME/WEB/API --dir ~/code/project \
  --prompt "keep the test suite green" --yes              # a ship, in one folder
nimitz start HOME/... HOME/WEB/API --yes                  # launch the leads and the ship
nimitz status                                             # what runs, what waits on you
nimitz open HOME/WEB/API                                  # its pane; ctrl-z detaches
```

Leave out `--yes` to read the plan and answer y/N; `--plan` only shows it.
Once a fleet runs, its admiral and commodores do most fleet work themselves:
ask them in plain words. To try it all without touching your setup:
`nimitz sandbox create /tmp/nz --herdr && eval "$(nimitz sandbox env /tmp/nz)"`.

## Most used

A path is `F` (an admiral), `F/SQ` (a commodore) or `F/SQ/SHIP` (a ship);
`F/...` is everything under one, `//...` every fleet.

| Command | Example |
| --- | --- |
| `status` | `nimitz status HOME/...`: the members, their state, their issues |
| `start` | `nimitz start HOME/WEB/API`: bring members up (the jobs, no pane) |
| `stop` | `nimitz stop HOME/WEB/...`: stop members, ships first |
| `restart` | `nimitz restart HOME/WEB/API`: relaunch, same conversation |
| `muster` | `nimitz muster`: after a reboot, a Claude Code update or a Herdr closure |
| `open` | `nimitz open HOME/WEB`: a pane for the member, focused |
| `signal` | `nimitz signal HOME/WEB/API "run the tests"`: typed into its tab |
| `squadron new\|set` | `nimitz squadron set HOME/WEB --effort high`: a squadron, its ships' defaults |
| `ship new\|set` | `nimitz ship set HOME/WEB/API --prompt-file brief.md`: folder, prompt, mode, effort |

## Less used

| Command | What it does |
| --- | --- |
| `fleet new\|set\|rm` | A fleet: `nimitz fleet new WORK --yes` |
| `machine add\|rm\|set` | Run a fleet on another machine too; `machine set laptop --provider anthropic`: the claude and API provider a machine's members must run on |
| `settings get\|set` | The fleet's `settings.json`: `nimitz settings get kb.remote` |
| `harness list\|add\|rm\|push` | Rules, agents, skills and settings every member of a fleet gets |
| `kb ...` | The fleet's knowledge base, a git repo every machine clones |
| `check` | `nimitz check HOME/...`: every problem, each with its fix |
| `doctor` | This machine's install: hooks, plugin, trust, versions |
| `init` | Data root, machine label, trust; then `doctor` |
| `trust` | `nimitz trust ~/code/project`: let Claude Code run that folder's own settings |
| `rules` | The role rules a member gets at session start |
| `migrate` | The format steps due after an update (`--shared`: every machine's) |
| `sweep` | `nimitz sweep HOME/...`: drop stale rows and copies of members |
| `adopt` | Make a stray session in a fleet folder one member's session |
| `rm` | Remove job rows (never transcripts) |
| `attach` | Attach this terminal to one member |
| `close` | Close the panes that show members; the jobs run on |
| `layout` | Herdr tabs for the leads (`--ships`: and each ship) |
| `sandbox` | A fake world for tests and demos |

Each command's full usage: `nimitz help <command>` (or `nimitz <command>
--help`); `nimitz help` lists them all.

## More

- [docs/usage.md](docs/usage.md): paths in full, muster, the two rules,
  Admiralty's keys, fixes for common problems, notes for maintainers.
- [docs/design/](docs/design/): how it works (start with
  [cli-ux.md](docs/design/cli-ux.md)).

Note: "Claude" is an Anthropic trademark. This name is for private use; check
Anthropic's trademark guidelines before any public release.
