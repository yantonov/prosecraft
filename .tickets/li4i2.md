---
title: "Editor owns every buffer mutation: flatten the public call graph, add Editor::clear"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-11T11:22:35Z
updated: 2026-11-11T11:39:30Z
closed-at: 2026-11-11T11:39:30Z
---
# Editor owns every buffer mutation: flatten the public call graph, add Editor::clear
Refactor. No behaviour change. Precondition for undo: makes "one public Editor call = one undoable step" true by construction.

Today src/editor.rs has public methods calling other public methods:

  Editor::insert_char -> Editor::insert_newline   (the `if c == '\n'` arm)
  Editor::insert_str  -> Editor::insert_char      (the char loop)

Any history recorded at public entry would therefore record a paste as N steps, one per char.

Also, the buffer is replaced wholesale from outside in exactly one place, View::clear in src/ui.rs:

  *editor = Editor::new();
  *wrap = WrapState::new();

which re-establishes the one-empty-line invariant by construction instead of asking the type that owns it.

Done when:
- Editor::clear() exists (truncate to one line, empty it, cursor to origin), keeps the one-empty-line and cursor invariants, and is idempotent on an empty buffer. Unit tests in src/editor.rs.
- No public method on Editor calls another public method. Structure:
    fn insert_one(&mut self, c: char)        // private funnel: the one place \n is routed
    pub fn insert_char / insert_newline / insert_str / clear
  insert_str keeps its char loop; it is one PUBLIC call, not one internal operation. The newline-routing rule must live in exactly one place, not be copied into insert_str.
- The rule is recorded in the module doc or the method docs: private primitives never record history; a future public op that composes others must take its own snapshot at entry rather than delegating to two recording publics.
- View::clear calls editor.clear() and no longer does *editor = Editor::new(), and no longer does *wrap = WrapState::new(). Verify, do not assume: add a test that clearing a pane which is scrolled down leaves a drawable pane (next draw shows the top, no panic on a zero-height window), since WrapState::scroll_to_cursor already clamps a stale offset and WrapState::render_with_cursor re-records the viewport every frame.
- The stale doc comment on View::clear claiming "Ctrl+K then Ctrl+V is the recovery the user has" is corrected or deleted: clear never wrote the clipboard, so that claim was never true. (Already rewritten to name Ctrl+Z as the recovery while aligning the spec; keep that, and keep it true once undo lands.)
- ui::tests::ctrl_k_clears_the_focused_editable_pane passes verbatim: 1 line, cursor (0,0), neighbour untouched, focus unchanged.
- make check is green.

Not in this ticket: any history type, any key binding. Note that Editor::clear() consumes one of the five methods that keep the #![allow(dead_code)] in src/editor.rs alive (see editor-dead-code.md); move_up/move_down are still unconsumed, so that attribute stays and that ticket stays open.
