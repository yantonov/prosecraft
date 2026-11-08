//! The text buffer shared by the two editable panes (spec §4, §9).
//!
//! The cursor is a `(line, column)` pair where `column` counts **chars**, not bytes. The
//! buffer holds human prose — em-dashes, accented letters, emoji — and a byte offset can
//! land in the middle of a code point, which would make `insert`/`remove` panic or split a
//! character. Counting chars costs one scan per edit and buys panic-free editing on any text.
//!
//! Wrapping is deliberately absent: this is logical-line state only, so a long line still
//! counts as one row. [`Editor::move_up`] and [`Editor::move_down`] therefore step one
//! *logical* line; visual rows, width-aware movement and scroll-to-cursor belong to the wrap
//! ticket and are built on top of this module.
//!
//! There is no selection and no undo (spec §2), and no key handling: the cursor is a model,
//! so every operation is directly testable without a terminal.

/// A position in the buffer: a line index and a char index within that line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cursor {
    line: usize,
    column: usize,
}

impl Cursor {
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }

    pub const fn line(self) -> usize {
        self.line
    }

    pub const fn column(self) -> usize {
        self.column
    }
}

/// A buffer of logical lines plus a cursor.
///
/// Invariants, which every method here preserves: `lines` is never empty, and the cursor
/// names an existing line with a column no greater than that line's char count. That is why
/// the indexing below needs no bounds checks and why an empty buffer is a valid state rather
/// than an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    lines: Vec<String>,
    cursor: Cursor,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    /// An empty buffer: exactly one empty line, cursor at the origin.
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: Cursor::default(),
        }
    }

    /// Splits `text` on `\n`, so a trailing newline leaves a final empty line and
    /// [`Editor::text`] reproduces the input exactly.
    pub fn from_text(text: &str) -> Self {
        let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
        // The end of the text is where a user resumes typing. `split` always yields at least
        // one line, so the subtraction cannot underflow.
        let cursor = Cursor::new(
            lines.len() - 1,
            lines.last().map_or(0, |last| last.chars().count()),
        );
        Self { lines, cursor }
    }

    /// The buffer as one string: lines joined with `\n`. The exact inverse of
    /// [`Editor::from_text`], so a round trip preserves trailing newlines.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// The lines separately, because joining and re-splitting loses the distinction between a
    /// trailing newline and an empty last line that `text` has to keep.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// Moves the cursor to `(line, column)`, clamped into the buffer.
    ///
    /// Clamping rather than trusting the caller is what keeps the invariant above true for
    /// positions that come from outside the model, e.g. a screen coordinate derived from a
    /// mouse event or a scroll offset.
    pub fn set_cursor(&mut self, line: usize, column: usize) {
        let line = line.min(self.lines.len() - 1);
        self.cursor = Cursor::new(line, column.min(self.line_len(line)));
    }

    /// Char length of `line`; 0 for an out-of-range index, which is the value the clamping
    /// arithmetic wants and so avoids a second bounds check at every call site.
    pub fn line_len(&self, line: usize) -> usize {
        self.lines.get(line).map_or(0, |text| text.chars().count())
    }

    /// Inserts one char at the cursor and moves the cursor past it. A newline is routed to
    /// [`Editor::insert_newline`] so a caller streaming chars needs no special case.
    pub fn insert_char(&mut self, c: char) {
        if c == '\n' {
            self.insert_newline();
            return;
        }
        let line = self.cursor.line();
        let at = byte_index(&self.lines[line], self.cursor.column());
        self.lines[line].insert(at, c);
        self.cursor = Cursor::new(line, self.cursor.column() + 1);
    }

    /// Inserts a whole `&str` at the cursor.
    ///
    /// Newlines inside it are real line breaks, not literal characters: a pane cannot display
    /// one, and pasted prose arrives with them. The cursor ends after the last inserted char.
    pub fn insert_str(&mut self, text: &str) {
        for c in text.chars() {
            self.insert_char(c);
        }
    }

    /// Splits the current line at the cursor and puts the cursor at the start of the new line,
    /// so the text after the cursor moves down with it.
    pub fn insert_newline(&mut self) {
        let line = self.cursor.line();
        let at = byte_index(&self.lines[line], self.cursor.column());
        let tail = self.lines[line].split_off(at);
        self.lines.insert(line + 1, tail);
        self.cursor = Cursor::new(line + 1, 0);
    }

    /// Removes the char before the cursor, or joins the current line onto the previous one at
    /// column 0. A no-op at the very start of the buffer.
    pub fn backspace(&mut self) {
        let line = self.cursor.line();
        let column = self.cursor.column();
        if column > 0 {
            let at = byte_index(&self.lines[line], column - 1);
            self.lines[line].remove(at);
            self.cursor = Cursor::new(line, column - 1);
        } else if line > 0 {
            // The join boundary is the previous line's length, which is where the cursor
            // lands: that is the gap the two halves were separated by.
            let boundary = self.line_len(line - 1);
            let current = self.lines.remove(line);
            self.lines[line - 1].push_str(&current);
            self.cursor = Cursor::new(line - 1, boundary);
        }
    }

    /// Removes the char at the cursor, or pulls the next line up at end of line. A no-op at
    /// the very end of the buffer. The cursor does not move: the next char takes the deleted
    /// one's place.
    pub fn delete(&mut self) {
        let line = self.cursor.line();
        if self.cursor.column() < self.line_len(line) {
            let at = byte_index(&self.lines[line], self.cursor.column());
            self.lines[line].remove(at);
        } else if line + 1 < self.lines.len() {
            let next = self.lines.remove(line + 1);
            self.lines[line].push_str(&next);
        }
    }

    /// One char left, crossing to the end of the previous line at column 0.
    pub fn move_left(&mut self) {
        let line = self.cursor.line();
        let column = self.cursor.column();
        if column > 0 {
            self.cursor = Cursor::new(line, column - 1);
        } else if line > 0 {
            self.cursor = Cursor::new(line - 1, self.line_len(line - 1));
        }
    }

    /// One char right, crossing to the start of the next line at end of line.
    pub fn move_right(&mut self) {
        let line = self.cursor.line();
        let column = self.cursor.column();
        if column < self.line_len(line) {
            self.cursor = Cursor::new(line, column + 1);
        } else if line + 1 < self.lines.len() {
            self.cursor = Cursor::new(line + 1, 0);
        }
    }
}

/// Byte offset of char index `column`, or the line's length when `column` is past the end.
/// Saturating keeps a stale column harmless instead of panicking mid-edit.
fn byte_index(line: &str, column: usize) -> usize {
    line.char_indices()
        .nth(column)
        .map_or(line.len(), |(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only accessor: the number of logical lines.
    ///
    /// `Editor` does not expose this because nothing in the app needs it — the wrap layer
    /// asks for `lines()` and counts them itself, since it works in visual rows. A test asking
    /// "how many lines did that edit produce" is a different question, so the helper lives
    /// here rather than widening the production type.
    trait LineCount {
        fn line_count(&self) -> usize;
    }

    impl LineCount for Editor {
        fn line_count(&self) -> usize {
            self.lines().len()
        }
    }

    /// Builds an editor from text and asserts the invariant the rest of the tests rely on:
    /// the cursor names a real position inside the buffer.
    fn editor(text: &str) -> Editor {
        let editor = Editor::from_text(text);
        let cursor = editor.cursor();
        assert!(
            cursor.line() < editor.line_count(),
            "cursor line {} outside {} lines for {text:?}",
            cursor.line(),
            editor.line_count()
        );
        assert!(
            cursor.column() <= editor.line_len(cursor.line()),
            "cursor column {} past the end of line {} for {text:?}",
            cursor.column(),
            cursor.line()
        );
        editor
    }

    /// A buffer with the cursor placed explicitly, for edits that only make sense from one
    /// exact position; walking there with arrow keys would bury the edit under test.
    fn at(text: &str, line: usize, column: usize) -> Editor {
        let mut editor = Editor::from_text(text);
        editor.set_cursor(line, column);
        assert_eq!(
            editor.cursor(),
            Cursor::new(line, column),
            "{line},{column} is not a valid position in {text:?}"
        );
        editor
    }

    fn assert_at(editor: &Editor, line: usize, column: usize) {
        assert_eq!(
            editor.cursor(),
            Cursor::new(line, column),
            "cursor after edit, text {:?}",
            editor.text()
        );
    }

    #[test]
    fn new_buffer_is_one_empty_line_at_the_origin() {
        let editor = Editor::new();

        assert_eq!(editor.text(), "");
        assert_eq!(editor.line_count(), 1);
        assert_eq!(editor.lines(), [""]);
        assert_eq!(editor.line_len(0), 0);
        assert_at(&editor, 0, 0);
    }

    #[test]
    fn empty_text_parses_to_one_empty_line() {
        let editor = editor("");

        assert_eq!(editor.line_count(), 1);
        assert_at(&editor, 0, 0);
    }

    #[test]
    fn from_text_and_text_round_trip_exactly() {
        for text in [
            "",
            "a",
            "a\nb",
            "a\nb\n",
            "\n",
            "\n\n",
            "line with spaces\nand a second",
            "trailing\n",
            "—é🙂\n—é🙂",
        ] {
            assert_eq!(editor(text).text(), text, "round trip of {text:?}");
        }
    }

    #[test]
    fn from_text_splits_lines_and_puts_the_cursor_at_the_end() {
        let editor = editor("abc\nde");

        assert_eq!(editor.line_count(), 2);
        assert_eq!(editor.lines(), ["abc", "de"]);
        assert_eq!(editor.line_len(0), 3);
        assert_eq!(editor.line_len(1), 2);
        assert_at(&editor, 1, 2);
    }

    #[test]
    fn from_text_cursor_on_a_trailing_newline_is_the_empty_last_line() {
        let editor = editor("abc\n");

        assert_eq!(editor.line_count(), 2);
        assert_eq!(editor.lines(), ["abc", ""]);
        assert_at(&editor, 1, 0);
    }

    #[test]
    fn line_len_of_an_out_of_range_line_is_zero() {
        let editor = editor("abc");

        assert_eq!(editor.line_len(1), 0);
        assert_eq!(editor.line_len(usize::MAX), 0);
    }

    #[test]
    fn set_cursor_clamps_into_the_buffer() {
        let mut editor = editor("ab\ncdef");

        editor.set_cursor(1, 4);
        assert_at(&editor, 1, 4);

        editor.set_cursor(1, 99);
        assert_at(&editor, 1, 4);

        editor.set_cursor(99, 0);
        assert_at(&editor, 1, 0);

        // The clamped position is still a legal place to edit.
        editor.insert_char('x');
        assert_eq!(editor.text(), "ab\nxcdef");
    }

    #[test]
    fn set_cursor_clamps_to_an_empty_buffer() {
        let mut editor = Editor::new();
        editor.set_cursor(7, 7);

        assert_at(&editor, 0, 0);
    }

    #[test]
    fn insert_char_at_the_start_middle_and_end() {
        let mut editor = editor("ac");
        editor.move_left();
        editor.insert_char('b');
        assert_eq!(editor.text(), "abc");
        assert_at(&editor, 0, 2);

        // Start of the buffer.
        editor.move_left();
        editor.move_left();
        editor.insert_char('_');
        assert_eq!(editor.text(), "_abc");
        assert_at(&editor, 0, 1);

        // End of the buffer.
        editor.move_right();
        editor.move_right();
        editor.move_right();
        assert_at(&editor, 0, 4);
        editor.insert_char('_');
        assert_eq!(editor.text(), "_abc_");
        assert_at(&editor, 0, 5);
    }

    #[test]
    fn insert_char_into_an_empty_buffer() {
        let mut editor = Editor::new();
        editor.insert_char('x');

        assert_eq!(editor.text(), "x");
        assert_at(&editor, 0, 1);
    }

    #[test]
    fn insert_char_on_a_later_line_lands_on_that_line() {
        let mut editor = editor("one\ntwo");
        editor.set_cursor(0, 3);
        editor.insert_char('!');

        assert_eq!(editor.text(), "one!\ntwo");
        assert_at(&editor, 0, 4);
    }

    #[test]
    fn insert_str_inserts_at_the_cursor() {
        let mut editor = editor("ad");
        editor.move_left();
        editor.insert_str("bc");

        assert_eq!(editor.text(), "abcd");
        assert_at(&editor, 0, 3);
    }

    #[test]
    fn insert_str_with_newlines_splits_into_lines() {
        let mut editor = editor("ac");
        editor.move_left();
        editor.insert_str("x\ny\nz");

        assert_eq!(editor.text(), "ax\ny\nzc");
        assert_eq!(editor.line_count(), 3);
        assert_at(&editor, 2, 1);
    }

    #[test]
    fn insert_empty_str_changes_nothing() {
        let mut editor = editor("abc");
        editor.insert_str("");

        assert_eq!(editor.text(), "abc");
        assert_at(&editor, 0, 3);
    }

    #[test]
    fn insert_newline_splits_the_line_at_the_cursor() {
        let mut editor = editor("abcd");
        editor.move_left();
        editor.move_left();
        editor.insert_newline();

        assert_eq!(editor.text(), "ab\ncd");
        assert_eq!(editor.line_count(), 2);
        assert_at(&editor, 1, 0);
    }

    #[test]
    fn insert_newline_at_end_of_line_starts_an_empty_next_line() {
        let mut editor = editor("abc");
        editor.insert_newline();

        assert_eq!(editor.text(), "abc\n");
        assert_at(&editor, 1, 0);
    }

    #[test]
    fn insert_newline_at_start_of_line_leaves_an_empty_first_line() {
        let mut editor = editor("abc");
        for _ in 0..3 {
            editor.move_left();
        }
        editor.insert_newline();

        assert_eq!(editor.text(), "\nabc");
        assert_at(&editor, 1, 0);
    }

    #[test]
    fn insert_newline_on_an_empty_buffer_makes_two_empty_lines() {
        let mut editor = Editor::new();
        editor.insert_newline();

        assert_eq!(editor.text(), "\n");
        assert_eq!(editor.line_count(), 2);
        assert_at(&editor, 1, 0);
    }

    #[test]
    fn insert_char_of_a_newline_is_a_line_break_not_a_literal_char() {
        let mut editor = editor("ab");
        editor.insert_char('\n');

        // A pane cannot display a newline cell, so the char takes the newline path and the
        // buffer never holds one inside a line.
        assert_eq!(editor.text(), "ab\n");
        assert_eq!(editor.line_count(), 2);
        assert_at(&editor, 1, 0);
    }

    #[test]
    fn backspace_removes_the_char_before_the_cursor() {
        let mut editor = editor("abc");
        editor.backspace();

        assert_eq!(editor.text(), "ab");
        assert_at(&editor, 0, 2);
    }

    #[test]
    fn backspace_on_a_single_char_line_empties_it() {
        let mut editor = editor("x");
        editor.backspace();

        assert_eq!(editor.text(), "");
        assert_eq!(editor.line_count(), 1);
        assert_at(&editor, 0, 0);
    }

    #[test]
    fn backspace_at_column_zero_joins_with_the_previous_line() {
        let mut editor = at("ab\ncd", 1, 0);

        editor.backspace();

        assert_eq!(editor.text(), "abcd");
        assert_eq!(editor.line_count(), 1);
        assert_at(&editor, 0, 2);
    }

    #[test]
    fn backspace_at_the_very_start_is_a_no_op_and_stays_one_when_repeated() {
        let mut editor = editor("abc");
        for _ in 0..3 {
            editor.move_left();
        }

        for _ in 0..5 {
            editor.backspace();
            assert_eq!(editor.text(), "abc");
            assert_at(&editor, 0, 0);
        }
    }

    #[test]
    fn backspace_on_an_empty_buffer_is_a_no_op() {
        let mut editor = Editor::new();
        editor.backspace();

        assert_eq!(editor.text(), "");
        assert_at(&editor, 0, 0);
    }

    #[test]
    fn backspace_joins_a_single_char_line_onto_its_neighbour() {
        let mut editor = at("a\nb", 1, 0);
        editor.backspace();

        assert_eq!(editor.text(), "ab");
        assert_at(&editor, 0, 1);
    }

    #[test]
    fn delete_removes_the_char_at_the_cursor() {
        let mut editor = at("abc", 0, 0);
        editor.delete();

        assert_eq!(editor.text(), "bc");
        // Delete does not move the cursor: the next char took the deleted one's place.
        assert_at(&editor, 0, 0);
    }

    #[test]
    fn delete_on_a_single_char_line_empties_it() {
        let mut editor = editor("x");
        editor.move_left();
        editor.delete();

        assert_eq!(editor.text(), "");
        assert_eq!(editor.line_count(), 1);
        assert_at(&editor, 0, 0);
    }

    #[test]
    fn delete_at_end_of_line_joins_with_the_next_line() {
        let mut editor = editor("ab\ncd");
        editor.set_cursor(0, 2);

        assert_at(&editor, 0, 2);
        editor.delete();

        assert_eq!(editor.text(), "abcd");
        assert_eq!(editor.line_count(), 1);
        assert_at(&editor, 0, 2);
    }

    #[test]
    fn delete_on_an_empty_line_joins_with_the_next_line() {
        let mut editor = at("a\n\nb", 1, 0);
        editor.delete();

        assert_eq!(editor.text(), "a\nb");
        assert_at(&editor, 1, 0);
    }

    #[test]
    fn delete_at_the_very_end_is_a_no_op_and_stays_one_when_repeated() {
        let mut editor = editor("abc");

        for _ in 0..5 {
            editor.delete();
            assert_eq!(editor.text(), "abc");
            assert_at(&editor, 0, 3);
        }
    }

    #[test]
    fn delete_on_an_empty_buffer_is_a_no_op() {
        let mut editor = Editor::new();
        editor.delete();

        assert_eq!(editor.text(), "");
        assert_at(&editor, 0, 0);
    }

    #[test]
    fn left_and_right_move_within_a_line() {
        let mut editor = editor("abc");
        for _ in 0..3 {
            editor.move_left();
        }
        assert_at(&editor, 0, 0);

        editor.move_right();
        assert_at(&editor, 0, 1);
        editor.move_right();
        editor.move_right();
        assert_at(&editor, 0, 3);
    }

    #[test]
    fn left_and_right_cross_line_boundaries() {
        let mut editor = editor("ab\ncd");
        editor.set_cursor(0, 2);
        assert_at(&editor, 0, 2);

        // Right at end of line steps onto the next line's first column.
        editor.move_right();
        assert_at(&editor, 1, 0);
        // Left at column 0 steps back to the previous line's end.
        editor.move_left();
        assert_at(&editor, 0, 2);
    }

    #[test]
    fn left_and_right_cross_an_empty_line() {
        let mut editor = at("ab\n\ncd", 1, 0);

        editor.move_left();
        assert_at(&editor, 0, 2);
        editor.move_right();
        assert_at(&editor, 1, 0);
        editor.move_right();
        assert_at(&editor, 2, 0);
    }

    #[test]
    fn horizontal_movement_clamps_at_the_buffer_edges() {
        let mut editor = editor("ab");
        for _ in 0..3 {
            editor.move_left();
        }
        assert_at(&editor, 0, 0);

        for _ in 0..5 {
            editor.move_right();
        }
        assert_at(&editor, 0, 2);
    }

    #[test]
    fn horizontal_movement_on_an_empty_buffer_is_a_no_op() {
        let mut editor = Editor::new();
        for _ in 0..3 {
            editor.move_left();
            editor.move_right();
        }

        assert_at(&editor, 0, 0);
        assert_eq!(editor.text(), "");
    }

    #[test]
    fn cursor_columns_count_chars_not_bytes() {
        let mut editor = editor("a—b");
        // Three chars but five bytes, so a byte-based cursor would report 3 after the em dash
        // and every later offset would be shifted.
        assert_eq!(editor.line_len(0), 3);
        editor.move_left();
        assert_at(&editor, 0, 2);

        editor.move_left();
        assert_at(&editor, 0, 1);
        editor.move_left();
        assert_at(&editor, 0, 0);
    }

    #[test]
    fn insert_and_delete_around_utf8_chars_keep_them_whole() {
        let mut editor = editor("café—🙂");
        assert_eq!(editor.line_len(0), 6);

        // Land before the emoji and insert: the neighbouring multi-byte chars must survive.
        editor.move_left();
        editor.insert_char('!');
        assert_eq!(editor.text(), "café—!🙂");

        editor.backspace();
        assert_eq!(editor.text(), "café—🙂");

        // The cursor sits before the emoji, so Delete takes the emoji and the following
        // Backspace takes the em dash behind the cursor.
        editor.delete();
        assert_eq!(editor.text(), "café—");
        assert_at(&editor, 0, 5);
        editor.backspace();
        assert_eq!(editor.text(), "café");
        assert_at(&editor, 0, 4);
    }

    #[test]
    fn backspace_behind_a_multibyte_char_removes_that_char_and_nothing_else() {
        let mut editor = editor("é—🙂");

        editor.backspace();
        assert_eq!(editor.text(), "é—");
        assert_at(&editor, 0, 2);

        editor.backspace();
        assert_eq!(editor.text(), "é", "the em dash must come off whole");
        assert_at(&editor, 0, 1);

        editor.backspace();
        assert_eq!(editor.text(), "", "the accented letter must come off whole");
        assert_at(&editor, 0, 0);
    }

    #[test]
    fn joins_and_splits_preserve_multibyte_text() {
        let mut editor = editor("aé\nb—");
        editor.set_cursor(0, 2);
        editor.delete();
        assert_eq!(editor.text(), "aéb—");

        editor.insert_newline();
        assert_eq!(editor.text(), "aé\nb—");

        // Backspace over the join reproduces the same single line.
        editor.backspace();
        assert_eq!(editor.text(), "aéb—");
        assert_at(&editor, 0, 2);
    }

    #[test]
    fn inserting_a_multibyte_str_advances_by_chars() {
        let mut editor = Editor::new();
        editor.insert_str("é—🙂");

        assert_eq!(editor.text(), "é—🙂");
        assert_eq!(editor.line_len(0), 3);
        assert_at(&editor, 0, 3);
    }

    #[test]
    fn text_round_trips_after_a_mixed_edit_session() {
        let mut editor = editor("hello\nworld");
        editor.set_cursor(0, 0);
        editor.insert_str(", ");
        editor.insert_newline();
        editor.insert_str("big ");
        editor.backspace();
        editor.insert_char('.');

        let text = editor.text();
        let reloaded = Editor::from_text(&text);
        assert_eq!(reloaded.text(), text);
        assert_eq!(reloaded.line_count(), editor.line_count());
        for line in 0..editor.line_count() {
            assert_eq!(reloaded.line_len(line), editor.line_len(line));
        }
    }

    #[test]
    fn every_operation_is_panic_free_on_an_empty_buffer() {
        let mut editor = Editor::new();

        // Repeated, so an operation that only "worked" once shows up as a panic on the second
        // pass instead of passing on a lucky first call.
        for _ in 0..4 {
            editor.insert_char('\0');
            editor.backspace();
            editor.insert_newline();
            editor.backspace();
            editor.move_left();
            editor.move_right();
            editor.delete();
        }

        assert_eq!(editor.text(), "");
        assert_eq!(editor.line_count(), 1);
        assert_at(&editor, 0, 0);
    }

    #[test]
    fn every_operation_is_panic_free_on_a_single_char_line() {
        let mut editor = editor("é");

        editor.move_left();
        editor.move_left();
        editor.delete();
        editor.delete();
        editor.backspace();
        editor.backspace();
        editor.move_right();
        editor.move_right();
        editor.insert_char('é');

        assert_eq!(editor.text(), "é");
        assert_at(&editor, 0, 1);
    }

    #[test]
    fn backspace_and_delete_undo_an_insert_at_every_position() {
        let original = "one\ntwö\n—three—";
        for line in 0..3 {
            for column in 0..=at(original, line, 0).line_len(line) {
                let mut forward = at(original, line, column);
                forward.insert_char('X');
                let mut back = forward.clone();
                back.backspace();
                assert_eq!(back.text(), original, "insert/backspace at {line},{column}");
                assert_at(&back, line, column);

                // Delete undoes the same insert once the cursor is moved before the new char.
                forward.move_left();
                forward.delete();
                assert_eq!(forward.text(), original, "insert/delete at {line},{column}");
                assert_at(&forward, line, column);
            }
        }
    }

    #[test]
    fn joining_both_directions_restores_the_same_text() {
        let original = "alpha\nbeta";

        // Backspace from the start of the second line.
        let mut via_backspace = at(original, 1, 0);
        via_backspace.backspace();
        assert_eq!(via_backspace.text(), "alphabeta");
        assert_at(&via_backspace, 0, 5);

        // Delete from the end of the first line: same text, same cursor landing, which is why
        // either key feels like "remove the line break".
        let mut via_delete = at(original, 0, 5);
        via_delete.delete();
        assert_eq!(via_delete.text(), "alphabeta");
        assert_at(&via_delete, 0, 5);
    }
}
