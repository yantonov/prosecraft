---
title: "Wire Editor and wrap into the editable panes"
status: closed
priority: 2
deps: [editor-wrap, pane-routing]
attachments: []
created: 2026-11-08T07:46:08Z
updated: 2026-11-08T08:01:22Z
closed-at: 2026-11-08T08:01:22Z
---
# Wire Editor and wrap into the editable panes

## Description
The gap between two finished tickets. `editor-widget` built the text buffer, `editor-wrap`
built soft wrapping and wrap-aware movement, and `pane-routing` built the read-only panes —
but nothing connects the editor to the source and instructions panes, so both are still
rendered from `&str` with a fake `▏` cursor appended to the text.

Consequence: `src/editor.rs` and `src/wrap.rs` have no consumer outside their own tests,
which is why both carry `#![allow(dead_code)]` and why `editor-dead-code` cannot be closed.
That ticket is blocked on this one.

## What exists

- `src/editor.rs` — `Editor { lines: Vec<String>, cursor: Cursor }`, columns counted in
  **chars**. `insert_char`, `insert_str`, `insert_newline`, `backspace`, `delete`,
  `move_left`, `move_right`, and logical-line `move_up`/`move_down`. It holds no width and
  knows nothing about the screen.
- `src/wrap.rs` — `buffer_rows`, `cursor_position`, and `WrapState` with `move_up`/
  `move_down` (visual rows, sticky column), `page_up`/`page_down`, `scroll_to_cursor`, and
  `render(&self, frame, editor, area)` which draws the rows and places the terminal cursor.
- `src/ui.rs` — `View { source: &'a str, instructions: &'a str, stream: StreamState, .. }`,
  and `draw_pane` which renders the editable panes as a `Paragraph` with
  `with_cursor(text)` appending the placeholder glyph.
- `src/panes.rs` — the read-only panes, already owned by `View`. This ticket mirrors that
  shape for the editable ones.

## Done when
- `View` owns an `Editor` for the source pane and one for the instructions pane, seeded from
  `Config::initial_source` / `Config::initial_instructions` (spec §3: read once at startup).
  Nothing re-reads the files.
- A `WrapState` per editable pane, so each keeps its own scroll offset and sticky column.
- Typing goes to the focused pane only: a printable character inserts at the cursor, `Enter`
  inserts a newline, `Backspace` and `Delete` do what they say. A read-only pane ignores all
  of them (it has no cursor to edit).
- The cursor keys route to the focused pane: `Left`/`Right` are logical, `Up`/`Down` move by
  **visual row** via `WrapState` (spec §4: a wrapped line is several rows, and Down must move
  one row, not one line).
- `PgUp`/`PgDn` page the focused **editable** pane too. `pane-routing` covers the read-only
  ones; spec §4 says the keys act on the focused pane of either kind.
- The editable panes are drawn by `WrapState::render` into `panes.inner(focus)`, not by a
  `Paragraph`. The fake `▏` glyph and `with_cursor` are deleted, since the real cursor is the
  terminal's.
- `WrapState::scroll_to_cursor` runs after a key that moves the cursor, so the cursor stays
  visible in a pane shorter than the text.
- After a resize the cursor stays on the same logical `(line, column)` and the text is
  byte-identical; no panic at width 0 or 1.
- `Ctrl+R`, `Ctrl+N`, `Ctrl+Y`, `Ctrl+V`, `Ctrl+E`, `Tab`/`Shift+Tab` and `Ctrl+←/→` keep
  working exactly as they do now. This ticket adds editing; it must not disturb focus
  routing or the read-only scrolling `pane-routing` added.
- Existing tests stay green. Two `ui.rs` tests reference the removed placeholder cursor and
  will need their expectations changed to "the terminal cursor is positioned in the pane",
  not deleted.

## Out of scope
- Reading or writing the clipboard (that is `clipboard`).
- Starting a rewrite, the tick loop, session ids (that is `rewrite-flow` and after).
- Validation that a pane is non-empty (that is `rewrite-flow`).
- Removing `#![allow(dead_code)]` from `src/editor.rs` and `src/wrap.rs`: that is
  `editor-dead-code`, which this ticket unblocks and which should be claimed straight after.

## Why this was not already covered
`pane-routing` reads as "wire the stream into the panes", and the stream is the read-only
half. `editor-wrap` deliberately stopped at a self-contained module because the ticket
bounded it there. Neither ticket names the join, so the join had no owner — which is exactly
the failure `editor-dead-code` was written to catch, caught late.
