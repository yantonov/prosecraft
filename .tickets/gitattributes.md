---
status: closed
priority: 3
deps: [tui-dummy]
attachments: []
created: 2026-11-07T17:03:36Z
updated: 2026-11-07T22:53:38Z
closed-at: 2026-11-07T22:53:38Z
---
# Pin LF line endings with .gitattributes

## Description
Every commit of a .rs file prints 'warning: in the working copy of X, CRLF will be replaced by LF the next time Git touches it'. The repo has no line-ending policy, so line endings depend on each contributor's core.autocrlf and on which editor last touched a file.

Done when:
- .gitattributes exists at the repo root.
- '*.rs text eol=lf' is set, so Rust sources are LF on disk in the worktree on every platform. rustfmt and clippy do not care, but a mixed-ending file makes diffs show whole-file rewrites.
- '*.md text eol=lf' is set for the spec, AGENTS.md and the tickets. Cards, docs and backlog are generated and compared as text.
- 'Cargo.lock text eol=lf' is set, so the lockfile does not churn.
- Binary-ish and generator files are marked correctly: 'target/' is already gitignored, so nothing is needed for build output.
- Committing after the change produces no CRLF warning for any file in the repo.
- No source file content changes: 'git diff --stat' after the renormalisation shows only line-ending normalisation, and 'make check' stays green.

Method note: adding .gitattributes alone does not renormalise files already committed with CRLF. Use 'git add --renormalize .' and commit that separately from the attribute file if it produces a large diff, so the policy change is reviewable on its own.
