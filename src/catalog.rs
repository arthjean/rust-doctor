//! Canonical rule metadata shared by configuration and every renderer.
#![expect(
    clippy::redundant_pub_crate,
    reason = "catalog items are consumed by sibling modules through this private crate module"
)]

use crate::diagnostics::{Category, Severity, SourceSurface};
use crate::trust::{
    AggregationPolicy, AnalyzerAvailability, Priority, RequiredEvidence, RuleDecision,
    TrustProfile, TrustTier,
};
use crate::{clippy, rules, trust};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(test)]
use std::collections::HashSet;
use std::sync::LazyLock;

/// Analyzer responsible for producing a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AnalyzerKind {
    SynAst,
    Clippy,
    Dependency,
    Project,
    External,
}

/// Adapter provenance used when a dynamic diagnostic has no exact descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdapterProvenance {
    Rustc,
    Clippy,
    RustSec,
    CargoDeny,
}

/// Confidence consumers should assign to a rule without type information.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Confidence {
    Low,
    Medium,
    High,
}

/// The strongest fix contract offered by a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FixCapability {
    None,
    Guidance,
    RustcSuggestion,
    MachineApplicable,
}

/// Where a rule belongs when large result sets are grouped for migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum MigrationDisposition {
    Actionable,
    Advisory,
    Audit,
}

/// Product-surface decision owned by the catalog.
///
/// Detection provenance, presentation severity, and confidence are
/// deliberately absent. A compiler-proven diagnostic may still be advisory,
/// and an error does not become CI-blocking unless this record says so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DiagnosticSurfacePolicy {
    pub(crate) local: bool,
    pub(crate) score: bool,
    pub(crate) ci: bool,
    pub(crate) pull_request: bool,
    pub(crate) sarif: bool,
    pub(crate) mcp: bool,
    pub(crate) migration: MigrationDisposition,
    pub(crate) audit_only: bool,
    pub(crate) reason_code: &'static str,
}

/// Inclusive numeric threshold range supported by a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NumericRange {
    pub(crate) min: u32,
    pub(crate) max: u32,
    pub(crate) default: u32,
}

/// Version and feature evidence required before a framework rule can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct FrameworkRequirement {
    pub(crate) framework: String,
    pub(crate) version: String,
    pub(crate) required_features: Vec<String>,
}

/// Trust contract carried by every catalog entry.
///
/// Trust tier, priority, and score eligibility stay independent from severity,
/// confidence, and category: no consumer may infer one from another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RuleTrust {
    pub(crate) tier: TrustTier,
    /// `None` means unranked. Unknown and dynamic rules never receive an
    /// invented priority (FR-002).
    pub(crate) priority: Option<Priority>,
    pub(crate) score_eligible: bool,
    pub(crate) required_evidence: Vec<RequiredEvidence>,
    pub(crate) aggregation: AggregationPolicy,
    pub(crate) calibration_version: Option<String>,
    pub(crate) availability: AnalyzerAvailability,
    pub(crate) supported_contexts: Vec<SourceSurface>,
    /// Recorded requalification verdict. `None` for analyzers whose authority
    /// comes from their tier rather than a calibration artifact.
    pub(crate) decision: Option<RuleDecision>,
}

impl RuleTrust {
    fn from_profile(rule: &str, profile: &TrustProfile, analyzer: AnalyzerKind) -> Self {
        Self {
            tier: profile.tier,
            priority: profile.priority,
            score_eligible: trust::resolve_score_eligibility(rule, profile, analyzer),
            required_evidence: profile.required_evidence.clone(),
            aggregation: profile.aggregation,
            calibration_version: trust::calibration_version_for(rule),
            availability: profile.availability,
            supported_contexts: profile.supported_contexts.clone(),
            decision: trust::decision_for(rule),
        }
    }

    pub(crate) fn resolve(rule: &str, analyzer: AnalyzerKind) -> Self {
        trust::profile_for(rule).map_or_else(
            || Self::from_profile(rule, &trust::unranked_profile(analyzer), analyzer),
            |profile| Self::from_profile(rule, profile, analyzer),
        )
    }

    /// A score-eligible entry only contributes to the Core Score when its
    /// analyzer ships with Rust Doctor and the toolchain. Optional external
    /// executables keep their diagnostics and completeness receipts but never
    /// move the number based on local installation state (FR-011).
    pub(crate) const fn contributes_to_core_score(&self) -> bool {
        self.score_eligible && matches!(self.availability, AnalyzerAvailability::Core)
    }
}

/// Complete metadata for one canonical rule identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RuleDescriptor {
    pub(crate) canonical_id: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) provider: String,
    pub(crate) category: Category,
    pub(crate) default_severity: Severity,
    pub(crate) tags: Vec<String>,
    pub(crate) analyzer_kind: AnalyzerKind,
    pub(crate) confidence: Confidence,
    pub(crate) default_enabled: bool,
    pub(crate) applicable_frameworks: Vec<String>,
    pub(crate) framework_requirements: Vec<FrameworkRequirement>,
    pub(crate) documentation_url: String,
    pub(crate) supported_threshold: Option<NumericRange>,
    pub(crate) fix_capability: FixCapability,
    pub(crate) description: String,
    pub(crate) fix_guidance: String,
    pub(crate) limitation_ids: Vec<String>,
    pub(crate) trust: RuleTrust,
    pub(crate) surface_policy: DiagnosticSurfacePolicy,
    /// True when the rule runs by default but contributes exactly zero to the
    /// score. Output labels it advisory so its presence never reads as a score
    /// penalty (US-007 AC-6).
    pub(crate) advisory: bool,
}

impl RuleDescriptor {
    fn custom(rule: &dyn rules::CustomRule) -> Self {
        let mut tags = vec![
            "heuristic".to_string(),
            category_tag(&rule.category()).to_string(),
        ];
        tags.sort();
        tags.dedup();
        let applicable_frameworks: Vec<String> = rule
            .applicable_frameworks()
            .iter()
            .map(|value| (*value).to_string())
            .collect();
        let framework_requirements = applicable_frameworks
            .iter()
            .map(|framework| FrameworkRequirement {
                framework: framework.clone(),
                version: rule
                    .framework_version_requirements()
                    .iter()
                    .find_map(|(name, version)| (*name == framework).then_some(*version))
                    .unwrap_or("*")
                    .to_string(),
                required_features: rule
                    .required_framework_features()
                    .iter()
                    .find_map(|(name, features)| (*name == framework).then_some(*features))
                    .unwrap_or_default()
                    .iter()
                    .map(|feature| (*feature).to_string())
                    .collect(),
            })
            .collect();
        let trust = RuleTrust::resolve(rule.name(), AnalyzerKind::SynAst);
        Self {
            canonical_id: rule.name().to_string(),
            aliases: Vec::new(),
            provider: "rust-doctor".to_string(),
            category: rule.category(),
            default_severity: rule.severity(),
            tags,
            analyzer_kind: AnalyzerKind::SynAst,
            confidence: rule.confidence(),
            default_enabled: rule.default_enabled(),
            applicable_frameworks,
            framework_requirements,
            documentation_url: format!("https://rust-doctor.vercel.app/rules/{}", rule.name()),
            supported_threshold: rule.supported_threshold(),
            fix_capability: FixCapability::Guidance,
            description: rule.description().to_string(),
            fix_guidance: advisory_fix_guidance(rule.name())
                .unwrap_or_else(|| rule.fix_hint())
                .to_string(),
            limitation_ids: rules::documented_limitations(rule.name())
                .iter()
                .map(|limitation| format!("{}:{limitation}", rule.name()))
                .collect(),
            advisory: rule.default_enabled() && !trust.score_eligible,
            surface_policy: surface_policy(rule.name(), &trust, true),
            trust,
        }
    }

    fn clippy(entry: &clippy::LintEntry) -> Self {
        let id = format!("clippy::{}", entry.name);
        let trust = RuleTrust::from_profile(
            &id,
            &trust::clippy_profile(&entry.category, entry.severity),
            AnalyzerKind::Clippy,
        );
        let surface_policy = surface_policy(&id, &trust, true);
        let fix_guidance = advisory_fix_guidance(&id)
            .unwrap_or("Apply the rustc suggestion when its applicability permits it.")
            .to_string();
        Self {
            canonical_id: id,
            aliases: vec![entry.name.to_string()],
            provider: "clippy".to_string(),
            category: entry.category.clone(),
            default_severity: entry.severity,
            tags: vec![
                "type-aware".to_string(),
                category_tag(&entry.category).to_string(),
            ],
            analyzer_kind: AnalyzerKind::Clippy,
            confidence: Confidence::High,
            default_enabled: true,
            applicable_frameworks: Vec::new(),
            framework_requirements: Vec::new(),
            documentation_url: format!(
                "https://rust-lang.github.io/rust-clippy/master/index.html#{}",
                entry.name
            ),
            supported_threshold: None,
            fix_capability: FixCapability::RustcSuggestion,
            description: format!("Clippy `{}` diagnostic", entry.name),
            fix_guidance,
            limitation_ids: Vec::new(),
            advisory: !trust.score_eligible,
            surface_policy,
            trust,
        }
    }

    fn external(
        id: &str,
        provider: &str,
        category: &Category,
        severity: Severity,
        analyzer_kind: AnalyzerKind,
        catalog_mapped: bool,
    ) -> Self {
        let trust = RuleTrust::resolve(id, analyzer_kind);
        Self {
            canonical_id: id.to_string(),
            aliases: Vec::new(),
            provider: provider.to_string(),
            category: category.clone(),
            default_severity: severity,
            tags: vec![category_tag(category).to_string(), "external".to_string()],
            analyzer_kind,
            confidence: Confidence::High,
            default_enabled: true,
            applicable_frameworks: Vec::new(),
            framework_requirements: Vec::new(),
            documentation_url: "https://rust-doctor.vercel.app/rules/external".to_string(),
            supported_threshold: None,
            fix_capability: FixCapability::Guidance,
            description: format!("Finding reported by {provider}"),
            fix_guidance: advisory_fix_guidance(id)
                .unwrap_or("Follow the originating analyzer guidance.")
                .to_string(),
            limitation_ids: Vec::new(),
            advisory: !trust.score_eligible,
            surface_policy: surface_policy(id, &trust, catalog_mapped),
            trust,
        }
    }
}

fn advisory_fix_guidance(rule: &str) -> Option<&'static str> {
    trust::is_unqualified_high_volume(rule).then_some(
        "Sample representative occurrences first, confirm the pattern is worth changing in this repository, then remediate the validated group.",
    )
}

/// One explicit default decision for every catalog rule.
///
/// Unknown dynamic rules take the final advisory branch. This keeps them
/// locally observable and exportable without fabricating rank, score impact,
/// CI authority, or pull-request urgency.
fn surface_policy(rule: &str, trust: &RuleTrust, catalog_mapped: bool) -> DiagnosticSurfacePolicy {
    if !catalog_mapped && rule.starts_with("RUSTSEC-") {
        return DiagnosticSurfacePolicy {
            local: true,
            score: false,
            ci: true,
            pull_request: true,
            sarif: true,
            mcp: true,
            migration: MigrationDisposition::Actionable,
            audit_only: false,
            reason_code: "confirmed-rustsec-advisory",
        };
    }
    if !catalog_mapped {
        return DiagnosticSurfacePolicy {
            local: true,
            score: false,
            ci: false,
            pull_request: false,
            sarif: true,
            mcp: true,
            migration: MigrationDisposition::Advisory,
            audit_only: false,
            reason_code: "unmapped-local-advisory",
        };
    }
    if trust.tier == TrustTier::AuditOnly || trust.aggregation == AggregationPolicy::AuditOnly {
        return DiagnosticSurfacePolicy {
            local: true,
            score: false,
            ci: false,
            pull_request: false,
            sarif: true,
            mcp: true,
            migration: MigrationDisposition::Audit,
            audit_only: true,
            reason_code: "audit-observation",
        };
    }
    if trust::is_unqualified_high_volume(rule) {
        return DiagnosticSurfacePolicy {
            local: true,
            score: false,
            ci: false,
            pull_request: false,
            sarif: true,
            mcp: true,
            migration: MigrationDisposition::Advisory,
            audit_only: false,
            reason_code: "unqualified-high-volume",
        };
    }
    if trust.score_eligible {
        return DiagnosticSurfacePolicy {
            local: true,
            score: true,
            ci: true,
            pull_request: true,
            sarif: true,
            mcp: true,
            migration: MigrationDisposition::Actionable,
            audit_only: false,
            reason_code: "qualified-actionable",
        };
    }
    DiagnosticSurfacePolicy {
        local: true,
        score: false,
        ci: false,
        pull_request: false,
        sarif: true,
        mcp: true,
        migration: MigrationDisposition::Advisory,
        audit_only: false,
        reason_code: "non-scoring-advisory",
    }
}

pub(crate) fn fallback_surface_policy(rule: &str) -> DiagnosticSurfacePolicy {
    surface_policy(
        rule,
        &RuleTrust::resolve(rule, AnalyzerKind::External),
        false,
    )
}

/// A validated index over canonical IDs and compatibility aliases.
#[derive(Debug)]
pub(crate) struct RuleCatalog {
    descriptors: Vec<RuleDescriptor>,
    lookup: HashMap<String, usize>,
}

impl RuleCatalog {
    pub(crate) fn build(mut descriptors: Vec<RuleDescriptor>) -> Result<Self, CatalogError> {
        descriptors.sort_by(|left, right| left.canonical_id.cmp(&right.canonical_id));
        let mut lookup = HashMap::new();
        for (index, descriptor) in descriptors.iter().enumerate() {
            validate_trust_contract(descriptor)?;
            register_key(&mut lookup, &descriptor.canonical_id, index)?;
            for alias in &descriptor.aliases {
                register_key(&mut lookup, alias, index)?;
            }
        }
        Ok(Self {
            descriptors,
            lookup,
        })
    }

    pub(crate) fn descriptors(&self) -> &[RuleDescriptor] {
        &self.descriptors
    }

    pub(crate) fn exact(&self, id_or_alias: &str) -> Option<&RuleDescriptor> {
        self.lookup
            .get(id_or_alias)
            .and_then(|index| self.descriptors.get(*index))
    }

    /// Resolve a raw adapter rule. Unknown dynamic codes receive a namespaced
    /// descriptor instead of being misrepresented as a built-in rule.
    pub(crate) fn resolve(
        &self,
        raw_rule: &str,
        category: &Category,
        severity: Severity,
    ) -> ResolvedDescriptor<'_> {
        self.resolve_with_provenance(raw_rule, category, severity, None)
    }

    pub(crate) fn resolve_with_provenance(
        &self,
        raw_rule: &str,
        category: &Category,
        severity: Severity,
        provenance: Option<AdapterProvenance>,
    ) -> ResolvedDescriptor<'_> {
        if let Some(descriptor) = self.exact(raw_rule) {
            return ResolvedDescriptor::Exact(descriptor);
        }

        let (provider, analyzer, url) = external_namespace(raw_rule, provenance);
        let mut descriptor =
            RuleDescriptor::external(raw_rule, provider, category, severity, analyzer, false);
        descriptor.documentation_url = url;
        ResolvedDescriptor::Fallback(Box::new(descriptor))
    }

    #[cfg(test)]
    fn custom_count(&self) -> usize {
        self.descriptors
            .iter()
            .filter(|descriptor| descriptor.analyzer_kind == AnalyzerKind::SynAst)
            .count()
    }

    #[cfg(test)]
    fn clippy_count(&self) -> usize {
        self.descriptors
            .iter()
            .filter(|descriptor| descriptor.analyzer_kind == AnalyzerKind::Clippy)
            .count()
    }
}

pub(crate) enum ResolvedDescriptor<'a> {
    Exact(&'a RuleDescriptor),
    Fallback(Box<RuleDescriptor>),
}

impl ResolvedDescriptor<'_> {
    pub(crate) fn as_descriptor(&self) -> &RuleDescriptor {
        match self {
            Self::Exact(descriptor) => descriptor,
            Self::Fallback(descriptor) => descriptor.as_ref(),
        }
    }

    pub(crate) const fn is_fallback(&self) -> bool {
        matches!(self, Self::Fallback(_))
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum CatalogError {
    #[error("duplicate canonical rule ID or alias: {0}")]
    DuplicateId(String),
    #[error("rule '{rule}' violates its trust contract: {reason}")]
    TrustContract { rule: String, reason: String },
}

/// A score-eligible rule must carry a complete evidence contract. Missing
/// calibration, evidence, or aggregation policy is a hard catalog failure
/// rather than a silently-scoring rule (US-002 AC-4).
fn validate_trust_contract(descriptor: &RuleDescriptor) -> Result<(), CatalogError> {
    let trust = &descriptor.trust;
    let surface = &descriptor.surface_policy;
    let reject = |reason: &str| {
        Err(CatalogError::TrustContract {
            rule: descriptor.canonical_id.clone(),
            reason: reason.to_string(),
        })
    };
    if surface.reason_code.is_empty() {
        return reject("surface policy has no reason code");
    }
    if surface.score != trust.score_eligible {
        return reject("surface score decision disagrees with trust score eligibility");
    }
    if surface.audit_only
        && (surface.score
            || surface.ci
            || surface.pull_request
            || surface.migration != MigrationDisposition::Audit)
    {
        return reject("audit-only rule is actionable on a default surface");
    }
    if !surface.audit_only && surface.migration == MigrationDisposition::Audit {
        return reject("non-audit rule is assigned to audit migration grouping");
    }
    // Every `syn` heuristic carries a recorded requalification decision, and
    // that decision is what decides whether it ships enabled. A rule present
    // before this program receives no exception (US-007 AC-1, AC-7).
    if descriptor.analyzer_kind == AnalyzerKind::SynAst {
        let Some(decision) = trust.decision else {
            return reject("custom rule ships without a recorded requalification decision");
        };
        if decision.is_default() != descriptor.default_enabled {
            return reject(
                "default activation disagrees with the recorded requalification decision",
            );
        }
        if trust.score_eligible && decision != RuleDecision::ScoreEligibleDefault {
            return reject("rule is score-eligible without a score-eligible-default decision");
        }
        // Claiming the score-eligible decision without the measurement to back
        // it is the failure mode this program exists to prevent: a rule that
        // misses a threshold is demoted, never quietly promoted (AC-3, AC-7).
        if decision == RuleDecision::ScoreEligibleDefault
            && trust
                .calibration_version
                .as_ref()
                .is_none_or(String::is_empty)
        {
            return reject(
                "score-eligible-default decision is not backed by a passing measurement",
            );
        }
    }
    if !trust.score_eligible {
        return Ok(());
    }
    if trust.required_evidence.is_empty() {
        return reject("score-eligible rule declares no required evidence");
    }
    if trust.aggregation == AggregationPolicy::AuditOnly {
        return reject("score-eligible rule uses the audit-only aggregation policy");
    }
    if trust.priority.is_none() {
        return reject("score-eligible rule is unranked");
    }
    if trust.tier == TrustTier::CalibratedHeuristic && trust.calibration_version.is_none() {
        return reject("calibrated heuristic is score-eligible without a calibration version");
    }
    if !trust
        .required_evidence
        .iter()
        .all(|evidence| evidence.supplied_by(descriptor.analyzer_kind))
    {
        return reject("required evidence is not supplied by the owning analyzer");
    }
    Ok(())
}

impl RuleDescriptor {
    /// Inputs this rule presents to its trust-tier promotion gate.
    pub(crate) fn gate_subject(&self) -> trust::GateSubject<'_> {
        trust::GateSubject {
            rule: &self.canonical_id,
            provider: &self.provider,
            tier: self.trust.tier,
            analyzer: self.analyzer_kind,
            default_enabled: self.default_enabled,
            score_eligible: self.trust.score_eligible,
            required_evidence: &self.trust.required_evidence,
        }
    }
}

/// Run the whole catalog through the promotion and demotion gates.
///
/// This is protected validation: it runs in the test suite, so a rule that
/// loses its authority fails CI rather than shipping.
///
/// Returns one entry per rule that fails or is proposed for demotion. An empty
/// result is the only shippable state: a default or score-eligible rule without
/// current authority is a release blocker, whether it was added yesterday or
/// predates this program (US-012).
#[cfg(test)]
pub(crate) fn promotion_gate_failures(
    today: &str,
) -> Result<Vec<trust::GateFailure>, CatalogError> {
    let catalog = built_in_catalog()?;
    Ok(catalog
        .descriptors()
        .iter()
        .filter_map(
            |descriptor| match trust::evaluate_gate(&descriptor.gate_subject(), today) {
                trust::GateVerdict::Failing(failure)
                | trust::GateVerdict::DemotionProposed(failure) => Some(failure),
                trust::GateVerdict::NotGated
                | trust::GateVerdict::Passing(_)
                | trust::GateVerdict::Exempt(_) => None,
            },
        )
        .collect())
}

/// Known boundaries where a rule can be wrong.
///
/// One source for `rules explain`, Report V1 diagnostic metadata, and rule
/// documentation: a false-positive boundary described in only one of them would
/// be a different product answer per surface (US-014 AC-2).
pub(crate) fn rule_limitations(
    descriptor: &RuleDescriptor,
    namespace_fallback: bool,
) -> Vec<String> {
    let specific = match descriptor.canonical_id.as_str() {
        "unwrap-in-production" => Some(
            "Syntactic matching cannot distinguish a provably infallible unwrap from a risky one.",
        ),
        "large-enum-variant" => Some(
            "The rule counts fields rather than calculating each variant's concrete byte layout.",
        ),
        "blocking-in-async" => Some(
            "The rule recognizes known call names but does not follow aliases or interprocedural calls.",
        ),
        "sql-injection-risk" => Some(
            "String-built queries are heuristic evidence; the rule cannot prove that interpolated data is untrusted.",
        ),
        _ => None,
    };
    let mut values = specific.into_iter().map(str::to_string).collect::<Vec<_>>();
    values.extend(
        descriptor
            .limitation_ids
            .iter()
            .map(|id| format!("Conformance limitation: {id}")),
    );
    if specific.is_none() && descriptor.analyzer_kind == AnalyzerKind::SynAst {
        values.push(
            "Syntactic analysis does not have rustc name resolution or inferred type information."
                .to_string(),
        );
    }
    if values.is_empty() {
        values.push(match descriptor.analyzer_kind {
            AnalyzerKind::Clippy => {
                "Compiler-aware evidence is unavailable when the package cannot complete Clippy analysis."
            }
            AnalyzerKind::Dependency | AnalyzerKind::External => {
                "Evidence depends on the originating external analyzer and its local data being available."
            }
            AnalyzerKind::Project => {
                "Project-level evidence does not identify a precise source span."
            }
            AnalyzerKind::SynAst => unreachable!("syn rules receive the generic syntax limitation"),
        }
        .to_string());
    }
    if namespace_fallback {
        values.push(
            "This rule family has no explicit descriptor in this Rust Doctor build, so category and severity use namespace defaults."
                .to_string(),
        );
    }
    values
}

/// What evidence an analyzer class can actually observe.
pub(crate) const fn evidence_model(analyzer: AnalyzerKind) -> &'static str {
    match analyzer {
        AnalyzerKind::SynAst => "Syntactic Rust AST evidence without name or type resolution.",
        AnalyzerKind::Clippy => {
            "Type-aware rustc and Clippy evidence with compiler spans and suggestions."
        }
        AnalyzerKind::Dependency => {
            "Dependency graph, advisory, or policy evidence from the named analyzer."
        }
        AnalyzerKind::Project => "Cargo metadata or aggregate project-level evidence.",
        AnalyzerKind::External => "Analyzer-provided evidence retained under its namespace.",
    }
}

/// Rules the gate proposes for automatic demotion, with their machine-readable
/// reason (US-012 AC-3).
#[cfg(test)]
pub(crate) fn proposed_demotions(today: &str) -> Result<Vec<trust::GateFailure>, CatalogError> {
    let catalog = built_in_catalog()?;
    Ok(catalog
        .descriptors()
        .iter()
        .filter_map(
            |descriptor| match trust::evaluate_gate(&descriptor.gate_subject(), today) {
                trust::GateVerdict::DemotionProposed(failure) => Some(failure),
                _ => None,
            },
        )
        .collect())
}

static BUILT_IN_CATALOG: LazyLock<Result<RuleCatalog, CatalogError>> = LazyLock::new(|| {
    let mut descriptors: Vec<RuleDescriptor> = rules::all_custom_rules()
        .iter()
        .map(|rule| RuleDescriptor::custom(rule.as_ref()))
        .collect();
    descriptors.extend(clippy::LINT_REGISTRY.iter().map(RuleDescriptor::clippy));
    descriptors.extend(external_descriptors());
    RuleCatalog::build(descriptors)
});

pub(crate) fn built_in_catalog() -> Result<&'static RuleCatalog, CatalogError> {
    BUILT_IN_CATALOG.as_ref().map_err(Clone::clone)
}

fn register_key(
    lookup: &mut HashMap<String, usize>,
    key: &str,
    index: usize,
) -> Result<(), CatalogError> {
    if lookup.insert(key.to_string(), index).is_some() {
        return Err(CatalogError::DuplicateId(key.to_string()));
    }
    Ok(())
}

const fn category_tag(category: &Category) -> &'static str {
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

fn external_namespace(
    rule: &str,
    provenance: Option<AdapterProvenance>,
) -> (&'static str, AnalyzerKind, String) {
    if provenance == Some(AdapterProvenance::RustSec) || rule.starts_with("RUSTSEC-") {
        return (
            "rustsec",
            AnalyzerKind::Dependency,
            format!("https://rustsec.org/advisories/{rule}.html"),
        );
    }
    if provenance == Some(AdapterProvenance::CargoDeny) || rule.starts_with("deny::") {
        return (
            "cargo-deny",
            AnalyzerKind::Dependency,
            "https://embarkstudios.github.io/cargo-deny/checks/index.html".to_string(),
        );
    }
    if provenance == Some(AdapterProvenance::Clippy) || rule.starts_with("clippy::") {
        let lint = rule.strip_prefix("clippy::").unwrap_or(rule);
        return (
            "clippy",
            AnalyzerKind::Clippy,
            format!("https://rust-lang.github.io/rust-clippy/master/index.html#{lint}"),
        );
    }
    if provenance == Some(AdapterProvenance::Rustc) {
        return (
            "rustc",
            AnalyzerKind::External,
            "https://doc.rust-lang.org/error_codes/error-index.html".to_string(),
        );
    }
    (
        "external",
        AnalyzerKind::External,
        "https://rust-doctor.vercel.app/rules/external".to_string(),
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "the external descriptor table is declarative catalog data"
)]
fn external_descriptors() -> Vec<RuleDescriptor> {
    [
        (
            "deny-advisory",
            "cargo-deny",
            Category::Dependencies,
            Severity::Error,
            AnalyzerKind::Dependency,
        ),
        (
            "deny-license",
            "cargo-deny",
            Category::Cargo,
            Severity::Warning,
            AnalyzerKind::Dependency,
        ),
        (
            "deny-ban",
            "cargo-deny",
            Category::Cargo,
            Severity::Error,
            AnalyzerKind::Dependency,
        ),
        (
            "deny-source",
            "cargo-deny",
            Category::Cargo,
            Severity::Warning,
            AnalyzerKind::Dependency,
        ),
        (
            "deny-unknown",
            "cargo-deny",
            Category::Dependencies,
            Severity::Warning,
            AnalyzerKind::Dependency,
        ),
        (
            "unused-dependency",
            "cargo-shear",
            Category::Dependencies,
            Severity::Warning,
            AnalyzerKind::Dependency,
        ),
        (
            "unsafe-dependency",
            "cargo-geiger",
            Category::Security,
            Severity::Warning,
            AnalyzerKind::Dependency,
        ),
        (
            "semver-violation",
            "cargo-semver-checks",
            Category::Correctness,
            Severity::Error,
            AnalyzerKind::Dependency,
        ),
        (
            "missing-msrv",
            "rust-doctor",
            Category::Cargo,
            Severity::Warning,
            AnalyzerKind::Project,
        ),
        (
            "msrv-outdated",
            "rust-doctor",
            Category::Cargo,
            Severity::Warning,
            AnalyzerKind::Project,
        ),
        (
            "msrv-incompatible",
            "rust-doctor",
            Category::Cargo,
            Severity::Error,
            AnalyzerKind::Project,
        ),
        (
            "low-coverage",
            "cargo-llvm-cov",
            Category::Correctness,
            Severity::Warning,
            AnalyzerKind::Project,
        ),
        (
            "uncovered-file",
            "cargo-llvm-cov",
            Category::Correctness,
            Severity::Warning,
            AnalyzerKind::Project,
        ),
        (
            "compiler-error",
            "rustc",
            Category::Correctness,
            Severity::Error,
            AnalyzerKind::Project,
        ),
        (
            "compiler-ice",
            "rustc",
            Category::Correctness,
            Severity::Error,
            AnalyzerKind::Project,
        ),
        (
            "unknown-rustc-level",
            "rustc",
            Category::Correctness,
            Severity::Info,
            AnalyzerKind::Project,
        ),
        (
            "skipped-pass",
            "rust-doctor",
            Category::Cargo,
            Severity::Info,
            AnalyzerKind::Project,
        ),
    ]
    .into_iter()
    .map(|(id, provider, category, severity, analyzer)| {
        RuleDescriptor::external(id, provider, &category, severity, analyzer, true)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(id: &str, aliases: &[&str]) -> RuleDescriptor {
        RuleDescriptor {
            canonical_id: id.to_string(),
            aliases: aliases.iter().map(|value| (*value).to_string()).collect(),
            provider: "test".to_string(),
            category: Category::Style,
            default_severity: Severity::Warning,
            tags: Vec::new(),
            analyzer_kind: AnalyzerKind::External,
            confidence: Confidence::High,
            default_enabled: true,
            applicable_frameworks: Vec::new(),
            framework_requirements: Vec::new(),
            documentation_url: String::new(),
            supported_threshold: None,
            fix_capability: FixCapability::None,
            description: String::new(),
            fix_guidance: String::new(),
            limitation_ids: Vec::new(),
            advisory: !RuleTrust::resolve(id, AnalyzerKind::External).score_eligible,
            surface_policy: surface_policy(
                id,
                &RuleTrust::resolve(id, AnalyzerKind::External),
                false,
            ),
            trust: RuleTrust::resolve(id, AnalyzerKind::External),
        }
    }

    #[test]
    fn built_in_catalog_covers_every_explicit_rule() {
        let catalog = built_in_catalog().unwrap();
        assert_eq!(catalog.custom_count(), rules::all_custom_rules().len());
        assert_eq!(catalog.clippy_count(), clippy::LINT_REGISTRY.len());
        for rule in rules::all_custom_rules() {
            assert!(catalog.exact(rule.name()).is_some(), "{}", rule.name());
        }
        for lint in clippy::known_lint_names() {
            assert!(catalog.exact(lint).is_some(), "{lint}");
        }
    }

    #[test]
    fn duplicate_ids_and_aliases_fail_deterministically() {
        let duplicate_id =
            RuleCatalog::build(vec![descriptor("same", &[]), descriptor("same", &[])]);
        assert!(matches!(duplicate_id, Err(CatalogError::DuplicateId(id)) if id == "same"));

        let duplicate_alias = RuleCatalog::build(vec![
            descriptor("first", &["shared"]),
            descriptor("second", &["shared"]),
        ]);
        assert!(matches!(duplicate_alias, Err(CatalogError::DuplicateId(id)) if id == "shared"));
    }

    #[test]
    fn dynamic_rustsec_codes_use_documented_namespace_fallback() {
        let catalog = built_in_catalog().unwrap();
        let resolved = catalog.resolve_with_provenance(
            "RUSTSEC-2099-9999",
            &Category::Security,
            Severity::Error,
            Some(AdapterProvenance::RustSec),
        );
        let descriptor = resolved.as_descriptor();
        assert!(resolved.is_fallback());
        assert_eq!(descriptor.provider, "rustsec");
        assert!(descriptor.documentation_url.contains("RUSTSEC-2099-9999"));
    }

    #[test]
    fn cargo_deny_codes_preserve_adapter_namespace() {
        let catalog = built_in_catalog().unwrap();
        let resolved = catalog.resolve_with_provenance(
            "future-deny-code",
            &Category::Dependencies,
            Severity::Warning,
            Some(AdapterProvenance::CargoDeny),
        );
        let descriptor = resolved.as_descriptor();
        assert!(resolved.is_fallback());
        assert_eq!(descriptor.provider, "cargo-deny");
        assert_eq!(descriptor.analyzer_kind, AnalyzerKind::Dependency);
    }

    #[test]
    fn unknown_compiler_diagnostics_preserve_adapter_namespace() {
        let catalog = built_in_catalog().unwrap();
        let clippy = catalog.resolve_with_provenance(
            "clippy::future_lint",
            &Category::Style,
            Severity::Warning,
            Some(AdapterProvenance::Clippy),
        );
        assert_eq!(clippy.as_descriptor().provider, "clippy");
        assert_eq!(clippy.as_descriptor().analyzer_kind, AnalyzerKind::Clippy);

        let rustc = catalog.resolve_with_provenance(
            "future_rustc_lint",
            &Category::Correctness,
            Severity::Warning,
            Some(AdapterProvenance::Rustc),
        );
        assert_eq!(rustc.as_descriptor().provider, "rustc");
        assert_eq!(rustc.as_descriptor().analyzer_kind, AnalyzerKind::External);
    }

    #[test]
    fn every_built_in_rule_declares_a_complete_trust_contract() {
        let catalog = built_in_catalog().unwrap();
        for descriptor in catalog.descriptors() {
            let trust = &descriptor.trust;
            assert!(
                !trust.required_evidence.is_empty(),
                "{} declares no required evidence",
                descriptor.canonical_id
            );
            assert!(
                !trust.supported_contexts.is_empty(),
                "{} declares no supported contexts",
                descriptor.canonical_id
            );
            if trust.score_eligible {
                assert!(trust.priority.is_some(), "{}", descriptor.canonical_id);
                assert_ne!(
                    trust.aggregation,
                    AggregationPolicy::AuditOnly,
                    "{}",
                    descriptor.canonical_id
                );
            }
        }
    }

    #[test]
    fn score_eligible_calibrated_heuristics_require_a_calibration_version() {
        let catalog = built_in_catalog().unwrap();
        for descriptor in catalog.descriptors() {
            if descriptor.trust.tier == TrustTier::CalibratedHeuristic
                && descriptor.trust.score_eligible
            {
                assert!(
                    descriptor.trust.calibration_version.is_some(),
                    "{} is score-eligible without calibration",
                    descriptor.canonical_id
                );
            }
        }
    }

    #[test]
    fn optional_external_findings_never_reach_the_core_score() {
        let catalog = built_in_catalog().unwrap();
        for descriptor in catalog.descriptors() {
            if descriptor.trust.availability == AnalyzerAvailability::OptionalExternal {
                assert!(
                    !descriptor.trust.contributes_to_core_score(),
                    "{} would move the Core Score",
                    descriptor.canonical_id
                );
            }
        }
    }

    #[test]
    fn trust_validation_rejects_a_fabricated_score_eligible_rule() {
        let mut descriptor = descriptor("fabricated", &[]);
        descriptor.trust.score_eligible = true;
        descriptor.trust.tier = TrustTier::CalibratedHeuristic;
        descriptor.trust.priority = Some(Priority::P1);
        descriptor.trust.calibration_version = None;
        let error = RuleCatalog::build(vec![descriptor]).unwrap_err();
        assert!(matches!(
            error,
            CatalogError::TrustContract { ref rule, .. } if rule == "fabricated"
        ));
    }

    #[test]
    fn dynamic_fallback_descriptors_are_unranked_and_ineligible() {
        let catalog = built_in_catalog().unwrap();
        let resolved = catalog.resolve_with_provenance(
            "clippy::future_lint",
            &Category::Style,
            Severity::Warning,
            Some(AdapterProvenance::Clippy),
        );
        let descriptor = resolved.as_descriptor();
        assert!(descriptor.trust.priority.is_none());
        assert!(!descriptor.trust.score_eligible);
        assert_eq!(descriptor.trust.aggregation, AggregationPolicy::AuditOnly);
        assert!(descriptor.surface_policy.local);
        assert!(descriptor.surface_policy.sarif);
        assert!(descriptor.surface_policy.mcp);
        assert!(!descriptor.surface_policy.score);
        assert!(!descriptor.surface_policy.ci);
        assert!(!descriptor.surface_policy.pull_request);
        assert!(!descriptor.surface_policy.audit_only);
        assert_eq!(
            descriptor.surface_policy.reason_code,
            "unmapped-local-advisory"
        );
    }

    #[test]
    fn every_catalog_rule_has_one_complete_surface_decision() {
        let catalog = built_in_catalog().unwrap();
        let mut ids = Vec::new();
        for descriptor in catalog.descriptors() {
            let policy = &descriptor.surface_policy;
            assert!(
                !policy.reason_code.is_empty(),
                "{}",
                descriptor.canonical_id
            );
            assert_eq!(
                policy.score, descriptor.trust.score_eligible,
                "{}",
                descriptor.canonical_id
            );
            assert!(
                policy.local || policy.sarif || policy.mcp,
                "{} is invisible on every inspection surface",
                descriptor.canonical_id
            );
            ids.push(descriptor.canonical_id.as_str());
        }
        assert!(
            ids.is_sorted(),
            "catalog iteration must be inventory-stable"
        );
    }

    #[test]
    fn named_high_volume_defaults_are_advisory_without_changing_ids() {
        let catalog = built_in_catalog().unwrap();
        for rule in [
            "excessive-clone",
            "clippy::let_underscore_must_use",
            "clippy::indexing_slicing",
            "deny-ban",
        ] {
            let descriptor = catalog.exact(rule).unwrap_or_else(|| panic!("{rule}"));
            assert_eq!(descriptor.canonical_id, rule);
            assert!(!descriptor.trust.score_eligible, "{rule}");
            assert!(descriptor.advisory, "{rule}");
            assert!(descriptor.surface_policy.local, "{rule}");
            assert!(descriptor.surface_policy.sarif, "{rule}");
            assert!(descriptor.surface_policy.mcp, "{rule}");
            assert!(!descriptor.surface_policy.score, "{rule}");
            assert!(!descriptor.surface_policy.ci, "{rule}");
            assert!(!descriptor.surface_policy.pull_request, "{rule}");
            assert_eq!(
                descriptor.surface_policy.migration,
                MigrationDisposition::Advisory,
                "{rule}"
            );
            assert_eq!(
                descriptor.surface_policy.reason_code, "unqualified-high-volume",
                "{rule}"
            );
            assert!(
                descriptor.fix_guidance.contains("Sample representative"),
                "{rule}"
            );
        }
    }

    #[test]
    fn compiler_provenance_and_error_severity_do_not_grant_product_authority() {
        let catalog = built_in_catalog().unwrap();
        let descriptor = catalog
            .exact("clippy::let_underscore_must_use")
            .expect("curated Clippy rule");
        assert_eq!(descriptor.trust.tier, TrustTier::CompilerProven);
        assert_eq!(descriptor.default_severity, Severity::Warning);
        assert!(!descriptor.surface_policy.score);
        assert!(!descriptor.surface_policy.ci);

        let deny = catalog.exact("deny-ban").expect("cargo-deny ban identity");
        assert_eq!(deny.default_severity, Severity::Error);
        assert!(!deny.surface_policy.score);
        assert!(!deny.surface_policy.ci);
    }

    #[test]
    fn impossible_surface_policies_fail_catalog_construction() {
        let mut missing_reason = descriptor("missing-decision", &[]);
        missing_reason.surface_policy.reason_code = "";
        let error = RuleCatalog::build(vec![missing_reason]).unwrap_err();
        assert!(matches!(
            error,
            CatalogError::TrustContract { ref reason, .. }
                if reason.contains("no reason code")
        ));

        let catalog = built_in_catalog().unwrap();
        let mut audit = catalog
            .exact("unsafe-block-audit")
            .expect("audit rule")
            .clone();
        audit.trust.score_eligible = true;
        audit.surface_policy.score = true;
        audit.surface_policy.ci = true;
        let error = RuleCatalog::build(vec![audit]).unwrap_err();
        assert!(matches!(
            error,
            CatalogError::TrustContract { ref reason, .. }
                if reason.contains("audit-only rule is actionable")
        ));
    }

    // ----------------------------------------------------------------------
    // US-007: requalification of every current custom rule
    // ----------------------------------------------------------------------

    #[test]
    fn every_custom_rule_carries_a_recorded_requalification_decision() {
        let catalog = built_in_catalog().unwrap();
        let artifact = trust::calibration().expect("calibration artifact");
        for descriptor in catalog.descriptors() {
            if descriptor.analyzer_kind != AnalyzerKind::SynAst {
                continue;
            }
            let record = trust::calibration_for(&descriptor.canonical_id).unwrap_or_else(|| {
                panic!("{} has no requalification record", descriptor.canonical_id)
            });
            assert!(
                !record.rationale.is_empty(),
                "{} records a decision without a rationale",
                descriptor.canonical_id
            );
            assert_eq!(
                record.decision.is_default(),
                descriptor.default_enabled,
                "{} activation disagrees with its decision",
                descriptor.canonical_id
            );
            let expected = record.passes(&artifact.thresholds)
                && !trust::is_unqualified_high_volume(&descriptor.canonical_id);
            assert_eq!(
                descriptor.trust.score_eligible, expected,
                "{} score eligibility disagrees with its measurement and surface decision",
                descriptor.canonical_id
            );
        }
    }

    #[test]
    fn a_default_rule_without_measured_recall_is_advisory_and_never_scores() {
        let catalog = built_in_catalog().unwrap();
        for descriptor in catalog.descriptors() {
            if descriptor.analyzer_kind != AnalyzerKind::SynAst || !descriptor.default_enabled {
                continue;
            }
            let record = trust::calibration_for(&descriptor.canonical_id).unwrap();
            if record.recall.is_none() {
                assert!(
                    !descriptor.trust.score_eligible,
                    "{} scores without measured recall",
                    descriptor.canonical_id
                );
                assert!(
                    descriptor.advisory,
                    "{} is a non-scoring default that is not labelled advisory",
                    descriptor.canonical_id
                );
            }
        }
    }

    #[test]
    fn promoting_an_unqualified_rule_to_score_eligible_fails_validation() {
        let mut descriptor = descriptor("unqualified", &[]);
        descriptor.analyzer_kind = AnalyzerKind::SynAst;
        descriptor.default_enabled = true;
        let error = RuleCatalog::build(vec![descriptor]).unwrap_err();
        assert!(matches!(
            error,
            CatalogError::TrustContract { ref reason, .. }
                if reason.contains("recorded requalification decision")
        ));
    }

    #[test]
    fn a_decision_that_contradicts_default_activation_fails_validation() {
        let mut descriptor = descriptor("unwrap-in-production", &[]);
        descriptor.analyzer_kind = AnalyzerKind::SynAst;
        descriptor.default_enabled = false;
        let error = RuleCatalog::build(vec![descriptor]).unwrap_err();
        assert!(matches!(
            error,
            CatalogError::TrustContract { ref reason, .. }
                if reason.contains("disagrees with the recorded requalification decision")
        ));
    }

    #[test]
    fn high_volume_rules_declare_an_explicit_aggregation_policy() {
        let catalog = built_in_catalog().unwrap();
        // No blanket policy: each high-volume identity states how repetition is
        // bounded, and the audit-only ones state that they never score.
        for (rule, expected) in [
            ("excessive-clone", AggregationPolicy::BoundedOccurrence),
            ("unwrap-in-production", AggregationPolicy::BoundedOccurrence),
            (
                "high-cyclomatic-complexity",
                AggregationPolicy::BoundedOccurrence,
            ),
            (
                "clippy::indexing_slicing",
                AggregationPolicy::BoundedOccurrence,
            ),
            ("unsafe-dependency", AggregationPolicy::AuditOnly),
            ("unsafe-block-audit", AggregationPolicy::AuditOnly),
        ] {
            let descriptor = catalog
                .exact(rule)
                .unwrap_or_else(|| panic!("{rule} is missing from the catalog"));
            assert_eq!(descriptor.trust.aggregation, expected, "{rule}");
            assert!(
                !descriptor.trust.supported_contexts.is_empty(),
                "{rule} declares no applicability contract"
            );
        }
    }

    // ------------------------------------------------------------------
    // Promotion and demotion gates (US-012)
    // ------------------------------------------------------------------

    /// The release blocker: every default or score-eligible rule in the
    /// shipping catalog must hold current authority.
    #[test]
    fn every_default_or_scored_rule_holds_current_gate_authority() {
        let failures = promotion_gate_failures("2026-07-27").unwrap();
        assert!(
            failures.is_empty(),
            "rules without current gate authority: {:?}",
            failures
                .iter()
                .map(|failure| format!("{} ({}): {}", failure.rule, failure.code, failure.detail))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn no_rule_is_currently_proposed_for_demotion() {
        assert!(proposed_demotions("2026-07-27").unwrap().is_empty());
    }

    /// A historical rule gets no exception: the gate reads the same evidence
    /// for a rule that predates this program as for a new one (AC-6).
    #[test]
    fn a_historical_rule_is_qualified_on_the_same_evidence_as_a_new_one() {
        let catalog = built_in_catalog().unwrap();
        let descriptor = catalog
            .exact("unwrap-in-production")
            .expect("a rule that predates this program");
        let subject = descriptor.gate_subject();
        assert!(matches!(
            trust::evaluate_gate(&subject, "2026-07-27"),
            trust::GateVerdict::Passing(trust::GateAuthority::Calibration)
        ));

        // Strip the measurement and the same rule immediately fails.
        let mut unmeasured = descriptor.clone();
        unmeasured.canonical_id = "never-calibrated-rule".to_string();
        unmeasured.trust.score_eligible = true;
        match trust::evaluate_gate(&unmeasured.gate_subject(), "2026-07-27") {
            trust::GateVerdict::Failing(failure) => {
                assert_eq!(failure.code, "missing-calibration");
            }
            other => panic!("expected a gate failure, got {other:?}"),
        }
    }

    #[test]
    fn a_compiler_rule_losing_conformance_loses_authority() {
        let catalog = built_in_catalog().unwrap();
        let descriptor = catalog.exact("compiler-error").expect("compiler rule");
        assert!(matches!(
            trust::evaluate_gate(&descriptor.gate_subject(), "2026-07-27"),
            trust::GateVerdict::Passing(trust::GateAuthority::CompilerConformance)
        ));

        // A provider with no conformance row cannot carry compiler authority,
        // whatever its corpus precision (AC-4).
        let mut drifted = descriptor.clone();
        drifted.provider = "unmapped-analyzer".to_string();
        drifted.analyzer_kind = AnalyzerKind::External;
        drifted.trust.required_evidence = vec![RequiredEvidence::AdvisoryDatabase];
        drifted.trust.tier = TrustTier::CompilerProven;
        match trust::evaluate_gate(&drifted.gate_subject(), "2026-07-27") {
            trust::GateVerdict::Failing(failure) => {
                assert_eq!(failure.code, "conformance-missing");
            }
            other => panic!("expected a conformance failure, got {other:?}"),
        }
    }

    #[test]
    fn an_advisory_rule_without_advisory_evidence_cannot_stay_eligible() {
        let catalog = built_in_catalog().unwrap();
        let descriptor = catalog.exact("deny-advisory").expect("advisory rule");
        assert!(matches!(
            trust::evaluate_gate(&descriptor.gate_subject(), "2026-07-27"),
            trust::GateVerdict::Passing(trust::GateAuthority::Advisory)
        ));

        let mut stripped = descriptor.clone();
        stripped.trust.required_evidence = vec![RequiredEvidence::SynAst];
        match trust::evaluate_gate(&stripped.gate_subject(), "2026-07-27") {
            trust::GateVerdict::Failing(failure) => {
                assert_eq!(failure.code, "missing-advisory-evidence");
            }
            other => panic!("expected an advisory failure, got {other:?}"),
        }
    }

    #[test]
    fn an_audit_only_rule_that_claims_score_eligibility_fails_the_gate() {
        let catalog = built_in_catalog().unwrap();
        let mut descriptor = catalog
            .exact("unsafe-block-audit")
            .expect("audit rule")
            .clone();
        assert!(matches!(
            trust::evaluate_gate(&descriptor.gate_subject(), "2026-07-27"),
            trust::GateVerdict::Passing(trust::GateAuthority::AuditOnly)
        ));
        descriptor.trust.score_eligible = true;
        match trust::evaluate_gate(&descriptor.gate_subject(), "2026-07-27") {
            trust::GateVerdict::Failing(failure) => {
                assert_eq!(failure.code, "audit-only-scored");
            }
            other => panic!("expected an audit-only failure, got {other:?}"),
        }
    }

    #[test]
    fn catalog_ids_and_aliases_are_unique() {
        let catalog = built_in_catalog().unwrap();
        let mut seen = HashSet::new();
        for descriptor in catalog.descriptors() {
            assert!(seen.insert(descriptor.canonical_id.as_str()));
            for alias in &descriptor.aliases {
                assert!(seen.insert(alias.as_str()));
            }
        }
    }
}
