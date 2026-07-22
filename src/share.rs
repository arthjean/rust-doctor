#![expect(
    clippy::redundant_pub_crate,
    reason = "the share contract is consumed by the CLI runner while its root module remains crate-private"
)]

use crate::diagnostics::{Category, CheckStatus, ScanResult};
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
    #[error("the share URL is {actual} bytes; the maximum is {maximum}")]
    TooLarge { actual: usize, maximum: usize },
}

/// Build a complete public summary in the query string. No report content is
/// uploaded or persisted by Rust Doctor.
pub(crate) fn build_url(result: &ScanResult) -> Result<String, ShareError> {
    if result.planned_files.is_empty() {
        return Err(ShareError::NothingToScan);
    }

    let authoritative = crate::completeness::score_is_authoritative(result);

    let mut category_counts = BTreeMap::<&'static str, usize>::new();
    for diagnostic in &result.diagnostics {
        *category_counts
            .entry(category_key(&diagnostic.category))
            .or_default() += 1;
    }

    let mut unavailable_checks = BTreeSet::new();
    for check in &result.execution.checks {
        if check.status == CheckStatus::Completed {
            continue;
        }
        if let Some(name) = canonical_check_name(&check.name) {
            unavailable_checks.insert(format!("{}:{name}", status_key(check.status)));
        }
    }
    let omitted_checks = unavailable_checks.len().saturating_sub(MAX_SHARED_CHECKS);

    let mut url = reqwest::Url::parse(SHARE_BASE_URL).map_err(|_| ShareError::InvalidBaseUrl)?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("v", SHARE_SCHEMA_VERSION);
        query.append_pair("tool", env!("CARGO_PKG_VERSION"));
        query.append_pair("score", &result.score.to_string());
        query.append_pair(
            "authoritative",
            if authoritative { "true" } else { "false" },
        );
        query.append_pair(
            "completeness",
            if authoritative {
                "complete"
            } else {
                "incomplete"
            },
        );
        query.append_pair("security", &result.dimension_scores.security.to_string());
        query.append_pair(
            "reliability",
            &result.dimension_scores.reliability.to_string(),
        );
        query.append_pair(
            "maintainability",
            &result.dimension_scores.maintainability.to_string(),
        );
        query.append_pair(
            "performance",
            &result.dimension_scores.performance.to_string(),
        );
        query.append_pair(
            "dependencies",
            &result.dimension_scores.dependencies.to_string(),
        );
        query.append_pair("errors", &result.error_count.to_string());
        query.append_pair("warnings", &result.warning_count.to_string());
        query.append_pair("info", &result.info_count.to_string());
        query.append_pair("planned", &result.planned_files.len().to_string());
        query.append_pair("analyzed", &result.analyzed_files.len().to_string());
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
    let name = name.trim();
    if name.is_empty() || name.len() > 64 {
        return None;
    }
    let mut canonical = String::with_capacity(name.len());
    for character in name.chars() {
        match character {
            'a'..='z' | '0'..='9' | '-' | ':' => canonical.push(character),
            ' ' => canonical.push('-'),
            _ => return None,
        }
    }
    Some(canonical)
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
        CheckState, Diagnostic, DimensionScores, ScanExecution, ScoreLabel, Severity,
    };
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
                        name: "cargo-audit".to_string(),
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
                ..ScanExecution::default()
            },
        }
    }

    #[test]
    fn stateless_url_contains_only_aggregate_contract_fields() {
        let url = build_url(&scan_result()).unwrap();
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
            pairs
                .iter()
                .any(|(key, value)| { key == "check" && value == "skipped:cargo-audit" })
        );
        assert!(!pairs.iter().any(|(_, value)| value.contains("/home")));
    }

    #[test]
    fn incomplete_required_work_marks_score_non_authoritative() {
        let mut result = scan_result();
        result.execution.checks[0].status = CheckStatus::TimedOut;
        let parsed = reqwest::Url::parse(&build_url(&result).unwrap()).unwrap();
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
    fn missing_required_checks_or_file_coverage_is_non_authoritative() {
        let mut result = scan_result();
        result.execution.checks.clear();
        let parsed = reqwest::Url::parse(&build_url(&result).unwrap()).unwrap();
        let pairs: BTreeMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(
            pairs.get("authoritative").map(String::as_str),
            Some("false")
        );

        let mut result = scan_result();
        result.analyzed_files.clear();
        let parsed = reqwest::Url::parse(&build_url(&result).unwrap()).unwrap();
        let pairs: BTreeMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(
            pairs.get("authoritative").map(String::as_str),
            Some("false")
        );
    }

    #[test]
    fn nothing_to_scan_has_no_share_url() {
        let mut result = scan_result();
        result.planned_files.clear();
        assert!(matches!(build_url(&result), Err(ShareError::NothingToScan)));
    }
}
