#![expect(
    clippy::redundant_pub_crate,
    reason = "the shared completeness predicate remains inside a crate-private root module"
)]

use crate::diagnostics::{CheckStatus, ScanResult};

/// One shared predicate for surfaces that present the health score outside the
/// full Report V1 contract.
pub(crate) fn score_is_authoritative(result: &ScanResult) -> bool {
    if result.planned_files.is_empty() || result.planned_files.len() != result.analyzed_files.len()
    {
        return false;
    }
    let mut required_check_count = 0_usize;
    for check in &result.execution.checks {
        if check.required {
            required_check_count += 1;
            if check.status != CheckStatus::Completed {
                return false;
            }
        }
    }
    if required_check_count == 0 {
        return false;
    }
    result
        .execution
        .baseline
        .as_ref()
        .is_none_or(|baseline| !baseline.baseline_degraded)
}
