use cargo_metadata::{Dependency, Metadata};

use crate::report::Severity;

const UNBOUNDED_REGISTRY_CODE: &str = "rust_doctor::cargo::unbounded_registry_dependency";
const UNPINNED_GIT_CODE: &str = "rust_doctor::cargo::unpinned_git_dependency";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rule {
    code: &'static str,
    category: &'static str,
    severity: Severity,
    help: &'static str,
}

const RULES: [Rule; 2] = [
    Rule {
        code: UNBOUNDED_REGISTRY_CODE,
        category: "reliability",
        severity: Severity::Warning,
        help: "Replace the unbounded version requirement with the minimum compatible version intended by the project.",
    },
    Rule {
        code: UNPINNED_GIT_CODE,
        category: "security",
        severity: Severity::Warning,
        help: "Set rev to the full 40-character commit SHA intended by the project.",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) code: &'static str,
    pub(crate) category: &'static str,
    pub(crate) severity: Severity,
    pub(crate) message: String,
    pub(crate) help: &'static str,
    pub(crate) package: String,
    pub(crate) manifest_path: Option<String>,
}

pub(crate) fn inspect(metadata: &Metadata) -> Vec<Candidate> {
    let mut candidates = Vec::new();

    for package in metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
    {
        let manifest_path = package
            .manifest_path
            .strip_prefix(&metadata.workspace_root)
            .ok()
            .map(|path| path.as_str().to_owned());

        for dependency in &package.dependencies {
            let key = dependency.rename.as_deref().unwrap_or(&dependency.name);
            if is_unbounded_registry(dependency) {
                let rule = &RULES[0];
                candidates.push(Candidate {
                    code: rule.code,
                    category: rule.category,
                    severity: rule.severity,
                    message: format!(
                        "Registry dependency \"{key}\" uses an unbounded \"*\" version requirement."
                    ),
                    help: rule.help,
                    package: package.name.to_string(),
                    manifest_path: manifest_path.clone(),
                });
            }
            if is_unpinned_git(dependency) {
                let rule = &RULES[1];
                candidates.push(Candidate {
                    code: rule.code,
                    category: rule.category,
                    severity: rule.severity,
                    message: format!(
                        "Git dependency \"{key}\" is not pinned to a full commit revision."
                    ),
                    help: rule.help,
                    package: package.name.to_string(),
                    manifest_path: manifest_path.clone(),
                });
            }
        }
    }

    candidates
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
    fn registry_is_the_exact_normative_inventory() {
        assert_eq!(
            RULES,
            [
                Rule {
                    code: UNBOUNDED_REGISTRY_CODE,
                    category: "reliability",
                    severity: Severity::Warning,
                    help: "Replace the unbounded version requirement with the minimum compatible version intended by the project.",
                },
                Rule {
                    code: UNPINNED_GIT_CODE,
                    category: "security",
                    severity: Severity::Warning,
                    help: "Set rev to the full 40-character commit SHA intended by the project.",
                },
            ]
        );
        assert!(RULES.windows(2).all(|pair| pair[0].code < pair[1].code));
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
                candidate.code == UNBOUNDED_REGISTRY_CODE
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
            assert_eq!(candidate.code, expected["code"]);
            assert_eq!(candidate.category, expected["category"]);
            assert_eq!(candidate.severity, Severity::Warning);
            assert_eq!(candidate.help, expected["help"]);
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
                format!(
                    "{} {} {:?} {} {} {} {}",
                    candidate.code,
                    candidate.category,
                    candidate.severity,
                    candidate.message,
                    candidate.help,
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
            let observable = format!(
                "{} {} {} {} {} {}",
                candidate.code,
                candidate.category,
                candidate.message,
                candidate.help,
                candidate.package,
                candidate.manifest_path.as_deref().unwrap_or_default()
            );
            !observable.contains("git.invalid")
                && !observable.contains("secret")
                && !observable.contains("0123456789abcdef")
        }));
        assert!(candidates.iter().any(|candidate| {
            candidate.code == UNBOUNDED_REGISTRY_CODE
                && candidate.message
                    == "Registry dependency \"registry_alias\" uses an unbounded \"*\" version requirement."
        }));
        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| candidate.code == UNPINNED_GIT_CODE)
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
}
