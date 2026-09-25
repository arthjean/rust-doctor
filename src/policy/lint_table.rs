//! The lint table `clippy-driver -W help` prints, read line by line.
//!
//! Two readers ask it questions. The coverage tests read every lint with its
//! default level and every group with its members, to size the candidate
//! queue. The scan reads which lints the installed Clippy knows before it
//! passes `-W` for any of them, so a catalogued rule the toolchain has renamed
//! or not yet shipped is published as not evaluated instead of vanishing
//! behind `unknown_lints`. One parser serves both, so the two cannot disagree
//! on what the table says.

use std::collections::BTreeSet;

/// One meaningful line of the table.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the levels and the groups are read by the coverage tests"
    )
)]
pub(crate) enum TableLine {
    /// A lint and the level column, `allow`, `warn` or `deny`.
    Lint { name: String, level: &'static str },
    /// A group and its members.
    Group { name: String, members: Vec<String> },
}

/// Canonical form of a lint or group name: the table prints kebab-case, the
/// catalog stores snake_case, and group members carry a trailing comma.
pub(crate) fn normalize(name: &str) -> Option<String> {
    let suffix = name.trim().trim_end_matches(',').strip_prefix("clippy::")?;
    (!suffix.is_empty()).then(|| format!("clippy::{}", suffix.replace('-', "_")))
}

/// A lint line carries a level in its second field, a group line carries its
/// first member there, so the two never mix. Anything else is not a line of
/// the table.
pub(crate) fn parse_line(line: &str) -> Option<TableLine> {
    let mut fields = line.split_whitespace();
    let name = fields.next().and_then(normalize)?;
    let second = fields.next()?;
    if let Some(level) = ["allow", "warn", "deny"]
        .into_iter()
        .find(|level| *level == second)
    {
        return Some(TableLine::Lint { name, level });
    }
    second.starts_with("clippy::").then(|| TableLine::Group {
        name,
        members: std::iter::once(second)
            .chain(fields)
            .filter_map(normalize)
            .collect(),
    })
}

/// Every Clippy lint the table lists, in the catalog's spelling.
pub(crate) fn lint_names(table: &str) -> BTreeSet<String> {
    table
        .lines()
        .filter_map(parse_line)
        .filter_map(|line| match line {
            TableLine::Lint { name, .. } => Some(name),
            TableLine::Group { .. } => None,
        })
        .collect()
}
