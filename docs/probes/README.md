# P0 probes

Requirements and how to run: `P0-requirements.md`. Script: `../../probes/p0_probe.py`.

## Comparison across builds

Both columns use the revised script. Bold marks a result that differs between the
builds. On ACME, `claude` on PATH is a wrapper that adds its own `--settings` to
every call; that is the only difference found (`findings-acme-2.1.288.md`).

| Probe | ACME 2.1.288, Bedrock (other Mac) | Public 2.1.288, subscription (home) |
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

## Rule for nimitz

Resume a stopped ship with the **native** binary and **no flags**, from the ship's
folder: `<native claude> --bg --resume <session id> [prompt]`. Both builds then keep
the same session id and its saved name, permission mode and labels. Never resume
a live ship.
