#[cfg(feature = "mcp")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// Severity of a diagnostic finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error => write!(f, "error"),
            Self::Warning => write!(f, "warning"),
            Self::Info => write!(f, "info"),
        }
    }
}

/// Category of a diagnostic rule.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    ErrorHandling,
    Performance,
    Security,
    Correctness,
    Architecture,
    Dependencies,
    Async,
    Framework,
    Cargo,
    Style,
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ErrorHandling => write!(f, "Error Handling"),
            Self::Performance => write!(f, "Performance"),
            Self::Security => write!(f, "Security"),
            Self::Correctness => write!(f, "Correctness"),
            Self::Architecture => write!(f, "Architecture"),
            Self::Dependencies => write!(f, "Dependencies"),
            Self::Async => write!(f, "Async"),
            Self::Framework => write!(f, "Framework"),
            Self::Cargo => write!(f, "Cargo"),
            Self::Style => write!(f, "Style"),
        }
    }
}

/// A machine-applicable code fix suggestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct CodeFix {
    /// The text to find (exact match in the source line).
    pub old_text: String,
    /// The replacement text.
    pub new_text: String,
    /// Line number (1-based) where the fix applies.
    pub line: u32,
}

/// A single diagnostic finding from an analysis pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct Diagnostic {
    /// Path to the source file (relative to project root).
    pub file_path: PathBuf,
    /// Rule identifier (e.g. "unwrap-in-production", "clippy::unwrap_used").
    pub rule: String,
    /// Category this rule belongs to.
    pub category: Category,
    /// Severity of the finding.
    pub severity: Severity,
    /// Human-readable description of the issue.
    pub message: String,
    /// Actionable suggestion for how to fix the issue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Line number (1-based) where the issue was found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Column number (1-based) where the issue was found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// Machine-applicable fix, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<CodeFix>,
}

/// Exact source evidence retained from a rustc or Clippy JSON diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerSpanEvidence {
    pub file_path: PathBuf,
    pub range: SourceRange,
}

/// Macro call-site evidence attached to a compiler span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerMacroEvidence {
    pub macro_name: String,
    pub call_site: CompilerSpanEvidence,
}

/// A rustc suggestion retained before conversion to the public report DTO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerFixEvidence {
    pub group_id: Option<String>,
    pub applicability: FixApplicability,
    pub replacement: String,
    pub span: CompilerSpanEvidence,
}

/// Compiler-only evidence that the legacy `Diagnostic` model cannot represent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerDiagnosticEvidence {
    pub(crate) provenance: crate::catalog::AdapterProvenance,
    pub rule: String,
    pub message: String,
    pub file_path: PathBuf,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub original_level: String,
    pub primary_span: Option<CompilerSpanEvidence>,
    pub related_locations: Vec<(String, CompilerSpanEvidence)>,
    pub macro_expansion: Option<CompilerMacroEvidence>,
    pub fixes: Vec<CompilerFixEvidence>,
}

impl CompilerDiagnosticEvidence {
    pub(crate) fn matches(&self, diagnostic: &Diagnostic) -> bool {
        self.rule == diagnostic.rule
            && self.message == diagnostic.message
            && self.file_path == diagnostic.file_path
            && self.line == diagnostic.line
            && self.column == diagnostic.column
    }
}

/// Human-readable health assessment label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub enum ScoreLabel {
    #[serde(rename = "Great")]
    Great,
    #[serde(rename = "Needs work")]
    NeedsWork,
    #[serde(rename = "Critical")]
    Critical,
}

impl std::fmt::Display for ScoreLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Great => write!(f, "Great"),
            Self::NeedsWork => write!(f, "Needs work"),
            Self::Critical => write!(f, "Critical"),
        }
    }
}

/// Per-dimension health scores.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct DimensionScores {
    /// Security dimension score (0–100). Covers Security and Dependencies (advisory) categories.
    pub security: u32,
    /// Reliability dimension score (0–100). Covers Correctness and ErrorHandling categories.
    pub reliability: u32,
    /// Maintainability dimension score (0–100). Covers Architecture and Style categories.
    pub maintainability: u32,
    /// Performance dimension score (0–100). Covers Performance category.
    pub performance: u32,
    /// Dependencies dimension score (0–100). Covers Cargo and Dependencies categories.
    pub dependencies: u32,
}

/// Result of a complete scan across all analysis passes.
#[derive(Debug, Serialize)]
pub struct ScanResult {
    /// All diagnostics found (after filtering).
    pub diagnostics: Vec<Diagnostic>,
    /// Health score (0–100).
    pub score: u32,
    /// Score label.
    pub score_label: ScoreLabel,
    /// Per-dimension health scores.
    pub dimension_scores: DimensionScores,
    /// Number of source files scanned.
    pub source_file_count: usize,
    /// Total scan duration.
    #[serde(serialize_with = "serialize_duration")]
    pub elapsed: Duration,
    /// Names of analysis passes that were skipped or failed.
    pub skipped_passes: Vec<String>,
    /// Number of errors found.
    pub error_count: usize,
    /// Number of warnings found.
    pub warning_count: usize,
    /// Number of info-severity findings.
    pub info_count: usize,
    /// Per-pass timing information (pass name → elapsed duration).
    #[serde(serialize_with = "serialize_pass_timings")]
    pub pass_timings: Vec<(String, Duration)>,
    /// Security findings disabled by project policy. Kept out of normal output
    /// and scoring, but retained for verbose audit metadata in Report V1.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suppressed_security: Vec<Diagnostic>,
    /// Eligible files planned before analysis and files whose root analysis ran.
    #[serde(skip)]
    pub planned_files: Vec<PathBuf>,
    #[serde(skip)]
    pub analyzed_files: Vec<PathBuf>,
    /// Rich rustc evidence joined to diagnostics when Report V1 is constructed.
    #[serde(skip)]
    pub compiler_evidence: Vec<CompilerDiagnosticEvidence>,
    /// Scope, check, baseline, and per-package execution accounting.
    #[serde(skip)]
    pub execution: ScanExecution,
}

/// Internal execution accounting retained until Report V1 construction.
#[derive(Debug, Clone)]
pub struct ScanExecution {
    pub execution_scope: String,
    pub reporting_scope: String,
    pub checks: Vec<CheckState>,
    /// Typed analyzer evidence. `checks` is the Report V1 compatibility view
    /// derived from these receipts at serialization boundaries.
    pub(crate) analyzer_receipts: Vec<crate::completeness::AnalyzerReceipt>,
    pub packages: Vec<PackageExecution>,
    pub baseline: Option<BaselineReport>,
    pub suppression_counts: SuppressionCounts,
    pub analysis_failures: Vec<AnalysisFailureReceipt>,
    /// Rules that declined to decide, aggregated per rule and reason. An
    /// abstention is an analyzer receipt, never a finding (US-006).
    pub abstentions: Vec<AbstentionReceipt>,
    /// Effective Cargo `[lints]` policy resolved per scanned package.
    pub lint_policy: Vec<LintPolicyEntry>,
}

impl Default for ScanExecution {
    fn default() -> Self {
        Self {
            execution_scope: "full_packages".to_string(),
            reporting_scope: "full".to_string(),
            checks: Vec::new(),
            analyzer_receipts: Vec::new(),
            packages: Vec::new(),
            baseline: None,
            suppression_counts: SuppressionCounts::default(),
            analysis_failures: Vec::new(),
            abstentions: Vec::new(),
            lint_policy: Vec::new(),
        }
    }
}

/// One package's executed work before it is converted to wire paths.
#[derive(Debug, Clone)]
pub struct PackageExecution {
    pub cargo_package_id: String,
    pub package_root: PathBuf,
    pub planned_files: Vec<PathBuf>,
    pub analyzed_files: Vec<PathBuf>,
    pub checks: Vec<CheckState>,
    pub elapsed: Duration,
    pub score: Option<u32>,
}

/// Stable machine-output mode vocabulary for Report V1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ScanMode {
    Full,
    Files,
    Lines,
    Staged,
    Baseline,
}

/// High-level outcome. `Partial` and `Failed` never mean clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReportOutcome {
    Clean,
    Findings,
    Partial,
    Failed,
    NothingToScan,
}

/// Quality-gate decision carried independently from scan outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum GateResult {
    Passed,
    Failed,
    NotEvaluated,
}

/// Completeness of planned analysis work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum CompletenessState {
    Complete,
    Partial,
    Incomplete,
}

/// Check execution state retained in machine output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Planned,
    Running,
    Completed,
    Skipped,
    Failed,
    TimedOut,
    Cancelled,
}

/// One planned analysis check and its final state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct CheckState {
    pub name: String,
    #[serde(default)]
    pub required: bool,
    pub status: CheckStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Aggregate execution-accounting contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct ReportCompleteness {
    pub state: CompletenessState,
    pub planned_files: usize,
    pub analyzed_files: usize,
    pub completed_checks: usize,
    pub skipped_checks: usize,
    pub failed_checks: usize,
    pub timed_out_checks: usize,
    pub cancelled_checks: usize,
    #[serde(default)]
    pub required_checks: usize,
    #[serde(default)]
    pub required_completed_checks: usize,
    #[serde(default)]
    pub score_authoritative: bool,
    /// Total rule-file decisions declined for missing or unsupported context.
    #[serde(default)]
    pub abstentions: usize,
}

/// Whether a health dimension was actually observed by an analyzer able to
/// speak for it. `Unobserved` and `Failed` never mean healthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum DimensionAuthority {
    /// Every required analyzer for this dimension completed.
    Authoritative,
    /// At least one analyzer completed while others were skipped.
    Partial,
    /// No analyzer able to speak for this dimension completed.
    Unobserved,
    /// A required analyzer failed, timed out, or was cancelled.
    Failed,
}

/// Planned, scheduled, and completed evidence for one health dimension.
///
/// Consumers read coverage from this record instead of recomputing it: an
/// unobserved dimension carries `score: None`, never a synthetic 100.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct DimensionCoverage {
    pub dimension: String,
    pub planned_analyzers: Vec<String>,
    pub scheduled_analyzers: Vec<String>,
    pub completed_analyzers: Vec<String>,
    pub skipped_analyzers: Vec<String>,
    pub failed_analyzers: Vec<String>,
    pub covered_scope: Vec<String>,
    pub uncovered_scope: Vec<String>,
    pub authority: DimensionAuthority,
    pub reasons: Vec<String>,
    pub score: Option<u32>,
}

/// Per-package score distribution for a workspace headline.
///
/// The headline is the lowest authoritative package score. Aggregate health is
/// descriptive portfolio data and never replaces the headline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct WorkspaceHealth {
    pub headline_package_id: Option<String>,
    pub headline_score: Option<u32>,
    pub authoritative: bool,
    pub minimum_score: Option<u32>,
    pub median_score: Option<u32>,
    pub maximum_score: Option<u32>,
    pub authoritative_packages: usize,
    pub non_authoritative_packages: usize,
    pub package_scores: Vec<PackageScore>,
}

/// One package's contribution to the workspace distribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct PackageScore {
    pub cargo_package_id: String,
    pub score: Option<u32>,
    pub authoritative: bool,
}

/// Source position with optional byte offset when the adapter provides it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct SourcePosition {
    pub line: u32,
    pub column: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_offset: Option<u32>,
}

/// Inclusive diagnostic range in a normalized project-relative file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct SourceRange {
    pub start: SourcePosition,
    pub end: SourcePosition,
}

/// Explicit source or project-level location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiagnosticLocation {
    Source { path: String, range: SourceRange },
    Project,
}

/// Cargo ownership is explicit even when a finding has no source span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiagnosticOwnership {
    Package { package_id: String },
    Workspace,
    Unowned,
}

/// Rust-native source context shared by policy and machine consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SourceSurface {
    Library,
    Binary,
    Test,
    Bench,
    Example,
    BuildScript,
    Generated,
    MacroExpansion,
    Unknown,
}

/// Suppression provenance retained for privacy-safe aggregate reporting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct SuppressionCounts {
    pub inline: usize,
    pub rule: usize,
    pub category: usize,
    pub tag: usize,
    pub path: usize,
    pub security_policy: usize,
}

/// Structured failed analysis work retained after panic containment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct AnalysisFailureReceipt {
    pub check: String,
    pub kind: String,
    pub path: String,
    pub rule: Option<String>,
    pub reason: String,
}

/// Aggregate record of one rule declining to decide.
///
/// Abstentions are analyzer receipts, not user-facing diagnostics: they degrade
/// completeness and feed rule metrics without inventing a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct AbstentionReceipt {
    /// Canonical rule identity that abstained.
    pub rule: String,
    /// Stable machine-readable reason: a missing context requirement or an
    /// unsupported source surface.
    pub reason: String,
    /// How many analyzed files the rule declined to decide on.
    pub count: usize,
}

/// Where a declared Cargo lint level came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum LintPolicySource {
    /// `[lints]` in the owning package manifest.
    Package,
    /// `[workspace.lints]`, reached through `[lints] workspace = true`.
    WorkspaceInherited,
}

/// One lint's effective Cargo level next to Rust Doctor's own mapping.
///
/// Both are preserved: the declared level explains why a lint is or is not
/// forced on, and the Rust Doctor severity explains how a finding is presented
/// once it exists (US-008).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct LintPolicyEntry {
    pub lint: String,
    pub declared_level: String,
    pub source: LintPolicySource,
    /// Rust Doctor's curated severity, when the lint is in the registry.
    pub rust_doctor_severity: Option<Severity>,
}

impl SuppressionCounts {
    #[must_use]
    pub const fn total(&self) -> usize {
        self.inline + self.rule + self.category + self.tag + self.path
    }
}

/// Related source evidence retained by adapters that expose it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct RelatedLocation {
    pub message: String,
    pub location: DiagnosticLocation,
}

/// Macro expansion call-site context, when supplied by rustc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct MacroExpansion {
    pub macro_name: String,
    pub call_site: DiagnosticLocation,
}

/// rustc-compatible applicability vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum FixApplicability {
    MachineApplicable,
    MaybeIncorrect,
    HasPlaceholders,
    Unspecified,
}

/// Whether Rust Doctor may apply an edit without human review.
///
/// Guidance is always safe to show. Applying an edit is not, so eligibility is
/// an explicit decision with a recorded reason rather than a consequence of a
/// suggestion merely existing (US-016).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum FixEligibility {
    /// Exact span, verified replacement, and a precondition hash: applying it
    /// is a mechanical edit inside one span.
    MachineApplicable,
    /// Shown as advice only. The default, so an unproven fix is never applied.
    #[default]
    GuidanceOnly,
}

/// One stable edit group. A group may resolve multiple findings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct CanonicalFix {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    pub applicability: FixApplicability,
    pub replacement: String,
    pub location: DiagnosticLocation,
    /// Whether this edit may be applied automatically.
    #[serde(default)]
    pub eligibility: FixEligibility,
    /// SHA-256 of the target file as analyzed. Applying against a different
    /// hash means the source moved since the scan and the fix is stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precondition_hash: Option<String>,
    /// Why the edit is guidance only, when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ineligible_reason: Option<String>,
}

/// Whether a diagnostic moved the Core Score, and why not when it did not.
///
/// Kept separate from severity, confidence, and priority: a loud finding can be
/// worth exactly zero points, and consumers must be able to say so rather than
/// implying that every reported issue lowered the number (US-014).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum ScoreImpact {
    /// Score-eligible, owned by a core analyzer, counted by Score Core V2.
    Scored,
    /// Runs by default and is reported for review, but contributes zero.
    Advisory,
    /// Not score-eligible: uncalibrated, audit-only, optional-external, or
    /// unmapped. The default, so an unknown rule never claims impact.
    #[default]
    Ineligible,
    /// Suppressed by policy: observable where exports permit it, never scored.
    Suppressed,
}

/// One canonical root cause.
///
/// The group owns priority and bounded score impact while every occurrence
/// stays inspectable through `site_ids` (US-014 AC-3).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct RootCauseGroup {
    pub key: String,
    pub title: String,
    pub rule: String,
    pub category: Category,
    /// `None` when the owning rule is unranked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    pub trust_tier: String,
    pub aggregation_policy: String,
    pub score_impact: ScoreImpact,
    pub occurrences: usize,
    pub file_count: usize,
    /// Score dimension affected by this group. Absent for advisory and audit
    /// groups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_dimension: Option<String>,
    /// Exact pre-rounding contribution of this group to its dimension.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_penalty: Option<f64>,
    /// Declared upper bound under the active aggregation policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum_penalty: Option<f64>,
    /// One bounded action title shared by terminal, plan, and handoff output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation_title: Option<String>,
    pub site_ids: Vec<String>,
}

/// Canonical diagnostic consumed by JSON, SARIF, MCP, CI, and editors.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct CanonicalDiagnostic {
    pub provider: String,
    pub rule: String,
    pub title: String,
    pub category: Category,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    pub url: String,
    pub tags: Vec<String>,
    pub analysis_kind: String,
    pub confidence: String,
    pub original_level: String,
    pub ownership: DiagnosticOwnership,
    pub source_surface: SourceSurface,
    pub location: DiagnosticLocation,
    pub related_locations: Vec<RelatedLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub macro_expansion: Option<MacroExpansion>,
    pub fixes: Vec<CanonicalFix>,
    pub visible_on: Vec<String>,
    pub site_id: String,
    pub baseline_key: String,
    pub namespace_fallback: bool,
    /// True when the owning rule runs by default but contributes exactly zero
    /// to the score. Consumers label it advisory rather than implying that its
    /// presence lowered the number (US-007).
    #[serde(default)]
    pub advisory: bool,

    // --- Canonical decision metadata (US-014) ---
    // Additive: every field above keeps its prior name, type, and meaning.
    /// Product urgency (`p0`–`p3`), independent from presentation severity.
    /// Absent when the rule is unranked: an unknown or dynamically discovered
    /// rule never receives a fabricated priority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Provenance class of the evidence behind this finding.
    #[serde(default)]
    pub trust_tier: String,
    /// Whether the owning rule satisfies its trust-tier evidence contract.
    #[serde(default)]
    pub score_eligible: bool,
    /// What this occurrence actually did to the Core Score.
    #[serde(default)]
    pub score_impact: ScoreImpact,
    /// How repeated findings from this rule aggregate for ranking and score.
    #[serde(default)]
    pub aggregation_policy: String,
    /// Stable identity of the underlying defect, shared by every occurrence.
    /// Absent for unmapped rules rather than hashed from message text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_cause_key: Option<String>,
    /// What evidence the analyzer used to reach this conclusion.
    #[serde(default)]
    pub evidence_summary: String,
    /// Known boundaries where this rule can be wrong.
    #[serde(default)]
    pub limitations: Vec<String>,
    /// Identifier of a validated fix recipe. Absent when remediation is
    /// guidance only, so output never claims machine applicability it lacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix_recipe: Option<String>,
    /// True when policy suppressed this finding. Suppressed records may appear
    /// in exports that permit them, always with zero score impact.
    #[serde(default)]
    pub suppressed: bool,
}

/// Per-package Report V1 payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct ProjectReport {
    pub cargo_package_id: String,
    pub package_root: String,
    pub targets: Vec<String>,
    pub framework_capabilities: Vec<String>,
    #[serde(default)]
    pub framework_gates: Vec<FrameworkCapabilityReport>,
    pub planned_files: Vec<String>,
    pub analyzed_files: Vec<String>,
    pub checks: Vec<CheckState>,
    pub skipped_reasons: Vec<String>,
    pub completeness: ReportCompleteness,
    pub score: Option<u32>,
    #[serde(default)]
    pub score_authoritative: bool,
    /// Planned, completed, skipped, and failed evidence per health dimension.
    #[serde(default)]
    pub dimensions: Vec<DimensionCoverage>,
    #[serde(default)]
    pub elapsed_ms: u64,
    #[serde(default)]
    pub diagnostics: Vec<CanonicalDiagnostic>,
}

/// Version, feature, and analyzed-target evidence for one framework gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct FrameworkCapabilityReport {
    pub framework: String,
    pub version: Option<String>,
    pub enabled_features: Vec<String>,
    pub target_contexts: Vec<String>,
    pub analyzed_target: Option<String>,
    pub active: bool,
    pub gate_reason: Option<String>,
    #[serde(default)]
    pub rule_gates: Vec<FrameworkRuleGateReport>,
}

/// Exact activation receipt for one framework-aware custom rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct FrameworkRuleGateReport {
    pub rule: String,
    pub active: bool,
    pub gate_reason: Option<String>,
}

/// Machine-readable summary independent of terminal presentation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct ReportSummary {
    pub score: Option<u32>,
    pub score_label: Option<ScoreLabel>,
    pub error_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    pub diagnostic_count: usize,
    #[serde(default)]
    pub score_authoritative: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub score_reasons: Vec<String>,
}

/// Head/base comparison metadata for baseline mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct BaselineReport {
    pub requested_base: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_base: Option<String>,
    pub base_commit: String,
    pub head_config_fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_config_fingerprint: Option<String>,
    pub new_count: usize,
    pub fixed_count: usize,
    pub base_total: usize,
    pub cross_file_match_count: usize,
    pub baseline_degraded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degraded_reason: Option<String>,
}

/// Structured failure retained even when discovery or scanning fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct ReportError {
    pub kind: String,
    pub message: String,
    pub causes: Vec<String>,
    pub exit_classification: String,
}

/// Verbose policy audit data that never re-enables suppressed findings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct AuditMetadata {
    pub suppressed_security: Vec<CanonicalDiagnostic>,
    #[serde(default)]
    pub suppression_counts: SuppressionCounts,
    #[serde(default)]
    pub analysis_failures: Vec<AnalysisFailureReceipt>,
    /// Rules that declined to decide, aggregated per rule and reason. An
    /// abstention is an analyzer receipt, never a finding (US-006).
    #[serde(default)]
    pub abstentions: Vec<AbstentionReceipt>,
    /// Effective Cargo `[lints]` policy resolved per package, kept next to
    /// Rust Doctor's own severity mapping (US-008).
    #[serde(default)]
    pub lint_policy: Vec<LintPolicyEntry>,
    /// Approved threshold exceptions currently in force. Empty in a normal
    /// build: a rule shipping on an exception is visible here rather than
    /// bypassing its gate invisibly (US-012 AC-7).
    #[serde(default)]
    pub trust_exceptions: Vec<TrustExceptionRecord>,
}

/// Trust-gate exceptions still inside their approval window today.
fn active_trust_exceptions() -> Vec<TrustExceptionRecord> {
    crate::trust::active_exceptions(&crate::trust::today_utc())
        .into_iter()
        .map(|exception| TrustExceptionRecord {
            rule: exception.rule,
            owner: exception.owner,
            granted_at: exception.granted_at,
            expires_at: exception.expires_at,
            reason: exception.reason,
            evidence: exception.evidence,
        })
        .collect()
}

/// One deliberate, time-bounded trust-gate exception, as reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct TrustExceptionRecord {
    /// The single rule this exception covers.
    pub rule: String,
    /// Accountable owner of the decision.
    pub owner: String,
    /// ISO-8601 date the exception was approved.
    pub granted_at: String,
    /// ISO-8601 date the exception stops applying.
    pub expires_at: String,
    pub reason: String,
    /// Link or identifier for the evidence behind the decision.
    pub evidence: String,
}

/// Versioned Report V1. Legacy top-level score/count fields remain during the
/// v0.2 migration release, while `summary` is the canonical replacement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(JsonSchema))]
pub struct ReportV1 {
    pub schema_version: String,
    pub tool_version: String,
    pub report_constructed: bool,
    pub outcome: ReportOutcome,
    pub requested_root: String,
    pub resolved_root: Option<String>,
    pub mode: ScanMode,
    pub gate_result: GateResult,
    #[serde(default)]
    pub execution_scope: String,
    #[serde(default)]
    pub reporting_scope: String,
    pub completeness: ReportCompleteness,
    /// Score Core version that produced every score in this report. Legacy
    /// values came from an unversioned model and are not comparable.
    #[serde(default)]
    pub score_model_version: String,
    /// Analyzer authority and coverage per health dimension.
    #[serde(default)]
    pub dimensions: Vec<DimensionCoverage>,
    /// Workspace distribution behind the headline score.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_health: Option<WorkspaceHealth>,
    /// Canonical root causes behind the diagnostics, highest impact first.
    /// Each group owns the priority and bounded score impact of its
    /// occurrences, which stay individually inspectable in `diagnostics`.
    #[serde(default)]
    pub root_causes: Vec<RootCauseGroup>,
    pub projects: Vec<ProjectReport>,
    pub diagnostics: Vec<CanonicalDiagnostic>,
    pub summary: ReportSummary,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub pass_timings_ms: BTreeMap<String, u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ReportError>,
    pub audit: AuditMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<BaselineReport>,

    // V1 migration aliases for the current GitHub Action and 0.1 JSON users.
    pub score: Option<u32>,
    pub score_label: Option<ScoreLabel>,
    pub dimension_scores: Option<DimensionScores>,
    pub source_file_count: usize,
    pub elapsed: f64,
    pub skipped_passes: Vec<String>,
    pub error_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
}

impl ReportV1 {
    pub(crate) fn from_scan(
        result: &ScanResult,
        project: &crate::discovery::ProjectInfo,
        config: &crate::config::ResolvedConfig,
        mode: ScanMode,
    ) -> Self {
        Self::from_scan_with_context(
            result,
            project,
            config,
            mode,
            &project.root_dir,
            GateResult::NotEvaluated,
        )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "report assembly keeps compatibility aliases and canonical V1 fields at one serialization boundary"
    )]
    pub(crate) fn from_scan_with_context(
        result: &ScanResult,
        project: &crate::discovery::ProjectInfo,
        config: &crate::config::ResolvedConfig,
        mode: ScanMode,
        requested_root: &std::path::Path,
        gate_result: GateResult,
    ) -> Self {
        let root = &project.root_dir;
        let mut diagnostics: Vec<_> = result
            .diagnostics
            .iter()
            .map(|diagnostic| {
                let evidence = result
                    .compiler_evidence
                    .iter()
                    .find(|evidence| evidence.matches(diagnostic));
                canonicalize(diagnostic, evidence, project, config)
            })
            .collect();
        // Deduplicate on the stable identity first, then apply the canonical
        // order every consumer shares (US-015 AC-1).
        diagnostics.sort_by(|left, right| {
            left.site_id
                .cmp(&right.site_id)
                .then(left.message.cmp(&right.message))
        });
        diagnostics.dedup_by(|left, right| left.site_id == right.site_id);
        let impact = crate::ordering::RootCauseImpact::measure(diagnostics.iter());
        crate::ordering::sort_owned(&mut diagnostics, &impact);
        let root_causes = crate::ordering::root_cause_groups(&diagnostics);
        let mut suppressed_security: Vec<_> = result
            .suppressed_security
            .iter()
            .map(|diagnostic| {
                let mut canonical = canonicalize(diagnostic, None, project, config);
                // A suppressed record stays inspectable, but its zero score
                // impact is explicit rather than implied (US-014 AC-6).
                canonical.suppressed = true;
                canonical.score_impact = ScoreImpact::Suppressed;
                canonical
            })
            .collect();
        suppressed_security.sort_by(|left, right| left.site_id.cmp(&right.site_id));

        let receipts = crate::completeness::effective_receipts(result);
        let checks = crate::completeness::effective_checks(result);
        let planned_files = normalize_report_files(root, &result.planned_files);
        let analyzed_files = normalize_report_files(root, &result.analyzed_files);
        let completeness = crate::completeness::compute(result);
        let score_decision = crate::completeness::score_decision(result);
        let nothing_to_scan = !crate::completeness::has_applicable_work(result);
        let computed_score = score_decision.value;
        let outcome = if nothing_to_scan {
            ReportOutcome::NothingToScan
        } else if completeness.state != CompletenessState::Complete {
            ReportOutcome::Partial
        } else if diagnostics.is_empty() {
            ReportOutcome::Clean
        } else {
            ReportOutcome::Findings
        };
        let covered_scope: Vec<String> = result
            .execution
            .packages
            .iter()
            .filter(|package| crate::completeness::package_score_is_authoritative(package))
            .map(|package| normalize_path(root, &package.package_root))
            .collect();
        let mut uncovered_scope: Vec<String> = result
            .execution
            .packages
            .iter()
            .filter(|package| !crate::completeness::package_score_is_authoritative(package))
            .map(|package| normalize_path(root, &package.package_root))
            .collect();
        // Compiling analyzers run the declared default feature profile. Every
        // other profile is unanalyzed, and unanalyzed is not healthy (US-008
        // AC-8): it is named here rather than inferred away.
        uncovered_scope.extend(unanalyzed_feature_profiles(project));
        let dimensions = crate::completeness::dimension_coverage(
            &receipts,
            &result.dimension_scores,
            &covered_scope,
            &uncovered_scope,
        );
        let projects = project_reports(
            project,
            result,
            &diagnostics,
            &planned_files,
            &analyzed_files,
            &checks,
            &completeness,
            computed_score,
            &dimensions,
        );
        let mut workspace_health = workspace_health(&projects);
        let score = score_decision.published_score();
        let score_label = score_decision.published_label();
        let score_authoritative = score_decision.is_authoritative();
        if score.is_none()
            && let Some(health) = &mut workspace_health
        {
            health.headline_package_id = None;
            health.headline_score = None;
            health.authoritative = false;
        }
        let summary = ReportSummary {
            score,
            score_label,
            error_count: result.error_count,
            warning_count: result.warning_count,
            info_count: result.info_count,
            diagnostic_count: diagnostics.len(),
            score_authoritative,
            score_reasons: score_decision.reasons,
        };
        let reporting_scope =
            if result.execution.reporting_scope == "full" && mode != ScanMode::Full {
                scan_mode_name(mode).to_string()
            } else {
                result.execution.reporting_scope.clone()
            };
        let mut pass_timings_ms = BTreeMap::new();
        for (pass, elapsed) in &result.pass_timings {
            let milliseconds = duration_millis(*elapsed);
            pass_timings_ms
                .entry(pass.clone())
                .and_modify(|total: &mut u64| *total = total.saturating_add(milliseconds))
                .or_insert(milliseconds);
        }

        Self {
            schema_version: "1.0".to_string(),
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            report_constructed: true,
            outcome,
            requested_root: display_path(requested_root),
            resolved_root: Some(display_path(root)),
            mode,
            gate_result,
            execution_scope: result.execution.execution_scope.clone(),
            reporting_scope,
            completeness,
            score_model_version: crate::output::score_model_version().to_string(),
            dimensions,
            workspace_health,
            root_causes,
            projects,
            diagnostics,
            summary,
            elapsed_ms: duration_millis(result.elapsed),
            pass_timings_ms,
            error: None,
            audit: AuditMetadata {
                abstentions: result.execution.abstentions.clone(),
                lint_policy: result.execution.lint_policy.clone(),
                suppressed_security,
                suppression_counts: result.execution.suppression_counts.clone(),
                analysis_failures: result.execution.analysis_failures.clone(),
                trust_exceptions: active_trust_exceptions(),
            },
            baseline: result.execution.baseline.clone(),
            score,
            score_label,
            dimension_scores: score.map(|_| result.dimension_scores.clone()),
            source_file_count: result.source_file_count,
            elapsed: result.elapsed.as_secs_f64(),
            skipped_passes: result.skipped_passes.clone(),
            error_count: result.error_count,
            warning_count: result.warning_count,
            info_count: result.info_count,
        }
    }

    pub(crate) fn failure(
        requested_root: &std::path::Path,
        mode: ScanMode,
        kind: &str,
        message: &str,
    ) -> Self {
        Self::failure_with_causes(requested_root, mode, kind, message, &[])
    }

    pub(crate) fn failure_with_causes(
        requested_root: &std::path::Path,
        mode: ScanMode,
        kind: &str,
        message: &str,
        causes: &[String],
    ) -> Self {
        let completeness = ReportCompleteness {
            state: CompletenessState::Incomplete,
            planned_files: 0,
            analyzed_files: 0,
            completed_checks: 0,
            skipped_checks: 0,
            failed_checks: 1,
            timed_out_checks: 0,
            cancelled_checks: 0,
            required_checks: 1,
            required_completed_checks: 0,
            score_authoritative: false,
            abstentions: 0,
        };
        Self {
            schema_version: "1.0".to_string(),
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            report_constructed: true,
            outcome: ReportOutcome::Failed,
            requested_root: display_path(requested_root),
            resolved_root: None,
            mode,
            gate_result: GateResult::NotEvaluated,
            execution_scope: "none".to_string(),
            reporting_scope: scan_mode_name(mode).to_string(),
            completeness,
            score_model_version: crate::output::score_model_version().to_string(),
            dimensions: Vec::new(),
            workspace_health: None,
            root_causes: Vec::new(),
            projects: Vec::new(),
            diagnostics: Vec::new(),
            summary: ReportSummary {
                score: None,
                score_label: None,
                error_count: 0,
                warning_count: 0,
                info_count: 0,
                diagnostic_count: 0,
                score_authoritative: false,
                score_reasons: Vec::new(),
            },
            elapsed_ms: 0,
            pass_timings_ms: BTreeMap::new(),
            error: Some(ReportError {
                kind: kind.to_string(),
                message: redact_error_message(requested_root, message),
                causes: causes
                    .iter()
                    .map(|cause| redact_error_message(requested_root, cause))
                    .collect(),
                exit_classification: exit_classification(kind).to_string(),
            }),
            audit: AuditMetadata::default(),
            baseline: None,
            score: None,
            score_label: None,
            dimension_scores: None,
            source_file_count: 0,
            elapsed: 0.0,
            skipped_passes: Vec::new(),
            error_count: 0,
            warning_count: 0,
            info_count: 0,
        }
    }
}

/// Descriptive per-package distribution behind the workspace headline.
fn workspace_health(projects: &[ProjectReport]) -> Option<WorkspaceHealth> {
    if projects.len() <= 1 {
        return None;
    }
    let package_scores: Vec<PackageScore> = projects
        .iter()
        .map(|project| PackageScore {
            cargo_package_id: project.cargo_package_id.clone(),
            score: project.score,
            authoritative: project.score_authoritative,
        })
        .collect();
    let mut authoritative: Vec<u32> = package_scores
        .iter()
        .filter(|package| package.authoritative)
        .filter_map(|package| package.score)
        .collect();
    authoritative.sort_unstable();
    let headline_score = authoritative.first().copied();
    let headline_package_id = headline_score.and_then(|lowest| {
        package_scores
            .iter()
            .filter(|package| package.authoritative && package.score == Some(lowest))
            .map(|package| package.cargo_package_id.clone())
            .min()
    });
    let median_score = (!authoritative.is_empty()).then(|| authoritative[authoritative.len() / 2]);
    let non_authoritative_packages = package_scores
        .iter()
        .filter(|package| !package.authoritative)
        .count();
    Some(WorkspaceHealth {
        headline_package_id,
        headline_score,
        authoritative: non_authoritative_packages == 0 && !authoritative.is_empty(),
        minimum_score: headline_score,
        median_score,
        maximum_score: authoritative.last().copied(),
        authoritative_packages: authoritative.len(),
        non_authoritative_packages,
        package_scores,
    })
}

/// Feature profiles the compiling analyzers did not build.
///
/// A scan analyzes the profile Cargo resolves by default. Features outside it
/// are never proven healthy, so they are reported as uncovered scope.
fn unanalyzed_feature_profiles(project: &crate::discovery::ProjectInfo) -> Vec<String> {
    let analyzed: std::collections::BTreeSet<&str> = project
        .enabled_features
        .iter()
        .map(String::as_str)
        .collect();
    let mut declared: Vec<&str> = project
        .declared_features
        .iter()
        .map(String::as_str)
        .filter(|feature| !analyzed.contains(feature))
        .collect();
    declared.sort_unstable();
    declared
        .into_iter()
        .map(|feature| format!("feature:{feature}"))
        .collect()
}

#[expect(
    clippy::too_many_lines,
    reason = "canonical construction keeps identity, policy, location, and fix evidence in one auditable boundary"
)]
fn canonicalize(
    diagnostic: &Diagnostic,
    compiler_evidence: Option<&CompilerDiagnosticEvidence>,
    project: &crate::discovery::ProjectInfo,
    config: &crate::config::ResolvedConfig,
) -> CanonicalDiagnostic {
    let root = &project.root_dir;
    let resolved = crate::catalog::built_in_catalog().ok().map(|catalog| {
        catalog.resolve_with_provenance(
            &diagnostic.rule,
            &diagnostic.category,
            diagnostic.severity,
            compiler_evidence.map(|evidence| evidence.provenance),
        )
    });
    let descriptor = resolved
        .as_ref()
        .map(crate::catalog::ResolvedDescriptor::as_descriptor);
    // A namespace fallback resolves presentation defaults but is not an
    // explicit catalog mapping: its decision metadata stays unfabricated.
    let mapped = resolved
        .as_ref()
        .is_some_and(|value| !crate::catalog::ResolvedDescriptor::is_fallback(value));
    let provider = descriptor.map_or("external", |value| value.provider.as_str());
    let rule = descriptor.map_or(diagnostic.rule.as_str(), |value| {
        value.canonical_id.as_str()
    });
    let category = descriptor.map_or_else(
        || diagnostic.category.clone(),
        |value| value.category.clone(),
    );
    let evidence_path = compiler_evidence
        .and_then(|evidence| evidence.primary_span.as_ref())
        .map_or(&diagnostic.file_path, |span| &span.file_path);
    // A span outside the project root is not a source location a consumer can
    // act on: Clippy attributes package-level lints to the isolated
    // configuration directory, whose path changes on every run. Keeping it
    // would leak an absolute temporary path and make `site_id` and
    // `baseline_key` unstable across identical scans (NFR-005, NFR-007).
    let outside_project =
        root.is_absolute() && evidence_path.is_absolute() && !evidence_path.starts_with(root);
    let normalized_path = if outside_project {
        "<project>".to_string()
    } else {
        normalize_path(root, evidence_path)
    };
    let project_level = (diagnostic.line.is_none()
        && matches!(
            rule,
            "compiler-error" | "compiler-ice" | "unknown-rustc-level"
        ))
        || normalized_path == "<unknown>"
        || outside_project;
    let location = if project_level {
        DiagnosticLocation::Project
    } else if let Some(span) = compiler_evidence.and_then(|evidence| evidence.primary_span.as_ref())
    {
        DiagnosticLocation::Source {
            path: normalize_path(root, &span.file_path),
            range: span.range.clone(),
        }
    } else {
        let line = diagnostic.line.unwrap_or(1);
        let column = diagnostic.column.unwrap_or(1);
        let byte_offset = source_byte_offset(root, &diagnostic.file_path, line, column);
        DiagnosticLocation::Source {
            path: normalized_path.clone(),
            range: SourceRange {
                start: SourcePosition {
                    line,
                    column,
                    byte_offset,
                },
                end: SourcePosition {
                    line,
                    column,
                    byte_offset,
                },
            },
        }
    };
    let mut fixes = compiler_evidence.map_or_else(
        || {
            diagnostic.fix.as_ref().map_or_else(Vec::new, |fix| {
                vec![CanonicalFix {
                    group_id: None,
                    applicability: FixApplicability::MachineApplicable,
                    replacement: fix.new_text.clone(),
                    location: DiagnosticLocation::Source {
                        path: normalized_path.clone(),
                        range: SourceRange {
                            start: SourcePosition {
                                line: fix.line,
                                column: 1,
                                byte_offset: None,
                            },
                            end: SourcePosition {
                                line: fix.line,
                                column: 1,
                                byte_offset: None,
                            },
                        },
                    },
                    eligibility: FixEligibility::GuidanceOnly,
                    precondition_hash: None,
                    ineligible_reason: None,
                }]
            })
        },
        |evidence| {
            evidence
                .fixes
                .iter()
                .map(|fix| CanonicalFix {
                    group_id: fix.group_id.clone(),
                    applicability: fix.applicability,
                    replacement: fix.replacement.clone(),
                    location: DiagnosticLocation::Source {
                        path: normalize_path(root, &fix.span.file_path),
                        range: fix.span.range.clone(),
                    },
                    eligibility: FixEligibility::GuidanceOnly,
                    precondition_hash: None,
                    ineligible_reason: None,
                })
                .collect()
        },
    );
    let macro_generated =
        compiler_evidence.is_some_and(|evidence| evidence.macro_expansion.is_some());
    for fix in &mut fixes {
        decide_fix_eligibility(fix, root, &diagnostic.category, rule, macro_generated);
    }
    let related_locations = compiler_evidence.map_or_else(Vec::new, |evidence| {
        evidence
            .related_locations
            .iter()
            .map(|(message, span)| RelatedLocation {
                message: message.clone(),
                location: DiagnosticLocation::Source {
                    path: normalize_path(root, &span.file_path),
                    range: span.range.clone(),
                },
            })
            .collect()
    });
    let macro_expansion = compiler_evidence
        .and_then(|evidence| evidence.macro_expansion.as_ref())
        .map(|expansion| MacroExpansion {
            macro_name: expansion.macro_name.clone(),
            call_site: DiagnosticLocation::Source {
                path: normalize_path(root, &expansion.call_site.file_path),
                range: expansion.call_site.range.clone(),
            },
        });
    let ownership = diagnostic_ownership(diagnostic, project);
    let source_surface = if macro_expansion.is_some() {
        SourceSurface::MacroExpansion
    } else {
        crate::discovery::source_surface_for_project_path(project, evidence_path)
    };
    let policy = descriptor.map(|value| config.rule_policy(value, Some(&diagnostic.file_path)));
    let demoted_to_audit =
        descriptor.is_some_and(|value| config.demotes_to_audit_only(&value.canonical_id));
    let visible_on = policy.map_or_else(all_surface_names, |value| {
        surface_names(|surface| {
            value.visible_on(surface)
                && !(demoted_to_audit
                    && matches!(
                        surface,
                        crate::config::VisibilitySurface::Score
                            | crate::config::VisibilitySurface::CiFailure
                            | crate::config::VisibilitySurface::PrComment
                    ))
        })
    });
    let effective_trust = descriptor.map(|value| config.effective_trust(value));
    let message = normalize_message(&diagnostic.message);
    let fingerprint_evidence = fingerprint_evidence(root, evidence_path, &location);
    let site_id = sha256_hex(&format!(
        "rust-doctor-site-v1\0{provider}\0{rule}\0{normalized_path}\0{fingerprint_evidence}\0{message}"
    ));
    let baseline_key = sha256_hex(&format!(
        "rust-doctor-baseline-v1\0{provider}\0{rule}\0{message}\0{fingerprint_evidence}"
    ));

    CanonicalDiagnostic {
        advisory: descriptor.is_some_and(|value| value.advisory),
        provider: provider.to_string(),
        rule: rule.to_string(),
        title: title_from_rule(rule),
        category,
        severity: diagnostic.severity,
        message: diagnostic.message.clone(),
        help: diagnostic.help.clone(),
        url: descriptor.map_or_else(
            || "https://rust-doctor.vercel.app/rules/external".to_string(),
            |value| value.documentation_url.clone(),
        ),
        tags: descriptor.map_or_else(Vec::new, |value| value.tags.clone()),
        analysis_kind: descriptor
            .map_or("external", |value| match value.analyzer_kind {
                crate::catalog::AnalyzerKind::SynAst => "syn_ast",
                crate::catalog::AnalyzerKind::Clippy => "clippy",
                crate::catalog::AnalyzerKind::Dependency => "dependency",
                crate::catalog::AnalyzerKind::Project => "project",
                crate::catalog::AnalyzerKind::External => "external",
            })
            .to_string(),
        confidence: descriptor
            .map_or("unknown", |value| match value.confidence {
                crate::catalog::Confidence::Low => "low",
                crate::catalog::Confidence::Medium => "medium",
                crate::catalog::Confidence::High => "high",
            })
            .to_string(),
        original_level: compiler_evidence.map_or_else(
            || diagnostic.severity.to_string(),
            |evidence| evidence.original_level.clone(),
        ),
        ownership,
        source_surface,
        location,
        related_locations,
        macro_expansion,
        fix_recipe: fix_recipe(descriptor, mapped, &fixes),
        fixes,
        visible_on,
        site_id,
        baseline_key,
        namespace_fallback: !mapped,
        priority: effective_trust
            .as_ref()
            .filter(|_| mapped)
            .and_then(|value| value.priority)
            .map(|priority| priority.as_str().to_string()),
        trust_tier: effective_trust
            .as_ref()
            .filter(|_| mapped)
            .map_or("unknown", |value| value.tier.as_str())
            .to_string(),
        score_eligible: mapped
            && effective_trust
                .as_ref()
                .is_some_and(|value| value.score_eligible),
        score_impact: score_impact(descriptor, effective_trust.as_ref(), mapped),
        aggregation_policy: effective_trust
            .as_ref()
            .filter(|_| mapped)
            .map_or("unmapped", |value| value.aggregation.as_str())
            .to_string(),
        root_cause_key: root_cause_key(descriptor.filter(|_| mapped), diagnostic, rule),
        evidence_summary: evidence_summary(descriptor, mapped),
        limitations: descriptor.map_or_else(Vec::new, |value| {
            crate::catalog::rule_limitations(value, !mapped)
        }),
        suppressed: false,
    }
}

/// What one occurrence did to the Core Score.
///
/// Severity and confidence are deliberately absent: only score eligibility,
/// analyzer availability, and the recorded advisory decision decide impact.
fn score_impact(
    descriptor: Option<&crate::catalog::RuleDescriptor>,
    trust: Option<&crate::catalog::RuleTrust>,
    mapped: bool,
) -> ScoreImpact {
    if !mapped {
        return ScoreImpact::Ineligible;
    }
    descriptor
        .zip(trust)
        .map_or(ScoreImpact::Ineligible, |(value, trust)| {
            if trust.contributes_to_core_score() {
                ScoreImpact::Scored
            } else if value.advisory || value.default_enabled {
                ScoreImpact::Advisory
            } else {
                ScoreImpact::Ineligible
            }
        })
}

/// Stable identity of the defect behind one or more occurrences.
///
/// Advisory identifiers, then the canonical rule ID. Message text is never
/// hashed into the key: it would split one root cause across wording changes.
fn root_cause_key(
    descriptor: Option<&crate::catalog::RuleDescriptor>,
    diagnostic: &Diagnostic,
    rule: &str,
) -> Option<String> {
    // An unmapped rule gets no fabricated root cause (US-014 AC-5).
    descriptor?;
    crate::passes::adapter::advisory_identity(rule)
        .or_else(|| crate::passes::adapter::advisory_identity(&diagnostic.message))
        .map_or_else(
            || Some(format!("rule:{rule}")),
            |advisory| Some(format!("advisory:{advisory}")),
        )
}

fn evidence_summary(descriptor: Option<&crate::catalog::RuleDescriptor>, mapped: bool) -> String {
    descriptor.filter(|_| mapped).map_or_else(
        || "No declared evidence contract: this rule has no explicit catalog mapping.".to_string(),
        |value| {
            let model = crate::catalog::evidence_model(value.analyzer_kind);
            if value.trust.required_evidence.is_empty() {
                model.to_string()
            } else {
                let required: Vec<_> = value
                    .trust
                    .required_evidence
                    .iter()
                    .map(|evidence| evidence.as_str())
                    .collect();
                format!("{model} Required evidence: {}.", required.join(", "))
            }
        },
    )
}

/// Rule families whose remediation is a policy decision, never an edit.
///
/// A change to the public API, unsafe code, Cargo features, MSRV, dependency
/// policy, or security hardening has consequences a span-local edit cannot
/// reason about, so it stays guidance regardless of how confident the
/// suggestion looks (US-016 AC-6).
fn policy_sensitive_reason(category: &Category, rule: &str) -> Option<&'static str> {
    if matches!(
        category,
        Category::Security | Category::Dependencies | Category::Cargo
    ) {
        return Some(
            "security, dependency, and Cargo policy changes are decisions, not mechanical edits",
        );
    }
    let sensitive = [
        ("unsafe", "unsafe code requires human review"),
        ("public-api", "a public API change is a semver decision"),
        ("msrv", "an MSRV change affects every consumer"),
        ("feature", "a Cargo feature change alters the build profile"),
    ];
    sensitive
        .into_iter()
        .find_map(|(needle, reason)| rule.contains(needle).then_some(reason))
}

/// Decide whether one suggestion may be applied automatically.
///
/// Every requirement is checked explicitly: rustc applicability, an exact
/// byte-addressed span, a readable UTF-8 target inside the project, a
/// precondition hash of that file, non-macro provenance, and a rule family
/// whose remediation is mechanical (US-016 AC-1, AC-3, AC-6).
pub(crate) fn decide_fix_eligibility(
    fix: &mut CanonicalFix,
    root: &std::path::Path,
    category: &Category,
    rule: &str,
    macro_generated: bool,
) {
    let reason = fix_ineligible_reason(fix, root, category, rule, macro_generated);
    if let Some(reason) = reason {
        fix.eligibility = FixEligibility::GuidanceOnly;
        fix.ineligible_reason = Some(reason.to_string());
    } else {
        fix.eligibility = FixEligibility::MachineApplicable;
        fix.ineligible_reason = None;
    }
}

fn fix_ineligible_reason(
    fix: &mut CanonicalFix,
    root: &std::path::Path,
    category: &Category,
    rule: &str,
    macro_generated: bool,
) -> Option<&'static str> {
    if fix.applicability != FixApplicability::MachineApplicable {
        return Some("the analyzer did not mark this suggestion machine-applicable");
    }
    if macro_generated {
        return Some("the span points into macro-generated code");
    }
    if let Some(reason) = policy_sensitive_reason(category, rule) {
        return Some(reason);
    }
    let DiagnosticLocation::Source { path, range } = &fix.location else {
        return Some("a project-level finding has no span to edit");
    };
    let (Some(start), Some(end)) = (range.start.byte_offset, range.end.byte_offset) else {
        return Some("the exact byte span is unavailable");
    };
    if end < start {
        return Some("the reported span is inverted");
    }
    let absolute = root.join(path);
    let Ok(source) = std::fs::read(&absolute) else {
        return Some("the target file could not be read as analyzed");
    };
    if std::str::from_utf8(&source).is_err() {
        return Some("the target file is not valid UTF-8");
    }
    if end as usize > source.len() {
        return Some("the span falls outside the analyzed file");
    }
    fix.precondition_hash = Some(sha256_hex_bytes(&source));
    None
}

/// Fix recipe identifier, present only when a validated recipe exists.
///
/// A rule with guidance-only remediation returns `None` so that `help` stays
/// advice instead of an implied machine-applicable edit (US-014 AC-4).
fn fix_recipe(
    descriptor: Option<&crate::catalog::RuleDescriptor>,
    mapped: bool,
    fixes: &[CanonicalFix],
) -> Option<String> {
    use crate::catalog::FixCapability;

    if !mapped {
        return None;
    }
    let descriptor = descriptor?;
    let capability = match descriptor.fix_capability {
        FixCapability::MachineApplicable => "machine-applicable",
        FixCapability::RustcSuggestion => "rustc-suggestion",
        FixCapability::None | FixCapability::Guidance => return None,
    };
    // Eligibility, not raw applicability: a suggestion that Rust Doctor refuses
    // to apply must not advertise a machine-applicable recipe (US-014 AC-4).
    fixes
        .iter()
        .any(|fix| fix.eligibility == FixEligibility::MachineApplicable)
        .then(|| format!("{}/{capability}", descriptor.canonical_id))
}

fn normalize_report_files(root: &std::path::Path, paths: &[PathBuf]) -> Vec<String> {
    let mut files: Vec<_> = paths
        .iter()
        .map(|path| normalize_path(root, path))
        .collect();
    files.sort();
    files.dedup();
    files
}

type ProjectReportMember<'a> = (
    String,
    &'a std::path::Path,
    Vec<String>,
    Vec<String>,
    Vec<FrameworkCapabilityReport>,
);

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "package reports combine scan execution, Cargo metadata, completeness, and canonical diagnostics in one projection"
)]
fn project_reports(
    project: &crate::discovery::ProjectInfo,
    result: &ScanResult,
    diagnostics: &[CanonicalDiagnostic],
    all_planned_files: &[String],
    all_analyzed_files: &[String],
    checks: &[CheckState],
    completeness: &ReportCompleteness,
    score: Option<u32>,
    report_dimensions: &[DimensionCoverage],
) -> Vec<ProjectReport> {
    if result.execution.packages.is_empty() && all_planned_files.is_empty() {
        return Vec::new();
    }
    if !result.execution.packages.is_empty() {
        let package_roots: Vec<String> = result
            .execution
            .packages
            .iter()
            .map(|execution| normalize_path(&project.root_dir, &execution.package_root))
            .collect();
        return result
            .execution
            .packages
            .iter()
            .enumerate()
            .map(|(package_index, execution)| {
                let member = project.workspace_members.iter().find(|member| {
                    member.package_id == execution.cargo_package_id
                        || member.root_dir == execution.package_root
                });
                let package_root = package_roots[package_index].clone();
                let planned_files =
                    normalize_report_files(&project.root_dir, &execution.planned_files);
                let analyzed_files =
                    normalize_report_files(&project.root_dir, &execution.analyzed_files);
                let package_receipts =
                    crate::completeness::receipts_for_package(result, &execution.cargo_package_id);
                let package_checks: Vec<_> = package_receipts
                    .iter()
                    .map(crate::completeness::AnalyzerReceipt::to_check_state)
                    .collect();
                let completeness = crate::completeness::compute_from_parts(
                    planned_files.len(),
                    analyzed_files.len(),
                    &package_checks,
                    !planned_files.is_empty() || !package_checks.is_empty(),
                    false,
                );
                let package_diagnostics: Vec<_> = diagnostics
                    .iter()
                    .filter(|diagnostic| match &diagnostic.ownership {
                        DiagnosticOwnership::Package { package_id } => {
                            package_id == &execution.cargo_package_id
                        }
                        DiagnosticOwnership::Workspace => true,
                        DiagnosticOwnership::Unowned => false,
                    })
                    .cloned()
                    .collect();
                let package_dimensions = crate::completeness::dimension_coverage(
                    &package_receipts,
                    &result.dimension_scores,
                    std::slice::from_ref(&package_root),
                    &[],
                );
                let score_authoritative = completeness.score_authoritative
                    && crate::completeness::dimensions_are_authoritative(&package_dimensions);
                ProjectReport {
                    cargo_package_id: execution.cargo_package_id.clone(),
                    package_root,
                    targets: member
                        .map_or_else(|| project.targets.clone(), |member| member.targets.clone()),
                    framework_capabilities: member.map_or_else(
                        || {
                            project
                                .frameworks
                                .iter()
                                .map(std::string::ToString::to_string)
                                .collect()
                        },
                        |member| {
                            member
                                .frameworks
                                .iter()
                                .map(std::string::ToString::to_string)
                                .collect()
                        },
                    ),
                    framework_gates: member.map_or_else(
                        || framework_capability_reports(&project.framework_capabilities),
                        |member| framework_capability_reports(&member.framework_capabilities),
                    ),
                    planned_files,
                    analyzed_files,
                    checks: package_checks.clone(),
                    skipped_reasons: package_checks
                        .iter()
                        .filter(|check| check.status != CheckStatus::Completed)
                        .filter_map(|check| check.reason.clone())
                        .collect(),
                    score: project_score_for_report(execution.score, score_authoritative),
                    score_authoritative,
                    dimensions: package_dimensions,
                    completeness,
                    elapsed_ms: duration_millis(execution.elapsed),
                    diagnostics: package_diagnostics,
                }
            })
            .collect();
    }

    let project_frameworks: Vec<String> = project
        .frameworks
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    let members: Vec<ProjectReportMember<'_>> = if project.workspace_members.is_empty() {
        vec![(
            project.package_id.clone(),
            project.root_dir.as_path(),
            project.targets.clone(),
            project_frameworks,
            framework_capability_reports(&project.framework_capabilities),
        )]
    } else {
        project
            .workspace_members
            .iter()
            .map(|member| {
                (
                    member.package_id.clone(),
                    member.root_dir.as_path(),
                    member.targets.clone(),
                    member
                        .frameworks
                        .iter()
                        .map(std::string::ToString::to_string)
                        .collect(),
                    framework_capability_reports(&member.framework_capabilities),
                )
            })
            .collect()
    };

    members
        .into_iter()
        .map(
            |(package_id, root, targets, framework_capabilities, framework_gates)| {
                let normalized_root = normalize_path(&project.root_dir, root);
                let prefix = if normalized_root == "." {
                    String::new()
                } else {
                    format!("{normalized_root}/")
                };
                let planned_files: Vec<String> = all_planned_files
                    .iter()
                    .filter(|path| prefix.is_empty() || path.starts_with(&prefix))
                    .cloned()
                    .collect();
                let analyzed_files: Vec<String> = all_analyzed_files
                    .iter()
                    .filter(|path| prefix.is_empty() || path.starts_with(&prefix))
                    .cloned()
                    .collect();
                ProjectReport {
                    cargo_package_id: package_id.clone(),
                    package_root: normalized_root,
                    targets,
                    framework_capabilities,
                    framework_gates,
                    analyzed_files,
                    planned_files,
                    checks: checks.to_vec(),
                    skipped_reasons: checks
                        .iter()
                        .filter(|check| check.status != CheckStatus::Completed)
                        .filter_map(|check| check.reason.clone())
                        .collect(),
                    completeness: completeness.clone(),
                    score,
                    score_authoritative: completeness.score_authoritative
                        && crate::completeness::dimensions_are_authoritative(report_dimensions),
                    dimensions: report_dimensions.to_vec(),
                    elapsed_ms: duration_millis(result.elapsed),
                    diagnostics: diagnostics
                        .iter()
                        .filter(|diagnostic| match &diagnostic.ownership {
                            DiagnosticOwnership::Package { package_id: owner } => {
                                owner == &package_id
                            }
                            DiagnosticOwnership::Workspace => true,
                            DiagnosticOwnership::Unowned => false,
                        })
                        .cloned()
                        .collect(),
                }
            },
        )
        .collect()
}

const fn project_score_for_report(score: Option<u32>, score_authoritative: bool) -> Option<u32> {
    if score_authoritative { score } else { None }
}

fn framework_capability_reports(
    capabilities: &[crate::discovery::FrameworkCapability],
) -> Vec<FrameworkCapabilityReport> {
    let rules = crate::rules::all_custom_rules();
    capabilities
        .iter()
        .map(|capability| {
            let framework = capability.framework.to_string();
            let rule_gates = rules
                .iter()
                .filter(|rule| {
                    rule.applicable_frameworks()
                        .first()
                        .is_some_and(|candidate| *candidate == framework)
                })
                .map(|rule| {
                    let gate = crate::rules::framework_packs::capability_decision(
                        rule.as_ref(),
                        capabilities,
                    );
                    FrameworkRuleGateReport {
                        rule: rule.name().to_string(),
                        active: gate.is_ok(),
                        gate_reason: gate.err(),
                    }
                })
                .collect();
            FrameworkCapabilityReport {
                framework,
                version: capability.version.clone(),
                enabled_features: capability.enabled_features.clone(),
                target_contexts: capability.target_contexts.clone(),
                analyzed_target: capability.analyzed_target.clone(),
                active: capability.active,
                gate_reason: capability.gate_reason.clone(),
                rule_gates,
            }
        })
        .collect()
}

fn diagnostic_ownership(
    diagnostic: &Diagnostic,
    project: &crate::discovery::ProjectInfo,
) -> DiagnosticOwnership {
    if diagnostic
        .file_path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
        || (diagnostic.file_path.has_root() && !diagnostic.file_path.starts_with(&project.root_dir))
    {
        return DiagnosticOwnership::Unowned;
    }
    let normalized = normalize_path(&project.root_dir, &diagnostic.file_path);
    if normalized == "<unknown>" {
        return DiagnosticOwnership::Unowned;
    }
    let virtual_workspace_manifest = normalized == "Cargo.toml"
        && project.is_workspace
        && !project
            .workspace_members
            .iter()
            .any(|member| member.root_dir == project.root_dir);
    if matches!(
        normalized.as_str(),
        "Cargo.lock" | "rust-doctor.toml" | "rust-toolchain" | "rust-toolchain.toml"
    ) || virtual_workspace_manifest
    {
        return DiagnosticOwnership::Workspace;
    }
    project
        .workspace_members
        .iter()
        .filter_map(|member| {
            let member_root = normalize_path(&project.root_dir, &member.root_dir);
            let matches = member_root == "."
                || normalized == member_root
                || normalized.starts_with(&format!("{member_root}/"));
            matches.then_some((member_root.matches('/').count(), member))
        })
        .max_by_key(|(depth, _)| *depth)
        .map_or_else(
            || {
                if project.workspace_members.is_empty() {
                    DiagnosticOwnership::Package {
                        package_id: project.package_id.clone(),
                    }
                } else {
                    DiagnosticOwnership::Unowned
                }
            },
            |(_, member)| DiagnosticOwnership::Package {
                package_id: member.package_id.clone(),
            },
        )
}

fn redact_error_message(requested_root: &std::path::Path, message: &str) -> String {
    let root = display_path(requested_root);
    let redacted = if root.is_empty() {
        message.to_string()
    } else {
        message.replace(&root, "<requested-root>")
    };
    std::env::var("HOME").map_or_else(
        |_| redacted.clone(),
        |home| redacted.replace(&home.replace('\\', "/"), "$HOME"),
    )
}

fn exit_classification(kind: &str) -> &'static str {
    match kind {
        "config" | "bootstrap" | "discovery" | "scan" => "scan_error",
        "gate" => "quality_gate",
        "incomplete" => "incomplete_analysis",
        _ => "internal_error",
    }
}

const fn scan_mode_name(mode: ScanMode) -> &'static str {
    match mode {
        ScanMode::Full => "full",
        ScanMode::Files => "files",
        ScanMode::Lines => "lines",
        ScanMode::Staged => "staged",
        ScanMode::Baseline => "baseline",
    }
}

fn normalize_path(root: &std::path::Path, path: &std::path::Path) -> String {
    let root = root.to_string_lossy().replace('\\', "/");
    let mut value = path.to_string_lossy().replace('\\', "/");
    if value == root {
        return ".".to_string();
    }
    if let Some(relative) = value.strip_prefix(&format!("{root}/")) {
        value = relative.to_string();
    }
    while value.starts_with("./") {
        value = value[2..].to_string();
    }
    value
}

fn display_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn source_byte_offset(
    root: &std::path::Path,
    diagnostic_path: &std::path::Path,
    line: u32,
    column: u32,
) -> Option<u32> {
    if diagnostic_path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    let path = if diagnostic_path.is_absolute() {
        diagnostic_path.to_path_buf()
    } else {
        root.join(diagnostic_path)
    };
    if path.is_absolute() && root.is_absolute() && !path.starts_with(root) {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    let line_index = usize::try_from(line.checked_sub(1)?).ok()?;
    let column_index = usize::try_from(column.checked_sub(1)?).ok()?;
    let mut offset = 0usize;
    let source_line = content.split_inclusive('\n').nth(line_index)?;
    for previous in content.split_inclusive('\n').take(line_index) {
        offset = offset.checked_add(previous.len())?;
    }
    let content_without_newline = source_line.trim_end_matches(['\r', '\n']);
    let column_offset = content_without_newline
        .char_indices()
        .nth(column_index)
        .map_or(content_without_newline.len(), |(index, _)| index);
    u32::try_from(offset.checked_add(column_offset)?).ok()
}

fn normalize_message(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Line-independent source anchor used by SARIF and baseline fingerprints.
///
/// The normalized source line plus its same-line ordinal stays stable when
/// unrelated lines are inserted before unchanged code. Falling back to the
/// reported range keeps project-level and unavailable-source diagnostics
/// deterministic without reading outside the scan root.
fn fingerprint_evidence(
    root: &std::path::Path,
    evidence_path: &std::path::Path,
    location: &DiagnosticLocation,
) -> String {
    let DiagnosticLocation::Source { range, .. } = location else {
        return "project".to_string();
    };
    let path = if evidence_path.is_absolute() {
        evidence_path.to_path_buf()
    } else {
        root.join(evidence_path)
    };
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
        || (path.is_absolute() && root.is_absolute() && !path.starts_with(root))
    {
        return range_evidence(range);
    }
    let Some(line_index) = range
        .start
        .line
        .checked_sub(1)
        .and_then(|value| usize::try_from(value).ok())
    else {
        return range_evidence(range);
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return range_evidence(range);
    };
    let normalized_lines: Vec<String> = content.lines().map(normalize_message).collect();
    let Some(source_line) = normalized_lines.get(line_index) else {
        return range_evidence(range);
    };
    let ordinal = normalized_lines
        .iter()
        .take(line_index + 1)
        .filter(|candidate| *candidate == source_line)
        .count();
    format!("source-line:{source_line}\0ordinal:{ordinal}")
}

fn range_evidence(range: &SourceRange) -> String {
    format!(
        "range:{}:{}:{}:{}",
        range.start.line, range.start.column, range.end.line, range.end.column
    )
}

fn sha256_hex(input: &str) -> String {
    sha256_hex_bytes(input.as_bytes())
}

pub(crate) fn sha256_hex_bytes(input: &[u8]) -> String {
    let digest = Sha256::digest(input);
    let mut output = String::with_capacity(digest.len() * 2);
    use std::fmt::Write;
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn title_from_rule(rule: &str) -> String {
    let name = rule
        .rsplit("::")
        .next()
        .unwrap_or(rule)
        .replace(['-', '_'], " ");
    let mut chars = name.chars();
    chars.next().map_or_else(
        || name.clone(),
        |first| first.to_uppercase().collect::<String>() + chars.as_str(),
    )
}

fn surface_names(predicate: impl Fn(crate::config::VisibilitySurface) -> bool) -> Vec<String> {
    use crate::config::VisibilitySurface;
    [
        (VisibilitySurface::Terminal, "terminal"),
        (VisibilitySurface::Score, "score"),
        (VisibilitySurface::CiFailure, "ci-failure"),
        (VisibilitySurface::PrComment, "pr-comment"),
        (VisibilitySurface::Sarif, "sarif"),
        (VisibilitySurface::Mcp, "mcp"),
    ]
    .into_iter()
    .filter(|(surface, _)| predicate(*surface))
    .map(|(_, name)| name.to_string())
    .collect()
}

fn all_surface_names() -> Vec<String> {
    surface_names(|_| true)
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn serialize_duration<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_f64(duration.as_secs_f64())
}

fn serialize_pass_timings<S>(
    timings: &[(String, Duration)],
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(timings.len()))?;
    for (name, duration) in timings {
        seq.serialize_element(&serde_json::json!({
            "pass": name,
            "elapsed_secs": duration.as_secs_f64()
        }))?;
    }
    seq.end()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_display() {
        assert_eq!(Severity::Error.to_string(), "error");
        assert_eq!(Severity::Warning.to_string(), "warning");
        assert_eq!(Severity::Info.to_string(), "info");
    }

    #[test]
    fn test_category_display() {
        assert_eq!(Category::ErrorHandling.to_string(), "Error Handling");
        assert_eq!(Category::Performance.to_string(), "Performance");
        assert_eq!(Category::Security.to_string(), "Security");
    }

    #[test]
    fn test_diagnostic_serialize() {
        let diag = Diagnostic {
            file_path: PathBuf::from("src/main.rs"),
            rule: "unwrap-in-production".to_string(),
            category: Category::ErrorHandling,
            severity: Severity::Warning,
            message: "Use of .unwrap() in production code".to_string(),
            help: Some("Use ? operator or handle the error explicitly".to_string()),
            line: Some(42),
            column: Some(10),
            fix: None,
        };
        let json = serde_json::to_value(&diag).unwrap();
        assert_eq!(json["rule"], "unwrap-in-production");
        assert_eq!(json["severity"], "warning");
        assert_eq!(json["category"], "error-handling");
        assert_eq!(json["line"], 42);
    }

    #[test]
    fn test_diagnostic_serialize_no_optionals() {
        let diag = Diagnostic {
            file_path: PathBuf::from("Cargo.toml"),
            rule: "unused-dependency".to_string(),
            category: Category::Dependencies,
            severity: Severity::Warning,
            message: "Unused dependency: serde".to_string(),
            help: None,
            line: None,
            column: None,
            fix: None,
        };
        let json = serde_json::to_value(&diag).unwrap();
        assert!(json.get("help").is_none());
        assert!(json.get("line").is_none());
        assert!(json.get("column").is_none());
    }

    #[test]
    fn test_scan_result_serialize() {
        let result = ScanResult {
            diagnostics: vec![],
            score: 100,
            score_label: ScoreLabel::Great,
            dimension_scores: DimensionScores {
                security: 100,
                reliability: 100,
                maintainability: 100,
                performance: 100,
                dependencies: 100,
            },
            source_file_count: 10,
            elapsed: Duration::from_millis(1500),
            skipped_passes: vec![],
            error_count: 0,
            warning_count: 0,
            info_count: 0,
            pass_timings: vec![
                ("clippy".to_string(), Duration::from_millis(800)),
                ("custom rules".to_string(), Duration::from_millis(200)),
            ],
            suppressed_security: vec![],
            planned_files: vec![PathBuf::from("src/lib.rs")],
            analyzed_files: vec![PathBuf::from("src/lib.rs")],
            compiler_evidence: vec![],
            execution: ScanExecution::default(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["score"], 100);
        assert_eq!(json["score_label"], "Great");
        assert_eq!(json["source_file_count"], 10);
        assert_eq!(json["elapsed"], 1.5);
        assert_eq!(json["error_count"], 0);
        // Verify pass_timings serialization
        let timings = json["pass_timings"].as_array().unwrap();
        assert_eq!(timings.len(), 2);
        assert_eq!(timings[0]["pass"], "clippy");
        assert!((timings[0]["elapsed_secs"].as_f64().unwrap() - 0.8).abs() < 0.001);
        assert_eq!(timings[1]["pass"], "custom rules");
    }

    fn test_completeness(score_authoritative: bool) -> ReportCompleteness {
        ReportCompleteness {
            state: if score_authoritative {
                CompletenessState::Complete
            } else {
                CompletenessState::Incomplete
            },
            planned_files: 1,
            analyzed_files: usize::from(score_authoritative),
            completed_checks: usize::from(score_authoritative),
            skipped_checks: 0,
            failed_checks: usize::from(!score_authoritative),
            timed_out_checks: 0,
            cancelled_checks: 0,
            required_checks: 1,
            required_completed_checks: usize::from(score_authoritative),
            score_authoritative,
            abstentions: 0,
        }
    }

    fn test_project_report(score: Option<u32>, score_authoritative: bool) -> ProjectReport {
        ProjectReport {
            cargo_package_id: "fixture".to_string(),
            package_root: "fixture".to_string(),
            targets: Vec::new(),
            framework_capabilities: Vec::new(),
            framework_gates: Vec::new(),
            planned_files: vec!["src/lib.rs".to_string()],
            analyzed_files: Vec::new(),
            checks: Vec::new(),
            skipped_reasons: Vec::new(),
            completeness: test_completeness(score_authoritative),
            score,
            score_authoritative,
            dimensions: Vec::new(),
            elapsed_ms: 0,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn workspace_health_reports_the_lowest_authoritative_package() {
        let projects = vec![
            test_project_report(Some(90), true),
            test_project_report(Some(62), true),
            test_project_report(None, false),
        ];
        let health = workspace_health(&projects).expect("multi-package workspace");
        assert_eq!(health.headline_score, Some(62));
        assert_eq!(health.minimum_score, Some(62));
        assert_eq!(health.maximum_score, Some(90));
        assert_eq!(health.authoritative_packages, 2);
        assert_eq!(health.non_authoritative_packages, 1);
        assert!(!health.authoritative);
    }

    #[test]
    fn non_authoritative_project_score_is_always_null() {
        assert_eq!(project_score_for_report(Some(42), false), None);
        assert_eq!(project_score_for_report(Some(42), true), Some(42));
    }
}
