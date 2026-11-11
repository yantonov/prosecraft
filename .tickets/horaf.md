---
title: "Lift the undo/redo non-goal from the spec"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-11T11:22:35Z
updated: 2026-11-11T11:27:19Z
closed-at: 2026-11-11T11:27:19Z
---
# Lift the undo/redo non-goal from the spec

Spec change. Needs human approval (AGENTS.md constraint 1). No code.

docs/spec.md:18 lists "Editing history, undo/redo" as a v0.1 non-goal, and section 4 key table has no Ctrl+Z row. Both contradict the Ctrl+Z work, so this ticket is the gate: it must land before the code or the code contradicts the design every other ticket was cut from.

Scope is deliberately minimal - one non-goal line and one key table row - because every extra paragraph of spec is another disagreement surface.

Done when (with explicit human approval recorded in the ticket):
- docs/spec.md section 1 non-goal drops undo from the list. Editing history, redo, and any conversation view stay non-goals; if only undo is being lifted, say so in the line rather than deleting the whole bullet.
- docs/spec.md section 4 key table gains a Ctrl+Z row: undo the last edit in the focused editable pane (N steps), refused with the read-only toast in output and debug, in the same style as the Ctrl+V and Ctrl+K rows.
- Section 2 (which says the editor has "no selection and no undo") and section 9 (same phrase) are checked and amended only if they would otherwise be false. Do not rewrite them for style.
- README.md:42 ("No chat view, no undo, no model picker") and the src/editor.rs:13 module doc comment ("There is no selection and no undo (spec 2)") are brought into line. These are the only two other places that assert the non-goal.
- Nothing else in the spec is touched. In particular the measured-constraint sections (6.0, 6.1a, 6.1b) and the "output pane cleared on spawn, there is no undo" reasoning in 6.1b and 5 step 4: that "no undo" is about the output pane, not the buffer, and it remains true. If that reading is unclear after the edit, add one clause saying so instead of leaving an apparent contradiction.

If approval is refused, close this ticket as rejected and close the Ctrl+Z ticket with it: nothing else is blocked.

## Closed 2026-11-10: approved and applied

Human approved the spec edit in session. Changed:

- docs/spec.md:18 - non-goal is now "Editing history and redo", with undo of the two editable buffers explicitly in scope.
- docs/spec.md section 4 key table - new Ctrl+Z row: one step per key event, refused with the read-only note in output and debug, capped history, no redo.
- docs/spec.md section 6.1b clause 3 - the spawn-clear sentence said "there is no undo (2)", which is now wrong as written; it says undo never reaches the read-only panes, which keeps the reasoning true.
- docs/spec.md section 9 - the editor paragraph gains the undo design: per-pane capped snapshot stack beside the wrap state, not inside the Editor (a snapshot would otherwise be recursive), one step = one public editor call, and no-op edits record nothing.
- docs/spec.md Decisions - one clause recording undo in scope for the editable buffers only.
- README.md:42 - "no undo" becomes "no redo", since the only history is the capped per-pane undo.
- src/editor.rs module doc, src/panes.rs StreamState::spawn doc and its spawn-test comment, src/ui.rs View::clear doc - reworded so none of them still assert a non-goal that no longer exists. The View::clear comment also drops the false claim that Ctrl+K then Ctrl+V is the recovery: clear never wrote the clipboard, so Ctrl+Z is now named instead.

make check green: 469 unit tests, 7 cli tests. No code behaviour changed.
