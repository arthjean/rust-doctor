use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const CORPUS_SCHEMA_VERSION: &str = "1.0";
pub(crate) const DELTA_SCHEMA_VERSION: &str = "1.0";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the versioned evaluation policy serializes explicit independent switches"
)]
pub(crate) struct EvaluationProfile {
    pub(crate) version: String,
    pub(crate) normalized_severity: String,
    pub(crate) force_candidate_rules: bool,
    pub(crate) respect_inline_suppressions: bool,
    pub(crate) respect_project_config: bool,
    pub(crate) offline: bool,
    pub(crate) adapter_policy: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CorpusManifest {
    pub(crate) schema_version: String,
    pub(crate) evaluation_profile: EvaluationProfile,
    pub(crate) repositories: Vec<RepositorySpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RepositorySpec {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) commit: String,
    pub(crate) minimum_project_roots: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreparedCorpus {
    pub(crate) schema_version: String,
    pub(crate) manifest_sha256: String,
    pub(crate) repositories: Vec<PreparedRepository>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreparedRepository {
    pub(crate) name: String,
    pub(crate) commit: String,
    pub(crate) checkout_dir: String,
    pub(crate) project_roots: Vec<String>,
    pub(crate) tree_digest: String,
    pub(crate) submodule_status: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SeverityCounts {
    pub(crate) error: usize,
    pub(crate) warning: usize,
    pub(crate) info: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluationDiagnostic {
    pub(crate) repository: String,
    pub(crate) package_root: String,
    pub(crate) rule: String,
    pub(crate) site_id: String,
    pub(crate) baseline_key: String,
    pub(crate) fingerprint: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluationRule {
    pub(crate) category: String,
    pub(crate) default_severity: String,
    pub(crate) default_enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FailureEvent {
    pub(crate) attempt: u8,
    pub(crate) kind: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CorpusRecord {
    pub(crate) schema_version: String,
    pub(crate) repository: String,
    pub(crate) commit: String,
    pub(crate) package_roots: Vec<String>,
    pub(crate) expected_roots: Vec<String>,
    pub(crate) attempted_roots: Vec<String>,
    pub(crate) reported_roots: Vec<String>,
    pub(crate) root_states: BTreeMap<String, RootState>,
    pub(crate) tool_revision: String,
    pub(crate) evaluation_profile_sha256: String,
    pub(crate) catalog_sha256: String,
    pub(crate) catalog: BTreeMap<String, EvaluationRule>,
    pub(crate) tree_digest: String,
    pub(crate) complete: bool,
    pub(crate) completeness: String,
    pub(crate) diagnostic_counts: SeverityCounts,
    pub(crate) per_rule_counts: BTreeMap<String, usize>,
    pub(crate) duration_ms: u64,
    pub(crate) attempts: u8,
    pub(crate) diagnostics: Vec<EvaluationDiagnostic>,
    pub(crate) failure_chain: Vec<FailureEvent>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RootState {
    NotAttempted,
    Failed,
    Incomplete,
    Complete,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReviewLabel {
    TruePositive,
    FalsePositive,
    Uncertain,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LabelFile {
    pub(crate) schema_version: String,
    pub(crate) candidate_sha256: String,
    pub(crate) approval: EvidenceApproval,
    pub(crate) labels: Vec<DiagnosticLabel>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiagnosticLabel {
    pub(crate) repository: String,
    pub(crate) package_root: String,
    pub(crate) rule: String,
    pub(crate) site_id: String,
    pub(crate) evidence_fingerprint: String,
    pub(crate) label: ReviewLabel,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceApproval {
    pub(crate) schema_version: String,
    pub(crate) subject_sha256: String,
    pub(crate) repository: String,
    pub(crate) head_sha: String,
    pub(crate) run_id: u64,
    pub(crate) artifact_id: u64,
    pub(crate) artifact_name: String,
    pub(crate) artifact_digest: String,
    pub(crate) artifact_url: String,
    pub(crate) reviewed_by: String,
    pub(crate) reviewed_at: String,
    pub(crate) review_source: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RuleDelta {
    pub(crate) rule: String,
    pub(crate) introduced: usize,
    pub(crate) removed: usize,
    pub(crate) changed: usize,
    pub(crate) count_growth: usize,
    pub(crate) affected_root_percent: f64,
    pub(crate) catalog_changed: bool,
    pub(crate) affected_repositories: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PromotionReview {
    pub(crate) rule: String,
    pub(crate) sample_size: usize,
    pub(crate) labeled: usize,
    pub(crate) false_positive_percent: f64,
    pub(crate) eligible_for_default: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RuntimeDelta {
    pub(crate) repository: String,
    pub(crate) baseline_ms: u64,
    pub(crate) candidate_ms: u64,
    pub(crate) delta_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DeltaReport {
    pub(crate) schema_version: String,
    pub(crate) baseline_sha256: String,
    pub(crate) candidate_sha256: String,
    pub(crate) complete_baseline_roots: usize,
    pub(crate) complete_candidate_roots: usize,
    pub(crate) introduced: usize,
    pub(crate) removed: usize,
    pub(crate) changed: usize,
    pub(crate) affected_root_percent: f64,
    pub(crate) incomplete_root_change_percentage_points: f64,
    pub(crate) median_runtime_delta_percent: f64,
    pub(crate) p95_runtime_delta_percent: f64,
    pub(crate) top_runtime_regressions: Vec<RuntimeDelta>,
    pub(crate) rule_deltas: Vec<RuleDelta>,
    pub(crate) promotion_reviews: Vec<PromotionReview>,
    pub(crate) blocked: bool,
    pub(crate) reasons: Vec<String>,
}
