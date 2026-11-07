# AGENTS.md

## Project
A TUI that rewrites a block of text with an externally installed LLM harness (`claude` or
`pi`) run in one-shot, non-interactive mode: paste text on the left, set instructions,
press one key, read the result on the right. It exists so an iterative rewrite costs one
keypress instead of a chat window.
Stack: Rust 2024 (toolchain 1.99.1), ratatui 0.30.2 + crossterm 0.29.0, no database, no
cache, no persistence.
Licence: Apache 2.0, held by Yury Antonov. The full text is in `LICENSE`; there are no
per-file headers, deliberately.

Design source: `docs/spec.md`. It is the contract every ticket is written against.

## Starting a session
Work lives in `bl` tickets under `.tickets/`; the committed `backlog.md` is generated from
them, so the tracker stays in the repo and never drifts from the code.
- `bl available` — claimable tickets (open, deps satisfied)
- `bl started` — tickets in progress
- `bl show <id> --json` — one ticket in full
- `bl help` — the full command surface

## Commands
- Setup: `make setup` (cargo fetch, prints the active toolchain). There is no
  `rust-toolchain.toml`, so any cargo new enough for edition 2024 works.
- Run: `make run` (extra args: `make run ARGS="..."`)
- Tests: `make test`
- Lint: `make clippy` (clippy over code and tests, warnings denied)
- Format: `make fmt`
- **Full verification: `make check`** (fmt-check + clippy + test, then `ALL CHECKS PASSED`)

`make help` lists public targets; `make help-all` adds the internal ones.

## Definition of Done
A feature is done only when:
- `make check` is green, including tests that existed before this session.
- The feature's specific verification passes. Today that is unit tests plus `make run`:
  there is no E2E harness yet, and the spec's per-milestone checks in `docs/spec.md` §10
  are the manual script to follow.
- The ticket is closed with `bl done <id>`.

NOT done: "code is written", "looks right", "unit tests pass", "works on my machine".

## Hard constraints (MUST / MUST NOT)
Each of these was decided in `docs/spec.md` after measurement. Undoing one silently
restores a failure mode the design exists to prevent.

1. **`docs/spec.md` is frozen.** MUST NOT edit it without explicit human approval.
   Enforced by convention only — there is no pinning mechanism, no checksum, no CI guard.
   Reason: every ticket is written against it; a silent edit makes open tickets disagree
   with the design they were cut from.
2. **No filesystem side effects at runtime** (spec §1). MUST NOT add a state file, a
   config file, or a session pointer beyond what the harness itself stores. Reason: the app
   reads and writes the clipboard and talks to the harness, nothing else, so there is
   nothing to migrate, corrupt, or leak — and `Ctrl+N` is the only state reset needed.
3. **The harness binary is resolved at startup** (spec §3). A missing binary means a clear
   one-line error naming the expected binary on **stderr** and a non-zero exit, before raw
   mode and before the first draw. MUST NOT move this check later or turn it into a TUI
   error. Reason: an app whose only job is to spawn a process should not open a window it
   cannot use. `--help`/`--version` still work without a harness.
4. **Instructions travel in the user turn** (spec §6.0). MUST NOT make a system-prompt flag
   the sole transport of the instructions. Reason: measured that `claude --resume` silently
   ignores both `--system-prompt` and `--append-system-prompt`, so flag-only delivery would
   look like it worked and quietly do nothing on claude. `--system-prompt` is still passed
   on first call for register only; correctness never depends on it.
5. **Output pane gets final text only; the debug pane gets the stream** (spec §6.1b). MUST
   NOT write deltas, reasoning, stderr, or partial text into the output pane, and MUST NOT
   reconstruct output by concatenating deltas. Reason: measured that plain-text output
   buffers the entire answer (claude wrote nothing for 13 s), and that delta-splicing turns
   a single dropped delta into silent truncation. The output pane is written once per run,
   from the terminal event; the debug pane is the diagnosable view.
6. **The rewritten text is delimited by `<<<TEXT>>>`/`<<<END>>>`** (spec §6.1a). MUST treat a
   missing marker as an **error**, never as a guess. MUST NOT fall back to trimming,
   preamble-stripping, or markdown unwrapping. Reason: the instructions pane is free text
   and the user may ask for commentary, so no reliable boundary exists in prose; markers
   turn extraction into a protocol, and a missing marker means truncation, refusal, or a
   confused model — silently presenting that as the rewrite is the exact failure the
   markers remove.
7. **The terminal is restored on every exit path**, including panic (spec §8). MUST NOT add
   terminal setup that bypasses the existing restore hook / `Drop` pair in `src/main.rs`.
   Reason: a panic in raw mode strands the user's shell in the alternate screen.

## Work rules
- **WIP=1.** Claim one ticket at a time: `bl start <id>`, and close it with `bl done <id>`
  before claiming another.
- Stay inside the claimed ticket. Record anything else with `bl add "<title>"` and move on.
- Change only what the ticket names. Do not reformat, rename, or "tidy" unrelated code: it
  buries the real diff.
- Priority order: correctness → performance → style. Style is `make fmt`'s job, not a
  review topic.
- Let the ticket's `deps` decide order; if a dependency looks wrong, say so instead of
  working around it.

## Escalate to human (stop and ask)
- Editing `docs/spec.md` in any way (constraint 1).
- Adding a dependency to `Cargo.toml`, or anything that could touch the filesystem at
  runtime (constraint 2).
- Changing harness argv, the prompt template, the marker protocol, or anything in the
  measured-constraint sections of the spec — these are tied to observed harness behaviour.
- Licence decisions: the licence and per-file headers are their own ticket; do not add
  either while doing something else.
- Deleting or rewriting something that cannot be recovered.

## Details (read when needed)
- `docs/spec.md` — the design, and the measured evidence behind constraints 2–7. Read the
  relevant section before implementing a ticket that cites it; do not edit it.
- `.tickets/` — ticket bodies carry the acceptance criteria; `backlog.md` is generated
  from them, never hand-edited.
- `src/main.rs` — terminal ownership: raw mode, alternate screen, panic hook, restore-once
  guard. Read before touching startup or teardown.
- `src/ui.rs` — the current single centered frame and the quit key handling. This is
  scaffolding from `tui-dummy`; the four-pane layout replaces it in `layout-panes`.
- `Makefile` — target definitions; `check` is the single verification verdict.
