---
status: open
priority: 1
deps: [r26q4]
attachments: []
created: 2026-11-08T18:05:48Z
updated: 2026-11-08T18:05:53Z
---
# Wire --harness codex through the CLI, the adapter factory, and the startup lookup

## Problem

`--harness` accepts `claude` and `pi`. Adding a third adapter is worthless if it cannot be
selected, and the startup check has to keep resolving the binary before raw mode (AGENTS.md
constraint 3).

## Scope

Mechanical wiring only, no new logic:

- `cli::Harness` gains `Codex`, `binary_name()` returns `"codex"`, and clap's value enum
  accepts it (the `--harness nope` exit-2-with-usage test already covers the unknown case).
- `main::adapter_for` maps `Harness::Codex` to `harness::codex::Codex`.
- The resolver is untouched: measured, codex installs as
  `…/Programs/OpenAI/Codex/bin/codex.exe`, a real `.exe` on `PATH`, so `resolve::lookup`
  finds it through `PATHEXT` on Windows and as a bare executable on unix. A test pins that
  the resolver is asked for `"codex"`, not a path.
- Help text and the doc comments that enumerate the harnesses (`cli.rs`, `main.rs`,
  `harness/mod.rs`) say `claude`, `pi`, `codex`.
- `README.md`: the requirements bullet and the `--harness` examples gain codex.

## Done when

- `--harness codex` parses; `--harness nope` still exits 2 with usage.
- `Harness::Codex.binary_name() == "codex"`, and the startup lookup receives exactly that.
- A missing `codex` binary produces the one-line stderr error naming `codex` and exit 1,
  before raw mode — asserted by the existing missing-harness test parameterised over all
  three values.
- `main::adapter_for` returns the codex adapter for `Harness::Codex`, asserted by a test
  (the existing one already distinguishes claude from pi, so it extends).
- `make check` green.
