//! Package, target, and source context resolved before a custom rule runs.
//!
//! `syn` sees one file at a time. Everything a heuristic needs to decide whether
//! its assumptions hold — which package owns the file, what the crate is for,
//! which edition and MSRV apply, which features and frameworks are active — comes
//! from Cargo metadata resolved once per member and handed to the rule here.
//!
//! When that evidence is missing or ambiguous the rule does not guess: it
//! abstains and the scan records an [`AbstentionReceipt`] so completeness
//! degrades instead of a package silently inheriting another package's context
//! (US-006).

pub use crate::diagnostics::AbstentionReceipt;

use crate::diagnostics::SourceSurface;
use crate::discovery::CargoTargetContext;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::LazyLock;

/// Merge per-file abstentions into one stable aggregate per (rule, reason).
#[must_use]
pub fn aggregate_abstentions(events: Vec<(String, String)>) -> Vec<AbstentionReceipt> {
    let mut totals: std::collections::BTreeMap<(String, String), usize> =
        std::collections::BTreeMap::new();
    for (rule, reason) in events {
        *totals.entry((rule, reason)).or_default() += 1;
    }
    totals
        .into_iter()
        .map(|((rule, reason), count)| AbstentionReceipt {
            rule,
            reason,
            count,
        })
        .collect()
}

/// What a Cargo package is for. A rule that only holds in a published library
/// uses this instead of guessing from the file path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CrateRole {
    Library,
    Binary,
    ProcMacro,
    /// The package builds both a library and one or more binaries.
    Mixed,
    /// Cargo metadata did not resolve a target set for this package.
    Unknown,
}

impl CrateRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Binary => "binary",
            Self::ProcMacro => "proc-macro",
            Self::Mixed => "mixed",
            Self::Unknown => "unknown",
        }
    }

    /// Derive the role from the Cargo-owned target set of one package.
    fn from_targets(targets: &[CargoTargetContext]) -> Self {
        if targets.iter().any(|target| target.is_proc_macro) {
            return Self::ProcMacro;
        }
        let has_library = targets
            .iter()
            .any(|target| target.source_surface == SourceSurface::Library);
        let has_binary = targets
            .iter()
            .any(|target| target.source_surface == SourceSurface::Binary);
        match (has_library, has_binary) {
            (true, true) => Self::Mixed,
            (true, false) => Self::Library,
            (false, true) => Self::Binary,
            (false, false) => Self::Unknown,
        }
    }
}

/// Whether the analyzed text was written by a human. Generated and expanded
/// sources carry no actionable span, so rules exclude them before traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceOrigin {
    Authored,
    Generated,
    MacroExpansion,
}

impl SourceOrigin {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authored => "authored",
            Self::Generated => "generated",
            Self::MacroExpansion => "macro-expansion",
        }
    }
}

/// One piece of context a rule declares it cannot decide without.
///
/// The rule engine checks every requirement before traversal. An unavailable or
/// ambiguous requirement produces an abstention receipt, never a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextRequirement {
    /// Cargo metadata resolved this package's identity and targets.
    PackageMetadata,
    /// The file's source surface is known (not `Unknown`).
    SourceSurface,
    /// The package declares an edition.
    Edition,
    /// The package declares `rust-version`.
    DeclaredMsrv,
    /// The resolved feature set for this package is known.
    FeatureProfile,
    /// Framework evidence was resolved from the dependency graph.
    Frameworks,
    /// Dependency capabilities (versioned framework edges) were resolved.
    DependencyCapabilities,
    /// The analyzed target triple and its cfg profile are known.
    CfgProfile,
}

impl ContextRequirement {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PackageMetadata => "package-metadata",
            Self::SourceSurface => "source-surface",
            Self::Edition => "edition",
            Self::DeclaredMsrv => "declared-msrv",
            Self::FeatureProfile => "feature-profile",
            Self::Frameworks => "frameworks",
            Self::DependencyCapabilities => "dependency-capabilities",
            Self::CfgProfile => "cfg-profile",
        }
    }
}

/// The Cargo-resolved identity of one package, before target ownership is
/// folded in. Grouped so the context constructor takes one evidence bundle
/// rather than a long positional list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageIdentity {
    pub package_id: String,
    pub edition: Option<String>,
    pub declared_msrv: Option<String>,
    pub enabled_features: Vec<String>,
    pub frameworks: Vec<String>,
    pub dependency_capabilities: Vec<String>,
    pub cfg_profile: Option<String>,
}

/// Package-scoped evidence resolved once per workspace member.
///
/// Two members with different metadata or features never share one instance:
/// the scan builds a context per member so a rule sees member-specific evidence
/// (US-006 AC-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageContext {
    /// Cargo's stable package identity, empty when metadata did not resolve.
    pub package_id: String,
    pub crate_role: CrateRole,
    pub edition: Option<String>,
    pub declared_msrv: Option<String>,
    /// Features Cargo resolved as enabled for this package.
    pub enabled_features: Vec<String>,
    /// Frameworks detected in this package's dependency graph.
    pub frameworks: Vec<String>,
    /// Versioned framework edges: `name@version`.
    pub dependency_capabilities: Vec<String>,
    /// Analyzed target triple, `None` when target discovery failed.
    pub cfg_profile: Option<String>,
    /// False when Cargo metadata could not be resolved or target ownership is
    /// unresolved. Rules with required context abstain rather than guess.
    pub metadata_complete: bool,
}

impl PackageContext {
    /// The context used when no Cargo metadata is available: everything a rule
    /// might require is explicitly unresolved.
    #[must_use]
    pub const fn unresolved() -> Self {
        Self {
            package_id: String::new(),
            crate_role: CrateRole::Unknown,
            edition: None,
            declared_msrv: None,
            enabled_features: Vec::new(),
            frameworks: Vec::new(),
            dependency_capabilities: Vec::new(),
            cfg_profile: None,
            metadata_complete: false,
        }
    }

    /// Build the package context for one workspace member.
    #[must_use]
    pub fn new(identity: PackageIdentity, targets: &[CargoTargetContext]) -> Self {
        let crate_role = CrateRole::from_targets(targets);
        Self {
            metadata_complete: !identity.package_id.is_empty()
                && crate_role != CrateRole::Unknown
                && identity.edition.is_some(),
            package_id: identity.package_id,
            crate_role,
            edition: identity.edition,
            declared_msrv: identity.declared_msrv,
            enabled_features: identity.enabled_features,
            frameworks: identity.frameworks,
            dependency_capabilities: identity.dependency_capabilities,
            cfg_profile: identity.cfg_profile,
        }
    }

    /// Reason this package cannot satisfy `requirement`, if any.
    fn missing(&self, requirement: ContextRequirement) -> Option<&'static str> {
        let unavailable = match requirement {
            // Handled against the file's own surface, not the package.
            ContextRequirement::SourceSurface => false,
            ContextRequirement::Edition => self.edition.is_none(),
            ContextRequirement::DeclaredMsrv => self.declared_msrv.is_none(),
            ContextRequirement::CfgProfile => self.cfg_profile.is_none(),
            ContextRequirement::DependencyCapabilities => {
                !self.metadata_complete || self.dependency_capabilities.is_empty()
            }
            // Identity, feature resolution, and framework detection all come
            // from the same Cargo metadata response.
            ContextRequirement::PackageMetadata
            | ContextRequirement::FeatureProfile
            | ContextRequirement::Frameworks => !self.metadata_complete,
        };
        unavailable.then(|| requirement.as_str())
    }
}

/// The shared unresolved context, used by editor analysis and by tests that
/// build a rule engine without Cargo metadata.
pub static UNRESOLVED_PACKAGE: LazyLock<PackageContext> = LazyLock::new(PackageContext::unresolved);

/// Exact source context handed to one rule for one file.
///
/// `Copy` because the package evidence is borrowed: the engine resolves it once
/// per member and every file in that member shares the same allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleContext<'a> {
    pub source_surface: SourceSurface,
    pub origin: SourceOrigin,
    pub package: &'a PackageContext,
}

impl<'a> RuleContext<'a> {
    /// Resolve the context for `path` from Cargo target ownership.
    #[must_use]
    pub fn for_path(
        path: &Path,
        cargo_targets: &[CargoTargetContext],
        package: &'a PackageContext,
    ) -> Self {
        let source_surface = if cargo_targets.is_empty() {
            crate::config::classify_source_surface(&path.to_string_lossy(), false)
        } else {
            let owned = crate::discovery::source_surface_for_path(path, cargo_targets);
            // Cargo does not own generated output, so a file it cannot claim
            // falls back to the path classifier before being called unknown.
            if owned == SourceSurface::Unknown {
                crate::config::classify_source_surface(&path.to_string_lossy(), false)
            } else {
                owned
            }
        };
        Self {
            source_surface,
            origin: origin_for(path, source_surface),
            package,
        }
    }

    /// Context for a file with no Cargo metadata behind it.
    #[must_use]
    pub fn unresolved_for_path(path: &Path) -> RuleContext<'static> {
        let source_surface = crate::config::classify_source_surface(&path.to_string_lossy(), false);
        RuleContext {
            source_surface,
            origin: origin_for(path, source_surface),
            package: &UNRESOLVED_PACKAGE,
        }
    }

    /// First requirement this context cannot satisfy.
    #[must_use]
    pub fn first_missing(&self, required: &[ContextRequirement]) -> Option<&'static str> {
        required.iter().find_map(|requirement| {
            if *requirement == ContextRequirement::SourceSurface
                && self.source_surface == SourceSurface::Unknown
            {
                return Some(ContextRequirement::SourceSurface.as_str());
            }
            self.package.missing(*requirement)
        })
    }
}

fn origin_for(path: &Path, source_surface: SourceSurface) -> SourceOrigin {
    match source_surface {
        SourceSurface::Generated => SourceOrigin::Generated,
        SourceSurface::MacroExpansion => SourceOrigin::MacroExpansion,
        _ => {
            let normalized = path.to_string_lossy().replace('\\', "/");
            if normalized.contains("/target/")
                || normalized.starts_with("target/")
                || normalized.contains("/generated/")
                || normalized.contains("/out/")
            {
                SourceOrigin::Generated
            } else {
                SourceOrigin::Authored
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str, path: &str, surface: SourceSurface) -> CargoTargetContext {
        CargoTargetContext {
            name: name.to_string(),
            src_path: std::path::PathBuf::from(path),
            source_surface: surface,
            is_proc_macro: false,
        }
    }

    #[test]
    fn crate_role_follows_the_cargo_target_set() {
        assert_eq!(
            CrateRole::from_targets(&[target("api", "src/lib.rs", SourceSurface::Library)]),
            CrateRole::Library
        );
        assert_eq!(
            CrateRole::from_targets(&[target("cli", "src/main.rs", SourceSurface::Binary)]),
            CrateRole::Binary
        );
        assert_eq!(
            CrateRole::from_targets(&[
                target("api", "src/lib.rs", SourceSurface::Library),
                target("cli", "src/main.rs", SourceSurface::Binary),
            ]),
            CrateRole::Mixed
        );
        assert_eq!(CrateRole::from_targets(&[]), CrateRole::Unknown);
    }

    #[test]
    fn unresolved_metadata_makes_every_package_requirement_missing() {
        let package = PackageContext::unresolved();
        let context = RuleContext {
            source_surface: SourceSurface::Library,
            origin: SourceOrigin::Authored,
            package: &package,
        };
        assert_eq!(
            context.first_missing(&[ContextRequirement::PackageMetadata]),
            Some("package-metadata")
        );
        assert_eq!(
            context.first_missing(&[ContextRequirement::Edition]),
            Some("edition")
        );
        assert_eq!(
            context.first_missing(&[ContextRequirement::CfgProfile]),
            Some("cfg-profile")
        );
    }

    #[test]
    fn a_resolved_member_satisfies_its_declared_requirements() {
        let package = PackageContext::new(
            PackageIdentity {
                package_id: "demo 0.1.0".to_string(),
                edition: Some("2024".to_string()),
                declared_msrv: Some("1.97".to_string()),
                enabled_features: vec!["default".to_string()],
                frameworks: vec!["tokio".to_string()],
                dependency_capabilities: vec!["tokio@1.40.0".to_string()],
                cfg_profile: Some("x86_64-unknown-linux-gnu".to_string()),
            },
            &[target("api", "src/lib.rs", SourceSurface::Library)],
        );
        assert!(package.metadata_complete);
        assert_eq!(package.crate_role, CrateRole::Library);
        let context = RuleContext {
            source_surface: SourceSurface::Library,
            origin: SourceOrigin::Authored,
            package: &package,
        };
        assert!(
            context
                .first_missing(&[
                    ContextRequirement::PackageMetadata,
                    ContextRequirement::SourceSurface,
                    ContextRequirement::Edition,
                    ContextRequirement::DeclaredMsrv,
                    ContextRequirement::FeatureProfile,
                    ContextRequirement::Frameworks,
                    ContextRequirement::DependencyCapabilities,
                    ContextRequirement::CfgProfile,
                ])
                .is_none()
        );
    }

    #[test]
    fn an_unknown_surface_is_a_missing_requirement() {
        let package = PackageContext::unresolved();
        let context = RuleContext {
            source_surface: SourceSurface::Unknown,
            origin: SourceOrigin::Authored,
            package: &package,
        };
        assert_eq!(
            context.first_missing(&[ContextRequirement::SourceSurface]),
            Some("source-surface")
        );
    }

    #[test]
    fn generated_output_is_never_reported_as_authored() {
        let context = RuleContext::unresolved_for_path(Path::new("target/debug/build/gen.rs"));
        assert_eq!(context.origin, SourceOrigin::Generated);
        let authored = RuleContext::unresolved_for_path(Path::new("src/lib.rs"));
        assert_eq!(authored.origin, SourceOrigin::Authored);
    }

    #[test]
    fn abstentions_aggregate_by_rule_and_reason() {
        let receipts = aggregate_abstentions(vec![
            (
                "panic-in-library".to_string(),
                "package-metadata".to_string(),
            ),
            (
                "panic-in-library".to_string(),
                "package-metadata".to_string(),
            ),
            (
                "panic-in-library".to_string(),
                "unsupported-source-surface:test".to_string(),
            ),
        ]);
        assert_eq!(receipts.len(), 2);
        assert_eq!(receipts[0].reason, "package-metadata");
        assert_eq!(receipts[0].count, 2);
        assert_eq!(receipts[1].count, 1);
    }
}
