---
title: "Robustness: cancel, timeout, resize, error paths"
status: closed
priority: 3
deps: [session-lifecycle]
attachments: []
created: 2026-11-07T15:33:50Z
updated: 2026-11-08T09:31:05Z
closed-at: 2026-11-08T09:31:05Z
---
# Robustness: cancel, timeout, resize, error paths

## Description
Harden the paths that only show up under failure, per docs/spec.md §7 and §8.

Done when:
- Ctrl+C during a run kills the harness, leaves the output pane as cleared on spawn, and sets state 'cancelled' (spec §5, §7).
- The 180s timeout kills and reports; the timeout is injectable so the test runs fast.
- Every row of the spec §8 error table is either covered by a test or has a dated manual-verification note in the ticket.
- Terminal resize during a run does not panic and does not lose the debug tail position.
- The debug pane ring buffer trims at 2000 lines and still auto-tails correctly after trimming.
- A harness that writes a non-JSON line to stdout mid-stream is labelled 'junk' and does not abort the run (spec §6.1a rule 6).
- Ctrl+Q while a rewrite is running asks for confirmation before quitting (spec §4: 'Quit; confirm if a rewrite is running'). Declining leaves the run untouched and still streaming.
- Ctrl+Q while idle quits immediately with no prompt.

## §8 error table audit, 2026-11-08

Every row is covered by a test. Named so the mapping does not have to be rebuilt:

| §8 condition | Test |
|---|---|
| Harness binary not found | `tests/cli.rs::missing_harness_exits_one_with_one_stderr_line_and_no_terminal_output` |
| Empty instructions | `ui::tests::ctrl_r_with_empty_instructions_is_refused_and_focuses_the_instructions_pane` |
| Empty source text | `ui::tests::ctrl_r_with_empty_source_is_refused_and_focuses_the_source_pane` |
| Non-zero exit | `ui::tests::a_failed_run_moves_to_error_with_its_reason`, `rewrite::tests::a_non_zero_exit_reaches_error_and_leaves_the_output_pane_empty` |
| Timeout | `rewrite::tests::a_run_that_never_ends_is_killed_at_the_injected_timeout` (real subprocess, 200ms injected) |
| Clipboard unavailable | `ui::tests::a_clipboard_failure_is_a_status_note_and_the_app_keeps_working` |
| Non-UTF-8 output | `process::tests::invalid_utf8_is_converted_lossily_and_the_reader_keeps_going` |
| Markers honoured | `panes::tests::the_output_pane_holds_the_final_text_after_a_successful_run` |
| Missing output markers | `panes::tests::a_marker_failure_is_an_error_and_never_writes_the_output_pane` |
| Tool-call XML between markers | `panes::tests::a_tool_call_inside_the_markers_is_an_error_that_shows_the_call` |
| Exit 0 but no terminal event | `panes::tests::exit_zero_with_no_terminal_event_is_an_error_not_a_concatenation` |

**Not testable, by design:** 'session id missing/invalid with --resume'. The spec calls it
unreachable: the id is internal state generated before first use, so there is no path that
sends a resume without one. `rewrite::tests::the_second_rewrite_resumes_the_session_the_first_one_created`
asserts the id on the resume flag matches the one the first call created, which is the
nearest observable property. A debug assert was considered and skipped: the id comes from
the adapter's own argv, so the assert could only fire on a bug the type system already
prevents.
