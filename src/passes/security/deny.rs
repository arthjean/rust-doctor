use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::process;
use crate::scanner::AnalysisPass;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DENY_TIMEOUT_SECS: u64 = 60;
const MAX_OUTPUT_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

/// cargo-deny analysis pass — checks advisories, licenses, bans, and sources.
pub struct DenyPass {
    pub offline: bool,
}

impl AnalysisPass for DenyPass {
    fn name(&self) -> &'static str {
        "dependencies (cargo-deny)"
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        if !is_cargo_deny_available() {
            return Err(crate::error::PassError::Skipped {
                pass: self.name().to_string(),
                reason: "cargo-deny is not installed — supply-chain checking disabled. \
                         Install with: cargo install cargo-deny"
                    .to_string(),
            });
        }
        run_deny(project_root, self.offline).map_err(|message| crate::error::PassError::Failed {
            pass: "dependencies (cargo-deny)".to_string(),
            message,
        })
    }
}

/// Check if `cargo deny` is available. Result is cached for the process lifetime.
pub fn is_cargo_deny_available() -> bool {
    process::is_cargo_subcommand_available("deny")
}

fn run_deny(project_root: &Path, offline: bool) -> Result<Vec<Diagnostic>, String> {
    let mut args = vec!["deny", "check", "--format", "json"];
    if offline {
        args.push("--disable-fetch");
    }
    let child = process::spawn_in_group(
        Command::new("cargo")
            .args(&args)
            .current_dir(project_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .map_err(|e| format!("failed to spawn cargo deny: {e}"))?;

    let result = process::run_with_timeout(child, DENY_TIMEOUT_SECS, MAX_OUTPUT_BYTES)?;

    if result.timed_out {
        eprintln!("Warning: cargo-deny timed out after {DENY_TIMEOUT_SECS}s");
        return Ok(vec![]);
    }

    // Parse JSON Lines output
    let output = &result.stdout;
    if output.is_empty() {
        return Ok(vec![]);
    }

    Ok(parse_deny_output(output))
}

/// Parse cargo-deny JSON Lines output into diagnostics.
///
/// cargo-deny emits one JSON object per line. Diagnostic lines have
/// `"type": "diagnostic"` with fields including `severity`, `message`,
/// and `code` (the check category).
fn parse_deny_output(output: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Ok(entry) = serde_json::from_str::<DenyLine>(line) else {
            continue;
        };

        // Only process diagnostic entries
        if entry.r#type != "diagnostic" {
            continue;
        }

        let fields = entry.fields;

        // Skip non-error/warning severities (e.g. "note", "help")
        let severity = match fields.severity.as_str() {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            _ => continue,
        };

        let code = fields.code.as_deref().unwrap_or("unknown");

        let (rule, category, severity) = classify_deny_finding(code, severity);

        let help = fields.labels.iter().find_map(|label| label.message.clone());

        diagnostics.push(Diagnostic {
            file_path: PathBuf::from("Cargo.toml"),
            rule: rule.to_string(),
            category,
            severity,
            message: fields.message,
            help,
            line: None,
            column: None,
            fix: None,
        });
    }

    diagnostics
}

/// Map a cargo-deny check code to a rule name, category, and severity.
fn classify_deny_finding(
    code: &str,
    default_severity: Severity,
) -> (&'static str, Category, Severity) {
    match code {
        // Advisory-related codes
        "A001" | "A002" | "A003" | "A004" | "A005" | "A006" | "A007" | "A008" | "A009" | "A010"
        | "A011" | "A012" => ("deny-advisory", Category::Dependencies, Severity::Error),
        // License-related codes
        "L001" | "L002" | "L003" | "L004" | "L005" | "L006" | "L007" | "L008" | "L009" | "L010"
        | "L011" | "L012" | "L013" | "L014" | "L015" => {
            ("deny-license", Category::Cargo, Severity::Warning)
        }
        // Ban-related codes
        "B001" | "B002" | "B003" | "B004" | "B005" | "B006" | "B007" | "B008" | "B009" | "B010"
        | "B011" | "B012" => ("deny-ban", Category::Cargo, Severity::Error),
        // Source-related codes
        "S001" | "S002" | "S003" | "S004" | "S005" | "S006" | "S007" | "S008" | "S009" | "S010"
        | "S011" | "S012" => ("deny-source", Category::Cargo, Severity::Warning),
        // Catch-all: classify by code prefix
        _ if code.starts_with('A') => ("deny-advisory", Category::Dependencies, Severity::Error),
        _ if code.starts_with('L') => ("deny-license", Category::Cargo, Severity::Warning),
        _ if code.starts_with('B') => ("deny-ban", Category::Cargo, Severity::Error),
        _ if code.starts_with('S') => ("deny-source", Category::Cargo, Severity::Warning),
        _ => ("deny-unknown", Category::Dependencies, default_severity),
    }
}

// ─── JSON deserialization types ─────────────────────────────────────────────

/// A single line of cargo-deny JSON Lines output.
#[derive(Deserialize)]
struct DenyLine {
    r#type: String,
    fields: DenyFields,
}

/// Fields within a cargo-deny diagnostic line.
#[derive(Deserialize)]
struct DenyFields {
    /// Severity: "error", "warning", "note", "help"
    severity: String,
    /// Human-readable message describing the issue
    message: String,
    /// Check code (e.g. "A001" for advisory, "L001" for license)
    #[serde(default)]
    code: Option<String>,
    /// Labels with span and message information
    #[serde(default)]
    labels: Vec<DenyLabel>,
}

/// A label within a cargo-deny diagnostic.
#[derive(Deserialize)]
struct DenyLabel {
    /// Optional descriptive message for this label
    #[serde(default)]
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_advisory_is_error() {
        let (rule, category, severity) = classify_deny_finding("A001", Severity::Warning);
        assert_eq!(rule, "deny-advisory");
        assert_eq!(category, Category::Dependencies);
        assert_eq!(severity, Severity::Error);
    }

    #[test]
    fn test_classify_license_is_warning() {
        let (rule, category, severity) = classify_deny_finding("L003", Severity::Error);
        assert_eq!(rule, "deny-license");
        assert_eq!(category, Category::Cargo);
        assert_eq!(severity, Severity::Warning);
    }

    #[test]
    fn test_classify_ban_is_error() {
        let (rule, category, severity) = classify_deny_finding("B002", Severity::Warning);
        assert_eq!(rule, "deny-ban");
        assert_eq!(category, Category::Cargo);
        assert_eq!(severity, Severity::Error);
    }

    #[test]
    fn test_classify_source_is_warning() {
        let (rule, category, severity) = classify_deny_finding("S001", Severity::Error);
        assert_eq!(rule, "deny-source");
        assert_eq!(category, Category::Cargo);
        assert_eq!(severity, Severity::Warning);
    }

    #[test]
    fn test_classify_unknown_code() {
        let (rule, category, severity) = classify_deny_finding("Z999", Severity::Warning);
        assert_eq!(rule, "deny-unknown");
        assert_eq!(category, Category::Dependencies);
        assert_eq!(severity, Severity::Warning);
    }

    #[test]
    fn test_classify_advisory_prefix_fallback() {
        let (rule, _, _) = classify_deny_finding("A999", Severity::Warning);
        assert_eq!(rule, "deny-advisory");
    }

    #[test]
    fn test_parse_empty_output() {
        let diags = parse_deny_output("");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_parse_non_diagnostic_lines_skipped() {
        let output = r#"{"type":"log","fields":{"message":"checking advisories"}}"#;
        let diags = parse_deny_output(output);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_parse_advisory_diagnostic() {
        let output = r#"{"type":"diagnostic","fields":{"severity":"error","code":"A001","message":"crate `rsa` has a vulnerability: RUSTSEC-2023-0071 Marvin Attack","labels":[{"message":"security vulnerability detected"}]}}"#;
        let diags = parse_deny_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "deny-advisory");
        assert_eq!(diags[0].category, Category::Dependencies);
        assert_eq!(diags[0].severity, Severity::Error);
        assert!(diags[0].message.contains("rsa"));
        assert!(diags[0].message.contains("RUSTSEC-2023-0071"));
        assert_eq!(diags[0].file_path, PathBuf::from("Cargo.toml"));
        assert!(diags[0].help.is_some());
    }

    #[test]
    fn test_parse_license_diagnostic() {
        let output = r#"{"type":"diagnostic","fields":{"severity":"warning","code":"L003","message":"crate `openssl` has license GPL-3.0 which is not in the allow list","labels":[]}}"#;
        let diags = parse_deny_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "deny-license");
        assert_eq!(diags[0].category, Category::Cargo);
        assert_eq!(diags[0].severity, Severity::Warning);
        assert!(diags[0].message.contains("openssl"));
    }

    #[test]
    fn test_parse_ban_diagnostic() {
        let output = r#"{"type":"diagnostic","fields":{"severity":"error","code":"B001","message":"crate `openssl` is banned","labels":[{"message":"banned crate used"}]}}"#;
        let diags = parse_deny_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "deny-ban");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn test_parse_source_diagnostic() {
        let output = r#"{"type":"diagnostic","fields":{"severity":"warning","code":"S002","message":"crate `my-crate` sourced from unknown registry","labels":[]}}"#;
        let diags = parse_deny_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "deny-source");
        assert_eq!(diags[0].severity, Severity::Warning);
    }

    #[test]
    fn test_parse_note_severity_skipped() {
        let output = r#"{"type":"diagnostic","fields":{"severity":"note","code":"A001","message":"some note","labels":[]}}"#;
        let diags = parse_deny_output(output);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_parse_help_severity_skipped() {
        let output = r#"{"type":"diagnostic","fields":{"severity":"help","code":null,"message":"some help text","labels":[]}}"#;
        let diags = parse_deny_output(output);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_parse_multiple_lines() {
        let output = concat!(
            r#"{"type":"diagnostic","fields":{"severity":"error","code":"A001","message":"vuln in crate-a","labels":[]}}"#,
            "\n",
            r#"{"type":"log","fields":{"message":"checking licenses"}}"#,
            "\n",
            r#"{"type":"diagnostic","fields":{"severity":"warning","code":"L001","message":"license issue in crate-b","labels":[]}}"#,
            "\n",
        );
        let diags = parse_deny_output(output);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].rule, "deny-advisory");
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[1].rule, "deny-license");
        assert_eq!(diags[1].severity, Severity::Warning);
    }

    #[test]
    fn test_parse_invalid_json_skipped() {
        let output = "not json at all\n{broken json}\n";
        let diags = parse_deny_output(output);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_parse_no_code_field() {
        let output = r#"{"type":"diagnostic","fields":{"severity":"error","message":"unknown issue","labels":[]}}"#;
        let diags = parse_deny_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "deny-unknown");
    }

    #[test]
    fn test_parse_label_help_extracted() {
        let output = r#"{"type":"diagnostic","fields":{"severity":"error","code":"B003","message":"banned crate","labels":[{"message":"consider using ring instead"}]}}"#;
        let diags = parse_deny_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags[0].help.as_deref(),
            Some("consider using ring instead")
        );
    }

    #[test]
    fn test_parse_empty_labels() {
        let output = r#"{"type":"diagnostic","fields":{"severity":"warning","code":"S001","message":"unknown source","labels":[]}}"#;
        let diags = parse_deny_output(output);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].help.is_none());
    }

    #[test]
    #[ignore = "depends on optional external tool cargo-deny"]
    fn test_cargo_deny_availability() {
        assert!(
            is_cargo_deny_available(),
            "cargo-deny should be installed for this test"
        );
    }
}
