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

use rust_doctor::{RuleTier, catalog};
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
    pub(crate) agent_population: AgentPopulation,
    pub(crate) artifact: String,
    pub(crate) catalog: Vec<CatalogRule>,
    pub(crate) epic: String,
    pub(crate) gate: GateOutcome,
    pub(crate) generated_at: String,
    pub(crate) harness: HarnessEvidence,
    /// λ table the measurement was taken under, one entry per dimension.
    ///
    /// Recorded beside the toolchain and for the same reason: the toolchain
    /// decides which diagnostics were produced and λ decides what they cost, so
    /// a corpus that names neither is a set of numbers nobody can place. The
    /// freeze test in `src/audit/tests/lambda_freeze.rs` compares it against the
    /// shipped table, so moving a λ without regenerating this file fails there
    /// and names the dimension that moved.
    pub(crate) lambdas: Vec<LambdaObservation>,
    pub(crate) manifest: Manifest,
    pub(crate) network_in_automated_tests: bool,
    pub(crate) observations: Vec<Observation>,
    pub(crate) precision: Vec<RulePrecision>,
    pub(crate) schema_version: u64,
    pub(crate) score_distribution: ScoreDistribution,
    pub(crate) toolchain: Toolchain,
    pub(crate) trust_boundary: TrustBoundary,
}

/// Second corpus population: repositories whose commit history documents agent
/// authorship. Their build code is untrusted, so the population is scanned
/// with every Clippy rule off: the native detectors parse source text and
/// compile nothing, which the confinement tests prove.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentPopulation {
    pub(crate) observations: Vec<Observation>,
    /// Per-rule precision on this population, derived from the sites of
    /// `adjudication.reviewed` marked `agent`, exactly as `precision` is derived
    /// from the ones marked `healthy`.
    ///
    /// It is a separate list rather than a second column of `precision` because
    /// the two populations do not observe the same rules: Clippy is switched off
    /// here, so no Clippy rule can ever carry a rate on this side, and a shared
    /// row would have to publish a hole for most of the catalog.
    pub(crate) precision: Vec<RulePrecision>,
    pub(crate) repositories: Vec<AgentRepository>,
    pub(crate) scan_arguments: Vec<String>,
    pub(crate) selection_criterion: String,
    pub(crate) structural_density: DensityComparison,
}

/// An agent-authored repository, pinned like the healthy ones and carrying the
/// mechanical evidence of its selection: an exact marker searched in commit
/// messages, and the counts that make the criterion falsifiable from the
/// repository record alone.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentRepository {
    pub(crate) commit: String,
    /// Exact string searched in the commit messages reachable from the pinned
    /// revision.
    pub(crate) marker: String,
    pub(crate) marked_commits: u64,
    pub(crate) name: String,
    pub(crate) rationale: String,
    pub(crate) total_commits: u64,
    pub(crate) url: String,
}

/// Structural finding density of both populations, in integer arithmetic so
/// two computations produce identical bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DensityComparison {
    pub(crate) agent: StructuralDensity,
    pub(crate) healthy: StructuralDensity,
    /// Agent density over healthy density, in thousandths.
    pub(crate) ratio_milli: u64,
    /// True when the ratio is at or below 1.0. Published as a refutation of
    /// the assumption that agent-written Rust carries more structural
    /// duplication, never omitted.
    pub(crate) refutes_density_assumption: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StructuralDensity {
    /// Distinct structural findings over the population.
    pub(crate) findings: u64,
    /// Lines in the population's Rust source files, measured on the
    /// materialised trees.
    pub(crate) lines: u64,
    /// Findings per thousand lines, in thousandths.
    pub(crate) per_thousand_lines_milli: u64,
}

impl AgentPopulation {
    /// The replay manifest of the population. Shape and tag carry no meaning
    /// for the harness, which only reads names and commits.
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

/// Distinct structural findings across a population's observations.
pub(crate) fn structural_findings(observations: &[Observation]) -> u64 {
    observations
        .iter()
        .flat_map(|observation| observation.rules.iter())
        .filter(|rule| rule.id.starts_with("rust_doctor::structure::"))
        .map(|rule| rule.distinct)
        .sum()
}

/// Findings per thousand lines, in thousandths: `findings * 1_000_000 / lines`.
pub(crate) fn density_milli(findings: u64, lines: u64) -> u64 {
    if lines == 0 {
        return 0;
    }
    findings.saturating_mul(1_000_000) / lines
}

/// Lines in every `.rs` file under `root`, newline-terminated or not.
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

/// One dimension of the λ table the corpus was measured under.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LambdaObservation {
    /// Name the report publishes the dimension under.
    pub(crate) name: String,
    /// λ in millionths, so the record is integer arithmetic like the rest of
    /// this file and two writings of it produce identical bytes.
    pub(crate) lambda_micro: u64,
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
    /// Production lines the scan measured, which is the denominator every
    /// per-kiloline density of `score.dimensions` is read against.
    ///
    /// Recorded on the observation rather than beside the score, because it is
    /// what makes the density reproducible from the record alone: a reader
    /// holding this count, the λ table and the recorded densities recomputes
    /// the published score without the scan. Zero on a repository that was
    /// skipped or that failed, since a scan that never ran measured no source.
    pub(crate) production_lines: u64,
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
    /// Model the score was computed under, published by the binary itself. A
    /// corpus is a measurement of one model, and a record that does not name
    /// which one is a rate nobody can place.
    pub(crate) model: String,
    pub(crate) value: u64,
    pub(crate) worst_tier: Option<String>,
}

/// One dimension of a recorded score, with the density it was computed from.
///
/// The report publishes the dimension score and not the density behind it, so
/// the harness recovers the density from the diagnostics the report carries: one
/// distinct site per production diagnostic, weighed by severity, summed per rule
/// and divided by the denominator the rule's producer chose.
///
/// Recomputing rather than reading is the point. The recovered density is
/// compared against the published score by the λ freeze test, so a density that
/// does not explain the number the binary printed fails the corpus instead of
/// being recorded beside it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DimensionObservation {
    /// Weighted distinct sites over the denominator, in millionths, so two
    /// computations of the record produce identical bytes.
    pub(crate) density_micro: u64,
    /// Name the report publishes the dimension under.
    pub(crate) name: String,
    /// Dimension score the binary published, ceiling included.
    pub(crate) score: u64,
    /// Worst tier among the rules that scored in this dimension, which is what
    /// caps it.
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
    /// What the provenance of a verdict means and what the reader may conclude
    /// from each value. Published beside the rates, because a sample judged by
    /// an agent and one judged by a person are not the same evidence.
    pub(crate) provenance: String,
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
    Benchmark,
    BuildScript,
    Example,
    Production,
    Tests,
}

impl SiteContext {
    /// Does this declared context match what the report published for the
    /// finding? A production diagnostic carries no `context` field at all,
    /// which is what `weighs` reads, so an absent field means production and
    /// nothing else.
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

/// Which population a reviewed site was drawn from.
///
/// The two answer different questions. `Healthy` says what a rule costs on code
/// nobody wants disturbed, which is what decides admission. `Agent` says what it
/// is worth on the code this tool exists for, which is what should decide what
/// the report recommends. A rate that does not say which it is is not a rate.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Population {
    Agent,
    Healthy,
}

/// Who produced a verdict.
///
/// A rate adjudicated by an agent and a rate adjudicated by a person do not
/// carry the same weight, and a measurement that claims methodological honesty
/// has to say which it is. `Unrecorded` names the sites judged before this
/// field existed: it is the truthful value for them, not a default to reuse.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Provenance {
    Agent,
    Human,
    Unrecorded,
}

/// A corpus site actually read back, with its value verdict.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewedSite {
    pub(crate) context: SiteContext,
    pub(crate) justification: String,
    pub(crate) line: u64,
    pub(crate) path: String,
    pub(crate) population: Population,
    pub(crate) provenance: Provenance,
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
    /// Distinct provenances of the verdicts this rate rests on, sorted. Empty
    /// when no rate is published. It rides the rate rather than staying buried
    /// in the site list, because this is where a reader looks: a rate is worth
    /// its weakest verdict, and that has to be visible without cross-checking
    /// a hundred and ten entries by hand.
    pub(crate) provenance: Vec<Provenance>,
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

/// Minimum spread the model must achieve over the two populations, in points.
///
/// A scale that cannot separate ranks nothing, whatever its range, so the
/// record carries the floor rather than a boolean saying the last measurement
/// happened to clear it. The number is the month-1 goal of the core-v3
/// recalibration PRD; raising it is a deliberate recalibration, and a model
/// that falls under it fails
/// `the_corpus_score_distribution_is_published_with_its_spread` naming the
/// spread it measured.
pub(crate) const MINIMUM_SPREAD: u64 = 20;

/// What the score does over the two pinned populations.
///
/// core-v2 published one population and a boolean saying it had collapsed into
/// a single band, which measured nothing beyond the collapse itself. What a
/// scale has to prove is that it separates, so each population publishes its
/// own spread and its own median, and the block publishes the distance between
/// the two medians: a model that cannot tell agent-written code from code
/// nobody wants disturbed ranks nothing, however wide its range.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScoreDistribution {
    pub(crate) agent: PopulationDistribution,
    pub(crate) healthy: PopulationDistribution,
    /// Highest score of the two populations together.
    pub(crate) maximum: u64,
    /// Lowest score of the two populations together.
    pub(crate) minimum: u64,
    /// Healthy median less agent median, in hundredths of a point. Signed,
    /// because a model that scores the agent population higher is a fact the
    /// record has to be able to state rather than a case it cannot represent.
    pub(crate) separation_centi: i64,
    /// Maximum less minimum over the two populations, which is what
    /// `MINIMUM_SPREAD` bounds from below.
    pub(crate) spread: u64,
}

/// The distribution of one population, published for each of the two.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PopulationDistribution {
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
    /// Middle score, in hundredths of a point, since a population of even size
    /// has no middle member and its median falls between two.
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
            // A wall-clock cutoff would make the observations of a large
            // repository depend on machine load; a measurement cannot.
            .env("RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS", "600")
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
pub(crate) fn materialise(repository: &Path, commit: &str, artifacts: &Path, work: &Path) {
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
        production_lines: 0,
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
        rules: Vec::new(),
        score: None,
        status: "failed".to_owned(),
    }
}

/// The five dimensions, in the order the score declares them.
///
/// Restated here rather than read off the report, because the report publishes
/// a map: a dimension dropped from the published object would silently leave
/// the record one row shorter instead of failing.
const DIMENSION_NAMES: [&str; 5] = [
    "security",
    "reliability",
    "maintainability",
    "performance",
    "dependencies",
];

/// Dimension a rule category is scored under.
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

/// What one distinct site weighs, by the severity the report published. An
/// unknown severity weighs nothing and costs the scan its authoritative flag,
/// which the observation records separately.
fn severity_weight(severity: &str) -> Option<u64> {
    match severity {
        "error" => Some(2),
        "warning" | "info" => Some(1),
        _ => None,
    }
}

/// Denominator the producer that raised a rule divides by, read off the rule's
/// own identifier.
///
/// The prefix is the contract: `validate_catalog` refuses any other, so the
/// five producers are exactly these heads, and the two that judge the manifests
/// and the tracked tree rather than the source divide by the workspace. A rule
/// the catalog does not hold has no producer to ask and is counted against the
/// workspace, which is what the binary does with it.
fn divisor(rule: &str, shipped: &BTreeMap<&str, RuleTier>, kilolines: f64) -> f64 {
    let per_kiloline = shipped.contains_key(rule)
        && (rule.starts_with("clippy::")
            || rule.starts_with("rust_doctor::source::")
            || rule.starts_with("rust_doctor::structure::"));
    if per_kiloline { kilolines } else { 1.0 }
}

/// The density behind each published dimension, recovered from the diagnostics.
///
/// This is deliberately an independent recomputation rather than a call into
/// the crate: a density recovered through the same helper the binary used would
/// agree with the published score by construction and prove nothing. What makes
/// it evidence is that the λ freeze test reads these densities back, rescores
/// them through the shipped curve and compares against the score the binary
/// printed, so a recomputation that drifts from the model fails the corpus.
///
/// One distinct site per production diagnostic, weighed by severity, summed per
/// rule in identifier order and divided by the denominator its producer chose:
/// the same additions in the same sequence as `DimensionState`, so the float is
/// the same float.
fn dimension_observations(report: &Value, production_lines: u64) -> Vec<DimensionObservation> {
    let shipped: BTreeMap<&str, RuleTier> =
        catalog().into_iter().map(|rule| (rule.id, rule.tier)).collect();
    let kilolines = (production_lines as f64 / 1000.0).max(2.0);

    let mut scored: BTreeMap<&str, (&'static str, u64, Option<RuleTier>)> = BTreeMap::new();
    for diagnostic in report["diagnostics"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        // A diagnostic marked with a non-production context stays published and
        // counted, and weighs nothing.
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
        let entry = scored
            .entry(rule)
            .or_insert((dimension, 0, shipped.get(rule).copied()));
        entry.1 = entry.1.saturating_add(weight);
    }

    let published = &report["audit"]["score"]["dimensions"];
    DIMENSION_NAMES
        .into_iter()
        .map(|name| {
            let mut density = 0.0;
            let mut worst: Option<RuleTier> = None;
            for (rule, (dimension, numerator, tier)) in &scored {
                if *dimension != name {
                    continue;
                }
                density += *numerator as f64 / divisor(rule, &shipped, kilolines);
                worst = match (worst, *tier) {
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
    /// Non-production mark the report published, absent for production code.
    ///
    /// Last on purpose, against the alphabetical order the rest of this file
    /// keeps: `Ord` is derived, `digest` walks the sorted set, and a field
    /// placed earlier would reorder every published `findings_digest` without
    /// a single finding having changed.
    pub(crate) context: Option<&'a str>,
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
    precision_of(catalog, observations, adjudication, Population::Healthy)
}

/// Per-rule precision on one population.
///
/// The population selects both sides at once: the findings come from that
/// population's observations and the verdicts from the sites drawn there.
/// Crossing them would relate a verdict to a corpus it never described.
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
                    provenance: Vec::new(),
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
                    provenance: Vec::new(),
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
            let provenance: BTreeSet<Provenance> =
                reviewed.iter().map(|site| site.provenance).collect();
            RulePrecision {
                false_positive_rate_basis_points: Some(
                    false_positives.saturating_mul(10_000) / count,
                ),
                false_positives: Some(false_positives),
                findings,
                id: id.to_owned(),
                provenance: provenance.into_iter().collect(),
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
/// The distribution of both populations, and the distance between them.
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

/// Median of a population in hundredths of a point.
///
/// An even population has no middle member, so its median is the mean of the
/// two that straddle the middle and can land on a half point. Hundredths rather
/// than halves because the record derives `Eq`, and a rational median stored as
/// a float is a record two machines can disagree on.
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
