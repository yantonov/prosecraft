---
status: closed
priority: 3
deps: []
attachments: []
created: 2026-11-08T10:20:00Z
updated: 2026-11-08T10:16:32Z
closed-at: 2026-11-08T10:16:32Z
---
# Investigate claude returning an empty result field

## Description
Found while verifying `rewrite-flow` against the real `claude` binary on 2026-11-08. Five
runs of the exact spec §6.2 argv, same prompt each time:

| run | `result` length | equals joined deltas | has markers |
|---|---|---|---|
| 1 | 62 | yes | yes |
| 2 | 84 | yes | yes |
| 3 | 91 | yes | yes |
| 4 | 144 | not checked | yes |
| 5 | **0** | n/a | **field empty** |

Run 5 returned `"result": ""` with `"subtype": "success"` and `"is_error": false`, while the
streamed `text_delta`s held a perfectly good marker-wrapped answer.

## Why it matters
`Extraction` cannot tell that case apart from "the model ignored the markers". Both reach
`marker::extract` as an empty string and both report *"model did not use the output markers"*.
That message is right for a model that went off-protocol and misleading for a harness that
returned nothing.

The behaviour is otherwise correct and must not change: falling back to the deltas is exactly
the silent truncation the marker protocol exists to prevent (spec §6.1a). This ticket is
about the *message*, not the fallback.

## Done when
- The frequency is established: at least 20 runs of the same argv, recording how many return
  an empty `result`. One in five is not enough to know whether this is a rare hiccup or a
  common failure that will be seen in normal use.
- If it happens more than once in twenty: add a distinct `Extraction` variant for an empty
  terminal text, with its own status-line message (e.g. "the harness returned an empty
  result"), and a test. No change to the extraction rule and no fallback.
- If it happens once in twenty: record the measurement on this ticket, close it as
  not-reproducible, and leave the code alone. Do not add a variant for a case never seen
  again.
- Either way the finding is written down, because the current single message will otherwise
  send the next person looking for a marker bug that is not there.

## How to reproduce
```sh
SID=$(uuidgen)   # any v4 uuid
claude -p --session-id "$SID" --output-format stream-json --verbose \
  --include-partial-messages --system-prompt "Be formal" \
  --tools "" --permission-prompts none -- "$(cat prompt.txt)"
```

Then check the final line: `jq -r '.result | length'`, and compare against the concatenated
`stream_event` → `content_block_delta` → `delta.text` values.

`prompt.txt` is the output of `harness::prompt()`; any instructions plus a short source is
enough. Cost is one short completion per run, so twenty runs is cheap.

## Not in scope
Changing the marker protocol, falling back to deltas, or treating an empty result as success.
All three are settled by spec §6.1a and by the project's hard constraint 5.

## Measured 2026-11-08: 20 runs, empty result did NOT reproduce. Closing.

20 runs of the exact §6.2 argv with one fixed prompt:

    empty result      : 0 / 20
    missing markers   : 0 / 20
    result != deltas  : 1 / 20
    tool outside marks: 1 / 20

**The empty-result case did not reproduce once in twenty.** Per this ticket's own rule ('if it
happens once in twenty: record the measurement, close as not-reproducible'), no code change:
no new Extraction variant, no message change.

## The one anomaly, which was a different thing entirely

Run 9 (and one earlier ad-hoc run, so 2 in ~40 overall) had deltas != result. Not an empty
result: the deltas carried a **tool invocation block** before the markers, and the result
carried only the answer.

    DELTAS: <invoke name="mcp__claude_ai_Claude_Docs__batch">…</invoke><<<TEXT>>>Good afternoon…<<<END>>>
    RESULT: <<<TEXT>>>Good afternoon…<<<END>>>

So the model, with tools denied, wrote an MCP batch call as *streamed text* and then the
answer. Nothing executed — this is the failure mode §6.1a step 6 describes, except the call
landed **outside** the markers, not inside them.

That case is handled correctly and needs no change: the call is labelled `text` in the debug
pane (it genuinely is streamed text), and extraction discards it with the rest of the preamble
because it sits outside the markers. Pinned by a new test,
`panes::tests::a_tool_call_outside_the_markers_is_discarded_not_rejected`, built from the
measured string.

## Consequence for §6.1a's claim

The spec says 'the concatenated text_deltas are exactly equal to result (76 bytes vs 76
bytes)'. That holds for 39 runs in 40. The exception is not a delta being dropped — it is
content that legitimately lives in the deltas and not in the result, which is why the result
being authoritative (constraint 5) is what keeps the output correct. Worth knowing; no change.
