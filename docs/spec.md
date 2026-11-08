# prosecraft — spec (draft v0.9)

A TUI that rewrites text with an LLM harness. Paste text on the left, set instructions for
how to rewrite, press one key, read the result on the right. Change the text or the
instructions and press the same key again.

Status: draft. Sections marked **[Q]** are unresolved, see [Open questions](#open-questions).

## 1. Goal

Rewrite a block of text with a mandatory, externally installed harness (`claude`, `pi`, or
`codex`) run in one-shot (non-interactive) mode. Iterate by editing the instructions, or the source
text, and pressing the same key again — the harness session carries the conversation
history forward.

### Non-goals (v0.1)

- Editing history, undo/redo.
- Multi-turn chat with the model, or any conversation view.
- **Any filesystem or side effect on the system.** The app shows text and talks to the
  harness. It reads the clipboard, writes the clipboard, and nothing else. Harness tool
  use is disabled, not merely discouraged. (codex cannot be denied its tools and is
  sandboxed instead — measured, §6.5; a human accepted the weaker guarantee. The intent
  above is unchanged: no run may reach the filesystem.)
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

  --harness <claude|pi|codex>  required; select the harness adapter
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
│ editable, ~8 lines             ││ read-only, stdout+stderr, ~8    │
└────────────────────────────────┘└────────────────────────────────┘
 source │ prose… │ idle          [Ctrl+R] rewrite  [F1] help  [Ctrl+Q] quit
```

Four panes, all focusable. Two columns, 50/50.

| Pane | Editable | Content |
|---|---|---|
| source | yes | text to rewrite |
| instructions | yes | task instruction for the current run |
| output | no | rewritten text from the last successful run |
| debug | no | streamed text, reasoning, stderr, app events |

- Focus is a single index over the four panes; `Tab` / `Shift+Tab` cycles forward and back
  in reading order. `Ctrl+Left` / `Ctrl+Right` jump to the other column, preserving row;
  `Ctrl+Up` / `Ctrl+Down` jump to the other row, preserving column, and do not wrap at the
  edge. The same four moves are also bound to `Shift+Left` / `Shift+Right` and
  `Shift+Up` / `Shift+Down`, so neighbouring panes are reachable without a hand on the
  control key; the two bindings are aliases and never differ in behaviour.
- A `Ctrl+R` that starts a rewrite sets focus to the `output` pane. Asking for a rewrite is
  the end of typing: the next thing the user does is read the result, and the pane that will
  hold it is where they are looking. The debug pane stays one `Ctrl+Up` away, which is what
  keeps cancelling reachable. A refused `Ctrl+R` does not go here; it moves focus to the
  culprit (§5).
- The focused pane shows a cursor (editable) or a highlighted border (read-only), and
  scrolls to keep the cursor/offset in view.
- `output` and `debug` keep their own scroll offsets. The `debug` pane auto-tails while the
  user has not scrolled up in it, and stops tailing once they have (a marker shows
  "not tailing").
- Every pane soft-wraps. Widths are recalculated on resize; text is never truncated.
- Both bottom panes (`instructions`, `debug`) share one height: `min(8, column / 3)`,
  minimum 3 lines, capped at 8. The same formula on equal-height columns puts their top
  border on one row, so the two halves of the frame line up.
- Status line: harness name, session id (short form, `—` before the first rewrite), state
  (`idle` / `running` / `error`), then the key hints. The hints are `[Ctrl+R] rewrite`,
  `[F1] help`, `[Ctrl+Q] quit` in that order; the rest of the key set is reachable only
  from the help panel, because the row is one line that clips. `Tab` is not hinted: `F1`
  is on the row, so the whole key table, focus cycling included, is one press away.

### Key bindings

| Key | Action |
|---|---|
| `Ctrl+R` | Rewrite. Starts a session on first use, otherwise resends current source + instructions into the existing session |
| `Ctrl+N` | New session: drop harness context, clear output and debug, keep source and instructions |
| `Ctrl+C` | Copy the focused pane's text. In the debug pane while a rewrite runs: cancel the harness, after a confirming second press |
| `Ctrl+Q` | Quit; confirm if a rewrite is running. The only quit key |
| `Tab` / `Shift+Tab` | Cycle focus |
| `Ctrl+←` / `Ctrl+→` | Switch column |
| `Ctrl+↑` / `Ctrl+↓` | Switch row (source ↔ instructions, output ↔ debug); no wrap |
| `Shift+←` / `Shift+→` | Switch column; same as `Ctrl+←` / `Ctrl+→` |
| `Shift+↑` / `Shift+↓` | Switch row; same as `Ctrl+↑` / `Ctrl+↓` |
| `Ctrl+V` | Paste the clipboard into the focused pane (editable panes only) |
| `Ctrl+K` | Clear the focused pane (editable panes only) |
| arrow keys | Move cursor (editable) or scroll (read-only) |
| `PgUp` / `PgDn` | Scroll by a page in the focused pane |
| `F1` | Toggle the key-bindings help panel. Any key closes it; the key that closes it does nothing else |
| `Esc` | Closes the help panel when it is open. Otherwise does nothing in v0.1 |

## 5. Rewrite flow

```
idle ──Ctrl+R──> validating ──fail──> validation error (stays idle, focus moves to culprit)
                      │ok: focus → output
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

One trait, three implementations. Adapter-specific argv lives only inside the adapter.

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
                           // Some => prompt written to stdin (Windows batch shims only, §6)
}

trait Harness {
    fn name(&self) -> &str;
    /// `Some` when the client names sessions; `None` when the harness names its own and
    /// reports the id on its event stream (codex, §6.5).
    fn new_session_id(&self) -> Option<String>;
    /// `session == None` means this is the first call of the session.
    fn build(&self, task: &Task, session: Option<&str>) -> Result<Invocation>;
    /// Classify one stdout line for the debug pane. Never produces output text.
    fn classify(&self, line: &str) -> Event;
    /// Authoritative final text, if this line was the terminal event.
    fn final_text(&self, line: &str) -> Option<String>;
    /// The session id, if this line was the one that announced it. Default `None`: a
    /// client-named harness has nothing to announce (§6.5).
    fn session_from_event(&self, line: &str) -> Option<String>;
}
```

`classify`/`final_text` are the streaming surface and the reason each adapter owns its own
event vocabulary instead of the app parsing JSON generically. `Event` is a small enum
(`Text`, `Think`, `Junk`), used only to label debug lines. A malformed or unrecognised line
maps to `Junk` — never an error — so an unknown event type cannot abort a run.

The two session methods on the trait are two halves of one question: **who names the
session.** claude and pi accept an id the client chose (`new_session_id` returns `Some`, and
the id is read back out of the argv the adapter wrote it into); codex generates the id
itself and reports it on the event stream, so `new_session_id` returns `None` and
`session_from_event` learns it from `thread.started` (§6.5). One method cannot express both
directions, and both defaults are safe, so there are two.

claude and pi take the prompt as an argument rather than stdin: prompts may contain
`-` and quoting is one implementation instead of two.

codex is the exception, and it is a property of the harness rather than of the platform, so
it is stated per adapter: it takes the prompt on **stdin** everywhere (§6.5). The
`Transport` in `Task` answers "does *this resolution on this OS* force stdin" — the Windows
shim question below — and a codex adapter does not consult it at all, because its transport
is fixed by which harness it is.

**Windows exception, measured.** The `pi` installation on Windows is an npm shim: a
`pi.cmd` batch file, with no `pi.exe`. A batch file's command line is re-parsed by
`cmd.exe`, and no argument carrying a newline reaches it at all — measured against the real
template, `spawn` fails with `InvalidInput: batch file arguments are invalid`, and going
through `cmd /c` instead truncates the prompt at the first newline. Since the prompt
template always contains newlines, argv cannot be the transport on that platform.

On stdin the same prompt round-trips verbatim through the shim (measured: one assistant
`message_end`, `agent_end` shows every newline and both markers intact). So, **only when
the resolved program is a Windows batch shim**, the prompt is written to stdin and no
positional prompt or `--` separator is passed. On unix, on macOS, and for claude anywhere,
the argv is unchanged and remains the transport. This is a per-platform rule, not a second
transport: the adapter asks the resolution, and there is no other caller.

The same limit applies to `--system-prompt`, which carries the instructions pane and
therefore the same free text: measured, a newline there also fails with `InvalidInput`.
That flag is dropped on a shim, which costs nothing because §6.0 already establishes that
it is passed for register only and correctness never depends on it — the instructions
travel in the user turn, which on a shim means the stdin prompt.

Consequence for `Task`: an adapter cannot hardcode "the prompt is an argument", so `build`
receives the resolution (see §3) and the prompt's transport is a property of the harness on
that OS, not of the harness.

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
every harness, both first call and continuation. `--system-prompt` is still passed on
first call for claude and pi (it sets the register for the whole session and is cheap), but
the app never depends on it for correctness. Consequence for the adapter: `build()` produces
nearly identical argv for first call and continuation — only the session flag differs.
codex has no system-prompt flag at all, which costs nothing for the same reason: measured,
an instruction in the user turn takes effect on a continuation (§6.5).

### 6.1 Fixed user prompt

Identical for every harness and for both first call and continuation:

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
into a *protocol*: measured on claude, pi and codex, the model wraps the result exactly as
asked, including with preamble present.

Measured with markers:

| input | output |
|---|---|
| instructions asking for an explanation | explanation, blank line, `<<<TEXT>>>`, text, `<<<END>>>` |
| instructions asking for formal text only | `<<<TEXT>>>` / text / `<<<END>>>` |
| pi, same template | `<<<TEXT>>>text<<<END>>>` — **single line, no newlines around markers** |

So extraction must not assume the markers sit on their own lines. It splits on the marker
strings, wherever they are.

### 6.1a Extracting the rewritten text

claude and pi are run in **streaming JSON mode** (§6.1b), and codex in its JSON mode, which
is not a stream (§6.5). Every stdout line is fed to the adapter, which routes it to the
debug pane; only the text **between the markers** reaches the output pane.

Per harness, measured on claude 2.1.319, pi 1.5.0 and codex-cli 0.159.0:

| | claude | pi | codex |
|---|---|---|---|
| flag | `--output-format stream-json --verbose --include-partial-messages` | `--mode json` | `exec --json` (same flag on `exec resume`) |
| final text event | `type == "result"` → `result` | last `message_end`, `role == "assistant"` | last `item.completed` with `item.type == "agent_message"` → `item.text` |
| streamed text delta | `stream_event` → `event.type == content_block_delta` → `delta.type == text_delta` → `delta.text` | `message_update` → `assistantMessageEvent.type == text_delta` → `delta` | **none** — measured, no delta events are emitted |
| reasoning delta | `delta.type == thinking_delta` | `assistantMessageEvent.type == thinking_delta` | **none emitted**, though `reasoning_output_tokens` is non-zero |
| session id | client-chosen, in the argv | client-chosen, in the argv | from `thread.started` → `thread_id` |

**The terminal text is the last `agent_message`, never a tool item.** codex reports shell
commands, file changes, web searches and MCP calls as their own
`item.completed` events carrying output; taking "the last item with text" would put a
command's stdout in the output pane. Only `item.type == "agent_message"` is prose.

**Nothing is ever reconstructed by concatenation.** Measured on claude: the terminal
`result` event carries the complete answer, and the concatenated `text_delta`s are exactly
equal to it (76 bytes vs 76 bytes). Deltas are display-only; their only consumer is the
debug pane. This removes the whole class of bugs where a dropped delta silently truncates
the output. For codex the rule is vacuous and still binding: there are no deltas to fall
back to, so "exit 0 but no `agent_message`" is an error with nothing to guess from.

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

claude and pi buffer stdout when emitting plain text — measured: claude wrote nothing for
13 s, then the whole answer at once — which is why JSON streaming is required rather than
optional (§6.1b). codex's JSON mode buffers per item instead (§6.5), which is the same
problem one level up and the reason §6.1b names the status line as its progress signal.

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

**codex cannot stream, and that is a limitation of the harness, not a missing flag.**
Measured on a 300-word answer: `exec --json` emits `thread.started`, `turn.started`, then
**one** `item.completed` carrying the whole `agent_message` at t≈21 s, then
`turn.completed`. There is no per-token event in that mode — the binary does contain an
`AgentMessageDeltaNotification`, but it belongs to the app-server protocol, not to
`exec --json`. So for codex the debug pane is inert between items by necessity: the first
lines land at t≈0.7 s and nothing follows until the answer arrives whole. The progress
signal during a codex run is therefore the **status line's `running` state**, which is why
it is drawn from the state machine rather than inferred from arriving text. Do not "fix"
this by reaching for an unpublished transport: the app already shows what the harness gives
it, and the output pane is written once per run either way.

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

The three harnesses differ in five places: the session flag for a new session vs a
continuation, the id format (UUID v4 vs `[A-Za-z0-9._-]` vs a harness-chosen `01a1…`), the
tool-denial flags, the event vocabulary (`stream_event`/`result` vs
`message_update`/`agent_end` vs `item.completed`/`thread.started`), and **who names the
session** (client vs harness, §6). Everything else — prompt template, output rules, process
management — is shared. The event vocabulary is the largest of the five and the reason
`classify`/`final_text` exist on the trait; who names the session is the reason
`new_session_id` returns an `Option` and `session_from_event` exists at all.
`Harness::build` is a pure function, so every adapter gets table tests over its argv
without spawning anything. The `Task` carries `session: Option<&str>`, where `None` means
"first call of this run"; the adapter, not the app, decides which flag that implies.

### 6.5 codex

Measured on codex-cli 0.159.0. This section is longer than §6.2 and §6.3 because two of its
findings contradict what the rest of §6 assumes about *any* harness: codex has no
tool-denial flag (only a sandbox), and codex cannot stream.

First call:

```
codex exec --json
      -c sandbox_mode="read-only" -c approval_policy="never"
      -c project_doc_max_bytes=0 -c features.apps=false
      -c web_search="disabled"
      --skip-git-repo-check
      [--model <name>]
      -- -
```

Continuation:

```
codex exec resume --json
      <same -c block and --skip-git-repo-check>
      [--model <name>]
      <thread-id>
      -- -
```

The prompt travels on **stdin** in both shapes (`-- -` makes the positional a literal dash
and codex reads the prompt from the pipe), so `Invocation.stdin` is always `Some` and `args`
never carries the prompt. Measured: both transports round-trip the real template with every
newline and both markers intact, so this is a choice, not a workaround — it keeps user prose
out of the OS argument list, and sidesteps the batch-shim limit §6 documents.

Each flag below was measured; the reason is what keeps it from being "tidied" away.

- **`-c sandbox_mode="read-only"` *and* `-c approval_policy="never"`, together.** This is
  the substitution for `--tools ""` and `--no-tools`, and the pair is load-bearing.
  Measured: `-s read-only` alone leaves approvals on-request and a write succeeded in 3 of 3
  runs; `-c sandbox_mode` alone likewise. The two together blocked every attempt (9 of 9,
  across a git repo and a plain directory, including a shell fetch and a write attempted
  through the residual JS REPL, which failed with `EPERM`). `-c` is used rather than `-s`
  because `exec resume` rejects `-s` (`unexpected argument`), and one flag shape for both
  call shapes is one place to get it wrong instead of two.
- **`-c project_doc_max_bytes=0`.** codex's `--no-context-files`. Measured: without it the
  user's own `AGENTS.md` is in the prompt and the model reproduces it verbatim when asked;
  with it, absent. A rewrite must not silently inherit the user's coding guidelines.
- **`-c features.apps=false`.** Measured: 147 tools offered become 29, and the whole Codex
  app MCP surface leaves the prompt.
- **`-c web_search="disabled"` — and it only works in combination with the flag above.**
  Measured, and the pairing is the whole point: with `features.apps=false` alone the model
  can still search (`apps=false only` returned a live headline through the `web__run`
  tool); with `web_search="disabled"` alone it can still search too. Only the two together
  produced `NO-WEB-ACCESS`. So neither flag is redundant, and neither is sufficient.
- **Not used: `-c tools.web_search=false`.** Measured: it does not remove the tool
  (`web__run` stayed), so it would read as a denial that denies nothing.
- **`--skip-git-repo-check`.** Measured: without it codex refuses to run outside a git repo
  with `Not inside a trusted directory`, which would make the app fail in every directory
  that is not a checkout.
- **`--json`.** The machine-readable mode; `exec resume` needs it repeated.

**Deliberately absent: `--ignore-user-config` and `-s`.** `--ignore-user-config` disables
the user's own `config.toml`, which is where their model choice lives, and measured it also
changed whether writes were blocked; using it would swap "the user's config is respected"
for a second, weaker layer of denial. A future change that reaches for either flag is a
regression, so §6.5 names them.

**Session lifecycle is inverted, and this is why the trait changed.** codex names its own
threads: `-c thread_id=…` is ignored, and the id appears only on stdout as
`{"type":"thread.started","thread_id":"01a1bc…"}`. So `new_session_id` returns `None` and the
app learns the id through `session_from_event` (`§6`), taking the first id it is offered and
keeping it for the run. `Ctrl+N` still works, because an unnamed first call is a new thread
and its id is learned from its own stream.

**Tool denial is sandbox-level here, which is weaker than §1 asks.** codex cannot be denied
its tools: the config keys that look like denials are rejected by `--strict-config`
(`unknown configuration field`) or do nothing, and a JS REPL (`mcp__node_repl__js`,
`mcp__cua_repl.js`) survives every filter. What holds is the sandbox: read-only filesystem,
approvals never answered, network refused (measured: a shell fetch fails). No run reached
the filesystem, which is §1's actual requirement, but it is enforced one layer lower than
for claude and pi, so "tools are disabled" would be a false statement about codex. A human
accepted this trade rather than dropping codex support; the alternative — refusing the
adapter — was rejected because the observable guarantee (nothing is written) holds.

**Residual cost, accepted.** codex injects its own developer prompt and a skills catalogue
(~30 KB of local `SKILL.md` descriptions, ~18 k input tokens) into every call, and no config
key found suppresses it (`skills_instructions`, `include_skills_usage_instructions`,
`features.skill_search=false` are all rejected). It costs tokens and register, not
correctness: measured, the markers still come back.

**No stream.** See §6.1b: codex emits one complete `agent_message`, so the debug pane fills
in jumps. Recorded there rather than here because it is a property of the JSON mode, not of
the argv.

Measured end to end on the exact argv above: first call `<<<TEXT>>>Arrr, send me the
report, matey.<<<END>>>`, continuation with changed instructions `<<<TEXT>>>You are hereby
requested to furnish the report to the undersigned.<<<END>>>` on the same thread id, the
user's `AGENTS.md` never present, and no file created.

## 7. Process handling

- `std::process::Command` with `stdout`/`stderr` piped; one reader thread per stream
  (`BufRead::lines`) sending `(Stream, String)` over an `mpsc` channel.
- The UI thread never blocks: it drains the channel each tick (~16 ms) and redraws once per
  tick. Measured event volume is ~280 lines for claude, ~550 for pi and **~7 for codex**
  per rewrite, so per-event redraws would be wasteful; per-tick batching is required, not
  an optimisation.
- Each stdout line goes to the active harness's `classify` and is appended to the debug pane
  with its kind. `final_text` is consulted on every line; at most one line per run returns
  `Some`, and that text is what the output pane receives. Lines that do not parse as JSON
  are shown as `junk` — a harness printing garbage is a diagnosable event, not a failure.
- For a harness that names its own session, `session_from_event` is also consulted on every
  line until an id is known, and only until then: codex announces its thread once, and a
  later id can only belong to a run the app did not start.
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
| Session id never announced by the harness | State `error` when the run ends with no id and no terminal text; a continuation cannot be attempted without one, and the status line keeps `—` |
| Non-zero exit | State `error`, debug tail visible (claude puts the reason on stderr); output pane keeps the cleared state for this run |
| Timeout | Kill, state `error`, debug pane notes the timeout |
| Clipboard unavailable | Status-line warning, everything else still works |
| Non-UTF-8 output | Lossy conversion |
| Markers honoured | Text between `<<<TEXT>>>` and `<<<END>>>` becomes the output; everything outside is discarded (§6.1a) |
| Missing output markers | State `error`, raw terminal text shown, debug pane holds the full output (§6.1a) |
| Tool-call XML between markers | State `error`, text shown, debug explains that tools are denied (§6.1a) |
| Exit 0 but no terminal event | State `error`: no `result`/`agent_end`/`agent_message` means no final text. Never fall back to concatenating deltas (§6.1a) |

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
src/harness/codex.rs
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
6. **M6 — codex adapter.** Third adapter, and the one that exercises the harness-named
   session path. Verify: argv table tests for both call shapes; the thread id is learned
   from `thread.started` and survives a continuation, so a changed instruction takes effect
   on the same thread (the §6.0 regression, measured: pirate → legal register); `Ctrl+N`
   produces a different thread id and drops the context; a prompt that tells the model to
   write a file leaves no file behind, and one that asks it to quote `AGENTS.md` yields
   nothing; the debug pane does **not** stream, per §6.1b, and that is not a defect.
6. **M6 — codex adapter.** Third adapter, and the one that exercises the harness-named
   session path. Verify: argv table tests for both call shapes; the thread id is learned
   from `thread.started` and survives a continuation, so a changed instruction takes effect
   on the same thread (the §6.0 regression, measured: pirate → legal register); `Ctrl+N`
   produces a different thread id and drops the context; a prompt that tells the model to
   write a file leaves no file behind, and one that asks it to quote `AGENTS.md` yields
   nothing; the debug pane does **not** stream, per §6.1b, and that is not a defect.

## Decisions

Settled since v0.1: instructions travel in the user turn (§6.0, measured); source text is
resent on every rewrite (cheap, a few paragraphs); one session per run; nothing is
persisted; `Esc` does nothing, `Ctrl+Q` quits and is the only quit key, `Ctrl+C` copies the
focused pane and cancels only in the debug pane after a confirming second press, so the key
keeps its terminal meaning of copy; output pane = final text only, debug pane
= streaming/reasoning/stderr/app events; output pane cleared on spawn;
harness binary is checked at startup and missing means a stderr error and non-zero exit;
tool denial is verified for claude and pi (no file was created in three claude attempts
and one pi attempt), and for codex it is a **sandbox** rather than a denial (§6.5) —
that is the one place the three harnesses are not equal, it was accepted deliberately, and
it is stated in §1 rather than left for a reader to infer; output **streams** (§6.1b),
because plain-text mode buffers the whole answer (`claude` wrote nothing for 13 s then
everything at once), except on codex, which cannot stream; harness and model are
fixed at init from the command line, with no TUI selection; instructions are read once at
startup and never reloaded (restart after editing the file); `Ctrl+N` while running is
ignored like `Ctrl+R`; the name stays `prosecraft`; the rewritten text is delimited by
`<<<TEXT>>>`/`<<<END>>>` markers so preambles are stripped by protocol, not by heuristic,
and a missing marker is an error rather than a guess.


