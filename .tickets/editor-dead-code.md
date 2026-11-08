---
status: open
priority: 3
deps: [editor-wiring]
attachments: []
created: 2026-11-07T17:03:36Z
updated: 2026-11-08T07:46:18Z
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
