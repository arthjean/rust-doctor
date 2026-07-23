#![expect(
    clippy::redundant_pub_crate,
    reason = "the shared completeness predicate remains inside a crate-private root module"
)]

use crate::diagnostics::{
    CheckState, CheckStatus, CompletenessState, ReportCompleteness, ScanResult,
};

/// Normalize legacy pass timing and skip receipts into the same check model
/// used by scoped scans.
pub(crate) fn effective_checks(result: &ScanResult) -> Vec<CheckState> {
    if !result.execution.checks.is_empty() {
        return result.execution.checks.clone();
    }
    let mut checks = std::collections::BTreeMap::new();
    for (name, _) in &result.pass_timings {
        checks.entry(name.clone()).or_insert_with(|| CheckState {
            name: name.clone(),
            required: is_required_check(name),
            status: CheckStatus::Completed,
            reason: None,
        });
    }
    for reason in &result.skipped_passes {
        let name = reason
            .split(':')
            .next()
            .unwrap_or(reason)
            .trim()
            .to_string();
        checks.insert(
            name.clone(),
            CheckState {
                name,
                required: is_required_check(reason),
                status: classify_check_status(reason),
                reason: Some(reason.clone()),
            },
        );
    }
    checks.into_values().collect()
}

/// Compute the only normative completeness value used by all consumers.
pub(crate) fn compute(result: &ScanResult) -> ReportCompleteness {
    let checks = effective_checks(result);
    let has_applicable_work = has_applicable_work(result);
    compute_from_parts(
        result.planned_files.len(),
        result.analyzed_files.len(),
        &checks,
        has_applicable_work,
        result
            .execution
            .baseline
            .as_ref()
            .is_some_and(|baseline| baseline.baseline_degraded),
    )
}

pub(crate) fn has_applicable_work(result: &ScanResult) -> bool {
    !result.planned_files.is_empty()
        || !effective_checks(result).is_empty()
        || !result.execution.packages.is_empty()
}

pub(crate) fn compute_from_parts(
    planned_files: usize,
    analyzed_files: usize,
    checks: &[CheckState],
    has_applicable_work: bool,
    baseline_degraded: bool,
) -> ReportCompleteness {
    let count = |status| checks.iter().filter(|check| check.status == status).count();
    let skipped = count(CheckStatus::Skipped);
    let failed = count(CheckStatus::Failed);
    let timed_out = count(CheckStatus::TimedOut);
    let cancelled = count(CheckStatus::Cancelled);
    let pending = count(CheckStatus::Planned) + count(CheckStatus::Running);
    let required_checks = checks.iter().filter(|check| check.required).count();
    let required_completed_checks = checks
        .iter()
        .filter(|check| check.required && check.status == CheckStatus::Completed)
        .count();
    let files_incomplete = analyzed_files != planned_files;
    let required_incomplete = required_checks != required_completed_checks;
    let required_plan_missing = has_applicable_work && required_checks == 0;
    let required_work_complete =
        !baseline_degraded && !files_incomplete && !required_incomplete && !required_plan_missing;
    let state = if !required_work_complete {
        CompletenessState::Incomplete
    } else if skipped + failed + timed_out + cancelled + pending > 0 {
        CompletenessState::Partial
    } else {
        CompletenessState::Complete
    };
    ReportCompleteness {
        state,
        planned_files,
        analyzed_files: analyzed_files.min(planned_files),
        completed_checks: count(CheckStatus::Completed),
        skipped_checks: skipped,
        failed_checks: failed,
        timed_out_checks: timed_out,
        cancelled_checks: cancelled,
        required_checks,
        required_completed_checks,
        score_authoritative: has_applicable_work && required_work_complete && required_checks > 0,
    }
}

/// One shared predicate for surfaces that present the health score outside the
/// full Report V1 contract.
pub(crate) fn score_is_authoritative(result: &ScanResult) -> bool {
    compute(result).score_authoritative
}

fn is_required_check(name: &str) -> bool {
    matches!(
        name.strip_prefix("base:")
            .unwrap_or(name)
            .split(':')
            .next()
            .unwrap_or(name)
            .trim(),
        "clippy" | "custom rules" | "msrv" | "baseline"
    )
}

fn classify_check_status(reason: &str) -> CheckStatus {
    let lower = reason.to_ascii_lowercase();
    if lower.contains("timed out") || lower.contains("timeout") {
        CheckStatus::TimedOut
    } else if lower.contains("cancel") {
        CheckStatus::Cancelled
    } else if lower.contains("failed") || lower.contains("panicked") {
        CheckStatus::Failed
    } else {
        CheckStatus::Skipped
    }
}
