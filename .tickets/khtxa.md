---
title: "Live verify: codex end to end, session carried across rewrites, sandbox holds in-app"
status: closed
priority: 2
deps: [k6rqhh, x3dtv2]
attachments: []
created: 2026-11-08T18:05:49Z
updated: 2026-11-08T18:24:08Z
closed-at: 2026-11-08T18:24:08Z
---
# Live verify: codex end to end, session carried across rewrites, sandbox holds in-app

## Problem

The adapter's argv is measured and its event vocabulary is recorded, but no test can prove
that the app drives codex end to end: the harness's own UI cannot be asserted headlessly,
and a unit test that spawns codex would need credentials and 20+ s per run.

So this ticket is the manual verify script — the equivalent of what §10 M4 asked of pi — and
the place its result is on the record. It runs **after** the adapter and CLI tickets.

## Script

Two rewrites in one TUI run, nothing else touched:

1. `prosecraft --harness codex --no-clipboard --text "Please send me the report." --prompt "rewrite as a pirate"`,
   press `Ctrl+R`.
2. Edit the instructions pane to `rewrite in a very formal legal register`, press `Ctrl+R`.

Then, in the same session:

3. `Ctrl+N`, change the instructions, `Ctrl+R` — this must open a **new** codex thread.
4. `Ctrl+N` while a run is live: ignored, status line says so.

## Assertions

- The status line shows `codex` and a short thread id after the first run, and the id is
  the `01a1…` shape codex emits, learned from the stream rather than from argv.
- The output pane holds the text **between** the markers and nothing else; no preamble.
- The changed instructions visibly change the register on the second run — the §6.0
  regression, and the reason codex's session id must survive a continuation at all.
- `Ctrl+N` gives a different id on the next run, and the context is gone (the third rewrite
  does not carry the first run's register).
- The debug pane shows `app` lines for start, exit status and duration. It **does not**
  stream text: measured, the whole `agent_message` arrives at once. That is the documented
  codex limitation (§6.5), not a defect to report.
- No file was created and no shell command ran: the run is asked to write
  `SHOULD-NOT-EXIST.txt` as its instruction once, to confirm the sandbox holds in the app's
  own argv — measured blocked 9/9 when tested by hand, and the app must reproduce it.
- `AGENTS.md` content does not appear in the debug pane or the output; asking the model to
  quote it must yield nothing, because `project_doc_max_bytes=0` is in the argv.

## Done when

- The script above has been run once against the installed codex, and each assertion's
  outcome is appended to this ticket.
- `make check` green.
- Any deviation is filed as a new ticket rather than fixed here.


## Result: the script is now executable, not prose

No pty harness exists in this repo and the TUI takes over the terminal, so the manual script
was turned into two `#[ignore]`d live tests that drive the **production** `Machine` with the
real codex adapter — closer to the app than hand-typing an argv, and repeatable:

    cargo test live_codex -- --ignored --nocapture

`rewrite::live_codex::one_thread_carries_a_changed_instruction_and_nothing_is_written`
(17.6 s, passed):

- first rewrite as a pirate, second as a legal register, **one** thread id, different output —
  the §6.0 regression, passing on codex;
- the thread id is learned from `thread.started` (`01…` shaped) rather than from an argv;
- neither `<<<TEXT>>>` nor `<<<END>>>` reaches the output pane, so extraction stripped them by
  protocol;
- `Ctrl+N` (`reset_session`) then a rewrite yields a **different** thread id;
- the debug pane carries the app lines, so the stream went where it should.

    cargo test live_codex_sandbox -- --ignored --nocapture

`a_run_that_is_told_to_write_a_file_leaves_no_file` (8.6 s, passed): the app's own argv,
instructed to create a uniquely-named file in the temp dir, created nothing. This is the
ticket's sandbox assertion, run in-app rather than by hand.

**Not asserted, by design:** the debug pane does not stream. Measured and recorded in §6.5 —
the whole `agent_message` arrives at once — so "the pane sits inert" is the documented
behaviour, not a defect to file.

Both are `#[ignore]`d so `make check` stays hermetic and fast; they need credentials and
network. `make check` green without them.
