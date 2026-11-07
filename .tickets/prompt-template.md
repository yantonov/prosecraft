---
status: open
priority: 2
deps: [rewrite-flow]
attachments: []
created: 2026-11-07T15:33:50Z
updated: 2026-11-07T15:41:37Z
---
# Prompt template and instruction delivery

## Description
The fixed template from docs/spec.md §6.1, and the §6.0 decision that instructions travel in the user turn.

Done when:
- The user-turn text is exactly, in order: 'INSTRUCTIONS:' line, instructions verbatim, blank line, 'TEXT TO REWRITE:' line, source verbatim, blank line, then the marker demand line: 'Return only the rewritten text, wrapped between the markers <<<TEXT>>> and <<<END>>>. Put nothing else between the markers. Anything outside the markers is ignored.'
- The marker demand is the LAST thing in the prompt (spec §6.1: models weight the tail most heavily).
- The same template is used for the first call and for continuations, and for both harnesses.
- Instructions also reach --system-prompt on the first call, for register only. Nothing depends on it (spec §6.0: claude ignores it on resume; pi honours it).
- A golden test asserts the template byte-for-byte for a sample instructions and source, including a source with newlines.

Known limitation, documented not fixed: if the SOURCE text itself contains '<<<TEXT>>>' or '<<<END>>>', extraction finds the first pair and may cut the rewrite short. Accepted risk: rewriting text that discusses these markers is unlikely, and if it happens the result is visible in the output pane and the text can be edited externally afterwards. Do not escape or rewrite the source: a silent transformation of the user's input is worse than the edge case. Add a comment recording this decision.
