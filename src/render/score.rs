use std::io::Write;

use crate::InspectReport;

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

fn dimension_ceiling_line(
    dimensions: crate::ScoreDimensions,
    ceiling: &crate::ScoreCeiling,
) -> Option<String> {
    let (name, value) = match ceiling.dimension {
        "security" => ("Security", dimensions.security),
        "reliability" => ("Reliability", dimensions.reliability),
        "maintainability" => ("Maintainability", dimensions.maintainability),
        "performance" => ("Performance", dimensions.performance),
        "dependencies" => ("Dependencies", dimensions.dependencies),
        _ => return None,
    };
    Some(format!(
        "{name} {value} ({} ceiling: {})",
        ceiling.tier.as_str(),
        named_rules(ceiling)
    ))
}

pub(super) fn ceiling_sentence(value: u8, ceiling: u8, tier: &str, blocking: &str) -> String {
    if value < ceiling {
        format!(
            "A {tier} finding caps the score at {ceiling}/100, and the dimensions already hold it lower: {blocking}"
        )
    } else {
        format!("Capped at {ceiling}/100 by a {tier} finding: {blocking}")
    }
}

pub(super) fn render_score<W: Write>(
    writer: &mut W,
    report: &InspectReport,
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
    {
        let sentence = ceiling_sentence(
            score.value,
            overall.ceiling,
            overall.tier.as_str(),
            &named_rules(overall),
        );
        line(writer, &sentence, options, Style::Warning)?;
    }
    score_header::render(writer, score, options, score_header::Cadence::DEFAULT)?;
    if score.authoritative {
        line(
            writer,
            &crate::score_block::dimensions_line(score.dimensions),
            options,
            Style::Plain,
        )?;
        for ceiling in &score.ceilings {
            if let Some(sentence) = dimension_ceiling_line(score.dimensions, ceiling) {
                line(writer, &sentence, options, Style::Muted)?;
            }
        }
    }
    for reason in &score.reasons {
        line(writer, reason.explanation(), options, Style::Warning)?;
    }
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
        ([only], _) => (format!("{only} reports here but is"), "it"),
        ([first, second], 2) => (format!("{first} and {second} report here but are"), "each"),
        ([first, second], total) => (
            format!("{first}, {second} and {} more report here but are", total - 2),
            "each",
        ),
        _ => return None,
    };
    Some(format!(
        "{subject} left out of what to fix first: the corpus found {found} wrong on most of \
         the sites it reviewed."
    ))
}
