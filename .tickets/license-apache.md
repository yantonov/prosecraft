---
status: open
priority: 2
deps: [tui-dummy]
attachments: []
created: 2026-11-07T15:42:27Z
updated: 2026-11-07T15:43:43Z
---
# Apache 2.0 licence: Yury Antonov, 2026

## Description
Add the Apache License 2.0 to the project, held by Yury Antonov.

Done when:
- LICENSE exists at the repo root and contains the Apache License 2.0 text fetched from https://www.apache.org/licenses/LICENSE-2.0.txt (11358 bytes). Do not retype or reformat it: Apache 2.0 forbids changes to its own text, and a retyped copy drifts.
- Exactly ONE edit is made to the fetched text: the appendix placeholder line 'Copyright [yyyy] [name of copyright owner]' becomes 'Copyright 2026 Yury Antonov'. Every other byte stays as fetched. Verify with a diff against the canonical file that shows only that one line changed.
  (The appendix is illustrative boilerplate, not part of the licence terms, so editing it is normal practice and does not modify the licence. Do not touch the numbered clauses.)
- Year only, no day or month: a bare year needs no maintenance and Apache 2.0 does not require a date at all.
- Cargo.toml declares 'license = "Apache-2.0"' and 'authors = ["Yury Antonov"]', so metadata matches the file on disk.
- AGENTS.md (from harness-bootstrap) names the licence in its project section, so a reader learns it without opening LICENSE.
- No NOTICE file is needed, since the appendix carries the copyright. Add one only if a dependency later requires attribution.

Scope guard: exactly one LICENSE file plus Cargo metadata. No per-file licence headers in source files: 18 copies of the same paragraph that drift independently. No second licence, no licence section in README.
