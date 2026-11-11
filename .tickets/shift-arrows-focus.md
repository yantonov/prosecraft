---
title: "Bind focus moves to Shift+arrows as well as Ctrl+arrows"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-08T12:44:44Z
updated: 2026-11-08T12:44:48Z
closed-at: 2026-11-08T12:44:48Z
---
# Bind focus moves to Shift+arrows as well as Ctrl+arrows

Ctrl+arrows are the only way to jump to a neighbouring pane. On a keyboard where Ctrl and the
arrows are both under the right hand that is one awkward reach for a key pressed constantly.
Bind the same four moves to Shift+arrows, which is closer to the arrows, and keep Ctrl+arrows
working. The two are aliases: same target, same no-wrap rule, same arrival in the output pane
after a rewrite.

Done when:
- `Shift+Left`/`Shift+Right` switch column and `Shift+Up`/`Shift+Down` switch row, with exactly the behaviour `Ctrl+Left`/`Ctrl+Right` and `Ctrl+Up`/`Ctrl+Down` have today, including no wrap at the column edge.
- The chords are handled before the editor and the scrollers, so a shifted arrow never types, moves a cursor, or scrolls a read-only pane; the editor's `CONTROL` guard is unchanged, because a terminal may spell an uppercase letter as `Char('A')`+`SHIFT`.
- New tests pin the aliasing (Shift+arrow equals Ctrl+arrow from every pane, for all four directions), the no-wrap rule for rows, and that neither an editable nor a read-only pane eats the chord.
- HELP_ROWS gains a `Shift+arrows` row and its length goes to 12; the F1 panel still lists every binding at 80x24.
- docs/spec.md §4 gains the Shift aliases in the focus bullet and two key-table rows (approved by the human, as constraint 1 requires). Nothing else in the spec changes: the status line still hints only Ctrl+R / F1 / Ctrl+Q, and §10's M1 check still reads `Ctrl+←→`, which remains a valid spelling.
- make check is green.