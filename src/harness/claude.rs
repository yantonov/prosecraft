//! The `claude` adapter (spec §6.2).
//!
//! Every flag here was measured, not guessed, and each one below carries the reason it
//! exists. Nothing in this file starts a process: [`Claude`] only assembles an argv, labels
//! a line, and reads one field out of the terminal event, so the whole adapter is testable
//! as a table of strings.
//!
//! Lines are read with the small JSON reader at the bottom of this file rather than a
//! `serde_json`. Lines are a single flat object produced by claude, one field of which is
//! needed; a JSON dependency would be a new entry in `Cargo.toml`, which this project
//! treats as an escalation, and the reader also makes the "malformed is `Junk`, never an
//! error" rule (spec §6) a property of one `Option` instead of an error-mapping layer.

use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{BuildError, Event, HarnessAdapter, Invocation, Task, prompt};

/// The claude harness as an adapter.
///
/// Holds no state. A session id is generated where it is used and reported through the argv
/// it appears in, so there is no cached value that could disagree with what was actually
/// sent (see [`HarnessAdapter::build`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct Claude;

impl HarnessAdapter for Claude {
    fn name(&self) -> &str {
        "claude"
    }

    fn new_session_id(&self) -> String {
        new_uuid_v4()
    }

    /// Builds the argv from spec §6.2, differing between first call and continuation only
    /// in the session flag — `--session-id` creates, `--resume` continues.
    ///
    /// `Task::session == None` selects `--session-id` with a freshly generated id, and that
    /// id is the element after the flag in the returned argv: it is not stored anywhere, so
    /// the caller persists it by reading it from there (spec §5 step 3, one session per
    /// run). Storing it here would make `--session-id` unreachable after the first call and
    /// would put mutable state in a function the spec asks to be pure.
    fn build(&self, task: &Task<'_>) -> Result<Invocation, BuildError> {
        let first_call = task.session.is_none();
        let session = task.session.map_or_else(new_uuid_v4, str::to_owned);

        let mut args = Vec::with_capacity(20);
        // `-p` is the one-shot, non-interactive form; the app depends on claude neither
        // opening a window nor waiting for input.
        args.push("-p".to_owned());
        args.push(
            if first_call {
                "--session-id"
            } else {
                "--resume"
            }
            .to_owned(),
        );
        args.push(session);

        // `--model` is not part of the measured spec §6.2 argv; it is a pass-through from
        // `--model` on the prosecraft command line. Placed directly after the session
        // because that keeps the measured streaming and tool-denial block contiguous and
        // in spec order. The position is a readability choice, not a measurement.
        if let Some(model) = task.model {
            args.push("--model".to_owned());
            args.push(model.to_owned());
        }

        // `--verbose` is mandatory with `-p` and `stream-json` (claude exits 1 without it),
        // and `--include-partial-messages` is what makes the run a stream at all: without
        // it claude emits zero `stream_event`s and the debug pane sits inert (spec §6.1b).
        args.push("--output-format".to_owned());
        args.push("stream-json".to_owned());
        args.push("--verbose".to_owned());
        args.push("--include-partial-messages".to_owned());

        // Passed on the first call to set the register for the session, and on
        // continuations because claude silently ignores it there anyway (spec §6.0).
        // Instructions also travel in the user turn, so correctness never depends on it.
        args.push("--system-prompt".to_owned());
        args.push(task.instructions.to_owned());

        // The flag must be present *with* an empty value: omitted means claude's default
        // tool set, and `""` is documented as "disable all tools" (spec §6.2).
        args.push("--tools".to_owned());
        args.push(String::new());

        // For latency, not safety: an unanswered permission prompt stalls the run until the
        // timeout, while the same request returns in seconds when the model is told there
        // is nobody to ask (spec §6.2).
        args.push("--permission-prompts".to_owned());
        args.push("none".to_owned());

        // The separator, then the prompt as exactly one argument. Newlines and a leading
        // `-` survive because the element is never joined or re-split (spec §6).
        args.push("--".to_owned());
        args.push(prompt(task));

        Ok(Invocation {
            program: self.name().to_owned(),
            args,
            // Both harnesses take the prompt as an argument, so there is nothing to write
            // to the child's stdin (spec §6).
            stdin: None,
        })
    }

    /// Labels one stdout line for the debug pane.
    ///
    /// `Text` means streamed prose and is reachable only from a `text_delta` that carries a
    /// string `text` field. That is the point of classifying per adapter: a reasoning
    /// delta, a signature, a tool call or an unparseable line can never be labelled the way
    /// prose is, so the debug pane's notion of "the model is writing" is exact.
    ///
    /// `signature_delta` maps to `Think` rather than to its own kind: it belongs to a
    /// thinking block, carries no prose, and the debug pane already shows that block as
    /// reasoning. A fourth kind would be a distinction nothing consumes.
    fn classify(&self, line: &str) -> Event {
        let Some(json) = parse(line) else {
            return Event::Junk;
        };

        // Before any match on the event shape: a tool call or an error object must never be
        // labelled prose, whatever else it looks like.
        if carries_tool_use(&json) {
            return Event::Junk;
        }

        match field(&json, "type").and_then(Json::as_str) {
            Some("stream_event") => classify_stream_event(&json),
            // The terminal event is not a delta. The output pane receives it through
            // `final_text`, so labelling it prose here would only duplicate the answer
            // under a label that means "a delta arrived".
            Some("result") => Event::Junk,
            // `system`, `assistant`, `rate_limit_event`, and anything a future claude
            // version adds: all diagnosable, none of them prose.
            _ => Event::Junk,
        }
    }

    /// The terminal `result` event carries the complete answer (spec §6.1a), which is why
    /// nothing is ever rebuilt from deltas.
    ///
    /// Every other line returns `None`: "exit 0 but no terminal event" is an error state,
    /// not a reason to fall back to concatenation. An `is_error` result is treated as no
    /// final text for the same reason — its `result` field holds a diagnostic message, and
    /// presenting a message as the rewrite is the failure the markers exist to prevent.
    fn final_text(&self, line: &str) -> Option<String> {
        let json = parse(line)?;
        if field(&json, "type").and_then(Json::as_str) != Some("result") {
            return None;
        }
        if field(&json, "is_error") == Some(&Json::Bool(true)) {
            return None;
        }
        match field(&json, "result")? {
            Json::Str(text) => Some(text.clone()),
            _ => None,
        }
    }
}

/// Classifies the inside of a `stream_event`.
fn classify_stream_event(json: &Json) -> Event {
    let Some(event) = field(json, "event") else {
        return Event::Junk;
    };
    if field(event, "type").and_then(Json::as_str) != Some("content_block_delta") {
        return Event::Junk;
    }
    let Some(delta) = field(event, "delta") else {
        return Event::Junk;
    };
    match field(delta, "type").and_then(Json::as_str) {
        // Prose is only prose when it actually carries text.
        Some("text_delta") if matches!(field(delta, "text"), Some(Json::Str(_))) => Event::Text,
        // Reasoning and the reasoning block's signature: both belong to the debug pane and
        // to nothing else.
        Some("thinking_delta" | "signature_delta") => Event::Think,
        _ => Event::Junk,
    }
}

/// Whether anything in the line names a tool use, a tool result, or an error.
///
/// Walks the whole tree instead of a fixed path, because a tool call arrives in a different
/// shape from each of claude's event types and a missed one would be shown as streamed
/// prose. Measured cause (spec §6.1a step 6): with tools denied the model prints the call
/// as text, and with a permission prompt pending the run stalls; either way nothing
/// executed, and neither is a rewrite.
fn carries_tool_use(value: &Json) -> bool {
    match value {
        Json::Object(fields) => fields.iter().any(|(key, value)| match key.as_str() {
            "type" => matches!(
                value.as_str(),
                Some("tool_use" | "server_tool_use" | "tool_result")
            ),
            // An empty list is how claude reports "no tools used", so only a non-empty one
            // is a call. Treating the field itself as the signal would label ordinary
            // turns as tool calls.
            "tool_calls" | "tool_call" => !value.is_empty(),
            "is_error" => value == &Json::Bool(true),
            _ => carries_tool_use(value),
        }),
        Json::Array(items) => items.iter().any(carries_tool_use),
        _ => false,
    }
}

/// A version 4 UUID in the canonical lowercase `8-4-4-4-12` form.
///
/// `claude --session-id` requires a valid v4 UUID: an invalid or empty id makes the next
/// call fail with exit 1 and `Error: --resume requires a valid session ID when used with
/// --print` (spec §6.2).
///
/// **Not a CSPRNG, and that is fine here.** The bits come from [`std::hash::RandomState`]
/// (randomly keyed per instance), the clock, and the process id, so two calls differ. The
/// id is a name the harness looks a session up by: not a secret, not a token, and nothing
/// is gained by predicting it. A `uuid` dependency would buy cryptographic quality this use
/// has no need for, and adding a dependency is on the project's escalate list.
fn new_uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();

    // Two independently keyed hashers, so the halves do not come from one 64-bit output
    // that a repeated call could line up with.
    for (half, salt) in [(0, 0u64), (8, 1u64)] {
        let mut hasher = std::hash::RandomState::new().build_hasher();
        hasher.write_u64(salt);
        hasher.write_u128(nanos);
        hasher.write_u32(std::process::id());
        bytes[half..half + 8].copy_from_slice(&hasher.finish().to_ne_bytes());
    }

    // Version 4 (random) and variant 10xx: the two nibbles a v4 parser checks.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// A JSON value, only as much as one claude stdout line needs.
#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    /// The text is kept only so `PartialEq` distinguishes values; nothing reads a number.
    Number,
    Str(String),
    Array(Vec<Json>),
    /// Field order is preserved, because a duplicate key's first occurrence wins.
    Object(Vec<(String, Json)>),
}

impl Json {
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(text) => Some(text),
            _ => None,
        }
    }

    /// `true` for an empty array or object, which is how claude says "none".
    fn is_empty(&self) -> bool {
        match self {
            Self::Array(items) => items.is_empty(),
            Self::Object(fields) => fields.is_empty(),
            _ => false,
        }
    }
}

/// The value under `key` in an object; `None` for a non-object or an absent key.
fn field<'a>(value: &'a Json, key: &str) -> Option<&'a Json> {
    match value {
        Json::Object(fields) => fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value),
        _ => None,
    }
}

/// Parses one line as a single JSON value, rejecting trailing content.
///
/// `None` for anything that is not strictly one JSON value: a harness printing garbage, a
/// truncated line, or a second object appended to the first. The caller maps that to
/// [`Event::Junk`], so a malformed line is a diagnosable debug entry and never an error
/// (spec §6).
fn parse(line: &str) -> Option<Json> {
    let mut parser = Parser {
        bytes: line.as_bytes(),
        at: 0,
    };
    let value = parser.value()?;
    parser.whitespace();
    (parser.at == parser.bytes.len()).then_some(value)
}

/// A strict recursive-descent reader. Iterative parsing would be faster, but a line is a
/// few hundred bytes and nesting is a handful of levels; clarity wins.
struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.at += 1;
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.whitespace();
        match self.peek()? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Json::Str),
            b't' => self.literal(b"true", Json::Bool(true)),
            b'f' => self.literal(b"false", Json::Bool(false)),
            b'n' => self.literal(b"null", Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn literal(&mut self, word: &[u8], value: Json) -> Option<Json> {
        if self.bytes[self.at..].starts_with(word) {
            self.at += word.len();
            Some(value)
        } else {
            None
        }
    }

    fn number(&mut self) -> Option<Json> {
        let start = self.at;
        self.eat(b'-');
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.at += 1;
        }
        // One char must have been consumed, or the `-` was alone.
        (self.at > start).then_some(Json::Number)
    }

    fn string(&mut self) -> Option<String> {
        if !self.eat(b'"') {
            return None;
        }
        let mut out = String::new();
        loop {
            match self.peek()? {
                b'"' => {
                    self.at += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.at += 1;
                    let escaped = self.peek()?;
                    self.at += 1;
                    out.push(match escaped {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode_escape()?,
                        _ => return None,
                    });
                }
                byte if byte < 0x20 => return None,
                _ => {
                    // Copy one whole UTF-8 char, so multi-byte content survives.
                    let text = std::str::from_utf8(&self.bytes[self.at..]).ok()?;
                    let c = text.chars().next()?;
                    self.at += c.len_utf8();
                    out.push(c);
                }
            }
        }
    }

    /// A `\uXXXX` escape, combining a surrogate pair when one follows.
    fn unicode_escape(&mut self) -> Option<char> {
        let first = self.hex4()?;
        let high = u32::from(first);
        let combined = if (0xd800..0xdc00).contains(&high) {
            if !self.eat(b'\\') || !self.eat(b'u') {
                return None;
            }
            let low = u32::from(self.hex4()?);
            0x10000 + ((high - 0xd800) << 10) + (low.checked_sub(0xdc00)?)
        } else {
            high
        };
        char::from_u32(combined)
    }

    fn hex4(&mut self) -> Option<u16> {
        let mut value = 0u16;
        for _ in 0..4 {
            let digit = self.peek()?;
            self.at += 1;
            value = value * 16
                + match digit {
                    b'0'..=b'9' => u16::from(digit - b'0'),
                    b'a'..=b'f' => u16::from(digit - b'a') + 10,
                    b'A'..=b'F' => u16::from(digit - b'A') + 10,
                    _ => return None,
                };
        }
        Some(value)
    }

    fn array(&mut self) -> Option<Json> {
        if !self.eat(b'[') {
            return None;
        }
        let mut items = Vec::new();
        self.whitespace();
        if self.eat(b']') {
            return Some(Json::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.whitespace();
            if self.eat(b']') {
                return Some(Json::Array(items));
            }
            if !self.eat(b',') {
                return None;
            }
        }
    }

    fn object(&mut self) -> Option<Json> {
        if !self.eat(b'{') {
            return None;
        }
        let mut fields = Vec::new();
        self.whitespace();
        if self.eat(b'}') {
            return Some(Json::Object(fields));
        }
        loop {
            self.whitespace();
            let key = self.string()?;
            self.whitespace();
            if !self.eat(b':') {
                return None;
            }
            fields.push((key, self.value()?));
            self.whitespace();
            if self.eat(b'}') {
                return Some(Json::Object(fields));
            }
            if !self.eat(b',') {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task<'a>(instructions: &'a str, source_text: &'a str, model: Option<&'a str>) -> Task<'a> {
        Task {
            instructions,
            source_text,
            model,
            session: None,
        }
    }

    fn harness() -> Claude {
        Claude
    }

    /// A `stream_event` line carrying one `content_block_delta`, spelled the way claude
    /// writes it, so the test fails when the path in `classify` drifts from the wire format.
    fn delta_line(delta_type: &str, extra_fields: &str) -> String {
        format!(
            r#"{{"type":"stream_event","event":{{"type":"content_block_delta","index":0,"delta":{{"type":"{delta_type}"{extra_fields}}}}}}}"#
        )
    }

    /// One argv row. The session id and the prompt are generated from the task, so the
    /// expected vector carries `{id}` and `{prompt}` and those two elements are checked
    /// against the generated values instead of a literal.
    struct Row {
        name: &'static str,
        instructions: &'static str,
        source_text: &'static str,
        model: Option<&'static str>,
        session: Option<&'static str>,
        expected: &'static [&'static str],
    }

    fn build_row(row: &Row) -> Invocation {
        let task = Task {
            instructions: row.instructions,
            source_text: row.source_text,
            model: row.model,
            session: row.session,
        };
        harness()
            .build(&task)
            .unwrap_or_else(|err| panic!("{}: build failed: {err}", row.name))
    }

    fn assert_row(row: &Row) {
        let invocation = build_row(row);
        assert_eq!(invocation.program, "claude", "{}: program", row.name);
        assert_eq!(
            invocation.stdin, None,
            "{}: the prompt is an argument, not stdin",
            row.name
        );

        let session = invocation
            .args
            .iter()
            .position(|arg| arg == "--session-id" || arg == "--resume")
            .map(|at| invocation.args[at + 1].clone())
            .unwrap_or_else(|| panic!("{}: no session flag", row.name));
        let expected_prompt = prompt(&Task {
            instructions: row.instructions,
            source_text: row.source_text,
            model: row.model,
            session: None,
        });
        let expected: Vec<String> = row
            .expected
            .iter()
            .map(|arg| match *arg {
                "{id}" => session.clone(),
                "{prompt}" => expected_prompt.clone(),
                other => other.to_owned(),
            })
            .collect();

        assert_eq!(
            invocation.args,
            expected,
            "{}: argv differs\n  actual: {}\n  want:   {}",
            row.name,
            invocation.args.join(" | "),
            row.expected.join(" | ")
        );

        // A generated id must look like the v4 UUID claude accepts, or the row above could
        // be satisfied by a string only this test agrees with.
        if row.session.is_none() {
            assert_uuid_v4(&session);
        }
    }

    #[test]
    fn argv_is_spec_6_2_for_the_first_call_without_a_model() {
        assert_row(&Row {
            name: "first call, no --model: --session-id",
            instructions: "be formal",
            source_text: "hello",
            model: None,
            session: None,
            expected: &[
                "-p",
                "--session-id",
                "{id}",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--system-prompt",
                "be formal",
                "--tools",
                "",
                "--permission-prompts",
                "none",
                "--",
                "{prompt}",
            ],
        });
    }

    #[test]
    fn argv_is_spec_6_2_for_a_continuation_without_a_model() {
        // Identical to the first-call row except the session flag, which is the whole point
        // of spec §6.0: instructions travel in the user turn, so a continuation needs no
        // other flag change.
        assert_row(&Row {
            name: "continuation, no --model: --resume",
            instructions: "be formal",
            source_text: "hello",
            model: None,
            session: Some("11111111-2222-4333-8444-555555555555"),
            expected: &[
                "-p",
                "--resume",
                "{id}",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--system-prompt",
                "be formal",
                "--tools",
                "",
                "--permission-prompts",
                "none",
                "--",
                "{prompt}",
            ],
        });
    }

    #[test]
    fn model_sits_after_the_session_flag_and_before_the_streaming_flags_by_choice() {
        // `--model` is not in the measured spec §6.2 argv, so its position is a readability
        // choice: directly after the session flag, keeping the measured block contiguous and
        // in spec order. Named in the test so a reader never has to guess whether the
        // placement was measured.
        assert_row(&Row {
            name: "first call with --model, placed by choice after the session flag",
            instructions: "be formal",
            source_text: "hello",
            model: Some("opus-4-5"),
            session: None,
            expected: &[
                "-p",
                "--session-id",
                "{id}",
                "--model",
                "opus-4-5",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--system-prompt",
                "be formal",
                "--tools",
                "",
                "--permission-prompts",
                "none",
                "--",
                "{prompt}",
            ],
        });
    }

    #[test]
    fn continuation_with_a_model_keeps_the_same_placement() {
        assert_row(&Row {
            name: "continuation with --model",
            instructions: "be formal",
            source_text: "hello",
            model: Some("opus-4-5"),
            session: Some("11111111-2222-4333-8444-555555555555"),
            expected: &[
                "-p",
                "--resume",
                "{id}",
                "--model",
                "opus-4-5",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--system-prompt",
                "be formal",
                "--tools",
                "",
                "--permission-prompts",
                "none",
                "--",
                "{prompt}",
            ],
        });
    }

    #[test]
    fn a_prompt_with_newlines_and_a_leading_dash_stays_one_argv_element() {
        // The regression this ticket exists for: joining argv into a string and letting a
        // shell-equivalent split it again would turn these lines into flags. The prompt must
        // remain the last element, whole.
        let instructions = "- drop the second person\n- use the active voice";
        let source_text = "--not-a-flag\nsecond line\n\nfourth line";
        let invocation = harness()
            .build(&task(instructions, source_text, None))
            .unwrap();

        let last = invocation.args.last().expect("prompt argument");
        assert_eq!(*last, prompt(&task(instructions, source_text, None)));
        assert!(last.starts_with("INSTRUCTIONS:\n- drop the second person"));
        assert!(last.contains("--not-a-flag\nsecond line\n\nfourth line"));

        // One element regardless of line count: a joined string would have grown with the
        // prompt's newlines once re-split.
        let short = harness().build(&task("", "", None)).unwrap();
        assert_eq!(invocation.args.len(), short.args.len());
    }

    #[test]
    fn instructions_reach_the_system_prompt_flag_verbatim_and_the_user_turn_too() {
        // Spec §6.0: the flag sets the register, the user turn carries correctness, so both
        // must be present and unmodified.
        let instructions = "instructions with \"quotes\" and\na newline";
        let invocation = harness().build(&task(instructions, "src", None)).unwrap();

        let at = invocation
            .args
            .iter()
            .position(|arg| arg == "--system-prompt")
            .expect("--system-prompt present");
        assert_eq!(invocation.args[at + 1], instructions);
        assert!(invocation.args.last().unwrap().contains(instructions));
    }

    #[test]
    fn tools_are_denied_by_an_empty_value_not_by_an_absent_flag() {
        // `--tools ""` means no tools; omitting the flag means claude's default set, so the
        // empty element is load-bearing (spec §6.2).
        let invocation = harness().build(&task("i", "s", None)).unwrap();
        let at = invocation
            .args
            .iter()
            .position(|arg| arg == "--tools")
            .expect("--tools present");

        assert_eq!(invocation.args[at + 1], "");
    }

    #[test]
    fn the_separator_sits_immediately_before_the_prompt_and_appears_once() {
        let invocation = harness().build(&task("i", "s", None)).unwrap();
        let len = invocation.args.len();

        assert_eq!(invocation.args[len - 2], "--");
        assert!(invocation.args[len - 1].starts_with("INSTRUCTIONS:"));
        // An extra separator would silently become an argument.
        assert_eq!(invocation.args.iter().filter(|arg| *arg == "--").count(), 1);
    }

    #[test]
    fn a_streamed_text_delta_is_text() {
        assert_eq!(
            harness().classify(&delta_line("text_delta", r#","text":"hello""#)),
            Event::Text
        );
    }

    #[test]
    fn a_text_delta_without_a_string_text_field_is_not_prose() {
        // Prose needs prose. Labelling an empty delta as Text would let the debug pane claim
        // the model is writing when nothing arrived.
        for extra in ["", r#","text":null"#, r#","text":42"#, r#","text":{}"#] {
            assert_eq!(
                harness().classify(&delta_line("text_delta", extra)),
                Event::Junk,
                "text_delta with {extra:?} must not be prose"
            );
        }
    }

    #[test]
    fn reasoning_and_its_signature_are_think_and_never_text() {
        let harness = harness();
        assert_eq!(
            harness.classify(&delta_line("thinking_delta", r#","thinking":"hmm""#)),
            Event::Think
        );
        // A signature carries no prose but belongs to the same thinking block, so it is
        // grouped with reasoning rather than given a kind nothing consumes.
        assert_eq!(
            harness.classify(&delta_line("signature_delta", r#","signature":"abc""#)),
            Event::Think
        );
    }

    #[test]
    fn every_other_delta_kind_is_junk() {
        for delta_type in ["input_json_delta", "citations_delta", "tool_use_delta", ""] {
            assert_eq!(
                harness().classify(&delta_line(delta_type, r#","partial_json":"{}""#)),
                Event::Junk,
                "delta type {delta_type:?} must not be prose"
            );
        }
    }

    #[test]
    fn a_tool_use_event_is_junk_rather_than_prose() {
        // Measured (spec §6.1a step 6): with tools denied, a tool call can still appear, and
        // nothing executed — showing it as streamed prose would misreport what happened.
        for line in [
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t","name":"Write","input":{}}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"t","name":"WebSearch"}}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t","name":"Write"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_result","tool_use_id":"t"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[],"tool_calls":[{"id":"t"}]}}"#,
        ] {
            assert_eq!(
                harness().classify(line),
                Event::Junk,
                "tool call labelled as prose: {line}"
            );
        }
    }

    #[test]
    fn an_empty_tool_list_is_not_a_tool_call() {
        // `"tool_calls": []` is how claude reports "no tools used"; treating the field
        // itself as the signal would label ordinary turns as tool calls.
        let line = r#"{"type":"system","subtype":"init","tool_calls":[],"tools":[]}"#;
        assert!(!carries_tool_use(&parse(line).unwrap()));
    }

    #[test]
    fn an_error_object_is_never_prose() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"x"}},"is_error":true}"#;
        assert_eq!(harness().classify(line), Event::Junk);
    }

    #[test]
    fn non_json_and_unrecognised_lines_are_junk_not_errors() {
        let harness = harness();
        for line in [
            "",
            "not json at all",
            "{",
            "[]",
            "42",
            "\"just a string\"",
            "{} trailing",
            r#"{"type":"rate_limit_event"}"#,
            r#"{"type":"system","subtype":"init"}"#,
            r#"{"type":"stream_event"}"#,
            r#"{"type":"stream_event","event":{"type":"message_start"}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta"}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":"nope"}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"a"}},"#,
        ] {
            assert_eq!(harness.classify(line), Event::Junk, "line {line:?}");
        }
    }

    #[test]
    fn the_result_event_is_not_labelled_prose() {
        // The output pane gets this text through `final_text`; calling it Text would make the
        // debug pane's one prose label mean two different things.
        let line = r#"{"type":"result","subtype":"success","result":"done"}"#;
        assert_eq!(harness().classify(line), Event::Junk);
    }

    #[test]
    fn final_text_comes_from_the_result_field_of_the_result_event() {
        let harness = harness();
        assert_eq!(
            harness.final_text(r#"{"type":"result","subtype":"success","result":"the rewrite"}"#),
            Some("the rewrite".to_owned())
        );

        // Newlines inside the answer are the answer: not stripped, not re-wrapped.
        assert_eq!(
            harness.final_text(r#"{"type":"result","result":"line one\n\nline two"}"#),
            Some("line one\n\nline two".to_owned())
        );

        // Escapes are decoded, so the output pane shows prose rather than JSON source.
        assert_eq!(
            harness.final_text(r#"{"type":"result","result":"a\tb \u00e9 \""}"#),
            Some("a\tb é \"".to_owned())
        );
    }

    #[test]
    fn final_text_is_none_for_every_other_line() {
        let harness = harness();
        for line in [
            "",
            "plain text",
            r#"{"type":"assistant","result":"not the terminal event"}"#,
            r#"{"type":"stream_event","result":"nested, not top level"}"#,
            r#"{"type":"result"}"#,
            r#"{"type":"result","result":null}"#,
            r#"{"type":"result","result":42}"#,
        ] {
            assert_eq!(harness.final_text(line), None, "line {line:?}");
        }

        // A streamed delta is display-only and must never stand in for the terminal event.
        assert_eq!(
            harness.final_text(&delta_line("text_delta", r#","text":"answer""#)),
            None
        );
    }

    #[test]
    fn an_error_result_yields_no_final_text() {
        // Its `result` field holds a diagnostic message, and presenting a message as the
        // rewrite is the failure the markers exist to prevent.
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"API error"}"#;
        assert_eq!(harness().final_text(line), None);
    }

    #[test]
    fn name_is_the_binary_name() {
        assert_eq!(harness().name(), "claude");
    }

    #[test]
    fn new_session_id_is_a_canonical_lowercase_uuid_v4() {
        let id = harness().new_session_id();

        assert_uuid_v4(&id);
        assert_eq!(id, id.to_lowercase(), "not lowercase: {id}");
    }

    #[test]
    fn consecutive_session_ids_differ() {
        // Uniqueness, not cryptographic quality: claude only parses the shape, but a
        // repeated id would silently continue a session that is not this one.
        let harness = harness();
        let first = harness.new_session_id();
        let second = harness.new_session_id();

        assert_ne!(first, second);
        assert_uuid_v4(&second);
    }

    #[test]
    fn a_first_call_generates_its_own_id_and_a_continuation_reuses_the_supplied_one() {
        // The id a first call creates is reported by its argv, which is where the caller
        // persists it (spec §5 step 3). Nothing is cached, so a second first call is a
        // second session rather than a silently resumed one.
        let harness = harness();
        let first = harness.build(&task("i", "s", None)).unwrap();
        let second = harness.build(&task("i", "s", None)).unwrap();

        assert_eq!(first.args[1], "--session-id");
        assert_eq!(second.args[1], "--session-id");
        assert_ne!(first.args[2], second.args[2]);
        assert_uuid_v4(&first.args[2]);
    }

    #[test]
    fn the_json_reader_rejects_malformed_input() {
        for line in [
            "",
            "{",
            "}",
            "[",
            "]",
            "{\"a\"}",
            "{\"a\":}",
            "{\"a\":1,}",
            "[1,]",
            "{'a':1}",
            "nope",
            "tru",
            "01x",
            "\"unterminated",
            "{\"a\":\"\\q\"}",
            "{\"a\":1} {}",
            "{\"a\":\"\\u12\"}",
        ] {
            assert_eq!(parse(line), None, "parsed {line:?}");
        }
    }

    #[test]
    fn the_json_reader_handles_the_shapes_claude_emits() {
        assert_eq!(
            parse("   {\"a\": [1, -2.5e3, true, false, null], \"b\": {\"c\": \"d\"}}  "),
            Some(Json::Object(vec![
                (
                    "a".to_owned(),
                    Json::Array(vec![
                        Json::Number,
                        Json::Number,
                        Json::Bool(true),
                        Json::Bool(false),
                        Json::Null,
                    ])
                ),
                (
                    "b".to_owned(),
                    Json::Object(vec![("c".to_owned(), Json::Str("d".to_owned()))])
                ),
            ]))
        );
    }

    #[test]
    fn the_json_reader_decodes_escapes_including_a_surrogate_pair() {
        assert_eq!(
            parse(r#""tab\there\n\"quoted\"\\slash\/ \u0041 \ud83d\ude00""#),
            Some(Json::Str("tab\there\n\"quoted\"\\slash/ A 😀".to_owned()))
        );
    }

    #[test]
    fn field_returns_the_first_occurrence_and_none_for_a_non_object() {
        let value = parse(r#"{"a":1,"a":2}"#).unwrap();
        assert_eq!(field(&value, "a"), Some(&Json::Number));
        assert_eq!(field(&value, "b"), None);
        assert_eq!(field(&Json::Number, "a"), None);
    }

    /// Asserts the canonical `8-4-4-4-12` lowercase hex form with a v4 version nibble and an
    /// RFC 4122 variant nibble, which is what claude validates.
    fn assert_uuid_v4(id: &str) {
        let groups: Vec<&str> = id.split('-').collect();
        assert_eq!(
            groups.iter().map(|group| group.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12],
            "not 8-4-4-4-12: {id:?}"
        );

        let bytes = id.as_bytes();
        let dashes = [8, 13, 18, 23];
        for (at, byte) in bytes.iter().enumerate() {
            if dashes.contains(&at) {
                assert_eq!(*byte, b'-', "wrong separator position: {id:?}");
            } else {
                assert!(
                    byte.is_ascii_digit() || (b'a'..=b'f').contains(byte),
                    "not lowercase hex at {at}: {id:?}"
                );
            }
        }
        assert_eq!(bytes[14], b'4', "version nibble: {id:?}");
        assert!(
            matches!(bytes[19], b'8' | b'9' | b'a' | b'b'),
            "variant nibble: {id:?}"
        );
    }
}
