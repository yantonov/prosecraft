//! Minimal ratatui shell: draw one frame, quit on `Ctrl+Q` (`q` also works).
//!
//! This exists only to prove terminal setup/teardown: raw mode, alternate screen, and a
//! panic that cannot leave the user's terminal broken. Deliberately no panes, no args,
//! no harness, no clipboard.
//!
//! The terminal-ownership code here is expected to be moved behind argument parsing by a
//! later ticket; duplicating it now is the shortest path to a proven clean exit.

use std::io::{self, Stdout, Write};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::cursor;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

mod ui;

fn main() -> io::Result<()> {
    Tui::start()?.run()
}

struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Tui {
    fn start() -> io::Result<Self> {
        // The hook must be in place before we touch the terminal, otherwise a panic
        // during setup (or the first draw) would strand the user in raw mode.
        install_panic_hook();

        terminal::enable_raw_mode()?;
        match Self::enter_alternate_screen() {
            Ok(tui) => Ok(tui),
            // Raw mode is already on, and the failed value never reaches a Drop guard.
            Err(err) => {
                restore_terminal();
                Err(err)
            }
        }
    }

    fn enter_alternate_screen() -> io::Result<Self> {
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        Ok(Self {
            terminal: Terminal::new(CrosstermBackend::new(stdout))?,
        })
    }

    fn run(&mut self) -> io::Result<()> {
        let terminal = &mut self.terminal;
        event_loop(
            || {
                terminal.draw(ui::render)?;
                Ok(())
            },
            event::read,
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
/// The terminal is passed in as a closure so the quit decision can be tested without
/// a TTY.
fn event_loop(
    mut draw: impl FnMut() -> io::Result<()>,
    mut next_event: impl FnMut() -> io::Result<Event>,
) -> io::Result<()> {
    loop {
        draw()?;
        if is_exit_event(next_event()?) {
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
            || {
                draws += 1;
                Ok(())
            },
            || {
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
            || Err(io::Error::other("draw failed")),
            || unreachable!("must not wait for an event after a failed draw"),
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
