use std::collections::BTreeSet;

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the levels and the groups are read by the coverage tests"
    )
)]
pub(crate) enum TableLine {
    Lint { name: String, level: &'static str },
    Group { name: String, members: Vec<String> },
}

pub(crate) fn normalize(name: &str) -> Option<String> {
    let suffix = name.trim().trim_end_matches(',').strip_prefix("clippy::")?;
    (!suffix.is_empty()).then(|| format!("clippy::{}", suffix.replace('-', "_")))
}

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
