---
title: "Resolve the harness program once, identically for the startup check and the spawn"
status: closed
priority: 1
deps: []
attachments: []
created: 2026-11-08T11:30:39Z
updated: 2026-11-08T11:39:22Z
closed-at: 2026-11-08T11:39:22Z
---
# Resolve the harness program once, identically for the startup check and the spawn

## Measured problem

The startup check and the spawn disagree about what "found" means, so the app opens a
window for a program it cannot run. Spec §3 and AGENTS.md constraint 3 both require the
one-line stderr error *before* raw mode; today that guarantee holds only on unix.

Measured on this machine (Windows, git bash and cmd.exe give the same answer because
`PATH` reaches the process already in Windows form and `Command` never goes through a
shell):

| call | result |
|---|---|
| `Command::new("pi")` | `NotFound` — "program not found" |
| `Command::new("pi.cmd")` | runs, prints `1.5.0` |
| `Command::new("claude")` | runs (`~/.local/bin/claude.exe`) |

`C:\dev\devtools\nvm\nodejs` holds `pi` (a POSIX shell script), `pi.cmd`, `pi.ps1`, and no
`pi.exe`. Three separate defects follow:

1. `executable_names` appends `PATHEXT` to the *bare* name and so probes `pi` before
   `pi.CMD`. The extensionless `pi` is a regular file, and on non-unix `is_executable_file`
   returns `true` for any regular file, so the check passes on a file `CreateProcess` will
   never run.
2. `CreateProcess` does not apply `PATHEXT`; it appends `.exe` only. So the spawn cannot
   find `pi`, and `pi.cmd` is only reachable if named explicitly.
3. Therefore a Windows user with the shim installed gets a TUI, a `Ctrl+R`, and then
   `could not start the harness: program not found` from `src/rewrite.rs:250` — the failure
   constraint 3 exists to prevent.

Note the third leg is *not* fixed by preferring `.cmd`: naming `pi.cmd` still leaves the
argv defect, which is [[glrti]].

## Scope: one resolver, platform rules kept apart

Extract the lookup into its own component (its own module, e.g. `src/harness/resolve.rs`)
and have both callers use it: the startup check in `cli::assemble` and the spawn path.
Nothing else in the app should know about platform lookup rules.

- The resolver answers one question — "what program do I hand to `Command` for this
  harness name, or why can there be none" — and returns the concrete program string, not a
  bool. A bool is what let the two callers disagree.
- Per-platform behaviour lives in one `cfg` block per platform, not one rule set applied
  everywhere. Unix: a `PATH` entry with the name and the execute bit. Windows: `PATHEXT`
  in `PATHEXT` order (so `.exe`/`.cmd` are preferred over the extensionless shim), and a
  name that resolves to a batch shim is reported as such rather than silently returned.
- No new dependency (Cargo.toml changes are an escalation).

## Done when

- One function decides the program, and both the startup check and `Machine::start` use
  its result. A test proves the two cannot disagree: for a fixture directory holding only
  `pi` and `pi.cmd`, the startup verdict and the spawned `program` are the same answer.
- On Windows, a directory holding `pi` (plain file) and `pi.cmd` resolves to `pi.cmd`.
  Tested with `PATHEXT` passed in, not read, so the test cannot race the process env.
- Extensionless `pi` on Windows is never selected when a `PATHEXT` candidate exists.
- `prosecraft --harness pi` on a machine where the only candidate is a batch shim exits 1
  with one line on stderr naming the binary, and draws nothing. If [[glrti]] has landed,
  the shim is supported instead and this row is the non-shim case.
- Unix behaviour unchanged: `lookup_in`, the execute-bit rule, and the separator rule keep
  their existing tests green, and no windows-only branch is compiled on unix.
- `make check` green.

## Out of scope

- Carrying the prompt to a batch shim (argv cannot hold a newline; that is [[glrti]]).
- `.ps1` shims, `PATHEXT` entries other than `.exe`/`.cmd`, and any process-group work.

## Done

`src/harness/resolve.rs` now owns the lookup, with one `cfg` body per platform:

- `lookup_in(path_var, pathext, name) -> Resolved` and the injected-environment form the
  tests use. `Resolved` is `Program { program }`, `Missing`, or `ShimUnsupported`; only the
  first names a spawnable file, and `program()` is the single verdict both callers read.
- Windows: `PATHEXT` candidates in `PATHEXT` order, `.exe`/`.com` are programs, `.cmd`/`.bat`
  are `ShimUnsupported`, and an extensionless file is never a program. Unix: the bare name
  and the execute bit; nothing appends an extension.
- `Machine` holds the resolved `PathBuf` and overrides `invocation.program` at the one place
  an invocation becomes a process, so the startup verdict and the spawn are one answer.
- `Error::HarnessMissing` carries the `Resolved`, so the message says "not found on PATH"
  or "only installed as a Windows batch shim", one line either way.

Verified on this machine:

    $ ./target/debug/prosecraft.exe --harness pi --no-clipboard
    harness binary `pi` is only installed as a Windows batch shim (a runnable .exe is
    missing), which this build cannot drive
    exit=1, stdout empty

`--harness claude` still opens the TUI and draws the four panes.

Tests: 422 unit + 7 integration. The regression test
`the_spawned_program_is_the_resolved_path_not_the_harness_name` was confirmed to fail with
the override removed (`left: ["claude"]`, `right: ["/opt/bin/claude.exe"]`) and pass with it
restored. `make check` green.

Not fixed, and not this ticket: `pi` still cannot be driven on Windows even when named,
because a batch shim cannot carry the newline-bearing prompt in argv. That is [[glrti]],
blocked on approval to amend spec §6.
