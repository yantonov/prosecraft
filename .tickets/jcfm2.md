---
status: open
priority: 2
deps: []
attachments: []
created: 2026-11-08T11:05:21Z
updated: 2026-11-08T11:05:24Z
---
# jcfm2
## Description
The instructions pane was `min(5, 25% of column)` and the debug pane `min(8, column / 3)`,
so the border between top and bottom landed on two different rows: at 80x24 instructions
started at row 18 and debug at row 16, leaving a 2-row step across the middle of the frame.
Both columns are the same height, so one shared formula is enough to make the two bottom
panes equal by construction.

Done when:
- `src/layout.rs` has a single `BOTTOM: Share { divisor: 3, cap: 8 }` used by both
  `split_column` calls; `INSTRUCTIONS` and `DEBUG` are gone.
- `panes.instructions.height == panes.debug.height` and `panes.instructions.y ==
  panes.debug.y` at every size, asserted by a sweep over widths and heights (not just
  80x24).
- The height formula is unit-tested for a short, a 24-row, a 40-row and a 60-row terminal:
  one third of the body, floored at 3, capped at 8.
- `docs/spec.md` §4 agrees with the code: the drawing labels both bottom panes "~8" and
  the height bullet states `min(8, column / 3)`, minimum 3, capped at 8.
- `make check` is green.

## Notes
The spec change was approved by the human before the code change; §4 previously pinned
`min(5, 25% of column)` for instructions. Cost: the instructions pane gains 2 rows at 80x24
(5 -> 7) and 3 on a tall screen (5 -> 8), taken from the source pane.
