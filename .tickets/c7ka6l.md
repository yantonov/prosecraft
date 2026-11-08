---
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-08T12:56:15Z
updated: 2026-11-08T12:57:38Z
closed-at: 2026-11-08T12:57:38Z
---
# README: why, install, usage examples

The repo has no README.md. Add one with a TOC and three sections, in this order:

1. Why: what the app is, the motivation (one keypress per rewrite instead of a chat window), and what it deliberately is not (no state, no history, no config).
2. Installation: concise - prerequisites (Rust toolchain, a resolved harness binary), `make install`, PATH note.
3. Usage: examples built on the `pi` harness and `--prompt` instructions such as "improve the style" and "make it informal".

Source material is docs/spec.md (read-only, constraint 1) and `prosecraft --help`. Do not edit the spec, Makefile or Cargo.toml.
