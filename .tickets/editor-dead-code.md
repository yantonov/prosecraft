---
status: in_progress
priority: 3
deps: [editor-wiring]
attachments: []
created: 2026-11-07T17:03:36Z
updated: 2026-11-08T08:52:46Z
---
# Remove the dead_code allowance from src/editor.rs

## Description
src/editor.rs carries a module-level '#![allow(dead_code)]' added while nothing outside its own tests consumed the editor API. It is a deferred obligation, not a permanent allowance: left in place it hides genuine dead code in the editor for the rest of the project's life.

Done when:
- The '#![allow(dead_code)]' line and its justification comment are gone from src/editor.rs.
- 'make check' is green without the attribute, which proves every public method now has a real consumer.
- If clippy reports specific methods as unused at that point, the fix is to delete or wire those methods, not to re-add the attribute or sprinkle '#[allow(dead_code)]' on them. Report which methods were unused instead of silencing them.

Timing: the attribute becomes removable as soon as the UI renders the editor (editor-wrap) and the keys edit it (pane-routing). Attempt the removal after pane-routing lands; if clippy still flags parts of the API, note which and leave the ticket open.

Why this is a ticket rather than a comment: a deferred 'remove this later' that is not tracked is the one kind of TODO that reliably survives to release, because every later reader assumes someone else owns it.

## Attempted 2026-11-08: blocked on a decision, not on work

Removing the attribute was measured, not guessed. It leaves five unused methods:

    text  line_count  insert_str  move_up  move_down

Satisfying clippy by deleting them is the wrong trade, and this ticket's own rule
('the fix is to delete or wire those methods') does not cover what deleting them costs:

- 18 of the 53 tests in src/editor.rs exercise one or more of them. move_up/move_down
  alone carry 10 tests that document real editor behaviour (column clamping, not
  leaving the buffer, empty lines, multibyte). Deleting the methods means deleting the
  tests, and the tests are the specification of the buffer.
- move_up/move_down are not dead in the sense of 'never wanted'. They are superseded at
  the *call site* by WrapState's visual-row moves, which are the ones the UI uses. The
  logical-line primitive is still the thing those are built to be equivalent to over a
  single logical line, and it is what a future single-line pane or a test harness would
  reach for.
- insert_str is the paste primitive. Ctrl+V lands in the clipboard ticket, which is
  blocked on adding arboard; deleting insert_str now means re-adding it there.
- text and line_count are accessors a caller asks for as soon as anything needs to
  serialise the buffer back out, which the session lifecycle work does.

So: leave the attribute, keep the API and its tests, and close this ticket when the
consumers land. The attribute is honest about the current state.

Unblocking options, for whoever picks this up:
1. clipboard lands insert_str a caller (paste) and text() (copy) -> 3 of the 5 gain real
   consumers, leaving only move_up/move_down, which are then a genuine delete-or-keep call.
2. A decision that Editor should expose no more than the UI calls would make the delete
   correct; it is a public-API decision, not a cleanup, so it needs to be made explicitly.

Status left open. Nothing in the tree changed: src/editor.rs is byte-identical to master
and make check is green.

Timing: the attribute becomes removable as soon as the UI renders the editor (editor-wrap) and the keys edit it (editor-wiring). CORRECTION 2026-11-08: the original note said pane-routing, which never wired the editor; that gap is why editor-wiring was filed.
