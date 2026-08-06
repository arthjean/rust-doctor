use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use cargo_metadata::{Dependency, Metadata, Package, TargetKind};
use serde::Deserialize;

use crate::policy::{
    CARGO_DUPLICATE_MAJOR_VERSIONS, CARGO_MISSING_LOCKFILE,
    CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE, CARGO_UNBOUNDED_REGISTRY, CARGO_UNPINNED_GIT,
    PolicyPlan, RuleDefinition,
};

/// Name of the offline resolved graph. `cargo metadata` runs with `--no-deps`,
/// so `metadata.resolve` is always absent: the only graph readable without a
/// registry index and without the network is the lockfile.
const LOCKFILE: &str = "Cargo.lock";

/// Bounds the pack's work on a hostile or gigantic lockfile. Beyond it, the
/// pack abstains instead of loading the file.
const MAX_LOCKFILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) definition: &'static RuleDefinition,
    pub(crate) message: String,
    pub(crate) package: String,
    pub(crate) manifest_path: Option<String>,
}

/// Bounded error of the pack: a closed code and a frozen message, with no
/// path, no environment variable and no escape sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CargoHealthError {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
}

#[derive(Debug, Default)]
pub(crate) struct CargoHealthScan {
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) errors: Vec<CargoHealthError>,
    #[allow(dead_code)]
    pub(crate) counters: CargoHealthCounters,
}

#[derive(Debug, Default)]
pub(crate) struct CargoHealthCounters {
    pub(crate) dependencies_evaluated: usize,
    pub(crate) unbounded_registry_predicates: usize,
    pub(crate) unpinned_git_predicates: usize,
    pub(crate) resolved_packages: usize,
}

/// Minimal shape of the lockfile: only the resolved list matters, and all the
/// rest of the document is ignored without making it invalid.
#[derive(Debug, Deserialize)]
struct LockDocument {
    package: Option<Vec<LockPackage>>,
}

#[derive(Debug, Deserialize)]
struct LockPackage {
    name: Option<String>,
    version: Option<String>,
}

/// State of the resolved graph as the pack can observe it offline.
#[derive(Debug, PartialEq, Eq)]
enum Resolution {
    /// No lockfile: this is an observed fact, not an error.
    Absent,
    /// The file exists but its resolution section is unusable.
    Unusable(CargoHealthError),
    /// (name, version) pairs of the resolved graph, in file order.
    Packages(Vec<(String, String)>),
}

pub(crate) fn inspect(metadata: &Metadata, plan: &PolicyPlan) -> CargoHealthScan {
    let unbounded_registry = plan.is_active(CARGO_UNBOUNDED_REGISTRY.id);
    let unpinned_git = plan.is_active(CARGO_UNPINNED_GIT.id);
    let outside_path = plan.is_active(CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE.id);
    let missing_lockfile = plan.is_active(CARGO_MISSING_LOCKFILE.id);
    let duplicate_majors = plan.is_active(CARGO_DUPLICATE_MAJOR_VERSIONS.id);
    if !unbounded_registry
        && !unpinned_git
        && !outside_path
        && !missing_lockfile
        && !duplicate_majors
    {
        return CargoHealthScan::default();
    }

    let mut scan = CargoHealthScan::default();
    let workspace_root = metadata.workspace_root.as_std_path();

    for package in workspace_packages(metadata) {
        let manifest_path = package
            .manifest_path
            .strip_prefix(&metadata.workspace_root)
            .ok()
            .map(|path| path.as_str().to_owned());

        for dependency in &package.dependencies {
            scan.counters.dependencies_evaluated += 1;
            let key = dependency.rename.as_deref().unwrap_or(&dependency.name);
            if unbounded_registry {
                scan.counters.unbounded_registry_predicates += 1;
            }
            if unbounded_registry && is_unbounded_registry(dependency) {
                scan.candidates.push(Candidate {
                    definition: &CARGO_UNBOUNDED_REGISTRY,
                    message: format!(
                        "Registry dependency \"{key}\" uses an unbounded \"*\" version requirement."
                    ),
                    package: package.name.to_string(),
                    manifest_path: manifest_path.clone(),
                });
            }
            if unpinned_git {
                scan.counters.unpinned_git_predicates += 1;
            }
            if unpinned_git && is_unpinned_git(dependency) {
                scan.candidates.push(Candidate {
                    definition: &CARGO_UNPINNED_GIT,
                    message: format!(
                        "Git dependency \"{key}\" is not pinned to a full commit revision."
                    ),
                    package: package.name.to_string(),
                    manifest_path: manifest_path.clone(),
                });
            }
            if outside_path && leaves_workspace(dependency, workspace_root) {
                scan.candidates.push(Candidate {
                    definition: &CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE,
                    message: format!("Path dependency \"{key}\" resolves outside the workspace."),
                    package: package.name.to_string(),
                    manifest_path: manifest_path.clone(),
                });
            }
        }
    }

    if !missing_lockfile && !duplicate_majors {
        return scan;
    }

    match read_resolution(workspace_root) {
        Resolution::Absent if missing_lockfile => {
            for package in workspace_packages(metadata).filter(|package| produces_a_binary(package))
            {
                scan.candidates.push(Candidate {
                    definition: &CARGO_MISSING_LOCKFILE,
                    // The pack observes the disk, not the version control
                    // index: the message therefore states what is measured,
                    // the missing file, and the rule help carries the fix.
                    message: format!(
                        "Package \"{}\" produces a binary but no {LOCKFILE} sits next to its workspace manifest.",
                        package.name
                    ),
                    package: package.name.to_string(),
                    manifest_path: package
                        .manifest_path
                        .strip_prefix(&metadata.workspace_root)
                        .ok()
                        .map(|path| path.as_str().to_owned()),
                });
            }
        }
        Resolution::Absent => {}
        Resolution::Unusable(error) => scan.errors.push(error),
        Resolution::Packages(packages) => {
            scan.counters.resolved_packages = packages.len();
            if duplicate_majors {
                let owner = resolution_owner(metadata);
                for (name, versions) in duplicate_major_versions(&packages) {
                    scan.candidates.push(Candidate {
                        definition: &CARGO_DUPLICATE_MAJOR_VERSIONS,
                        message: format!(
                            "Crate \"{name}\" is resolved with incompatible major versions {versions}."
                        ),
                        package: owner.clone(),
                        manifest_path: Some(LOCKFILE.to_owned()),
                    });
                }
            }
        }
    }

    scan
}

fn workspace_packages(metadata: &Metadata) -> impl Iterator<Item = &Package> {
    metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
}

fn produces_a_binary(package: &Package) -> bool {
    package
        .targets
        .iter()
        .any(|target| target.kind.contains(&TargetKind::Bin))
}

/// The resolved graph belongs to the workspace, not to a member. The diagnostic
/// is therefore attached to the root package when it exists, otherwise to the
/// first member by name order, which stays deterministic on a virtual
/// workspace.
fn resolution_owner(metadata: &Metadata) -> String {
    let root_manifest = metadata.workspace_root.join("Cargo.toml");
    if let Some(root) = workspace_packages(metadata)
        .find(|package| package.manifest_path == root_manifest)
        .or_else(|| workspace_packages(metadata).min_by_key(|package| package.name.to_string()))
    {
        return root.name.to_string();
    }
    metadata
        .workspace_root
        .file_name()
        .unwrap_or_default()
        .to_owned()
}

fn leaves_workspace(dependency: &Dependency, workspace_root: &Path) -> bool {
    dependency
        .path
        .as_ref()
        .is_some_and(|path| !path.as_std_path().starts_with(workspace_root))
}

fn read_resolution(workspace_root: &Path) -> Resolution {
    let path = workspace_root.join(LOCKFILE);
    let Ok(metadata) = fs::metadata(&path) else {
        return Resolution::Absent;
    };
    if !metadata.is_file() || metadata.len() > MAX_LOCKFILE_BYTES {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-unreadable",
            message: "the lockfile is not a readable regular file within the published size limit",
        });
    }
    let Ok(contents) = fs::read_to_string(&path) else {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-unreadable",
            message: "the lockfile could not be read as UTF-8 text",
        });
    };
    let Ok(document) = toml::from_str::<LockDocument>(&contents) else {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-invalid",
            message: "the lockfile is not valid TOML",
        });
    };
    let Some(entries) = document.package else {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-resolution-absent",
            message: "the lockfile carries no resolved package section",
        });
    };

    let packages: Vec<_> = entries
        .iter()
        .filter_map(|entry| Some((entry.name.clone()?, entry.version.clone()?)))
        .collect();
    if packages.len() != entries.len() {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-resolution-absent",
            message: "the lockfile carries a resolved package without a name or a version",
        });
    }
    Resolution::Packages(packages)
}

/// Two versions of the same crate whose major number differs are not
/// interchangeable: their types do not unify and the binary embeds both copies.
/// Pre-releases and build metadata are ignored, only the major part of the
/// triplet is compared.
fn duplicate_major_versions(packages: &[(String, String)]) -> Vec<(String, String)> {
    let mut majors = BTreeMap::<&str, Vec<&str>>::new();
    for (name, version) in packages {
        let entry = majors.entry(name.as_str()).or_default();
        if !entry.contains(&version.as_str()) {
            entry.push(version.as_str());
        }
    }

    majors
        .into_iter()
        .filter_map(|(name, mut versions)| {
            versions.sort_unstable();
            let distinct: Vec<_> = versions
                .iter()
                .filter_map(|version| major_of(version))
                .collect();
            let mut unique = distinct.clone();
            unique.sort_unstable();
            unique.dedup();
            (unique.len() > 1).then(|| (name.to_owned(), versions.join(", ")))
        })
        .collect()
}

fn major_of(version: &str) -> Option<&str> {
    let major = version
        .split(['+', '-'])
        .next()?
        .split('.')
        .next()
        .filter(|major| !major.is_empty() && major.bytes().all(|byte| byte.is_ascii_digit()))?;
    Some(major)
}

fn is_unbounded_registry(dependency: &Dependency) -> bool {
    dependency.path.is_none()
        && dependency
            .source
            .as_ref()
            .is_some_and(|source| source.repr.starts_with("registry+"))
        && dependency.req == cargo_metadata::semver::VersionReq::STAR
}

fn is_unpinned_git(dependency: &Dependency) -> bool {
    dependency.path.is_none()
        && dependency.source.as_ref().is_some_and(|source| {
            source.repr.starts_with("git+") && !has_full_git_revision(&source.repr)
        })
}

fn has_full_git_revision(source: &str) -> bool {
    let Some((_, query_and_fragment)) = source.split_once('?') else {
        return false;
    };
    let query = query_and_fragment.split('#').next().unwrap_or_default();
    let mut parameters = query.split('&');
    let Some(parameter) = parameters.next() else {
        return false;
    };
    if parameters.next().is_some() {
        return false;
    }
    let Some(("rev", revision)) = parameter.split_once('=') else {
        return false;
    };

    revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cargo_metadata::semver::VersionReq;
    use serde_json::Value;

    use super::*;
    use crate::policy::{PolicyInput, RuleLevel};

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
                CARGO_UNBOUNDED_REGISTRY.id,
                CARGO_UNPINNED_GIT.id,
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
            include_str!("../Cargo.lock")
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

        assert_eq!(positive.len(), 10);
        assert_eq!(negative_keys.len(), 12);
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

        let member = metadata
            .packages
            .iter()
            .find(|package| package.name == "cargo-health-precision")
            .expect("precision member should be present");
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

        assert!(
            member
                .dependencies
                .iter()
                .find(|dependency| dependency.rename.as_deref() == Some("registry_optional_alias"))
                .is_some_and(|dependency| dependency.optional)
        );
        assert!(
            member
                .dependencies
                .iter()
                .find(|dependency| dependency.rename.as_deref() == Some("registry_development"))
                .is_some_and(|dependency| {
                    dependency.kind == cargo_metadata::DependencyKind::Development
                })
        );
        assert!(
            member
                .dependencies
                .iter()
                .find(|dependency| dependency.rename.as_deref() == Some("registry_build"))
                .is_some_and(|dependency| {
                    dependency.kind == cargo_metadata::DependencyKind::Build
                })
        );
        assert!(
            member
                .dependencies
                .iter()
                .find(|dependency| dependency.rename.as_deref() == Some("registry_target"))
                .is_some_and(|dependency| dependency.target.is_some())
        );
        assert!(
            metadata
                .packages
                .iter()
                .any(|package| package.name == "cargo-health-empty-member")
        );
        assert!(
            member
                .dependencies
                .iter()
                .find(|dependency| {
                    dependency.rename.as_deref().unwrap_or(&dependency.name) == "path_external"
                })
                .is_some_and(|dependency| dependency.path.is_some())
        );
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.package != "cargo-health-external-path")
        );

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
}
