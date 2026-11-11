---
title: "CLI args and startup validation"
status: closed
priority: 0
deps: [tui-dummy]
attachments: []
created: 2026-11-07T15:33:44Z
updated: 2026-11-07T16:37:13Z
closed-at: 2026-11-07T16:37:13Z
---
# CLI args and startup validation

## Description
Parse the CLI from docs/spec.md §3 with clap derive, and validate before any terminal work.

Flags: --harness (required), --prompt, --prompt-file, --text, --text-file, --no-clipboard, --model, -h, -V.

Done when:
- '--help' and '-V' work with no harness installed (they return before the binary check).
- '--harness nope' exits 2 with usage.
- '--prompt X --prompt-file Y' exits 2 (mutually exclusive).
- A missing --harness binary prints one line naming the expected binary to stderr and exits non-zero, with the terminal never entering raw mode.
- '--text-file missing.txt' and '--prompt-file missing.txt' both fail with a clear stderr message and non-zero exit.
- --prompt / --prompt-file are read once into the instructions pane at startup and never re-read (spec §3: no reload hotkey).
- '--text' beats '--text-file'; both beat the clipboard.
- --no-clipboard suppresses the startup clipboard read; the app still starts.
- --model is accepted and stored for pass-through; it changes no UI.
- Unit tests cover each of the above as pure arg-validation cases.

Out of scope: actually reading the clipboard (that is clipboard), and spawning anything.
