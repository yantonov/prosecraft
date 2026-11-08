---
status: closed
priority: 1
deps: []
attachments: []
created: 2026-11-08T21:38:00Z
updated: 2026-11-08T21:39:57Z
closed-at: 2026-11-08T21:39:57Z
---
# Plain q must not quit: it types in an editable pane
Typing `q` in the source or instructions pane quits the app instead of inserting `q`.
Every other letter types fine, so one character is silently unreachable — and the panes
exist to be typed into.

Cause: `is_quit` in `src/ui.rs` accepts `Char('q')` with no modifiers as well as with
`Ctrl`, and `handle_key` checks it before the editable-pane block. The plain-`q` branch
came from the initial scaffolding commit (0f5d0a9), when there was no editing; its comment
says it exists so a pty check that only types printable keys can drive the quit path.
That reason is obsolete, and the branch contradicts the spec.

Audit (no other key is affected):
- `Ctrl+R/V/K/N` and `Ctrl+C` all require `KeyModifiers::CONTROL`, so the plain letters
  reach the editor; `Char('Q')` (shifted) does not match `is_quit` either.
- `F(1)` and `Tab`/`BackTab` are not `Char`, so they cannot be typed away; Esc is a no-op.
- In a read-only pane a plain character is ignored, before and after the fix.

Done when:
- `is_quit` is true only for `Char('q')` with `KeyModifiers::CONTROL`; spec §4 says
  `Ctrl+Q` is "the only quit key". The doc comment loses the plain-`q` rationale.
- Typing `q` into a focused source or instructions pane inserts `q` and does not quit;
  every printable ASCII letter does the same (a table test, so the next such branch
  cannot hide behind one character).
- Typing `q` in a read-only pane (output, debug) is a no-op and does not quit.
- `plain_q_quits` is replaced by a test asserting the opposite, and
  `loop_keeps_running_until_a_press` in `src/main.rs` sends `Ctrl+Q` for its quitting
  press instead of a plain `q`.
- docs/spec.md is untouched.
- make check is green.
