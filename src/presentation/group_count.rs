use std::fmt;

use serde::Serialize;

use crate::{DiagnosticContext, Severity};

use super::GroupDiagnostic;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct GroupCount {
    pub weighing: usize,
    pub info: usize,
    pub unknown: usize,
    pub in_tests: usize,
    pub in_benches: usize,
    pub in_examples: usize,
    pub in_build_scripts: usize,
}

pub const fn context_label(context: DiagnosticContext) -> &'static str {
    match context {
        DiagnosticContext::Tests => "in tests",
        DiagnosticContext::Benchmark => "in benches",
        DiagnosticContext::Example => "in examples",
        DiagnosticContext::BuildScript => "in build scripts",
    }
}

impl GroupCount {
    pub fn of<'a>(diagnostics: impl IntoIterator<Item = &'a GroupDiagnostic>) -> Self {
        let mut count = Self::default();
        for diagnostic in diagnostics {
            let slot = match (diagnostic.context, diagnostic.severity) {
                (Some(DiagnosticContext::Tests), _) => &mut count.in_tests,
                (Some(DiagnosticContext::Benchmark), _) => &mut count.in_benches,
                (Some(DiagnosticContext::Example), _) => &mut count.in_examples,
                (Some(DiagnosticContext::BuildScript), _) => &mut count.in_build_scripts,
                (None, Severity::Error | Severity::Warning) => &mut count.weighing,
                (None, Severity::Info) => &mut count.info,
                (None, Severity::Unknown) => &mut count.unknown,
            };
            *slot = slot.saturating_add(diagnostic.occurrences);
        }
        count
    }

    pub const fn weighs(&self) -> bool {
        self.weighing > 0
    }

    pub fn suffix(&self) -> String {
        let parts: Vec<String> = [
            (self.info, "info"),
            (self.unknown, "unknown"),
            (self.in_tests, context_label(DiagnosticContext::Tests)),
            (self.in_benches, context_label(DiagnosticContext::Benchmark)),
            (self.in_examples, context_label(DiagnosticContext::Example)),
            (self.in_build_scripts, context_label(DiagnosticContext::BuildScript)),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, kind)| format!("+{count} {kind}"))
        .collect();
        if parts.is_empty() {
            String::new()
        } else {
            format!(" ({})", parts.join(", "))
        }
    }
}

impl fmt::Display for GroupCount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "x{}{}", self.weighing, self.suffix())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(severity: Severity, context: Option<DiagnosticContext>, occurrences: usize) -> GroupDiagnostic {
        GroupDiagnostic {
            message: String::new(),
            help: None,
            suggestion: None,
            base_severity: severity,
            severity,
            context,
            unscored: None,
            path: None,
            span: None,
            related: Vec::new(),
            occurrences,
        }
    }

    #[test]
    fn every_count_beside_the_weighing_one_names_its_kind() {
        let sites = [
            site(Severity::Warning, None, 3),
            site(Severity::Error, None, 1),
            site(Severity::Info, None, 51),
            site(Severity::Warning, Some(DiagnosticContext::Tests), 68),
        ];
        assert_eq!(GroupCount::of(&sites).to_string(), "x4 (+51 info, +68 in tests)");
        assert_eq!(GroupCount::of(&sites[..2]).to_string(), "x4");
        assert_eq!(GroupCount::of(&sites[2..3]).to_string(), "x0 (+51 info)");
        assert_eq!(GroupCount::of(&sites[3..]).to_string(), "x0 (+68 in tests)");
    }

    #[test]
    fn a_site_outside_production_code_is_counted_by_its_context_whatever_its_severity() {
        let sites = [
            site(Severity::Info, Some(DiagnosticContext::BuildScript), 2),
            site(Severity::Error, Some(DiagnosticContext::Example), 1),
            site(Severity::Unknown, None, 1),
        ];
        let count = GroupCount::of(&sites);
        assert!(!count.weighs());
        assert_eq!(
            count.to_string(),
            "x0 (+1 unknown, +1 in examples, +2 in build scripts)"
        );
    }
}
