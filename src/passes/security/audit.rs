use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::process;
use crate::scanner::AnalysisPass;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn is_cargo_audit_available() -> bool {
    process::is_cargo_subcommand_available("audit")
}

const AUDIT_TIMEOUT_SECS: u64 = 60;
const MAX_OUTPUT_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

/// cargo-audit analysis pass — checks dependencies for known CVEs.
pub struct AuditPass {
    pub offline: bool,
}

impl AnalysisPass for AuditPass {
    fn name(&self) -> &'static str {
        "dependencies (cargo-audit)"
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        if !is_cargo_audit_available() {
            return Err(crate::error::PassError::Skipped {
                pass: self.name().to_string(),
                reason: "cargo-audit is not installed — CVE scanning disabled. \
                         Install with: cargo install cargo-audit"
                    .to_string(),
            });
        }
        run_audit(project_root, self.offline).map_err(|message| crate::error::PassError::Failed {
            pass: "dependencies (cargo-audit)".to_string(),
            message,
        })
    }
}

fn run_audit(project_root: &Path, offline: bool) -> Result<Vec<Diagnostic>, String> {
    let mut args = vec!["audit", "--json"];
    if offline {
        args.push("--no-fetch");
    }
    let child = process::spawn_in_group(
        Command::new("cargo")
            .args(&args)
            .current_dir(project_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .map_err(|e| format!("failed to spawn cargo audit: {e}"))?;

    let result = process::run_with_timeout(child, AUDIT_TIMEOUT_SECS, MAX_OUTPUT_BYTES)?;

    if result.timed_out {
        eprintln!("Warning: cargo-audit timed out after {AUDIT_TIMEOUT_SECS}s");
        return Ok(vec![]);
    }

    // Exit code 2 = operational error (no Cargo.lock, etc.)
    if result.exit_code == Some(2) {
        return Err(
            "cargo-audit encountered an error (missing Cargo.lock or fetch failure)".into(),
        );
    }

    // Parse JSON
    let output = &result.stdout;
    if output.is_empty() {
        return Ok(vec![]);
    }

    let report: AuditReport = serde_json::from_str(output)
        .map_err(|e| format!("failed to parse cargo-audit JSON: {e}"))?;

    let mut diagnostics = Vec::new();

    // Process vulnerabilities
    if let Some(vulns) = &report.vulnerabilities {
        for vuln in &vulns.list {
            let advisory = &vuln.advisory;
            let pkg = &vuln.package;

            let severity = advisory_to_severity(advisory);

            let patched = &vuln.versions.patched;
            let fix_hint = if patched.is_empty() {
                "No patched version available — consider an alternative crate".to_string()
            } else {
                format!("Upgrade {} to {}", pkg.name, patched.join(" or "))
            };

            let url_hint = advisory
                .url
                .as_deref()
                .map(|u| format!("\n  {u}"))
                .unwrap_or_default();

            diagnostics.push(Diagnostic {
                file_path: PathBuf::from("Cargo.lock"),
                rule: advisory.id.clone(),
                category: Category::Dependencies,
                severity,
                message: format!(
                    "{}: {} v{} — {}",
                    advisory.id, pkg.name, pkg.version, advisory.title
                ),
                help: Some(format!("{fix_hint}{url_hint}")),
                line: None,
                column: None,
                fix: None,
            });
        }
    }

    // Process warnings (unmaintained, yanked, etc.) as low-severity
    for (kind, warnings) in &report.warnings {
        for warn in warnings {
            if let Some(advisory) = &warn.advisory {
                diagnostics.push(Diagnostic {
                    file_path: PathBuf::from("Cargo.lock"),
                    rule: advisory.id.clone(),
                    category: Category::Dependencies,
                    severity: Severity::Warning,
                    message: format!(
                        "{}: {} v{} — {} ({})",
                        advisory.id, warn.package.name, warn.package.version, advisory.title, kind
                    ),
                    help: advisory
                        .url
                        .as_deref()
                        .map(std::string::ToString::to_string),
                    line: None,
                    column: None,
                    fix: None,
                });
            }
        }
    }

    Ok(diagnostics)
}

/// Map advisory severity to rust-doctor severity.
/// Uses cargo-audit's `severity` field (critical/high → Error, medium/low → Warning).
/// Falls back to CVSS vector parsing if severity field is absent.
fn advisory_to_severity(advisory: &Advisory) -> Severity {
    // Prefer the severity string from cargo-audit (most accurate)
    if let Some(ref sev) = advisory.severity {
        return match sev.as_str() {
            "critical" | "high" => Severity::Error,
            _ => Severity::Warning,
        };
    }

    // Fallback: parse CVSS base score from vector string
    if let Some(ref cvss) = advisory.cvss
        && let Some(score) = parse_cvss_base_score(cvss)
    {
        return if score >= 7.0 {
            Severity::Error
        } else {
            Severity::Warning
        };
    }

    Severity::Warning
}

/// Extract the base score from a CVSS 3.x vector string.
/// Format: "CVSS:3.1/AV:N/AC:L/..." — we look for the numeric score if appended,
/// or estimate from the vector metrics.
fn parse_cvss_base_score(cvss: &str) -> Option<f32> {
    // Some cargo-audit versions include the score directly
    // Check if the CVSS string starts with a bare number
    if let Ok(score) = cvss.parse::<f32>() {
        return Some(score);
    }

    // Heuristic from vector: Network + Low complexity + No user interaction → likely High
    let is_network = cvss.contains("AV:N");
    let is_low_complexity = cvss.contains("AC:L");
    let is_no_priv = cvss.contains("PR:N");
    let has_high_impact = cvss.contains("C:H") || cvss.contains("I:H") || cvss.contains("A:H");

    if has_high_impact && is_network && is_low_complexity && is_no_priv {
        Some(9.0) // Critical-range estimate
    } else if has_high_impact && is_network {
        Some(7.5) // High-range estimate
    } else if has_high_impact {
        Some(6.0) // Medium-range estimate
    } else {
        None
    }
}

// ─── JSON deserialization types ─────────────────────────────────────────────

#[derive(Deserialize)]
struct AuditReport {
    vulnerabilities: Option<Vulnerabilities>,
    #[serde(default)]
    warnings: std::collections::HashMap<String, Vec<WarningEntry>>,
}

#[derive(Deserialize)]
struct Vulnerabilities {
    #[serde(default)]
    list: Vec<VulnerabilityEntry>,
}

#[derive(Deserialize)]
struct VulnerabilityEntry {
    advisory: Advisory,
    versions: Versions,
    package: Package,
}

#[derive(Deserialize)]
struct Advisory {
    id: String,
    title: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    cvss: Option<String>,
    #[serde(default)]
    severity: Option<String>,
}

#[derive(Deserialize)]
struct Versions {
    #[serde(default)]
    patched: Vec<String>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct WarningEntry {
    advisory: Option<Advisory>,
    package: Package,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_advisory(severity: Option<&str>, cvss: Option<&str>) -> Advisory {
        Advisory {
            id: "TEST-001".into(),
            title: "Test".into(),
            url: None,
            cvss: cvss.map(std::string::ToString::to_string),
            severity: severity.map(std::string::ToString::to_string),
        }
    }

    #[test]
    fn test_severity_critical_is_error() {
        let adv = make_advisory(Some("critical"), None);
        assert_eq!(advisory_to_severity(&adv), Severity::Error);
    }

    #[test]
    fn test_severity_high_is_error() {
        let adv = make_advisory(Some("high"), None);
        assert_eq!(advisory_to_severity(&adv), Severity::Error);
    }

    #[test]
    fn test_severity_medium_is_warning() {
        let adv = make_advisory(Some("medium"), None);
        assert_eq!(advisory_to_severity(&adv), Severity::Warning);
    }

    #[test]
    fn test_severity_low_is_warning() {
        let adv = make_advisory(Some("low"), None);
        assert_eq!(advisory_to_severity(&adv), Severity::Warning);
    }

    #[test]
    fn test_cvss_fallback_network_critical() {
        let adv = make_advisory(None, Some("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:N/A:N"));
        assert_eq!(advisory_to_severity(&adv), Severity::Error);
    }

    #[test]
    fn test_cvss_fallback_local_medium() {
        let adv = make_advisory(None, Some("CVSS:3.1/AV:L/AC:L/PR:N/UI:N/S:U/C:L/I:N/A:N"));
        assert_eq!(advisory_to_severity(&adv), Severity::Warning);
    }

    #[test]
    fn test_no_severity_no_cvss_is_warning() {
        let adv = make_advisory(None, None);
        assert_eq!(advisory_to_severity(&adv), Severity::Warning);
    }

    #[test]
    fn test_parse_audit_report_empty() {
        let json = r#"{"vulnerabilities":{"found":false,"count":0,"list":[]},"warnings":{}}"#;
        let report: AuditReport = serde_json::from_str(json).unwrap();
        assert!(report.vulnerabilities.unwrap().list.is_empty());
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn test_parse_audit_report_with_vuln() {
        let json = r#"{
            "vulnerabilities": {
                "found": true,
                "count": 1,
                "list": [{
                    "advisory": {
                        "id": "RUSTSEC-2023-0071",
                        "title": "Marvin Attack",
                        "url": "https://example.com",
                        "cvss": "CVSS:3.1/AV:N/AC:H/PR:N/UI:N/S:U/C:H/I:N/A:N"
                    },
                    "versions": {
                        "patched": [">=0.10.0"]
                    },
                    "package": {
                        "name": "rsa",
                        "version": "0.9.6"
                    }
                }]
            },
            "warnings": {}
        }"#;
        let report: AuditReport = serde_json::from_str(json).unwrap();
        let vulns = report.vulnerabilities.unwrap();
        assert_eq!(vulns.list.len(), 1);
        assert_eq!(vulns.list[0].advisory.id, "RUSTSEC-2023-0071");
        assert_eq!(vulns.list[0].package.name, "rsa");
        assert_eq!(vulns.list[0].versions.patched, vec![">=0.10.0"]);
    }

    #[test]
    fn test_parse_audit_report_with_warning() {
        let json = r#"{
            "vulnerabilities": {"found": false, "count": 0, "list": []},
            "warnings": {
                "unmaintained": [{
                    "advisory": {
                        "id": "RUSTSEC-2021-0145",
                        "title": "Potential unaligned read",
                        "url": null,
                        "cvss": null
                    },
                    "package": {
                        "name": "atty",
                        "version": "0.2.14"
                    }
                }]
            }
        }"#;
        let report: AuditReport = serde_json::from_str(json).unwrap();
        assert_eq!(report.warnings["unmaintained"].len(), 1);
        assert_eq!(report.warnings["unmaintained"][0].package.name, "atty");
    }

    #[test]
    #[ignore = "depends on optional external tool cargo-audit"]
    fn test_cargo_audit_availability() {
        assert!(
            is_cargo_audit_available(),
            "cargo-audit should be installed for this test"
        );
    }
}
