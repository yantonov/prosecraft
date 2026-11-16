---
title: "Shift+Enter clears the pane: a spelled key arrives as a text burst"
status: closed
priority: 1
deps: []
attachments: []
created: 2026-11-16T15:51:23Z
updated: 2026-11-16T15:51:34Z
closed-at: 2026-11-16T15:51:34Z
---
Reported by a user: pressing `Shift+Enter` in an editable pane cleared the pane and left the
literal text `[13;2u` in it. `Ctrl+Z` was the only way back.

## Cause

The terminal emits `Shift+Enter` as the keyboard-enhancement sequence `CSI 13;2u`. crossterm's
Windows backend reads console key records rather than escape sequences, so the app receives
`Esc` (a no-op per spec §4) followed by six ordinary `KeyCode::Char` events: `[`, `1`, `3`,
`;`, `2`, `u`.

Those six chars arrive inside `BURST_GRACE` (10 ms), so `collect_text_run` groups them and
`count > 1` sends the string to `View::handle_text_run` -> `handle_paste`. That is the route the
Windows `Ctrl+V` workaround uses (ticket `ilghh`), and it replaces the pane: `replace_text`
calls `editor.clear()` before `insert_str`. Hence a cleared pane holding `[13;2u`, and one
`Ctrl+Z` restoring it because the whole route is one undo step.

The bug is that the burst heuristic is undefended: a run is treated as a paste on timing alone,
so terminal plumbing is indistinguishable from prose.

## Decision (user-approved)

`Shift+Enter` acts as plain `Enter`. The run classifier drops the sequence's modifiers, so the
decoded key is `Enter`.

## Done when

- `src/main.rs` classifies a collected run before applying it (`classify_run`): ordinary text is
  a paste as before, a complete `[code;modifiersu` sequence is decoded and routed through
  `View::handle_key`, and the truncated head of one is dropped rather than applied.
- The guard stays narrow: leading `[`, then only digits, `;` and `u`, at most 16 chars, so
  prose that opens with a bracket (`[1]`, `table [1;2] here`) stays text.
- Tests: the sequence classifies as `Enter`; a truncated head is a fragment; prose with
  brackets and digits stays text; and end to end, a pane holding text plus a `[13;2u` burst
  gains a line instead of losing the buffer.
- `docs/spec.md` §4.1 documents the heuristic, the three-way classification, why the modifier
  is dropped, why the fragment case exists, and why the guard is narrow. §9 and §10 name it.
- `make check` green, plus `make run`: type in the source pane, press `Shift+Enter`, get a new
  line with the text intact; a multi-line `Ctrl+V` paste still replaces the pane as one step.
