---
status: open
priority: 2
deps: [clipboard, marker-extract]
attachments: []
created: 2026-11-07T15:33:47Z
updated: 2026-11-07T15:36:19Z
---
# Rewrite validation and state machine

## Description
The validating/running/done/error/cancelled transitions from docs/spec.md §5, plus required validation.

Done when:
- Empty or whitespace-only instructions refuse the rewrite, show the reason in the status line, and move focus to the instructions pane.
- Empty source text refuses, shows the reason, and moves focus to the source pane.
- A harness binary that stopped resolving is refused at rewrite time too, not only at startup.
- Ctrl+R while running is ignored with a status-line note (spec §5: 'Pressing Ctrl+R ... is ignored, and the status line says so').
- Ctrl+N while running is ignored with a status-line note (spec §5, same sentence after the v0.7 decision).
- The four states are reachable and no others: idle -> running -> done | error | cancelled -> idle.
- Output pane is cleared on spawn, per spec §5 step 4.
- The state machine is tested headlessly with the terminal backend stubbed; no test opens a real TTY.

Note: timeout and Ctrl+C cancellation behaviour is robustness; here the 'cancelled' and 'error' states only need to exist and be reachable from a stubbed transition.
