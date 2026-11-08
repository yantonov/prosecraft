---
status: closed
priority: 2
deps: [o4snh]
attachments: []
created: 2026-11-08T11:30:39Z
updated: 2026-11-08T11:45:28Z
closed-at: 2026-11-08T11:45:28Z
---
# pi on Windows: deliver the prompt on stdin when the harness resolves to a batch shim

## Measured problem

Even with the resolver of [[o4snh]] naming `pi.cmd`, the prompt cannot travel in argv.
`cmd.exe` re-parses a batch file's command line, and Rust refuses the argument before
that (`CreateProcess` rejects a newline in a batch argv: `InvalidInput: batch file arguments
are invalid`). The prompt template always contains newlines and `<<<TEXT>>>`, so this hits
every run, not an edge case.

Measured, spawning `pi.cmd` directly with the prompt as one argument:

| prompt arg | result |
|---|---|
| `hello world`, `say "hi"`, `a ^ b`, `it's fine`, `100% sure` | ok |
| any newline (including the real template) | `InvalidInput: batch file arguments are invalid` |
| `a & b`, `a \| b` | mangled / run as a second command |
| `<<<TEXT>>>...<<<END>>>` | `<< was unexpected at this time.` |
| via `cmd /c pi.cmd ...` instead of a direct spawn | prompt truncated at the first newline: pi received `INSTRUCTIONS:` alone |

And the same prompt **on stdin** round-trips verbatim through the `pi.cmd` shim:

```
printf "INSTRUCTIONS:\nbe formal\n\nTEXT...<<<TEXT>>>hello there<<<END>>>" | pi --print --mode json ...
```

measured: 39 lines of JSON, exactly one assistant `message_end`, and `agent_end` shows the
user turn with all newlines and both markers intact. A full run through
`--session-id psc-... {--mode json,--system-prompt,--no-*,--offline}` on stdin also emits a
clean terminal `message_end`, so classification and extraction are unaffected.

`src/process.rs` already supports this: `Invocation.stdin` is written on a thread when it
is `Some`, and the template is sent as an argument only because both harnesses happen to
accept it that way.

## Tension with the frozen spec — needs a decision before implementing

`docs/spec.md` §6 says: *"Both harnesses take the prompt as an argument rather than
stdin: prompts may contain `-` and quoting is one implementation instead of two."* This
ticket changes that for one adapter on one platform. Per AGENTS.md constraint 1 and the
escalation list, **this cannot be implemented without explicit human approval to amend
§6**; the ticket is filed so the measurement is on the record and the approval has
something concrete to look at.

The narrowest wording that keeps §6 true on unix and macos: on Windows, when the resolved
program is a batch shim, the prompt is written to stdin instead of passed as an argument;
argv is then identical on both platforms apart from the omission of `--` and the prompt.

## Done when

- (After approval) the pi adapter sets `stdin` and drops the positional prompt when, and
  only when, the resolved program is a batch shim on Windows. On unix and macos the argv
  is byte-identical to today, proven by the existing table tests staying unchanged.
- The claude adapter is untouched: `claude` resolves to a real `.exe` here.
- `--` and the prompt are omitted together or passed together; never a lone `--`.
- One live run on Windows with `--harness pi` completes: prompt arrives verbatim (assert
  on the `agent_end` user turn in a fixture transcript), one terminal `message_end`, and
  the output pane gets the text between the markers.
- The argv table tests grow a stdin row instead of a speculative one: the existing rows
  assert `stdin == None`, so a new row states the shim case explicitly.
- `make check` green.

## Blocked on

Human approval to amend spec §6. Do not start before it.

## Done

Spec §6 amended (with approval): the argv-only sentence now carries a **Windows exception**
paragraph explaining why a batch shim cannot take the prompt in argv, what is measured, and
that unix/macOS/claude argv is unchanged. `Invocation.stdin`'s comment in the struct sketch
records the two meanings.

Implementation:

- `Resolved` lost `ShimUnsupported` and gained `Program { program, batch }`; a batch shim is
  now a *supported* program. `needs_stdin_prompt()` is the one predicate, so `cli::assemble`
  and `Machine` read the same fact.
- `Task` gained `transport: Transport::{Argv, Stdin}`, decided by `Machine` from `batch` and
  never by the caller, because it is a fact about the resolved program.
- pi adapter: on `Stdin` it omits `--system-prompt` **and** the `--` separator, and sets
  `stdin` to the prompt. Measuring turned up the `--system-prompt` half: the instructions
  pane is free text, and a newline there fails the same way (`InvalidInput`). Dropping it
  costs nothing because §6.0 already makes it register-only.
- claude adapter returns `BuildError` on a non-argv transport rather than silently losing
  the prompt. It resolves to a real `.exe` on every platform, so this is a guard, not a path.
- `Config` carries `harness_is_batch` beside `harness_program`.

Verified live on this machine through the real `pi.cmd`, with the exact argv the adapter now
produces (prompt on stdin, no `--`, no `--system-prompt`):

    exit 0
    terminal message_end: :"<<<TEXT>>>Good day. How are you?<<<END>>>"
    prompt arrived verbatim on stdin: true

And `./target/debug/prosecraft.exe --harness pi --no-clipboard` now opens the TUI instead of
exiting 1 — the refusal from [[o4snh]] is gone because a shim is no longer undrivable.

Tests: 424 unit + 7 integration. `make check` green.

Not done: no live `Ctrl+R`-in-the-TUI run was captured — the fixture-level round trip above
plus the machine-level transport test cover the same argv, but the manual script in spec §10
has not been walked for pi on Windows.
