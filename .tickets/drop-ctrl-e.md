---
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-08T12:22:00Z
updated: 2026-11-08T12:24:49Z
closed-at: 2026-11-08T12:24:49Z
---
# Drop Ctrl+E: Ctrl+V on the focused pane is enough

Ctrl+E is a second, redundant way to get the clipboard into the source pane: it is a whole-pane replace that ignores focus and then moves focus to source. Ctrl+V into a focused source pane does the same job with one key that already exists, so Ctrl+E is retired and the clipboard surface shrinks to Ctrl+Y / Ctrl+V.

Done when:
- The Ctrl+E arm is gone from View::handle_key and load_into_source is gone; no dead code left behind.
- The F1 panel no longer lists Ctrl+E and HELP_ROWS is 12 rows with its length assertion updated.
- The tests that pinned Ctrl+E (ctrl_e_replaces_the_source_from_any_focus_and_moves_focus_there, ctrl_e_with_an_empty_clipboard_never_wipes_the_source) are replaced by a test that Ctrl+E now does nothing: it types no character, wipes no pane, and leaves focus where it was. The Ctrl+V empty-clipboard test stays.
- docs/spec.md §4 drops the Ctrl+E row (approved by the human, as constraint 1 requires). The
  rest of the spec is unchanged: no other section documents Ctrl+E, and §10's M1 check still
  reads "clipboard load/copy/paste", which Ctrl+Y/Ctrl+V cover.
- make check is green.
