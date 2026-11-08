//! Extracting the rewritten text from the harness's terminal event (spec §6.1a).
//!
//! The instructions pane is free text, so the model may be asked for commentary as well as a
//! rewrite; measured, that produces an explanation plus `**Improved text:**` in the same
//! reply, and no reliable boundary exists in that prose. Asking for `<<<TEXT>>>`/`<<<END>>>`
//! turns extraction from a *guess* into a *protocol*, which is the whole reason these markers
//! exist.
//!
//! Which makes the failure rule the important part: **a missing marker is an error, never a
//! guess.** Markers were honoured in every measured case, including when the prompt also
//! asked for commentary, so their absence means truncation, a refusal, or a confused model.
//! Silently presenting whatever text arrived as the rewrite is the exact failure this design
//! exists to prevent, and the debug pane already holds the raw output, so diagnosis costs one
//! keypress rather than a heuristic here.
//!
//! Everything is a plain substring search on purpose. Measured, pi returns
//! `<<<TEXT>>>text<<<END>>>` with no newlines at all, so a rule that required the markers to
//! sit on their own lines would reject a perfectly good answer.

/// The marker that opens the rewritten text, verbatim from spec §6.1.
pub const START: &str = "<<<TEXT>>>";

/// The marker that closes it.
pub const END: &str = "<<<END>>>";

/// The substring that opens a tool invocation block, which is never a rewrite (spec §6.1a
/// step 6). Measured cause: with tools denied the model prints the call as text and still
/// exits 0, so the exit status cannot be the signal.
const TOOL_INVOKE: &str = "<invoke name=";

/// What extracting one terminal event produced.
///
/// The variants are separate rather than an error string because the caller renders them
/// differently: a missing marker is the model going off-protocol, while a tool call is the
/// model trying to act. Both are errors, but the user needs to read which one happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extraction {
    /// The text between the markers, trimmed at its edges with internal lines intact.
    Rewrite(String),
    /// No `<<<TEXT>>>`: the model answered outside the protocol.
    MissingStart,
    /// A start marker with no `<<<END>>>`: the answer was cut off, or the model forgot to
    /// close it. Distinguished from [`Extraction::MissingStart`] because a truncated answer
    /// and a refused one read very differently.
    MissingEnd,
    /// Both markers present, but the text inside is a tool call rather than prose.
    ToolCall(String),
}

impl Extraction {
    /// The line to put on the status line, in the words spec §6.1a step 4 asks for.
    pub fn message(&self) -> Option<&'static str> {
        match self {
            Self::Rewrite(_) => None,
            Self::MissingStart | Self::MissingEnd => Some("model did not use the output markers"),
            Self::ToolCall(_) => {
                Some("model tried to call a tool; tools are denied, so that is not a rewrite")
            }
        }
    }
}

/// Extracts the rewrite from the terminal text of one run.
///
/// Applied once per run, to the whole terminal text rather than to the stream: the markers
/// delimit a single answer, and searching each delta would neither find a marker split across
/// two of them nor benefit from trying.
pub fn extract(text: &str) -> Extraction {
    // Step 1: plain substring search, start before end. `<<<END>>>` occurring before
    // `<<<TEXT>>>` is not a rewrite with content, it is markers in the wrong order, and the
    // search below reports it as a missing start rather than as an empty success.
    let Some(start) = text.find(START) else {
        return Extraction::MissingStart;
    };
    let body = &text[start + START.len()..];
    let Some(end) = body.find(END) else {
        return Extraction::MissingEnd;
    };

    // Steps 2 and 3: everything between, and nothing else. Whatever surrounded the markers —
    // a preamble, a leaked `<thinking>` tag, a closing remark — is discarded, which is the
    // entire point of the protocol. A second marker pair is ignored: the first complete pair
    // is the answer.
    //
    // Step 5: trimmed only at the edges, because measured, paragraphs inside the markers keep
    // their blank lines and those are part of the rewrite.
    let inner = body[..end].trim();

    // Step 6: a tool call is not a rewrite. Checked here, after trimming, so a call that the
    // model indented inside the markers is still caught.
    if inner.contains(TOOL_INVOKE) {
        return Extraction::ToolCall(inner.to_owned());
    }

    // Step 7: no markdown unwrapping and no fence stripping. A fence the model put *inside*
    // the markers is part of the text.
    Extraction::Rewrite(inner.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asserts a rewrite and returns it, so the happy-path rows read as one line.
    fn rewrite(text: &str) -> String {
        match extract(text) {
            Extraction::Rewrite(text) => text,
            other => panic!("expected a rewrite from {text:?}, got {other:?}"),
        }
    }

    #[test]
    fn markers_on_their_own_lines_as_claude_returns_them() {
        // Measured shape (spec §6.1a): the answer between two markers, each on its own line.
        let text = "<<<TEXT>>>\nThe rewritten paragraph.\n<<<END>>>";

        assert_eq!(rewrite(text), "The rewritten paragraph.");
    }

    #[test]
    fn the_single_line_form_as_pi_returns_it() {
        // Measured on pi: no newlines around the markers at all. A rule requiring them on
        // their own lines would reject this real answer.
        assert_eq!(rewrite("<<<TEXT>>>text<<<END>>>"), "text");
    }

    #[test]
    fn a_preamble_and_a_closing_remark_are_discarded() {
        // The measured case that motivates markers: instructions asking for an explanation
        // produce commentary around the answer.
        let text = "Here is what I changed and why:\n\n\
                    - removed the passive voice\n\n\
                    <<<TEXT>>>\nThe report was written by the team.\n<<<END>>>\n\n\
                    Let me know if you want it shorter.";

        assert_eq!(rewrite(text), "The report was written by the team.");
    }

    #[test]
    fn leaked_reasoning_tags_before_the_marker_are_discarded() {
        // Measured: claude emitted these before the start marker. They are outside it, so the
        // protocol removes them without knowing what they are.
        let text = "<thinking>deliberating</thinking>\n\
                    <reasoning_effort>high</reasoning_effort>\n\
                    <thinking_mode>enabled</thinking_mode>\n\
                    <<<TEXT>>>\nclean\n<<<END>>>";

        assert_eq!(rewrite(text), "clean");
    }

    #[test]
    fn internal_blank_lines_and_newlines_are_preserved_verbatim() {
        // Measured: paragraphs inside the markers keep their blank lines (spec §6.1a step 5).
        let text = "<<<TEXT>>>\nfirst paragraph\n\nsecond paragraph\n\n    indented\n<<<END>>>";

        assert_eq!(
            rewrite(text),
            "first paragraph\n\nsecond paragraph\n\n    indented"
        );
    }

    #[test]
    fn only_the_edges_are_trimmed() {
        assert_eq!(
            rewrite("<<<TEXT>>>\n\n   padded   \n\n<<<END>>>"),
            "padded",
            "leading and trailing whitespace should go, internal padding should not"
        );
        // Trailing spaces on an internal line survive: they are inside the text.
        assert_eq!(rewrite("<<<TEXT>>>a  \nb<<<END>>>"), "a  \nb");
    }

    #[test]
    fn a_fence_inside_the_markers_is_part_of_the_text() {
        // Step 7: the markers already isolate the answer, so markdown inside them is content.
        let text = "<<<TEXT>>>```rust\nfn main() {}\n```<<<END>>>";

        assert_eq!(rewrite(text), "```rust\nfn main() {}\n```");
    }

    #[test]
    fn bold_and_emphasis_inside_the_markers_survive() {
        assert_eq!(
            rewrite("<<<TEXT>>>**bold** and _italic_<<<END>>>"),
            "**bold** and _italic_"
        );
    }

    #[test]
    fn a_missing_start_marker_is_an_error_not_a_trimmed_guess() {
        // The strict rule (spec §6.1a step 4): never fall back to preamble-stripping.
        for text in [
            "just an answer with no markers",
            "**Improved text:**\n\nThe rewrite.\n",
            "<<<END>>>\ntext after a lone end marker",
            "",
        ] {
            let extraction = extract(text);
            assert!(
                matches!(extraction, Extraction::MissingStart),
                "{text:?} produced {extraction:?}"
            );
            assert_eq!(
                extraction.message(),
                Some("model did not use the output markers")
            );
            assert!(!matches!(extraction, Extraction::Rewrite(_)));
        }
    }

    #[test]
    fn a_missing_end_marker_is_an_error_and_is_named_apart() {
        // A truncated answer and a refused one are different failures; the status line says
        // the same thing, but the variant keeps them distinguishable in a test or a log.
        let extraction = extract("<<<TEXT>>>\nan answer that was cut off");

        assert_eq!(extraction, Extraction::MissingEnd);
        assert!(!matches!(extraction, Extraction::Rewrite(_)));
        assert_eq!(
            extraction.message(),
            Some("model did not use the output markers")
        );
    }

    #[test]
    fn markers_in_reversed_order_are_not_a_rewrite() {
        // An end marker first, then a start marker with nothing after it. There is a start
        // marker, so this is not `MissingStart`; there is no end marker *following* it, which
        // is exactly what `MissingEnd` names. Either way it is an error: reporting success
        // with an empty body would be the guess the protocol exists to remove.
        let extraction = extract("<<<END>>>content<<<TEXT>>>");

        assert_eq!(extraction, Extraction::MissingEnd);
        assert!(!matches!(extraction, Extraction::Rewrite(_)));
        assert!(
            extraction.message().is_some(),
            "an error must say something"
        );
    }

    #[test]
    fn empty_content_between_the_markers_is_success_with_empty_text() {
        // Both markers present, so the protocol was followed; the model simply produced
        // nothing. That is a valid extraction and the caller decides what to show.
        assert_eq!(rewrite("<<<TEXT>>><<<END>>>"), "");
        assert_eq!(rewrite("<<<TEXT>>>   \n  <<<END>>>"), "");
    }

    #[test]
    fn duplicated_marker_pairs_resolve_to_the_first_complete_one() {
        // The first complete pair is the answer; a repeated pair is noise around it.
        assert_eq!(
            rewrite("<<<TEXT>>>first<<<END>>>\n<<<TEXT>>>second<<<END>>>"),
            "first"
        );
    }

    #[test]
    fn a_nested_start_marker_is_kept_as_text_because_it_is_between_the_pair() {
        // A start marker inside the body is not a delimiter to the search: once a start is
        // found, only the end marker closes it. The text is what the model wrote.
        assert_eq!(
            rewrite("<<<TEXT>>>a <<<TEXT>>> b<<<END>>>"),
            "a <<<TEXT>>> b"
        );
    }

    #[test]
    fn a_tool_invocation_inside_the_markers_is_an_error() {
        // Measured (spec §6.1a step 6): with tools denied the model prints the call as text
        // and still exits 0, so the exit status cannot be the signal. Nothing executed, and it
        // is not a rewrite.
        let text = "<<<TEXT>>>\n<invoke name=\"Write\">\n<parameter name=\"path\">x</parameter>\n</invoke>\n<<<END>>>";
        let extraction = extract(text);

        match &extraction {
            Extraction::ToolCall(call) => assert!(
                call.contains("<invoke name="),
                "the call should be shown so the user can see what was attempted"
            ),
            other => panic!("expected a tool call, got {other:?}"),
        }
        assert!(!matches!(extraction, Extraction::Rewrite(_)));
        assert!(
            extraction.message().unwrap().contains("tool"),
            "the message should explain that tools are denied"
        );
    }

    #[test]
    fn a_tool_call_in_the_preamble_is_discarded_with_the_rest_of_it() {
        // Outside the markers everything is discarded, so a call the model printed before the
        // answer does not poison a good rewrite. The rewritten text is what matters.
        let text = "<invoke name=\"Write\"></invoke>\n<<<TEXT>>>the rewrite<<<END>>>";

        assert_eq!(rewrite(text), "the rewrite");
    }

    #[test]
    fn a_partial_tool_tag_is_not_a_tool_call() {
        // Only the measured opener counts. A rewrite that merely mentions the tag in prose
        // must not be rejected by a substring that is not a real invocation.
        assert_eq!(
            rewrite("<<<TEXT>>>the tag <invoke> is not a call<<<END>>>"),
            "the tag <invoke> is not a call"
        );
    }

    #[test]
    fn a_marker_echoed_inside_the_answer_cuts_it_short() {
        // The known limitation, recorded rather than fixed (see the `prompt-template` ticket):
        // extraction finds the first complete pair, so markers quoted *inside* the rewrite
        // close it early. Escaping them would be a silent transformation of the user's text,
        // which is worse than an edge case the user can see and edit afterwards.
        let text =
            "<<<TEXT>>>The model should see <<<TEXT>>> and <<<END>>> in the prompt.<<<END>>>";

        assert_eq!(rewrite(text), "The model should see <<<TEXT>>> and");
    }

    #[test]
    fn unicode_and_windows_line_endings_pass_through() {
        assert_eq!(
            rewrite("<<<TEXT>>>café — naïve\r\nsecond line 😀<<<END>>>"),
            "café — naïve\r\nsecond line 😀",
            "text is not rewritten, only delimited"
        );
    }

    #[test]
    fn the_search_does_not_depend_on_line_structure() {
        // Neither harness wraps a marker across lines, but the search is a plain substring
        // search, so the surrounding lines must not matter in either direction: an answer on
        // its own line and one buried in a paragraph extract identically.
        assert_eq!(
            rewrite("leading prose <<<TEXT>>>text<<<END>>> trailing"),
            "text"
        );
        assert_eq!(
            rewrite(
                "<<<TEXT>>>text<<<END>>>
"
            ),
            "text"
        );

        // And an unclosed marker is still an error, however it is surrounded.
        assert_eq!(
            extract(
                "leading prose <<<TEXT>>>text
"
            ),
            Extraction::MissingEnd
        );
    }
}
