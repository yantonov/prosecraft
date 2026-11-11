//! Undo: a per-pane stack of whole-buffer snapshots (spec §4, §9).
//!
//! Snapshots rather than inverse operations: the buffer is "a few paragraphs" by design, so a
//! snapshot is cheap, while an inverse per [`Editor`] method would be six more hand-written
//! definitions of "the exact opposite of that edit", each of which has to stay correct against
//! every future change to the editor.
//!
//! This type lives beside the buffer in the view, not inside [`Editor`]: a snapshot *of* an
//! editor held *in* an editor would be a type containing a history of itself (spec §9). The
//! consequence is that it cannot observe edits, so the caller wraps each edit in
//! [`Undo::edit`] — which is what makes one key event one step.
//!
//! There is no redo. Undoing a large paste loses it; that is the accepted trade and the reason
//! the history is capped rather than kept whole.

use crate::editor::{Cursor, Editor};

/// How many steps one pane keeps.
///
/// The cap bounds a long session's memory; it is not a budget the user is meant to reach. A
/// snapshot is one document's worth of prose, so a hundred of them is far more typing than a
/// rewrite session holds and still a rounding error against the process itself.
const CAP: usize = 100;

/// One buffer state, as the pair undo has to restore together: the text and the cursor.
///
/// The lines are kept separately rather than joined, because that is what the buffer holds: a
/// joined copy would lose the difference between a trailing newline and an empty last line that
/// [`Editor::text`] has to keep, and comparing against the buffer then needs a join per edit.
#[derive(Debug)]
pub struct Snapshot {
    lines: Vec<String>,
    cursor: Cursor,
}

impl Snapshot {
    fn of(editor: &Editor) -> Self {
        Self {
            lines: editor.lines().to_vec(),
            cursor: editor.cursor(),
        }
    }

    /// Whether `editor` still holds exactly this state.
    ///
    /// An edit that leaves text and cursor where they were is a no-op by definition, so this
    /// single comparison is the whole of the change detection: no per-operation list of which
    /// edits can turn out to be no-ops, and no way for such a list to fall behind the editor.
    fn matches(&self, editor: &Editor) -> bool {
        self.cursor == editor.cursor() && self.lines == editor.lines()
    }

    /// Puts this state back into `editor`, text and cursor together.
    ///
    /// Consumes the snapshot because a popped step has no second use, and a restore that could
    /// be applied twice would invite callers to replay one.
    pub fn restore(self, editor: &mut Editor) {
        // `text` and `from_text` are exact inverses, so the join round-trips the lines that were
        // copied out of a buffer this very type came from.
        *editor = Editor::from_text(&self.lines.join("\n"));
        // `from_text` leaves the cursor at the end of the text; the state being restored is
        // wherever the user actually was, which is the other half of what undo has to bring
        // back. The position came from a real buffer state, so it is inside this one.
        editor.set_cursor(self.cursor.line(), self.cursor.column());
    }
}

/// The undo history of one pane. Oldest step first, so the newest is a pop.
#[derive(Debug, Default)]
pub struct Undo {
    steps: Vec<Snapshot>,
}

impl Undo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs one public [`Editor`] call and records the state before it.
    ///
    /// Recording here rather than at each call site is what gives every editing path the same
    /// granularity for free: a typed char, an `Enter`, a whole pasted paragraph and a clear are
    /// each one call, so each is one step. The comparison is made after the call rather than
    /// predicted from the operation, so a call that happens to change nothing — Backspace at
    /// the start of the buffer — costs nothing and leaves no step that appears to do nothing.
    pub fn edit(&mut self, editor: &mut Editor, operation: impl FnOnce(&mut Editor)) {
        let before = Snapshot::of(editor);
        operation(editor);
        if !before.matches(editor) {
            self.push(before);
        }
    }

    /// The previous state, popped. `None` means there is nothing left to undo, which the caller
    /// shows as nothing at all rather than as a failure: at the bottom of the history the key
    /// has simply done all it can.
    pub fn undo(&mut self) -> Option<Snapshot> {
        self.steps.pop()
    }

    fn push(&mut self, snapshot: Snapshot) {
        // Oldest-first eviction: the step furthest back is the one a user is least likely to
        // ask for, and at this cap the shift costs nothing worth measuring.
        if self.steps.len() == CAP {
            self.steps.remove(0);
        }
        self.steps.push(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A buffer with the cursor placed explicitly, for edits that only make sense from one
    /// exact position.
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

    #[test]
    fn an_empty_history_has_nothing_to_undo() {
        let mut undo = Undo::new();

        assert!(undo.undo().is_none(), "an unused history produced a step");
    }

    #[test]
    fn each_edit_is_one_step_and_repeated_undos_walk_back() {
        let mut editor = Editor::new();
        let mut undo = Undo::new();
        for c in ['a', 'b', 'c'] {
            undo.edit(&mut editor, |editor| editor.insert_char(c));
        }
        assert_eq!(editor.text(), "abc");

        // Three typed chars, three presses: the cursor moves back with the text.
        for (text, column) in [("ab", 2), ("a", 1), ("", 0)] {
            let step = undo.undo().expect("a step was not recorded");
            step.restore(&mut editor);
            assert_eq!(editor.text(), text);
            assert_eq!(
                editor.cursor(),
                Cursor::new(0, column),
                "cursor for {text:?}"
            );
        }

        assert!(
            undo.undo().is_none(),
            "the typed chars produced extra steps"
        );
    }

    #[test]
    fn enter_and_backspace_are_each_one_step() {
        let mut editor = at("ab", 0, 1);
        let mut undo = Undo::new();

        undo.edit(&mut editor, |editor| editor.insert_newline());
        assert_eq!(editor.text(), "a\nb");
        undo.edit(&mut editor, |editor| editor.backspace());
        assert_eq!(editor.text(), "ab");

        let step = undo.undo().expect("the backspace was not recorded");
        step.restore(&mut editor);
        assert_eq!(editor.text(), "a\nb", "the join was not undone");

        let step = undo.undo().expect("the Enter was not recorded");
        step.restore(&mut editor);
        assert_eq!(editor.text(), "ab", "the split was not undone");
        assert_eq!(editor.cursor(), Cursor::new(0, 1));

        assert!(undo.undo().is_none());
    }

    #[test]
    fn a_multiline_paste_is_exactly_one_step() {
        let before = at("start|end", 0, 6);
        let mut editor = before.clone();
        let mut undo = Undo::new();

        undo.edit(&mut editor, |editor| editor.insert_str("one\ntwo\nthree"));
        assert_eq!(editor.text(), "start|one\ntwo\nthreeend");

        let step = undo
            .undo()
            .expect("the paste was recorded as zero or many steps");
        step.restore(&mut editor);
        // Byte-identical to before the paste, cursor included: that is the whole point of one
        // step per public call.
        assert_eq!(editor.text(), before.text());
        assert_eq!(editor.cursor(), before.cursor());

        assert!(
            undo.undo().is_none(),
            "the paste was recorded as more than one step"
        );
    }

    #[test]
    fn a_clear_is_one_step_that_restores_every_line() {
        let before = at("first\nsecond\nthird", 1, 4);
        let mut editor = before.clone();
        let mut undo = Undo::new();

        undo.edit(&mut editor, |editor| editor.clear());
        assert_eq!(editor.text(), "");

        let step = undo.undo().expect("the clear was not recorded");
        step.restore(&mut editor);
        assert_eq!(editor.text(), before.text());
        assert_eq!(editor.cursor(), before.cursor());
        assert!(undo.undo().is_none(), "the clear was recorded twice");
    }

    #[test]
    fn an_edit_that_changes_nothing_records_nothing() {
        // The two no-ops the editor documents as no-ops, plus an empty paste, which is a third
        // way to call a public edit method and change nothing.
        let mut editor = at("abc", 0, 0);
        let mut undo = Undo::new();
        undo.edit(&mut editor, |editor| editor.backspace());
        assert_eq!(
            editor.text(),
            "abc",
            "backspace at the start is not a no-op"
        );

        editor.set_cursor(0, 3);
        undo.edit(&mut editor, |editor| editor.delete());
        assert_eq!(editor.text(), "abc", "delete at the end is not a no-op");

        undo.edit(&mut editor, |editor| editor.insert_str(""));

        assert!(
            undo.undo().is_none(),
            "an edit that changed nothing left a step that appears to do nothing"
        );
    }

    #[test]
    fn a_run_of_no_op_edits_never_fills_the_history() {
        // The first no-op after a real edit, then a long run of them: a history that recorded
        // unconditionally would hide the real step under presses that do nothing.
        let mut editor = at("ab", 0, 0);
        let mut undo = Undo::new();
        undo.edit(&mut editor, |editor| editor.insert_char('x'));
        // Back to the very start, where a backspace removes nothing; the move itself is not a
        // step, so what the run of backspaces must leave behind is nothing at all.
        editor.set_cursor(0, 0);
        for _ in 0..50 {
            undo.edit(&mut editor, |editor| editor.backspace());
        }

        let step = undo.undo().expect("the real edit was not recorded");
        step.restore(&mut editor);
        assert_eq!(editor.text(), "ab");
        assert_eq!(editor.cursor(), Cursor::new(0, 0));
        assert!(undo.undo().is_none(), "a no-op left a step behind");
    }

    #[test]
    fn the_cap_evicts_the_oldest_step_first() {
        let mut editor = Editor::new();
        let mut undo = Undo::new();
        // CAP + 5 typed chars: five more steps than the history can hold.
        for n in 0..CAP + 5 {
            let c = char::from(b'a' + (n % 26) as u8);
            undo.edit(&mut editor, |editor| editor.insert_char(c));
        }
        let typed = editor.text();

        // The newest step is there, and its state is one char shorter.
        let step = undo.undo().expect("the newest step was evicted");
        step.restore(&mut editor);
        assert_eq!(editor.text().chars().count(), CAP + 4);

        let mut undone = 1;
        while let Some(step) = undo.undo() {
            step.restore(&mut editor);
            undone += 1;
        }
        assert_eq!(undone, CAP, "the history was not capped");

        // Five oldest steps were dropped, so the walk stops five chars into the text rather
        // than at the empty buffer it started from.
        let kept: String = typed.chars().take(5).collect();
        assert_eq!(
            editor.text(),
            kept,
            "eviction did not drop the oldest steps"
        );
        assert_eq!(editor.cursor(), Cursor::new(0, 5));
    }

    #[test]
    fn a_trailing_newline_survives_a_snapshot_round_trip() {
        // The buffer's own distinction: `"a\n"` is two lines, the second empty, and a snapshot
        // that flattened the text would restore `"a"` instead.
        for text in ["a\n", "\n", "\n\n", "a\nb", "—é🙂\n"] {
            let before = at(text, 0, 0);
            let mut editor = before.clone();
            let mut undo = Undo::new();
            undo.edit(&mut editor, |editor| editor.insert_char('x'));

            undo.undo().expect("no step").restore(&mut editor);

            assert_eq!(editor.text(), before.text(), "round trip of {text:?}");
            assert_eq!(editor.lines(), before.lines(), "round trip of {text:?}");
        }
    }

    #[test]
    fn a_snapshot_matches_only_the_state_it_was_taken_from() {
        // The change detection is the one rule that decides what is recorded, so it is checked
        // against each half of the state it compares.
        let editor = at("abc", 0, 2);
        let snapshot = Snapshot::of(&editor);

        assert!(snapshot.matches(&editor));
        // Same text, different cursor, and the state is different: undo restores both halves.
        assert!(!snapshot.matches(&at("abc", 0, 1)));
        assert!(!snapshot.matches(&at("ab", 0, 2)));
        assert!(!snapshot.matches(&Editor::new()));
    }
}
