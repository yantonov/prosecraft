---
status: in_progress
priority: 2
deps: [pane-routing]
attachments: []
created: 2026-11-07T15:33:47Z
updated: 2026-11-08T07:28:50Z
---
# Marker extraction and error states

## Description
Extract <<<TEXT>>>..<<<END>>> per docs/spec.md §6.1a, plus the error paths.

Done when:
- Text between the markers becomes the output, for both 'markers on their own lines' (claude) and the single-line form '<<<TEXT>>>text<<<END>>>' (pi, measured).
- Markers are found as plain substrings; no requirement for newlines, indentation, or whole-line matches.
- Everything outside the markers is discarded, including leaked <thinking>, <reasoning_effort>, <thinking_mode> tags that claude was measured emitting before the start marker.
- A missing start or end marker sets state 'error', shows the raw terminal text, and never writes the output pane. No fallback to trimming or preamble-stripping.
- A tool-invocation block (<invoke name=...>) inside the extracted text sets state 'error' and is shown.
- Internal newlines and blank lines inside the markers are preserved verbatim; only surrounding whitespace is trimmed.
- No markdown unwrapping and no fence stripping after extraction: a fence *inside* the markers is part of the text.
- Exit 0 with no terminal event (no 'result' / no assistant message_end) sets state 'error' rather than falling back to concatenated deltas.
- Table tests cover each row of the spec's measured marker table, plus: markers reversed, only start marker, only end marker, empty content between markers, and markers nested/duplicated.
