---
status: open
priority: 2
deps: [harness-trait]
attachments: []
created: 2026-11-07T15:33:47Z
updated: 2026-11-07T15:36:14Z
---
# Process spawn and stream readers

## Description
Spawn the harness, pipe stdout/stderr, drain both on threads into an mpsc channel. Per docs/spec.md §7.

Done when:
- A fake binary (test fixture script, checked into the repo) that echoes fixed stdout/stderr is fully captured, line by line, with the stream labelled.
- The UI loop never blocks: events are drained per tick, not per line. A test proves that a slow producer does not stall the tick loop.
- Non-UTF-8 bytes are converted lossily, no panic.
- A binary that exits non-zero reports the exit status AND its stderr, without losing already-received stdout.
- A binary that never exits is killed at the timeout (180s default, injected/overridden in the test so the test is fast) and reported as timeout.
- Cancellation kills the child, and state becomes 'cancelled' with the output pane left as it was cleared on spawn (spec §7: nothing to reconcile).
- Killing a process that ignores a polite signal still terminates (escalation or documented platform behaviour).
- The reader threads terminate when the child exits; no thread leak across repeated runs (assert thread count or that all senders drop).

Note: fixture is a shell script; Windows portability is an explicit non-goal (spec §2).
