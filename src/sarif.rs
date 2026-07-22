//! SARIF 2.1.0 output for CI/CD integration (GitHub Code Scanning, GitLab SAST).
//!
//! Produces a valid SARIF 2.1.0 JSON file from a `ScanResult`.
//! See <https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html>

use crate::diagnostics::{Diagnostic, ScanResult, Severity};
use serde::Serialize;
use std::borrow::Cow;

const SARIF_SCHEMA: &str =
    "https://schemastore.azurewebsites.net/schemas/json/sarif-2.1.0-rtm.5.json";
const SARIF_VERSION: &str = "2.1.0";
const TOOL_NAME: &str = "rust-doctor";

// ---------------------------------------------------------------------------
// SARIF types (minimal subset for GitHub Code Scanning compatibility)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifLog<'a> {
    #[serde(rename = "$schema")]
    schema: &'static str,
    version: &'static str,
    runs: Vec<Run<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Run<'a> {
    tool: Tool<'a>,
    results: Vec<Result_<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Tool<'a> {
    driver: ToolComponent<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolComponent<'a> {
    name: &'static str,
    version: String,
    information_uri: &'static str,
    rules: Vec<ReportingDescriptor<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportingDescriptor<'a> {
    id: &'a str,
    short_description: Message<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    help_uri: Option<&'a str>,
    default_configuration: DefaultConfiguration,
    /// Standard SARIF property bag; carries `heuristic` for syn-only rules so
    /// GitHub Code Scanning consumers can calibrate confidence (US-013).
    /// Omitted for type-aware clippy lints and external-tool findings, keeping
    /// the output backward-compatible.
    #[serde(skip_serializing_if = "Option::is_none")]
    properties: Option<RuleProperties>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuleProperties {
    heuristic: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DefaultConfiguration {
    level: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Result_<'a> {
    rule_id: &'a str,
    level: &'static str,
    message: Message<'a>,
    locations: Vec<Location<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Message<'a> {
    text: Cow<'a, str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Location<'a> {
    physical_location: PhysicalLocation<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PhysicalLocation<'a> {
    artifact_location: ArtifactLocation<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<Region>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactLocation<'a> {
    uri: Cow<'a, str>,
    uri_base_id: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Region {
    start_line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_column: Option<u32>,
}

// ---------------------------------------------------------------------------
// Conversion
// ---------------------------------------------------------------------------

const fn severity_to_sarif_level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    }
}

fn build_rules(diagnostics: &[Diagnostic]) -> Vec<ReportingDescriptor<'_>> {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut rules = Vec::new();

    for d in diagnostics {
        if seen.insert(&d.rule) {
            rules.push(ReportingDescriptor {
                id: &d.rule,
                short_description: Message {
                    text: Cow::Borrowed(&d.message),
                },
                help_uri: d.help.as_deref(),
                default_configuration: DefaultConfiguration {
                    level: severity_to_sarif_level(d.severity),
                },
                properties: crate::rules::is_heuristic_rule(&d.rule)
                    .then_some(RuleProperties { heuristic: true }),
            });
        }
    }

    rules
}

fn diagnostic_to_result(d: &Diagnostic) -> Result_<'_> {
    let region = d.line.map(|line| Region {
        start_line: line,
        start_column: d.column,
    });

    let text = d
        .help
        .as_ref()
        .map_or(Cow::Borrowed(d.message.as_str()), |help| {
            Cow::Owned(format!("{} — {help}", d.message))
        });

    Result_ {
        rule_id: &d.rule,
        level: severity_to_sarif_level(d.severity),
        message: Message { text },
        locations: vec![Location {
            physical_location: PhysicalLocation {
                artifact_location: ArtifactLocation {
                    uri: d.file_path.to_string_lossy(),
                    uri_base_id: "%SRCROOT%",
                },
                region,
            },
        }],
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Convert a `ScanResult` into a SARIF 2.1.0 JSON string.
///
/// # Errors
///
/// Returns an error if JSON serialization fails.
pub fn render_sarif(scan_result: &ScanResult) -> Result<String, serde_json::Error> {
    let rules = build_rules(&scan_result.diagnostics);
    let results: Vec<Result_<'_>> = scan_result
        .diagnostics
        .iter()
        .map(diagnostic_to_result)
        .collect();

    let log = SarifLog {
        schema: SARIF_SCHEMA,
        version: SARIF_VERSION,
        runs: vec![Run {
            tool: Tool {
                driver: ToolComponent {
                    name: TOOL_NAME,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    information_uri: "https://github.com/arthjean/rust-doctor",
                    rules,
                },
            },
            results,
        }],
    };

    serde_json::to_string_pretty(&log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Category, ScoreLabel};
    use std::path::PathBuf;
    use std::time::Duration;

    fn make_scan_result(diagnostics: Vec<Diagnostic>) -> ScanResult {
        let error_count = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        let warning_count = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count();
        let info_count = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Info)
            .count();

        ScanResult {
            diagnostics,
            score: 85,
            score_label: ScoreLabel::Great,
            dimension_scores: crate::diagnostics::DimensionScores {
                security: 100,
                reliability: 100,
                maintainability: 100,
                performance: 100,
                dependencies: 100,
            },
            source_file_count: 10,
            elapsed: Duration::from_millis(100),
            skipped_passes: vec![],
            error_count,
            warning_count,
            info_count,
            pass_timings: vec![],
        }
    }

    #[test]
    fn test_empty_scan_produces_valid_sarif() {
        let result = make_scan_result(vec![]);
        let sarif = render_sarif(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&sarif).unwrap();
        assert_eq!(parsed["version"], "2.1.0");
        assert_eq!(parsed["runs"][0]["tool"]["driver"]["name"], "rust-doctor");
        assert!(parsed["runs"][0]["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_diagnostics_map_to_sarif_results() {
        let diags = vec![
            Diagnostic {
                file_path: PathBuf::from("src/main.rs"),
                rule: "unwrap-in-production".to_string(),
                category: Category::ErrorHandling,
                severity: Severity::Warning,
                message: "Use of .unwrap() in production code".to_string(),
                help: Some("Use ? operator instead".to_string()),
                line: Some(42),
                column: Some(10),
                fix: None,
            },
            Diagnostic {
                file_path: PathBuf::from("src/lib.rs"),
                rule: "hardcoded-secrets".to_string(),
                category: Category::Security,
                severity: Severity::Error,
                message: "Hardcoded secret detected".to_string(),
                help: None,
                line: Some(7),
                column: None,
                fix: None,
            },
        ];
        let result = make_scan_result(diags);
        let sarif = render_sarif(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&sarif).unwrap();

        let results = parsed["runs"][0]["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["ruleId"], "unwrap-in-production");
        assert_eq!(results[0]["level"], "warning");
        assert_eq!(
            results[0]["locations"][0]["physicalLocation"]["region"]["startLine"],
            42
        );
        assert_eq!(results[1]["ruleId"], "hardcoded-secrets");
        assert_eq!(results[1]["level"], "error");
    }

    #[test]
    fn test_severity_mapping() {
        assert_eq!(severity_to_sarif_level(Severity::Error), "error");
        assert_eq!(severity_to_sarif_level(Severity::Warning), "warning");
        assert_eq!(severity_to_sarif_level(Severity::Info), "note");
    }

    #[test]
    fn test_heuristic_rule_carries_property() {
        let diags = vec![
            Diagnostic {
                file_path: PathBuf::from("src/main.rs"),
                rule: "unwrap-in-production".to_string(),
                category: Category::ErrorHandling,
                severity: Severity::Warning,
                message: "heuristic finding".to_string(),
                help: None,
                line: Some(1),
                column: None,
                fix: None,
            },
            Diagnostic {
                file_path: PathBuf::from("src/main.rs"),
                rule: "clippy::needless_return".to_string(),
                category: Category::Style,
                severity: Severity::Warning,
                message: "clippy finding".to_string(),
                help: None,
                line: Some(2),
                column: None,
                fix: None,
            },
        ];
        let result = make_scan_result(diags);
        let sarif = render_sarif(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&sarif).unwrap();
        let rules = parsed["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap();

        let heuristic = rules
            .iter()
            .find(|r| r["id"] == "unwrap-in-production")
            .unwrap();
        assert_eq!(heuristic["properties"]["heuristic"], true);

        let clippy = rules
            .iter()
            .find(|r| r["id"] == "clippy::needless_return")
            .unwrap();
        assert!(
            clippy.get("properties").is_none(),
            "clippy lints must not carry the heuristic property"
        );
    }

    #[test]
    fn test_rules_are_deduplicated() {
        let diags = vec![
            Diagnostic {
                file_path: PathBuf::from("a.rs"),
                rule: "same-rule".to_string(),
                category: Category::Style,
                severity: Severity::Warning,
                message: "msg1".to_string(),
                help: None,
                line: Some(1),
                column: None,
                fix: None,
            },
            Diagnostic {
                file_path: PathBuf::from("b.rs"),
                rule: "same-rule".to_string(),
                category: Category::Style,
                severity: Severity::Warning,
                message: "msg2".to_string(),
                help: None,
                line: Some(5),
                column: None,
                fix: None,
            },
        ];
        let result = make_scan_result(diags);
        let sarif = render_sarif(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&sarif).unwrap();

        let rules = parsed["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap();
        assert_eq!(rules.len(), 1, "duplicate rules should be deduplicated");
        assert_eq!(
            parsed["runs"][0]["results"].as_array().unwrap().len(),
            2,
            "all results should be present"
        );
    }
}
