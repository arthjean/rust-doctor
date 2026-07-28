//! Normative trust vocabulary shared by the rule catalog, Score Core V2, and
//! every report consumer.
//!
//! The contract in `tasks/prd-diagnostic-trust-parity.md` keeps trust tier,
//! severity, confidence, priority, score eligibility, and category independent.
//! Nothing in this module infers one from another at report time: the tables
//! below are the single declarative source and catalog validation refuses any
//! score-eligible rule that lacks its evidence contract.
#![expect(
    clippy::redundant_pub_crate,
    reason = "trust vocabulary is consumed by sibling modules through this private crate module"
)]

use crate::catalog::AnalyzerKind;
use crate::diagnostics::{Category, Severity, SourceSurface};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

/// Provenance class of a rule's evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TrustTier {
    /// rustc, Cargo, or Clippy structured evidence.
    CompilerProven,
    /// File-local heuristic backed by a measured calibration artifact.
    CalibratedHeuristic,
    /// Advisory database or declarative dependency policy evidence.
    AdvisoryBacked,
    /// Observation reported for review, never asserted as a defect.
    AuditOnly,
}

impl TrustTier {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::CompilerProven => "compiler-proven",
            Self::CalibratedHeuristic => "calibrated-heuristic",
            Self::AdvisoryBacked => "advisory-backed",
            Self::AuditOnly => "audit-only",
        }
    }
}

/// Product urgency, deliberately independent from presentation severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Priority {
    P0,
    P1,
    P2,
    P3,
}

impl Priority {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::P0 => "p0",
            Self::P1 => "p1",
            Self::P2 => "p2",
            Self::P3 => "p3",
        }
    }
}

/// How repeated findings from one rule influence ranking and score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AggregationPolicy {
    /// One penalty per distinct root-cause key.
    RootCause,
    /// Repeated occurrences saturate against the model's multiplier cap.
    BoundedOccurrence,
    /// One penalty per violated rule, whatever the occurrence count.
    UniqueRule,
    /// Observed and reported, never scored.
    AuditOnly,
}

impl AggregationPolicy {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::RootCause => "root-cause",
            Self::BoundedOccurrence => "bounded-occurrence",
            Self::UniqueRule => "unique-rule",
            Self::AuditOnly => "audit-only",
        }
    }
}

/// Evidence a rule needs before it can decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RequiredEvidence {
    SynAst,
    CompilerJson,
    CargoMetadata,
    AdvisoryDatabase,
    TypeResolution,
    MacroExpansion,
    Interprocedural,
    MirDataflow,
}

impl RequiredEvidence {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::SynAst => "syn-ast",
            Self::CompilerJson => "compiler-json",
            Self::CargoMetadata => "cargo-metadata",
            Self::AdvisoryDatabase => "advisory-database",
            Self::TypeResolution => "type-resolution",
            Self::MacroExpansion => "macro-expansion",
            Self::Interprocedural => "interprocedural",
            Self::MirDataflow => "mir-dataflow",
        }
    }

    /// True when the analyzer that owns the rule can actually supply this
    /// evidence on the stable toolchain. `syn` cannot resolve types, expand
    /// macros, or reason across functions, so a `syn` rule that declares one of
    /// those needs is refused score eligibility by catalog validation.
    pub(crate) const fn supplied_by(self, analyzer: AnalyzerKind) -> bool {
        match self {
            Self::SynAst => matches!(analyzer, AnalyzerKind::SynAst),
            Self::CompilerJson => matches!(
                analyzer,
                AnalyzerKind::Clippy | AnalyzerKind::Project | AnalyzerKind::External
            ),
            Self::CargoMetadata => matches!(
                analyzer,
                AnalyzerKind::Project | AnalyzerKind::Dependency | AnalyzerKind::Clippy
            ),
            Self::AdvisoryDatabase => matches!(analyzer, AnalyzerKind::Dependency),
            Self::TypeResolution | Self::MacroExpansion | Self::Interprocedural => {
                matches!(analyzer, AnalyzerKind::Clippy)
            }
            Self::MirDataflow => false,
        }
    }
}

/// Whether the analyzer ships with Rust Doctor and the Rust toolchain, or is an
/// optional executable whose presence must never move the Core Score
/// (FR-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AnalyzerAvailability {
    Core,
    OptionalExternal,
}

impl AnalyzerAvailability {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::OptionalExternal => "optional-external",
        }
    }
}

/// Declarative trust contract for one canonical rule identity.
#[derive(Debug, Clone)]
pub(crate) struct TrustProfile {
    pub(crate) tier: TrustTier,
    /// `None` means unranked: unknown and dynamically discovered rules stay
    /// observable but never receive a fabricated priority.
    pub(crate) priority: Option<Priority>,
    pub(crate) aggregation: AggregationPolicy,
    pub(crate) required_evidence: Vec<RequiredEvidence>,
    pub(crate) availability: AnalyzerAvailability,
    pub(crate) supported_contexts: Vec<SourceSurface>,
}

/// Source surfaces every file-local production rule supports.
const PRODUCTION_SURFACES: &[SourceSurface] = &[SourceSurface::Library, SourceSurface::Binary];

/// Surfaces a project-scoped or dependency-scoped analyzer covers.
const PROJECT_SURFACES: &[SourceSurface] = &[
    SourceSurface::Library,
    SourceSurface::Binary,
    SourceSurface::BuildScript,
];

const SYN_EVIDENCE: &[RequiredEvidence] = &[RequiredEvidence::SynAst];
const COMPILER_EVIDENCE: &[RequiredEvidence] = &[RequiredEvidence::CompilerJson];
const METADATA_EVIDENCE: &[RequiredEvidence] = &[RequiredEvidence::CargoMetadata];
const ADVISORY_EVIDENCE: &[RequiredEvidence] = &[
    RequiredEvidence::AdvisoryDatabase,
    RequiredEvidence::CargoMetadata,
];

/// Trust table for every built-in rule that is not a Clippy lint.
///
/// Columns: rule, tier, priority, aggregation, evidence, availability, contexts.
type TrustRow = (
    &'static str,
    TrustTier,
    Option<Priority>,
    AggregationPolicy,
    &'static [RequiredEvidence],
    AnalyzerAvailability,
    &'static [SourceSurface],
);

#[expect(
    clippy::too_many_lines,
    reason = "the trust table is declarative catalog data kept in one auditable place"
)]
fn trust_rows() -> Vec<TrustRow> {
    use AggregationPolicy::{AuditOnly as AggAuditOnly, BoundedOccurrence, RootCause, UniqueRule};
    use AnalyzerAvailability::{Core, OptionalExternal};
    use Priority::{P0, P1, P2, P3};
    use TrustTier::{AdvisoryBacked, AuditOnly, CalibratedHeuristic, CompilerProven};

    vec![
        // ── Error handling ────────────────────────────────────────────────
        (
            "unwrap-in-production",
            CalibratedHeuristic,
            Some(P2),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "panic-in-library",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            &[SourceSurface::Library],
        ),
        (
            "box-dyn-error-in-public-api",
            CalibratedHeuristic,
            Some(P2),
            UniqueRule,
            SYN_EVIDENCE,
            Core,
            &[SourceSurface::Library],
        ),
        (
            "result-unit-error",
            CalibratedHeuristic,
            Some(P2),
            UniqueRule,
            SYN_EVIDENCE,
            Core,
            &[SourceSurface::Library],
        ),
        // ── Performance ───────────────────────────────────────────────────
        (
            "excessive-clone",
            CalibratedHeuristic,
            Some(P2),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "unnecessary-allocation",
            CalibratedHeuristic,
            Some(P2),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "collect-then-iterate",
            CalibratedHeuristic,
            Some(P2),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "large-enum-variant",
            CalibratedHeuristic,
            Some(P2),
            BoundedOccurrence,
            // Variant size cannot be proven without layout and type evidence.
            &[RequiredEvidence::SynAst, RequiredEvidence::TypeResolution],
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "string-from-literal",
            CalibratedHeuristic,
            Some(P3),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "regex-created-in-loop",
            CalibratedHeuristic,
            Some(P2),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "unbounded-collect",
            CalibratedHeuristic,
            Some(P2),
            BoundedOccurrence,
            &[RequiredEvidence::SynAst, RequiredEvidence::Interprocedural],
            Core,
            PRODUCTION_SURFACES,
        ),
        // ── Architecture ──────────────────────────────────────────────────
        (
            "high-cyclomatic-complexity",
            CalibratedHeuristic,
            Some(P3),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        // ── Security ──────────────────────────────────────────────────────
        (
            "hardcoded-secrets",
            CalibratedHeuristic,
            Some(P0),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "sql-injection-risk",
            CalibratedHeuristic,
            Some(P0),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "command-shell-interpolation",
            CalibratedHeuristic,
            Some(P0),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "unsafe-block-audit",
            // Presence of `unsafe` is an observation, never a proven defect.
            AuditOnly,
            Some(P3),
            AggAuditOnly,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "insecure-http-client",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "ffi-cstring-lifetime",
            CalibratedHeuristic,
            Some(P0),
            BoundedOccurrence,
            &[RequiredEvidence::SynAst, RequiredEvidence::Interprocedural],
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "temporary-cstring-pointer",
            CalibratedHeuristic,
            Some(P0),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        // ── Correctness ───────────────────────────────────────────────────
        (
            "catch-unwind-discarded",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "mem-forget-resource",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            &[RequiredEvidence::SynAst, RequiredEvidence::TypeResolution],
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "process-exit-in-library",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            &[SourceSurface::Library],
        ),
        // ── Async ─────────────────────────────────────────────────────────
        (
            "blocking-in-async",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            &[RequiredEvidence::SynAst, RequiredEvidence::TypeResolution],
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "block-on-in-async",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "blocking-lock-in-async",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            &[RequiredEvidence::SynAst, RequiredEvidence::TypeResolution],
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "await-holding-refcell-ref",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            &[RequiredEvidence::SynAst, RequiredEvidence::TypeResolution],
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "spawn-in-drop",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        // ── Framework packs ───────────────────────────────────────────────
        (
            "tokio-main-missing",
            CalibratedHeuristic,
            Some(P1),
            UniqueRule,
            SYN_EVIDENCE,
            Core,
            &[SourceSurface::Binary],
        ),
        (
            "tokio-spawn-without-move",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "tokio-unbounded-channel",
            CalibratedHeuristic,
            Some(P2),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "axum-handler-not-async",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "axum-extension-request-state",
            CalibratedHeuristic,
            Some(P2),
            BoundedOccurrence,
            SYN_EVIDENCE,
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "actix-blocking-handler",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            &[RequiredEvidence::SynAst, RequiredEvidence::TypeResolution],
            Core,
            PRODUCTION_SURFACES,
        ),
        (
            "actix-web-data-lock",
            CalibratedHeuristic,
            Some(P1),
            BoundedOccurrence,
            &[RequiredEvidence::SynAst, RequiredEvidence::TypeResolution],
            Core,
            PRODUCTION_SURFACES,
        ),
        // ── Compiler-backed project analyzers ─────────────────────────────
        (
            "compiler-error",
            CompilerProven,
            Some(P0),
            BoundedOccurrence,
            COMPILER_EVIDENCE,
            Core,
            PROJECT_SURFACES,
        ),
        (
            "compiler-ice",
            CompilerProven,
            Some(P0),
            BoundedOccurrence,
            COMPILER_EVIDENCE,
            Core,
            PROJECT_SURFACES,
        ),
        (
            "unknown-rustc-level",
            AuditOnly,
            None,
            AggAuditOnly,
            COMPILER_EVIDENCE,
            Core,
            PROJECT_SURFACES,
        ),
        (
            "missing-msrv",
            CompilerProven,
            Some(P2),
            UniqueRule,
            METADATA_EVIDENCE,
            Core,
            PROJECT_SURFACES,
        ),
        (
            "msrv-outdated",
            CompilerProven,
            Some(P2),
            UniqueRule,
            METADATA_EVIDENCE,
            Core,
            PROJECT_SURFACES,
        ),
        (
            "msrv-incompatible",
            CompilerProven,
            Some(P0),
            UniqueRule,
            METADATA_EVIDENCE,
            Core,
            PROJECT_SURFACES,
        ),
        (
            "skipped-pass",
            AuditOnly,
            None,
            AggAuditOnly,
            METADATA_EVIDENCE,
            Core,
            PROJECT_SURFACES,
        ),
        // ── Optional external adapters ────────────────────────────────────
        (
            "deny-advisory",
            AdvisoryBacked,
            Some(P0),
            RootCause,
            ADVISORY_EVIDENCE,
            OptionalExternal,
            PROJECT_SURFACES,
        ),
        (
            "deny-license",
            AdvisoryBacked,
            Some(P2),
            RootCause,
            ADVISORY_EVIDENCE,
            OptionalExternal,
            PROJECT_SURFACES,
        ),
        (
            "deny-ban",
            AdvisoryBacked,
            Some(P2),
            RootCause,
            ADVISORY_EVIDENCE,
            OptionalExternal,
            PROJECT_SURFACES,
        ),
        (
            "deny-source",
            AdvisoryBacked,
            Some(P2),
            RootCause,
            ADVISORY_EVIDENCE,
            OptionalExternal,
            PROJECT_SURFACES,
        ),
        (
            "deny-unknown",
            AdvisoryBacked,
            Some(P3),
            RootCause,
            ADVISORY_EVIDENCE,
            OptionalExternal,
            PROJECT_SURFACES,
        ),
        (
            "unused-dependency",
            AdvisoryBacked,
            Some(P2),
            RootCause,
            METADATA_EVIDENCE,
            OptionalExternal,
            PROJECT_SURFACES,
        ),
        (
            "unsafe-dependency",
            // cargo-geiger measures unsafe exposure, never a confirmed defect.
            AuditOnly,
            Some(P3),
            AggAuditOnly,
            METADATA_EVIDENCE,
            OptionalExternal,
            PROJECT_SURFACES,
        ),
        (
            "semver-violation",
            AdvisoryBacked,
            Some(P1),
            RootCause,
            METADATA_EVIDENCE,
            OptionalExternal,
            PROJECT_SURFACES,
        ),
        (
            "low-coverage",
            AuditOnly,
            None,
            AggAuditOnly,
            METADATA_EVIDENCE,
            OptionalExternal,
            PROJECT_SURFACES,
        ),
        (
            "uncovered-file",
            AuditOnly,
            None,
            AggAuditOnly,
            METADATA_EVIDENCE,
            OptionalExternal,
            PROJECT_SURFACES,
        ),
    ]
}

static TRUST_TABLE: LazyLock<HashMap<&'static str, TrustProfile>> = LazyLock::new(|| {
    trust_rows()
        .into_iter()
        .map(
            |(rule, tier, priority, aggregation, evidence, availability, contexts)| {
                (
                    rule,
                    TrustProfile {
                        tier,
                        priority,
                        aggregation,
                        required_evidence: evidence.to_vec(),
                        availability,
                        supported_contexts: contexts.to_vec(),
                    },
                )
            },
        )
        .collect()
});

/// Trust profile for a built-in non-Clippy rule.
pub(crate) fn profile_for(rule: &str) -> Option<&'static TrustProfile> {
    TRUST_TABLE.get(rule)
}

/// Trust profile derived from a curated Clippy lint entry. Clippy evidence is
/// compiler-proven, so priority follows the curated category and severity
/// instead of a calibration artifact.
pub(crate) fn clippy_profile(category: &Category, severity: Severity) -> TrustProfile {
    let priority = match (category, severity) {
        (Category::Security, Severity::Error) => Priority::P0,
        (_, Severity::Error) => Priority::P1,
        (_, Severity::Warning) => Priority::P2,
        (_, Severity::Info) => Priority::P3,
    };
    TrustProfile {
        tier: TrustTier::CompilerProven,
        priority: Some(priority),
        aggregation: AggregationPolicy::BoundedOccurrence,
        required_evidence: COMPILER_EVIDENCE.to_vec(),
        availability: AnalyzerAvailability::Core,
        supported_contexts: PROJECT_SURFACES.to_vec(),
    }
}

/// Unknown, dynamic, or unmapped rules stay observable, unranked, and
/// score-ineligible (FR-002).
pub(crate) fn unranked_profile(analyzer: AnalyzerKind) -> TrustProfile {
    TrustProfile {
        tier: TrustTier::AuditOnly,
        priority: None,
        aggregation: AggregationPolicy::AuditOnly,
        required_evidence: match analyzer {
            AnalyzerKind::SynAst => SYN_EVIDENCE.to_vec(),
            AnalyzerKind::Clippy | AnalyzerKind::External => COMPILER_EVIDENCE.to_vec(),
            AnalyzerKind::Dependency | AnalyzerKind::Project => METADATA_EVIDENCE.to_vec(),
        },
        availability: match analyzer {
            AnalyzerKind::Dependency | AnalyzerKind::External => {
                AnalyzerAvailability::OptionalExternal
            }
            AnalyzerKind::SynAst | AnalyzerKind::Clippy | AnalyzerKind::Project => {
                AnalyzerAvailability::Core
            }
        },
        supported_contexts: PROJECT_SURFACES.to_vec(),
    }
}

// ---------------------------------------------------------------------------
// Calibration artifact
// ---------------------------------------------------------------------------

/// Measured gate thresholds shared by calibration and promotion validation.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct CalibrationThresholds {
    pub(crate) confidence_level: f64,
    pub(crate) max_false_positive_rate: f64,
    pub(crate) min_recall: f64,
    pub(crate) min_positive_samples: usize,
    pub(crate) min_negative_samples: usize,
    pub(crate) min_context_coverage: f64,
}

/// The requalification verdict recorded for one rule.
///
/// Every custom rule carries one, historical rules included: US-007 grants no
/// grandfathering, so a rule that ships enabled without measured evidence is an
/// explicit advisory default rather than an unqualified scorer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RuleDecision {
    /// Measured against its labeled dataset and cleared to move the score.
    ScoreEligibleDefault,
    /// Runs by default, reported as advisory, contributes exactly zero.
    NonScoringDefault,
    /// Ships disabled; a project must enable it explicitly.
    OptIn,
    /// Withdrawn from the default catalog entirely.
    Disabled,
}

impl RuleDecision {
    /// A decision that keeps the rule enabled out of the box.
    pub(crate) const fn is_default(self) -> bool {
        matches!(self, Self::ScoreEligibleDefault | Self::NonScoringDefault)
    }
}

/// Per-rule measured evidence produced by the truth baseline job.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct CalibrationRecord {
    pub(crate) rule: String,
    pub(crate) decision: RuleDecision,
    pub(crate) evidence_complete: bool,
    pub(crate) true_positives: usize,
    pub(crate) false_positives: usize,
    pub(crate) false_negatives: usize,
    pub(crate) positive_samples: usize,
    pub(crate) negative_samples: usize,
    pub(crate) precision: Option<f64>,
    pub(crate) recall: Option<f64>,
    pub(crate) false_positive_rate: Option<f64>,
    pub(crate) false_positive_upper_bound: Option<f64>,
    pub(crate) confidence_level: f64,
    pub(crate) context_coverage: f64,
    pub(crate) reviewer: String,
    pub(crate) reviewed_at: String,
    #[serde(default)]
    pub(crate) rationale: String,
}

impl CalibrationRecord {
    /// A record only grants score eligibility when every measured gate is
    /// satisfied. Missing metrics never count as success (US-012).
    pub(crate) fn passes(&self, thresholds: &CalibrationThresholds) -> bool {
        if !self.evidence_complete || self.decision != RuleDecision::ScoreEligibleDefault {
            return false;
        }
        let (Some(recall), Some(false_positive_upper_bound)) =
            (self.recall, self.false_positive_upper_bound)
        else {
            return false;
        };
        self.positive_samples >= thresholds.min_positive_samples
            && self.negative_samples >= thresholds.min_negative_samples
            && recall >= thresholds.min_recall
            && (self.confidence_level - thresholds.confidence_level).abs() < f64::EPSILON
            && false_positive_upper_bound <= thresholds.max_false_positive_rate
            && self.context_coverage >= thresholds.min_context_coverage
    }
}

/// The reviewed calibration artifact compiled into the binary.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct CalibrationArtifact {
    pub(crate) schema_version: String,
    pub(crate) calibration_version: String,
    pub(crate) dataset_version: String,
    pub(crate) toolchain: String,
    pub(crate) thresholds: CalibrationThresholds,
    pub(crate) rules: Vec<CalibrationRecord>,
}

const CALIBRATION_SOURCE: &str = include_str!("../evaluation/calibration-v1.json");

static CALIBRATION: LazyLock<Option<CalibrationArtifact>> =
    LazyLock::new(|| serde_json::from_str(CALIBRATION_SOURCE).ok());

pub(crate) fn calibration() -> Option<&'static CalibrationArtifact> {
    CALIBRATION.as_ref()
}

/// Reviewed calibration record for one rule, if the artifact declares it.
pub(crate) fn calibration_for(rule: &str) -> Option<&'static CalibrationRecord> {
    calibration()?
        .rules
        .iter()
        .find(|record| record.rule == rule)
}

/// Calibration version attached to a rule descriptor. Only a passing record
/// exposes a version: an unmeasured or failing rule must not look calibrated.
pub(crate) fn calibration_version_for(rule: &str) -> Option<String> {
    let artifact = calibration()?;
    let record = calibration_for(rule)?;
    record
        .passes(&artifact.thresholds)
        .then(|| artifact.calibration_version.clone())
}

/// The recorded requalification decision for one rule, if the artifact declares
/// it. A rule with no record is unqualified: catalog validation refuses to ship
/// it as a default (US-007 AC-1).
pub(crate) fn decision_for(rule: &str) -> Option<RuleDecision> {
    calibration_for(rule).map(|record| record.decision)
}

/// Resolve score eligibility from the trust contract alone.
///
/// A rule is score-eligible when its tier's evidence contract is satisfied:
/// compiler-proven and advisory-backed rules carry authority from their
/// analyzer, a calibrated heuristic needs a passing calibration record, and
/// audit-only rules never score. Declaring evidence the owning analyzer cannot
/// supply also removes eligibility (US-006 AC-4).
pub(crate) fn resolve_score_eligibility(
    rule: &str,
    profile: &TrustProfile,
    analyzer: AnalyzerKind,
) -> bool {
    if is_unqualified_high_volume(rule) {
        return false;
    }
    if profile.aggregation == AggregationPolicy::AuditOnly {
        return false;
    }
    if !profile
        .required_evidence
        .iter()
        .all(|evidence| evidence.supplied_by(analyzer))
    {
        return false;
    }
    match profile.tier {
        TrustTier::AuditOnly => false,
        TrustTier::CompilerProven | TrustTier::AdvisoryBacked => true,
        TrustTier::CalibratedHeuristic => calibration_version_for(rule).is_some(),
    }
}

/// Rules whose detection evidence does not establish default action value.
///
/// They remain visible locally and in audit-capable exports. Promotion back to
/// score or CI requires a later, versioned catalog decision backed by the
/// independent qualification program.
pub(crate) fn is_unqualified_high_volume(rule: &str) -> bool {
    matches!(
        rule,
        "excessive-clone"
            | "clippy::let_underscore_must_use"
            | "clippy::indexing_slicing"
            | "deny-ban"
    )
}

// ---------------------------------------------------------------------------
// Promotion and demotion gates
// ---------------------------------------------------------------------------

/// Why a rule is allowed to ship enabled or to move the score.
///
/// Every default or score-eligible rule needs exactly one of these. There is no
/// fifth arm for "it was already here": a rule that predates this program is
/// qualified on the same evidence as a new one (US-012 AC-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GateAuthority {
    /// A passing calibration record on the current dataset.
    Calibration,
    /// Compiler or Cargo evidence whose parser still matches its conformance row.
    CompilerConformance,
    /// Advisory-database identity, or declarative policy evidence from a
    /// conforming tool.
    Advisory,
    /// The rule is audit-only: observable, never scored, nothing to qualify.
    AuditOnly,
}

/// A gate failure with a stable machine-readable reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GateFailure {
    pub(crate) rule: String,
    /// Stable reason code consumers can branch on.
    pub(crate) code: &'static str,
    pub(crate) detail: String,
}

impl GateFailure {
    fn new(rule: &str, code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            rule: rule.to_string(),
            code,
            detail: detail.into(),
        }
    }
}

/// Outcome of running one rule through its trust-tier gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GateVerdict {
    /// Neither default nor score-eligible: outside the gate's scope.
    NotGated,
    Passing(GateAuthority),
    /// An approved, time-bounded, owner-attributed exception applies.
    Exempt(TrustException),
    /// Fails its gate on the current evidence.
    Failing(GateFailure),
    /// Failed on the last two approved corpus revisions: the next default
    /// catalog should demote it (US-012 AC-3).
    DemotionProposed(GateFailure),
}

/// A deliberate, documented threshold exception.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct TrustException {
    pub(crate) rule: String,
    pub(crate) owner: String,
    /// ISO-8601 date the exception was approved.
    pub(crate) granted_at: String,
    /// ISO-8601 date the exception stops applying.
    pub(crate) expires_at: String,
    pub(crate) reason: String,
    /// Link or identifier for the evidence behind the decision.
    pub(crate) evidence: String,
}

impl TrustException {
    /// An exception missing an owner, a window, a reason, or evidence is not an
    /// exception, it is an untracked bypass.
    pub(crate) fn well_formed(&self) -> bool {
        !self.rule.is_empty()
            && !self.owner.is_empty()
            && !self.reason.is_empty()
            && !self.evidence.is_empty()
            && is_iso_date(&self.granted_at)
            && is_iso_date(&self.expires_at)
            && self.granted_at < self.expires_at
    }

    /// ISO-8601 dates compare correctly as strings, so the window check needs
    /// no calendar arithmetic.
    pub(crate) fn active_on(&self, today: &str) -> bool {
        self.well_formed() && today >= self.granted_at.as_str() && today < self.expires_at.as_str()
    }
}

fn is_iso_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes().iter().enumerate().all(|(index, byte)| {
            if index == 4 || index == 7 {
                *byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct TrustExceptions {
    pub(crate) schema_version: String,
    pub(crate) exceptions: Vec<TrustException>,
}

const EXCEPTIONS_SOURCE: &str = include_str!("../evaluation/gate-exceptions-v1.json");

static EXCEPTIONS: LazyLock<Option<TrustExceptions>> =
    LazyLock::new(|| serde_json::from_str(EXCEPTIONS_SOURCE).ok());

/// Approved exceptions still inside their window on `today`.
pub(crate) fn active_exceptions(today: &str) -> Vec<TrustException> {
    EXCEPTIONS.as_ref().map_or_else(Vec::new, |artifact| {
        artifact
            .exceptions
            .iter()
            .filter(|exception| exception.active_on(today))
            .cloned()
            .collect()
    })
}

fn exception_for(rule: &str, today: &str) -> Option<TrustException> {
    EXCEPTIONS
        .as_ref()?
        .exceptions
        .iter()
        .find_map(|exception| {
            (exception.rule == rule && exception.active_on(today)).then(|| exception.clone())
        })
}

/// Per-revision gate outcomes, oldest revision first.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct GateHistory {
    pub(crate) schema_version: String,
    /// Approved corpus revisions, oldest first.
    pub(crate) revisions: Vec<String>,
    pub(crate) rules: std::collections::BTreeMap<String, Vec<GateOutcome>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct GateOutcome {
    pub(crate) revision: String,
    pub(crate) passed: bool,
}

const HISTORY_SOURCE: &str = include_str!("../evaluation/gate-history-v1.json");

static HISTORY: LazyLock<Option<GateHistory>> =
    LazyLock::new(|| serde_json::from_str(HISTORY_SOURCE).ok());

pub(crate) fn gate_history() -> Option<&'static GateHistory> {
    HISTORY.as_ref()
}

/// How many approved revisions in a row this rule has failed, counting back
/// from the newest.
fn consecutive_failures(rule: &str) -> usize {
    let Some(history) = gate_history() else {
        return 0;
    };
    let Some(outcomes) = history.rules.get(rule) else {
        return 0;
    };
    history
        .revisions
        .iter()
        .rev()
        .map_while(|revision| {
            outcomes
                .iter()
                .find(|outcome| &outcome.revision == revision)
                .filter(|outcome| !outcome.passed)
        })
        .count()
}

/// Two consecutive failing revisions is the demotion trigger.
const DEMOTION_STREAK: usize = 2;

/// What the gate needs to know about one rule.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GateSubject<'a> {
    pub(crate) rule: &'a str,
    pub(crate) provider: &'a str,
    pub(crate) tier: TrustTier,
    pub(crate) analyzer: AnalyzerKind,
    pub(crate) default_enabled: bool,
    pub(crate) score_eligible: bool,
    pub(crate) required_evidence: &'a [RequiredEvidence],
}

/// Conformance row an analyzer's evidence must still match.
fn conformance_adapter(subject: &GateSubject<'_>) -> Option<&'static str> {
    if let Some(row) = crate::passes::conformance::entry(subject.provider) {
        return Some(row.adapter);
    }
    match subject.analyzer {
        AnalyzerKind::Clippy => Some("clippy"),
        AnalyzerKind::Project | AnalyzerKind::Dependency | AnalyzerKind::External => {
            if subject
                .required_evidence
                .contains(&RequiredEvidence::CompilerJson)
            {
                Some("rustc")
            } else if subject
                .required_evidence
                .contains(&RequiredEvidence::CargoMetadata)
            {
                Some("cargo-metadata")
            } else {
                None
            }
        }
        AnalyzerKind::SynAst => None,
    }
}

/// Parser contract an adapter currently declares, for the conformance check.
fn declared_parser_contract(adapter: &str) -> Option<&'static str> {
    match adapter {
        "clippy" | "rustc" => Some(crate::passes::static_analysis::clippy::PARSER_CONTRACT_VERSION),
        "cargo-audit" => Some(crate::passes::security::audit::CONTRACT.parser_contract_version),
        "cargo-deny" => Some(crate::passes::security::deny::CONTRACT.parser_contract_version),
        "cargo-geiger" => Some(crate::passes::security::geiger::CONTRACT.parser_contract_version),
        "cargo-shear" => Some(crate::passes::quality::shear::CONTRACT.parser_contract_version),
        "cargo-semver-checks" => {
            Some(crate::passes::quality::semver_checks::CONTRACT.parser_contract_version)
        }
        // Cargo metadata is read through the `cargo_metadata` crate at the
        // pinned format version rather than through an adapter contract.
        "cargo-metadata" => Some("cargo-metadata-format-1"),
        _ => None,
    }
}

/// Run one rule through the gate its trust tier defines.
///
/// Uncertainty never counts as success: a missing calibration record, an
/// unavailable metric, or a lost conformance row all fail (US-012 AC-8).
pub(crate) fn evaluate_gate(subject: &GateSubject<'_>, today: &str) -> GateVerdict {
    if !subject.default_enabled && !subject.score_eligible {
        return GateVerdict::NotGated;
    }
    if subject.tier == TrustTier::AuditOnly {
        return if subject.score_eligible {
            GateVerdict::Failing(GateFailure::new(
                subject.rule,
                "audit-only-scored",
                "an audit-only rule cannot be score-eligible",
            ))
        } else {
            GateVerdict::Passing(GateAuthority::AuditOnly)
        };
    }

    let verdict = match subject.tier {
        TrustTier::CalibratedHeuristic => calibration_gate(subject),
        TrustTier::CompilerProven => conformance_gate(subject, GateAuthority::CompilerConformance),
        TrustTier::AdvisoryBacked => advisory_gate(subject),
        TrustTier::AuditOnly => unreachable!("audit-only is handled above"),
    };

    match verdict {
        GateVerdict::Failing(failure) => exception_for(subject.rule, today).map_or_else(
            || {
                if consecutive_failures(subject.rule) >= DEMOTION_STREAK {
                    GateVerdict::DemotionProposed(failure.clone())
                } else {
                    GateVerdict::Failing(failure.clone())
                }
            },
            GateVerdict::Exempt,
        ),
        other => other,
    }
}

fn calibration_gate(subject: &GateSubject<'_>) -> GateVerdict {
    let Some(artifact) = calibration() else {
        return GateVerdict::Failing(GateFailure::new(
            subject.rule,
            "missing-calibration",
            "no calibration artifact is compiled into this build",
        ));
    };
    let Some(record) = calibration_for(subject.rule) else {
        return GateVerdict::Failing(GateFailure::new(
            subject.rule,
            "missing-calibration",
            "calibrated heuristic has no reviewed calibration record",
        ));
    };
    if !subject.score_eligible {
        // A non-scoring or opt-in heuristic still needs a recorded decision,
        // which the record above provides, but no threshold clearance.
        return GateVerdict::Passing(GateAuthority::Calibration);
    }
    if record.passes(&artifact.thresholds) {
        return GateVerdict::Passing(GateAuthority::Calibration);
    }
    GateVerdict::Failing(GateFailure::new(
        subject.rule,
        "threshold-breach",
        threshold_detail(record, &artifact.thresholds),
    ))
}

fn threshold_detail(record: &CalibrationRecord, thresholds: &CalibrationThresholds) -> String {
    let mut reasons = Vec::new();
    if !record.evidence_complete {
        reasons.push("evidence incomplete".to_string());
    }
    match record.recall {
        None => reasons.push("recall unavailable".to_string()),
        Some(recall) if recall < thresholds.min_recall => {
            reasons.push(format!("recall {recall:.3} < {:.3}", thresholds.min_recall));
        }
        Some(_) => {}
    }
    match record.false_positive_upper_bound {
        None => reasons.push("false-positive upper bound unavailable".to_string()),
        Some(rate) if rate > thresholds.max_false_positive_rate => reasons.push(format!(
            "false-positive upper bound {rate:.3} > {:.3}",
            thresholds.max_false_positive_rate
        )),
        Some(_) => {}
    }
    if (record.confidence_level - thresholds.confidence_level).abs() >= f64::EPSILON {
        reasons.push(format!(
            "confidence level {:.3} != {:.3}",
            record.confidence_level, thresholds.confidence_level
        ));
    }
    if record.context_coverage < thresholds.min_context_coverage {
        reasons.push(format!(
            "context coverage {:.3} < {:.3}",
            record.context_coverage, thresholds.min_context_coverage
        ));
    }
    if record.positive_samples < thresholds.min_positive_samples {
        reasons.push(format!(
            "{} reviewed positive opportunities < {}",
            record.positive_samples, thresholds.min_positive_samples
        ));
    }
    if record.negative_samples < thresholds.min_negative_samples {
        reasons.push(format!(
            "{} reviewed negative contexts < {}",
            record.negative_samples, thresholds.min_negative_samples
        ));
    }
    if reasons.is_empty() {
        "recorded decision is not score-eligible-default".to_string()
    } else {
        reasons.join("; ")
    }
}

fn conformance_gate(subject: &GateSubject<'_>, authority: GateAuthority) -> GateVerdict {
    let Some(adapter) = conformance_adapter(subject) else {
        return GateVerdict::Failing(GateFailure::new(
            subject.rule,
            "conformance-missing",
            "no conformance row covers this analyzer",
        ));
    };
    let Some(declared) = declared_parser_contract(adapter) else {
        return GateVerdict::Failing(GateFailure::new(
            subject.rule,
            "conformance-missing",
            format!("adapter '{adapter}' declares no parser contract"),
        ));
    };
    if crate::passes::conformance::conformant(adapter, declared) {
        GateVerdict::Passing(authority)
    } else {
        // Corpus precision is irrelevant here: a parser that drifted away from
        // its recorded conformance cannot speak authoritatively (AC-4).
        GateVerdict::Failing(GateFailure::new(
            subject.rule,
            "conformance-lost",
            format!("adapter '{adapter}' parses '{declared}' outside its conformance row"),
        ))
    }
}

fn advisory_gate(subject: &GateSubject<'_>) -> GateVerdict {
    if subject
        .required_evidence
        .contains(&RequiredEvidence::AdvisoryDatabase)
    {
        return GateVerdict::Passing(GateAuthority::Advisory);
    }
    // Declarative dependency policy is the other advisory-backed authority, and
    // it still has to come from a tool with a conformance row.
    if subject
        .required_evidence
        .contains(&RequiredEvidence::CargoMetadata)
    {
        return conformance_gate(subject, GateAuthority::Advisory);
    }
    GateVerdict::Failing(GateFailure::new(
        subject.rule,
        "missing-advisory-evidence",
        "advisory-backed rule declares neither advisory-database nor cargo-metadata evidence",
    ))
}

/// Today's UTC date as `YYYY-MM-DD`.
///
/// Exception windows are the only thing that needs a calendar, and pulling a
/// date dependency in for one comparison is not worth the binary size.
pub(crate) fn today_utc() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (year, month, day) = civil_from_days(i64::try_from(seconds / 86_400).unwrap_or(0));
    format!("{year:04}-{month:02}-{day:02}")
}

/// Days since the Unix epoch to a civil date (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = u32::try_from(day_of_year - (153 * shifted_month + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    })
    .unwrap_or(1);
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_artifact_parses_and_pins_its_dataset() {
        let artifact = calibration().expect("calibration artifact must parse");
        assert_eq!(artifact.schema_version, "1.0");
        assert!(!artifact.calibration_version.is_empty());
        assert!(!artifact.dataset_version.is_empty());
        assert!((artifact.thresholds.max_false_positive_rate - 0.02).abs() < f64::EPSILON);
        assert!((artifact.thresholds.min_recall - 0.80).abs() < f64::EPSILON);
    }

    #[test]
    fn trust_table_rows_are_unique() {
        let rows = trust_rows();
        let mut names: Vec<_> = rows.iter().map(|row| row.0).collect();
        names.sort_unstable();
        let total = names.len();
        names.dedup();
        assert_eq!(names.len(), total, "duplicate trust rows");
    }

    #[test]
    fn syn_rules_needing_type_evidence_are_never_score_eligible() {
        let profile = profile_for("blocking-in-async").expect("rule is mapped");
        assert!(!resolve_score_eligibility(
            "blocking-in-async",
            profile,
            AnalyzerKind::SynAst
        ));
    }

    #[test]
    fn audit_only_rules_never_score() {
        let profile = profile_for("unsafe-block-audit").expect("rule is mapped");
        assert!(!resolve_score_eligibility(
            "unsafe-block-audit",
            profile,
            AnalyzerKind::SynAst
        ));
        let geiger = profile_for("unsafe-dependency").expect("rule is mapped");
        assert_eq!(geiger.tier, TrustTier::AuditOnly);
    }

    #[test]
    fn unknown_rules_are_unranked_and_ineligible() {
        let profile = unranked_profile(AnalyzerKind::Clippy);
        assert!(profile.priority.is_none());
        assert!(!resolve_score_eligibility(
            "clippy::future_lint",
            &profile,
            AnalyzerKind::Clippy
        ));
    }

    #[test]
    fn compiler_rules_carry_authority_without_calibration() {
        let profile = profile_for("compiler-error").expect("rule is mapped");
        assert!(resolve_score_eligibility(
            "compiler-error",
            profile,
            AnalyzerKind::Project
        ));
        assert!(calibration_version_for("compiler-error").is_none());
    }

    #[test]
    fn optional_external_adapters_are_marked_as_such() {
        for rule in [
            "deny-advisory",
            "unused-dependency",
            "unsafe-dependency",
            "semver-violation",
        ] {
            let profile = profile_for(rule).expect("rule is mapped");
            assert_eq!(profile.availability, AnalyzerAvailability::OptionalExternal);
        }
    }

    #[test]
    fn the_wire_vocabulary_is_stable() {
        assert_eq!(TrustTier::CompilerProven.as_str(), "compiler-proven");
        assert_eq!(
            TrustTier::CalibratedHeuristic.as_str(),
            "calibrated-heuristic"
        );
        assert_eq!(TrustTier::AdvisoryBacked.as_str(), "advisory-backed");
        assert_eq!(TrustTier::AuditOnly.as_str(), "audit-only");
        assert_eq!(
            AggregationPolicy::BoundedOccurrence.as_str(),
            "bounded-occurrence"
        );
        assert_eq!(AggregationPolicy::RootCause.as_str(), "root-cause");
        assert_eq!(AggregationPolicy::UniqueRule.as_str(), "unique-rule");
        assert_eq!(Priority::P0.as_str(), "p0");
        assert_eq!(Priority::P3.as_str(), "p3");
        assert_eq!(RequiredEvidence::SynAst.as_str(), "syn-ast");
        assert_eq!(AnalyzerAvailability::Core.as_str(), "core");
    }

    // ----------------------------------------------------------------------
    // Truth dataset contract
    // ----------------------------------------------------------------------

    const DATASET: &str = include_str!("../evaluation/truth-dataset-v1.json");

    fn dataset() -> serde_json::Value {
        serde_json::from_str(DATASET).expect("truth dataset must parse")
    }

    #[test]
    fn the_truth_dataset_covers_ten_rules_with_reviewed_evidence() {
        let dataset = dataset();
        let rules = dataset["rules"].as_array().expect("rules array");
        assert!(rules.len() >= 10, "dataset covers {} rules", rules.len());
        let cases = dataset["cases"].as_array().expect("cases array");
        for rule in rules {
            let rule = rule.as_str().expect("rule id");
            let counted = cases
                .iter()
                .filter(|case| case["rule"] == rule && case["reviewer_state"] == "reviewed");
            let (positive, negative): (Vec<_>, Vec<_>) =
                counted.partition(|case| case["expected_emission"] == true);
            assert!(
                positive.len() >= 20,
                "{rule} has {} reviewed positive opportunities",
                positive.len()
            );
            assert!(
                negative.len() >= 20,
                "{rule} has {} reviewed negative contexts",
                negative.len()
            );
        }
    }

    #[test]
    fn every_truth_fixture_matches_its_recorded_digest_and_parses_on_msrv() {
        use sha2::{Digest, Sha256};
        let dataset = dataset();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for fixture in dataset["fixtures"].as_array().expect("fixtures array") {
            let path = root.join(fixture["path"].as_str().expect("fixture path"));
            let content = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let digest = format!("{:x}", Sha256::digest(content.as_bytes()));
            assert_eq!(
                digest,
                fixture["sha256"].as_str().expect("digest"),
                "fixture {} changed without a dataset update",
                path.display()
            );
            syn::parse_file(&content).unwrap_or_else(|error| {
                panic!("fixture {} does not parse: {error}", path.display())
            });
        }
    }

    #[test]
    fn each_labelled_line_is_derivable_from_its_fixture() {
        let dataset = dataset();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let cases = dataset["cases"].as_array().expect("cases array");
        for fixture in dataset["fixtures"].as_array().expect("fixtures array") {
            let relative = fixture["path"].as_str().expect("fixture path");
            let content =
                std::fs::read_to_string(root.join(relative)).expect("fixture is readable");
            let derived: Vec<(usize, bool)> = content
                .lines()
                .enumerate()
                .filter_map(|(index, line)| {
                    if line.contains("//~ pos") {
                        Some((index + 1, true))
                    } else if line.contains("//~ neg") {
                        Some((index + 1, false))
                    } else {
                        None
                    }
                })
                .collect();
            let declared: Vec<(usize, bool)> = cases
                .iter()
                .filter(|case| case["fixture"] == relative)
                .map(|case| {
                    (
                        usize::try_from(case["location"]["line"].as_u64().expect("line"))
                            .expect("line fits"),
                        case["expected_emission"] == true,
                    )
                })
                .collect();
            assert_eq!(derived, declared, "labels drifted in {relative}");
        }
    }

    // ----------------------------------------------------------------------
    // Promotion and demotion gates
    // ----------------------------------------------------------------------

    fn subject<'a>(
        rule: &'a str,
        tier: TrustTier,
        evidence: &'a [RequiredEvidence],
    ) -> GateSubject<'a> {
        GateSubject {
            rule,
            provider: "rust-doctor",
            tier,
            analyzer: AnalyzerKind::SynAst,
            default_enabled: true,
            score_eligible: true,
            required_evidence: evidence,
        }
    }

    #[test]
    fn a_rule_that_is_neither_default_nor_scored_is_outside_the_gate() {
        let mut inert = subject("anything", TrustTier::CalibratedHeuristic, SYN_EVIDENCE);
        inert.default_enabled = false;
        inert.score_eligible = false;
        assert_eq!(evaluate_gate(&inert, "2026-07-27"), GateVerdict::NotGated);
    }

    #[test]
    fn a_missing_calibration_record_fails_rather_than_passing_quietly() {
        let unmeasured = subject(
            "rule-that-was-never-measured",
            TrustTier::CalibratedHeuristic,
            SYN_EVIDENCE,
        );
        match evaluate_gate(&unmeasured, "2026-07-27") {
            GateVerdict::Failing(failure) => assert_eq!(failure.code, "missing-calibration"),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn the_threshold_detail_names_every_breached_metric() {
        let thresholds = CalibrationThresholds {
            confidence_level: 0.95,
            max_false_positive_rate: 0.02,
            min_recall: 0.80,
            min_positive_samples: 50,
            min_negative_samples: 149,
            min_context_coverage: 0.90,
        };
        let record = CalibrationRecord {
            rule: "demo".to_string(),
            decision: RuleDecision::ScoreEligibleDefault,
            evidence_complete: true,
            true_positives: 1,
            false_positives: 9,
            false_negatives: 9,
            positive_samples: 10,
            negative_samples: 5,
            precision: Some(0.1),
            recall: Some(0.10),
            false_positive_rate: Some(0.50),
            false_positive_upper_bound: Some(0.75),
            confidence_level: 0.95,
            context_coverage: 0.20,
            reviewer: "test".to_string(),
            reviewed_at: "2026-07-27".to_string(),
            rationale: String::new(),
        };
        assert!(!record.passes(&thresholds));
        let detail = threshold_detail(&record, &thresholds);
        for expected in [
            "recall",
            "false-positive upper bound",
            "context coverage",
            "positive",
            "negative",
        ] {
            assert!(detail.contains(expected), "{detail}");
        }
    }

    #[test]
    fn an_unavailable_metric_can_never_be_counted_as_success() {
        let thresholds = CalibrationThresholds {
            confidence_level: 0.95,
            max_false_positive_rate: 0.02,
            min_recall: 0.80,
            min_positive_samples: 20,
            min_negative_samples: 20,
            min_context_coverage: 0.90,
        };
        let record = CalibrationRecord {
            rule: "demo".to_string(),
            decision: RuleDecision::ScoreEligibleDefault,
            evidence_complete: true,
            true_positives: 30,
            false_positives: 0,
            false_negatives: 0,
            positive_samples: 30,
            negative_samples: 30,
            precision: Some(1.0),
            // No labeled opportunity means no recall, which is not a pass.
            recall: None,
            false_positive_rate: Some(0.0),
            false_positive_upper_bound: Some(0.095),
            confidence_level: 0.95,
            context_coverage: 1.0,
            reviewer: "test".to_string(),
            reviewed_at: "2026-07-27".to_string(),
            rationale: String::new(),
        };
        assert!(!record.passes(&thresholds));
        assert!(threshold_detail(&record, &thresholds).contains("recall unavailable"));
    }

    #[test]
    fn an_exception_must_be_owned_dated_reasoned_and_evidenced() {
        let complete = TrustException {
            rule: "demo".to_string(),
            owner: "arthur".to_string(),
            granted_at: "2026-07-01".to_string(),
            expires_at: "2026-10-01".to_string(),
            reason: "measurement pending on a new corpus revision".to_string(),
            evidence: "tasks/prd-diagnostic-trust-parity.md#us-012".to_string(),
        };
        assert!(complete.well_formed());
        assert!(complete.active_on("2026-07-27"));
        // Time-bounded in both directions.
        assert!(!complete.active_on("2026-06-30"));
        assert!(!complete.active_on("2026-10-01"));

        for missing in [
            TrustException {
                owner: String::new(),
                ..complete.clone()
            },
            TrustException {
                reason: String::new(),
                ..complete.clone()
            },
            TrustException {
                evidence: String::new(),
                ..complete.clone()
            },
            TrustException {
                expires_at: "not-a-date".to_string(),
                ..complete.clone()
            },
            TrustException {
                expires_at: "2026-06-01".to_string(),
                ..complete
            },
        ] {
            assert!(
                !missing.well_formed(),
                "{missing:?} must not be well formed"
            );
            assert!(!missing.active_on("2026-07-27"));
        }
    }

    #[test]
    fn the_shipped_catalog_grants_no_active_exception() {
        // An exception is a visible, owner-attributed decision. The shipping
        // default is none at all.
        assert!(active_exceptions(&today_utc()).is_empty());
    }

    #[test]
    fn the_gate_history_covers_every_calibrated_rule_on_the_current_revision() {
        let history = gate_history().expect("gate history must parse");
        let revision = history.revisions.last().expect("one approved revision");
        let artifact = calibration().expect("calibration artifact");
        for record in &artifact.rules {
            let outcomes = history
                .rules
                .get(&record.rule)
                .unwrap_or_else(|| panic!("{} has no recorded gate outcome", record.rule));
            assert!(
                outcomes.iter().any(|outcome| &outcome.revision == revision),
                "{} was not evaluated on {revision}",
                record.rule
            );
        }
    }

    #[test]
    fn two_consecutive_failing_revisions_propose_a_demotion() {
        assert_eq!(consecutive_failures("rule-with-no-history"), 0);
        // Every shipped rule passes on the current revision, so nothing is on
        // a failing streak.
        let history = gate_history().expect("gate history");
        for rule in history.rules.keys() {
            assert!(
                consecutive_failures(rule) < DEMOTION_STREAK,
                "{rule} is already on a demotion streak"
            );
        }
    }

    #[test]
    fn the_date_helper_produces_a_comparable_iso_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_000), (2022, 1, 8));
        let today = today_utc();
        assert!(is_iso_date(&today), "{today}");
        assert!(today.as_str() > "2026-01-01");
    }

    #[test]
    fn calibrated_heuristics_only_score_with_a_passing_record() {
        let artifact = calibration().expect("calibration artifact");
        for record in &artifact.rules {
            let version = calibration_version_for(&record.rule);
            assert_eq!(
                version.is_some(),
                record.passes(&artifact.thresholds),
                "{} exposes a calibration version that disagrees with its measurement",
                record.rule
            );
        }
    }
}
