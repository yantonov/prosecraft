---
title: "Makefile install target: release build, stamp git hash, copy to ~/.local/bin"
status: closed
priority: 2
deps: []
attachments: []
created: 2026-11-07T17:15:31Z
updated: 2026-11-07T17:38:24Z
closed-at: 2026-11-07T17:38:24Z
---
# Makefile install target: release build, stamp git hash, copy to ~/.local/bin

## Description
Add `make install`: build release, copy the binary to `$HOME/.local/bin/prosecraft`. Also
stamp the commit hash into the binary from `build.rs`, so the installed `prosecraft` can say
which revision it is. Reference for both: ../catalog-cli (`install: build` -> `bin/install.sh`;
`git describe` -> `-ldflags -X ...version`).

Decisions taken before starting (do not reopen):
- Inline Makefile recipe. No `bin/` scripts: this repo's Makefile is already plain `cargo`
  invocations, and a copy is two lines.
- Release only. `make build` keeps meaning the debug build.
- `$HOME/.local/bin` hard-coded. No `INSTALL_DIR`/`PREFIX`/`DESTDIR` override.

## Part 1: install target

Done when:
- `make install` builds release and copies `target/release/prosecraft` to
  `$HOME/.local/bin/prosecraft`, mode 0755, overwriting any previous copy.
- One target does everything: `make install` alone is enough, no prior `make build`.
- `mkdir -p "$HOME/.local/bin"` runs before the copy, so it works where that directory does
  not exist yet.
- A failed build aborts before anything is copied: the copy must not run off a stale
  `target/release` artifact. A binary on PATH that silently runs the old code is worse than
  no install.
- Afterwards `prosecraft --version`, run from a directory that is not the repo, exits 0.
- `install` is in `.PHONY` and is listed in both `make help` and `make help-all`, in the
  existing single-line `@echo` style.
- `make check` is unchanged and does not run install: a verification target that writes
  outside the repo is not a verification target.
- `make install` works from a clean `target/`, and `make check` stays green.

## Part 2: commit hash in `--version`

Done when:
- `build.rs` at the crate root runs `git rev-parse --short HEAD` at build time and exposes
  the result through `cargo:rustc-env=...`; `src/cli.rs` uses it as clap's `version`. No new
  dependency: `std::process::Command` is enough.
- `prosecraft --version` prints the crate version plus the short hash, e.g.
  `prosecraft 0.1.0 (a1b2c3d)`, matching `git rev-parse --short HEAD` for the checkout that
  built it.
- A dirty worktree gets a `-dirty` suffix (`git describe --always --dirty`, or
  `git diff --quiet` in build.rs), so a build from uncommitted changes is not mistaken for a
  clean revision.
- `cargo:rerun-if-changed` is emitted for `.git/HEAD` and for the ref it points at, so a
  commit or a checkout rebuilds the stamp instead of freezing the first value seen.
- Git being absent, or the build running outside a checkout, is not an error: build.rs falls
  back to a constant and `build`, `make check` and `--version` all still succeed.
- build.rs adds no runtime behaviour (build time only), so spec §1 (no runtime filesystem
  side effects) is untouched and `docs/spec.md` needs no change.
- The hash appears nowhere but `--version`: not in the TUI, the status line, or any error
  message.

Verification: `make check` green; `make install`; `cd /tmp && prosecraft --version` matches
`git -C <repo> rev-parse --short HEAD`; commit something, `make install` again, new hash;
a build with `git` hidden from `PATH` still succeeds.

Scope guard: the Makefile `install` target, `build.rs`, the `version` attribute in
`src/cli.rs`, and the existing `-V` test kept passing. No `uninstall`, no shell completions,
no man page, no tarball, no CI release job, no `Cargo.toml` dependency, no `docs/spec.md`
edit.
