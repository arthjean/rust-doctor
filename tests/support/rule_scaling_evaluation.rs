use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::support::GitRepositoryState;
use super::support::rule_scaling::{ClippyDefault, ExpectedSpan, SignalAdmission};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceVerdict {
    Pass,
    Blocked,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScanStatus {
    Complete,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ObservedReportStatus {
    Complete,
    Incomplete,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NetworkMode {
    Offline,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdmissionBasis {
    PerRule,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FindingVerdict {
    TruePositive,
    FalsePositive,
    Ambiguous,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluationArtifact {
    pub(crate) artifact: String,
    pub(crate) commands: CommandEvidence,
    pub(crate) epic: String,
    pub(crate) generated_at: String,
    pub(crate) matrix: MatrixEvidence,
    pub(crate) network_in_automated_tests: bool,
    pub(crate) reconstruction: ReconstructionEvidence,
    pub(crate) repositories: Vec<RepositoryEvaluation>,
    pub(crate) rule_results: Vec<RuleResult>,
    pub(crate) scan_network_mode: NetworkMode,
    pub(crate) schema_version: u64,
    pub(crate) toolchain: ToolchainEvidence,
    pub(crate) totals: EvaluationTotals,
    pub(crate) trust_boundary: TrustBoundary,
    pub(crate) verdict: EvidenceVerdict,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TrustBoundary {
    pub(crate) cargo_build_code_acknowledged: bool,
    pub(crate) repositories: String,
    pub(crate) substitution_allowed: bool,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolchainEvidence {
    pub(crate) cargo: String,
    pub(crate) clippy: String,
    pub(crate) rustc: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommandEvidence {
    pub(crate) expanded: Vec<String>,
    pub(crate) legacy: Vec<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MatrixEvidence {
    pub(crate) admission_basis: AdmissionBasis,
    pub(crate) contexts: MatrixContextEvidence,
    pub(crate) global_rate_used_for_admission: bool,
    pub(crate) rules: Vec<MatrixRuleEvidence>,
    pub(crate) totals: MatrixTotals,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MatrixRuleEvidence {
    #[serde(rename = "fn")]
    pub(crate) false_negatives: usize,
    pub(crate) fp: usize,
    pub(crate) id: String,
    pub(crate) positive_span_hash: String,
    pub(crate) tn: usize,
    pub(crate) tp: usize,
    pub(crate) verdict: EvidenceVerdict,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MatrixContextEvidence {
    pub(crate) build_output_candidate_diagnostics: usize,
    pub(crate) external_expansion_contract: String,
    pub(crate) local_macro_contract: String,
    pub(crate) missing_primary_span_contract: String,
    pub(crate) non_unix_permissions: NonUnixContextEvidence,
    pub(crate) unicode_primary_span: SpanEvidence,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NonUnixContextEvidence {
    pub(crate) candidate_diagnostics: usize,
    pub(crate) fixture: String,
    pub(crate) primary_span: SpanEvidence,
    pub(crate) target: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MatrixTotals {
    #[serde(rename = "fn")]
    pub(crate) false_negatives: usize,
    pub(crate) fp: usize,
    pub(crate) tn: usize,
    pub(crate) tp: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpanEvidence {
    pub(crate) column_end: u64,
    pub(crate) column_start: u64,
    pub(crate) line_end: u64,
    pub(crate) line_start: u64,
}

impl From<&ExpectedSpan> for SpanEvidence {
    fn from(span: &ExpectedSpan) -> Self {
        Self {
            column_end: span.column_end,
            column_start: span.column_start,
            line_end: span.line_end,
            line_start: span.line_start,
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RepositoryEvaluation {
    pub(crate) build_code_warning_acknowledged: bool,
    pub(crate) commit: String,
    pub(crate) expanded: ExpandedScanEvidence,
    pub(crate) legacy: LegacyScanEvidence,
    pub(crate) name: String,
    pub(crate) network: NetworkMode,
    pub(crate) repository_state: RepositoryStateEvidence,
    pub(crate) signal_classification: SignalClassification,
    pub(crate) trusted: bool,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LegacyScanEvidence {
    pub(crate) command: Vec<String>,
    pub(crate) counts: BTreeMap<String, usize>,
    pub(crate) exit_code: i32,
    pub(crate) status: ScanStatus,
    pub(crate) structured_id_hash: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExpandedScanEvidence {
    pub(crate) ambiguous: usize,
    pub(crate) command: Vec<String>,
    pub(crate) counts: BTreeMap<String, usize>,
    pub(crate) exit_code: i32,
    pub(crate) false_positives: usize,
    pub(crate) finding_id_hash: String,
    pub(crate) findings: Vec<FindingEvidence>,
    pub(crate) manual_verdicts: Vec<ManualVerdictEvidence>,
    pub(crate) status: ScanStatus,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ExpandedReportObservation {
    pub(crate) status: ObservedReportStatus,
    pub(crate) scan: ScanObservation,
    pub(crate) diagnostics: Vec<DiagnosticObservation>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ScanObservation {
    pub(crate) command: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DiagnosticObservation {
    pub(crate) id: String,
    pub(crate) code: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindingEvidence {
    pub(crate) code: String,
    pub(crate) id: String,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManualVerdictEvidence {
    pub(crate) finding_id: String,
    pub(crate) justification: String,
    pub(crate) verdict: FindingVerdict,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignalClassification {
    pub(crate) baseline_warn_contracts: BTreeMap<String, BaselineWarnContract>,
    pub(crate) opt_in_findings: BTreeMap<String, usize>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BaselineWarnContract {
    pub(crate) expanded_findings: usize,
    pub(crate) legacy_findings: usize,
    pub(crate) metadata_admitted: bool,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RepositoryStateEvidence {
    pub(crate) after: GitRepositoryState,
    pub(crate) before: GitRepositoryState,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleResult {
    pub(crate) admission: SignalAdmission,
    pub(crate) ambiguous: usize,
    pub(crate) clippy_default: ClippyDefault,
    pub(crate) expanded_findings: usize,
    pub(crate) false_positives: usize,
    pub(crate) id: String,
    pub(crate) legacy_findings: usize,
    pub(crate) true_positives: usize,
    pub(crate) verdict: EvidenceVerdict,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluationTotals {
    pub(crate) ambiguous: usize,
    pub(crate) expanded_findings: usize,
    pub(crate) expanded_scans: usize,
    pub(crate) false_positives: usize,
    pub(crate) legacy_scans: usize,
    pub(crate) manual_verdicts: usize,
    pub(crate) repositories: usize,
    pub(crate) repositories_unchanged: usize,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReconstructionEvidence {
    pub(crate) automated_default: String,
    pub(crate) network: NetworkMode,
    pub(crate) test: String,
}
