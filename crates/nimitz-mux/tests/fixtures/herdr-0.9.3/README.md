# Herdr 0.9.3 samples (protocol 22)

Real output, anonymised (fleet, squadron, user, host, paths, terminal ids and
session ids replaced; ids `wN`, `wN:tN`, `wN:pN`, numbers and the structure kept).

- `snapshot.json`: `herdr api snapshot` of a machine that runs two fleets, one
  workspace per fleet (`ALPHA`, `BRAVO`) and one per squadron (`ALPHA.GH`,
  `BRAVO.SQA`, ...). Lead tabs use both the 0.3 short labels (`ADMIRAL`,
  `COMMODORE`) and the full labels (`BRAVO.ADMIRAL`, `BRAVO.SQA.COMMODORE`).
  `panes[]` has no agent fields; `agents[]` lists the panes that show an agent,
  with `agent_session {agent, kind: "id", source: "herdr:claude", value: <session id>}`.
  A Claude pane's `terminal_title` is `<status glyph> <session name>`.
- `process-info.json`: `herdr pane process-info --pane <id>`. This sample's
  foreground process is the `herdr` command itself (it ran in that pane); a pane
  that runs `claude attach <job>` lists that argv in `foreground_processes[]`.
