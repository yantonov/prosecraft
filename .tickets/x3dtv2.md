---
title: "codex adapter: exec/exec resume argv, agent_message terminal text, thread id from the stream"
status: closed
priority: 1
deps: [r26q4, v2ffc]
attachments: []
created: 2026-11-08T18:05:42Z
updated: 2026-11-08T18:22:20Z
closed-at: 2026-11-08T18:22:20Z
---
# codex adapter: exec/exec resume argv, agent_message terminal text, thread id from the stream

## Problem

There is no codex adapter. This ticket adds `src/harness/codex.rs` as a pure adapter —
argv, `classify`, `final_text`, `session_from_event` — with no process and no terminal, so
every measured fact from §6.5 can be asserted as a table row.

## argv (both call shapes, from spec §6.5 once v2ffc lands)

First call:

    codex exec --json
      -c sandbox_mode="read-only" -c approval_policy="never"
      -c project_doc_max_bytes=0 -c features.apps=false -c web_search="disabled"
      --skip-git-repo-check
      [-m <model>]
      -- -

Continuation:

    codex exec resume --json
      <same -c block and --skip-git-repo-check> [-m <model>] <thread-id> -- -

Why each flag, all measured on codex-cli 0.159.0:

- `exec` / `exec resume` — the one-shot form and its continuation subcommand.
- `--json` — the only machine-readable mode; `exec resume` needs the same flag.
- `-c sandbox_mode="read-only"` **and** `-c approval_policy="never"` — together they
  blocked every write attempt (9/9). Either alone does not: `-s read-only` with approvals
  on-request wrote the file 3/3, and `-c sandbox_mode` alone did too.
- `-c project_doc_max_bytes=0` — without it the user's `AGENTS.md` is reproduced verbatim
  in the prompt. codex's `--no-context-files`.
- `-c features.apps=false` — 147 tools offered become 29; without it the whole Codex app
  MCP surface is in the prompt.
- `-c web_search="disabled"` — drops the web search tool.
- `--skip-git-repo-check` — measured: without it codex refuses to run outside a git repo
  with `Not inside a trusted directory`.
- `-c` rather than `-s`/`--ignore-user-config` — measured: `exec resume` rejects both
  (`unexpected argument`), and `--ignore-user-config` disables the user's model config and
  changed sandbox behaviour. Using `-c` keeps one flag shape for both call shapes.
- `-- -` — the prompt goes on stdin (see below), so the positional is `-`, after `--` so a
  future flag cannot be mistaken for it.

## Prompt transport: stdin, unlike the other two adapters

Measured: codex reads the prompt from stdin when no positional is given or the positional
is `-`. Both transports round-trip the real template with newlines and markers intact.

Stdin is chosen for two reasons: `argv` on Windows is the same batch-shim hazard §6 already
documents, and stdin keeps the prompt — user prose — out of the OS argument list entirely.
`Transport` therefore does **not** apply to this adapter: it always sets `Invocation.stdin`
and never appends the prompt to `args`, whatever the resolution says. Spec §6.5 must state
that this is a per-adapter choice, not a platform rule.

## Event vocabulary (spec §6.1a, codex column)

| what | line |
|---|---|
| session id | `{"type":"thread.started","thread_id":…}` |
| streamed prose | **none** — codex emits no `text_delta` in `exec --json` |
| reasoning | **none emitted**, though tokens are spent |
| terminal text | last `item.completed` with `item.type == "agent_message"`, field `item.text` |
| tool-related items | `command_execution`, `file_change`, `web_search`, `mcp_tool_call` |
| failure | `turn.failed` → non-zero exit |

`classify` labels `agent_message` items `Text` only when `item.type == "agent_message"` and
a string `text` is present; tool items are `Junk`; everything else is `Junk`. `final_text`
returns the `text` of an `agent_message` item, and only that — never a tool item's output.

## Done when

- `src/harness/codex.rs` implements the adapter; `src/harness/mod.rs` declares it.
- argv table tests mirror the claude and pi tables: first call, continuation, with and
  without `--model`; the `-c` block asserted in order and once each; `--skip-git-repo-check`
  present; `stdin` set and no prompt element in `args` for every row.
- A test asserts `--ignore-user-config`, `-s`, and `--ignore-user-config`-style flags are
  **absent**, because a future "tidy" that switches to them would silently re-open writes.
- `new_session_id()` returns `None`, asserting that codex names its own threads.
- `session_from_event` returns the `thread_id` for a `thread.started` line and `None` for
  every other line, including other events that carry an `id` field (`item.*` do).
- `classify`/`final_text` rows cover: agent message, agent message without a string text,
  `command_execution`, `file_change`, `web_search`, `mcp_tool_call`, `turn.completed`,
  `turn.failed`, `thread.started`, non-JSON, and a truncated line — each mapping to the kind
  the table above names and none panicking.
- A test asserts the prompt template is byte-identical to claude's and pi's, per §6.4.
- `make check` green; live run recorded in the ticket.

## Progress note

The adapter is written and its 18 unit tests pass. `Codex` is not yet constructed anywhere,
so `cargo build` reports `dead_code` for it and `OVERRIDES`; that clears in k6rqhh, which
wires `--harness codex` to `adapter_for`. Leaving the ticket open until the live run and
`make check` are both recorded.


## Live run, recorded on codex-cli 0.159.0

Ran the adapter's exact argv by hand — the same flags the const produces, the prompt on
stdin via `-- -` — so the evidence covers the argv and not a hand-typed variant.

**Both call shapes, one thread:**

    thread_id 01a1bc64-7d1d-7a81-a785-16b8c63ebaba
    call 1 (pirate): <<<TEXT>>>Arrr, send me the report, matey.<<<END>>>
    call 2 (legal):  <<<TEXT>>>You are hereby requested to furnish the undersigned with the
                     aforementioned report.<<<END>>>

The id came from `thread.started` on stdout and the same id carried into `exec resume`, so
the changed instructions took effect on one thread — the §6.0 regression, passing.

**Markers with preamble requested** (the case the protocol exists for): instructions asked for
a short explanation first; the reply was `<<<TEXT>>>...<<<END>>>` with nothing outside the
markers, so extraction yields clean text and no commentary-stripping heuristic is involved.

**Denial, through the app's own argv:**

| attempt | result |
|---|---|
| write `PROBE-OUT.txt` | `access denied because the filesystem is read-only`, no file created |
| quote the user's `AGENTS.md` | `NONE` |
| search the web | `NO-WEB-ACCESS` |

**Startup:** `--harness codex --version` exits 0 with no harness needed; with `PATH` pointed
at an empty directory the app prints `harness binary codex not found on PATH` and exits 1
before any terminal setup.

**Not asserted, by design:** the debug pane does not stream. The whole `agent_message`
arrives at once (§6.5), which is the harness's behaviour and is recorded in the spec rather
than treated as a defect.

`make check` green: 465 unit tests plus 7 integration tests.
