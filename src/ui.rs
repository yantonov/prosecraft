//! The only thing this build draws: one bordered frame with a static label.
//!
//! Everything here is TTY-free so it can be checked with a ratatui `TestBackend`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Block;

/// Fixed frame size, small enough to fit a default 80x24 terminal.
const FRAME_WIDTH: u16 = 36;
const FRAME_HEIGHT: u16 = 7;
const LABEL: &str = "prosecraft";

/// Draws the frame centered inside whatever area the terminal gives us.
pub fn render(frame: &mut Frame) {
    frame.render_widget(Block::bordered().title(LABEL), centered(frame.area()));
}

/// Clamping matters because a terminal can be smaller than the frame; without it the
/// frame's right/bottom edge would land outside the buffer and panic.
fn centered(area: Rect) -> Rect {
    let width = FRAME_WIDTH.min(area.width);
    let height = FRAME_HEIGHT.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// Plain `q` is accepted alongside `Ctrl+Q` so the quit path can be driven without a
/// control chord, e.g. by a pty-based check that only types printable keys.
pub fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('q')
        && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn draw_at(width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(render).unwrap();
        terminal.backend().buffer().clone()
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn render_draws_centered_bordered_frame_with_label() {
        // 36x7 frame inside 40x10 leaves exactly 2 columns and 1 row of margin.
        let buffer = draw_at(40, 10);
        let (left, right, top, bottom) = (2, 37, 1, 7);

        let top_row = row(&buffer, top);
        assert_eq!(buffer[(left, top)].symbol(), "┌");
        assert_eq!(buffer[(right, top)].symbol(), "┐");
        assert_eq!(buffer[(left, bottom)].symbol(), "└");
        assert_eq!(buffer[(right, bottom)].symbol(), "┘");
        assert!(
            top_row.trim_start().contains(LABEL),
            "label missing from title: {top_row:?}"
        );

        for y in top + 1..bottom {
            assert_eq!(buffer[(left, y)].symbol(), "│", "left border at row {y}");
            assert_eq!(buffer[(right, y)].symbol(), "│", "right border at row {y}");
        }

        // Empty margins on all four sides are what "centered" means here.
        for y in 0..10 {
            for x in 0..40 {
                let inside = (left..=right).contains(&x) && (top..=bottom).contains(&y);
                if !inside {
                    assert_eq!(buffer[(x, y)].symbol(), " ", "stray cell at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn tiny_terminal_keeps_frame_inside_the_buffer() {
        // 10x3 is narrower and shorter than the frame; clamping must still fill it.
        let buffer = draw_at(10, 3);

        assert_eq!(buffer[(0, 0)].symbol(), "┌");
        assert_eq!(buffer[(9, 0)].symbol(), "┐");
        assert_eq!(buffer[(0, 2)].symbol(), "└");
        assert_eq!(buffer[(9, 2)].symbol(), "┘");
    }

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn ctrl_q_quits() {
        assert!(is_quit(press(KeyCode::Char('q'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn plain_q_quits() {
        assert!(is_quit(press(KeyCode::Char('q'), KeyModifiers::NONE)));
    }

    #[test]
    fn other_keys_do_not_quit() {
        assert!(!is_quit(press(KeyCode::Char('Q'), KeyModifiers::CONTROL)));
        assert!(!is_quit(press(KeyCode::Char('q'), KeyModifiers::ALT)));
        assert!(!is_quit(press(KeyCode::Char('x'), KeyModifiers::NONE)));
        assert!(!is_quit(press(KeyCode::Esc, KeyModifiers::NONE)));
    }
}
