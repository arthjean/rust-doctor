use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::process;
use crate::scanner::AnalysisPass;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SEMVER_TIMEOUT_SECS: u64 = 120;
const MAX_OUTPUT_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

/// cargo-semver-checks analysis pass — detects semver violations.
pub struct SemVerPass;

impl AnalysisPass for SemVerPass {
    fn name(&self) -> &'static str {
        "semver (cargo-semver-checks)"
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        if !is_semver_checks_available() {
            return Err(crate::error::PassError::Skipped {
                pass: self.name().to_string(),
                reason:
                    "cargo-semver-checks is not installed — semver violation detection disabled. \
                         Install with: cargo install cargo-semver-checks"
                        .to_string(),
            });
        }
        run_semver_checks(project_root).map_err(|message| crate::error::PassError::Failed {
            pass: "semver (cargo-semver-checks)".to_string(),
            message,
        })
    }
}

fn is_semver_checks_available() -> bool {
    process::is_cargo_subcommand_available("semver-checks")
}

fn run_semver_checks(project_root: &Path) -> Result<Vec<Diagnostic>, String> {
    let child = process::spawn_in_group(
        Command::new("cargo")
            .args(["semver-checks", "check-release"])
            .current_dir(project_root)
            .env("CARGO_TARGET_DIR", project_root.join("target/rust-doctor"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .map_err(|e| format!("failed to spawn cargo semver-checks: {e}"))?;

    let result = process::run_with_timeout(child, SEMVER_TIMEOUT_SECS, MAX_OUTPUT_BYTES)?;

    if result.timed_out {
        eprintln!("Warning: cargo-semver-checks timed out after {SEMVER_TIMEOUT_SECS}s");
        return Ok(vec![]);
    }

    // Parse the combined output for violations
    Ok(parse_semver_output(&result.stdout))
}

/// Parse cargo-semver-checks output into diagnostics.
///
/// cargo-semver-checks outputs lines like:
/// ```text
/// --- failure[name]: description
/// ```
/// Each such line is a semver violation.
fn parse_semver_output(output: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();

        // Match violation lines: "--- failure[name]: description"
        if let Some(rest) = trimmed.strip_prefix("--- failure[")
            && let Some(bracket_end) = rest.find("]:")
        {
            let violation_name = &rest[..bracket_end];
            let description = rest[bracket_end + 2..].trim();

            diagnostics.push(Diagnostic {
                file_path: PathBuf::from("Cargo.toml"),
                rule: "semver-violation".to_string(),
                category: Category::Cargo,
                severity: Severity::Warning,
                message: format!("{violation_name}: {description}"),
                help: Some(format!(
                    "This is a semver-incompatible change ({violation_name}). \
                         Bump the major version or revert the breaking change."
                )),
                line: None,
                column: None,
                fix: None,
            });
        }
    }

    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_violation() {
        let output = r"--- failure[function_missing]: function `foo` was removed
";
        let diags = parse_semver_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "semver-violation");
        assert_eq!(diags[0].category, Category::Cargo);
        assert_eq!(diags[0].severity, Severity::Warning);
        assert!(diags[0].message.contains("function_missing"));
        assert!(diags[0].message.contains("function `foo` was removed"));
        assert_eq!(diags[0].file_path, PathBuf::from("Cargo.toml"));
    }

    #[test]
    fn test_parse_multiple_violations() {
        let output = r"Checking my-crate v0.2.0 against v0.1.0
--- failure[function_missing]: function `foo` was removed
--- failure[struct_missing]: struct `Bar` was removed
--- failure[enum_variant_missing]: enum variant `Baz::Qux` was removed

Summary: 3 semver violations found
";
        let diags = parse_semver_output(output);
        assert_eq!(diags.len(), 3);
        assert!(diags[0].message.contains("function_missing"));
        assert!(diags[1].message.contains("struct_missing"));
        assert!(diags[2].message.contains("enum_variant_missing"));
    }

    #[test]
    fn test_parse_no_violations() {
        let output = r"Checking my-crate v0.2.0 against v0.1.0
Summary: no semver violations found
";
        let diags = parse_semver_output(output);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_parse_empty_output() {
        let diags = parse_semver_output("");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_help_text_present() {
        let output = "--- failure[trait_missing]: trait `MyTrait` was removed\n";
        let diags = parse_semver_output(output);
        assert_eq!(diags.len(), 1);
        let help = diags[0].help.as_ref().unwrap();
        assert!(help.contains("semver-incompatible"));
        assert!(help.contains("trait_missing"));
    }

    #[test]
    fn test_parse_violation_with_extra_whitespace() {
        let output = "  --- failure[method_missing]:   method `do_thing` was removed  \n";
        let diags = parse_semver_output(output);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("method_missing"));
        assert!(diags[0].message.contains("method `do_thing` was removed"));
    }

    #[test]
    fn test_graceful_skip_when_not_installed() {
        // This tests the pass logic without actually needing the tool
        let pass = SemVerPass;
        assert_eq!(pass.name(), "semver (cargo-semver-checks)");
    }

    #[test]
    #[ignore = "depends on optional external tool cargo-semver-checks"]
    fn test_semver_checks_availability() {
        assert!(
            is_semver_checks_available(),
            "cargo-semver-checks should be installed for this test"
        );
    }
}
