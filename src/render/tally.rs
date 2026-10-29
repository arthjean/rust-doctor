//! What the text counts: the category tallies and the occurrences a group
//! heading announces.
//!
//! Both count only what weighs. A finding outside production code, or one the
//! catalog does not describe, is published and costs nothing, and counting it
//! among the warnings is how this crate's own Maintainability line read 138
//! warnings of which one was scored. The rest is shown apart, so nothing the
//! report publishes vanishes from the text.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use crate::presentation::{DiagnosticGroup, GroupDiagnostic};
use crate::{AuditCategoryName, Diagnostic, InspectReport, Severity};

use super::{RenderError, Style, TerminalOptions, line};

/// What one category or one group counts, split by whether it weighs.
#[derive(Default)]
struct Split {
    weighing: crate::SeverityCounts,
    outside_production: usize,
    not_scored: usize,
}

impl Split {
    fn add(&mut self, severity: Severity, context: bool, unscored: bool, occurrences: usize) {
        if context {
            self.outside_production = self.outside_production.saturating_add(occurrences);
        } else if unscored {
            self.not_scored = self.not_scored.saturating_add(occurrences);
        } else {
            self.weighing.add(severity, occurrences);
        }
    }

    /// The occurrences set apart, in parentheses, or nothing when none were.
    fn apart(&self) -> String {
        let parts: Vec<String> = [
            (self.outside_production, "outside production code"),
            (self.not_scored, "not scored"),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, what)| format!("+{count} {what}"))
        .collect();
        if parts.is_empty() {
            String::new()
        } else {
            format!(" ({})", parts.join(", "))
        }
    }
}

/// The diagnostics the text speaks for: the introduced ones under a baseline
/// comparison, which is also the population `audit.categories` was rebuilt on.
fn scoped(report: &InspectReport) -> Vec<&Diagnostic> {
    let introduced: Option<BTreeSet<&str>> = report
        .delta
        .as_ref()
        .map(|delta| delta.introduced.iter().map(String::as_str).collect());
    report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            introduced
                .as_ref()
                .is_none_or(|introduced| introduced.contains(diagnostic.id.as_str()))
        })
        .collect()
}

fn bucket(category: Option<&str>) -> AuditCategoryName {
    category
        .and_then(crate::audit::category_mapping)
        .map_or(AuditCategoryName::Other, |(name, _)| name)
}

/// One line per category the audit published, counting what weighs, with the
/// rest set apart.
pub(super) fn render_categories<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let mut splits = BTreeMap::<AuditCategoryName, Split>::new();
    for diagnostic in scoped(report) {
        splits
            .entry(bucket(diagnostic.category.as_deref()))
            .or_default()
            .add(
                diagnostic.severity,
                diagnostic.context.is_some(),
                diagnostic.unscored.is_some(),
                diagnostic.occurrences,
            );
    }
    for category in &report.audit.categories {
        let split = splits.remove(&category.name).unwrap_or_default();
        let counts = split.weighing;
        line(
            writer,
            &format!(
                "{}: {} errors, {} warnings, {} info, {} unknown (occurrences){}",
                category.name,
                counts.errors,
                counts.warnings,
                counts.info,
                counts.unknown,
                split.apart()
            ),
            options,
            Style::Plain,
        )?;
    }
    if report.audit.categories.is_empty() {
        line(writer, "Categories: none", options, Style::Plain)?;
    }
    Ok(())
}

/// What a group announces beside its title: the weighing occurrences of its
/// own severity, then whatever else it holds.
///
/// A group takes the worst severity among its sites, and its count used to be
/// every site's: `Warning: Print stderr (33 occurrences)` over one warning and
/// thirty-two sites published at Info, since a binary's own output channel is
/// not a defect.
pub(super) fn group_occurrences(group: &DiagnosticGroup) -> String {
    let mut split = Split::default();
    for diagnostic in &group.diagnostics {
        split.add(
            diagnostic.severity,
            diagnostic.context.is_some(),
            diagnostic.unscored.is_some(),
            diagnostic.occurrences,
        );
    }
    let own = severity_count(split.weighing, group.severity);
    let unit = if own == 1 { "occurrence" } else { "occurrences" };
    let others: Vec<String> = [
        Severity::Error,
        Severity::Warning,
        Severity::Info,
        Severity::Unknown,
    ]
    .into_iter()
    .filter(|severity| *severity != group.severity)
    .filter_map(|severity| {
        let count = severity_count(split.weighing, severity);
        (count > 0).then(|| format!("{count} {severity}{}", where_published(group, severity)))
    })
    .chain(
        [
            (split.outside_production, "outside production code"),
            (split.not_scored, "not scored"),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, what)| format!("{count} {what}")),
    )
    .collect();
    if others.is_empty() {
        format!("{own} {unit}")
    } else {
        format!("{own} {unit}, {}", others.join(", "))
    }
}

/// Why a rule's sites sit at a severity below the one its group takes, when
/// the report knows: the binary output channels are published at Info.
fn where_published(group: &DiagnosticGroup, severity: Severity) -> &'static str {
    let binary_channel = crate::report::BINARY_CHANNEL_RULES.contains(&group.rule_id.as_str());
    if binary_channel && severity == Severity::Info {
        " in binary targets"
    } else {
        ""
    }
}

const fn severity_count(counts: crate::SeverityCounts, severity: Severity) -> usize {
    match severity {
        Severity::Error => counts.errors,
        Severity::Warning => counts.warnings,
        Severity::Info => counts.info,
        Severity::Unknown => counts.unknown,
    }
}

/// The occurrences of one site the summary row counts: the same split, so the
/// row and the verbose heading never disagree on a group.
pub(super) fn summary_occurrences(group: &DiagnosticGroup) -> String {
    let weighing: usize = group
        .diagnostics
        .iter()
        .filter(|diagnostic| counts_at(diagnostic, group.severity))
        .map(|diagnostic| diagnostic.occurrences)
        .sum();
    let rest = group.occurrences.saturating_sub(weighing);
    if rest == 0 {
        format!("x{weighing}")
    } else {
        format!("x{weighing} (+{rest} more)")
    }
}

fn counts_at(diagnostic: &GroupDiagnostic, severity: Severity) -> bool {
    diagnostic.severity == severity && diagnostic.context.is_none() && diagnostic.unscored.is_none()
}
