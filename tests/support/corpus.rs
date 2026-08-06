//! Pinned corpus, confined execution harness and precision admission gate.
//!
//! The corpus is never committed: the harness reads a local cache whose path is
//! declared at the call site, materialises every pinned revision under its
//! artifact directory, and writes nowhere else. Precision is an adjudicated
//! measurement, not an impression: every finding carries a verdict, and the
//! gate refuses default activation for a rule whose precision is not proven.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Published threshold, in basis points. The rate is computed in integer
/// arithmetic so that two runs of the report produce identical bytes.
pub(crate) const THRESHOLD_BASIS_POINTS: u64 = 500;

/// Number of repositories the manifest must pin.
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
    /// Findings observed and fully adjudicated: the rate is published.
    Measured,
    /// Adjudication missing or stale: the rate stays withheld.
    Incomplete,
    /// No finding on the corpus: the rule is unproven, not perfect.
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
    /// Report produced and usable.
    Processed,
    /// Revision materialised with no Cargo manifest: nothing to scan.
    Skipped,
    /// Scan with no usable report: the failure stays isolated on this
    /// repository.
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CorpusArtifact {
    pub(crate) adjudication: Adjudication,
    pub(crate) artifact: String,
    pub(crate) catalog: Vec<CatalogRule>,
    pub(crate) epic: String,
    pub(crate) gate: GateOutcome,
    pub(crate) generated_at: String,
    pub(crate) harness: HarnessEvidence,
    pub(crate) manifest: Manifest,
    pub(crate) network_in_automated_tests: bool,
    pub(crate) observations: Vec<Observation>,
    pub(crate) precision: Vec<RulePrecision>,
    pub(crate) schema_version: u64,
    pub(crate) score_distribution: ScoreDistribution,
    pub(crate) toolchain: Toolchain,
    pub(crate) trust_boundary: TrustBoundary,
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
    pub(crate) label: String,
    pub(crate) value: u64,
    pub(crate) worst_tier: Option<String>,
}

/// Adjudication in two deliberately distinct quantities.
///
/// `trigger_verification` is mechanical and covers 100% of the findings: it
/// only proves that the rule's pattern is present where it reported, hence
/// that no span is corrupted. Confirming a pattern says nothing about its
/// value: the lint looking for `.unwrap()` always finds an `.unwrap()`.
///
/// `reviewed` carries the only quantity precision is derived from: sites
/// actually read back, each judged on the question "should this site be
/// changed" and not "is the pattern present". The published rate is that of
/// this sample, never that of the population.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Adjudication {
    pub(crate) criterion: String,
    pub(crate) reviewed: Vec<ReviewedSite>,
    pub(crate) sampling: String,
    pub(crate) trigger_verification: TriggerVerification,
}

/// Mechanical guard rail: the adjudicated pattern is present in the reported
/// span.
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

/// Where the reviewed site lives. The context carries no verdict by itself, but
/// it makes the dominant cause of a high rate visible: a rule aimed at
/// production panics, applied to a test or to a build script, reports there a
/// pattern that is not a defect there.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SiteContext {
    BuildScript,
    Example,
    Production,
    Tests,
}

/// A corpus site actually read back, with its value verdict.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewedSite {
    pub(crate) context: SiteContext,
    pub(crate) justification: String,
    pub(crate) line: u64,
    pub(crate) path: String,
    pub(crate) repository: String,
    pub(crate) rule: String,
    pub(crate) verdict: Verdict,
}

/// Minimum reviewed sample size for a rate to be publishable, except when the
/// whole population is smaller and entirely reviewed.
pub(crate) const MINIMUM_REVIEWED_SITES: u64 = 5;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RulePrecision {
    pub(crate) false_positive_rate_basis_points: Option<u64>,
    pub(crate) false_positives: Option<u64>,
    /// Population observed on the corpus. Never the denominator of the rate.
    pub(crate) findings: u64,
    pub(crate) id: String,
    /// Sites actually read back. This is the denominator of the published rate.
    pub(crate) reviewed: u64,
    pub(crate) status: PrecisionStatus,
    pub(crate) tier: String,
    pub(crate) true_positives: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateOutcome {
    /// Rules active by default that the gate does not refuse. A noisy or
    /// unproven rule appears there: the two following lists annotate it, they
    /// do not disqualify it.
    pub(crate) admitted: Vec<String>,
    /// Rules whose noise rate on healthy code exceeds the published threshold.
    /// Named so that their contribution to the score can be decided, never to
    /// remove their default activation: the corpus measures what they cost on
    /// healthy code, not what they are worth on code that is not.
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScoreDistribution {
    pub(crate) bands: Vec<ScoreBand>,
    pub(crate) ceilings_applied: usize,
    /// Every score carries the same label. This is the exact question of the
    /// criterion: a corpus where every repository falls in the same band proves
    /// nothing about the ability of the score to separate.
    pub(crate) collapsed_into_one_band: bool,
    /// Every score is worth the same, a collapse harsher still than the shared
    /// band.
    pub(crate) collapsed_into_one_value: bool,
    pub(crate) maximum: u64,
    pub(crate) minimum: u64,
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

/// Complete result of a harness run.
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

/// Closed defects of the manifest, each naming the repository concerned.
///
/// A message cites neither a path nor an escape sequence: it carries only the
/// declared name of the repository and the nature of the defect.
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

/// A revision is immutable when it is a complete object identifier. A tag, a
/// branch or an abbreviated prefix stays movable.
fn is_immutable_revision(commit: &str) -> bool {
    commit.len() == 40 && commit.chars().all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
}

/// Manifest repositories missing from the local cache or pinned on another
/// revision. The evaluation does not start while the list is not empty.
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
}

/// Replays the complete catalog on the corpus.
///
/// The run is refused while a repository is missing: a partial corpus would
/// produce a precision measured on an unknown sample. Every repository is
/// materialised from its pinned revision under `artifacts`, never in the cache,
/// and every failure stays isolated on its repository.
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
        materialise(&paths.cache.join(&entry.name), &entry.commit, paths.artifacts, &work);

        if !work.join("Cargo.toml").is_file() {
            run.skipped.push(entry.name.clone());
            run.observations.push(skipped_observation(entry));
            continue;
        }

        let target = fresh_directory(&paths.artifacts.join("target").join(&entry.name));
        let scan = Command::new(paths.binary)
            .arg("inspect")
            .arg(&work)
            .args(scan_arguments)
            .env("CARGO_TARGET_DIR", &target)
            .output();

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

/// Materialises the pinned revision into `work` through a temporary index kept
/// under the artifacts: the cache stays read-only, index included.
fn materialise(repository: &Path, commit: &str, artifacts: &Path, work: &Path) {
    let index = fresh_directory(&artifacts.join("index")).join(
        work.file_name()
            .expect("a materialised repository should carry a name"),
    );
    let read_tree = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["read-tree", commit])
        .env("GIT_INDEX_FILE", &index)
        .output()
        .expect("git should start");
    assert!(read_tree.status.success(), "read-tree should resolve the pinned revision");

    let mut prefix = std::ffi::OsString::from("--prefix=");
    prefix.push(work.as_os_str());
    prefix.push(std::path::MAIN_SEPARATOR_STR);
    let checkout = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["checkout-index", "--all", "--force"])
        .arg(prefix)
        .env("GIT_INDEX_FILE", &index)
        .output()
        .expect("git should start");
    assert!(checkout.status.success(), "checkout-index should materialise the pinned revision");
}

/// A fresh directory on every run: no partial state of an interrupted run can
/// mix into the result.
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

fn write_atomically(path: &Path, bytes: &[u8]) {
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
        rules: Vec::new(),
        score: None,
        status: "failed".to_owned(),
    }
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

    let score = report["audit"]["score"].as_object().map(|score| ScoreObservation {
        applied_ceiling: score["applied_ceiling"].as_u64(),
        label: score["label"].as_str().unwrap_or_default().to_owned(),
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

/// Finding retained for the measurement: a categorized diagnostic, hence
/// carried by a catalog rule.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Finding<'a> {
    pub(crate) column: u64,
    pub(crate) column_end: u64,
    pub(crate) line: u64,
    pub(crate) line_end: u64,
    pub(crate) occurrences: u64,
    pub(crate) path: &'a str,
    pub(crate) rule: &'a str,
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
        })
        .collect();
    findings.sort_unstable();
    findings
}

/// Exact source text covered by the span of a finding, at the materialised
/// revision. This is the piece that makes an adjudication verdict verifiable
/// rather than declarative.
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

/// The adjudicated trigger is present where the rule reported the defect.
///
/// A finding with no span bears on a whole file, a manifest or a lockfile: the
/// evidence is then looked for in the reported file.
pub(crate) fn evidence_holds(root: &Path, finding: &Finding<'_>, evidence: &str) -> bool {
    match span_text(root, finding) {
        Some(text) => text.contains(evidence),
        None => fs::read_to_string(root.join(finding.path))
            .is_ok_and(|source| source.contains(evidence)),
    }
}

/// Canonical fingerprint of the whole finding set of a repository.
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

/// Per-rule precision, derived from the actually reviewed sites alone.
///
/// The denominator of the rate is the size of the reviewed sample, never the
/// observed population: relating value verdicts to a population that was not
/// reviewed would publish a precision nobody measured. A rule whose sample is
/// smaller than `MINIMUM_REVIEWED_SITES`, without its whole population being
/// reviewed, stays incomplete and the gate refuses it.
///
/// A reviewed site matching no observed finding makes the rule incomplete: the
/// sample has drifted from the population it claims to describe.
pub(crate) fn precision(
    catalog: &[CatalogRule],
    observations: &[Observation],
    adjudication: &Adjudication,
) -> Vec<RulePrecision> {
    let mut observed: BTreeMap<(&str, &str), u64> = BTreeMap::new();
    for observation in observations {
        for rule in &observation.rules {
            *observed
                .entry((observation.name.as_str(), rule.id.as_str()))
                .or_insert(0) += rule.distinct;
        }
    }

    let mut sites: BTreeMap<&str, Vec<&ReviewedSite>> = BTreeMap::new();
    let mut duplicated: BTreeSet<&str> = BTreeSet::new();
    let mut seen: BTreeSet<(&str, &str, &str, u64)> = BTreeSet::new();
    for site in &adjudication.reviewed {
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
    let stale: BTreeSet<&str> = adjudication
        .reviewed
        .iter()
        .filter(|site| !observed.contains_key(&(site.repository.as_str(), site.rule.as_str())))
        .map(|site| site.rule.as_str())
        .collect();

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
                    false_positive_rate_basis_points: None,
                    false_positives: None,
                    findings: 0,
                    id: id.to_owned(),
                    reviewed: 0,
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
                    false_positive_rate_basis_points: None,
                    false_positives: None,
                    findings,
                    id: id.to_owned(),
                    reviewed: count,
                    status: PrecisionStatus::Incomplete,
                    tier: rule.tier.clone(),
                    true_positives: None,
                };
            }

            let false_positives = reviewed
                .iter()
                .filter(|site| site.verdict == Verdict::FalsePositive)
                .count() as u64;
            RulePrecision {
                false_positive_rate_basis_points: Some(
                    false_positives.saturating_mul(10_000) / count,
                ),
                false_positives: Some(false_positives),
                findings,
                id: id.to_owned(),
                reviewed: count,
                status: PrecisionStatus::Measured,
                tier: rule.tier.clone(),
                true_positives: Some(count - false_positives),
            }
        })
        .collect()
}

/// Admission gate.
///
/// The only refusal is that of a zero-tolerance tier showing a false positive:
/// a `P0` caps the whole score, so a single false alarm on healthy code costs
/// all of it there. The rest is published, not refused. A high noise rate on
/// healthy code says nothing about the value of a rule on code that is not
/// healthy, and a rule never observed on ten healthy repositories is not
/// imprecise: both are named so that the decision is taken knowingly, never
/// held against default activation.
pub(crate) fn gate(
    catalog: &[CatalogRule],
    precision: &[RulePrecision],
    threshold_basis_points: u64,
) -> GateOutcome {
    let measured: BTreeMap<&str, &RulePrecision> =
        precision.iter().map(|rule| (rule.id.as_str(), rule)).collect();
    let mut admitted = Vec::new();
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
        noisy_on_healthy_code,
        refused,
        threshold_basis_points,
        unproven,
    }
}

/// Distribution of the corpus scores, published to observe whether tier capping
/// crushes every score into a single band.
pub(crate) fn score_distribution(observations: &[Observation]) -> ScoreDistribution {
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

    ScoreDistribution {
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
        maximum: values.iter().map(|value| value.value).max().unwrap_or_default(),
        minimum: values.iter().map(|value| value.value).min().unwrap_or_default(),
        values,
    }
}

/// Catalog published by a report, reduced to the quantities the gate depends
/// on.
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
