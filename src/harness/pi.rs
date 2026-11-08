//! The `pi` adapter (spec §6.3).
//!
//! Shares the prompt template and the marker protocol with [`super::claude`], and differs in
//! exactly the four places §6.4 predicts: the session flag, the id format, the tool-denial
//! flags, and the event vocabulary. That last one is why `classify` and `final_text` exist on
//! the trait at all.
//!
//! Two things here differ from what §6.3's prose suggests, both measured against pi 1.5 on
//! this machine:
//!
//! * A `text_delta` carries its payload in `assistantMessageEvent.delta` as a **plain string**.
//!   claude nests it one level deeper as `delta.text`, so the two adapters cannot share a
//!   JSON path even though the concept is the same.
//! * The terminal event is a `message_end` whose `message.content` is an **array of blocks**,
//!   not a `result` string. The text block is the answer; a thinking block sits beside it and
//!   must never be picked up.
//!
//! The JSON reader is the one in [`super::claude`]: pi's lines are the same shape of thing,
//! and a second reader would be a second place for a parsing bug to live.

use super::claude::parse as parse_json;
use super::{BuildError, Event, HarnessAdapter, Invocation, Task, prompt};

/// The pi harness as an adapter.
///
/// Holds no state, for the same reason [`super::claude::Claude`] does not: the session id is
/// generated where it is used and reported through the argv it appears in.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pi;

impl HarnessAdapter for Pi {
    fn name(&self) -> &str {
        "pi"
    }

    /// `psc-<uuid>`: pi's ids accept `[A-Za-z0-9._-]` and must start and end with a letter or
    /// digit, so the prefix supplies the leading letter and the uuid's last hex digit is the
    /// trailing one. No trailing hyphen, which is why the prefix is joined with `-` rather
    /// than appended (spec §6.3).
    fn new_session_id(&self) -> String {
        format!("psc-{}", super::claude::new_uuid_v4())
    }

    /// Builds the argv from spec §6.3.
    ///
    /// First call and continuation produce the **same** argv: pi's `--session-id` means "open
    /// the session with this id, or create it", so there is no `--resume` equivalent and no
    /// branch on `task.session`. That is the sharpest difference from claude and the reason
    /// the flag cannot be shared between the adapters.
    fn build(&self, task: &Task<'_>) -> Result<Invocation, BuildError> {
        // A first call has no id yet, so one is generated and reported through the argv, which
        // is where the caller reads it back (spec §6.3).
        let session = task
            .session
            .map_or_else(|| self.new_session_id(), str::to_owned);

        let mut args = Vec::with_capacity(18);
        // `--print` is pi's one-shot form; the app depends on pi neither opening a window nor
        // waiting for input.
        args.push("--print".to_owned());
        args.push("--session-id".to_owned());
        args.push(session);

        // `--model` is a pass-through from the prosecraft command line, placed directly after
        // the session flag to keep the measured block below contiguous and in spec order. The
        // position is a readability choice, not a measurement, exactly as in the claude
        // adapter.
        if let Some(model) = task.model {
            args.push("--model".to_owned());
            args.push(model.to_owned());
        }

        // `--mode json` is the streaming form, and unlike claude pi needs no extra flag to
        // make it stream (spec §6.3).
        args.push("--mode".to_owned());
        args.push("json".to_owned());

        // Replaces pi's agent prompt. With every tool, skill, extension and context file
        // disabled that prompt has nothing left to do, so replacing it is correct.
        args.push("--system-prompt".to_owned());
        args.push(task.instructions.to_owned());

        // The denial block. `--no-context-files` is load-bearing and is the one flag here
        // whose absence is invisible: measured, without it pi injects the user's own
        // AGENTS.md (2759 bytes) into the system message, so a rewrite would silently inherit
        // their coding guidelines. The rest keep tools, MCP servers, skills and templates out
        // of the request for the same reason: none of them are part of rewriting a paragraph.
        for flag in [
            "--no-tools",
            "--no-context-files",
            "--no-extensions",
            "--no-mcp",
            "--no-skills",
            "--no-prompt-templates",
            // Nothing here needs the network beyond the provider call itself.
            "--offline",
        ] {
            args.push(flag.to_owned());
        }

        // The separator, then the prompt as exactly one argument, so newlines and a leading
        // `-` survive without quoting (spec §6).
        args.push("--".to_owned());
        args.push(prompt(task));

        Ok(Invocation {
            program: self.name().to_owned(),
            args,
            // pi takes the prompt as an argument, so nothing is written to stdin.
            stdin: None,
        })
    }

    /// Labels one stdout line for the debug pane.
    ///
    /// The delta path is `message_update` → `assistantMessageEvent`, and `delta` is a plain
    /// string there, not an object. `Text` is reachable only from a `text_delta` that
    /// actually carries a string, so reasoning can never be mistaken for streamed prose.
    fn classify(&self, line: &str) -> Event {
        let Some(json) = parse_json(line) else {
            return Event::Junk;
        };
        if super::claude::carries_tool_use(&json) {
            return Event::Junk;
        }
        match super::claude::field(&json, "type").and_then(|v| v.as_str()) {
            Some("message_update") => {
                let Some(event) = super::claude::field(&json, "assistantMessageEvent") else {
                    return Event::Junk;
                };
                match super::claude::field(event, "type").and_then(|v| v.as_str()) {
                    Some("text_delta") if matches!(super::claude::field(event, "delta"), Some(v) if v.as_str().is_some()) => {
                        Event::Text
                    }
                    // Reasoning, and the block boundaries that surround it, belong to the
                    // debug pane and nothing else.
                    Some("thinking_delta" | "thinking_start" | "thinking_end") => Event::Think,
                    _ => Event::Junk,
                }
            }
            // The terminal event's text reaches the output pane through `final_text`; calling
            // it prose here would make the pane's one prose label mean two things.
            Some("message_end") => Event::Junk,
            _ => Event::Junk,
        }
    }

    /// The answer from the final `message_end` whose role is `assistant`.
    ///
    /// The content is an array of blocks and the answer is the `text` block; a thinking block
    /// sits beside it in the same array and must not be concatenated in. Nothing is ever
    /// rebuilt from deltas (spec §6.1a).
    fn final_text(&self, line: &str) -> Option<String> {
        let json = parse_json(line)?;
        if super::claude::field(&json, "type").and_then(|v| v.as_str()) != Some("message_end") {
            return None;
        }
        let message = super::claude::field(&json, "message")?;
        if super::claude::field(message, "role").and_then(|v| v.as_str()) != Some("assistant") {
            return None;
        }
        // Every `text` block, joined: a multi-block answer is still one answer, and dropping
        // all but the first would truncate it.
        let content = super::claude::field(message, "content")?;
        let blocks = content.as_array()?;
        let mut text = String::new();
        let mut found = false;
        for block in blocks {
            if super::claude::field(block, "type").and_then(|v| v.as_str()) != Some("text") {
                continue;
            }
            if let Some(part) = super::claude::field(block, "text").and_then(|v| v.as_str()) {
                text.push_str(part);
                found = true;
            }
        }
        found.then_some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::HarnessAdapter;

    fn harness() -> Pi {
        Pi
    }

    fn task<'a>(instructions: &'a str, source_text: &'a str, model: Option<&'a str>) -> Task<'a> {
        Task {
            instructions,
            source_text,
            model,
            session: None,
        }
    }

    /// One argv row, mirroring the shape of the claude table so the two cannot drift apart
    /// without one of the tables failing.
    struct Row {
        name: &'static str,
        model: Option<&'static str>,
        session: Option<&'static str>,
        expected: &'static [&'static str],
    }

    fn assert_row(row: &Row) {
        let task = Task {
            instructions: "be formal",
            source_text: "hello",
            model: row.model,
            session: row.session,
        };
        let invocation = harness()
            .build(&task)
            .unwrap_or_else(|err| panic!("{}: build failed: {err}", row.name));

        assert_eq!(invocation.program, "pi", "{}: program", row.name);
        assert_eq!(
            invocation.stdin, None,
            "{}: the prompt is an argument, not stdin",
            row.name
        );

        // The session id is generated on a first call, so the expected vector carries `{id}`
        // and `{prompt}` and those two elements are checked against the generated values.
        let session = invocation.args[2].clone();
        let expected_prompt = prompt(&Task {
            instructions: "be formal",
            source_text: "hello",
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
    }

    #[test]
    fn argv_is_spec_6_3_without_a_model() {
        assert_row(&Row {
            name: "first call, no --model",
            model: None,
            session: None,
            expected: &[
                "--print",
                "--session-id",
                "{id}",
                "--mode",
                "json",
                "--system-prompt",
                "be formal",
                "--no-tools",
                "--no-context-files",
                "--no-extensions",
                "--no-mcp",
                "--no-skills",
                "--no-prompt-templates",
                "--offline",
                "--",
                "{prompt}",
            ],
        });
    }

    #[test]
    fn argv_with_a_model_matches_the_same_shape() {
        assert_row(&Row {
            name: "first call with --model",
            model: Some("opus-4-5"),
            session: None,
            expected: &[
                "--print",
                "--session-id",
                "{id}",
                "--model",
                "opus-4-5",
                "--mode",
                "json",
                "--system-prompt",
                "be formal",
                "--no-tools",
                "--no-context-files",
                "--no-extensions",
                "--no-mcp",
                "--no-skills",
                "--no-prompt-templates",
                "--offline",
                "--",
                "{prompt}",
            ],
        });
    }

    #[test]
    fn a_continuation_produces_the_identical_argv_apart_from_the_id() {
        // Spec §6.3: pi's `--session-id` means "open it or create it", so there is no branch
        // on `task.session` and no `--resume`. This is the sharpest difference from claude.
        let id = "psc-11111111-2222-4333-8444-555555555555";
        let first = harness()
            .build(&Task {
                instructions: "be formal",
                source_text: "hello",
                model: None,
                session: None,
            })
            .unwrap();
        let next = harness()
            .build(&Task {
                instructions: "be formal",
                source_text: "hello",
                model: None,
                session: Some(id),
            })
            .unwrap();

        // Same flag, supplied id.
        assert_eq!(next.args[1], "--session-id");
        assert_eq!(next.args[2], id);
        // And nothing else differs, including the length: a resume flag would show up here.
        assert_eq!(
            first.args.len(),
            next.args.len(),
            "the continuation argv has a different shape: {:?} vs {:?}",
            first.args,
            next.args
        );
        assert_eq!(first.args[1], next.args[1]);
        assert_eq!(first.args[3..], next.args[3..]);
        assert!(
            !next.args.iter().any(|arg| arg == "--resume"),
            "pi has no --resume: {:?}",
            next.args
        );
    }

    #[test]
    fn context_files_are_disabled_because_without_it_the_users_agents_md_leaks_in() {
        // Measured: without `--no-context-files` the system message carries `project_context`,
        // 2759 bytes of the user's own AGENTS.md, so a rewrite would silently inherit their
        // coding guidelines. This is the one flag whose absence is invisible.
        for model in [None, Some("opus-4-5")] {
            let invocation = harness().build(&task("i", "s", model)).unwrap();
            assert!(
                invocation
                    .args
                    .iter()
                    .any(|arg| arg == "--no-context-files"),
                "the context files were not disabled: {:?}",
                invocation.args
            );
        }
    }

    #[test]
    fn every_tool_and_extension_is_disabled() {
        let invocation = harness().build(&task("i", "s", None)).unwrap();

        for flag in [
            "--no-tools",
            "--no-extensions",
            "--no-mcp",
            "--no-skills",
            "--no-prompt-templates",
            "--offline",
        ] {
            assert!(
                invocation.args.iter().any(|arg| arg == flag),
                "{flag} is missing: {:?}",
                invocation.args
            );
        }
    }

    #[test]
    fn the_separator_sits_immediately_before_the_prompt_and_appears_once() {
        let invocation = harness().build(&task("i", "s", None)).unwrap();
        let len = invocation.args.len();

        assert_eq!(invocation.args[len - 2], "--");
        assert!(invocation.args[len - 1].starts_with("INSTRUCTIONS:"));
        assert_eq!(invocation.args.iter().filter(|arg| *arg == "--").count(), 1);
    }

    #[test]
    fn a_prompt_with_newlines_and_a_leading_dash_stays_one_argv_element() {
        let instructions = "- drop the second person";
        let source = "--not-a-flag\nsecond line";
        let invocation = harness().build(&task(instructions, source, None)).unwrap();

        let last = invocation.args.last().expect("prompt argument");
        assert!(last.starts_with("INSTRUCTIONS:\n- drop the second person"));
        assert!(last.contains("--not-a-flag\nsecond line"));
        // One element regardless of the newlines inside it.
        let short = harness().build(&task("", "", None)).unwrap();
        assert_eq!(invocation.args.len(), short.args.len());
    }

    #[test]
    fn a_session_id_is_a_psc_prefixed_uuid_in_pis_alphabet() {
        let id = harness().new_session_id();

        assert!(id.starts_with("psc-"), "no psc- prefix: {id}");
        // pi's rule: `[A-Za-z0-9._-]`, starting and ending with a letter or digit.
        for c in id.chars() {
            assert!(
                c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'),
                "character {c:?} is outside pi's alphabet: {id}"
            );
        }
        let first = id.chars().next().expect("empty id");
        let last = id.chars().last().expect("empty id");
        for (edge, c) in [("first", first), ("last", last)] {
            assert!(
                c.is_ascii_alphanumeric(),
                "the {edge} character is not a letter or digit: {id}"
            );
        }
        // And the tail really is a v4 uuid, so `--session-id` parses.
        let uuid = id.trim_start_matches("psc-");
        let groups: Vec<&str> = uuid.split('-').collect();
        assert_eq!(
            groups.iter().map(|g| g.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12],
            "the tail is not a uuid: {id}"
        );
        assert_eq!(uuid.as_bytes()[14], b'4', "not a v4 uuid: {id}");
    }

    #[test]
    fn consecutive_session_ids_differ() {
        let harness = harness();
        assert_ne!(harness.new_session_id(), harness.new_session_id());
    }

    #[test]
    fn name_is_the_binary_name() {
        assert_eq!(harness().name(), "pi");
    }

    /// A real pi event line, spelled the way pi 1.5 writes it.
    fn update(event_type: &str, delta: &str) -> String {
        format!(
            r#"{{"type":"message_update","assistantMessageEvent":{{"type":"{event_type}","delta":"{delta}"}}}}"#
        )
    }

    #[test]
    fn a_text_delta_carrying_a_string_is_text() {
        // Measured: pi puts the payload in `assistantMessageEvent.delta` as a plain string,
        // one level shallower than claude's `delta.text`.
        assert_eq!(
            harness().classify(&update("text_delta", "hello")),
            Event::Text
        );
    }

    #[test]
    fn a_text_delta_without_a_string_payload_is_not_prose() {
        for line in [
            r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta"}}"#,
            r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":null}}"#,
            r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":42}}"#,
            r#"{"type":"message_update"}"#,
        ] {
            assert_eq!(harness().classify(line), Event::Junk, "line {line:?}");
        }
    }

    #[test]
    fn reasoning_and_its_block_boundaries_are_think() {
        let harness = harness();
        for event_type in ["thinking_delta", "thinking_start", "thinking_end"] {
            assert_eq!(
                harness.classify(&update(event_type, "hmm")),
                Event::Think,
                "{event_type} should be reasoning"
            );
        }
    }

    #[test]
    fn every_other_event_type_is_junk() {
        let harness = harness();
        for line in [
            "",
            "not json",
            r#"{"type":"session","id":"psc-x"}"#,
            r#"{"type":"agent_start"}"#,
            r#"{"type":"turn_start"}"#,
            r#"{"type":"message_start"}"#,
            r#"{"type":"turn_end"}"#,
            r#"{"type":"agent_end"}"#,
            r#"{"type":"agent_settled"}"#,
            &update("text_start", ""),
            &update("tool_use", ""),
        ] {
            assert_eq!(harness.classify(line), Event::Junk, "line {line:?}");
        }
    }

    #[test]
    fn a_tool_call_is_junk_rather_than_prose() {
        let line = r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"x"},"message":{"role":"assistant","content":[{"type":"tool_use","id":"t"}]}}"#;
        assert_eq!(harness().classify(line), Event::Junk);
    }

    #[test]
    fn the_terminal_event_is_the_assistant_message_end_text_block() {
        // Measured shape: `message.content` is an array of blocks and the answer is the `text`
        // one. A thinking block sits beside it in the same array.
        let line = r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"deliberating"},{"type":"text","text":"<<<TEXT>>>the rewrite<<<END>>>"}]}}"#;

        assert_eq!(
            harness().final_text(line),
            Some("<<<TEXT>>>the rewrite<<<END>>>".to_owned())
        );
    }

    #[test]
    fn the_thinking_block_is_never_part_of_the_answer() {
        let line = r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"SECRET"},{"type":"text","text":"visible"}]}}"#;

        let text = harness().final_text(line).expect("no text");
        assert_eq!(text, "visible");
        assert!(!text.contains("SECRET"), "reasoning leaked: {text}");
    }

    #[test]
    fn multiple_text_blocks_are_joined() {
        let line = r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"one "},{"type":"text","text":"two"}]}}"#;

        assert_eq!(
            harness().final_text(line),
            Some("one two".to_owned()),
            "a multi-block answer was truncated"
        );
    }

    #[test]
    fn only_an_assistant_message_end_is_terminal() {
        let harness = harness();
        for line in [
            // The system and user messages also end; only the assistant one is the answer.
            r#"{"type":"message_end","message":{"role":"system","content":[{"type":"text","text":"preamble"}]}}"#,
            r#"{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"the prompt"}]}}"#,
            r#"{"type":"message_end","message":{"content":[{"type":"text","text":"no role"}]}}"#,
            r#"{"type":"message_end"}"#,
            r#"{"type":"message_end","message":{"role":"assistant","content":[]}}"#,
            r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"thinking","thinking":"only reasoning"}]}}"#,
            "",
            "not json",
        ] {
            assert_eq!(harness.final_text(line), None, "line {line:?}");
        }
    }

    #[test]
    fn a_delta_line_is_never_the_final_text() {
        assert_eq!(harness().final_text(&update("text_delta", "x")), None);
    }

    #[test]
    fn the_two_adapters_agree_on_the_prompt_but_not_on_their_argv() {
        // Spec §6.4 in one assertion: the prompt template is shared, everything adapter
        // specific is not. A change that made one adapter send a different prompt would break
        // here rather than in a live run.
        let task = task("be formal", "hello", None);
        let claude = crate::harness::claude::Claude.build(&task).unwrap();
        let pi = harness().build(&task).unwrap();

        assert_eq!(
            claude.args.last(),
            pi.args.last(),
            "the prompt template differs between adapters"
        );
        // A *first* call names the session the same way in both; the divergence is on a
        // continuation, where claude switches to `--resume` and pi keeps `--session-id`
        // because there it means "open this session or create it".
        assert_eq!(
            claude.args[1], pi.args[1],
            "both adapters create a session with --session-id"
        );

        let resumed = Task {
            instructions: "be formal",
            source_text: "hello",
            model: None,
            session: Some("id-in-use"),
        };
        let claude = crate::harness::claude::Claude.build(&resumed).unwrap();
        let pi = harness().build(&resumed).unwrap();
        assert_eq!(claude.args[1], "--resume", "claude should resume");
        assert_eq!(
            pi.args[1], "--session-id",
            "pi keeps --session-id: it means open-or-create"
        );
    }
}
