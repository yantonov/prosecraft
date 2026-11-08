//! The `codex` adapter (spec §6.5).
//!
//! Shares the prompt template and the marker protocol with [`super::claude`] and [`super::pi`],
//! and differs from both in the places §6.4 predicts. Two of those differences are not
//! cosmetic and are the reason each one carries its measurement in a comment below:
//!
//! * **No tool-denial flag exists.** `claude --tools ""` and `pi --no-tools` have no codex
//!   equivalent, so the run is *sandboxed* instead — read-only filesystem, approvals never
//!   answered. That is a weaker guarantee than §1 asks for and it was accepted deliberately
//!   (see §6.5); a test below pins the exact flags so a later "tidy" cannot quietly drop one
//!   of the pair.
//! * **The harness names its own session.** There is no `--session-id`; the id appears on
//!   stdout as `thread.started`, which is why this adapter's [`HarnessAdapter::new_session_id`]
//!   returns `None` and it implements [`HarnessAdapter::session_from_event`].
//!
//! It also cannot stream (§6.1b): `exec --json` emits one complete `agent_message`, so
//! `classify` has no `Text` case to report and the debug pane fills in jumps. That is a
//! property of the harness, not an omission here.
//!
//! The JSON reader is the one in [`super::claude`]: codex's lines are the same shape of thing,
//! and a second reader would be a second place for a parsing bug to live.

use super::claude::{Json, carries_tool_use, field, parse as parse_json};
use super::{BuildError, Event, HarnessAdapter, Invocation, Task, prompt};

/// The codex harness as an adapter.
///
/// Holds no state: the session id is not chosen here at all, which is the whole point.
#[derive(Debug, Clone, Copy, Default)]
pub struct Codex;

/// The `-c` overrides that make one invocation, in order (spec §6.5).
///
/// A single const rather than flags pushed inline, because these are the invocation's *safety*
/// and each one's reason is subtle enough that the block should read as one thing. Every entry
/// was measured on codex-cli 0.159.0; see the comment above each.
const OVERRIDES: [&str; 5] = [
    // The sandbox. `sandbox_mode="read-only"` **and** `approval_policy="never"` must both be
    // present: measured, either alone still allowed a write (3 of 3 runs each), while the pair
    // blocked every attempt (9 of 9). This is the substitution for claude's `--tools ""` and
    // pi's `--no-tools`, and it denies one layer lower — the sandbox, not the tool list.
    "sandbox_mode=\"read-only\"",
    "approval_policy=\"never\"",
    // codex's equivalent of pi's `--no-context-files`: without it the user's own AGENTS.md is
    // in the prompt and the model reproduces it verbatim when asked. A rewrite must not
    // silently inherit the user's coding guidelines.
    "project_doc_max_bytes=0",
    // Trims the offered tool set from 147 to 29 and takes the whole Codex app MCP surface out
    // of the prompt.
    "features.apps=false",
    // Only load-bearing *together with* `features.apps=false`: measured, each alone still let
    // the model search the web, and only the pair refused. Neither flag is redundant.
    "web_search=\"disabled\"",
];

impl HarnessAdapter for Codex {
    fn name(&self) -> &str {
        "codex"
    }

    /// `None`, because codex mints its own thread id and reports it on stdout (spec §6.5).
    ///
    /// Returning a generated id here would put a name in the status line that no process ever
    /// used, while the real thread ran under the id codex chose.
    fn new_session_id(&self) -> Option<String> {
        None
    }

    /// Builds the argv from spec §6.5.
    ///
    /// First call and continuation differ by the `resume <id>` subcommand rather than by a
    /// flag, so the two shapes are built separately but share [`OVERRIDES`] — one flag block,
    /// one place to get the sandbox wrong instead of two.
    fn build(&self, task: &Task<'_>) -> Result<Invocation, BuildError> {
        let mut args: Vec<String> = Vec::with_capacity(24);
        args.push("exec".to_owned());
        if task.session.is_some() {
            args.push("resume".to_owned());
        }
        // `--json` is the machine-readable mode, and `exec resume` needs it repeated. It is
        // *not* a stream: measured, one `item.completed` carries the whole answer (§6.1b).
        args.push("--json".to_owned());

        // The sandbox, the context-file cut, and the tool-set trim. Values stay one element
        // each: `-c` takes `key=value` whole, and unquoted spaces in `sandbox_mode="read-only"`
        // are part of the value codex parses as TOML, not shell syntax to be stripped here.
        for override_value in OVERRIDES {
            args.push("-c".to_owned());
            args.push(override_value.to_owned());
        }

        // Measured: without it codex refuses to run outside a git repo with
        // `Not inside a trusted directory`, which would break the app in every ordinary
        // directory that is not a checkout.
        args.push("--skip-git-repo-check".to_owned());

        // A pass-through from `--model` on the prosecraft command line, placed after the
        // measured block to keep that block contiguous and in spec order.
        if let Some(model) = task.model {
            args.push("--model".to_owned());
            args.push(model.to_owned());
        }

        // The continuation's thread id is positional and must come before the prompt's `--`.
        if let Some(id) = task.session {
            args.push(id.to_owned());
        }

        // `-- -` puts the prompt on stdin: the positional is a literal dash, so codex reads the
        // prompt from the pipe. Stdin on every platform, not just on a Windows shim — measured,
        // the real template round-trips verbatim both ways, and this keeps user prose out of the
        // OS argument list entirely (spec §6.5). `Transport` is therefore not consulted: for
        // this adapter the transport is a property of the harness, not of the resolution.
        args.push("--".to_owned());
        args.push("-".to_owned());

        Ok(Invocation {
            program: self.name().to_owned(),
            args,
            stdin: Some(prompt(task)),
        })
    }

    /// Labels one stdout line for the debug pane.
    ///
    /// `Text` is **unreachable**, and that is the measured truth rather than an omission:
    /// `exec --json` emits no per-token delta (§6.1b). Every recognised event is therefore
    /// `Junk` — labelled in the debug pane, never mistaken for prose — and the answer arrives
    /// through [`HarnessAdapter::final_text`] alone.
    fn classify(&self, line: &str) -> Event {
        let Some(json) = parse_json(line) else {
            return Event::Junk;
        };
        // Before anything else: a command, a file change, a web search or an MCP call must
        // never be labelled prose, whatever shape it arrives in.
        if carries_tool_use(&json) {
            return Event::Junk;
        }
        // Deliberately exhaustive-but-flat: `thread.started`, `turn.started`,
        // `item.started/completed`, `turn.completed`, `turn.failed`, `error` and anything a
        // future version adds all map to `Junk`, so an unknown event cannot abort a run.
        Event::Junk
    }

    /// The answer from an `agent_message` item, and from nothing else.
    ///
    /// codex reports `item.completed` for every kind of item, including shell commands and file
    /// changes that carry their own output. Taking "the item with text" would put a command's
    /// stdout in the output pane, so the item's `type` is checked first and the text is read
    /// from `item.text`.
    fn final_text(&self, line: &str) -> Option<String> {
        let json = parse_json(line)?;
        if field(&json, "type").and_then(Json::as_str) != Some("item.completed") {
            return None;
        }
        let item = field(&json, "item")?;
        // Only `agent_message` is prose. `command_execution`, `file_change`, `web_search`,
        // `mcp_tool_call` and `reasoning` all carry text and none of them is the rewrite.
        if field(item, "type").and_then(Json::as_str) != Some("agent_message") {
            return None;
        }
        field(item, "text")
            .and_then(Json::as_str)
            .map(str::to_owned)
    }

    /// The `thread_id` from `thread.started`, which is the only line that announces one.
    ///
    /// codex ignores a client-supplied id, so this is the only way the app can learn the
    /// thread it must resume (spec §6.5). Every other event returns `None`; several carry an
    /// `id` field of their own (`item.*`) and those identify items, not threads.
    fn session_from_event(&self, line: &str) -> Option<String> {
        let json = parse_json(line)?;
        if field(&json, "type").and_then(Json::as_str) != Some("thread.started") {
            return None;
        }
        field(&json, "thread_id")
            .and_then(Json::as_str)
            .map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::HarnessAdapter;
    use crate::harness::Transport;
    use crate::harness::claude::Claude;

    fn harness() -> Codex {
        Codex
    }

    fn task<'a>(instructions: &'a str, source_text: &'a str, model: Option<&'a str>) -> Task<'a> {
        Task {
            instructions,
            source_text,
            model,
            session: None,
            transport: Transport::Argv,
        }
    }

    /// One argv row, mirroring the claude and pi tables so the three cannot drift apart.
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
            transport: Transport::Argv,
        };
        let invocation = harness()
            .build(&task)
            .unwrap_or_else(|err| panic!("{}: build failed: {err}", row.name));

        assert_eq!(invocation.program, "codex", "{}: program", row.name);
        // The prompt is always on stdin for this adapter, whatever the resolution said, so
        // `args` never carries it and there is nothing to leak into the OS argument list.
        assert_eq!(
            invocation.stdin.as_deref(),
            Some(prompt(&task).as_str()),
            "{}: the prompt must be the whole stdin payload",
            row.name
        );
        assert!(
            !invocation
                .args
                .iter()
                .any(|arg| arg.starts_with("INSTRUCTIONS:")),
            "{}: the prompt leaked into argv: {:?}",
            row.name,
            invocation.args
        );

        let expected: Vec<String> = row.expected.iter().map(|arg| (*arg).to_owned()).collect();
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
    fn argv_is_spec_6_5_for_the_first_call_without_a_model() {
        assert_row(&Row {
            name: "first call, no --model",
            model: None,
            session: None,
            expected: &[
                "exec",
                "--json",
                "-c",
                "sandbox_mode=\"read-only\"",
                "-c",
                "approval_policy=\"never\"",
                "-c",
                "project_doc_max_bytes=0",
                "-c",
                "features.apps=false",
                "-c",
                "web_search=\"disabled\"",
                "--skip-git-repo-check",
                "--",
                "-",
            ],
        });
    }

    #[test]
    fn argv_is_spec_6_5_for_a_continuation_without_a_model() {
        // The difference is the `resume` subcommand and the positional id, not a flag: codex
        // continues a thread by naming it after `resume` (spec §6.5).
        assert_row(&Row {
            name: "continuation, no --model",
            model: None,
            session: Some("01a1bc23-1144-7de2-9a09-7a777c4a420c"),
            expected: &[
                "exec",
                "resume",
                "--json",
                "-c",
                "sandbox_mode=\"read-only\"",
                "-c",
                "approval_policy=\"never\"",
                "-c",
                "project_doc_max_bytes=0",
                "-c",
                "features.apps=false",
                "-c",
                "web_search=\"disabled\"",
                "--skip-git-repo-check",
                "01a1bc23-1144-7de2-9a09-7a777c4a420c",
                "--",
                "-",
            ],
        });
    }

    #[test]
    fn a_model_lands_after_the_measured_block_in_both_shapes() {
        // `--model` is a pass-through, not part of the measured argv, so its position is a
        // readability choice: directly after `--skip-git-repo-check`, keeping the flagged block
        // in spec order. Named here so nobody has to guess whether it was measured.
        assert_row(&Row {
            name: "first call with --model",
            model: Some("gpt-6.1-sol"),
            session: None,
            expected: &[
                "exec",
                "--json",
                "-c",
                "sandbox_mode=\"read-only\"",
                "-c",
                "approval_policy=\"never\"",
                "-c",
                "project_doc_max_bytes=0",
                "-c",
                "features.apps=false",
                "-c",
                "web_search=\"disabled\"",
                "--skip-git-repo-check",
                "--model",
                "gpt-6.1-sol",
                "--",
                "-",
            ],
        });
        assert_row(&Row {
            name: "continuation with --model",
            model: Some("gpt-6.1-sol"),
            session: Some("01a1bc23-1144-7de2-9a09-7a777c4a420c"),
            expected: &[
                "exec",
                "resume",
                "--json",
                "-c",
                "sandbox_mode=\"read-only\"",
                "-c",
                "approval_policy=\"never\"",
                "-c",
                "project_doc_max_bytes=0",
                "-c",
                "features.apps=false",
                "-c",
                "web_search=\"disabled\"",
                "--skip-git-repo-check",
                "--model",
                "gpt-6.1-sol",
                "01a1bc23-1144-7de2-9a09-7a777c4a420c",
                "--",
                "-",
            ],
        });
    }

    #[test]
    fn tools_are_denied_by_a_sandbox_not_by_a_missing_tool() {
        // The load-bearing pair. Measured: `sandbox_mode="read-only"` alone left approvals
        // on-request and a write succeeded 3 of 3; `approval_policy="never"` alone likewise.
        // Together they blocked 9 of 9. codex has no `--tools ""` equivalent, so this is the
        // whole of the denial and neither flag may be dropped as "redundant".
        for row_session in [None, Some("id")] {
            let invocation = harness()
                .build(&Task {
                    session: row_session,
                    ..task("i", "s", None)
                })
                .unwrap();
            let joined = invocation.args.join(" ");

            assert!(
                joined.contains("sandbox_mode=\"read-only\""),
                "the filesystem sandbox is missing: {joined}"
            );
            assert!(
                joined.contains("approval_policy=\"never\""),
                "approvals are not disabled, so a write can be approved: {joined}"
            );
        }
    }

    #[test]
    fn the_context_files_are_cut_because_without_it_agents_md_leaks_in() {
        // Measured: without `project_doc_max_bytes=0` the user's own AGENTS.md is in the prompt
        // and the model reproduces it verbatim when asked.
        let invocation = harness().build(&task("i", "s", None)).unwrap();

        assert!(
            invocation
                .args
                .iter()
                .any(|arg| arg == "project_doc_max_bytes=0"),
            "the user's AGENTS.md is not excluded: {:?}",
            invocation.args
        );
    }

    #[test]
    fn the_web_is_closed_by_two_flags_that_only_work_together() {
        // Measured: `features.apps=false` alone still allowed a live web search, and
        // `web_search="disabled"` alone did too; only the pair refused. So a change that drops
        // either one re-opens the web, which is why both are asserted by name.
        let invocation = harness().build(&task("i", "s", None)).unwrap();
        let joined = invocation.args.join(" ");

        assert!(
            joined.contains("features.apps=false"),
            "the Codex app tool surface is still offered: {joined}"
        );
        assert!(
            joined.contains("web_search=\"disabled\""),
            "web search is still enabled: {joined}"
        );
    }

    #[test]
    fn the_flags_that_would_regress_the_sandbox_are_absent() {
        // `--ignore-user-config` disables the user's own config.toml (their model choice) and
        // measured it also changed whether writes were blocked. `-s` is the same policy in a
        // form `exec resume` rejects. `tools.web_search=false` does not remove the tool.
        // Reaching for any of them would be a regression, so they are named here.
        for session in [None, Some("01a1bc23-1144-7de2-9a09-7a777c4a420c")] {
            let invocation = harness()
                .build(&Task {
                    session,
                    ..task("i", "s", None)
                })
                .unwrap();

            for forbidden in [
                "--ignore-user-config",
                "-s",
                "--dangerously-bypass-approvals-and-sandbox",
                "tools.web_search=false",
            ] {
                assert!(
                    !invocation.args.iter().any(|arg| arg == forbidden),
                    "{forbidden} must not appear: {:?}",
                    invocation.args
                );
            }
        }
    }

    #[test]
    fn the_separator_sits_immediately_before_the_dash_prompt_and_appears_once() {
        let invocation = harness().build(&task("i", "s", None)).unwrap();
        let len = invocation.args.len();

        assert_eq!(invocation.args[len - 2], "--");
        assert_eq!(invocation.args[len - 1], "-");
        assert_eq!(invocation.args.iter().filter(|arg| *arg == "--").count(), 1);
    }

    #[test]
    fn a_prompt_with_newlines_and_a_leading_dash_never_reaches_argv() {
        // The strongest form of the transport rule: whatever the prompt contains, it is one
        // stdin payload and adds no argv element, so no newline or leading `-` can be
        // misread as a flag on any platform (spec §6.5).
        let instructions = "- drop the second person\n- use the active voice";
        let source = "--not-a-flag\nsecond line\n\nfourth line";
        let invocation = harness().build(&task(instructions, source, None)).unwrap();

        assert_eq!(
            invocation.stdin.as_deref(),
            Some(prompt(&task(instructions, source, None)).as_str())
        );
        // One argv regardless of how long the prompt is: nothing about it is echoed there.
        let short = harness().build(&task("", "", None)).unwrap();
        assert_eq!(invocation.args.len(), short.args.len());
        assert!(!invocation.stdin.as_deref().unwrap().is_empty());
    }

    #[test]
    fn the_stdin_transport_is_used_even_when_the_resolution_says_argv() {
        // `Transport` answers the Windows-shim question. codex's transport is fixed by the
        // harness, so the adapter must ignore the resolution rather than follow it: a shim
        // rule leaking in here would move the prompt into argv on unix.
        let argv_task = Task {
            transport: Transport::Argv,
            ..task("i", "s", None)
        };
        let stdin_task = Task {
            transport: Transport::Stdin,
            ..task("i", "s", None)
        };

        let from_argv = harness().build(&argv_task).unwrap();
        let from_stdin = harness().build(&stdin_task).unwrap();

        assert_eq!(from_argv.args, from_stdin.args);
        assert_eq!(from_argv.stdin, from_stdin.stdin);
        assert!(from_argv.stdin.is_some());
    }

    #[test]
    fn codex_names_its_own_sessions() {
        // `None` is the whole reason the trait grew `session_from_event`: codex ignores a
        // client-supplied id, so no id may be invented here (spec §6.5).
        assert_eq!(harness().new_session_id(), None);
    }

    #[test]
    fn the_thread_id_comes_from_thread_started_and_nowhere_else() {
        let id = "01a1bc23-1144-7de2-9a09-7a777c4a420c";

        assert_eq!(
            harness().session_from_event(&format!(
                r#"{{"type":"thread.started","thread_id":"{id}"}}"#
            )),
            Some(id.to_owned())
        );

        for line in [
            // `item.*` events carry their own `id`, which identifies an item, not a thread.
            r#"{"type":"item.completed","id":"item_2","item":{"type":"agent_message","text":"hi"}}"#,
            r#"{"type":"turn.started"}"#,
            r#"{"type":"turn.completed","usage":{}}"#,
            r#"{"type":"thread.started"}"#,
            r#"{"type":"thread.started","thread_id":42}"#,
            "",
            "not json",
        ] {
            assert_eq!(harness().session_from_event(line), None, "line {line:?}");
        }
    }

    #[test]
    fn the_answer_is_the_agent_message_item_text() {
        // Measured shape: the whole answer arrives in one `item.completed`.
        let line = r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"<<<TEXT>>>the rewrite<<<END>>>"}}"#;

        assert_eq!(
            harness().final_text(line),
            Some("<<<TEXT>>>the rewrite<<<END>>>".to_owned())
        );
        // Newlines inside the answer are the answer.
        assert_eq!(
            harness().final_text(
                r#"{"type":"item.completed","item":{"type":"agent_message","text":"one\n\ntwo"}}"#
            ),
            Some("one\n\ntwo".to_owned())
        );
    }

    #[test]
    fn no_other_item_is_ever_the_answer() {
        // The failure this prevents: codex reports commands, file changes, web searches and MCP
        // calls with text of their own, so "the last item with text" would put a command's
        // stdout or a search result in the output pane.
        for line in [
            r#"{"type":"item.completed","item":{"type":"command_execution","aggregated_output":"SECRET-STDOUT","exit_code":0}}"#,
            r#"{"type":"item.completed","item":{"type":"file_change","changes":[]}}"#,
            r#"{"type":"item.completed","item":{"type":"web_search","results":[{"type":"text_result"}]}}"#,
            r#"{"type":"item.completed","item":{"type":"mcp_tool_call","result":{"content":[]}}}"#,
            r#"{"type":"item.completed","item":{"type":"reasoning","text":"SECRET-REASONING"}}"#,
            r#"{"type":"item.completed","item":{"type":"error","message":"boom"}}"#,
            // An agent message with no string text is not prose.
            r#"{"type":"item.completed","item":{"type":"agent_message"}}"#,
            r#"{"type":"item.completed","item":{"type":"agent_message","text":null}}"#,
            r#"{"type":"item.completed","item":{"type":"agent_message","text":42}}"#,
            // Item lifecycle events carry no text at all.
            r#"{"type":"item.started","item":{"type":"command_execution"}}"#,
            r#"{"type":"item.updated","item":{"type":"command_execution"}}"#,
            r#"{"type":"turn.completed","item":{"type":"agent_message","text":"not an item"}}"#,
            r#"{"type":"agent_message","text":"not wrapped in an item"}"#,
            "",
            "not json",
        ] {
            assert_eq!(harness().final_text(line), None, "line {line:?}");
        }
    }

    #[test]
    fn no_line_is_ever_labelled_prose_because_codex_does_not_stream() {
        // Measured (§6.1b): `exec --json` emits no per-token delta, so `Text` has no source.
        // A future codex that started streaming should fail this test rather than silently
        // stream into a pane whose one prose label used to mean "a delta arrived".
        for line in [
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"the answer"}}"#,
            r#"{"type":"item.started","item":{"type":"command_execution"}}"#,
            r#"{"type":"thread.started","thread_id":"01a1"}"#,
            r#"{"type":"turn.started"}"#,
            r#"{"type":"turn.completed","usage":{}}"#,
            r#"{"type":"turn.failed","error":{"message":"boom"}}"#,
            r#"{"type":"error","message":"boom"}"#,
            "",
            "not json",
            "{",
        ] {
            assert_eq!(harness().classify(line), Event::Junk, "line {line:?}");
        }
    }

    #[test]
    fn a_tool_call_event_is_junk_rather_than_prose() {
        // Whatever shape a tool call arrives in, it must never be labelled prose. Shared with
        // the other two adapters, because the walk covers the whole tree.
        for line in [
            r#"{"type":"item.completed","item":{"type":"command_execution","aggregated_output":"x"}}"#,
            r#"{"type":"item.started","item":{"type":"mcp_tool_call","server":"codex_apps"}}"#,
            r#"{"type":"item.completed","item":{"type":"web_search"}}"#,
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"x"},"is_error":true}"#,
        ] {
            assert_eq!(harness().classify(line), Event::Junk, "line {line:?}");
        }
    }

    #[test]
    fn name_is_the_binary_name() {
        assert_eq!(harness().name(), "codex");
    }

    #[test]
    fn all_three_adapters_share_the_prompt_template() {
        // Spec §6.4 in one assertion: the prompt is the app's, the argv is the harness's. A
        // change that made one adapter send a different prompt would break here rather than in
        // a live run.
        let task = task("be formal", "hello", None);
        let claude = Claude.build(&task).unwrap();
        let codex = harness().build(&task).unwrap();
        let expected = prompt(&task);

        assert_eq!(
            claude.args.last(),
            Some(&expected),
            "claude's prompt drifted"
        );
        assert_eq!(
            codex.stdin.as_deref(),
            Some(expected.as_str()),
            "codex's prompt drifted"
        );
    }
}
