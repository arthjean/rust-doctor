//! What the interactive report draws, derived once from the inspection.
//!
//! A [`Row`] is React Doctor's `DiagnosticRow`: one catalogued rule, its worst
//! site and everywhere else it fired. [`Entry`] interleaves the category
//! headers the list column shows, and [`Layout`] is the transposition of
//! `resolve-report-layout.ts`, so the same terminal geometry picks the same
//! split, stacked or compact arrangement it does.

use rust_doctor::presentation::{DiagnosticGroup, GroupLocation, ReportPresentation};
use rust_doctor::{AuditCategoryName, ScoreLabel, Severity};

use super::text::Color;

pub const PERFECT_SCORE: u8 = 100;

const SCORE_BAR_WIDTH_CHARS: usize = 50;
const SCORE_BAR_MIN_WIDTH_CHARS: usize = 10;

pub const HORIZONTAL_PADDING_COLUMNS: usize = 2;
pub const DETAIL_INDENT_COLUMNS: usize = 2;
pub const ACTION_MENU_MARGIN_ROWS: usize = 1;
pub const ACTION_MENU_ITEM_GAP_ROWS: usize = 1;

const REPORT_DETAIL_ROWS: usize = 15;
const REPORT_STATUS_ROWS: usize = 3;
const REPORT_DIVIDER_ROWS: usize = 1;
const REPORT_LIST_MARGIN_ROWS: usize = 1;
const REPORT_MIN_LIST_ROWS: usize = 3;
const REPORT_STACKED_MAX_LIST_ROWS: usize = 16;
const REPORT_VIEWPORT_MARGIN_ROWS: usize = 1;
const REPORT_VIEWER_SCORE_HEADER_ROWS: usize = 4;
const REPORT_COMPACT_STATUS_ROWS: usize = 1;
const REPORT_COMPACT_MAX_ROWS: usize = REPORT_LIST_MARGIN_ROWS
    + REPORT_DIVIDER_ROWS
    + REPORT_STATUS_ROWS
    + REPORT_DETAIL_ROWS
    + REPORT_MIN_LIST_ROWS;
const REPORT_MIN_WIDTH_CHARS: usize = 1;
const REPORT_WIDE_MIN_COLUMNS: usize = 120;
const REPORT_WIDE_MIN_ROWS: usize = 22;
const REPORT_DETAIL_WIDTH_NUMERATOR: usize = 3;
const REPORT_DETAIL_WIDTH_DENOMINATOR: usize = 5;
const REPORT_COLUMN_GUTTER_COLUMNS: usize = 3;
const REPORT_MIN_COLUMN_WIDTH_CHARS: usize = 20;
pub const REPORT_SPLIT_MARGIN_COLUMNS: usize = 1;
pub const REPORT_SPLIT_PADDING_COLUMNS: usize = 1;
const SCORE_FACE_OFFSET_COLUMNS: usize = 11;
const SCORE_RIGHT_EDGE_SAFETY_COLUMNS: usize = 2;

/// One catalogued rule and every site it fired on.
#[derive(Debug, Clone)]
pub struct Row {
    pub rule_id: String,
    pub title: String,
    pub category: AuditCategoryName,
    pub severity: Severity,
    pub site_count: usize,
    pub location: String,
    pub message: String,
    pub help: Option<String>,
    pub rule_url: String,
    /// The site the detail pane frames: the first one carrying a span.
    pub frame: Option<GroupLocation>,
    /// Every distinct `path:line` the rule reached, in report order.
    pub sites: Vec<String>,
}

impl Row {
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

#[derive(Debug, Clone)]
pub enum Entry {
    Header(AuditCategoryName),
    Item { row_index: usize },
}

impl Entry {
    pub const fn row_index(&self) -> Option<usize> {
        match self {
            Self::Header(_) => None,
            Self::Item { row_index } => Some(*row_index),
        }
    }
}

pub fn build_rows(presentation: &ReportPresentation) -> Vec<Row> {
    presentation.groups.iter().map(row_from_group).collect()
}

fn row_from_group(group: &DiagnosticGroup) -> Row {
    let representative = group
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.location().is_some())
        .or_else(|| group.diagnostics.first());
    let frame = representative.and_then(|diagnostic| diagnostic.location());
    let mut sites: Vec<String> = Vec::new();
    for diagnostic in &group.diagnostics {
        for location in diagnostic
            .location()
            .into_iter()
            .chain(diagnostic.related.iter().cloned())
        {
            let site = format_site(&location);
            if !sites.contains(&site) {
                sites.push(site);
            }
        }
    }
    Row {
        rule_id: group.rule_id.clone(),
        title: group.title.clone(),
        category: group.category.unwrap_or(AuditCategoryName::Other),
        severity: group.severity,
        site_count: group.occurrences,
        location: frame.as_ref().map_or_else(String::new, format_site),
        message: representative.map_or_else(String::new, |diagnostic| diagnostic.message.clone()),
        help: representative.and_then(|diagnostic| {
            diagnostic
                .help
                .clone()
                .or_else(|| rust_doctor::presentation::canonical_rule_help(&group.rule_id).map(str::to_owned))
        }),
        rule_url: group.rule_url.clone(),
        frame,
        sites,
    }
}

pub fn format_site(location: &GroupLocation) -> String {
    if location.span.line_start > 0 {
        format!("{}:{}", location.path, location.span.line_start)
    } else {
        location.path.clone()
    }
}

/// Groups the rows under their category header, categories in the order the
/// audit publishes them and rows in the order the report ranked them.
pub fn build_entries(rows: &[Row]) -> Vec<Entry> {
    const ORDER: [AuditCategoryName; 6] = [
        AuditCategoryName::Security,
        AuditCategoryName::Bugs,
        AuditCategoryName::Performance,
        AuditCategoryName::Dependencies,
        AuditCategoryName::Maintainability,
        AuditCategoryName::Other,
    ];
    let mut entries = Vec::new();
    for category in ORDER {
        let mut matching = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.category == category)
            .peekable();
        if matching.peek().is_none() {
            continue;
        }
        entries.push(Entry::Header(category));
        entries.extend(matching.map(|(row_index, _)| Entry::Item { row_index }));
    }
    entries
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrangement {
    Compact,
    Split,
    Stacked,
}

#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub arrangement: Arrangement,
    pub width: usize,
    pub list_column_width: usize,
    pub detail_column_width: usize,
    pub list_height: usize,
    pub detail_height: usize,
    pub shows_viewer_score_header: bool,
}

/// Direct transposition of `resolveReportLayout`. The two thresholds it turns
/// on are the ones that matter: a terminal at least 120 columns by 22 rows gets
/// the list and the detail side by side, and anything under 23 rows drops the
/// detail pane rather than squeezing both into nothing.
pub fn resolve_layout(columns: usize, rows: usize, entry_count: usize) -> Layout {
    let width = columns
        .saturating_sub(HORIZONTAL_PADDING_COLUMNS)
        .max(REPORT_MIN_WIDTH_CHARS);
    let report_rows = rows.saturating_sub(REPORT_VIEWPORT_MARGIN_ROWS);
    let is_wide = columns >= REPORT_WIDE_MIN_COLUMNS && rows >= REPORT_WIDE_MIN_ROWS;
    let is_compact = !is_wide && rows <= REPORT_COMPACT_MAX_ROWS;
    let stacked_fixed_rows = REPORT_VIEWER_SCORE_HEADER_ROWS
        + REPORT_LIST_MARGIN_ROWS
        + REPORT_DIVIDER_ROWS
        + REPORT_STATUS_ROWS;
    let shows_viewer_score_header = !is_compact
        || report_rows
            >= REPORT_VIEWER_SCORE_HEADER_ROWS + REPORT_COMPACT_STATUS_ROWS + REPORT_MIN_LIST_ROWS;

    let detail_height = if is_wide {
        report_rows.saturating_sub(REPORT_STATUS_ROWS)
    } else {
        REPORT_DETAIL_ROWS.min(
            report_rows
                .saturating_sub(stacked_fixed_rows)
                .saturating_sub(REPORT_MIN_LIST_ROWS),
        )
    };

    let available_list_height = if is_wide {
        detail_height
            .saturating_sub(REPORT_VIEWER_SCORE_HEADER_ROWS)
            .saturating_sub(REPORT_LIST_MARGIN_ROWS)
            .max(REPORT_MIN_LIST_ROWS)
    } else {
        report_rows
            .saturating_sub(stacked_fixed_rows)
            .saturating_sub(detail_height)
            .max(REPORT_MIN_LIST_ROWS)
    };

    let list_height = if is_compact {
        let header_rows = if shows_viewer_score_header {
            REPORT_VIEWER_SCORE_HEADER_ROWS
        } else {
            0
        };
        entry_count.min(
            report_rows
                .saturating_sub(REPORT_COMPACT_STATUS_ROWS)
                .saturating_sub(header_rows)
                .max(REPORT_MIN_LIST_ROWS),
        )
    } else if is_wide {
        available_list_height
    } else {
        entry_count
            .min(REPORT_STACKED_MAX_LIST_ROWS)
            .min(available_list_height)
    };

    let detail_column_width = (width * REPORT_DETAIL_WIDTH_NUMERATOR
        / REPORT_DETAIL_WIDTH_DENOMINATOR)
        .max(REPORT_MIN_COLUMN_WIDTH_CHARS);
    let list_column_width = width
        .saturating_sub(detail_column_width)
        .saturating_sub(REPORT_COLUMN_GUTTER_COLUMNS)
        .max(REPORT_MIN_COLUMN_WIDTH_CHARS);

    let arrangement = if is_compact {
        Arrangement::Compact
    } else if is_wide {
        Arrangement::Split
    } else {
        Arrangement::Stacked
    };

    Layout {
        arrangement,
        width,
        list_column_width,
        detail_column_width,
        list_height,
        detail_height,
        shows_viewer_score_header,
    }
}

/// Bar width of the score block, and whether the block fits the face at all.
pub fn score_bar_width(available_width: usize) -> Option<usize> {
    let minimum =
        SCORE_FACE_OFFSET_COLUMNS + SCORE_BAR_MIN_WIDTH_CHARS + SCORE_RIGHT_EDGE_SAFETY_COLUMNS;
    if available_width < minimum {
        return None;
    }
    Some(
        score_right_column_width(available_width)
            .clamp(SCORE_BAR_MIN_WIDTH_CHARS, SCORE_BAR_WIDTH_CHARS),
    )
}

/// Room the score, the bar and the branding share to the right of the face.
/// One guard column stays free on the right edge: a row that fills the terminal
/// exactly wraps implicitly on some emulators, which would desynchronize the
/// rewind between two frames.
pub fn score_right_column_width(available_width: usize) -> usize {
    available_width
        .saturating_sub(SCORE_FACE_OFFSET_COLUMNS)
        .saturating_sub(SCORE_RIGHT_EDGE_SAFETY_COLUMNS)
}

pub struct SeverityVariant {
    pub color: Color,
    pub icon: &'static str,
    pub label: &'static str,
}

pub const fn severity_variant(severity: Severity) -> SeverityVariant {
    match severity {
        Severity::Error => SeverityVariant {
            color: Color::Red,
            icon: "✖",
            label: "error",
        },
        Severity::Warning => SeverityVariant {
            color: Color::Yellow,
            icon: "⚠",
            label: "warning",
        },
        Severity::Info => SeverityVariant {
            color: Color::Cyan,
            icon: "ℹ",
            label: "info",
        },
        Severity::Unknown => SeverityVariant {
            color: Color::Gray,
            icon: "•",
            label: "unknown",
        },
    }
}

pub const fn score_color(label: ScoreLabel) -> Color {
    match label {
        ScoreLabel::Great => Color::Green,
        ScoreLabel::NeedsWork => Color::Yellow,
        ScoreLabel::Critical => Color::Red,
    }
}

pub const fn doctor_face(label: ScoreLabel) -> [&'static str; 2] {
    match label {
        ScoreLabel::Great => ["◠ ◠", " ▽ "],
        ScoreLabel::NeedsWork => ["• •", " ─ "],
        ScoreLabel::Critical => ["x x", " ▽ "],
    }
}

/// What the reader loses by leaving a category unfixed. `Other` has none: a
/// diagnostic with no catalogued category has no promise attached to it.
pub const fn category_impact(category: AuditCategoryName) -> Option<&'static str> {
    match category {
        AuditCategoryName::Security => Some(
            "An attacker can read data you hold, act as your users, or run code you never shipped.",
        ),
        AuditCategoryName::Bugs => {
            Some("Real users hit panics, wrong output, or state that silently corrupts.")
        }
        AuditCategoryName::Performance => {
            Some("Every run pays for the extra work, in latency and in allocations.")
        }
        AuditCategoryName::Dependencies => {
            Some("Your build resolves code you did not choose and cannot reproduce.")
        }
        AuditCategoryName::Maintainability => {
            Some("Every future change gets slower and riskier to make.")
        }
        AuditCategoryName::Other => None,
    }
}

pub fn pluralize(count: usize, singular: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {singular}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_terminal_splits_the_list_and_the_detail() {
        let layout = resolve_layout(160, 48, 20);
        assert_eq!(layout.arrangement, Arrangement::Split);
        assert_eq!(layout.width, 158);
        assert_eq!(layout.detail_column_width, 94);
        assert_eq!(layout.list_column_width, 61);
        // The split body is exactly the detail height, and the score header
        // plus its margin plus the list fills the same column.
        assert_eq!(layout.detail_height, 44);
        assert_eq!(layout.list_height, 44 - 4 - 1);
    }

    #[test]
    fn a_narrow_terminal_stacks_and_a_short_one_drops_the_detail() {
        let stacked = resolve_layout(100, 40, 30);
        assert_eq!(stacked.arrangement, Arrangement::Stacked);
        assert_eq!(stacked.detail_height, 15);
        assert!(stacked.list_height <= REPORT_STACKED_MAX_LIST_ROWS);

        let compact = resolve_layout(100, 20, 30);
        assert_eq!(compact.arrangement, Arrangement::Compact);
        assert!(compact.shows_viewer_score_header);

        let tiny = resolve_layout(100, 6, 30);
        assert_eq!(tiny.arrangement, Arrangement::Compact);
        assert!(!tiny.shows_viewer_score_header);
        assert!(tiny.list_height >= REPORT_MIN_LIST_ROWS);
    }

    #[test]
    fn the_score_block_declines_a_terminal_too_narrow_for_its_face() {
        assert_eq!(score_bar_width(22), None);
        assert_eq!(score_bar_width(23), Some(10));
        assert_eq!(score_bar_width(80), Some(50));
    }

    #[test]
    fn entries_open_each_category_in_audit_order() {
        let rows = vec![
            row("clippy::a", AuditCategoryName::Maintainability),
            row("clippy::b", AuditCategoryName::Security),
            row("clippy::c", AuditCategoryName::Maintainability),
        ];
        let entries = build_entries(&rows);
        let shape: Vec<String> = entries
            .iter()
            .map(|entry| match entry {
                Entry::Header(category) => category.as_str().to_owned(),
                Entry::Item { row_index } => rows[*row_index].rule_id.clone(),
            })
            .collect();
        assert_eq!(
            shape,
            [
                "Security",
                "clippy::b",
                "Maintainability",
                "clippy::a",
                "clippy::c"
            ]
        );
    }

    fn row(rule_id: &str, category: AuditCategoryName) -> Row {
        Row {
            rule_id: rule_id.to_owned(),
            title: rule_id.to_owned(),
            category,
            severity: Severity::Warning,
            site_count: 1,
            location: String::new(),
            message: String::new(),
            help: None,
            rule_url: String::new(),
            frame: None,
            sites: Vec::new(),
        }
    }
}
