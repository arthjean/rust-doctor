use cargo_metadata::{DependencyKind, MetadataCommand, TargetKind};
use std::collections::BTreeSet;
#[cfg(test)]
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Detected framework or runtime in the project's dependencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Framework {
    Tokio,
    AsyncStd,
    Smol,
    Axum,
    ActixWeb,
    Rocket,
    Warp,
    Diesel,
    Sqlx,
    SeaOrm,
    Tonic,
    WasmBindgen,
    WebSys,
    Embassy,
    CortexM,
}

/// Package-specific dependency evidence used to gate framework rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameworkCapability {
    pub(crate) framework: Framework,
    pub(crate) version: Option<String>,
    pub(crate) enabled_features: Vec<String>,
    pub(crate) target_contexts: Vec<String>,
    pub(crate) active: bool,
    pub(crate) gate_reason: Option<String>,
}

impl std::fmt::Display for Framework {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tokio => write!(f, "tokio"),
            Self::AsyncStd => write!(f, "async-std"),
            Self::Smol => write!(f, "smol"),
            Self::Axum => write!(f, "axum"),
            Self::ActixWeb => write!(f, "actix-web"),
            Self::Rocket => write!(f, "rocket"),
            Self::Warp => write!(f, "warp"),
            Self::Diesel => write!(f, "diesel"),
            Self::Sqlx => write!(f, "sqlx"),
            Self::SeaOrm => write!(f, "sea-orm"),
            Self::Tonic => write!(f, "tonic"),
            Self::WasmBindgen => write!(f, "wasm-bindgen"),
            Self::WebSys => write!(f, "web-sys"),
            Self::Embassy => write!(f, "embassy"),
            Self::CortexM => write!(f, "cortex-m"),
        }
    }
}

/// Maps crate dependency names to Framework variants.
/// For prefix-based matching (embassy-*), see `detect_frameworks`.
const FRAMEWORK_MAP: &[(&str, Framework)] = &[
    ("tokio", Framework::Tokio),
    ("async-std", Framework::AsyncStd),
    ("smol", Framework::Smol),
    ("axum", Framework::Axum),
    ("actix-web", Framework::ActixWeb),
    ("rocket", Framework::Rocket),
    ("warp", Framework::Warp),
    ("diesel", Framework::Diesel),
    ("sqlx", Framework::Sqlx),
    ("sea-orm", Framework::SeaOrm),
    ("tonic", Framework::Tonic),
    ("wasm-bindgen", Framework::WasmBindgen),
    ("web-sys", Framework::WebSys),
    ("cortex-m", Framework::CortexM),
];

/// Discovered project information from cargo metadata.
#[derive(Debug)]
pub struct ProjectInfo {
    /// Absolute path to the workspace or project root.
    pub root_dir: PathBuf,
    /// Primary package name (first workspace member, or the single package).
    pub name: String,
    /// Primary package version.
    pub version: String,
    /// Cargo's stable package identity string.
    pub package_id: String,
    /// Primary Cargo targets with their target kinds.
    pub targets: Vec<String>,
    /// Rust edition of the primary package.
    pub edition: String,
    /// Detected frameworks/runtimes from dependencies.
    pub frameworks: Vec<Framework>,
    /// Versioned framework evidence across the selected package graph.
    pub(crate) framework_capabilities: Vec<FrameworkCapability>,
    /// Whether this is a Cargo workspace (>1 member).
    pub is_workspace: bool,
    /// Number of workspace members.
    pub member_count: usize,
    /// Whether the primary package has a build script (build.rs).
    pub has_build_script: bool,
    /// The `rust-version` (MSRV) field, if specified.
    pub rust_version: Option<String>,
    /// Whether the project declares `#![no_std]`.
    pub is_no_std: bool,
    /// The `[package.metadata]` table from Cargo.toml (for config fallback).
    pub package_metadata: serde_json::Value,
    /// Workspace member names and their root directories.
    pub workspace_members: Vec<WorkspaceMember>,
    /// Cargo package IDs selected by workspace `default-members` semantics.
    pub default_member_ids: Vec<String>,
}

/// A workspace member package.
#[derive(Debug, Clone)]
pub struct WorkspaceMember {
    /// Package name.
    pub name: String,
    /// Absolute path to the member's root directory (parent of Cargo.toml).
    pub root_dir: PathBuf,
    /// Cargo's package identity string.
    pub package_id: String,
    /// Cargo target names and kinds.
    pub targets: Vec<String>,
    /// Framework capabilities detected for this member.
    pub frameworks: Vec<Framework>,
    /// Versioned framework evidence isolated to this package.
    pub(crate) framework_capabilities: Vec<FrameworkCapability>,
    /// The member's declared minimum supported Rust version.
    pub rust_version: Option<String>,
}

/// Run cargo metadata and discover project characteristics.
///
/// `manifest_path` should point to the Cargo.toml file.
/// If `offline` is true, passes `--offline` to cargo to prevent network access.
/// Returns `Ok(ProjectInfo)` on success, or an error if cargo metadata fails.
pub fn discover_project(
    manifest_path: &Path,
    offline: bool,
) -> Result<ProjectInfo, crate::error::DiscoveryError> {
    discover_project_for_scan(manifest_path, offline, false)
}

pub(crate) fn discover_project_for_scan(
    manifest_path: &Path,
    offline: bool,
    evaluation_profile: bool,
) -> Result<ProjectInfo, crate::error::DiscoveryError> {
    use crate::error::DiscoveryError;

    let mut cmd = MetadataCommand::new();
    cmd.manifest_path(manifest_path);
    if evaluation_profile {
        cmd.no_deps();
    }
    if offline {
        cmd.other_options(["--offline".to_string()]);
    }
    let metadata = cmd
        .exec()
        .map_err(|source| DiscoveryError::CargoMetadata { source })?;

    let workspace_root = PathBuf::from(metadata.workspace_root.as_std_path());
    let members = metadata.workspace_packages();
    let member_count = members.len();
    let is_workspace = metadata.root_package().is_none() || member_count > 1;
    let default_member_ids: Vec<String> = metadata
        .workspace_default_members
        .iter()
        .map(|package_id| package_id.repr.clone())
        .collect();

    // Use first workspace member as "primary" package
    let primary = members.first().ok_or(DiscoveryError::NoPackages)?;

    let name = primary.name.clone();
    let version = primary.version.to_string();
    let package_id = primary.id.repr.clone();
    let targets = package_targets(primary);
    let edition = primary.edition.as_str().to_string();
    let rust_version = primary
        .rust_version
        .as_ref()
        .map(std::string::ToString::to_string);

    // Detect build script
    let has_build_script = primary
        .targets
        .iter()
        .any(|t| t.kind.contains(&TargetKind::CustomBuild));

    // Detect #![no_std] from primary package's lib.rs or main.rs
    let is_no_std = detect_no_std(primary);

    let package_metadata = primary.metadata.clone();

    // Collect workspace member info
    let workspace_members_info: Vec<WorkspaceMember> = members
        .iter()
        .map(|pkg| {
            let framework_capabilities = framework_capabilities(pkg, &metadata);
            WorkspaceMember {
                name: pkg.name.clone(),
                root_dir: PathBuf::from(pkg.manifest_path.parent().map_or(
                    workspace_root.as_path(),
                    cargo_metadata::camino::Utf8Path::as_std_path,
                )),
                package_id: pkg.id.repr.clone(),
                targets: package_targets(pkg),
                frameworks: active_frameworks(&framework_capabilities),
                framework_capabilities,
                rust_version: pkg
                    .rust_version
                    .as_ref()
                    .map(std::string::ToString::to_string),
            }
        })
        .collect();
    let mut framework_capabilities: Vec<_> = workspace_members_info
        .iter()
        .flat_map(|member| member.framework_capabilities.iter().cloned())
        .collect();
    framework_capabilities.sort_by(|left, right| {
        left.framework
            .to_string()
            .cmp(&right.framework.to_string())
            .then(left.version.cmp(&right.version))
            .then(left.enabled_features.cmp(&right.enabled_features))
    });
    framework_capabilities.dedup();
    let frameworks = active_frameworks(&framework_capabilities);

    Ok(ProjectInfo {
        root_dir: workspace_root,
        name,
        version,
        package_id,
        targets,
        edition,
        frameworks,
        framework_capabilities,
        is_workspace,
        member_count,
        has_build_script,
        rust_version,
        is_no_std,
        package_metadata,
        workspace_members: workspace_members_info,
        default_member_ids,
    })
}

fn framework_capabilities(
    package: &cargo_metadata::Package,
    metadata: &cargo_metadata::Metadata,
) -> Vec<FrameworkCapability> {
    let package_node = metadata
        .resolve
        .as_ref()
        .and_then(|resolve| resolve.nodes.iter().find(|node| node.id == package.id));
    let mut capabilities = Vec::new();

    for dependency in package
        .dependencies
        .iter()
        .filter(|dependency| dependency.kind == DependencyKind::Normal)
    {
        let Some(framework) = framework_for_dependency(&dependency.name) else {
            continue;
        };
        let resolved_dependency = package_node.and_then(|node| {
            node.deps.iter().find(|candidate| {
                metadata
                    .packages
                    .iter()
                    .find(|resolved| resolved.id == candidate.pkg)
                    .is_some_and(|resolved| resolved.name == dependency.name)
            })
        });
        let resolved_package = resolved_dependency.and_then(|node| {
            metadata
                .packages
                .iter()
                .find(|resolved| resolved.id == node.pkg)
        });
        let enabled_features = resolved_dependency
            .and_then(|node| {
                metadata.resolve.as_ref().and_then(|resolve| {
                    resolve
                        .nodes
                        .iter()
                        .find(|resolved| resolved.id == node.pkg)
                })
            })
            .map_or_else(Vec::new, |node| {
                let mut features = node.features.clone();
                features.sort();
                features.dedup();
                features
            });
        let target_contexts = resolved_dependency.map_or_else(Vec::new, |node| {
            let mut targets: Vec<_> = node
                .dep_kinds
                .iter()
                .filter(|kind| kind.kind == DependencyKind::Normal)
                .map(|kind| {
                    kind.target.as_ref().map_or_else(
                        || "all-targets".to_string(),
                        std::string::ToString::to_string,
                    )
                })
                .collect();
            targets.sort();
            targets.dedup();
            targets
        });
        let gate_reason = if dependency.rename.is_some() {
            Some("renamed dependency requires an explicit capability mapping".to_string())
        } else if resolved_dependency.is_none() && dependency.optional {
            Some("optional dependency feature is disabled".to_string())
        } else if resolved_package.is_none() {
            Some("resolved dependency version is unavailable".to_string())
        } else if target_contexts.is_empty() {
            Some("dependency has no active normal target context".to_string())
        } else {
            None
        };
        capabilities.push(FrameworkCapability {
            framework,
            version: resolved_package.map(|resolved| resolved.version.to_string()),
            enabled_features,
            target_contexts,
            active: gate_reason.is_none(),
            gate_reason,
        });
    }
    capabilities.sort_by_key(|capability| capability.framework.to_string());
    capabilities
}

fn active_frameworks(capabilities: &[FrameworkCapability]) -> Vec<Framework> {
    let mut seen = BTreeSet::new();
    capabilities
        .iter()
        .filter(|capability| capability.active)
        .filter_map(|capability| {
            seen.insert(capability.framework.to_string())
                .then_some(capability.framework)
        })
        .collect()
}

fn framework_for_dependency(name: &str) -> Option<Framework> {
    FRAMEWORK_MAP
        .iter()
        .find_map(|(crate_name, framework)| (*crate_name == name).then_some(*framework))
        .or_else(|| name.starts_with("embassy-").then_some(Framework::Embassy))
}

fn package_targets(package: &cargo_metadata::Package) -> Vec<String> {
    let mut targets: Vec<String> = package
        .targets
        .iter()
        .map(|target| format!("{}:{:?}", target.name, target.kind))
        .collect();
    targets.sort();
    targets
}

/// Detect frameworks from dependency names.
#[cfg(test)]
fn detect_frameworks(dep_names: &HashSet<&str>) -> Vec<Framework> {
    let mut frameworks: Vec<Framework> = FRAMEWORK_MAP
        .iter()
        .filter(|(crate_name, _)| dep_names.contains(crate_name))
        .map(|(_, framework)| *framework)
        .collect();

    // Prefix-based detection for embassy-* crates
    if dep_names.iter().any(|name| name.starts_with("embassy-"))
        && !frameworks.contains(&Framework::Embassy)
    {
        frameworks.push(Framework::Embassy);
    }

    frameworks
}

/// Detect `#![no_std]` by scanning the primary source file's first 10 lines.
fn detect_no_std(pkg: &cargo_metadata::Package) -> bool {
    // Find lib or bin target's source path
    let src_path = pkg
        .targets
        .iter()
        .find(|t| {
            t.kind.contains(&TargetKind::Lib)
                || t.kind.contains(&TargetKind::RLib)
                || t.kind.contains(&TargetKind::Bin)
        })
        .map(|t| t.src_path.as_std_path());

    src_path.is_some_and(file_declares_no_std)
}

/// Returns `true` if the file declares `#![no_std]` in its first 10 lines.
fn file_declares_no_std(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let reader = BufReader::new(file);

    for line in reader.lines().take(10) {
        let Ok(line) = line else {
            break;
        };
        let trimmed = line.trim();
        // Check for #![no_std], tolerating internal whitespace like #![ no_std ]
        if trimmed
            .strip_prefix("#![")
            .and_then(|s| s.strip_suffix(']'))
            .is_some_and(|inner| inner.trim() == "no_std")
        {
            return true;
        }
    }
    false
}

/// Validate a directory, discover the project, and load file config.
///
/// Shared bootstrap logic used by both the CLI entry point and the MCP server.
/// Returns the canonicalized directory, project info, and file config.
pub fn bootstrap_project(
    directory: &Path,
    offline: bool,
) -> Result<(PathBuf, ProjectInfo, Option<crate::config::FileConfig>), crate::error::BootstrapError>
{
    bootstrap_project_for_scan(directory, offline, false)
}

pub(crate) fn bootstrap_project_for_scan(
    directory: &Path,
    offline: bool,
    evaluation_profile: bool,
) -> Result<(PathBuf, ProjectInfo, Option<crate::config::FileConfig>), crate::error::BootstrapError>
{
    let target_dir = directory.canonicalize().map_err(|source| {
        crate::error::BootstrapError::InvalidDirectory {
            path: directory.display().to_string(),
            source,
        }
    })?;

    let manifest_root =
        find_manifest_root(&target_dir).ok_or_else(|| crate::error::BootstrapError::NoCargo {
            path: target_dir.clone(),
        })?;
    let cargo_toml = manifest_root.join("Cargo.toml");

    let project_info = discover_project_for_scan(&cargo_toml, offline, evaluation_profile)?;

    let file_config = crate::config::load_file_config(
        &project_info.root_dir,
        Some(&project_info.package_metadata),
    )?;

    Ok((target_dir, project_info, file_config))
}

/// Find the nearest Cargo project boundary without running Cargo or touching the network.
pub(crate) fn find_manifest_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|ancestor| ancestor.join("Cargo.toml").is_file())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_detect_frameworks_tokio() {
        let deps: HashSet<&str> = ["tokio", "serde"].into_iter().collect();
        let frameworks = detect_frameworks(&deps);
        assert!(frameworks.contains(&Framework::Tokio));
        assert!(!frameworks.contains(&Framework::Axum));
    }

    #[test]
    fn test_detect_frameworks_web_stack() {
        let deps: HashSet<&str> = ["tokio", "axum", "sqlx", "serde"].into_iter().collect();
        let frameworks = detect_frameworks(&deps);
        assert!(frameworks.contains(&Framework::Tokio));
        assert!(frameworks.contains(&Framework::Axum));
        assert!(frameworks.contains(&Framework::Sqlx));
    }

    #[test]
    fn test_detect_frameworks_embassy_prefix() {
        let deps: HashSet<&str> = ["embassy-executor", "embassy-time"].into_iter().collect();
        let frameworks = detect_frameworks(&deps);
        assert!(frameworks.contains(&Framework::Embassy));
    }

    #[test]
    fn test_detect_frameworks_cortex_m() {
        let deps: HashSet<&str> = ["cortex-m", "cortex-m-rt"].into_iter().collect();
        let frameworks = detect_frameworks(&deps);
        assert!(frameworks.contains(&Framework::CortexM));
    }

    #[test]
    fn test_detect_frameworks_empty() {
        let deps: HashSet<&str> = HashSet::new();
        let frameworks = detect_frameworks(&deps);
        assert!(frameworks.is_empty());
    }

    #[test]
    fn test_detect_frameworks_no_match() {
        let deps: HashSet<&str> = ["serde", "rand", "log"].into_iter().collect();
        let frameworks = detect_frameworks(&deps);
        assert!(frameworks.is_empty());
    }

    #[test]
    fn test_file_declares_no_std_true() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("lib.rs");
        let mut f = File::create(&file_path).unwrap();
        writeln!(f, "#![no_std]").unwrap();
        writeln!(f, "pub fn hello() {{}}").unwrap();
        drop(f);

        assert!(file_declares_no_std(&file_path));
    }

    #[test]
    fn test_file_declares_no_std_false() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("lib.rs");
        let mut f = File::create(&file_path).unwrap();
        writeln!(f, "use std::io;").unwrap();
        writeln!(f, "pub fn hello() {{}}").unwrap();
        drop(f);

        assert!(!file_declares_no_std(&file_path));
    }

    #[test]
    fn test_file_declares_no_std_with_comments() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("lib.rs");
        let mut f = File::create(&file_path).unwrap();
        writeln!(f, "// Copyright 2026").unwrap();
        writeln!(f, "//! Crate documentation").unwrap();
        writeln!(f, "#![no_std]").unwrap();
        writeln!(f, "pub fn hello() {{}}").unwrap();
        drop(f);

        assert!(file_declares_no_std(&file_path));
    }

    #[test]
    fn test_file_declares_no_std_beyond_line_10() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("lib.rs");
        let mut f = File::create(&file_path).unwrap();
        for i in 1..=11 {
            writeln!(f, "// Line {i}").unwrap();
        }
        writeln!(f, "#![no_std]").unwrap();
        drop(f);

        // no_std is on line 12, beyond the 10-line scan window
        assert!(!file_declares_no_std(&file_path));
    }

    #[test]
    fn test_file_declares_no_std_nonexistent() {
        assert!(!file_declares_no_std(Path::new("/nonexistent/lib.rs")));
    }

    #[test]
    fn test_framework_display() {
        assert_eq!(Framework::Tokio.to_string(), "tokio");
        assert_eq!(Framework::ActixWeb.to_string(), "actix-web");
        assert_eq!(Framework::SeaOrm.to_string(), "sea-orm");
        assert_eq!(Framework::WasmBindgen.to_string(), "wasm-bindgen");
    }

    #[test]
    fn test_file_declares_no_std_with_internal_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("lib.rs");
        let mut f = File::create(&file_path).unwrap();
        writeln!(f, "#![ no_std ]").unwrap();
        drop(f);

        assert!(file_declares_no_std(&file_path));
    }

    #[test]
    fn test_discover_project_on_self() {
        // Run discovery on rust-doctor itself
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let info = discover_project(&manifest, false).unwrap();

        assert_eq!(info.name, "rust-doctor");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(info.edition, "2024");
        assert!(!info.is_workspace);
        assert_eq!(info.member_count, 1);
        assert_eq!(info.default_member_ids, vec![info.package_id.clone()]);
        assert!(!info.has_build_script);
        assert!(!info.is_no_std);
        // rust-doctor depends on tokio (for MCP server)
        assert!(info.frameworks.contains(&Framework::Tokio));
    }

    #[test]
    fn test_discover_project_bad_path() {
        let result = discover_project(Path::new("/nonexistent/Cargo.toml"), false);
        assert!(result.is_err());
    }

    #[test]
    fn evaluation_discovery_does_not_fetch_uncached_dependencies() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("src")).unwrap();
        std::fs::write(
            directory.path().join("Cargo.toml"),
            r#"
                [package]
                name = "offline-evaluation"
                version = "0.1.0"
                edition = "2024"

                [dependencies]
                rust-doctor-intentionally-unavailable = "=999.999.999"
            "#,
        )
        .unwrap();
        std::fs::write(directory.path().join("src/lib.rs"), "pub fn value() {}\n").unwrap();

        let project =
            discover_project_for_scan(&directory.path().join("Cargo.toml"), true, true).unwrap();
        assert_eq!(project.name, "offline-evaluation");
        assert_eq!(project.workspace_members.len(), 1);
    }

    #[test]
    fn bootstrap_discovers_the_nearest_parent_project() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("src/nested")).unwrap();
        std::fs::write(
            directory.path().join("Cargo.toml"),
            "[package]\nname='nested-bootstrap'\nversion='0.1.0'\nedition='2024'\n",
        )
        .unwrap();
        std::fs::write(directory.path().join("src/lib.rs"), "pub fn value() {}\n").unwrap();

        let (requested, project, _) =
            bootstrap_project(&directory.path().join("src/nested"), true).unwrap();
        assert!(requested.ends_with("src/nested"));
        assert_eq!(project.root_dir, directory.path().canonicalize().unwrap());
    }
}
