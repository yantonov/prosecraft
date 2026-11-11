---
status: in_progress
priority: 2
deps: []
attachments: []
created: 2026-11-11T14:05:00Z
updated: 2026-11-11T13:17:54Z
---
# Strip progress from the spec, and bring it in line with the shipped app

Approved spec edit (AGENTS.md constraint 1). No code.

`docs/spec.md` has been a working document that accumulated its own history: a
draft version number, a `[Q]` marker convention whose `#open-questions` anchor
never existed, an "M1..M6 milestones" section with M6 written twice, and several
places where the text narrates a decision instead of stating it ("this is why the
trait changed", "This corrects an earlier assumption", "Settled since v0.1").

It also drifted from the code in ways a reader would not catch:

- section 1 promised only `claude` and `pi`; codex ships
- section 9 listed `uuid` and `anyhow`, neither of which is in `Cargo.toml`, and
  a `src/app.rs` that does not exist, while omitting `cli.rs`, `panes.rs`,
  `marker.rs`, `rewrite.rs` and `harness/resolve.rs`
- the section 6 trait sketch had no `session` or `transport` on `Task` and a
  `build` signature that takes the session as a second parameter
- section 8 said a missing session id is a `debug_assert` that does not exist,
  and contradicted section 7 on what happens when a harness never announces an id
- the section 4 frame drew a `process` pane title and a `source` status field;
  the code draws `debug` and the harness name
- section 6.1a step 4 cited "constraint 5", which is AGENTS.md numbering, not
  spec numbering

## Done when

- Every statement is a decision, the measurement behind it, or a manual check.
  No draft number, no dead anchor, no milestone roadmap, no narration of how a
  decision changed.
- Section 9 names the crates and the modules that exist.
- Section 10 (was "Milestones") holds the same checks as the manual script,
  once each.
- The trait sketch, the frame drawing, the status-line fields and the error
  table match `src/`.
- `make check` is green. Nothing in `src/` changes behaviour.

## Scope note

The measured-constraint sections (6.0, 6.1a, 6.1b, 6.5) keep every number and
every argv. Rewording is allowed; a changed measurement is not. AGENTS.md gets
the same treatment, because Definition of Done points at the section this
ticket renames.
