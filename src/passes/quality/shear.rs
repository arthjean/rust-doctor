use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::process;
use crate::scanner::AnalysisPass;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SHEAR_TIMEOUT_SECS: u64 = 30;
const MAX_OUTPUT_BYTES: u64 = 10 * 1024 * 1024;

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
            return Err(crate::error::PassError::Skipped {
                pass: self.name().to_string(),
                reason: "cargo-shear is not installed - unused dependency detection disabled. \
                         Install with: cargo install cargo-shear"
                    .to_string(),
            });
        }
        run_shear(project_root, self.offline).map_err(|error| crate::error::PassError::Failed {
            pass: self.name().to_string(),
            message: error.to_string(),
        })
    }
}

#[derive(Debug, thiserror::Error)]
enum ShearError {
    #[error("failed to spawn cargo shear: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("cargo-shear process failed: {0}")]
    Process(String),
    #[error("cargo-shear encountered an error during analysis")]
    Analysis,
    #[error("cargo-shear returned unexpected exit code {0}")]
    UnexpectedExit(i32),
    #[error("cargo-shear terminated without an exit code")]
    MissingExit,
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

fn run_shear(project_root: &Path, offline: bool) -> Result<Vec<Diagnostic>, ShearError> {
    let child = process::spawn_in_group(
        Command::new("cargo")
            .args(shear_arguments(offline))
            .current_dir(project_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .map_err(ShearError::Spawn)?;

    let result = process::run_with_timeout(child, SHEAR_TIMEOUT_SECS, MAX_OUTPUT_BYTES)
        .map_err(ShearError::Process)?;

    if result.timed_out {
        eprintln!("Warning: cargo-shear timed out after {SHEAR_TIMEOUT_SECS}s");
        return Ok(vec![]);
    }

    match result.exit_code {
        Some(0 | 1) => parse_shear_output(&result.stdout),
        Some(2) => Err(ShearError::Analysis),
        Some(code) => Err(ShearError::UnexpectedExit(code)),
        None => Err(ShearError::MissingExit),
    }
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

fn parse_shear_output(output: &str) -> Result<Vec<Diagnostic>, ShearError> {
    let output: ShearOutput = serde_json::from_str(output)?;
    Ok(output
        .findings
        .into_iter()
        .filter(|finding| is_unused_dependency(&finding.code))
        .map(|finding| Diagnostic {
            file_path: finding.file.unwrap_or_else(|| PathBuf::from("Cargo.toml")),
            rule: "unused-dependency".to_string(),
            category: Category::Dependencies,
            severity: Severity::Warning,
            message: finding.message,
            help: finding.help,
            line: None,
            column: None,
            fix: None,
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
        let diagnostics = parse_shear_output(output).unwrap();
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
        assert_eq!(
            diagnostics[0].help.as_deref(),
            Some("remove this dependency")
        );
        assert!(diagnostics[1].help.is_none());
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
        let diagnostics = parse_shear_output(output).unwrap();
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn rejects_non_json_output() {
        assert!(parse_shear_output("cargo-shear failed").is_err());
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
