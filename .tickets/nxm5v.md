---
title: "Drop Ctrl+Y: Ctrl+C is the only copy key"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-08T12:35:41Z
updated: 2026-11-08T12:36:12Z
closed-at: 2026-11-08T12:36:12Z
---
# Drop Ctrl+Y: Ctrl+C is the only copy key

Ctrl+Y is documented as "same as Ctrl+C outside a running debug pane", but the two are not equivalent: in the debug pane while a rewrite runs, Ctrl+C arms and then performs a cancel, so Ctrl+Y is the only key that copies there — which makes it a differently-behaving key, not a duplicate. Rather than carry two copy keys with one exception, retire Ctrl+Y and copy through Ctrl+C alone.

Done when:
- The Ctrl+Y arm is gone from View::handle_key and Ctrl+Y is inert in every pane and state: it types no character, copies nothing, cancels nothing, and leaves focus where it was.
- `copy` is reached only from `interrupt`, so the helper is the debug-pane fallthrough it is named for. The empty-pane note, the clipboard-failure note, and the read-only refusal are unchanged.
- `ctrl_y_copies_the_focused_pane` and `ctrl_y_copies_the_source_pane_too` are replaced by one test that Ctrl+Y does nothing in every focus; debug-while-running copy is already covered by `ctrl_c_copies_the_focused_pane_instead_of_quitting`.
- `ctrl_y_on_an_empty_pane_says_so_and_writes_nothing` moves to Ctrl+C, keeps its assertions, and stays distinct from the source-pane copy case.
- `any_key_dismisses_the_toast_without_also_acting` drives the dismissal with Ctrl+C instead of Ctrl+Y and keeps both assertions.
- HELP_ROWS drops the Ctrl+Y row and its length goes to 11; the F1 panel still lists every remaining binding at 80x24.
- docs/spec.md §4 drops the Ctrl+Y row (approved by the human, as constraint 1 requires). Nothing else in the spec documents Ctrl+Y: §1 and §10 M1 read "clipboard load/copy/paste", which Ctrl+C/Ctrl+V cover.
- make check is green.
