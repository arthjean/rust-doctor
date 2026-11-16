use std::collections::BTreeMap;

use serde::Serialize;

use crate::Diagnostic;

const LISTED_CAUSES: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CauseCount {
    pub cause: String,
    pub findings: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RuleCauses {
    pub findings: usize,
    pub causes: usize,
    pub shared: Vec<CauseCount>,
}

impl RuleCauses {
    pub fn of<'a>(diagnostics: impl IntoIterator<Item = &'a Diagnostic>) -> Self {
        let mut findings = 0usize;
        let mut singles = 0usize;
        let mut declared = BTreeMap::<&str, usize>::new();
        for diagnostic in diagnostics {
            findings += 1;
            match diagnostic.cause.as_deref() {
                Some(cause) => *declared.entry(cause).or_insert(0) += 1,
                None => singles += 1,
            }
        }
        let causes = declared.len() + singles;
        let mut shared: Vec<CauseCount> = declared
            .into_iter()
            .filter(|(_, count)| *count > 1)
            .map(|(cause, count)| CauseCount {
                cause: cause.to_owned(),
                findings: count,
            })
            .collect();
        shared.sort_by(|left, right| {
            right
                .findings
                .cmp(&left.findings)
                .then_with(|| left.cause.cmp(&right.cause))
        });
        Self {
            findings,
            causes,
            shared,
        }
    }

    pub fn line(&self) -> Option<String> {
        if self.shared.is_empty() {
            return None;
        }
        let listed: Vec<String> = self
            .shared
            .iter()
            .take(LISTED_CAUSES)
            .map(|cause| format!("{} x{}", cause.cause, cause.findings))
            .collect();
        let shown = listed.len();
        let more = if self.causes > shown { ", ..." } else { "" };
        Some(format!(
            "{} finding{}, {} cause{}: {}{more}",
            self.findings,
            if self.findings == 1 { "" } else { "s" },
            self.causes,
            if self.causes == 1 { "" } else { "s" },
            listed.join(", ")
        ))
    }

    pub fn dominant(&self) -> Option<&CauseCount> {
        self.shared
            .first()
            .filter(|cause| cause.findings.saturating_mul(2) > self.findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiagnosticSource, Severity};

    fn finding(cause: Option<&str>) -> Diagnostic {
        Diagnostic {
            id: String::new(),
            source: DiagnosticSource::Clippy,
            code: Some("clippy::large_types_passed_by_value".to_owned()),
            base_severity: Severity::Warning,
            severity: Severity::Warning,
            category: Some("performance".to_owned()),
            message: String::new(),
            help: None,
            package: None,
            target: None,
            context: None,
            unscored: None,
            path: None,
            span: None,
            related: Vec::new(),
            similarity_basis_points: None,
            cause: cause.map(str::to_owned),
            complexity: None,
            suggestion: None,
            occurrences: 1,
        }
    }

    fn findings(causes: &[(Option<&str>, usize)]) -> Vec<Diagnostic> {
        causes
            .iter()
            .flat_map(|(cause, count)| std::iter::repeat_n(finding(*cause), *count))
            .collect()
    }

    #[test]
    fn the_causes_of_a_rule_are_listed_largest_first_at_most_three() {
        let all = findings(&[
            (Some("RowPalette (264 byte)"), 3),
            (Some("UiColors (292 byte)"), 303),
            (Some("Lone (300 byte)"), 1),
            (Some("Pair (280 byte)"), 2),
            (Some("Other (270 byte)"), 2),
            (None, 2),
        ]);
        let causes = RuleCauses::of(&all);
        assert_eq!(causes.findings, 313);
        assert_eq!(causes.causes, 7);
        assert_eq!(
            causes.line().as_deref(),
            Some(
                "313 findings, 7 causes: UiColors (292 byte) x303, RowPalette (264 byte) x3, Other (270 byte) x2, ..."
            )
        );
        assert_eq!(
            causes.dominant().map(|cause| cause.cause.as_str()),
            Some("UiColors (292 byte)")
        );
    }

    #[test]
    fn a_rule_whose_causes_are_all_single_lists_none() {
        let causes = RuleCauses::of(&findings(&[(Some("A (300 byte)"), 1), (None, 4)]));
        assert_eq!(causes.causes, 5);
        assert_eq!(causes.line(), None);
        assert_eq!(causes.dominant(), None);
        let one = RuleCauses::of(&findings(&[(Some("API_KEY"), 3)]));
        assert_eq!(one.line().as_deref(), Some("3 findings, 1 cause: API_KEY x3"));
    }

    #[test]
    fn a_cause_names_the_spread_only_when_it_holds_more_than_half() {
        let half = RuleCauses::of(&findings(&[(Some("A (300 byte)"), 2), (None, 2)]));
        assert_eq!(half.dominant(), None);
        let most = RuleCauses::of(&findings(&[(Some("A (300 byte)"), 3), (None, 2)]));
        assert_eq!(most.dominant().map(|cause| cause.findings), Some(3));

        let advisory = |cause: Option<CauseCount>| super::super::MigrationAdvisory {
            rule_id: "clippy::large_types_passed_by_value".to_owned(),
            occurrences: 310,
            files: 52,
            cause,
        };
        assert_eq!(
            advisory(most.dominant().cloned().map(|cause| CauseCount { findings: 303, ..cause }))
                .sentence("clippy::large_types_passed_by_value"),
            "Spread across 52 files: clippy::large_types_passed_by_value (310 findings, 303 of them from A (300 byte)). Plan it as one change, not 310 fixes."
        );
        assert_eq!(
            advisory(None).sentence("clippy::large_types_passed_by_value"),
            "Spread across 52 files: clippy::large_types_passed_by_value (310 findings). Plan it as one change, not 310 fixes."
        );
    }
}
