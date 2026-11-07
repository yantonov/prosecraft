---
status: open
priority: 2
deps: [editor-wrap, process-spawn]
attachments: []
created: 2026-11-07T15:33:47Z
updated: 2026-11-07T15:36:18Z
---
# Output pane from final text, debug pane from stream

## Description
Wire the stream into the panes per docs/spec.md §5 and §6.1b.

Done when:
- Every stdout line lands in the debug pane labelled 'text' / 'think' / 'junk' per classify; every stderr line as 'err'; app-generated lines as 'app'.
- The output pane is cleared on spawn and written exactly once, from final_text, on exit 0.
- Reasoning lines never reach the output pane (asserted by a test that feeds a thinking_delta and checks the output pane is untouched).
- The output pane keeps the last good result between runs and is never partially written: an assertion that no intermediate state is rendered.
- The debug pane auto-tails while the user has not scrolled up, and holds its offset once they have, with a visible 'not tailing' marker.
- PgUp/PgDn scroll the focused read-only pane (output or debug) by a page, per spec §4 key table.
- Run start, exit status and duration appear as app lines.
- Rendering is batched per UI tick, not per event: a test feeding 1000 events causes one redraw, not 1000.

Note: Ctrl+Y copies from either pane — that is clipboard's ticket. This ticket only renders content and owns scroll offsets.
