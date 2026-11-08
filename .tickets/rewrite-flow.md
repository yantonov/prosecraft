---
status: in_progress
priority: 2
deps: [clipboard, marker-extract]
attachments: []
created: 2026-11-07T15:33:47Z
updated: 2026-11-08T08:21:46Z
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

## Measured 2026-11-08 against the real claude binary

Ran the exact spec §6.2 argv end to end, five times. Four of five runs put the marker-wrapped
rewrite in the terminal event's `result` field, and in all four `result` was byte-identical to
the concatenated `text_delta`s (62/62, 84/84, 91/91 bytes). That confirms spec §6.1a's claim
that deltas are display-only and the terminal event is authoritative.

**One run out of five returned `"result":""` with `subtype: success` and `is_error: false`,
while the streamed deltas held a perfectly good marker-wrapped answer.** So an empty `result`
is a real failure mode, and it is indistinguishable from 'the model ignored the markers' by
the time extraction sees it. Both currently produce 'model did not use the output markers',
which is misleading for the empty case but is the strict behaviour §6.1a step 4 requires:
no fallback to the deltas, because a dropped delta silently truncating the rewrite is the
failure the whole design exists to prevent.

Worth knowing, not fixed here: if this turns out to be frequent rather than a one-off, the
fix is a distinct message ('the harness returned an empty result'), which needs no change to
the extraction rule. Recorded so the next person does not have to re-measure it.
