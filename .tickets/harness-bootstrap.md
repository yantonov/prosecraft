---
status: closed
priority: 0
deps: [tui-dummy]
attachments: []
created: 2026-11-07T15:33:41Z
updated: 2026-11-07T15:56:35Z
closed-at: 2026-11-07T15:56:35Z
---
# Bootstrap harness: AGENTS.md + Makefile

## Description
Apply the minimal-harness skill to this repo.

Done when:
- AGENTS.md exists at repo root, <=150 lines, every template section filled or marked TODO.
- Makefile exists with help/help-all plus public targets; 'check' runs fmt-check + clippy + test and prints ALL CHECKS PASSED.
- 'make check' exits 0 on the dummy app from tui-dummy.
- Cold start test: reading only AGENTS.md answers what the project is, how to run it, how to verify it, which rules are hard, and where work lives.

AGENTS.md must record these spec decisions so a later reader cannot undo them by accident:
- docs/spec.md is frozen; do not edit it without explicit human approval. Enforced by convention only; no pinning mechanism.
- No filesystem side effects at runtime: no state file, no config file, no session pointer (spec §1).
- Harness binary is resolved at startup; missing means stderr error and non-zero exit (spec §3).
- Instructions travel in the user turn, never as the sole transport (spec §6.0 — measured: claude ignores --system-prompt on resume).
- Output pane gets final text only; the debug pane gets the stream (spec §6.1b).
- The rewritten text is delimited by <<<TEXT>>>/<<<END>>> markers; a missing marker is an error, not a guess (spec §6.1a).
