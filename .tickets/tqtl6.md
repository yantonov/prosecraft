---
status: closed
priority: 2
deps: [horaf, li4i2]
attachments: []
created: 2026-11-11T11:22:43Z
updated: 2026-11-11T11:49:07Z
closed-at: 2026-11-11T11:49:07Z
---
# Ctrl+Z: undo the last edits in an editable pane, capped history

Feature. Blocked on the two tickets above.

Ctrl+Z restores the previous buffer state, text and cursor together, in the focused editable pane. Capped history, no redo.

Design decisions taken (do not rediscover them mid-implementation):
- History lives in a new src/undo.rs and holds snapshots of the buffer, not inverse operations. A snapshot is (Vec<String>, Cursor); prose is "a few paragraphs" by design, so a cap is cheap and inverse ops would be six hand-written inverses to keep correct.
- One step = one public Editor call (see the flattening ticket). Typing a char is one step, a paste is one step, Ctrl+K is one step. That is what makes paste undoable in one press instead of one per char.
- History is NOT inside Editor: a snapshot is not an Editor, or the type contains a history of itself. It sits beside each buffer in View, next to source_wrap. Editor keeps no knowledge of undo.
- If implementation shows that placing it inside Editor would be materially simpler, the two costs must be handled, not ignored: (a) derive(PartialEq) must become manual so "did this edit change anything" does not compare histories; (b) derive(Clone) either clones history (correct, wasteful, src/editor.rs:825 does clone mid-test) or must be manual and documented.
- Granularity is one key event per step. No word coalescing in v1.
- Per-pane history, not one global stack: undoing must not surprise the user by changing a pane they are not looking at.
- No redo. Undoing a large paste loses it; that is the accepted trade and is why the ticket is capped-history, not full history.

Done when:
- Ctrl+Z in source and instructions restores text AND cursor of the previous step; repeated presses walk back; at the bottom it is a no-op with no message spam (a status note is fine only if it says something true, e.g. nothing to undo).
- A typed char is one step, Enter is one step, Backspace/Delete are one step, and a multi-line Ctrl+V of a paragraph is exactly ONE step: paste a block, one Ctrl+Z, the buffer is byte-identical to before the paste including the cursor.
- Ctrl+K is one step: clear a pane, one Ctrl+Z, the full previous text and cursor are back.
- No-op edits record nothing: Backspace at the very start and Delete at the very end must not create steps that appear to do nothing. Cover the first press after loading, and a run of repeated no-ops.
- The history is capped and the cap evicts oldest-first; a test drives past the cap and asserts the oldest step is gone and the newest N still undo.
- Ctrl+Z in output and debug is refused through require_editable (src/ui.rs:544) with the same "X is read-only" toast Ctrl+V and Ctrl+K use. No panic, focus unchanged, pane content unchanged.
- After an undo the view follows the restored cursor: wrap.scroll_to_cursor is called with the cached viewport width, and a test asserts a cursor restored far down the buffer is visible on screen.
- The two panes never cross-contaminate: undoing in instructions leaves source untouched, and vice versa.
- HELP_ROWS (src/ui.rs:642) gains a Ctrl+Z row and the array length is updated; the key table in docs/spec.md section 4 already has the row from the spec ticket, and the panel is that table on-screen copy.
- make check is green.

Not in this ticket: redo, undo across panes, undo of the read-only panes, undo of StreamState (Ctrl+N keeps its current meaning: it clears output and debug, and there is no undo for that), word-level coalescing, and any persistence of history.

## Cross-check 2026-11-10 (after the spec ticket landed)

The spec now carries the Ctrl+Z row, so this ticket is unblocked on horaf and its design paragraph is normative: docs/spec.md section 9 says the history is a per-pane capped snapshot stack beside the wrap state, not inside Editor, one step per public editor call, no-op edits record nothing. If an implementation choice contradicts that paragraph, it is a spec change and needs approval - not a local decision.

Two line references in the body above have drifted; resolve by symbol, not by number:
- require_editable is currently src/ui.rs:547 (was 544).
- HELP_ROWS is currently src/ui.rs:640 with 13 rows (was 642).

Also note HELP_ROWS is a fixed-size array whose length is part of its type and whose rows are asserted by f1_opens_the_panel_and_every_binding_is_visible_at_80x24; adding a row means changing the 13 in the type, not just pushing an element.
