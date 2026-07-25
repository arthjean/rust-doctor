#![expect(
    clippy::redundant_pub_crate,
    reason = "the share contract is consumed by the CLI runner while its root module remains crate-private"
)]

use crate::diagnostics::{Category, CheckStatus, CompletenessState, ReportOutcome, ReportV1};
use std::collections::{BTreeMap, BTreeSet};

const SHARE_BASE_URL: &str = "https://rust-doctor.vercel.app/share/";
const SHARE_SCHEMA_VERSION: &str = "1";
const MAX_SHARE_URL_BYTES: usize = 8 * 1024;
const MAX_SHARED_CHECKS: usize = 32;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ShareError {
    #[error("the scan contains no eligible Rust files")]
    NothingToScan,
    #[error("the built-in share URL is invalid")]
    InvalidBaseUrl,
    #[error("the report does not contain a shareable score payload")]
    InvalidPayload,
    #[error("the share URL is {actual} bytes; the maximum is {maximum}")]
    TooLarge { actual: usize, maximum: usize },
}

/// Build a complete public summary in the query string. No report content is
/// uploaded or persisted by Rust Doctor.
pub(crate) fn build_url(report: &ReportV1) -> Result<String, ShareError> {
    if report.outcome == ReportOutcome::NothingToScan {
        return Err(ShareError::NothingToScan);
    }
    if !report.report_constructed || report.error.is_some() {
        return Err(ShareError::InvalidPayload);
    }
    let score = report.score.ok_or(ShareError::InvalidPayload)?;
    let dimensions = report
        .dimension_scores
        .as_ref()
        .ok_or(ShareError::InvalidPayload)?;

    let mut category_counts = BTreeMap::<&'static str, usize>::new();
    for diagnostic in &report.diagnostics {
        *category_counts
            .entry(category_key(&diagnostic.category))
            .or_default() += 1;
    }

    let mut unavailable_checks = BTreeSet::new();
    for project in &report.projects {
        for check in &project.checks {
            if check.status == CheckStatus::Completed {
                continue;
            }
            if let Some(name) = canonical_check_name(&check.name) {
                unavailable_checks.insert(format!("{}:{name}", status_key(check.status)));
            }
        }
    }
    let omitted_checks = unavailable_checks.len().saturating_sub(MAX_SHARED_CHECKS);

    let mut url = reqwest::Url::parse(SHARE_BASE_URL).map_err(|_| ShareError::InvalidBaseUrl)?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("v", SHARE_SCHEMA_VERSION);
        query.append_pair("tool", &report.tool_version);
        query.append_pair("score", &score.to_string());
        query.append_pair(
            "authoritative",
            if report.summary.score_authoritative {
                "true"
            } else {
                "false"
            },
        );
        query.append_pair(
            "completeness",
            match report.completeness.state {
                CompletenessState::Complete => "complete",
                CompletenessState::Partial => "partial",
                CompletenessState::Incomplete => "incomplete",
            },
        );
        query.append_pair("security", &dimensions.security.to_string());
        query.append_pair("reliability", &dimensions.reliability.to_string());
        query.append_pair("maintainability", &dimensions.maintainability.to_string());
        query.append_pair("performance", &dimensions.performance.to_string());
        query.append_pair("dependencies", &dimensions.dependencies.to_string());
        query.append_pair("errors", &report.summary.error_count.to_string());
        query.append_pair("warnings", &report.summary.warning_count.to_string());
        query.append_pair("info", &report.summary.info_count.to_string());
        query.append_pair("planned", &report.completeness.planned_files.to_string());
        query.append_pair("analyzed", &report.completeness.analyzed_files.to_string());
        for (category, count) in category_counts {
            query.append_pair("category", &format!("{category}:{count}"));
        }
        for check in unavailable_checks.iter().take(MAX_SHARED_CHECKS) {
            query.append_pair("check", check);
        }
        if omitted_checks > 0 {
            query.append_pair("checks_omitted", &omitted_checks.to_string());
        }
    }

    let rendered = url.to_string();
    if rendered.len() > MAX_SHARE_URL_BYTES {
        return Err(ShareError::TooLarge {
            actual: rendered.len(),
            maximum: MAX_SHARE_URL_BYTES,
        });
    }
    Ok(rendered)
}

fn canonical_check_name(name: &str) -> Option<String> {
    let name = name
        .trim()
        .strip_prefix("workspace:")
        .unwrap_or_else(|| name.trim());
    match name {
        "clippy" => Some("clippy"),
        "custom rules" => Some("custom-rules"),
        "dependencies (cargo-audit)" => Some("dependencies-cargo-audit"),
        "dependencies (cargo-deny)" => Some("dependencies-cargo-deny"),
        "dependencies (cargo-machete)" => Some("dependencies-cargo-machete"),
        "unsafe audit (cargo-geiger)" => Some("unsafe-audit-cargo-geiger"),
        "coverage" => Some("coverage"),
        "msrv" => Some("msrv"),
        "semver (cargo-semver-checks)" => Some("semver-cargo-semver-checks"),
        "baseline" => Some("baseline"),
        "staged snapshot" => Some("staged-snapshot"),
        "package scan" => Some("package-scan"),
        "scope:lines" => Some("scope-lines"),
        _ => None,
    }
    .map(str::to_string)
}

const fn category_key(category: &Category) -> &'static str {
    match category {
        Category::ErrorHandling => "error-handling",
        Category::Performance => "performance",
        Category::Security => "security",
        Category::Correctness => "correctness",
        Category::Architecture => "architecture",
        Category::Dependencies => "dependencies",
        Category::Async => "async",
        Category::Framework => "framework",
        Category::Cargo => "cargo",
        Category::Style => "style",
    }
}

const fn status_key(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Planned => "planned",
        CheckStatus::Running => "running",
        CheckStatus::Completed => "completed",
        CheckStatus::Skipped => "skipped",
        CheckStatus::Failed => "failed",
        CheckStatus::TimedOut => "timed-out",
        CheckStatus::Cancelled => "cancelled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{
        CheckState, Diagnostic, DimensionScores, ScanExecution, ScanMode, ScanResult, ScoreLabel,
        Severity, SuppressionCounts,
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
                    CheckState {
                        name: "/home/arthur/private".to_string(),
                        required: false,
                        status: CheckStatus::Skipped,
                        reason: None,
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
    fn stateless_url_contains_only_aggregate_contract_fields() {
        let url = build_url(&report()).unwrap();
        assert!(url.len() < MAX_SHARE_URL_BYTES);
        for prohibited in [
            "arthur",
            "private-rule",
            "TOP_SECRET",
            "private+remediation",
            "credential",
        ] {
            assert!(!url.contains(prohibited), "share leaked {prohibited}");
        }

        let parsed = reqwest::Url::parse(&url).unwrap();
        let pairs: Vec<_> = parsed.query_pairs().collect();
        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "score" && value == "73")
        );
        assert!(
            pairs
                .iter()
                .any(|(key, value)| { key == "category" && value == "security:1" })
        );
        assert!(
            pairs.iter().any(|(key, value)| {
                key == "check" && value == "skipped:dependencies-cargo-audit"
            })
        );
        assert!(!pairs.iter().any(|(_, value)| value.contains("/home")));
    }

    #[test]
    fn incomplete_required_work_marks_score_non_authoritative() {
        let mut report = report();
        report.completeness.state = CompletenessState::Incomplete;
        report.summary.score_authoritative = false;
        let parsed = reqwest::Url::parse(&build_url(&report).unwrap()).unwrap();
        let pairs: BTreeMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(
            pairs.get("completeness").map(String::as_str),
            Some("incomplete")
        );
        assert_eq!(
            pairs.get("authoritative").map(String::as_str),
            Some("false")
        );
    }

    #[test]
    fn completeness_and_score_authority_are_copied_without_collapsing() {
        let mut report = report();
        report.completeness.state = CompletenessState::Partial;
        report.summary.score_authoritative = true;
        let parsed = reqwest::Url::parse(&build_url(&report).unwrap()).unwrap();
        let pairs: BTreeMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(pairs.get("authoritative").map(String::as_str), Some("true"));
        assert_eq!(
            pairs.get("completeness").map(String::as_str),
            Some("partial")
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
    fn oversized_payload_is_rejected_without_a_partial_url() {
        let mut report = report();
        report.tool_version = "x".repeat(MAX_SHARE_URL_BYTES);
        assert!(matches!(
            build_url(&report),
            Err(ShareError::TooLarge {
                maximum: MAX_SHARE_URL_BYTES,
                ..
            })
        ));
    }

    #[test]
    fn skipped_checks_are_fixed_allowlisted_names_with_bounded_cardinality() {
        let allowed = [
            "clippy",
            "custom rules",
            "dependencies (cargo-audit)",
            "dependencies (cargo-deny)",
            "dependencies (cargo-machete)",
            "unsafe audit (cargo-geiger)",
            "coverage",
            "msrv",
            "semver (cargo-semver-checks)",
            "baseline",
            "staged snapshot",
            "package scan",
            "scope:lines",
        ];
        let mut report = report();
        report.projects[0].checks = allowed
            .into_iter()
            .chain(std::iter::once("private-project-name"))
            .map(|name| CheckState {
                name: name.to_string(),
                required: false,
                status: CheckStatus::Skipped,
                reason: Some("private reason".to_string()),
            })
            .collect();
        let parsed = reqwest::Url::parse(&build_url(&report).unwrap()).unwrap();
        let checks: Vec<_> = parsed
            .query_pairs()
            .filter(|(key, _)| key == "check")
            .map(|(_, value)| value.into_owned())
            .collect();

        assert_eq!(checks.len(), allowed.len());
        assert!(checks.iter().all(|check| !check.contains("private")));
        assert!(checks.len() <= MAX_SHARED_CHECKS);
    }
}
