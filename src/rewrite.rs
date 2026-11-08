//! The rewrite state machine: validate, run, settle (spec §5).
//!
//! The four states and the arrows between them are the whole of this module. It owns no
//! terminal and draws nothing: it takes a task, decides whether the task may run, hands back
//! the stream lines it collected, and reports how the run ended. That split is what lets the
//! transitions be tested headlessly, which the ticket asks for explicitly — no test here opens
//! a TTY or a real harness.
//!
//! Two rules from §5 are easy to get subtly wrong and are therefore stated in code rather than
//! commented around:
//!
//! * **Validation happens before anything is spawned.** A refused rewrite leaves the app idle
//!   with a status-line reason and focus on the culprit, so the user fixes the pane that caused
//!   it instead of hunting for one.
//! * **`Ctrl+R` and `Ctrl+N` are ignored while a run is live.** Not queued and not applied on
//!   completion: resuming a discarded session id, or starting a second run over a live one,
//!   would both be worse than telling the user to wait.
//!
//! [`Child`] is a trait rather than a direct [`crate::process::Run`] for one reason: a test can
//! then drive every transition — done, error, cancelled, timeout — without a process, and the
//! production path is one thin implementation of three methods.

use std::time::{Duration, Instant};

use crate::harness::{HarnessAdapter, Invocation, Task};
use crate::panes::{Settled, StreamState};
use crate::process::{self, Line, Outcome};

/// The wall-clock limit for one run, re-exported so callers name it in one place (spec §7).
///
/// Nothing reads this yet: the timeout is applied inside [`process::Run`], and the ticket that
/// surfaces it to the UI is `robustness`. The test below pins the value so the two cannot drift.
#[allow(dead_code)]
pub const DEFAULT_TIMEOUT: Duration = process::DEFAULT_TIMEOUT;

/// Why a rewrite was refused before it started.
///
/// One variant per row of the §5 step 1 list, because each names a different pane to move
/// focus to and a different sentence to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Instructions are empty or whitespace-only. The required validation.
    NoInstructions,
    /// Source text is empty or whitespace-only.
    NoSource,
    /// The harness binary no longer resolves on `PATH`, having resolved at startup.
    HarnessGone,
    /// A rewrite is already running.
    AlreadyRunning,
}

impl Refusal {
    /// What the status line says. Short, and phrased so the missing thing is named.
    pub fn message(self) -> &'static str {
        match self {
            Self::NoInstructions => "instructions are empty",
            Self::NoSource => "source text is empty",
            Self::HarnessGone => "the harness binary is no longer on PATH",
            Self::AlreadyRunning => "a rewrite is already running",
        }
    }
}

/// Something that behaves like a spawned harness for the length of one run.
///
/// The three methods are the whole of what the state machine needs, so a test fake is a
/// handful of lines and production is a pass-through to [`crate::process::Run`].
pub trait Child {
    /// Every line that has already arrived, without blocking.
    fn drain(&mut self) -> Vec<Line>;
    /// How the run ended, once, or `None` while it is still going.
    fn poll(&mut self) -> Option<Outcome>;
    /// Asks the child to stop. Not required to be immediate.
    ///
    /// Unused until `robustness` wires `Ctrl+C` mid-run; the fake children implement it and
    /// one test drives it, so the contract is exercised even though production does not call
    /// it yet.
    #[allow(dead_code)]
    fn cancel(&mut self);
}

impl Child for process::Run {
    fn drain(&mut self) -> Vec<Line> {
        process::Run::drain(self)
    }

    fn poll(&mut self) -> Option<Outcome> {
        process::Run::poll(self)
    }

    fn cancel(&mut self) {
        process::Run::cancel(self)
    }
}

/// Creates a child from an invocation. Injected so a test can supply a fake.
pub type Spawner = Box<dyn Fn(&Invocation) -> std::io::Result<Box<dyn Child>>>;

/// What one [`Machine::tick`] did, for the caller to draw or to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tick {
    /// Nothing to do: no run is live.
    Idle,
    /// Lines arrived and were routed; the run is still going.
    Streaming,
    /// The run ended and the panes were settled. The value is what to do with the state word.
    Finished(Settled),
}

/// The rewrite state machine.
///
/// Holds at most one live child, which is what makes "one harness process at a time"
/// (spec §2) a property of the type rather than of every call site.
pub struct Machine {
    /// The live run, if any.
    child: Option<Box<dyn Child>>,
    /// The session id for this TUI run, created on the first rewrite and reset by
    /// `Ctrl+N` (spec §5 step 3). `None` means the next call opens a new session.
    session: Option<String>,
    adapter: Box<dyn HarnessAdapter>,
    spawner: Spawner,
}

impl std::fmt::Debug for Machine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Machine")
            .field("running", &self.child.is_some())
            .field("session", &self.session)
            .field("harness", &self.adapter.name())
            .finish()
    }
}

impl Machine {
    /// A machine that spawns real processes through [`process::Run`].
    pub fn new(adapter: Box<dyn HarnessAdapter>) -> Self {
        Self {
            child: None,
            session: None,
            adapter,
            spawner: Box::new(|invocation| {
                process::Run::spawn(invocation).map(|run| Box::new(run) as Box<dyn Child>)
            }),
        }
    }

    /// A machine that spawns whatever `spawner` returns, for tests.
    #[allow(dead_code)]
    pub fn with_spawner(adapter: Box<dyn HarnessAdapter>, spawner: Spawner) -> Self {
        Self {
            child: None,
            session: None,
            adapter,
            spawner,
        }
    }

    /// Whether a run is live. `Ctrl+R` and `Ctrl+N` are refused while it is.
    pub fn running(&self) -> bool {
        self.child.is_some()
    }

    /// The session id, once one exists.
    pub fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }

    /// `Ctrl+N`: drop the session so the next rewrite opens a new one (spec §5 step 3).
    ///
    /// Refused while a run is live, because the live process is using the id.
    pub fn reset_session(&mut self) -> Result<(), Refusal> {
        if self.running() {
            return Err(Refusal::AlreadyRunning);
        }
        self.session = None;
        Ok(())
    }

    /// Validates and starts a run (spec §5 steps 1 and 2).
    ///
    /// `harness_present` is a parameter rather than a lookup so the "the binary disappeared"
    /// row can be tested without touching `PATH`, and so the caller decides what "present"
    /// means, exactly as the startup path does.
    ///
    /// On success the panes are cleared and a `run started` line is added, which is the
    /// visible signal that a new action began (spec §5 step 4). On refusal nothing is touched.
    pub fn start(
        &mut self,
        task: &Task<'_>,
        harness_present: bool,
        stream: &mut StreamState,
        now: Instant,
    ) -> Result<(), Refusal> {
        // Order matters: the list in §5 step 1 is the order the user can act on. Instructions
        // first because they are the required validation.
        if task.instructions.trim().is_empty() {
            return Err(Refusal::NoInstructions);
        }
        if task.source_text.trim().is_empty() {
            return Err(Refusal::NoSource);
        }
        if !harness_present {
            return Err(Refusal::HarnessGone);
        }
        if self.running() {
            return Err(Refusal::AlreadyRunning);
        }

        // `None` on the first call of the run: the adapter generates the id and reports it
        // through the argv, which is where it is read back (spec §6.2).
        let invocation = self.adapter.build(task).map_err(|_| Refusal::HarnessGone)?;

        let child = match (self.spawner)(&invocation) {
            Ok(child) => child,
            // A spawn failure is a runtime failure, not a validation one: the binary existed a
            // moment ago. §8 puts it in the error state with the reason on the debug pane.
            Err(err) => {
                stream.spawn(now);
                stream.app(format!("could not start the harness: {err}"));
                return Err(Refusal::HarnessGone);
            }
        };

        // Remember the id the adapter generated, so every later call continues the session.
        if self.session.is_none() {
            self.session = session_from(&invocation);
        }
        self.child = Some(child);
        stream.spawn(now);
        Ok(())
    }

    /// Drains the live run, routes what arrived into the panes, and settles it if it ended.
    ///
    /// Called once per UI tick. Returns what happened so the caller can update the state word
    /// without this module needing to know what a status line is.
    pub fn tick(&mut self, stream: &mut StreamState) -> Tick {
        let Some(child) = self.child.as_mut() else {
            return Tick::Idle;
        };

        // Drain before polling: `poll` only reports once the readers have finished, so every
        // line is already in the channel by then (see `process::Run::poll`). A caller that
        // polled first would still be safe, but this order makes "nothing is lost" obvious.
        let lines = child.drain();
        if !lines.is_empty() {
            stream.ingest(&lines, self.adapter.as_ref());
        }

        let Some(outcome) = child.poll() else {
            return Tick::Streaming;
        };
        let settled = stream.settle(&outcome);
        // Drop the child before the caller draws: an ended run holds no process, and clearing
        // it here is what makes `running()` false from the next keystroke on.
        self.child = None;
        Tick::Finished(settled)
    }

    /// Asks a live run to stop. The outcome arrives through the next [`Machine::tick`].
    ///
    /// `Ctrl+C` mid-run is the `robustness` ticket, so nothing calls this yet; the tests drive
    /// it to prove the call reaches the child.
    #[allow(dead_code)]
    pub fn cancel(&mut self) {
        if let Some(child) = self.child.as_mut() {
            child.cancel();
        }
    }
}

/// The session id an invocation carries, read back out of its argv.
///
/// The adapter generates the id and reports it through the argv rather than caching it, which
/// keeps `build` pure (spec §6.2). So this is where the caller learns it: after `--session-id`
/// for a first call, after `--resume` for a continuation.
fn session_from(invocation: &Invocation) -> Option<String> {
    let at = invocation
        .args
        .iter()
        .position(|arg| arg == "--session-id" || arg == "--resume")?;
    invocation.args.get(at + 1).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Task;
    use crate::harness::claude::Claude;
    use crate::process::Stream as Origin;

    /// A child that replays a scripted sequence of drains and outcomes.
    ///
    /// Scripted rather than threaded so a test reads as the sequence of ticks it asserts on,
    /// and so no test depends on a process starting.
    struct Fake {
        /// Lines to hand out on each successive drain.
        lines: Vec<Vec<String>>,
        /// The next poll result.
        outcome: Option<Outcome>,
        /// How many times `cancel` was called.
        cancels: usize,
    }

    impl Fake {
        fn new(lines: Vec<Vec<String>>, outcome: Option<Outcome>) -> Self {
            Self {
                lines,
                outcome,
                cancels: 0,
            }
        }
    }

    impl Child for Fake {
        fn drain(&mut self) -> Vec<Line> {
            let next = if self.lines.is_empty() {
                Vec::new()
            } else {
                self.lines.remove(0)
            };
            next.into_iter()
                .map(|text| Line {
                    stream: Origin::Stdout,
                    text,
                })
                .collect()
        }

        fn poll(&mut self) -> Option<Outcome> {
            self.outcome.clone()
        }

        fn cancel(&mut self) {
            self.cancels += 1;
        }
    }

    /// An `ExitStatus` cannot be constructed, so a real one comes from a process that exits
    /// immediately. This is the cheapest way to get the type the state machine compares.
    fn exited(code: i32) -> Outcome {
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("exit {code}"))
            .spawn()
            .expect("spawn sh");
        Outcome::Exited(child.wait().expect("wait"))
    }

    /// A child holding a shared fake, so a test can both script it and inspect it.
    struct Scripted {
        inner: std::rc::Rc<std::cell::RefCell<Fake>>,
    }

    impl Child for Scripted {
        fn drain(&mut self) -> Vec<Line> {
            self.inner.borrow_mut().drain()
        }

        fn poll(&mut self) -> Option<Outcome> {
            self.inner.borrow_mut().poll()
        }

        fn cancel(&mut self) {
            self.inner.borrow_mut().cancel()
        }
    }

    /// A machine whose one child is `fake`, plus a handle on that fake.
    fn machine_with(fake: Fake) -> (Machine, std::rc::Rc<std::cell::RefCell<Fake>>) {
        let shared = std::rc::Rc::new(std::cell::RefCell::new(fake));
        let handed = shared.clone();
        let machine = Machine::with_spawner(
            Box::new(Claude),
            Box::new(move |_| {
                Ok(Box::new(Scripted {
                    inner: handed.clone(),
                }) as Box<dyn Child>)
            }),
        );
        (machine, shared)
    }

    fn task<'a>(instructions: &'a str, source: &'a str) -> Task<'a> {
        Task {
            instructions,
            source_text: source,
            model: None,
            session: None,
        }
    }

    fn stream() -> StreamState {
        StreamState::new()
    }

    #[test]
    fn empty_instructions_are_refused_before_anything_is_spawned() {
        let (mut machine, fake) = machine_with(Fake::new(Vec::new(), Some(exited(0))));
        let mut stream = stream();

        let result = machine.start(
            &task("   \n\t ", "some source"),
            true,
            &mut stream,
            Instant::now(),
        );

        assert_eq!(result, Err(Refusal::NoInstructions));
        assert_eq!(Refusal::NoInstructions.message(), "instructions are empty");
        // Nothing ran and the panes were not cleared, because nothing started.
        assert!(!machine.running());
        assert_eq!(stream.output_text(), "");
        // The fake was never polled: a refusal must not touch the child path at all.
        assert_eq!(fake.borrow().cancels, 0);
    }

    #[test]
    fn empty_source_is_refused_with_its_own_reason() {
        let (mut machine, _) = machine_with(Fake::new(Vec::new(), Some(exited(0))));

        let result = machine.start(&task("formal", "  "), true, &mut stream(), Instant::now());

        assert_eq!(result, Err(Refusal::NoSource));
        assert_eq!(Refusal::NoSource.message(), "source text is empty");
        assert!(!machine.running());
    }

    #[test]
    fn a_harness_that_stopped_resolving_is_refused_at_rewrite_time() {
        // The binary resolved at startup and is gone now; §5 step 1 says to refuse rather than
        // spawn and fail.
        let (mut machine, _) = machine_with(Fake::new(Vec::new(), Some(exited(0))));

        let result = machine.start(
            &task("formal", "text"),
            false,
            &mut stream(),
            Instant::now(),
        );

        assert_eq!(result, Err(Refusal::HarnessGone));
        assert!(!machine.running());
    }

    #[test]
    fn a_second_rewrite_while_running_is_refused() {
        let (mut machine, _) = machine_with(Fake::new(Vec::new(), None));
        let mut stream = stream();
        machine
            .start(&task("formal", "text"), true, &mut stream, Instant::now())
            .unwrap();

        let result = machine.start(&task("formal", "text"), true, &mut stream, Instant::now());

        assert_eq!(result, Err(Refusal::AlreadyRunning));
        assert_eq!(
            Refusal::AlreadyRunning.message(),
            "a rewrite is already running"
        );
    }

    #[test]
    fn ctrl_n_reset_is_refused_while_running() {
        let (mut machine, _) = machine_with(Fake::new(Vec::new(), None));
        machine
            .start(&task("formal", "text"), true, &mut stream(), Instant::now())
            .unwrap();

        assert_eq!(machine.reset_session(), Err(Refusal::AlreadyRunning));
    }

    #[test]
    fn ctrl_n_reset_drops_the_session_when_idle() {
        // The run has to finish first: resetting is refused while one is live, which the
        // neighbouring test covers.
        let (mut machine, _) = machine_with(Fake::new(vec![Vec::new()], Some(exited(0))));
        let mut stream = stream();
        machine
            .start(&task("formal", "text"), true, &mut stream, Instant::now())
            .unwrap();
        assert!(machine.session().is_some(), "no session was created");
        machine.tick(&mut stream);
        assert!(!machine.running(), "the fake run did not end");

        machine.reset_session().unwrap();

        assert_eq!(machine.session(), None, "the session survived Ctrl+N");
    }

    #[test]
    fn the_session_id_is_kept_from_the_first_call_and_reused() {
        let (mut machine, _) = machine_with(Fake::new(vec![Vec::new()], Some(exited(0))));
        machine
            .start(&task("formal", "text"), true, &mut stream(), Instant::now())
            .unwrap();
        let first = machine.session().map(str::to_owned).expect("no session");

        // Finish the run, then start another: the id must be the same one.
        machine.tick(&mut stream());
        assert!(!machine.running());
        machine
            .start(&task("formal", "text"), true, &mut stream(), Instant::now())
            .unwrap();

        assert_eq!(machine.session().map(str::to_owned), Some(first));
    }

    #[test]
    fn starting_clears_the_output_pane_and_adds_a_run_line() {
        let (mut machine, _) = machine_with(Fake::new(Vec::new(), None));
        let mut stream = stream();
        // Give the pane something to lose.
        stream.spawn(Instant::now());
        stream.ingest(
            &[Line {
                stream: Origin::Stdout,
                text: r#"{"type":"result","result":"<<<TEXT>>>old<<<END>>>"}"#.to_owned(),
            }],
            &Claude,
        );
        stream.settle(&exited(0));
        assert_eq!(stream.output_text(), "old");

        machine
            .start(&task("formal", "text"), true, &mut stream, Instant::now())
            .unwrap();

        assert_eq!(
            stream.output_text(),
            "",
            "the previous result survived a start"
        );
        assert!(
            stream.debug_text().contains("run started"),
            "no run-start line: {}",
            stream.debug_text()
        );
    }

    #[test]
    fn a_tick_with_no_run_is_idle_and_harmless() {
        let (mut machine, _) = machine_with(Fake::new(Vec::new(), None));

        assert_eq!(machine.tick(&mut stream()), Tick::Idle);
    }

    #[test]
    fn a_tick_while_streaming_reports_streaming_and_routes_the_lines() {
        let (mut machine, _) = machine_with(Fake::new(
            vec![vec![r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}}"#.to_owned()]],
            None,
        ));
        let mut stream = stream();
        machine
            .start(&task("formal", "text"), true, &mut stream, Instant::now())
            .unwrap();

        assert_eq!(machine.tick(&mut stream), Tick::Streaming);

        assert!(
            stream.debug_text().contains("text  "),
            "the streamed line did not reach the debug pane: {}",
            stream.debug_text()
        );
        // The output pane gets nothing during a run.
        assert_eq!(stream.output_text(), "");
        assert!(machine.running(), "a streaming run was ended early");
    }

    #[test]
    fn a_successful_run_reaches_done_and_the_output_pane_gets_the_text() {
        let (mut machine, _) = machine_with(Fake::new(
            vec![vec![
                r#"{"type":"result","result":"<<<TEXT>>>the rewrite<<<END>>>"}"#.to_owned(),
            ]],
            Some(exited(0)),
        ));
        let mut stream = stream();
        machine
            .start(&task("formal", "text"), true, &mut stream, Instant::now())
            .unwrap();

        let tick = machine.tick(&mut stream);

        assert_eq!(tick, Tick::Finished(Settled::Rewritten));
        assert_eq!(stream.output_text(), "the rewrite");
        assert!(!machine.running(), "a finished run is still live");
    }

    #[test]
    fn a_non_zero_exit_reaches_error_and_leaves_the_output_pane_empty() {
        let (mut machine, _) = machine_with(Fake::new(
            vec![vec![
                r#"{"type":"result","result":"<<<TEXT>>>nope<<<END>>>"}"#.to_owned(),
                "boom".to_owned(),
            ]],
            Some(exited(1)),
        ));
        let mut stream = stream();
        machine
            .start(&task("formal", "text"), true, &mut stream, Instant::now())
            .unwrap();

        match machine.tick(&mut stream) {
            Tick::Finished(Settled::Error(reason)) => {
                assert!(reason.contains("non-zero"), "reason was {reason:?}")
            }
            other => panic!("expected an error settle, got {other:?}"),
        }
        assert_eq!(stream.output_text(), "", "a failed run showed its text");
        assert!(!machine.running());
    }

    #[test]
    fn a_cancelled_run_reaches_cancelled_and_clears_the_child() {
        let (mut machine, fake) =
            machine_with(Fake::new(vec![Vec::new()], Some(Outcome::Cancelled)));
        let mut stream = stream();
        machine
            .start(&task("formal", "text"), true, &mut stream, Instant::now())
            .unwrap();

        machine.cancel();
        assert_eq!(fake.borrow().cancels, 1, "cancel did not reach the child");

        assert!(matches!(
            machine.tick(&mut stream),
            Tick::Finished(Settled::Error(_))
        ));
        assert!(!machine.running());
    }

    #[test]
    fn a_timeout_reaches_error_with_the_timeout_named() {
        let (mut machine, _) = machine_with(Fake::new(vec![Vec::new()], Some(Outcome::TimedOut)));
        let mut stream = stream();
        machine
            .start(&task("formal", "text"), true, &mut stream, Instant::now())
            .unwrap();

        let tick = machine.tick(&mut stream);

        assert!(matches!(tick, Tick::Finished(Settled::Error(_))));
        assert!(stream.debug_text().contains("timed out"));
    }

    #[test]
    fn the_machine_is_idle_again_after_every_terminal_outcome() {
        // The four-state round trip the ticket asks for: every ending returns to a startable
        // state rather than leaving the machine wedged.
        for outcome in [exited(0), exited(2), Outcome::Cancelled, Outcome::TimedOut] {
            let (mut machine, _) = machine_with(Fake::new(vec![Vec::new()], Some(outcome)));
            let mut stream = stream();
            machine
                .start(&task("formal", "text"), true, &mut stream, Instant::now())
                .unwrap();
            machine.tick(&mut stream);
            assert!(!machine.running(), "the machine stayed live");

            // And it can start again, which is what "back to idle" has to mean.
            machine
                .start(&task("formal", "text"), true, &mut stream, Instant::now())
                .expect("could not start after a terminal outcome");
        }
    }

    #[test]
    fn a_spawn_failure_is_reported_and_leaves_the_machine_idle() {
        let mut machine = Machine::with_spawner(
            Box::new(Claude),
            Box::new(|_| Err(std::io::Error::other("no such binary"))),
        );
        let mut stream = stream();

        let result = machine.start(&task("formal", "text"), true, &mut stream, Instant::now());

        assert_eq!(result, Err(Refusal::HarnessGone));
        assert!(!machine.running());
        assert!(
            stream.debug_text().contains("could not start"),
            "the spawn failure was not reported: {}",
            stream.debug_text()
        );
    }

    #[test]
    fn the_session_id_is_read_back_out_of_the_argv_it_was_written_into() {
        // The adapter generates the id inside `build` and reports it through the argv, so this
        // is the only place the machine can learn it (spec §6.2).
        let invocation = crate::harness::HarnessAdapter::build(
            &Claude,
            &Task {
                instructions: "formal",
                source_text: "text",
                model: None,
                session: None,
            },
        )
        .unwrap();

        let id = session_from(&invocation).expect("no session id in the argv");
        assert_eq!(invocation.args[1], "--session-id");
        assert_eq!(invocation.args[2], id);
    }

    #[test]
    fn an_invocation_with_no_session_flag_reports_none() {
        let invocation = Invocation {
            program: "claude".to_owned(),
            args: vec!["-p".to_owned()],
            stdin: None,
        };

        assert_eq!(session_from(&invocation), None);
    }

    #[test]
    fn the_default_timeout_is_the_specced_one() {
        // 180 s from spec §7, named once and re-exported so no caller invents its own number.
        assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(180));
    }
}
