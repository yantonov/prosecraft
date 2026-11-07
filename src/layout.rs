//! Pane geometry and focus state (spec §4).
//!
//! Kept apart from the widgets so the arithmetic can be asserted directly, including at
//! sizes where a rendered frame is unreadable (1x1, odd widths) and where the column split
//! rule needs checking without counting border cells. Nothing here touches a terminal.

use ratatui::layout::Rect;

/// Instructions pane: `min(5, 25% of the column)` (spec §4), then floored at
/// [`BOTTOM_MIN`]. The divisor is the 25% and the cap the 5.
pub const INSTRUCTIONS: Share = Share { divisor: 4, cap: 5 };

/// Debug pane. The spec pins only the instructions formula; its drawing gives the debug
/// pane about a third of a 24-row terminal and marks it "~8", so a third reproduces the
/// drawing and 8 is the ceiling that stops it eating the output pane on a tall screen.
pub const DEBUG: Share = Share { divisor: 3, cap: 8 };

/// A bottom pane's proportional height: `min(cap, column / divisor)`.
#[derive(Debug, Clone, Copy)]
pub struct Share {
    divisor: u16,
    cap: u16,
}

/// A pane shorter than 3 rows cannot show a border plus one line of text, so both bottom
/// panes keep at least this many rows while the column has them to give.
pub const BOTTOM_MIN: u16 = 3;

/// The status line is always the last row and is never optional.
pub const STATUS_HEIGHT: u16 = 1;

/// One of the four panes. Its order is the reading order used by `Tab`, and the index is
/// what `Ctrl+Left`/`Ctrl+Right` keeps constant by switching to the other column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Source,
    Instructions,
    Output,
    Debug,
}

impl Focus {
    /// Reading order: left column top to bottom, then right column top to bottom.
    pub const ALL: [Self; 4] = [Self::Source, Self::Instructions, Self::Output, Self::Debug];

    /// `Tab`. Wraps, so focus never gets stuck at the end of the order.
    pub fn next(self) -> Self {
        match self {
            Self::Source => Self::Instructions,
            Self::Instructions => Self::Output,
            Self::Output => Self::Debug,
            Self::Debug => Self::Source,
        }
    }

    /// `Shift+Tab`, the exact inverse of [`Focus::next`].
    pub fn prev(self) -> Self {
        match self {
            Self::Source => Self::Debug,
            Self::Instructions => Self::Source,
            Self::Output => Self::Instructions,
            Self::Debug => Self::Output,
        }
    }

    /// `Ctrl+Left`/`Ctrl+Right`: the other column at the same row (spec §4). The pairs are
    /// source/output (top row) and instructions/debug (bottom row).
    pub fn switch_column(self) -> Self {
        match self {
            Self::Source => Self::Output,
            Self::Instructions => Self::Debug,
            Self::Output => Self::Source,
            Self::Debug => Self::Instructions,
        }
    }

    /// Spec §4 table: only the left column takes text.
    pub fn is_editable(self) -> bool {
        matches!(self, Self::Source | Self::Instructions)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Instructions => "instructions",
            Self::Output => "output",
            Self::Debug => "debug",
        }
    }
}

/// Where every part of the frame lands. `Rect`s are in absolute terminal coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panes {
    pub source: Rect,
    pub instructions: Rect,
    pub output: Rect,
    pub debug: Rect,
    pub status: Rect,
}

impl Panes {
    fn rect(self, focus: Focus) -> Rect {
        match focus {
            Focus::Source => self.source,
            Focus::Instructions => self.instructions,
            Focus::Output => self.output,
            Focus::Debug => self.debug,
        }
    }

    /// The text area inside a pane, borders excluded. This is the width the wrap ticket has
    /// to wrap to: counting the border columns would push the last glyph onto the border.
    pub fn inner(self, focus: Focus) -> Rect {
        let rect = self.rect(focus);
        // Saturating, because a pane can be 0 or 1 rows wide at tiny terminal sizes.
        Rect {
            x: rect.x + 1,
            y: rect.y + 1,
            width: rect.width.saturating_sub(2),
            height: rect.height.saturating_sub(2),
        }
    }

    /// In reading order, so a renderer can loop instead of naming four fields.
    pub fn all(self) -> [(Focus, Rect); 4] {
        Focus::ALL.map(|focus| (focus, self.rect(focus)))
    }
}

/// Splits `area` into the four panes and the status line.
///
/// Recomputed on every draw, so a resize needs no state: the frame size is the only input.
pub fn panes(area: Rect) -> Panes {
    // `min` keeps the status line from claiming a row a 0- or 1-row terminal does not have.
    let status_height = area.height.min(STATUS_HEIGHT);
    let body = Rect {
        height: area.height - status_height,
        ..area
    };
    let status = Rect {
        y: area.y + body.height,
        height: status_height,
        ..area
    };

    // 50/50 with the spare column (odd widths) going left, matching reading order.
    let left_width = body.width.div_ceil(2);
    let left = Rect {
        width: left_width,
        ..body
    };
    let right = Rect {
        x: body.x + left_width,
        width: body.width - left_width,
        ..body
    };

    let (source, instructions) = split_column(left, INSTRUCTIONS);
    let (output, debug) = split_column(right, DEBUG);

    Panes {
        source,
        instructions,
        output,
        debug,
        status,
    }
}

/// Splits a column into the top pane and a bottom pane that takes `share` of the rows.
fn split_column(column: Rect, share: Share) -> (Rect, Rect) {
    let bottom_height = bottom_height(column.height, share);
    let top = Rect {
        height: column.height - bottom_height,
        ..column
    };
    let bottom = Rect {
        y: column.y + top.height,
        height: bottom_height,
        ..column
    };
    (top, bottom)
}

/// `min(cap, column / divisor)` floored at [`BOTTOM_MIN`], then clamped to the column.
///
/// The final clamp is what keeps degenerate sizes panic-free: below 3 rows the bottom pane
/// takes the whole column and the top one collapses to zero, so the two heights still sum
/// to exactly `column` at every size.
fn bottom_height(column: u16, share: Share) -> u16 {
    (column / share.divisor)
        .min(share.cap)
        .max(BOTTOM_MIN)
        .min(column)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(width: u16, height: u16) -> Rect {
        Rect::new(0, 0, width, height)
    }

    #[test]
    fn tab_cycles_forward_in_reading_order() {
        assert_eq!(Focus::Source.next(), Focus::Instructions);
        assert_eq!(Focus::Instructions.next(), Focus::Output);
        assert_eq!(Focus::Output.next(), Focus::Debug);
        assert_eq!(Focus::Debug.next(), Focus::Source);
    }

    #[test]
    fn shift_tab_cycles_backward_in_reading_order() {
        assert_eq!(Focus::Source.prev(), Focus::Debug);
        assert_eq!(Focus::Debug.prev(), Focus::Output);
        assert_eq!(Focus::Output.prev(), Focus::Instructions);
        assert_eq!(Focus::Instructions.prev(), Focus::Source);
    }

    #[test]
    fn next_and_prev_are_inverses_for_every_pane() {
        for focus in Focus::ALL {
            assert_eq!(focus.next().prev(), focus);
            assert_eq!(focus.prev().next(), focus);
        }
    }

    #[test]
    fn four_tabs_return_to_the_starting_pane() {
        for focus in Focus::ALL {
            let cycled = focus.next().next().next().next();
            assert_eq!(cycled, focus);
        }
    }

    #[test]
    fn switch_column_swaps_columns_and_keeps_the_row() {
        assert_eq!(Focus::Source.switch_column(), Focus::Output);
        assert_eq!(Focus::Instructions.switch_column(), Focus::Debug);
        assert_eq!(Focus::Output.switch_column(), Focus::Source);
        assert_eq!(Focus::Debug.switch_column(), Focus::Instructions);
    }

    #[test]
    fn switch_column_is_its_own_inverse_for_every_pane() {
        for focus in Focus::ALL {
            assert_eq!(focus.switch_column().switch_column(), focus);
        }
    }

    #[test]
    fn switch_column_changes_the_column_but_not_the_row() {
        // The row is spelled out here instead of read from a helper, so the assertion cannot
        // agree with a broken mapping: 0 is the top row, 1 the bottom one.
        fn row(focus: Focus) -> usize {
            match focus {
                Focus::Source | Focus::Output => 0,
                Focus::Instructions | Focus::Debug => 1,
            }
        }

        for focus in Focus::ALL {
            let other = focus.switch_column();
            assert_ne!(focus, other);
            assert_eq!(
                row(focus),
                row(other),
                "{focus:?} -> {other:?} changed rows"
            );
            assert_eq!(
                focus.is_editable(),
                !other.is_editable(),
                "{focus:?} -> {other:?} stayed in the same column"
            );
        }
    }

    #[test]
    fn editability_and_titles_match_the_spec_table() {
        let expected = [
            (Focus::Source, true, "source"),
            (Focus::Instructions, true, "instructions"),
            (Focus::Output, false, "output"),
            (Focus::Debug, false, "debug"),
        ];
        for (focus, editable, title) in expected {
            assert_eq!(focus.is_editable(), editable, "{focus:?}");
            assert_eq!(focus.title(), title, "{focus:?}");
        }
    }

    #[test]
    fn reading_order_lists_the_left_column_first() {
        assert_eq!(
            Focus::ALL,
            [
                Focus::Source,
                Focus::Instructions,
                Focus::Output,
                Focus::Debug
            ]
        );
    }

    #[test]
    fn columns_are_50_50_and_adjacent_at_80x24() {
        let panes = panes(area(80, 24));

        assert_eq!(panes.source.width, 40);
        assert_eq!(panes.output.width, 40);
        assert_eq!(panes.source.x, 0);
        assert_eq!(
            panes.output.x, 40,
            "right column must start where left ends"
        );
        assert_eq!(panes.instructions.width, 40);
        assert_eq!(panes.debug.width, 40);
    }

    #[test]
    fn odd_widths_give_the_spare_column_to_the_left_pane() {
        let panes = panes(area(81, 24));

        assert_eq!(panes.source.width, 41);
        assert_eq!(panes.output.width, 40);
        assert_eq!(panes.source.width + panes.output.width, 81);
    }

    #[test]
    fn panes_stack_in_spec_order_at_80x24() {
        // Body is 23 rows: 25% floors to 5, which is also the instructions cap.
        let panes = panes(area(80, 24));

        assert_eq!(panes.source, Rect::new(0, 0, 40, 18));
        assert_eq!(panes.instructions, Rect::new(0, 18, 40, 5));
        assert_eq!(panes.output, Rect::new(40, 0, 40, 16));
        assert_eq!(panes.debug, Rect::new(40, 16, 40, 7));
        assert_eq!(panes.status, Rect::new(0, 23, 80, 1));

        assert_eq!(panes.source.height + panes.instructions.height, 23);
        assert_eq!(panes.output.height + panes.debug.height, 23);
    }

    #[test]
    fn instructions_height_is_min_5_25_percent_floored_at_3() {
        // (terminal height, expected instructions height)
        let cases = [
            (24, 5), // 23 body rows -> 25% = 5
            (40, 5), // 39 body rows -> 25% = 9, capped at 5
            (60, 5), // 59 body rows -> 25% = 14, capped at 5
            (10, 3), // 9 body rows  -> 25% = 2, floored to 3
            (8, 3),  // 7 body rows  -> 25% = 1, floored to 3
            (5, 3),  // 4 body rows  -> floor wins over a 1-row share
            (4, 3),  // 3 body rows  -> floor is the whole column
            (3, 2),  // 2 body rows  -> clamped to what exists
            (2, 1),  // 1 body row   -> clamped to what exists
            (1, 0),  // status line takes the only row
        ];

        for (height, expected) in cases {
            let panes = panes(area(80, height));
            assert_eq!(
                panes.instructions.height, expected,
                "instructions height at 80x{height}"
            );
            assert_eq!(
                panes.source.height + panes.instructions.height,
                height.saturating_sub(1),
                "column rows must be fully and exactly distributed at 80x{height}"
            );
        }
    }

    #[test]
    fn debug_height_is_min_8_third_floored_at_3() {
        // (terminal height, expected debug height): a third reproduces the spec drawing.
        let cases = [
            (24, 7), // 23 body rows -> 23/3 = 7
            (40, 8), // 39 body rows -> 13, capped at 8
            (10, 3), // 9 body rows  -> 3
            (5, 3),  // 4 body rows  -> 1, floored to 3
        ];

        for (height, expected) in cases {
            let panes = panes(area(80, height));
            assert_eq!(panes.debug.height, expected, "debug height at 80x{height}");
            assert_eq!(
                panes.output.height + panes.debug.height,
                height.saturating_sub(1),
                "column rows must be fully and exactly distributed at 80x{height}"
            );
        }
    }

    #[test]
    fn tall_terminal_caps_both_bottom_panes() {
        let panes = panes(area(80, 60));

        assert_eq!(panes.instructions.height, 5);
        assert_eq!(panes.debug.height, 8);
        assert_eq!(panes.source.height, 54);
        assert_eq!(panes.output.height, 51);
    }

    #[test]
    fn every_size_distributes_rows_without_overflow() {
        for width in [1, 2, 3, 7, 8, 9, 20, 40, 79, 80, 81] {
            for height in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 24, 60] {
                let panes = panes(area(width, height));
                let body = height.saturating_sub(1);

                let left_rows = panes.source.height + panes.instructions.height;
                let right_rows = panes.output.height + panes.debug.height;
                assert_eq!(left_rows, body, "left column at {width}x{height}");
                assert_eq!(right_rows, body, "right column at {width}x{height}");

                assert!(
                    panes.source.x + panes.source.width <= width,
                    "left column escapes the frame at {width}x{height}"
                );
                assert!(
                    panes.output.x + panes.output.width <= width,
                    "right column escapes the frame at {width}x{height}"
                );
                assert!(
                    panes.source.y + left_rows <= body,
                    "panes escape the body at {width}x{height}"
                );

                let columns = panes.source.width + panes.output.width;
                assert_eq!(columns, width, "columns at {width}x{height}");
            }
        }
    }

    #[test]
    fn one_by_one_gives_every_pane_zero_height_and_a_status_row() {
        let panes = panes(area(1, 1));

        // Every column row goes to the status line, so the panes exist but are empty.
        for (focus, rect) in panes.all() {
            assert_eq!(rect.height, 0, "{focus:?} has rows in a 1x1 frame");
        }
        assert_eq!(panes.status, Rect::new(0, 0, 1, 1));
    }

    #[test]
    fn inner_area_excludes_the_borders() {
        let panes = panes(area(40, 10));

        let source = panes.inner(Focus::Source);
        assert_eq!(source, Rect::new(1, 1, 18, 4));
        // Whatever the pane, the inner area is one cell in from every side.
        for (focus, rect) in panes.all() {
            let inner = panes.inner(focus);
            assert_eq!(
                inner.width + 2,
                rect.width,
                "{focus:?} inner width at 40x10"
            );
            assert_eq!(
                inner.height + 2,
                rect.height,
                "{focus:?} inner height at 40x10"
            );
        }
    }

    #[test]
    fn inner_area_saturates_instead_of_underflowing() {
        let panes = panes(area(1, 1));

        for focus in Focus::ALL {
            let inner = panes.inner(focus);
            assert_eq!(inner.width, 0, "{focus:?} inner width at 1x1");
            assert_eq!(inner.height, 0, "{focus:?} inner height at 1x1");
        }
    }

    #[test]
    fn pane_rects_can_be_looked_up_by_focus() {
        let panes = panes(area(80, 24));

        assert_eq!(panes.all()[0], (Focus::Source, panes.source));
        assert_eq!(panes.all()[1], (Focus::Instructions, panes.instructions));
        assert_eq!(panes.all()[2], (Focus::Output, panes.output));
        assert_eq!(panes.all()[3], (Focus::Debug, panes.debug));
    }
}
