use crate::catalog::{RuleDescriptor, built_in_catalog};
use crate::cli::{Cli, FailOn};
use crate::diagnostics::{Category, Severity};
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

/// Typed severity and activation value used by every policy selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleLevel {
    Off,
    Info,
    Warning,
    Error,
}

impl RuleLevel {
    pub(crate) const fn severity(self) -> Option<Severity> {
        match self {
            Self::Off => None,
            Self::Info => Some(Severity::Info),
            Self::Warning => Some(Severity::Warning),
            Self::Error => Some(Severity::Error),
        }
    }
}

impl std::fmt::Display for RuleLevel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Off => "off",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        };
        formatter.write_str(value)
    }
}

impl From<Severity> for RuleLevel {
    fn from(value: Severity) -> Self {
        match value {
            Severity::Error => Self::Error,
            Severity::Warning => Self::Warning,
            Severity::Info => Self::Info,
        }
    }
}

/// Rendering and policy surfaces. Surface selection never activates a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VisibilitySurface {
    Terminal,
    Score,
    CiFailure,
    PrComment,
    Sarif,
    Mcp,
}

impl VisibilitySurface {
    const ALL: [Self; 6] = [
        Self::Terminal,
        Self::Score,
        Self::CiFailure,
        Self::PrComment,
        Self::Sarif,
        Self::Mcp,
    ];
}

/// Configuration as read from a file (all fields optional).
#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct FileConfig {
    /// Rules and files to ignore.
    pub ignore: IgnoreConfig,
    /// Enable/disable linting pass.
    pub lint: Option<bool>,
    /// Enable/disable dependency analysis pass.
    pub dependencies: Option<bool>,
    /// Enable verbose output.
    pub verbose: Option<bool>,
    /// Diff mode base branch.
    pub diff: Option<String>,
    /// Fail-on level ("error", "warning", "none").
    pub fail_on: Option<String>,
    /// Typed exact-rule overrides.
    #[serde(default)]
    pub rules: HashMap<String, RuleConfig>,
    /// Typed category overrides.
    #[serde(default)]
    pub categories: HashMap<String, PolicyConfig>,
    /// Typed tag overrides.
    #[serde(default)]
    pub tags: HashMap<String, PolicyConfig>,
    /// Highest-precedence path overrides, in declaration order.
    #[serde(default)]
    pub path_overrides: Vec<PathOverride>,
    /// Deprecated exact-rule overrides, retained for one migration release.
    #[serde(default)]
    pub rules_config: HashMap<String, RuleConfig>,
    /// Score configuration.
    #[serde(default)]
    pub score: ScoreConfig,
}

/// Per-rule configuration overrides.
#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct RuleConfig {
    /// Override severity for this rule.
    pub severity: Option<RuleLevel>,
    /// Deprecated activation switch. Prefer `severity = "off"`.
    pub enabled: Option<bool>,
    /// Custom threshold (rule-specific).
    pub threshold: Option<u32>,
    /// Surfaces allowed to render this rule. An empty list hides but does not disable it.
    pub surfaces: Option<BTreeSet<VisibilitySurface>>,
}

/// Category, tag, or path policy value.
#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct PolicyConfig {
    pub severity: Option<RuleLevel>,
    pub surfaces: Option<BTreeSet<VisibilitySurface>>,
}

/// Policy applied to every diagnostic whose normalized relative path matches.
#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct PathOverride {
    pub pattern: String,
    pub severity: Option<RuleLevel>,
    pub surfaces: Option<BTreeSet<VisibilitySurface>>,
}

/// Score configuration.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct ScoreConfig {
    /// Fail the scan if the score falls below this threshold.
    pub fail_below: Option<u32>,
}

/// Ignore configuration for rules and file patterns.
#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub struct IgnoreConfig {
    /// Rule names to ignore globally.
    pub rules: Vec<String>,
    /// File glob patterns to ignore.
    pub files: Vec<String>,
    /// Rule names to explicitly enable (for opt-in rules like string-from-literal).
    pub enable: Vec<String>,
}

/// Fully resolved configuration with concrete defaults.
/// Produced by merging CLI flags over file config over defaults.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub ignore_rules: Vec<String>,
    pub ignore_files: Vec<String>,
    pub lint: bool,
    pub dependencies: bool,
    pub verbose: bool,
    pub diff: Option<String>,
    pub fail_on: FailOn,
    pub rules_config: HashMap<String, RuleConfig>,
    pub category_config: HashMap<String, PolicyConfig>,
    pub tag_config: HashMap<String, PolicyConfig>,
    pub path_overrides: Vec<PathOverride>,
    pub enable_rules: Vec<String>,
    pub score_fail_below: Option<u32>,
    /// Whether source-level rust-doctor disable directives remove findings.
    pub respect_inline_disables: bool,
    /// Optional bound for concurrently scanned workspace packages.
    pub max_parallelism: Option<usize>,
}

/// Fully resolved policy for one rule at one optional source path.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedRulePolicy {
    pub(crate) severity: Option<Severity>,
    pub(crate) threshold: Option<u32>,
    surfaces: BTreeSet<VisibilitySurface>,
}

impl ResolvedRulePolicy {
    pub(crate) fn visible_on(&self, surface: VisibilitySurface) -> bool {
        self.surfaces.contains(&surface)
    }
}

/// Load configuration from `rust-doctor.toml` (first priority) or
/// `[package.metadata.rust-doctor]` in Cargo.toml (fallback).
///
/// Returns `Ok(None)` if no config is found. Returns `Err` on I/O or parse errors.
pub fn load_file_config(
    project_root: &Path,
    cargo_metadata: Option<&serde_json::Value>,
) -> Result<Option<FileConfig>, crate::error::ConfigError> {
    use crate::error::ConfigError;

    // Priority 1: rust-doctor.toml in project root
    let config_path = project_root.join("rust-doctor.toml");
    match std::fs::read_to_string(&config_path) {
        Ok(content) => {
            let config =
                toml::from_str::<FileConfig>(&content).map_err(|source| ConfigError::Parse {
                    path: config_path.clone(),
                    source,
                })?;
            validate_file_config(&config, &config_path)?;
            emit_legacy_deprecations(&config);
            return Ok(Some(config));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // File doesn't exist — fall through to Cargo.toml metadata
        }
        Err(source) => {
            return Err(ConfigError::Io {
                path: config_path,
                source,
            });
        }
    }

    // Priority 2: [package.metadata.rust-doctor] in Cargo.toml
    if let Some(metadata) = cargo_metadata {
        if let Some(section) = metadata.get("rust-doctor") {
            let config = serde_json::from_value::<FileConfig>(section.clone())?;
            validate_file_config(&config, &project_root.join("Cargo.toml"))?;
            emit_legacy_deprecations(&config);
            return Ok(Some(config));
        }
    }

    Ok(None)
}

/// Parse a `fail_on` string from config into a `FailOn` enum.
/// Returns `None` and prints a warning if the value is invalid.
fn parse_fail_on(value: &str) -> Option<FailOn> {
    match value {
        "error" => Some(FailOn::Error),
        "warning" => Some(FailOn::Warning),
        "info" => Some(FailOn::Info),
        "none" => Some(FailOn::None),
        _ => {
            eprintln!(
                "Warning: invalid fail_on value '{value}' in config. Valid values: error, warning, info, none"
            );
            None
        }
    }
}

/// Merge CLI flags with file config to produce a fully resolved configuration.
///
/// Precedence: CLI flags > config file values > hardcoded defaults.
pub fn resolve_config(cli: &Cli, file_config: Option<&FileConfig>) -> ResolvedConfig {
    let fc = file_config.cloned().unwrap_or_default();

    // For bool flags: CLI true always wins; if CLI false (not passed), use config
    let verbose = cli.verbose || fc.verbose.unwrap_or(false);
    let lint = fc.lint.unwrap_or(true);
    let dependencies = fc.dependencies.unwrap_or(true);
    let rules_config = merge_rule_config(&fc);

    // For Option fields: CLI Some wins; if CLI None, use config
    let diff = cli.diff.clone().or(fc.diff);

    // For fail_on: CLI Some wins; if CLI None, parse config value
    let fail_on = cli
        .blocking
        .or(cli.fail_on)
        .or_else(|| fc.fail_on.as_deref().and_then(parse_fail_on))
        .unwrap_or(FailOn::None);

    ResolvedConfig {
        ignore_rules: fc.ignore.rules,
        ignore_files: fc.ignore.files,
        lint,
        dependencies,
        verbose,
        diff,
        fail_on,
        rules_config,
        category_config: fc.categories,
        tag_config: fc.tags,
        path_overrides: fc.path_overrides,
        enable_rules: fc.ignore.enable,
        score_fail_below: fc.score.fail_below,
        respect_inline_disables: !cli.no_respect_inline_disables,
        max_parallelism: cli.jobs,
    }
}

/// Resolve configuration with file config only, no CLI overrides.
/// Used by the MCP server and programmatic API.
pub fn resolve_config_defaults(file_config: Option<&FileConfig>) -> ResolvedConfig {
    let fc = file_config.cloned().unwrap_or_default();
    let rules_config = merge_rule_config(&fc);
    ResolvedConfig {
        verbose: fc.verbose.unwrap_or(false),
        lint: fc.lint.unwrap_or(true),
        dependencies: fc.dependencies.unwrap_or(true),
        diff: fc.diff,
        fail_on: fc
            .fail_on
            .as_deref()
            .and_then(parse_fail_on)
            .unwrap_or(FailOn::None),
        ignore_rules: fc.ignore.rules,
        ignore_files: fc.ignore.files,
        rules_config,
        category_config: fc.categories,
        tag_config: fc.tags,
        path_overrides: fc.path_overrides,
        enable_rules: fc.ignore.enable,
        score_fail_below: fc.score.fail_below,
        respect_inline_disables: true,
        max_parallelism: None,
    }
}

impl ResolvedConfig {
    /// Resolve policy in ascending precedence: catalog default, tag, category,
    /// exact rule, then the last matching path override.
    pub(crate) fn rule_policy(
        &self,
        descriptor: &RuleDescriptor,
        path: Option<&Path>,
    ) -> ResolvedRulePolicy {
        let mut level = if descriptor.default_enabled {
            RuleLevel::from(descriptor.default_severity)
        } else {
            RuleLevel::Off
        };
        let mut threshold = descriptor.supported_threshold.map(|range| range.default);
        let mut surfaces: BTreeSet<_> = VisibilitySurface::ALL.into_iter().collect();

        for tag in &descriptor.tags {
            if let Some(policy) = self.tag_config.get(tag) {
                apply_policy(policy, &mut level, &mut surfaces);
            }
        }
        if let Some(policy) = self.category_config.get(category_key(&descriptor.category)) {
            apply_policy(policy, &mut level, &mut surfaces);
        }
        if let Some(policy) = self.rules_config.get(&descriptor.canonical_id) {
            apply_rule_policy(
                policy,
                RuleLevel::from(descriptor.default_severity),
                &mut level,
                &mut threshold,
                &mut surfaces,
            );
        }
        for alias in &descriptor.aliases {
            if let Some(policy) = self.rules_config.get(alias) {
                apply_rule_policy(
                    policy,
                    RuleLevel::from(descriptor.default_severity),
                    &mut level,
                    &mut threshold,
                    &mut surfaces,
                );
            }
        }
        if let Some(path) = path {
            let normalized = normalize_config_path(path);
            for path_override in &self.path_overrides {
                if globset::Glob::new(&path_override.pattern)
                    .is_ok_and(|glob| glob.compile_matcher().is_match(&normalized))
                {
                    apply_path_policy(path_override, &mut level, &mut surfaces);
                }
            }
        }

        ResolvedRulePolicy {
            severity: level.severity(),
            threshold,
            surfaces,
        }
    }

    /// Explain the ordered selectors that contribute to one effective policy.
    pub(crate) fn rule_policy_trace(
        &self,
        descriptor: &RuleDescriptor,
        path: Option<&Path>,
    ) -> Vec<String> {
        let default_level = RuleLevel::from(descriptor.default_severity);
        let mut level = if descriptor.default_enabled {
            default_level
        } else {
            RuleLevel::Off
        };
        let mut trace = vec![format!("catalog default: {level}")];
        for tag in &descriptor.tags {
            if let Some(policy) = self.tag_config.get(tag) {
                if let Some(policy_level) = policy.severity {
                    level = policy_level;
                    trace.push(format!("tag {tag}: {level}"));
                }
            }
        }
        let category = category_key(&descriptor.category);
        if let Some(policy) = self.category_config.get(category) {
            if let Some(policy_level) = policy.severity {
                level = policy_level;
                trace.push(format!("category {category}: {level}"));
            }
        }
        if let Some(policy) = self.rules_config.get(&descriptor.canonical_id) {
            trace_rule_selector(
                &mut trace,
                &format!("rule {}", descriptor.canonical_id),
                policy,
                default_level,
                &mut level,
            );
        }
        for alias in &descriptor.aliases {
            if let Some(policy) = self.rules_config.get(alias) {
                trace_rule_selector(
                    &mut trace,
                    &format!("rule alias {alias}"),
                    policy,
                    default_level,
                    &mut level,
                );
            }
        }
        if let Some(path) = path {
            let normalized = normalize_config_path(path);
            for path_override in &self.path_overrides {
                if globset::Glob::new(&path_override.pattern)
                    .is_ok_and(|glob| glob.compile_matcher().is_match(&normalized))
                {
                    if let Some(policy_level) = path_override.severity {
                        level = policy_level;
                        trace.push(format!("path {}: {level}", path_override.pattern));
                    }
                }
            }
        }
        trace
    }
}

fn trace_rule_selector(
    trace: &mut Vec<String>,
    source: &str,
    policy: &RuleConfig,
    default_level: RuleLevel,
    level: &mut RuleLevel,
) {
    if let Some(policy_level) = policy.severity {
        *level = policy_level;
        trace.push(format!("{source}: {level}"));
    } else if let Some(enabled) = policy.enabled {
        if enabled && *level == RuleLevel::Off {
            *level = default_level;
        } else if !enabled {
            *level = RuleLevel::Off;
        }
        trace.push(format!("{source}: {level}"));
    }
    if let Some(threshold) = policy.threshold {
        trace.push(format!("{source} threshold: {threshold}"));
    }
}

fn merge_rule_config(config: &FileConfig) -> HashMap<String, RuleConfig> {
    let mut merged = config.rules_config.clone();
    for ignored in &config.ignore.rules {
        merged.entry(ignored.clone()).or_default().severity = Some(RuleLevel::Off);
    }
    for enabled in &config.ignore.enable {
        let entry = merged.entry(enabled.clone()).or_default();
        if entry.severity == Some(RuleLevel::Off) {
            entry.severity = None;
        }
        entry.enabled = Some(true);
    }
    merged.extend(config.rules.clone());
    merged
}

fn apply_policy(
    policy: &PolicyConfig,
    level: &mut RuleLevel,
    surfaces: &mut BTreeSet<VisibilitySurface>,
) {
    if let Some(value) = policy.severity {
        *level = value;
    }
    if let Some(value) = &policy.surfaces {
        surfaces.clone_from(value);
    }
}

fn apply_rule_policy(
    policy: &RuleConfig,
    default_level: RuleLevel,
    level: &mut RuleLevel,
    threshold: &mut Option<u32>,
    surfaces: &mut BTreeSet<VisibilitySurface>,
) {
    if let Some(value) = policy.severity {
        *level = value;
    } else if let Some(enabled) = policy.enabled {
        if enabled && *level == RuleLevel::Off {
            *level = default_level;
        } else if !enabled {
            *level = RuleLevel::Off;
        }
    }
    if policy.threshold.is_some() {
        *threshold = policy.threshold;
    }
    if let Some(value) = &policy.surfaces {
        surfaces.clone_from(value);
    }
}

fn apply_path_policy(
    policy: &PathOverride,
    level: &mut RuleLevel,
    surfaces: &mut BTreeSet<VisibilitySurface>,
) {
    if let Some(value) = policy.severity {
        *level = value;
    }
    if let Some(value) = &policy.surfaces {
        surfaces.clone_from(value);
    }
}

pub(crate) fn validate_file_config(
    config: &FileConfig,
    path: &Path,
) -> Result<(), crate::error::ConfigError> {
    use crate::error::ConfigError;

    let catalog = built_in_catalog().map_err(|source| ConfigError::Catalog {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    let all_rule_ids = config
        .rules
        .keys()
        .chain(config.rules_config.keys())
        .chain(config.ignore.rules.iter())
        .chain(config.ignore.enable.iter());
    for rule in all_rule_ids {
        let Some(descriptor) = catalog.exact(rule) else {
            return Err(ConfigError::UnknownRule {
                path: path.to_path_buf(),
                rule: rule.clone(),
            });
        };
        if let Some(threshold) = config
            .rules
            .get(rule)
            .or_else(|| config.rules_config.get(rule))
            .and_then(|value| value.threshold)
        {
            let Some(range) = descriptor.supported_threshold else {
                return Err(ConfigError::UnsupportedThreshold {
                    path: path.to_path_buf(),
                    rule: rule.clone(),
                });
            };
            if !(range.min..=range.max).contains(&threshold) {
                return Err(ConfigError::ThresholdOutOfRange {
                    path: path.to_path_buf(),
                    rule: rule.clone(),
                    value: threshold,
                    min: range.min,
                    max: range.max,
                });
            }
        }
    }

    let categories: HashSet<_> = catalog
        .descriptors()
        .iter()
        .map(|descriptor| category_key(&descriptor.category))
        .collect();
    for category in config.categories.keys() {
        if !categories.contains(category.as_str()) {
            return Err(ConfigError::UnknownCategory {
                path: path.to_path_buf(),
                category: category.clone(),
            });
        }
    }

    let tags: HashSet<_> = catalog
        .descriptors()
        .iter()
        .flat_map(|descriptor| descriptor.tags.iter().map(String::as_str))
        .collect();
    for tag in config.tags.keys() {
        if !tags.contains(tag.as_str()) {
            return Err(ConfigError::UnknownTag {
                path: path.to_path_buf(),
                tag: tag.clone(),
            });
        }
    }

    for path_override in &config.path_overrides {
        if path_override.pattern.is_empty() || globset::Glob::new(&path_override.pattern).is_err() {
            return Err(ConfigError::InvalidPathOverride {
                path: path.to_path_buf(),
                pattern: path_override.pattern.clone(),
            });
        }
    }
    Ok(())
}

fn emit_legacy_deprecations(config: &FileConfig) {
    if !config.rules_config.is_empty() {
        eprintln!("Warning: `rules_config` is deprecated; use `[rules.<id>]` before v0.3");
    }
    if !config.ignore.rules.is_empty() || !config.ignore.enable.is_empty() {
        eprintln!(
            "Warning: `ignore.rules` and `ignore.enable` are deprecated; use typed rule severities before v0.3"
        );
    }
}

pub(crate) const fn category_key(category: &Category) -> &'static str {
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

fn normalize_config_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Validate that ignored rule names are known. Prints warnings for unknown rules.
/// Returns the list of unknown rule names found.
pub fn validate_ignored_rules<'a>(ignored: &'a [String], known_rules: &[&str]) -> Vec<&'a str> {
    let unknown: Vec<&str> = ignored
        .iter()
        .filter(|rule| !known_rules.contains(&rule.as_str()))
        .map(String::as_str)
        .collect();
    if !unknown.is_empty() {
        eprintln!(
            "Warning: unknown rule(s) in ignore config: {}\nValid rules: {}",
            unknown.join(", "),
            known_rules.join(", ")
        );
    }
    unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn cli_from(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap()
    }

    // --- FileConfig parsing ---

    #[test]
    fn test_parse_minimal_toml() {
        let toml_str = "";
        let config: FileConfig = toml::from_str(toml_str).unwrap();
        assert!(config.ignore.rules.is_empty());
        assert!(config.ignore.files.is_empty());
        assert_eq!(config.lint, None);
    }

    #[test]
    fn test_parse_full_toml() {
        let toml_str = r#"
            lint = false
            dependencies = true
            verbose = true
            diff = "main"
            fail_on = "error"

            [ignore]
            rules = ["unwrap-in-production", "excessive-clone"]
            files = ["**/generated/**", "tests/**"]
        "#;
        let config: FileConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.lint, Some(false));
        assert_eq!(config.dependencies, Some(true));
        assert_eq!(config.verbose, Some(true));
        assert_eq!(config.diff, Some("main".to_string()));
        assert_eq!(config.fail_on, Some("error".to_string()));
        assert_eq!(
            config.ignore.rules,
            vec!["unwrap-in-production", "excessive-clone"]
        );
        assert_eq!(config.ignore.files, vec!["**/generated/**", "tests/**"]);
    }

    #[test]
    fn test_parse_partial_toml() {
        let toml_str = r#"
            verbose = true
            [ignore]
            rules = ["hardcoded-secrets"]
        "#;
        let config: FileConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.verbose, Some(true));
        assert_eq!(config.lint, None);
        assert_eq!(config.ignore.rules, vec!["hardcoded-secrets"]);
        assert!(config.ignore.files.is_empty());
    }

    #[test]
    fn test_parse_invalid_toml() {
        let toml_str = "this is not valid toml [[[";
        let result = toml::from_str::<FileConfig>(toml_str);
        assert!(result.is_err());
    }

    // --- Config from Cargo.toml metadata (serde_json::Value) ---

    #[test]
    fn test_parse_cargo_metadata_section() {
        let json = serde_json::json!({
            "rust-doctor": {
                "verbose": true,
                "fail_on": "warning",
                "ignore": {
                    "rules": ["panic-in-library"]
                }
            }
        });
        let section = &json["rust-doctor"];
        let config: FileConfig = serde_json::from_value(section.clone()).unwrap();
        assert_eq!(config.verbose, Some(true));
        assert_eq!(config.fail_on, Some("warning".to_string()));
        assert_eq!(config.ignore.rules, vec!["panic-in-library"]);
    }

    #[test]
    fn test_load_file_config_from_metadata() {
        let json = serde_json::json!({
            "rust-doctor": {
                "lint": false
            }
        });
        let config = load_file_config(Path::new("/nonexistent"), Some(&json)).unwrap();
        assert!(config.is_some());
        assert_eq!(config.unwrap().lint, Some(false));
    }

    #[test]
    fn test_load_file_config_no_sources() {
        let config = load_file_config(Path::new("/nonexistent"), None).unwrap();
        assert!(config.is_none());
    }

    #[test]
    fn test_load_file_config_empty_metadata() {
        let json = serde_json::json!({});
        let config = load_file_config(Path::new("/nonexistent"), Some(&json)).unwrap();
        assert!(config.is_none());
    }

    // --- Merge / resolve tests ---

    #[test]
    fn test_resolve_defaults_no_config() {
        let cli = cli_from(&["rust-doctor"]);
        let resolved = resolve_config(&cli, None);
        assert!(!resolved.verbose);
        assert!(resolved.lint);
        assert!(resolved.dependencies);
        assert_eq!(resolved.diff, None);
        assert_eq!(resolved.fail_on, FailOn::None);
        assert!(resolved.ignore_rules.is_empty());
        assert!(resolved.ignore_files.is_empty());
    }

    #[test]
    fn test_resolve_config_values_used() {
        let cli = cli_from(&["rust-doctor"]);
        let fc = FileConfig {
            verbose: Some(true),
            lint: Some(false),
            dependencies: Some(false),
            diff: Some("develop".to_string()),
            fail_on: Some("error".to_string()),
            ignore: IgnoreConfig {
                rules: vec!["rule1".to_string()],
                files: vec!["test/**".to_string()],
                ..Default::default()
            },
            ..Default::default()
        };
        let resolved = resolve_config(&cli, Some(&fc));
        assert!(resolved.verbose);
        assert!(!resolved.lint);
        assert!(!resolved.dependencies);
        assert_eq!(resolved.diff, Some("develop".to_string()));
        assert_eq!(resolved.fail_on, FailOn::Error);
        assert_eq!(resolved.ignore_rules, vec!["rule1"]);
        assert_eq!(resolved.ignore_files, vec!["test/**"]);
    }

    #[test]
    fn test_cli_overrides_config_verbose() {
        let cli = cli_from(&["rust-doctor", "--verbose"]);
        let fc = FileConfig {
            verbose: Some(false),
            ..Default::default()
        };
        let resolved = resolve_config(&cli, Some(&fc));
        assert!(resolved.verbose);
    }

    #[test]
    fn test_cli_overrides_config_fail_on() {
        let cli = cli_from(&["rust-doctor", "--fail-on", "warning"]);
        let fc = FileConfig {
            fail_on: Some("error".to_string()),
            ..Default::default()
        };
        let resolved = resolve_config(&cli, Some(&fc));
        assert_eq!(resolved.fail_on, FailOn::Warning);
    }

    #[test]
    fn test_cli_overrides_config_diff() {
        let cli = cli_from(&["rust-doctor", "--diff", "main"]);
        let fc = FileConfig {
            diff: Some("develop".to_string()),
            ..Default::default()
        };
        let resolved = resolve_config(&cli, Some(&fc));
        assert_eq!(resolved.diff, Some("main".to_string()));
    }

    #[test]
    fn test_config_diff_used_when_cli_absent() {
        let cli = cli_from(&["rust-doctor"]);
        let fc = FileConfig {
            diff: Some("develop".to_string()),
            ..Default::default()
        };
        let resolved = resolve_config(&cli, Some(&fc));
        assert_eq!(resolved.diff, Some("develop".to_string()));
    }

    #[test]
    fn test_invalid_fail_on_in_config_falls_to_default() {
        let cli = cli_from(&["rust-doctor"]);
        let fc = FileConfig {
            fail_on: Some("critical".to_string()),
            ..Default::default()
        };
        let resolved = resolve_config(&cli, Some(&fc));
        assert_eq!(resolved.fail_on, FailOn::None);
    }

    // --- Rule validation ---

    #[test]
    fn test_validate_ignored_rules_all_known() {
        let ignored = vec!["unwrap-in-production".to_string()];
        let known = &["unwrap-in-production", "excessive-clone"];
        let unknown = validate_ignored_rules(&ignored, known);
        assert!(unknown.is_empty());
    }

    #[test]
    fn test_validate_ignored_rules_with_unknown() {
        let ignored = vec![
            "nonexistent-rule".to_string(),
            "unwrap-in-production".to_string(),
        ];
        let known = &["unwrap-in-production", "excessive-clone"];
        let unknown = validate_ignored_rules(&ignored, known);
        assert_eq!(unknown, vec!["nonexistent-rule"]);
    }

    #[test]
    fn test_validate_ignored_rules_empty() {
        let unknown = validate_ignored_rules(&[], &["rule1"]);
        assert!(unknown.is_empty());
    }

    // --- load_file_config with real TOML file ---

    #[test]
    fn test_load_file_config_from_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("rust-doctor.toml");
        std::fs::write(
            &config_path,
            r#"
            verbose = true
            fail_on = "warning"
            [ignore]
            rules = ["unwrap-in-production"]
            "#,
        )
        .unwrap();

        let config = load_file_config(dir.path(), None).unwrap();
        assert!(config.is_some());
        let fc = config.unwrap();
        assert_eq!(fc.verbose, Some(true));
        assert_eq!(fc.fail_on, Some("warning".to_string()));
        assert_eq!(fc.ignore.rules, vec!["unwrap-in-production"]);
    }

    #[test]
    fn test_toml_file_takes_priority_over_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("rust-doctor.toml");
        std::fs::write(&config_path, "verbose = true\n").unwrap();

        let json = serde_json::json!({
            "rust-doctor": { "verbose": false }
        });
        let config = load_file_config(dir.path(), Some(&json)).unwrap();
        assert!(config.is_some());
        assert_eq!(config.unwrap().verbose, Some(true));
    }

    #[test]
    fn test_load_invalid_toml_file_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("rust-doctor.toml");
        std::fs::write(&config_path, "not valid [[[toml").unwrap();

        let result = load_file_config(dir.path(), None);
        assert!(result.is_err());
    }

    // --- Per-rule config and score config ---

    #[test]
    fn test_parse_config_with_rules_config() {
        let toml_str = r#"
            [rules_config.excessive-clone]
            threshold = 5

            [rules_config.unwrap-in-production]
            severity = "error"
            enabled = false
        "#;
        let config: FileConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.rules_config.len(), 2);

        let clone_cfg = config.rules_config.get("excessive-clone").unwrap();
        assert_eq!(clone_cfg.threshold, Some(5));
        assert_eq!(clone_cfg.severity, None);
        assert_eq!(clone_cfg.enabled, None);

        let unwrap_cfg = config.rules_config.get("unwrap-in-production").unwrap();
        assert_eq!(unwrap_cfg.severity, Some(RuleLevel::Error));
        assert_eq!(unwrap_cfg.enabled, Some(false));
        assert_eq!(unwrap_cfg.threshold, None);
    }

    #[test]
    fn test_parse_config_with_score_fail_below() {
        let toml_str = r"
            [score]
            fail_below = 80
        ";
        let config: FileConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.score.fail_below, Some(80));
    }

    #[test]
    fn test_parse_config_with_enable_rules() {
        let toml_str = r#"
            [ignore]
            rules = ["clippy::too_many_lines"]
            enable = ["string-from-literal"]
            files = ["generated/**"]
        "#;
        let config: FileConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.ignore.enable, vec!["string-from-literal"]);
        assert_eq!(config.ignore.rules, vec!["clippy::too_many_lines"]);
        assert_eq!(config.ignore.files, vec!["generated/**"]);
    }

    #[test]
    fn test_resolve_config_merges_new_fields() {
        let cli = cli_from(&["rust-doctor"]);
        let mut rules_config = HashMap::new();
        rules_config.insert(
            "excessive-clone".to_string(),
            RuleConfig {
                threshold: Some(10),
                ..Default::default()
            },
        );
        let fc = FileConfig {
            ignore: IgnoreConfig {
                rules: vec!["some-rule".to_string()],
                files: vec![],
                enable: vec!["string-from-literal".to_string()],
            },
            rules_config,
            score: ScoreConfig {
                fail_below: Some(75),
            },
            ..Default::default()
        };
        let resolved = resolve_config(&cli, Some(&fc));
        assert_eq!(resolved.enable_rules, vec!["string-from-literal"]);
        assert_eq!(resolved.score_fail_below, Some(75));
        assert_eq!(resolved.rules_config.len(), 3);
        assert_eq!(
            resolved.rules_config.get("some-rule").unwrap().severity,
            Some(RuleLevel::Off)
        );
        assert_eq!(
            resolved
                .rules_config
                .get("string-from-literal")
                .unwrap()
                .enabled,
            Some(true)
        );
        assert_eq!(
            resolved
                .rules_config
                .get("excessive-clone")
                .unwrap()
                .threshold,
            Some(10)
        );
    }

    #[test]
    fn test_resolve_config_defaults_merges_new_fields() {
        let mut rules_config = HashMap::new();
        rules_config.insert(
            "unwrap-in-production".to_string(),
            RuleConfig {
                severity: Some(RuleLevel::Warning),
                ..Default::default()
            },
        );
        let fc = FileConfig {
            ignore: IgnoreConfig {
                enable: vec!["string-from-literal".to_string()],
                ..Default::default()
            },
            rules_config,
            score: ScoreConfig {
                fail_below: Some(90),
            },
            ..Default::default()
        };
        let resolved = resolve_config_defaults(Some(&fc));
        assert_eq!(resolved.enable_rules, vec!["string-from-literal"]);
        assert_eq!(resolved.score_fail_below, Some(90));
        assert_eq!(resolved.rules_config.len(), 2);
        assert_eq!(
            resolved
                .rules_config
                .get("string-from-literal")
                .unwrap()
                .enabled,
            Some(true)
        );
    }

    #[test]
    fn test_parse_full_example_config() {
        let toml_str = r#"
            [ignore]
            rules = ["clippy::too_many_lines"]
            enable = ["string-from-literal"]
            files = ["generated/**"]

            [rules_config.excessive-clone]
            threshold = 5

            [rules_config.unwrap-in-production]
            severity = "error"

            [score]
            fail_below = 80
        "#;
        let config: FileConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.ignore.rules, vec!["clippy::too_many_lines"]);
        assert_eq!(config.ignore.enable, vec!["string-from-literal"]);
        assert_eq!(config.ignore.files, vec!["generated/**"]);
        assert_eq!(config.rules_config.len(), 2);
        assert_eq!(
            config
                .rules_config
                .get("excessive-clone")
                .unwrap()
                .threshold,
            Some(5)
        );
        assert_eq!(
            config
                .rules_config
                .get("unwrap-in-production")
                .unwrap()
                .severity,
            Some(RuleLevel::Error)
        );
        assert_eq!(config.score.fail_below, Some(80));
    }

    #[test]
    fn test_deny_unknown_fields_rejects_typos() {
        let toml_str = r#"
            igonre = ["rule"]
        "#;
        let result = toml::from_str::<FileConfig>(toml_str);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unknown field"),
            "Expected 'unknown field' error, got: {err}"
        );
    }

    #[test]
    fn test_missing_new_sections_backward_compatible() {
        // Ensure old config format without new fields still parses correctly
        let toml_str = r#"
            lint = true
            verbose = false
            [ignore]
            rules = ["unwrap-in-production"]
        "#;
        let config: FileConfig = toml::from_str(toml_str).unwrap();
        assert!(config.rules_config.is_empty());
        assert_eq!(config.score.fail_below, None);
        assert!(config.ignore.enable.is_empty());
    }

    #[test]
    fn typed_policy_precedence_and_surfaces_are_deterministic() {
        let config: FileConfig = toml::from_str(
            r#"
            [tags.heuristic]
            severity = "info"

            [categories.error-handling]
            severity = "warning"

            [rules.unwrap-in-production]
            severity = "error"
            surfaces = ["terminal", "mcp"]

            [[path_overrides]]
            pattern = "tests/**"
            severity = "off"
            "#,
        )
        .unwrap();
        validate_file_config(&config, Path::new("rust-doctor.toml")).unwrap();
        let resolved = resolve_config_defaults(Some(&config));
        let catalog = built_in_catalog().unwrap();
        let descriptor = catalog.exact("unwrap-in-production").unwrap();

        let source = resolved.rule_policy(descriptor, Some(Path::new("src/lib.rs")));
        assert_eq!(source.severity, Some(Severity::Error));
        assert!(source.visible_on(VisibilitySurface::Terminal));
        assert!(!source.visible_on(VisibilitySurface::Score));

        let test = resolved.rule_policy(descriptor, Some(Path::new("tests/lib.rs")));
        assert_eq!(test.severity, None);
    }

    #[test]
    fn invalid_rule_threshold_and_path_return_typed_errors() {
        let unknown: FileConfig =
            toml::from_str("[rules.not-a-rule]\nseverity = \"warning\"\n").unwrap();
        assert!(matches!(
            validate_file_config(&unknown, Path::new("config.toml")),
            Err(crate::error::ConfigError::UnknownRule { .. })
        ));

        let unsupported: FileConfig =
            toml::from_str("[rules.unwrap-in-production]\nthreshold = 3\n").unwrap();
        assert!(matches!(
            validate_file_config(&unsupported, Path::new("config.toml")),
            Err(crate::error::ConfigError::UnsupportedThreshold { .. })
        ));

        let malformed: FileConfig =
            toml::from_str("[[path_overrides]]\npattern = \"[\"\nseverity = \"off\"\n").unwrap();
        assert!(matches!(
            validate_file_config(&malformed, Path::new("config.toml")),
            Err(crate::error::ConfigError::InvalidPathOverride { .. })
        ));
    }
}
