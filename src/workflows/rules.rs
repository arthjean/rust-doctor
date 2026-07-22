//! Read-only rule discovery and transactional `rust-doctor.toml` policy edits.
//!
//! This module deliberately has no discovery or analyzer dependency. Listing,
//! explanation, and mutation preparation only inspect the in-process catalog
//! and an optional configuration file.

use crate::catalog::{
    AnalyzerKind, Confidence, FixCapability, NumericRange, RuleCatalog, RuleDescriptor,
    built_in_catalog,
};
use crate::config::{FileConfig, PolicyConfig, ResolvedConfig, RuleConfig, VisibilitySurface};
use crate::diagnostics::{Category, Severity};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt::Write as FmtWrite;
use std::fs::{self, Permissions};
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use toml_edit::{Decor, DocumentMut, Item, RawString, Table, TableLike, value};

const CONFIG_FILE_NAME: &str = "rust-doctor.toml";
const VALID_LEVELS: [&str; 4] = ["off", "info", "warning", "error"];
const KNOWN_CONFIG_KEYS: [&str; 12] = [
    "ignore",
    "lint",
    "dependencies",
    "verbose",
    "diff",
    "fail_on",
    "rules",
    "categories",
    "tags",
    "path_overrides",
    "rules_config",
    "score",
];

/// Filters accepted by `rules list`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct RuleListFilter {
    pub(super) category: Option<String>,
    pub(super) tag: Option<String>,
    pub(super) framework: Option<String>,
    pub(super) analyzer: Option<String>,
    pub(super) configured_only: bool,
}

/// One policy source in ascending precedence order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PolicyProvenance {
    kind: String,
    selector: String,
    fields: Vec<String>,
}

/// Effective policy projected into stable, renderer-friendly values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct EffectiveRulePolicy {
    level: String,
    active: bool,
    threshold: Option<u32>,
    surfaces: Vec<String>,
    configuration_source: PolicyProvenance,
    provenance: Vec<PolicyProvenance>,
}

/// One deterministic row returned by `rules list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct RuleListEntry {
    canonical_id: String,
    category: String,
    tags: Vec<String>,
    analyzer: String,
    confidence: String,
    default_enabled: bool,
    applicable_frameworks: Vec<String>,
    effective_policy: EffectiveRulePolicy,
}

/// Serializable threshold contract used by rule explanations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct ThresholdRange {
    min: u32,
    max: u32,
    default: u32,
}

impl From<NumericRange> for ThresholdRange {
    fn from(value: NumericRange) -> Self {
        Self {
            min: value.min,
            max: value.max,
            default: value.default,
        }
    }
}

/// Complete output for `rules explain`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct RuleExplanation {
    canonical_id: String,
    aliases: Vec<String>,
    provider: String,
    category: String,
    default_severity: Severity,
    tags: Vec<String>,
    analyzer: String,
    confidence: String,
    default_enabled: bool,
    applicable_frameworks: Vec<String>,
    supported_threshold: Option<ThresholdRange>,
    fix_capability: String,
    rationale: String,
    evidence_model: String,
    limitations: Vec<String>,
    fix_guidance: String,
    official_documentation: Vec<String>,
    effective_policy: EffectiveRulePolicy,
    namespace_fallback: bool,
}

/// Up to five deterministic nearest values carried by typed usage errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NearestSuggestions(Vec<String>);

impl NearestSuggestions {
    fn for_value<'a>(value: &str, candidates: impl IntoIterator<Item = &'a str>) -> Self {
        let mut ranked: Vec<_> = candidates
            .into_iter()
            .map(|candidate| {
                (
                    levenshtein(value, candidate),
                    candidate.to_ascii_lowercase(),
                    candidate.to_string(),
                )
            })
            .collect();
        ranked.sort();
        ranked.dedup_by(|left, right| left.2 == right.2);
        Self(
            ranked
                .into_iter()
                .take(5)
                .map(|(_, _, candidate)| candidate)
                .collect(),
        )
    }

    #[cfg(test)]
    fn as_slice(&self) -> &[String] {
        &self.0
    }
}

impl std::fmt::Display for NearestSuggestions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            formatter.write_str("no close valid values")
        } else {
            write!(formatter, "nearest valid values: {}", self.0.join(", "))
        }
    }
}

/// Local workflow failures always identify a concrete recovery action.
#[derive(Debug, thiserror::Error)]
pub(super) enum RuleWorkflowError {
    #[error(
        "rule catalog validation failed: {message}. Recovery: repair duplicate catalog IDs before retrying"
    )]
    Catalog { message: String },

    #[error(
        "unknown rule '{rule}' ({suggestions}). Recovery: choose a canonical rule ID from `rust-doctor rules list`"
    )]
    UnknownRule {
        rule: String,
        suggestions: NearestSuggestions,
    },

    #[error(
        "unknown {filter} filter '{value}' ({suggestions}). Recovery: replace it with one of the suggested catalog values"
    )]
    UnknownFilter {
        filter: &'static str,
        value: String,
        suggestions: NearestSuggestions,
    },

    #[error(
        "invalid rule level '{level}' ({suggestions}). Recovery: use off, info, warning, or error"
    )]
    InvalidLevel {
        level: String,
        suggestions: NearestSuggestions,
    },

    #[error(
        "rule '{rule}' does not support a numeric threshold. Recovery: omit --threshold or select a rule that declares a threshold range"
    )]
    UnsupportedThreshold { rule: String },

    #[error(
        "threshold {value} for rule '{rule}' is outside {min}..={max}. Recovery: retry with a value inside the supported range"
    )]
    ThresholdOutOfRange {
        rule: String,
        value: u32,
        min: u32,
        max: u32,
    },

    #[error(
        "failed to parse '{}': {message}. Recovery: fix the TOML syntax; the original file was not changed",
        path.display()
    )]
    MalformedToml { path: PathBuf, message: String },

    #[error(
        "invalid configuration in '{}': {message}. Recovery: correct the reported policy value; the original file was not changed",
        path.display()
    )]
    InvalidConfiguration { path: PathBuf, message: String },

    #[error(
        "cannot edit section '{section}' in '{}': expected a TOML table. Recovery: convert the section to a table and retry",
        path.display()
    )]
    InvalidDocumentShape { path: PathBuf, section: String },

    #[error(
        "configuration path '{}' is not a regular file. Recovery: replace the symlink or special file with a regular rust-doctor.toml",
        path.display()
    )]
    UnsupportedFileType { path: PathBuf },

    #[error(
        "failed to read '{}': {source}. Recovery: restore read access and retry; no file content was changed",
        path.display()
    )]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "'{}' changed after it was read. Recovery: review the concurrent edit, then rerun the command",
        path.display()
    )]
    ConcurrentModification { path: PathBuf },

    #[error(
        "atomic configuration write failed during {operation} for '{}': {source}. Recovery: verify directory permissions and free space, then retry; the previous destination was not partially written",
        path.display()
    )]
    AtomicWrite {
        path: PathBuf,
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "failed to serialize rule output as JSON: {source}. Recovery: report this deterministic catalog serialization failure"
    )]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

/// Return sorted catalog rows after validating every requested filter.
pub(super) fn list_rules(
    resolved: &ResolvedConfig,
    filter: &RuleListFilter,
) -> Result<Vec<RuleListEntry>, RuleWorkflowError> {
    let catalog = catalog()?;
    validate_list_filters(catalog, filter)?;

    Ok(catalog
        .descriptors()
        .iter()
        .filter(|descriptor| matches_filter(descriptor, resolved, filter))
        .map(|descriptor| RuleListEntry {
            canonical_id: descriptor.canonical_id.clone(),
            category: category_key(&descriptor.category).to_string(),
            tags: descriptor.tags.clone(),
            analyzer: analyzer_key(descriptor.analyzer_kind).to_string(),
            confidence: confidence_key(descriptor.confidence).to_string(),
            default_enabled: descriptor.default_enabled,
            applicable_frameworks: descriptor.applicable_frameworks.clone(),
            effective_policy: effective_policy(resolved, descriptor),
        })
        .collect())
}

/// Explain an exact rule, compatibility alias, or documented dynamic namespace.
pub(super) fn explain_rule(
    resolved: &ResolvedConfig,
    rule: &str,
) -> Result<RuleExplanation, RuleWorkflowError> {
    let catalog = catalog()?;
    if let Some(descriptor) = catalog.exact(rule) {
        return Ok(explanation_for(resolved, descriptor, false));
    }
    if let Some(descriptor) = namespace_descriptor(rule) {
        return Ok(explanation_for(resolved, &descriptor, true));
    }

    Err(unknown_rule(catalog, rule))
}

/// Stable pretty JSON for `rules list --json`.
pub(super) fn render_rule_list_json(
    entries: &[RuleListEntry],
) -> Result<String, RuleWorkflowError> {
    serde_json::to_string_pretty(entries).map_err(|source| RuleWorkflowError::Json { source })
}

/// Stable pretty JSON for `rules explain --json`.
pub(super) fn render_rule_explanation_json(
    explanation: &RuleExplanation,
) -> Result<String, RuleWorkflowError> {
    serde_json::to_string_pretty(explanation).map_err(|source| RuleWorkflowError::Json { source })
}

/// Deterministic human-readable list output.
pub(super) fn render_rule_list(entries: &[RuleListEntry]) -> String {
    let id_width = entries
        .iter()
        .map(|entry| entry.canonical_id.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let category_width = entries
        .iter()
        .map(|entry| entry.category.len())
        .max()
        .unwrap_or(8)
        .max(8);
    let analyzer_width = entries
        .iter()
        .map(|entry| entry.analyzer.len())
        .max()
        .unwrap_or(8)
        .max(8);
    let mut output = String::new();
    let _ = writeln!(
        output,
        "{:<id_width$}  {:<7}  {:<category_width$}  {:<analyzer_width$}  {:<10}  {:<7}  {:<24}  TAGS",
        "RULE", "LEVEL", "CATEGORY", "ANALYZER", "CONFIDENCE", "ACTIVE", "SOURCE"
    );
    for entry in entries {
        let source = source_label(&entry.effective_policy.configuration_source);
        let _ = writeln!(
            output,
            "{:<id_width$}  {:<7}  {:<category_width$}  {:<analyzer_width$}  {:<10}  {:<7}  {:<24}  {}",
            entry.canonical_id,
            entry.effective_policy.level,
            entry.category,
            entry.analyzer,
            entry.confidence,
            entry.effective_policy.active,
            source,
            entry.tags.join(",")
        );
    }
    output
}

/// Deterministic human-readable explanation output.
pub(super) fn render_rule_explanation(explanation: &RuleExplanation) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "Rule: {}", explanation.canonical_id);
    let _ = writeln!(output, "Provider: {}", explanation.provider);
    let _ = writeln!(output, "Category: {}", explanation.category);
    let _ = writeln!(output, "Analyzer: {}", explanation.analyzer);
    let _ = writeln!(output, "Confidence: {}", explanation.confidence);
    let _ = writeln!(
        output,
        "Effective policy: {} ({})",
        explanation.effective_policy.level,
        source_label(&explanation.effective_policy.configuration_source)
    );
    let _ = writeln!(output, "Active: {}", explanation.effective_policy.active);
    let _ = writeln!(
        output,
        "Threshold: {}",
        explanation
            .effective_policy
            .threshold
            .map_or_else(|| "none".to_string(), |value| value.to_string())
    );
    let _ = writeln!(
        output,
        "Surfaces: {}",
        explanation.effective_policy.surfaces.join(", ")
    );
    let _ = writeln!(output, "Policy provenance:");
    for source in &explanation.effective_policy.provenance {
        let fields = if source.fields.is_empty() {
            "no effective fields".to_string()
        } else {
            source.fields.join(", ")
        };
        let _ = writeln!(output, "  - {} ({fields})", source_label(source));
    }
    let _ = writeln!(output, "Rationale: {}", explanation.rationale);
    let _ = writeln!(output, "Evidence: {}", explanation.evidence_model);
    if !explanation.limitations.is_empty() {
        let _ = writeln!(output, "Limitations:");
        for limitation in &explanation.limitations {
            let _ = writeln!(output, "  - {limitation}");
        }
    }
    let _ = writeln!(output, "Fix: {}", explanation.fix_guidance);
    for url in &explanation.official_documentation {
        let _ = writeln!(output, "Documentation: {url}");
    }
    if explanation.namespace_fallback {
        let _ = writeln!(
            output,
            "Catalog resolution: namespace fallback (no explicit descriptor in this build)"
        );
    }
    output
}

fn explanation_for(
    resolved: &ResolvedConfig,
    descriptor: &RuleDescriptor,
    namespace_fallback: bool,
) -> RuleExplanation {
    RuleExplanation {
        canonical_id: descriptor.canonical_id.clone(),
        aliases: descriptor.aliases.clone(),
        provider: descriptor.provider.clone(),
        category: category_key(&descriptor.category).to_string(),
        default_severity: descriptor.default_severity,
        tags: descriptor.tags.clone(),
        analyzer: analyzer_key(descriptor.analyzer_kind).to_string(),
        confidence: confidence_key(descriptor.confidence).to_string(),
        default_enabled: descriptor.default_enabled,
        applicable_frameworks: descriptor.applicable_frameworks.clone(),
        supported_threshold: descriptor.supported_threshold.map(ThresholdRange::from),
        fix_capability: fix_capability_key(descriptor.fix_capability).to_string(),
        rationale: descriptor.description.clone(),
        evidence_model: evidence_model(descriptor.analyzer_kind).to_string(),
        limitations: limitations(descriptor, namespace_fallback),
        fix_guidance: descriptor.fix_guidance.clone(),
        official_documentation: vec![descriptor.documentation_url.clone()],
        effective_policy: effective_policy(resolved, descriptor),
        namespace_fallback,
    }
}

fn effective_policy(resolved: &ResolvedConfig, descriptor: &RuleDescriptor) -> EffectiveRulePolicy {
    let policy = resolved.rule_policy(descriptor, None);
    let mut provenance = vec![PolicyProvenance {
        kind: "catalog-default".to_string(),
        selector: descriptor.canonical_id.clone(),
        fields: vec![
            "activation".to_string(),
            "severity".to_string(),
            "surfaces".to_string(),
            "threshold".to_string(),
        ],
    }];

    for tag in &descriptor.tags {
        if let Some(config) = resolved.tag_config.get(tag) {
            provenance.push(PolicyProvenance {
                kind: "tag".to_string(),
                selector: tag.clone(),
                fields: policy_config_fields(config),
            });
        }
    }
    let category = category_key(&descriptor.category);
    if let Some(config) = resolved.category_config.get(category) {
        provenance.push(PolicyProvenance {
            kind: "category".to_string(),
            selector: category.to_string(),
            fields: policy_config_fields(config),
        });
    }
    if let Some(config) = resolved.rules_config.get(&descriptor.canonical_id) {
        provenance.push(PolicyProvenance {
            kind: "rule".to_string(),
            selector: descriptor.canonical_id.clone(),
            fields: rule_config_fields(config),
        });
    }
    for alias in &descriptor.aliases {
        if let Some(config) = resolved.rules_config.get(alias) {
            provenance.push(PolicyProvenance {
                kind: "rule-alias".to_string(),
                selector: alias.clone(),
                fields: rule_config_fields(config),
            });
        }
    }

    let configuration_source = provenance
        .iter()
        .rev()
        .find(|source| {
            source
                .fields
                .iter()
                .any(|field| matches!(field.as_str(), "severity" | "activation"))
        })
        .cloned()
        .unwrap_or_else(|| PolicyProvenance {
            kind: "catalog-default".to_string(),
            selector: descriptor.canonical_id.clone(),
            fields: Vec::new(),
        });
    let surfaces = surface_values()
        .into_iter()
        .filter_map(|(surface, name)| policy.visible_on(surface).then_some(name.to_string()))
        .collect();
    EffectiveRulePolicy {
        level: policy
            .severity
            .map_or_else(|| "off".to_string(), |severity| severity.to_string()),
        active: policy.severity.is_some(),
        threshold: policy.threshold,
        surfaces,
        configuration_source,
        provenance,
    }
}

fn policy_config_fields(config: &PolicyConfig) -> Vec<String> {
    let mut fields = Vec::new();
    if config.severity.is_some() {
        fields.push("severity".to_string());
    }
    if config.surfaces.is_some() {
        fields.push("surfaces".to_string());
    }
    fields
}

fn rule_config_fields(config: &RuleConfig) -> Vec<String> {
    let mut fields = Vec::new();
    if config.severity.is_some() {
        fields.push("severity".to_string());
    }
    if config.enabled.is_some() {
        fields.push("activation".to_string());
    }
    if config.threshold.is_some() {
        fields.push("threshold".to_string());
    }
    if config.surfaces.is_some() {
        fields.push("surfaces".to_string());
    }
    fields
}

fn source_label(source: &PolicyProvenance) -> String {
    if source.selector.is_empty() {
        source.kind.clone()
    } else {
        format!("{}:{}", source.kind, source.selector)
    }
}

fn matches_filter(
    descriptor: &RuleDescriptor,
    resolved: &ResolvedConfig,
    filter: &RuleListFilter,
) -> bool {
    filter
        .category
        .as_deref()
        .is_none_or(|category| category_key(&descriptor.category) == category)
        && filter
            .tag
            .as_ref()
            .is_none_or(|tag| descriptor.tags.contains(tag))
        && filter
            .framework
            .as_ref()
            .is_none_or(|framework| descriptor.applicable_frameworks.contains(framework))
        && filter
            .analyzer
            .as_deref()
            .is_none_or(|analyzer| analyzer_key(descriptor.analyzer_kind) == analyzer)
        && (!filter.configured_only || is_configured(descriptor, resolved))
}

fn is_configured(descriptor: &RuleDescriptor, resolved: &ResolvedConfig) -> bool {
    resolved.rules_config.contains_key(&descriptor.canonical_id)
        || descriptor
            .aliases
            .iter()
            .any(|alias| resolved.rules_config.contains_key(alias))
        || resolved
            .category_config
            .contains_key(category_key(&descriptor.category))
        || descriptor
            .tags
            .iter()
            .any(|tag| resolved.tag_config.contains_key(tag))
}

fn validate_list_filters(
    catalog: &RuleCatalog,
    filter: &RuleListFilter,
) -> Result<(), RuleWorkflowError> {
    validate_filter(
        "category",
        filter.category.as_deref(),
        &valid_categories(catalog),
    )?;
    validate_filter("tag", filter.tag.as_deref(), &valid_tags(catalog))?;
    validate_filter(
        "framework",
        filter.framework.as_deref(),
        &valid_frameworks(catalog),
    )?;
    validate_filter(
        "analyzer",
        filter.analyzer.as_deref(),
        &valid_analyzers(catalog),
    )
}

fn validate_filter(
    filter: &'static str,
    value: Option<&str>,
    candidates: &[String],
) -> Result<(), RuleWorkflowError> {
    let Some(value) = value else {
        return Ok(());
    };
    if candidates.iter().any(|candidate| candidate == value) {
        return Ok(());
    }
    Err(RuleWorkflowError::UnknownFilter {
        filter,
        value: value.to_string(),
        suggestions: NearestSuggestions::for_value(value, candidates.iter().map(String::as_str)),
    })
}

fn valid_categories(catalog: &RuleCatalog) -> Vec<String> {
    sorted_values(
        catalog
            .descriptors()
            .iter()
            .map(|descriptor| category_key(&descriptor.category).to_string()),
    )
}

fn valid_tags(catalog: &RuleCatalog) -> Vec<String> {
    sorted_values(
        catalog
            .descriptors()
            .iter()
            .flat_map(|descriptor| descriptor.tags.iter().cloned()),
    )
}

fn valid_frameworks(catalog: &RuleCatalog) -> Vec<String> {
    sorted_values(
        catalog
            .descriptors()
            .iter()
            .flat_map(|descriptor| descriptor.applicable_frameworks.iter().cloned()),
    )
}

fn valid_analyzers(catalog: &RuleCatalog) -> Vec<String> {
    sorted_values(
        catalog
            .descriptors()
            .iter()
            .map(|descriptor| analyzer_key(descriptor.analyzer_kind).to_string()),
    )
}

fn sorted_values(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut values: Vec<_> = values.into_iter().collect();
    values.sort();
    values.dedup();
    values
}

fn unknown_rule(catalog: &RuleCatalog, rule: &str) -> RuleWorkflowError {
    RuleWorkflowError::UnknownRule {
        rule: rule.to_string(),
        suggestions: NearestSuggestions::for_value(
            rule,
            catalog
                .descriptors()
                .iter()
                .map(|descriptor| descriptor.canonical_id.as_str()),
        ),
    }
}

fn namespace_descriptor(rule: &str) -> Option<RuleDescriptor> {
    if let Some(lint) = rule.strip_prefix("clippy::").filter(|lint| {
        !lint.is_empty()
            && lint
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    }) {
        return Some(RuleDescriptor {
            canonical_id: rule.to_string(),
            aliases: vec![lint.to_string()],
            provider: "clippy".to_string(),
            category: Category::Style,
            default_severity: Severity::Warning,
            tags: vec!["style".to_string(), "type-aware".to_string()],
            analyzer_kind: AnalyzerKind::Clippy,
            confidence: Confidence::High,
            default_enabled: true,
            applicable_frameworks: Vec::new(),
            documentation_url: format!(
                "https://rust-lang.github.io/rust-clippy/master/index.html#{lint}"
            ),
            supported_threshold: None,
            fix_capability: FixCapability::RustcSuggestion,
            description: format!("Clippy `{lint}` diagnostic"),
            fix_guidance: "Apply the rustc suggestion when its applicability permits it."
                .to_string(),
        });
    }
    if is_rustsec_id(rule) {
        return Some(RuleDescriptor {
            canonical_id: rule.to_string(),
            aliases: Vec::new(),
            provider: "rustsec".to_string(),
            category: Category::Security,
            default_severity: Severity::Error,
            tags: vec!["external".to_string(), "security".to_string()],
            analyzer_kind: AnalyzerKind::Dependency,
            confidence: Confidence::High,
            default_enabled: true,
            applicable_frameworks: Vec::new(),
            documentation_url: format!("https://rustsec.org/advisories/{rule}.html"),
            supported_threshold: None,
            fix_capability: FixCapability::Guidance,
            description: "Security advisory reported by RustSec".to_string(),
            fix_guidance:
                "Review the advisory and upgrade, replace, or isolate the affected dependency."
                    .to_string(),
        });
    }
    if rule.strip_prefix("deny::").is_some_and(|code| {
        !code.is_empty()
            && code.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
            })
    }) {
        return Some(RuleDescriptor {
            canonical_id: rule.to_string(),
            aliases: Vec::new(),
            provider: "cargo-deny".to_string(),
            category: Category::Dependencies,
            default_severity: Severity::Warning,
            tags: vec!["dependencies".to_string(), "external".to_string()],
            analyzer_kind: AnalyzerKind::Dependency,
            confidence: Confidence::High,
            default_enabled: true,
            applicable_frameworks: Vec::new(),
            documentation_url: "https://embarkstudios.github.io/cargo-deny/checks/index.html"
                .to_string(),
            supported_threshold: None,
            fix_capability: FixCapability::Guidance,
            description: "Dependency policy finding reported by cargo-deny".to_string(),
            fix_guidance: "Follow the cargo-deny check guidance and update the dependency policy."
                .to_string(),
        });
    }
    None
}

fn is_rustsec_id(rule: &str) -> bool {
    let Some(remainder) = rule.strip_prefix("RUSTSEC-") else {
        return false;
    };
    let mut parts = remainder.split('-');
    let (Some(year), Some(sequence), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    year.len() == 4
        && sequence.len() == 4
        && year.bytes().all(|byte| byte.is_ascii_digit())
        && sequence.bytes().all(|byte| byte.is_ascii_digit())
}

fn limitations(descriptor: &RuleDescriptor, namespace_fallback: bool) -> Vec<String> {
    let specific = match descriptor.canonical_id.as_str() {
        "unwrap-in-production" => Some(
            "Syntactic matching cannot distinguish a provably infallible unwrap from a risky one.",
        ),
        "large-enum-variant" => Some(
            "The rule counts fields rather than calculating each variant's concrete byte layout.",
        ),
        "blocking-in-async" => Some(
            "The rule recognizes known call names but does not follow aliases or interprocedural calls.",
        ),
        "sql-injection-risk" => Some(
            "String-built queries are heuristic evidence; the rule cannot prove that interpolated data is untrusted.",
        ),
        _ => None,
    };
    let mut values = specific.into_iter().map(str::to_string).collect::<Vec<_>>();
    if specific.is_none() && descriptor.analyzer_kind == AnalyzerKind::SynAst {
        values.push(
            "Syntactic analysis does not have rustc name resolution or inferred type information."
                .to_string(),
        );
    }
    if namespace_fallback {
        values.push(
            "This rule family has no explicit descriptor in this Rust Doctor build, so category and severity use namespace defaults."
                .to_string(),
        );
    }
    values
}

const fn analyzer_key(analyzer: AnalyzerKind) -> &'static str {
    match analyzer {
        AnalyzerKind::SynAst => "syn-ast",
        AnalyzerKind::Clippy => "clippy",
        AnalyzerKind::Dependency => "dependency",
        AnalyzerKind::Project => "project",
        AnalyzerKind::External => "external",
    }
}

const fn confidence_key(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Low => "low",
        Confidence::Medium => "medium",
        Confidence::High => "high",
    }
}

const fn fix_capability_key(capability: FixCapability) -> &'static str {
    match capability {
        FixCapability::None => "none",
        FixCapability::Guidance => "guidance",
        FixCapability::RustcSuggestion => "rustc-suggestion",
        FixCapability::MachineApplicable => "machine-applicable",
    }
}

const fn evidence_model(analyzer: AnalyzerKind) -> &'static str {
    match analyzer {
        AnalyzerKind::SynAst => "Syntactic Rust AST evidence without name or type resolution.",
        AnalyzerKind::Clippy => {
            "Type-aware rustc and Clippy evidence with compiler spans and suggestions."
        }
        AnalyzerKind::Dependency => {
            "Dependency graph, advisory, or policy evidence from the named analyzer."
        }
        AnalyzerKind::Project => "Cargo metadata or aggregate project-level evidence.",
        AnalyzerKind::External => "Analyzer-provided evidence retained under its namespace.",
    }
}

const fn category_key(category: &Category) -> &'static str {
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

const fn surface_values() -> [(VisibilitySurface, &'static str); 6] {
    [
        (VisibilitySurface::Terminal, "terminal"),
        (VisibilitySurface::Score, "score"),
        (VisibilitySurface::CiFailure, "ci-failure"),
        (VisibilitySurface::PrComment, "pr-comment"),
        (VisibilitySurface::Sarif, "sarif"),
        (VisibilitySurface::Mcp, "mcp"),
    ]
}

fn catalog() -> Result<&'static RuleCatalog, RuleWorkflowError> {
    built_in_catalog().map_err(|source| RuleWorkflowError::Catalog {
        message: source.to_string(),
    })
}

fn levenshtein(left: &str, right: &str) -> usize {
    let right_chars: Vec<_> = right.chars().collect();
    let mut previous: Vec<_> = (0..=right_chars.len()).collect();
    let mut current = vec![0; right_chars.len() + 1];
    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let substitution = previous[right_index] + usize::from(left_char != *right_char);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right_chars.len()]
}

/// Canonical policy mutations supported by the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RuleMutation {
    Set {
        rule: String,
        level: String,
        threshold: Option<u32>,
    },
    Enable {
        rule: String,
    },
    Disable {
        rule: String,
    },
    Category {
        category: String,
        level: String,
    },
    IgnoreTag {
        tag: String,
    },
    UnignoreTag {
        tag: String,
    },
}

/// Prepared two-phase edit. `diff` is exactly the content `commit` will persist.
#[derive(Debug)]
struct PreparedRuleMutation {
    path: PathBuf,
    original: FileSnapshot,
    proposed: String,
    diff: String,
}

impl PreparedRuleMutation {
    #[cfg(test)]
    fn proposed_content(&self) -> &str {
        &self.proposed
    }

    fn changed(&self) -> bool {
        self.original.content.as_deref().unwrap_or_default() != self.proposed
    }

    /// Persist with a final optimistic revision check and atomic sibling rename.
    fn commit(self) -> Result<MutationCommit, RuleWorkflowError> {
        if !self.changed() {
            return Ok(MutationCommit::Unchanged);
        }

        let parent = self
            .path
            .parent()
            .ok_or_else(|| RuleWorkflowError::AtomicWrite {
                path: self.path.clone(),
                operation: "resolving the destination directory",
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "configuration path has no parent directory",
                ),
            })?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".rust-doctor.toml.")
            .tempfile_in(parent)
            .map_err(|source| RuleWorkflowError::AtomicWrite {
                path: self.path.clone(),
                operation: "creating a sibling temporary file",
                source,
            })?;
        if let Some(permissions) = &self.original.permissions {
            temporary
                .as_file()
                .set_permissions(permissions.clone())
                .map_err(|source| RuleWorkflowError::AtomicWrite {
                    path: self.path.clone(),
                    operation: "preserving file permissions",
                    source,
                })?;
        }
        temporary
            .write_all(self.proposed.as_bytes())
            .and_then(|()| temporary.flush())
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|source| RuleWorkflowError::AtomicWrite {
                path: self.path.clone(),
                operation: "writing and syncing the temporary file",
                source,
            })?;

        let current = read_snapshot(&self.path)?;
        if !self.original.same_revision(&current) {
            return Err(RuleWorkflowError::ConcurrentModification { path: self.path });
        }

        temporary
            .persist(&self.path)
            .map_err(|error| RuleWorkflowError::AtomicWrite {
                path: self.path.clone(),
                operation: "renaming the temporary file",
                source: error.error,
            })?;
        Ok(MutationCommit::Written)
    }
}

/// Result of the commit phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MutationCommit {
    Written,
    Unchanged,
}

/// Convenience result for callers that do not need to hold a prepared edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuleMutationResult {
    pub(super) path: PathBuf,
    pub(super) diff: String,
    pub(super) changed: bool,
    pub(super) written: bool,
}

/// Parse, validate, migrate, and stage a policy mutation without writing.
fn prepare_rule_mutation(
    project_root: &Path,
    mutation: RuleMutation,
) -> Result<PreparedRuleMutation, RuleWorkflowError> {
    let path = project_root.join(CONFIG_FILE_NAME);
    let catalog = catalog()?;
    let validated = validate_mutation(catalog, mutation)?;
    let original = read_snapshot(&path)?;
    let original_content = original.content.as_deref().unwrap_or_default();
    let mut document = parse_document(original_content, &path)?;

    let existing = deserialize_known_config(&document, &path)?;
    validate_config(&existing, &path, catalog)?;
    let legacy_policy = crate::config::resolve_config_defaults(Some(&existing));
    migrate_legacy_rules(&mut document, &path, catalog, &legacy_policy)?;
    apply_validated_mutation(&mut document, &path, catalog, validated)?;

    let proposed = document.to_string();
    let proposed_document = parse_document(&proposed, &path)?;
    let proposed_config = deserialize_known_config(&proposed_document, &path)?;
    validate_config(&proposed_config, &path, catalog)?;
    let diff = unified_diff(original.content.is_some(), original_content, &proposed);
    Ok(PreparedRuleMutation {
        path,
        original,
        proposed,
        diff,
    })
}

/// Execute a mutation or return its exact dry-run diff without writing.
pub(super) fn execute_rule_mutation(
    project_root: &Path,
    mutation: RuleMutation,
    dry_run: bool,
) -> Result<RuleMutationResult, RuleWorkflowError> {
    let prepared = prepare_rule_mutation(project_root, mutation)?;
    let path = prepared.path.clone();
    let diff = prepared.diff.clone();
    let changed = prepared.changed();
    let written = if dry_run {
        false
    } else {
        prepared.commit()? == MutationCommit::Written
    };
    Ok(RuleMutationResult {
        path,
        diff,
        changed,
        written,
    })
}

#[derive(Debug)]
enum ValidatedMutation {
    Set {
        canonical_id: String,
        level: String,
        threshold: Option<u32>,
    },
    Enable {
        canonical_id: String,
        level: String,
    },
    Disable {
        canonical_id: String,
    },
    Category {
        category: String,
        level: String,
    },
    IgnoreTag {
        tag: String,
    },
    UnignoreTag {
        tag: String,
    },
}

fn validate_mutation(
    catalog: &RuleCatalog,
    mutation: RuleMutation,
) -> Result<ValidatedMutation, RuleWorkflowError> {
    match mutation {
        RuleMutation::Set {
            rule,
            level,
            threshold,
        } => {
            let descriptor = exact_descriptor(catalog, &rule)?;
            let level = validate_level(&level)?;
            validate_threshold(descriptor, threshold)?;
            Ok(ValidatedMutation::Set {
                canonical_id: descriptor.canonical_id.clone(),
                level,
                threshold,
            })
        }
        RuleMutation::Enable { rule } => {
            let descriptor = exact_descriptor(catalog, &rule)?;
            Ok(ValidatedMutation::Enable {
                canonical_id: descriptor.canonical_id.clone(),
                level: descriptor.default_severity.to_string(),
            })
        }
        RuleMutation::Disable { rule } => {
            let descriptor = exact_descriptor(catalog, &rule)?;
            Ok(ValidatedMutation::Disable {
                canonical_id: descriptor.canonical_id.clone(),
            })
        }
        RuleMutation::Category { category, level } => {
            let categories = valid_categories(catalog);
            if !categories.contains(&category) {
                return Err(RuleWorkflowError::UnknownFilter {
                    filter: "category",
                    value: category.clone(),
                    suggestions: NearestSuggestions::for_value(
                        &category,
                        categories.iter().map(String::as_str),
                    ),
                });
            }
            Ok(ValidatedMutation::Category {
                category,
                level: validate_level(&level)?,
            })
        }
        RuleMutation::IgnoreTag { tag } => {
            validate_tag(catalog, &tag)?;
            Ok(ValidatedMutation::IgnoreTag { tag })
        }
        RuleMutation::UnignoreTag { tag } => {
            validate_tag(catalog, &tag)?;
            Ok(ValidatedMutation::UnignoreTag { tag })
        }
    }
}

fn exact_descriptor<'a>(
    catalog: &'a RuleCatalog,
    rule: &str,
) -> Result<&'a RuleDescriptor, RuleWorkflowError> {
    catalog
        .exact(rule)
        .ok_or_else(|| unknown_rule(catalog, rule))
}

fn validate_level(level: &str) -> Result<String, RuleWorkflowError> {
    if VALID_LEVELS.contains(&level) {
        Ok(level.to_string())
    } else {
        Err(RuleWorkflowError::InvalidLevel {
            level: level.to_string(),
            suggestions: NearestSuggestions::for_value(level, VALID_LEVELS),
        })
    }
}

fn validate_threshold(
    descriptor: &RuleDescriptor,
    threshold: Option<u32>,
) -> Result<(), RuleWorkflowError> {
    let Some(value) = threshold else {
        return Ok(());
    };
    let Some(range) = descriptor.supported_threshold else {
        return Err(RuleWorkflowError::UnsupportedThreshold {
            rule: descriptor.canonical_id.clone(),
        });
    };
    if (range.min..=range.max).contains(&value) {
        Ok(())
    } else {
        Err(RuleWorkflowError::ThresholdOutOfRange {
            rule: descriptor.canonical_id.clone(),
            value,
            min: range.min,
            max: range.max,
        })
    }
}

fn validate_tag(catalog: &RuleCatalog, tag: &str) -> Result<(), RuleWorkflowError> {
    let tags = valid_tags(catalog);
    if tags.iter().any(|candidate| candidate == tag) {
        Ok(())
    } else {
        Err(RuleWorkflowError::UnknownFilter {
            filter: "tag",
            value: tag.to_string(),
            suggestions: NearestSuggestions::for_value(tag, tags.iter().map(String::as_str)),
        })
    }
}

fn parse_document(content: &str, path: &Path) -> Result<DocumentMut, RuleWorkflowError> {
    content
        .parse::<DocumentMut>()
        .map_err(|source| RuleWorkflowError::MalformedToml {
            path: path.to_path_buf(),
            message: source.to_string(),
        })
}

fn deserialize_known_config(
    document: &DocumentMut,
    path: &Path,
) -> Result<FileConfig, RuleWorkflowError> {
    let mut known = document.clone();
    let keys: Vec<_> = known.iter().map(|(key, _)| key.to_string()).collect();
    for key in keys {
        if !KNOWN_CONFIG_KEYS.contains(&key.as_str()) {
            known.as_table_mut().remove(&key);
        }
    }
    toml::from_str::<FileConfig>(&known.to_string()).map_err(|source| {
        RuleWorkflowError::InvalidConfiguration {
            path: path.to_path_buf(),
            message: source.to_string(),
        }
    })
}

fn validate_config(
    config: &FileConfig,
    path: &Path,
    catalog: &RuleCatalog,
) -> Result<(), RuleWorkflowError> {
    let configured_rules: BTreeSet<_> = config
        .rules
        .keys()
        .chain(config.rules_config.keys())
        .chain(config.ignore.rules.iter())
        .chain(config.ignore.enable.iter())
        .map(String::as_str)
        .collect();
    for rule in configured_rules {
        let descriptor = exact_descriptor(catalog, rule)?;
        let threshold = config
            .rules
            .get(rule)
            .or_else(|| config.rules_config.get(rule))
            .and_then(|policy| policy.threshold);
        validate_threshold(descriptor, threshold)?;
    }

    let categories = valid_categories(catalog);
    let configured_categories: BTreeSet<_> = config.categories.keys().map(String::as_str).collect();
    for category in configured_categories {
        if !categories.iter().any(|candidate| candidate == category) {
            return Err(RuleWorkflowError::UnknownFilter {
                filter: "category",
                value: category.to_string(),
                suggestions: NearestSuggestions::for_value(
                    category,
                    categories.iter().map(String::as_str),
                ),
            });
        }
    }
    let tags = valid_tags(catalog);
    let configured_tags: BTreeSet<_> = config.tags.keys().map(String::as_str).collect();
    for tag in configured_tags {
        if !tags.iter().any(|candidate| candidate == tag) {
            return Err(RuleWorkflowError::UnknownFilter {
                filter: "tag",
                value: tag.to_string(),
                suggestions: NearestSuggestions::for_value(tag, tags.iter().map(String::as_str)),
            });
        }
    }
    for path_override in &config.path_overrides {
        if path_override.pattern.is_empty() || globset::Glob::new(&path_override.pattern).is_err() {
            return Err(RuleWorkflowError::InvalidConfiguration {
                path: path.to_path_buf(),
                message: format!(
                    "path override '{}' is empty or is not a valid glob",
                    path_override.pattern
                ),
            });
        }
    }
    Ok(())
}

fn migrate_legacy_rules(
    document: &mut DocumentMut,
    path: &Path,
    catalog: &RuleCatalog,
    legacy_policy: &ResolvedConfig,
) -> Result<(), RuleWorkflowError> {
    let Some(legacy_item) = document.as_table_mut().remove("rules_config") else {
        return Ok(());
    };
    let mut legacy =
        legacy_item
            .into_table()
            .map_err(|_| RuleWorkflowError::InvalidDocumentShape {
                path: path.to_path_buf(),
                section: "rules_config".to_string(),
            })?;
    canonicalize_legacy_enabled(&mut legacy, path, catalog, legacy_policy)?;
    if !document.as_table().contains_key("rules") {
        document.as_table_mut().insert("rules", Item::Table(legacy));
        return Ok(());
    }

    let rules_item = document.as_table_mut().get_mut("rules").ok_or_else(|| {
        RuleWorkflowError::InvalidDocumentShape {
            path: path.to_path_buf(),
            section: "rules".to_string(),
        }
    })?;
    let rules =
        rules_item
            .as_table_like_mut()
            .ok_or_else(|| RuleWorkflowError::InvalidDocumentShape {
                path: path.to_path_buf(),
                section: "rules".to_string(),
            })?;
    let legacy_keys: Vec<_> = legacy.iter().map(|(key, _)| key.to_string()).collect();
    for key in legacy_keys {
        if !rules.contains_key(&key) {
            let Some(item) = legacy.remove(&key) else {
                continue;
            };
            rules.insert(&key, item);
        }
    }
    Ok(())
}

fn canonicalize_legacy_enabled(
    legacy: &mut Table,
    path: &Path,
    catalog: &RuleCatalog,
    legacy_policy: &ResolvedConfig,
) -> Result<(), RuleWorkflowError> {
    let rule_ids: Vec<_> = legacy.iter().map(|(rule, _)| rule.to_string()).collect();
    for rule in rule_ids {
        let descriptor = exact_descriptor(catalog, &rule)?;
        let policy = legacy
            .get_mut(&rule)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| RuleWorkflowError::InvalidDocumentShape {
                path: path.to_path_buf(),
                section: format!("rules_config.{rule}"),
            })?;
        if policy.get("enabled").and_then(Item::as_bool).is_none() {
            if policy.contains_key("enabled") {
                return Err(RuleWorkflowError::InvalidConfiguration {
                    path: path.to_path_buf(),
                    message: format!("rules_config.{rule}.enabled must be a boolean"),
                });
            }
            continue;
        }
        let level = legacy_policy
            .rule_policy(descriptor, None)
            .severity
            .map_or_else(|| "off".to_string(), |severity| severity.to_string());
        replace_enabled_with_severity(policy, &level);
    }
    Ok(())
}

fn replace_enabled_with_severity(policy: &mut dyn TableLike, level: &str) {
    let key_decor = policy
        .key("enabled")
        .map(|key| (key.leaf_decor().clone(), key.dotted_decor().clone()));
    let Some(enabled_item) = policy.remove("enabled") else {
        return;
    };
    let value_decor = enabled_item
        .as_value()
        .map(|enabled| enabled.decor().clone());

    if policy.contains_key("severity") {
        if let Some((leaf, dotted)) = key_decor {
            if let Some(mut severity_key) = policy.key_mut("severity") {
                merge_decor(severity_key.leaf_decor_mut(), &leaf);
                merge_decor(severity_key.dotted_decor_mut(), &dotted);
            }
        }
        if let Some(enabled_decor) = value_decor {
            if let Some(severity) = policy.get_mut("severity").and_then(Item::as_value_mut) {
                merge_decor(severity.decor_mut(), &enabled_decor);
            }
        }
        return;
    }

    let mut severity = value(level);
    if let (Some(enabled_decor), Some(severity_value)) = (value_decor, severity.as_value_mut()) {
        *severity_value.decor_mut() = enabled_decor;
    }
    policy.insert("severity", severity);
    if let Some((leaf, dotted)) = key_decor {
        if let Some(mut severity_key) = policy.key_mut("severity") {
            *severity_key.leaf_decor_mut() = leaf;
            *severity_key.dotted_decor_mut() = dotted;
        }
    }
}

fn merge_decor(target: &mut Decor, source: &Decor) {
    let prefix = format!(
        "{}{}",
        raw_string(source.prefix()),
        raw_string(target.prefix())
    );
    let suffix = format!(
        "{}{}",
        raw_string(target.suffix()),
        raw_string(source.suffix())
    );
    target.set_prefix(prefix);
    target.set_suffix(suffix);
}

fn raw_string(raw: Option<&RawString>) -> &str {
    raw.and_then(RawString::as_str).unwrap_or_default()
}

fn apply_validated_mutation(
    document: &mut DocumentMut,
    path: &Path,
    catalog: &RuleCatalog,
    mutation: ValidatedMutation,
) -> Result<(), RuleWorkflowError> {
    match mutation {
        ValidatedMutation::Set {
            canonical_id,
            level,
            threshold,
        } => {
            canonicalize_target_aliases(document, path, catalog, &canonical_id)?;
            let rules = ensure_section(document, path, "rules")?;
            let policy =
                ensure_child_table(rules, path, &format!("rules.{canonical_id}"), &canonical_id)?;
            set_value_preserving_decor(policy, "severity", value(level));
            if let Some(threshold) = threshold {
                set_value_preserving_decor(policy, "threshold", value(i64::from(threshold)));
            }
        }
        ValidatedMutation::Enable {
            canonical_id,
            level,
        } => {
            canonicalize_target_aliases(document, path, catalog, &canonical_id)?;
            let rules = ensure_section(document, path, "rules")?;
            let policy =
                ensure_child_table(rules, path, &format!("rules.{canonical_id}"), &canonical_id)?;
            set_value_preserving_decor(policy, "severity", value(level));
        }
        ValidatedMutation::Disable { canonical_id } => {
            canonicalize_target_aliases(document, path, catalog, &canonical_id)?;
            let rules = ensure_section(document, path, "rules")?;
            let policy =
                ensure_child_table(rules, path, &format!("rules.{canonical_id}"), &canonical_id)?;
            set_value_preserving_decor(policy, "severity", value("off"));
        }
        ValidatedMutation::Category { category, level } => {
            let categories = ensure_section(document, path, "categories")?;
            let policy = ensure_child_table(
                categories,
                path,
                &format!("categories.{category}"),
                &category,
            )?;
            set_value_preserving_decor(policy, "severity", value(level));
        }
        ValidatedMutation::IgnoreTag { tag } => {
            let tags = ensure_section(document, path, "tags")?;
            let policy = ensure_child_table(tags, path, &format!("tags.{tag}"), &tag)?;
            set_value_preserving_decor(policy, "severity", value("off"));
        }
        ValidatedMutation::UnignoreTag { tag } => {
            remove_tag_severity(document, path, &tag)?;
        }
    }
    Ok(())
}

fn ensure_section<'a>(
    document: &'a mut DocumentMut,
    path: &Path,
    section: &str,
) -> Result<&'a mut dyn TableLike, RuleWorkflowError> {
    let root = document.as_table_mut();
    if !root.contains_key(section) {
        let mut table = Table::new();
        table.set_implicit(true);
        root.insert(section, Item::Table(table));
    }
    root.get_mut(section)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| RuleWorkflowError::InvalidDocumentShape {
            path: path.to_path_buf(),
            section: section.to_string(),
        })
}

fn ensure_child_table<'a>(
    parent: &'a mut dyn TableLike,
    path: &Path,
    section: &str,
    key: &str,
) -> Result<&'a mut dyn TableLike, RuleWorkflowError> {
    if !parent.contains_key(key) {
        parent.insert(key, Item::Table(Table::new()));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| RuleWorkflowError::InvalidDocumentShape {
            path: path.to_path_buf(),
            section: section.to_string(),
        })
}

fn set_value_preserving_decor(table: &mut dyn TableLike, key: &str, replacement: Item) {
    if let Some(existing) = table.get_mut(key) {
        let decor = existing.as_value().map(|value| value.decor().clone());
        *existing = replacement;
        if let (Some(decor), Some(value)) = (decor, existing.as_value_mut()) {
            *value.decor_mut() = decor;
        }
    } else {
        table.insert(key, replacement);
    }
}

fn canonicalize_target_aliases(
    document: &mut DocumentMut,
    path: &Path,
    catalog: &RuleCatalog,
    canonical_id: &str,
) -> Result<(), RuleWorkflowError> {
    let Some(descriptor) = catalog.exact(canonical_id) else {
        return Err(unknown_rule(catalog, canonical_id));
    };
    if descriptor.aliases.is_empty() || !document.as_table().contains_key("rules") {
        return Ok(());
    }
    let rules = document
        .as_table_mut()
        .get_mut("rules")
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| RuleWorkflowError::InvalidDocumentShape {
            path: path.to_path_buf(),
            section: "rules".to_string(),
        })?;

    for alias in &descriptor.aliases {
        let Some(alias_item) = rules.remove(alias) else {
            continue;
        };
        if !rules.contains_key(canonical_id) {
            rules.insert(canonical_id, alias_item);
            continue;
        }
        let mut alias_table =
            alias_item
                .into_table()
                .map_err(|_| RuleWorkflowError::InvalidDocumentShape {
                    path: path.to_path_buf(),
                    section: format!("rules.{alias}"),
                })?;
        let canonical = rules
            .get_mut(canonical_id)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| RuleWorkflowError::InvalidDocumentShape {
                path: path.to_path_buf(),
                section: format!("rules.{canonical_id}"),
            })?;
        let keys: Vec<_> = alias_table.iter().map(|(key, _)| key.to_string()).collect();
        for key in keys {
            if let Some(item) = alias_table.remove(&key) {
                canonical.insert(&key, item);
            }
        }
    }
    Ok(())
}

fn remove_tag_severity(
    document: &mut DocumentMut,
    path: &Path,
    tag: &str,
) -> Result<(), RuleWorkflowError> {
    let Some(tags_item) = document.as_table_mut().get_mut("tags") else {
        return Ok(());
    };
    let tags =
        tags_item
            .as_table_like_mut()
            .ok_or_else(|| RuleWorkflowError::InvalidDocumentShape {
                path: path.to_path_buf(),
                section: "tags".to_string(),
            })?;
    let Some(tag_item) = tags.get_mut(tag) else {
        return Ok(());
    };
    let tag_table =
        tag_item
            .as_table_like_mut()
            .ok_or_else(|| RuleWorkflowError::InvalidDocumentShape {
                path: path.to_path_buf(),
                section: format!("tags.{tag}"),
            })?;
    tag_table.remove("severity");
    let remove_tag = tag_table.is_empty();
    if remove_tag {
        tags.remove(tag);
    }
    let remove_section = tags.is_empty();
    if remove_section {
        document.as_table_mut().remove("tags");
    }
    Ok(())
}

#[derive(Debug)]
struct FileSnapshot {
    content: Option<String>,
    len: Option<u64>,
    modified: Option<SystemTime>,
    permissions: Option<Permissions>,
}

impl FileSnapshot {
    fn same_revision(&self, other: &Self) -> bool {
        self.content == other.content && self.len == other.len && self.modified == other.modified
    }
}

fn read_snapshot(path: &Path) -> Result<FileSnapshot, RuleWorkflowError> {
    let before = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileSnapshot {
                content: None,
                len: None,
                modified: None,
                permissions: None,
            });
        }
        Err(source) => {
            return Err(RuleWorkflowError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if !before.file_type().is_file() {
        return Err(RuleWorkflowError::UnsupportedFileType {
            path: path.to_path_buf(),
        });
    }
    let content = fs::read_to_string(path).map_err(|source| RuleWorkflowError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let after = fs::symlink_metadata(path).map_err(|source| RuleWorkflowError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(RuleWorkflowError::ConcurrentModification {
            path: path.to_path_buf(),
        });
    }
    Ok(FileSnapshot {
        content: Some(content),
        len: Some(after.len()),
        modified: after.modified().ok(),
        permissions: Some(after.permissions()),
    })
}

#[derive(Debug, Clone, Copy)]
enum DiffOperation<'a> {
    Equal(&'a str),
    Delete(&'a str),
    Insert(&'a str),
}

impl DiffOperation<'_> {
    const fn consumes_old(self) -> bool {
        !matches!(self, Self::Insert(_))
    }

    const fn consumes_new(self) -> bool {
        !matches!(self, Self::Delete(_))
    }

    const fn changed(self) -> bool {
        !matches!(self, Self::Equal(_))
    }
}

fn unified_diff(original_exists: bool, original: &str, proposed: &str) -> String {
    if original == proposed {
        return String::new();
    }
    let original_lines: Vec<_> = original.split_inclusive('\n').collect();
    let proposed_lines: Vec<_> = proposed.split_inclusive('\n').collect();
    let operations = diff_operations(&original_lines, &proposed_lines);
    let mut old_before = Vec::with_capacity(operations.len() + 1);
    let mut new_before = Vec::with_capacity(operations.len() + 1);
    let (mut old_count, mut new_count) = (0_usize, 0_usize);
    for operation in &operations {
        old_before.push(old_count);
        new_before.push(new_count);
        old_count += usize::from(operation.consumes_old());
        new_count += usize::from(operation.consumes_new());
    }
    old_before.push(old_count);
    new_before.push(new_count);

    let mut ranges = Vec::<(usize, usize)>::new();
    for index in operations
        .iter()
        .enumerate()
        .filter_map(|(index, operation)| operation.changed().then_some(index))
    {
        let start = index.saturating_sub(3);
        let end = (index + 4).min(operations.len());
        if let Some((_, previous_end)) = ranges.last_mut().filter(|(_, end)| start <= *end) {
            *previous_end = (*previous_end).max(end);
        } else {
            ranges.push((start, end));
        }
    }

    let mut output = String::new();
    if original_exists {
        output.push_str("--- a/rust-doctor.toml\n");
    } else {
        output.push_str("--- /dev/null\n");
    }
    output.push_str("+++ b/rust-doctor.toml\n");
    for (start, end) in ranges {
        let old_hunk_count = operations[start..end]
            .iter()
            .filter(|operation| operation.consumes_old())
            .count();
        let new_hunk_count = operations[start..end]
            .iter()
            .filter(|operation| operation.consumes_new())
            .count();
        let old_start = if old_hunk_count == 0 {
            old_before[start]
        } else {
            old_before[start] + 1
        };
        let new_start = if new_hunk_count == 0 {
            new_before[start]
        } else {
            new_before[start] + 1
        };
        let _ = writeln!(
            output,
            "@@ -{} +{} @@",
            diff_range(old_start, old_hunk_count),
            diff_range(new_start, new_hunk_count)
        );
        for operation in &operations[start..end] {
            match operation {
                DiffOperation::Equal(line) => push_diff_line(&mut output, ' ', line),
                DiffOperation::Delete(line) => push_diff_line(&mut output, '-', line),
                DiffOperation::Insert(line) => push_diff_line(&mut output, '+', line),
            }
        }
    }
    output
}

fn diff_range(start: usize, count: usize) -> String {
    if count == 1 {
        start.to_string()
    } else {
        format!("{start},{count}")
    }
}

fn push_diff_line(output: &mut String, prefix: char, line: &str) {
    output.push(prefix);
    output.push_str(line);
    if !line.ends_with('\n') {
        output.push('\n');
        output.push_str("\\ No newline at end of file\n");
    }
}

fn diff_operations<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<DiffOperation<'a>> {
    if old.len().saturating_mul(new.len()) > 1_000_000 {
        return old
            .iter()
            .map(|line| DiffOperation::Delete(line))
            .chain(new.iter().map(|line| DiffOperation::Insert(line)))
            .collect();
    }
    let width = new.len() + 1;
    let mut lengths = vec![0_usize; (old.len() + 1) * width];
    for old_index in (0..old.len()).rev() {
        for new_index in (0..new.len()).rev() {
            lengths[old_index * width + new_index] = if old[old_index] == new[new_index] {
                lengths[(old_index + 1) * width + new_index + 1] + 1
            } else {
                lengths[(old_index + 1) * width + new_index]
                    .max(lengths[old_index * width + new_index + 1])
            };
        }
    }

    let mut operations = Vec::with_capacity(old.len() + new.len());
    let (mut old_index, mut new_index) = (0_usize, 0_usize);
    while old_index < old.len() && new_index < new.len() {
        if old[old_index] == new[new_index] {
            operations.push(DiffOperation::Equal(old[old_index]));
            old_index += 1;
            new_index += 1;
        } else if lengths[(old_index + 1) * width + new_index]
            >= lengths[old_index * width + new_index + 1]
        {
            operations.push(DiffOperation::Delete(old[old_index]));
            old_index += 1;
        } else {
            operations.push(DiffOperation::Insert(new[new_index]));
            new_index += 1;
        }
    }
    operations.extend(
        old[old_index..]
            .iter()
            .map(|line| DiffOperation::Delete(line)),
    );
    operations.extend(
        new[new_index..]
            .iter()
            .map(|line| DiffOperation::Insert(line)),
    );
    operations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::resolve_config_defaults;

    #[test]
    fn list_filters_and_reports_policy_provenance() {
        let file: FileConfig = toml::from_str(
            r#"
                [tags.security]
                severity = "info"

                [categories.security]
                severity = "warning"

                [rules.hardcoded-secrets]
                severity = "error"
            "#,
        )
        .unwrap();
        let resolved = resolve_config_defaults(Some(&file));
        let entries = list_rules(
            &resolved,
            &RuleListFilter {
                category: Some("security".to_string()),
                configured_only: true,
                ..RuleListFilter::default()
            },
        )
        .unwrap();
        let rule = entries
            .iter()
            .find(|entry| entry.canonical_id == "hardcoded-secrets")
            .unwrap();
        assert_eq!(rule.effective_policy.level, "error");
        assert_eq!(rule.effective_policy.configuration_source.kind, "rule");
        assert_eq!(
            rule.effective_policy
                .provenance
                .iter()
                .map(|source| source.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["catalog-default", "tag", "category", "rule"]
        );
        let json = render_rule_list_json(&entries).unwrap();
        assert_eq!(json, render_rule_list_json(&entries).unwrap());
    }

    #[test]
    fn unknown_filter_returns_five_or_fewer_nearest_values() {
        let resolved = resolve_config_defaults(None);
        let error = list_rules(
            &resolved,
            &RuleListFilter {
                category: Some("securty".to_string()),
                ..RuleListFilter::default()
            },
        )
        .unwrap_err();
        let RuleWorkflowError::UnknownFilter { suggestions, .. } = error else {
            panic!("expected an unknown-filter error");
        };
        assert_eq!(
            suggestions.as_slice().first().map(String::as_str),
            Some("security")
        );
        assert!(suggestions.as_slice().len() <= 5);
    }

    #[test]
    fn explain_supports_dynamic_namespaces_without_catalog_insertion() {
        let resolved = resolve_config_defaults(None);
        let clippy = explain_rule(&resolved, "clippy::future_lint").unwrap();
        assert_eq!(clippy.provider, "clippy");
        assert!(clippy.namespace_fallback);
        assert!(clippy.official_documentation[0].ends_with("#future_lint"));

        let rustsec = explain_rule(&resolved, "RUSTSEC-2026-0001").unwrap();
        assert_eq!(rustsec.provider, "rustsec");
        assert!(rustsec.namespace_fallback);
    }

    #[test]
    fn dry_run_migrates_legacy_rules_and_preserves_comments_without_writing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(CONFIG_FILE_NAME);
        let original = r#"# project policy
[future-section]
answer = 42 # keep this

[rules_config.excessive-clone]
severity = "info" # legacy level
threshold = 4
"#;
        fs::write(&path, original).unwrap();

        let result = execute_rule_mutation(
            directory.path(),
            RuleMutation::Set {
                rule: "excessive-clone".to_string(),
                level: "warning".to_string(),
                threshold: Some(6),
            },
            true,
        )
        .unwrap();
        assert!(result.changed);
        assert!(!result.written);
        assert!(result.diff.contains("-severity = \"info\" # legacy level"));
        assert!(
            result
                .diff
                .contains("+severity = \"warning\" # legacy level")
        );
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }

    #[test]
    fn commit_preserves_unknown_sections_and_writes_canonical_policy() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(CONFIG_FILE_NAME);
        fs::write(
            &path,
            "# keep\n[future-section]\nanswer = 42\n\n[rules_config.excessive-clone]\nthreshold = 4\n",
        )
        .unwrap();
        let prepared = prepare_rule_mutation(
            directory.path(),
            RuleMutation::Disable {
                rule: "excessive-clone".to_string(),
            },
        )
        .unwrap();
        assert_eq!(prepared.commit().unwrap(), MutationCommit::Written);
        let written = fs::read_to_string(path).unwrap();
        assert!(written.contains("[future-section]"));
        assert!(written.contains("[rules.excessive-clone]"));
        assert!(written.contains("severity = \"off\""));
        assert!(!written.contains("rules_config"));
    }

    #[test]
    fn commit_refuses_a_concurrent_change() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(CONFIG_FILE_NAME);
        fs::write(&path, "verbose = false\n").unwrap();
        let prepared = prepare_rule_mutation(
            directory.path(),
            RuleMutation::Disable {
                rule: "hardcoded-secrets".to_string(),
            },
        )
        .unwrap();
        fs::write(&path, "verbose = true\n").unwrap();

        let error = prepared.commit().unwrap_err();
        assert!(matches!(
            error,
            RuleWorkflowError::ConcurrentModification { .. }
        ));
        assert_eq!(fs::read_to_string(path).unwrap(), "verbose = true\n");
    }

    #[test]
    fn invalid_threshold_leaves_the_file_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(CONFIG_FILE_NAME);
        fs::write(&path, "# untouched\n").unwrap();
        let error = prepare_rule_mutation(
            directory.path(),
            RuleMutation::Set {
                rule: "hardcoded-secrets".to_string(),
                level: "warning".to_string(),
                threshold: Some(3),
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RuleWorkflowError::UnsupportedThreshold { .. }
        ));
        assert_eq!(fs::read_to_string(path).unwrap(), "# untouched\n");
    }

    #[test]
    fn unignore_tag_keeps_other_tag_policy_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(CONFIG_FILE_NAME);
        fs::write(
            &path,
            "[tags.security]\nseverity = \"off\"\nsurfaces = [\"sarif\"]\n",
        )
        .unwrap();
        let prepared = prepare_rule_mutation(
            directory.path(),
            RuleMutation::UnignoreTag {
                tag: "security".to_string(),
            },
        )
        .unwrap();
        assert!(!prepared.proposed_content().contains("severity"));
        assert!(
            prepared
                .proposed_content()
                .contains("surfaces = [\"sarif\"]")
        );
    }

    #[test]
    fn unified_diff_marks_missing_final_newlines() {
        let diff = unified_diff(true, "a", "b\n");
        assert!(diff.contains("-a\n\\ No newline at end of file\n+b\n"));
    }
}
