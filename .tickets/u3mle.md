---
title: "u3mle"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-11T14:39:50Z
updated: 2026-11-11T14:40:53Z
closed-at: 2026-11-11T14:40:53Z
---
# u3mle
Shift+Insert is the second real paste convention — conhost, xterm, PuTTY and VS Code all speak it — and the app ignores it today: it misses every arm in the editable block and dies in route_focus. Ctrl+Shift+V, Cmd+V and Ctrl+Y stay out on purpose: the first two are consumed by the terminal so the app never sees them, and Ctrl+Y is the readline kill-ring yank, which this app has no ring for (and Ctrl+Y is retired here anyway, see nxm5v).

Done when:
- Shift+Insert routes to the same View::paste as Ctrl+V: same clipboard read, same replace, same read-only toast, same status-line note for an empty or unavailable clipboard.
- The arm sits with the other clipboard keys, before the editable-pane block, so the chord never reaches the editor as a stray key. Any spelling of it that crossterm can produce is accepted: (Insert, SHIFT) at minimum.
- Ctrl+Insert is not a paste key here: it stays inert, like Ctrl+E after drop-ctrl-e.
- HELP_ROWS gains one row reading Shift+Insert next to Ctrl+V, so 14 becomes 15. The 80x24 panel-height test still passes (15 + 2 = 17), and every row is still visible at 80x24.
- docs/spec.md §4 gains the Shift+Insert row directly under Ctrl+V. Human approved the spec edit. No other spec section changes, and §10's M1 check still reads "clipboard load/copy/paste".
- README.md says Shift+Insert pastes too, in the key paragraph.
- Tests: Shift+Insert replaces the focused editable pane exactly like Ctrl+V; it is refused in output and debug with the read-only toast and edits nothing; Ctrl+Insert is inert (no pane change, no focus move, no note); the F1 row assertion keeps covering every HELP_ROWS entry.
- make check is green.
