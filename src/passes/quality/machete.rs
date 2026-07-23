use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::process;
use crate::scanner::AnalysisPass;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MACHETE_TIMEOUT_SECS: u64 = 30;
const MAX_OUTPUT_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

/// cargo-machete analysis pass — detects unused dependencies.
pub struct MachetePass;

impl AnalysisPass for MachetePass {
    fn name(&self) -> &'static str {
        "dependencies (cargo-machete)"
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        if !is_machete_available() {
            return Err(crate::error::PassError::Skipped {
                pass: self.name().to_string(),
                reason: "cargo-machete is not installed — unused dependency detection disabled. \
                         Install with: cargo install cargo-machete"
                    .to_string(),
            });
        }
        run_machete(project_root).map_err(|message| crate::error::PassError::Failed {
            pass: "dependencies (cargo-machete)".to_string(),
            message,
        })
    }
}

fn is_machete_available() -> bool {
    process::is_cargo_subcommand_available("machete")
}

fn run_machete(project_root: &Path) -> Result<Vec<Diagnostic>, String> {
    let child = process::spawn_in_group(
        Command::new("cargo")
            .args(["machete", "--with-metadata"])
            .current_dir(project_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .map_err(|e| format!("failed to spawn cargo machete: {e}"))?;

    let result = process::run_with_timeout(child, MACHETE_TIMEOUT_SECS, MAX_OUTPUT_BYTES)?;

    if result.timed_out {
        eprintln!("Warning: cargo-machete timed out after {MACHETE_TIMEOUT_SECS}s");
        return Ok(vec![]);
    }

    // Exit code 2 = operational error
    if result.exit_code == Some(2) {
        return Err("cargo-machete encountered an error during analysis".into());
    }

    // Parse text output
    Ok(parse_machete_output(&result.stdout))
}

/// Parse cargo-machete text output into diagnostics.
///
/// Format:
/// ```text
/// package-name -- /path/to/Cargo.toml:
/// \tdep-name
/// \tanother-dep
/// ```
fn parse_machete_output(output: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut current_manifest: Option<PathBuf> = None;

    for line in output.lines() {
        // Package header: "package-name -- /path/to/Cargo.toml:"
        if line.contains(" -- ") && line.ends_with(':') {
            if let Some(path_part) = line.split(" -- ").nth(1) {
                let manifest = path_part.trim_end_matches(':').trim();
                current_manifest = Some(PathBuf::from(manifest));
            }
            continue;
        }

        // Dependency line: "\tdep-name"
        if line.starts_with('\t') {
            let dep_name = line.trim();
            if !dep_name.is_empty() {
                let file_path = current_manifest
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("Cargo.toml"));

                diagnostics.push(Diagnostic {
                    file_path,
                    rule: "unused-dependency".to_string(),
                    category: Category::Dependencies,
                    severity: Severity::Warning,
                    message: format!("Unused dependency: `{dep_name}`"),
                    help: Some(format!(
                        "Remove `{dep_name}` from [dependencies] in Cargo.toml"
                    )),
                    line: None,
                    column: None,
                    fix: None,
                });
            }
        }
    }

    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_package() {
        let output = r"cargo-machete found the following unused dependencies in /tmp/project:

my-crate -- /tmp/project/Cargo.toml:
	serde
	tokio
";
        let diags = parse_machete_output(output);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].rule, "unused-dependency");
        assert_eq!(diags[0].severity, Severity::Warning);
        assert!(diags[0].message.contains("serde"));
        assert!(diags[1].message.contains("tokio"));
        assert_eq!(diags[0].file_path, PathBuf::from("/tmp/project/Cargo.toml"));
    }

    #[test]
    fn test_parse_multiple_packages() {
        let output = r"cargo-machete found the following unused dependencies in /tmp/workspace:

crate-a -- /tmp/workspace/crate-a/Cargo.toml:
	unused-dep

crate-b -- /tmp/workspace/crate-b/Cargo.toml:
	another-unused
	yet-another
";
        let diags = parse_machete_output(output);
        assert_eq!(diags.len(), 3);
        assert_eq!(
            diags[0].file_path,
            PathBuf::from("/tmp/workspace/crate-a/Cargo.toml")
        );
        assert_eq!(
            diags[1].file_path,
            PathBuf::from("/tmp/workspace/crate-b/Cargo.toml")
        );
        assert_eq!(
            diags[2].file_path,
            PathBuf::from("/tmp/workspace/crate-b/Cargo.toml")
        );
    }

    #[test]
    fn test_parse_no_unused() {
        let output =
            "cargo-machete didn't find any unused dependencies in /tmp/project. Good job!\n";
        let diags = parse_machete_output(output);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_parse_empty_output() {
        let diags = parse_machete_output("");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_help_text() {
        let output = "pkg -- /tmp/Cargo.toml:\n\tserde\n";
        let diags = parse_machete_output(output);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].help.as_ref().unwrap().contains("Remove `serde`"));
    }

    #[test]
    #[ignore = "depends on optional external tool cargo-machete"]
    fn test_machete_availability() {
        assert!(
            is_machete_available(),
            "cargo-machete should be installed for this test"
        );
    }
}
