//! The score block both reports draw.
//!
//! [`crate::render`] paints it as styled strings for the linear report, and the
//! interactive report composes it out of styled spans. Everything the two must
//! agree on lives here: the faces, the label a score carries, how a value fills
//! the bar, how much room the block needs and the cadence of its count-up.
//!
//! They were two independent transpositions of one React Doctor component, each
//! carrying its own copy of those tables, and they had already drifted apart on
//! the fill rounding, on the guard column and on the count-up easing. One table
//! is what keeps a score reading the same in both reports.

use std::time::Duration;

use crate::ScoreLabel;

pub const PERFECT_SCORE: u8 = 100;

pub const BRANDING_NAME: &str = "Rust Doctor";
pub const BRANDING_URL: &str = "https://rust-doctor.com";

pub const FACE_TOP: &str = "┌─────┐";
pub const FACE_BOTTOM: &str = "└─────┘";

/// Columns left of the face, and between the face and the right column.
pub const INDENT_COLUMNS: usize = 2;
pub const GAP_COLUMNS: usize = 2;
/// Display width of every face row.
pub const FACE_WIDTH_COLUMNS: usize = 7;
/// Where the right column starts, counted from the left edge of the block.
pub const FACE_OFFSET_COLUMNS: usize = INDENT_COLUMNS + FACE_WIDTH_COLUMNS + GAP_COLUMNS;

/// One column stays free on the right edge: a row that fills the terminal
/// exactly wraps implicitly on some emulators, which would desynchronize the
/// cursor rewind both reports rely on between two frames.
pub const RIGHT_EDGE_SAFETY_COLUMNS: usize = 1;

/// Under this the block is worth nothing, and the caller falls back on a single
/// line.
pub const BAR_MIN_WIDTH_CHARS: usize = 10;
pub const BAR_MAX_WIDTH_CHARS: usize = 50;

/// Columns a box must carry for the block to be worth drawing: the face, its
/// paddings, the guard column and the shortest bar. A caller that guarantees
/// this width upstream needs no narrow-terminal branch at all.
pub const MIN_BLOCK_COLUMNS: usize =
    FACE_OFFSET_COLUMNS + RIGHT_EDGE_SAFETY_COLUMNS + BAR_MIN_WIDTH_CHARS;

/// Rows the block occupies, whatever the score. Both reports rewind by this
/// many rows between two frames, so the face, the right column and the rewind
/// count one number rather than three: a row that grew without the rewind
/// following it moves the cursor away from where the next frame expects it.
pub const BLOCK_ROWS: usize = 4;

/// The count-up of `use-animated-score.ts`, shared by both reports. What
/// follows it differs (the linear report scrolls a rainbow over a perfect
/// score, the interactive one grows the projected gain), so each owns the
/// cadence of its own second phase.
pub const COUNT_UP_FRAME_COUNT: u32 = 40;
pub const COUNT_UP_FRAME_DELAY: Duration = Duration::from_millis(50);

pub const fn face(label: ScoreLabel) -> [&'static str; 2] {
    match label {
        ScoreLabel::Great => ["◠ ◠", " ▽ "],
        ScoreLabel::NeedsWork => ["• •", " ─ "],
        ScoreLabel::Critical => ["x x", " ▽ "],
    }
}

/// The four rows of the face, top to bottom.
pub fn face_rows(label: ScoreLabel) -> [String; BLOCK_ROWS] {
    let [eyes, mouth] = face(label);
    [
        FACE_TOP.to_owned(),
        format!("│ {eyes} │"),
        format!("│ {mouth} │"),
        FACE_BOTTOM.to_owned(),
    ]
}

/// What the score calls itself. A report whose core passes could not be judged
/// says so rather than publishing a band it did not earn.
pub const fn label_text(label: ScoreLabel, authoritative: bool) -> &'static str {
    if authoritative {
        label.as_str()
    } else {
        "Core partial"
    }
}

/// The bands the audit publishes, applied to a projected value the audit does
/// not label itself.
pub const fn label_for(value: u8) -> ScoreLabel {
    if value >= 75 {
        ScoreLabel::Great
    } else if value >= 50 {
        ScoreLabel::NeedsWork
    } else {
        ScoreLabel::Critical
    }
}

/// Cells a value fills on a bar of `bar_width`. It rounds up, so a workspace
/// that scored at all never reads as an empty bar, however narrow the terminal.
pub fn bar_fill(value: u8, bar_width: usize) -> usize {
    (usize::from(value) * bar_width)
        .div_ceil(usize::from(PERFECT_SCORE))
        .min(bar_width)
}

/// Room the score, the bar and the branding share to the right of the face,
/// inside a box `columns` wide.
///
/// Total, because "how much room is there" always has an answer: a box too
/// narrow simply leaves none. Whether the block is worth drawing at all is
/// [`bar_width`], and keeping the two questions apart is what lets a caller
/// that already guarantees [`MIN_BLOCK_COLUMNS`] carry no fallback.
pub const fn right_column_width(columns: usize) -> usize {
    columns.saturating_sub(FACE_OFFSET_COLUMNS + RIGHT_EDGE_SAFETY_COLUMNS)
}

/// Bar width inside a box `columns` wide, `None` when the box cannot carry the
/// shortest bar and the caller must fall back on a single line.
pub fn bar_width(columns: usize) -> Option<usize> {
    let available = right_column_width(columns);
    (available >= BAR_MIN_WIDTH_CHARS).then(|| available.min(BAR_MAX_WIDTH_CHARS))
}

fn ease_out_cubic(progress: f64) -> f64 {
    1.0 - (1.0 - progress).powi(3)
}

/// Value shown at one frame of an eased run from `from` to `to`: it starts on
/// `from`, lands exactly on `to`, and never goes back.
pub fn eased(from: u8, to: u8, frame: u32, frames: u32) -> u8 {
    if frames == 0 {
        return to;
    }
    let progress = ease_out_cubic(f64::from(frame) / f64::from(frames));
    let value = f64::from(to)
        .mul_add(progress, f64::from(from) * (1.0 - progress))
        .round();
    if value <= 0.0 {
        0
    } else if value >= f64::from(PERFECT_SCORE) {
        PERFECT_SCORE
    } else {
        value as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_text::display_width;

    #[test]
    fn the_declared_face_width_is_the_width_the_face_measures() {
        assert_eq!(display_width(FACE_TOP), FACE_WIDTH_COLUMNS);
        assert_eq!(display_width(FACE_BOTTOM), FACE_WIDTH_COLUMNS);
        for label in [
            ScoreLabel::Great,
            ScoreLabel::NeedsWork,
            ScoreLabel::Critical,
        ] {
            for row in face_rows(label) {
                assert_eq!(display_width(&row), FACE_WIDTH_COLUMNS, "row {row:?}");
            }
        }
    }

    /// The block keeps exactly one column free on the right edge, and declines
    /// a box that cannot carry the shortest bar. [`MIN_BLOCK_COLUMNS`] is the
    /// threshold itself, so a caller can guarantee it instead of branching on
    /// it.
    #[test]
    fn the_block_declines_a_box_too_narrow_for_its_face_and_shortest_bar() {
        assert_eq!(MIN_BLOCK_COLUMNS, 22);
        assert_eq!(bar_width(MIN_BLOCK_COLUMNS - 1), None);
        assert_eq!(bar_width(MIN_BLOCK_COLUMNS), Some(BAR_MIN_WIDTH_CHARS));
        assert_eq!(bar_width(80), Some(BAR_MAX_WIDTH_CHARS));
        for columns in [MIN_BLOCK_COLUMNS, 40, 61, 200] {
            let width = bar_width(columns).unwrap();
            assert!(
                FACE_OFFSET_COLUMNS + width <= columns - RIGHT_EDGE_SAFETY_COLUMNS,
                "the block overflows a box of {columns} columns"
            );
        }
    }

    /// The right column is total: a box narrower than the block leaves nothing
    /// to its right rather than refusing to answer.
    #[test]
    fn a_box_narrower_than_the_block_leaves_no_right_column() {
        assert_eq!(right_column_width(0), 0);
        assert_eq!(right_column_width(FACE_OFFSET_COLUMNS), 0);
        assert_eq!(right_column_width(MIN_BLOCK_COLUMNS), BAR_MIN_WIDTH_CHARS);
        assert_eq!(right_column_width(80), 68);
    }

    /// The rounding a bar reads by: a workspace that scored shows at least one
    /// cell, and a perfect one fills the bar exactly.
    #[test]
    fn a_score_that_is_not_zero_always_lights_at_least_one_cell() {
        assert_eq!(bar_fill(0, 10), 0);
        assert_eq!(bar_fill(1, 10), 1);
        assert_eq!(bar_fill(1, 50), 1);
        assert_eq!(bar_fill(100, 10), 10);
        assert_eq!(bar_fill(100, 50), 50);
        for width in [BAR_MIN_WIDTH_CHARS, 23, BAR_MAX_WIDTH_CHARS] {
            for value in 1..=PERFECT_SCORE {
                let fill = bar_fill(value, width);
                assert!((1..=width).contains(&fill), "{value} on {width} gave {fill}");
            }
        }
    }

    #[test]
    fn the_count_up_starts_where_it_is_told_lands_on_the_target_and_never_goes_back() {
        for value in [1u8, 42, 99, 100] {
            let counted: Vec<u8> = (0..=COUNT_UP_FRAME_COUNT)
                .map(|frame| eased(0, value, frame, COUNT_UP_FRAME_COUNT))
                .collect();
            assert_eq!(counted[0], 0);
            assert_eq!(counted[counted.len() - 1], value);
            assert!(counted.windows(2).all(|pair| pair[0] <= pair[1]));
        }
        assert_eq!(eased(60, 90, 0, 16), 60);
        assert_eq!(eased(60, 90, 16, 16), 90);
        assert_eq!(eased(60, 90, 8, 0), 90);
    }

    #[test]
    fn a_projected_value_is_labelled_by_the_bands_the_audit_publishes() {
        assert_eq!(label_for(100), ScoreLabel::Great);
        assert_eq!(label_for(75), ScoreLabel::Great);
        assert_eq!(label_for(74), ScoreLabel::NeedsWork);
        assert_eq!(label_for(50), ScoreLabel::NeedsWork);
        assert_eq!(label_for(49), ScoreLabel::Critical);
        assert_eq!(label_text(ScoreLabel::Great, false), "Core partial");
        assert_eq!(
            label_text(ScoreLabel::Great, true),
            ScoreLabel::Great.as_str()
        );
    }
}
