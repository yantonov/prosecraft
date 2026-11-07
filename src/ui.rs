//! The four-pane frame, its focus styling and the status line (spec §4).
//!
//! Everything here is TTY-free so a ratatui `TestBackend` can assert on what was drawn.
//! Content is placeholder text for now: the editor ticket owns buffers and the wrap ticket
//! owns wrapping, so this module only has to give each pane the right inner rectangle.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Paragraph, Wrap};

use crate::cli::Config;
use crate::layout::{self, Focus};

/// Glyph standing in for the cursor until the editor ticket brings a real one. A block on a
/// single cell reads as a caret without needing a styled-background pass over the buffer.
const CURSOR: &str = "▏";

/// Everything the frame needs that is not geometry. `Debug` so tests can build a fixed one.
#[derive(Debug, Clone)]
pub struct View<'a> {
    pub config: &'a Config,
    pub focus: Focus,
    pub source: &'a str,
    pub instructions: &'a str,
    pub output: &'a str,
    pub debug: &'a str,
    pub output_scroll: u16,
    pub debug_scroll: u16,
}

impl<'a> View<'a> {
    pub fn new(config: &'a Config, focus: Focus) -> Self {
        Self {
            config,
            focus,
            source: &config.initial_source,
            instructions: &config.initial_instructions,
            output: "",
            debug: "",
            output_scroll: 0,
            debug_scroll: 0,
        }
    }

    /// Applies the §4 focus keys: `Tab`/`Shift+Tab` in reading order, `Ctrl+Left`/`Ctrl+Right`
    /// across columns. Every other key is ignored here because the rest of the key table is
    /// owned by the pane-routing, rewrite and quit tickets.
    pub fn handle_key(&mut self, key: &KeyEvent) {
        // Windows reports a press and a release for every keystroke; acting on both would
        // cycle focus twice per Tab press.
        if key.kind != KeyEventKind::Press {
            return;
        }
        self.focus = match (key.code, key.modifiers) {
            // Terminals disagree on how Shift+Tab arrives, so both spellings are accepted.
            (KeyCode::Tab, KeyModifiers::SHIFT) | (KeyCode::BackTab, _) => self.focus.prev(),
            (KeyCode::Tab, _) => self.focus.next(),
            // Ctrl+Left and Ctrl+Right are one action: both cross to the other column.
            (KeyCode::Left, KeyModifiers::CONTROL) | (KeyCode::Right, KeyModifiers::CONTROL) => {
                self.focus.switch_column()
            }
            _ => self.focus,
        };
    }

    fn content(&self, focus: Focus) -> (&'a str, u16) {
        match focus {
            Focus::Source => (self.source, 0),
            Focus::Instructions => (self.instructions, 0),
            Focus::Output => (self.output, self.output_scroll),
            Focus::Debug => (self.debug, self.debug_scroll),
        }
    }
}

/// Draws the panes, then the status line over whatever the panes left.
pub fn render(frame: &mut Frame, view: &View) {
    let panes = layout::panes(frame.area());
    for (focus, area) in panes.all() {
        // A zero-sized pane is normal at tiny terminal sizes and must draw nothing at all.
        if area.width > 0 && area.height > 0 {
            draw_pane(frame, view, &panes, focus, area);
        }
    }
    if panes.status.height > 0 {
        draw_status(frame, view, panes.status);
    }
}

fn draw_pane(frame: &mut Frame, view: &View, panes: &layout::Panes, focus: Focus, area: Rect) {
    let focused = focus == view.focus;
    let block = Block::bordered()
        .border_type(border_type(focus, focused))
        .border_style(border_style(focused))
        .title(focus.title());

    let (text, scroll) = view.content(focus);
    // `Wrap` is what makes text soft-wrap instead of truncate; the widths it uses are the
    // pane's own, so each pane wraps at the width this layout gave it. Scroll is applied as
    // a row offset: the setup for the scrolling keys the pane-routing ticket adds.
    let content = if focused && focus.is_editable() {
        with_cursor(text)
    } else {
        text.to_owned()
    };
    // The block and the text are rendered separately so the text is laid out in exactly
    // `panes.inner(focus)`: that rectangle is the width the wrap ticket will wrap to, and
    // taking it from one place keeps the two from disagreeing.
    frame.render_widget(block, area);
    let paragraph = Paragraph::new(content)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, panes.inner(focus));
}

/// Read-only panes are identified by their border, so the focused one switches to a doubled
/// line; editable panes already show a cursor, so a colour change is enough and keeps their
/// single-line border. Unfocused panes stay plain, which is the contrast the user reads.
fn border_type(focus: Focus, focused: bool) -> BorderType {
    if focused && !focus.is_editable() {
        BorderType::Double
    } else {
        BorderType::Plain
    }
}

fn border_style(focused: bool) -> Style {
    if focused {
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    }
}

/// Placeholder caret: appended to the text so the glyph lands after the last character
/// without a second render pass or a character-cell measurement.
fn with_cursor(text: &str) -> String {
    format!("{text}{CURSOR}")
}

/// One line, always: the spec puts a long error here and a wrapped status line would resize
/// the panes. Content is refined by the status-quit ticket.
fn draw_status(frame: &mut Frame, view: &View, area: Rect) {
    let session = view.config.session_id.as_deref().unwrap_or("—");
    let line = format!(
        " {} │ {} │ {} │ [Ctrl+R] rewrite  [Tab] focus  [Ctrl+Q] quit",
        view.config.harness.binary_name(),
        session,
        "idle",
    );
    let paragraph = Paragraph::new(Line::from(line)).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
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
    use crate::cli::Harness;
    use crate::layout;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;

    fn config() -> Config {
        Config {
            harness: Harness::Claude,
            model: None,
            session_id: None,
            initial_source: "source text".to_owned(),
            initial_instructions: "make it formal".to_owned(),
        }
    }

    fn draw_at(view: &View, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, view)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn draw(width: u16, height: u16) -> Buffer {
        draw_at(&View::new(&config(), Focus::Source), width, height)
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    /// Reads the title out of a top border by dropping the corner and the fill glyph.
    fn title_at(buffer: &Buffer, area: Rect) -> String {
        (area.x + 1..area.x + area.width - 1)
            .map(|x| buffer[(x, area.y)].symbol())
            .collect::<String>()
            .trim_start_matches('─')
            .split('─')
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned()
    }

    /// The four corners of a pane as drawn, so a test can prove a border exists at all.
    fn corners(buffer: &Buffer, area: Rect) -> [&str; 4] {
        let (left, right) = (area.x, area.x + area.width - 1);
        let (top, bottom) = (area.y, area.y + area.height - 1);
        [
            buffer[(left, top)].symbol(),
            buffer[(right, top)].symbol(),
            buffer[(left, bottom)].symbol(),
            buffer[(right, bottom)].symbol(),
        ]
    }

    fn is_corner(symbol: &str) -> bool {
        matches!(symbol, "┌" | "┐" | "└" | "┘" | "╔" | "╗" | "╚" | "╝")
    }

    #[test]
    fn draws_four_bordered_panes_in_spec_order_at_80x24() {
        let buffer = draw(80, 24);
        let panes = layout::panes(Rect::new(0, 0, 80, 24));

        for (focus, area) in panes.all() {
            let corners = corners(&buffer, area);
            assert!(
                corners.iter().all(|symbol| is_corner(symbol)),
                "{focus:?} has no border at {area:?}: {corners:?}"
            );
            // Vertical borders only; the corner rows are covered by `corners`.
            for y in area.y + 1..area.y + area.height - 1 {
                assert!(
                    matches!(buffer[(area.x, y)].symbol(), "│" | "║"),
                    "{focus:?} left border missing at row {y}"
                );
                let right = area.x + area.width - 1;
                assert!(
                    matches!(buffer[(right, y)].symbol(), "│" | "║"),
                    "{focus:?} right border missing at row {y}"
                );
            }
        }

        assert_eq!(title_at(&buffer, panes.source), "source");
        assert_eq!(title_at(&buffer, panes.instructions), "instructions");
        assert_eq!(title_at(&buffer, panes.output), "output");
        assert_eq!(title_at(&buffer, panes.debug), "debug");

        // Spec order, expressed as positions: source over instructions on the left,
        // output over debug on the right.
        assert_eq!((panes.source.x, panes.source.y), (0, 0));
        assert_eq!((panes.instructions.x, panes.instructions.y), (0, 18));
        assert_eq!((panes.output.x, panes.output.y), (40, 0));
        assert_eq!((panes.debug.x, panes.debug.y), (40, 16));
    }

    #[test]
    fn columns_split_fifty_fifty_at_80x24() {
        let buffer = draw(80, 24);

        // Both top borders run to column 39, and the right column starts at 40.
        assert_eq!(buffer[(0, 0)].symbol(), "┌");
        assert_eq!(buffer[(39, 0)].symbol(), "┐");
        assert_eq!(buffer[(40, 0)].symbol(), "┌");
        assert_eq!(buffer[(79, 0)].symbol(), "┐");
        // The columns end right where the status row begins, i.e. both are 23 rows tall.
        assert_eq!(buffer[(39, 22)].symbol(), "┘");
        assert_eq!(buffer[(40, 22)].symbol(), "└");
    }

    #[test]
    fn panes_still_draw_at_40x10() {
        let buffer = draw(40, 10);
        let panes = layout::panes(Rect::new(0, 0, 40, 10));

        for (focus, area) in panes.all() {
            assert!(
                corners(&buffer, area)
                    .iter()
                    .all(|symbol| is_corner(symbol)),
                "{focus:?} has no border at {area:?}"
            );
            assert!(
                !title_at(&buffer, area).is_empty(),
                "{focus:?} lost its title"
            );
        }

        assert_eq!(title_at(&buffer, panes.source), "source");
        assert_eq!(title_at(&buffer, panes.instructions), "instructions");
        assert_eq!(title_at(&buffer, panes.output), "output");
        assert_eq!(title_at(&buffer, panes.debug), "debug");

        // 9 body rows: 25% floors to 2, so the floor-3 rule decides the instructions height.
        assert_eq!(panes.instructions.height, 3);
        assert_eq!(panes.source.height, 6);
    }

    #[test]
    fn one_by_one_does_not_panic() {
        let buffer = draw(1, 1);

        // Nothing can be drawn but the status line, and the frame must survive anyway.
        assert_eq!(buffer.area, Rect::new(0, 0, 1, 1));
    }

    #[test]
    fn resizing_redistributes_panes_without_panicking() {
        let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
        let config = config();
        let view = View::new(&config, Focus::Output);

        // Far bigger and far smaller than the minimum, in both directions.
        for (width, height) in [
            (1, 1),
            (2, 2),
            (40, 10),
            (81, 25),
            (200, 60),
            (3, 1),
            (1, 40),
        ] {
            terminal.backend_mut().resize(width, height);
            terminal.draw(|frame| render(frame, &view)).unwrap();
            assert_eq!(
                terminal.backend().buffer().area,
                Rect::new(0, 0, width, height)
            );
        }
    }

    #[test]
    fn focused_read_only_pane_uses_a_different_border() {
        let config = config();
        let unfocused = draw_at(&View::new(&config, Focus::Source), 80, 24);
        let focused = draw_at(&View::new(&config, Focus::Output), 80, 24);
        let panes = layout::panes(Rect::new(0, 0, 80, 24));

        assert_eq!(corners(&unfocused, panes.output), ["┌", "┐", "└", "┘"]);
        assert_eq!(corners(&focused, panes.output), ["╔", "╗", "╚", "╝"]);
        // Only the focused pane changes: the others keep the plain border.
        assert_eq!(corners(&focused, panes.source), ["┌", "┐", "└", "┘"]);
        assert_eq!(corners(&focused, panes.debug), ["┌", "┐", "└", "┘"]);
    }

    #[test]
    fn focused_pane_border_is_highlighted_at_every_position() {
        let config = config();
        let panes = layout::panes(Rect::new(0, 0, 80, 24));

        for focus in Focus::ALL {
            let buffer = draw_at(&View::new(&config, focus), 80, 24);
            for (other, area) in panes.all() {
                let style = buffer[(area.x, area.y)].style();
                if other == focus {
                    assert!(
                        style.fg == Some(Color::Yellow)
                            && style.add_modifier.contains(Modifier::BOLD),
                        "{other:?} is focused but its border is not highlighted"
                    );
                } else {
                    // `Reset` is what ratatui reports for "no foreground was set".
                    assert!(
                        matches!(style.fg, None | Some(Color::Reset)),
                        "{other:?} is not focused but its border is fg {:?}",
                        style.fg
                    );
                }
            }
        }
    }

    #[test]
    fn editable_focused_pane_shows_a_cursor_and_read_only_ones_do_not() {
        let config = config();

        let source = draw_at(&View::new(&config, Focus::Source), 80, 24);
        let instructions = draw_at(&View::new(&config, Focus::Instructions), 80, 24);
        let output = draw_at(&View::new(&config, Focus::Output), 80, 24);

        assert!(contains(&source, CURSOR), "source pane lost its cursor");
        assert!(
            contains(&instructions, CURSOR),
            "instructions pane lost its cursor"
        );
        assert!(
            !contains(&output, CURSOR),
            "a read-only pane must not show a cursor"
        );
    }

    #[test]
    fn unfocused_editable_pane_shows_no_cursor() {
        let config = config();
        let buffer = draw_at(&View::new(&config, Focus::Output), 80, 24);

        assert!(!contains(&buffer, CURSOR), "cursor drawn while unfocused");
    }

    #[test]
    fn seeded_buffers_render_in_their_own_panes() {
        let buffer = draw(80, 24);
        let panes = layout::panes(Rect::new(0, 0, 80, 24));

        assert!(row(&buffer, panes.source.y + 1).contains("source text"));
        assert!(row(&buffer, panes.instructions.y + 1).contains("make it formal"));
    }

    #[test]
    fn scroll_offsets_pane_content_without_touching_the_other_pane() {
        let config = config();
        let lines = (0..20)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut view = View::new(&config, Focus::Debug);
        view.output = &lines;
        view.debug = &lines;

        let top = draw_at(&view, 80, 24);
        view.output_scroll = 3;
        let scrolled = draw_at(&view, 80, 24);

        let panes = layout::panes(Rect::new(0, 0, 80, 24));
        let output_row = row(&top, panes.output.y + 1);
        assert!(
            output_row.contains("line0"),
            "output did not start at 0: {output_row:?}"
        );

        let output_row = row(&scrolled, panes.output.y + 1);
        assert!(
            output_row.contains("line3"),
            "output scroll ignored: {output_row:?}"
        );

        // The debug pane has its own offset and must be unaffected by the output one.
        let debug_row = row(&scrolled, panes.debug.y + 1);
        assert!(
            debug_row.contains("line0"),
            "debug pane scrolled with output: {debug_row:?}"
        );
    }

    #[test]
    fn debug_scroll_offset_is_independent_of_the_output_offset() {
        let config = config();
        let lines = (0..20)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut view = View::new(&config, Focus::Debug);
        view.output = &lines;
        view.debug = &lines;
        view.debug_scroll = 2;

        let buffer = draw_at(&view, 80, 24);
        let panes = layout::panes(Rect::new(0, 0, 80, 24));

        let debug_row = row(&buffer, panes.debug.y + 1);
        assert!(
            debug_row.contains("line2"),
            "debug scroll ignored: {debug_row:?}"
        );
        let output_row = row(&buffer, panes.output.y + 1);
        assert!(
            output_row.contains("line0"),
            "output pane scrolled with debug: {output_row:?}"
        );
    }

    #[test]
    fn long_text_wraps_inside_the_pane_instead_of_truncating() {
        // Inner width is 18 at 40x10, so this line must occupy several rows and keep its
        // last word. Truncation would drop the tail, which is what the spec forbids.
        let mut config = config();
        config.initial_source = (1..=9)
            .map(|n| format!("word{n}"))
            .collect::<Vec<_>>()
            .join(" ");

        let buffer = draw_at(&View::new(&config, Focus::Source), 40, 10);
        let panes = layout::panes(Rect::new(0, 0, 40, 10));
        let rendered = (panes.source.y + 1..panes.source.y + panes.source.height - 1)
            .map(|y| row(&buffer, y))
            .collect::<String>();

        assert!(rendered.contains("word9"), "tail dropped: {rendered:?}");
        assert!(rendered.contains("word1 "), "head dropped: {rendered:?}");
    }

    #[test]
    fn status_line_shows_harness_session_state_and_hints() {
        let buffer = draw(80, 24);
        let status = row(&buffer, 23);

        assert!(status.contains("claude"), "harness missing: {status:?}");
        assert!(status.contains('—'), "session em-dash missing: {status:?}");
        assert!(status.contains("idle"), "state missing: {status:?}");
        assert!(status.contains("Ctrl+R"), "hints missing: {status:?}");

        // Order is fixed by the spec: harness, session, state, hints.
        let positions = ["claude", "—", "idle", "Ctrl+R"].map(|needle| {
            status
                .find(needle)
                .unwrap_or_else(|| panic!("{needle:?} missing from {status:?}"))
        });
        let mut sorted = positions;
        sorted.sort_unstable();
        assert_eq!(
            positions, sorted,
            "status line out of spec order: {status:?}"
        );
    }

    #[test]
    fn status_line_names_the_selected_harness() {
        let mut config = config();
        config.harness = Harness::Pi;
        let buffer = draw_at(&View::new(&config, Focus::Source), 80, 10);

        assert!(
            row(&buffer, 9).contains("pi"),
            "pi not named: {:?}",
            row(&buffer, 9)
        );
    }

    #[test]
    fn status_line_shows_a_real_session_id_once_one_exists() {
        let mut config = config();
        config.session_id = Some("abc123".to_owned());
        let buffer = draw_at(&View::new(&config, Focus::Source), 80, 10);
        let status = row(&buffer, 9);

        assert!(status.contains("abc123"), "session id missing: {status:?}");
        assert!(
            !status.contains('—'),
            "em-dash kept past the first rewrite: {status:?}"
        );
    }

    #[test]
    fn status_line_keeps_one_row_at_every_size() {
        for (width, height) in [(1, 1), (1, 2), (40, 10), (80, 24)] {
            let buffer = draw(width, height);
            let panes = layout::panes(Rect::new(0, 0, width, height));
            assert_eq!(panes.status.height, 1, "no status row at {width}x{height}");
            assert_eq!(buffer.area.height, height);
        }
    }

    fn contains(buffer: &Buffer, needle: &str) -> bool {
        (0..buffer.area.height).any(|y| row(buffer, y).contains(needle))
    }

    fn focus_after(key: KeyEvent, from: Focus) -> Focus {
        let config = config();
        let mut view = View::new(&config, from);
        view.handle_key(&key);
        view.focus
    }

    #[test]
    fn tab_and_shift_tab_cycle_focus_through_all_four_panes() {
        for from in Focus::ALL {
            assert_eq!(
                focus_after(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), from),
                from.next(),
                "Tab from {from:?}"
            );
            assert_eq!(
                focus_after(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT), from),
                from.prev(),
                "Shift+Tab from {from:?}"
            );
            // Terminals that report Shift+Tab as BackTab must get the same answer.
            assert_eq!(
                focus_after(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE), from),
                from.prev(),
                "BackTab from {from:?}"
            );
        }
    }

    #[test]
    fn four_tabs_walk_source_instructions_output_debug_and_back() {
        let mut focus = Focus::Source;
        let mut seen = vec![focus];
        for _ in 0..4 {
            focus = focus_after(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), focus);
            seen.push(focus);
        }

        assert_eq!(
            seen,
            [
                Focus::Source,
                Focus::Instructions,
                Focus::Output,
                Focus::Debug,
                Focus::Source
            ]
        );
    }

    #[test]
    fn ctrl_arrows_jump_columns_preserving_the_row() {
        for from in Focus::ALL {
            let expected = from.switch_column();
            for code in [KeyCode::Left, KeyCode::Right] {
                let key = KeyEvent::new(code, KeyModifiers::CONTROL);
                assert_eq!(focus_after(key, from), expected, "{code:?} from {from:?}");
            }
        }

        // Spelled out, because this is the mapping the spec names.
        assert_eq!(
            focus_after(ctrl(KeyCode::Right), Focus::Source),
            Focus::Output
        );
        assert_eq!(
            focus_after(ctrl(KeyCode::Left), Focus::Output),
            Focus::Source
        );
        assert_eq!(
            focus_after(ctrl(KeyCode::Right), Focus::Instructions),
            Focus::Debug
        );
        assert_eq!(
            focus_after(ctrl(KeyCode::Left), Focus::Debug),
            Focus::Instructions
        );
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn key_releases_do_not_move_focus() {
        // Windows sends a release after every press; acting on both would double-step.
        let release =
            KeyEvent::new_with_kind(KeyCode::Tab, KeyModifiers::NONE, KeyEventKind::Release);

        assert_eq!(focus_after(release, Focus::Source), Focus::Source);
    }

    #[test]
    fn keys_owned_by_other_tickets_leave_focus_alone() {
        for (code, modifiers) in [
            (KeyCode::Char('r'), KeyModifiers::CONTROL),
            (KeyCode::Left, KeyModifiers::NONE),
            (KeyCode::Right, KeyModifiers::NONE),
            (KeyCode::Esc, KeyModifiers::NONE),
            (KeyCode::Char('x'), KeyModifiers::NONE),
        ] {
            assert_eq!(
                focus_after(KeyEvent::new(code, modifiers), Focus::Debug),
                Focus::Debug,
                "{code:?} with {modifiers:?} moved focus"
            );
        }
    }

    #[test]
    fn focus_changes_redraw_the_highlight_on_the_next_frame() {
        let config = config();
        let mut view = View::new(&config, Focus::Source);
        let panes = layout::panes(Rect::new(0, 0, 80, 24));

        let before = {
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal.draw(|frame| render(frame, &view)).unwrap();
            terminal.backend().buffer().clone()
        };
        assert!(before[(panes.source.x, panes.source.y)].style().fg == Some(Color::Yellow));

        view.handle_key(&ctrl(KeyCode::Right));
        let after = {
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal.draw(|frame| render(frame, &view)).unwrap();
            terminal.backend().buffer().clone()
        };

        assert!(after[(panes.output.x, panes.output.y)].style().fg == Some(Color::Yellow));
        assert!(
            matches!(
                after[(panes.source.x, panes.source.y)].style().fg,
                None | Some(Color::Reset)
            ),
            "the pane we left is still highlighted"
        );
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
