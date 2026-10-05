# claude-nimitz

A fleet system for Claude Code, built on the `claude` CLI only, so it works with
a Claude subscription login and with Bedrock.

- `nimitz`: the command that builds and runs fleets, squadrons and ships.
- **Admiralty**: the TUI where you watch and control every fleet.
- A Claude Code plugin: the fleet skills and the hooks that report state and
  tell a commodore at once when one of its ships or tenders waits on you.

Status: in testing (Claude Code 2.1.288 - 2.1.289).

## Install for testing

Needs Rust (cargo) and Claude Code. Herdr is needed to run admirals and
commodores; status, config and ships work without it.

```sh
git clone -b dev https://github.com/avatarye/claude-nimitz && cd claude-nimitz
cargo build --release
cargo install --path crates/nimitz       # nimitz: the CLI and the hook binary
cargo install --path crates/admiralty    # admiralty: the TUI
claude plugin marketplace add "$PWD" && claude plugin install nimitz@claude-nimitz
nimitz init --label <this-machine>       # data root and machine label
nimitz doctor                            # hooks, trust, Claude Code versions; --fix sets crossSessionInbound
```

`nimitz init` asks you to run `claude` once in the data root
(`~/.claude-nimitz`) to accept the trust prompt. Run `nimitz doctor` again until
it says `[OK] doctor: ready`. To try nimitz without touching your setup, use a
sandbox: `nimitz sandbox create /tmp/nz --herdr && eval "$(nimitz sandbox env /tmp/nz)"`.

## Documents

| Where | What |
| --- | --- |
| `docs/design/plan-v3.md` | The plan: decisions, vocabulary, setup and CRUD, adapters, compatibility, phases. |
| `docs/design/cc-control-model.md` | How `nimitz` controls Claude Code sessions along the chain of command. |
| `docs/design/plan-v2.md` | Previous plan (kept for history). |
| `docs/design/review-identity-and-status.md` | How Claude Code, the old fleet skill and the old watch identify agents; the gaps. |
| `docs/probes/` | P0 probes: requirements, findings per Claude Code build, comparison. |
| `probes/p0_probe.py` | The probe script. |

Vocabulary: a **fleet** has one **admiral**; a fleet has **squadrons**, each
commanded by a **commodore**; a squadron has **ships** (background agents);
a commodore can send out **tenders** (helpers); sessions carry **crew**
(sub-agents). Admiralty is the office where the admirals work.

Note: "Claude" is an Anthropic trademark. This name is for private use; check
Anthropic's trademark guidelines before any public release.
