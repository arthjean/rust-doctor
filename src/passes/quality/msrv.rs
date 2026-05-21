use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::scanner::AnalysisPass;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Minimum MSRV considered "not outdated". Versions below this trigger an info diagnostic.
const MIN_RECENT_MSRV: (u64, u64) = (1, 70);

/// MSRV validation analysis pass.
///
/// Checks whether the project declares a `rust-version` (MSRV) in Cargo.toml,
/// whether the current rustc satisfies it, and whether it is unreasonably old.
pub struct MsrvPass {
    /// The declared `rust-version` from Cargo.toml, if any.
    pub rust_version: Option<String>,
}

impl AnalysisPass for MsrvPass {
    fn name(&self) -> &'static str {
        "msrv"
    }

    fn produces_category(&self, filter: crate::cli::CategoryFilter) -> bool {
        filter.matches(&Category::Cargo)
    }

    fn run(&self, _project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        let mut diagnostics = Vec::new();

        let Some(ref msrv) = self.rust_version else {
            // No MSRV declared
            diagnostics.push(Diagnostic {
                file_path: PathBuf::from("Cargo.toml"),
                rule: "missing-msrv".to_string(),
                category: Category::Cargo,
                severity: Severity::Info,
                message: "No `rust-version` field in Cargo.toml \u{2014} MSRV not declared"
                    .to_string(),
                help: Some(
                    "Add `rust-version = \"1.XX\"` to Cargo.toml for compatibility guarantees"
                        .to_string(),
                ),
                line: None,
                column: None,
                fix: None,
            });
            return Ok(diagnostics);
        };

        let Some(msrv_parsed) = parse_semver(msrv) else {
            return Ok(diagnostics);
        };

        // Check if MSRV is very old
        if msrv_parsed < MIN_RECENT_MSRV {
            diagnostics.push(Diagnostic {
                file_path: PathBuf::from("Cargo.toml"),
                rule: "msrv-outdated".to_string(),
                category: Category::Cargo,
                severity: Severity::Info,
                message: format!("Declared MSRV {msrv} is very old \u{2014} consider updating"),
                help: Some(
                    "Updating the MSRV lets you use newer language features and better diagnostics"
                        .to_string(),
                ),
                line: None,
                column: None,
                fix: None,
            });
        }

        // Check current rustc against declared MSRV
        if let Some((major, minor)) = get_rustc_version() {
            if (major, minor) < msrv_parsed {
                diagnostics.push(Diagnostic {
                    file_path: PathBuf::from("Cargo.toml"),
                    rule: "msrv-incompatible".to_string(),
                    category: Category::Cargo,
                    severity: Severity::Warning,
                    message: format!(
                        "Current rustc ({major}.{minor}) is older than declared MSRV ({msrv})"
                    ),
                    help: Some(format!(
                        "Install Rust {msrv} or newer: rustup update stable"
                    )),
                    line: None,
                    column: None,
                    fix: None,
                });
            }
        }

        Ok(diagnostics)
    }
}

/// Get the current rustc version as (major, minor). Cached for the process lifetime.
fn get_rustc_version() -> Option<(u64, u64)> {
    static VERSION: OnceLock<Option<(u64, u64)>> = OnceLock::new();
    *VERSION.get_or_init(|| {
        let output = Command::new("rustc")
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let version_str = String::from_utf8_lossy(&output.stdout);
        parse_rustc_version_output(&version_str)
    })
}

/// Parse `rustc X.Y.Z (hash date)` into (major, minor).
fn parse_rustc_version_output(output: &str) -> Option<(u64, u64)> {
    // Expected format: "rustc 1.82.0 (f6e511eec 2024-10-15)"
    let version_part = output.strip_prefix("rustc ")?.split_whitespace().next()?;
    parse_semver(version_part)
}

/// Parse a semver-like string "X.Y" or "X.Y.Z" into (major, minor).
fn parse_semver(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_msrv_emits_info() {
        let pass = MsrvPass { rust_version: None };
        let diagnostics = pass.run(Path::new(".")).unwrap();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, "missing-msrv");
        assert_eq!(diagnostics[0].severity, Severity::Info);
        assert_eq!(diagnostics[0].category, Category::Cargo);
        assert!(diagnostics[0].help.is_some());
    }

    #[test]
    fn test_parse_rustc_version_output_standard() {
        let output = "rustc 1.82.0 (f6e511eec 2024-10-15)\n";
        assert_eq!(parse_rustc_version_output(output), Some((1, 82)));
    }

    #[test]
    fn test_parse_rustc_version_output_nightly() {
        let output = "rustc 1.84.0-nightly (abc1234 2024-12-01)\n";
        assert_eq!(parse_rustc_version_output(output), Some((1, 84)));
    }

    #[test]
    fn test_parse_rustc_version_output_invalid() {
        assert_eq!(parse_rustc_version_output("not a version"), None);
        assert_eq!(parse_rustc_version_output(""), None);
    }

    #[test]
    fn test_parse_semver_full() {
        assert_eq!(parse_semver("1.70.0"), Some((1, 70)));
    }

    #[test]
    fn test_parse_semver_partial() {
        assert_eq!(parse_semver("1.70"), Some((1, 70)));
    }

    #[test]
    fn test_parse_semver_invalid() {
        assert_eq!(parse_semver("abc"), None);
        assert_eq!(parse_semver("1"), None);
        assert_eq!(parse_semver(""), None);
    }

    #[test]
    fn test_outdated_msrv_emits_info() {
        let pass = MsrvPass {
            rust_version: Some("1.56".to_string()),
        };
        let diagnostics = pass.run(Path::new(".")).unwrap();
        let outdated = diagnostics.iter().find(|d| d.rule == "msrv-outdated");
        assert!(outdated.is_some(), "Expected msrv-outdated diagnostic");
        let d = outdated.unwrap();
        assert_eq!(d.severity, Severity::Info);
        assert!(d.message.contains("1.56"));
    }

    #[test]
    fn test_recent_msrv_no_outdated_diagnostic() {
        let pass = MsrvPass {
            rust_version: Some("1.75".to_string()),
        };
        let diagnostics = pass.run(Path::new(".")).unwrap();
        assert!(
            !diagnostics.iter().any(|d| d.rule == "msrv-outdated"),
            "Should not emit msrv-outdated for recent MSRV"
        );
    }

    #[test]
    fn test_incompatible_detection_logic() {
        // This tests the comparison logic directly, not the subprocess call.
        // If current rustc is 1.80 and MSRV is 1.85, it should be incompatible.
        let current = (1u64, 80u64);
        let msrv = (1u64, 85u64);
        assert!(current < msrv, "1.80 should be less than 1.85");

        // And the reverse
        let current = (1u64, 90u64);
        let msrv = (1u64, 85u64);
        assert!(current >= msrv, "1.90 should satisfy 1.85");
    }
}
