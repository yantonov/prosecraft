---
title: "Four-pane layout and focus cycling"
status: closed
priority: 2
deps: [args-cli, tui-dummy]
attachments: []
created: 2026-11-07T15:33:44Z
updated: 2026-11-07T16:46:20Z
closed-at: 2026-11-07T16:46:20Z
---
# Four-pane layout and focus cycling

## Description
Render the four panes from docs/spec.md §4: source and instructions in the left column, output and debug in the right. 50/50 split, exact columns and order as drawn in the spec.

Done when:
- All four panes draw with a title and a visible border, in spec order: source (top-left), instructions (bottom-left), output (top-right), debug (bottom-right).
- Tab and Shift+Tab cycle focus in reading order across all four panes (source, instructions, output, debug); Ctrl+Left/Ctrl+Right jump to the other column, preserving row.
- The focused pane is visually distinct: border style change for read-only panes, cursor for editable ones.
- Instructions pane height is min(5, 25% of column), minimum 3 lines, computed and asserted for tiny and tall terminals.
- Output and debug panes each keep their own scroll offset field (rendering is wired later).
- Resizing the terminal redistributes panes with no panic; minimum supported size is 1x1 without panic.
- Panes never truncate text: content soft-wraps (wrapping itself is editor-wrap's job; here the layout must give each pane the right inner width to wrap to).
- Layout is unit-tested via ratatui TestBackend at several fixed sizes: 80x24, 40x10, 1x1.
