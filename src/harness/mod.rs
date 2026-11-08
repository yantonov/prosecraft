//! The harness adapter contract (spec §6): what the app needs from `claude` or `pi`.
//!
//! Everything here is pure. An adapter assembles strings, labels one stdout line, and
//! extracts one field; starting a process, draining a pipe and drawing a pane live
//! elsewhere. That split is what makes an adapter testable as a table of argv rows with no
//! binary on the machine and no child process to clean up.
//!
//! Not to be confused with [`crate::cli::Harness`]: that enum is the *command-line*
//! selector (`--harness claude|pi`) and answers only "which binary was asked for". This
//! trait is the behaviour behind it — session id form, argv, and event vocabulary — which
//! is why it is named `HarnessAdapter` and never plain `Harness`: at a call site the two
//! would otherwise be interchangeable to a reader.

pub mod claude;
pub mod pi;
pub mod resolve;

use std::fmt;

/// One rewrite request: the user's instructions, the text to rewrite, and the session it
/// runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Task<'a> {
    /// The instructions pane, verbatim. Also sent as the harness system prompt on the
    /// first call, for register only: claude ignores that flag on resume, so correctness
    /// never depends on it (spec §6.0).
    pub instructions: &'a str,
    /// The source pane, verbatim, resent on every call (spec §5).
    pub source_text: &'a str,
    /// `--model` value, passed through unchanged; `None` leaves the harness's default.
    pub model: Option<&'a str>,
    /// `None` means the first call of this run (spec §6.4): there is no session to
    /// continue. `Some` is the id of the session to resume.
    pub session: Option<&'a str>,
    /// Where the prompt has to travel, decided by the resolution the machine holds.
    ///
    /// Not a per-adapter choice: the same pi binary takes the prompt in `argv` on unix and on
    /// stdin on Windows when it is installed as a batch shim, because `cmd.exe` re-parses a
    /// batch file's command line and no argument carrying a newline survives (spec §6).
    pub transport: Transport,
}

/// How the prompt reaches the harness (spec §6).
///
/// Two variants and no default: an adapter that guessed would be right on one platform and
/// silently truncate the prompt on the other, which is the failure this removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// The prompt is one element of `argv`, after a `--` separator.
    Argv,
    /// The prompt is written to the child's stdin; `argv` carries no `--` and no prompt.
    Stdin,
}

/// One process invocation: the program, every argument after it, and what to write to the
/// child's stdin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: String,
    /// Everything after the program name, in order, including the `--` separator and the
    /// user prompt. Elements are never joined: a prompt containing a newline or a leading
    /// `-` survives only if it stays one element (spec §6).
    pub args: Vec<String>,
    /// `None` means nothing is written to stdin; `Some` is the prompt, which is how a Windows
    /// batch shim receives it (spec §6). Exactly one of this and the prompt in `args` is set.
    pub stdin: Option<String>,
}

/// The kind of one stdout line, for the debug pane only (spec §6, §6.1b).
///
/// The enum is small on purpose: it labels a debug line and nothing else, so it must never
/// become a second channel for output text. Only [`Event::Text`] means "this line is
/// streamed prose"; the output pane is written from [`HarnessAdapter::final_text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Streamed prose: a `text_delta`.
    Text,
    /// Reasoning, or the signature of a reasoning block. Visible in the debug pane on
    /// purpose, and never mistaken for the result.
    Think,
    /// Anything else: not JSON, not recognised, or recognised and deliberately not shown
    /// as prose. Never an error (spec §6) — a harness printing garbage is a diagnosable
    /// event, not a reason to abort a run.
    Junk,
}

/// Why an adapter could not turn a task into an invocation.
///
/// No adapter fails today: building an argv is string assembly, and both id forms are
/// generated. The type exists because spec §6 gives [`HarnessAdapter::build`] a `Result`,
/// and it carries a message so a future failure can reach the status line unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError(pub String);

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BuildError {}

/// The contract from spec §6, implemented once per harness binary.
pub trait HarnessAdapter {
    /// The binary name, which is also the name users know (`claude`, `pi`).
    fn name(&self) -> &str;

    /// A fresh session id in the form this harness accepts.
    ///
    /// The one method that needs randomness, and the reason it is on the trait: claude
    /// takes a UUID v4 and pi takes its own restricted alphabet (spec §6.2, §6.3).
    fn new_session_id(&self) -> String;

    /// Builds the argv for one call.
    ///
    /// Pure: no process is started, nothing outside `task` is read, so every flag can be
    /// asserted directly. The one call that needs randomness is [`Self::new_session_id`],
    /// and an adapter uses it here for a first call.
    ///
    /// The arguments come from `task` alone. Spec §6's code block passes the session as a
    /// second parameter while §6.4 says `Task` carries it; this follows §6.4, so there is
    /// one source of truth and no parameter that can disagree with the task.
    ///
    /// `task.session == None` is the first call of the run, so the adapter creates the
    /// session itself and the id it chose is the value after the session flag in the
    /// returned argv. That is the id the caller keeps and passes as `Some` on every later
    /// call — the adapter holds no state, so the argv is the only place it can be reported.
    fn build(&self, task: &Task<'_>) -> Result<Invocation, BuildError>;

    /// Classifies one stdout line for the debug pane. Never produces output text.
    fn classify(&self, line: &str) -> Event;

    /// Authoritative final text, if this line was the terminal event.
    ///
    /// At most one line per run answers `Some`; that text is what the output pane gets
    /// (spec §6.1b). Nothing is ever reconstructed from deltas.
    fn final_text(&self, line: &str) -> Option<String>;
}

/// The sentence that asks for the markers. Verbatim from spec §6.1, on one line so nothing
/// can split it.
const MARKER_DEMAND: &str = "Return only the rewritten text, wrapped between the markers <<<TEXT>>> and <<<END>>>. Put nothing else between the markers. Anything outside the markers is ignored.";

/// The fixed user-turn prompt from spec §6.1: instructions, source text, then the marker
/// demand, in that order, for both harnesses and for first call and continuation alike.
///
/// Kept separate from [`HarnessAdapter::build`] because it is the one part of a request
/// that every adapter shares and the app designs rather than the harness: revisiting the
/// template changes this function and no adapter.
///
/// The demand sits last because models weight the tail of the prompt most heavily, and
/// the instructions are a free-text pane, so the markers are what make the answer
/// extractable at all rather than a guess at where the rewrite starts (spec §6.1, §6.1a).
///
/// **Known limitation, deliberately not fixed.** If the source text itself contains
/// `<<<TEXT>>>` or `<<<END>>>`, extraction finds the first complete pair and can cut the
/// rewrite short. Escaping or rewriting the source here would be a silent transformation of
/// the user's input, which is worse than the edge case: rewriting text that *discusses* these
/// markers is unlikely, and when it happens the result is visible in the output pane and can
/// be corrected outside the app. The alternative — hashing the markers, or picking a delimiter
/// that cannot occur — trades a rare, visible problem for a permanent one, since the model
/// must be told the delimiter in the prompt anyway.
pub fn prompt(task: &Task<'_>) -> String {
    format!(
        "INSTRUCTIONS:\n{}\n\nTEXT TO REWRITE:\n{}\n\n{MARKER_DEMAND}",
        task.instructions, task.source_text
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task<'a>(instructions: &'a str, source_text: &'a str) -> Task<'a> {
        Task {
            instructions,
            source_text,
            model: None,
            session: None,
            transport: Transport::Argv,
        }
    }

    #[test]
    fn prompt_is_instructions_source_and_the_marker_demand_in_that_order() {
        let text = prompt(&task("make it formal", "hello\nthere"));

        // Written out rather than read back from the implementation: the template is a
        // contract with the model, so the test has to fail when the shape drifts.
        assert_eq!(
            text,
            "INSTRUCTIONS:\nmake it formal\n\nTEXT TO REWRITE:\nhello\nthere\n\n\
             Return only the rewritten text, wrapped between the markers <<<TEXT>>> and \
             <<<END>>>. Put nothing else between the markers. Anything outside the markers \
             is ignored."
        );
    }

    #[test]
    fn prompt_ends_with_the_marker_demand() {
        // The demand is the tail of the prompt on purpose (spec §6.1); anything appended
        // after it would weaken it.
        let text = prompt(&task("anything", "anything"));

        assert!(text.ends_with(MARKER_DEMAND));
    }

    #[test]
    fn prompt_passes_text_through_verbatim() {
        // Nothing is escaped, trimmed or re-wrapped: the model is asked to rewrite what the
        // user typed, so changing it on the way out would rewrite something else.
        let instructions = "- keep the dashes\nand the blank line\n\nend";
        let source = "a\r\nb";
        let text = prompt(&task(instructions, source));

        assert!(
            text.contains(instructions),
            "instructions altered: {text:?}"
        );
        assert!(text.contains(source), "source altered: {text:?}");
        assert!(text.starts_with("INSTRUCTIONS:\n- keep the dashes"));
    }

    #[test]
    fn a_first_call_and_a_continuation_carry_the_identical_prompt() {
        // Spec §6.0: instructions travel in the user turn precisely so that a continuation
        // behaves like a first call. If the template differed, editing the instructions and
        // pressing Ctrl+R again would work on one call shape and not the other.
        use crate::harness::HarnessAdapter;
        use crate::harness::claude::Claude;

        let first = HarnessAdapter::build(
            &Claude,
            &Task {
                instructions: "make it formal",
                source_text: "hello
there",
                model: None,
                session: None,
                transport: Transport::Argv,
            },
        )
        .unwrap();
        let next = HarnessAdapter::build(
            &Claude,
            &Task {
                instructions: "make it formal",
                source_text: "hello
there",
                model: None,
                session: Some("11111111-2222-4333-8444-555555555555"),
                transport: Transport::Argv,
            },
        )
        .unwrap();

        // The prompt is the last argv element in both, and it must be byte-identical.
        assert_eq!(
            first.args.last(),
            next.args.last(),
            "the continuation prompt differs from the first-call prompt"
        );
        // And it is the template, not something the adapter invented.
        assert_eq!(
            first.args.last().unwrap(),
            &prompt(&task(
                "make it formal",
                "hello
there"
            ))
        );
    }

    #[test]
    fn the_golden_template_is_byte_for_byte_what_the_ticket_specifies() {
        // The ticket asks for this in as many words, so it is spelled out as one literal with
        // no formatting splices: the point is that a reader can compare it against spec §6.1
        // by eye, and that a stray space or a reordered line fails the test rather than
        // slipping through a helper.
        let instructions = "rewrite in a formal register";
        let source = "hey can u send me that file

thanks";
        let expected = concat!(
            "INSTRUCTIONS:
",
            "rewrite in a formal register
",
            "
",
            "TEXT TO REWRITE:
",
            "hey can u send me that file
",
            "
",
            "thanks
",
            "
",
            "Return only the rewritten text, wrapped between the markers <<<TEXT>>> and <<<END>>>. ",
            "Put nothing else between the markers. Anything outside the markers is ignored.",
        );

        assert_eq!(prompt(&task(instructions, source)), expected);
    }

    #[test]
    fn a_source_containing_the_markers_is_passed_through_unescaped() {
        // The documented limitation, pinned as behaviour rather than left to prose: the source
        // is never transformed, even when it contains the delimiters. Extraction may then cut
        // the rewrite short, which is visible in the output pane and correctable outside.
        let source = "the protocol uses <<<TEXT>>> and <<<END>>> as delimiters";
        let text = prompt(&task("explain it", source));

        assert!(
            text.contains(source),
            "the source was escaped or altered: {text:?}"
        );
        assert_eq!(
            text.matches("<<<TEXT>>>").count(),
            2,
            "the source's marker should appear exactly as written, alongside the demand"
        );
    }

    #[test]
    fn an_empty_task_still_produces_a_complete_prompt() {
        // Validation refuses empty instructions before a call is built (spec §5); this
        // pins what the template does anyway, so the shape never depends on the caller.
        let text = prompt(&task("", ""));

        assert!(text.contains("INSTRUCTIONS:\n\n\nTEXT TO REWRITE:\n\n\n"));
        assert!(text.ends_with(MARKER_DEMAND));
    }
}
