//! The shape a scan publishes, and the one place its version is declared.
//!
//! This file is the wire format and nothing else: the request that starts a
//! scan, the report that comes back, and the closed vocabularies its members
//! draw from. How a report is assembled lives in [`assembly`], how a
//! producer's finding becomes one of its diagnostics in [`normalize`], and
//! what is taken out of the text a scan produced in [`sanitize`].
//!
//! Any change to the shape below bumps [`SCHEMA_VERSION`], and the frozen v7
//! archive keeps projecting from it.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::ser::{Error as _, SerializeStruct};
use serde::{Serialize, Serializer};

use crate::audit::{Audit, SeverityCounts};
use crate::delta::DeltaReport;
use crate::git_scope::{ScopeReport, ScopeRequest};
use crate::policy::{
    BlockingLevel, BlockingLevelSource, CategoryOverride, PolicyInput, PolicyPlan, RuleLevel,
    RuleLevelSource, RuleOverride, RuleTier,
};

mod assembly;
mod normalize;
mod sanitize;

pub(crate) use assembly::{
    baseline_report_failure, from_baseline_execution, from_execution_scoped, policy_failure,
    preparation_failure, scope_failure,
};

pub const SCHEMA_VERSION: u8 = 15;

#[derive(Debug, Clone)]
pub struct InspectRequest {
    pub path: PathBuf,
    policy: PolicyInput,
    scope: ScopeRequest,
}

impl InspectRequest {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            policy: PolicyInput::default(),
            scope: ScopeRequest::Full,
        }
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

    pub fn with_files_scope(mut self, base: impl Into<String>) -> Self {
        self.scope = ScopeRequest::Files { base: base.into() };
        self
    }

    pub fn with_baseline_scope(mut self, base: impl Into<String>) -> Self {
        self.scope = ScopeRequest::Baseline { base: base.into() };
        self
    }

    pub(crate) const fn policy(&self) -> &PolicyInput {
        &self.policy
    }

    pub(crate) const fn scope(&self) -> &ScopeRequest {
        &self.scope
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
    pub delta: Option<DeltaReport>,
    pub errors: Vec<ReportError>,
    pub summary: Summary,
    pub gate: GateReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyReport {
    pub config_file: Option<String>,
    pub blocking: PolicyBlockingReport,
    pub rules: Vec<PolicyRuleReport>,
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
    /// Adjudicated false-positive rate of this rule on the pinned corpus, in
    /// basis points, absent when the corpus never adjudicated it.
    ///
    /// It is published because it ranks: the report tells the user what to fix
    /// first by discounting each rule's cost by this rate, so a rule with many
    /// findings can be left out of that list, and without the number the
    /// omission reads as a defect of the tool rather than a measurement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corpus_noise_basis_points: Option<u16>,
}

impl PolicyReport {
    fn from_plan(plan: &PolicyPlan) -> Self {
        Self {
            config_file: plan.config_file().map(str::to_owned),
            blocking: PolicyBlockingReport {
                level: plan.blocking(),
                source: plan.blocking_source(),
            },
            rules: plan
                .effective_rules()
                .map(|(definition, level, source)| PolicyRuleReport {
                    id: definition.id.to_owned(),
                    category: definition.category.to_owned(),
                    tier: definition.tier,
                    level,
                    source,
                    corpus_noise_basis_points: crate::policy::corpus_noise(definition.id),
                })
                .collect(),
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
    /// A report is publishable only when its counts agree.
    ///
    /// `summary` describes the whole set of diagnostics of the report. `audit`
    /// describes the score scope: the complete report, or the introduced
    /// diagnostics alone when a delta is present. Both quantities, distinct
    /// diagnostics and occurrences, are checked separately.
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

impl Serialize for InspectReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if !self.is_valid() {
            return Err(S::Error::custom("invalid report state"));
        }
        let mut state = serializer.serialize_struct("InspectReport", 14)?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("audit", &self.audit)?;
        state.serialize_field("status", &self.status)?;
        state.serialize_field("complete", &self.complete)?;
        state.serialize_field("policy", &self.policy)?;
        state.serialize_field("scope", &self.scope)?;
        state.serialize_field("project", &self.project)?;
        state.serialize_field("toolchain", &self.toolchain)?;
        state.serialize_field("scan", &self.scan)?;
        state.serialize_field("diagnostics", &self.diagnostics)?;
        state.serialize_field("delta", &self.delta)?;
        state.serialize_field("errors", &self.errors)?;
        state.serialize_field("summary", &self.summary)?;
        state.serialize_field("gate", &self.gate)?;
        state.end()
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanReport {
    pub command: Option<Vec<String>>,
    pub exit_code: Option<i32>,
    pub build_finished: Option<bool>,
    pub noise_lines: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    /// Non-production target the diagnostic comes from, absent otherwise.
    ///
    /// Shipped code is not marked: its lack of a mark is what designates it,
    /// exactly like react-doctor's `fileContext`, which stamps only `test` and
    /// `story`. A marked diagnostic stays published and counted, but stops
    /// weighing on the score and stops blocking: a `println!` in `build.rs` is
    /// the channel Cargo imposes, not a defect of the shipped codebase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<DiagnosticContext>,
    pub path: Option<String>,
    pub span: Option<DiagnosticSpan>,
    /// Every other site the finding spans, workspace-relative.
    ///
    /// A structural finding is a family: reporting one member per diagnostic
    /// would turn a helper cloned six times into six unrelated spans. The key
    /// is absent rather than empty when a finding names a single site, so a
    /// per-site diagnostic serializes exactly as it did before this field
    /// existed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedLocation>,
    /// How alike the sites of the finding are, in basis points, when the rule
    /// grouped them on a similarity rather than on an equality.
    ///
    /// A finding whose members are exactly equal does not carry it: publishing
    /// 10000 on every exact family would say nothing, and absence is what
    /// distinguishes "these are the same" from "these are 87 % the same".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity_basis_points: Option<u16>,
    /// Cyclomatic and cognitive complexity of the reported function, when the
    /// rule measured them. Absent on every other diagnostic, so a report that
    /// carries no hotspot serializes exactly as it did before this field
    /// existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity: Option<ComplexityFigures>,
    pub occurrences: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelatedLocation {
    pub path: String,
    pub span: DiagnosticSpan,
}

/// Both complexity figures of one function, published together because each
/// answers what the other cannot: cyclomatic counts the paths a test suite has
/// to cover, cognitive weights the nesting a reader has to hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ComplexityFigures {
    pub cyclomatic: u32,
    pub cognitive: u32,
}

/// Non-production target a diagnostic comes from, derived from the target kind
/// Cargo declares. A library or a binary are not represented: they are the
/// production, and a diagnostic coming from them simply does not carry this
/// field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticContext {
    /// Integration test target, under `tests/`.
    Tests,
    /// Measurement bench, under `benches/`.
    Benchmark,
    /// Demonstration, under `examples/`.
    Example,
    /// Build script executed by Cargo.
    BuildScript,
}

impl DiagnosticContext {
    /// Closed reading of a Cargo target kind. A library, a binary and an
    /// unknown value are not marked: when in doubt, the diagnostic counts,
    /// because silencing a defect of the shipped code is the only mistake this
    /// field can make expensive.
    pub(crate) fn from_target_kinds(kinds: &[String]) -> Option<Self> {
        kinds.iter().find_map(|kind| match kind.as_str() {
            "test" => Some(Self::Tests),
            "bench" => Some(Self::Benchmark),
            "example" => Some(Self::Example),
            "custom-build" => Some(Self::BuildScript),
            _ => None,
        })
    }

    /// Closed reading of a workspace-relative path, for a file no Cargo target
    /// and no module declaration speaks for.
    ///
    /// This is the last evidence there is, and the only producer that needs it
    /// is the orphan walk: a file compiled by nothing is reached by nothing, so
    /// neither the target kind above nor the `cfg(test)` gate the source walk
    /// propagates has anything to say about it. The convention is Cargo's own,
    /// `tests/`, `benches/` and `examples/`, read on the outermost directory
    /// that matches, so `benches/tests.rs` is a bench and not a test. A file
    /// named `tests.rs` is the module spelling of the same convention and is
    /// read last, since a directory above it is the stronger claim.
    ///
    /// Anything else is not marked, for the reason `from_target_kinds` gives:
    /// silencing a defect of the shipped code is the only mistake this field
    /// can make expensive.
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

    /// Does a diagnostic weigh on the score and on the gate?
    ///
    /// This is the decision react-doctor makes in `filterForSurface`: a
    /// diagnostic stamped with a non-production context leaves the `score` and
    /// `ciFailure` surfaces, and stays in `cli`. It is not removed, it stops
    /// costing.
    pub(crate) const fn weighs(diagnostic: &Diagnostic) -> bool {
        diagnostic.context.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

/// Counts of the report, published under two explicit quantities.
///
/// The five flat fields are the historical alias of `distinct`: a diagnostic
/// reported by two compilation targets counts as one distinct diagnostic and
/// two occurrences.
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
    /// The only admitted derivation of the counts: a report whose `summary`
    /// departs from this function is refused at serialization.
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
    pub const fn exit_code(&self) -> u8 {
        match (self.status, self.gate.status) {
            (Status::Complete, GateStatus::Passed) => 0,
            (Status::Complete, GateStatus::Failed | GateStatus::NotEvaluated)
            | (Status::Incomplete, _) => 1,
            (Status::Failed, _) => 2,
        }
    }
}

























































#[cfg(test)]
mod tests;
