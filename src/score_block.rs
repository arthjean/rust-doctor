use std::time::Duration;

use crate::{ScoreDimensions, ScoreLabel};

pub const PERFECT_SCORE: u8 = 100;

pub const BRANDING_NAME: &str = "Rust Doctor";
pub const BRANDING_URL: &str = "https://rust-doctor.com";

pub const FACE_TOP: &str = "┌─────┐";
pub const FACE_BOTTOM: &str = "└─────┘";

pub const INDENT_COLUMNS: usize = 2;
pub const GAP_COLUMNS: usize = 2;
pub const FACE_WIDTH_COLUMNS: usize = 7;
pub const FACE_OFFSET_COLUMNS: usize = INDENT_COLUMNS + FACE_WIDTH_COLUMNS + GAP_COLUMNS;

pub const RIGHT_EDGE_SAFETY_COLUMNS: usize = 1;

pub const BAR_MIN_WIDTH_CHARS: usize = 10;
pub const BAR_MAX_WIDTH_CHARS: usize = 50;

pub const MIN_BLOCK_COLUMNS: usize =
    FACE_OFFSET_COLUMNS + RIGHT_EDGE_SAFETY_COLUMNS + BAR_MIN_WIDTH_CHARS;

pub const BLOCK_ROWS: usize = 4;

pub const COUNT_UP_FRAME_COUNT: u32 = 40;
pub const COUNT_UP_FRAME_DELAY: Duration = Duration::from_millis(50);

pub const fn face(label: ScoreLabel) -> [&'static str; 2] {
    match label {
        ScoreLabel::Great => ["◠ ◠", " ▽ "],
        ScoreLabel::NeedsWork => ["• •", " ─ "],
        ScoreLabel::Critical => ["x x", " ▽ "],
    }
}

pub fn face_rows(label: ScoreLabel) -> [String; BLOCK_ROWS] {
    let [eyes, mouth] = face(label);
    [
        FACE_TOP.to_owned(),
        format!("│ {eyes} │"),
        format!("│ {mouth} │"),
        FACE_BOTTOM.to_owned(),
    ]
}

pub const fn label_text(label: ScoreLabel, authoritative: bool) -> &'static str {
    if authoritative {
        label.as_str()
    } else {
        "Core partial"
    }
}

pub fn dimensions_line(dimensions: ScoreDimensions) -> String {
    format!(
        "Security {} · Reliability {} · Maintainability {} · Performance {} · Dependencies {}",
        dimensions.security,
        dimensions.reliability,
        dimensions.maintainability,
        dimensions.performance,
        dimensions.dependencies
    )
}

pub const fn label_for(value: u8) -> ScoreLabel {
    if value >= 75 {
        ScoreLabel::Great
    } else if value >= 50 {
        ScoreLabel::NeedsWork
    } else {
        ScoreLabel::Critical
    }
}

pub fn bar_fill(value: u8, bar_width: usize) -> usize {
    (usize::from(value) * bar_width)
        .div_ceil(usize::from(PERFECT_SCORE))
        .min(bar_width)
}

pub const fn right_column_width(columns: usize) -> usize {
    columns.saturating_sub(FACE_OFFSET_COLUMNS + RIGHT_EDGE_SAFETY_COLUMNS)
}

pub fn bar_width(columns: usize) -> Option<usize> {
    let available = right_column_width(columns);
    (available >= BAR_MIN_WIDTH_CHARS).then(|| available.min(BAR_MAX_WIDTH_CHARS))
}

fn ease_out_cubic(progress: f64) -> f64 {
    1.0 - (1.0 - progress).powi(3)
}

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

    #[test]
    fn a_box_narrower_than_the_block_leaves_no_right_column() {
        assert_eq!(right_column_width(0), 0);
        assert_eq!(right_column_width(FACE_OFFSET_COLUMNS), 0);
        assert_eq!(right_column_width(MIN_BLOCK_COLUMNS), BAR_MIN_WIDTH_CHARS);
        assert_eq!(right_column_width(80), 68);
    }

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
