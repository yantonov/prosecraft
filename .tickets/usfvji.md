---
title: "Status line drops the Tab hint"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-08T12:37:48Z
updated: 2026-11-08T12:38:27Z
closed-at: 2026-11-08T12:38:27Z
---
# Status line drops the Tab hint

The status line spends 13 of its cells on `[Tab] focus` while `[F1] help` sits two hints
later and opens the panel that lists `Tab / Shift+Tab` anyway. Once `F1` is on the row, the
focus hint is redundant and the row is short one hint it does not need — the row is one
line that clips (`fit`, `src/ui.rs`), so every cell returned is a cell the harness name, the
session id and the status message keep.

Spec §4 was updated with the human's go-ahead (constraint 1): the hint list is
`[Ctrl+R] rewrite`, `[F1] help`, `[Ctrl+Q] quit`, and the §4 frame diagram now matches the
real line — it had gone stale, still showing the pre-`F1` hints. Nothing else in the spec
changed.

Done when:
- `draw_status` emits `│ [Ctrl+R] rewrite  [F1] help  [Ctrl+Q] quit` and no `[Tab]` hint.
- The 80-column status-line test still asserts `[F1] help` and `claude`, and additionally
  asserts the `[Tab]` hint is gone, so re-adding it is a test failure rather than a choice.
- `HELP_ROWS` still lists `Tab / Shift+Tab`, which is what makes the drop safe.
- `make check` green, plus `make run` showing the shorter row at 80 columns.

Out of scope: dropping any other hint, changing the hint order, and the `Tab` binding
itself.
