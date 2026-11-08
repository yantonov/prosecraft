---
status: closed
priority: 0
deps: []
attachments: []
created: 2026-11-08T10:58:45Z
updated: 2026-11-08T10:58:47Z
closed-at: 2026-11-08T10:58:47Z
---
# Keys are drawn only when a tick reports a change, so nothing typed shows

Found by running the app: typing into source or instructions showed no characters, Ctrl+R produced no visible response, and Tab changed focus invisibly, so the app looked dead.

Cause: View::handle_key mutated the view but nothing told View::tick, and tick_loop draws only when tick returns true. tick returned true only for streaming or a finished run, so every key press reached the screen only if a rewrite happened to be running.

Fixed in this session: View::dirty is set on every key press and taken by View::tick. Regression tests at both levels: one per symptom in ui::tests, and one through tick_loop in main::tests.

Remaining: the app has no E2E harness, so this was caught only by hand. Whether a pty-level check belongs in tests/ is still open.

Closed in the same session it was filed: the fix is View::dirty (src/ui.rs), verified by make check and by one real claude run confirming the harness path was never the problem (exit 0, 30 stream_event lines, result = '<<<TEXT>>>\nGood day.\n<<<END>>>').
