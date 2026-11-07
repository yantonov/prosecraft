---
status: open
priority: 3
deps: [session-lifecycle]
attachments: []
created: 2026-11-07T15:33:50Z
updated: 2026-11-07T15:36:53Z
---
# Robustness: cancel, timeout, resize, error paths

## Description
Harden the paths that only show up under failure, per docs/spec.md §7 and §8.

Done when:
- Ctrl+C during a run kills the harness, leaves the output pane as cleared on spawn, and sets state 'cancelled' (spec §5, §7).
- The 180s timeout kills and reports; the timeout is injectable so the test runs fast.
- Every row of the spec §8 error table is either covered by a test or has a dated manual-verification note in the ticket.
- Terminal resize during a run does not panic and does not lose the debug tail position.
- The debug pane ring buffer trims at 2000 lines and still auto-tails correctly after trimming.
- A harness that writes a non-JSON line to stdout mid-stream is labelled 'junk' and does not abort the run (spec §6.1a rule 6).
- Ctrl+Q while a rewrite is running asks for confirmation before quitting (spec §4: 'Quit; confirm if a rewrite is running'). Declining leaves the run untouched and still streaming.
- Ctrl+Q while idle quits immediately with no prompt.
