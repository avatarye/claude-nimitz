# A fleet's harness (0.7.0)

Status: built in 0.7.0 (P4). What the user, or the fleet's admiral, gives
every member of one fleet beyond nimitz's own role rules: rules, subagents,
skills and settings, kept in the fleet's folder so every machine of the fleet
has them.

## 1. Where

`<NIMITZ_HOME>/<fleet>/harness/` (shared, beside fleet.toml):

| Path | What | When a member has it |
| --- | --- | --- |
| `rules/<name>.md` | text after the role rules (`## Fleet F's own rules`, which say they never change who may do what), names in order, at most 4000 characters in all, cut further so the whole context stays under 10,000 | at its next session start; a live member at its next prompt (once per change) |
| `agents/<name>.md` | a subagent (frontmatter with a description; no `hooks:`, `mcpServers:` or `permissionMode:`, which Claude Code ignores for a plugin agent) | at its next launch; a running one after `/reload-plugins` |
| `skills/<name>/SKILL.md` (and plain files beside it) | a skill that registers no hooks and runs nothing by itself (no `hooks:`, no `allowed-tools:`, no `` !`...` ``) | as agents |
| `settings/fleet.json`, `settings/squadrons/<SQ>.json`, `settings/members/<F.SQ.SHIP>.json` (an admiral's `F.json`, a commodore's `F.SQ.json`) | Claude Code settings, merged in that order (objects key by key, arrays joined) under nimitz's own `--settings` (its labels and its allow rule win) | at its next launch |

A harness is the admiral's to change, so what it holds is narrow, and held at
every launch (a file put in the folder by hand gives no more than `add`
takes; `check` names what is dropped, `harness.invalid`):

- settings: `model`, `outputStyle`, `permissions.allow|ask|deny`, and `env`
  names in capitals that steer no program, traffic or credential (not `PATH`,
  `HOME`, `LD_*`, `DYLD_*`, `NODE_*`, `PYTHON*`, `GIT_*`, `ANTHROPIC_*`,
  `CLAUDE*`, `NIMITZ_*`, `HERDR_*`, `AWS_*`, `*_PROXY`, ...). A `deny` or
  `ask` that would stop `nimitz` (`Bash`, `Bash(nimitz *)`, ...) is dropped.
  Everything else (a permission mode, more folders, plugins, MCP servers,
  hooks) is the user's own Claude Code settings; a member's mode is
  `ship set --mode` and the like.
- no file is a link, and none is over 64 KiB.
- `add` by the admiral takes a file from its own folder only: never a file of
  the user's it could read (the harness syncs to every machine and reaches
  every member's context).

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
| `add rule\|agent\|skill\|settings <file\|folder> [--name N] [--for F/SQ\|F/SQ/SHIP [--member]]` | the fleet's admiral (from its own folder), you | copied in (`--plan`, then `--yes`; a skill added again loses the files it no longer has); settings for a squadron, or one member's own with `--member` |
| `rm rule\|agent\|skill <name>`, `rm settings [--for ID [--member]]` | the admiral, you | renamed `.archived` (`.archived-2`, ... when taken; planned) |
| `push` | the admiral, you | the plugin made again here; what is live now and what needs `nimitz start F -f` |

Inside a member's session the fleet is the member's own (`--fleet` may only
repeat it); the user names it with `--fleet F`.
