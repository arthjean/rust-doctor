#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Corpus épinglé, harness confiné, précision adjudiquée et gate d'admission.
//!
//! Les preuves du harness portent sur un corpus synthétique construit à
//! l'exécution: elles restent déterministes et hors ligne. Les preuves de
//! mesure portent sur l'artefact publié, reproductible depuis le cache local
//! par `RUST_DOCTOR_CORPUS_DIR`.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use support::corpus::{
    ARTIFACTS_DIRECTORY_ENV, Adjudication, AdjudicationException, AdjudicationGroup, CACHE_DIRECTORY_ENV,
    CatalogRule, EXPECTED_REPOSITORIES, GateVerdict, HarnessPaths, Manifest, ManifestEntry, Observation,
    PrecisionStatus, RefusalReason, RepositoryOutcome, RepositoryShape, RuleObservation, THRESHOLD_BASIS_POINTS,
    Verdict, artifact, catalog_from_report, curated_findings, evidence_holds, gate, manifest_defects,
    missing_repositories, precision, run, score_distribution,
};

static NEXT_SCOPE: AtomicUsize = AtomicUsize::new(0);

const SCAN_ARGUMENTS: [&str; 2] = ["--json", "--yes"];

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rust-doctor"))
}

fn scope(label: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/corpus-harness")
        .join(format!(
            "{label}-{}-{}",
            std::process::id(),
            NEXT_SCOPE.fetch_add(1, Ordering::Relaxed)
        ));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(&root).unwrap();
    root
}

fn git(repository: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args([
            "-c",
            "user.name=corpus",
            "-c",
            "user.email=corpus@example.invalid",
        ])
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn commit_repository(cache: &Path, name: &str, files: &[(&str, &str)]) -> String {
    let root = cache.join(name);
    fs::create_dir_all(&root).unwrap();
    for (relative, contents) in files {
        write(&root.join(relative), contents);
    }
    git(&root, &["init", "--initial-branch=main", "--quiet"]);
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "--message=pinned"]);
    let output = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn entry(name: &str, commit: String, shape: RepositoryShape) -> ManifestEntry {
    ManifestEntry {
        commit,
        name: name.to_owned(),
        rationale: "synthetic harness fixture".to_owned(),
        shape,
        tag: "v0".to_owned(),
        url: format!("https://example.invalid/{name}.git"),
    }
}

const LIBRARY_SHAPE: RepositoryShape = RepositoryShape {
    asynchronous: false,
    binary: false,
    library: true,
    proc_macro: false,
    workspace_members: 1,
};

const BINARY_SHAPE: RepositoryShape = RepositoryShape {
    asynchronous: false,
    binary: true,
    library: false,
    proc_macro: false,
    workspace_members: 1,
};

/// Corpus synthétique: une bibliothèque qui déclenche une règle, un binaire
/// dont le script de construction laisse une trace, un dépôt dont le manifeste
/// est illisible et un dépôt sans manifeste Cargo.
fn synthetic_corpus(label: &str) -> (PathBuf, PathBuf, Manifest) {
    let root = scope(label);
    let cache = root.join("cache");
    let artifacts = root.join("artifacts");
    fs::create_dir_all(&cache).unwrap();
    fs::create_dir_all(&artifacts).unwrap();

    let alpha = commit_repository(
        &cache,
        "alpha-lib",
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"alpha-lib\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            ),
            (
                "src/lib.rs",
                "pub fn value(input: Option<u8>) -> u8 {\n    input.unwrap()\n}\n",
            ),
        ],
    );
    let beta = commit_repository(
        &cache,
        "beta-bin",
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"beta-bin\"\nversion = \"0.1.0\"\nedition = \"2021\"\nbuild = \"build.rs\"\n",
            ),
            (
                "build.rs",
                "fn main() {\n    std::fs::write(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/build-code-ran\"), b\"1\").unwrap();\n}\n",
            ),
            ("src/main.rs", "fn main() {\n    println!(\"beta\");\n}\n"),
        ],
    );
    let gamma = commit_repository(&cache, "gamma-broken", &[("Cargo.toml", "[package\nname = \"gamma\"\n")]);
    let delta = commit_repository(&cache, "delta-empty", &[("README.md", "no cargo manifest here\n")]);

    let manifest = Manifest {
        repositories: vec![
            entry("alpha-lib", alpha, LIBRARY_SHAPE),
            entry("beta-bin", beta, BINARY_SHAPE),
            entry("gamma-broken", gamma, LIBRARY_SHAPE),
            entry("delta-empty", delta, LIBRARY_SHAPE),
        ],
    };
    (cache, artifacts, manifest)
}

fn paths<'a>(cache: &'a Path, artifacts: &'a Path, binary: &'a Path) -> HarnessPaths<'a> {
    HarnessPaths {
        artifacts,
        binary,
        cache,
    }
}

fn tree_state(root: &Path) -> BTreeMap<String, String> {
    fn visit(root: &Path, directory: &Path, states: &mut BTreeMap<String, String>) {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, states);
            } else {
                states.insert(
                    path.strip_prefix(root).unwrap().to_string_lossy().into_owned(),
                    blake3::hash(&fs::read(&path).unwrap_or_default()).to_hex().to_string(),
                );
            }
        }
    }
    let mut states = BTreeMap::new();
    visit(root, root, &mut states);
    states
}

fn report_of(artifacts: &Path, name: &str) -> Value {
    let bytes = fs::read(artifacts.join("reports").join(format!("{name}.json"))).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// ---------------------------------------------------------------------------
// US-077: manifeste épinglé par révision
// ---------------------------------------------------------------------------

#[test]
fn the_manifest_pins_ten_repositories_by_immutable_revision_with_a_rationale() {
    let artifact = artifact();
    assert_eq!(artifact.manifest.repositories.len(), EXPECTED_REPOSITORIES);
    assert_eq!(manifest_defects(&artifact.manifest), Vec::<String>::new());
    for repository in &artifact.manifest.repositories {
        assert_eq!(repository.commit.len(), 40, "{}", repository.name);
        assert!(
            repository.commit.chars().all(|value| value.is_ascii_hexdigit()),
            "{}",
            repository.name
        );
        assert!(repository.rationale.len() > 20, "{}", repository.name);
        assert!(repository.url.starts_with("https://github.com/"), "{}", repository.name);
    }
}

#[test]
fn the_manifest_covers_a_binary_a_library_a_multi_member_workspace_and_an_asynchronous_project() {
    let manifest = artifact().manifest;
    let shapes: Vec<_> = manifest.repositories.iter().map(|entry| entry.shape).collect();
    assert!(shapes.iter().any(|shape| shape.binary));
    assert!(shapes.iter().any(|shape| shape.library));
    assert!(shapes.iter().any(|shape| shape.workspace_members >= 2));
    assert!(shapes.iter().any(|shape| shape.asynchronous));
    assert!(shapes.iter().any(|shape| shape.proc_macro));
}

#[test]
fn a_truncated_or_mutable_revision_is_refused_and_named_without_leaking_a_path() {
    let mut manifest = artifact().manifest;
    manifest.repositories[0].commit = manifest.repositories[0].commit[..12].to_owned();
    manifest.repositories[1].commit = "refs/tags/1.0.104".to_owned();
    manifest.repositories[2].commit = manifest.repositories[2].commit.to_uppercase();
    manifest.repositories[3].rationale = String::new();

    let defects = manifest_defects(&manifest);
    for (index, expected) in [
        (0, "revision-not-immutable"),
        (1, "revision-not-immutable"),
        (2, "revision-not-immutable"),
        (3, "rationale-missing"),
    ] {
        let name = &manifest.repositories[index].name;
        assert!(
            defects.iter().any(|defect| defect == &format!("{name}: {expected}")),
            "{defects:?}"
        );
    }
    assert!(defects.iter().all(|defect| !defect.contains('/') && !defect.contains('\u{1b}')));
}

#[test]
fn no_corpus_repository_is_committed_in_this_repository() {
    let manifest = artifact().manifest;
    let names: BTreeSet<&str> = manifest.repositories.iter().map(|entry| entry.name.as_str()).collect();
    let output = Command::new("git")
        .arg("-C")
        .arg(env!("CARGO_MANIFEST_DIR"))
        .args(["ls-files"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let tracked = String::from_utf8(output.stdout).unwrap();
    for path in tracked.lines() {
        let components: Vec<&str> = path.split('/').collect();
        for component in &components[..components.len() - 1] {
            assert!(!names.contains(component), "corpus code committed at {path}");
        }
    }
}

#[test]
fn a_missing_corpus_repository_stops_the_run_before_any_scan() {
    let (cache, artifacts, mut manifest) = synthetic_corpus("missing");
    manifest
        .repositories
        .push(entry("absent-crate", "0".repeat(40), LIBRARY_SHAPE));

    assert_eq!(missing_repositories(&cache, &manifest), vec!["absent-crate".to_owned()]);
    let binary = binary();
    let error = run(&paths(&cache, &artifacts, &binary), &manifest, &SCAN_ARGUMENTS).unwrap_err();
    assert_eq!(error, vec!["absent-crate".to_owned()]);
    assert!(!artifacts.join("reports").exists());
}

// ---------------------------------------------------------------------------
// US-078: harness reproductible et confiné
// ---------------------------------------------------------------------------

#[test]
fn two_runs_on_the_same_corpus_produce_identical_observations() {
    let (cache, artifacts, manifest) = synthetic_corpus("determinism");
    let binary = binary();
    let paths = paths(&cache, &artifacts, &binary);
    let first = run(&paths, &manifest, &SCAN_ARGUMENTS).unwrap();
    let second = run(&paths, &manifest, &SCAN_ARGUMENTS).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first.observations).unwrap(),
        serde_json::to_string(&second.observations).unwrap()
    );
}

#[test]
fn the_harness_writes_only_inside_its_declared_artifacts_directory() {
    let (cache, artifacts, manifest) = synthetic_corpus("confinement");
    let before = tree_state(&cache);
    let binary = binary();
    let run = run(&paths(&cache, &artifacts, &binary), &manifest, &SCAN_ARGUMENTS).unwrap();

    assert_eq!(tree_state(&cache), before, "the corpus cache stays read-only");
    assert!(!run.observations.is_empty());
    for repository in &run.processed {
        assert!(artifacts.join("reports").join(format!("{repository}.json")).is_file());
        assert!(artifacts.join("work").join(repository).is_dir());
    }
    // Le script de construction du corpus n'écrit que dans l'arbre matérialisé,
    // qui vit lui-même sous les artefacts.
    assert!(artifacts.join("work/beta-bin/build-code-ran").is_file());
    assert!(!cache.join("beta-bin/build-code-ran").exists());
}

#[test]
fn native_detectors_never_compile_or_execute_corpus_build_code() {
    let (cache, artifacts, manifest) = synthetic_corpus("native-only");
    let binary = binary();
    let probe = run(&paths(&cache, &artifacts, &binary), &manifest, &SCAN_ARGUMENTS).unwrap();
    let catalog = catalog_from_report(&report_of(&artifacts, "alpha-lib"));
    assert!(probe.processed.contains(&"beta-bin".to_owned()));
    assert!(artifacts.join("work/beta-bin/build-code-ran").is_file());

    let mut arguments: Vec<String> = SCAN_ARGUMENTS.iter().map(|value| (*value).to_owned()).collect();
    for rule in catalog.iter().filter(|rule| rule.id.starts_with("clippy::")) {
        arguments.push("--rule".to_owned());
        arguments.push(format!("{}=off", rule.id));
    }
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let native = run(&paths(&cache, &artifacts, &binary), &manifest, &arguments).unwrap();

    assert!(native.processed.contains(&"beta-bin".to_owned()));
    assert!(
        !artifacts.join("work/beta-bin/build-code-ran").exists(),
        "no corpus build script may run when only native detectors are active"
    );
    let report = report_of(&artifacts, "beta-bin");
    assert_eq!(report["status"], "complete");
    assert!(curated_findings(&report).iter().all(|finding| !finding.rule.starts_with("clippy::")));
}

#[test]
fn a_failing_repository_is_isolated_named_and_leaves_the_others_intact() {
    let (cache, artifacts, manifest) = synthetic_corpus("isolation");
    let binary = binary();
    let run = run(&paths(&cache, &artifacts, &binary), &manifest, &SCAN_ARGUMENTS).unwrap();

    assert_eq!(run.failed, vec!["gamma-broken".to_owned()]);
    assert_eq!(run.processed, vec!["alpha-lib".to_owned(), "beta-bin".to_owned()]);
    let alpha = run
        .observations
        .iter()
        .find(|observation| observation.name == "alpha-lib")
        .unwrap();
    assert_eq!(alpha.outcome, RepositoryOutcome::Processed);
    assert!(alpha.rules.iter().any(|rule| rule.id == "clippy::unwrap_used"));
}

#[test]
fn an_interrupted_run_leaves_no_partial_state_behind() {
    let (cache, artifacts, manifest) = synthetic_corpus("interruption");
    let binary = binary();
    let paths = paths(&cache, &artifacts, &binary);
    let reference = run(&paths, &manifest, &SCAN_ARGUMENTS).unwrap();

    // État partiel d'une exécution interrompue: un rapport périmé, un arbre
    // matérialisé tronqué et un fichier de transit resté en place.
    write(&artifacts.join("reports/alpha-lib.json"), "{\"status\":\"stale\"}");
    write(&artifacts.join("reports/alpha-lib.json.partial"), "truncated");
    write(&artifacts.join("work/beta-bin/src/main.rs"), "fn main() { }\n");
    fs::remove_dir_all(artifacts.join("work/alpha-lib")).unwrap();

    let replayed = run(&paths, &manifest, &SCAN_ARGUMENTS).unwrap();
    assert_eq!(replayed.observations, reference.observations);
    assert_eq!(report_of(&artifacts, "alpha-lib")["status"], "complete");
}

#[test]
fn the_harness_publishes_processed_skipped_and_failed_counts() {
    let (cache, artifacts, manifest) = synthetic_corpus("counters");
    let binary = binary();
    let run = run(&paths(&cache, &artifacts, &binary), &manifest, &SCAN_ARGUMENTS).unwrap();
    let evidence = run.evidence(&SCAN_ARGUMENTS);

    assert_eq!(evidence.processed, 2);
    assert_eq!(evidence.skipped, 1);
    assert_eq!(evidence.failed, 1);
    assert_eq!(run.skipped, vec!["delta-empty".to_owned()]);
    assert_eq!(evidence.cache_directory_env, CACHE_DIRECTORY_ENV);
    assert_eq!(evidence.artifacts_directory_env, ARTIFACTS_DIRECTORY_ENV);
    assert_eq!(
        evidence.processed + evidence.skipped + evidence.failed,
        manifest.repositories.len()
    );
}

// ---------------------------------------------------------------------------
// US-079: précision par règle après adjudication
// ---------------------------------------------------------------------------

#[test]
fn every_corpus_finding_carries_a_verdict_and_a_justification() {
    let artifact = artifact();
    let mut observed: BTreeMap<(&str, &str), u64> = BTreeMap::new();
    for observation in &artifact.observations {
        for rule in &observation.rules {
            observed.insert((observation.name.as_str(), rule.id.as_str()), rule.distinct);
        }
    }
    let adjudicated: BTreeMap<(&str, &str), &AdjudicationGroup> = artifact
        .adjudication
        .groups
        .iter()
        .map(|group| ((group.repository.as_str(), group.rule.as_str()), group))
        .collect();

    assert_eq!(adjudicated.len(), artifact.adjudication.groups.len());
    assert_eq!(observed.keys().collect::<Vec<_>>(), adjudicated.keys().collect::<Vec<_>>());
    for (key, count) in &observed {
        let group = adjudicated[key];
        assert_eq!(group.findings, *count, "{key:?}");
        assert!(!group.justification.trim().is_empty(), "{key:?}");
        assert!(!group.evidence.is_empty(), "{key:?}");
        assert!(group.exceptions.len() as u64 <= group.findings, "{key:?}");
        for exception in &group.exceptions {
            assert!(!exception.justification.trim().is_empty(), "{key:?}");
        }
    }
    assert!(artifact.adjudication.criterion.len() > 80);
    assert!(artifact.adjudication.manual_review.len() > 80);
    assert_eq!(
        artifact
            .adjudication
            .groups
            .iter()
            .map(|group| group.findings)
            .sum::<u64>(),
        artifact
            .observations
            .iter()
            .map(|observation| observation.distinct)
            .sum::<u64>()
    );
}

#[test]
fn the_precision_report_publishes_true_and_false_positives_per_rule() {
    let artifact = artifact();
    let computed = precision(&artifact.catalog, &artifact.observations, &artifact.adjudication);
    assert_eq!(computed, artifact.precision);

    let measured: Vec<_> = computed
        .iter()
        .filter(|rule| rule.status == PrecisionStatus::Measured)
        .collect();
    assert!(!measured.is_empty());
    for rule in measured {
        let true_positives = rule.true_positives.unwrap();
        let false_positives = rule.false_positives.unwrap();
        assert_eq!(true_positives + false_positives, rule.findings, "{}", rule.id);
        assert_eq!(
            rule.false_positive_rate_basis_points.unwrap(),
            false_positives * 10_000 / rule.findings,
            "{}",
            rule.id
        );
    }
}

#[test]
fn a_rule_without_any_corpus_finding_is_listed_as_unobserved() {
    let artifact = artifact();
    let unobserved: Vec<&str> = artifact
        .precision
        .iter()
        .filter(|rule| rule.status == PrecisionStatus::Unobserved)
        .map(|rule| rule.id.as_str())
        .collect();

    assert!(!unobserved.is_empty());
    for rule in artifact.precision.iter().filter(|rule| rule.status == PrecisionStatus::Unobserved) {
        assert_eq!(rule.findings, 0, "{}", rule.id);
        assert_eq!(rule.true_positives, None, "{}", rule.id);
        assert_eq!(rule.false_positives, None, "{}", rule.id);
        assert_eq!(rule.false_positive_rate_basis_points, None, "{}", rule.id);
    }
    let observed: BTreeSet<&str> = artifact
        .observations
        .iter()
        .flat_map(|observation| observation.rules.iter().map(|rule| rule.id.as_str()))
        .collect();
    assert!(unobserved.iter().all(|rule| !observed.contains(rule)));
}

#[test]
fn an_unadjudicated_finding_marks_its_rule_incomplete_and_withholds_its_rate() {
    let artifact = artifact();
    let target = artifact
        .adjudication
        .groups
        .iter()
        .find(|group| group.findings > 1)
        .unwrap()
        .clone();

    let mut adjudication = artifact.adjudication.clone();
    for group in &mut adjudication.groups {
        if group.repository == target.repository && group.rule == target.rule {
            group.findings -= 1;
        }
    }
    let computed = precision(&artifact.catalog, &artifact.observations, &adjudication);
    let rule = computed.iter().find(|rule| rule.id == target.rule).unwrap();
    assert_eq!(rule.status, PrecisionStatus::Incomplete);
    assert_eq!(rule.false_positive_rate_basis_points, None);
    assert_eq!(rule.true_positives, None);
    assert!(rule.findings > 0);

    let mut dropped = artifact.adjudication.clone();
    dropped
        .groups
        .retain(|group| group.repository != target.repository || group.rule != target.rule);
    let computed = precision(&artifact.catalog, &artifact.observations, &dropped);
    assert_eq!(
        computed.iter().find(|rule| rule.id == target.rule).unwrap().status,
        PrecisionStatus::Incomplete
    );

    // Une règle incomplète est refusée à l'activation par défaut, au même titre
    // qu'une règle jamais observée.
    let outcome = gate(&artifact.catalog, &computed, THRESHOLD_BASIS_POINTS);
    assert!(outcome.unproven.contains(&target.rule));
    assert!(
        outcome
            .refused
            .iter()
            .any(|refusal| refusal.id == target.rule && refusal.reason == RefusalReason::PrecisionNotMeasured)
    );
}

#[test]
fn two_computations_of_the_precision_report_are_identical() {
    let artifact = artifact();
    let first = precision(&artifact.catalog, &artifact.observations, &artifact.adjudication);
    let second = precision(&artifact.catalog, &artifact.observations, &artifact.adjudication);
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    assert_eq!(
        serde_json::to_string(&score_distribution(&artifact.observations)).unwrap(),
        serde_json::to_string(&artifact.score_distribution).unwrap()
    );
}

// ---------------------------------------------------------------------------
// US-080: gate d'admission opposable
// ---------------------------------------------------------------------------

fn catalog_of(id: &str, tier: &str) -> Vec<CatalogRule> {
    vec![CatalogRule {
        default_level: "warn".to_owned(),
        id: id.to_owned(),
        tier: tier.to_owned(),
    }]
}

fn observations_of(id: &str, findings: u64) -> Vec<Observation> {
    vec![Observation {
        authoritative: true,
        commit: "0".repeat(40),
        distinct: findings,
        exit_code: 0,
        findings_digest: String::new(),
        name: "probe".to_owned(),
        occurrences: findings,
        outcome: RepositoryOutcome::Processed,
        rules: vec![RuleObservation {
            distinct: findings,
            id: id.to_owned(),
            occurrences: findings,
        }],
        score: None,
        status: "complete".to_owned(),
    }]
}

fn adjudication_of(id: &str, findings: u64, false_positives: u64) -> Adjudication {
    Adjudication {
        criterion: "probe".to_owned(),
        manual_review: "probe".to_owned(),
        groups: vec![AdjudicationGroup {
            evidence: "probe".to_owned(),
            exceptions: (0..false_positives)
                .map(|index| AdjudicationException {
                    justification: "probe".to_owned(),
                    line: index + 1,
                    path: "src/lib.rs".to_owned(),
                    verdict: Verdict::FalsePositive,
                })
                .collect(),
            findings,
            justification: "probe".to_owned(),
            repository: "probe".to_owned(),
            rule: id.to_owned(),
            verdict: Verdict::TruePositive,
        }],
    }
}

#[test]
fn a_rule_above_the_false_positive_threshold_is_refused_and_named() {
    let catalog = catalog_of("clippy::probe", "P2");
    let measured = precision(
        &catalog,
        &observations_of("clippy::probe", 100),
        &adjudication_of("clippy::probe", 100, 6),
    );
    assert_eq!(measured[0].false_positive_rate_basis_points, Some(600));

    let outcome = gate(&catalog, &measured, THRESHOLD_BASIS_POINTS);
    assert_eq!(outcome.verdict, GateVerdict::Failed);
    assert_eq!(outcome.refused[0].id, "clippy::probe");
    assert_eq!(outcome.refused[0].reason, RefusalReason::FalsePositiveRateAboveThreshold);
    assert!(outcome.admitted.is_empty());

    // Exactement au seuil publié, la règle reste admise: le refus vise ce qui
    // dépasse 5 %, pas ce qui l'atteint.
    let at_threshold = precision(
        &catalog,
        &observations_of("clippy::probe", 100),
        &adjudication_of("clippy::probe", 100, 5),
    );
    let outcome = gate(&catalog, &at_threshold, THRESHOLD_BASIS_POINTS);
    assert_eq!(outcome.verdict, GateVerdict::Passed);
    assert_eq!(outcome.admitted, vec!["clippy::probe".to_owned()]);
}

#[test]
fn a_zero_tolerance_rule_with_a_single_false_positive_is_refused() {
    let catalog = catalog_of("rust_doctor::source::probe", "P0");
    let measured = precision(
        &catalog,
        &observations_of("rust_doctor::source::probe", 1_000),
        &adjudication_of("rust_doctor::source::probe", 1_000, 1),
    );
    assert_eq!(measured[0].false_positive_rate_basis_points, Some(10));

    let outcome = gate(&catalog, &measured, THRESHOLD_BASIS_POINTS);
    assert_eq!(outcome.verdict, GateVerdict::Failed);
    assert_eq!(
        outcome.refused,
        vec![support::corpus::GateRefusal {
            id: "rust_doctor::source::probe".to_owned(),
            reason: RefusalReason::ZeroToleranceTierWithFalsePositive,
        }]
    );
}

#[test]
fn a_catalog_whose_rules_all_satisfy_the_threshold_passes_and_publishes_every_rate() {
    let catalog = catalog_of("clippy::probe", "P2");
    let measured = precision(
        &catalog,
        &observations_of("clippy::probe", 40),
        &adjudication_of("clippy::probe", 40, 0),
    );
    let outcome = gate(&catalog, &measured, THRESHOLD_BASIS_POINTS);

    assert_eq!(outcome.verdict, GateVerdict::Passed);
    assert!(outcome.refused.is_empty());
    assert!(outcome.unproven.is_empty());
    assert_eq!(outcome.admitted, vec!["clippy::probe".to_owned()]);
    assert!(
        measured
            .iter()
            .all(|rule| rule.false_positive_rate_basis_points.is_some())
    );
}

#[test]
fn an_unobserved_rule_is_unproven_and_refused_by_default() {
    let catalog = catalog_of("clippy::probe", "P2");
    let measured = precision(&catalog, &[], &adjudication_of("clippy::probe", 0, 0));
    assert_eq!(measured[0].status, PrecisionStatus::Unobserved);

    let outcome = gate(&catalog, &measured, THRESHOLD_BASIS_POINTS);
    assert_eq!(outcome.verdict, GateVerdict::Failed);
    assert_eq!(outcome.unproven, vec!["clippy::probe".to_owned()]);
    assert_eq!(outcome.refused[0].reason, RefusalReason::PrecisionNotMeasured);

    // Une règle désactivée par défaut n'est pas soumise au gate: le gate porte
    // sur l'activation par défaut, pas sur l'existence d'une règle.
    let disabled = vec![CatalogRule {
        default_level: "off".to_owned(),
        id: "clippy::probe".to_owned(),
        tier: "P2".to_owned(),
    }];
    let outcome = gate(&disabled, &measured, THRESHOLD_BASIS_POINTS);
    assert_eq!(outcome.verdict, GateVerdict::Passed);
    assert!(outcome.unproven.is_empty());
}

#[test]
fn the_corpus_score_distribution_is_published_with_its_spread() {
    let artifact = artifact();
    let distribution = score_distribution(&artifact.observations);
    assert_eq!(distribution, artifact.score_distribution);
    assert_eq!(
        distribution.values.len(),
        artifact
            .observations
            .iter()
            .filter(|observation| observation.score.is_some())
            .count()
    );
    assert!(distribution.minimum <= distribution.maximum);
    assert_eq!(distribution.collapsed_into_one_band, distribution.bands.len() <= 1);
    assert_eq!(
        distribution.collapsed_into_one_value,
        distribution
            .values
            .iter()
            .map(|value| value.value)
            .collect::<BTreeSet<_>>()
            .len()
            <= 1
    );
    assert_eq!(
        distribution.bands.iter().map(|band| band.repositories).sum::<usize>(),
        distribution.values.len()
    );
}

#[test]
fn the_published_gate_is_the_gate_recomputed_from_the_shipped_catalog() {
    let artifact = artifact();
    let recomputed = gate(&artifact.catalog, &artifact.precision, THRESHOLD_BASIS_POINTS);
    assert_eq!(recomputed, artifact.gate);
    assert_eq!(artifact.gate.threshold_basis_points, THRESHOLD_BASIS_POINTS);

    let refused: BTreeSet<&str> = artifact.gate.refused.iter().map(|rule| rule.id.as_str()).collect();
    let admitted: BTreeSet<&str> = artifact.gate.admitted.iter().map(String::as_str).collect();
    assert!(refused.is_disjoint(&admitted));
    assert_eq!(
        refused.len() + admitted.len(),
        artifact.catalog.iter().filter(|rule| rule.default_level != "off").count()
    );
}

/// Dette d'admission figée: les règles actives par défaut que le corpus n'a pas
/// pu prouver, parce qu'un dépôt sain ne commet pas le défaut qu'elles visent.
/// Inscription nominative et sens unique: une règle prouvée en sort, aucune n'y
/// entre sans que la validation le dise.
const ADMISSION_DEBT: [&str; 24] = [
    "clippy::arc_with_non_send_sync",
    "clippy::await_holding_lock",
    "clippy::await_holding_refcell_ref",
    "clippy::format_collect",
    "clippy::large_types_passed_by_value",
    "clippy::manual_memcpy",
    "clippy::mut_mutex_lock",
    "clippy::non_send_fields_in_send_ty",
    "clippy::permissions_set_readonly_false",
    "clippy::rc_mutex",
    "clippy::redundant_allocation",
    "clippy::suspicious_command_arg_space",
    "clippy::todo",
    "clippy::unimplemented",
    "clippy::unnecessary_to_owned",
    "clippy::unused_async",
    "clippy::vec_init_then_push",
    "clippy::zombie_processes",
    "rust_doctor::cargo::missing_lockfile",
    "rust_doctor::cargo::path_dependency_outside_workspace",
    "rust_doctor::cargo::unbounded_registry_dependency",
    "rust_doctor::cargo::unpinned_git_dependency",
    "rust_doctor::source::disabled_tls_verification",
    "rust_doctor::source::dynamic_shell_command",
];

/// Le seuil est opposable ici, et nulle part ailleurs: ce test échoue dès
/// qu'une règle rejoint le catalogue par défaut sans précision mesurée, et dès
/// qu'une règle mesurée au-dessus du seuil y reste.
#[test]
fn no_rule_is_active_by_default_outside_the_frozen_admission_debt() {
    let artifact = artifact();
    let outcome = gate(&artifact.catalog, &artifact.precision, THRESHOLD_BASIS_POINTS);
    let debt: BTreeSet<&str> = ADMISSION_DEBT.iter().copied().collect();
    assert_eq!(debt.len(), ADMISSION_DEBT.len(), "the debt names a rule twice");

    // Une règle non prouvée qui n'est pas nommée dans la dette est un
    // élargissement silencieux du catalogue.
    let unnamed: Vec<&str> = outcome
        .unproven
        .iter()
        .map(String::as_str)
        .filter(|rule| !debt.contains(rule))
        .collect();
    assert!(unnamed.is_empty(), "active by default without proof: {unnamed:?}");

    // La dette est à sens unique: une règle prouvée depuis doit en sortir.
    let settled: Vec<&str> = artifact
        .gate
        .admitted
        .iter()
        .map(String::as_str)
        .filter(|rule| debt.contains(rule))
        .collect();
    assert!(settled.is_empty(), "proven, remove from the debt: {settled:?}");

    // Une entrée qui ne correspond plus à aucune règle active par défaut
    // masquerait la disparition de la règle qu'elle couvrait.
    let active: BTreeSet<&str> = artifact
        .catalog
        .iter()
        .filter(|rule| rule.default_level != "off")
        .map(|rule| rule.id.as_str())
        .collect();
    let stale: Vec<&str> = debt.difference(&active).copied().collect();
    assert!(stale.is_empty(), "stale debt entry: {stale:?}");

    // La dette couvre l'absence de mesure, jamais une mesure mauvaise: aucune
    // règle active par défaut ne peut dépasser le seuil ni porter un faux
    // positif en tier zéro tolérance.
    for refusal in &outcome.refused {
        assert_eq!(
            refusal.reason,
            RefusalReason::PrecisionNotMeasured,
            "{} is active by default despite its measured precision",
            refusal.id
        );
    }
}

#[test]
fn the_published_catalog_matches_the_shipped_policy() {
    let (cache, artifacts, manifest) = synthetic_corpus("catalog");
    let binary = binary();
    run(&paths(&cache, &artifacts, &binary), &manifest, &SCAN_ARGUMENTS).unwrap();
    let shipped = catalog_from_report(&report_of(&artifacts, "alpha-lib"));
    assert_eq!(shipped, artifact().catalog);
}

// ---------------------------------------------------------------------------
// Reproduction du corpus épinglé, quand le cache local est disponible
// ---------------------------------------------------------------------------

#[test]
fn the_published_observations_reproduce_the_pinned_corpus_run() {
    let (Some(cache), Some(artifacts)) = (
        env::var_os(CACHE_DIRECTORY_ENV),
        env::var_os(ARTIFACTS_DIRECTORY_ENV),
    ) else {
        return;
    };
    let cache = PathBuf::from(cache);
    let artifacts = PathBuf::from(artifacts);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(
        !artifacts.starts_with(repository) && !cache.starts_with(repository),
        "the corpus and its artifacts belong outside this repository"
    );
    let published = artifact();
    let binary = binary();

    let replayed = run(
        &paths(&cache, &artifacts, &binary),
        &published.manifest,
        &SCAN_ARGUMENTS,
    )
    .unwrap();

    if replayed.observations != published.observations {
        fs::write(
            artifacts.join("observations.json"),
            serde_json::to_vec_pretty(&replayed.observations).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(replayed.observations, published.observations);
    assert_eq!(replayed.evidence(&SCAN_ARGUMENTS), published.harness);

    // Chaque verdict est vérifiable: le déclencheur adjudiqué est présent dans
    // le span signalé, à la révision épinglée.
    for group in &published.adjudication.groups {
        let root = artifacts.join("work").join(&group.repository);
        let bytes = fs::read(artifacts.join("reports").join(format!("{}.json", group.repository))).unwrap();
        let report: Value = serde_json::from_slice(&bytes).unwrap();
        let findings: Vec<_> = curated_findings(&report)
            .into_iter()
            .filter(|finding| finding.rule == group.rule)
            .collect();
        assert_eq!(findings.len() as u64, group.findings, "{}", group.rule);
        for finding in &findings {
            assert!(
                evidence_holds(&root, finding, &group.evidence),
                "{}/{} at {}:{}",
                group.repository,
                group.rule,
                finding.path,
                finding.line
            );
        }
    }
}
