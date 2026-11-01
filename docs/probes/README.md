# P0 probes

Requirements and how to run: `P0-requirements.md`. Script: `../../probes/p0_probe.py`.

## Comparison across builds

Both columns use the revised script. Bold marks a result that differs between the
builds. On a wrapper build (a corporate machine whose `claude` on PATH is a wrapper
that adds its own `--settings` to every call), that wrapper is the only difference
found. The public results: `findings-public-2.1.288.md`.

| Probe | Wrapper build 2.1.288, Bedrock | Public 2.1.288, subscription |
| --- | --- | --- |
| 0. `--bg` in untrusted folder | refused; **home folder trusted as parent** | refused; **home-folder trust not saved, probe folder trusted on its own** |
| 1. `--bg --session-id` | ignored, warning | ignored, warning |
| 2a. `--bg --resume <unknown uuid>` | session never runs (SessionEnd only), row `failed` | session never runs (SessionEnd only), row `failed`; cannot choose an id |
| 2b. resume while live, no flags | copy with new id; no hooks (labels lost) | copy with new id; copy fires no hooks (labels lost) |
| 2. resume while live, all flags | copy with new id, `source: fork`; copies stay `done`/idle | copy with new id, `source: fork`; copies stay `done`/idle |
| 2c1. resume after stop, no flags | **through the PATH wrapper: copy, labels lost (the wrapper adds flags)**; through the native binary: same id, `source: resume`, labels kept | same id, `source: resume`, saved name, mode and labels restored |
| 2c2. resume after stop, `--name` only | copy with new id; no hooks (labels lost) | copy with new id; copy fires no hooks (labels lost) |
| 2c3. resume after stop, all flags | new id, `source: fork`; old row stays | new id, `source: fork`; old row stays |
| 3. labels reach hooks in `--bg` | yes; SessionStart, UserPromptSubmit, Stop, SessionEnd | yes; same hooks (Notification only when idle long enough) |
| 4. hook `sessionTitle` changes the address | no (title only) | no (title only) |
| 5. two live `--bg` with one name | allowed | allowed |
| P0.6 labels reach Bash-tool processes | yes | yes (2.1.289) |
| P0.7 PreToolUse sees and can deny `SendMessage` | yes (`to`/`recipient`/`message`; deny blocks) | yes (same fields; deny blocks; **hook input adds `scratchpad_dir`**) |
| P0.8 interactive `claude -c` keeps the id | yes (`startup`, then `resume`) | yes (`startup`, then `resume`) |

## Rule for nimitz

Resume a stopped ship with the **native** binary and **no flags**, from the ship's
folder: `<native claude> --bg --resume <session id> [prompt]`. Both builds then keep
the same session id and its saved name, permission mode and labels. Never resume
a live ship.

## Found by the adversarial tester, 2026-10-04 (real wrapper build 2.1.288)

`--bg --resume <id>` with flags (`--name`, `--settings`, ...) while the session's
job row still exists starts a **copy with a new id and no conversation history**
(verified twice, with code words PINEAPPLE and MANGO). After `claude rm <row>`
(native), the same command **continues <id> itself**: same id, its history, and
the new name and labels. So nimitz relabels by stop, `rm`, then resume with every
flag.

## P0.9: resume with flags (found by the adversarial tester, 2026-10-04)

| Case | Wrapper build 2.1.288.1000 |
| --- | --- |
| `--bg --resume <id>` with flags while the job row exists, with a prompt | new id; history kept (the first turn writes the fork's transcript) |
| the same with no prompt, stopped before its first turn | new id; history LOST (no transcript was written) |
| `claude rm <row>` first, then `--bg --resume <id>` with flags | same id; new name and labels; history kept |

nimitz uses the last row for every relabel (`probes/p0_relabel.py`).
