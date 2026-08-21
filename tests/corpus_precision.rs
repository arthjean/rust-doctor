#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Pinned corpus, confined harness, adjudicated precision and admission gate.
//!
//! The harness proofs bear on a synthetic corpus built at run time: they stay
//! deterministic and offline. The measurement proofs bear on the published
//! artifact, reproducible from the local cache through
//! `RUST_DOCTOR_CORPUS_DIR`.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use support::corpus::agreement::Agreement;
use support::corpus::{
    ARTIFACTS_DIRECTORY_ENV, Adjudication, CACHE_DIRECTORY_ENV, CatalogRule, CorpusArtifact,
    EXPECTED_REPOSITORIES,
    GateVerdict, HarnessPaths, MINIMUM_REVIEWED_SITES, MINIMUM_SPREAD, Manifest, ManifestEntry, Observation,
    PrecisionStatus, Population, Provenance, RefusalReason, RepositoryOutcome, RepositoryShape, ReviewedSite,
    RuleObservation, RuleTrigger, SiteContext, THRESHOLD_BASIS_POINTS, TriggerVerification, Verdict,
    artifact,
    catalog_from_report, curated_findings, density_milli, evidence_holds, gate, manifest_defects,
    missing_repositories, precision, precision_of, run, rust_line_count, score_distribution, structural_findings,
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

/// Synthetic corpus: a library that triggers a rule, a binary whose build
/// script leaves a trace, a repository whose manifest is unreadable and a
/// repository with no Cargo manifest.
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
// US-077: manifest pinned by revision
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
    let artifact = artifact();
    let manifest = &artifact.manifest;
    let names: BTreeSet<&str> = manifest
        .repositories
        .iter()
        .map(|entry| entry.name.as_str())
        .chain(
            artifact
                .agent_population
                .repositories
                .iter()
                .map(|entry| entry.name.as_str()),
        )
        .collect();
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
// US-078: reproducible and confined harness
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
    // The build script of the corpus only writes into the materialised tree,
    // which itself lives under the artifacts.
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

    // Partial state of an interrupted run: a stale report, a truncated
    // materialised tree and a transit file left in place.
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
// US-079: per-rule precision after adjudication
// ---------------------------------------------------------------------------

/// Every reviewed site carries a verdict, a justification and a context, and
/// designates a finding the corpus actually produced.
#[test]
fn every_reviewed_site_carries_a_verdict_a_justification_and_a_real_finding() {
    let artifact = artifact();
    // Each population is checked against its own observations. A site drawn
    // from one and matched against the other would be free to name a finding
    // that corpus never produced.
    let observed_in = |observations: &[Observation]| {
        let mut observed: BTreeMap<(String, String), u64> = BTreeMap::new();
        for observation in observations {
            for rule in &observation.rules {
                observed.insert(
                    (observation.name.clone(), rule.id.clone()),
                    rule.distinct,
                );
            }
        }
        observed
    };
    let healthy = observed_in(&artifact.observations);
    let agent = observed_in(&artifact.agent_population.observations);

    assert!(!artifact.adjudication.reviewed.is_empty());
    let mut identities: BTreeSet<(&str, &str, &str, u64)> = BTreeSet::new();
    for site in &artifact.adjudication.reviewed {
        assert!(!site.justification.trim().is_empty(), "{site:?}");
        assert!(!site.path.is_empty(), "{site:?}");
        let observed = match site.population {
            Population::Agent => &agent,
            Population::Healthy => &healthy,
        };
        assert!(
            observed.contains_key(&(site.repository.clone(), site.rule.clone())),
            "reviewed site with no matching finding in its population: {site:?}"
        );
        assert!(
            identities.insert((
                site.rule.as_str(),
                site.repository.as_str(),
                site.path.as_str(),
                site.line
            )),
            "site relu deux fois: {site:?}"
        );
    }

    assert!(artifact.adjudication.criterion.len() > 80);
    assert!(artifact.adjudication.provenance.len() > 80);
    assert!(artifact.adjudication.sampling.len() > 80);
    assert!(artifact.adjudication.trigger_verification.method.len() > 80);
    assert!(!artifact.adjudication.trigger_verification.triggers.is_empty());
    for trigger in &artifact.adjudication.trigger_verification.triggers {
        assert!(!trigger.evidence.is_empty(), "{trigger:?}");
    }
}

/// The published rate is that of the reviewed sample. Relating value verdicts
/// to an unreviewed population would publish a precision nobody measured: that
/// is the defect this test locks down.
#[test]
fn the_published_rate_is_the_rate_of_the_reviewed_sample_not_of_the_population() {
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
        assert_eq!(true_positives + false_positives, rule.reviewed, "{}", rule.id);
        assert!(rule.reviewed <= rule.findings, "{}", rule.id);
        assert!(
            rule.reviewed >= MINIMUM_REVIEWED_SITES.min(rule.findings),
            "{}",
            rule.id
        );
        assert_eq!(
            rule.false_positive_rate_basis_points.unwrap(),
            false_positives * 10_000 / rule.reviewed,
            "{}",
            rule.id
        );
    }

    // A large population of which only a sample is reviewed cannot publish a
    // rate computed on the population.
    let sampled = computed
        .iter()
        .find(|rule| rule.status == PrecisionStatus::Measured && rule.findings > rule.reviewed);
    let sampled = sampled.expect("the corpus contains at least one sampled rule");
    assert_ne!(
        sampled.false_positive_rate_basis_points.unwrap(),
        sampled.false_positives.unwrap() * 10_000 / sampled.findings
    );
}

/// A published rate names who produced the verdicts it rests on.
///
/// The rate is the visible artifact; the hundred and ten sites behind it are
/// not. Carrying the provenance up to the rate is what lets a reader tell a
/// number an agent produced from one a person produced, without which the
/// distinction exists in the file and nowhere a reader looks.
#[test]
fn a_published_rate_carries_the_provenance_of_its_verdicts() {
    let artifact = artifact();
    check_provenance(&artifact, Population::Healthy);
    check_provenance(&artifact, Population::Agent);
}

/// The same assertion on one population, sites and rates drawn from that side
/// alone.
///
/// The population selects both, because three rules now carry sites on each:
/// a map built over every reviewed site would hand the healthy rate of
/// `duplicate_function_body` the provenance of the agent verdicts it never
/// rested on, which is the crossing `each_population_publishes_its_own_rate`
/// exists against.
fn check_provenance(artifact: &CorpusArtifact, population: Population) {
    let computed = precision_of(
        &artifact.catalog,
        match population {
            Population::Agent => &artifact.agent_population.observations,
            Population::Healthy => &artifact.observations,
        },
        &artifact.adjudication,
        population,
    );

    let mut declared: BTreeMap<&str, BTreeSet<Provenance>> = BTreeMap::new();
    for site in &artifact.adjudication.reviewed {
        if site.population == population {
            declared.entry(site.rule.as_str()).or_default().insert(site.provenance);
        }
    }

    let mut measured = 0usize;
    for rule in &computed {
        match rule.status {
            PrecisionStatus::Measured => {
                measured += 1;
                assert!(!rule.provenance.is_empty(), "{}", rule.id);
                let expected: Vec<Provenance> = declared
                    .get(rule.id.as_str())
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
                assert_eq!(rule.provenance, expected, "{}", rule.id);
            }
            // No rate, nothing to attribute: a provenance here would credit a
            // measurement that was never published.
            PrecisionStatus::Incomplete | PrecisionStatus::Unobserved => {
                assert!(rule.provenance.is_empty(), "{}", rule.id);
                assert_eq!(rule.false_positive_rate_basis_points, None, "{}", rule.id);
            }
        }
    }
    assert!(measured > 0);
}

/// Two populations, two rates, and no verdict crossing between them.
///
/// The healthy corpus says what a rule costs on code nobody wants disturbed;
/// the agent corpus says what it is worth on the code this tool exists for. A
/// site counted on the wrong side would relate a verdict to a corpus it never
/// described, and the two disagree enough for that to matter: the same rule
/// measures 8709 basis points on one and 7000 on the other.
#[test]
fn each_population_publishes_its_own_rate_from_its_own_sites() {
    let artifact = artifact();

    let healthy = precision_of(
        &artifact.catalog,
        &artifact.observations,
        &artifact.adjudication,
        Population::Healthy,
    );
    let agent = precision_of(
        &artifact.catalog,
        &artifact.agent_population.observations,
        &artifact.adjudication,
        Population::Agent,
    );
    assert_eq!(healthy, artifact.precision);
    assert_eq!(agent, artifact.agent_population.precision);

    // Clippy is switched off on untrusted code, so no Clippy rule can ever
    // carry a rate on the agent side. A measured one would mean the population
    // was scanned with the compiler after all.
    assert!(
        agent
            .iter()
            .filter(|rule| rule.status == PrecisionStatus::Measured)
            .all(|rule| !rule.id.starts_with("clippy::")),
        "a Clippy rule cannot be measured on a population it never ran against"
    );

    // Dropping every agent site leaves the healthy rates untouched, which is
    // what says the two sides are actually separate.
    let mut healthy_only = artifact.adjudication.clone();
    healthy_only
        .reviewed
        .retain(|site| site.population == Population::Healthy);
    assert_eq!(
        precision_of(
            &artifact.catalog,
            &artifact.observations,
            &healthy_only,
            Population::Healthy
        ),
        artifact.precision
    );
}

/// A structural rate is a production rate.
///
/// A family marked as a test, benchmark, example or build-script context is
/// published and counted but does not weigh on the score, and healthy test
/// suites duplicate on purpose, one named case per scenario. Measuring a
/// structural rule over those families publishes the cost of an idiom rather
/// than the cost of the rule, so the sample is drawn from the production
/// subpopulation and this test is what keeps it there.
#[test]
fn every_reviewed_structural_site_is_production_context() {
    let artifact = artifact();
    let structural: Vec<&ReviewedSite> = artifact
        .adjudication
        .reviewed
        .iter()
        .filter(|site| site.rule.starts_with("rust_doctor::structure::"))
        .collect();

    assert!(!structural.is_empty());
    for site in structural {
        assert_eq!(
            site.context,
            SiteContext::Production,
            "{}/{} at {}:{}",
            site.repository,
            site.rule,
            site.path,
            site.line
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
fn an_undersized_sample_marks_its_rule_incomplete_and_withholds_its_rate() {
    let artifact = artifact();
    let target = artifact
        .precision
        .iter()
        .find(|rule| rule.status == PrecisionStatus::Measured && rule.reviewed >= MINIMUM_REVIEWED_SITES)
        .expect("the corpus contains a rule measured on a complete sample")
        .id
        .clone();

    // Removing a single reviewed site takes the sample below the publishable
    // minimum: the rate is withheld rather than extrapolated.
    let mut adjudication = artifact.adjudication.clone();
    let mut removed = false;
    adjudication.reviewed.retain(|site| {
        let drop = site.rule == target && !removed;
        removed |= drop;
        !drop
    });
    let computed = precision(&artifact.catalog, &artifact.observations, &adjudication);
    let rule = computed.iter().find(|rule| rule.id == target).unwrap();
    assert_eq!(rule.status, PrecisionStatus::Incomplete);
    assert_eq!(rule.false_positive_rate_basis_points, None);
    assert_eq!(rule.true_positives, None);
    assert!(rule.findings > 0);

    let mut dropped = artifact.adjudication.clone();
    dropped.reviewed.retain(|site| site.rule != target);
    let computed = precision(&artifact.catalog, &artifact.observations, &dropped);
    assert_eq!(
        computed.iter().find(|rule| rule.id == target).unwrap().status,
        PrecisionStatus::Incomplete
    );

    // An incomplete rule joins the unproven ones, just like a rule never
    // observed: its rate is withheld, its activation stays editorial.
    let outcome = gate(&artifact.catalog, &computed, THRESHOLD_BASIS_POINTS);
    assert!(outcome.unproven.contains(&target));
    assert!(!outcome.noisy_on_healthy_code.contains(&target));
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

    // The interval rides the rate, so it rides this too: the bounds come out of
    // a floating-point computation, and a statistic two runs can disagree on is
    // a record neither of them can reproduce. IEEE-754 mandates correctly
    // rounded arithmetic and square root, which is what makes the equality
    // above hold over the bounds rather than merely over the counts.
    let bounds: Vec<_> = first
        .iter()
        .zip(&second)
        .map(|(first, second)| {
            assert_eq!(first.id, second.id);
            assert_eq!(
                (first.interval_low_basis_points, first.interval_high_basis_points, first.separation),
                (second.interval_low_basis_points, second.interval_high_basis_points, second.separation),
                "{} bounds differ between two computations",
                first.id
            );
            first.interval_low_basis_points
        })
        .collect();
    assert!(
        bounds.iter().any(Option::is_some),
        "no rate carries an interval at all"
    );
    assert_eq!(
        serde_json::to_string(&score_distribution(
            &artifact.observations,
            &artifact.agent_population.observations
        ))
        .unwrap(),
        serde_json::to_string(&artifact.score_distribution).unwrap()
    );
}

// ---------------------------------------------------------------------------
// US-080: enforceable admission gate
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
        production_lines: 0,
        rules: vec![RuleObservation {
            distinct: findings,
            id: id.to_owned(),
            occurrences: findings,
        }],
        score: None,
        status: "complete".to_owned(),
    }]
}

/// Synthetic adjudication: `reviewed` reviewed sites of which
/// `false_positives` are judged false positives. The rate derives from this
/// sample, not from `findings`.
fn adjudication_of(id: &str, reviewed: u64, false_positives: u64) -> Adjudication {
    Adjudication {
        adjudicated_after_cutoff: Vec::new(),
        agreement: Agreement::default(),
        criterion: "probe".to_owned(),
        protocol_cutoff: "2026-08-21".to_owned(),
        provenance: "probe".to_owned(),
        sampling: "probe".to_owned(),
        sampling_plan: Vec::new(),
        trigger_verification: TriggerVerification {
            confirmed: reviewed,
            findings: reviewed,
            method: "probe".to_owned(),
            triggers: vec![RuleTrigger {
                evidence: "probe".to_owned(),
                rule: id.to_owned(),
            }],
        },
        reviewed: (0..reviewed)
            .map(|index| ReviewedSite {
                context: SiteContext::Production,
                justification: "probe".to_owned(),
                line: index + 1,
                path: "src/lib.rs".to_owned(),
                population: Population::Healthy,
                provenance: Provenance::Unrecorded,
                repository: "probe".to_owned(),
                rule: id.to_owned(),
                verdict: if index < false_positives {
                    Verdict::FalsePositive
                } else {
                    Verdict::TruePositive
                },
            })
            .collect(),
    }
}

/// Noise on healthy code names a rule, it does not disqualify it.
///
/// Measuring that a rule reports many non-defects on ten repositories chosen
/// for their health says nothing about what it is worth on code that is not
/// healthy. The published list serves to decide its contribution to the score,
/// not its default level.
#[test]
fn a_noisy_rule_is_named_without_losing_its_default_activation() {
    let catalog = catalog_of("clippy::probe", "P2");
    let measured = precision(
        &catalog,
        &observations_of("clippy::probe", 100),
        &adjudication_of("clippy::probe", 100, 6),
    );
    assert_eq!(measured[0].false_positive_rate_basis_points, Some(600));

    let outcome = gate(&catalog, &measured, THRESHOLD_BASIS_POINTS);
    assert_eq!(outcome.verdict, GateVerdict::Passed);
    assert!(outcome.refused.is_empty());
    assert_eq!(outcome.noisy_on_healthy_code, vec!["clippy::probe".to_owned()]);
    assert_eq!(outcome.admitted, vec!["clippy::probe".to_owned()]);

    // Exactly at the published threshold, the rule is not named noisy: the mark
    // aims at what goes past 5%, not at what reaches it.
    let at_threshold = precision(
        &catalog,
        &observations_of("clippy::probe", 100),
        &adjudication_of("clippy::probe", 100, 5),
    );
    let outcome = gate(&catalog, &at_threshold, THRESHOLD_BASIS_POINTS);
    assert_eq!(outcome.verdict, GateVerdict::Passed);
    assert!(outcome.noisy_on_healthy_code.is_empty());
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
fn an_unobserved_rule_is_named_unproven_without_being_refused() {
    let catalog = catalog_of("clippy::probe", "P2");
    let measured = precision(&catalog, &[], &adjudication_of("clippy::probe", 0, 0));
    assert_eq!(measured[0].status, PrecisionStatus::Unobserved);

    // The silence of a healthy corpus on a rule measures the absence of the
    // defect it aims at, not the rule. It is named, never refused for that.
    let outcome = gate(&catalog, &measured, THRESHOLD_BASIS_POINTS);
    assert_eq!(outcome.verdict, GateVerdict::Passed);
    assert_eq!(outcome.unproven, vec!["clippy::probe".to_owned()]);
    assert_eq!(outcome.admitted, vec!["clippy::probe".to_owned()]);
    assert!(outcome.refused.is_empty());

    // A rule disabled by default is not subject to the gate: the gate bears on
    // default activation, not on the existence of a rule.
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
    let distribution = score_distribution(
        &artifact.observations,
        &artifact.agent_population.observations,
    );
    assert_eq!(distribution, artifact.score_distribution);

    // The spread is the criterion, not the collapse. A boolean saying the
    // measurement did not collapse says nothing about how far the model
    // separates: what the record has to carry is the distance itself, against a
    // floor a future model has to clear rather than a state it happened to be
    // in.
    assert!(
        distribution.spread >= MINIMUM_SPREAD,
        "the model spreads {} points over the two populations, under the {MINIMUM_SPREAD} the corpus requires",
        distribution.spread
    );
    assert_eq!(
        distribution.spread,
        distribution.maximum - distribution.minimum
    );
    assert_eq!(
        distribution.separation_centi,
        i64::try_from(distribution.healthy.median_centi).unwrap()
            - i64::try_from(distribution.agent.median_centi).unwrap()
    );
    assert_eq!(
        distribution.minimum,
        distribution.healthy.minimum.min(distribution.agent.minimum)
    );
    assert_eq!(
        distribution.maximum,
        distribution.healthy.maximum.max(distribution.agent.maximum)
    );

    for (population, observations) in [
        (&distribution.healthy, &artifact.observations),
        (&distribution.agent, &artifact.agent_population.observations),
    ] {
        assert_eq!(
            population.values.len(),
            observations
                .iter()
                .filter(|observation| observation.score.is_some())
                .count()
        );
        assert!(population.minimum <= population.maximum);
        assert_eq!(population.spread, population.maximum - population.minimum);
        assert_eq!(population.collapsed_into_one_band, population.bands.len() <= 1);
        assert!(
            !population.collapsed_into_one_band,
            "every repository of a population falls in one band, which measures nothing"
        );
        assert_eq!(
            population.collapsed_into_one_value,
            population
                .values
                .iter()
                .map(|value| value.value)
                .collect::<BTreeSet<_>>()
                .len()
                <= 1
        );
        assert_eq!(
            population.bands.iter().map(|band| band.repositories).sum::<usize>(),
            population.values.len()
        );
        // The median falls on a whole or a half point, never anywhere else: it
        // is one member of an odd population, or the mean of the two straddling
        // the middle of an even one.
        assert_eq!(population.median_centi % 50, 0);
        assert!(
            population.median_centi >= population.minimum * 100
                && population.median_centi <= population.maximum * 100
        );
    }
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

    // Both lists annotate the admitted set, they do not reduce it.
    for annotated in [&artifact.gate.noisy_on_healthy_code, &artifact.gate.unproven] {
        assert!(annotated.iter().all(|rule| admitted.contains(rule.as_str())));
    }
    let noisy: BTreeSet<&str> = artifact.gate.noisy_on_healthy_code.iter().map(String::as_str).collect();
    let unproven: BTreeSet<&str> = artifact.gate.unproven.iter().map(String::as_str).collect();
    assert!(noisy.is_disjoint(&unproven), "a measured rule is not unproven");
}

/// Frozen admission debt: the rules active by default whose precision the
/// corpus does not publish. Registration is nominative and one-way: a proven
/// rule leaves it, none enters without the validation saying so.
///
/// Almost every name here is `unobserved`: the corpus never triggered it,
/// because a healthy repository does not commit the defect it aims at. The six
/// structural rules that used to sit here as `incomplete` left on 2026-08-09,
/// when their corpus sites were adjudicated and their rates published; only
/// `orphan_module_file` remains, which the ten repositories never triggered.
/// The one exception is `stacked_allow_attribute`, `incomplete` since
/// 2026-08-10: the corpus produced exactly one finding for it and that finding
/// sits in an integration test, outside the production subpopulation a
/// structural rate is drawn from, so there is nothing to adjudicate and the
/// rate stays withheld.
///
/// Three entries were added on 2026-08-04, which the one-way rule normally
/// forbids. They do not cover a rule admitted without evidence: they cover the
/// withdrawal of a measurement that was never valid. `dbg_macro`,
/// `print_stdout` and `useless_vec` had a population only in `tests/` and
/// `examples/`, outside the shipped code, and the return to Cargo's default
/// targets made them silent. Their silence says what the 24 others already say:
/// `fd`, `hexyl` and `ripgrep` all write into a locked `stdout` rather than
/// with `println!`, and none leaves a `dbg!` in what it publishes.
/// Eleven rules entered on 2026-08-10 with EP-001 to EP-004 of the
/// suppression, dependency and hygiene PRD, admitted on trigger fixtures and
/// unmeasured until that PRD's EP-005 replayed the corpus. Nine of them stayed:
/// the ten healthy repositories declare no permissive `[lints]` table, no
/// neutralizing rustflag, no unused or test-only dependency, no tracked secret
/// and no unignored target directory, and none ships full debug symbols
/// unstripped. `crate_level_allow` and `unchecked_release_overflow` left the
/// same day, both observed and adjudicated, both published above the threshold.
const ADMISSION_DEBT: [&str; 38] = [
    "clippy::arc_with_non_send_sync",
    "clippy::await_holding_lock",
    "clippy::await_holding_refcell_ref",
    "clippy::dbg_macro",
    "clippy::format_collect",
    "clippy::large_types_passed_by_value",
    "clippy::manual_memcpy",
    "clippy::mut_mutex_lock",
    "clippy::non_send_fields_in_send_ty",
    "clippy::permissions_set_readonly_false",
    "clippy::print_stdout",
    "clippy::rc_mutex",
    "clippy::redundant_allocation",
    "clippy::suspicious_command_arg_space",
    "clippy::todo",
    "clippy::type_complexity",
    "clippy::unimplemented",
    "clippy::unnecessary_to_owned",
    "clippy::unused_async",
    "clippy::useless_vec",
    "clippy::vec_init_then_push",
    "clippy::zombie_processes",
    "rust_doctor::cargo::missing_lockfile",
    "rust_doctor::cargo::path_dependency_outside_workspace",
    "rust_doctor::cargo::permissive_lint_table",
    "rust_doctor::cargo::permissive_rustflags",
    "rust_doctor::cargo::release_debug_symbols",
    "rust_doctor::cargo::test_only_dependency",
    "rust_doctor::cargo::unbounded_registry_dependency",
    "rust_doctor::cargo::unpinned_git_dependency",
    "rust_doctor::cargo::unused_dependency",
    "rust_doctor::repo::hardcoded_credential",
    "rust_doctor::repo::tracked_secret_file",
    "rust_doctor::repo::unignored_build_output",
    "rust_doctor::source::disabled_tls_verification",
    "rust_doctor::source::dynamic_shell_command",
    "rust_doctor::structure::orphan_module_file",
    "rust_doctor::structure::stacked_allow_attribute",
];

/// The threshold is enforceable here, and nowhere else: this test fails as soon
/// as a rule joins the default catalog without a measured precision, and as
/// soon as a rule measured above the threshold stays in it.
#[test]
fn no_rule_is_active_by_default_outside_the_frozen_admission_debt() {
    let artifact = artifact();
    let outcome = gate(&artifact.catalog, &artifact.precision, THRESHOLD_BASIS_POINTS);
    let debt: BTreeSet<&str> = ADMISSION_DEBT.iter().copied().collect();
    assert_eq!(debt.len(), ADMISSION_DEBT.len(), "the debt names a rule twice");

    // An unproven rule that is not named in the debt is a silent widening of
    // the catalog.
    let unnamed: Vec<&str> = outcome
        .unproven
        .iter()
        .map(String::as_str)
        .filter(|rule| !debt.contains(rule))
        .collect();
    assert!(unnamed.is_empty(), "active by default without proof: {unnamed:?}");

    // The debt is one-way: a rule the corpus ended up observing leaves it. The
    // criterion is the measurement, not the admission: since the gate no longer
    // opposes anything to an unproven rule, every rule active by default is
    // admitted, and reading the admission would empty the whole debt.
    let unproven: BTreeSet<&str> = artifact.gate.unproven.iter().map(String::as_str).collect();
    let settled: Vec<&str> = debt.difference(&unproven).copied().collect();
    assert!(settled.is_empty(), "observed since, remove from the debt: {settled:?}");

    // An entry matching no rule active by default any more would mask the
    // disappearance of the rule it covered.
    let active: BTreeSet<&str> = artifact
        .catalog
        .iter()
        .filter(|rule| rule.default_level != "off")
        .map(|rule| rule.id.as_str())
        .collect();
    let stale: Vec<&str> = debt.difference(&active).copied().collect();
    assert!(stale.is_empty(), "stale debt entry: {stale:?}");

    // The only enforceable refusal is the zero-tolerance tier carrying a false
    // positive: a `P0` caps the whole score, so a false alarm on healthy code
    // costs all of it there.
    let measured: BTreeMap<&str, &support::corpus::RulePrecision> = artifact
        .precision
        .iter()
        .map(|rule| (rule.id.as_str(), rule))
        .collect();
    let refused: Vec<String> = outcome
        .refused
        .iter()
        .map(|refusal| {
            let rule = measured[refusal.id.as_str()];
            format!(
                "{} ({:?}, {} FP sur {} relus, population {})",
                refusal.id,
                refusal.reason,
                rule.false_positives.unwrap_or_default(),
                rule.reviewed,
                rule.findings
            )
        })
        .collect();
    assert!(
        refused.is_empty(),
        "tier zéro tolérance avec faux positif, actif par défaut:\n  {}",
        refused.join("\n  ")
    );

    // The noisy list is published in full: it names exactly the rules measured
    // above the mark, no more and no less. Appearing in it removes nothing;
    // omission would hide what the rule costs on healthy code.
    let expected_noisy: Vec<&str> = artifact
        .precision
        .iter()
        .filter(|rule| {
            rule.status == PrecisionStatus::Measured
                && rule.false_positive_rate_basis_points.unwrap_or_default() > THRESHOLD_BASIS_POINTS
                && active.contains(rule.id.as_str())
        })
        .map(|rule| rule.id.as_str())
        .collect();
    assert_eq!(
        artifact.gate.noisy_on_healthy_code,
        expected_noisy.iter().map(|rule| (*rule).to_owned()).collect::<Vec<_>>()
    );
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
// US-019: agent-generated population
// ---------------------------------------------------------------------------

#[test]
fn the_agent_population_pins_at_least_five_repositories_with_mechanical_evidence() {
    let population = artifact().agent_population;
    assert!(population.repositories.len() >= 5);
    assert!(population.selection_criterion.len() > 80);

    let mut names = BTreeSet::new();
    for repository in &population.repositories {
        let name = repository.name.as_str();
        assert_eq!(repository.commit.len(), 40, "{name}");
        assert!(
            repository
                .commit
                .chars()
                .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase()),
            "{name}"
        );
        assert!(names.insert(name), "{name} pinned twice");
        assert!(
            repository.url.starts_with("https://") && repository.url.ends_with(".git"),
            "{name}"
        );
        assert!(!repository.marker.trim().is_empty(), "{name}");
        assert!(repository.rationale.len() > 20, "{name}");
        // The selection threshold, falsifiable from the pinned history alone:
        // at least half of the reachable commits carry the marker.
        assert!(repository.total_commits > 0, "{name}");
        assert!(repository.marked_commits <= repository.total_commits, "{name}");
        assert!(
            repository.marked_commits * 2 >= repository.total_commits,
            "{name}: {} of {} commits carry the marker",
            repository.marked_commits,
            repository.total_commits
        );
    }
}

/// The agent population executes no untrusted build code: every Clippy rule is
/// turned off, which the confinement proofs show keeps Cargo out entirely, and
/// no observation may carry a Clippy finding.
#[test]
fn the_agent_population_is_scanned_without_executing_untrusted_build_code() {
    let artifact = artifact();
    let population = &artifact.agent_population;

    for rule in artifact.catalog.iter().filter(|rule| rule.id.starts_with("clippy::")) {
        assert!(
            population.scan_arguments.contains(&format!("{}=off", rule.id)),
            "{} stays active on untrusted code",
            rule.id
        );
    }
    for observation in &population.observations {
        assert!(
            observation
                .rules
                .iter()
                .all(|rule| !rule.id.starts_with("clippy::")),
            "{} carries a Clippy finding",
            observation.name
        );
    }
}

/// Both populations publish per-rule findings, and the density ratio is
/// recomputable from the published quantities. A ratio at or below 1.0 is
/// published as a refutation, never omitted.
#[test]
fn the_density_ratio_is_published_for_both_populations_whatever_its_verdict() {
    let artifact = artifact();
    let density = &artifact.agent_population.structural_density;

    assert!(
        artifact
            .agent_population
            .observations
            .iter()
            .any(|observation| !observation.rules.is_empty()),
        "the agent population publishes per-rule findings"
    );
    assert_eq!(density.healthy.findings, structural_findings(&artifact.observations));
    assert_eq!(
        density.agent.findings,
        structural_findings(&artifact.agent_population.observations)
    );
    for side in [&density.healthy, &density.agent] {
        assert!(side.lines > 0);
        assert_eq!(
            side.per_thousand_lines_milli,
            density_milli(side.findings, side.lines)
        );
    }
    assert_eq!(
        density.ratio_milli,
        density
            .agent
            .per_thousand_lines_milli
            .saturating_mul(1_000)
            / density.healthy.per_thousand_lines_milli
    );
    assert_eq!(density.refutes_density_assumption, density.ratio_milli <= 1_000);
}

/// Replays the agent population from the local cache: observations, selection
/// evidence and line counts all reproduce. Runs under its own artifacts
/// subdirectory so the healthy reproduction can run concurrently.
#[test]
fn the_agent_population_reproduces_from_the_local_cache() {
    let (Some(cache), Some(artifacts)) = (
        env::var_os(CACHE_DIRECTORY_ENV),
        env::var_os(ARTIFACTS_DIRECTORY_ENV),
    ) else {
        return;
    };
    let cache = PathBuf::from(cache);
    let artifacts = PathBuf::from(artifacts).join("agent-replay");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(
        !artifacts.starts_with(repository) && !cache.starts_with(repository),
        "the corpus and its artifacts belong outside this repository"
    );
    let published = artifact();
    let population = &published.agent_population;
    let binary = binary();

    let arguments: Vec<&str> = population.scan_arguments.iter().map(String::as_str).collect();
    let replayed = run(&paths(&cache, &artifacts, &binary), &population.manifest(), &arguments).unwrap();
    if replayed.observations != population.observations {
        fs::write(
            artifacts.join("agent-observations.json"),
            serde_json::to_vec_pretty(&replayed.observations).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(replayed.observations, population.observations);

    // Every site drawn from this population names a finding it actually
    // produced, at its published position and in its published context. The
    // healthy corpus has the same check against its own reports; without this
    // one the agent sites would be the only verdicts in the file that nothing
    // can locate.
    for site in &published
        .adjudication
        .reviewed
        .iter()
        .filter(|site| site.population == Population::Agent)
        .collect::<Vec<_>>()
    {
        let bytes = fs::read(artifacts.join("reports").join(format!("{}.json", site.repository)))
            .expect("a report for every repository a reviewed site names");
        let report: Value = serde_json::from_slice(&bytes).unwrap();
        let located: Vec<_> = curated_findings(&report)
            .into_iter()
            .filter(|finding| {
                finding.rule == site.rule && finding.path == site.path && finding.line == site.line
            })
            .collect();
        assert!(
            !located.is_empty(),
            "reviewed site absent from the agent population: {}/{} at {}:{}",
            site.repository,
            site.rule,
            site.path,
            site.line
        );
        assert!(
            located.iter().any(|finding| site.context.matches(finding.context)),
            "reviewed site published as {:?} while the report marked it {:?}: {}/{} at {}:{}",
            site.context,
            located.first().and_then(|finding| finding.context),
            site.repository,
            site.rule,
            site.path,
            site.line
        );
    }

    // The selection evidence recounts from the pinned history alone.
    for entry in &population.repositories {
        let log = Command::new("git")
            .arg("-C")
            .arg(cache.join(&entry.name))
            .args(["log", "--format=%B%x00", &entry.commit])
            .output()
            .unwrap();
        assert!(log.status.success(), "{}", entry.name);
        let messages = String::from_utf8_lossy(&log.stdout);
        let total = messages.split('\0').filter(|message| !message.trim().is_empty()).count() as u64;
        let marked = messages
            .split('\0')
            .filter(|message| message.contains(&entry.marker))
            .count() as u64;
        assert_eq!(total, entry.total_commits, "{}", entry.name);
        assert_eq!(marked, entry.marked_commits, "{}", entry.name);
    }

    // Line counts recount from freshly materialised trees of both populations.
    let mut agent_lines = 0;
    for entry in &population.repositories {
        agent_lines += rust_line_count(&artifacts.join("work").join(&entry.name));
    }
    assert_eq!(agent_lines, population.structural_density.agent.lines);

    let mut healthy_lines = 0;
    for entry in &published.manifest.repositories {
        let work = artifacts.join("healthy-trees").join(&entry.name);
        if work.exists() {
            fs::remove_dir_all(&work).unwrap();
        }
        fs::create_dir_all(&work).unwrap();
        support::corpus::materialise(&cache.join(&entry.name), &entry.commit, &artifacts, &work);
        healthy_lines += rust_line_count(&work);
    }
    assert_eq!(healthy_lines, population.structural_density.healthy.lines);
}

// ---------------------------------------------------------------------------
// Reproduction of the pinned corpus, when the local cache is available
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

    // Mechanical guard rail: the published trigger is present in every reported
    // span, at the pinned revision. This check only proves that no span is
    // corrupted; it says nothing about the value of the site, and that is why
    // precision is derived from the reviewed sites, not from it.
    let triggers: BTreeMap<&str, &str> = published
        .adjudication
        .trigger_verification
        .triggers
        .iter()
        .map(|trigger| (trigger.rule.as_str(), trigger.evidence.as_str()))
        .collect();
    let mut confirmed = 0u64;
    for observation in &published.observations {
        let root = artifacts.join("work").join(&observation.name);
        let bytes = fs::read(artifacts.join("reports").join(format!("{}.json", observation.name))).unwrap();
        let report: Value = serde_json::from_slice(&bytes).unwrap();
        for finding in curated_findings(&report) {
            let Some(evidence) = triggers.get(finding.rule) else {
                continue;
            };
            assert!(
                evidence_holds(&root, &finding, evidence),
                "{}/{} at {}:{}",
                observation.name,
                finding.rule,
                finding.path,
                finding.line
            );
            confirmed += 1;
        }
    }
    assert_eq!(confirmed, published.adjudication.trigger_verification.confirmed);
    assert_eq!(confirmed, published.adjudication.trigger_verification.findings);

    // Every reviewed site matches a finding the corpus actually produced, at
    // its published position and in its published context.
    //
    // The position is what makes a verdict verifiable rather than declarative:
    // a site nobody can locate is a rate nobody can recompute. The context
    // matters just as much and is easier to overlook, because it decides which
    // subpopulation the sample belongs to: the structural rules draw from the
    // production families alone, so a site declared production while the report
    // marked it as test material would move the denominator of a published rate
    // without moving a single finding.
    //
    // Healthy sites only. The two populations are replayed into artifact
    // directories of their own, so the reports this one wrote are the ten of
    // the healthy manifest and nothing else; the agent sites are located by
    // `the_agent_population_reproduces_from_the_local_cache`, against the
    // reports of its own replay. Reading the whole list here looked for an
    // agent repository under the healthy reports and died on a missing file
    // rather than on a verdict nobody can locate.
    for site in published
        .adjudication
        .reviewed
        .iter()
        .filter(|site| site.population == Population::Healthy)
    {
        let bytes = fs::read(artifacts.join("reports").join(format!("{}.json", site.repository))).unwrap();
        let report: Value = serde_json::from_slice(&bytes).unwrap();
        let findings = curated_findings(&report);
        let located: Vec<_> = findings
            .iter()
            .filter(|finding| {
                finding.rule == site.rule && finding.path == site.path && finding.line == site.line
            })
            .collect();
        assert!(
            !located.is_empty(),
            "reviewed site absent from the corpus: {}/{} at {}:{}",
            site.repository,
            site.rule,
            site.path,
            site.line
        );
        assert!(
            located.iter().any(|finding| site.context.matches(finding.context)),
            "reviewed site published as {:?} while the report marked it {:?}: {}/{} at {}:{}",
            site.context,
            located.first().and_then(|finding| finding.context),
            site.repository,
            site.rule,
            site.path,
            site.line
        );
    }
}
