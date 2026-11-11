# AGENTS.md

## Project
A TUI that rewrites a block of text with an externally installed LLM harness (`claude`, `pi`,
or `codex`) in one-shot, non-interactive mode. Paste text on the left, set the instructions,
press one key, read the result on the right. The app exists so that one iterative rewrite
costs one keypress instead of a chat window.
Stack: Rust 2024 (toolchain 1.99.1), ratatui 0.30.2 + crossterm 0.29.0, no database, no
cache, no persistence.
Licence: Apache 2.0, held by Yury Antonov. The full text is in `LICENSE`. There are no
per-file headers, on purpose.

The design source is `docs/spec.md`, and every ticket follows it.

## Starting a session
Work lives in `bl` tickets under `.tickets/`. The committed `backlog.md` is generated from
those tickets, so the tracker stays in the repo and never drifts from the code.
- `bl available` — claimable tickets (open, deps satisfied)
- `bl started` — tickets in progress
- `bl show <id> --json` — one ticket in full
- `bl help` — the full command surface

## Commands
- Setup: `make setup` (cargo fetch, then prints the active toolchain). There is no
  `rust-toolchain.toml`, so any cargo new enough for edition 2024 works.
- Run: `make run` (extra args: `make run ARGS="..."`)
- Tests: `make test`
- Lint: `make clippy` (clippy over the code and the tests, warnings denied)
- Format: `make fmt`
- **Full verification: `make check`** (fmt-check, clippy, test, then `ALL CHECKS PASSED`)

`make help` lists the public targets. `make help-all` adds the internal ones.

## Definition of Done
A feature is done only when all three hold:
- `make check` is green, including the tests that existed before this session.
- The feature's own verification passes. That is the unit tests plus `make run`, because
  the project has no E2E harness. The checks in `docs/spec.md` §10 are the manual script.
- You closed the ticket with `bl done <id>`.

A feature is not done when the code exists, when it looks right, when the unit tests pass,
or when it works on one machine.

## Hard constraints
`docs/spec.md` settles each constraint below by measurement. A change that ignores one
restores a failure mode that the design prevents.

1. **`docs/spec.md` is frozen.** Do not edit it without explicit human approval. A
   convention enforces this: there is no pinning mechanism, no checksum, no CI guard.
   Reason: every ticket follows the spec, so a silent edit makes the open tickets disagree
   with the design they came from.
2. **No filesystem side effect at runtime** (spec §1). Do not add a state file, a config
   file, or a session pointer beyond what the harness itself stores. Reason: the app reads
   the clipboard, writes the clipboard and talks to the harness, and it does nothing else.
   So there is nothing to migrate, corrupt or leak, and `Ctrl+N` is the only state reset
   that the app needs.
3. **The app resolves the harness binary at startup** (spec §3). A missing binary produces a
   one-line error that names the expected binary on **stderr**, then a non-zero exit, before
   raw mode and before the first draw. That check stays at startup and stays out of the TUI.
   Reason: an app whose only job is to spawn a process does not open a window it cannot use.
   `--help` and `--version` work with no harness.
4. **The instructions travel in the user turn** (spec §6.0). A system-prompt flag never
   carries the instructions alone. Reason: measured, `claude --resume` silently ignores both
   `--system-prompt` and `--append-system-prompt`, so flag-only delivery looks correct and
   does nothing on claude. The app still passes `--system-prompt` on the first call, for
   register only, and correctness never depends on it.
5. **The output pane receives final text only, and the debug pane receives the stream**
   (spec §6.1b). Deltas, reasoning, stderr and partial text go to the debug pane. The app
   builds output from the terminal event and never from concatenated deltas. Reason:
   measured, plain-text output buffers the whole answer (claude wrote nothing for 13 s), and
   delta-splicing turns one dropped delta into silent truncation.
6. **The rewritten text sits between `<<<TEXT>>>` and `<<<END>>>`** (spec §6.1a). A missing
   marker is an **error**, never a guess. No fallback to trimming, preamble-stripping or
   markdown unwrapping. Reason: the instructions pane is free text and the user may ask for
   commentary, so prose offers no reliable boundary. The markers turn extraction into a
   protocol. A missing marker means truncation, a refusal, or a confused model, and showing
   that text as the rewrite is the exact failure that the markers remove.
7. **The app restores the terminal on every exit path**, including a panic (spec §8). New
   terminal setup goes through the existing restore hook and `Drop` pair in `src/main.rs`.
   Reason: a panic in raw mode strands the user's shell in the alternate screen.

## Work rules
- **WIP=1.** Claim one ticket at a time with `bl start <id>`, and close it with `bl done <id>`
  before you claim another.
- Stay inside the claimed ticket. Record anything else with `bl add "<title>"` and continue.
- Change only what the ticket names. Leave unrelated code, names and formatting alone,
  because a tidy-up buries the real diff.
- Priority order: correctness, then performance, then style. `make fmt` owns style.
- The ticket `deps` decide the order. When a dependency looks wrong, say so instead of
  working around it.

## Escalate to human (stop and ask)
- A change to `docs/spec.md`.
- A new dependency in `Cargo.toml`, or anything that could touch the filesystem at runtime.
- A change to the harness argv, the prompt template, the marker protocol, or any
  measured-constraint section of the spec. Measurement ties these to observed harness
  behaviour.
- A licence decision. The licence and the per-file headers have their own ticket, so do not
  add either while you do something else.
- The deletion or rewrite of anything that cannot be recovered.

## Details (read when needed)
- `docs/spec.md` — the design and the measurements behind constraints 2 to 7. Read the
  section a ticket cites before you implement it. Do not edit the file.
- `.tickets/` — each ticket body carries its acceptance criteria. Tooling generates
  `backlog.md` from the tickets, so never edit it by hand.
- `src/main.rs` — terminal ownership: raw mode, alternate screen, panic hook, restore-once
  guard. Read it before you touch startup or teardown.
- `src/ui.rs` — the four-pane frame, key routing, status line and help panel. Read it before
  you add a key or a pane.
- `Makefile` — target definitions. `check` is the single verification verdict.
