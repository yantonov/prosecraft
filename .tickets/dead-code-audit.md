---
status: open
priority: 3
deps: []
attachments: []
created: 2026-11-08T10:20:00Z
updated: 2026-11-08T10:20:00Z
---
# Re-audit the dead_code allowances now the backlog is closed

## Description
Six `dead_code` allowances were added while the tickets that would consume each API were
still open. All of those tickets have now closed. Three of the six comments still name
`rewrite-flow`, `pane-routing` or `process-spawn` as future work, which is no longer true —
so the comments are stale and the allowances may be hiding nothing.

Measured 2026-11-08 by stripping every allowance and running
`cargo clippy --all-targets -- -D warnings`. **Four symbols are genuinely unused**; the other
allowances were suppressing nothing:

| Location | Symbol | Why it is unused |
|---|---|---|
| `src/process.rs:135` | `Run::spawn` | Production calls `spawn_with_timeout` directly, because the machine injects a timeout. `spawn` is a two-line convenience wrapper nothing uses. |
| `src/rewrite.rs:155` | `Machine::with_timeout` | Used only by tests. Production has no `--timeout` flag to wire it to. |
| `src/rewrite.rs:155` | `Machine::with_spawner` | Test-only constructor, by design. |
| `src/wrap.rs:250` | `wrap::cursor_position` | `WrapState::render` computes the same thing internally; nothing calls the free function. |
| `src/wrap.rs:322` | `WrapState::scroll` | `scroll_to_cursor` and `page` cover every caller; nothing reads the offset. |

## Done when
- Every `dead_code` allowance in `src/` is either gone or carries a reason that is still true.
  In particular the three stale comments naming closed tickets are corrected or deleted.
- The five symbols above are each resolved by one of the two rules `editor-dead-code`
  established: **wire it** if the app already duplicates the behaviour, otherwise **delete
  it**. Not by re-adding the attribute.
- Specifically, decide and record:
  - `Run::spawn`: either delete the wrapper (production already uses the timeout form) or
    make it the only public entry point and pass the default explicitly. One of the two, not
    both.
  - `wrap::cursor_position`: delete it, or make `WrapState::render` call it so there is one
    implementation of "where is the cursor" rather than two that can drift.
  - `WrapState::scroll`: delete it unless something is about to read the offset.
  - `Machine::with_timeout` and `with_spawner`: keep with a test-only comment naming the
    tests, or delete if the timeout is wired to a flag.
- `make check` green with no new allowances, and the test count does not fall: a deletion that
  takes tests with it is a deletion of specification, which is the mistake `editor-dead-code`
  was written to avoid.

## Method that worked last time
`editor-dead-code` found that **two of five** "unused" methods already had hand-rolled
duplicates at the call site. Check for that before deleting: grep the call site for the
behaviour, not the name. A `#[allow]` with a stale reason is worse than no attribute, because
it reads as a decision someone made.
