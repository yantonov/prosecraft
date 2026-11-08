---
status: open
priority: 2
deps: [x3dtv2]
attachments: []
created: 2026-11-08T18:05:49Z
updated: 2026-11-08T18:05:53Z
---
# README: codex as a third harness, and the two ways it is not equal

## Problem

`README.md` documents two harnesses in three places: the requirements bullet ("One harness
installed and authenticated by you: `claude`, or `pi`"), the `--harness` examples, and the
Windows shim note. A third harness is selectable but undocumented until this lands.

## Scope

- Requirements: name all three, and say what codex needs — a logged-in `codex` on `PATH`,
  and nothing else; prosecraft still manages no credentials.
- `--harness` examples: at least one `--harness codex` invocation, so a reader sees it is a
  first-class choice and not an alias.
- A short **Harness differences** subsection, honest about the two ways codex is not equal:
  - it does not stream, so the process pane fills in jumps rather than continuously, and
    the status line's `running` is the progress signal;
  - its tools cannot be removed the way `claude --tools ""` and `pi --no-tools` remove
    them: codex runs sandboxed instead (read-only filesystem, approvals never), which the
    spec records as a deliberate, weaker guarantee. Do not describe it as "tools disabled".
- The Windows paragraph stays true: codex delivers the prompt on stdin on every platform,
  so the batch-shim hazard does not apply to it. Say so in one sentence.
- Keep the README's existing voice: short paragraphs, concrete examples, no bullet-point
  wall. It is the front door, not the spec — link to `docs/spec.md` §6.5 rather than
  restating the measurements.

## Done when

- `README.md` names all three harnesses everywhere it enumerates them.
- The differences subsection exists and states the sandbox-not-no-tools caveat without
  claiming parity.
- No measurement tables are copied into the README; they live in the spec and the ticket.
- `make check` green (touched no code, so this is only a guard).
