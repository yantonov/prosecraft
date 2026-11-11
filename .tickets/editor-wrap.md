---
title: "Editor wrapping: soft-wrap rendering and wrap-aware movement"
status: closed
priority: 2
deps: [editor-widget]
attachments: []
created: 2026-11-07T15:34:58Z
updated: 2026-11-07T22:52:20Z
closed-at: 2026-11-07T22:52:20Z
---
# Editor wrapping: soft-wrap rendering and wrap-aware movement

## Description
Soft-wrap for the panes, built on the editor core from editor-widget. This is the half where the bugs live, which is why it is its own ticket.

Done when:
- A long logical line is soft-wrapped to the pane width at render time; the buffer is never mutated by wrapping.
- Rendering a buffer into a ratatui Frame is correct via TestBackend at a fixed width, including a line that wraps to exactly 2 and exactly 3 visual rows.
- A word longer than the pane width is hard-split across visual rows. Spec §4 says text is never truncated, so overflow is not an option. No hyphenation, no ellipsis.
- Up/Down move by *visual* row, not logical line: pressing Down inside a wrapped line moves one row and keeps the target visual column where possible, and pressing Down inside a hard-split word moves one row within that word. Crossing a wrap boundary behaves like a normal editor.
- The cursor's visual position is computed from (line, column, width) and is asserted in unit tests for widths 1, 10, and a width narrower than the longest word.
- The view scrolls to keep the cursor visible inside a pane shorter than the wrapped text, and PgUp/PgDn scroll the focused editable pane by a page (spec §4 key table).
- Re-wrapping on terminal resize recomputes visual rows; the cursor stays on the same logical (line, column) and never panics, including width 0 and width 1.
- A width change does not lose text: the buffer is byte-identical before and after a resize.
- Zero-cost when text is short: a line shorter than the width occupies one visual row.
- Unit tests cover wrap boundaries, cursor at a wrap point, hard-split words, multi-wrapped lines, view-scroll-to-cursor, and the resize cases above.
