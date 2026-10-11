# A fleet's harness (0.7.0)

Status: built in 0.7.0 (P4). What the user, or the fleet's admiral, gives
every member of one fleet beyond nimitz's own role rules: rules, subagents,
skills and settings, kept in the fleet's folder so every machine of the fleet
has them.

## 1. Where

`<NIMITZ_HOME>/<fleet>/harness/` (shared, beside fleet.toml):

| Path | What | When a member has it |
| --- | --- | --- |
| `rules/<name>.md` | text after the role rules (`## Fleet F's own rules`), names in order, at most 4000 characters in all | at its next session start; a live member at its next prompt (once per change) |
| `agents/<name>.md` | a subagent (frontmatter with a description; no `hooks:`, which Claude Code ignores for a plugin agent) | at its next launch; a running one after `/reload-plugins` |
| `skills/<name>/SKILL.md` (and files beside it) | a skill | as agents |
| `settings/fleet.json`, `settings/admiral.json`, `settings/<SQ>.json`, `settings/<SQ>/COMMODORE.json`, `settings/<SQ>/<SHIP>.json` | Claude Code settings, merged in that order (objects key by key, arrays joined) under nimitz's own `--settings` (its labels and its allow rule win) | at its next launch |

A harness never sets what runs a program or steers nimitz: `hooks`,
`apiKeyHelper`, `awsAuthRefresh`, `awsCredentialExport`, `otelHeadersHelper`,
`statusLine`, `disableAllHooks`, nor `env.NIMITZ_*`, `env.HERDR_*`,
`env.CLAUDE_CODE_*`, `env.CLAUDECODE`. `add` refuses such a file; one put
there by hand is left out of every launch and named by `check`
(`harness.invalid`).

## 2. The plugin

Agents and skills reach Claude Code as a plugin nimitz makes on each machine,
`NIMITZ_STATE/harness/<fleet>/plugin` (named `fleet-<fleet>`: an agent runs as
`fleet-home:reviewer`), and gives every launch with `--plugin-dir`. Claude
Code keeps the flag in the job's `respawnFlags`, so a wake (a native resume)
keeps the plugin too (seen on 2.1.296). The plugin is made again only when the
harness's agents or skills changed (a stamp of their content), made beside the
old one and swapped in.

## 3. Rules, live

`rules::text_for` is the role rules, then the harness rules. A delivery
records the hash of the harness rules it gave (`rules-delivered` format 2:
`harness`); `update_for` sends the whole text again at the next prompt when
the rules version or that hash changed, once per change.

## 4. `nimitz harness`

| Command | Who | What |
| --- | --- | --- |
| `list` | every member (its own fleet), you | what the harness holds, the size of the rules, its problems |
| `add rule\|agent\|skill\|settings <file\|folder> [--name N] [--for F/SQ\|F/SQ/SHIP]` | the fleet's admiral, you | copied in (`--plan`, then `--yes`); settings for a squadron or a member with `--for` |
| `rm rule\|agent\|skill\|settings <name\|id>` | the admiral, you | renamed `.archived` (planned) |
| `push` | the admiral, you | the plugin made again here; what is live now and what needs `nimitz start F -f` |

Inside a member's session the fleet is the member's own (`--fleet` may only
repeat it); the user names it with `--fleet F`.
