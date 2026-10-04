# claude-nimitz

A fleet system for Claude Code, built on the `claude` CLI only, so it works with
a Claude subscription login and with Bedrock.

- `nimitz`: the command that builds and runs fleets, squadrons and ships.
- **Admiralty**: the TUI where you watch and control every fleet.
- A Claude Code plugin: the fleet skills and the hooks that report state.

Status: design and probes. Nothing to install yet.

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
