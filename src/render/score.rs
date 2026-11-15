use std::io::Write;

use crate::{AuditCategoryName, InspectReport};
use crate::presentation::{DiagnosticGroup, ReportPresentation};

use super::{RenderError, Style, TerminalOptions, line, score_header};

fn named_rules(ceiling: &crate::ScoreCeiling) -> String {
    const MAX_NAMED_RULES: usize = 3;
    let mut named: Vec<&str> = ceiling
        .rule_ids
        .iter()
        .take(MAX_NAMED_RULES)
        .map(String::as_str)
        .collect();
    let rest = ceiling.rule_ids.len().saturating_sub(MAX_NAMED_RULES);
    let more = format!("{rest} more");
    if rest > 0 {
        named.push(&more);
    }
    named.join(", ")
}

fn dimension_value(
    dimensions: crate::ScoreDimensions,
    dimension: &str,
) -> Option<(&'static str, u8, AuditCategoryName)> {
    Some(match dimension {
        "security" => ("Security", dimensions.security, AuditCategoryName::Security),
        "reliability" => ("Reliability", dimensions.reliability, AuditCategoryName::Bugs),
        "maintainability" => (
            "Maintainability",
            dimensions.maintainability,
            AuditCategoryName::Maintainability,
        ),
        "performance" => ("Performance", dimensions.performance, AuditCategoryName::Performance),
        "dependencies" => (
            "Dependencies",
            dimensions.dependencies,
            AuditCategoryName::Dependencies,
        ),
        _ => return None,
    })
}

fn driving_rule(
    presentation: &ReportPresentation,
    category: AuditCategoryName,
) -> Option<&DiagnosticGroup> {
    presentation
        .groups
        .iter()
        .filter(|group| group.contribution > 0 && group.category == Some(category))
        .min_by(|left, right| {
            right
                .contribution
                .cmp(&left.contribution)
                .then_with(|| left.rule_id.cmp(&right.rule_id))
        })
}

fn dimension_ceiling_line(
    dimensions: crate::ScoreDimensions,
    ceiling: &crate::ScoreCeiling,
    presentation: &ReportPresentation,
) -> Option<String> {
    let (name, value, dimension) = dimension_value(dimensions, ceiling.dimension)?;
    if value == ceiling.ceiling {
        return Some(capped_sentence(name, ceiling));
    }
    let driver = match driving_rule(presentation, dimension) {
        Some(group) => {
            let findings = group.count.weighing;
            let unit = if findings == 1 { "finding" } else { "findings" };
            format!("{} ({findings} {unit})", group.rule_id)
        }
        None => named_rules(ceiling),
    };
    Some(format!("{name} {value}: driven by {driver}"))
}

fn capped_sentence(subject: &str, ceiling: &crate::ScoreCeiling) -> String {
    format!(
        "{subject} is capped at {} by a {} finding: {}",
        ceiling.ceiling,
        ceiling.tier.as_str(),
        named_rules(ceiling)
    )
}

pub(super) const TERMS: &str = "P0 to P3 rank a rule's severity. Noise is the share of reviewed \
    sites where the corpus (pinned public repositories) found the rule wrong; unmeasured means no \
    site was reviewed.";

pub(super) const WORKSPACE_ONLY: &str =
    "Findings in CI files and the lockfile weigh on the workspace score only.";

fn workspace_below_every_member(report: &InspectReport, value: u8) -> bool {
    let packages = &report.audit.packages;
    !packages.is_empty()
        && packages.iter().all(|package| {
            package
                .score
                .as_ref()
                .is_some_and(|member| member.value > value)
        })
}

pub(super) fn render_score<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    presentation: &ReportPresentation,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let Some(score) = &report.audit.score else {
        return line(
            writer,
            crate::presentation::no_score_message(report),
            options,
            Style::Warning,
        );
    };
    if let Some(overall) = score
        .ceilings
        .iter()
        .find(|ceiling| ceiling.dimension == crate::audit::OVERALL)
        .filter(|overall| score.value == overall.ceiling)
    {
        line(writer, &capped_sentence("The score", overall), options, Style::Warning)?;
    }
    score_header::render(writer, score, options, score_header::Cadence::DEFAULT)?;
    if score.authoritative {
        render_dimensions(writer, report, score, presentation, options)?;
    }
    for reason in &score.reasons {
        line(writer, reason.explanation(), options, Style::Warning)?;
    }
    render_what_to_fix(writer, score, options)?;
    if presentation.groups.iter().any(|group| group.tier.is_some()) {
        line(writer, TERMS, options, Style::Muted)?;
    }
    Ok(())
}

fn render_dimensions<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    score: &crate::AuditScore,
    presentation: &ReportPresentation,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    line(
        writer,
        &crate::score_block::dimensions_line(score.dimensions),
        options,
        Style::Plain,
    )?;
    if let Some(unchecked) = report.unchecked_advisories() {
        line(
            writer,
            crate::score_block::unchecked_advisories_line(unchecked),
            options,
            Style::Muted,
        )?;
    }
    for ceiling in &score.ceilings {
        if let Some(sentence) = dimension_ceiling_line(score.dimensions, ceiling, presentation) {
            line(writer, &sentence, options, Style::Muted)?;
        }
    }
    if workspace_below_every_member(report, score.value) {
        line(writer, WORKSPACE_ONLY, options, Style::Muted)?;
    }
    Ok(())
}

fn render_what_to_fix<W: Write>(
    writer: &mut W,
    score: &crate::AuditScore,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    if let Some(projected) = score
        .projected_after_top_three
        .filter(|projected| *projected > score.value)
    {
        let named = named_with_measurement(&score.projected_rule_ids);
        let sentence = match score.projected_rule_ids.len() {
            1 => format!("Fix this rule to reach a projected {projected}/100: {named}"),
            count => format!("Fix the top {count} rules to reach a projected {projected}/100: {named}"),
        };
        line(writer, &sentence, options, Style::Accent)?;
    }
    if let Some(withheld) = withheld_sentence(&score.withheld_rule_ids) {
        line(writer, "", options, Style::Plain)?;
        line(writer, &withheld, options, Style::Plain)?;
    }
    Ok(())
}

pub(super) fn measurement_note(id: &str) -> String {
    match crate::policy::corpus_measurement(id) {
        Some(measurement) => {
            let percent = (u32::from(measurement.noise_basis_points()) + 50) / 100;
            let sites = measurement.reviewed();
            let unit = if sites == 1 { "site" } else { "sites" };
            format!(" ({percent}% noise on {sites} {unit})")
        }
        None => " (unmeasured)".to_owned(),
    }
}

fn named_with_measurement(ids: &[String]) -> String {
    ids.iter()
        .map(|id| format!("{id}{}", measurement_note(id)))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn withheld_sentence(withheld: &[String]) -> Option<String> {
    let named: Vec<String> = withheld
        .iter()
        .take(2)
        .map(|id| format!("{id}{}", measurement_note(id)))
        .collect();
    let (subject, found) = match (named.as_slice(), withheld.len()) {
        ([], _) => return None,
        ([only], _) => (only.clone(), "it"),
        ([first, second], 2) => (format!("{first} and {second}"), "each"),
        ([first, second], total) => (format!("{first}, {second} and {} more", total - 2), "each"),
        _ => return None,
    };
    Some(format!(
        "Left out of what to fix first: {subject}. The corpus found {found} wrong on most of \
         the sites it reviewed."
    ))
}
