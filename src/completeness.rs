#![expect(
    clippy::redundant_pub_crate,
    reason = "the shared completeness predicate remains inside a crate-private root module"
)]

use crate::diagnostics::{
    CheckState, CheckStatus, CompletenessState, DimensionAuthority, DimensionCoverage,
    DimensionScores, PackageExecution, ReportCompleteness, ScanResult,
};
use crate::output::Dimension;
use std::collections::{BTreeMap, BTreeSet};

/// Stable internal identity of an analyzer. Unknown identities remain explicit
/// evidence and can never acquire authority through string heuristics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AnalyzerIdentity {
    Clippy,
    CustomRules,
    Msrv,
    CargoAudit,
    CargoDeny,
    CargoShear,
    CargoGeiger,
    SemverChecks,
    Coverage,
    Baseline,
    Scope,
    PackageScan,
    StagedSnapshot,
    Unknown(String),
}

impl AnalyzerIdentity {
    pub(crate) fn from_pass_name(name: &str) -> Self {
        match name {
            "clippy" => Self::Clippy,
            "custom rules" => Self::CustomRules,
            "msrv" => Self::Msrv,
            "dependencies (cargo-audit)" => Self::CargoAudit,
            "dependencies (cargo-deny)" => Self::CargoDeny,
            "dependencies (cargo-shear)" => Self::CargoShear,
            "unsafe audit (cargo-geiger)" => Self::CargoGeiger,
            "semver (cargo-semver-checks)" => Self::SemverChecks,
            "coverage" => Self::Coverage,
            "baseline" => Self::Baseline,
            "scope" | "scope:lines" => Self::Scope,
            "package scan" => Self::PackageScan,
            "staged snapshot" => Self::StagedSnapshot,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub(crate) fn display_name(&self) -> &str {
        match self {
            Self::Clippy => "clippy",
            Self::CustomRules => "custom rules",
            Self::Msrv => "msrv",
            Self::CargoAudit => "dependencies (cargo-audit)",
            Self::CargoDeny => "dependencies (cargo-deny)",
            Self::CargoShear => "dependencies (cargo-shear)",
            Self::CargoGeiger => "unsafe audit (cargo-geiger)",
            Self::SemverChecks => "semver (cargo-semver-checks)",
            Self::Coverage => "coverage",
            Self::Baseline => "baseline",
            Self::Scope => "scope:lines",
            Self::PackageScan => "package scan",
            Self::StagedSnapshot => "staged snapshot",
            Self::Unknown(name) => name,
        }
    }
}

/// Scope at which one analyzer receipt was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AnalyzerScope {
    Global,
    Root,
    Package,
    Workspace,
    Baseline,
}

/// Typed execution evidence retained until Report V1 serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnalyzerReceipt {
    pub(crate) analyzer: AnalyzerIdentity,
    pub(crate) scope: AnalyzerScope,
    pub(crate) package_id: Option<String>,
    pub(crate) required: bool,
    pub(crate) status: CheckStatus,
    pub(crate) reason: Option<String>,
}

impl AnalyzerReceipt {
    pub(crate) fn root(check: &CheckState) -> Self {
        Self {
            analyzer: AnalyzerIdentity::from_pass_name(&check.name),
            scope: AnalyzerScope::Root,
            package_id: None,
            required: check.required,
            status: check.status,
            reason: check.reason.clone(),
        }
    }

    pub(crate) fn global(check: &CheckState) -> Self {
        let mut receipt = Self::root(check);
        receipt.scope = AnalyzerScope::Global;
        receipt
    }

    pub(crate) fn for_package(mut self, package_id: String, multi_package: bool) -> Self {
        self.scope = if multi_package {
            AnalyzerScope::Package
        } else {
            AnalyzerScope::Root
        };
        self.package_id = Some(package_id);
        self
    }

    pub(crate) fn for_workspace(mut self) -> Self {
        self.scope = AnalyzerScope::Workspace;
        self.package_id = None;
        self
    }

    pub(crate) const fn for_baseline(mut self) -> Self {
        self.scope = AnalyzerScope::Baseline;
        self
    }

    pub(crate) fn to_check_state(&self) -> CheckState {
        let name = match self.scope {
            AnalyzerScope::Global | AnalyzerScope::Root => self.analyzer.display_name().to_string(),
            AnalyzerScope::Package => self.package_id.as_ref().map_or_else(
                || format!("package:{}", self.analyzer.display_name()),
                |package| format!("{package}:{}", self.analyzer.display_name()),
            ),
            AnalyzerScope::Workspace => format!("workspace:{}", self.analyzer.display_name()),
            AnalyzerScope::Baseline => format!("base:{}", self.analyzer.display_name()),
        };
        CheckState {
            name,
            required: self.required,
            status: self.status,
            reason: self.reason.clone(),
        }
    }
}

/// Which health dimensions an analyzer can speak for, and whether it ships with
/// Rust Doctor and the toolchain (`core`) or is an optional executable.
struct AnalyzerCoverage {
    analyzer: AnalyzerIdentity,
    core: bool,
    dimensions: &'static [Dimension],
}

const PRODUCT_DIMENSIONS: &[Dimension] = &[
    Dimension::Security,
    Dimension::Reliability,
    Dimension::Maintainability,
    Dimension::Performance,
];

/// Analyzer-to-dimension table. Optional adapters extend coverage but never
/// carry a dimension on their own, so an absent tool degrades completeness
/// without changing the Core Score.
const ANALYZER_COVERAGE: &[AnalyzerCoverage] = &[
    AnalyzerCoverage {
        analyzer: AnalyzerIdentity::Clippy,
        core: true,
        dimensions: PRODUCT_DIMENSIONS,
    },
    AnalyzerCoverage {
        analyzer: AnalyzerIdentity::CustomRules,
        core: true,
        dimensions: PRODUCT_DIMENSIONS,
    },
    AnalyzerCoverage {
        analyzer: AnalyzerIdentity::Msrv,
        core: true,
        dimensions: &[Dimension::Dependencies],
    },
    AnalyzerCoverage {
        analyzer: AnalyzerIdentity::CargoAudit,
        core: false,
        dimensions: &[Dimension::Security],
    },
    AnalyzerCoverage {
        analyzer: AnalyzerIdentity::CargoDeny,
        core: false,
        dimensions: &[Dimension::Dependencies],
    },
    AnalyzerCoverage {
        analyzer: AnalyzerIdentity::CargoShear,
        core: false,
        dimensions: &[Dimension::Dependencies],
    },
    AnalyzerCoverage {
        analyzer: AnalyzerIdentity::CargoGeiger,
        core: false,
        dimensions: &[Dimension::Security],
    },
    AnalyzerCoverage {
        analyzer: AnalyzerIdentity::SemverChecks,
        core: false,
        dimensions: &[Dimension::Reliability],
    },
    AnalyzerCoverage {
        analyzer: AnalyzerIdentity::Coverage,
        core: false,
        dimensions: &[Dimension::Reliability],
    },
];

fn coverage_for(analyzer: &AnalyzerIdentity) -> Option<&'static AnalyzerCoverage> {
    ANALYZER_COVERAGE
        .iter()
        .find(|coverage| &coverage.analyzer == analyzer)
}

/// Compute per-dimension coverage and authority from analyzer receipts.
///
/// A dimension only becomes authoritative when a core analyzer that can speak
/// for it actually completed. Missing evidence is reported as unobserved rather
/// than silently scored as healthy.
pub(crate) fn dimension_coverage(
    receipts: &[AnalyzerReceipt],
    dimensions: &DimensionScores,
    covered_scope: &[String],
    uncovered_scope: &[String],
) -> Vec<DimensionCoverage> {
    Dimension::ALL
        .into_iter()
        .map(|dimension| {
            let relevant: Vec<&AnalyzerReceipt> = receipts
                .iter()
                .filter(|check| {
                    coverage_for(&check.analyzer)
                        .is_some_and(|coverage| coverage.dimensions.contains(&dimension))
                })
                .collect();
            let names = |predicate: fn(CheckStatus) -> bool| -> Vec<String> {
                let mut names: Vec<_> = relevant
                    .iter()
                    .filter(|check| predicate(check.status))
                    .map(|check| check.analyzer.display_name().to_string())
                    .collect();
                names.sort();
                names.dedup();
                names
            };
            let mut planned_analyzers: Vec<String> = relevant
                .iter()
                .map(|check| check.analyzer.display_name().to_string())
                .collect();
            planned_analyzers.sort();
            planned_analyzers.dedup();
            let scheduled_analyzers =
                names(|status| !matches!(status, CheckStatus::Planned | CheckStatus::Skipped));
            let completed_analyzers = names(|status| status == CheckStatus::Completed);
            let skipped_analyzers = names(|status| status == CheckStatus::Skipped);
            let failed_analyzers = names(|status| {
                matches!(
                    status,
                    CheckStatus::Failed | CheckStatus::TimedOut | CheckStatus::Cancelled
                )
            });
            let core_completed = relevant.iter().any(|check| {
                check.status == CheckStatus::Completed
                    && coverage_for(&check.analyzer).is_some_and(|coverage| coverage.core)
            });
            let required_failed = relevant
                .iter()
                .any(|check| check.required && check.status != CheckStatus::Completed)
                || relevant
                    .iter()
                    .any(|check| check.required && !receipt_problem_codes(check).is_empty());
            let authority = if required_failed {
                DimensionAuthority::Failed
            } else if !core_completed {
                DimensionAuthority::Unobserved
            } else if !skipped_analyzers.is_empty() || !failed_analyzers.is_empty() {
                DimensionAuthority::Partial
            } else {
                DimensionAuthority::Authoritative
            };
            let observed = matches!(
                authority,
                DimensionAuthority::Authoritative | DimensionAuthority::Partial
            );
            let mut reasons: Vec<String> = relevant
                .iter()
                .filter(|check| check.status != CheckStatus::Completed)
                .filter_map(|check| check.reason.clone())
                .collect();
            if !observed && reasons.is_empty() {
                reasons.push(format!(
                    "no completed core analyzer covers the {} dimension",
                    dimension.as_str()
                ));
            }
            reasons.sort();
            reasons.dedup();
            DimensionCoverage {
                dimension: dimension.as_str().to_string(),
                planned_analyzers,
                scheduled_analyzers,
                completed_analyzers,
                skipped_analyzers,
                failed_analyzers,
                covered_scope: if observed {
                    covered_scope.to_vec()
                } else {
                    Vec::new()
                },
                uncovered_scope: if observed {
                    uncovered_scope.to_vec()
                } else {
                    covered_scope
                        .iter()
                        .chain(uncovered_scope.iter())
                        .cloned()
                        .collect()
                },
                authority,
                reasons,
                score: observed.then(|| dimension_value(dimensions, dimension)),
            }
        })
        .collect()
}

const fn dimension_value(scores: &DimensionScores, dimension: Dimension) -> u32 {
    match dimension {
        Dimension::Security => scores.security,
        Dimension::Reliability => scores.reliability,
        Dimension::Maintainability => scores.maintainability,
        Dimension::Performance => scores.performance,
        Dimension::Dependencies => scores.dependencies,
    }
}

/// Every dimension must be observed before an overall score can claim
/// authority (US-003 AC-2).
pub(crate) fn dimensions_are_authoritative(coverage: &[DimensionCoverage]) -> bool {
    !coverage.is_empty()
        && coverage.iter().all(|dimension| {
            matches!(
                dimension.authority,
                DimensionAuthority::Authoritative | DimensionAuthority::Partial
            )
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScoreVisibility {
    Visible,
    Hidden,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScoreAuthority {
    AuthoritativeCore,
    NonAuthoritative,
}

/// The only internal decision that may publish or gate on a score.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ScoreDecision {
    pub(crate) value: Option<u32>,
    pub(crate) label: Option<crate::diagnostics::ScoreLabel>,
    pub(crate) visibility: ScoreVisibility,
    pub(crate) authority: ScoreAuthority,
    pub(crate) reasons: Vec<String>,
}

impl ScoreDecision {
    pub(crate) const fn published_score(&self) -> Option<u32> {
        match (self.visibility, self.authority) {
            (ScoreVisibility::Visible, ScoreAuthority::AuthoritativeCore) => self.value,
            _ => None,
        }
    }

    pub(crate) const fn published_label(&self) -> Option<crate::diagnostics::ScoreLabel> {
        match (self.visibility, self.authority) {
            (ScoreVisibility::Visible, ScoreAuthority::AuthoritativeCore) => self.label,
            _ => None,
        }
    }

    pub(crate) const fn is_authoritative(&self) -> bool {
        matches!(
            (self.visibility, self.authority),
            (ScoreVisibility::Visible, ScoreAuthority::AuthoritativeCore)
        )
    }

    pub(crate) fn primary_reason(&self) -> &str {
        self.reasons
            .first()
            .map_or("score_not_authoritative", String::as_str)
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the canonical authority decision intentionally keeps all blocking and evidence reasons in one auditable pure function"
)]
pub(crate) fn score_decision(result: &ScanResult) -> ScoreDecision {
    if !has_applicable_work(result) || result.planned_files.is_empty() {
        return ScoreDecision {
            value: None,
            label: None,
            visibility: ScoreVisibility::Absent,
            authority: ScoreAuthority::NonAuthoritative,
            reasons: vec!["nothing_to_scan".to_string()],
        };
    }

    let receipts = effective_receipts(result);
    let completeness = compute(result);
    let dimensions = dimension_coverage(&receipts, &result.dimension_scores, &[], &[]);
    let mut blocking_reasons: BTreeSet<String> =
        receipt_set_problem_codes(&receipts).into_iter().collect();
    blocking_reasons.extend(receipt_owner_problem_codes(result, &receipts));
    let mut evidence_reasons = BTreeSet::new();

    if completeness.planned_files != completeness.analyzed_files {
        blocking_reasons.insert("planned_files_incomplete".to_string());
    }
    if result
        .execution
        .baseline
        .as_ref()
        .is_some_and(|baseline| baseline.baseline_degraded)
    {
        blocking_reasons.insert("baseline_degraded".to_string());
    }
    for receipt in &receipts {
        if receipt.required && receipt.status != CheckStatus::Completed {
            blocking_reasons.insert(format!(
                "required_analysis_{}:{}",
                check_status_code(receipt.status),
                receipt.analyzer.display_name()
            ));
            if let Some(reason) = receipt.reason.as_deref() {
                blocking_reasons.insert(format!(
                    "analysis_reason:{}:{}",
                    receipt.analyzer.display_name(),
                    execution_reason_code(reason)
                ));
            }
        } else if !receipt.required && receipt.status != CheckStatus::Completed {
            evidence_reasons.insert(format!(
                "partial_evidence_{}:{}",
                check_status_code(receipt.status),
                receipt.analyzer.display_name()
            ));
            if let Some(reason) = receipt.reason.as_deref() {
                evidence_reasons.insert(format!(
                    "partial_evidence_reason:{}:{}",
                    receipt.analyzer.display_name(),
                    execution_reason_code(reason)
                ));
            }
        }
    }
    for dimension in &dimensions {
        match dimension.authority {
            DimensionAuthority::Unobserved => {
                blocking_reasons.insert(format!("dimension_unobserved:{}", dimension.dimension));
            }
            DimensionAuthority::Failed => {
                blocking_reasons.insert(format!("dimension_failed:{}", dimension.dimension));
            }
            DimensionAuthority::Partial => {
                evidence_reasons.insert(format!("partial_evidence:{}", dimension.dimension));
            }
            DimensionAuthority::Authoritative => {}
        }
    }

    if !result.execution.packages.is_empty() {
        for package in &result.execution.packages {
            let package_receipts = receipts_for_package(result, &package.cargo_package_id);
            let package_checks: Vec<_> = package_receipts
                .iter()
                .map(AnalyzerReceipt::to_check_state)
                .collect();
            let package_completeness = compute_from_parts(
                package.planned_files.len(),
                package.analyzed_files.len(),
                &package_checks,
                !package.planned_files.is_empty() || !package_checks.is_empty(),
                false,
            );
            let package_dimensions =
                dimension_coverage(&package_receipts, &result.dimension_scores, &[], &[]);
            if package.score.is_none()
                || !package_completeness.score_authoritative
                || !dimensions_are_authoritative(&package_dimensions)
            {
                blocking_reasons.insert(format!(
                    "package_non_authoritative:{}",
                    package.cargo_package_id
                ));
            }
        }
    }

    let mut reasons: Vec<String> = blocking_reasons
        .iter()
        .chain(evidence_reasons.iter())
        .cloned()
        .collect();
    reasons.sort();
    reasons.dedup();
    if blocking_reasons.is_empty() && completeness.score_authoritative {
        ScoreDecision {
            value: Some(result.score),
            label: Some(result.score_label),
            visibility: ScoreVisibility::Visible,
            authority: ScoreAuthority::AuthoritativeCore,
            reasons,
        }
    } else {
        if reasons.is_empty() {
            reasons.push("required_analysis_incomplete".to_string());
        }
        ScoreDecision {
            value: Some(result.score),
            label: Some(result.score_label),
            visibility: ScoreVisibility::Hidden,
            authority: ScoreAuthority::NonAuthoritative,
            reasons,
        }
    }
}

const fn check_status_code(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Planned => "planned",
        CheckStatus::Running => "running",
        CheckStatus::Completed => "completed",
        CheckStatus::Skipped => "skipped",
        CheckStatus::Failed => "failed",
        CheckStatus::TimedOut => "timed_out",
        CheckStatus::Cancelled => "cancelled",
    }
}

fn execution_reason_code(reason: &str) -> &'static str {
    let reason = reason.to_ascii_lowercase();
    if reason.contains("status 2") || reason.contains("failed exit") {
        "failed_exit"
    } else if reason.contains("timeout") || reason.contains("timed out") {
        "timeout"
    } else if reason.contains("cancel") {
        "cancelled"
    } else if reason.contains("panic") {
        "panic"
    } else if reason.contains("truncat") || reason.contains("capture limit") {
        "truncated_output"
    } else if reason.contains("malformed") || reason.contains("json") {
        "malformed_output"
    } else if reason.contains("unsupported") || reason.contains("outside the qualified") {
        "unsupported_tool_version"
    } else if reason.contains("missing") || reason.contains("not installed") {
        "missing_analyzer"
    } else {
        "analysis_failed"
    }
}

pub(crate) fn effective_receipts(result: &ScanResult) -> Vec<AnalyzerReceipt> {
    let mut receipts = if result.execution.analyzer_receipts.is_empty() {
        effective_checks(result)
            .iter()
            .map(|check| legacy_receipt(check, result))
            .collect()
    } else {
        result.execution.analyzer_receipts.clone()
    };
    sort_receipts(&mut receipts);
    receipts
}

pub(crate) fn receipts_for_package(result: &ScanResult, package_id: &str) -> Vec<AnalyzerReceipt> {
    effective_receipts(result)
        .into_iter()
        .filter(|receipt| match receipt.scope {
            AnalyzerScope::Global | AnalyzerScope::Workspace => true,
            AnalyzerScope::Root | AnalyzerScope::Package => {
                receipt.package_id.as_deref() == Some(package_id)
            }
            AnalyzerScope::Baseline => receipt
                .package_id
                .as_deref()
                .is_none_or(|owner| owner == package_id),
        })
        .collect()
}

fn legacy_receipt(check: &CheckState, result: &ScanResult) -> AnalyzerReceipt {
    let default_scope = if result.execution.packages.is_empty() {
        AnalyzerScope::Global
    } else {
        AnalyzerScope::Root
    };
    let (scope, name) = check.name.strip_prefix("base:").map_or_else(
        || {
            check
                .name
                .strip_prefix("workspace:")
                .map_or((default_scope, check.name.as_str()), |name| {
                    (AnalyzerScope::Workspace, name)
                })
        },
        |name| (AnalyzerScope::Baseline, name),
    );
    let analyzer = AnalyzerIdentity::from_pass_name(name);
    let package_id = (scope == AnalyzerScope::Root && result.execution.packages.len() == 1)
        .then(|| result.execution.packages[0].cargo_package_id.clone());
    AnalyzerReceipt {
        analyzer,
        scope,
        package_id,
        required: check.required,
        status: check.status,
        reason: check.reason.clone(),
    }
}

pub(crate) fn sort_receipts(receipts: &mut [AnalyzerReceipt]) {
    receipts.sort_by(|left, right| {
        left.scope
            .cmp(&right.scope)
            .then(left.package_id.cmp(&right.package_id))
            .then(left.analyzer.cmp(&right.analyzer))
            .then(left.required.cmp(&right.required))
            .then(status_rank(left.status).cmp(&status_rank(right.status)))
            .then(left.reason.cmp(&right.reason))
    });
}

pub(crate) fn receipt_problem_codes(receipt: &AnalyzerReceipt) -> Vec<String> {
    let mut reasons = Vec::new();
    if let AnalyzerIdentity::Unknown(name) = &receipt.analyzer {
        reasons.push(format!("unknown_analyzer:{name}"));
    }
    if matches!(receipt.scope, AnalyzerScope::Root | AnalyzerScope::Package)
        && receipt.package_id.is_none()
        && !matches!(
            receipt.analyzer,
            AnalyzerIdentity::Scope | AnalyzerIdentity::Baseline
        )
    {
        reasons.push(format!(
            "missing_package_ownership:{}",
            receipt.analyzer.display_name()
        ));
    }
    reasons
}

pub(crate) fn receipt_set_problem_codes(receipts: &[AnalyzerReceipt]) -> Vec<String> {
    let mut reasons = BTreeSet::new();
    let mut by_identity: BTreeMap<_, Vec<&AnalyzerReceipt>> = BTreeMap::new();
    for receipt in receipts {
        reasons.extend(receipt_problem_codes(receipt));
        by_identity
            .entry((
                receipt.scope,
                receipt.package_id.clone(),
                receipt.analyzer.clone(),
            ))
            .or_default()
            .push(receipt);
    }
    for ((scope, package, analyzer), matching) in by_identity {
        if matching.len() <= 1 {
            continue;
        }
        let identity = format!(
            "{scope:?}:{}:{}",
            package.as_deref().unwrap_or("workspace"),
            analyzer.display_name()
        );
        reasons.insert(format!("duplicate_receipt:{identity}"));
        let first_status = matching[0].status;
        if matching
            .iter()
            .skip(1)
            .any(|receipt| receipt.status != first_status)
        {
            reasons.insert(format!("conflicting_terminal_status:{identity}"));
        }
    }
    reasons.into_iter().collect()
}

fn receipt_owner_problem_codes(result: &ScanResult, receipts: &[AnalyzerReceipt]) -> Vec<String> {
    if result.execution.packages.is_empty() {
        return Vec::new();
    }
    let package_ids: BTreeSet<&str> = result
        .execution
        .packages
        .iter()
        .map(|package| package.cargo_package_id.as_str())
        .collect();
    let mut reasons = BTreeSet::new();
    for receipt in receipts {
        if matches!(receipt.scope, AnalyzerScope::Root | AnalyzerScope::Package)
            && let Some(owner) = receipt.package_id.as_deref()
            && !package_ids.contains(owner)
        {
            reasons.insert(format!(
                "unknown_package_ownership:{}:{owner}",
                receipt.analyzer.display_name()
            ));
        }
    }
    reasons.into_iter().collect()
}

const fn status_rank(status: CheckStatus) -> u8 {
    match status {
        CheckStatus::Planned => 0,
        CheckStatus::Running => 1,
        CheckStatus::Completed => 2,
        CheckStatus::Skipped => 3,
        CheckStatus::Failed => 4,
        CheckStatus::TimedOut => 5,
        CheckStatus::Cancelled => 6,
    }
}

/// Normalize legacy pass timing and skip receipts into the same check model
/// used by scoped scans.
pub(crate) fn effective_checks(result: &ScanResult) -> Vec<CheckState> {
    if !result.execution.analyzer_receipts.is_empty() {
        return effective_receipts(result)
            .iter()
            .map(AnalyzerReceipt::to_check_state)
            .collect();
    }
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
    .with_abstentions(&result.execution.abstentions)
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
        abstentions: 0,
    }
}

/// Attach the aggregate abstention volume without changing any other value:
/// declining to decide degrades observability, it never fails a required check.
trait WithAbstentions {
    fn with_abstentions(self, receipts: &[crate::rules::AbstentionReceipt]) -> Self;
}

impl WithAbstentions for ReportCompleteness {
    fn with_abstentions(mut self, receipts: &[crate::rules::AbstentionReceipt]) -> Self {
        self.abstentions = receipts.iter().map(|receipt| receipt.count).sum();
        self
    }
}

/// Compute package completeness with the same invariant used by Report V1.
pub(crate) fn compute_package(package: &PackageExecution) -> ReportCompleteness {
    compute_from_parts(
        package.planned_files.len(),
        package.analyzed_files.len(),
        &package.checks,
        !package.planned_files.is_empty() || !package.checks.is_empty(),
        false,
    )
}

pub(crate) fn package_score_is_authoritative(package: &PackageExecution) -> bool {
    compute_package(package).score_authoritative
}

#[cfg(test)]
pub(crate) fn score_is_reportable(result: &ScanResult) -> bool {
    score_decision(result).published_score().is_some()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{DimensionScores, ScanExecution, ScoreLabel};
    use std::path::PathBuf;
    use std::time::Duration;

    fn receipt(
        analyzer: AnalyzerIdentity,
        status: CheckStatus,
        package_id: &str,
    ) -> AnalyzerReceipt {
        AnalyzerReceipt {
            analyzer,
            scope: AnalyzerScope::Root,
            package_id: Some(package_id.to_string()),
            required: true,
            status,
            reason: (status != CheckStatus::Completed).then(|| "fixture failure".to_string()),
        }
    }

    fn complete_receipts(package_id: &str) -> Vec<AnalyzerReceipt> {
        vec![
            receipt(AnalyzerIdentity::Clippy, CheckStatus::Completed, package_id),
            receipt(
                AnalyzerIdentity::CustomRules,
                CheckStatus::Completed,
                package_id,
            ),
            receipt(AnalyzerIdentity::Msrv, CheckStatus::Completed, package_id),
        ]
    }

    fn result_with(receipts: Vec<AnalyzerReceipt>) -> ScanResult {
        let source = PathBuf::from("src/lib.rs");
        ScanResult {
            diagnostics: Vec::new(),
            score: 87,
            score_label: ScoreLabel::Great,
            dimension_scores: DimensionScores {
                security: 92,
                reliability: 88,
                maintainability: 86,
                performance: 90,
                dependencies: 80,
            },
            source_file_count: 1,
            elapsed: Duration::ZERO,
            skipped_passes: Vec::new(),
            error_count: 0,
            warning_count: 0,
            info_count: 0,
            pass_timings: Vec::new(),
            suppressed_security: Vec::new(),
            planned_files: vec![source.clone()],
            analyzed_files: vec![source],
            compiler_evidence: Vec::new(),
            execution: ScanExecution {
                analyzer_receipts: receipts,
                ..ScanExecution::default()
            },
        }
    }

    #[test]
    fn optional_adapter_absence_keeps_the_core_score_visible() {
        let mut receipts = complete_receipts("fixture 0.1.0");
        receipts.push(AnalyzerReceipt {
            analyzer: AnalyzerIdentity::CargoDeny,
            scope: AnalyzerScope::Workspace,
            package_id: None,
            required: false,
            status: CheckStatus::Skipped,
            reason: Some("cargo-deny is not installed".to_string()),
        });
        let decision = score_decision(&result_with(receipts));
        assert_eq!(decision.published_score(), Some(87));
        assert_eq!(decision.authority, ScoreAuthority::AuthoritativeCore);
        assert!(
            decision
                .reasons
                .iter()
                .any(|reason| reason.starts_with("partial_evidence"))
        );
    }

    #[test]
    fn required_failure_hides_score_and_gives_a_stable_reason() {
        let mut receipts = complete_receipts("fixture 0.1.0");
        receipts[0].status = CheckStatus::Failed;
        let decision = score_decision(&result_with(receipts));
        assert_eq!(decision.published_score(), None);
        assert_eq!(decision.visibility, ScoreVisibility::Hidden);
        assert!(
            decision
                .reasons
                .contains(&"required_analysis_failed:clippy".to_string())
        );
    }

    #[test]
    fn opaque_package_identity_is_never_parsed() {
        let package = "path+file:///workspace/member#crate@0.1.0:opaque";
        let receipt = receipt(AnalyzerIdentity::Clippy, CheckStatus::Completed, package)
            .for_package(package.to_string(), true);
        assert_eq!(receipt.package_id.as_deref(), Some(package));
        assert_eq!(receipt.to_check_state().name, format!("{package}:clippy"));
    }

    #[test]
    fn duplicates_conflicts_and_unknown_analyzers_fail_closed() {
        let mut receipts = complete_receipts("fixture 0.1.0");
        let mut conflict = receipts[0].clone();
        conflict.status = CheckStatus::Failed;
        receipts.push(conflict);
        receipts.push(AnalyzerReceipt {
            analyzer: AnalyzerIdentity::Unknown("future analyzer".to_string()),
            scope: AnalyzerScope::Root,
            package_id: None,
            required: false,
            status: CheckStatus::Completed,
            reason: None,
        });
        let reasons = receipt_set_problem_codes(&receipts);
        assert!(
            reasons
                .iter()
                .any(|reason| reason.starts_with("duplicate_receipt:"))
        );
        assert!(
            reasons
                .iter()
                .any(|reason| reason.starts_with("conflicting_terminal_status:"))
        );
        assert!(
            reasons
                .iter()
                .any(|reason| reason == "unknown_analyzer:future analyzer")
        );
        assert!(
            reasons
                .iter()
                .any(|reason| reason == "missing_package_ownership:future analyzer")
        );
    }

    #[test]
    fn one_workspace_failure_hides_every_package_headline() {
        let mut receipts: Vec<_> = ["first", "second"]
            .into_iter()
            .flat_map(|package| {
                complete_receipts(package)
                    .into_iter()
                    .map(move |receipt| receipt.for_package(package.to_string(), true))
            })
            .collect();
        receipts.push(AnalyzerReceipt {
            analyzer: AnalyzerIdentity::CargoDeny,
            scope: AnalyzerScope::Workspace,
            package_id: None,
            required: true,
            status: CheckStatus::Failed,
            reason: Some("status 2".to_string()),
        });
        let mut result = result_with(receipts);
        result
            .planned_files
            .push(PathBuf::from("second/src/lib.rs"));
        result
            .analyzed_files
            .push(PathBuf::from("second/src/lib.rs"));
        result.execution.packages = ["first", "second"]
            .into_iter()
            .map(|package| PackageExecution {
                cargo_package_id: package.to_string(),
                package_root: PathBuf::from(package),
                planned_files: vec![PathBuf::from(format!("{package}/src/lib.rs"))],
                analyzed_files: vec![PathBuf::from(format!("{package}/src/lib.rs"))],
                checks: complete_receipts(package)
                    .iter()
                    .map(AnalyzerReceipt::to_check_state)
                    .collect(),
                elapsed: Duration::ZERO,
                score: Some(87),
            })
            .collect();
        let decision = score_decision(&result);
        assert_eq!(decision.published_score(), None);
        assert!(
            decision
                .reasons
                .contains(&"required_analysis_failed:dependencies (cargo-deny)".to_string())
        );
        assert!(
            decision
                .reasons
                .iter()
                .any(|reason| reason.starts_with("package_non_authoritative:"))
        );
    }

    #[test]
    fn receipt_owned_by_an_unknown_package_cannot_authorize_a_single_package_scan() {
        let expected = "known 0.1.0";
        let mut result = result_with(complete_receipts("wrong 0.1.0"));
        result.execution.packages.push(PackageExecution {
            cargo_package_id: expected.to_string(),
            package_root: PathBuf::from("."),
            planned_files: result.planned_files.clone(),
            analyzed_files: result.analyzed_files.clone(),
            checks: Vec::new(),
            elapsed: Duration::ZERO,
            score: Some(87),
        });
        let decision = score_decision(&result);
        assert_eq!(decision.published_score(), None);
        assert!(
            decision
                .reasons
                .iter()
                .any(|reason| reason.starts_with("unknown_package_ownership:"))
        );
    }

    #[test]
    fn at_least_one_hundred_receipt_permutations_have_identical_decision_json() {
        let mut receipts = complete_receipts("fixture 0.1.0");
        receipts.push(AnalyzerReceipt {
            analyzer: AnalyzerIdentity::CargoDeny,
            scope: AnalyzerScope::Workspace,
            package_id: None,
            required: false,
            status: CheckStatus::Skipped,
            reason: Some("optional".to_string()),
        });
        receipts.push(AnalyzerReceipt {
            analyzer: AnalyzerIdentity::Coverage,
            scope: AnalyzerScope::Root,
            package_id: Some("fixture 0.1.0".to_string()),
            required: false,
            status: CheckStatus::Completed,
            reason: None,
        });
        let mut permutations = Vec::new();
        collect_permutations(&mut receipts, 0, &mut permutations);
        assert!(permutations.len() >= 100);
        let expected = serde_json::to_vec(&score_decision(&result_with(permutations[0].clone())))
            .expect("decision serializes");
        for permutation in permutations.iter().take(100) {
            let observed = serde_json::to_vec(&score_decision(&result_with(permutation.clone())))
                .expect("decision serializes");
            assert_eq!(observed, expected);
        }
    }

    fn collect_permutations(
        receipts: &mut [AnalyzerReceipt],
        index: usize,
        output: &mut Vec<Vec<AnalyzerReceipt>>,
    ) {
        if index == receipts.len() {
            output.push(receipts.to_vec());
            return;
        }
        for next in index..receipts.len() {
            receipts.swap(index, next);
            collect_permutations(receipts, index + 1, output);
            receipts.swap(index, next);
        }
    }
}
