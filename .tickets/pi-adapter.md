---
status: open
priority: 3
deps: [harness-trait, session-lifecycle]
attachments: []
created: 2026-11-07T15:33:50Z
updated: 2026-11-07T15:36:22Z
---
# pi adapter

## Description
Second Harness implementation, proving the trait per docs/spec.md §6.3.

Done when:
- build produces the exact argv from spec §6.3: --print, --session-id, --mode json, --system-prompt, --no-tools, --no-context-files, --no-extensions, --no-mcp, --no-skills, --no-prompt-templates, --offline, then '--' and the user prompt.
- new_session_id returns psc-<uuid>, satisfying pi's rule that ids accept [A-Za-z0-9._-] and start and end with a letter or digit.
- classify handles message_update -> assistantMessageEvent.type text_delta (Text) and thinking_delta (Think); anything unparseable is Junk.
- final_text returns Some only for the last message_end with role == 'assistant'.
- argv table tests mirror the claude ones exactly, so the two adapters cannot drift.
- A unit test asserts --no-context-files is present, because without it pi injects the user's AGENTS.md into the system message (measured: 2759 bytes) and a rewrite would silently inherit coding guidelines.
- One live run confirms a changed instruction takes effect on the same session id, and that the same argv shape works for first call and continuation (pi's --session-id means create-or-open).
