#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::string_slice))]

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use support::corpus::agreement::{Agreement, agreement_defects_under, quote_failures};
use support::corpus::position::{PositionProof, attest, published_sites};
use support::corpus::reproduction;
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

const BASE_ARGUMENTS: [&str; 4] = ["--json", "--yes", "--advisory-db", "/dev/null"];

const SCAN_ARGUMENTS: [&str; 8] = [
    "--json",
    "--yes",
    "--advisory-db",
    "/dev/null",
    "--rule",
    "clippy::indexing_slicing=warn",
    "--rule",
    "clippy::string_slice=warn",
];

#[test]
fn the_corpus_scan_switches_on_every_opt_in_rule() {
    let switched: Vec<String> = SCAN_ARGUMENTS
        .iter()
        .filter_map(|argument| argument.strip_suffix("=warn"))
        .map(str::to_owned)
        .collect();
    let opt_in: Vec<String> = rust_doctor::catalog()
        .iter()
        .filter(|rule| rule.default_level == rust_doctor::RuleLevel::Off)
        .map(|rule| rule.id.to_owned())
        .collect();
    assert_eq!(switched, opt_in);
}

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
        unpinned: false,
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
    assert!(artifacts.join("work/beta-bin/build-code-ran").is_file());
    assert!(!cache.join("beta-bin/build-code-ran").exists());
}

#[cfg(target_os = "linux")]
fn arm_beta(cache: &Path, mut manifest: Manifest) -> Manifest {
    use std::os::unix::fs::PermissionsExt;

    let root = cache.join("beta-bin");
    write(&root.join(".cargo/config.toml"), "[alias]\nclippy = [\"run\", \"--quiet\", \"--\"]\n");
    write(&root.join("rust-toolchain.toml"), "[toolchain]\npath = \"/proc/self/cwd/tc\"\n");
    for tool in ["cargo", "rustc"] {
        let program = root.join("tc/bin").join(tool);
        write(&program, &format!("#!/bin/sh\ntouch \"$PWD/path-{tool}-ran\"\nexit 1\n"));
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    }
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "--message=armed"]);
    let output = Command::new("git").arg("-C").arg(&root).args(["rev-parse", "HEAD"]).output().unwrap();
    let commit = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    for entry in &mut manifest.repositories {
        if entry.name == "beta-bin" {
            entry.commit.clone_from(&commit);
        }
    }
    manifest
}

#[test]
fn native_detectors_never_compile_or_execute_corpus_build_code() {
    let (cache, artifacts, manifest) = synthetic_corpus("native-only");
    let binary = binary();
    let probe = run(&paths(&cache, &artifacts, &binary), &manifest, &BASE_ARGUMENTS).unwrap();
    let catalog = catalog_from_report(&report_of(&artifacts, "alpha-lib"));
    assert!(probe.processed.contains(&"beta-bin".to_owned()));
    assert!(artifacts.join("work/beta-bin/build-code-ran").is_file());

    #[cfg(target_os = "linux")]
    let manifest = arm_beta(&cache, manifest);
    let mut arguments: Vec<String> = BASE_ARGUMENTS.iter().map(|value| (*value).to_owned()).collect();
    for rule in catalog.iter().filter(|rule| rule.id.starts_with("clippy::")) {
        arguments.push("--rule".to_owned());
        arguments.push(format!("{}=off", rule.id));
    }
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let markers = ["build-code-ran", "path-cargo-ran", "path-rustc-ran"];

    #[cfg(target_os = "linux")]
    {
        let unpinned = HarnessPaths {
            unpinned: true,
            ..paths(&cache, &artifacts, &binary)
        };
        let refused = run(&unpinned, &manifest, &arguments).unwrap();
        assert!(refused.failed.contains(&"beta-bin".to_owned()));
        for marker in markers {
            assert!(!artifacts.join("work/beta-bin").join(marker).exists(), "{marker}");
        }
        assert_eq!(
            report_of(&artifacts, "beta-bin")["errors"][0]["message"],
            "the workspace selects its toolchain by path; set RUSTUP_TOOLCHAIN to a toolchain name to scan it without Clippy"
        );
    }

    let native = run(&paths(&cache, &artifacts, &binary), &manifest, &arguments).unwrap();
    assert!(native.processed.contains(&"beta-bin".to_owned()));
    for marker in markers {
        assert!(
            !artifacts.join("work/beta-bin").join(marker).exists(),
            "no program the corpus tree selects may run when only native detectors are active: {marker}"
        );
    }
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

#[test]
fn every_reviewed_site_carries_a_verdict_a_justification_and_a_real_finding() {
    let artifact = artifact();
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

    let sampled = computed
        .iter()
        .find(|rule| rule.status == PrecisionStatus::Measured && rule.findings > rule.reviewed);
    let sampled = sampled.expect("the corpus contains at least one sampled rule");
    assert_ne!(
        sampled.false_positive_rate_basis_points.unwrap(),
        sampled.false_positives.unwrap() * 10_000 / sampled.findings
    );
}

#[test]
fn a_published_rate_carries_the_provenance_of_its_verdicts() {
    let artifact = artifact();
    check_provenance(&artifact, Population::Healthy);
    check_provenance(&artifact, Population::Agent);
}

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
            PrecisionStatus::Incomplete | PrecisionStatus::Unobserved => {
                assert!(rule.provenance.is_empty(), "{}", rule.id);
                assert_eq!(rule.false_positive_rate_basis_points, None, "{}", rule.id);
            }
        }
    }
    assert!(measured > 0);
}

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

    assert!(
        agent
            .iter()
            .filter(|rule| rule.status == PrecisionStatus::Measured)
            .all(|rule| !rule.id.starts_with("clippy::")),
        "a Clippy rule cannot be measured on a population it never ran against"
    );

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

const PRODUCTION_ON_A_TEST_PATH: &[(&str, &str, u64)] = &[];

fn conventional_target(path: &str) -> Option<&'static str> {
    let path = Path::new(path);
    path.parent()
        .into_iter()
        .flat_map(Path::components)
        .find_map(|component| match component {
            Component::Normal(name) if name == "tests" => Some("tests"),
            Component::Normal(name) if name == "benches" => Some("benches"),
            Component::Normal(name) if name == "examples" => Some("examples"),
            _ => None,
        })
        .or_else(|| (path.file_name()? == "tests.rs").then_some("tests"))
}

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
    for site in &structural {
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

    let allowed: BTreeSet<(&str, &str, u64)> = PRODUCTION_ON_A_TEST_PATH.iter().copied().collect();
    let mut charged: Vec<String> = Vec::new();
    for site in &structural {
        let key = (site.repository.as_str(), site.path.as_str(), site.line);
        if allowed.contains(&key) {
            continue;
        }
        if let Some(directory) = conventional_target(&site.path) {
            charged.push(format!(
                "{}/{} at {}:{} is under {directory}/",
                site.repository, site.rule, site.path, site.line
            ));
        }
    }
    assert!(
        charged.is_empty(),
        "{} reviewed structural sites are measured on test material:\n{}",
        charged.len(),
        charged.join("\n")
    );

    let published: BTreeSet<(&str, &str, u64)> = structural
        .iter()
        .map(|site| (site.repository.as_str(), site.path.as_str(), site.line))
        .collect();
    let stale: Vec<&(&str, &str, u64)> = PRODUCTION_ON_A_TEST_PATH
        .iter()
        .filter(|entry| !published.contains(*entry))
        .collect();
    assert!(
        stale.is_empty(),
        "the allow list names sites the record no longer publishes: {stale:?}"
    );
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
        reasons: Vec::new(),
        rules: vec![RuleObservation {
            distinct: findings,
            id: id.to_owned(),
            occurrences: findings,
        }],
        score: None,
        status: "complete".to_owned(),
    }]
}

fn adjudication_of(id: &str, reviewed: u64, false_positives: u64) -> Adjudication {
    Adjudication {
        adjudicated_after_cutoff: Vec::new(),
        agreement: Agreement::default(),
        criterion: "probe".to_owned(),
        evidence_cutoff: "2026-10-14".to_owned(),
        position_proof: PositionProof {
            date: "2026-08-21".to_owned(),
            digest: "probe".to_owned(),
            sites: 0,
            toolchain: "probe".to_owned(),
        },
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
                family: None,
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

    let outcome = gate(&catalog, &measured, THRESHOLD_BASIS_POINTS);
    assert_eq!(outcome.verdict, GateVerdict::Passed);
    assert_eq!(outcome.unproven, vec!["clippy::probe".to_owned()]);
    assert_eq!(outcome.admitted, vec!["clippy::probe".to_owned()]);
    assert!(outcome.refused.is_empty());

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

    assert!(
        distribution.spread >= MINIMUM_SPREAD,
        "the model spreads {} points over the two populations, under the {MINIMUM_SPREAD} the corpus requires",
        distribution.spread
    );
    assert_eq!(
        distribution.spread,
        distribution.maximum - distribution.minimum
    );
    let bands: BTreeSet<&str> = distribution
        .healthy
        .bands
        .iter()
        .chain(&distribution.agent.bands)
        .map(|band| band.label.as_str())
        .collect();
    assert!(
        bands.len() > 1,
        "both populations fall in one band together, which measures nothing"
    );
    assert!(
        distribution.separation_centi > 0,
        "the healthy median does not sit above the agent one"
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

    for annotated in [&artifact.gate.noisy_on_healthy_code, &artifact.gate.unproven] {
        assert!(annotated.iter().all(|rule| admitted.contains(rule.as_str())));
    }
    let noisy: BTreeSet<&str> = artifact.gate.noisy_on_healthy_code.iter().map(String::as_str).collect();
    let unproven: BTreeSet<&str> = artifact.gate.unproven.iter().map(String::as_str).collect();
    assert!(noisy.is_disjoint(&unproven), "a measured rule is not unproven");
}

const ADMISSION_DEBT: [&str; 120] = [
    "clippy::absurd_extreme_comparisons",
    "clippy::almost_swapped",
    "clippy::approx_constant",
    "clippy::arc_with_non_send_sync",
    "clippy::async_yields_async",
    "clippy::await_holding_lock",
    "clippy::await_holding_refcell_ref",
    "clippy::bad_bit_mask",
    "clippy::cast_slice_different_sizes",
    "clippy::char_indices_as_byte_indices",
    "clippy::crosspointer_transmute",
    "clippy::dbg_macro",
    "clippy::deprecated_semver",
    "clippy::derive_ord_xor_partial_ord",
    "clippy::derived_hash_with_manual_eq",
    "clippy::eager_transmute",
    "clippy::enum_clike_unportable_variant",
    "clippy::eq_op",
    "clippy::erasing_op",
    "clippy::format_collect",
    "clippy::if_let_mutex",
    "clippy::ifs_same_cond",
    "clippy::impl_hash_borrow_with_str_and_bytes",
    "clippy::impossible_comparisons",
    "clippy::ineffective_bit_mask",
    "clippy::infinite_iter",
    "clippy::inherent_to_string_shadow_display",
    "clippy::inline_fn_without_body",
    "clippy::inverted_saturating_sub",
    "clippy::invisible_characters",
    "clippy::iter_next_loop",
    "clippy::iter_skip_zero",
    "clippy::iterator_step_by_zero",
    "clippy::large_types_passed_by_value",
    "clippy::lint_groups_priority",
    "clippy::manual_memcpy",
    "clippy::match_str_case_mismatch",
    "clippy::mem_replace_with_uninit",
    "clippy::min_max",
    "clippy::missing_transmute_annotations",
    "clippy::mistyped_literal_suffixes",
    "clippy::modulo_one",
    "clippy::mut_from_ref",
    "clippy::mut_mutex_lock",
    "clippy::never_loop",
    "clippy::non_octal_unix_permissions",
    "clippy::non_send_fields_in_send_ty",
    "clippy::nonsensical_open_options",
    "clippy::not_unsafe_ptr_arg_deref",
    "clippy::option_env_unwrap",
    "clippy::out_of_bounds_indexing",
    "clippy::panicking_overflow_checks",
    "clippy::panicking_unwrap",
    "clippy::permissions_set_readonly_false",
    "clippy::possible_missing_comma",
    "clippy::print_stdout",
    "clippy::rc_mutex",
    "clippy::read_line_without_trim",
    "clippy::recursive_format_impl",
    "clippy::redundant_allocation",
    "clippy::redundant_comparisons",
    "clippy::reversed_empty_ranges",
    "clippy::self_assignment",
    "clippy::size_of_in_element_count",
    "clippy::suspicious_command_arg_space",
    "clippy::suspicious_splitn",
    "clippy::todo",
    "clippy::transmute_null_to_fn",
    "clippy::transmuting_null",
    "clippy::type_complexity",
    "clippy::unimplemented",
    "clippy::uninit_assumed_init",
    "clippy::uninit_vec",
    "clippy::unit_cmp",
    "clippy::unit_hash",
    "clippy::unit_return_expecting_ord",
    "clippy::unnecessary_to_owned",
    "clippy::unsound_collection_transmute",
    "clippy::unused_async",
    "clippy::unused_io_amount",
    "clippy::useless_attribute",
    "clippy::useless_vec",
    "clippy::vec_init_then_push",
    "clippy::vec_resize_to_zero",
    "clippy::while_immutable_condition",
    "clippy::wrong_transmute",
    "clippy::zombie_processes",
    "clippy::zst_offset",
    "rust_doctor::cargo::duplicate_major_versions",
    "rust_doctor::cargo::missing_lockfile",
    "rust_doctor::cargo::path_dependency_outside_workspace",
    "rust_doctor::cargo::permissive_lint_table",
    "rust_doctor::cargo::permissive_rustflags",
    "rust_doctor::cargo::release_debug_symbols",
    "rust_doctor::cargo::test_only_dependency",
    "rust_doctor::cargo::unbounded_registry_dependency",
    "rust_doctor::cargo::unmaintained_dependency",
    "rust_doctor::cargo::unpinned_git_dependency",
    "rust_doctor::cargo::unused_dependency",
    "rust_doctor::cargo::vulnerable_dependency",
    "rust_doctor::repo::hardcoded_credential",
    "rust_doctor::repo::ignored_lockfile",
    "rust_doctor::repo::secret_in_build_step",
    "rust_doctor::repo::tracked_secret_file",
    "rust_doctor::repo::unignored_build_output",
    "rust_doctor::source::block_in_place_on_current_thread",
    "rust_doctor::source::block_on_in_async",
    "rust_doctor::source::blocking_call_in_async",
    "rust_doctor::source::disabled_jwt_validation",
    "rust_doctor::source::disabled_tls_verification",
    "rust_doctor::source::dynamic_shell_command",
    "rust_doctor::source::insecure_auth_cookie",
    "rust_doctor::source::permissive_cors",
    "rust_doctor::source::secret_env_fallback",
    "rust_doctor::source::sql_built_from_format",
    "rust_doctor::source::unchecked_http_status",
    "rust_doctor::structure::crate_level_allow",
    "rust_doctor::structure::orphan_module_file",
    "rust_doctor::structure::stacked_allow_attribute",
    "rust_doctor::structure::unreasoned_allow_attribute",
];

#[test]
fn no_rule_is_active_by_default_outside_the_frozen_admission_debt() {
    let artifact = artifact();
    let outcome = gate(&artifact.catalog, &artifact.precision, THRESHOLD_BASIS_POINTS);
    let debt: BTreeSet<&str> = ADMISSION_DEBT.iter().copied().collect();
    assert_eq!(debt.len(), ADMISSION_DEBT.len(), "the debt names a rule twice");

    let unnamed: Vec<&str> = outcome
        .unproven
        .iter()
        .map(String::as_str)
        .filter(|rule| !debt.contains(rule))
        .collect();
    assert!(unnamed.is_empty(), "active by default without proof: {unnamed:?}");

    let unproven: BTreeSet<&str> = artifact.gate.unproven.iter().map(String::as_str).collect();
    let settled: Vec<&str> = debt.difference(&unproven).copied().collect();
    assert!(settled.is_empty(), "observed since, remove from the debt: {settled:?}");

    let active: BTreeSet<&str> = artifact
        .catalog
        .iter()
        .filter(|rule| rule.default_level != "off")
        .map(|rule| rule.id.as_str())
        .collect();
    let stale: Vec<&str> = debt.difference(&active).copied().collect();
    assert!(stale.is_empty(), "stale debt entry: {stale:?}");

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
    run(&paths(&cache, &artifacts, &binary), &manifest, &BASE_ARGUMENTS).unwrap();
    let shipped = catalog_from_report(&report_of(&artifacts, "alpha-lib"));
    assert_eq!(shipped, artifact().catalog);
}

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

#[test]
fn the_agent_population_reproduces_from_the_local_cache() {
    let Some((cache, root)) = reproduction::requested().directories(Path::new(env!("CARGO_MANIFEST_DIR")))
    else {
        return;
    };
    let artifacts = root.join("agent-replay");
    let published = artifact();
    let population = &published.agent_population;
    let binary = binary();

    let active: Vec<&str> = rust_doctor::catalog()
        .into_iter()
        .map(|rule| rule.id)
        .filter(|id| id.starts_with("clippy::"))
        .filter(|id| !population.scan_arguments.contains(&format!("{id}=off")))
        .collect();
    assert!(active.is_empty(), "refusing to compile untrusted code: {active:?} stay active");
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

    let mut confirmed = 0u64;
    for site in published_sites(&published.adjudication, Population::Agent) {
        let bytes = fs::read(artifacts.join("reports").join(format!("{}.json", site.repository)))
            .expect("a report for every repository a published site names");
        let report: Value = serde_json::from_slice(&bytes).unwrap();
        let located: Vec<_> = curated_findings(&report)
            .into_iter()
            .filter(|finding| {
                finding.rule == site.rule && finding.path == site.path && finding.line == site.line
            })
            .collect();
        assert!(
            !located.is_empty(),
            "published site absent from the agent population: {}/{} at {}:{}",
            site.repository,
            site.rule,
            site.path,
            site.line
        );
        assert!(
            located.iter().any(|finding| site.context.matches(finding.context)),
            "site published as {:?} while the report marked it {:?}: {}/{} at {}:{}",
            site.context,
            located.first().and_then(|finding| finding.context),
            site.repository,
            site.rule,
            site.path,
            site.line
        );
        if let Some(family) = site.family {
            assert!(
                located.iter().any(|finding| finding.id == family),
                "site anchors family {family} where the report locates {:?}: {}/{} at {}:{}",
                located.iter().map(|finding| finding.id).collect::<Vec<&str>>(),
                site.repository,
                site.rule,
                site.path,
                site.line
            );
        }
        confirmed += 1;
    }
    assert_quotes_hold(&published, Population::Agent, &artifacts);
    attest(&root, &published, Population::Agent, confirmed);

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
        support::corpus::materialise(&cache.join(&entry.name), &entry.commit, &work);
        healthy_lines += rust_line_count(&work);
    }
    assert_eq!(healthy_lines, population.structural_density.healthy.lines);
}

#[test]
fn the_published_observations_reproduce_the_pinned_corpus_run() {
    let Some((cache, artifacts)) = reproduction::requested().directories(Path::new(env!("CARGO_MANIFEST_DIR")))
    else {
        return;
    };
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

    let mut confirmed = 0u64;
    for site in published_sites(&published.adjudication, Population::Healthy) {
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
            "published site absent from the corpus: {}/{} at {}:{}",
            site.repository,
            site.rule,
            site.path,
            site.line
        );
        assert!(
            located.iter().any(|finding| site.context.matches(finding.context)),
            "site published as {:?} while the report marked it {:?}: {}/{} at {}:{}",
            site.context,
            located.first().and_then(|finding| finding.context),
            site.repository,
            site.rule,
            site.path,
            site.line
        );
        if let Some(family) = site.family {
            assert!(
                located.iter().any(|finding| finding.id == family),
                "site anchors family {family} where the report locates {:?}: {}/{} at {}:{}",
                located.iter().map(|finding| finding.id).collect::<Vec<&str>>(),
                site.repository,
                site.rule,
                site.path,
                site.line
            );
        }
        confirmed += 1;
    }

    assert_quotes_hold(&published, Population::Healthy, &artifacts);
    attest(&artifacts, &published, Population::Healthy, confirmed);
}

fn assert_quotes_hold(published: &CorpusArtifact, population: Population, artifacts: &Path) {
    let work = artifacts.join("work");
    let pairs = published
        .adjudication
        .agreement
        .pairs
        .iter()
        .filter(|pair| pair.population == population);
    let failures = quote_failures(pairs, |repository, path| {
        let relative = Path::new(path);
        relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
            .then(|| fs::read_to_string(work.join(repository).join(relative)).ok())
            .flatten()
    });
    let demoted = failures.keys().copied().collect();
    let defects: Vec<String> = failures
        .values()
        .cloned()
        .chain(agreement_defects_under(&published.adjudication, &demoted))
        .collect();
    assert!(defects.is_empty(), "{}", defects.join("\n"));
}
