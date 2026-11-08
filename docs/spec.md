# prosecraft — spec (draft v0.8)

A TUI that rewrites text with an LLM harness. Paste text on the left, set instructions for
how to rewrite, press one key, read the result on the right. Change the text or the
instructions and press the same key again.

Status: draft. Sections marked **[Q]** are unresolved, see [Open questions](#open-questions).

## 1. Goal

Rewrite a block of text with a mandatory, externally installed harness (`claude` or `pi`)
run in one-shot (non-interactive) mode. Iterate by editing the instructions, or the source
text, and pressing the same key again — the harness session carries the conversation
history forward.

### Non-goals (v0.1)

- Editing history, undo/redo.
- Multi-turn chat with the model, or any conversation view.
- **Any filesystem or side effect on the system.** The app shows text and talks to the
  harness. It reads the clipboard, writes the clipboard, and nothing else. Harness tool
  use is disabled, not merely discouraged.
- Persisting anything across runs: no state file, no config file, no session pointer
  beyond what the harness itself stores.

## 2. Assumptions

1. The harness binary is installed and authenticated by the user. prosecraft does not
   manage credentials, models, or provider config.
2. "Instructions" (the system prompt) are the user's *task instruction* for this rewrite:
   "rewrite in a formal register", "make it shorter", "use slang". They are valid only for
   the current run. No persistence, no global config, no profile management.
3. A rewrite is refused while the instructions are empty. This is the required validation.
4. Only one harness process runs at a time.
5. Linux/macOS first; Windows best-effort.

## 3. CLI

```
prosecraft [OPTIONS]

  --harness <claude|pi>        required; select the harness adapter
  --prompt <TEXT>              instructions for the rewrite
  --prompt-file <PATH>         read instructions from a file at startup
  --text <TEXT>                initial source text
  --text-file <PATH>           read initial source text from a file at startup
  --no-clipboard               do not read the clipboard at startup
  --model <NAME>               passed through to the harness
  -h, --help
  -V, --version
```

Rules:

- `--harness` is mandatory. Unknown value → exit code 2 with usage.
- Harness and model are chosen **once, on the command line**, and cannot be changed from
  the UI. Switching harness means restarting the app.
- `--prompt` and `--prompt-file` are mutually exclusive → exit code 2. Both are read once
  at startup into the instructions pane; the file is **never** re-read. To pick up edits to
  the file, restart the app. There is no reload hotkey.
- Source text precedence: `--text` / `--text-file` > clipboard > empty.
- **The harness binary must resolve on `PATH` at startup.** If it does not, print a clear
  one-line error naming the expected binary to **stderr** and exit non-zero, before the
  terminal is put into raw mode and before any UI is drawn. An app whose only job is to
  spawn a process should not open a window it cannot use. `--help` and `--version` still
  work without any harness installed, since they return before this check.

## 4. TUI

```
┌ source ────────────────────────┐┌ output ────────────────────────┐
│                                ││                                │
│  editable                      ││  read-only, rewritten text      │
│                                ││                                │
├ instructions ──────────────────┤├ process ───────────────────────┤
│ editable, ~5 lines             ││ read-only, stdout+stderr, ~8    │
└────────────────────────────────┘└────────────────────────────────┘
 source │ prose… │ idle          [Ctrl+R] rewrite  [Tab] focus  [Ctrl+Q] quit
```

Four panes, all focusable. Two columns, 50/50.

| Pane | Editable | Content |
|---|---|---|
| source | yes | text to rewrite |
| instructions | yes | task instruction for the current run |
| output | no | rewritten text from the last successful run |
| debug | no | streamed text, reasoning, stderr, app events |

- Focus is a single index over the four panes; `Tab` / `Shift+Tab` cycles forward and back
  in reading order. `Ctrl+Left` / `Ctrl+Right` jump to the other column, preserving row.
- The focused pane shows a cursor (editable) or a highlighted border (read-only), and
  scrolls to keep the cursor/offset in view.
- `output` and `debug` keep their own scroll offsets. The `debug` pane auto-tails while the
  user has not scrolled up in it, and stops tailing once they have (a marker shows
  "not tailing").
- Every pane soft-wraps. Widths are recalculated on resize; text is never truncated.
- Instructions pane height: `min(5, 25% of column)`, minimum 3 lines.
- Status line: harness name, session id (short form, `—` before the first rewrite), state
  (`idle` / `running` / `error`), then the key hints.

### Key bindings

| Key | Action |
|---|---|
| `Ctrl+R` | Rewrite. Starts a session on first use, otherwise resends current source + instructions into the existing session |
| `Ctrl+N` | New session: drop harness context, clear output and debug, keep source and instructions |
| `Ctrl+C` | If running: cancel the harness. Otherwise: quit |
| `Ctrl+Q` | Quit; confirm if a rewrite is running |
| `Tab` / `Shift+Tab` | Cycle focus |
| `Ctrl+←` / `Ctrl+→` | Switch column |
| `Ctrl+Y` | Copy the focused pane's text to the clipboard |
| `Ctrl+V` | Paste the clipboard into the focused pane (editable panes only) |
| `Ctrl+E` | Load the clipboard into the source pane, replacing it |
| arrow keys | Move cursor (editable) or scroll (read-only) |
| `PgUp` / `PgDn` | Scroll by a page in the focused pane |
| `Esc` | Does nothing in v0.1 |

## 5. Rewrite flow

```
idle ──Ctrl+R──> validating ──fail──> validation error (stays idle, focus moves to culprit)
                      │ok
                      v
             running ──(every stdout/stderr line)──> debug pane only
                      │
                      ├──> done    exit 0 && terminal event → output pane ← final text
                      ├──> error   non-zero exit, tool-call text, or no terminal event
                      └──> cancelled
```

1. **Validate.** Refuse, show the reason in the status line, and move focus to the
   offending pane if:
   - instructions are empty or whitespace-only (**required validation**);
   - source text is empty or whitespace-only;
   - the harness binary does not resolve on `PATH`;
   - a rewrite is already running.
2. **Send.** One invocation carries:
   - instructions → **in the user turn**, in the fixed template (see §6.1). Not as a
     harness system prompt — see §6.0, which is a measured constraint, not a preference;
   - source text → also in the user turn, under the instructions;
   - a trailing demand to wrap the answer in `<<<TEXT>>>`/`<<<END>>>` (§6.1).

   The source text is sent on **every** rewrite, changed or not. Editing the source and
   pressing `Ctrl+R` again is the normal way to work; no source hashing and no automatic
   session invalidation is needed. Texts are at most a few paragraphs, so resending is
   cheap.
3. **Session.** The session id is created lazily on the first `Ctrl+R` of the run and
   reused for every subsequent rewrite. `Ctrl+N` discards it and starts a new one on the
   next rewrite. One session per TUI run, always.
4. **Stream.** The **output pane is reset on spawn** so the view says plainly that a new
   action started. stdout is a **JSON event stream** (§6.1a); every event type is rendered
   into the **debug pane** — streamed `text_delta`, reasoning deltas, stderr, app events —
   so progress is visible for debugging even when the final text differs from what
   streamed. The output pane gets **nothing** during the run.
5. **Settle.** Exit 0 → put the harness's final text in the output pane, trim, state `idle`.
   Non-zero exit, tool-call text, or unparseable output → state `error`, output pane stays
   empty for this run, debug tail visible.

The instructions are re-read from the command line on every invocation and are also
present in the user turn, so editing them always takes effect on the next rewrite.

Note: every rewrite appends another copy of the source text to the harness history.
`Ctrl+N` is the reset. No automatic compaction is implemented; harness-side compaction is

the harness's business. With "a few paragraphs" as the expected input this is not a
practical limit.

Pressing `Ctrl+R` **or** `Ctrl+N` while a rewrite is running is ignored, and the status line
says so. Resetting the session mid-run would discard the id the live process is using.

## 6. Harness adapters

One trait, two implementations. Adapter-specific argv lives only inside the adapter.

```rust
struct Task<'a> {
    instructions: &'a str,
    source_text: &'a str,
    model: Option<&'a str>,
}

struct Invocation {
    program: String,
    args: Vec<String>,
    stdin: Option<String>, // None => prompt passed as an argument
}

trait Harness {
    fn name(&self) -> &str;
    fn new_session_id(&self) -> String;
    /// `session == None` means this is the first call of the session.
    fn build(&self, task: &Task, session: Option<&str>) -> Result<Invocation>;
    /// Classify one stdout line for the debug pane. Never produces output text.
    fn classify(&self, line: &str) -> Event;
    /// Authoritative final text, if this line was the terminal event.
    fn final_text(&self, line: &str) -> Option<String>;
}
```

`classify`/`final_text` are the streaming surface and the reason each adapter owns its own
event vocabulary instead of the app parsing JSON generically. `Event` is a small enum
(`Text`, `Think`, `Junk`), used only to label debug lines. A malformed or unrecognised line
maps to `Junk` — never an error — so an unknown event type cannot abort a run.

Both harnesses take the prompt as an argument rather than stdin: prompts may contain
`-` and quoting is one implementation instead of two.

### 6.0 Measured constraint: system prompt flags do not survive resume

Verified on `claude` 2.1.319 and `pi` 1.5.0. This determines the design.

| Test | Result |
|---|---|
| claude fresh session, `--system-prompt` = pirate | obeys pirate |
| claude `--resume`, `--system-prompt` = pirate | **ignored**, neutral output |
| claude `--resume`, `--append-system-prompt` = pirate | **ignored**, neutral output |
| claude `--resume`, instructions in the **user turn** | obeys the new instructions |
| pi `--session-id` (create) → same id with changed `--system-prompt` | obeys the new system prompt |

So claude silently discards both system-prompt flags on resume, while pi honours them.
Relying on the flag would make "edit the instructions and press `Ctrl+R`" work on pi and
silently do nothing on claude — the worst failure mode, since the output looks plausible.

**Decision: instructions always travel in the user turn.** The same template is used for
both harnesses, both first call and continuation. `--system-prompt` is still passed on
first call (it sets the register for the whole session and is cheap), but the app never
depends on it for correctness. Consequence for the adapter: `build()` produces nearly
identical argv for first call and continuation — only the session flag differs.

### 6.1 Fixed user prompt

Identical for both harnesses and for both first call and continuation:

```
INSTRUCTIONS:
<instructions pane, verbatim>

TEXT TO REWRITE:
<source pane, verbatim>

Return only the rewritten text, wrapped between the markers <<<TEXT>>> and <<<END>>>.
Put nothing else between the markers. Anything outside the markers is ignored.
```

The marker instruction sits at the end because models weight the tail of the prompt most
heavily. The template is fixed, not user-editable: task instructions go in the
instructions pane, the wire format is the app's business.

**Why markers rather than "return only the text".** The instructions pane is free text and
the user may ask for commentary — measured: instructions saying "add a short explanation of
your changes" produce explanation plus `**Improved text:**` in the same reply, and no
reliable boundary exists in that prose. Asking for markers converts an extraction *guess*
into a *protocol*: measured on both harnesses, the model wraps the result exactly as asked,
including with preamble present.

Measured with markers:

| input | output |
|---|---|
| instructions asking for an explanation | explanation, blank line, `<<<TEXT>>>`, text, `<<<END>>>` |
| instructions asking for formal text only | `<<<TEXT>>>` / text / `<<<END>>>` |
| pi, same template | `<<<TEXT>>>text<<<END>>>` — **single line, no newlines around markers** |

So extraction must not assume the markers sit on their own lines. It splits on the marker
strings, wherever they are.

### 6.1a Extracting the rewritten text

Both harnesses are run in **streaming JSON mode** (§6.1b). Every stdout line is fed to the
adapter, which routes it to the debug pane; only the text **between the markers** reaches
the output pane.

Per harness, measured on claude 2.1.319 and pi 1.5.0:

| | claude | pi |
|---|---|---|
| flag | `--output-format stream-json --verbose --include-partial-messages` | `--mode json` |
| final text event | `type == "result"` → `result` | last `message_end`, `role == "assistant"` |
| streamed text delta | `stream_event` → `event.type == content_block_delta` → `delta.type == text_delta` → `delta.text` | `message_update` → `assistantMessageEvent.type == text_delta` → `delta` |
| reasoning delta | `delta.type == thinking_delta` | `assistantMessageEvent.type == thinking_delta` |

**Nothing is ever reconstructed by concatenation.** Measured on claude: the terminal
`result` event carries the complete answer, and the concatenated `text_delta`s are exactly
equal to it (76 bytes vs 76 bytes). Deltas are display-only; their only consumer is the
debug pane. This removes the whole class of bugs where a dropped delta silently truncates
the output.

**`--include-partial-messages` is required for claude.** Measured without it: the run emits
`system`, `assistant`, `rate_limit_event`, `result` and **zero** `stream_event`s — no
progress at all, even though the answer is complete. Measured with it: 11 `stream_event`s,
6 text deltas, deltas match `result`. The flag is what turns the run into a stream.

Extraction, applied once per run to the terminal text:

1. Find `<<<TEXT>>>` and `<<<END>>>` as plain substrings, in that order.
2. Take everything strictly between them. Do not require newlines, indentation, or markers
   on their own lines — measured: pi returns `<<<TEXT>>>text<<<END>>>` on one line.
3. If both markers are present, the result is **success** regardless of what surrounds
   them. Everything outside is discarded, which is the point of the protocol.
4. **Fallback when a marker is missing:** the run is an **error**, not a guess. Show the raw
   terminal text in the **debug pane** — never in the output pane, which holds final text only
   (constraint 5) and stays as cleared on spawn (§5 step 5) — and set state `error` with "model
   did not use the output markers". Never fall back to trimming or preamble-stripping: that is
   precisely the heuristic the markers exist to remove.
5. Trim surrounding whitespace inside the extracted text; preserve internal newlines
   verbatim. Measured: paragraphs inside the markers keep their blank lines.
6. If the extracted text contains a tool-invocation block (`<invoke name=…>`), the run is an
   **error**: show it, explain. Measured cause: with tools denied the model prints the call
   as text with exit 0. Nothing executed, but it is not a rewrite.
7. No markdown unwrapping and no commentary stripping after step 2. The markers already
   isolate the answer, so a fence the model put *inside* them is part of the text.

Why the fallback is strict: markers are cheap for the model and were honoured in every
measured case, including when the prompt also asked for commentary. A missing marker means
something is genuinely wrong (truncation, refusal, a confused model), and silently
presenting whatever text arrived as the rewrite is the failure mode this design exists to
prevent. The debug pane always holds the full raw output, so diagnosis costs one keypress.

Both harnesses buffer stdout when emitting plain text — measured: claude wrote nothing for
13 s, then the whole answer at once — which is why JSON streaming is required rather than
optional (§6.1b).

- pi's stderr carries one line, `Warning: No project session found … creating a new
  session`. It goes to the debug pane.
- **Rejected alternative:** `claude --output-format json` (single result object, `.result`
  is exactly the answer with no wrapper). It cannot stream: no progress signal, and it
  gives claude a JSON shape pi does not share.

### 6.1b Streaming

Progress is rendered incrementally into the **debug pane**, never the output pane.

Measured throughput on a long rewrite: claude emits continuously from t=1s, pi from t=1s.
Both were fully buffered in plain-text mode (claude wrote nothing for 13 s, then the whole
answer), so JSON streaming is the only way to avoid an inert window during a run.

Consequences:

- The **output pane** is written exactly once per run, when the terminal event arrives.
  Between runs it shows the last good result. It never shows partial text.
- The **debug pane** receives every stdout event (`text`, `think`, `junk`), all stderr
  (`err`), and app events (`app`: run start, exit, duration, errors). Reasoning is visible
  here on purpose — it is the debug view, and it cannot be mistaken for the result.
- On spawn the output pane is cleared, so the view plainly states that a new action started.
  There is no undo (§2); the previous result is deliberately lost at that moment.
- Nothing is interpreted as markdown. A half-emitted fence or `**` is shown literally, so a
  partial stream can never corrupt the display.
- Events are drained from the `mpsc` channel once per UI tick (~16 ms) and rendered once per
  tick. Measured volume is ~280 lines (claude) to ~550 (pi) per run; per-event redraws would
  be wasteful, so per-tick batching is required, not an optimisation.
- The debug pane auto-tails unless the user has scrolled up, in which case it keeps its
  offset and shows a "not tailing" marker.

**Streaming requires `--verbose` and `--include-partial-messages` for claude.** Measured:
`--output-format stream-json` with `-p` exits 1 and prints
`Error: When using --print, --output-format=stream-json requires --verbose`; and without
`--include-partial-messages` the run emits zero `stream_event`s, so there is no progress to
show. Both flags are mandatory. pi needs neither — `--mode json` streams as-is.

### 6.2 claude

First call:

```
claude -p --session-id <uuid> --output-format stream-json --verbose
       --include-partial-messages
       --system-prompt <instructions>
       --tools ""
       --permission-prompts none
       -- <user prompt>
```

Continuation:

```
claude -p --resume <uuid> --output-format stream-json --verbose
       --include-partial-messages
       --system-prompt <instructions>
       --tools ""
       --permission-prompts none
       -- <user prompt>
```

- `--session-id` requires a valid UUID v4 → generate one. Measured: an empty or invalid
  id makes `--resume` fail with exit 1 and
  `Error: --resume requires a valid session ID or session title when used with --print`.
  Treat a missing id as a bug, never send it.
- `--system-prompt` replaces Claude's coding-agent prompt entirely. Passed for register
  only (§6.0); correctness never depends on it.
- `--tools ""` is documented as "use `""` to disable all tools". The argv must carry the
  flag with an empty value; omitting it means default tools. Measured: three attempts to
  make claude write a file all failed to create it, and asked to enumerate its tools it
  answered `NONE`.
- `--permission-prompts none` is required for latency, not for safety. Measured: a prompt
  that makes the model attempt a denied tool call stalls until the timeout when a
  permission prompt would be raised, because nothing answers it. With this flag the same
  case returns in seconds with the call printed as text (§6.1a step 3).
- `--verbose` is mandatory with `-p` and `stream-json`, see §6.1b.
- `--include-partial-messages` is **required**; without it the run emits no `stream_event`
  at all (§6.1a). This corrects an earlier assumption that `content_block_delta` arrives by
  default — it does not.

### 6.3 pi

First call and continuation are the *same* argv; pi's `--session-id` means "open the
existing session with this id, or create it":

```
pi --print --session-id <id> --mode json
   --system-prompt <instructions>
   --no-tools --no-context-files --no-extensions --no-mcp
   --no-skills --no-prompt-templates --offline
   -- <user prompt>
```

- Session ids accept `[A-Za-z0-9._-]` and must start and end with a letter or digit.
  Generated as `psc-<uuid>` (no trailing hyphen, starts with a letter).
- `--mode json` is the streaming form. Measured: text deltas arrive from t=1s, ~547 JSON
  lines per run. Unlike claude, pi needs no extra flag to make this work.
- `--system-prompt` replaces pi's agent prompt. With all tools, skills, MCP and context
  files disabled, that agent prompt has nothing to do; replacing it is correct.
- `--no-context-files` is load-bearing and was verified separately: without it the system
  message carries `project_context` (2759 bytes of the user's own `AGENTS.md`) into every
  request. With it, the system message sections are only `preamble` and `cwd`. A rewrite
  must not silently inherit the user's coding guidelines.
- Measured: pi obeys a changed `--system-prompt` on an existing session, unlike claude. It
  is still sent through the user turn for uniformity (§6.0) — one template, one code path,
  no per-harness instruction plumbing.
- Measured: the first use of an id prints
  `Warning: No project session found with id '…'; creating a new session with that id.` to
  **stderr**. It appears in the debug pane and is harmless; no filtering needed.

### 6.4 Why a trait, and what it costs

The two harnesses differ in four places: the session flag for a new session vs a
continuation, the id format (UUID v4 vs `[A-Za-z0-9._-]`), the tool-denial flags, and the
event vocabulary (`stream_event`/`result` vs `message_update`/`agent_end`). Everything else
— prompt template, output rules, process management — is shared. The event vocabulary is
the largest of the four and the reason `classify`/`final_text` exist on the trait.
`Harness::build` is a pure function, so both adapters get table tests over their argv
without spawning anything. The `Task` carries `session: Option<&str>`, where `None` means
"first call of this run"; the adapter, not the app, decides which flag that implies.

## 7. Process handling

- `std::process::Command` with `stdout`/`stderr` piped; one reader thread per stream
  (`BufRead::lines`) sending `(Stream, String)` over an `mpsc` channel.
- The UI thread never blocks: it drains the channel each tick (~16 ms) and redraws once per
  tick. Measured event volume is ~280 lines for claude and ~550 for pi on one rewrite, so
  per-event redraws would be wasteful; per-tick batching is required, not an optimisation.
- Each stdout line goes to the active harness's `classify` and is appended to the debug pane
  with its kind. `final_text` is consulted on every line; at most one line per run returns
  `Some`, and that text is what the output pane receives. Lines that do not parse as JSON
  are shown as `junk` — a harness printing garbage is a diagnosable event, not a failure.
- Cancellation: kill the child, state `cancelled`, output pane stays as cleared on spawn.
  A cancelled run produces no final text by definition, so there is nothing to show and
  nothing to reconcile.
- Wall-clock timeout: 180 s default. On timeout, kill and report.
- The debug pane is a bounded ring buffer (2000 lines) so memory stays flat.
- Non-UTF-8 output is converted lossily.
- **Both streams go to the debug pane, and neither goes to the output pane.** stdout is the
  event stream, labelled by kind; stderr is labelled `err`; the app adds `app` lines for run
  start/end, exit status, duration and errors. The output pane is written once, from the
  terminal event only.

## 8. Errors

Every failure produces a human-readable status-line message and a debug-pane entry. The app must
not panic on harness misbehaviour, and must restore the terminal on any exit path
(success, error, panic).

Measured failure modes worth encoding:

| Condition | Behaviour |
|---|---|
| Harness binary not found | Startup error to stderr, exit non-zero, no UI drawn (§3) |
| Empty instructions | Validation error, focus to instructions pane |
| Empty source text | Validation error, focus to source pane |
| Session id missing/invalid with `--resume` | Never reachable: id is internal state, generated before use. Treat as a bug — debug assert |
| Non-zero exit | State `error`, debug tail visible (claude puts the reason on stderr); output pane keeps the cleared state for this run |
| Timeout | Kill, state `error`, debug pane notes the timeout |
| Clipboard unavailable | Status-line warning, everything else still works |
| Non-UTF-8 output | Lossy conversion |
| Markers honoured | Text between `<<<TEXT>>>` and `<<<END>>>` becomes the output; everything outside is discarded (§6.1a) |
| Missing output markers | State `error`, raw terminal text shown, debug pane holds the full output (§6.1a) |
| Tool-call XML between markers | State `error`, text shown, debug explains that tools are denied (§6.1a) |
| Exit 0 but no terminal event | State `error`: no `result`/`agent_end` means no final text. Never fall back to concatenating deltas (§6.1a) |

Exit codes: `0` normal, `1` runtime failure (including harness missing), `2` CLI misuse.

## 9. Implementation

Crates: `ratatui`, `crossterm`, `clap` (derive), `arboard`, `uuid`, `anyhow`.

```
src/main.rs         arg parsing, terminal setup/teardown, panic hook restoring the TTY
src/app.rs          App state, event loop, rewrite state machine
src/ui.rs           layout, pane rendering, keybindings -> actions
src/harness/mod.rs  Harness trait, Task, Invocation
src/harness/claude.rs
src/harness/pi.rs
src/process.rs      spawn, stream readers, cancel, timeout
```

Ratatui ships no text editor widget, so the two editable panes need a small shared editor
type: buffer, cursor, soft-wrap-aware vertical movement, insert/delete, and no selection.
It is written once and used by both panes.

Testability: `Harness::build` is pure → argv table tests. `process.rs` is tested against a
fake binary (a shell script echoing fixed output, sleeping, or exiting non-zero). The state
machine is tested headlessly, with the terminal backend stubbed.

## 10. Milestones

1. **M1 — shell.** Args, terminal setup/teardown, four-pane layout, focus cycling
   (`Tab`/`Shift+Tab`/`Ctrl+←→`), editing, clipboard load/copy/paste, quit. Verify:
   manual run; no harness needed; every pane reachable and focus visible.
2. **M2 — streaming rewrite.** Claude adapter (`stream-json`, `--verbose`,
   `--include-partial-messages`), process spawn, JSON line → `classify` → debug pane,
   `final_text` → output pane, validation. Verify: the debug pane advances during a >10 s
   rewrite instead of sitting inert; the output pane stays empty until the run ends, then
   shows the complete text; empty instructions are refused and focus jumps to the
   instructions pane; `think` lines appear in the debug pane only; a response with no
   markers sets state `error` and never lands in the output pane.
3. **M3 — sessions.** Session id lifecycle, resend into an existing session, `Ctrl+N`.
   Verify: with a new instruction ("now as a pirate") the second rewrite visibly changes
   register — this is the regression test for §6.0, which would silently pass with a
   plausible-looking answer if instructions went back to the system-prompt flag.
   `Ctrl+N` then forgets the context.
4. **M4 — pi adapter.** Second adapter proves the trait. Verify: argv table tests, plus one
   live run asserting a changed instruction takes effect on the same session id, and that
   `--no-context-files` keeps `AGENTS.md` out of the system message.
5. **M5 — robustness.** Cancel, timeout, error paths, resize, missing-terminal-event
   handling, debug pane ring-buffer trimming.

## Decisions

Settled since v0.1: instructions travel in the user turn (§6.0, measured); source text is
resent on every rewrite (cheap, a few paragraphs); one session per run; nothing is
persisted; `Esc` does nothing, `Ctrl+Q` quits; output pane = final text only, debug pane
= streaming/reasoning/stderr/app events; output pane cleared on spawn;
harness binary is checked at startup and missing means a stderr error and non-zero exit;
tool denial is verified for both harnesses (no file was created in three claude attempts
and one pi attempt); output **streams** (§6.1b), because plain-text mode buffers the whole
answer (`claude` wrote nothing for 13 s then everything at once); harness and model are
fixed at init from the command line, with no TUI selection; instructions are read once at
startup and never reloaded (restart after editing the file); `Ctrl+N` while running is
ignored like `Ctrl+R`; the name stays `prosecraft`; the rewritten text is delimited by
`<<<TEXT>>>`/`<<<END>>>` markers so preambles are stripped by protocol, not by heuristic,
and a missing marker is an error rather than a guess.


