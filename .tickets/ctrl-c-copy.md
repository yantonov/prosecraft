---
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-08T12:30:16Z
updated: 2026-11-08T12:32:33Z
closed-at: 2026-11-08T12:32:33Z
---
# Ctrl+C copies; halting a run needs the debug pane and a confirmation

Ctrl+C currently carries two unrelated meanings: cancel a live run, or quit when idle. That
is confusing, and it makes the key every terminal user reaches for as *copy* do something
destructive instead. The rewrite gives it one meaning per pane.

New rule (spec §4; the spec edit needs human approval, per constraint 1):

- **Only the debug pane can halt.** It is the pane that shows the stream, so it is where
  stopping a run belongs. Its Ctrl+C cancels a live run — after a confirmation prompt, the
  same shape as Ctrl+Q while running. When nothing is running there is nothing to halt, so
  it falls through to the copy rule below.
- **Every other pane copies through the path Ctrl+Y already uses**, and so does debug when
  idle: source copies the source, instructions the instructions, output the rewritten text,
  debug the debug text. Ctrl+Y stays, because Ctrl+C in a running debug pane does not copy.
- **Ctrl+C never quits.** Ctrl+Q is the only quit, and it already confirms while a run is live.

The confirmation is a second press, and it is what teaches the two-step key: the status line
says so. Nothing is silently destructive.

Done when:
- In source, instructions and output, Ctrl+C copies that pane's text and returns
  `Outcome::Continue`; nothing is cancelled and nothing quits, in every `State`.
- An empty focused pane makes Ctrl+C a `<pane> is empty` status note with no clipboard write,
  the behaviour Ctrl+Y has today. Both keys go through one helper so they cannot drift.
- In debug while `State::Running`, the first Ctrl+C arms a cancel and writes a status-line
  prompt naming Ctrl+C; the child keeps running. The second Ctrl+C cancels and does not quit.
  Disarming follows Ctrl+Q's existing rule — other keys do not disarm it, and the flag is
  cleared when the run settles, so a new run cannot inherit an armed cancel.
- In debug while `Idle` or `Error`, Ctrl+C copies the debug text like any other pane.
- `ctrl_c_quits_only_when_nothing_is_running` and `ctrl_c_when_idle_quits_with_no_prompt` are
  replaced: a per-pane copy test, a debug two-press cancel test, and a test that idle Ctrl+C
  in source copies and the outcome stays `Continue`.
- `HELP_ROWS` and the spec §4 key table agree on the new Ctrl+C row, and the Ctrl+Q row says
  it is the only quit. The F1 panel is the key table's on-screen copy, so both change together.
- docs/spec.md §4's Ctrl+C row is rewritten. Nothing else in the spec documents Ctrl+C except
  §5's cancelled transition and §10's M5 line, both of which stay true.
- make check is green.