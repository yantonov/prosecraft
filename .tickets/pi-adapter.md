---
status: closed
priority: 3
deps: [harness-trait, session-lifecycle]
attachments: []
created: 2026-11-07T15:33:50Z
updated: 2026-11-08T09:56:02Z
closed-at: 2026-11-08T09:56:02Z
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

## Live run, 2026-11-08, pi 1.5 on this machine

Ran the exact spec §6.3 argv twice against one session id:

    id psc-d90852c8-ae54-4a90-9038-7b06a094f602
    call 1 (formal):   <<<TEXT>>>Please send the file.<<<END>>>
    call 2 (informal): <<<TEXT>>>Send the doc when you can.<<<END>>>

Both halves of the criterion hold: the same id creates then continues, and the changed
instructions take effect on the continuation. That is the behaviour §6.0 measured and the
reason instructions travel in the user turn.

**Two measured shapes that differ from §6.3's prose**, both now handled and tested:

1. A text delta carries its payload in `assistantMessageEvent.delta` as a **plain string**.
   claude nests it as `delta.text`, so the two adapters cannot share a JSON path even
   though they mean the same thing.
2. The terminal event is `message_end` whose `message.content` is an **array of blocks**,
   not a `result` string. The answer is the `text` block; a `thinking` block sits beside
   it in the same array and must not be concatenated in. Multiple `text` blocks are joined,
   since a multi-block answer is still one answer.

Observed event types in one run: session, agent_start, turn_start, message_start,
message_end, message_update, turn_end, agent_end, agent_settled. Only `message_update`
with a text/thinking delta is labelled, and only the assistant `message_end` is terminal;
everything else is junk and shown as such.
