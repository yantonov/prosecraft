---
title: "Re-audit the dead_code allowances now the backlog is closed"
status: closed
priority: 3
deps: []
attachments: []
created: 2026-11-08T10:20:00Z
updated: 2026-11-08T10:40:52Z
closed-at: 2026-11-08T10:40:52Z
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

## Resolved 2026-11-08: zero allowances remain, test count unchanged

All four module-level allowances removed; both targeted ones in rewrite.rs and the one in
ui.rs replaced. **No allowance survives**, and the test count is unchanged at 398 unit + 7
integration, so nothing was deleted to make the lint quiet.

Only five symbols were ever genuinely unused, and each was resolved without suppressing a
warning:

| Symbol | Resolution |
|---|---|
| `Run::spawn` | `#[cfg(test)]` — production always injects a timeout, and 8 tests use the default form rather than each repeating `DEFAULT_TIMEOUT` |
| `Machine::with_timeout` | `#[cfg(test)]` — no `--timeout` flag exists; the timeout tests drive the production `start` path |
| `Machine::with_spawner` | `#[cfg(test)]` — test constructor by design |
| `wrap::cursor_position` | `#[cfg(test)]` — in-module callers already hold the rows they computed, so they call `project` directly |
| `WrapState::scroll` | `#[cfg(test)]` — kept as a reader rather than exposing the field, so a test cannot write past the clamping |

Four stale comments also named closed tickets as future work (`process-spawn`, `rewrite-flow`,
`pane-routing`, and three others). All corrected: they now say what is true rather than what was
expected to happen.

**The pattern worth keeping:** `#[cfg(test)]` is the honest marker for "exists for tests".
It is not the same as `#[allow(dead_code)]`, which says "exists for nothing yet and I am not
saying why". The first is checkable, the second rots.
