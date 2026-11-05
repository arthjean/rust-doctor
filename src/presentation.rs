use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::Serialize;

use crate::audit::{AuditCategoryName, aggregate_rules};
use crate::workspace_path;
use crate::{
    Diagnostic, DiagnosticContext, DiagnosticSpan, InspectReport, Severity, Suggestion,
    UnscoredReason,
};

mod code_frame;

pub use code_frame::{
    CodeFrame, CodeFrameLine, CodeFrameMarker, CodeFrameUnavailable, CodeFrameUnavailableReason,
    code_frame,
};

pub fn canonical_rule_help(rule_id: &str) -> Option<&'static str> {
    crate::policy::find(rule_id).map(|definition| definition.help)
}

pub fn canonical_not_a_defect(rule_id: &str) -> Option<&'static str> {
    crate::policy::find(rule_id).map(|definition| definition.not_a_defect_when)
}

const RULE_BASE_URL: &str = "https://rust-doctor.com/rules/";

pub fn rule_url(rule_id: &str) -> String {
    format!("{RULE_BASE_URL}{}", percent_encode_path_segment(rule_id))
}

pub const COMPILER_NOTES_HEADING: &str = "Compiler notes (not scored)";
const MIGRATION_FILE_THRESHOLD: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportPresentation {
    pub groups: Vec<DiagnosticGroup>,
    pub migration_advisories: Vec<MigrationAdvisory>,
    pub issue_count: usize,
    pub finding_count: usize,
    pub note_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticGroup {
    pub rule_id: String,
    pub title: String,
    pub rule_url: String,
    pub severity: Severity,
    pub category: Option<AuditCategoryName>,
    pub occurrences: usize,
    pub diagnostics: Vec<GroupDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupDiagnostic {
    pub message: String,
    pub help: Option<String>,
    pub suggestion: Option<Suggestion>,
    pub base_severity: Severity,
    pub severity: Severity,
    pub context: Option<DiagnosticContext>,
    pub unscored: Option<UnscoredReason>,
    pub path: Option<String>,
    pub span: Option<DiagnosticSpan>,
    pub related: Vec<GroupLocation>,
    pub occurrences: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupLocation {
    pub path: String,
    pub span: DiagnosticSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MigrationAdvisory {
    pub rule_id: String,
    pub occurrences: usize,
    pub files: usize,
}

impl DiagnosticGroup {
    pub fn is_unscored(&self) -> bool {
        !self.diagnostics.is_empty()
            && self
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.context.is_some())
    }

    pub fn is_compiler_note(&self) -> bool {
        self.diagnostics
            .first()
            .is_some_and(|diagnostic| diagnostic.unscored.is_some())
    }

    pub fn representative(&self) -> Option<&GroupDiagnostic> {
        self.diagnostics
            .iter()
            .find(|diagnostic| diagnostic.location().is_some())
            .or_else(|| self.diagnostics.first())
    }

    pub fn resolved_help(&self) -> Option<&str> {
        self.representative()
            .and_then(|diagnostic| diagnostic.help.as_deref())
            .or_else(|| canonical_rule_help(&self.rule_id))
    }
}

impl GroupDiagnostic {
    pub fn site_note(&self) -> Option<String> {
        self.context
            .map(|context| format!("{}, not scored", context.label()))
    }

    pub fn location(&self) -> Option<GroupLocation> {
        self.path
            .as_ref()
            .zip(self.span.as_ref())
            .map(|(path, span)| GroupLocation {
                path: path.clone(),
                span: span.clone(),
            })
    }
}

pub fn incomplete_message(report: &InspectReport) -> Option<String> {
    if report.status == crate::Status::Complete {
        return None;
    }
    let mut stages: Vec<&str> = report
        .errors
        .iter()
        .filter(|error| crate::audit::ScoreReason::of_error(&error.code).is_some())
        .map(|error| error.stage.as_str())
        .collect();
    stages.sort_unstable();
    stages.dedup();
    let named = match stages.as_slice() {
        [] => None,
        [stage] => Some(format!("{stage} checks")),
        [rest @ .., last] => Some(format!("{} and {last} checks", rest.join(", "))),
    };
    Some(match (report.diagnostics.is_empty(), named) {
        (true, named) => format!(
            "No issues detected, but {} did not complete.",
            named.as_deref().unwrap_or("the scan")
        ),
        (false, Some(named)) => format!("{named} did not complete: results are incomplete."),
        (false, None) => "The scan did not complete: results are incomplete.".to_owned(),
    })
}

pub fn no_score_message(report: &InspectReport) -> &'static str {
    let packages = &report.audit.packages;
    if !packages.is_empty() && packages.iter().all(|package| package.unlinted) {
        "Score unavailable: the scan could not score, since no package was linted."
    } else if report.audit.source_files > 0 && report.audit.production_lines == 0 {
        "Score unavailable: only tests, benches, examples or build scripts were found."
    } else {
        "Score unavailable: no Rust files were analyzed."
    }
}

impl ReportPresentation {
    pub fn derive(report: &InspectReport) -> Self {
        Self::from_diagnostics(report.audit.production_lines, report.audit.packages.len(), &report.diagnostics)
    }

    pub fn derive_terminal(report: &InspectReport) -> Self {
        let Some(delta) = &report.delta else {
            return Self::derive(report);
        };
        let introduced: BTreeSet<_> = delta.introduced.iter().map(String::as_str).collect();
        Self::from_diagnostics(
            report.audit.production_lines,
            report.audit.packages.len(),
            report
                .diagnostics
                .iter()
                .filter(|diagnostic| introduced.contains(diagnostic.id.as_str())),
        )
    }

    fn from_diagnostics<'a>(
        production_lines: usize,
        members: usize,
        diagnostics: impl IntoIterator<Item = &'a Diagnostic>,
    ) -> Self {
        let diagnostics: Vec<_> = diagnostics.into_iter().collect();
        let issue_count = diagnostics.iter().fold(0usize, |total, diagnostic| {
            total.saturating_add(diagnostic.occurrences)
        });
        let finding_count = diagnostics.len();
        let note_count = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.unscored.is_some())
            .count();
        let groups = diagnostic_groups(production_lines, members, &diagnostics);
        let migration_advisories = migration_advisories(&groups);
        Self {
            groups,
            migration_advisories,
            issue_count,
            finding_count,
            note_count,
        }
    }
}

fn diagnostic_groups(
    production_lines: usize,
    members: usize,
    diagnostics: &[&Diagnostic],
) -> Vec<DiagnosticGroup> {
    let aggregation = aggregate_rules(production_lines, members, diagnostics.iter().copied());
    let projected: BTreeMap<String, usize> = aggregation
        .projection()
        .0
        .into_iter()
        .enumerate()
        .map(|(position, rule)| (rule.id.clone(), position))
        .collect();
    let mut aggregates: BTreeMap<_, _> = aggregation
        .rules
        .into_iter()
        .map(|aggregate| (aggregate.id.clone(), aggregate))
        .collect();
    let mut grouped = BTreeMap::<String, Vec<&Diagnostic>>::new();
    for diagnostic in diagnostics {
        let Some(rule_id) = diagnostic.code.as_deref().filter(|code| !code.is_empty()) else {
            continue;
        };
        grouped
            .entry(rule_id.to_owned())
            .or_default()
            .push(diagnostic);
    }

    let mut ranked = Vec::with_capacity(grouped.len());
    for (rule_id, diagnostics) in grouped {
        let Some(aggregate) = aggregates.remove(&rule_id) else {
            continue;
        };
        let severity = aggregate.effective_severity;
        let category = aggregate.category;
        let repair_value = aggregate.expected_repair_value();
        let contribution = aggregate.contribution();
        let occurrences = aggregate.occurrences;
        let projected_position = projected.get(&rule_id).copied().unwrap_or(usize::MAX);
        let is_note = diagnostics
            .first()
            .is_some_and(|diagnostic| diagnostic.unscored.is_some());
        let diagnostics = diagnostics
            .into_iter()
            .map(|diagnostic| GroupDiagnostic {
                message: diagnostic.message.clone(),
                help: diagnostic.help.clone(),
                suggestion: diagnostic.suggestion.clone(),
                base_severity: diagnostic.base_severity,
                severity: diagnostic.severity,
                context: diagnostic.context,
                unscored: diagnostic.unscored,
                path: diagnostic
                    .path
                    .as_deref()
                    .filter(|path| workspace_path::decode_normalized_relative(path).is_some())
                    .map(str::to_owned),
                span: diagnostic.span.clone(),
                related: diagnostic
                    .related
                    .iter()
                    .filter(|location| {
                        workspace_path::decode_normalized_relative(&location.path).is_some()
                    })
                    .map(|location| GroupLocation {
                        path: location.path.clone(),
                        span: location.span.clone(),
                    })
                    .collect(),
                occurrences: diagnostic.occurrences,
            })
            .collect();
        ranked.push((
            (
                is_note,
                severity.rank(),
                projected_position,
                std::cmp::Reverse(repair_value),
                std::cmp::Reverse(contribution),
                std::cmp::Reverse(occurrences),
            ),
            DiagnosticGroup {
                title: rule_title(&rule_id),
                rule_url: rule_url(&rule_id),
                rule_id,
                severity,
                category,
                occurrences,
                diagnostics,
            },
        ));
    }
    ranked.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.rule_id.cmp(&right.1.rule_id))
    });
    ranked.into_iter().map(|(_, group)| group).collect()
}

fn migration_advisories(groups: &[DiagnosticGroup]) -> Vec<MigrationAdvisory> {
    groups
        .iter()
        .filter_map(|group| {
            let files: BTreeSet<_> = group
                .diagnostics
                .iter()
                .filter_map(|diagnostic| diagnostic.path.as_deref())
                .collect();
            (files.len() >= MIGRATION_FILE_THRESHOLD).then(|| MigrationAdvisory {
                rule_id: group.rule_id.clone(),
                occurrences: group.occurrences,
                files: files.len(),
            })
        })
        .collect()
}

fn rule_title(rule_id: &str) -> String {
    let leaf = rule_id.rsplit("::").next().unwrap_or(rule_id);
    let words = leaf.replace(['_', '-'], " ");
    let mut characters = words.chars();
    let Some(first) = characters.next() else {
        return rule_id.to_owned();
    };
    first.to_uppercase().chain(characters).collect()
}

fn percent_encode_path_segment(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(char::from(byte));
        } else {
            let _ = write!(output, "%{byte:02X}");
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::Audit;
    use crate::{
        BlockingLevel, DeltaReport, DeltaSummary, GateReport, GateStatus, ScanReport, Status,
        Summary, ToolchainReport,
    };

    fn diagnostic(
        rule: &str,
        severity: Severity,
        category: &str,
        path: Option<String>,
        occurrences: usize,
    ) -> Diagnostic {
        Diagnostic {
            context: None,
            id: format!("{rule}-{occurrences}"),
            source: crate::DiagnosticSource::Clippy,
            code: Some(rule.to_owned()),
            base_severity: severity,
            severity,
            category: Some(category.to_owned()),
            message: format!("message for {rule}"),
            help: None,
            package: None,
            target: None,
            path,
            span: Some(DiagnosticSpan {
                line_start: 3,
                column_start: 5,
                line_end: 3,
                column_end: 8,
            }),
            related: Vec::new(),
            similarity_basis_points: None,
            unscored: None,
            complexity: None,
            suggestion: None,
            occurrences,
        }
    }

    fn report(diagnostics: Vec<Diagnostic>) -> InspectReport {
        let audit = Audit::build(50, 5_000, Status::Complete, &diagnostics);
        InspectReport {
            schema_version: 8,
            audit,
            status: Status::Complete,
            complete: true,
            policy: None,
            scope: None,
            project: None,
            toolchain: ToolchainReport {
                rust_doctor: env!("CARGO_PKG_VERSION"),
                rustc: None,
                cargo: None,
                clippy: None,
                removed_lint_flags: Vec::new(),
            },
            scan: ScanReport {
                command: None,
                exit_code: Some(0),
                build_finished: Some(true),
                noise_lines: Some(0),
                ignored: 0,
                excluded_generated: 0,
                advisory_db: None,
            },
            diagnostics,
            suppressions: Vec::new(),
            delta: None,
            errors: Vec::new(),
            summary: Summary::default(),
            gate: GateReport {
                blocking: BlockingLevel::Error,
                status: GateStatus::Passed,
                blocking_diagnostics: Some(0),
            },
            timings: None,
            base_side_cache: None,
        }
    }

    #[test]
    fn groups_use_rule_identity_and_normative_priority() {
        let presentation = ReportPresentation::derive(&report(vec![
            diagnostic(
                "clippy::low_error",
                Severity::Error,
                "maintainability",
                Some("src/a.rs".to_owned()),
                1,
            ),
            diagnostic(
                "clippy::security_error",
                Severity::Error,
                "security",
                Some("src/b.rs".to_owned()),
                1,
            ),
            diagnostic(
                "clippy::many_warnings",
                Severity::Warning,
                "security",
                Some("src/c.rs".to_owned()),
                20,
            ),
            diagnostic(
                "clippy::same_warning",
                Severity::Warning,
                "security",
                Some("src/d.rs".to_owned()),
                2,
            ),
            diagnostic(
                "clippy::same_warning",
                Severity::Error,
                "security",
                Some("src/e.rs".to_owned()),
                3,
            ),
        ]));
        let ids: Vec<_> = presentation
            .groups
            .iter()
            .map(|group| group.rule_id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "clippy::same_warning",
                "clippy::security_error",
                "clippy::low_error",
                "clippy::many_warnings"
            ]
        );
        let same_warning = presentation
            .groups
            .iter()
            .find(|group| group.rule_id == "clippy::same_warning")
            .expect("the two same_warning findings should be one group");
        assert_eq!(same_warning.occurrences, 5);
        assert_eq!(same_warning.severity, Severity::Error);
        assert_eq!(same_warning.title, "Same warning");
        assert_eq!(
            same_warning.rule_url,
            "https://rust-doctor.com/rules/clippy%3A%3Asame_warning"
        );
    }

    #[test]
    fn a_rule_the_corpus_measured_wrong_is_not_ranked_first() {
        let presentation = ReportPresentation::derive(&report(vec![
            diagnostic(
                "clippy::exit",
                Severity::Error,
                "correctness",
                Some("src/a.rs".to_owned()),
                40,
            ),
            diagnostic(
                "clippy::todo",
                Severity::Error,
                "correctness",
                Some("src/b.rs".to_owned()),
                1,
            ),
        ]));

        assert_eq!(
            presentation
                .groups
                .iter()
                .map(|group| group.rule_id.as_str())
                .collect::<Vec<_>>(),
            ["clippy::todo", "clippy::exit"],
            "the quieter rule leads even though the noisy one fired forty times"
        );
    }

    #[test]
    fn a_finding_outside_production_code_stays_listed_and_ranks_last() {
        let mut in_tests = diagnostic(
            "clippy::dbg_macro",
            Severity::Error,
            "correctness",
            Some("src/a.rs".to_owned()),
            40,
        );
        in_tests.context = Some(crate::report::DiagnosticContext::Tests);
        let presentation = ReportPresentation::derive(&report(vec![
            in_tests,
            diagnostic(
                "clippy::todo",
                Severity::Error,
                "correctness",
                Some("src/b.rs".to_owned()),
                1,
            ),
        ]));

        assert_eq!(
            presentation
                .groups
                .iter()
                .map(|group| (group.rule_id.as_str(), group.occurrences))
                .collect::<Vec<_>>(),
            [("clippy::todo", 1), ("clippy::dbg_macro", 40)]
        );
    }

    #[test]
    fn internally_incoherent_rule_has_no_score_contribution() {
        let presentation = ReportPresentation::derive(&report(vec![
            diagnostic(
                "clippy::mixed",
                Severity::Error,
                "security",
                Some("src/a.rs".to_owned()),
                20,
            ),
            diagnostic(
                "clippy::mixed",
                Severity::Error,
                "maintainability",
                Some("src/b.rs".to_owned()),
                20,
            ),
            diagnostic(
                "clippy::valid",
                Severity::Error,
                "maintainability",
                Some("src/c.rs".to_owned()),
                1,
            ),
        ]));

        assert_eq!(presentation.groups[0].rule_id, "clippy::valid");
        assert_eq!(presentation.groups[1].rule_id, "clippy::mixed");
        assert_eq!(presentation.groups[1].category, None);
    }

    #[test]
    fn display_severity_does_not_hide_a_higher_unscorable_occurrence() {
        let presentation = ReportPresentation::derive(&report(vec![
            diagnostic(
                "clippy::mixed",
                Severity::Warning,
                "maintainability",
                Some("src/a.rs".to_owned()),
                1,
            ),
            diagnostic(
                "clippy::mixed",
                Severity::Error,
                "future-category",
                Some("src/b.rs".to_owned()),
                1,
            ),
        ]));

        assert_eq!(presentation.groups[0].severity, Severity::Error);
        assert_eq!(
            presentation.groups[0].category,
            Some(AuditCategoryName::Maintainability)
        );
    }

    #[test]
    fn terminal_presentation_is_the_canonical_baseline_view() {
        let introduced = diagnostic(
            "clippy::introduced",
            Severity::Warning,
            "maintainability",
            Some("src/new.rs".to_owned()),
            2,
        );
        let pre_existing = diagnostic(
            "clippy::pre_existing",
            Severity::Error,
            "security",
            Some("src/old.rs".to_owned()),
            5,
        );
        let mut report = report(vec![introduced.clone(), pre_existing]);
        report.delta = Some(DeltaReport {
            fingerprint_version: 1,
            base_diagnostics: 1,
            current_diagnostics: 2,
            introduced: vec![introduced.id],
            pre_existing: Vec::new(),
            fixed: Vec::new(),
            summary: DeltaSummary {
                introduced: 2,
                pre_existing: 5,
                fixed: 0,
                cross_file_matches: 0,
                rule_count_matches: 0,
            },
        });

        let terminal = ReportPresentation::derive_terminal(&report);
        assert_eq!(terminal.issue_count, 2);
        assert_eq!(terminal.groups.len(), 1);
        assert_eq!(terminal.groups[0].rule_id, "clippy::introduced");
        assert_eq!(ReportPresentation::derive(&report).groups.len(), 2);
    }

    #[test]
    fn migration_advisory_requires_forty_distinct_files() {
        let mut diagnostics: Vec<_> = (0..40)
            .map(|index| {
                diagnostic(
                    "clippy::sweep",
                    Severity::Warning,
                    "maintainability",
                    Some(format!("src/{index}.rs")),
                    2,
                )
            })
            .collect();
        for diagnostic in &mut diagnostics {
            diagnostic.span = None;
        }
        let at_threshold = ReportPresentation::derive(&report(diagnostics.clone()));
        assert_eq!(
            at_threshold.migration_advisories,
            [MigrationAdvisory {
                rule_id: "clippy::sweep".to_owned(),
                occurrences: 80,
                files: 40,
            }]
        );
        let below = ReportPresentation::derive(&report(diagnostics.into_iter().take(39).collect()));
        assert!(below.migration_advisories.is_empty());
    }

    #[test]
    fn empty_report_does_not_attempt_filesystem_access() {
        let presentation = ReportPresentation::derive(&report(Vec::new()));
        assert!(presentation.groups.is_empty());
        assert!(presentation.migration_advisories.is_empty());
    }
}
