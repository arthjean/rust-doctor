use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use crate::presentation::context_label;
use crate::{AuditCategoryName, Diagnostic, DiagnosticContext, InspectReport};

use super::{RenderError, Style, TerminalOptions, line};

#[derive(Default)]
struct Tally {
    production: crate::SeverityCounts,
    outside: [usize; 4],
    not_scored: usize,
    by_rule: BTreeMap<String, usize>,
}

impl Tally {
    fn add(&mut self, diagnostic: &Diagnostic) {
        let occurrences = diagnostic.occurrences;
        if let Some(context) = diagnostic.context {
            if let Some(slot) = CONTEXTS
                .iter()
                .position(|known| *known == context)
                .and_then(|index| self.outside.get_mut(index))
            {
                *slot = slot.saturating_add(occurrences);
            }
        } else if diagnostic.unscored.is_some() {
            self.not_scored = self.not_scored.saturating_add(occurrences);
        } else {
            self.production.add(diagnostic.severity, occurrences);
            if let Some(code) = diagnostic.code.as_deref() {
                let slot = self.by_rule.entry(code.to_owned()).or_default();
                *slot = slot.saturating_add(occurrences);
            }
        }
    }

    fn total(&self) -> usize {
        let counts = self.production;
        counts
            .errors
            .saturating_add(counts.warnings)
            .saturating_add(counts.info)
            .saturating_add(counts.unknown)
    }

    fn counts(&self) -> String {
        let counts = self.production;
        let parts: Vec<String> = [
            (counts.errors, "error", "errors"),
            (counts.warnings, "warning", "warnings"),
            (counts.info, "info", "info"),
            (counts.unknown, "unknown", "unknown"),
        ]
        .into_iter()
        .filter(|(count, _, _)| *count > 0)
        .map(|(count, one, many)| format!("{count} {}", if count == 1 { one } else { many }))
        .collect();
        if parts.is_empty() {
            "0 findings".to_owned()
        } else {
            parts.join(", ")
        }
    }

    fn largest(&self) -> Option<String> {
        let total = self.total();
        self.by_rule
            .iter()
            .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(left.0)))
            .filter(|(_, count)| count.saturating_mul(2) > total)
            .map(|(rule, count)| format!("{count} from {}", crate::presentation::rule_title(rule)))
    }

    fn apart(&self) -> Option<String> {
        let parts: Vec<String> = CONTEXTS
            .iter()
            .zip(self.outside)
            .map(|(context, count)| (count, context_label(*context)))
            .chain(std::iter::once((self.not_scored, "not scored")))
            .filter(|(count, _)| *count > 0)
            .map(|(count, kind)| format!("+{count} {kind}"))
            .collect();
        (!parts.is_empty()).then(|| parts.join(", "))
    }

    fn notes(&self) -> String {
        match (self.largest(), self.apart()) {
            (None, None) => String::new(),
            (Some(note), None) | (None, Some(note)) => format!(" ({note})"),
            (Some(largest), Some(apart)) => format!(" ({largest}; {apart})"),
        }
    }
}

const CONTEXTS: [DiagnosticContext; 4] = [
    DiagnosticContext::Tests,
    DiagnosticContext::Benchmark,
    DiagnosticContext::Example,
    DiagnosticContext::BuildScript,
];

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

pub(super) fn render_categories<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let mut tallies = BTreeMap::<AuditCategoryName, Tally>::new();
    for diagnostic in scoped(report) {
        tallies
            .entry(bucket(diagnostic.category.as_deref()))
            .or_default()
            .add(diagnostic);
    }
    let published: BTreeSet<AuditCategoryName> =
        report.audit.categories.iter().map(|category| category.name).collect();
    for name in AuditCategoryName::IN_DIMENSION_ORDER {
        if !published.contains(&name) {
            continue;
        }
        let tally = tallies.remove(&name).unwrap_or_default();
        line(
            writer,
            &format!("{}: {}{}", name.label(), tally.counts(), tally.notes()),
            options,
            Style::Plain,
        )?;
    }
    if published.is_empty() {
        line(writer, "Categories: none", options, Style::Plain)?;
    }
    Ok(())
}
