---
title: "g4nxst"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-11T14:33:40Z
updated: 2026-11-11T14:34:44Z
closed-at: 2026-11-11T14:34:44Z
---
# g4nxst
The paste surface inserts at the cursor today, so a second paste appends a second text to the first. The panes are slots, not documents: the app exists to rewrite one block of prose, and anything that needs real editing is done outside it. So a paste loads the focused editable pane, replacing whatever was there, and a single `Ctrl+Z` is the recovery.

Done when:
- `View::insert_text` becomes a replace: one `undo.edit(editor, |e| { e.clear(); e.insert_str(text); })` call, so a paste is one edit and one `Ctrl+Z` whether the pane was full or empty. No `is_empty` branch: a clear plus an insert into an empty pane is an insert.
- Both paste routes change, not just `Ctrl+V`. Terminal paste, `Ctrl+Shift+V`, middle-click and the Windows burst collected in `main.rs` all converge on the same funnel, so the rule is one rule on every platform. A replace added to the `Ctrl+V` arm alone would not exist on Windows, where the terminal swallows the chord and injects char events instead.
- The `!text.is_empty()` guards stay and become load-bearing: an empty paste must never wipe a pane. The doc comments say so, because the reason changed from "nothing to insert" to "a wipe with no text to put back".
- Focus does not move: both routes already act on `self.focus` and `require_editable` already refused `output`/`debug`.
- The comment at `src/ui.rs:235-241` is rewritten. It currently claims a `Ctrl+V` into source "replaces what is there", which was false when it was written; it becomes true by this ticket, and the comment must state the rule rather than a coincidence.
- `HELP_ROWS` Ctrl+V row and the `src/clipboard.rs` module doc say "replace". `HELP_ROWS` keeps 14 rows; only its wording changes.
- docs/spec.md §4 `Ctrl+V` row reworded to say the paste replaces the focused editable pane and is one undo step. Human approved the spec edit. No other spec section changes.
- README.md:142 says `Ctrl+V` replaces the focused editable pane.
- Tests: `ctrl_v_pastes_into_the_focused_editable_pane_at_the_cursor` becomes the replace test (assert the old text is gone, not `"apastedb"`); add that a paste into `instructions` replaces instructions and leaves source alone; add that one `Ctrl+Z` restores the previous text and cursor; `ctrl_v_of_a_paragraph_is_one_step_that_restores_bytes_and_cursor` and `a_terminal_paste_is_one_undo_step` updated for the replace. `ctrl_v_with_an_empty_clipboard_never_wipes_a_pane` and both read-only refusal tests stay as they are.
- make check is green.
