//! Soft wrapping for the editable panes: logical lines to visual rows, and the movement
//! that follows those rows instead of the lines (spec §4).
//!
//! [`crate::editor::Editor`] is logical-line state. It counts **chars** per line, because
//! prose is multi-byte and a byte offset can split a character, and it never sees a width,
//! which keeps it right for editing and useless for a screen. This module is the width-aware
//! layer above it: rows are recomputed from `(buffer, width)` on every draw and the buffer is
//! never reflowed into them. Wrapping is a rendering decision, so a terminal resize must not
//! be able to edit the user's text, and a mistake here must be recoverable by redrawing.
//!
//! Two widths are in play and are never mixed. The editor measures in chars; the terminal
//! measures in cells, where a wide glyph takes two. A row is a *char range* whose width is
//! checked in cells, and a cursor column is converted to a cell offset inside its row on
//! demand. That split is what keeps `Right`/`Left` exact at a wrap point while a CJK glyph
//! still occupies the right number of cells.
//!
//! Decisions the ticket left open, made once and tested:
//!
//! - **Wrap point side.** A cursor on the first char of the later row belongs to the later
//!   row, so `Right` then `Left` around a wrap returns to the same cell.
//! - **Width 0.** Every logical line is one empty visual row: there is no cell to place any
//!   text in, so drawing nothing is the only answer that neither panics nor loops.
//! - **The space that ends a row** is consumed: it is not drawn, and neither are the spaces
//!   that follow it, so the next row starts at a word rather than losing width to blanks.
//!   The char stays in the buffer; only the drawing drops it.
//! - **A glyph wider than the whole pane** gets its own row. Dropping it would truncate text
//!   (spec §4), and a row that fits nothing would make no progress.
//! - **The caret at the end of a line that fills every cell** is one past the last column
//!   of the pane, which no terminal can show. The arithmetic keeps that cell — it is a real
//!   position — and only the drawn position is clamped onto the last column, which is what
//!   a terminal does at the edge anyway.

// The pane wiring that consumes this API is a later ticket, so nothing outside the tests
// calls it yet. The attribute goes away with that ticket.
#![allow(dead_code)]

use std::cell::Cell;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

use crate::editor::{Cursor, Editor};

/// One visual row: a char range of one logical line, plus where that range sits in bytes.
///
/// Char indices are what the cursor speaks, so they are the public ones; the byte offsets are
/// private bookkeeping for [`VisualRow::text`], which would otherwise have to rescan the line
/// from its start for every row it draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualRow {
    /// Index into [`Editor::lines`].
    pub line: usize,
    /// First char on this row, as a char index into the line.
    pub start: usize,
    /// One past the last char on this row. Never less than [`VisualRow::start`].
    pub end: usize,
    start_byte: usize,
    end_byte: usize,
}

impl VisualRow {
    /// The chars this row draws, as a slice of its logical line. Empty for an empty row.
    pub fn text<'a>(&self, line: &'a str) -> &'a str {
        &line[self.start_byte..self.end_byte]
    }

    /// Zero for an empty row, in which case nothing on it is drawn.
    pub fn chars(&self) -> usize {
        self.end - self.start
    }
}

/// Where the cursor is on screen: a row in [`buffer_rows`] plus a cell offset in that row.
///
/// The offset can equal the row's width at the end of a logical line — that is the cell a
/// caret sits in when it is past the last character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualCursor {
    pub row: usize,
    pub column: usize,
}

/// The visual rows of one logical line, wrapped at `width` display cells.
///
/// The returned rows carry `line: 0`, since a line wrapped on its own has no index;
/// [`buffer_rows`] is the numbered version. Rows are ordered, non-overlapping and cover the
/// line except for whitespace consumed at a wrap point, so no non-space char is ever dropped.
pub fn line_rows(text: &str, width: u16) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    push_line(&mut rows, 0, text, width);
    rows
}

/// Every visual row of the buffer, in reading order, with each row numbered by its line.
///
/// Recomputed per draw on purpose: caching rows would be a second copy of the buffer that
/// every edit has to invalidate correctly, and wrapping a pane's worth of text is cheap next
/// to the drawing itself. The buffer is only read.
pub fn buffer_rows(editor: &Editor, width: u16) -> Vec<VisualRow> {
    let lines = editor.lines();
    let mut rows = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        push_line(&mut rows, index, line, width);
    }
    rows
}

/// Appends `width`-wrapped rows for one logical line.
///
/// The loop is written so every pass consumes at least one char: the empty pane and the
/// over-wide glyph both used to be ways to emit a row that fits nothing and spin forever.
fn push_line(out: &mut Vec<VisualRow>, line: usize, text: &str, width: u16) {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let count = chars.len();
    let cells = usize::from(width);
    // Nothing to draw: an empty line, or a pane with no cells in it at all.
    if count == 0 || cells == 0 {
        out.push(VisualRow {
            line,
            start: 0,
            end: 0,
            start_byte: 0,
            end_byte: 0,
        });
        return;
    }

    let mut start = 0;
    while start < count {
        // One pass per row, and each row is a contiguous run of `chars`, so a line costs
        // O(chars) even when it wraps many times. Nothing here allocates per row.
        // The furthest char index that still fits, and the cells it uses.
        let mut limit = start;
        let mut used = 0;
        while limit < count {
            let next = char_cells(chars[limit].1);
            if used + next > cells {
                break;
            }
            used += next;
            limit += 1;
        }

        if limit == count {
            out.push(row_of(&chars, text, line, start, count));
            return;
        }
        if limit == start {
            // Not one glyph fits. It still has to be drawn (spec §4: never truncated), so it
            // gets a row to itself and the next pass starts after it.
            out.push(row_of(&chars, text, line, start, start + 1));
            start += 1;
            continue;
        }

        // The last space that fits, or the one sitting in the cell just past the last that
        // fits: either way that space ends the row, so the row keeps its width and the next
        // row begins with a word.
        let space = chars[start..=limit]
            .iter()
            .rposition(|&(_, c)| c == ' ')
            .map(|offset| start + offset);
        match space {
            Some(space) if space > start => {
                out.push(row_of(&chars, text, line, start, space.min(limit)));
                start = space + 1;
                // Wrapped whitespace is not content: consuming the run keeps the next row
                // from starting with the cells it would otherwise waste.
                while start < count && chars[start].1 == ' ' {
                    start += 1;
                }
            }
            _ => {
                // No space to break at, so a word is longer than the row: hard-split it at
                // exactly the last cell that fits, with no hyphen and no ellipsis.
                out.push(row_of(&chars, text, line, start, limit));
                start = limit;
            }
        }
    }
}

/// Builds one row from a char range, translating the range to bytes once.
fn row_of(chars: &[(usize, char)], text: &str, line: usize, start: usize, end: usize) -> VisualRow {
    VisualRow {
        line,
        start,
        end,
        start_byte: chars.get(start).map_or(text.len(), |&(byte, _)| byte),
        end_byte: chars.get(end).map_or(text.len(), |&(byte, _)| byte),
    }
}

/// Display cells taken by one char.
///
/// Printable ASCII is one cell by definition and is the whole hot path, so prose costs no
/// measurement. A control char (a tab, say) costs nothing: ratatui's `set_stringn` drops it,
/// so a row that counted it would wrap early and leave a blank cell the renderer never fills.
/// Everything else asks ratatui's own width rule, the one the renderer uses to place the next
/// cell, so a CJK glyph costs two and a combining mark costs nothing.
fn char_cells(c: char) -> usize {
    if c.is_ascii_control() {
        0
    } else if c.is_ascii() {
        1
    } else {
        // On the stack: this runs per non-ASCII char on every draw, and wrapping must not
        // allocate per character.
        let mut buf = [0u8; 4];
        Line::from(&*c.encode_utf8(&mut buf)).width()
    }
}

/// Cells used by the chars in `start..end`.
fn cell_offset(text: &str, start: usize, end: usize) -> usize {
    text.chars()
        .skip(start)
        .take(end - start)
        .map(char_cells)
        .sum()
}

/// Where the cursor sits in `rows`, given the editor's logical cursor.
///
/// A cursor column on a wrap point has two candidate rows; it takes the later one, so moving
/// right into it and left out of it lands on the same cell. The last row of a line also owns
/// the position one past its last char, which is where the caret rests at end of line.
fn project(rows: &[VisualRow], editor: &Editor) -> VisualCursor {
    let cursor = editor.cursor();
    let line = &editor.lines()[cursor.line()];
    // The first row always starts at column 0, so there is always a candidate.
    let mut found = 0;
    for (index, row) in rows.iter().enumerate() {
        if row.line > cursor.line() {
            break;
        }
        if row.line == cursor.line() && row.start <= cursor.column() {
            found = index;
        }
    }
    let row = rows[found];
    // A column can sit past the drawn range when the row ended in a consumed space; it
    // still belongs to this row, at the cell after the last drawn one.
    let end = cursor.column().min(row.end);
    VisualCursor {
        row: found,
        column: cell_offset(line, row.start, end),
    }
}

/// The cursor's visual position for an editor and a width.
pub fn cursor_position(editor: &Editor, width: u16) -> VisualCursor {
    let rows = buffer_rows(editor, width);
    project(&rows, editor)
}

/// The logical cursor nearest to cell `goal` on row `index`.
///
/// The result always projects back to row `index`, which is what keeps one `Down` from
/// skipping a row: on any row but the last of its line, the position one past the last cell
/// *is* the first cell of the next row, so the column stops one char short of the wrap.
fn cursor_in_row(rows: &[VisualRow], editor: &Editor, index: usize, goal: usize) -> Cursor {
    let row = rows[index];
    let last_of_line = rows.get(index + 1).is_none_or(|next| next.line != row.line);
    let line = &editor.lines()[row.line];

    let mut used = 0;
    let mut column = row.start;
    for (offset, c) in line.chars().skip(row.start).take(row.chars()).enumerate() {
        let cells = char_cells(c);
        // A wide glyph cannot be split: stopping before it is the only exact placement.
        if used + cells > goal {
            break;
        }
        used += cells;
        column = row.start + offset + 1;
    }
    if !last_of_line {
        // `column == row.end` would be the next row's first cell; the row cannot hold it.
        // Every non-final row draws at least one char, so this cannot underflow.
        column = column.min(row.end - 1);
    }
    Cursor::new(row.line, column)
}

/// Viewport and vertical-movement state for one editable pane.
///
/// The scroll offset lives here rather than in [`Editor`] because it depends on a width and
/// a height, and because the buffer must stay a pure text model. The sticky column lives here
/// for the same reason: it is a property of a *run of vertical moves*, not of the text, and
/// storing it in the editor would make it survive edits that should cancel it.
#[derive(Debug, Default, Clone)]
pub struct WrapState {
    /// First visible row, counted from the top of the buffer.
    scroll: usize,
    /// The cell column a vertical run is aiming at, with the cursor it was captured at.
    goal: Option<Goal>,
    /// The inner geometry of the last draw, in cells: `(width, height)`.
    ///
    /// A key arrives between draws, so a text operation has no other way to learn the width it
    /// must wrap at or the height it must page by. A `Cell` because `render` only needs to
    /// record it, and threading `&mut self` through the draw for a cache would be worse.
    viewport: Cell<(u16, u16)>,
}

/// The column a run of `Up`/`Down` is holding, keyed by the cursor it started from.
///
/// The key is what makes the stickiness safe: a horizontal move or an edit changes the
/// cursor, which retires the goal, while the run itself leaves the cursor exactly here.
#[derive(Debug, Clone, Copy)]
struct Goal {
    cursor: Cursor,
    width: u16,
    cell: usize,
}

impl WrapState {
    pub fn new() -> Self {
        Self::default()
    }

    /// First visible row, as last set. Not clamped here: the buffer can have shrunk since,
    /// and only a draw knows the width and height to clamp against.
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// The width of the last draw, for wrapping. Zero before the first draw, which is the
    /// same degenerate case a zero-width pane has: one empty row per line.
    pub fn viewport_width(&self) -> u16 {
        self.viewport.get().0
    }

    /// The height of the last draw, for paging.
    pub fn viewport_height(&self) -> u16 {
        self.viewport.get().1
    }

    /// One visual row up, holding the target cell column.
    pub fn move_up(&mut self, editor: &mut Editor, width: u16) {
        self.move_rows(editor, width, -1);
    }

    /// One visual row down, holding the target cell column.
    pub fn move_down(&mut self, editor: &mut Editor, width: u16) {
        self.move_rows(editor, width, 1);
    }

    /// `PgUp`: back by a viewport, less one row, so the rows on screen keep a line of
    /// context.
    pub fn page_up(&mut self, editor: &mut Editor, width: u16, height: u16) {
        self.page(editor, width, height, -1);
    }

    /// `PgDn`: forward by a viewport, less one row of overlap.
    pub fn page_down(&mut self, editor: &mut Editor, width: u16, height: u16) {
        self.page(editor, width, height, 1);
    }

    /// One page: the cursor moves a viewport's worth of rows and the window moves with it,
    /// so the cursor keeps its row on screen.
    ///
    /// The step is `height - 1` for the overlap. A one-row viewport has no row to spare for
    /// overlap, so it steps one row: a key that did nothing at all would look broken, and
    /// the overlap exists to preserve context, not to forbid scrolling.
    fn page(&mut self, editor: &mut Editor, width: u16, height: u16, direction: isize) {
        if height == 0 {
            return;
        }
        let height = usize::from(height);
        let step = height.saturating_sub(1).max(1);
        let rows = buffer_rows(editor, width);
        let current = project(&rows, editor);
        let delta = direction * step as isize;

        // A page is a vertical run like any other, so it holds the same sticky column: a
        // `PgDn` followed by `Down` keeps the cell instead of snapping to the first char.
        let goal = self.goal_cell(editor, width, current);
        let target = current.row.saturating_add_signed(delta).min(rows.len() - 1);
        let cursor = cursor_in_row(&rows, editor, target, goal);
        editor.set_cursor(cursor.line(), cursor.column());
        self.goal = Some(Goal {
            cursor: editor.cursor(),
            width,
            cell: goal,
        });

        // The window moves by the same page, clamped to the buffer; `scroll_to_cursor` then
        // repairs the one case where the cursor could not follow (the end of the text), and
        // it moves the minimum because the cursor is still inside every other window.
        let max_scroll = rows.len().saturating_sub(height);
        self.scroll = self.scroll.saturating_add_signed(delta).min(max_scroll);
        self.scroll_to_cursor(editor, width, height as u16);
    }

    /// The cell column a vertical run is aiming at.
    ///
    /// A run keeps the cell it started on. Without this, one `Down` over a short paragraph
    /// snaps the cursor to that line's end and every later `Down` — and the `Up` back —
    /// inherits the clamped column. A horizontal move or a width change makes the stored
    /// cursor stale, which is how the goal retires itself.
    fn goal_cell(&self, editor: &Editor, width: u16, current: VisualCursor) -> usize {
        match self.goal {
            Some(goal) if goal.cursor == editor.cursor() && goal.width == width => goal.cell,
            _ => current.column,
        }
    }

    fn move_rows(&mut self, editor: &mut Editor, width: u16, delta: isize) {
        let rows = buffer_rows(editor, width);
        let current = project(&rows, editor);
        let goal = self.goal_cell(editor, width, current);
        let target = current.row.saturating_add_signed(delta).min(rows.len() - 1);
        let cursor = cursor_in_row(&rows, editor, target, goal);
        editor.set_cursor(cursor.line(), cursor.column());
        self.goal = Some(Goal {
            cursor: editor.cursor(),
            width,
            cell: goal,
        });
    }

    /// Scrolls the minimum amount that puts the cursor's row inside a viewport of `height`
    /// rows. Not a recentre: `Down` at the bottom edge moves the view by one row.
    pub fn scroll_to_cursor(&mut self, editor: &Editor, width: u16, height: u16) {
        let height = usize::from(height);
        if height == 0 {
            return;
        }
        let rows = buffer_rows(editor, width);
        let cursor = project(&rows, editor);
        if cursor.row < self.scroll {
            self.scroll = cursor.row;
        } else if cursor.row >= self.scroll + height {
            self.scroll = cursor.row + 1 - height;
        }
        // The buffer can shrink under an existing offset (an edit, a resize). Clamping here
        // is what keeps the next `PgDn` at the end from leaving a blank pane.
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));
    }

    /// Draws the visible rows into `area` and puts the terminal cursor where
    /// [`cursor_position`] says it is.
    ///
    /// Reads `self` only: the caller scrolls first (see [`WrapState::scroll_to_cursor`]) so
    /// one draw cannot move the view. The cursor is drawn whenever it is in the window and
    /// left hidden otherwise, which is ratatui's default when it is not set.
    pub fn render(&mut self, frame: &mut Frame, editor: &Editor, area: Rect) {
        self.render_with_cursor(frame, editor, area, true);
    }

    /// Draws the pane without placing the cursor, for an editable pane that does not have
    /// focus.
    ///
    /// A pane drawn a moment before the focused one would otherwise leave the terminal cursor
    /// behind the wrong pane: the last `set_cursor_position` of a draw wins, and both editable
    /// panes run through this function.
    pub fn render_unfocused(&mut self, frame: &mut Frame, editor: &Editor, area: Rect) {
        self.render_with_cursor(frame, editor, area, false);
    }

    fn render_with_cursor(
        &mut self,
        frame: &mut Frame,
        editor: &Editor,
        area: Rect,
        focused: bool,
    ) {
        // Recorded before the degenerate-size early return: a zero-sized pane is still the
        // pane's geometry, and the next key has to work in it rather than in a stale one.
        self.viewport.set((area.width, area.height));
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Follow the cursor before drawing. Without this a buffer seeded with more rows than
        // the pane is tall draws from the top with the cursor off-screen, and the pane looks
        // as if it lost the user's position; a key that moves the cursor would then have to
        // be pressed twice to see any effect.
        self.scroll_to_cursor(editor, area.width, area.height);
        let rows = buffer_rows(editor, area.width);
        let height = usize::from(area.height);
        // Same clamp as `scroll_to_cursor`, for the case where the content changed after the
        // offset was set: drawing an empty pane would look like lost text.
        let scroll = self.scroll.min(rows.len().saturating_sub(height));
        let lines = editor.lines();
        {
            let buffer = frame.buffer_mut();
            for (offset, row) in rows.iter().skip(scroll).take(height).enumerate() {
                let y = area.y + offset as u16;
                // Capped at the pane so a glyph too wide for the width cannot paint over the
                // border; every other row fits by construction.
                buffer.set_stringn(
                    area.x,
                    y,
                    row.text(&lines[row.line]),
                    usize::from(area.width),
                    Style::default(),
                );
            }
        }

        if !focused {
            return;
        }
        let cursor = project(&rows, editor);
        if cursor.row >= scroll && cursor.row < scroll + height {
            let x = area.x + cursor.column.min(usize::from(area.width) - 1) as u16;
            let y = area.y + (cursor.row - scroll) as u16;
            frame.set_cursor_position((x, y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Position;

    fn editor(text: &str) -> Editor {
        Editor::from_text(text)
    }

    /// The char ranges of the rows of one line, which is the whole wrap decision.
    fn ranges(text: &str, width: u16) -> Vec<(usize, usize)> {
        line_rows(text, width)
            .iter()
            .map(|row| (row.start, row.end))
            .collect()
    }

    /// What each row actually draws, so a test reads like the pane it describes.
    fn drawn(text: &str, width: u16) -> Vec<String> {
        line_rows(text, width)
            .iter()
            .map(|row| row.text(text).to_owned())
            .collect()
    }

    fn visual(editor: &Editor, width: u16) -> (usize, usize) {
        let cursor = cursor_position(editor, width);
        (cursor.row, cursor.column)
    }

    fn at(text: &str, line: usize, column: usize) -> Editor {
        let mut editor = Editor::from_text(text);
        editor.set_cursor(line, column);
        assert_eq!(editor.cursor(), Cursor::new(line, column));
        editor
    }

    /// Asserts the logical cursor and, beside it, where that lands visually.
    fn assert_at(editor: &Editor, width: u16, line: usize, column: usize, row: usize, cell: usize) {
        assert_eq!(
            editor.cursor(),
            Cursor::new(line, column),
            "logical cursor for {:?} at width {width}",
            editor.text()
        );
        assert_eq!(
            visual(editor, width),
            (row, cell),
            "visual cursor for {:?} at width {width}",
            editor.text()
        );
    }

    struct Drawn {
        buffer: Buffer,
        cursor: Position,
        visible: bool,
    }

    impl Drawn {
        fn row(&self, y: u16) -> String {
            (0..self.buffer.area.width)
                .map(|x| self.buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        }
    }

    fn draw_in(editor: &Editor, state: &mut WrapState, area: Rect) -> Drawn {
        let mut terminal =
            Terminal::new(TestBackend::new(area.right().max(1), area.bottom().max(1))).unwrap();
        terminal
            .draw(|frame| state.render(frame, editor, area))
            .unwrap();
        let backend = terminal.backend();
        Drawn {
            buffer: backend.buffer().clone(),
            cursor: backend.cursor_position(),
            visible: backend.cursor_visible(),
        }
    }

    fn draw(editor: &Editor, state: &mut WrapState, width: u16, height: u16) -> Drawn {
        draw_in(editor, state, Rect::new(0, 0, width, height))
    }

    // --- wrapping ---------------------------------------------------------

    #[test]
    fn a_line_shorter_than_the_width_is_one_row() {
        assert_eq!(ranges("short", 80), [(0, 5)]);
        assert_eq!(drawn("short", 80), ["short"]);
        // Exactly the width still fits in one row.
        assert_eq!(ranges("abcde", 5), [(0, 5)]);
    }

    #[test]
    fn an_empty_line_is_one_empty_row() {
        assert_eq!(ranges("", 10), [(0, 0)]);
        assert_eq!(drawn("", 10), [""]);
    }

    #[test]
    fn width_zero_is_one_empty_row_per_line_and_never_loops() {
        // Nothing can be drawn in zero cells, so the only sane answer is one empty row per
        // line: the cursor still needs a row to live on.
        assert_eq!(ranges("abcdef", 0), [(0, 0)]);
        assert_eq!(ranges("", 0), [(0, 0)]);

        let editor = editor("ab\ncd\n");
        let rows = buffer_rows(&editor, 0);
        assert_eq!(rows.len(), 3, "one row per logical line");
        assert_eq!(
            rows.iter().map(|row| row.line).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        for row in &rows {
            assert_eq!(row.chars(), 0);
            assert_eq!(row.text(&editor.lines()[row.line]), "");
        }
    }

    #[test]
    fn width_one_gives_every_char_its_own_row() {
        assert_eq!(ranges("abc", 1), [(0, 1), (1, 2), (2, 3)]);
        assert_eq!(drawn("abc", 1), ["a", "b", "c"]);
    }

    #[test]
    fn a_soft_wrap_breaks_at_the_last_fitting_space() {
        // 11 chars at width 10: "hello worl" fit, so the space at index 5 is the break.
        assert_eq!(ranges("hello world", 10), [(0, 5), (6, 11)]);
        assert_eq!(drawn("hello world", 10), ["hello", "world"]);
    }

    #[test]
    fn a_space_in_the_last_cell_ends_the_row_instead_of_starting_the_next() {
        // The space at index 9 fills the row exactly; carrying it over would waste a cell
        // of the next row, so it ends this one.
        let text = "aaaaaaaaa bbbbbbbbb";
        assert_eq!(ranges(text, 10), [(0, 9), (10, 19)]);
        assert_eq!(drawn(text, 10), ["aaaaaaaaa", "bbbbbbbbb"]);
        assert_eq!(line_rows(text, 10).len(), 2, "exactly two visual rows");
    }

    #[test]
    fn a_line_wraps_to_exactly_three_rows() {
        let text = "aaaaaaaaa bbbbbbbbb ccccccccc";
        assert_eq!(ranges(text, 10), [(0, 9), (10, 19), (20, 29)]);
        assert_eq!(line_rows(text, 10).len(), 3);
    }

    #[test]
    fn the_spaces_at_a_break_are_consumed_not_drawn() {
        // One space at the break: the row takes the five that fit, the space is consumed
        // with the break, and the next row starts on 'w'.
        assert_eq!(ranges("hello world", 6), [(0, 5), (6, 11)]);
        // A run of spaces: the break consumes the one holding the last cell and the rest of
        // the run, so the next row does not begin with blanks that would waste its width.
        assert_eq!(ranges("hello  world", 5), [(0, 5), (7, 12)]);
        assert_eq!(drawn("hello  world", 5), ["hello", "world"]);
        // A trailing space is not a row of its own.
        assert_eq!(ranges("hello ", 5), [(0, 5)]);
    }

    #[test]
    fn a_word_longer_than_the_row_is_hard_split_without_dropping_anything() {
        let text = "abcdefghijklm";
        assert_eq!(ranges(text, 5), [(0, 5), (5, 10), (10, 13)]);
        assert_eq!(drawn(text, 5), ["abcde", "fghij", "klm"]);
        // No hyphen, no ellipsis: the split is the whole of the text, in order.
        assert_eq!(drawn(text, 5).join(""), text);
    }

    #[test]
    fn a_long_word_inside_a_sentence_is_hard_split_and_the_tail_carries_on() {
        // "abc" fits, then "defghijklmnop" is longer than the 4 remaining cells, so it is
        // split and the rest of the line keeps wrapping after it.
        assert_eq!(ranges("abc defghijklmnop", 10), [(0, 3), (4, 14), (14, 17)]);
        assert_eq!(drawn("abc defghijklmnop", 10), ["abc", "defghijklm", "nop"]);
    }

    #[test]
    fn a_word_exactly_one_cell_longer_than_the_width_wraps_to_two_rows() {
        assert_eq!(ranges("abcdef", 5), [(0, 5), (5, 6)]);
    }

    #[test]
    fn a_glyph_wider_than_the_row_gets_its_own_row() {
        // 世 is two cells, so at width 4 the row holds exactly two of them.
        assert_eq!(ranges("世界世界", 4), [(0, 2), (2, 4)]);
        assert_eq!(drawn("世界世界", 4), ["世界", "世界"]);

        // At width 1 no glyph fits; each still has to be drawn, so each takes a row.
        assert_eq!(ranges("世界", 1), [(0, 1), (1, 2)]);
        assert_eq!(drawn("世界", 1), ["世", "界"]);
    }

    #[test]
    fn a_control_char_takes_no_cell_because_the_renderer_drops_it() {
        // A tab copied out of a document: ratatui draws nothing for it, so counting a cell
        // here would wrap a row the pane could have shown in full and leave a blank cell.
        assert_eq!(ranges("a\tb", 2), [(0, 3)]);
        assert_eq!(drawn("a\tb", 2), ["a\tb"]);
        // It costs nothing, but it is still a char, so the split lands after 'b'.
        assert_eq!(ranges("a\tbc", 2), [(0, 3), (3, 4)]);
        assert_eq!(drawn("a\tbc", 2), ["a\tb", "c"]);
    }

    #[test]
    fn no_row_is_wider_than_the_width_in_cells() {
        for (text, width) in [
            ("hello world again", 7),
            ("café — naïve prose", 5),
            ("世界 hello 世界", 4),
            ("aaaaaaaaaaaaaaaaaaaa", 3),
        ] {
            for row in line_rows(text, width) {
                let cells = Line::from(row.text(text)).width();
                assert!(
                    cells <= usize::from(width),
                    "{:?} is {cells} cells wide in a {width}-cell row of {text:?}",
                    row.text(text)
                );
            }
        }
    }

    #[test]
    fn no_non_space_char_is_lost_or_duplicated() {
        // Spaces are the only thing wrapping may drop, and only at a break. Anything else
        // going missing is the truncation spec §4 forbids.
        let nonspace = |text: &str| text.chars().filter(|c| *c != ' ').collect::<Vec<char>>();
        for (text, width) in [
            ("hello world", 10),
            ("hello world", 5),
            ("hello  world", 5),
            ("abcdefghijklm", 4),
            ("abc defghijklmnop qrs", 6),
            ("café — naïve prose", 3),
            ("世界 hello 世界", 2),
            ("    indented text here", 4),
            ("trailing spaces   ", 6),
        ] {
            let rows = line_rows(text, width);
            let reconstructed = rows.iter().map(|row| row.text(text)).collect::<String>();
            assert_eq!(
                nonspace(&reconstructed),
                nonspace(text),
                "{text:?} at width {width} lost or repeated text: {rows:?}"
            );
        }
    }

    #[test]
    fn rows_are_ordered_and_start_where_the_previous_one_ended() {
        for (text, width) in [
            ("hello world again and again", 7),
            ("abcdefghijklm", 4),
            ("café — naïve", 3),
        ] {
            let rows = line_rows(text, width);
            assert_eq!(rows[0].start, 0, "{text:?} does not start at 0");
            for pair in rows.windows(2) {
                assert!(
                    pair[1].start > pair[0].start,
                    "rows of {text:?} do not advance: {rows:?}"
                );
                assert!(
                    pair[1].start >= pair[0].end,
                    "rows of {text:?} overlap: {rows:?}"
                );
            }
            // Every char after the first row's start is owned by some row, except spaces
            // dropped at a break.
            for row in &rows {
                assert!(row.end <= text.chars().count(), "{row:?} past the line");
            }
        }
    }

    #[test]
    fn buffer_rows_numbers_rows_by_their_logical_line() {
        let editor = editor("ab\n\nabcd");
        let rows = buffer_rows(&editor, 3);

        assert_eq!(editor.line_count(), 3);
        assert_eq!(
            rows.iter()
                .map(|row| (row.line, row.start))
                .collect::<Vec<_>>(),
            [(0, 0), (1, 0), (2, 0), (2, 3)]
        );
        // An empty line still owns one row, so the cursor always has somewhere to be.
        assert_eq!(rows[1].chars(), 0);
    }

    // --- cursor projection ------------------------------------------------

    #[test]
    fn the_cursor_column_is_a_cell_offset_inside_its_row() {
        let editor = at("hello world", 0, 3);

        assert_eq!(visual(&editor, 10), (0, 3));
        assert_eq!(visual(&editor, 5), (0, 3));
    }

    #[test]
    fn a_cursor_on_a_wrap_point_belongs_to_the_later_row() {
        let editor = at("abcdefghij", 0, 5);

        // Column 5 is the first cell of the second row, not the cell after row 0's last one.
        assert_eq!(visual(&editor, 5), (1, 0));
        // Column 4 is the last cell of row 0.
        assert_eq!(visual(&at("abcdefghij", 0, 4), 5), (0, 4));
    }

    #[test]
    fn right_then_left_at_a_wrap_point_returns_to_the_same_cell() {
        let mut editor = at("abcdefghij", 0, 5);
        assert_eq!(visual(&editor, 5), (1, 0));

        editor.move_right();
        assert_eq!(visual(&editor, 5), (1, 1));
        editor.move_left();
        assert_eq!(visual(&editor, 5), (1, 0), "the wrap point moved");
    }

    #[test]
    fn a_cursor_on_a_consumed_space_sits_after_the_rows_last_cell() {
        // "hello world" at width 10 consumes the space at index 5. A cursor on it is still
        // on row 0, just past the drawn cells.
        let editor = at("hello world", 0, 5);

        assert_eq!(visual(&editor, 10), (0, 5));
        // The first char of the next row is the later row's column 0.
        assert_eq!(visual(&at("hello world", 0, 6), 10), (1, 0));
    }

    #[test]
    fn end_of_line_is_the_cell_after_the_last_char() {
        let editor = at("abcdefghij", 0, 10);

        assert_eq!(visual(&editor, 5), (1, 5));
        assert_eq!(visual(&at("abc", 0, 3), 10), (0, 3));
    }

    #[test]
    fn cursor_positions_are_exact_at_width_one() {
        for (column, expected) in [(0, (0, 0)), (1, (1, 0)), (2, (2, 0)), (3, (2, 1))] {
            assert_eq!(
                visual(&at("abc", 0, column), 1),
                expected,
                "column {column} at width 1"
            );
        }
    }

    #[test]
    fn cursor_positions_are_exact_at_width_ten() {
        for (column, expected) in [
            (0, (0, 0)),
            (5, (0, 5)),
            (6, (1, 0)),
            (10, (1, 4)),
            (11, (1, 5)),
        ] {
            assert_eq!(
                visual(&at("hello world", 0, column), 10),
                expected,
                "column {column} at width 10"
            );
        }
    }

    #[test]
    fn cursor_positions_are_exact_at_a_width_narrower_than_the_longest_word() {
        // Width 4, "abc defghij": rows are "abc" | "defg" | "hij".
        for (column, expected) in [
            (0, (0, 0)),
            (3, (0, 3)),
            (4, (1, 0)),
            (7, (1, 3)),
            (8, (2, 0)),
            (11, (2, 3)),
        ] {
            assert_eq!(
                visual(&at("abc defghij", 0, column), 4),
                expected,
                "column {column} at width 4"
            );
        }
    }

    #[test]
    fn cursor_rows_are_global_across_the_whole_buffer() {
        // Line 0 is two rows, so line 1 starts on global row 2.
        let editor = at("abcdefghij\nx", 1, 0);
        assert_eq!(visual(&editor, 5), (2, 0));

        let editor = at("abc\nde", 1, 1);
        assert_eq!(visual(&editor, 10), (1, 1));
    }

    #[test]
    fn cursor_projection_is_panic_free_at_width_zero() {
        let editor = at("abc\ndef", 1, 2);

        assert_eq!(visual(&editor, 0), (1, 0));
    }

    // --- visual-row movement ---------------------------------------------

    #[test]
    fn down_moves_one_visual_row_inside_a_wrapped_line() {
        let mut editor = at("abcdefghijklm", 0, 2);
        let mut state = WrapState::new();

        state.move_down(&mut editor, 5);
        assert_at(&editor, 5, 0, 7, 1, 2);

        state.move_down(&mut editor, 5);
        assert_at(&editor, 5, 0, 12, 2, 2);

        // The last row is the end of the buffer, so a further `Down` stays put.
        state.move_down(&mut editor, 5);
        assert_at(&editor, 5, 0, 12, 2, 2);
    }

    #[test]
    fn up_moves_one_visual_row_inside_a_wrapped_line() {
        let mut editor = at("abcdefghijklm", 0, 12);
        let mut state = WrapState::new();

        state.move_up(&mut editor, 5);
        assert_at(&editor, 5, 0, 7, 1, 2);

        state.move_up(&mut editor, 5);
        assert_at(&editor, 5, 0, 2, 0, 2);

        state.move_up(&mut editor, 5);
        assert_at(&editor, 5, 0, 2, 0, 2);
    }

    #[test]
    fn down_inside_a_hard_split_word_stays_inside_that_word() {
        // Width 5, one 13-char word: rows "abcde" | "fghij" | "klm".
        let mut editor = at("abcdefghijklm", 0, 1);
        let mut state = WrapState::new();

        state.move_down(&mut editor, 5);
        assert_at(&editor, 5, 0, 6, 1, 1);

        state.move_down(&mut editor, 5);
        assert_at(&editor, 5, 0, 11, 2, 1);
    }

    #[test]
    fn down_crosses_a_logical_line_at_the_matching_visual_column() {
        let mut editor = at("abcde\nfghij", 0, 2);
        let mut state = WrapState::new();

        state.move_down(&mut editor, 5);
        assert_at(&editor, 5, 1, 2, 1, 2);
    }

    #[test]
    fn up_crosses_into_the_previous_lines_last_visual_row() {
        let mut editor = at("abcdefghij\nxy", 1, 1);
        let mut state = WrapState::new();

        state.move_up(&mut editor, 5);
        assert_at(&editor, 5, 0, 6, 1, 1);
        state.move_up(&mut editor, 5);
        assert_at(&editor, 5, 0, 1, 0, 1);
    }

    #[test]
    fn a_row_that_cannot_hold_the_goal_stops_one_cell_short() {
        // Line 0 is one row of 5 cells, so the cursor sits at cell 5. Line 1 at width 6 has
        // rows "hello" (0..5) and "world" (6..11). Cell 5 of the first row is exactly the
        // first cell of the second row, so `Down` must stop at cell 4 rather than land on a
        // cell that projects one row lower and skip it.
        let mut editor = at("abcde\nhello world", 0, 5);
        let mut state = WrapState::new();

        state.move_down(&mut editor, 6);
        assert_at(&editor, 6, 1, 4, 1, 4);

        // The last row of a line has no such successor, so the goal applies in full there.
        state.move_down(&mut editor, 6);
        assert_at(&editor, 6, 1, 11, 2, 5);
    }

    #[test]
    fn a_target_column_survives_a_run_of_moves_through_a_short_line() {
        // The whole point of the sticky column: passing over "ab" must not permanently
        // snap the cursor to column 2.
        let mut editor = at("abcdefghij\nab\nabcdefghij", 0, 8);
        let mut state = WrapState::new();

        state.move_down(&mut editor, 5);
        assert_at(&editor, 5, 1, 2, 2, 2);

        state.move_down(&mut editor, 5);
        assert_at(&editor, 5, 2, 3, 3, 3);
    }

    #[test]
    fn a_horizontal_move_retires_the_sticky_column() {
        let mut editor = at("abcdefghij\nab\nabcdefghij", 0, 8);
        let mut state = WrapState::new();
        state.move_down(&mut editor, 5);
        assert_eq!(editor.cursor(), Cursor::new(1, 2));

        editor.move_left();
        assert_eq!(editor.cursor(), Cursor::new(1, 1));

        // The remembered column was 3; the cursor is now on a new column, so 3 must be
        // forgotten and the run continues from 1.
        state.move_down(&mut editor, 5);
        assert_at(&editor, 5, 2, 1, 3, 1);
    }

    #[test]
    fn a_width_change_retires_the_sticky_column() {
        let mut editor = at("abcdefghij\nab\nabcdefghij", 0, 8);
        let mut state = WrapState::new();
        state.move_down(&mut editor, 5);
        assert_eq!(editor.cursor(), Cursor::new(1, 2));

        // At width 10 each of these lines is a single row, so the cursor is one row down
        // from line 0 and the remembered cell 3 does not apply to the new rows.
        state.move_down(&mut editor, 10);
        assert_at(&editor, 10, 2, 2, 2, 2);
    }

    #[test]
    fn vertical_movement_at_the_buffer_edges_does_not_move() {
        let mut editor = editor("one");
        let mut state = WrapState::new();

        for _ in 0..3 {
            state.move_up(&mut editor, 10);
            assert_at(&editor, 10, 0, 3, 0, 3);
            state.move_down(&mut editor, 10);
            assert_at(&editor, 10, 0, 3, 0, 3);
        }
    }

    #[test]
    fn vertical_movement_at_width_zero_steps_logical_lines() {
        let mut editor = at("ab\ncd", 0, 1);
        let mut state = WrapState::new();

        state.move_down(&mut editor, 0);
        assert_eq!(editor.cursor(), Cursor::new(1, 0));
        state.move_up(&mut editor, 0);
        assert_eq!(editor.cursor(), Cursor::new(0, 0));
    }

    #[test]
    fn movement_never_changes_the_text() {
        let mut editor = editor("abcdefghijklm\nab\ncafé — naïve prose");
        let before = editor.text();
        let mut state = WrapState::new();

        for width in [1, 3, 5, 10, 80, 0] {
            for _ in 0..4 {
                state.move_down(&mut editor, width);
                state.move_up(&mut editor, width);
            }
        }
        assert_eq!(editor.text(), before);
    }

    #[test]
    fn a_page_down_keeps_the_sticky_column_for_the_next_down() {
        // Line 1 is one row of 2 cells; a `Down` over it would snap to cell 2. `PgDn` must
        // not do that to the run either.
        let mut editor = at("abcdefghij\nab\nabcdefghij", 0, 8);
        let mut state = WrapState::new();

        state.page_down(&mut editor, 5, 2);
        assert_at(&editor, 5, 1, 2, 2, 2);

        state.move_down(&mut editor, 5);
        assert_at(&editor, 5, 2, 3, 3, 3);
    }

    // --- scrolling and paging --------------------------------------------

    fn numbered(count: usize) -> Editor {
        let lines = (0..count)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        Editor::from_text(&lines)
    }

    #[test]
    fn scroll_to_cursor_moves_the_minimum_number_of_rows() {
        let mut editor = numbered(10);
        editor.set_cursor(0, 0);
        let mut state = WrapState::new();

        // One row per line at this width; a 3-row viewport.
        state.scroll_to_cursor(&editor, 80, 3);
        assert_eq!(state.scroll(), 0);

        editor.set_cursor(2, 0);
        state.scroll_to_cursor(&editor, 80, 3);
        assert_eq!(state.scroll(), 0, "row 2 is already visible");

        editor.set_cursor(3, 0);
        state.scroll_to_cursor(&editor, 80, 3);
        assert_eq!(state.scroll(), 1, "one row, not a recentre");

        editor.set_cursor(9, 0);
        state.scroll_to_cursor(&editor, 80, 3);
        assert_eq!(state.scroll(), 7);

        // Back up: the view follows only when the cursor leaves the window.
        editor.set_cursor(7, 0);
        state.scroll_to_cursor(&editor, 80, 3);
        assert_eq!(state.scroll(), 7, "row 7 is the top visible row");

        editor.set_cursor(6, 0);
        state.scroll_to_cursor(&editor, 80, 3);
        assert_eq!(state.scroll(), 6);
    }

    #[test]
    fn scroll_to_cursor_clamps_a_stale_offset_after_the_text_shrinks() {
        let mut editor = numbered(10);
        let mut state = WrapState::new();
        editor.set_cursor(9, 0);
        state.scroll_to_cursor(&editor, 80, 3);
        assert_eq!(state.scroll(), 7);

        // The buffer becomes too short to scroll: keeping offset 7 would show nothing.
        let short = numbered(2);
        state.scroll_to_cursor(&short, 80, 3);
        assert_eq!(state.scroll(), 0);
    }

    #[test]
    fn a_zero_height_viewport_is_a_no_op() {
        let mut editor = numbered(4);
        let mut state = WrapState::new();

        editor.set_cursor(3, 0);
        state.scroll_to_cursor(&editor, 80, 0);
        assert_eq!(state.scroll(), 0);
        state.page_down(&mut editor, 80, 0);
        assert_eq!(editor.cursor(), Cursor::new(3, 0));
        state.page_up(&mut editor, 80, 0);
        assert_eq!(editor.cursor(), Cursor::new(3, 0));
    }

    #[test]
    fn page_down_steps_by_the_viewport_less_one_row_of_overlap() {
        let mut editor = numbered(10);
        editor.set_cursor(0, 0);
        let mut state = WrapState::new();

        // Viewport 3, so a page is 2 rows: the cursor and the window both move by 2, and
        // the cursor stays on the pane's first row.
        state.page_down(&mut editor, 80, 3);
        assert_eq!(editor.cursor(), Cursor::new(2, 0));
        assert_eq!(state.scroll(), 2);

        state.page_down(&mut editor, 80, 3);
        assert_eq!(editor.cursor(), Cursor::new(4, 0));
        assert_eq!(state.scroll(), 4);

        // Near the end the window stops at the last page while the cursor runs on to the
        // end of the text, so the pane shows the buffer's tail and not blank rows.
        state.page_down(&mut editor, 80, 3);
        assert_eq!(editor.cursor(), Cursor::new(6, 0));
        assert_eq!(state.scroll(), 6);
        state.page_down(&mut editor, 80, 3);
        assert_eq!(editor.cursor(), Cursor::new(8, 0));
        assert_eq!(state.scroll(), 7);
        state.page_down(&mut editor, 80, 3);
        assert_eq!(editor.cursor(), Cursor::new(9, 0));
        assert_eq!(state.scroll(), 7);
        state.page_down(&mut editor, 80, 3);
        assert_eq!(editor.cursor(), Cursor::new(9, 0), "paged past the end");
        assert_eq!(state.scroll(), 7);
    }

    #[test]
    fn page_up_is_page_down_in_reverse() {
        let mut editor = numbered(10);
        let mut state = WrapState::new();
        editor.set_cursor(9, 0);
        state.scroll_to_cursor(&editor, 80, 3);
        assert_eq!(state.scroll(), 7);

        state.page_up(&mut editor, 80, 3);
        assert_eq!(editor.cursor(), Cursor::new(7, 0));
        assert_eq!(state.scroll(), 5);

        state.page_up(&mut editor, 80, 3);
        assert_eq!(editor.cursor(), Cursor::new(5, 0));
        assert_eq!(state.scroll(), 3);

        state.page_up(&mut editor, 80, 3);
        assert_eq!(editor.cursor(), Cursor::new(3, 0));
        assert_eq!(state.scroll(), 1);

        // At the top the window sticks at 0 while the cursor reaches row 0.
        state.page_up(&mut editor, 80, 3);
        assert_eq!(editor.cursor(), Cursor::new(1, 0));
        assert_eq!(state.scroll(), 0);
        state.page_up(&mut editor, 80, 3);
        assert_eq!(editor.cursor(), Cursor::new(0, 0), "paged past the start");
        assert_eq!(state.scroll(), 0);
    }

    #[test]
    fn paging_a_single_row_viewport_still_keeps_the_cursor_visible() {
        let mut editor = numbered(5);
        let mut state = WrapState::new();
        editor.set_cursor(4, 0);

        // One row means no row to give up for overlap, so a page is one row rather than
        // none: a key that did nothing would look broken.
        state.page_down(&mut editor, 80, 1);
        assert_eq!(editor.cursor(), Cursor::new(4, 0), "past the last row");
        assert_eq!(state.scroll(), 4);

        state.page_up(&mut editor, 80, 1);
        assert_eq!(editor.cursor(), Cursor::new(3, 0));
        assert_eq!(state.scroll(), 3);
    }

    // --- rendering --------------------------------------------------------

    #[test]
    fn render_draws_a_line_that_wraps_to_exactly_two_rows() {
        let editor = editor("aaaaaaaaa bbbbbbbbb");
        let mut state = WrapState::new();
        state.scroll_to_cursor(&editor, 10, 4);

        let drawn = draw(&editor, &mut state, 10, 4);
        assert_eq!(drawn.row(0), "aaaaaaaaa");
        assert_eq!(drawn.row(1), "bbbbbbbbb");
        assert_eq!(drawn.row(2), "");
    }

    #[test]
    fn render_draws_a_line_that_wraps_to_exactly_three_rows() {
        let editor = editor("aaaaaaaaa bbbbbbbbb ccccccccc");
        let mut state = WrapState::new();
        state.scroll_to_cursor(&editor, 10, 4);

        let drawn = draw(&editor, &mut state, 10, 4);
        assert_eq!(drawn.row(0), "aaaaaaaaa");
        assert_eq!(drawn.row(1), "bbbbbbbbb");
        assert_eq!(drawn.row(2), "ccccccccc");
    }

    #[test]
    fn render_hard_splits_a_long_word_over_several_rows() {
        let editor = editor("abcdefghijklm");
        let mut state = WrapState::new();
        state.scroll_to_cursor(&editor, 5, 4);

        let drawn = draw(&editor, &mut state, 5, 4);
        assert_eq!(drawn.row(0), "abcde");
        assert_eq!(drawn.row(1), "fghij");
        assert_eq!(drawn.row(2), "klm");

        let visible = (0..4).map(|y| drawn.row(y)).collect::<String>();
        assert_eq!(visible, "abcdefghijklm", "text was dropped or mangled");
        assert!(!visible.contains('…'), "an ellipsis appeared");
        assert!(!visible.contains('-'), "a hyphen appeared");
    }

    #[test]
    fn render_shows_the_scrolled_window_with_the_cursor_visible() {
        let mut editor = numbered(10);
        let mut state = WrapState::new();
        editor.set_cursor(9, 0);
        state.scroll_to_cursor(&editor, 80, 3);

        let drawn = draw(&editor, &mut state, 80, 3);
        assert_eq!(drawn.row(0), "line7");
        assert_eq!(drawn.row(1), "line8");
        assert_eq!(drawn.row(2), "line9");
        assert!(drawn.visible, "the cursor was hidden");
        assert_eq!(
            drawn.cursor,
            Position::new(0, 2),
            "cursor row inside the window"
        );
    }

    #[test]
    fn render_follows_the_cursor_rather_than_drawing_it_off_screen() {
        // A buffer taller than the pane used to draw from the top, leaving the cursor outside
        // the window; `render` now scrolls to it first, which is what makes a seeded buffer
        // and a resize behave. The pane is three rows and the cursor is on row 10.
        let mut editor = numbered(10);
        editor.set_cursor(9, 0);
        let mut state = WrapState::new();

        let drawn = draw(&editor, &mut state, 80, 3);
        assert!(
            drawn.visible,
            "the cursor was left outside the window instead of scrolling to it"
        );
        // And it is on the last visible row, since scrolling moves the minimum.
        assert_eq!(drawn.cursor.y, 2, "cursor is not on the final row");
    }

    #[test]
    fn render_places_the_cursor_at_the_arithmetic_position() {
        let mut editor = at("abcdefghij", 0, 7);
        let mut state = WrapState::new();
        state.scroll_to_cursor(&editor, 5, 3);

        let drawn = draw(&editor, &mut state, 5, 3);
        // Column 7 is row 1, cell 2: one row below the top, two cells in.
        assert_eq!(drawn.cursor, Position::new(2, 1));

        editor.set_cursor(0, 2);
        let drawn = draw(&editor, &mut state, 5, 3);
        assert_eq!(drawn.cursor, Position::new(2, 0));
    }

    #[test]
    fn render_respects_the_area_offset() {
        let editor = at("abcdefghij", 0, 7);
        let mut state = WrapState::new();
        state.scroll_to_cursor(&editor, 5, 2);

        let drawn = draw_in(&editor, &mut state, Rect::new(3, 1, 5, 2));
        assert_eq!(drawn.buffer[(3, 1)].symbol(), "a");
        assert_eq!(drawn.buffer[(3, 2)].symbol(), "f");
        // Row 1 of the pane is terminal row 2, cell 2 is terminal column 5.
        assert_eq!(drawn.cursor, Position::new(5, 2));
    }

    #[test]
    fn render_clamps_the_end_of_line_caret_onto_the_last_column() {
        // "abcde" fills a 5-cell row, so the caret after 'e' is cell 5: a real position that
        // no terminal can draw. The drawn cursor stops one cell short and nothing else
        // changes, so the buffer and the logical cursor are unaffected.
        let editor = at("abcde", 0, 5);
        let mut state = WrapState::new();

        assert_eq!(
            visual(&editor, 5),
            (0, 5),
            "the arithmetic must keep cell 5"
        );
        let drawn = draw(&editor, &mut state, 5, 2);
        assert_eq!(drawn.cursor, Position::new(4, 0));
        assert_eq!(editor.text(), "abcde");
        assert_eq!(editor.cursor(), Cursor::new(0, 5));
    }

    #[test]
    fn render_gives_a_glyph_too_wide_for_the_pane_its_own_row() {
        // 世 is two cells. At width 2 exactly one of them fits, so each gets a row. At
        // width 1 a terminal cannot show half a glyph, so the cell stays blank — the row
        // still exists, so the cursor and the arithmetic are unaffected and no text is
        // silently rewritten.
        let editor = editor("世界");
        let mut state = WrapState::new();
        state.scroll_to_cursor(&editor, 1, 2);
        let drawn = draw(&editor, &mut state, 1, 2);
        assert_eq!(drawn.buffer[(0, 0)].symbol(), " ");
        assert_eq!(editor.text(), "世界");

        state.scroll_to_cursor(&editor, 2, 2);
        let drawn = draw(&editor, &mut state, 2, 2);
        assert_eq!(drawn.row(0), "世");
        assert_eq!(drawn.row(1), "界");
    }

    #[test]
    fn render_at_width_one_does_not_panic_and_keeps_the_text() {
        let editor = editor("abc");
        let mut state = WrapState::new();
        state.scroll_to_cursor(&editor, 1, 3);

        let drawn = draw(&editor, &mut state, 1, 3);
        assert_eq!(drawn.row(0), "a");
        assert_eq!(drawn.row(1), "b");
        assert_eq!(drawn.row(2), "c");
        assert_eq!(editor.text(), "abc");
    }

    #[test]
    fn render_at_a_zero_sized_area_draws_nothing() {
        let editor = editor("abc");
        let mut state = WrapState::new();

        // A zero-width pane is what a 1-cell-wide terminal gives a bordered frame; a
        // zero-height one is a pane squeezed by the layout. Neither may panic or draw.
        let drawn = draw_in(&editor, &mut state, Rect::new(0, 0, 0, 3));
        assert!(!drawn.visible);
        assert_eq!(
            drawn.buffer.area.width, 1,
            "the test backend has no 0-wide buffer"
        );

        let drawn = draw_in(&editor, &mut state, Rect::new(0, 0, 3, 0));
        assert!(!drawn.visible);
    }

    #[test]
    fn a_resize_keeps_the_buffer_byte_identical_and_the_cursor_in_place() {
        let editor = at("some long text that wraps\nand a second line", 0, 12);
        let text = editor.text();
        let cursor = editor.cursor();
        let mut state = WrapState::new();

        for width in [40, 20, 5, 1, 0, 1, 7, 80] {
            state.scroll_to_cursor(&editor, width, 4);
            let drawn = draw(&editor, &mut state, width.max(1), 4);
            assert!(drawn.visible || width == 0, "cursor lost at width {width}");
            assert_eq!(editor.text(), text, "width {width} edited the buffer");
            assert_eq!(editor.cursor(), cursor, "width {width} moved the cursor");
        }
    }

    #[test]
    fn a_width_change_rewraps_without_losing_a_character() {
        let editor = editor("the quick brown fox jumps over the lazy dog");
        let mut state = WrapState::new();
        let before = buffer_rows(&editor, 40).len();

        // Rows are a function of the width passed in, so the new width is all it takes.
        state.scroll_to_cursor(&editor, 10, 3);
        let narrow = buffer_rows(&editor, 10);
        assert!(narrow.len() > before, "narrowing did not rewrap");

        let reconstructed = narrow
            .iter()
            .map(|row| row.text(&editor.lines()[row.line]))
            .collect::<String>();
        assert_eq!(
            reconstructed
                .chars()
                .filter(|c| *c != ' ')
                .collect::<String>(),
            editor
                .text()
                .chars()
                .filter(|c| *c != ' ')
                .collect::<String>()
        );
    }
}
