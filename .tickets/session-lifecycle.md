---
title: "Session id lifecycle and Ctrl+N"
status: closed
priority: 2
deps: [rewrite-flow]
attachments: []
created: 2026-11-07T15:33:50Z
updated: 2026-11-08T08:42:30Z
closed-at: 2026-11-08T08:42:30Z
---
# Session id lifecycle and Ctrl+N

## Description
The session id lifecycle per docs/spec.md §5 step 3 and the Decisions list: created lazily on the first rewrite, reused for every later rewrite, discarded by Ctrl+N.

Done when:
- No id exists until the first Ctrl+R; the status line shows an emdash before that.
- The first rewrite generates a v4 UUID (claude form, spec §6.2) and the second rewrite passes it as --resume, not --session-id.
- Ctrl+N clears the id, the output pane and the debug pane, and keeps source and instructions.
- The next rewrite after Ctrl+N creates a fresh id and uses --session-id again.
- Status line shows the current short id.
- One session per TUI run, always (spec §5 step 3). No id is ever persisted to disk (spec §1 non-goal: no state file).
- Unit tests cover the lifecycle with the harness invocation intercepted; no process spawned.
