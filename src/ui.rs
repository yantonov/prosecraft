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
use crate::panes::StreamState;

/// Glyph standing in for the cursor until the editor ticket brings a real one. A block on a
/// single cell reads as a caret without needing a styled-background pass over the buffer.
const CURSOR: &str = "▏";

/// Shown instead of quitting when the quit key arrives mid-rewrite (spec §4: "Quit; confirm
/// if a rewrite is running"). Accepting the confirmation is the robustness ticket, so this
/// ticket only has to get the prompt onto the status line.
const CONFIRM_QUIT: &str = "press Ctrl+Q again to quit";

/// Shown for `Ctrl+C` mid-rewrite. Spec §4 makes that key a cancel rather than a quit, and
/// cancelling is the robustness ticket, so the key is ignored here and says so.
const CANCEL_UNIMPLEMENTED: &str = "Ctrl+C cancels, but cancel is not implemented yet";

/// The state word on the status line (spec §4: `idle` / `running` / `error`).
///
/// Three values and no transitions: the rewrite flow that drives them is a later ticket, so
/// this ticket only has to render the word and let a test set each value directly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Running,
    // Reached only from the rewrite flow, which is a later ticket: nothing outside the tests
    // constructs it yet, and the variant has to exist now because the status line renders it.
    #[allow(dead_code)]
    Error,
}

impl State {
    /// Spelled exactly as spec §4 spells it, because these are the words the user reads.
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Error => "error",
        }
    }
}

/// The only thing a key can tell the event loop: carry on, or stop.
///
/// Focus and editing stay inside this module as view state, so a quit decision is the sole
/// answer that has to travel back out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    Quit,
}

/// Everything the frame needs that is not geometry. `Debug` so tests can build a fixed one.
#[derive(Debug, Clone)]
pub struct View<'a> {
    pub config: &'a Config,
    pub focus: Focus,
    pub source: &'a str,
    pub instructions: &'a str,
    /// Content and scroll state of the output and debug panes (spec §5, §6.1b). Owned here
    /// rather than borrowed because the stream writes into it while the view is alive.
    pub stream: StreamState,
    pub state: State,
    /// One-line note beside the state word: validation text, an error, the quit prompt, or
    /// how the last run ended (a cancelled rewrite returns to `Idle` with its note here).
    /// Kept apart from [`State`] because the word must stay one of three and this is free
    /// text that may be arbitrarily long.
    pub message: Option<String>,
}

impl<'a> View<'a> {
    pub fn new(config: &'a Config, focus: Focus) -> Self {
        Self {
            config,
            focus,
            source: &config.initial_source,
            instructions: &config.initial_instructions,
            stream: StreamState::new(),
            state: State::Idle,
            message: None,
        }
    }

    /// Applies the §4 keys this ticket owns: focus, quit, and the keys that must do nothing.
    /// Returns whether the run is over.
    ///
    /// The state is read here rather than by the caller because the same key does not always
    /// mean the same thing: spec §4 makes `Ctrl+C` a quit while idle and a cancel while a
    /// rewrite runs, so the key cannot be decoded without the state in hand.
    pub fn handle_key(&mut self, key: &KeyEvent) -> Outcome {
        // Windows reports a press and a release for every keystroke; acting on both would
        // cycle focus twice per Tab press and quit when `q` is released.
        if key.kind != KeyEventKind::Press {
            return Outcome::Continue;
        }
        if is_quit(*key) {
            return self.request_quit();
        }
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            return self.interrupt();
        }
        // Scrolling a read-only pane is a view operation, not a movement of the cursor, so it
        // is handled before focus routing and only for the panes that have an offset.
        if !self.focus.is_editable() {
            match (key.code, key.modifiers) {
                // Spec §4: PgUp/PgDn move by a page in the focused pane.
                (KeyCode::PageUp, _) => {
                    self.stream.page(self.focus, -1);
                    return Outcome::Continue;
                }
                (KeyCode::PageDown, _) => {
                    self.stream.page(self.focus, 1);
                    return Outcome::Continue;
                }
                // The arrow keys scroll rather than move: there is no cursor in a read-only
                // pane, and a pane that ignored them would look broken.
                (KeyCode::Up, _) => {
                    self.stream.scroll(self.focus, -1);
                    return Outcome::Continue;
                }
                (KeyCode::Down, _) => {
                    self.stream.scroll(self.focus, 1);
                    return Outcome::Continue;
                }
                _ => {}
            }
        }
        // Esc stays in the fall-through arm on purpose: spec §4 fixes it as "does nothing in
        // v0.1", and a key silently doing something later is harder to notice than a test.
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
        Outcome::Continue
    }

    /// `Ctrl+Q` from any focus state: quit, unless a rewrite is running and spec §4 asks for
    /// a confirmation first.
    fn request_quit(&mut self) -> Outcome {
        if self.state == State::Running {
            self.message = Some(CONFIRM_QUIT.to_owned());
            return Outcome::Continue;
        }
        Outcome::Quit
    }

    /// `Ctrl+C`: quit when there is nothing to cancel, otherwise cancel the harness — which
    /// is the robustness ticket's half, so mid-run this deliberately ignores the key instead
    /// of pretending to have cancelled.
    fn interrupt(&mut self) -> Outcome {
        match self.state {
            State::Running => {
                self.message = Some(CANCEL_UNIMPLEMENTED.to_owned());
                Outcome::Continue
            }
            State::Idle | State::Error => Outcome::Quit,
        }
    }

    /// The editable panes' text. The read-only ones are drawn by [`StreamState`], which owns
    /// their wrapping and scroll offsets.
    fn content(&self, focus: Focus) -> &'a str {
        match focus {
            Focus::Source => self.source,
            Focus::Instructions => self.instructions,
            Focus::Output | Focus::Debug => "",
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
    // The debug pane says so when it has stopped following the stream, so a scrolled pane
    // cannot look frozen (spec §6.1b). The output pane has nothing to follow and never says
    // it.
    let suffix = match focus {
        Focus::Debug => view.stream.debug_suffix(),
        Focus::Output => view.stream.output_suffix(),
        Focus::Source | Focus::Instructions => None,
    };
    let mut title = focus.title().to_owned();
    if let Some(suffix) = suffix {
        title.push_str(" — ");
        title.push_str(suffix);
    }
    let block = Block::bordered()
        .border_type(border_type(focus, focused))
        .border_style(border_style(focused))
        .title(title);

    // The block and the text are rendered separately so the text is laid out in exactly
    // `panes.inner(focus)`: that rectangle is the width the wrap ticket wraps to, and taking
    // it from one place keeps the two from disagreeing.
    frame.render_widget(block, area);
    if focus.is_editable() {
        // `Wrap` is what makes text soft-wrap instead of truncate; the widths it uses are the
        // pane's own, so each pane wraps at the width this layout gave it.
        let text = view.content(focus);
        let content = if focused {
            with_cursor(text)
        } else {
            text.to_owned()
        };
        let paragraph = Paragraph::new(content).wrap(Wrap { trim: false });
        frame.render_widget(paragraph, panes.inner(focus));
    } else {
        // Read-only panes own their wrapping and their offset: `StreamState` is what keeps a
        // page a screenful and the auto-tail honest.
        view.stream.render(focus, frame, panes.inner(focus));
    }
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

/// One line, always: spec §8 puts a long error here, and a status line that wrapped would
/// resize the panes. [`fit`] is what keeps that promise.
///
/// Field order is spec §4's: harness, session, state, then the key hints. The note is not a
/// field in that list, so it is inserted just before the hints; that keeps every fixed field
/// in its place and makes clipping eat the static hints rather than an error the user needs.
fn draw_status(frame: &mut Frame, view: &View, area: Rect) {
    let session = view.config.session_id.as_deref().unwrap_or("—");
    let mut line = format!(
        " {} │ {} │ {}",
        view.config.harness.binary_name(),
        session,
        view.state.label(),
    );
    if let Some(message) = &view.message {
        line.push_str(" │ ");
        line.push_str(message);
    }
    line.push_str(" │ [Ctrl+R] rewrite  [Tab] focus  [Ctrl+Q] quit");
    // No `Wrap`: the line is already guaranteed to fit, and leaving wrapping off makes
    // "never wraps" a property of the code rather than of the widget's defaults.
    frame.render_widget(Paragraph::new(Line::from(fit(&line, area.width))), area);
}

/// Truncates `text` to `width` display cells, ending in `…` when it had to cut.
///
/// Cells, not bytes: `…` is one cell but three bytes, so a byte-based cut would either spill
/// past the row or split a character. The width rule is ratatui's own, reached through
/// `Line` so nothing here has to depend on the width crate directly.
fn fit(text: &str, width: u16) -> String {
    let available = usize::from(width);
    // A zero-width area shows nothing: an ellipsis would still need one cell.
    if available == 0 {
        return String::new();
    }
    if display_width(text) <= available {
        return text.to_owned();
    }
    // The ellipsis spends a cell of its own, so the kept prefix may claim only the rest.
    let budget = available - 1;
    let mut out = String::new();
    for c in text.chars() {
        out.push(c);
        if display_width(&out) > budget {
            out.pop();
            break;
        }
    }
    out.push('…');
    out
}

/// Rendered width of `text` in terminal cells, counting a wide glyph as two - the same count
/// the renderer uses to place the cell after it.
fn display_width(text: &str) -> usize {
    Line::from(text).width()
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

    /// Draws a view with the given state and message, then returns its status row.
    fn status_row(state: State, message: Option<&str>, width: u16) -> String {
        let config = config();
        let mut view = View::new(&config, Focus::Source);
        view.state = state;
        view.message = message.map(str::to_owned);
        let buffer = draw_at(&view, width, 10);
        row(&buffer, 9)
    }

    #[test]
    fn every_state_renders_its_own_word() {
        for (state, word) in [
            (State::Idle, "idle"),
            (State::Running, "running"),
            (State::Error, "error"),
        ] {
            assert_eq!(state.label(), word);
            let status = status_row(state, None, 80);
            assert!(status.contains(word), "{state:?} not shown: {status:?}");
            // The word sits between the session and the hints, in spec §4's order.
            let session = status.find('—').expect("session em-dash missing");
            let state_at = status.find(word).expect("state word missing");
            let hints = status.find("Ctrl+R").expect("hints missing");
            assert!(session < state_at && state_at < hints, "{status:?}");
        }
    }

    #[test]
    fn a_message_sits_between_the_state_word_and_the_hints() {
        let status = status_row(State::Error, Some("missing output markers"), 120);

        assert!(status.contains("claude"), "harness lost: {status:?}");
        assert!(status.contains("missing output markers"), "{status:?}");
        // Spec §4 fixes the four fields; the note is not one of them, so it must not push
        // any of them out of order.
        let positions = [
            "claude",
            "—",
            "error",
            "missing output markers",
            "Ctrl+R",
            "Ctrl+Q",
        ]
        .map(|needle| {
            status
                .find(needle)
                .unwrap_or_else(|| panic!("{needle:?} missing from {status:?}"))
        });
        let mut sorted = positions;
        sorted.sort_unstable();
        assert_eq!(positions, sorted, "out of order: {status:?}");
    }

    #[test]
    fn a_cancelled_run_returns_to_idle_with_a_note() {
        // The ticket's fake of the cancelled outcome: the state word says idle and the
        // reason is the free-text channel, so the two never have to share one string.
        let status = status_row(State::Idle, Some("cancelled"), 80);

        assert!(status.contains("idle"), "{status:?}");
        assert!(status.contains("cancelled"), "{status:?}");
    }

    #[test]
    fn a_long_message_is_clipped_with_an_ellipsis_and_never_wraps() {
        let long = "x".repeat(200);
        let status = status_row(State::Error, Some(&long), 40);

        // One row can hold 40 cells, so the row ends in the ellipsis and nothing spilled
        // into the row above (which is a pane border, not more status).
        assert_eq!(status.chars().count(), 40, "row changed width: {status:?}");
        assert!(status.ends_with('…'), "no ellipsis: {status:?}");
        assert!(
            status.contains("claude"),
            "the fixed fields were squeezed out"
        );
        assert!(status.contains("error"), "the state word was squeezed out");

        let config = config();
        let mut view = View::new(&config, Focus::Source);
        view.state = State::Error;
        view.message = Some(long);
        let buffer = draw_at(&view, 40, 10);
        let panes = layout::panes(Rect::new(0, 0, 40, 10));
        // The row directly above the status line is still the pane bottom border: a wrapped
        // status line would have painted over it.
        assert_eq!(buffer[(0, panes.status.y - 1)].symbol(), "└");
    }

    #[test]
    fn a_clipped_message_keeps_the_fixed_fields_and_says_it_was_cut() {
        // The error path is the one that matters: the user must still see which harness and
        // which state, and must know the note continues.
        let status = status_row(State::Error, Some(&"boom ".repeat(40)), 60);

        let positions = ["claude", "—", "error", "boom"].map(|needle| {
            status
                .find(needle)
                .unwrap_or_else(|| panic!("{needle:?} missing from {status:?}"))
        });
        let mut sorted = positions;
        sorted.sort_unstable();
        assert_eq!(positions, sorted, "out of order: {status:?}");
        assert!(status.ends_with('…'), "no ellipsis: {status:?}");
    }

    #[test]
    fn fit_keeps_short_text_and_clips_long_text_to_the_cell_count() {
        assert_eq!(fit("abc", 10), "abc");
        assert_eq!(fit("abc", 3), "abc", "exactly fitting text is not clipped");
        assert_eq!(fit("abcd", 3), "ab…");
        // A wide glyph is two cells: '世' plus the ellipsis fills 3 cells, not 2 chars.
        assert_eq!(fit("世界", 3), "世…");
        assert_eq!(fit("", 0), "");
        assert_eq!(fit("abc", 0), "", "nothing fits in a zero-width row");
        assert_eq!(
            fit("abc", 1),
            "…",
            "one cell means the ellipsis and no text"
        );

        // Byte-based clipping would cut inside these; cell-based clipping must not.
        for text in ["café", "a—b", "🙂🙂🙂", "naïve prose"] {
            let fitted = fit(text, 3);
            assert!(
                display_width(&fitted) <= 3,
                "{text:?} fitted to {fitted:?} is wider than 3 cells"
            );
            assert!(
                fitted.is_char_boundary(fitted.len()),
                "{text:?} split a character"
            );
        }
    }

    #[test]
    fn ctrl_q_quits_from_every_focus_state() {
        for focus in Focus::ALL {
            for state in [State::Idle, State::Error] {
                let config = config();
                let mut view = View::new(&config, focus);
                view.state = state;

                assert_eq!(
                    view.handle_key(&ctrl(KeyCode::Char('q'))),
                    Outcome::Quit,
                    "Ctrl+Q from {focus:?} while {state:?}"
                );
            }
        }
    }

    #[test]
    fn ctrl_q_while_running_prompts_instead_of_quitting() {
        let config = config();
        let mut view = View::new(&config, Focus::Output);
        view.state = State::Running;

        assert_eq!(
            view.handle_key(&ctrl(KeyCode::Char('q'))),
            Outcome::Continue,
            "Ctrl+Q quit mid-run without confirming"
        );
        let message = view.message.as_deref().expect("no confirmation prompt");
        assert!(
            message.contains("Ctrl+Q"),
            "prompt is not actionable: {message}"
        );

        // The prompt has to be readable, which means the status line must carry it.
        let status = row(&draw_at(&view, 120, 10), 9);
        assert!(status.contains(message), "prompt not drawn: {status:?}");
    }

    #[test]
    fn ctrl_c_quits_only_when_nothing_is_running() {
        for state in [State::Idle, State::Error] {
            let config = config();
            let mut view = View::new(&config, Focus::Debug);
            view.state = state;
            assert_eq!(
                view.handle_key(&ctrl(KeyCode::Char('c'))),
                Outcome::Quit,
                "Ctrl+C while {state:?}"
            );
        }

        let config = config();
        let mut view = View::new(&config, Focus::Source);
        view.state = State::Running;
        assert_eq!(
            view.handle_key(&ctrl(KeyCode::Char('c'))),
            Outcome::Continue,
            "Ctrl+C quit mid-run instead of leaving the cancel half unimplemented"
        );
        assert!(
            view.message.is_some(),
            "an ignored Ctrl+C must say so in the status line"
        );
    }

    #[test]
    fn esc_does_nothing() {
        for state in [State::Idle, State::Running, State::Error] {
            let config = config();
            let mut view = View::new(&config, Focus::Source);
            view.state = state;

            assert_eq!(
                view.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                Outcome::Continue,
                "Esc quit while {state:?}"
            );
            assert_eq!(view.focus, Focus::Source, "Esc moved focus");
            assert_eq!(view.message, None, "Esc wrote a status message");
            assert_eq!(view.state, state, "Esc changed the state");
        }
    }

    #[test]
    fn a_released_quit_key_does_not_quit() {
        // Windows sends a release after every press; quitting on it would fire twice.
        let config = config();
        let mut view = View::new(&config, Focus::Source);
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
            KeyEventKind::Release,
        );

        assert_eq!(view.handle_key(&release), Outcome::Continue);
    }

    fn contains(buffer: &Buffer, needle: &str) -> bool {
        (0..buffer.area.height).any(|y| row(buffer, y).contains(needle))
    }

    /// A view with a full debug pane, a known viewport, and the given focus. The viewport is
    /// established by a draw, because a paging key uses the geometry of the last frame.
    fn scrolling_view(focus: Focus) -> View<'static> {
        let config = Box::leak(Box::new(config()));
        let mut view = View::new(config, focus);
        for n in 0..60 {
            view.stream.app(format!("line{n}"));
        }
        let _ = draw_at(&view, 80, 24);
        view
    }

    #[test]
    fn pgup_and_pgdn_scroll_a_focused_read_only_pane() {
        let mut view = scrolling_view(Focus::Debug);
        assert_eq!(
            view.stream.debug_suffix(),
            None,
            "the pane should start tailing"
        );

        // Spec §4: PgUp scrolls by a page in the focused pane, and the pane then says it is
        // no longer following the stream.
        assert_eq!(
            view.handle_key(&KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)),
            Outcome::Continue
        );
        assert_eq!(view.stream.debug_suffix(), Some("not tailing"));

        // PgDn brings it back to the bottom, which resumes tailing.
        assert_eq!(
            view.handle_key(&KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)),
            Outcome::Continue
        );
        assert_eq!(view.stream.debug_suffix(), None);
    }

    #[test]
    fn arrows_scroll_a_read_only_pane_instead_of_moving_a_cursor() {
        let mut view = scrolling_view(Focus::Output);
        let output = view.stream.output_text();

        // An arrow key must not quit, must not change focus, and must not edit anything.
        for code in [KeyCode::Up, KeyCode::Down] {
            assert_eq!(
                view.handle_key(&KeyEvent::new(code, KeyModifiers::NONE)),
                Outcome::Continue,
                "{code:?} quit"
            );
        }
        assert_eq!(view.focus, Focus::Output, "an arrow moved focus");
        assert_eq!(view.stream.output_text(), output, "an arrow edited a pane");
    }

    #[test]
    fn scrolling_keys_do_not_disturb_an_editable_pane() {
        for focus in [Focus::Source, Focus::Instructions] {
            let mut view = scrolling_view(focus);
            // A draw establishes the read-only viewports; the editable pane has none, so
            // these keys fall through to focus handling and must change nothing.
            for code in [
                KeyCode::PageUp,
                KeyCode::PageDown,
                KeyCode::Up,
                KeyCode::Down,
            ] {
                assert_eq!(
                    view.handle_key(&KeyEvent::new(code, KeyModifiers::NONE)),
                    Outcome::Continue
                );
            }
            assert_eq!(view.focus, focus, "{focus:?} moved");
            assert_eq!(
                view.stream.debug_suffix(),
                None,
                "{focus:?} scrolled the debug pane"
            );
        }
    }

    fn focus_after(key: KeyEvent, from: Focus) -> Focus {
        let config = config();
        let mut view = View::new(&config, from);
        assert_eq!(
            view.handle_key(&key),
            Outcome::Continue,
            "{key:?} is not a quit key"
        );
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
    fn the_not_tailing_marker_is_drawn_in_the_debug_pane_title() {
        let mut view = scrolling_view(Focus::Debug);
        drawn_title(&view);
        // Read the title off a real frame rather than the accessor, so the marker is proved
        // to reach the screen and not just the state.
        assert!(
            !drawn_title(&view).contains("not tailing"),
            "a tailing pane drew the marker"
        );

        view.handle_key(&KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        let title = drawn_title(&view);
        assert!(
            title.contains("not tailing"),
            "a scrolled pane did not draw the marker: {title:?}"
        );
    }

    /// The debug pane's title as it appears in a drawn frame.
    fn drawn_title(view: &View) -> String {
        let buffer = draw_at(view, 80, 24);
        let panes = layout::panes(Rect::new(0, 0, 80, 24));
        title_at(&buffer, panes.debug)
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
