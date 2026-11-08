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

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::harness::{HarnessAdapter, Invocation, Task, Transport};
use crate::panes::{Settled, StreamState};
use crate::process::{self, Line, Outcome};

/// The wall-clock limit for one run, re-exported so callers name it in one place (spec §7).
///
/// Applied by [`Machine`], and injectable so a test does not have to wait it out.
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
    /// Asks the child to stop. Not required to be immediate: the outcome arrives through the
    /// next [`Child::poll`], which is what makes a cancel look like any other ending.
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
pub type Spawner = Box<dyn Fn(&Invocation, Duration) -> std::io::Result<Box<dyn Child>>>;

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
    /// The program to start, exactly as the startup resolver returned it.
    ///
    /// The adapter names its harness (`pi`), which is the right *name* and the wrong
    /// *program*: on Windows `Command::new("pi")` fails with "program not found" when the
    /// installation is `pi.cmd` or a bare `pi.exe`, because `CreateProcess` appends only
    /// `.exe`. Holding the resolved path here is what makes the startup check and the spawn
    /// the same answer rather than two guesses (spec §3).
    program: PathBuf,
    /// Whether that program is a Windows batch shim, which cannot carry the prompt in argv.
    /// Decided once by the resolver and passed along with the program, so the two facts about
    /// the same file can never drift apart (spec §6).
    batch: bool,
    spawner: Spawner,
    /// Wall-clock limit for each run. A field rather than a constant so a test can pass
    /// hundreds of milliseconds instead of waiting out the 180 s default (spec §7).
    timeout: Duration,
}

impl std::fmt::Debug for Machine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Machine")
            .field("running", &self.child.is_some())
            .field("session", &self.session)
            .field("harness", &self.adapter.name())
            .field("program", &self.program)
            .field("batch", &self.batch)
            .finish()
    }
}

impl Machine {
    /// A machine that spawns real processes through [`process::Run`].
    ///
    /// `program` is the resolver's answer from startup: the concrete file to start, not the
    /// harness's name. Passing it in keeps this module free of any platform lookup rule.
    pub fn new(adapter: Box<dyn HarnessAdapter>, program: PathBuf, batch: bool) -> Self {
        Self {
            child: None,
            session: None,
            adapter,
            program,
            batch,
            spawner: Box::new(|invocation, timeout| {
                process::Run::spawn_with_timeout(invocation, timeout)
                    .map(|run| Box::new(run) as Box<dyn Child>)
            }),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// The wall-clock limit applied to each run started from here.
    ///
    /// The timeout is a property of the machine rather than a constant inside the runner
    /// because the ticket that tests it must not wait three minutes: injecting it here keeps
    /// the production path and the tested path the same code.
    ///
    /// Test-only, and honestly marked: there is no `--timeout` flag, so nothing in `main`
    /// needs it. It stays a normal method rather than a test-only constructor because the
    /// timeout tests drive the production `start` path, and a `#[cfg(test)]` builder would
    /// change the shape of the machine they are testing.
    #[cfg(test)]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// A machine that spawns whatever `spawner` returns, for tests.
    ///
    /// The program is the adapter's own name: no test here reaches the OS, so there is
    /// nothing for a platform rule to decide.
    #[cfg(test)]
    pub fn with_spawner(adapter: Box<dyn HarnessAdapter>, spawner: Spawner) -> Self {
        let program = PathBuf::from(adapter.name());
        Self {
            child: None,
            session: None,
            adapter,
            program,
            batch: false,
            spawner,
            timeout: DEFAULT_TIMEOUT,
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
        // The session flag comes from the machine, never from the caller: `None` on the first
        // call of the run, so the adapter generates an id and reports it through the argv
        // (spec §6.2), and the stored id on every later call, so the adapter sends `--resume`.
        //
        // Built here rather than trusting `task.session`, because a caller passing `None` every
        // time would open a fresh session on every rewrite while the status line showed a
        // stable id — the stored id would be decoration. This is what makes `--resume` reach
        // the argv at all.
        let built = Task {
            session: self.session.as_deref(),
            // The machine owns this rather than the caller: it is a fact about the resolved
            // program, and the caller has no resolution to consult (spec §6).
            transport: if self.batch {
                Transport::Stdin
            } else {
                Transport::Argv
            },
            ..*task
        };
        let mut invocation = self
            .adapter
            .build(&built)
            .map_err(|_| Refusal::HarnessGone)?;
        // The adapter supplies the harness *name*; the program that is actually started is the
        // one the startup resolver chose. Overriding it here, at the single place an
        // invocation becomes a process, is what makes "found at startup" and "runnable now"
        // the same statement (spec §3).
        invocation.program = self.program.to_string_lossy().into_owned();

        let child = match (self.spawner)(&invocation, self.timeout) {
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
    /// `Ctrl+C` mid-run: ask the live run to stop (spec §5, §7).
    ///
    /// Returns whether anything was running, so the caller can tell a cancel from a no-op
    /// without asking twice.
    pub fn cancel(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => {
                child.cancel();
                true
            }
            None => false,
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
            Box::new(move |_, _| {
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
            transport: Transport::Argv,
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
    fn the_second_rewrite_resumes_the_session_the_first_one_created() {
        // The bug this pins: the machine stored the id and reported it, but built the
        // invocation from the caller's *task*, and every caller passed `session: None`. So
        // each rewrite opened a new session, `--resume` never reached the argv, and the id on
        // the status line was decoration.
        //
        // Asserted on the argv rather than on `session()`, because the stored id was already
        // correct when the bug was live: only the flags the adapter was asked for can tell a
        // resume from a second creation.
        let seen: std::rc::Rc<std::cell::RefCell<Vec<Vec<String>>>> = Default::default();
        let recorder = seen.clone();
        let mut machine = Machine::with_spawner(
            Box::new(Claude),
            Box::new(move |invocation, _| {
                recorder.borrow_mut().push(invocation.args.clone());
                Ok(Box::new(Scripted {
                    inner: std::rc::Rc::new(std::cell::RefCell::new(Fake::new(
                        vec![Vec::new(), Vec::new()],
                        Some(exited(0)),
                    ))),
                }) as Box<dyn Child>)
            }),
        );
        let mut stream = stream();

        machine
            .start(&task("formal", "text"), true, &mut stream, Instant::now())
            .unwrap();
        let first = machine.session().map(str::to_owned).expect("no session");
        machine.tick(&mut stream);

        machine
            .start(&task("formal", "text"), true, &mut stream, Instant::now())
            .unwrap();

        let seen = seen.borrow();
        assert_eq!(seen.len(), 2, "expected two invocations");

        // First call creates; second resumes. Both the absence and the presence matter.
        assert!(
            seen[0].iter().any(|arg| arg == "--session-id"),
            "the first call did not create a session: {:?}",
            seen[0]
        );
        assert!(
            seen[1].iter().any(|arg| arg == "--resume"),
            "the second call did not resume: {:?}",
            seen[1]
        );
        assert!(
            !seen[1].iter().any(|arg| arg == "--session-id"),
            "the second call opened another session: {:?}",
            seen[1]
        );

        let resumed = seen[1]
            .iter()
            .position(|arg| arg == "--resume")
            .map(|at| seen[1][at + 1].clone());
        assert_eq!(
            resumed.as_deref(),
            Some(first.as_str()),
            "the second call resumed a different id"
        );
    }

    #[test]
    fn a_rewrite_after_ctrl_n_creates_a_session_again() {
        // The other half of the lifecycle: dropping the id must put the next call back on
        // `--session-id`, or Ctrl+N would look like it worked while resuming the old session.
        let seen: std::rc::Rc<std::cell::RefCell<Vec<Vec<String>>>> = Default::default();
        let recorder = seen.clone();
        let mut machine = Machine::with_spawner(
            Box::new(Claude),
            Box::new(move |invocation, _| {
                recorder.borrow_mut().push(invocation.args.clone());
                Ok(Box::new(Scripted {
                    inner: std::rc::Rc::new(std::cell::RefCell::new(Fake::new(
                        vec![Vec::new(), Vec::new()],
                        Some(exited(0)),
                    ))),
                }) as Box<dyn Child>)
            }),
        );
        let mut stream = stream();

        machine
            .start(&task("formal", "text"), true, &mut stream, Instant::now())
            .unwrap();
        let first = machine.session().map(str::to_owned).expect("no session");
        machine.tick(&mut stream);

        machine.reset_session().unwrap();
        machine
            .start(&task("formal", "text"), true, &mut stream, Instant::now())
            .unwrap();
        let second = machine.session().map(str::to_owned).expect("no session");

        let seen = seen.borrow();
        assert!(
            seen[1].iter().any(|arg| arg == "--session-id"),
            "a rewrite after Ctrl+N resumed instead of creating: {:?}",
            seen[1]
        );
        assert_ne!(first, second, "Ctrl+N did not produce a fresh id");
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
            Box::new(|_, _| Err(std::io::Error::other("no such binary"))),
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
    fn the_spawned_program_is_the_resolved_path_not_the_harness_name() {
        // The defect this pins down: the adapter names its harness (`claude`) and the spawn
        // used that name. On Windows `Command::new("claude")` fails with "program not found"
        // when the installation is `claude.exe`, so the startup check accepted a binary the
        // spawn could not start. The machine now starts the resolver's answer instead.
        let recorded = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let handed = recorded.clone();
        let mut machine = Machine::new(
            Box::new(Claude),
            PathBuf::from("/opt/bin/claude.exe"),
            false,
        );
        machine.spawner = Box::new(move |invocation, _| {
            handed.borrow_mut().push(invocation.program.clone());
            Ok(Box::new(Scripted {
                inner: std::rc::Rc::new(std::cell::RefCell::new(Fake::new(
                    Vec::new(),
                    Some(exited(0)),
                ))),
            }) as Box<dyn Child>)
        });

        machine
            .start(&task("formal", "text"), true, &mut stream(), Instant::now())
            .expect("the machine refused a valid task");

        assert_eq!(
            recorded.borrow().as_slice(),
            ["/opt/bin/claude.exe".to_owned()],
            "the adapter's bare name reached the spawner instead of the resolved path"
        );
        // And the argv is still the adapter's: only the program was overridden.
        assert_eq!(Claude.name(), "claude");
    }

    #[test]
    fn a_batch_harness_gets_the_prompt_on_stdin_and_an_exe_gets_it_in_argv() {
        // The transport is a fact about the resolved program, so the machine decides it and
        // the caller cannot pass the wrong one. Both halves are asserted, because "always
        // stdin" would silently break every unix install.
        fn program_and_stdin(batch: bool) -> (Vec<String>, Option<String>) {
            let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let captured = std::rc::Rc::new(std::cell::RefCell::new(None));
            let handed_seen = seen.clone();
            let handed_captured = captured.clone();
            let mut machine = Machine::new(
                Box::new(crate::harness::pi::Pi),
                PathBuf::from("pi.cmd"),
                batch,
            );
            machine.spawner = Box::new(move |invocation, _| {
                *handed_seen.borrow_mut() = invocation.args.clone();
                *handed_captured.borrow_mut() = invocation.stdin.clone();
                Ok(Box::new(Scripted {
                    inner: std::rc::Rc::new(std::cell::RefCell::new(Fake::new(
                        Vec::new(),
                        Some(exited(0)),
                    ))),
                }) as Box<dyn Child>)
            });
            machine
                .start(&task("formal", "text"), true, &mut stream(), Instant::now())
                .expect("the machine refused a valid task");
            let args = seen.borrow().clone();
            let stdin = captured.borrow().clone();
            (args, stdin)
        }

        let (shim_args, shim_stdin) = program_and_stdin(true);
        assert!(
            shim_stdin.is_some(),
            "a batch shim must get the prompt on stdin"
        );
        assert!(
            !shim_args.iter().any(|arg| arg == "--"),
            "the shim argv must not carry the separator: {shim_args:?}"
        );
        assert!(
            !shim_args.iter().any(|arg| arg == "--system-prompt"),
            "the instructions cannot travel in argv on a shim: {shim_args:?}"
        );

        let (exe_args, exe_stdin) = program_and_stdin(false);
        assert_eq!(exe_stdin, None, "an .exe must not get the prompt on stdin");
        assert!(
            exe_args.iter().any(|arg| arg == "--"),
            "the argv form keeps the separator: {exe_args:?}"
        );
        assert!(exe_args.iter().any(|arg| arg == "--system-prompt"));
        assert!(
            exe_args
                .last()
                .is_some_and(|arg| arg.starts_with("INSTRUCTIONS:")),
            "an .exe carries the prompt as its last argument: {exe_args:?}"
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
                transport: Transport::Argv,
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
    fn a_run_that_never_ends_is_killed_at_the_injected_timeout() {
        // The real runner, not a fake: the timeout is the one behaviour that only a genuine
        // child can prove, because it is the runner that has to notice the deadline. The
        // fixture spins on builtins, so nothing outlives the kill and the test costs ~200ms
        // rather than the 180 s default.
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hangs.sh");
        assert!(fixture.is_file(), "fixture missing: {}", fixture.display());

        let spawned = std::cell::RefCell::new(Some(Invocation {
            program: "sh".to_owned(),
            args: vec![fixture.to_string_lossy().into_owned()],
            stdin: None,
        }));
        // Drive the real spawner with the fixture's argv rather than the adapter's, so the
        // test does not depend on `claude` being installed.
        let machine_spawner = std::rc::Rc::new(spawned);
        let mut machine = Machine::with_spawner(Box::new(Claude), {
            let argv = machine_spawner.clone();
            Box::new(move |_invocation, timeout| {
                let invocation = argv
                    .borrow_mut()
                    .take()
                    .unwrap_or_else(|| panic!("the fixture should be spawned once"));
                process::Run::spawn_with_timeout(&invocation, timeout)
                    .map(|run| Box::new(run) as Box<dyn Child>)
            })
        })
        .with_timeout(Duration::from_millis(200));

        let mut stream = stream();
        let started = Instant::now();
        machine
            .start(&task("formal", "text"), true, &mut stream, Instant::now())
            .expect("the fixture should start");

        // Tick until the run ends, bounded so a hang fails rather than stalls.
        let mut ended = false;
        while started.elapsed() < Duration::from_secs(10) {
            if let Tick::Finished(_) = machine.tick(&mut stream) {
                ended = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(ended, "the run was never killed");
        assert!(
            started.elapsed() >= Duration::from_millis(200),
            "killed before the deadline: {:?}",
            started.elapsed()
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the injected timeout was not honoured: {:?}",
            started.elapsed()
        );
        assert!(
            stream.debug_text().contains("timed out"),
            "the timeout was not reported: {}",
            stream.debug_text()
        );
    }

    #[test]
    fn the_default_timeout_is_the_specced_one() {
        // 180 s from spec §7, named once and re-exported so no caller invents its own number.
        assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(180));
    }
}
