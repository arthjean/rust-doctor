//! Tests for the dependency pack.
//!
//! They live in a file of their own so that every file of the module stays
//! under the thousand lines `oversized_unit` reports at: the pack that judges a
//! manifest has to pass the rule it publishes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use cargo_metadata::semver::VersionReq;
use serde_json::Value;

use super::*;
use super::resolution::{duplicate_major_versions, major_of};
use crate::policy::{PolicyInput, RuleLevel};

/// The pack passes the rule it judges a manifest against. It was one file of
/// 1872 lines with no such test, the largest producer of the crate and the only
/// one near the bound and unguarded.
#[test]
fn the_pack_holds_the_size_bound_it_judges_for() {
    for own in [
        include_str!("../cargo_health.rs"),
        include_str!("resolution.rs"),
        include_str!("tests.rs"),
    ] {
        let lines = own.lines().count();
        assert!(
            lines < crate::structure::FILE_LINES,
            "a file of the dependency pack is {lines} lines long, over the {} it reports",
            crate::structure::FILE_LINES
        );
    }
}

fn inspect(metadata: &Metadata) -> Vec<Candidate> {
    super::inspect(metadata, &PolicyPlan::default()).candidates
}

static NEXT_CARGO_HOME: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cargo-health/protocol")
}

fn protocol_metadata() -> (Metadata, Value) {
    offline_metadata(&fixture(), "protocol")
}

fn offline_metadata(fixture: &Path, label: &str) -> (Metadata, Value) {
    let cargo_home = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!(
            "cargo-health-{label}-home-{}-{}",
            std::process::id(),
            NEXT_CARGO_HOME.fetch_add(1, Ordering::Relaxed)
        ));
    if cargo_home.exists() {
        fs::remove_dir_all(&cargo_home).expect("stale protocol Cargo home should be removable");
    }

    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
            "--manifest-path",
            "Cargo.toml",
        ])
        .current_dir(fixture)
        .env("CARGO_HOME", &cargo_home)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .expect("Cargo metadata should start");

    if cargo_home.exists() {
        fs::remove_dir_all(&cargo_home).expect("protocol Cargo home should be removable");
    }
    assert!(
        output.status.success(),
        "Cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value =
        serde_json::from_slice(&output.stdout).expect("metadata should be valid JSON");
    let metadata =
        serde_json::from_value(json.clone()).expect("cargo_metadata should decode the corpus");
    (metadata, json)
}

fn fixture_hashes(root: &Path) -> BTreeMap<String, blake3::Hash> {
    fn visit(root: &Path, directory: &Path, hashes: &mut BTreeMap<String, blake3::Hash>) {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .expect("fixture directory should be readable")
            .map(|entry| entry.expect("fixture entry should be readable").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "target") {
                    continue;
                }
                visit(root, &path, hashes);
            } else if path.file_name().is_none_or(|name| name != "Cargo.lock") {
                let relative = path
                    .strip_prefix(root)
                    .expect("fixture file should be below its root")
                    .to_string_lossy()
                    .into_owned();
                hashes.insert(
                    relative,
                    blake3::hash(&fs::read(path).expect("fixture file should be readable")),
                );
            }
        }
    }

    let mut hashes = BTreeMap::new();
    visit(root, root, &mut hashes);
    hashes
}

fn dependency<'a>(metadata: &'a Metadata, key: &str) -> &'a Dependency {
    let package = metadata
        .packages
        .first()
        .expect("protocol metadata should contain its member");
    package
        .dependencies
        .iter()
        .find(|dependency| dependency.rename.as_deref().unwrap_or(&dependency.name) == key)
        .expect("protocol dependency should exist")
}

#[test]
fn producer_uses_its_canonical_catalog_entries() {
    let definitions: Vec<_> = PolicyPlan::default()
        .active_rules(crate::policy::Producer::CargoHealth)
        .map(|(definition, _)| definition.id)
        .collect();
    assert_eq!(
        definitions,
        [
            CARGO_DUPLICATE_MAJOR_VERSIONS.id,
            CARGO_MISSING_LOCKFILE.id,
            CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE.id,
            CARGO_PERMISSIVE_LINT_TABLE.id,
            CARGO_PERMISSIVE_RUSTFLAGS.id,
            CARGO_RELEASE_DEBUG_SYMBOLS.id,
            CARGO_TEST_ONLY_DEPENDENCY.id,
            CARGO_UNBOUNDED_REGISTRY.id,
            CARGO_UNCHECKED_RELEASE_OVERFLOW.id,
            CARGO_UNPINNED_GIT.id,
            CARGO_UNUSED_DEPENDENCY.id,
        ]
    );
    assert!(
        definitions
            .iter()
            .filter_map(|id| crate::policy::find(id))
            .filter(|definition| definition.category == "dependencies")
            .count()
            >= 3
    );
}

#[test]
fn semver_oracle_distinguishes_only_the_total_wildcard() {
    let star = VersionReq::parse("*").expect("star should parse");
    assert_eq!(star, VersionReq::STAR);
    assert!(star.comparators.is_empty());

    for requirement in [
        "1.*",
        "1.2.*",
        "1.x",
        "1.X",
        "1",
        "1.2",
        "^1.2.3",
        "=1.2.3",
        ">=1.2,<2",
        "1.2.3-alpha.1",
    ] {
        let requirement =
            VersionReq::parse(requirement).expect("bounded requirement should parse");
        assert_ne!(requirement, VersionReq::STAR);
        assert!(!requirement.comparators.is_empty());
    }
}

#[test]
fn target_protocol_is_offline_complete_and_classifiable() {
    let oracle: Value = serde_json::from_slice(
        &fs::read(fixture().join("oracle.json")).expect("protocol oracle should be readable"),
    )
    .expect("protocol oracle should be valid JSON");
    let cargo_version = Command::new(env!("CARGO"))
        .arg("--version")
        .output()
        .expect("Cargo version should start");
    assert!(cargo_version.status.success());
    assert_eq!(
        String::from_utf8(cargo_version.stdout)
            .expect("Cargo version should be UTF-8")
            .trim(),
        oracle["cargo"]
            .as_str()
            .expect("Cargo oracle should be a string")
    );
    assert_eq!(oracle["cargo_metadata"], "0.23.1");
    assert!(
        include_str!("../../Cargo.lock")
            .contains("name = \"cargo_metadata\"\nversion = \"0.23.1\"")
    );

    let (metadata, json) = protocol_metadata();
    assert_eq!(json["version"], oracle["metadata_format"]);
    assert_eq!(metadata.workspace_members.len(), 1);

    let registry = dependency(&metadata, "registry_alias");
    assert_eq!(registry.req, VersionReq::STAR);
    assert!(registry.req.comparators.is_empty());
    assert!(registry.optional);
    assert_eq!(registry.rename.as_deref(), Some("registry_alias"));
    assert_eq!(registry.kind, cargo_metadata::DependencyKind::Normal);
    assert!(
        registry
            .source
            .as_ref()
            .is_some_and(|source| source.repr.starts_with("registry+"))
    );

    let alternative = dependency(&metadata, "registry_alternative");
    assert!(alternative.registry.is_some());
    assert!(
        alternative
            .source
            .as_ref()
            .is_some_and(|source| source.repr.starts_with("registry+"))
    );
    assert!(dependency(&metadata, "path_dependency").path.is_some());
    assert!(
        dependency(&metadata, "development_dependency").kind
            == cargo_metadata::DependencyKind::Development
    );
    assert!(
        dependency(&metadata, "build_dependency").kind == cargo_metadata::DependencyKind::Build
    );
    assert!(dependency(&metadata, "target_dependency").target.is_some());

    for key in [
        "git_default",
        "git_branch",
        "git_tag",
        "git_short",
        "git_non_hex",
    ] {
        let source = &dependency(&metadata, key)
            .source
            .as_ref()
            .expect("Git dependency should expose its source")
            .repr;
        assert!(!has_full_git_revision(source), "{key}");
    }
    let full = &dependency(&metadata, "git_full")
        .source
        .as_ref()
        .expect("full Git dependency should expose its source")
        .repr;
    assert!(has_full_git_revision(full));
    assert!(
        full.contains(
            oracle["full_git_revision"]
                .as_str()
                .expect("revision oracle should be a string")
        )
    );
    assert!(!has_full_git_revision(
        "git+https://git.invalid/private.git#0123456789abcdef0123456789abcdef01234567"
    ));
}

#[test]
fn every_declaration_kind_is_evaluated_and_nonmembers_are_excluded() {
    let (mut metadata, _) = protocol_metadata();
    let package = metadata
        .packages
        .first_mut()
        .expect("protocol metadata should contain its member");
    for key in [
        "development_dependency",
        "build_dependency",
        "target_dependency",
    ] {
        dependency_mut(package, key).req = VersionReq::STAR;
    }

    let mut nonmember = package.clone();
    nonmember.id.repr.push_str("-nonmember");
    metadata.packages.push(nonmember);

    let candidates = inspect(&metadata);
    assert_eq!(candidates.len(), 9);
    for key in [
        "registry_alias",
        "development_dependency",
        "build_dependency",
        "target_dependency",
    ] {
        assert!(candidates.iter().any(|candidate| {
            candidate.definition.id == CARGO_UNBOUNDED_REGISTRY.id
                && candidate.message
                    == format!(
                        "Registry dependency \"{key}\" uses an unbounded \"*\" version requirement."
                    )
        }));
    }
    assert!(candidates.iter().all(|candidate| {
        candidate.package == "cargo-health-protocol"
            && candidate.manifest_path.as_deref() == Some("Cargo.toml")
    }));
}

/// Every candidate the oracle names, and nothing but them.
fn assert_positive_oracle(candidates: &[Candidate], positive: &[Value]) {
    assert_eq!(candidates.len(), positive.len());
    for expected in positive {
        let message = expected["message"]
            .as_str()
            .expect("expected message should be a string");
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.message == message)
            .expect("every positive oracle should produce one candidate");
        let definition = candidate.definition;
        assert_eq!(definition.id, expected["code"]);
        assert_eq!(definition.category, expected["category"]);
        assert_eq!(definition.default_level, RuleLevel::Warn);
        assert_eq!(definition.help, expected["help"]);
        assert_eq!(candidate.package, "cargo-health-precision");
        assert_eq!(
            candidate.manifest_path.as_deref(),
            Some("member/Cargo.toml")
        );
    }
}

/// Every dependency the oracle expects the pack to stay quiet on is present in
/// the metadata and produced nothing. Both halves matter: a key the fixture
/// stopped declaring would pass a silence check on its own.
fn assert_negative_oracle(member: &Package, candidates: &[Candidate], negative_keys: &[Value]) {
    for key in negative_keys {
        let key = key
            .as_str()
            .expect("negative dependency key should be a string");
        assert!(
            member.dependencies.iter().any(|dependency| dependency
                .rename
                .as_deref()
                .unwrap_or(&dependency.name)
                == key),
            "negative dependency {key} should be represented in metadata"
        );
        assert!(
            candidates
                .iter()
                .all(|candidate| !candidate.message.contains(&format!("\"{key}\""))),
            "negative dependency {key} produced a candidate"
        );
    }
}

/// The reason each silence is a silence: the fixture really does declare an
/// optional, a development, a build and a target-gated entry, and a member
/// outside the workspace. Without this the negative oracle would pass over a
/// fixture that had quietly lost the shapes it exists to exercise.
fn assert_the_negatives_are_the_shapes_they_claim(
    metadata: &Metadata,
    member: &Package,
    candidates: &[Candidate],
) {
    let entry = |key: &str| {
        member
            .dependencies
            .iter()
            .find(|dependency| dependency.rename.as_deref().unwrap_or(&dependency.name) == key)
    };
    assert!(entry("registry_optional_alias").is_some_and(|entry| entry.optional));
    assert!(
        entry("registry_development")
            .is_some_and(|entry| entry.kind == cargo_metadata::DependencyKind::Development)
    );
    assert!(
        entry("registry_build")
            .is_some_and(|entry| entry.kind == cargo_metadata::DependencyKind::Build)
    );
    assert!(entry("registry_target").is_some_and(|entry| entry.target.is_some()));
    assert!(entry("path_external").is_some_and(|entry| entry.path.is_some()));
    assert!(
        metadata
            .packages
            .iter()
            .any(|package| package.name == "cargo-health-empty-member")
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.package != "cargo-health-external-path")
    );
}

/// Nothing a candidate publishes carries a source URL, a credential, a query,
/// an escape sequence or an absolute path.
fn assert_nothing_observable_leaks(candidates: &[Candidate], fixture: &Path) {
    let observable = candidates
        .iter()
        .map(|candidate| {
            let definition = candidate.definition;
            format!(
                "{} {} {:?} {} {} {} {}",
                definition.id,
                definition.category,
                definition.default_level,
                candidate.message,
                definition.help,
                candidate.package,
                candidate.manifest_path.as_deref().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    for secret in [
        "://",
        "git+",
        "user",
        "secret",
        "?",
        "#",
        "\u{1b}",
        fixture.to_string_lossy().as_ref(),
    ] {
        assert!(!observable.contains(secret), "leaked {secret:?}");
    }
}

#[test]
fn precision_matrix_has_exact_positive_and_negative_oracles_without_mutation_or_leaks() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cargo-health/precision");
    let before = fixture_hashes(&fixture);
    let oracle: Value = serde_json::from_slice(
        &fs::read(fixture.join("oracle.json")).expect("precision oracle should be readable"),
    )
    .expect("precision oracle should be valid JSON");
    let (metadata, _) = offline_metadata(&fixture, "precision");
    let candidates = inspect(&metadata);
    let positive = oracle["positive"]
        .as_array()
        .expect("positive oracle should be an array");
    let negative_keys = oracle["negative_keys"]
        .as_array()
        .expect("negative oracle should be an array");
    let member = metadata
        .packages
        .iter()
        .find(|package| package.name == "cargo-health-precision")
        .expect("precision member should be present");

    assert_eq!(positive.len(), 10);
    assert_eq!(negative_keys.len(), 12);
    assert_positive_oracle(&candidates, positive);
    assert_negative_oracle(member, &candidates, negative_keys);
    assert_the_negatives_are_the_shapes_they_claim(&metadata, member, &candidates);
    assert_nothing_observable_leaks(&candidates, &fixture);
    assert_eq!(fixture_hashes(&fixture), before);
}

#[test]
fn policy_prunes_the_producer_and_each_inactive_predicate() {
    let (metadata, _) = protocol_metadata();
    let all_off = PolicyPlan::default()
        .active_rules(crate::policy::Producer::CargoHealth)
        .map(|(definition, _)| definition.id)
        .fold(PolicyInput::default(), |input, id| {
            input.with_rule(id, RuleLevel::Off)
        });
    let all_off = PolicyPlan::compile(&all_off).expect("policy should compile");
    let scan = super::inspect(&metadata, &all_off);
    assert!(scan.candidates.is_empty());
    assert_eq!(scan.counters.dependencies_evaluated, 0);
    assert_eq!(scan.counters.unbounded_registry_predicates, 0);
    assert_eq!(scan.counters.unpinned_git_predicates, 0);

    let git_off = PolicyInput::default()
        .with_rule(CARGO_UNPINNED_GIT.id, RuleLevel::Off)
        .with_rule(CARGO_DUPLICATE_MAJOR_VERSIONS.id, RuleLevel::Off)
        .with_rule(CARGO_MISSING_LOCKFILE.id, RuleLevel::Off);
    let git_off = PolicyPlan::compile(&git_off).expect("policy should compile");
    let scan = super::inspect(&metadata, &git_off);
    assert!(scan.counters.dependencies_evaluated > 0);
    assert!(scan.counters.unbounded_registry_predicates > 0);
    assert_eq!(scan.counters.unpinned_git_predicates, 0);
    assert!(
        scan.candidates
            .iter()
            .all(|candidate| candidate.definition.id != CARGO_UNPINNED_GIT.id)
    );
}

#[test]
fn empty_dependency_and_unknown_source_inputs_are_safe_and_empty() {
    let (metadata, _) = protocol_metadata();

    let mut no_packages = metadata.clone();
    no_packages.packages.clear();
    assert!(inspect(&no_packages).is_empty());

    let mut no_dependencies = metadata.clone();
    no_dependencies
        .packages
        .first_mut()
        .expect("protocol member should exist")
        .dependencies
        .clear();
    assert!(inspect(&no_dependencies).is_empty());

    let mut unknown_source = metadata;
    let package = unknown_source
        .packages
        .first_mut()
        .expect("protocol member should exist");
    let dependency = package
        .dependencies
        .first_mut()
        .expect("protocol dependency should exist");
    dependency.req = VersionReq::STAR;
    dependency.path = None;
    dependency
        .source
        .as_mut()
        .expect("protocol dependency should expose a source")
        .repr = "future+opaque\u{1b}[31m?credential=secret#fragment".to_owned();
    package.dependencies.truncate(1);
    assert!(inspect(&unknown_source).is_empty());
}

#[test]
fn git_oracle_rejects_duplicate_empty_and_future_queries_without_leaking_sources() {
    for source in [
        "git+https://secret@git.invalid/repo.git?",
        "git+https://secret@git.invalid/repo.git?branch=main",
        "git+https://secret@git.invalid/repo.git?rev=0123456",
        "git+https://secret@git.invalid/repo.git?rev=gggggggggggggggggggggggggggggggggggggggg",
        "git+https://secret@git.invalid/repo.git?rev=0123456789abcdef0123456789abcdef01234567&rev=0123456789abcdef0123456789abcdef01234567",
        "git+https://secret@git.invalid/repo.git?future=0123456789abcdef0123456789abcdef01234567",
    ] {
        assert!(!has_full_git_revision(source));
    }

    let (metadata, _) = protocol_metadata();
    let candidates = inspect(&metadata);
    assert_eq!(candidates.len(), 6);
    assert!(candidates.iter().all(|candidate| {
        let definition = candidate.definition;
        let observable = format!(
            "{} {} {} {} {} {}",
            definition.id,
            definition.category,
            candidate.message,
            definition.help,
            candidate.package,
            candidate.manifest_path.as_deref().unwrap_or_default()
        );
        !observable.contains("git.invalid")
            && !observable.contains("secret")
            && !observable.contains("0123456789abcdef")
    }));
    assert!(candidates.iter().any(|candidate| {
        candidate.definition.id == CARGO_UNBOUNDED_REGISTRY.id
            && candidate.message
                == "Registry dependency \"registry_alias\" uses an unbounded \"*\" version requirement."
    }));
    assert_eq!(
        candidates
            .iter()
            .filter(|candidate| candidate.definition.id == CARGO_UNPINNED_GIT.id)
            .count(),
        5
    );
}

/// US-008: the reader returns the profile values with byte spans that
/// convert to the same line and column the crate's span helpers publish
/// elsewhere, and a virtual workspace manifest with no `[package]` still
/// yields its profiles.
#[test]
fn the_profile_reader_returns_values_with_line_accurate_spans() {
    let source = "[workspace]\nmembers = []\n\n[profile.release]\ndebug = 2\nstrip = \"none\"\noverflow-checks = false\n";
    let document: ManifestProfileDocument =
        toml::from_str(source).expect("the virtual manifest should parse");
    let profiles = document.profile.expect("the profiles should be read");
    let (key, settings) = profiles
        .iter()
        .next()
        .expect("the release profile should be read");
    assert_eq!(key.get_ref(), "release");
    let starts = line_starts(source);
    assert_eq!(byte_range_span(key.span(), &starts, source).line_start, 4);

    let debug = settings.debug.as_ref().expect("debug should be read");
    assert!(matches!(debug.get_ref(), TomlScalar::Level(2)));
    assert_eq!(byte_range_span(debug.span(), &starts, source).line_start, 5);

    let strip = settings.strip.as_ref().expect("strip should be read");
    assert!(matches!(strip.get_ref(), TomlScalar::Name(name) if name == "none"));
    assert_eq!(byte_range_span(strip.span(), &starts, source).line_start, 6);

    let checks = settings
        .overflow_checks
        .as_ref()
        .expect("overflow-checks should be read");
    assert!(matches!(checks.get_ref(), TomlScalar::Truth(false)));
    let span = byte_range_span(checks.span(), &starts, source);
    assert_eq!((span.line_start, span.line_end), (7, 7));
}

/// US-008: an absent manifest is an observed fact, an oversized one and an
/// unparseable one are closed errors, never a partial read.
#[test]
fn the_profile_reader_bounds_what_it_opens() {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!(
        "cargo-health-profile-reader-{}",
        std::process::id()
    ));
    fs::create_dir_all(&scratch).expect("the scratch directory should be creatable");

    assert!(
        read_manifest::<ManifestProfileDocument>(
            &scratch.join("absent").join("Cargo.toml"),
            PROFILE_TABLE_UNPARSEABLE,
        )
            .expect("an absent manifest is not an error")
            .is_none()
    );

    let oversized = scratch.join("oversized.toml");
    fs::write(&oversized, vec![b'#'; MAX_MANIFEST_BYTES as usize + 1])
        .expect("the oversized manifest should be writable");
    let error = read_manifest::<ManifestProfileDocument>(&oversized, PROFILE_TABLE_UNPARSEABLE).expect_err("the bound should refuse");
    assert_eq!(error.code, "manifest-unreadable");

    let invalid = scratch.join("invalid.toml");
    fs::write(&invalid, "[profile.release]\ndebug = 1.5\n")
        .expect("the invalid manifest should be writable");
    let error = read_manifest::<ManifestProfileDocument>(&invalid, PROFILE_TABLE_UNPARSEABLE).expect_err("the shape should refuse");
    assert_eq!(error.code, "manifest-invalid");
    assert!(!error.message.contains('/'));

    fs::remove_dir_all(&scratch).expect("the scratch directory should be removable");
}

/// US-010: the closed list matches every published spelling and nothing
/// else, and each hit is returned under its canonical separated form.
#[test]
fn the_closed_rustflags_list_matches_every_spelling_and_nothing_else() {
    let arguments = |flags: &[&'static str]| {
        flags
            .iter()
            .map(|flag| (*flag, 0..flag.len()))
            .collect::<Vec<_>>()
    };
    let matched = |flags: &[&'static str]| {
        neutralizing_rustflags(&arguments(flags))
            .into_iter()
            .map(|(flag, _)| flag)
            .collect::<Vec<_>>()
    };

    assert_eq!(matched(&["--cap-lints", "allow"]), ["--cap-lints allow"]);
    assert_eq!(matched(&["--cap-lints=allow"]), ["--cap-lints allow"]);
    for spelling in [
        ["-A", "warnings"].as_slice(),
        ["-Awarnings"].as_slice(),
        ["--allow", "warnings"].as_slice(),
        ["--allow=warnings"].as_slice(),
    ] {
        assert_eq!(matched(spelling), ["-A warnings"], "{spelling:?}");
    }
    for spelling in [
        ["-C", "overflow-checks=off"].as_slice(),
        ["-Coverflow-checks=false"].as_slice(),
        ["--codegen", "overflow-checks=no"].as_slice(),
        ["--codegen=overflow-checks=0"].as_slice(),
    ] {
        assert_eq!(matched(spelling), ["-C overflow-checks=off"], "{spelling:?}");
    }
    assert_eq!(
        matched(&["-C", "target-cpu=native", "--cap-lints", "allow", "-Awarnings"]),
        ["--cap-lints allow", "-A warnings"]
    );

    for negative in [
        ["-C", "target-cpu=native"].as_slice(),
        ["--cap-lints", "warn"].as_slice(),
        ["--cap-lints"].as_slice(),
        ["-A", "dead_code"].as_slice(),
        ["-D", "warnings"].as_slice(),
        ["-Coverflow-checks=on"].as_slice(),
        ["overflow-checks=off"].as_slice(),
        ["-C"].as_slice(),
    ] {
        assert_eq!(matched(negative), [""; 0], "{negative:?}");
    }
}

/// US-009: `debug = "full"` is the named spelling of the level `2`, and a
/// strip mode that removes debug info silences the finding while
/// `strip = false` does not.
#[test]
fn the_debug_symbols_rule_reads_the_named_level_and_the_strip_modes() {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!(
        "cargo-health-debug-full-{}",
        std::process::id()
    ));
    fs::create_dir_all(&scratch).expect("the scratch directory should be creatable");
    let (mut metadata, _) = protocol_metadata();
    metadata.workspace_root = scratch
        .to_str()
        .expect("the scratch path should be UTF-8")
        .into();

    let observed = |manifest: &str| {
        fs::write(scratch.join("Cargo.toml"), manifest)
            .expect("the scratch manifest should be writable");
        let scan = super::inspect(&metadata, &PolicyPlan::default());
        assert!(scan.errors.is_empty(), "{:?}", scan.errors);
        scan.candidates
            .iter()
            .filter(|candidate| {
                candidate.definition.id == CARGO_RELEASE_DEBUG_SYMBOLS.id
            })
            .count()
    };

    assert_eq!(observed("[profile.release]\ndebug = \"full\"\nstrip = false\n"), 1);
    assert_eq!(observed("[profile.release]\ndebug = \"full\"\nstrip = \"debuginfo\"\n"), 0);
    assert_eq!(observed("[profile.release]\ndebug = 1\n"), 0);

    fs::remove_dir_all(&scratch).expect("the scratch directory should be removable");
}

/// US-010: a cargo configuration the parser cannot read is a bounded
/// error, never a partial judgement, and the rest of the pack still
/// reports.
#[test]
fn a_malformed_cargo_config_abstains_with_a_bounded_error() {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!(
        "cargo-health-config-invalid-{}",
        std::process::id()
    ));
    fs::create_dir_all(scratch.join(".cargo")).expect("the scratch should be creatable");
    fs::write(scratch.join(".cargo/config.toml"), "[build\nrustflags = [\n")
        .expect("the malformed configuration should be writable");

    let (mut metadata, _) = protocol_metadata();
    metadata.workspace_root = scratch
        .to_str()
        .expect("the scratch path should be UTF-8")
        .into();
    let scan = super::inspect(&metadata, &PolicyPlan::default());
    assert!(
        scan.errors
            .iter()
            .any(|error| error.code == "cargo-config-invalid"),
        "{:?}",
        scan.errors
    );
    assert!(
        scan.candidates
            .iter()
            .all(|candidate| candidate.definition.id != CARGO_PERMISSIVE_RUSTFLAGS.id),
        "a malformed configuration still produced a rustflags finding"
    );
    assert!(
        !scan.candidates.is_empty(),
        "the rest of the pack should still report"
    );

    fs::remove_dir_all(&scratch).expect("the scratch directory should be removable");
}

fn dependency_mut<'a>(
    package: &'a mut cargo_metadata::Package,
    key: &str,
) -> &'a mut Dependency {
    package
        .dependencies
        .iter_mut()
        .find(|dependency| dependency.rename.as_deref().unwrap_or(&dependency.name) == key)
        .expect("protocol dependency should exist")
}

fn resolution_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cargo-health/resolution")
        .join(name)
}

/// `cargo metadata --no-deps` resolves nothing, so it never writes a
/// lockfile: the resolution fixtures stay intact.
fn resolution_scan(name: &str) -> CargoHealthScan {
    let root = resolution_fixture(name);
    let before = fixture_hashes(&root);
    let (metadata, _) = offline_metadata(&root, &format!("resolution-{name}"));
    let scan = super::inspect(&metadata, &PolicyPlan::default());
    assert_eq!(fixture_hashes(&root), before, "{name} was mutated");
    scan
}

fn candidates_for(scan: &CargoHealthScan, definition: &RuleDefinition) -> Vec<String> {
    scan.candidates
        .iter()
        .filter(|candidate| candidate.definition.id == definition.id)
        .map(|candidate| candidate.message.clone())
        .collect()
}

/// US-076: two major versions of the same crate in the resolved graph are
/// named, versions included, and a minor divergence is not.
#[test]
fn duplicate_major_versions_are_named_with_their_versions() {
    let scan = resolution_scan("duplicate");
    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert_eq!(scan.counters.resolved_packages, 5);

    assert_eq!(
        candidates_for(&scan, &CARGO_DUPLICATE_MAJOR_VERSIONS),
        ["Crate \"shared\" is resolved with incompatible major versions 1.4.2, 2.0.0."]
    );
    let candidate = scan
        .candidates
        .iter()
        .find(|candidate| candidate.definition.id == CARGO_DUPLICATE_MAJOR_VERSIONS.id)
        .expect("the duplicate candidate should exist");
    assert_eq!(candidate.package, "duplicate-resolution");
    assert_eq!(candidate.manifest_path.as_deref(), Some("Cargo.lock"));
    assert_eq!(candidate.definition.category, "dependencies");

    // The fixture also carries two minor versions of the same crate: they
    // stay compatible, so they are not reported.
    assert!(!candidate.message.contains("aligned"));
    assert!(candidates_for(&scan, &CARGO_MISSING_LOCKFILE).is_empty());
}

/// US-076: a workspace clean on these criteria produces no diagnostic.
#[test]
fn a_clean_resolution_produces_no_candidate() {
    let scan = resolution_scan("clean");
    assert!(scan.candidates.is_empty(), "{:?}", scan.candidates);
    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert_eq!(scan.counters.resolved_packages, 2);
}

/// US-076: a binary with no lockfile is reported, naming the package and
/// its manifest.
#[test]
fn a_binary_without_a_lockfile_is_named() {
    let scan = resolution_scan("absent");
    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert_eq!(
        candidates_for(&scan, &CARGO_MISSING_LOCKFILE),
        [
            "Package \"absent-resolution\" produces a binary but no Cargo.lock sits next to its workspace manifest."
        ]
    );
    let candidate = &scan.candidates[0];
    assert_eq!(candidate.package, "absent-resolution");
    assert_eq!(candidate.manifest_path.as_deref(), Some("Cargo.toml"));
    assert!(candidates_for(&scan, &CARGO_DUPLICATE_MAJOR_VERSIONS).is_empty());
}

/// US-076: a missing resolution section makes the pack abstain with a
/// bounded error, and the scan stays usable.
#[test]
fn an_unusable_resolution_abstains_with_a_bounded_error() {
    let scan = resolution_scan("unusable");
    assert!(scan.candidates.is_empty(), "{:?}", scan.candidates);
    assert_eq!(
        scan.errors,
        [CargoHealthError {
            code: "lockfile-resolution-absent",
            message: "the lockfile carries no resolved package section",
        }]
    );
    for error in &scan.errors {
        assert!(!error.message.contains('/'));
        assert!(!error.message.contains('\u{1b}'));
    }
}

/// US-076: a path dependency that leaves the workspace is reported without
/// publishing the absolute path it points at.
#[test]
fn a_path_dependency_leaving_the_workspace_is_named_without_its_path() {
    let scan = resolution_scan("outside-path");
    assert_eq!(
        candidates_for(&scan, &CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE),
        ["Path dependency \"outside-crate\" resolves outside the workspace."]
    );
    let candidate = &scan.candidates[0];
    assert_eq!(candidate.package, "outside-path");
    assert_eq!(candidate.manifest_path.as_deref(), Some("Cargo.toml"));
    assert!(!candidate.message.contains(env!("CARGO_MANIFEST_DIR")));
    assert!(!candidate.message.contains(".."));
}

/// The ordering of major versions is independent of file order and ignores
/// pre-releases and build metadata.
#[test]
fn major_comparison_is_order_independent_and_ignores_prerelease_metadata() {
    let packages = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(name, version)| ((*name).to_owned(), (*version).to_owned()))
            .collect::<Vec<_>>()
    };

    assert_eq!(
        duplicate_major_versions(&packages(&[("a", "2.0.0"), ("a", "1.0.0")])),
        [("a".to_owned(), "1.0.0, 2.0.0".to_owned())]
    );
    assert!(
        duplicate_major_versions(&packages(&[("a", "1.0.0-rc.1"), ("a", "1.0.0+build")]))
            .is_empty()
    );
    assert!(duplicate_major_versions(&packages(&[("a", "1.0.0"), ("b", "2.0.0")])).is_empty());
    assert!(duplicate_major_versions(&packages(&[("a", "1.0.0"), ("a", "1.0.0")])).is_empty());
    // An unreadable version cannot be compared, so it cannot ground a
    // duplication verdict.
    assert!(duplicate_major_versions(&packages(&[("a", "x.0.0"), ("a", "1.0.0")])).is_empty());
    assert_eq!(major_of("10.2.3"), Some("10"));
    assert_eq!(major_of(""), None);
}
