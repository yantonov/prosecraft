---
title: "lzmlv"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-08T10:45:48Z
updated: 2026-11-08T10:46:11Z
closed-at: 2026-11-08T10:46:11Z
---
# lzmlv
## Description
`make help` currently lists `test`, `fmt` and `clippy` next to `check`. They are
components of `check`, and `check` is the single verification verdict (AGENTS.md). Listing
them as peers invites running one green step and calling it verification, which is the
failure the `check` target exists to prevent.

Decision taken before starting (do not reopen):
- Keep the three targets runnable and in `.PHONY`; only move their `@echo` lines from the
  `help` block to the internal block of `help-all`. No behaviour change.
- Public stays: `check`, `build`, `install`, `run`.

## Done when
- `make help` lists check, build, install, run, and nothing else.
- `make help-all` lists those four as public, then build-release, fmt-check, test, fmt,
  clippy, setup, clean as internal.
- `make test`, `make fmt`, `make clippy` still run unchanged.
- `make check` unchanged and green.
