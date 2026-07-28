//! SARIF 2.1.0 output for CI/CD integration (GitHub Code Scanning, GitLab SAST).
//!
//! Produces a valid SARIF 2.1.0 JSON file from a `ScanResult`.
//! See <https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html>

use crate::diagnostics::{
    CanonicalDiagnostic, Diagnostic, DiagnosticLocation, ReportV1, ScanResult, Severity,
};
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
    #[serde(skip_serializing_if = "Option::is_none")]
    properties: Option<RunProperties>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_field_names,
    reason = "SARIF property-bag keys share the rustDoctorScore namespace by contract"
)]
struct RunProperties {
    rust_doctor_score: Option<u32>,
    rust_doctor_score_label: Option<String>,
    rust_doctor_score_authoritative: bool,
    rust_doctor_score_reasons: Vec<String>,
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
    properties: Option<RuleProperties<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuleProperties<'a> {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    heuristic: bool,
    /// SARIF-standard confidence hint derived from the rule's trust tier, so
    /// GitHub Code Scanning can calibrate how much to trust the result.
    #[serde(skip_serializing_if = "Option::is_none")]
    precision: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trust_tier: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    score_eligible: Option<bool>,
}

/// Per-result properties: the canonical decision metadata that has no native
/// SARIF field survives here rather than being dropped (US-015 AC-7).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResultProperties<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root_cause_key: Option<&'a str>,
    score_impact: &'static str,
}

/// SARIF precision vocabulary for one trust tier.
const fn precision_for(trust_tier: &str) -> Option<&'static str> {
    match trust_tier.as_bytes() {
        b"compiler-proven" => Some("very-high"),
        b"advisory-backed" => Some("high"),
        b"calibrated-heuristic" => Some("medium"),
        b"audit-only" => Some("low"),
        _ => None,
    }
}

const fn score_impact_key(impact: crate::diagnostics::ScoreImpact) -> &'static str {
    use crate::diagnostics::ScoreImpact;
    match impact {
        ScoreImpact::Scored => "scored",
        ScoreImpact::Advisory => "advisory",
        ScoreImpact::Ineligible => "ineligible",
        ScoreImpact::Suppressed => "suppressed",
    }
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
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    partial_fingerprints: std::collections::BTreeMap<&'static str, &'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    properties: Option<ResultProperties<'a>>,
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
                properties: crate::rules::is_heuristic_rule(&d.rule).then_some(RuleProperties {
                    heuristic: true,
                    precision: None,
                    priority: None,
                    trust_tier: None,
                    score_eligible: None,
                }),
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
        partial_fingerprints: std::collections::BTreeMap::new(),
        properties: None,
    }
}

fn build_report_rules<'a>(
    diagnostics: &'a [&'a CanonicalDiagnostic],
) -> Vec<ReportingDescriptor<'a>> {
    let mut seen = std::collections::HashSet::new();
    let mut rules = Vec::new();
    for diagnostic in diagnostics {
        if seen.insert(diagnostic.rule.as_str()) {
            rules.push(ReportingDescriptor {
                id: &diagnostic.rule,
                short_description: Message {
                    text: Cow::Borrowed(&diagnostic.title),
                },
                help_uri: Some(&diagnostic.url),
                default_configuration: DefaultConfiguration {
                    level: severity_to_sarif_level(diagnostic.severity),
                },
                properties: Some(RuleProperties {
                    heuristic: diagnostic.tags.iter().any(|tag| tag == "heuristic"),
                    precision: precision_for(&diagnostic.trust_tier),
                    priority: diagnostic.priority.as_deref(),
                    trust_tier: (!diagnostic.trust_tier.is_empty())
                        .then_some(diagnostic.trust_tier.as_str()),
                    score_eligible: Some(diagnostic.score_eligible),
                }),
            });
        }
    }
    rules
}

fn canonical_to_result(diagnostic: &CanonicalDiagnostic) -> Result_<'_> {
    let locations = match &diagnostic.location {
        DiagnosticLocation::Source { path, range } => vec![Location {
            physical_location: PhysicalLocation {
                artifact_location: ArtifactLocation {
                    uri: Cow::Borrowed(path),
                    uri_base_id: "%SRCROOT%",
                },
                region: Some(Region {
                    start_line: range.start.line,
                    start_column: Some(range.start.column),
                }),
            },
        }],
        DiagnosticLocation::Project => Vec::new(),
    };
    let text = diagnostic
        .help
        .as_ref()
        .map_or(Cow::Borrowed(diagnostic.message.as_str()), |help| {
            Cow::Owned(format!("{}: {help}", diagnostic.message))
        });
    Result_ {
        rule_id: &diagnostic.rule,
        level: severity_to_sarif_level(diagnostic.severity),
        message: Message { text },
        locations,
        partial_fingerprints: root_cause_fingerprints(diagnostic),
        properties: Some(ResultProperties {
            priority: diagnostic.priority.as_deref(),
            root_cause_key: diagnostic.root_cause_key.as_deref(),
            score_impact: score_impact_key(diagnostic.score_impact),
        }),
    }
}

/// Site identity plus root-cause correlation.
///
/// Two results that share `rustDoctorRootCause/v1` are one defect: a SARIF
/// consumer can collapse them without re-deriving the grouping.
fn root_cause_fingerprints(
    diagnostic: &CanonicalDiagnostic,
) -> std::collections::BTreeMap<&'static str, &str> {
    let mut fingerprints =
        std::collections::BTreeMap::from([("rustDoctorSiteId/v1", diagnostic.site_id.as_str())]);
    if let Some(key) = diagnostic.root_cause_key.as_deref() {
        fingerprints.insert("rustDoctorRootCause/v1", key);
    }
    fingerprints
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
            properties: None,
        }],
    };

    serde_json::to_string_pretty(&log)
}

/// Convert the immutable canonical report into SARIF without rewriting findings.
pub fn render_report_sarif(report: &ReportV1) -> Result<String, serde_json::Error> {
    let diagnostics: Vec<&CanonicalDiagnostic> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.visible_on.iter().any(|value| value == "sarif"))
        .collect();
    let rules = build_report_rules(&diagnostics);
    let results = diagnostics
        .iter()
        .map(|diagnostic| canonical_to_result(diagnostic))
        .collect();
    let log = SarifLog {
        schema: SARIF_SCHEMA,
        version: SARIF_VERSION,
        runs: vec![Run {
            tool: Tool {
                driver: ToolComponent {
                    name: TOOL_NAME,
                    version: report.tool_version.clone(),
                    information_uri: "https://github.com/arthjean/rust-doctor",
                    rules,
                },
            },
            results,
            properties: Some(RunProperties {
                rust_doctor_score: report.summary.score,
                rust_doctor_score_label: report.summary.score_label.map(|label| label.to_string()),
                rust_doctor_score_authoritative: report.summary.score_authoritative,
                rust_doctor_score_reasons: report.summary.score_reasons.clone(),
            }),
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
            suppressed_security: vec![],
            planned_files: vec![],
            analyzed_files: vec![],
            compiler_evidence: vec![],
            execution: crate::diagnostics::ScanExecution::default(),
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
