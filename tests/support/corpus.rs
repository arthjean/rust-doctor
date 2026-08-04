//! Corpus épinglé, harness d'exécution confiné et gate d'admission par précision.
//!
//! Le corpus n'est jamais commité: le harness lit un cache local dont le chemin
//! est déclaré à l'appel, matérialise chaque révision épinglée sous son
//! répertoire d'artefacts, et n'écrit nulle part ailleurs. La précision est une
//! mesure adjudiquée, pas une impression: chaque finding porte un verdict, et le
//! gate refuse l'activation par défaut d'une règle dont la précision n'est pas
//! prouvée.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Seuil publié, en points de base. Le taux est calculé en arithmétique entière
/// pour que deux exécutions du rapport produisent des octets identiques.
pub(crate) const THRESHOLD_BASIS_POINTS: u64 = 500;

/// Nombre de dépôts que le manifeste doit épingler.
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
    /// Findings observés et intégralement adjudiqués: le taux est publié.
    Measured,
    /// Adjudication absente ou périmée: le taux reste retenu.
    Incomplete,
    /// Aucun finding sur le corpus: la règle n'est pas prouvée, pas parfaite.
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
    FalsePositiveRateAboveThreshold,
    PrecisionNotMeasured,
    ZeroToleranceTierWithFalsePositive,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RepositoryOutcome {
    /// Rapport produit et exploitable.
    Processed,
    /// Révision matérialisée sans manifeste Cargo: rien à scanner.
    Skipped,
    /// Scan sans rapport exploitable: l'échec est isolé sur ce dépôt.
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Adjudication {
    pub(crate) criterion: String,
    pub(crate) groups: Vec<AdjudicationGroup>,
    /// Ce qu'un humain a réellement relu, par-dessus la vérification mécanique
    /// du déclencheur sur la totalité des findings.
    pub(crate) manual_review: String,
}

/// Verdict porté par tous les findings d'une règle sur un dépôt, sauf ceux que
/// `exceptions` adjudique individuellement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdjudicationGroup {
    /// Déclencheur vérifié dans le span signalé, à la révision épinglée.
    pub(crate) evidence: String,
    pub(crate) exceptions: Vec<AdjudicationException>,
    pub(crate) findings: u64,
    pub(crate) justification: String,
    pub(crate) repository: String,
    pub(crate) rule: String,
    pub(crate) verdict: Verdict,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdjudicationException {
    pub(crate) justification: String,
    pub(crate) line: u64,
    pub(crate) path: String,
    pub(crate) verdict: Verdict,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RulePrecision {
    pub(crate) false_positive_rate_basis_points: Option<u64>,
    pub(crate) false_positives: Option<u64>,
    pub(crate) findings: u64,
    pub(crate) id: String,
    pub(crate) status: PrecisionStatus,
    pub(crate) tier: String,
    pub(crate) true_positives: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateOutcome {
    pub(crate) admitted: Vec<String>,
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
    /// Toutes les notes portent le même label. C'est la question exacte du
    /// critère: un corpus dont chaque dépôt tombe dans la même bande ne prouve
    /// rien de la capacité du score à séparer.
    pub(crate) collapsed_into_one_band: bool,
    /// Toutes les notes valent la même chose, un effondrement plus dur encore
    /// que la bande commune.
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

/// Résultat complet d'une exécution du harness.
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tasks/rust-doctor-score-credibility-corpus.json")
}

pub(crate) fn artifact() -> CorpusArtifact {
    let bytes = fs::read(artifact_path()).expect("the corpus artifact should be readable");
    serde_json::from_slice(&bytes).expect("the corpus artifact should match its typed schema")
}

/// Défauts fermés du manifeste, chacun nommant le dépôt concerné.
///
/// Un message ne cite ni chemin ni séquence d'échappement: il ne transporte que
/// le nom déclaré du dépôt et la nature du défaut.
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

/// Une révision est immuable lorsqu'elle est un identifiant d'objet complet.
/// Un tag, une branche ou un préfixe abrégé reste déplaçable.
fn is_immutable_revision(commit: &str) -> bool {
    commit.len() == 40 && commit.chars().all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
}

/// Dépôts du manifeste absents du cache local ou épinglés sur une autre
/// révision. L'évaluation ne démarre pas tant que la liste n'est pas vide.
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

/// Rejoue le catalogue complet sur le corpus.
///
/// L'exécution est refusée tant qu'un dépôt manque: un corpus partiel
/// produirait une précision mesurée sur un échantillon inconnu. Chaque dépôt est
/// matérialisé depuis sa révision épinglée sous `artifacts`, jamais dans le
/// cache, et chaque échec reste isolé sur son dépôt.
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

/// Matérialise la révision épinglée dans `work` par un index temporaire tenu
/// sous les artefacts: le cache reste en lecture seule, index compris.
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

/// Un répertoire neuf à chaque exécution: aucun état partiel d'une exécution
/// interrompue ne peut se mêler au résultat.
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

/// Finding retenu pour la mesure: un diagnostic catégorisé, donc porté par une
/// règle du catalogue.
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

/// Texte source exact couvert par le span d'un finding, à la révision
/// matérialisée. C'est la pièce qui rend un verdict d'adjudication vérifiable
/// plutôt que déclaratif.
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

/// Le déclencheur adjudiqué est présent là où la règle a signalé le défaut.
///
/// Un finding sans span porte sur un fichier entier, un manifeste ou un
/// verrou: la preuve est alors cherchée dans le fichier signalé.
pub(crate) fn evidence_holds(root: &Path, finding: &Finding<'_>, evidence: &str) -> bool {
    match span_text(root, finding) {
        Some(text) => text.contains(evidence),
        None => fs::read_to_string(root.join(finding.path))
            .is_ok_and(|source| source.contains(evidence)),
    }
}

/// Empreinte canonique de l'ensemble des findings d'un dépôt.
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

/// Précision par règle, dérivée des seules grandeurs publiées.
///
/// Une règle dont un seul finding échappe à l'adjudication est incomplète: son
/// taux n'est pas publié, et le gate la refuse comme non prouvée.
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

    let mut groups: BTreeMap<(&str, &str), &AdjudicationGroup> = BTreeMap::new();
    let mut duplicated: BTreeSet<&str> = BTreeSet::new();
    for group in &adjudication.groups {
        let key = (group.repository.as_str(), group.rule.as_str());
        if groups.insert(key, group).is_some() {
            duplicated.insert(group.rule.as_str());
        }
    }
    let stale: BTreeSet<&str> = groups
        .keys()
        .filter(|key| !observed.contains_key(*key))
        .map(|key| key.1)
        .collect();

    catalog
        .iter()
        .map(|rule| {
            let id = rule.id.as_str();
            let pairs: Vec<_> = observed
                .iter()
                .filter(|((_, observed_rule), _)| *observed_rule == id)
                .collect();
            let findings: u64 = pairs.iter().map(|(_, count)| **count).sum();
            if findings == 0 {
                return RulePrecision {
                    false_positive_rate_basis_points: None,
                    false_positives: None,
                    findings: 0,
                    id: id.to_owned(),
                    status: PrecisionStatus::Unobserved,
                    tier: rule.tier.clone(),
                    true_positives: None,
                };
            }

            let mut false_positives = 0;
            let mut complete = !duplicated.contains(id) && !stale.contains(id);
            for (key, count) in pairs {
                match groups.get(key) {
                    Some(group) if group.findings == *count && group.exceptions.len() as u64 <= *count => {
                        let exceptions = group.exceptions.len() as u64;
                        false_positives += group
                            .exceptions
                            .iter()
                            .filter(|exception| exception.verdict == Verdict::FalsePositive)
                            .count() as u64;
                        if group.verdict == Verdict::FalsePositive {
                            false_positives += count - exceptions;
                        }
                    }
                    _ => complete = false,
                }
            }

            if !complete {
                return RulePrecision {
                    false_positive_rate_basis_points: None,
                    false_positives: None,
                    findings,
                    id: id.to_owned(),
                    status: PrecisionStatus::Incomplete,
                    tier: rule.tier.clone(),
                    true_positives: None,
                };
            }

            RulePrecision {
                false_positive_rate_basis_points: Some(false_positives.saturating_mul(10_000) / findings),
                false_positives: Some(false_positives),
                findings,
                id: id.to_owned(),
                status: PrecisionStatus::Measured,
                tier: rule.tier.clone(),
                true_positives: Some(findings - false_positives),
            }
        })
        .collect()
}

/// Gate d'admission: une règle active par défaut doit porter une précision
/// mesurée sous le seuil, et zéro faux positif lorsque son tier est `P0`.
pub(crate) fn gate(
    catalog: &[CatalogRule],
    precision: &[RulePrecision],
    threshold_basis_points: u64,
) -> GateOutcome {
    let measured: BTreeMap<&str, &RulePrecision> =
        precision.iter().map(|rule| (rule.id.as_str(), rule)).collect();
    let mut admitted = Vec::new();
    let mut refused = Vec::new();
    let mut unproven = Vec::new();

    for rule in catalog.iter().filter(|rule| rule.default_level != "off") {
        let id = rule.id.as_str();
        let Some(measure) = measured.get(id) else {
            unproven.push(id.to_owned());
            refused.push(GateRefusal {
                id: id.to_owned(),
                reason: RefusalReason::PrecisionNotMeasured,
            });
            continue;
        };
        match measure.status {
            PrecisionStatus::Unobserved | PrecisionStatus::Incomplete => {
                unproven.push(id.to_owned());
                refused.push(GateRefusal {
                    id: id.to_owned(),
                    reason: RefusalReason::PrecisionNotMeasured,
                });
            }
            PrecisionStatus::Measured => {
                let false_positives = measure.false_positives.unwrap_or_default();
                let rate = measure.false_positive_rate_basis_points.unwrap_or_default();
                if rule.tier == "P0" && false_positives > 0 {
                    refused.push(GateRefusal {
                        id: id.to_owned(),
                        reason: RefusalReason::ZeroToleranceTierWithFalsePositive,
                    });
                } else if rate > threshold_basis_points {
                    refused.push(GateRefusal {
                        id: id.to_owned(),
                        reason: RefusalReason::FalsePositiveRateAboveThreshold,
                    });
                } else {
                    admitted.push(id.to_owned());
                }
            }
        }
    }

    GateOutcome {
        verdict: if refused.is_empty() {
            GateVerdict::Passed
        } else {
            GateVerdict::Failed
        },
        admitted,
        refused,
        threshold_basis_points,
        unproven,
    }
}

/// Distribution des notes du corpus, publiée pour constater si le plafonnement
/// par tier écrase toutes les notes dans une même bande.
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

/// Catalogue publié par un rapport, réduit aux grandeurs dont dépend le gate.
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
