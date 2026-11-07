# Development entry points for prosecraft.
#
# One tool (`make`) covers build, run and verification, so a new contributor or agent
# never has to guess cargo invocations. Help is written as explicit `@echo` lines rather
# than `grep`+`##` self-documenting comments: the public/internal split is a judgement
# call that belongs in prose, and `##` markers drift out of sync with the targets.

.PHONY: help help-all check build build-release install run test fmt clippy fmt-check setup clean

.DEFAULT_GOAL := help

CARGO ?= cargo

help:
	@echo "Usage: make [target]"
	@echo ""
	@echo "  check    - Full verification: format check + clippy (deny warnings) + tests"
	@echo "  build    - Debug build"
	@echo "  install  - Release build, copied to ~/.local/bin/prosecraft"
	@echo "  run      - Build and run the TUI (extra args: make run ARGS=\"...\")"
	@echo "  test     - Run the test suite"
	@echo "  fmt      - Apply rustfmt to the whole crate"
	@echo "  clippy   - Run clippy over the code and the tests"
	@echo ""
	@echo "Run 'make help-all' for the full list, including internal targets."

help-all:
	@echo "Public targets:"
	@echo "  check    - Full verification: format check + clippy (deny warnings) + tests"
	@echo "  build    - Debug build"
	@echo "  install  - Release build, copied to ~/.local/bin/prosecraft"
	@echo "  run      - Build and run the TUI (extra args: make run ARGS=\"...\")"
	@echo "  test     - Run the test suite"
	@echo "  fmt      - Apply rustfmt to the whole crate"
	@echo "  clippy   - Run clippy over the code and the tests"
	@echo ""
	@echo "Internal targets (typically called via check or install):"
	@echo "  build-release - Release build, the artifact install copies"
	@echo "  fmt-check     - Verify formatting without writing files"
	@echo "  setup         - Fetch dependencies and print the active toolchain"
	@echo "  clean         - Remove build artifacts (target/)"

# Prerequisites run in order and make stops at the first failure. That is deliberate:
# formatting noise should be cleared before type errors are read, and a red step is
# already a red verdict, so later steps have nothing to add to the exit code.
check: fmt-check clippy test
	@echo "ALL CHECKS PASSED"

build:
	@$(CARGO) build

# The release profile is a separate target because it produces a different artifact in a
# different directory; `build` stays the fast debug build the dev loop uses.
build-release:
	@$(CARGO) build --release

# Prerequisite, not a recipe line: make stops at the first failing prerequisite, so a
# broken build leaves the binary on PATH untouched instead of replacing it with a stale
# copy. The artifact path is the build-release output by construction.
install: build-release
	@mkdir -p "$$HOME/.local/bin"
	@cp target/release/prosecraft "$$HOME/.local/bin/prosecraft"
	@echo "Installed: $$HOME/.local/bin/prosecraft"

run:
	@$(CARGO) run -- $(ARGS)

test:
	@$(CARGO) test

# `--all` keeps the workspace case working if a member is added later; it is a no-op for
# the single crate today.
fmt:
	@$(CARGO) fmt --all

clippy:
	@$(CARGO) clippy --all-targets -- -D warnings

# Separate from `fmt` so `check` can never leave a dirty tree behind.
fmt-check:
	@$(CARGO) fmt --all -- --check

setup:
	@$(CARGO) fetch
	@echo "Dependencies fetched; toolchain: $$(rustc --version)"

clean:
	@$(CARGO) clean
