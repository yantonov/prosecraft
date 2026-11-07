---
status: in_progress
priority: 2
deps: [tui-dummy]
attachments: []
created: 2026-11-07T15:33:44Z
updated: 2026-11-07T21:40:39Z
---
# Status line and quit paths

## Description
The bottom status/help line and the exit paths, per docs/spec.md §4 and the §4 key table.

Status line order is fixed by spec §4: harness name, session id (short form, an emdash '—' before the first rewrite), state ('idle' / 'running' / 'error'), then the key hints.

Done when:
- Status line shows harness name, session id, state and key hints in that order.
- State is one of idle / running / error; the cancelled outcome returns to idle with a status-line note.
- Ctrl+Q quits from any focus state. While a rewrite is running, Ctrl+Q asks for confirmation (the confirm UI itself is robustness; here the status line must be able to display the prompt).
- Ctrl+C quits only when idle (spec §4: 'If running: cancel the harness. Otherwise: quit' — the cancel half is robustness).
- Terminal is restored on normal quit, on error, and on panic (verified by deliberately panicking).
- Esc does nothing (spec decision, recorded in the Decisions section).
- Status-line messages are single-line and clipped with an ellipsis rather than wrapping, so a long error cannot push the layout around.
- Exit code 0 on normal quit, 1 on runtime failure, 2 on CLI misuse (spec §8).

Scope: no harness calls. State transitions are faked for this ticket.
