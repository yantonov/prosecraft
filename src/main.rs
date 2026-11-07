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
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use cli::{Config, PlaceholderClipboard};
use layout::Focus;

mod cli;
mod layout;
mod ui;

fn main() -> ExitCode {
    // `parse` is the exit-0 path for -h/-V and the exit-2 path for CLI misuse; both
    // happen here, before the harness check, which is what makes them work uninstalled.
    let args = cli::parse();

    let mut clipboard = PlaceholderClipboard;
    let config = match cli::assemble(args, cli::binary_on_path, &mut clipboard) {
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
    output_scroll: u16,
    debug_scroll: u16,
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
            output_scroll: 0,
            debug_scroll: 0,
        })
    }

    fn run(&mut self) -> io::Result<()> {
        let Self {
            terminal,
            config,
            focus,
            output_scroll,
            debug_scroll,
        } = self;

        let mut view = ui::View::new(config, *focus);
        view.output_scroll = *output_scroll;
        view.debug_scroll = *debug_scroll;

        event_loop(
            &mut view,
            |view| {
                terminal.draw(|frame| ui::render(frame, view))?;
                Ok(())
            },
            |view| {
                let event = event::read()?;
                if let Event::Key(key) = &event {
                    // Focus keys only for now; the rest of the §4 key table belongs to the
                    // pane-routing, rewrite and quit tickets.
                    view.handle_key(key);
                }
                Ok(event)
            },
        )
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Draws, then waits for one event, until an event asks to quit.
///
/// The terminal and the event source are closures so both the draw and the quit decision can
/// be tested without a TTY. The state they read and update is a parameter rather than a
/// capture, because two closures capturing the same `&mut` cannot be built at all.
fn event_loop<S>(
    state: &mut S,
    mut draw: impl FnMut(&mut S) -> io::Result<()>,
    mut next_event: impl FnMut(&mut S) -> io::Result<Event>,
) -> io::Result<()> {
    loop {
        draw(state)?;
        if is_exit_event(next_event(state)?) {
            return Ok(());
        }
    }
}

/// Windows reports a press and a release for every keystroke, so only presses count.
fn is_exit_event(event: Event) -> bool {
    matches!(event, Event::Key(key) if key.kind == KeyEventKind::Press && ui::is_quit(key))
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

        let mut draws = 0;
        let result = event_loop(
            &mut draws,
            |draws| {
                *draws += 1;
                Ok(())
            },
            |_| {
                Ok(events
                    .next()
                    .expect("event_loop over-read its event source"))
            },
        );

        assert!(result.is_ok());
        // One draw per event, including the one that quits.
        assert_eq!(draws, 3);
    }

    #[test]
    fn loop_propagates_draw_errors() {
        let result = event_loop(
            &mut (),
            |_| Err(io::Error::other("draw failed")),
            |_| unreachable!("must not wait for an event after a failed draw"),
        );

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Other);
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
