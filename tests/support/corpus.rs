use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rust_doctor::{RuleTier, catalog};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) mod agreement;
pub(crate) mod coefficients;
pub(crate) mod comparison;
pub(crate) mod interval;
pub(crate) mod position;
pub(crate) mod reproduction;
pub(crate) mod sampling;

use agreement::{Agreement, ProtocolScope};
use comparison::RateComparison;
use interval::Separation;
use position::PositionProof;
use sampling::SamplingPlan;

pub(crate) const THRESHOLD_BASIS_POINTS: u64 = 500;

pub(crate) const EXPECTED_REPOSITORIES: usize = 10;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Verdict {
    FalsePositive,
    TruePositive,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PrecisionStatus {
    Measured,
    Incomplete,
    Unobserved,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GateVerdict {
    Failed,
    Passed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RefusalReason {
    ZeroToleranceTierWithFalsePositive,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RepositoryOutcome {
    Processed,
    Skipped,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CorpusArtifact {
    pub(crate) adjudication: Adjudication,
    pub(crate) agent_population: AgentPopulation,
    pub(crate) artifact: String,
    pub(crate) catalog: Vec<CatalogRule>,
    pub(crate) epic: String,
    pub(crate) gate: GateOutcome,
    pub(crate) generated_at: String,
    pub(crate) harness: HarnessEvidence,
    pub(crate) lambdas: Vec<LambdaObservation>,
    pub(crate) manifest: Manifest,
    pub(crate) network_in_automated_tests: bool,
    pub(crate) observations: Vec<Observation>,
    pub(crate) precision: Vec<RulePrecision>,
    pub(crate) schema_version: u64,
    pub(crate) score_distribution: ScoreDistribution,
    pub(crate) smoothing: Smoothing,
    pub(crate) toolchain: Toolchain,
    pub(crate) trust_boundary: TrustBoundary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentPopulation {
    pub(crate) measured_rules: u64,
    pub(crate) observations: Vec<Observation>,
    pub(crate) precision: Vec<RulePrecision>,
    pub(crate) rate_comparison: Vec<RateComparison>,
    pub(crate) repositories: Vec<AgentRepository>,
    pub(crate) scan_arguments: Vec<String>,
    pub(crate) selection_criterion: String,
    pub(crate) structural_density: DensityComparison,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentRepository {
    pub(crate) commit: String,
    pub(crate) marker: String,
    pub(crate) marked_commits: u64,
    pub(crate) name: String,
    pub(crate) rationale: String,
    pub(crate) total_commits: u64,
    pub(crate) url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DensityComparison {
    pub(crate) agent: StructuralDensity,
    pub(crate) healthy: StructuralDensity,
    pub(crate) ratio_milli: u64,
    pub(crate) refutes_density_assumption: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StructuralDensity {
    pub(crate) findings: u64,
    pub(crate) lines: u64,
    pub(crate) per_thousand_lines_milli: u64,
}

impl AgentPopulation {
    pub(crate) fn manifest(&self) -> Manifest {
        Manifest {
            repositories: self
                .repositories
                .iter()
                .map(|repository| ManifestEntry {
                    commit: repository.commit.clone(),
                    name: repository.name.clone(),
                    rationale: repository.rationale.clone(),
                    shape: RepositoryShape {
                        asynchronous: false,
                        binary: false,
                        library: false,
                        proc_macro: false,
                        workspace_members: 1,
                    },
                    tag: "pinned".to_owned(),
                    url: repository.url.clone(),
                })
                .collect(),
        }
    }
}

pub(crate) fn structural_findings(observations: &[Observation]) -> u64 {
    observations
        .iter()
        .flat_map(|observation| observation.rules.iter())
        .filter(|rule| rule.id.starts_with("rust_doctor::structure::"))
        .map(|rule| rule.distinct)
        .sum()
}

pub(crate) fn density_milli(findings: u64, lines: u64) -> u64 {
    if lines == 0 {
        return 0;
    }
    findings.saturating_mul(1_000_000) / lines
}

pub(crate) fn rust_line_count(root: &Path) -> u64 {
    fn visit(directory: &Path, total: &mut u64) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        let mut paths: Vec<_> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                visit(&path, total);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && let Ok(source) = fs::read_to_string(&path)
            {
                *total += source.lines().count() as u64;
            }
        }
    }
    let mut total = 0;
    visit(root, &mut total);
    total
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LambdaObservation {
    pub(crate) name: String,
    pub(crate) lambda_micro: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Smoothing {
    pub(crate) prior_false_positives: u64,
    pub(crate) prior_sites: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Toolchain {
    pub(crate) cargo: String,
    pub(crate) clippy: String,
    pub(crate) rustc: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TrustBoundary {
    pub(crate) clippy_executes_corpus_build_code: bool,
    pub(crate) corpus_materialised_outside_repository: bool,
    pub(crate) native_detectors_compile_corpus_code: bool,
    pub(crate) rationale: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HarnessEvidence {
    pub(crate) artifacts_directory_env: String,
    pub(crate) cache_directory_env: String,
    pub(crate) failed: usize,
    pub(crate) processed: usize,
    pub(crate) scan_arguments: Vec<String>,
    pub(crate) skipped: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub(crate) repositories: Vec<ManifestEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestEntry {
    pub(crate) commit: String,
    pub(crate) name: String,
    pub(crate) rationale: String,
    pub(crate) shape: RepositoryShape,
    pub(crate) tag: String,
    pub(crate) url: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RepositoryShape {
    pub(crate) asynchronous: bool,
    pub(crate) binary: bool,
    pub(crate) library: bool,
    pub(crate) proc_macro: bool,
    pub(crate) workspace_members: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogRule {
    pub(crate) default_level: String,
    pub(crate) id: String,
    pub(crate) tier: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Observation {
    pub(crate) authoritative: bool,
    pub(crate) commit: String,
    pub(crate) distinct: u64,
    pub(crate) exit_code: i32,
    pub(crate) findings_digest: String,
    pub(crate) name: String,
    pub(crate) occurrences: u64,
    pub(crate) outcome: RepositoryOutcome,
    pub(crate) production_lines: u64,
    pub(crate) reasons: Vec<String>,
    pub(crate) rules: Vec<RuleObservation>,
    pub(crate) score: Option<ScoreObservation>,
    pub(crate) status: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleObservation {
    pub(crate) distinct: u64,
    pub(crate) id: String,
    pub(crate) occurrences: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScoreObservation {
    pub(crate) applied_ceiling: Option<u64>,
    pub(crate) dimensions: Vec<DimensionObservation>,
    pub(crate) label: String,
    pub(crate) model: String,
    pub(crate) value: u64,
    pub(crate) worst_tier: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DimensionObservation {
    pub(crate) density_micro: u64,
    pub(crate) name: String,
    pub(crate) score: u64,
    pub(crate) worst_tier: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Adjudication {
    pub(crate) adjudicated_after_cutoff: Vec<ProtocolScope>,
    pub(crate) agreement: Agreement,
    pub(crate) criterion: String,
    pub(crate) evidence_cutoff: String,
    pub(crate) position_proof: PositionProof,
    pub(crate) protocol_cutoff: String,
    pub(crate) provenance: String,
    pub(crate) reviewed: Vec<ReviewedSite>,
    pub(crate) sampling: String,
    pub(crate) sampling_plan: Vec<SamplingPlan>,
    pub(crate) trigger_verification: TriggerVerification,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TriggerVerification {
    pub(crate) confirmed: u64,
    pub(crate) findings: u64,
    pub(crate) method: String,
    pub(crate) triggers: Vec<RuleTrigger>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleTrigger {
    pub(crate) evidence: String,
    pub(crate) rule: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SiteContext {
    Benchmark,
    BuildScript,
    Example,
    Production,
    Tests,
}

impl SiteContext {
    pub(crate) fn matches(self, reported: Option<&str>) -> bool {
        let expected = match self {
            Self::Benchmark => Some("benchmark"),
            Self::BuildScript => Some("build-script"),
            Self::Example => Some("example"),
            Self::Production => None,
            Self::Tests => Some("tests"),
        };
        expected == reported
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Population {
    Agent,
    Healthy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Provenance {
    Agent,
    Human,
    Unrecorded,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewedSite {
    pub(crate) context: SiteContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) family: Option<String>,
    pub(crate) justification: String,
    pub(crate) line: u64,
    pub(crate) path: String,
    pub(crate) population: Population,
    pub(crate) provenance: Provenance,
    pub(crate) repository: String,
    pub(crate) rule: String,
    pub(crate) verdict: Verdict,
}

pub(crate) const MINIMUM_REVIEWED_SITES: u64 = 5;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RulePrecision {
    pub(crate) doubly_judged: u64,
    pub(crate) false_positive_rate_basis_points: Option<u64>,
    pub(crate) false_positives: Option<u64>,
    pub(crate) findings: u64,
    pub(crate) id: String,
    pub(crate) interval_high_basis_points: Option<u64>,
    pub(crate) interval_low_basis_points: Option<u64>,
    pub(crate) judges: Vec<String>,
    pub(crate) provenance: Vec<Provenance>,
    pub(crate) reviewed: u64,
    pub(crate) separation: Option<Separation>,
    pub(crate) status: PrecisionStatus,
    pub(crate) tier: String,
    pub(crate) true_positives: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateOutcome {
    pub(crate) admitted: Vec<String>,
    pub(crate) indecisive: Vec<String>,
    pub(crate) noisy_on_healthy_code: Vec<String>,
    pub(crate) refused: Vec<GateRefusal>,
    pub(crate) threshold_basis_points: u64,
    pub(crate) unproven: Vec<String>,
    pub(crate) verdict: GateVerdict,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateRefusal {
    pub(crate) id: String,
    pub(crate) reason: RefusalReason,
}

pub(crate) const MINIMUM_SPREAD: u64 = 20;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScoreDistribution {
    pub(crate) agent: PopulationDistribution,
    pub(crate) healthy: PopulationDistribution,
    pub(crate) maximum: u64,
    pub(crate) minimum: u64,
    pub(crate) separation_centi: i64,
    pub(crate) spread: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PopulationDistribution {
    pub(crate) bands: Vec<ScoreBand>,
    pub(crate) ceilings_applied: usize,
    pub(crate) collapsed_into_one_band: bool,
    pub(crate) collapsed_into_one_value: bool,
    pub(crate) maximum: u64,
    pub(crate) median_centi: u64,
    pub(crate) minimum: u64,
    pub(crate) spread: u64,
    pub(crate) values: Vec<ScoreValue>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScoreBand {
    pub(crate) label: String,
    pub(crate) repositories: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScoreValue {
    pub(crate) label: String,
    pub(crate) name: String,
    pub(crate) value: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HarnessRun {
    pub(crate) failed: Vec<String>,
    pub(crate) observations: Vec<Observation>,
    pub(crate) processed: Vec<String>,
    pub(crate) skipped: Vec<String>,
}

impl HarnessRun {
    pub(crate) fn evidence(&self, scan_arguments: &[&str]) -> HarnessEvidence {
        HarnessEvidence {
            artifacts_directory_env: ARTIFACTS_DIRECTORY_ENV.to_owned(),
            cache_directory_env: CACHE_DIRECTORY_ENV.to_owned(),
            failed: self.failed.len(),
            processed: self.processed.len(),
            scan_arguments: scan_arguments.iter().map(|value| (*value).to_owned()).collect(),
            skipped: self.skipped.len(),
        }
    }
}

pub(crate) const CACHE_DIRECTORY_ENV: &str = "RUST_DOCTOR_CORPUS_DIR";
pub(crate) const ARTIFACTS_DIRECTORY_ENV: &str = "RUST_DOCTOR_CORPUS_ARTIFACTS";

pub(crate) fn artifact_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus.json")
}

pub(crate) fn artifact() -> CorpusArtifact {
    let bytes = fs::read(artifact_path()).expect("the corpus artifact should be readable");
    serde_json::from_slice(&bytes).expect("the corpus artifact should match its typed schema")
}

pub(crate) fn manifest_defects(manifest: &Manifest) -> Vec<String> {
    let mut defects = Vec::new();
    if manifest.repositories.len() != EXPECTED_REPOSITORIES {
        defects.push(format!(
            "repository-count: expected {EXPECTED_REPOSITORIES}, found {}",
            manifest.repositories.len()
        ));
    }

    let mut seen = BTreeSet::new();
    for entry in &manifest.repositories {
        let name = entry.name.as_str();
        if !name
            .chars()
            .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == '-' || value == '_')
            || name.is_empty()
        {
            defects.push(format!("{name}: name-not-a-plain-identifier"));
        }
        if !seen.insert(name) {
            defects.push(format!("{name}: duplicate-repository"));
        }
        if !is_immutable_revision(&entry.commit) {
            defects.push(format!("{name}: revision-not-immutable"));
        }
        if entry.tag.trim().is_empty() {
            defects.push(format!("{name}: tag-missing"));
        }
        if entry.rationale.trim().is_empty() {
            defects.push(format!("{name}: rationale-missing"));
        }
        if !entry.url.starts_with("https://") || !entry.url.ends_with(".git") {
            defects.push(format!("{name}: url-not-a-pinned-https-remote"));
        }
    }

    let shapes = manifest.repositories.iter().map(|entry| entry.shape);
    let mut binary = false;
    let mut library = false;
    let mut workspace = false;
    let mut asynchronous = false;
    for shape in shapes {
        binary |= shape.binary;
        library |= shape.library;
        workspace |= shape.workspace_members >= 2;
        asynchronous |= shape.asynchronous;
    }
    for (covered, kind) in [
        (binary, "binary"),
        (library, "library"),
        (workspace, "multi-member-workspace"),
        (asynchronous, "asynchronous"),
    ] {
        if !covered {
            defects.push(format!("coverage-missing: {kind}"));
        }
    }
    defects
}

fn is_immutable_revision(commit: &str) -> bool {
    commit.len() == 40 && commit.chars().all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
}

pub(crate) fn missing_repositories(cache: &Path, manifest: &Manifest) -> Vec<String> {
    manifest
        .repositories
        .iter()
        .filter(|entry| head_revision(&cache.join(&entry.name)).as_deref() != Some(entry.commit.as_str()))
        .map(|entry| entry.name.clone())
        .collect()
}

fn head_revision(repository: &Path) -> Option<String> {
    if !repository.join(".git").exists() {
        return None;
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub(crate) struct HarnessPaths<'a> {
    pub(crate) artifacts: &'a Path,
    pub(crate) binary: &'a Path,
    pub(crate) cache: &'a Path,
    pub(crate) unpinned: bool,
}

pub(crate) fn run(
    paths: &HarnessPaths<'_>,
    manifest: &Manifest,
    scan_arguments: &[&str],
) -> Result<HarnessRun, Vec<String>> {
    let missing = missing_repositories(paths.cache, manifest);
    if !missing.is_empty() {
        return Err(missing);
    }

    let mut run = HarnessRun {
        failed: Vec::new(),
        observations: Vec::new(),
        processed: Vec::new(),
        skipped: Vec::new(),
    };
    for entry in &manifest.repositories {
        let work = fresh_directory(&paths.artifacts.join("work").join(&entry.name));
        let report_path = paths.artifacts.join("reports").join(format!("{}.json", entry.name));
        remove_if_present(&report_path);
        materialise(&paths.cache.join(&entry.name), &entry.commit, &work);

        if !work.join("Cargo.toml").is_file() {
            run.skipped.push(entry.name.clone());
            run.observations.push(skipped_observation(entry));
            continue;
        }

        let target = fresh_directory(&paths.artifacts.join("target").join(&entry.name));
        let mut scan = Command::new(paths.binary);
        scan.arg("inspect")
            .arg(&work)
            .args(scan_arguments)
            .env("CARGO_TARGET_DIR", &target)
            .env("RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS", "600");
        if paths.unpinned {
            scan.env_remove("RUSTUP_TOOLCHAIN");
        }
        let scan = scan.output();

        let Ok(output) = scan else {
            run.failed.push(entry.name.clone());
            run.observations.push(failed_observation(entry, -1));
            continue;
        };
        let exit_code = output.status.code().unwrap_or(-1);
        let Ok(report) = serde_json::from_slice::<Value>(&output.stdout) else {
            run.failed.push(entry.name.clone());
            run.observations.push(failed_observation(entry, exit_code));
            continue;
        };
        write_atomically(&report_path, &output.stdout);

        let observation = observation(entry, exit_code, &report);
        if observation.outcome == RepositoryOutcome::Failed {
            run.failed.push(entry.name.clone());
        } else {
            run.processed.push(entry.name.clone());
        }
        run.observations.push(observation);
    }
    Ok(run)
}

pub(crate) fn materialise(repository: &Path, commit: &str, work: &Path) {
    let clone = Command::new("git")
        .args(["clone", "--quiet", "--shared", "--no-checkout"])
        .arg(repository)
        .arg(work)
        .output()
        .expect("git should start");
    assert!(clone.status.success(), "the cache should clone: {}", String::from_utf8_lossy(&clone.stderr));

    let checkout = Command::new("git")
        .arg("-C")
        .arg(work)
        .args(["checkout", "--quiet", "--detach", commit])
        .output()
        .expect("git should start");
    assert!(checkout.status.success(), "the pinned revision should check out");
}

fn fresh_directory(path: &Path) -> PathBuf {
    if path.exists() {
        fs::remove_dir_all(path).expect("a stale artifact directory should be removable");
    }
    fs::create_dir_all(path).expect("an artifact directory should be creatable");
    path.to_path_buf()
}

fn remove_if_present(path: &Path) {
    if path.exists() {
        fs::remove_file(path).expect("a stale artifact file should be removable");
    }
}

pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) {
    let parent = path.parent().expect("an artifact file should sit in a directory");
    fs::create_dir_all(parent).expect("an artifact directory should be creatable");
    let staging = parent.join(format!(
        "{}.partial",
        path.file_name()
            .expect("an artifact file should carry a name")
            .to_string_lossy()
    ));
    fs::write(&staging, bytes).expect("an artifact file should be writable");
    fs::rename(&staging, path).expect("an artifact file should be publishable");
}

fn skipped_observation(entry: &ManifestEntry) -> Observation {
    Observation {
        authoritative: false,
        commit: entry.commit.clone(),
        distinct: 0,
        exit_code: 0,
        findings_digest: digest(&[]),
        name: entry.name.clone(),
        occurrences: 0,
        outcome: RepositoryOutcome::Skipped,
        production_lines: 0,
        reasons: Vec::new(),
        rules: Vec::new(),
        score: None,
        status: "skipped".to_owned(),
    }
}

fn failed_observation(entry: &ManifestEntry, exit_code: i32) -> Observation {
    Observation {
        authoritative: false,
        commit: entry.commit.clone(),
        distinct: 0,
        exit_code,
        findings_digest: digest(&[]),
        name: entry.name.clone(),
        occurrences: 0,
        outcome: RepositoryOutcome::Failed,
        production_lines: 0,
        reasons: Vec::new(),
        rules: Vec::new(),
        score: None,
        status: "failed".to_owned(),
    }
}

const DIMENSION_NAMES: [&str; 5] = [
    "security",
    "reliability",
    "maintainability",
    "performance",
    "dependencies",
];

fn dimension_of(category: &str) -> Option<&'static str> {
    match category {
        "security" => Some("security"),
        "correctness" | "reliability" => Some("reliability"),
        "performance" => Some("performance"),
        "cargo" | "dependencies" => Some("dependencies"),
        "maintainability" => Some("maintainability"),
        _ => None,
    }
}

fn severity_weight(severity: &str) -> Option<u64> {
    match severity {
        "error" => Some(2),
        "warning" => Some(1),
        "info" => Some(0),
        _ => None,
    }
}

fn tier_weight(tier: Option<RuleTier>) -> u64 {
    match tier {
        Some(RuleTier::P0) => 8,
        Some(RuleTier::P1) => 4,
        Some(RuleTier::P2) => 2,
        Some(RuleTier::P3) | None => 1,
    }
}

fn noise_basis_points(report: &Value, rule: &str) -> Option<u64> {
    report["policy"]["rules"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .find(|entry| entry["id"].as_str() == Some(rule))
        .and_then(|entry| entry["corpus_noise_basis_points"].as_u64())
}

fn kept_basis_points(report: &Value, rule: &str) -> u64 {
    noise_basis_points(report, rule).map_or(10_000, |noise| 10_000u64.saturating_sub(noise))
}

fn is_withheld(report: &Value, rule: &str) -> bool {
    noise_basis_points(report, rule).is_some_and(|noise| noise > 5_000)
}

fn divisor(rule: &str, shipped: &BTreeMap<&str, RuleTier>, kilolines: f64) -> f64 {
    let per_kiloline = shipped.contains_key(rule)
        && (rule.starts_with("clippy::")
            || rule.starts_with("rust_doctor::source::")
            || rule.starts_with("rust_doctor::structure::"));
    if per_kiloline { kilolines } else { 1.0 }
}

fn dimension_observations(report: &Value, production_lines: u64) -> Vec<DimensionObservation> {
    let shipped: BTreeMap<&str, RuleTier> =
        catalog().into_iter().map(|rule| (rule.id, rule.tier)).collect();
    let kilolines = (production_lines as f64 / 1000.0).max(2.0);
    let members = report["audit"]["packages"]
        .as_array()
        .map_or(1, Vec::len)
        .max(1) as f64;

    let mut scored: BTreeMap<&str, (&'static str, u64, Option<RuleTier>, u64)> = BTreeMap::new();
    for diagnostic in report["diagnostics"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        if !diagnostic["context"].is_null() {
            continue;
        }
        let (Some(rule), Some(category), Some(severity)) = (
            diagnostic["code"].as_str().filter(|code| !code.is_empty()),
            diagnostic["category"].as_str(),
            diagnostic["severity"].as_str(),
        ) else {
            continue;
        };
        let (Some(dimension), Some(weight)) = (dimension_of(category), severity_weight(severity))
        else {
            continue;
        };
        if weight == 0 {
            continue;
        }
        let entry = scored
            .entry(rule)
            .or_insert((dimension, 0, shipped.get(rule).copied(), 0));
        entry.1 = entry.1.saturating_add(weight);
        if !diagnostic["package"].is_null() {
            entry.3 = entry.3.saturating_add(weight);
        }
    }

    let published = &report["audit"]["score"]["dimensions"];
    DIMENSION_NAMES
        .into_iter()
        .map(|name| {
            let mut density = 0.0;
            let mut worst: Option<RuleTier> = None;
            for (rule, (dimension, numerator, tier, owned)) in &scored {
                if *dimension != name {
                    continue;
                }
                let charge = |sites: u64| {
                    let weighted = sites.saturating_mul(tier_weight(*tier));
                    weighted as f64 * kept_basis_points(report, rule) as f64 / 10_000.0
                };
                let divisor = divisor(rule, &shipped, kilolines);
                density += if divisor == 1.0 {
                    let owned = (*owned).min(*numerator);
                    charge(numerator - owned) + charge(owned) / members
                } else {
                    charge(*numerator) / divisor
                };
                let caps = if is_withheld(report, rule) { None } else { *tier };
                worst = match (worst, caps) {
                    (Some(current), Some(candidate)) => Some(current.min(candidate)),
                    (current, candidate) => current.or(candidate),
                };
            }
            DimensionObservation {
                density_micro: (density * 1_000_000.0).round() as u64,
                name: name.to_owned(),
                score: published[name].as_u64().unwrap_or_default(),
                worst_tier: worst.map(|tier| tier.as_str().to_owned()),
            }
        })
        .collect()
}

fn observation(entry: &ManifestEntry, exit_code: i32, report: &Value) -> Observation {
    let status = report["status"].as_str().unwrap_or("failed").to_owned();
    let findings = curated_findings(report);
    let mut rules: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    let mut occurrences = 0;
    for finding in &findings {
        let entry = rules.entry(finding.rule).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += finding.occurrences;
        occurrences += finding.occurrences;
    }

    let production_lines = report["audit"]["production_lines"].as_u64().unwrap_or_default();
    let score = report["audit"]["score"].as_object().map(|score| ScoreObservation {
        applied_ceiling: score["applied_ceiling"].as_u64(),
        dimensions: dimension_observations(report, production_lines),
        label: score["label"].as_str().unwrap_or_default().to_owned(),
        model: score["model"].as_str().unwrap_or_default().to_owned(),
        value: score["value"].as_u64().unwrap_or_default(),
        worst_tier: score["worst_tier"].as_str().map(str::to_owned),
    });

    Observation {
        authoritative: report["audit"]["score"]["authoritative"] == Value::Bool(true),
        commit: entry.commit.clone(),
        distinct: findings.len() as u64,
        exit_code,
        findings_digest: digest(&findings),
        name: entry.name.clone(),
        occurrences,
        outcome: if status == "failed" {
            RepositoryOutcome::Failed
        } else {
            RepositoryOutcome::Processed
        },
        production_lines,
        reasons: report["audit"]["score"]["reasons"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        rules: rules
            .into_iter()
            .map(|(id, (distinct, occurrences))| RuleObservation {
                distinct,
                id: id.to_owned(),
                occurrences,
            })
            .collect(),
        score,
        status,
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Finding<'a> {
    pub(crate) column: u64,
    pub(crate) column_end: u64,
    pub(crate) line: u64,
    pub(crate) line_end: u64,
    pub(crate) occurrences: u64,
    pub(crate) path: &'a str,
    pub(crate) rule: &'a str,
    pub(crate) context: Option<&'a str>,
    pub(crate) id: &'a str,
}

pub(crate) fn curated_findings(report: &Value) -> Vec<Finding<'_>> {
    let mut findings: Vec<_> = report["diagnostics"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|diagnostic| !diagnostic["category"].is_null())
        .map(|diagnostic| Finding {
            column: diagnostic["span"]["column_start"].as_u64().unwrap_or_default(),
            column_end: diagnostic["span"]["column_end"].as_u64().unwrap_or_default(),
            line: diagnostic["span"]["line_start"].as_u64().unwrap_or_default(),
            line_end: diagnostic["span"]["line_end"].as_u64().unwrap_or_default(),
            occurrences: diagnostic["occurrences"].as_u64().unwrap_or_default(),
            path: diagnostic["path"].as_str().unwrap_or_default(),
            rule: diagnostic["code"].as_str().unwrap_or_default(),
            context: diagnostic["context"].as_str(),
            id: diagnostic["id"].as_str().unwrap_or_default(),
        })
        .collect();
    findings.sort_unstable();
    findings
}

pub(crate) fn span_text(root: &Path, finding: &Finding<'_>) -> Option<String> {
    if finding.line == 0 {
        return None;
    }
    let source = fs::read_to_string(root.join(finding.path)).ok()?;
    let lines: Vec<&str> = source.split('\n').collect();
    let first = lines.get(finding.line as usize - 1)?;
    let last = lines.get(finding.line_end as usize - 1)?;
    if finding.line == finding.line_end {
        return Some(
            first
                .get(finding.column as usize - 1..finding.column_end as usize - 1)
                .unwrap_or(first)
                .to_owned(),
        );
    }
    let mut text = first.get(finding.column as usize - 1..).unwrap_or(first).to_owned();
    for line in &lines[finding.line as usize..finding.line_end as usize - 1] {
        text.push('\n');
        text.push_str(line);
    }
    text.push('\n');
    text.push_str(last.get(..finding.column_end as usize - 1).unwrap_or(last));
    Some(text)
}

pub(crate) fn evidence_holds(root: &Path, finding: &Finding<'_>, evidence: &str) -> bool {
    match span_text(root, finding) {
        Some(text) => text.contains(evidence),
        None => fs::read_to_string(root.join(finding.path))
            .is_ok_and(|source| source.contains(evidence)),
    }
}

pub(crate) fn digest(findings: &[Finding<'_>]) -> String {
    let mut hasher = blake3::Hasher::new();
    for finding in findings {
        hasher.update(finding.rule.as_bytes());
        hasher.update(&[0]);
        hasher.update(finding.path.as_bytes());
        hasher.update(&[0]);
        hasher.update(format!("{}:{}:{}", finding.line, finding.column, finding.occurrences).as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

pub(crate) fn precision(
    catalog: &[CatalogRule],
    observations: &[Observation],
    adjudication: &Adjudication,
) -> Vec<RulePrecision> {
    precision_of(catalog, observations, adjudication, Population::Healthy)
}

pub(crate) fn precision_of(
    catalog: &[CatalogRule],
    observations: &[Observation],
    adjudication: &Adjudication,
    population: Population,
) -> Vec<RulePrecision> {
    let mut observed: BTreeMap<(&str, &str), u64> = BTreeMap::new();
    for observation in observations {
        for rule in &observation.rules {
            *observed
                .entry((observation.name.as_str(), rule.id.as_str()))
                .or_insert(0) += rule.distinct;
        }
    }

    let reviewed: Vec<&ReviewedSite> = adjudication
        .reviewed
        .iter()
        .filter(|site| site.population == population)
        .collect();

    let mut sites: BTreeMap<&str, Vec<&ReviewedSite>> = BTreeMap::new();
    let mut duplicated: BTreeSet<&str> = BTreeSet::new();
    let mut seen: BTreeSet<(&str, &str, &str, u64)> = BTreeSet::new();
    for site in reviewed.iter().copied() {
        let identity = (
            site.rule.as_str(),
            site.repository.as_str(),
            site.path.as_str(),
            site.line,
        );
        if !seen.insert(identity) {
            duplicated.insert(site.rule.as_str());
        }
        sites.entry(site.rule.as_str()).or_default().push(site);
    }
    let stale: BTreeSet<&str> = reviewed
        .iter()
        .filter(|site| !observed.contains_key(&(site.repository.as_str(), site.rule.as_str())))
        .map(|site| site.rule.as_str())
        .collect();

    let backing = agreement::backing(&adjudication.agreement);

    catalog
        .iter()
        .map(|rule| {
            let id = rule.id.as_str();
            let findings: u64 = observed
                .iter()
                .filter(|((_, observed_rule), _)| *observed_rule == id)
                .map(|(_, count)| *count)
                .sum();
            if findings == 0 {
                return RulePrecision {
                    doubly_judged: 0,
                    false_positive_rate_basis_points: None,
                    false_positives: None,
                    findings: 0,
                    id: id.to_owned(),
                    interval_high_basis_points: None,
                    interval_low_basis_points: None,
                    judges: Vec::new(),
                    provenance: Vec::new(),
                    reviewed: 0,
                    separation: None,
                    status: PrecisionStatus::Unobserved,
                    tier: rule.tier.clone(),
                    true_positives: None,
                };
            }

            let reviewed = sites.get(id).map(Vec::as_slice).unwrap_or_default();
            let count = reviewed.len() as u64;
            let publishable = count >= MINIMUM_REVIEWED_SITES.min(findings)
                && count <= findings
                && !duplicated.contains(id)
                && !stale.contains(id);
            if !publishable {
                return RulePrecision {
                    doubly_judged: 0,
                    false_positive_rate_basis_points: None,
                    false_positives: None,
                    findings,
                    id: id.to_owned(),
                    interval_high_basis_points: None,
                    interval_low_basis_points: None,
                    judges: Vec::new(),
                    provenance: Vec::new(),
                    reviewed: count,
                    separation: None,
                    status: PrecisionStatus::Incomplete,
                    tier: rule.tier.clone(),
                    true_positives: None,
                };
            }

            let false_positives = reviewed
                .iter()
                .filter(|site| site.verdict == Verdict::FalsePositive)
                .count() as u64;
            let provenance: BTreeSet<Provenance> =
                reviewed.iter().map(|site| site.provenance).collect();
            let mut doubly_judged = 0u64;
            let mut judges: BTreeSet<&str> = BTreeSet::new();
            for site in reviewed {
                let identity = (
                    site.rule.as_str(),
                    population,
                    site.repository.as_str(),
                    site.path.as_str(),
                    site.line,
                );
                if let Some(pair) = backing.get(&identity) {
                    doubly_judged += 1;
                    for pass in &pair.passes {
                        judges.insert(pass.judge.as_str());
                    }
                }
            }
            let (low, high) = interval::wilson_95(false_positives, count);
            RulePrecision {
                doubly_judged,
                false_positive_rate_basis_points: Some(interval::rate_basis_points(
                    false_positives,
                    count,
                )),
                false_positives: Some(false_positives),
                findings,
                id: id.to_owned(),
                interval_high_basis_points: Some(high),
                interval_low_basis_points: Some(low),
                judges: judges.into_iter().map(str::to_owned).collect(),
                provenance: provenance.into_iter().collect(),
                reviewed: count,
                separation: Some(interval::separation(low, high, THRESHOLD_BASIS_POINTS)),
                status: PrecisionStatus::Measured,
                tier: rule.tier.clone(),
                true_positives: Some(count - false_positives),
            }
        })
        .collect()
}

pub(crate) fn gate(
    catalog: &[CatalogRule],
    precision: &[RulePrecision],
    threshold_basis_points: u64,
) -> GateOutcome {
    let measured: BTreeMap<&str, &RulePrecision> =
        precision.iter().map(|rule| (rule.id.as_str(), rule)).collect();
    let mut admitted = Vec::new();
    let mut indecisive = Vec::new();
    let mut noisy_on_healthy_code = Vec::new();
    let mut refused = Vec::new();
    let mut unproven = Vec::new();

    for rule in catalog.iter().filter(|rule| rule.default_level != "off") {
        let id = rule.id.as_str();
        let measure = measured.get(id).copied();
        let observed = measure.filter(|measure| measure.status == PrecisionStatus::Measured);
        let Some(measure) = observed else {
            unproven.push(id.to_owned());
            admitted.push(id.to_owned());
            continue;
        };

        let false_positives = measure.false_positives.unwrap_or_default();
        let rate = measure.false_positive_rate_basis_points.unwrap_or_default();
        if rule.tier == "P0" && false_positives > 0 {
            refused.push(GateRefusal {
                id: id.to_owned(),
                reason: RefusalReason::ZeroToleranceTierWithFalsePositive,
            });
            continue;
        }
        if rate > threshold_basis_points {
            noisy_on_healthy_code.push(id.to_owned());
        } else if separation_of(measure, threshold_basis_points) != Some(Separation::Below) {
            indecisive.push(id.to_owned());
        }
        admitted.push(id.to_owned());
    }

    GateOutcome {
        verdict: if refused.is_empty() {
            GateVerdict::Passed
        } else {
            GateVerdict::Failed
        },
        admitted,
        indecisive,
        noisy_on_healthy_code,
        refused,
        threshold_basis_points,
        unproven,
    }
}

fn separation_of(measure: &RulePrecision, threshold_basis_points: u64) -> Option<Separation> {
    let low = measure.interval_low_basis_points?;
    let high = measure.interval_high_basis_points?;
    Some(interval::separation(low, high, threshold_basis_points))
}

pub(crate) fn score_distribution(
    healthy: &[Observation],
    agent: &[Observation],
) -> ScoreDistribution {
    let healthy = population_distribution(healthy);
    let agent = population_distribution(agent);
    let minimum = healthy.minimum.min(agent.minimum);
    let maximum = healthy.maximum.max(agent.maximum);
    ScoreDistribution {
        maximum,
        minimum,
        separation_centi: i64::try_from(healthy.median_centi).unwrap_or_default()
            - i64::try_from(agent.median_centi).unwrap_or_default(),
        spread: maximum.saturating_sub(minimum),
        agent,
        healthy,
    }
}

fn population_distribution(observations: &[Observation]) -> PopulationDistribution {
    let values: Vec<_> = observations
        .iter()
        .filter_map(|observation| {
            observation.score.as_ref().map(|score| ScoreValue {
                label: score.label.clone(),
                name: observation.name.clone(),
                value: score.value,
            })
        })
        .collect();

    let mut bands: BTreeMap<&str, usize> = BTreeMap::new();
    for value in &values {
        *bands.entry(value.label.as_str()).or_insert(0) += 1;
    }
    let distinct_bands = bands.len();
    let maximum = values.iter().map(|value| value.value).max().unwrap_or_default();
    let minimum = values.iter().map(|value| value.value).min().unwrap_or_default();

    PopulationDistribution {
        bands: bands
            .into_iter()
            .map(|(label, repositories)| ScoreBand {
                label: label.to_owned(),
                repositories,
            })
            .collect(),
        ceilings_applied: observations
            .iter()
            .filter(|observation| {
                observation
                    .score
                    .as_ref()
                    .is_some_and(|score| score.applied_ceiling.is_some())
            })
            .count(),
        collapsed_into_one_band: distinct_bands <= 1,
        collapsed_into_one_value: values
            .iter()
            .map(|value| value.value)
            .collect::<BTreeSet<_>>()
            .len()
            <= 1,
        maximum,
        median_centi: median_centi(&values),
        minimum,
        spread: maximum.saturating_sub(minimum),
        values,
    }
}

fn median_centi(values: &[ScoreValue]) -> u64 {
    let mut sorted: Vec<u64> = values.iter().map(|value| value.value).collect();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    match (sorted.len(), sorted.get(middle), middle.checked_sub(1).and_then(|index| sorted.get(index))) {
        (0, _, _) => 0,
        (length, Some(upper), Some(lower)) if length % 2 == 0 => (lower + upper) * 50,
        (_, Some(middle), _) => middle * 100,
        _ => 0,
    }
}

pub(crate) fn catalog_from_report(report: &Value) -> Vec<CatalogRule> {
    report["policy"]["rules"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|rule| CatalogRule {
            default_level: rule["level"].as_str().unwrap_or_default().to_owned(),
            id: rule["id"].as_str().unwrap_or_default().to_owned(),
            tier: rule["tier"].as_str().unwrap_or_default().to_owned(),
        })
        .collect()
}
