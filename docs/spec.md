# prosecraft — spec

A TUI that rewrites text with an LLM harness. Paste text on the left, set instructions for
how to rewrite, press one key, read the result on the right. Change the text or the
instructions and press the same key again.

Status: implemented for `claude`, `pi` and `codex`. Each statement below states a decision,
the measurement behind it, or a manual check (§10).

## 1. Goal

Rewrite a block of text with a mandatory, externally installed harness (`claude`, `pi`, or
`codex`) in one-shot, non-interactive mode. To iterate, edit the instructions or the source
text and press the same key again. The harness session carries the conversation history
forward.

### Non-goals

- Edit history and redo. Undo covers the two editable buffers only (§4).
- Multi-turn chat, or a conversation view.
- **A filesystem side effect of any kind.** The app shows text and talks to the harness. It
  reads the clipboard, writes the clipboard, and does nothing else. Harness tool use is
  disabled. (A tool denial is not possible on codex, so codex runs in a sandbox instead —
  measured, §6.5. A human accepted the weaker guarantee. The intent stands: no run reaches
  the filesystem.)
- Persistence across runs: no state file, no config file, no session pointer beyond what the
  harness itself stores.

## 2. Assumptions

1. The user installed and authenticated the harness binary. prosecraft manages no
   credentials, models, or provider config.
2. "Instructions" (the instructions pane) are the user's *task instruction* for this
   rewrite: "rewrite in a formal register", "make it shorter", "use slang". They apply to
   the current run only. There is no persistence, no global config, no profile management.
3. The app must refuse a rewrite while the instructions are empty.
4. One harness process runs at a time.
5. Linux and macOS first. Windows is best-effort.

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

- `--harness` is mandatory. An unknown value exits with code 2 and prints usage.
- The user chooses the harness and the model **once, on the command line**. The UI cannot
  change them. A switch of harness means a restart.
- `--prompt` and `--prompt-file` are mutually exclusive and exit with code 2. The app reads
  the value once at startup, into the instructions pane, and never re-reads the file. To
  pick up an edit, restart the app. There is no reload hotkey.
- Source text precedence: `--text` / `--text-file`, then clipboard, then empty.
- **The harness binary must resolve on `PATH` at startup.** If it does not, the app prints
  a one-line error that names the expected binary to **stderr**, then exits non-zero. This
  happens before raw mode and before the first draw. An app whose only job is to spawn a
  process does not open a window it cannot use. `--help` and `--version` work with no
  harness installed, because they return before this check.

## 4. TUI

```
┌ source ────────────────────────┐┌ output ────────────────────────┐
│                                ││                                │
│  editable                      ││  read-only, rewritten text      │
│                                ││                                │
├ instructions ──────────────────┤├ debug ─────────────────────────┤
│ editable, ~8 lines             ││ read-only, stdout+stderr, ~8    │
└────────────────────────────────┘└────────────────────────────────┘
 pi │ 9f1c4a2b │ idle          [Ctrl+R] rewrite  [F1] help  [Ctrl+Q] quit
```

Four panes, all focusable. Two columns, 50/50.

| Pane | Editable | Content |
|---|---|---|
| source | yes | text to rewrite |
| instructions | yes | task instruction for the current run |
| output | no | rewritten text from the last successful run |
| debug | no | streamed text, reasoning, stderr, app events |

- Focus is one index over the four panes. `Tab` / `Shift+Tab` cycle forward and back in
  reading order. `Ctrl+Left` / `Ctrl+Right` move to the other column and keep the row.
  `Ctrl+Up` / `Ctrl+Down` move to the other row and keep the column, with no wrap at the
  edge. `Shift+Left` / `Shift+Right` and `Shift+Up` / `Shift+Down` are aliases for the same
  four moves, so a neighbouring pane is reachable without a hand on the control key. The
  two bindings never differ in behaviour.
- A `Ctrl+R` that starts a rewrite sets focus to the `output` pane. A rewrite request ends
  typing: the next action is to read the result, so focus goes to the pane that will hold
  it. The debug pane stays one `Ctrl+Up` away, which keeps cancel reachable. A refused
  `Ctrl+R` moves focus to the culprit instead (§5).
- The focused pane shows a cursor (editable) or a highlighted border (read-only), and
  scrolls to keep the cursor or offset in view.
- `output` and `debug` keep separate scroll offsets. The `debug` pane tails the stream until
  the user scrolls up in it. After that it keeps its offset and shows a "not tailing"
  marker.
- Every pane soft-wraps. A resize recalculates the widths. No pane truncates text.
- Both bottom panes (`instructions`, `debug`) share one height: `min(8, column / 3)`, with
  a minimum of 3 lines and a cap of 8. The two columns have equal height, so this one
  formula puts both top borders on one row.
- Status line: harness name, session id, state (`idle` / `running` / `error`), then the key
  hints. The session id shows `—` before the first rewrite and clips to the row width. The
  hints are `[Ctrl+R] rewrite`, `[F1] help`, `[Ctrl+Q] quit`, in that order. The row is one
  line, so it clips, and the help panel carries the rest of the key set. `Tab` has no hint,
  because `F1` is on the row and the whole key table is one press away.

### Key bindings

| Key | Action |
|---|---|
| `Ctrl+R` | Rewrite. Starts a session on first use, or resends the current source and instructions into the existing session |
| `Ctrl+N` | New session. Drops the harness context and clears output and debug. Keeps source and instructions |
| `Ctrl+C` | Copy the focused pane's text. In the debug pane during a run: cancel the harness after a second press |
| `Ctrl+Q` | Quit, with a second press during a run. The only quit key |
| `Tab` / `Shift+Tab` | Cycle focus |
| `Ctrl+←` / `Ctrl+→` | Switch column |
| `Ctrl+↑` / `Ctrl+↓` | Switch row (source ↔ instructions, output ↔ debug). No wrap |
| `Shift+←` / `Shift+→` | Switch column. Same as `Ctrl+←` / `Ctrl+→` |
| `Shift+↑` / `Shift+↓` | Switch row. Same as `Ctrl+↑` / `Ctrl+↓` |
| `Ctrl+V` | Load the clipboard into the focused pane, replacing it (editable panes only) |
| `Ctrl+K` | Clear the focused pane (editable panes only) |
| `Ctrl+Z` | Undo the last edit in the focused editable pane, one step per key event. The app refuses in `output` and `debug` with the read-only note. Capped history, no redo |
| arrow keys | Move cursor (editable) or scroll (read-only) |
| `PgUp` / `PgDn` | Scroll by a page in the focused pane |
| `F1` | Toggle the key-bindings help panel. Any key closes it, and that key does nothing else |
| `Esc` | Close the help panel when it is open. Otherwise nothing |

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

1. **Check.** Refuse, show the reason in the status line, and move focus to the offending
   pane when:
   - the instructions are empty or whitespace-only (**required check**)
   - the source text is empty or whitespace-only
   - the harness binary does not resolve on `PATH`
   - a rewrite is already running
2. **Send.** One invocation carries three parts:
   - the instructions, **in the user turn**, in the fixed template (§6.1). The app does not
     use a harness system prompt — see §6.0, a measured constraint and not a preference.
   - the source text, also in the user turn, under the instructions
   - a trailing demand to wrap the answer in `<<<TEXT>>>`/`<<<END>>>` (§6.1)

   The app sends the source text on **every** rewrite, changed or not. To work, edit the
   source and press `Ctrl+R` again. The app needs no source hashing and no automatic session
   invalidation. A text is at most a few paragraphs, so a resend is cheap.
3. **Session.** The app creates the session id lazily, on the first `Ctrl+R` of the run, and
   reuses it for every later rewrite. `Ctrl+N` discards it, so the next rewrite starts a new
   one. One session per TUI run, always.
4. **Stream.** The app resets the **output pane on spawn**, so the view shows that a new
   action started. stdout is a **JSON event stream** (§6.1a). The app renders every event
   type into the **debug pane** — streamed `text_delta`, reasoning deltas, stderr, app
   events — so progress stays visible even when the final text differs from the stream. The
   output pane gets **nothing** during the run.
5. **Settle.** Exit 0 sends the harness's final text to the output pane, trimmed, and the
   state becomes `idle`. A non-zero exit, tool-call text, or unparseable output sets state
   `error`, leaves the output pane empty for this run, and shows the debug tail.

The app reads the instructions from the instructions pane on every invocation and sends them
in the user turn, so an edit takes effect on the next rewrite.

Note: each rewrite appends another copy of the source text to the harness history. `Ctrl+N`
is the reset. There is no automatic compaction, because compaction is the harness's business.
With "a few paragraphs" as the expected input, this is no practical limit.

The app ignores `Ctrl+R` **and** `Ctrl+N` while a rewrite runs, and the status line says so.
A session reset mid-run would discard the id that the live process uses.

## 6. Harness adapters

One trait, three implementations. Adapter-specific argv lives only inside the adapter.

```rust
struct Task<'a> {
    instructions: &'a str,
    source_text: &'a str,
    model: Option<&'a str>,
    /// `None` is the first call of this run: there is no session to continue.
    session: Option<&'a str>,
    /// Where the prompt travels, as the resolution on this OS decided (§6).
    transport: Transport,
}

struct Invocation {
    program: String,
    args: Vec<String>,
    stdin: Option<String>, // None => prompt passed as an argument
                           // Some => prompt written to stdin: a Windows batch shim (§6),
                           // or codex, whose transport is stdin everywhere (§6.5)
}

trait Harness {
    fn name(&self) -> &str;
    /// `Some` when the client names sessions. `None` when the harness names its own and
    /// reports the id on its event stream (codex, §6.5).
    fn new_session_id(&self) -> Option<String>;
    /// Builds the argv for one call. `task.session == None` is the first call of the run.
    fn build(&self, task: &Task) -> Result<Invocation>;
    /// Classifies one stdout line for the debug pane. Produces no output text.
    fn classify(&self, line: &str) -> Event;
    /// The authoritative final text, when this line is the terminal event.
    fn final_text(&self, line: &str) -> Option<String>;
    /// The session id, when this line announced it. Default `None`: a client-named
    /// harness has nothing to announce (§6.5).
    fn session_from_event(&self, line: &str) -> Option<String>;
}
```

`classify` and `final_text` form the streaming surface. They are the reason each adapter
owns its event vocabulary instead of the app parsing JSON generically. `Event` is a small
enum (`Text`, `Think`, `Junk`) that labels debug lines only. A malformed or unrecognised
line maps to `Junk`, never to an error, so an unknown event type cannot abort a run.

The two session methods answer one question: **who names the session.** claude and pi accept
an id that the client chose. For them `new_session_id` returns `Some`, and the app reads the
id back out of the argv that the adapter wrote. codex generates its own id and reports it on
the event stream, so `new_session_id` returns `None` and `session_from_event` learns the id
from `thread.started` (§6.5). One method cannot express both directions, and both defaults
are safe, so the trait has two.

claude and pi take the prompt as an argument. A prompt may contain `-`, and one transport
is one implementation instead of two.

codex is the exception. Its transport is a property of the harness and not of the platform,
so §6.5 states it per adapter: codex takes the prompt on **stdin** everywhere. `Transport` in
`Task` answers a different question — "does *this resolution on this OS* force stdin" (the
Windows shim question below). A codex adapter ignores `Transport`, because its transport
follows from which harness it is.

**Windows exception, measured.** The `pi` installation on Windows is an npm shim: a `pi.cmd`
batch file, with no `pi.exe`. `cmd.exe` re-parses the command line of a batch file and
delivers no argument that carries a newline. Measured against the real template: `spawn`
fails with `InvalidInput: batch file arguments are invalid`. A `cmd /c` route truncates the
prompt at the first newline instead. The prompt template always contains newlines, so argv
cannot be the transport on that platform.

On stdin the same prompt round-trips verbatim through the shim. Measured: one assistant
`message_end`, and `agent_end` shows every newline and both markers intact. So the app
writes the prompt to stdin, and passes no positional prompt and no `--` separator, **only
when the resolved program is a Windows batch shim**. On unix, on macOS, and for claude
anywhere, argv stays the transport. This is one per-platform rule, not a second transport:
the resolution decides, and `Task` carries the answer.

`--system-prompt` carries the instructions pane, and therefore the same free text, so the
same limit applies: measured, a newline in its value also fails with `InvalidInput`. The
adapter drops that flag on a shim, which costs nothing. §6.0 shows that the flag sets
register only and that correctness never depends on it. The instructions travel in the user
turn, which on a shim is the stdin prompt.

Consequence for `Task`: an adapter cannot hardcode "the prompt is an argument". The prompt's
transport is a property of the harness on that OS, so `Task` carries the transport that the
resolution decided (§3).

### 6.0 Measured constraint: system prompt flags do not survive resume

Verified on `claude` 2.1.319 and `pi` 1.5.0. This measurement determines the design.

| Test | Result |
|---|---|
| claude fresh session, `--system-prompt` = pirate | obeys pirate |
| claude `--resume`, `--system-prompt` = pirate | **ignored**, neutral output |
| claude `--resume`, `--append-system-prompt` = pirate | **ignored**, neutral output |
| claude `--resume`, instructions in the **user turn** | obeys the new instructions |
| pi `--session-id` (create) → same id with changed `--system-prompt` | obeys the new system prompt |

claude silently discards both system-prompt flags on resume. pi honours them. A design that
relied on the flag would make "edit the instructions and press `Ctrl+R`" work on pi and do
nothing on claude, and the output would still look plausible.

**Decision: the instructions always travel in the user turn.** Every harness uses the same
template, for the first call and for a continuation. The app still passes `--system-prompt`
on the first call for claude and pi, because it sets the register for the whole session and
costs nothing, but correctness never depends on it. For the adapter this means that
`build()` produces nearly identical argv for the first call and a continuation, and only the
session flag differs. codex has no system-prompt flag, which costs nothing for the same
reason: measured, an instruction in the user turn takes effect on a continuation (§6.5).

### 6.1 Fixed user prompt

Identical for every harness and for both the first call and a continuation:

```
INSTRUCTIONS:
<instructions pane, verbatim>

TEXT TO REWRITE:
<source pane, verbatim>

Return only the rewritten text, wrapped between the markers <<<TEXT>>> and <<<END>>>.
Put nothing else between the markers. Anything outside the markers is ignored.
```

The marker instruction sits last, because a model weights the tail of the prompt most
heavily. The app fixes the template and the user cannot edit it: task instructions go in
the instructions pane, and the wire format is the app's business.

**Why markers, and not "return only the text".** The instructions pane is free text, so the
user may ask for commentary. Measured: instructions that say "add a short explanation of your
changes" produce an explanation plus `**Improved text:**` in the same reply, and that prose
offers no reliable boundary. A marker turns extraction from a *guess* into a *protocol*.
Measured on claude, pi and codex: the model wraps the result exactly as asked, even with a
preamble present.

Measured with markers:

| input | output |
|---|---|
| instructions asking for an explanation | explanation, blank line, `<<<TEXT>>>`, text, `<<<END>>>` |
| instructions asking for formal text only | `<<<TEXT>>>` / text / `<<<END>>>` |
| pi, same template | `<<<TEXT>>>text<<<END>>>` — **single line, no newlines around markers** |

So extraction does not assume that the markers stand on their own lines. It splits on the
marker strings, wherever they are.

### 6.1a Extracting the rewritten text

claude and pi run in **streaming JSON mode** (§6.1b). codex runs in its JSON mode, which is
not a stream (§6.5). The app feeds every stdout line to the adapter, which routes the line
to the debug pane. Only the text **between the markers** reaches the output pane.

Per harness, measured on claude 2.1.319, pi 1.5.0 and codex-cli 0.159.0:

| | claude | pi | codex |
|---|---|---|---|
| flag | `--output-format stream-json --verbose --include-partial-messages` | `--mode json` | `exec --json` (same flag on `exec resume`) |
| final text event | `type == "result"` → `result` | last `message_end`, `role == "assistant"` | last `item.completed` with `item.type == "agent_message"` → `item.text` |
| streamed text delta | `stream_event` → `event.type == content_block_delta` → `delta.type == text_delta` → `delta.text` | `message_update` → `assistantMessageEvent.type == text_delta` → `delta` | **none** — measured: codex emits no delta events |
| reasoning delta | `delta.type == thinking_delta` | `assistantMessageEvent.type == thinking_delta` | **none**, though `reasoning_output_tokens` is non-zero |
| session id | client-chosen, in the argv | client-chosen, in the argv | from `thread.started` → `thread_id` |

**The terminal text is the last `agent_message`, and never a tool item.** codex reports
shell commands, file changes, web searches and MCP calls as their own `item.completed`
events, each with output. A rule of "the last item with text" would put the stdout of a
command in the output pane. Only `item.type == "agent_message"` is prose.

**The app never reconstructs text by concatenation.** Measured on claude: the terminal
`result` event carries the complete answer, and the concatenated `text_delta`s equal it
exactly (76 bytes against 76 bytes). The app shows a delta in the debug pane and nowhere
else, so no output text depends on a delta. This rule removes a whole class of bugs, where a
dropped delta silently truncates the output. For codex the rule is trivial and still
binding: codex emits no deltas to fall back on, so "exit 0 with no `agent_message`" is an
error with nothing to guess from.

**claude requires `--include-partial-messages`.** Measured without it: the run emits
`system`, `assistant`, `rate_limit_event`, `result` and **zero** `stream_event`s, so there
is no progress even though the answer is complete. Measured with it: 11 `stream_event`s, 6
text deltas, and the deltas match `result`. The flag is what turns the run into a stream.

Extraction. The app applies these steps once per run, to the terminal text:

1. Find `<<<TEXT>>>` and `<<<END>>>` as plain substrings, in that order.
2. Take everything strictly between them. Newlines, indentation and markers on their own
   lines are not required — measured: pi returns `<<<TEXT>>>text<<<END>>>` on one line.
3. When both markers are present, the run is a **success**, whatever surrounds them. The
   app discards everything outside, which is the point of the protocol.
4. **When a marker is missing, the run is an error, not a guess.** Show the raw terminal text
   in the **debug pane**. The output pane holds final text only (§6.1b) and stays as the
   spawn left it (§5, step 5). Set state `error` with "model did not use the output
   markers". A fallback to trimming or preamble-stripping is the heuristic that the markers
   exist to remove.
5. Trim the whitespace around the extracted text and keep internal newlines verbatim.
   Measured: paragraphs inside the markers keep their blank lines.
6. When the extracted text contains a tool-invocation block (`<invoke name=…>`), the run is
   an **error**. Show the block and explain it. Measured cause: with tools denied the model
   prints the call as text and exits 0. Nothing executed, but the text is not a rewrite.
7. No markdown unwrapping and no commentary stripping after step 2. The markers already
   isolate the answer, so a fence that the model put *inside* them is part of the text.

Why the rule is strict: markers are cheap for the model, and every measured case honoured
them, including when the prompt also asked for commentary. A missing marker means that
something is wrong: truncation, a refusal, or a confused model. Showing whatever text arrived
as the rewrite is the failure mode that this design prevents. The debug pane always holds the
full raw output, so diagnosis costs one keypress.

claude and pi buffer stdout in plain-text mode, so the design requires JSON streaming rather
than treating it as optional (§6.1b). The codex JSON mode buffers per item instead (§6.5),
which is the same problem one level up.

**Rejected alternative:** `claude --output-format json` (one result object, and `.result` is
exactly the answer with no wrapper). It cannot stream, so it gives no progress signal, and it
gives claude a JSON shape that pi does not share.

### 6.1b Streaming

The debug pane shows progress incrementally. The output pane never shows progress.

Measured throughput on a long rewrite: claude emits continuously from t=1s, pi from t=1s.
Plain-text mode buffered both (claude wrote nothing for 13 s, then the whole answer), so
JSON streaming is the only way to keep the window live during a run.

Consequences:

- The app writes the **output pane** exactly once per run, when the terminal event arrives.
  Between runs the pane shows the last good result. It never shows partial text.
- The **debug pane** receives every stdout event, all stderr, and app events. §7 gives the
  labels. A rejected answer's raw text arrives under `raw`, so a reader never mistakes it for
  progress. Reasoning appears here on purpose: this is the debug view, and its content cannot
  be mistaken for the result.
- A spawn clears the output pane, so the previous result is deliberately lost at that
  moment. `Ctrl+Z` undoes edits to the two editable buffers only, and never the read-only
  panes.
- The panes show markdown syntax literally. A half-emitted fence or `**` appears as text, so
  a partial stream cannot damage what the user sees.
- The app drains the `mpsc` channel once per UI tick (~16 ms) and renders once per tick. §7
  gives the measured event volume. Per-event redraws would be wasteful, so per-tick
  batching is a requirement, not an optimisation.

**claude requires `--verbose` and `--include-partial-messages` for streaming.** Measured:
`--output-format stream-json` with `-p` exits 1 and prints
`Error: When using --print, --output-format=stream-json requires --verbose`. Without
`--include-partial-messages` the run emits zero `stream_event`s, so there is no progress to
show. Both flags are mandatory. pi needs neither, because `--mode json` streams as-is.

**codex cannot stream. This is a limitation of the harness, not a missing flag.** Measured on
a 300-word answer: `exec --json` emits `thread.started`, `turn.started`, then **one**
`item.completed` that carries the whole `agent_message` at t≈21 s, then `turn.completed`.
That mode has no per-token event. The binary does contain an `AgentMessageDeltaNotification`,
but that notification belongs to the app-server protocol and not to `exec --json`. So on
codex the debug pane is inert between items by necessity: the first lines land at t≈0.7 s,
and nothing follows until the answer arrives whole. During a codex run the **status line's
`running` state** is the progress signal. That is why the app draws the state from the state
machine and does not infer it from arriving text. Reach for the app-server transport only if
it becomes public: the app already shows what the harness gives it, and the output pane is
written once per run either way.

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

- `--session-id` requires a valid UUID v4, so the adapter generates one. Measured: an empty
  or invalid id makes `--resume` fail with exit 1 and `Error: --resume requires a valid
  session ID or session title when used with --print`. A missing id is a bug, and the app
  never sends one.
- `--system-prompt` replaces the claude coding-agent prompt entirely. The app passes it for
  register only (§6.0), and correctness never depends on it.
- `--tools ""` is documented as "use `""` to disable all tools". The argv must carry the flag
  with an empty value, because an omitted flag gives the default tool set. Measured: three
  attempts to make claude write a file each failed, and asked to enumerate its tools claude
  answered `NONE`.
- Measured: with `--permission-prompts none` absent, a prompt that makes the model attempt a
  denied tool call stalls until the timeout, because no one answers the permission prompt.
  With the flag the same case returns in seconds with the call printed as text (§6.1a, step
  6). The flag serves latency, not safety.
- `--verbose` is mandatory with `-p` and `stream-json`. See §6.1b.
- `--include-partial-messages` is **required**, because without it the run emits no
  `stream_event` at all (§6.1a). `content_block_delta` does not arrive by default.

### 6.3 pi

The first call and a continuation use the *same* argv. The pi `--session-id` means "open the
existing session with this id, or create it":

```
pi --print --session-id <id> --mode json
   --system-prompt <instructions>
   --no-tools --no-context-files --no-extensions --no-mcp
   --no-skills --no-prompt-templates --offline
   -- <user prompt>
```

- Session ids accept `[A-Za-z0-9._-]` and must start and end with a letter or digit. The
  adapter generates `psc-<uuid>`, which has no trailing hyphen and starts with a letter.
- `--mode json` is the streaming form. Measured: text deltas arrive from t=1s, ~547 JSON
  lines per run. pi needs no extra flag to stream.
- `--system-prompt` replaces the pi agent prompt. With all tools, skills, MCP and context
  files disabled, that agent prompt has nothing to do, so a replacement prompt is the right
  choice.
- `--no-context-files` is load-bearing. Measured separately: without it the system message
  carries `project_context`, 2759 bytes of the user's own `AGENTS.md`, into every request.
  With it the system message sections are only `preamble` and `cwd`. A rewrite must not
  inherit the user's coding guidelines by accident.
- Measured: pi obeys a changed `--system-prompt` on an existing session, unlike claude. The
  app sends the instructions through the user turn anyway, for uniformity (§6.0): one
  template, one code path, no per-harness instruction plumbing.
- Measured: the first use of an id prints this line to **stderr**:
  `Warning: No project session found with id '…'; creating a new session with that id.`
  The debug pane shows it. The line is harmless, so the app filters nothing.

### 6.4 Why a trait, and what it costs

The three harnesses differ in five places:

- the session flag for a new session and for a continuation
- the id format: UUID v4, or `[A-Za-z0-9._-]`, or a harness-chosen `01a1…`
- the tool-denial flags
- the event vocabulary: `stream_event`/`result`, `message_update`/`agent_end`, or
  `item.completed`/`thread.started`
- **who names the session**: the client, or the harness (§6)

All else is common to the three: the prompt template, the output rules, and process
management. The event vocabulary is the largest of the five, and it is the reason `classify`
and `final_text` sit on the trait. Who names the session is the reason `new_session_id`
returns an `Option` and `session_from_event` exists.

`Harness::build` is a pure function, so each adapter gets table tests over its argv and
starts no process. `Task` carries `session: Option<&str>`, where `None` means "first call of
this run". The adapter decides which flag that implies, not the app.

### 6.5 codex

Measured on codex-cli 0.159.0. This section is longer than §6.2 and §6.3 because two of its
findings contradict what the rest of §6 assumes about *any* harness: codex has no
tool-denial flag and runs in a sandbox instead, and codex cannot stream.

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

The prompt travels on **stdin** in both shapes. `-- -` makes the positional a literal dash,
so codex reads the prompt from the pipe. `Invocation.stdin` is therefore always `Some`, and
`args` never carries the prompt. Measured: both transports round-trip the real template with
every newline and both markers intact, so this is a choice and not a workaround. It keeps
user prose out of the OS argument list, and it avoids the batch-shim limit that §6 documents.

The app measured each flag below. Each reason keeps its flag from being "tidied" away.

- **`-c sandbox_mode="read-only"` *and* `-c approval_policy="never"`, together.** This pair
  replaces `--tools ""` and `--no-tools`, and the pair is load-bearing. Measured:
  `-s read-only` alone leaves approvals on-request, and a write succeeded in 3 of 3 runs.
  `-c sandbox_mode` alone behaves the same way. The two flags together blocked every attempt
  (9 of 9, across a git repo and a plain directory, including a shell download and a write
  attempted through the residual JS REPL, which failed with `EPERM`). The adapter uses `-c`
  and not `-s` because `exec resume` rejects `-s` (`unexpected argument`), and one flag shape
  for both call shapes is one place to get wrong instead of two.
- **`-c project_doc_max_bytes=0`.** This is codex's `--no-context-files`. Measured: without it
  the prompt contains the user's own `AGENTS.md`, and the model reproduces that file verbatim
  when asked. With it, the file is absent. A rewrite must not inherit the user's coding
  guidelines by accident.
- **`-c features.apps=false`.** Measured: the offered tools drop from 147 to 29, and the
  whole Codex app MCP surface leaves the prompt.
- **`-c web_search="disabled"`, which works only in combination with the flag above.**
  Measured, and the pairing is the whole point: with `features.apps=false` alone the model
  can still search (`apps=false only` returned a live headline through the `web__run` tool),
  and with `web_search="disabled"` alone it can still search too. Only the two together
  produced `NO-WEB-ACCESS`. So neither flag is redundant, and neither flag is sufficient.
- **Not used: `-c tools.web_search=false`.** Measured: it does not remove the tool
  (`web__run` stayed), so it would read as a denial that denies nothing.
- **`--skip-git-repo-check`.** Measured: without it codex refuses to run outside a git repo
  with `Not inside a trusted directory`, which would fail the app in every directory that is
  not a checkout.
- **`--json`.** This is the machine-readable mode. `exec resume` needs the flag repeated.

**Deliberately absent: `--ignore-user-config` and `-s`.** `--ignore-user-config` disables the
user's own `config.toml`, which holds their model choice, and measured it also changed
whether codex blocked a write. Using it would trade "codex respects the user's config" for a
second, weaker layer of denial. A later change that reaches for either flag is a regression,
so §6.5 names both.

**codex names its own threads, so a session starts with the harness.** codex ignores
`-c thread_id=…`, and the id appears only on stdout as
`{"type":"thread.started","thread_id":"01a1bc…"}`. So `new_session_id` returns `None`, and
the app learns the id through `session_from_event` (`§6`) when it takes the first id on
offer. The app keeps that id for the run. `Ctrl+N` works, because an unnamed first call is a
new thread and the app learns its id from its own stream.

**On codex, tool denial is sandbox-level, which is weaker than §1 asks.** codex cannot lose
its tools: `--strict-config` rejects the config keys that look like denials (`unknown
configuration field`), or they do nothing, and a JS REPL (`mcp__node_repl__js`,
`mcp__cua_repl.js`) survives every filter. The sandbox holds: read-only filesystem, no
approval ever answered, network refused (measured: a shell download fails). No measured run
reached the filesystem, which is §1's actual requirement. The sandbox enforces that
requirement one layer lower than the tool denial does for claude and pi, so "tools are
disabled" would be false for codex. A human accepted this trade instead of dropping codex
support. The app could refuse the adapter, and the design rejected that alternative because
the observable guarantee holds: no measured run writes anything.

**Residual cost, accepted.** codex injects its own developer prompt and a skills catalogue
(~30 KB of local `SKILL.md` descriptions, ~18 k input tokens) into every call, and no config
key found so far suppresses it (`skills_instructions`,
`include_skills_usage_instructions`, `features.skill_search=false` are all rejected). It
costs tokens and register, not correctness: measured, the markers still come back.

**No stream.** See §6.1b: codex emits one complete `agent_message`, so the debug pane fills
in jumps. §6.1b records this because it is a property of the JSON mode, not of the argv.

Measured end to end on the exact argv above: the first call returned
`<<<TEXT>>>Arrr, send me the report, matey.<<<END>>>`. A continuation with changed
instructions returned `<<<TEXT>>>You are hereby requested to furnish the report to the
undersigned.<<<END>>>` on the same thread id. The user's `AGENTS.md` never appeared, and no
measured run created a file.

## 7. Process handling

- `std::process::Command` with `stdout` and `stderr` piped. One reader thread per stream
  reads with `BufRead::lines` and sends `(Stream, String)` over an `mpsc` channel.
- The UI thread never blocks. It drains the channel each tick (~16 ms) and redraws once per
  tick. Measured event volume per rewrite: ~280 lines for claude, ~550 for pi and **~7 for
  codex**. Per-event redraws would be wasteful, so per-tick batching is a requirement, not an
  optimisation.
- Each stdout line goes to the active harness's `classify` and reaches the debug pane with its
  kind. The app consults `final_text` on every line. At most one line per run returns `Some`,
  and that text is what the output pane receives. A line that is not JSON shows as `junk`: a
  harness that prints garbage is a diagnosable event, not a failure.
- For a harness that names its own session, the app also consults `session_from_event` on
  every line until it has an id, and only until then. codex announces its thread once, and a
  later id can only belong to a run that the app did not start.
- Cancellation: the app kills the child and settles the run as an error with the note
  `cancelled`. The output pane stays as the spawn left it. A cancelled run produces no final
  text by definition, so there is nothing to show and nothing to reconcile.
- Wall-clock timeout: 180 s by default. On timeout the app kills the child and reports.
- The debug pane is a bounded ring buffer (2000 lines), so memory stays flat.
- The app converts non-UTF-8 output lossily.
- **Both streams reach the debug pane, and neither reaches the output pane.** The app labels
  every debug line by origin: stdout events by kind, stderr as `err`, the model's raw text for
  a rejected answer as `raw`, and its own run start, run end, exit status, duration and error
  notes as `app`. The app writes the output pane once, from the terminal event.

## 8. Errors

Each failure produces a human-readable status-line message and a debug-pane entry. The app
must not panic on harness misbehaviour, and must restore the terminal on every exit path:
success, error, and panic.

Measured failure modes worth encoding:

| Condition | Behaviour |
|---|---|
| Harness binary not found | Startup error to stderr, non-zero exit, no UI drawn (§3) |
| Empty instructions | Validation error, focus moves to the instructions pane |
| Empty source text | Validation error, focus moves to the source pane |
| Session id missing or invalid with `--resume` | Never reachable. The id is internal state and the app generates it before use. An adapter that receives no id opens a new session, and does not send `--resume` with an empty one |
| The harness never announces a session id | The id stays unknown and the status line keeps `—`. The next rewrite is a first call, which opens a new session. This is not an error by itself. Only a run that also produced no terminal text ends in error |
| Non-zero exit | State `error`, debug tail visible (claude puts the reason on stderr), output pane keeps the cleared state for this run |
| Timeout | Kill, state `error`, debug pane notes the timeout |
| Clipboard unavailable | Status-line warning, and every other feature still works |
| Non-UTF-8 output | Lossy conversion |
| Markers honoured | The text between `<<<TEXT>>>` and `<<<END>>>` becomes the output, and the app discards everything outside (§6.1a) |
| Missing output markers | State `error`, raw terminal text shown, debug pane holds the full output (§6.1a) |
| Tool-call XML between markers | State `error`, text shown, debug pane explains the tool denial (§6.1a) |
| Exit 0 with no terminal event | State `error`. Without a `result`, `agent_end`, or `agent_message` there is no final text, and the app never concatenates deltas as a fallback (§6.1a) |

Exit codes: `0` normal, `1` runtime failure (including a missing harness), `2` CLI misuse.

## 9. Implementation

Crates: `ratatui`, `crossterm`, `clap` (derive), `arboard`. The adapter generates session ids
from `std::hash::RandomState` plus the clock (`src/harness/claude.rs`), so there is no `uuid`
crate. Errors are per-module types, so there is no `anyhow` crate.

```
src/main.rs          startup, terminal setup/teardown, panic hook that restores the TTY,
                     the event loop and the paste-burst collector
src/cli.rs           arg parsing and startup validation
src/layout.rs        pane geometry and focus
src/ui.rs            pane rendering, keybindings to actions, status line, help panel
src/editor.rs        buffer, cursor, edit operations
src/wrap.rs          soft wrap and wrap-aware movement
src/undo.rs          per-pane undo stack
src/clipboard.rs     the system clipboard
src/rewrite.rs       the rewrite state machine
src/panes.rs         output and debug pane contents, stream routing
src/process.rs       spawn, stream readers, cancel, timeout
src/marker.rs        extracting the rewritten text from the terminal event
src/harness/mod.rs   Harness trait, Task, Invocation, the fixed prompt
src/harness/resolve.rs  harness name to the program that a spawn needs
src/harness/claude.rs
src/harness/pi.rs
src/harness/codex.rs
```

Ratatui ships no text editor widget, so the two editable panes need one small shared editor
type. It holds a buffer and a cursor, moves vertically with soft-wrap awareness, and applies
edits. It has no selection. The app owns one definition and both panes share it.

Undo is a bounded stack of buffer snapshots, kept per pane beside the wrap state and not
inside the editor type. A snapshot is `(lines, cursor)`, and a history of editors stored
inside an editor would make the snapshot recursive. One step is one public editor call,
which is why the editor's public methods do not call each other: a paste is one call, and
therefore one step. The app records a snapshot only when the buffer changes, so a no-op edit
costs nothing.

Testability: `Harness::build` is pure, so each adapter gets argv table tests. The app tests
`process.rs` against `tests/fixtures/*.sh` run under `sh` (fixed output, a sleep, a hang, a
non-zero exit). The state machine runs headless, with the terminal backend stubbed.

## 10. Verification

Unit tests cover each module. The checks below need a real harness or a real terminal, and
they are the manual script to follow before a harness-related change ships.

1. **Shell.** Args, terminal setup and teardown, the four-pane layout, focus cycling
   (`Tab`/`Shift+Tab`/`Ctrl+←→`), editing, clipboard load/copy/paste, quit. Check: a manual
   run needs no harness, every pane is reachable, and focus is visible.
2. **Streaming rewrite.** Claude adapter (`stream-json`, `--verbose`,
   `--include-partial-messages`), process spawn, JSON line → `classify` → debug pane,
   `final_text` → output pane, validation. Check: the debug pane advances during a >10 s
   rewrite instead of sitting inert. The output pane stays empty until the run ends, then
   shows the complete text. The app refuses empty instructions and moves focus to the
   instructions pane. `think` lines appear in the debug pane only. A response with no markers
   sets state `error` and never lands in the output pane.
3. **Sessions.** Session id lifecycle, resend into an existing session, `Ctrl+N`. Check: with
   a new instruction ("now as a pirate") the second rewrite visibly changes register. This is
   the regression test for §6.0, which would silently pass with a plausible-looking answer if
   the instructions went back to the system-prompt flag. `Ctrl+N` then forgets the context.
4. **pi adapter.** Check: argv table tests, plus one live run that asserts that a changed
   instruction takes effect on the same session id, and that `--no-context-files` keeps
   `AGENTS.md` out of the system message.
5. **Robustness.** Cancel, timeout, error paths, resize, a missing terminal event, and
   ring-buffer trimming in the debug pane.
6. **codex adapter.** Check: argv table tests for both call shapes. The app learns the thread
   id from `thread.started`, and the id survives a continuation, so a changed instruction
   takes effect on the same thread (§6.0 regression, measured pirate → legal register).
   `Ctrl+N` produces a different thread id and drops the context. A prompt that tells the
   model to write a file leaves no file behind, and a prompt that asks the model to quote
   `AGENTS.md` yields nothing. The debug pane does **not** stream, per §6.1b, and that is not
   a defect.

## Decisions

Instructions travel in the user turn (§6.0, measured). The app resends the source text on
every rewrite, which is cheap for a few paragraphs. There is one session per run, and there
is no persistence. `Esc` does nothing. `Ctrl+Q` quits and is the only quit key. `Ctrl+C`
copies the focused pane, and cancels only in the debug pane after a second press, so the key
keeps its terminal meaning of copy. The output pane holds final text only. The debug pane
holds the stream, reasoning, stderr and app events. A spawn clears the output pane. The app
checks the harness binary at startup, and a missing binary means a stderr error and a
non-zero exit.

Tool denial holds for claude and pi (three claude attempts and one pi attempt created no
file). On codex it is a **sandbox** rather than a denial (§6.5). That is the one place where
the three harnesses are not equal. A human accepted it deliberately, and §1 states it rather
than leaving it for a reader to infer.

Output **streams** (§6.1b). codex cannot stream. The user fixes the harness and the model at
init from the command line, and the UI offers no selection. The app reads `--prompt-file` and
`--text-file` once at startup and never re-reads them, so a restart picks up an edit to
either file. The
instructions pane itself is editable, and every rewrite sends it. A `Ctrl+N` while running is
ignored like `Ctrl+R`. The name stays `prosecraft`. The rewritten text sits between the
markers `<<<TEXT>>>` and `<<<END>>>`. The markers make extraction a protocol, so a preamble
cannot leak through and a missing marker is an error rather than a guess. Undo covers the two
editable buffers and only them (§4, §9): a capped per-pane snapshot stack, one step per key
event, no redo, and it never reaches `output` or `debug`.
