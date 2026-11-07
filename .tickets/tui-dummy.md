---
status: in_progress
priority: 0
deps: []
attachments: []
created: 2026-11-07T15:33:41Z
updated: 2026-11-07T15:45:24Z
---
# Dummy empty TUI app

## Description
Cargo project that opens a ratatui terminal and draws a single bordered frame with a static label.

Done when:
- 'cargo run' draws a frame and stays interactive.
- 'Ctrl+Q' exits with code 0 and the terminal is usable afterwards (no raw-mode leak).
- A panic still restores the terminal (panic hook installed before the first draw).
- Scrolls nothing, calls no harness, holds no state beyond quit.

Scope guard: no panes (one frame only), no args, no clipboard. This ticket exists to prove terminal setup/teardown and nothing else. It deliberately violates nothing in the spec because it implements no spec feature.

Note: this writes the terminal-ownership code that args-cli later moves behind argument parsing. That duplication is intended: the shortest path to a proven clean exit is a window that does nothing else.
