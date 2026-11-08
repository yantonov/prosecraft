//! Running the harness and draining its two output streams (spec §7).
//!
//! This layer only moves bytes: it starts the child, reads stdout and stderr on one thread
//! each, and hands back lines labelled with the stream they came from. Nothing here parses,
//! classifies or displays anything. Turning a stdout line into a debug entry is the
//! adapter's job and writing it into a pane is the UI's, which is what keeps this module
//! testable against a fixture script with no harness installed and no terminal attached.
//!
//! The three properties everything else follows from:
//!
//! * **The caller never blocks on the child.** [`Run::drain`] is `try_recv` in a loop and
//!   [`Run::poll`] is `try_wait` plus [`JoinHandle::is_finished`], so one tick costs work
//!   proportional to the lines that have already arrived, never to the lines that have not.
//!   Measured volume is ~280 lines for claude and ~550 for pi per rewrite, which is why
//!   batching per tick is required rather than an optimisation (spec §7).
//! * **The outcome is reported only once both readers have ended, or a short grace has
//!   passed.** A caller that polls and then drains cannot miss the terminal event, which
//!   matters more than it sounds: exit 0 with no terminal event is an error state (spec
//!   §8), so a line lost to that race would turn a good run into a reported failure. The
//!   grace exists for the case where the pipes are held by a process that is not ours (see
//!   *Killing* below): waiting for it is unbounded, and reporting without it would lose the
//!   lines it still has to deliver.
//! * **No reader outlives a normal run.** Both readers end at end of pipe, so a
//!   run-per-keypress app does not accumulate threads.
//!
//! Nothing is written to disk and no state outlives a [`Run`] (spec §1).
//!
//! **Killing.** `Child::kill` raises `SIGKILL` on unix and calls `TerminateProcess` on
//! Windows. There is deliberately no polite-then-forced ladder: on Windows there is no
//! `SIGTERM` to send at all, and on unix a harness that traps `SIGTERM` would need the
//! escalation step anyway, so the ladder would be dead code on one platform and a delay on
//! the other. Neither signal can be trapped, so a child that ignores a polite signal still
//! dies. What this cannot reach is a *grandchild* that inherited the pipes: it survives,
//! keeps the readers alive, and only closes them on its own schedule. Reaching it would take
//! a process group, which is a platform API this crate has no dependency for. That is not
//! hypothetical — measured on Windows, Cygwin's `sh` does not `exec` a native binary, so a
//! shell-shim harness leaves its real process behind — so the design absorbs it instead of
//! assuming it away: after the child is gone the result waits at most [`READER_GRACE`] for
//! the readers, then reports anyway. The orphan keeps running with nobody to read it, which
//! is the harness's own leak, not the app's.
//!
//! **Windows.** Portability is best-effort (spec §2) and the tests drive a `sh` script.

// Nothing outside the tests calls this yet: the tick loop and the pane routing that consume
// it are the `rewrite-flow` and `pane-routing` tickets. The attribute goes away with them.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::harness::Invocation;

/// Wall-clock limit for one run, from spec §7. Exceeded means kill and report
/// [`Outcome::TimedOut`].
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(180);

/// How long the result waits for the readers once the child is gone.
///
/// Normally the readers see end of pipe within microseconds of the child exiting, so this
/// only matters when a surviving grandchild holds the pipes open. It is bounded so a `poll`
/// never becomes an unbounded wait, and short enough that the status line does not visibly
/// lag a kill; the tick loop keeps drawing throughout either way.
pub const READER_GRACE: Duration = Duration::from_millis(500);

/// Most lines one [`Run::drain`] returns.
///
/// A bound exists so a harness stuck in a print loop cannot make a single tick spend an
/// unbounded amount of time and starve the frame; the rest stays queued for the next tick.
/// Measured volume is ~550 lines per run, so a normal run never reaches it.
pub const DRAIN_LIMIT: usize = 1024;

/// Which pipe a line came from (spec §7: both streams reach the debug pane, and they are
/// labelled apart so the user can tell the harness's diagnostics from its event stream).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// One line of harness output, already lossily decoded (spec §7: non-UTF-8 is converted,
/// never an error and never a panic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub stream: Stream,
    /// The line without its terminator. An unterminated last line is still a line: the child
    /// wrote it, and dropping it would truncate the one thing a crash report needs.
    pub text: String,
}

/// How a run ended.
///
/// `TimedOut` and `Cancelled` are distinct because they read differently to the user and
/// because only one of them was asked for. Both produce the same debug-pane evidence, since
/// the lines received before the kill are still in the channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The child exited (and has been reaped). A non-zero status is an error state in the UI
    /// (spec §8), but it is reported here as an ordinary exit: this layer does not decide
    /// what an exit code means.
    Exited(ExitStatus),
    /// The deadline passed; the child was killed.
    TimedOut,
    /// [`Run::cancel`] was called; the child was killed.
    Cancelled,
}

/// A running harness process and its readers.
///
/// Dropping a `Run` that has not finished kills the child first, so an abandoned run cannot
/// leave a harness writing into a pipe nobody reads.
#[derive(Debug)]
pub struct Run {
    child: Child,
    lines: Receiver<Line>,
    /// One per stream, joined once both have ended. Empty only for a finished run.
    readers: Vec<JoinHandle<()>>,
    deadline: Instant,
    /// Set when the child is known to be over — it exited, or this module killed it — but
    /// the readers have not finished draining yet. The reported [`Outcome`] waits on them,
    /// up to [`READER_GRACE`].
    ending: Option<Outcome>,
    /// When `ending` was set. The grace is measured from the kill to the report.
    ending_at: Option<Instant>,
    /// Set once the run has been reported, so `poll` answers the same thing to every later
    /// caller.
    outcome: Option<Outcome>,
}

impl Run {
    /// Starts the child with both pipes, the default timeout, and one reader thread per
    /// stream.
    ///
    /// `Err` is the only case where nothing was started: a spawn failure is reported to the
    /// caller so the UI can name the binary, which is not something that can be recovered
    /// from inside a run.
    pub fn spawn(invocation: &Invocation) -> std::io::Result<Self> {
        Self::spawn_with_timeout(invocation, DEFAULT_TIMEOUT)
    }

    /// [`Run::spawn`] with the wall-clock limit injected, so the timeout path can be tested
    /// in hundreds of milliseconds instead of three minutes.
    pub fn spawn_with_timeout(invocation: &Invocation, timeout: Duration) -> std::io::Result<Self> {
        let mut command = Command::new(&invocation.program);
        // Arguments stay separate elements all the way down (spec §6): a prompt that is one
        // element here is one element in `argv`, whatever it contains.
        command
            .args(&invocation.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;

        let stdin = child.stdin.take().expect("stdin was piped above");
        let stdout = child.stdout.take().expect("stdout was piped above");
        let stderr = child.stderr.take().expect("stderr was piped above");

        match invocation.stdin.clone() {
            // Written on its own thread, because a payload larger than the pipe buffer would
            // block its writer until the child reads, and a harness that never reads stdin
            // would then stall the caller's tick. The thread ends when the child's stdin
            // closes or the write fails; nothing joins it, so it cannot delay a teardown.
            Some(payload) => {
                thread::spawn(move || {
                    let mut stdin = stdin;
                    // A failed write means the child closed the pipe, which is the child's
                    // business: it already has whatever it read, and `std` surfaces the same
                    // condition as a non-zero exit.
                    let _ = stdin.write_all(payload.as_bytes());
                    let _ = stdin.flush();
                });
            }
            // Both harnesses take the prompt as an argument (spec §6), so this is the normal
            // path. The handle must still be dropped: an open write end means the child
            // waits for input that will never come.
            None => drop(stdin),
        }

        let (sender, lines) = mpsc::channel();
        let run = Self {
            child,
            lines,
            readers: vec![
                reader_thread(stdout, Stream::Stdout, sender.clone()),
                reader_thread(stderr, Stream::Stderr, sender.clone()),
            ],
            deadline: Instant::now() + timeout,
            ending: None,
            ending_at: None,
            outcome: None,
        };
        // The run's own sender goes too, so the channel reports `Disconnected` once both
        // readers are done instead of staying open forever.
        drop(sender);
        Ok(run)
    }

    /// Every line that has already arrived, up to [`DRAIN_LIMIT`].
    ///
    /// Never blocks and never waits for a producer: an empty channel returns an empty
    /// vector on the spot. This is the whole "drained per tick, not per line" rule (spec
    /// §7) — the caller drains on its own schedule and a slow harness simply delivers
    /// fewer lines to a tick.
    pub fn drain(&mut self) -> Vec<Line> {
        let mut lines = Vec::new();
        while lines.len() < DRAIN_LIMIT {
            match self.lines.try_recv() {
                Ok(line) => lines.push(line),
                // `Disconnected` means both readers have finished and everything they sent
                // has been received, so there is nothing left to wait for.
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        lines
    }

    /// The outcome, once — the same answer every time it is asked afterwards — or `None`
    /// while the run is still under way. Never blocks.
    ///
    /// `Some` is normally only returned once both reader threads have ended, so everything
    /// the child wrote is already in the channel and the caller's next [`Run::drain`]
    /// collects all of it. That is why polling and draining in either order on a tick is
    /// safe. The exception is a pipe still held [`READER_GRACE`] after the child is gone; see
    /// the module note on killing.
    pub fn poll(&mut self) -> Option<Outcome> {
        if let Some(outcome) = &self.outcome {
            return Some(outcome.clone());
        }

        if self.ending.is_none() {
            match self.child.try_wait() {
                Ok(Some(status)) => self.record_ending(Outcome::Exited(status)),
                // The child is alive; only the deadline can end the run from here.
                Ok(None) if Instant::now() >= self.deadline => self.kill(Outcome::TimedOut),
                // A failing `try_wait` is not reachable for a child this process spawned
                // unless something else reaped it. Leaving the run to the deadline ends it
                // (killed, reported as a timeout) instead of panicking inside a tick.
                Ok(None) | Err(_) => {}
            }
        }

        let ending = self.ending.clone()?;
        if self.readers.iter().all(JoinHandle::is_finished) {
            // Instantaneous: `is_finished` just said both threads have returned.
            for reader in self.readers.drain(..) {
                let _ = reader.join();
            }
        } else {
            // A reader is still in `read` after the child is gone, so something else holds
            // the write end of that pipe. Waiting for it would be an unbounded wait inside a
            // tick; the grace bounds it and then reports regardless, leaving the reader to
            // end whenever its pipe closes.
            let waited = self.ending_at.map(|at| at.elapsed()).unwrap_or_default();
            if waited < READER_GRACE {
                return None;
            }
            self.readers.clear();
        }
        self.outcome = Some(ending.clone());
        Some(ending)
    }

    /// Cancels the run: kills the child and records [`Outcome::Cancelled`], which
    /// [`Run::poll`] then reports.
    ///
    /// A no-op once the run has ended, so cancelling a run that has just exited cannot
    /// overwrite a real exit status with `Cancelled`. The UI keeps the output pane as it was
    /// on spawn because a cancelled run produced no final text and there is nothing to
    /// reconcile (spec §7).
    pub fn cancel(&mut self) {
        if self.outcome.is_some() || self.ending.is_some() {
            return;
        }
        self.kill(Outcome::Cancelled);
        // Usually resolves now, since killing the child closes its pipe ends. If a reader is
        // still a moment behind, the caller's next `poll` reports the same outcome.
        let _ = self.poll();
    }

    /// Kills the child and records `outcome` as the ending to report.
    ///
    /// `wait` cannot hang here: the signal cannot be ignored, so the process is gone or is
    /// already gone, and `wait` only reaps it. Reaping matters — an unreaped child on unix
    /// stays a zombie for the life of the app.
    fn kill(&mut self, outcome: Outcome) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.record_ending(outcome);
    }

    /// Notes that the child is over, starting the grace the readers get to finish.
    fn record_ending(&mut self, outcome: Outcome) {
        self.ending = Some(outcome);
        self.ending_at = Some(Instant::now());
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        // An abandoned run must not keep a harness alive behind a pipe nobody reads.
        if self.outcome.is_none() {
            self.kill(Outcome::Cancelled);
        }
        // Join the readers that have ended so their channel senders are gone, and leave any
        // one still blocked in `read` alone: waiting on it here would make quitting the app
        // wait for a process that is not ours to wait for.
        for reader in self.readers.drain(..) {
            if reader.is_finished() {
                let _ = reader.join();
            }
        }
    }
}

/// One reader thread: splits a pipe into lines and sends each one labelled with `stream`.
fn reader_thread<R: Read + Send + 'static>(
    pipe: R,
    stream: Stream,
    sender: Sender<Line>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut chunk = Vec::new();
        loop {
            chunk.clear();
            match reader.read_until(b'\n', &mut chunk) {
                // End of pipe: the child closed it, or was killed.
                Ok(0) => return,
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                // A read error ends this stream only. Sending what already arrived and
                // stopping beats panicking on a thread whose panic nothing would observe.
                Err(_) => return,
            }
            let line = Line {
                stream,
                text: decode(&chunk),
            };
            // A closed receiver means the run was dropped; there is nobody left to tell.
            if sender.send(line).is_err() {
                return;
            }
        }
    })
}

/// Decodes one raw line, one terminator removed, converting invalid UTF-8 lossily.
///
/// `BufRead::lines` is deliberately not used: it yields `Err` for invalid UTF-8, which would
/// turn one harness's stray byte into a dead reader thread and a truncated run (spec §7).
fn decode(chunk: &[u8]) -> String {
    let mut end = chunk.len();
    if end > 0 && chunk[end - 1] == b'\n' {
        end -= 1;
    }
    // Stripped only immediately before the newline, so the panes never receive a carriage
    // return that would move the cursor to the start of the row mid-text.
    if end > 0 && chunk[end - 1] == b'\r' {
        end -= 1;
    }
    String::from_utf8_lossy(&chunk[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, MutexGuard};

    /// Where a test gives up on a run. Generous: it only has to fail a hang, not measure
    /// anything.
    const PATIENCE: Duration = Duration::from_secs(10);

    /// Serialises the tests that start a process.
    ///
    /// On Windows the fixtures run under Cygwin's `sh`, whose fork emulation fails under
    /// enough concurrent spawns (`couldn't create signal pipe, Win32 error 5`), which made
    /// these tests flake while passing on unix. One test at a time costs a second in total
    /// and makes the platform behave. Poisoning is ignored: a failing test must report its
    /// own failure, not a poisoned mutex in the next one.
    static SPAWN_GUARD: Mutex<()> = Mutex::new(());

    fn serialized() -> MutexGuard<'static, ()> {
        SPAWN_GUARD.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// A fixture script, run through `sh` rather than executed directly.
    ///
    /// The repo targets Linux and macOS first and Windows best-effort (spec §2), and an
    /// executable-bit script is not reliable on Windows, so the program is `sh` and the
    /// script is its single argument. That is also why the runner assumes nothing about the
    /// program being a real binary: it runs whatever `Invocation` names.
    fn invocation(script: &str) -> Invocation {
        let path = fixture_path(script);
        assert!(path.is_file(), "fixture missing: {}", path.display());
        Invocation {
            program: "sh".to_owned(),
            args: vec![path.to_string_lossy().into_owned()],
            stdin: None,
        }
    }

    fn fixture_path(script: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(script)
    }

    /// A one-off `sh -c` program, for cases a checked-in fixture would only obscure.
    fn shell(script: &str) -> Run {
        Run::spawn(&Invocation {
            program: "sh".to_owned(),
            args: vec!["-c".to_owned(), script.to_owned()],
            stdin: None,
        })
        .unwrap_or_else(|err| panic!("spawn sh -c {script:?}: {err}"))
    }

    fn spawn(script: &str) -> Run {
        Run::spawn(&invocation(script)).unwrap_or_else(|err| panic!("spawn {script}: {err}"))
    }

    fn spawn_with_timeout(script: &str, timeout: Duration) -> Run {
        Run::spawn_with_timeout(&invocation(script), timeout)
            .unwrap_or_else(|err| panic!("spawn {script}: {err}"))
    }

    /// Ticks like the UI does until the run ends, and returns the outcome plus every line.
    fn run_to_end(run: &mut Run) -> (Outcome, Vec<Line>) {
        let give_up = Instant::now() + PATIENCE;
        let mut lines = Vec::new();
        loop {
            // Accumulated across ticks: a line that arrived on an earlier tick is still part
            // of the run's output.
            lines.extend(run.drain());
            if let Some(outcome) = run.poll() {
                // The contract: `Some` means the readers have ended, so a drain now finds
                // everything and the channel is closed behind it.
                lines.extend(run.drain());
                assert!(
                    run.readers.is_empty(),
                    "the outcome was reported with a reader still running"
                );
                assert_eq!(
                    run.lines.try_recv(),
                    Err(TryRecvError::Disconnected),
                    "a reader still holds the channel after the outcome"
                );
                return (outcome, lines);
            }
            assert!(Instant::now() < give_up, "the run never ended");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn texts_of(lines: &[Line], stream: Stream) -> Vec<&str> {
        lines
            .iter()
            .filter(|line| line.stream == stream)
            .map(|line| line.text.as_str())
            .collect()
    }

    /// Asserts a normal exit, spelled out rather than built from the code under test.
    fn assert_exited_zero(outcome: &Outcome) {
        match outcome {
            Outcome::Exited(status) => assert_eq!(status.code(), Some(0), "exit status"),
            other => panic!("expected an exit, got {other:?}"),
        }
    }

    #[test]
    fn both_streams_are_captured_line_by_line_with_their_label() {
        let _guard = serialized();
        let mut run = spawn("echo.sh");
        let (outcome, lines) = run_to_end(&mut run);

        assert_exited_zero(&outcome);
        assert_eq!(lines.len(), 3, "captured {lines:?}");
        // Written out rather than derived from the fixture: the fixture and the reader have
        // to agree about what a line is, and a shared constant would hide a drift.
        assert_eq!(texts_of(&lines, Stream::Stdout), ["out one", "out two"]);
        assert_eq!(texts_of(&lines, Stream::Stderr), ["err one"]);
    }

    #[test]
    fn a_non_zero_exit_reports_the_status_and_keeps_both_streams() {
        // The failure mode this covers: reading the exit status first and abandoning the
        // pipes, which loses the stderr line that explains the failure.
        let _guard = serialized();
        let mut run = spawn("nonzero.sh");
        let (outcome, lines) = run_to_end(&mut run);

        let Outcome::Exited(status) = outcome else {
            panic!("expected an exit, got {outcome:?}");
        };
        assert!(!status.success(), "exit 3 must not look successful");
        assert_eq!(status.code(), Some(3));
        assert_eq!(texts_of(&lines, Stream::Stdout), ["out one", "out two"]);
        assert_eq!(texts_of(&lines, Stream::Stderr), ["err one"]);
    }

    #[test]
    fn invalid_utf8_is_converted_lossily_and_the_reader_keeps_going() {
        // 0xff 0xfe are not UTF-8 at all; each becomes one replacement character. The line
        // after them is the real assertion: a reader that died on the bad bytes would lose
        // it.
        let _guard = serialized();
        let mut run = spawn("lossy.sh");
        let (outcome, lines) = run_to_end(&mut run);

        assert_exited_zero(&outcome);
        assert_eq!(
            texts_of(&lines, Stream::Stdout),
            ["bad \u{fffd}\u{fffd} bytes", "after"]
        );
    }

    #[test]
    fn a_run_that_never_exits_is_killed_at_the_timeout_and_cannot_be_stopped_politely() {
        // The fixture traps SIGTERM and SIGINT and spins. Nothing here sends either: the
        // kill is SIGKILL (unix) / TerminateProcess (Windows), and neither can be trapped,
        // so a child that ignores a polite signal dies anyway. The run ending at all is the
        // proof that the kill works, and the elapsed bound is only there to fail a hang.
        let _guard = serialized();
        let injected = Duration::from_millis(200);
        let mut run = spawn_with_timeout("hangs.sh", injected);

        let started = Instant::now();
        let (outcome, lines) = run_to_end(&mut run);
        let elapsed = started.elapsed();

        assert_eq!(outcome, Outcome::TimedOut);
        assert!(
            lines.is_empty(),
            "a spinning child wrote nothing: {lines:?}"
        );
        // The injected limit is honoured, not the 180 s default. Generous both ways so a
        // loaded machine cannot fail the test on timing alone.
        assert!(
            elapsed >= injected,
            "killed at {elapsed:?}, before the {injected:?} deadline"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "the child survived the kill: {elapsed:?}"
        );
    }

    #[test]
    fn cancellation_kills_the_child_and_reports_cancelled() {
        let _guard = serialized();
        let mut run = spawn_with_timeout("hangs.sh", PATIENCE);

        // Long enough for the readers to have started on the pipes.
        thread::sleep(Duration::from_millis(50));
        let started = Instant::now();
        run.cancel();

        let (outcome, lines) = run_to_end(&mut run);
        assert_eq!(outcome, Outcome::Cancelled);
        assert!(lines.is_empty(), "{lines:?}");
        // No waiting for the 10 s deadline: the kill is what ended it.
        assert!(started.elapsed() < Duration::from_secs(2));

        // Idempotent, and it does not overwrite the recorded outcome with a second one.
        run.cancel();
        assert_eq!(run.poll(), Some(Outcome::Cancelled));
    }

    #[test]
    fn cancelling_after_the_exit_does_not_replace_the_exit_status() {
        // A cancel that loses the race with the exit must leave the real status visible, or
        // a successful run would be reported as cancelled.
        let _guard = serialized();
        let mut run = spawn("echo.sh");
        let (outcome, lines) = run_to_end(&mut run);
        assert_exited_zero(&outcome);
        assert_eq!(texts_of(&lines, Stream::Stdout), ["out one", "out two"]);

        run.cancel();
        let after = run.poll().expect("the outcome is still reported");
        assert_exited_zero(&after);
    }

    #[test]
    fn poll_is_none_while_the_child_runs_and_answers_the_same_every_time_after() {
        let _guard = serialized();
        let mut run = spawn_with_timeout("hangs.sh", PATIENCE);
        assert_eq!(run.poll(), None, "a sleeping child has not finished");

        run.cancel();
        let (outcome, _) = run_to_end(&mut run);
        assert_eq!(outcome, Outcome::Cancelled);
        // A second ask must answer, or the UI would report nothing after the first tick
        // that happened to see the end.
        assert_eq!(run.poll(), Some(Outcome::Cancelled));
    }

    #[test]
    fn draining_a_slow_producer_returns_at_once_and_line_by_line() {
        // slow.sh writes "first", sleeps 300 ms, writes "second". A drain that waited for
        // the producer would show both lines together at the end; the assertions are that
        // the first arrives while the run is still going and that no tick blocks.
        let _guard = serialized();
        let mut run = spawn("slow.sh");
        let give_up = Instant::now() + PATIENCE;

        let started = Instant::now();
        let mut first_seen: Option<Duration> = None;
        let mut second_seen: Option<Duration> = None;
        let mut slowest_tick = Duration::ZERO;
        let mut ticks = 0;
        loop {
            // One tick: drain, then ask whether the run ended. Neither call may block.
            let tick_started = Instant::now();
            let lines = run.drain();
            let outcome = run.poll();
            slowest_tick = slowest_tick.max(tick_started.elapsed());
            ticks += 1;
            for line in &lines {
                match line.text.as_str() {
                    "first" => first_seen.get_or_insert_with(|| started.elapsed()),
                    "second" => second_seen.get_or_insert_with(|| started.elapsed()),
                    other => panic!("unexpected line {other:?}"),
                };
            }
            if outcome.is_some() {
                break;
            }
            assert!(Instant::now() < give_up, "the run never ended");
            thread::sleep(Duration::from_millis(2));
        }

        let first_seen = first_seen.expect("first line was never drained");
        let second_seen = second_seen.expect("second line was never drained");
        assert!(
            first_seen < Duration::from_millis(150),
            "first line took {first_seen:?}: the tick waited for the producer"
        );
        assert!(
            second_seen > first_seen,
            "both lines arrived on one tick, so draining waited for the producer"
        );
        // A tick over an empty channel is microseconds. The bound leaves room for a loaded
        // machine while still failing anything that waits on the child.
        assert!(
            slowest_tick < Duration::from_millis(100),
            "a tick took {slowest_tick:?}, so draining blocked"
        );
        // More than one tick really happened, or the two timings above would be one event.
        assert!(ticks > 10, "only {ticks} ticks");
    }

    #[test]
    fn reader_threads_end_with_the_child_and_no_sender_survives_repeated_runs() {
        // The leak this guards against: a reader left running keeps the channel open, and a
        // run-per-keypress app would accumulate one thread per rewrite. `run_to_end` already
        // asserts the channel is disconnected and the handles are joined, so repeating it is
        // the whole test.
        let _guard = serialized();
        for cycle in 0..5 {
            let mut run = spawn("echo.sh");
            let (outcome, lines) = run_to_end(&mut run);

            assert_exited_zero(&outcome);
            assert_eq!(
                texts_of(&lines, Stream::Stdout),
                ["out one", "out two"],
                "cycle {cycle}"
            );
            assert_eq!(
                texts_of(&lines, Stream::Stderr),
                ["err one"],
                "cycle {cycle}"
            );
        }
    }

    #[test]
    fn an_unterminated_last_line_is_still_delivered() {
        let _guard = serialized();
        let mut run = shell("printf 'no newline at end'");

        let (outcome, lines) = run_to_end(&mut run);
        assert_exited_zero(&outcome);
        assert_eq!(texts_of(&lines, Stream::Stdout), ["no newline at end"]);
    }

    #[test]
    fn stdin_payload_is_written_to_the_child() {
        // Not a harness path (both harnesses take the prompt as an argument, spec §6), but
        // `Invocation` carries a stdin field and a caller must not be able to hang a tick
        // writing it.
        let _guard = serialized();
        let mut run = Run::spawn(&Invocation {
            program: "sh".to_owned(),
            args: vec!["-c".to_owned(), "cat; echo done >&2".to_owned()],
            stdin: Some("two words\nand a line\n".to_owned()),
        })
        .expect("spawn sh -c cat");

        let (outcome, lines) = run_to_end(&mut run);
        assert_exited_zero(&outcome);
        assert_eq!(
            texts_of(&lines, Stream::Stdout),
            ["two words", "and a line"]
        );
        assert_eq!(texts_of(&lines, Stream::Stderr), ["done"]);
    }

    #[test]
    fn closing_stdin_when_there_is_no_payload_lets_the_child_finish() {
        // An inherited stdin handle would leave `cat` waiting for input forever; the run has
        // to see the pipe closed and exit.
        let _guard = serialized();
        let mut run = shell("cat; echo after");

        let (outcome, lines) = run_to_end(&mut run);
        assert_exited_zero(&outcome);
        assert_eq!(texts_of(&lines, Stream::Stdout), ["after"]);
    }

    #[test]
    fn a_missing_program_is_a_spawn_error_not_a_panic() {
        let _guard = serialized();
        let err = Run::spawn(&Invocation {
            program: "prosecraft-no-such-harness".to_owned(),
            args: Vec::new(),
            stdin: None,
        })
        .expect_err("a missing binary must not spawn");

        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn dropping_an_unfinished_run_kills_the_child() {
        // The drop path is what stops a quit from leaving a harness running, and the pid is
        // the only place that can be observed from outside.
        let _guard = serialized();
        let run = spawn_with_timeout("hangs.sh", PATIENCE);
        let pid = run.child.id();
        assert!(pid_is_alive(pid), "the fixture is not running");

        drop(run);

        assert!(!pid_is_alive(pid), "the child outlived the run");
    }

    /// A successful `kill -0` means the pid exists and is signallable.
    #[cfg(unix)]
    fn pid_is_alive(pid: u32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    /// Windows has no `kill -0`; `tasklist` filtered by pid is the equivalent existence check.
    #[cfg(windows)]
    fn pid_is_alive(pid: u32) -> bool {
        match Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
        {
            Ok(out) => String::from_utf8_lossy(&out.stdout).contains(&pid.to_string()),
            Err(_) => false,
        }
    }

    #[test]
    fn decode_removes_one_terminator_and_decodes_lossily() {
        assert_eq!(decode(b"plain\n"), "plain");
        // CRLF is a Windows line ending; the carriage return is not part of the line.
        assert_eq!(decode(b"crlf\r\n"), "crlf");
        // Invalid bytes become replacement characters rather than failing the line.
        assert_eq!(decode(b"bad \xff tail\n"), "bad \u{fffd} tail");
        // An unterminated final line keeps everything before the bytes are decoded.
        assert_eq!(decode(b"no terminator"), "no terminator");
        // A carriage return that is not before a newline is content.
        assert_eq!(decode(b"a\rb\n"), "a\rb");
        assert_eq!(decode(b""), "");
    }
}
