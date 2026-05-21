use crate::cli::{CategoryFilter, Cli, FailOn};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Configuration as read from a file (all fields optional).
#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
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
    /// Per-rule configuration overrides.
    #[serde(default)]
    pub rules_config: HashMap<String, RuleConfig>,
    /// Score configuration.
    #[serde(default)]
    pub score: ScoreConfig,
}

/// Per-rule configuration overrides.
#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub struct RuleConfig {
    /// Override severity for this rule.
    pub severity: Option<String>,
    /// Enable or disable this specific rule.
    pub enabled: Option<bool>,
    /// Custom threshold (rule-specific).
    pub threshold: Option<u32>,
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
#[derive(Debug)]
pub struct ResolvedConfig {
    pub ignore_rules: Vec<String>,
    pub ignore_files: Vec<String>,
    pub lint: bool,
    pub dependencies: bool,
    pub verbose: bool,
    pub diff: Option<String>,
    pub fail_on: FailOn,
    pub rules_config: HashMap<String, RuleConfig>,
    pub enable_rules: Vec<String>,
    pub score_fail_below: Option<u32>,
    pub category_filter: Option<CategoryFilter>,
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
                    path: config_path,
                    source,
                })?;
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

    // For Option fields: CLI Some wins; if CLI None, use config
    let diff = cli.diff.clone().or(fc.diff);

    // For fail_on: CLI Some wins; if CLI None, parse config value
    let fail_on = cli
        .fail_on
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
        rules_config: fc.rules_config,
        enable_rules: fc.ignore.enable,
        score_fail_below: fc.score.fail_below,
        category_filter: cli.category,
    }
}

/// Resolve configuration with file config only, no CLI overrides.
/// Used by the MCP server and programmatic API.
pub fn resolve_config_defaults(file_config: Option<&FileConfig>) -> ResolvedConfig {
    let fc = file_config.cloned().unwrap_or_default();
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
        rules_config: fc.rules_config,
        enable_rules: fc.ignore.enable,
        score_fail_below: fc.score.fail_below,
        category_filter: None,
    }
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
            rules = ["test-rule"]
            "#,
        )
        .unwrap();

        let config = load_file_config(dir.path(), None).unwrap();
        assert!(config.is_some());
        let fc = config.unwrap();
        assert_eq!(fc.verbose, Some(true));
        assert_eq!(fc.fail_on, Some("warning".to_string()));
        assert_eq!(fc.ignore.rules, vec!["test-rule"]);
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
        assert_eq!(unwrap_cfg.severity, Some("error".to_string()));
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
        assert_eq!(resolved.rules_config.len(), 1);
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
                severity: Some("warning".to_string()),
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
        assert_eq!(resolved.rules_config.len(), 1);
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
            Some("error".to_string())
        );
        assert_eq!(config.score.fail_below, Some(80));
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
}
