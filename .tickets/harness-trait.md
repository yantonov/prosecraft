---
status: closed
priority: 2
deps: [harness-bootstrap]
attachments: []
created: 2026-11-07T15:33:47Z
updated: 2026-11-07T22:22:31Z
closed-at: 2026-11-07T22:22:31Z
---
# Harness trait and claude argv builder

## Description
The Harness trait from docs/spec.md §6: name, new_session_id, build, classify, final_text. Pure functions only, no process spawning.

Done when:
- Task/Invocation/Event types exist as specified in §6, including Task.session: Option<&str> where None means 'first call of this run'.
- claude::build produces the exact argv from spec §6.2 for both first call (--session-id) and continuation (--resume), including --output-format stream-json, --verbose, --include-partial-messages, --tools "", --permission-prompts none, --system-prompt, and the '--' separator.
- claude's new_session_id returns a valid v4 UUID (spec §6.2: --session-id requires one; an invalid id makes --resume exit 1).
- classify labels text_delta as Text, thinking_delta as Think, signature_delta as Think (or its own kind — document the choice), and any non-JSON or unrecognised line as Junk. It must be impossible for anything but Text to be mistaken for output.
- final_text returns Some only for the 'result' event (type == "result", use the 'result' field) and None for every other line.
- Table tests assert exact argv for: first call, continuation, with --model, without --model, and with instructions containing newlines and a leading '-'.
- No process is spawned by any test.
