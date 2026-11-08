//! Startup: parse the command line, validate it, then take over the terminal.
//!
//! The split matters (spec §3): clap runs first so `--help`/`--version` work with no
//! harness installed, the harness binary is checked next, the `--*-file` reads follow,
//! and only then is raw mode entered. Nothing in [`cli`] touches the terminal.

use std::io::{self, Stdout, Write};
use std::panic;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::cursor;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use cli::Config;
use layout::Focus;
use ui::Outcome;

mod cli;
mod clipboard;
mod editor;
mod harness;
mod layout;
mod marker;
mod panes;
mod process;
mod rewrite;
mod ui;
mod wrap;

fn main() -> ExitCode {
    // `parse` is the exit-0 path for -h/-V and the exit-2 path for CLI misuse; both
    // happen here, before the harness check, which is what makes them work uninstalled.
    let args = cli::parse();

    // `System` connects lazily and `assemble` skips the read entirely under `--no-clipboard`,
    // so this opens no display connection unless the source text actually comes from it.
    let config = match cli::assemble(
        args,
        cli::resolve_harness,
        &mut crate::clipboard::System::new(),
    ) {
        Ok(config) => config,
        Err(err) => {
            // One line on stderr. No terminal state was touched yet, so the user's shell
            // is still usable when the app refuses to start.
            eprintln!("{err}");
            return ExitCode::from(err.exit_code());
        }
    };

    match Tui::start(config).and_then(|mut tui| tui.run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("prosecraft: {err}");
            ExitCode::FAILURE
        }
    }
}

struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    /// Assembled at startup and immutable for the run, per spec §3; the pane tickets
    /// draw from it.
    config: Config,
    /// UI state, not terminal state: the four-pane layout and its focus index.
    focus: Focus,
}

impl Tui {
    fn start(config: Config) -> io::Result<Self> {
        // The hook must be in place before we touch the terminal, otherwise a panic
        // during setup (or the first draw) would strand the user in raw mode.
        install_panic_hook();

        terminal::enable_raw_mode()?;
        match Self::enter_alternate_screen(config) {
            Ok(tui) => Ok(tui),
            // Raw mode is already on, and the failed value never reaches a Drop guard.
            Err(err) => {
                restore_terminal();
                Err(err)
            }
        }
    }

    fn enter_alternate_screen(config: Config) -> io::Result<Self> {
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        Ok(Self {
            terminal: Terminal::new(CrosstermBackend::new(stdout))?,
            config,
            focus: Focus::Source,
        })
    }

    fn run(&mut self) -> io::Result<()> {
        let Self {
            terminal,
            config,
            focus,
        } = self;

        let mut view = ui::View::new(config, *focus);
        // The real clipboard and the real harness adapter for the running app; tests build a
        // view with the recording clipboard and a scripted child instead.
        view.clipboard = Box::new(crate::clipboard::System::new());
        view.machine = Some(Box::new(rewrite::Machine::new(
            adapter_for(config.harness),
            config.harness_program.clone(),
        )));

        // Ticking rather than blocking: a live rewrite delivers lines on the child's threads,
        // and nothing can arrive if the loop is parked in `event::read` until the next key
        // (spec §6.1b: drained once per tick, ~16 ms).
        tick_loop(
            &mut view,
            |view| {
                terminal.draw(|frame| ui::render(frame, view))?;
                Ok(())
            },
            |view, timeout| {
                if !event::poll(timeout)? {
                    return Ok(Outcome::Continue);
                }
                let outcome = match event::read()? {
                    Event::Key(key) => view.handle_key(&key),
                    // Resize and every other event leave the state alone; there is still a
                    // frame to draw for them, so they are not a quit.
                    _ => Outcome::Continue,
                };
                Ok(outcome)
            },
            |view| view.tick(),
        )
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// The adapter for the harness chosen on the command line.
///
/// `--harness` selects a binary, and each adapter is that binary's argv and event vocabulary.
/// The two are not interchangeable: claude resumes with `--resume` while pi keeps
/// `--session-id`, and their event vocabularies share no type names.
fn adapter_for(harness: cli::Harness) -> Box<dyn harness::HarnessAdapter> {
    match harness {
        cli::Harness::Claude => Box::new(harness::claude::Claude),
        cli::Harness::Pi => Box::new(harness::pi::Pi),
    }
}

/// How long the loop waits for input before ticking again.
///
/// ~16 ms is one frame, which is what makes a running rewrite look live (spec §6.1b: events
/// are drained once per tick and rendered once per tick).
const TICK: std::time::Duration = std::time::Duration::from_millis(16);

/// Draws, ticks, then waits at most one tick for input, until the view asks to quit.
///
/// Different from a blocking read loop in the one way that matters: it never parks
/// indefinitely, so a run producing lines gets drained whether or not the user is typing.
///
/// The draw is driven by the tick, not by the event: a frame is drawn when a tick reports that
/// something changed, so an idle app draws nothing and a streaming run is not redrawn once per
/// arriving line (spec §6.1b).
fn tick_loop<S>(
    state: &mut S,
    mut draw: impl FnMut(&mut S) -> io::Result<()>,
    mut next: impl FnMut(&mut S, std::time::Duration) -> io::Result<Outcome>,
    mut tick: impl FnMut(&mut S) -> bool,
) -> io::Result<()> {
    // The first frame, so the panes appear before the user touches anything.
    draw(state)?;
    loop {
        // A key gets one tick of budget to arrive, so typing feels immediate; whatever a live
        // run produced in that window is drained on the following pass.
        if next(state, TICK)? == Outcome::Quit {
            return Ok(());
        }
        // One draw per change, not one per event and not one per tick: an idle app draws
        // nothing, and a streaming run draws once however many lines arrived (spec §6.1b).
        if tick(state) {
            draw(state)?;
        }
    }
}

/// Restoration runs once: the panic hook and `Tui`'s drop both call it, and repeating
/// the escape sequences would print them after the panic message on the normal screen.
static TERMINAL_RESTORED: AtomicBool = AtomicBool::new(false);

fn restore_terminal() {
    if !claim_restore(&TERMINAL_RESTORED) {
        return;
    }
    // Each step is best-effort: a failing one must not stop the remaining ones, and
    // there is no useful way to report an error from a drop or a panic hook.
    let _ = terminal::disable_raw_mode();
    let _ = write_restore(&mut io::stdout());
}

/// Returns `true` only for the first caller, so the escape sequences are emitted once.
fn claim_restore(flag: &AtomicBool) -> bool {
    !flag.swap(true, Ordering::SeqCst)
}

fn write_restore(out: &mut impl Write) -> io::Result<()> {
    execute!(out, LeaveAlternateScreen, cursor::Show)
}

fn install_panic_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        // Leave the alternate screen first so the panic message lands in scrollback.
        restore_terminal();
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn key(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> Event {
        Event::Key(KeyEvent::new_with_kind(code, modifiers, kind))
    }

    fn view() -> ui::View<'static> {
        // Leaked so the view can outlive the temporary `Config`: these tests only need a
        // config to draw from, and a leak in a test binary costs nothing.
        let config = Box::leak(Box::new(Config {
            harness: cli::Harness::Claude,
            harness_program: std::path::PathBuf::from("stub-harness"),
            model: None,
            session_id: None,
            initial_source: String::new(),
            initial_instructions: String::new(),
        }));
        ui::View::new(config, Focus::Source)
    }

    /// The real `Tui::run` event source: a key is decoded by the view, everything else
    /// continues. Repeating it here keeps the loop test honest about which keys quit.
    fn outcome_of(view: &mut ui::View, event: &Event) -> Outcome {
        match event {
            Event::Key(key) => view.handle_key(key),
            _ => Outcome::Continue,
        }
    }

    /// A key must reach the screen: `tick_loop` draws only when a tick asks, so a key applied
    /// without being reported is invisible. Driven through the loop rather than by calling
    /// `tick` directly, because that is where `handle_key` and `tick` meet.
    #[test]
    fn loop_draws_after_a_key_that_changes_the_view() {
        let mut events = vec![
            key(KeyCode::Char('h'), KeyModifiers::NONE, KeyEventKind::Press),
            key(
                KeyCode::Char('q'),
                KeyModifiers::CONTROL,
                KeyEventKind::Press,
            ),
        ]
        .into_iter();

        let mut view = view();
        let mut draws = 0;
        let result = tick_loop(
            &mut view,
            |_| {
                draws += 1;
                Ok(())
            },
            |view, _| {
                let event = events.next().expect("tick_loop over-read its event source");
                Ok(outcome_of(view, &event))
            },
            |view| view.tick(),
        );

        assert!(result.is_ok());
        assert_eq!(view.source.text(), "h", "the key was not applied");
        // The first frame, plus one for the typed character.
        assert_eq!(draws, 2, "a keystroke was applied and never drawn");
    }

    #[test]
    fn loop_keeps_running_until_a_press() {
        let mut events = vec![
            Event::Resize(80, 24),
            key(
                KeyCode::Char('q'),
                KeyModifiers::CONTROL,
                KeyEventKind::Release,
            ),
            key(KeyCode::Char('q'), KeyModifiers::NONE, KeyEventKind::Press),
        ]
        .into_iter();

        let mut view = view();
        let mut draws = 0;
        let result = tick_loop(
            &mut view,
            |_| {
                draws += 1;
                Ok(())
            },
            |view, _| {
                let event = events.next().expect("tick_loop over-read its event source");
                Ok(outcome_of(view, &event))
            },
            |_| false,
        );

        // Ok here is the normal-quit path: `main` then returns success and `Tui` is dropped,
        // which is where the terminal is restored.
        assert!(result.is_ok());
        // One frame: the tick closure reports no change, so nothing redraws, which is the
        // batching rule. What this test proves is that a Release event did not end the loop
        // before the Press was reached, and that quitting still reaches `Ok`.
        assert_eq!(draws, 1, "an unchanged loop redrew");
    }

    #[test]
    fn loop_keeps_running_while_ctrl_c_is_ignored_mid_run() {
        let mut events = [key(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        )]
        .into_iter();

        let mut view = view();
        view.state = ui::State::Running;
        let mut ticks = 0;
        let result = tick_loop(
            &mut view,
            |_| Ok(()),
            |view, _| match events.next() {
                Some(event) => Ok(outcome_of(view, &event)),
                // The script is spent, so this is the only way out.
                None => Ok(Outcome::Quit),
            },
            |_| {
                ticks += 1;
                false
            },
        );

        assert!(result.is_ok());
        // The loop kept going after the ignored Ctrl+C, which is the property under test.
        assert!(ticks >= 1, "the loop stopped on an ignored Ctrl+C");
        assert_eq!(view.state, ui::State::Running, "the state changed");
    }

    #[test]
    fn loop_propagates_draw_errors() {
        // The error path out of `run`, after which `main` prints and returns 1 and the
        // `Tui` drop guard restores the terminal.
        let result = tick_loop(
            &mut view(),
            |_| Err(io::Error::other("draw failed")),
            |_, _| unreachable!("must not wait for an event after a failed draw"),
            |_| false,
        );

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Other);
    }

    #[test]
    fn loop_does_not_swallow_a_panic_from_the_draw() {
        // A swallowed panic would leave the loop spinning in raw mode; unwinding is what
        // lets the installed panic hook restore the terminal.
        let result = panic::catch_unwind(|| {
            let mut view = view();
            let _ = tick_loop(
                &mut view,
                |_| panic!("draw exploded"),
                |_, _| unreachable!("must not wait for an event after a panicking draw"),
                |_| false,
            );
        });

        assert!(result.is_err(), "the panic was swallowed");
    }

    #[test]
    fn an_idle_tick_draws_nothing_and_a_dirty_one_draws_once() {
        // This is the batching rule from spec §6.1b: one draw per change, not one per tick.
        let mut draws = 0;
        let mut rounds = 0;
        let result = tick_loop(
            &mut view(),
            |_| {
                draws += 1;
                Ok(())
            },
            |_, _| {
                rounds += 1;
                // Two clean rounds then quit, so a draw can only come from the first frame.
                if rounds > 2 {
                    Ok(Outcome::Quit)
                } else {
                    Ok(Outcome::Continue)
                }
            },
            |_| false,
        );

        assert!(result.is_ok());
        assert_eq!(draws, 1, "an idle loop drew {draws} frames");
    }

    #[test]
    fn a_dirty_tick_redraws() {
        let mut draws = 0;
        let mut rounds = 0;
        let result = tick_loop(
            &mut view(),
            |_| {
                draws += 1;
                Ok(())
            },
            |_, _| {
                rounds += 1;
                if rounds > 1 {
                    Ok(Outcome::Quit)
                } else {
                    Ok(Outcome::Continue)
                }
            },
            |_| true,
        );

        assert!(result.is_ok());
        // The first frame, then one per dirty tick before the quit.
        assert_eq!(draws, 2, "a dirty tick did not redraw: {draws} draws");
    }

    #[test]
    fn the_panic_hook_restores_the_terminal_once() {
        // The hook is the path that matters most: a panic in raw mode strands the shell in
        // the alternate screen. It is proved here rather than over a TTY by asserting the
        // restore-once guard, which only `restore_terminal` can set.
        //
        // Both statics are process-wide, so this test is the only one that installs a hook
        // and the only one that lets `restore_terminal` touch `TERMINAL_RESTORED`.
        let before = TERMINAL_RESTORED.load(Ordering::SeqCst);
        assert!(!before, "the terminal was already restored by another test");

        install_panic_hook();
        let result = panic::catch_unwind(|| panic!("deliberate panic, hook under test"));

        assert!(result.is_err(), "the panic did not unwind");
        assert!(
            TERMINAL_RESTORED.load(Ordering::SeqCst),
            "the panic hook did not restore the terminal"
        );
    }

    #[test]
    fn restore_is_claimed_once() {
        let flag = AtomicBool::new(false);

        assert!(claim_restore(&flag), "first restore must run");
        assert!(!claim_restore(&flag), "second restore must be skipped");
    }

    #[test]
    fn restore_writes_leave_screen_and_show_cursor() {
        let mut out = Vec::new();
        write_restore(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();

        assert!(
            out.contains("\u{1b}[?1049l"),
            "alternate screen kept: {out:?}"
        );
        assert!(out.contains("\u{1b}[?25h"), "cursor left hidden: {out:?}");
    }
}
