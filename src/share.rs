#![expect(
    clippy::redundant_pub_crate,
    reason = "the share contract is consumed by the CLI runner while its root module remains crate-private"
)]

use crate::diagnostics::{ReportOutcome, ReportV1};
use std::fmt::Write as _;

const SHARE_BASE_URL: &str = "https://rust-doctor.vercel.app/share";
const MAX_SHARED_COUNT: usize = 1_000_000;
const MAX_SHARED_SCORE: u32 = 100;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ShareError {
    #[error("the scan contains no eligible Rust files")]
    NothingToScan,
    #[error("the report does not contain a shareable score payload")]
    InvalidPayload,
}

/// Build a short public summary in the query string: the score, the severity
/// counts, and the analyzed file count. No report content is uploaded or
/// persisted by Rust Doctor.
pub(crate) fn build_url(report: &ReportV1) -> Result<String, ShareError> {
    if report.outcome == ReportOutcome::NothingToScan {
        return Err(ShareError::NothingToScan);
    }
    if !report.report_constructed || report.error.is_some() {
        return Err(ShareError::InvalidPayload);
    }
    let score = report.score.ok_or(ShareError::InvalidPayload)?;
    let counts = [
        ("e", report.summary.error_count),
        ("w", report.summary.warning_count),
        ("i", report.summary.info_count),
        ("f", report.completeness.analyzed_files),
    ];

    if score > MAX_SHARED_SCORE || counts.iter().any(|(_, count)| *count > MAX_SHARED_COUNT) {
        return Err(ShareError::InvalidPayload);
    }

    let mut url = format!("{SHARE_BASE_URL}?s={score}");
    for (key, count) in counts {
        if count > 0 {
            let _ = write!(url, "&{key}={count}");
        }
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{
        Category, CheckState, CheckStatus, Diagnostic, DimensionScores, ScanExecution, ScanMode,
        ScanResult, ScoreLabel, Severity, SuppressionCounts,
    };
    use crate::discovery::ProjectInfo;
    use std::path::PathBuf;
    use std::time::Duration;

    fn scan_result() -> ScanResult {
        ScanResult {
            diagnostics: vec![Diagnostic {
                file_path: PathBuf::from("/home/arthur/private/src/lib.rs"),
                rule: "private-rule".to_string(),
                category: Category::Security,
                severity: Severity::Error,
                message: "TOP_SECRET".to_string(),
                help: Some("private remediation".to_string()),
                line: Some(42),
                column: Some(7),
                fix: None,
            }],
            score: 73,
            score_label: ScoreLabel::NeedsWork,
            dimension_scores: DimensionScores {
                security: 60,
                reliability: 90,
                maintainability: 80,
                performance: 75,
                dependencies: 100,
            },
            source_file_count: 1,
            elapsed: Duration::from_secs(2),
            skipped_passes: vec!["cargo-audit: private reason".to_string()],
            error_count: 1,
            warning_count: 0,
            info_count: 0,
            pass_timings: Vec::new(),
            suppressed_security: Vec::new(),
            planned_files: vec![PathBuf::from("/home/arthur/private/src/lib.rs")],
            analyzed_files: vec![PathBuf::from("/home/arthur/private/src/lib.rs")],
            compiler_evidence: Vec::new(),
            execution: ScanExecution {
                checks: vec![
                    CheckState {
                        name: "clippy".to_string(),
                        required: true,
                        status: CheckStatus::Completed,
                        reason: None,
                    },
                    CheckState {
                        name: "dependencies (cargo-audit)".to_string(),
                        required: false,
                        status: CheckStatus::Skipped,
                        reason: Some("/home/arthur/private credential".to_string()),
                    },
                ],
                suppression_counts: SuppressionCounts {
                    inline: 1,
                    rule: 2,
                    category: 3,
                    tag: 4,
                    path: 5,
                    security_policy: 6,
                },
                ..ScanExecution::default()
            },
        }
    }

    fn report() -> ReportV1 {
        let root = PathBuf::from("/home/arthur/private");
        let project = ProjectInfo {
            root_dir: root,
            name: "private".to_string(),
            version: "0.1.0".to_string(),
            package_id: "private 0.1.0".to_string(),
            targets: vec!["private:[Lib]".to_string()],
            cargo_targets: Vec::new(),
            edition: "2024".to_string(),
            frameworks: Vec::new(),
            framework_capabilities: Vec::new(),
            is_workspace: false,
            member_count: 1,
            has_build_script: false,
            rust_version: Some("1.97".to_string()),
            is_no_std: false,
            package_metadata: serde_json::Value::Null,
            workspace_members: Vec::new(),
            default_member_ids: vec!["private 0.1.0".to_string()],
        };
        ReportV1::from_scan(
            &scan_result(),
            &project,
            &crate::config::resolve_config_defaults(None),
            ScanMode::Full,
        )
    }

    #[test]
    fn stateless_url_stays_short_and_source_free() {
        let url = build_url(&report()).unwrap();
        assert_eq!(url, "https://rust-doctor.vercel.app/share?s=73&e=1&f=1");
        assert!(url.len() < 120);
        for prohibited in [
            "arthur",
            "private",
            "private-rule",
            "TOP_SECRET",
            "credential",
            "cargo-audit",
            "security",
        ] {
            assert!(!url.contains(prohibited), "share leaked {prohibited}");
        }
    }

    #[test]
    fn zero_counts_are_omitted() {
        let mut report = report();
        report.summary.error_count = 0;
        report.completeness.analyzed_files = 0;
        assert_eq!(
            build_url(&report).unwrap(),
            "https://rust-doctor.vercel.app/share?s=73"
        );
    }

    #[test]
    fn every_aggregate_count_is_carried() {
        let mut report = report();
        report.summary.error_count = 8;
        report.summary.warning_count = 723;
        report.summary.info_count = 2;
        report.completeness.analyzed_files = 108;
        assert_eq!(
            build_url(&report).unwrap(),
            "https://rust-doctor.vercel.app/share?s=73&e=8&w=723&i=2&f=108"
        );
    }

    #[test]
    fn nothing_to_scan_has_no_share_url() {
        let mut report = report();
        report.outcome = ReportOutcome::NothingToScan;
        assert!(matches!(build_url(&report), Err(ShareError::NothingToScan)));
    }

    #[test]
    fn invalid_report_has_no_partial_share_url() {
        let mut report = report();
        report.score = None;
        assert!(matches!(
            build_url(&report),
            Err(ShareError::InvalidPayload)
        ));
    }

    #[test]
    fn aggregate_contract_rejects_values_outside_public_bounds() {
        let mut invalid = Vec::new();

        let mut score = report();
        score.score = Some(MAX_SHARED_SCORE + 1);
        invalid.push(("score above 100", score));

        let mut errors = report();
        errors.summary.error_count = MAX_SHARED_COUNT + 1;
        invalid.push(("error count above limit", errors));

        let mut files = report();
        files.completeness.analyzed_files = MAX_SHARED_COUNT + 1;
        invalid.push(("analyzed file count above limit", files));

        for (label, report) in invalid {
            assert!(
                matches!(build_url(&report), Err(ShareError::InvalidPayload)),
                "{label} was accepted"
            );
        }
    }

    #[test]
    fn aggregate_contract_accepts_inclusive_boundaries() {
        let mut report = report();
        report.score = Some(MAX_SHARED_SCORE);
        report.summary.warning_count = MAX_SHARED_COUNT;
        report.completeness.analyzed_files = MAX_SHARED_COUNT;
        assert_eq!(
            build_url(&report).unwrap(),
            "https://rust-doctor.vercel.app/share?s=100&e=1&w=1000000&f=1000000"
        );
    }
}
