//! Canonical rule metadata shared by configuration and every renderer.
#![expect(
    clippy::redundant_pub_crate,
    reason = "catalog items are consumed by sibling modules through this private crate module"
)]

use crate::diagnostics::{Category, Severity};
use crate::{clippy, rules};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(test)]
use std::collections::HashSet;
use std::sync::LazyLock;

/// Analyzer responsible for producing a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum AnalyzerKind {
    SynAst,
    Clippy,
    Dependency,
    Project,
    External,
}

/// Confidence consumers should assign to a rule without type information.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Confidence {
    Low,
    Medium,
    High,
}

/// The strongest fix contract offered by a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FixCapability {
    None,
    Guidance,
    RustcSuggestion,
    MachineApplicable,
}

/// Inclusive numeric threshold range supported by a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NumericRange {
    pub(crate) min: u32,
    pub(crate) max: u32,
    pub(crate) default: u32,
}

/// Complete metadata for one canonical rule identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RuleDescriptor {
    pub(crate) canonical_id: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) provider: String,
    pub(crate) category: Category,
    pub(crate) default_severity: Severity,
    pub(crate) tags: Vec<String>,
    pub(crate) analyzer_kind: AnalyzerKind,
    pub(crate) confidence: Confidence,
    pub(crate) default_enabled: bool,
    pub(crate) applicable_frameworks: Vec<String>,
    pub(crate) documentation_url: String,
    pub(crate) supported_threshold: Option<NumericRange>,
    pub(crate) fix_capability: FixCapability,
    pub(crate) description: String,
    pub(crate) fix_guidance: String,
}

impl RuleDescriptor {
    fn custom(rule: &dyn rules::CustomRule) -> Self {
        let mut tags = vec![
            "heuristic".to_string(),
            category_tag(&rule.category()).to_string(),
        ];
        tags.sort();
        tags.dedup();
        Self {
            canonical_id: rule.name().to_string(),
            aliases: Vec::new(),
            provider: "rust-doctor".to_string(),
            category: rule.category(),
            default_severity: rule.severity(),
            tags,
            analyzer_kind: AnalyzerKind::SynAst,
            confidence: rule.confidence(),
            default_enabled: rule.default_enabled(),
            applicable_frameworks: rule
                .applicable_frameworks()
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            documentation_url: format!("https://rust-doctor.vercel.app/rules/{}", rule.name()),
            supported_threshold: rule.supported_threshold(),
            fix_capability: FixCapability::Guidance,
            description: rule.description().to_string(),
            fix_guidance: rule.fix_hint().to_string(),
        }
    }

    fn clippy(entry: &clippy::LintEntry) -> Self {
        let id = format!("clippy::{}", entry.name);
        Self {
            canonical_id: id,
            aliases: vec![entry.name.to_string()],
            provider: "clippy".to_string(),
            category: entry.category.clone(),
            default_severity: entry.severity,
            tags: vec![
                "type-aware".to_string(),
                category_tag(&entry.category).to_string(),
            ],
            analyzer_kind: AnalyzerKind::Clippy,
            confidence: Confidence::High,
            default_enabled: true,
            applicable_frameworks: Vec::new(),
            documentation_url: format!(
                "https://rust-lang.github.io/rust-clippy/master/index.html#{}",
                entry.name
            ),
            supported_threshold: None,
            fix_capability: FixCapability::RustcSuggestion,
            description: format!("Clippy `{}` diagnostic", entry.name),
            fix_guidance: "Apply the rustc suggestion when its applicability permits it."
                .to_string(),
        }
    }

    fn external(
        id: &str,
        provider: &str,
        category: &Category,
        severity: Severity,
        analyzer_kind: AnalyzerKind,
    ) -> Self {
        Self {
            canonical_id: id.to_string(),
            aliases: Vec::new(),
            provider: provider.to_string(),
            category: category.clone(),
            default_severity: severity,
            tags: vec![category_tag(category).to_string(), "external".to_string()],
            analyzer_kind,
            confidence: Confidence::High,
            default_enabled: true,
            applicable_frameworks: Vec::new(),
            documentation_url: "https://rust-doctor.vercel.app/rules/external".to_string(),
            supported_threshold: None,
            fix_capability: FixCapability::Guidance,
            description: format!("Finding reported by {provider}"),
            fix_guidance: "Follow the originating analyzer guidance.".to_string(),
        }
    }
}

/// A validated index over canonical IDs and compatibility aliases.
#[derive(Debug)]
pub(crate) struct RuleCatalog {
    descriptors: Vec<RuleDescriptor>,
    lookup: HashMap<String, usize>,
}

impl RuleCatalog {
    pub(crate) fn build(mut descriptors: Vec<RuleDescriptor>) -> Result<Self, CatalogError> {
        descriptors.sort_by(|left, right| left.canonical_id.cmp(&right.canonical_id));
        let mut lookup = HashMap::new();
        for (index, descriptor) in descriptors.iter().enumerate() {
            register_key(&mut lookup, &descriptor.canonical_id, index)?;
            for alias in &descriptor.aliases {
                register_key(&mut lookup, alias, index)?;
            }
        }
        Ok(Self {
            descriptors,
            lookup,
        })
    }

    pub(crate) fn descriptors(&self) -> &[RuleDescriptor] {
        &self.descriptors
    }

    pub(crate) fn exact(&self, id_or_alias: &str) -> Option<&RuleDescriptor> {
        self.lookup
            .get(id_or_alias)
            .and_then(|index| self.descriptors.get(*index))
    }

    /// Resolve a raw adapter rule. Unknown dynamic codes receive a namespaced
    /// descriptor instead of being misrepresented as a built-in rule.
    pub(crate) fn resolve(
        &self,
        raw_rule: &str,
        category: &Category,
        severity: Severity,
    ) -> ResolvedDescriptor<'_> {
        if let Some(descriptor) = self.exact(raw_rule) {
            return ResolvedDescriptor::Exact(descriptor);
        }

        let (provider, analyzer, url) = external_namespace(raw_rule);
        let mut descriptor =
            RuleDescriptor::external(raw_rule, provider, category, severity, analyzer);
        descriptor.documentation_url = url;
        ResolvedDescriptor::Fallback(Box::new(descriptor))
    }

    #[cfg(test)]
    fn custom_count(&self) -> usize {
        self.descriptors
            .iter()
            .filter(|descriptor| descriptor.analyzer_kind == AnalyzerKind::SynAst)
            .count()
    }

    #[cfg(test)]
    fn clippy_count(&self) -> usize {
        self.descriptors
            .iter()
            .filter(|descriptor| descriptor.analyzer_kind == AnalyzerKind::Clippy)
            .count()
    }
}

pub(crate) enum ResolvedDescriptor<'a> {
    Exact(&'a RuleDescriptor),
    Fallback(Box<RuleDescriptor>),
}

impl ResolvedDescriptor<'_> {
    pub(crate) fn as_descriptor(&self) -> &RuleDescriptor {
        match self {
            Self::Exact(descriptor) => descriptor,
            Self::Fallback(descriptor) => descriptor.as_ref(),
        }
    }

    pub(crate) const fn is_fallback(&self) -> bool {
        matches!(self, Self::Fallback(_))
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum CatalogError {
    #[error("duplicate canonical rule ID or alias: {0}")]
    DuplicateId(String),
}

static BUILT_IN_CATALOG: LazyLock<Result<RuleCatalog, CatalogError>> = LazyLock::new(|| {
    let mut descriptors: Vec<RuleDescriptor> = rules::all_custom_rules()
        .iter()
        .map(|rule| RuleDescriptor::custom(rule.as_ref()))
        .collect();
    descriptors.extend(clippy::LINT_REGISTRY.iter().map(RuleDescriptor::clippy));
    descriptors.extend(external_descriptors());
    RuleCatalog::build(descriptors)
});

pub(crate) fn built_in_catalog() -> Result<&'static RuleCatalog, CatalogError> {
    BUILT_IN_CATALOG.as_ref().map_err(Clone::clone)
}

fn register_key(
    lookup: &mut HashMap<String, usize>,
    key: &str,
    index: usize,
) -> Result<(), CatalogError> {
    if lookup.insert(key.to_string(), index).is_some() {
        return Err(CatalogError::DuplicateId(key.to_string()));
    }
    Ok(())
}

const fn category_tag(category: &Category) -> &'static str {
    match category {
        Category::ErrorHandling => "error-handling",
        Category::Performance => "performance",
        Category::Security => "security",
        Category::Correctness => "correctness",
        Category::Architecture => "architecture",
        Category::Dependencies => "dependencies",
        Category::Async => "async",
        Category::Framework => "framework",
        Category::Cargo => "cargo",
        Category::Style => "style",
    }
}

fn external_namespace(rule: &str) -> (&'static str, AnalyzerKind, String) {
    if rule.starts_with("RUSTSEC-") {
        return (
            "rustsec",
            AnalyzerKind::Dependency,
            format!("https://rustsec.org/advisories/{rule}.html"),
        );
    }
    if rule.starts_with("deny::") {
        return (
            "cargo-deny",
            AnalyzerKind::Dependency,
            "https://embarkstudios.github.io/cargo-deny/checks/index.html".to_string(),
        );
    }
    (
        "external",
        AnalyzerKind::External,
        "https://rust-doctor.vercel.app/rules/external".to_string(),
    )
}

fn external_descriptors() -> Vec<RuleDescriptor> {
    [
        (
            "unused-dependency",
            "cargo-machete",
            Category::Dependencies,
            Severity::Warning,
            AnalyzerKind::Dependency,
        ),
        (
            "unsafe-dependency",
            "cargo-geiger",
            Category::Security,
            Severity::Warning,
            AnalyzerKind::Dependency,
        ),
        (
            "semver-violation",
            "cargo-semver-checks",
            Category::Correctness,
            Severity::Error,
            AnalyzerKind::Dependency,
        ),
        (
            "missing-msrv",
            "rust-doctor",
            Category::Cargo,
            Severity::Warning,
            AnalyzerKind::Project,
        ),
        (
            "msrv-outdated",
            "rust-doctor",
            Category::Cargo,
            Severity::Warning,
            AnalyzerKind::Project,
        ),
        (
            "msrv-incompatible",
            "rust-doctor",
            Category::Cargo,
            Severity::Error,
            AnalyzerKind::Project,
        ),
        (
            "low-coverage",
            "cargo-llvm-cov",
            Category::Correctness,
            Severity::Warning,
            AnalyzerKind::Project,
        ),
        (
            "uncovered-file",
            "cargo-llvm-cov",
            Category::Correctness,
            Severity::Warning,
            AnalyzerKind::Project,
        ),
        (
            "compiler-error",
            "rustc",
            Category::Correctness,
            Severity::Error,
            AnalyzerKind::Project,
        ),
        (
            "compiler-ice",
            "rustc",
            Category::Correctness,
            Severity::Error,
            AnalyzerKind::Project,
        ),
        (
            "unknown-rustc-level",
            "rustc",
            Category::Correctness,
            Severity::Info,
            AnalyzerKind::Project,
        ),
        (
            "skipped-pass",
            "rust-doctor",
            Category::Cargo,
            Severity::Info,
            AnalyzerKind::Project,
        ),
    ]
    .into_iter()
    .map(|(id, provider, category, severity, analyzer)| {
        RuleDescriptor::external(id, provider, &category, severity, analyzer)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(id: &str, aliases: &[&str]) -> RuleDescriptor {
        RuleDescriptor {
            canonical_id: id.to_string(),
            aliases: aliases.iter().map(|value| (*value).to_string()).collect(),
            provider: "test".to_string(),
            category: Category::Style,
            default_severity: Severity::Warning,
            tags: Vec::new(),
            analyzer_kind: AnalyzerKind::External,
            confidence: Confidence::High,
            default_enabled: true,
            applicable_frameworks: Vec::new(),
            documentation_url: String::new(),
            supported_threshold: None,
            fix_capability: FixCapability::None,
            description: String::new(),
            fix_guidance: String::new(),
        }
    }

    #[test]
    fn built_in_catalog_covers_every_explicit_rule() {
        let catalog = built_in_catalog().unwrap();
        assert_eq!(catalog.custom_count(), 19);
        assert_eq!(catalog.clippy_count(), 74);
        for rule in rules::all_custom_rules() {
            assert!(catalog.exact(rule.name()).is_some(), "{}", rule.name());
        }
        for lint in clippy::known_lint_names() {
            assert!(catalog.exact(lint).is_some(), "{lint}");
        }
    }

    #[test]
    fn duplicate_ids_and_aliases_fail_deterministically() {
        let duplicate_id =
            RuleCatalog::build(vec![descriptor("same", &[]), descriptor("same", &[])]);
        assert!(matches!(duplicate_id, Err(CatalogError::DuplicateId(id)) if id == "same"));

        let duplicate_alias = RuleCatalog::build(vec![
            descriptor("first", &["shared"]),
            descriptor("second", &["shared"]),
        ]);
        assert!(matches!(duplicate_alias, Err(CatalogError::DuplicateId(id)) if id == "shared"));
    }

    #[test]
    fn dynamic_rustsec_codes_use_documented_namespace_fallback() {
        let catalog = built_in_catalog().unwrap();
        let resolved = catalog.resolve("RUSTSEC-2099-9999", &Category::Security, Severity::Error);
        let descriptor = resolved.as_descriptor();
        assert!(resolved.is_fallback());
        assert_eq!(descriptor.provider, "rustsec");
        assert!(descriptor.documentation_url.contains("RUSTSEC-2099-9999"));
    }

    #[test]
    fn catalog_ids_and_aliases_are_unique() {
        let catalog = built_in_catalog().unwrap();
        let mut seen = HashSet::new();
        for descriptor in catalog.descriptors() {
            assert!(seen.insert(descriptor.canonical_id.as_str()));
            for alias in &descriptor.aliases {
                assert!(seen.insert(alias.as_str()));
            }
        }
    }
}
