# P0 probes

Requirements and how to run: `P0-requirements.md`. Script: `../../probes/p0_probe.py`.

## Comparison across builds

| Probe | ACME 2.1.288, Bedrock (other Mac) | Public build, subscription (home) |
| --- | --- | --- |
| 0. `--bg` in untrusted folder | refused | _to run_ |
| 1. `--bg --session-id` | ignored, warning | _to run_ |
| 2. resume while live | copy with new id; copies end `failed` | _to run_ |
| 2c. resume after stop | new id, SessionStart `source: fork`; old row stays | _to run_ |
| 3. labels reach hooks in `--bg` | yes; SessionStart, UserPromptSubmit, Stop, SessionEnd | _to run_ |
| 4. hook `sessionTitle` changes the address | no (title only) | _to run_ |
| 5. two live `--bg` with one name | allowed | _to run_ |
