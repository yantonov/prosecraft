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
        cli::binary_on_path,
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
        // The real clipboard for the running app; tests build views with the recording one.
        view.clipboard = Box::new(crate::clipboard::System::new());

        event_loop(
            &mut view,
            |view| {
                terminal.draw(|frame| ui::render(frame, view))?;
                Ok(())
            },
            |view| {
                let event = event::read()?;
                let outcome = match &event {
                    Event::Key(key) => view.handle_key(key),
                    // Resize and every other event leave the state alone; there is still a
                    // frame to draw for them, so they are not a quit.
                    _ => Outcome::Continue,
                };
                Ok(outcome)
            },
        )
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Draws, then waits for one event, until the view asks to quit.
///
/// The terminal and the event source are closures so both the draw and the quit decision can
/// be tested without a TTY. The state they read and update is a parameter rather than a
/// capture, because two closures capturing the same `&mut` cannot be built at all.
fn event_loop<S>(
    state: &mut S,
    mut draw: impl FnMut(&mut S) -> io::Result<()>,
    mut next_event: impl FnMut(&mut S) -> io::Result<Outcome>,
) -> io::Result<()> {
    loop {
        draw(state)?;
        if next_event(state)? == Outcome::Quit {
            return Ok(());
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
        let result = event_loop(
            &mut view,
            |_| {
                draws += 1;
                Ok(())
            },
            |view| {
                let event = events
                    .next()
                    .expect("event_loop over-read its event source");
                Ok(outcome_of(view, &event))
            },
        );

        // Ok here is the normal-quit path: `main` then returns success and `Tui` is dropped,
        // which is where the terminal is restored.
        assert!(result.is_ok());
        // One draw per event, including the one that quits.
        assert_eq!(draws, 3);
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
        let mut draws = 0;
        let result = event_loop(
            &mut view,
            |_| {
                draws += 1;
                Ok(())
            },
            |view| match events.next() {
                Some(event) => Ok(outcome_of(view, &event)),
                // The script is spent, so this is the only way out. If the ignored Ctrl+C
                // had already ended the loop, the two draws below would be one.
                None => Ok(Outcome::Quit),
            },
        );

        assert!(result.is_ok());
        // Two draws: one before the key and one after it was ignored. Stopping on the
        // Ctrl+C would have exited after the first draw, and the note it wrote would never
        // have been painted.
        assert_eq!(draws, 2, "the loop stopped on an ignored Ctrl+C");
        assert_eq!(view.state, ui::State::Running, "the state changed");
    }

    #[test]
    fn loop_propagates_draw_errors() {
        // The error path out of `run`, after which `main` prints and returns 1 and the
        // `Tui` drop guard restores the terminal.
        let result = event_loop(
            &mut view(),
            |_| Err(io::Error::other("draw failed")),
            |_| unreachable!("must not wait for an event after a failed draw"),
        );

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Other);
    }

    #[test]
    fn loop_does_not_swallow_a_panic_from_the_draw() {
        // A swallowed panic would leave the loop spinning in raw mode; unwinding is what
        // lets the installed panic hook restore the terminal.
        let result = panic::catch_unwind(|| {
            let mut view = view();
            let _ = event_loop(
                &mut view,
                |_| panic!("draw exploded"),
                |_| unreachable!("must not wait for an event after a panicking draw"),
            );
        });

        assert!(result.is_err(), "the panic was swallowed");
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
