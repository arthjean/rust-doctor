#![expect(
    clippy::redundant_pub_crate,
    reason = "adapter parsers and contracts are consumed by the sibling conformance module through this private crate module"
)]

use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::passes::adapter::{self, AdapterContract, EvidenceSource};
use crate::process;
use crate::scanner::AnalysisPass;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SHEAR_TIMEOUT_SECS: u64 = 30;
const MAX_OUTPUT_BYTES: u64 = 10 * 1024 * 1024;

/// cargo-shear reports unused dependencies from a structured JSON document.
pub(crate) const CONTRACT: AdapterContract = AdapterContract {
    pass: "dependencies (cargo-shear)",
    subcommand: "shear",
    parser_contract_version: "shear-json-v1",
    evidence_source: EvidenceSource::StructuredJson,
};

/// cargo-shear analysis pass that detects unused dependencies.
pub struct ShearPass {
    pub offline: bool,
}

impl AnalysisPass for ShearPass {
    fn name(&self) -> &'static str {
        "dependencies (cargo-shear)"
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        if !is_shear_available() {
            return Err(CONTRACT.skipped(
                "unused dependency detection disabled. Install with: cargo install cargo-shear",
            ));
        }
        run_shear(project_root, self.offline)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ShearError {
    #[error("failed to spawn cargo shear: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("cargo-shear process failed: {0}")]
    Process(String),
    #[error("cargo-shear returned invalid JSON: {0}")]
    InvalidOutput(#[from] serde_json::Error),
}

#[derive(Deserialize)]
struct ShearOutput {
    findings: Vec<ShearFinding>,
}

#[derive(Deserialize)]
struct ShearFinding {
    code: String,
    message: String,
    file: Option<PathBuf>,
    help: Option<String>,
}

fn is_shear_available() -> bool {
    process::is_cargo_subcommand_available("shear")
}

fn shear_arguments(offline: bool) -> Vec<&'static str> {
    let mut arguments = vec!["shear", "--format=json", "--color=never"];
    if offline {
        arguments.push("--offline");
    }
    arguments
}

fn run_shear(
    project_root: &Path,
    offline: bool,
) -> Result<Vec<Diagnostic>, crate::error::PassError> {
    let child = process::spawn_in_group(
        Command::new("cargo")
            .args(shear_arguments(offline))
            .current_dir(project_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .map_err(|error| CONTRACT.failed(ShearError::Spawn(error)))?;

    let result = process::run_with_timeout(child, SHEAR_TIMEOUT_SECS, MAX_OUTPUT_BYTES)
        .map_err(|error| CONTRACT.failed(ShearError::Process(error)))?;

    // Exit 1 means unused dependencies were found; exit 2 is an analysis error.
    CONTRACT.require_complete_run(&result, &[0, 1])?;

    parse_shear_output(
        &result.stdout,
        &CONTRACT.provenance(&CONTRACT.tool_version()),
    )
    .map_err(|error| CONTRACT.failed(error))
}

fn is_unused_dependency(code: &str) -> bool {
    matches!(
        code,
        "shear/unused_dependency"
            | "shear/unused_workspace_dependency"
            | "shear/unused_optional_dependency"
            | "shear/unused_feature_dependency"
    )
}

pub(crate) fn parse_shear_output(
    output: &str,
    provenance: &str,
) -> Result<Vec<Diagnostic>, ShearError> {
    let output: ShearOutput = serde_json::from_str(output)?;
    Ok(output
        .findings
        .into_iter()
        .filter(|finding| is_unused_dependency(&finding.code))
        .map(|finding| {
            // The manifest that declares the dependency is the package scope
            // this finding belongs to (US-009 AC-3).
            let manifest = finding
                .file
                .unwrap_or_else(|| PathBuf::from("Cargo.toml"))
                .to_string_lossy()
                .into_owned();
            adapter::project_diagnostic(
                "unused-dependency",
                Category::Dependencies,
                Severity::Warning,
                &finding.message,
                Some(&finding.help.map_or_else(
                    || format!("via {provenance}"),
                    |help| format!("{help}\n  via {provenance}"),
                )),
                &manifest,
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unused_dependency_findings_from_json() {
        let output = r#"{
          "summary": {"errors": 1, "warnings": 1, "fixed": 0},
          "findings": [
            {
              "code": "shear/unused_dependency",
              "severity": "error",
              "message": "unused dependency: `serde`",
              "file": "Cargo.toml",
              "location": {"offset": 42, "length": 5},
              "help": "remove this dependency",
              "fixable": true
            },
            {
              "code": "shear/unused_optional_dependency",
              "severity": "warning",
              "message": "unused optional dependency: `tokio`",
              "file": "crates/app/Cargo.toml",
              "location": {"offset": 84, "length": 5},
              "fixable": false
            }
          ]
        }"#;
        let diagnostics = parse_shear_output(output, "cargo-shear test").unwrap();
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].rule, "unused-dependency");
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(diagnostics[0].message.contains("serde"));
        assert!(diagnostics[1].message.contains("tokio"));
        assert_eq!(diagnostics[0].file_path, PathBuf::from("Cargo.toml"));
        assert_eq!(
            diagnostics[1].file_path,
            PathBuf::from("crates/app/Cargo.toml")
        );
        let help = diagnostics[0].help.as_deref().unwrap();
        assert!(help.starts_with("remove this dependency"));
        assert!(help.contains("cargo-shear test"));
        assert_eq!(diagnostics[1].help.as_deref(), Some("via cargo-shear test"));
    }

    #[test]
    fn ignores_non_dependency_findings() {
        let output = r#"{
          "summary": {"errors": 0, "warnings": 1, "fixed": 0},
          "findings": [{
            "code": "shear/unlinked_files",
            "severity": "warning",
            "message": "1 unlinked file in `demo`",
            "fixable": false
          }]
        }"#;
        let diagnostics = parse_shear_output(output, "cargo-shear test").unwrap();
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn rejects_non_json_output() {
        assert!(parse_shear_output("cargo-shear failed", "cargo-shear test").is_err());
    }

    #[test]
    fn recognizes_every_unused_dependency_code() {
        for code in [
            "shear/unused_dependency",
            "shear/unused_workspace_dependency",
            "shear/unused_optional_dependency",
            "shear/unused_feature_dependency",
        ] {
            assert!(is_unused_dependency(code), "{code}");
        }
        assert!(!is_unused_dependency("shear/misplaced_dependency"));
    }

    #[test]
    fn offline_mode_is_forwarded_to_cargo_shear() {
        assert_eq!(
            shear_arguments(false),
            vec!["shear", "--format=json", "--color=never"]
        );
        assert_eq!(
            shear_arguments(true),
            vec!["shear", "--format=json", "--color=never", "--offline"]
        );
    }

    #[test]
    #[ignore = "depends on optional external tool cargo-shear"]
    fn test_shear_availability() {
        assert!(
            is_shear_available(),
            "cargo-shear should be installed for this test"
        );
    }
}
