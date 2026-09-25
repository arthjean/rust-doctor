//! What the configuration and the reader took out of the report, and the
//! score of each member: the lines that say the report is narrower than the
//! workspace, and by how much.

use std::io::Write;

use crate::{InspectReport, SuppressionStatus};

use super::{RenderError, Style, TerminalOptions, line};

/// One line for what left the report, one for the directives, and under
/// `--verbose` one line per scanned member.
pub(super) fn render_narrowing<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let scan = &report.scan;
    if scan.ignored > 0 || scan.excluded_generated > 0 {
        line(
            writer,
            &format!(
                "Left out: {} in ignored paths, {} in generated or vendored files",
                findings(scan.ignored),
                findings(scan.excluded_generated)
            ),
            options,
            Style::Muted,
        )?;
    }
    if let Some(summary) = suppression_summary(report) {
        line(writer, &summary, options, Style::Muted)?;
    }
    if options.verbose {
        for package in &report.audit.packages {
            let score = package.score.as_ref().map_or_else(
                || "no production lines".to_owned(),
                |score| {
                    let tier = score.worst_tier.map_or("none", |tier| tier.as_str());
                    format!("{}/100, worst tier {tier}", score.value)
                },
            );
            line(
                writer,
                &format!("Member {}: {score}", package.name),
                options,
                Style::Plain,
            )?;
        }
    }
    Ok(())
}

fn findings(count: usize) -> String {
    match count {
        1 => "1 finding".to_owned(),
        count => format!("{count} findings"),
    }
}

/// `Suppressions: 3 applied, 1 unused, 1 missing-reason`, statuses in their
/// declared order, absent ones left out.
fn suppression_summary(report: &InspectReport) -> Option<String> {
    if report.suppressions.is_empty() {
        return None;
    }
    let mut statuses: Vec<SuppressionStatus> = report
        .suppressions
        .iter()
        .map(|suppression| suppression.status)
        .collect();
    statuses.sort();
    let mut parts = Vec::new();
    for status in statuses.iter().copied() {
        if parts.last().is_some_and(|(last, _)| *last == status) {
            if let Some((_, count)) = parts.last_mut() {
                *count += 1;
            }
        } else {
            parts.push((status, 1_usize));
        }
    }
    let parts: Vec<String> = parts
        .into_iter()
        .map(|(status, count)| format!("{count} {}", status.as_str()))
        .collect();
    Some(format!("Suppressions: {}", parts.join(", ")))
}
