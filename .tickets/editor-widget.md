---
title: "Editor core: buffer, cursor, edit operations"
status: closed
priority: 2
deps: [layout-panes, tui-dummy]
attachments: []
created: 2026-11-07T15:33:44Z
updated: 2026-11-07T16:51:09Z
closed-at: 2026-11-07T16:51:09Z
---
# Editor core: buffer, cursor, edit operations

## Description
The buffer and cursor model shared by the source and instructions panes. No wrapping, no rendering — pure text state, so it is testable without a terminal.

Done when:
- A buffer holds text and a cursor as (line, column).
- Insert a character, a string, and a newline at the cursor; each is verifiable by asserting the resulting text and cursor.
- Backspace removes the character before the cursor, and at column 0 joins with the previous line.
- Delete removes the character at the cursor, and at end of line joins with the next line.
- Left/Right move within and across lines, clamping at the buffer edges without panicking on an empty buffer.
- Up/Down move by logical line only, clamping the column to the target line's length (no wrap awareness yet — that is editor-wrap).
- No selection and no undo (explicit non-goals per spec §2). Do not add Home/End or any key binding: the cursor API is a model, and spec §4 lists no Home/End binding.
- Unit tests cover every operation at the boundaries: empty buffer, start of buffer, end of buffer, single-character line, and multi-line join in both directions.
- Zero terminal or ratatui dependency: this module compiles and tests standalone.

Note: this ticket adds no key handling. It exposes a cursor/buffer API; key mapping lives with the app.
