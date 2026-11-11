# prosecraft

A terminal rewrite bench: paste text on the left, type how you want it rewritten, press one
key, read the result on the right.

- [Why](#why)
- [Installation](#installation)
- [Usage](#usage)

## Why

Rewriting prose with an LLM normally means a chat window: retype the text or paste it,
restate the instruction, scroll past the commentary, copy the answer out. The instruction
and the text are the only two inputs that change, and the answer is the only output that
matters — so the round trip is mostly overhead.

prosecraft collapses it to one keypress. The screen has four panes:

```
┌ source ────────────────────────┐┌ output ────────────────────────┐
│  the text to rewrite           ││  the rewritten text            │
├ instructions ──────────────────┤├ process ───────────────────────┤
│  how to rewrite it             ││  live stream, for debugging    │
└────────────────────────────────┘└────────────────────────────────┘
  pi │ 9f1c4a2b │ idle          [Ctrl+R] rewrite  [F1] help  [Ctrl+Q] quit
```

`Ctrl+R` sends the two editable panes to an installed harness (`claude`, `pi`, or `codex`)
running in one-shot, non-interactive mode. Edit the instructions, or the text, and press
`Ctrl+R` again: the harness session carries the conversation, so "shorter", then "less
formal" builds on the previous answer. The output pane only ever shows the final text; the
process pane shows the stream, the reasoning and stderr, so a slow run is visible instead of
inert.

What it deliberately is not:

- **No state.** Nothing is persisted — no history, no config file, no session pointer beyond
  what the harness itself keeps. `Ctrl+N` resets the harness context and is the only state
  reset there is.
- **No filesystem side effects.** It reads and writes the clipboard and talks to the
  harness. That is all.
- **No chat view, no redo, no model picker.** The harness and the model are chosen on the
  command line and fixed for the run. `Ctrl+Z` undoes edits in the two editable panes; that
  is the whole history surface, and it is capped.
- **No guessing.** The harness is told to wrap the answer in `<<<TEXT>>>`/`<<<END>>>`
  markers. Missing markers are an error, not a preamble to strip.

## Installation

Requirements:

- A Rust toolchain new enough for edition 2024 (there is no `rust-toolchain.toml`, so any
  recent stable works).
- One harness installed and authenticated by you: `claude`, `pi`, or `codex`. prosecraft
  does not manage credentials, models, or provider config.

```sh
git clone git@github.com:yantonov/prosecraft.git && cd prosecraft
make setup      # fetch dependencies, print the toolchain
make install    # release build, copied to ~/.local/bin/prosecraft
```

`make install` copies the binary to `~/.local/bin`; make sure that directory is on your
`PATH`. To try it without installing, use `make run ARGS="..."`.

`make help` lists the public targets (`check`, `build`, `install`, `run`), `make help-all`
the internal ones. `make check` is the full verification: format check, clippy with warnings
denied, and the test suite.

The harness binary must resolve on `PATH`. If it does not, the app prints a one-line error
on stderr and exits non-zero **before** taking over the terminal, rather than opening a
window it cannot use. `--help` and `--version` work either way.

Linux and macOS are the primary targets; Windows is best-effort — the Windows npm shim for
`pi` is handled by delivering the prompt on stdin instead of argv. `codex` takes its prompt
on stdin on every platform, so that limit does not apply to it.

## Usage

`--harness` is mandatory; everything else is optional.

```sh
# Rewrite the clipboard, with the instruction set at startup
prosecraft --harness pi --prompt "Improve the style"

# Start from a file instead of the clipboard
prosecraft --harness pi --prompt "Improve the style" --text-file draft.md

# Keep the instruction in a file (read once, at startup)
prosecraft --harness pi --prompt-file style.md --text-file draft.md

# Different register, different model, no clipboard read at startup
prosecraft --harness pi --model <name> --no-clipboard \
  --prompt "Make it informal, contractions are fine" --text "We regret to inform you…"

# The same rewrite, through codex instead
prosecraft --harness codex --prompt "Improve the style" --text-file draft.md
```

## Harness differences

All three are first-class, but they are not identical, and two of the differences are worth
knowing before you pick one. Details and measurements are in `docs/spec.md` §6.

**codex does not stream.** `claude` and `pi` report progress as the answer is written, so
the process pane fills continuously. `codex` delivers the whole answer in one event: the pane
fills in jumps, and the `running` word on the status line is what tells you a rewrite is
under way.

**codex is sandboxed rather than tool-stripped.** `claude` and `pi` can be told to have no
tools at all. `codex` has no such switch — its tools are built in — so prosecraft runs it
with a read-only filesystem and approvals never answered, which is what keeps a rewrite from
touching the filesystem or the network. The observable guarantee is the same (nothing is
written), but it is enforced one layer lower, so "tools are disabled" would be the wrong way
to describe it.

The practical differences are smaller: `codex` names its own session, so the short id on the
status line appears after the first rewrite rather than before it, and `Ctrl+N` starts a new
thread exactly as it does elsewhere.

Inside the TUI the instruction is just text, so the second and third passes are cheap: put
the cursor in the **instructions** pane, edit the line, press `Ctrl+R`.

```text
instructions: Improve the style
Ctrl+R   →  tightened, more formal prose in the output pane

instructions: Improve the style, but informal — contractions are fine
Ctrl+R   →  the same text rewritten as a friendlier second pass
```

A few things worth knowing:

- `--prompt` / `--prompt-file` are read **once**, at startup, into the instructions pane.
  Editing the file afterwards changes nothing; edit the pane instead. Restart the app to
  re-read the file.
- Source text precedence: `--text`, then `--text-file`, then the clipboard, then empty.
- `Ctrl+R` with empty instructions, or empty source text, is refused: the status line says
  why and focus moves to the offending pane.
- `Ctrl+N` starts a new harness session — the context is dropped, source and instructions
  are kept. Use it when the instructions drifted away from the original text.
- `Ctrl+V` replaces the focused editable pane with the clipboard, `Ctrl+K` clears it,
  `Ctrl+C` copies the focused pane, `Ctrl+Q` quits. A replaced pane comes back with one
  `Ctrl+Z`. `Shift+Insert` is the same paste under its other conventional spelling. `Ctrl+C`
  in the process pane during a run cancels it, after a second confirming press.
- `F1` shows every key binding; the status line only carries the three that fit.
