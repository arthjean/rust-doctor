use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::Serialize;

use crate::audit::{AuditCategoryName, aggregate_rules};
use crate::workspace_path;
use crate::{Diagnostic, DiagnosticSpan, InspectReport, Severity};

mod code_frame;

pub use code_frame::{
    CodeFrame, CodeFrameLine, CodeFrameMarker, CodeFrameUnavailable, CodeFrameUnavailableReason,
    code_frame,
};

pub fn canonical_rule_help(rule_id: &str) -> Option<&'static str> {
    crate::policy::find(rule_id).map(|definition| definition.help)
}

const RULE_BASE_URL: &str = "https://rust-doctor.vercel.app/rules/";
const MIGRATION_FILE_THRESHOLD: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportPresentation {
    pub groups: Vec<DiagnosticGroup>,
    pub migration_advisories: Vec<MigrationAdvisory>,
    /// Nombre d'occurrences, la grandeur publiée par `summary.occurrences.total`.
    pub issue_count: usize,
    /// Nombre de diagnostics distincts, la grandeur publiée par `summary.total`.
    pub finding_count: usize,
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
    pub base_severity: Severity,
    pub severity: Severity,
    pub path: Option<String>,
    pub span: Option<DiagnosticSpan>,
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

impl GroupDiagnostic {
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

impl ReportPresentation {
    pub fn derive(report: &InspectReport) -> Self {
        Self::from_diagnostics(&report.diagnostics)
    }

    pub fn derive_terminal(report: &InspectReport) -> Self {
        let Some(delta) = &report.delta else {
            return Self::derive(report);
        };
        let introduced: BTreeSet<_> = delta.introduced.iter().map(String::as_str).collect();
        Self::from_diagnostics(
            report
                .diagnostics
                .iter()
                .filter(|diagnostic| introduced.contains(diagnostic.id.as_str())),
        )
    }

    fn from_diagnostics<'a>(diagnostics: impl IntoIterator<Item = &'a Diagnostic>) -> Self {
        let diagnostics: Vec<_> = diagnostics.into_iter().collect();
        let issue_count = diagnostics.iter().fold(0usize, |total, diagnostic| {
            total.saturating_add(diagnostic.occurrences)
        });
        let finding_count = diagnostics.len();
        let groups = diagnostic_groups(&diagnostics);
        let migration_advisories = migration_advisories(&groups);
        Self {
            groups,
            migration_advisories,
            issue_count,
            finding_count,
        }
    }
}

fn diagnostic_groups(diagnostics: &[&Diagnostic]) -> Vec<DiagnosticGroup> {
    let mut aggregates: BTreeMap<_, _> = aggregate_rules(diagnostics.iter().copied())
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
        let contribution = aggregate.contribution();
        let occurrences = diagnostics.iter().fold(0usize, |total, diagnostic| {
            total.saturating_add(diagnostic.occurrences)
        });
        let diagnostics = diagnostics
            .into_iter()
            .map(|diagnostic| GroupDiagnostic {
                message: diagnostic.message.clone(),
                help: diagnostic.help.clone(),
                base_severity: diagnostic.base_severity,
                severity: diagnostic.severity,
                path: diagnostic
                    .path
                    .as_deref()
                    .filter(|path| workspace_path::decode_normalized_relative(path).is_some())
                    .map(str::to_owned),
                span: diagnostic.span.clone(),
                occurrences: diagnostic.occurrences,
            })
            .collect();
        ranked.push((
            severity.rank(),
            std::cmp::Reverse(contribution),
            std::cmp::Reverse(occurrences),
            DiagnosticGroup {
                title: rule_title(&rule_id),
                rule_url: format!("{RULE_BASE_URL}{}", percent_encode_path_segment(&rule_id)),
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
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.3.rule_id.cmp(&right.3.rule_id))
    });
    ranked.into_iter().map(|(_, _, _, group)| group).collect()
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
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::code_frame::{FRAME_MAX_COLUMNS, FRAME_MAX_LINES, read_code_frame};
    use super::*;
    use crate::audit::Audit;
    use crate::{
        BlockingLevel, DeltaReport, DeltaSummary, GateReport, GateStatus, ScanReport, Status,
        Summary, ToolchainReport,
    };

    static TEMPORARY: AtomicUsize = AtomicUsize::new(0);

    fn temporary_root(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/presentation-tests")
            .join(format!(
                "{}-{name}-{}",
                std::process::id(),
                TEMPORARY.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn diagnostic(
        rule: &str,
        severity: Severity,
        category: &str,
        path: Option<String>,
        occurrences: usize,
    ) -> Diagnostic {
        Diagnostic {
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
            occurrences,
        }
    }

    fn report(diagnostics: Vec<Diagnostic>) -> InspectReport {
        let audit = Audit::build(1, Status::Complete, &diagnostics);
        InspectReport {
            schema_version: 8,
            audit,
            status: Status::Complete,
            complete: true,
            policy: None,
            scope: None,
            project: None,
            toolchain: ToolchainReport {
                rustc: None,
                cargo: None,
                clippy: None,
            },
            scan: ScanReport {
                command: None,
                exit_code: Some(0),
                build_finished: Some(true),
                noise_lines: Some(0),
            },
            diagnostics,
            delta: None,
            errors: Vec::new(),
            summary: Summary::default(),
            gate: GateReport {
                blocking: BlockingLevel::Error,
                status: GateStatus::Passed,
                blocking_diagnostics: Some(0),
            },
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
        assert_eq!(presentation.groups[0].occurrences, 5);
        assert_eq!(presentation.groups[0].severity, Severity::Error);
        assert_eq!(presentation.groups[0].title, "Same warning");
        assert_eq!(
            presentation.groups[0].rule_url,
            "https://rust-doctor.vercel.app/rules/clippy%3A%3Asame_warning"
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
    fn frame_is_bounded_marks_primary_location_and_neutralizes_controls() {
        let root = temporary_root("bounded");
        fs::create_dir_all(root.join("src")).unwrap();
        let long = "x".repeat(200);
        fs::write(
            root.join("src/lib.rs"),
            format!("one\ntwo\n\tlet value = 1;\x1b]52;secret\u{009b}31m{long}\nfour\nfive\nsix\n"),
        )
        .unwrap();
        let location = GroupLocation {
            path: "src/lib.rs".to_owned(),
            span: DiagnosticSpan {
                line_start: 3,
                column_start: 5,
                line_end: 3,
                column_end: 8,
            },
        };

        let frame = code_frame(&root, &location).unwrap();
        assert_eq!(frame.location, "src/lib.rs:3:5");
        assert!(frame.lines.len() <= FRAME_MAX_LINES);
        assert_eq!(frame.lines.iter().filter(|line| line.primary).count(), 1);
        assert_eq!(
            frame.lines.iter().find(|line| line.primary).unwrap().marker,
            Some(CodeFrameMarker {
                column_start: 8,
                column_end: 11,
            })
        );
        assert!(
            frame
                .lines
                .iter()
                .all(|line| line.text.len() <= FRAME_MAX_COLUMNS)
        );
        let rendered = serde_json::to_string(&frame).unwrap();
        assert!(!rendered.contains("52;secret"));
        assert!(!rendered.contains('\u{001b}'));
        assert!(!rendered.contains('\u{009b}'));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn frame_resolves_normalized_paths_and_maps_source_to_terminal_columns() {
        let root = temporary_root("normalized-path");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/100%.rs"),
            format!("\t界e\u{301}{}\n", "界".repeat(100)),
        )
        .unwrap();
        let location = GroupLocation {
            path: "src/100%25.rs".to_owned(),
            span: DiagnosticSpan {
                line_start: 1,
                column_start: 5,
                line_end: 1,
                column_end: 6,
            },
        };

        let frame = code_frame(&root, &location).unwrap();
        let line = &frame.lines[0];
        assert_eq!(frame.location, "src/100%25.rs:1:5");
        assert_eq!(
            line.marker,
            Some(CodeFrameMarker {
                column_start: 21,
                column_end: 29,
            })
        );
        assert!(line.text.len() <= FRAME_MAX_COLUMNS);
        assert!(line.text.matches("\\u{754C}").count() < 100);

        let raw_path = GroupLocation {
            path: "src/100%.rs".to_owned(),
            span: location.span,
        };
        assert_eq!(
            code_frame(&root, &raw_path).unwrap_err().reason,
            CodeFrameUnavailableReason::OutsideWorkspace
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hostile_paths_binary_and_invalid_utf8_never_render_source() {
        let root = temporary_root("hostile");
        fs::write(root.join("binary.rs"), b"safe\0secret").unwrap();
        fs::write(root.join("invalid.rs"), [0xff, 0xfe]).unwrap();
        let span = DiagnosticSpan {
            line_start: 1,
            column_start: 1,
            line_end: 1,
            column_end: 2,
        };
        for (path, reason) in [
            ("../secret.rs", CodeFrameUnavailableReason::OutsideWorkspace),
            (
                "/private/secret.rs",
                CodeFrameUnavailableReason::OutsideWorkspace,
            ),
            ("binary.rs", CodeFrameUnavailableReason::Binary),
            ("invalid.rs", CodeFrameUnavailableReason::InvalidUtf8),
        ] {
            let error = code_frame(
                &root,
                &GroupLocation {
                    path: path.to_owned(),
                    span: span.clone(),
                },
            )
            .unwrap_err();
            assert_eq!(error.reason, reason, "{path}");
            assert!(error.message.len() < 1024);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_and_replacement_between_validation_and_open_are_rejected() {
        use std::os::unix::fs::symlink;

        let root = temporary_root("race-root");
        let outside = temporary_root("race-outside");
        fs::write(outside.join("secret.rs"), "TOP_SECRET\n").unwrap();
        symlink(outside.join("secret.rs"), root.join("escape.rs")).unwrap();
        let location = GroupLocation {
            path: "escape.rs".to_owned(),
            span: DiagnosticSpan {
                line_start: 1,
                column_start: 1,
                line_end: 1,
                column_end: 2,
            },
        };
        assert_eq!(
            code_frame(&root, &location).unwrap_err().reason,
            CodeFrameUnavailableReason::OutsideWorkspace
        );

        fs::write(root.join("replace.rs"), "safe\n").unwrap();
        let replacement = GroupLocation {
            path: "replace.rs".to_owned(),
            span: location.span,
        };
        let error = read_code_frame(&root, &replacement, || {
            fs::remove_file(root.join("replace.rs")).unwrap();
            symlink(outside.join("secret.rs"), root.join("replace.rs")).unwrap();
        })
        .unwrap_err();
        assert_eq!(error.reason, CodeFrameUnavailableReason::OutsideWorkspace);

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn empty_report_does_not_attempt_filesystem_access() {
        let presentation = ReportPresentation::derive(&report(Vec::new()));
        assert!(presentation.groups.is_empty());
        assert!(presentation.migration_advisories.is_empty());
    }
}
