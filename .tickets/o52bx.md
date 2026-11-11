---
title: "o52bx"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-08T11:08:52Z
updated: 2026-11-08T11:11:06Z
closed-at: 2026-11-08T11:11:06Z
---
# o52bx
## Description
`Tab` cycles all four panes and `Ctrl+Left`/`Ctrl+Right` cross columns, but nothing moves
within a column: from source to instructions means three `Tab` presses, and the same for
output to debug. The row step is the missing half of the column jump, so the natural pair is
`Ctrl+Down` (top pane to bottom pane, same column) and `Ctrl+Up` (bottom to top).

Done when:
- `Ctrl+Down` moves source -> instructions and output -> debug.
- `Ctrl+Up` moves instructions -> source and debug -> output.
- Neither key wraps: `Ctrl+Up` on a top pane and `Ctrl+Down` on a bottom pane leave focus
  where it is, so the row keys cannot be used to cycle by accident.
- Neither key edits a pane or scrolls a read-only pane; a plain `Up`/`Down` keeps its
  current meaning (cursor movement when editable, scroll when read-only).
- `make check` is green.

## Notes
Spec §4 lists `Ctrl+←`/`Ctrl+→` as "Switch column" and says nothing about a row equivalent.
Adding the pair means one bullet in §4 and one row in the key-binding table; the spec is
frozen, so that edit needs human approval before it lands.

Files: `src/layout.rs` (`Focus::switch_row`), `src/ui.rs` (`route_focus`, with Ctrl+Up/Down
routed before the editable-pane and read-only-pane arrow handling so they are not eaten as
cursor or scroll keys).
