# P0 probes

Requirements and how to run: `P0-requirements.md`. Script: `../../probes/p0_probe.py`.

## Comparison across builds

The ACME column was measured with the first version of the script, which passed
flags on every resume. Probes 2a, 2b, 2c1 and 2c2 are new in the revised script;
"not run" means the ACME build has not been probed for them yet. Bold marks a
result that differs between the builds.

| Probe | ACME 2.1.288, Bedrock (other Mac) | Public 2.1.288, subscription (home) |
| --- | --- | --- |
| 0. `--bg` in untrusted folder | refused; home folder trusted as parent | refused; **home-folder trust not saved, probe folder trusted on its own** |
| 1. `--bg --session-id` | ignored, warning | ignored, warning |
| 2a. `--bg --resume <unknown uuid>` | not run (hit by accident: row ended `failed`) | session never runs (SessionEnd only), row `failed`; cannot choose an id |
| 2b. resume while live, no flags | not run | copy with new id; copy fires no hooks (labels lost) |
| 2. resume while live, all flags | copy with new id; **copies end `failed`** | copy with new id, `source: fork`; **copies stay `done`/idle** |
| 2c1. resume after stop, no flags | not run | **same id**, `source: resume`, saved name, mode and labels restored |
| 2c2. resume after stop, `--name` only | not run | copy with new id; copy fires no hooks (labels lost) |
| 2c3. resume after stop, all flags (old "2c") | new id, SessionStart `source: fork`; old row stays | new id, `source: fork`; old row stays |
| 3. labels reach hooks in `--bg` | yes; SessionStart, UserPromptSubmit, Stop, SessionEnd | yes; same hooks (Notification only when idle long enough) |
| 4. hook `sessionTitle` changes the address | no (title only) | no (title only) |
| 5. two live `--bg` with one name | allowed | allowed |
