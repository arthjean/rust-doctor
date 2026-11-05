use std::collections::BTreeSet;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::ser::Error as _;
use serde::{Deserialize, Serialize, Serializer};

use crate::audit::{Audit, SeverityCounts};
use crate::cargo_health::{AdvisoryDatabase, AdvisoryDbReport};
use crate::delta::DeltaReport;
use crate::execution::CacheOutcome;
use crate::execution::{RunDeadline, RunOptions};
use crate::progress::ProgressSink;
use crate::timings::{TimingRecorder, Timings};
use crate::git_scope::{ChangeMode, ChangeOptions, ScopeMode, ScopeReport, ScopeRequest};
use crate::policy::{
    BlockingLevel, BlockingLevelSource, CategoryOverride, CorpusMeasurement, PathOverrideReport,
    PolicyInput, PolicyPlan, RuleLevel, RuleLevelSource, RuleOverride, RuleTier,
};

mod assembly;
mod exclusion;
pub(crate) use exclusion::edit_distance;
mod normalize;
pub(crate) use normalize::BINARY_CHANNEL_RULES;
mod sanitize;
pub(crate) use sanitize::{home_directory, replace_path};

pub(crate) use assembly::{
    baseline_report_failure, from_baseline_execution, from_execution_scoped,
    interrupted_failure, policy_failure, preparation_failure, scope_failure,
};

pub const SCHEMA_VERSION: u8 = 24;

pub(crate) const DEADLINE_EXCEEDED: &str = "deadline-exceeded";

#[derive(Debug, Clone)]
pub struct InspectRequest {
    pub path: PathBuf,
    policy: PolicyInput,
    scope_mode: ScopeMode,
    base: Option<CallerBase>,
    change_options: ChangeOptions,
    max_duration: Option<Duration>,
    progress: Option<ProgressSink>,
    packages: BTreeSet<String>,
    advisory_db: Option<AdvisoryDatabase>,
    timings: bool,
}

impl InspectRequest {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            policy: PolicyInput::default(),
            scope_mode: ScopeMode::Full,
            base: None,
            change_options: ChangeOptions::default(),
            max_duration: None,
            progress: None,
            packages: BTreeSet::new(),
            advisory_db: None,
            timings: false,
        }
    }

    pub const fn with_timings(mut self) -> Self {
        self.timings = true;
        self
    }

    pub fn with_advisory_db(mut self, database: AdvisoryDatabase) -> Self {
        self.advisory_db = Some(database);
        self
    }

    pub fn with_package(mut self, name: impl Into<String>) -> Self {
        self.packages.insert(name.into());
        self
    }

    pub(crate) fn packages(&self) -> Option<&BTreeSet<String>> {
        (!self.packages.is_empty()).then_some(&self.packages)
    }

    pub const fn with_max_duration(mut self, limit: Duration) -> Self {
        self.max_duration = Some(limit);
        self
    }

    pub fn with_progress(mut self, progress: ProgressSink) -> Self {
        self.progress = Some(progress);
        self
    }

    pub fn with_rule_override(mut self, rule_override: RuleOverride) -> Self {
        self.policy.push_rule(rule_override);
        self
    }

    pub fn with_category_override(mut self, category_override: CategoryOverride) -> Self {
        self.policy.push_category(category_override);
        self
    }

    pub fn with_blocking(mut self, blocking: BlockingLevel) -> Self {
        self.policy = self.policy.with_blocking(blocking);
        self
    }

    pub fn with_files_scope(self, base: impl Into<String>) -> Self {
        self.with_scope(ScopeMode::Files, Some(base.into()))
    }

    pub fn with_baseline_scope(self, base: impl Into<String>) -> Self {
        self.with_scope(ScopeMode::Baseline, Some(base.into()))
    }

    pub fn with_scope(mut self, mode: ScopeMode, base: Option<String>) -> Self {
        self.scope_mode = mode;
        self.base = base.map(CallerBase);
        self
    }

    pub const fn with_staged(mut self) -> Self {
        self.change_options.staged = true;
        self
    }

    pub const fn with_untracked(mut self) -> Self {
        self.change_options.include_untracked = true;
        self
    }

    pub(crate) const fn policy(&self) -> &PolicyInput {
        &self.policy
    }

    pub(crate) fn scope(&self) -> ScopeRequest {
        let mode = match self.scope_mode {
            ScopeMode::Full => return ScopeRequest::Full,
            ScopeMode::Files => ChangeMode::Files,
            ScopeMode::Lines => ChangeMode::Lines,
            ScopeMode::Contextual => ChangeMode::Contextual,
            ScopeMode::Baseline => ChangeMode::Baseline,
        };
        ScopeRequest::Changed {
            mode,
            base: self.base.as_ref().map(|base| base.0.clone()),
            options: self.change_options,
        }
    }

    pub(crate) fn run_options(&self) -> RunOptions {
        RunOptions {
            deadline: self.max_duration.map(RunDeadline::starting_now),
            progress: self.progress.clone(),
            packages: self.packages().cloned(),
            advisory_db: self.advisory_db.clone(),
            timings: self.timings.then(TimingRecorder::start),
        }
    }
}

#[derive(Clone)]
struct CallerBase(String);

impl fmt::Debug for CallerBase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

impl Default for InspectRequest {
    fn default() -> Self {
        Self::new(".")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectReport {
    pub schema_version: u8,
    pub audit: Audit,
    pub status: Status,
    pub complete: bool,
    pub policy: Option<PolicyReport>,
    pub scope: Option<ScopeReport>,
    pub project: Option<ProjectReport>,
    pub toolchain: ToolchainReport,
    pub scan: ScanReport,
    pub diagnostics: Vec<Diagnostic>,
    pub suppressions: Vec<SuppressionReport>,
    pub delta: Option<DeltaReport>,
    pub errors: Vec<ReportError>,
    pub summary: Summary,
    pub gate: GateReport,
    pub timings: Option<Timings>,
    pub base_side_cache: Option<CacheOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuppressionReport {
    pub path: String,
    pub line: usize,
    pub rules: Vec<String>,
    pub status: SuppressionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SuppressionStatus {
    Applied,
    Unused,
    MissingReason,
    UseExpect,
    UnknownRule,
}

impl SuppressionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Unused => "unused",
            Self::MissingReason => "missing-reason",
            Self::UseExpect => "use-expect",
            Self::UnknownRule => "unknown-rule",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyReport {
    pub config_file: Option<String>,
    pub blocking: PolicyBlockingReport,
    pub rules: Vec<PolicyRuleReport>,
    pub ignore: Vec<String>,
    pub overrides: Vec<PathOverrideReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PolicyBlockingReport {
    pub level: BlockingLevel,
    pub source: BlockingLevelSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyRuleReport {
    pub id: String,
    pub category: String,
    pub tier: RuleTier,
    pub level: RuleLevel,
    pub source: RuleLevelSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corpus_noise_basis_points: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corpus_reviewed_sites: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_evaluated: Option<NotEvaluated>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotEvaluated {
    UnknownToToolchain,
    PackageScoped,
    AdvisoryDbMissing,
    CrateAbsent,
    CrateVersionUnsupported,
}

impl PolicyReport {
    fn from_plan(plan: &PolicyPlan, not_evaluated: &[(&str, NotEvaluated)]) -> Self {
        Self {
            config_file: plan.config_file().map(str::to_owned),
            blocking: PolicyBlockingReport {
                level: plan.blocking(),
                source: plan.blocking_source(),
            },
            rules: plan
                .effective_rules()
                .map(|(definition, level, source)| {
                    let measurement = crate::policy::corpus_measurement(definition.id);
                    PolicyRuleReport {
                        id: definition.id.to_owned(),
                        category: definition.category.to_owned(),
                        tier: definition.tier,
                        level,
                        source,
                        corpus_noise_basis_points: measurement
                            .map(CorpusMeasurement::noise_basis_points),
                        corpus_reviewed_sites: measurement.map(CorpusMeasurement::reviewed),
                        not_evaluated: not_evaluated
                            .iter()
                            .find(|(id, _)| *id == definition.id)
                            .map(|(_, reason)| *reason),
                    }
                })
                .collect(),
            ignore: plan.paths().ignore_report(),
            overrides: plan.paths().overrides_report(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Complete,
    Incomplete,
    Failed,
}

impl Status {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
            Self::Failed => "failed",
        }
    }
}

impl InspectReport {
    pub fn is_valid(&self) -> bool {
        if self.schema_version != SCHEMA_VERSION || !self.audit.is_valid() {
            return false;
        }
        if self.summary != Summary::from_diagnostics(&self.diagnostics) {
            return false;
        }
        let Some(delta) = &self.delta else {
            let (distinct, occurrences) = self.audit.totals();
            return self.audit == self.audit.rebuild_for_scope(self.status, &self.diagnostics)
                && distinct == self.summary.distinct
                && occurrences == self.summary.occurrences;
        };
        let introduced: BTreeSet<_> = delta.introduced.iter().map(String::as_str).collect();
        let scoped: Vec<_> = self
            .diagnostics
            .iter()
            .filter(|diagnostic| introduced.contains(diagnostic.id.as_str()))
            .cloned()
            .collect();
        self.audit == self.audit.rebuild_for_scope(self.status, &scoped)
    }
}

#[derive(Serialize)]
#[serde(rename = "InspectReport")]
struct Wire<'a> {
    schema_version: u8,
    audit: &'a Audit,
    status: Status,
    complete: bool,
    policy: &'a Option<PolicyReport>,
    scope: &'a Option<ScopeReport>,
    project: &'a Option<ProjectReport>,
    toolchain: &'a ToolchainReport,
    scan: &'a ScanReport,
    diagnostics: &'a [Diagnostic],
    suppressions: &'a [SuppressionReport],
    delta: &'a Option<DeltaReport>,
    errors: &'a [ReportError],
    summary: &'a Summary,
    gate: &'a GateReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    timings: Option<&'a Timings>,
}

impl Serialize for InspectReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if !self.is_valid() {
            return Err(S::Error::custom("invalid report state"));
        }
        Wire {
            schema_version: self.schema_version,
            audit: &self.audit,
            status: self.status,
            complete: self.complete,
            policy: &self.policy,
            scope: &self.scope,
            project: &self.project,
            toolchain: &self.toolchain,
            scan: &self.scan,
            diagnostics: &self.diagnostics,
            suppressions: &self.suppressions,
            delta: &self.delta,
            errors: &self.errors,
            summary: &self.summary,
            gate: &self.gate,
            timings: self.timings.as_ref(),
        }
        .serialize(serializer)
    }
}

impl fmt::Display for Status {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectReport {
    pub workspace_root: String,
    pub manifest_path: String,
    pub packages: Vec<PackageReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageReport {
    pub name: String,
    pub manifest_path: Option<String>,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolchainReport {
    pub rustc: Option<String>,
    pub cargo: Option<String>,
    pub clippy: Option<String>,
    pub rust_doctor: &'static str,
    pub removed_lint_flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanReport {
    pub command: Option<Vec<String>>,
    pub exit_code: Option<i32>,
    pub build_finished: Option<bool>,
    pub noise_lines: Option<usize>,
    pub ignored: usize,
    pub excluded_generated: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advisory_db: Option<AdvisoryDbReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub id: String,
    pub source: DiagnosticSource,
    pub code: Option<String>,
    pub base_severity: Severity,
    pub severity: Severity,
    pub category: Option<String>,
    pub message: String,
    pub help: Option<String>,
    pub package: Option<String>,
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<DiagnosticContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unscored: Option<UnscoredReason>,
    pub path: Option<String>,
    pub span: Option<DiagnosticSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity_basis_points: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity: Option<ComplexityFigures>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<Suggestion>,
    pub occurrences: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedLocation {
    pub path: String,
    pub span: DiagnosticSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComplexityFigures {
    pub cyclomatic: u32,
    pub cognitive: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    pub replacement: String,
    pub applicability: Applicability,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Applicability {
    MachineApplicable,
    MaybeIncorrect,
    HasPlaceholders,
    Unspecified,
}

impl Applicability {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "MachineApplicable" => Some(Self::MachineApplicable),
            "MaybeIncorrect" => Some(Self::MaybeIncorrect),
            "HasPlaceholders" => Some(Self::HasPlaceholders),
            "Unspecified" => Some(Self::Unspecified),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MachineApplicable => "machine-applicable",
            Self::MaybeIncorrect => "maybe-incorrect",
            Self::HasPlaceholders => "has-placeholders",
            Self::Unspecified => "unspecified",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticContext {
    Tests,
    Benchmark,
    Example,
    BuildScript,
}

impl DiagnosticContext {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tests => "tests",
            Self::Benchmark => "bench",
            Self::Example => "example",
            Self::BuildScript => "build script",
        }
    }

    pub(crate) fn from_target_kinds(kinds: &[String]) -> Option<Self> {
        kinds.iter().find_map(|kind| match kind.as_str() {
            "test" => Some(Self::Tests),
            "bench" => Some(Self::Benchmark),
            "example" => Some(Self::Example),
            "custom-build" => Some(Self::BuildScript),
            _ => None,
        })
    }

    pub(crate) fn from_conventional_path(path: &str) -> Option<Self> {
        let path = Path::new(path);
        path.parent()
            .into_iter()
            .flat_map(Path::components)
            .find_map(|component| match component {
                Component::Normal(name) if name == "tests" => Some(Self::Tests),
                Component::Normal(name) if name == "benches" => Some(Self::Benchmark),
                Component::Normal(name) if name == "examples" => Some(Self::Example),
                _ => None,
            })
            .or_else(|| (path.file_name()? == "tests.rs").then_some(Self::Tests))
    }

    pub(crate) const fn weighs(diagnostic: &Diagnostic) -> bool {
        diagnostic.context.is_none() && diagnostic.unscored.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnscoredReason {
    Uncatalogued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSource {
    Rustc,
    Clippy,
    #[serde(rename = "rust-doctor")]
    RustDoctor,
}

impl DiagnosticSource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Rustc => "rustc",
            Self::Clippy => "clippy",
            Self::RustDoctor => "rust-doctor",
        }
    }
}

impl fmt::Display for DiagnosticSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
    Unknown,
}

impl Severity {
    pub(crate) const fn rank(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warning => 1,
            Self::Info => 2,
            Self::Unknown => 3,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticSpan {
    pub line_start: usize,
    pub column_start: usize,
    pub line_end: usize,
    pub column_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportError {
    pub stage: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
    pub unknown: usize,
    pub total: usize,
    pub distinct: SeverityCounts,
    pub occurrences: SeverityCounts,
}

impl Summary {
    pub fn from_diagnostics(diagnostics: &[Diagnostic]) -> Self {
        let mut distinct = SeverityCounts::default();
        let mut occurrences = SeverityCounts::default();
        for diagnostic in diagnostics {
            distinct.add(diagnostic.severity, 1);
            occurrences.add(diagnostic.severity, diagnostic.occurrences);
        }
        Self {
            errors: distinct.errors,
            warnings: distinct.warnings,
            info: distinct.info,
            unknown: distinct.unknown,
            total: distinct.total,
            distinct,
            occurrences,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GateStatus {
    Passed,
    Failed,
    NotEvaluated,
}

impl GateStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::NotEvaluated => "not-evaluated",
        }
    }
}

impl fmt::Display for GateStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GateReport {
    pub blocking: BlockingLevel,
    pub status: GateStatus,
    pub blocking_diagnostics: Option<usize>,
}

impl InspectReport {
    pub fn exit_code(&self) -> u8 {
        if self
            .errors
            .iter()
            .any(|error| error.code == DEADLINE_EXCEEDED)
        {
            return 2;
        }
        match (self.status, self.gate.status) {
            (Status::Complete, GateStatus::Passed) => 0,
            (Status::Complete, GateStatus::Failed | GateStatus::NotEvaluated)
            | (Status::Incomplete, _) => 1,
            (Status::Failed, _) => 2,
        }
    }

    pub fn exit_code_with_warning(&self, warning: Option<u8>) -> u8 {
        let code = self.exit_code();
        let Some(warning) = warning else {
            return code;
        };
        if code != 0 || self.gate.blocking == BlockingLevel::None {
            return code;
        }
        let introduced: Option<BTreeSet<&str>> = self
            .delta
            .as_ref()
            .map(|delta| delta.introduced.iter().map(String::as_str).collect());
        let warns = self.diagnostics.iter().any(|diagnostic| {
            DiagnosticContext::weighs(diagnostic)
                && matches!(diagnostic.severity, Severity::Error | Severity::Warning)
                && introduced
                    .as_ref()
                    .is_none_or(|introduced| introduced.contains(diagnostic.id.as_str()))
        });
        if warns { warning } else { 0 }
    }
}

#[cfg(test)]
mod tests;
