//! Decision-quality release certification (EP-004 / US-013).
//!
//! Certification is a gate, not a report. It reads the artifacts the earlier
//! epics produce (the labeled truth baseline, the calibration record, the
//! conformance matrices, and a required corpus run) and refuses to mark the
//! release certified when any of them is missing, stale, or below its approved
//! threshold. Uncertainty never certifies.

use super::manifest::{
    evaluation_profile_sha256, hex_digest, read_json, sha256_file, validate_corpus_manifest,
    write_json_atomic,
};
use super::model::{
    CORPUS_SCHEMA_VERSION, CorpusManifest, CorpusRecord, EvidenceApproval, RootState,
};
use super::process::{ProcessOutput, run_capped};
use super::truth::{BaselineThresholds, TruthBaseline};
use super::{EvalError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Thresholds the PRD fixes for every score-eligible calibrated heuristic.
const APPROVED_THRESHOLDS: BaselineThresholds = BaselineThresholds {
    confidence_level: 0.95,
    max_false_positive_rate: 0.02,
    min_recall: 0.8,
    min_positive_samples: 50,
    min_negative_samples: 149,
    min_context_coverage: 0.9,
};

/// Minimum complete Cargo roots on the pinned corpus.
const MIN_COMPLETE_ROOTS: usize = 260;
/// Allowed incompleteness regression, in percentage points.
const MAX_INCOMPLETENESS_REGRESSION: f64 = 0.2;
const CERTIFICATION_TIMEOUT: Duration = Duration::from_mins(5);
const CERTIFICATION_OUTPUT_CAP: usize = 32 * 1024 * 1024;

/// Adapters that can carry analyzer authority. Each needs a conformance matrix.
const AUTHORITY_ADAPTERS: [&str; 7] = [
    "cargo-audit",
    "cargo-deny",
    "cargo-geiger",
    "cargo-metadata",
    "cargo-semver-checks",
    "cargo-shear",
    "clippy",
];

const ADAPTER_VERSIONS: [(&str, &[&str]); 7] = [
    ("cargo-audit", &["0.20", "0.21"]),
    ("cargo-deny", &["0.18", "0.19"]),
    ("cargo-geiger", &["0.11", "0.12"]),
    ("cargo-metadata", &["1.0"]),
    ("cargo-semver-checks", &["0.41", "0.42"]),
    ("cargo-shear", &["1.5", "1.6"]),
    ("clippy", &["1.97", "1.98"]),
];

const RELEASE_GATES: [&str; 7] = [
    "quality-gates",
    "cross-surface",
    "artifact-smoke",
    "workspace-scale",
    "decision-overhead",
    "corpus-runtime",
    "interruption",
];

#[derive(Debug)]
pub(crate) struct CertifyArgs {
    pub(crate) binary: PathBuf,
    pub(crate) baseline: PathBuf,
    pub(crate) calibration: PathBuf,
    pub(crate) dataset: PathBuf,
    pub(crate) conformance: PathBuf,
    pub(crate) corpus: PathBuf,
    pub(crate) corpus_baseline: PathBuf,
    pub(crate) corpus_manifest: PathBuf,
    pub(crate) corpus_approval: PathBuf,
    pub(crate) score_model: PathBuf,
    pub(crate) score_migration: PathBuf,
    pub(crate) release_evidence: PathBuf,
    pub(crate) self_scan_root: PathBuf,
    pub(crate) tool_revision: String,
    pub(crate) generated_at_utc: u64,
    pub(crate) output: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
struct CalibrationArtifact {
    calibration_version: String,
    dataset_version: String,
    thresholds: BaselineThresholds,
    rules: Vec<CalibrationRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct CalibrationRecord {
    rule: String,
    evidence_complete: bool,
    positive_samples: usize,
    negative_samples: usize,
    recall: Option<f64>,
    false_positive_upper_bound: Option<f64>,
    confidence_level: f64,
    context_coverage: f64,
}

impl CalibrationRecord {
    /// A record passes only when every metric is present and inside its bound.
    /// A missing metric is a failure, never a silent pass (US-017 AC-2).
    fn passes(&self, thresholds: &BaselineThresholds) -> Option<String> {
        if !self.evidence_complete {
            return Some("evidence incomplete".to_string());
        }
        match self.recall {
            None => return Some("recall unavailable".to_string()),
            Some(value) if value < thresholds.min_recall => {
                return Some(format!("recall {value:.3} < {:.3}", thresholds.min_recall));
            }
            Some(_) => {}
        }
        match self.false_positive_upper_bound {
            None => return Some("false-positive upper bound unavailable".to_string()),
            Some(value) if value > thresholds.max_false_positive_rate => {
                return Some(format!(
                    "false-positive upper bound {value:.3} > {:.3}",
                    thresholds.max_false_positive_rate
                ));
            }
            Some(_) => {}
        }
        if (self.confidence_level - thresholds.confidence_level).abs() >= f64::EPSILON {
            return Some(format!(
                "confidence level {:.3} != {:.3}",
                self.confidence_level, thresholds.confidence_level
            ));
        }
        if self.context_coverage < thresholds.min_context_coverage {
            return Some(format!(
                "context coverage {:.3} < {:.3}",
                self.context_coverage, thresholds.min_context_coverage
            ));
        }
        if self.positive_samples < thresholds.min_positive_samples {
            return Some(format!(
                "{} positive samples < {}",
                self.positive_samples, thresholds.min_positive_samples
            ));
        }
        if self.negative_samples < thresholds.min_negative_samples {
            return Some(format!(
                "{} negative samples < {}",
                self.negative_samples, thresholds.min_negative_samples
            ));
        }
        None
    }
}

/// One rule as the shipped catalog declares it.
#[derive(Debug, Clone, Deserialize)]
struct CatalogEntry {
    canonical_id: String,
    default_enabled: bool,
    trust: CatalogTrust,
    gate: CatalogGate,
}

#[derive(Debug, Clone, Deserialize)]
struct CatalogTrust {
    tier: String,
    score_eligible: bool,
    calibration_version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CatalogGate {
    status: String,
    reason_code: Option<String>,
}

/// One certification gate and its verdict.
#[derive(Debug, Clone, Serialize)]
struct GateResult {
    gate: &'static str,
    passed: bool,
    detail: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ArtifactIdentity {
    path: String,
    sha256: String,
}

#[derive(Debug, Clone, Serialize)]
struct CorpusSummary {
    repositories: usize,
    roots_total: usize,
    roots_complete: usize,
    roots_incomplete: usize,
    roots_failed: usize,
    roots_not_attempted: usize,
    tool_revision: String,
}

#[derive(Debug, Clone, Serialize)]
struct AdapterEvidence {
    adapter: &'static str,
    supported_versions: &'static [&'static str],
    conformance_sha256: String,
    passed: bool,
}

#[derive(Debug, Clone, Serialize)]
struct RemediationSummary {
    rule: String,
    title: String,
    priority: Option<String>,
    sites: u64,
    files: u64,
    score_penalty: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct SelfScanSummary {
    case: &'static str,
    score: Option<u64>,
    label: Option<String>,
    authoritative: bool,
    reason_codes: Vec<String>,
    scored_groups: usize,
    advisory_groups: usize,
    audit_groups: usize,
    top_remediations: Vec<RemediationSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseEvidence {
    schema_version: String,
    release_binary_sha256: String,
    tool_revision: String,
    generated_at_utc_unix_seconds: u64,
    gates: Vec<ReleaseGateEvidence>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseGateEvidence {
    gate: String,
    passed: bool,
    command: String,
    detail: String,
    artifact: ArtifactIdentity,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct GateArtifact {
    schema_version: String,
    gate: String,
    release_binary_sha256: String,
    tool_revision: String,
    generated_at_utc_unix_seconds: u64,
    passed: bool,
    command: String,
    detail: String,
}

#[derive(Debug, Clone, Serialize)]
struct CertificationFailure {
    code: &'static str,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
struct Certification {
    schema_version: &'static str,
    certified: bool,
    generated_at_utc_unix_seconds: u64,
    command: Vec<String>,
    tool_version: String,
    release_binary_sha256: String,
    score_model_version: String,
    rust_toolchain: String,
    operating_system: &'static str,
    architecture: &'static str,
    dataset_version: String,
    calibration_version: String,
    artifacts: BTreeMap<&'static str, ArtifactIdentity>,
    corpus: CorpusSummary,
    adapters: Vec<AdapterEvidence>,
    self_scan: SelfScanSummary,
    release_evidence: ReleaseEvidence,
    gates: Vec<GateResult>,
    /// Rules that block certification, with the reason each one blocks it.
    blocking_rules: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<CertificationFailure>,
}

pub(crate) fn run(args: &CertifyArgs) -> Result<()> {
    match run_inner(args) {
        Ok(()) => Ok(()),
        Err(error @ EvalError::CertificationFailed(_)) => Err(error),
        Err(error) => {
            let failure = serde_json::json!({
                "schema_version": "2.0",
                "certified": false,
                "generated_at_utc_unix_seconds": args.generated_at_utc.max(1),
                "command": portable_command(args),
                "error": {
                    "code": "certification-input-invalid",
                    "message": sanitize_certification_text(&error.to_string(), &[]),
                }
            });
            if portable_input_path(&args.output) {
                let _ = write_json_atomic(&args.output, &failure);
            }
            Err(EvalError::CertificationFailed(
                "certification inputs or evidence are invalid; uncertified evidence was emitted"
                    .to_string(),
            ))
        }
    }
}

fn run_inner(args: &CertifyArgs) -> Result<()> {
    validate_portable_inputs(args)?;
    let manifest: CorpusManifest = read_json(&args.corpus_manifest)?;
    validate_corpus_manifest(&manifest)?;
    let baseline: TruthBaseline = read_json(&args.baseline)?;
    let calibration: CalibrationArtifact = read_json(&args.calibration)?;
    let catalog = shipped_catalog(&args.binary)?;
    let score_model_version = shipped_score_model(&args.binary)?;
    let tool_version = shipped_tool_version(&args.binary)?;
    let release_binary_sha256 = sha256_file(&args.binary)?;
    let release_evidence: ReleaseEvidence = read_json(&args.release_evidence)?;
    let release_gates = validate_release_evidence(
        &release_evidence,
        &release_binary_sha256,
        &args.tool_revision,
        args.generated_at_utc,
        &manifest,
    )?;
    let rust_toolchain = rustc_identity()?;
    let (corpus_gate, corpus_runtime_gate, corpus) =
        corpus_gate(args, &manifest, &catalog, &release_binary_sha256)?;
    let (conformance_gate, adapters, conformance_sha256) = conformance_gate(&args.conformance)?;
    let self_scan = self_scan(&args.binary, &args.self_scan_root)?;

    let mut gates = Vec::new();
    let mut blocking = BTreeMap::new();

    gates.push(dataset_freshness_gate(args, &baseline)?);
    gates.push(threshold_gate(&baseline, &calibration));
    gates.push(toolchain_gate(&rust_toolchain));
    gates.push(tool_identity_gate(&baseline, &tool_version));
    gates.push(model_identity_gate(&baseline, &score_model_version));
    gates.push(calibration_gate(&catalog, &calibration, &mut blocking));
    gates.push(catalog_gate(&catalog, &mut blocking));
    gates.push(conformance_gate);
    gates.push(corpus_gate);
    gates.push(corpus_runtime_gate);
    gates.push(self_scan_gate(&self_scan, &score_model_version));
    gates.extend(release_gates);

    let certified = gates.iter().all(|gate| gate.passed);
    let failure_details: Vec<_> = gates
        .iter()
        .filter(|gate| !gate.passed)
        .map(|gate| format!("{}: {}", gate.gate, gate.detail))
        .collect();
    let artifacts = artifact_identities(args, conformance_sha256)?;
    let certification = Certification {
        schema_version: "2.0",
        certified,
        generated_at_utc_unix_seconds: args.generated_at_utc,
        command: portable_command(args),
        tool_version,
        release_binary_sha256,
        score_model_version,
        rust_toolchain,
        operating_system: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        dataset_version: baseline.dataset_version,
        calibration_version: calibration.calibration_version.clone(),
        artifacts,
        corpus,
        adapters,
        self_scan,
        release_evidence,
        gates,
        blocking_rules: blocking,
        error: (!certified).then(|| CertificationFailure {
            code: "release-gates-failed",
            message: failure_details.join("; "),
        }),
    };
    write_json_atomic(&args.output, &certification)?;

    if certified {
        println!(
            "Score Core V2 certified: model {} on {} ({}).",
            certification.score_model_version,
            certification.rust_toolchain,
            certification.dataset_version
        );
        return Ok(());
    }
    Err(EvalError::CertificationFailed(format!(
        "Score Core V2 is not certified. {}",
        failure_details.join("; ")
    )))
}

fn validate_portable_inputs(args: &CertifyArgs) -> Result<()> {
    for (name, path) in [
        ("binary", &args.binary),
        ("baseline", &args.baseline),
        ("calibration", &args.calibration),
        ("dataset", &args.dataset),
        ("conformance", &args.conformance),
        ("corpus", &args.corpus),
        ("corpus baseline", &args.corpus_baseline),
        ("corpus manifest", &args.corpus_manifest),
        ("corpus approval", &args.corpus_approval),
        ("score model", &args.score_model),
        ("score migration", &args.score_migration),
        ("release evidence", &args.release_evidence),
        ("self-scan root", &args.self_scan_root),
        ("output", &args.output),
    ] {
        if !portable_input_path(path) {
            return Err(EvalError::InvalidManifest(format!(
                "certification {name} path must be relative and contain no parent traversal"
            )));
        }
    }
    if args.tool_revision.len() != 40
        || !args
            .tool_revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(EvalError::InvalidManifest(
            "certification tool revision must be a full 40-character hexadecimal commit"
                .to_string(),
        ));
    }
    if args.generated_at_utc == 0 {
        return Err(EvalError::InvalidManifest(
            "certification UTC timestamp must be non-zero".to_string(),
        ));
    }
    let expected_binary = if cfg!(windows) {
        Path::new("target/release/rust-doctor.exe")
    } else {
        Path::new("target/release/rust-doctor")
    };
    if args.binary != expected_binary {
        return Err(EvalError::InvalidManifest(
            "certification must execute the canonical target/release rust-doctor binary"
                .to_string(),
        ));
    }
    Ok(())
}

fn portable_input_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| component != std::path::Component::ParentDir)
}

fn portable_command(args: &CertifyArgs) -> Vec<String> {
    let mut command = vec!["rust-doctor-eval".to_string(), "certify".to_string()];
    for (flag, path) in [
        ("--binary", &args.binary),
        ("--baseline", &args.baseline),
        ("--calibration", &args.calibration),
        ("--dataset", &args.dataset),
        ("--conformance", &args.conformance),
        ("--corpus", &args.corpus),
        ("--corpus-baseline", &args.corpus_baseline),
        ("--corpus-manifest", &args.corpus_manifest),
        ("--corpus-approval", &args.corpus_approval),
        ("--score-model", &args.score_model),
        ("--score-migration", &args.score_migration),
        ("--release-evidence", &args.release_evidence),
        ("--self-scan-root", &args.self_scan_root),
        ("--output", &args.output),
    ] {
        command.push(flag.to_string());
        command.push(portable_path(path));
    }
    command.extend([
        "--tool-revision".to_string(),
        args.tool_revision.clone(),
        "--generated-at-utc".to_string(),
        args.generated_at_utc.to_string(),
    ]);
    command
}

fn artifact_identities(
    args: &CertifyArgs,
    conformance_sha256: String,
) -> Result<BTreeMap<&'static str, ArtifactIdentity>> {
    let mut artifacts = BTreeMap::new();
    let certification_schema = PathBuf::from("evaluation/schemas/certification-v2.schema.json");
    let release_gate_runner = PathBuf::from("scripts/release/certification-gate.sh");
    let release_evidence_assembler =
        PathBuf::from("scripts/release/assemble-certification-evidence.sh");
    for (name, path) in [
        ("truth_baseline", &args.baseline),
        ("calibration", &args.calibration),
        ("truth_dataset", &args.dataset),
        ("corpus", &args.corpus),
        ("corpus_baseline", &args.corpus_baseline),
        ("corpus_manifest", &args.corpus_manifest),
        ("corpus_approval", &args.corpus_approval),
        ("score_model", &args.score_model),
        ("score_migration", &args.score_migration),
        ("release_evidence", &args.release_evidence),
        ("certification_schema", &certification_schema),
        ("release_gate_runner", &release_gate_runner),
        ("release_evidence_assembler", &release_evidence_assembler),
    ] {
        artifacts.insert(
            name,
            ArtifactIdentity {
                path: portable_path(path),
                sha256: sha256_file(path)?,
            },
        );
    }
    artifacts.insert(
        "conformance",
        ArtifactIdentity {
            path: portable_path(&args.conformance),
            sha256: conformance_sha256,
        },
    );
    Ok(artifacts)
}

fn validate_release_evidence(
    evidence: &ReleaseEvidence,
    binary_sha256: &str,
    tool_revision: &str,
    generated_at_utc: u64,
    manifest: &CorpusManifest,
) -> Result<Vec<GateResult>> {
    if evidence.schema_version != "1.0"
        || evidence.release_binary_sha256 != binary_sha256
        || evidence.tool_revision != tool_revision
        || evidence.generated_at_utc_unix_seconds != generated_at_utc
    {
        return Err(EvalError::InvalidManifest(
            "release evidence is stale relative to the certified binary or source revision"
                .to_string(),
        ));
    }
    let mut seen = BTreeSet::new();
    let mut artifact_paths = BTreeSet::new();
    let mut gates = Vec::with_capacity(RELEASE_GATES.len());
    for required in RELEASE_GATES {
        let matching: Vec<_> = evidence
            .gates
            .iter()
            .filter(|gate| gate.gate == required)
            .collect();
        if matching.len() != 1
            || matching[0].command.trim().is_empty()
            || matching[0].detail.trim().is_empty()
            || !portable_input_path(Path::new(&matching[0].artifact.path))
            || !artifact_paths.insert(matching[0].artifact.path.as_str())
            || !seen.insert(required)
        {
            return Err(EvalError::InvalidManifest(format!(
                "release evidence must contain one complete {required} verdict"
            )));
        }
        let gate = matching[0];
        let expected_command = format!("bash scripts/release/certification-gate.sh {}", gate.gate);
        let expected_detail = format!("{} gate completed against the release binary", gate.gate);
        let expected_artifact = format!("evaluation/certifications/evidence/{}.json", gate.gate);
        if gate.command != expected_command
            || gate.detail != expected_detail
            || gate.artifact.path != expected_artifact
        {
            return Err(EvalError::InvalidManifest(format!(
                "{required} release evidence does not use the canonical gate runner"
            )));
        }
        validate_portable_text(&gate.command, manifest)?;
        validate_portable_text(&gate.detail, manifest)?;
        if sha256_file(Path::new(&gate.artifact.path))? != gate.artifact.sha256 {
            return Err(EvalError::InvalidManifest(format!(
                "{required} release evidence artifact hash is stale"
            )));
        }
        let artifact: GateArtifact = read_json(Path::new(&gate.artifact.path))?;
        if artifact.schema_version != "1.0"
            || artifact.gate != gate.gate
            || artifact.release_binary_sha256 != binary_sha256
            || artifact.tool_revision != tool_revision
            || artifact.generated_at_utc_unix_seconds != evidence.generated_at_utc_unix_seconds
            || artifact.passed != gate.passed
            || artifact.command != gate.command
            || artifact.detail != gate.detail
        {
            return Err(EvalError::InvalidManifest(format!(
                "{required} release evidence artifact does not match its verdict"
            )));
        }
        validate_portable_text(&artifact.command, manifest)?;
        validate_portable_text(&artifact.detail, manifest)?;
        gates.push(GateResult {
            gate: required,
            passed: gate.passed,
            detail: gate.detail.clone(),
        });
    }
    if evidence.gates.len() != RELEASE_GATES.len() {
        return Err(EvalError::InvalidManifest(
            "release evidence contains unknown or duplicate gate verdicts".to_string(),
        ));
    }
    Ok(gates)
}

fn validate_portable_text(value: &str, manifest: &CorpusManifest) -> Result<()> {
    let cwd = std::env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
    let home = std::env::var("HOME").ok();
    let leaks_identity = cwd.as_deref().is_some_and(|path| value.contains(path))
        || home.as_deref().is_some_and(|path| value.contains(path))
        || value.contains("/home/")
        || value.contains("/Users/")
        || value.contains(":\\")
        || manifest
            .repositories
            .iter()
            .any(|repository| value.contains(&repository.name));
    if value.len() > 1_024
        || value.trim().is_empty()
        || value.chars().any(char::is_control)
        || leaks_identity
    {
        return Err(EvalError::InvalidManifest(
            "release evidence contains a non-portable or private command/detail".to_string(),
        ));
    }
    Ok(())
}

fn sanitize_certification_text(value: &str, private_names: &[&str]) -> String {
    let mut sanitized = value.to_string();
    if let Ok(cwd) = std::env::current_dir() {
        sanitized = sanitized.replace(cwd.to_string_lossy().as_ref(), "<workspace>");
    }
    if let Ok(home) = std::env::var("HOME") {
        sanitized = sanitized.replace(&home, "<home>");
    }
    for name in private_names {
        sanitized = sanitized.replace(name, "<repository>");
    }
    sanitized
        .replace("/home/", "<home>/")
        .replace("/Users/", "<home>/")
        .chars()
        .take(1_024)
        .collect()
}

fn portable_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The baseline must describe the dataset that is actually on disk.
fn dataset_freshness_gate(args: &CertifyArgs, baseline: &TruthBaseline) -> Result<GateResult> {
    let digest = sha256_file(&args.dataset)?;
    let passed = digest == baseline.dataset_sha256;
    Ok(GateResult {
        gate: "dataset-freshness",
        passed,
        detail: if passed {
            format!("dataset {} matches the baseline", baseline.dataset_version)
        } else {
            format!(
                "dataset on disk hashes {digest}, baseline recorded {}",
                baseline.dataset_sha256
            )
        },
    })
}

/// Baseline and calibration must both use the approved thresholds.
fn threshold_gate(baseline: &TruthBaseline, calibration: &CalibrationArtifact) -> GateResult {
    let mismatches: Vec<&str> = [
        ("baseline", &baseline.thresholds),
        ("calibration", &calibration.thresholds),
    ]
    .into_iter()
    .filter(|(_, thresholds)| **thresholds != APPROVED_THRESHOLDS)
    .map(|(name, _)| name)
    .collect();
    GateResult {
        gate: "approved-thresholds",
        passed: mismatches.is_empty(),
        detail: if mismatches.is_empty() {
            "baseline and calibration use the approved thresholds".to_string()
        } else {
            format!(
                "{} deviates from the approved thresholds",
                mismatches.join(" and ")
            )
        },
    }
}

fn toolchain_gate(toolchain: &str) -> GateResult {
    let passed = toolchain.starts_with("rustc 1.97.");
    GateResult {
        gate: "rust-toolchain-identity",
        passed,
        detail: if passed {
            format!("{toolchain} matches the release certification toolchain")
        } else {
            format!("{toolchain} does not match the required Rust 1.97 toolchain")
        },
    }
}

fn tool_identity_gate(baseline: &TruthBaseline, shipped: &str) -> GateResult {
    let passed = baseline.tool_version == shipped;
    GateResult {
        gate: "tool-version-identity",
        passed,
        detail: if passed {
            format!("release binary and truth baseline both identify as {shipped}")
        } else {
            format!(
                "release binary identifies as {shipped}, truth baseline measured {}",
                baseline.tool_version
            )
        },
    }
}

/// The shipped binary must score with the model the baseline measured.
fn model_identity_gate(baseline: &TruthBaseline, shipped: &str) -> GateResult {
    let passed = baseline.score_model_version == shipped;
    GateResult {
        gate: "score-model-identity",
        passed,
        detail: if passed {
            format!("binary and baseline both use score model {shipped}")
        } else {
            format!(
                "binary ships score model {shipped}, baseline measured {}",
                baseline.score_model_version
            )
        },
    }
}

/// Every default score-eligible calibrated heuristic must clear its thresholds.
fn calibration_gate(
    catalog: &[CatalogEntry],
    calibration: &CalibrationArtifact,
    blocking: &mut BTreeMap<String, String>,
) -> GateResult {
    let records: BTreeMap<&str, &CalibrationRecord> = calibration
        .rules
        .iter()
        .map(|record| (record.rule.as_str(), record))
        .collect();
    let mut checked = 0;
    for entry in catalog {
        if !(entry.default_enabled
            && entry.trust.score_eligible
            && entry.trust.tier == "calibrated-heuristic")
        {
            continue;
        }
        checked += 1;
        match records.get(entry.canonical_id.as_str()) {
            None => {
                blocking.insert(
                    entry.canonical_id.clone(),
                    "no calibration record".to_string(),
                );
            }
            Some(record) => {
                if let Some(reason) = record.passes(&APPROVED_THRESHOLDS) {
                    blocking.insert(entry.canonical_id.clone(), reason);
                }
            }
        }
    }
    let failing = blocking.len();
    GateResult {
        gate: "heuristic-calibration",
        passed: failing == 0,
        detail: if failing == 0 {
            format!(
                "{checked} default score-eligible heuristic(s) cleared calibration {}",
                calibration.dataset_version
            )
        } else {
            format!("{failing} of {checked} default score-eligible heuristic(s) failed")
        },
    }
}

/// No score-eligible rule may ship with a failing or missing trust gate.
fn catalog_gate(catalog: &[CatalogEntry], blocking: &mut BTreeMap<String, String>) -> GateResult {
    let mut failing = 0;
    for entry in catalog {
        let scored = entry.trust.score_eligible;
        if scored
            && entry.trust.tier == "calibrated-heuristic"
            && entry.trust.calibration_version.is_none()
        {
            failing += 1;
            blocking.insert(
                entry.canonical_id.clone(),
                "score-eligible heuristic without a calibration version".to_string(),
            );
        }
        if matches!(entry.gate.status.as_str(), "failing" | "demotion-proposed") {
            failing += 1;
            blocking.insert(
                entry.canonical_id.clone(),
                format!(
                    "trust gate {} ({})",
                    entry.gate.status,
                    entry
                        .gate
                        .reason_code
                        .as_deref()
                        .unwrap_or("no reason code")
                ),
            );
        }
    }
    GateResult {
        gate: "catalog-trust",
        passed: failing == 0,
        detail: if failing == 0 {
            format!(
                "{} catalog rule(s) satisfy their trust contract",
                catalog.len()
            )
        } else {
            format!("{failing} catalog rule(s) violate their trust contract")
        },
    }
}

/// Every shipping adapter must have hash-bound conformance fixtures and a
/// declared qualified version range.
fn conformance_gate(directory: &Path) -> Result<(GateResult, Vec<AdapterEvidence>, String)> {
    let mut evidence = Vec::with_capacity(ADAPTER_VERSIONS.len());
    let mut missing = Vec::new();
    for (adapter, versions) in ADAPTER_VERSIONS {
        let adapter_path = directory.join(adapter);
        let passed = adapter_path.is_dir()
            && validate_adapter_conformance(adapter, &adapter_path, versions)?;
        if !passed {
            missing.push(adapter);
        }
        evidence.push(AdapterEvidence {
            adapter,
            supported_versions: versions,
            conformance_sha256: if passed {
                sha256_tree(&adapter_path)?
            } else {
                String::new()
            },
            passed,
        });
    }
    let declared: BTreeSet<_> = evidence.iter().map(|entry| entry.adapter).collect();
    let expected: BTreeSet<_> = AUTHORITY_ADAPTERS.into_iter().collect();
    if declared != expected {
        return Err(EvalError::InvalidManifest(
            "certification adapter matrix differs from the shipping adapter set".to_string(),
        ));
    }
    let gate = GateResult {
        gate: "adapter-conformance",
        passed: missing.is_empty(),
        detail: if missing.is_empty() {
            format!(
                "{} shipping adapter(s) have versioned conformance evidence",
                evidence.len()
            )
        } else {
            format!("no conformance fixtures for: {}", missing.join(", "))
        },
    };
    Ok((gate, evidence, sha256_tree(directory)?))
}

fn validate_adapter_conformance(
    adapter: &str,
    directory: &Path,
    versions: &[&str],
) -> Result<bool> {
    let read = |name: &str| {
        let path = directory.join(name);
        std::fs::read(&path)
            .map_err(|error| EvalError::io("cannot read conformance fixture", &path, error))
    };
    let json = |name: &str| -> Result<serde_json::Value> {
        let path = directory.join(name);
        serde_json::from_slice(&read(name)?).map_err(|source| EvalError::Json { path, source })
    };
    match adapter {
        "cargo-audit" => Ok(json("report.json")?["vulnerabilities"]["list"]
            .as_array()
            .is_some_and(|entries| !entries.is_empty())),
        "cargo-deny" => {
            let matrix = json("matrix.json")?;
            let captures = matrix["captures"].as_array().ok_or_else(|| {
                EvalError::InvalidManifest(
                    "cargo-deny conformance matrix has no captures".to_string(),
                )
            })?;
            if captures.len() != versions.len() {
                return Ok(false);
            }
            for version in versions {
                let capture = captures.iter().find(|capture| {
                    capture["version"]
                        .as_str()
                        .is_some_and(|value| value.starts_with(version))
                });
                let Some(capture) = capture else {
                    return Ok(false);
                };
                if capture["capture_status"] != "verified_real_binary" {
                    return Ok(false);
                }
                for (kind, field) in [("clean", "clean_fixture"), ("findings", "finding_fixture")] {
                    let Some(filename) = capture[field].as_str() else {
                        return Ok(false);
                    };
                    let digest = sha256_file(&directory.join(filename))?;
                    if capture["fixture_sha256"][kind].as_str() != Some(&digest) {
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        }
        "cargo-geiger" => Ok(String::from_utf8_lossy(&read("report.txt")?).contains("Dependency")),
        "cargo-metadata" => Ok(json("additive-fields.json")?["packages"]
            .as_array()
            .is_some_and(|packages| !packages.is_empty())),
        "cargo-semver-checks" => {
            Ok(String::from_utf8_lossy(&read("report.txt")?).contains("Summary semver requires"))
        }
        "cargo-shear" => Ok(json("report.json")?["findings"]
            .as_array()
            .is_some_and(|findings| !findings.is_empty())),
        "clippy" => {
            let stream_bytes = read("stream.jsonl")?;
            let degraded_bytes = read("degraded-stream.jsonl")?;
            let stream = String::from_utf8_lossy(&stream_bytes);
            let degraded = String::from_utf8_lossy(&degraded_bytes);
            Ok(stream.contains("\"reason\":\"compiler-message\"")
                && stream.contains("\"reason\":\"build-finished\"")
                && degraded.contains("\"reason\":\"future-cargo-event\"")
                && degraded.contains("\"success\":false"))
        }
        _ => Ok(false),
    }
}

/// The pinned corpus must stay complete enough to support the measurement.
fn corpus_gate(
    args: &CertifyArgs,
    manifest: &CorpusManifest,
    shipped_catalog: &[CatalogEntry],
    binary_sha256: &str,
) -> Result<(GateResult, GateResult, CorpusSummary)> {
    let records = corpus_records(&args.corpus)?;
    let baseline_records = corpus_records(&args.corpus_baseline)?;
    validate_corpus_approval(&args.corpus_approval, &args.corpus_baseline)?;
    let baseline_roots =
        validate_corpus_records(&baseline_records, manifest, shipped_catalog, None, None)?;
    validate_corpus_records(
        &records,
        manifest,
        shipped_catalog,
        Some((binary_sha256, args.tool_revision.as_str())),
        Some(&baseline_roots),
    )?;
    let summary = corpus_summary(&records)?;
    let baseline = corpus_summary(&baseline_records)?;

    let incompleteness = percentage(
        summary.roots_total.saturating_sub(summary.roots_complete),
        summary.roots_total,
    );
    let baseline_incompleteness = percentage(
        baseline.roots_total.saturating_sub(baseline.roots_complete),
        baseline.roots_total,
    );
    let regression = incompleteness - baseline_incompleteness;
    let passed = summary.roots_complete >= MIN_COMPLETE_ROOTS
        && summary.roots_total > 0
        && regression <= MAX_INCOMPLETENESS_REGRESSION;
    let gate = GateResult {
        gate: "corpus-completeness",
        passed,
        detail: format!(
            "{} of {} Cargo roots complete ({incompleteness:.2}% incomplete, {regression:+.2} pp vs baseline, binary and revision matched)",
            summary.roots_complete, summary.roots_total
        ),
    };
    let baseline_by_name: BTreeMap<_, _> = baseline_records
        .iter()
        .map(|record| (record.repository.as_str(), record))
        .collect();
    let mut runtime_deltas: Vec<_> = records
        .iter()
        .filter(|record| record.complete)
        .filter_map(|record| {
            let baseline_record = baseline_by_name.get(record.repository.as_str())?;
            baseline_record
                .complete
                .then(|| percentage_change(baseline_record.duration_ms, record.duration_ms))
        })
        .collect();
    runtime_deltas.sort_by(f64::total_cmp);
    let median_runtime_delta = percentile(&runtime_deltas, 50);
    let runtime_gate = GateResult {
        gate: "corpus-runtime-regression",
        passed: !runtime_deltas.is_empty() && median_runtime_delta <= 10.0,
        detail: format!(
            "median pinned-corpus runtime changed {median_runtime_delta:+.2}% across {} complete pair(s)",
            runtime_deltas.len()
        ),
    };
    Ok((gate, runtime_gate, summary))
}

#[expect(
    clippy::too_many_lines,
    reason = "corpus proof validation keeps manifest, root, catalog, and binary identity inseparable"
)]
fn validate_corpus_records(
    records: &[CorpusRecord],
    manifest: &CorpusManifest,
    shipped_catalog: &[CatalogEntry],
    expected_binary: Option<(&str, &str)>,
    expected_roots: Option<&BTreeMap<String, BTreeSet<String>>>,
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    if records.len() != manifest.repositories.len() {
        return Err(EvalError::InvalidManifest(
            "corpus repository count differs from the pinned manifest".to_string(),
        ));
    }
    let profile_sha256 = evaluation_profile_sha256(&manifest.evaluation_profile)?;
    let specs: BTreeMap<_, _> = manifest
        .repositories
        .iter()
        .map(|repository| (repository.name.as_str(), repository))
        .collect();
    let shipped: BTreeMap<_, _> = shipped_catalog
        .iter()
        .map(|entry| (entry.canonical_id.as_str(), entry.default_enabled))
        .collect();
    let mut names = BTreeSet::new();
    let mut roots_by_repository = BTreeMap::new();
    let mut catalog_hashes = BTreeSet::new();
    for record in records {
        if record.schema_version != CORPUS_SCHEMA_VERSION
            || !names.insert(record.repository.as_str())
        {
            return Err(EvalError::InvalidManifest(
                "corpus contains a duplicate repository or unsupported record schema".to_string(),
            ));
        }
        let spec = specs.get(record.repository.as_str()).ok_or_else(|| {
            EvalError::InvalidManifest(
                "corpus contains a repository outside the pinned manifest".to_string(),
            )
        })?;
        if record.commit != spec.commit
            || record.evaluation_profile_sha256 != profile_sha256
            || record.tree_digest.trim().is_empty()
        {
            return Err(EvalError::InvalidManifest(
                "corpus record is stale relative to its pinned source or evaluation profile"
                    .to_string(),
            ));
        }
        let roots: BTreeSet<_> = record.expected_roots.iter().cloned().collect();
        let packages: BTreeSet<_> = record.package_roots.iter().cloned().collect();
        let state_roots: BTreeSet<_> = record.root_states.keys().cloned().collect();
        let attempted: BTreeSet<_> = record.attempted_roots.iter().cloned().collect();
        let reported: BTreeSet<_> = record.reported_roots.iter().cloned().collect();
        if roots.len() != record.expected_roots.len()
            || roots.len() < spec.minimum_project_roots
            || roots.iter().any(|root| !safe_corpus_root(root))
            || packages != roots
            || state_roots != roots
            || !attempted.is_subset(&roots)
            || !reported.is_subset(&roots)
            || !reported.is_subset(&attempted)
            || expected_roots
                .and_then(|expected| expected.get(&record.repository))
                .is_some_and(|expected| expected != &roots)
        {
            return Err(EvalError::InvalidManifest(
                "corpus root coverage differs from its approved pinned root set".to_string(),
            ));
        }
        let internally_complete = attempted == roots
            && reported == roots
            && record
                .root_states
                .values()
                .all(|state| *state == RootState::Complete)
            && record.failure_chain.is_empty();
        if record.complete != internally_complete
            || record.complete != (record.completeness == "complete")
            || !(1..=3).contains(&record.attempts)
            || record.diagnostics.iter().any(|diagnostic| {
                diagnostic.repository != record.repository
                    || !roots.contains(&diagnostic.package_root)
            })
        {
            return Err(EvalError::InvalidManifest(
                "corpus completeness or diagnostic ownership is internally inconsistent"
                    .to_string(),
            ));
        }
        let catalog_bytes = serde_json::to_vec(&record.catalog).map_err(|error| {
            EvalError::InvalidManifest(format!("corpus catalog cannot be fingerprinted: {error}"))
        })?;
        let catalog_matches_release = expected_binary.is_none()
            || (record.catalog.len() == shipped.len()
                && record.catalog.iter().all(|(rule, entry)| {
                    shipped.get(rule.as_str()).copied() == Some(entry.default_enabled)
                }));
        if hex_digest(&catalog_bytes) != record.catalog_sha256 || !catalog_matches_release {
            return Err(EvalError::InvalidManifest(
                "corpus catalog does not match the shipped release catalog".to_string(),
            ));
        }
        catalog_hashes.insert(record.catalog_sha256.as_str());
        if let Some((binary, revision)) = expected_binary
            && (record.binary_sha256 != binary || record.tool_revision != revision)
        {
            return Err(EvalError::InvalidManifest(
                "candidate corpus is not bound to the certified binary and source revision"
                    .to_string(),
            ));
        }
        roots_by_repository.insert(record.repository.clone(), roots);
    }
    if names.len() != specs.len() || catalog_hashes.len() != 1 {
        return Err(EvalError::InvalidManifest(
            "corpus repository or catalog identity is inconsistent".to_string(),
        ));
    }
    Ok(roots_by_repository)
}

fn safe_corpus_root(root: &str) -> bool {
    root == "."
        || (!root.is_empty()
            && Path::new(root)
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))))
}

fn corpus_records(path: &Path) -> Result<Vec<CorpusRecord>> {
    if path.is_dir() {
        let mut record_paths = Vec::new();
        for entry in std::fs::read_dir(path)
            .map_err(|error| EvalError::io("cannot read corpus directory", path, error))?
        {
            let entry =
                entry.map_err(|error| EvalError::io("cannot read corpus entry", path, error))?;
            let record_path = entry.path();
            if record_path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                record_paths.push(record_path);
            }
        }
        record_paths.sort();
        return record_paths
            .iter()
            .map(|record_path| read_json(record_path))
            .collect();
    }
    let file = std::fs::File::open(path)
        .map_err(|error| EvalError::io("cannot read corpus NDJSON", path, error))?;
    let mut records = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|error| EvalError::io("cannot read corpus NDJSON", path, error))?;
        if line.trim().is_empty() {
            continue;
        }
        records.push(serde_json::from_str(&line).map_err(|error| {
            EvalError::InvalidManifest(format!(
                "{}:{} is invalid corpus NDJSON: {error}",
                path.display(),
                index + 1
            ))
        })?);
    }
    Ok(records)
}

fn corpus_summary(records: &[CorpusRecord]) -> Result<CorpusSummary> {
    if records.is_empty() {
        return Err(EvalError::InvalidManifest(
            "corpus evidence contains no repository records".to_string(),
        ));
    }
    let revisions: BTreeSet<_> = records
        .iter()
        .map(|record| record.tool_revision.as_str())
        .collect();
    let tool_revision = if revisions.len() == 1 {
        revisions
            .first()
            .map_or_else(String::new, |revision| (*revision).to_string())
    } else {
        "mixed".to_string()
    };
    let mut summary = CorpusSummary {
        repositories: records.len(),
        roots_total: 0,
        roots_complete: 0,
        roots_incomplete: 0,
        roots_failed: 0,
        roots_not_attempted: 0,
        tool_revision,
    };
    for state in records
        .iter()
        .flat_map(|record| record.root_states.values())
    {
        summary.roots_total += 1;
        match state {
            RootState::Complete => summary.roots_complete += 1,
            RootState::Incomplete => summary.roots_incomplete += 1,
            RootState::Failed => summary.roots_failed += 1,
            RootState::NotAttempted => summary.roots_not_attempted += 1,
        }
    }
    Ok(summary)
}

fn validate_corpus_approval(approval_path: &Path, baseline: &Path) -> Result<()> {
    let approval: EvidenceApproval = read_json(approval_path)?;
    let baseline_sha256 = sha256_file(baseline)?;
    if approval.schema_version != "1.0"
        || approval.subject_sha256 != baseline_sha256
        || approval.run_id == 0
        || approval.artifact_id == 0
        || approval.reviewed_by.trim().is_empty()
        || approval.reviewed_at.trim().is_empty()
    {
        return Err(EvalError::InvalidManifest(
            "corpus baseline is absent, stale, or lacks protected review evidence".to_string(),
        ));
    }
    Ok(())
}

fn percentage(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    // Corpus counts are in the hundreds; f64 represents them exactly.
    let part = u32::try_from(part).unwrap_or(u32::MAX);
    let whole = u32::try_from(whole).unwrap_or(u32::MAX);
    f64::from(part) * 100.0 / f64::from(whole)
}

fn percentage_change(baseline: u64, candidate: u64) -> f64 {
    if baseline == 0 {
        return if candidate == 0 { 0.0 } else { f64::INFINITY };
    }
    let baseline = u32::try_from(baseline).unwrap_or(u32::MAX);
    let candidate = u32::try_from(candidate).unwrap_or(u32::MAX);
    (f64::from(candidate) - f64::from(baseline)) * 100.0 / f64::from(baseline)
}

fn percentile(values: &[f64], percentile: usize) -> f64 {
    if values.is_empty() {
        return f64::INFINITY;
    }
    let index = values.len().saturating_sub(1).saturating_mul(percentile) / 100;
    values[index]
}

fn sha256_tree(root: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_tree_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = Sha256::new();
    for (relative, path) in files {
        let bytes = std::fs::read(&path)
            .map_err(|error| EvalError::io("cannot hash conformance fixture", &path, error))?;
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update(bytes.len().to_le_bytes());
        hasher.update(bytes);
    }
    Ok(hex_digest(&hasher.finalize()))
}

fn collect_tree_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| EvalError::io("cannot inspect conformance evidence", directory, error))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| EvalError::io("cannot inspect conformance evidence", directory, error))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| EvalError::io("cannot inspect conformance entry", &path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(EvalError::InvalidManifest(format!(
                "conformance evidence cannot contain symlink '{}'",
                path.display()
            )));
        }
        if metadata.is_dir() {
            collect_tree_files(root, &path, files)?;
        } else if metadata.is_file() {
            let relative = path.strip_prefix(root).map_err(|_| {
                EvalError::InvalidManifest(
                    "conformance entry escaped its evidence root".to_string(),
                )
            })?;
            files.push((portable_path(relative), path));
        }
    }
    Ok(())
}

fn rustc_identity() -> Result<String> {
    let mut command = Command::new("rustc");
    command.arg("--version");
    let output = certification_command(command, "rustc identity")?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn self_scan(binary: &Path, root: &Path) -> Result<SelfScanSummary> {
    let mut command = Command::new(binary);
    command.arg(root).args([
        "--json",
        "--json-compact",
        "--offline",
        "--no-project-config",
        "--blocking",
        "none",
        "--no-color",
    ]);
    let output = certification_command(command, "release self-scan")?;
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|error| EvalError::Json {
            path: PathBuf::from("<release-binary>"),
            source: error,
        })?;
    if report["report_constructed"].as_bool() != Some(true) {
        return Err(EvalError::Command(
            "release self-scan did not construct Report V1".to_string(),
        ));
    }
    let groups = report["root_causes"].as_array().ok_or_else(|| {
        EvalError::Command("release self-scan has no root-cause inventory".to_string())
    })?;
    let scored_groups = groups
        .iter()
        .filter(|group| group["score_impact"] == "scored")
        .count();
    let advisory_groups = groups
        .iter()
        .filter(|group| group["score_impact"] == "advisory")
        .count();
    let audit_groups = groups
        .iter()
        .filter(|group| group["trust_tier"] == "audit-only")
        .count();
    let top_remediations = groups
        .iter()
        .filter(|group| group["score_impact"] == "scored")
        .take(3)
        .map(|group| RemediationSummary {
            rule: group["rule"].as_str().unwrap_or("unknown").to_string(),
            title: group["title"]
                .as_str()
                .unwrap_or("Unknown rule")
                .to_string(),
            priority: group["priority"].as_str().map(str::to_string),
            sites: group["occurrences"].as_u64().unwrap_or(0),
            files: group["file_count"].as_u64().unwrap_or(0),
            score_penalty: group["current_penalty"].as_f64(),
        })
        .collect();
    Ok(SelfScanSummary {
        case: "rust-doctor-self-scan",
        score: report["summary"]["score"].as_u64(),
        label: report["summary"]["score_label"]
            .as_str()
            .map(str::to_string),
        authoritative: report["summary"]["score_authoritative"]
            .as_bool()
            .unwrap_or(false),
        reason_codes: report["summary"]["score_reasons"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect(),
        scored_groups,
        advisory_groups,
        audit_groups,
        top_remediations,
    })
}

fn self_scan_gate(summary: &SelfScanSummary, score_model_version: &str) -> GateResult {
    let passed = summary.authoritative && summary.score.is_some() && summary.label.is_some();
    GateResult {
        gate: "release-self-scan",
        passed,
        detail: if passed {
            format!(
                "release binary produced authoritative score model {score_model_version} with {} scored, {} advisory, and {} audit group(s)",
                summary.scored_groups, summary.advisory_groups, summary.audit_groups
            )
        } else {
            format!(
                "release self-scan is non-authoritative: {}",
                if summary.reason_codes.is_empty() {
                    "no canonical reason".to_string()
                } else {
                    summary.reason_codes.join(", ")
                }
            )
        },
    }
}

/// Read the catalog the binary actually ships, not a checked-in copy.
fn shipped_catalog(binary: &Path) -> Result<Vec<CatalogEntry>> {
    let mut command = Command::new(binary);
    command.args(["rules", "list", "--json", "--no-project-config"]);
    let output = certification_command(command, "shipped catalog")?;
    serde_json::from_slice(&output.stdout).map_err(|source| EvalError::Json {
        path: PathBuf::from("<release-binary>"),
        source,
    })
}

/// Read the score-model identifier the binary stamps into every report.
fn shipped_score_model(binary: &Path) -> Result<String> {
    let directory = tempfile::tempdir().map_err(|source| EvalError::Io {
        action: "create certification workspace",
        path: PathBuf::from("<release-binary>"),
        source,
    })?;
    let mut command = Command::new(binary);
    command
        .arg(directory.path())
        .args(["--json", "--no-project-config"]);
    let output = certification_command(command, "score-model probe")?;
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|source| EvalError::Json {
            path: PathBuf::from("<release-binary>"),
            source,
        })?;
    report
        .get("score_model_version")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            EvalError::Command("the probe report carries no score_model_version".to_string())
        })
}

fn shipped_tool_version(binary: &Path) -> Result<String> {
    let mut command = Command::new(binary);
    command.arg("--version");
    let output = certification_command(command, "release binary identity")?;
    let value = String::from_utf8_lossy(&output.stdout);
    let version = value
        .trim()
        .strip_prefix("rust-doctor ")
        .unwrap_or_else(|| value.trim());
    if version.is_empty() {
        return Err(EvalError::Command(
            "release binary emitted no version identity".to_string(),
        ));
    }
    Ok(version.to_string())
}

fn certification_command(command: Command, label: &str) -> Result<ProcessOutput> {
    let output = run_capped(command, CERTIFICATION_TIMEOUT, CERTIFICATION_OUTPUT_CAP)?;
    if output.timed_out {
        return Err(EvalError::Command(format!(
            "{label} exceeded the five-minute certification timeout"
        )));
    }
    if output.output_overflow {
        return Err(EvalError::Command(format!(
            "{label} exceeded the certification output cap"
        )));
    }
    if !output.status.success() {
        return Err(EvalError::Command(format!("{label} exited unsuccessfully")));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(recall: Option<f64>, upper_bound: Option<f64>, coverage: f64) -> CalibrationRecord {
        CalibrationRecord {
            rule: "demo".to_string(),
            evidence_complete: true,
            positive_samples: 50,
            negative_samples: 149,
            recall,
            false_positive_upper_bound: upper_bound,
            confidence_level: 0.95,
            context_coverage: coverage,
        }
    }

    #[test]
    fn a_record_clearing_every_threshold_passes() {
        assert!(
            record(Some(0.9), Some(0.01), 0.95)
                .passes(&APPROVED_THRESHOLDS)
                .is_none()
        );
    }

    #[test]
    fn an_unavailable_metric_never_counts_as_success() {
        assert!(
            record(None, Some(0.0), 1.0)
                .passes(&APPROVED_THRESHOLDS)
                .is_some()
        );
        assert!(
            record(Some(1.0), None, 1.0)
                .passes(&APPROVED_THRESHOLDS)
                .is_some()
        );
        let mut incomplete = record(Some(1.0), Some(0.0), 1.0);
        incomplete.evidence_complete = false;
        assert!(incomplete.passes(&APPROVED_THRESHOLDS).is_some());
    }

    #[test]
    fn thresholds_are_enforced_at_their_exact_boundaries() {
        assert!(
            record(Some(0.8), Some(0.02), 0.9)
                .passes(&APPROVED_THRESHOLDS)
                .is_none()
        );
        assert!(
            record(Some(0.799), Some(0.02), 0.9)
                .passes(&APPROVED_THRESHOLDS)
                .is_some()
        );
        assert!(
            record(Some(0.8), Some(0.021), 0.9)
                .passes(&APPROVED_THRESHOLDS)
                .is_some()
        );
        assert!(
            record(Some(0.8), Some(0.02), 0.899)
                .passes(&APPROVED_THRESHOLDS)
                .is_some()
        );
    }

    #[test]
    fn empty_corpus_evidence_is_rejected() {
        assert!(corpus_summary(&[]).is_err());
    }

    #[test]
    fn a_missing_conformance_matrix_blocks_certification() {
        let directory = tempfile::tempdir().unwrap();
        let (gate, _, _) = conformance_gate(directory.path()).unwrap();
        assert!(!gate.passed);
        assert!(gate.detail.contains("cargo-audit"));
    }

    #[test]
    fn the_checked_conformance_matrices_cover_every_authority_adapter() {
        let (gate, evidence, _) = conformance_gate(Path::new("evaluation/conformance")).unwrap();
        assert!(gate.passed, "{}", gate.detail);
        assert_eq!(evidence.len(), AUTHORITY_ADAPTERS.len());
    }

    #[test]
    fn certification_schema_accepts_checked_manifest() {
        let path = Path::new("evaluation/certifications/decision-quality-v1.json");
        if !path.is_file() {
            return;
        }
        let schema: serde_json::Value =
            read_json(Path::new("evaluation/schemas/certification-v2.schema.json")).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let manifest: serde_json::Value = read_json(path).unwrap();
        let errors: Vec<_> = validator
            .iter_errors(&manifest)
            .map(|error| error.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn certification_schema_accepts_minimal_failure_evidence() {
        let schema: serde_json::Value =
            read_json(Path::new("evaluation/schemas/certification-v2.schema.json")).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let failure = serde_json::json!({
            "schema_version": "2.0",
            "certified": false,
            "generated_at_utc_unix_seconds": 1,
            "command": ["rust-doctor-eval", "certify"],
            "error": {
                "code": "certification-input-invalid",
                "message": "required corpus evidence is missing"
            }
        });
        let errors: Vec<_> = validator
            .iter_errors(&failure)
            .map(|error| error.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
