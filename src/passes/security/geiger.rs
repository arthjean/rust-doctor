//! cargo-geiger integration — unsafe code budget across the dependency tree.

use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::process;
use crate::scanner::AnalysisPass;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const GEIGER_TIMEOUT_SECS: u64 = 120;
const MAX_OUTPUT_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

/// cargo-geiger analysis pass — audits unsafe code usage across the dependency tree.
pub struct GeigerPass;

impl AnalysisPass for GeigerPass {
    fn name(&self) -> &'static str {
        "unsafe audit (cargo-geiger)"
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        if !is_geiger_available() {
            return Err(crate::error::PassError::Skipped {
                pass: self.name().to_string(),
                reason: "cargo-geiger is not installed — unsafe dependency auditing disabled. \
                         Install with: cargo install cargo-geiger"
                    .to_string(),
            });
        }
        run_geiger(project_root).map_err(|message| crate::error::PassError::Failed {
            pass: "unsafe audit (cargo-geiger)".to_string(),
            message,
        })
    }
}

fn is_geiger_available() -> bool {
    process::is_cargo_subcommand_available("geiger")
}

fn run_geiger(project_root: &Path) -> Result<Vec<Diagnostic>, String> {
    let child = process::spawn_in_group(
        Command::new("cargo")
            .args(["geiger"])
            .current_dir(project_root)
            .env("CARGO_TARGET_DIR", project_root.join("target/rust-doctor"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .map_err(|e| format!("failed to spawn cargo geiger: {e}"))?;

    let output = process::run_with_timeout(child, GEIGER_TIMEOUT_SECS, MAX_OUTPUT_BYTES)?;

    if output.timed_out {
        return Err(format!(
            "cargo geiger timed out after {GEIGER_TIMEOUT_SECS}s"
        ));
    }

    Ok(parse_geiger_ascii(&output.stdout))
}

/// Parse cargo-geiger ASCII output.
///
/// Each dependency line looks like:
/// `0/0  77/98  1/5  0/0  2/2  !  │ └── crate-name 1.2.3`
///
/// The columns are: Functions, Expressions, Impls, Traits, Methods.
/// Format per column: `unsafe_used/total`. The `!` means unsafe detected.
fn parse_geiger_ascii(output: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();

        // Skip header, summary, empty, and non-dependency lines
        if trimmed.is_empty()
            || trimmed.starts_with("Functions")
            || trimmed.starts_with("error")
            || trimmed.starts_with("Failed")
            || !trimmed.contains('/')
        {
            continue;
        }

        // Look for lines with `!` marker (unsafe detected) and a crate name
        if !trimmed.contains('!') {
            continue;
        }

        // Extract the crate name+version after tree-drawing characters
        let crate_part = trimmed
            .split('!')
            .nth(1)
            .unwrap_or("")
            .trim()
            .trim_start_matches(|c: char| "│├└─ ".contains(c))
            .trim();

        if crate_part.is_empty() {
            continue;
        }

        // Extract unsafe function count (first column: "N/M")
        let columns: Vec<&str> = trimmed.split_whitespace().collect();
        if columns.len() < 5 {
            continue;
        }

        // Parse first column (functions) and second (expressions) as unsafe/total
        let (Some(col_fns), Some(col_exprs)) = (columns.first(), columns.get(1)) else {
            continue;
        };
        let unsafe_fns = parse_unsafe_count(col_fns);
        let unsafe_exprs = parse_unsafe_count(col_exprs);
        let total_unsafe = unsafe_fns + unsafe_exprs;

        if total_unsafe == 0 {
            continue;
        }

        let severity = if total_unsafe > 50 {
            Severity::Warning
        } else {
            Severity::Info
        };

        diagnostics.push(Diagnostic {
            file_path: PathBuf::from("Cargo.toml"),
            rule: "unsafe-dependency".to_string(),
            category: Category::Security,
            severity,
            message: format!(
                "Dependency `{crate_part}` uses unsafe: {unsafe_fns} functions, {unsafe_exprs} expressions"
            ),
            help: Some(format!(
                "Review unsafe usage in `{crate_part}` or consider alternatives"
            )),
            line: None,
            column: None,
            fix: None,
        });
    }

    diagnostics
}

/// Parse "N/M" format, returning N (the unsafe count).
fn parse_unsafe_count(s: &str) -> u64 {
    s.split('/')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_output() {
        let diags = parse_geiger_ascii("");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_parse_ascii_with_unsafe_dependency() {
        let output = "Functions  Expressions  Impls  Traits  Methods  Dependency\n\
                       3/10       20/100       0/0    0/0     0/0      !  └── some-crate 1.0.0\n";
        let diags = parse_geiger_ascii(output);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("some-crate 1.0.0"));
        assert!(diags[0].message.contains("3 functions"));
        assert!(diags[0].message.contains("20 expressions"));
    }

    #[test]
    fn test_parse_ascii_safe_crate_no_diagnostic() {
        let output = "Functions  Expressions  Impls  Traits  Methods  Dependency\n\
                       0/50       0/200        0/0    0/0     0/0      :) └── safe-crate 0.1.0\n";
        let diags = parse_geiger_ascii(output);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_high_unsafe_count_is_warning() {
        let output = "Functions  Expressions  Impls  Traits  Methods  Dependency\n\
                       30/35      25/30        0/0    0/0     0/0      !  └── risky-crate 2.0.0\n";
        let diags = parse_geiger_ascii(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Warning);
    }

    #[test]
    fn test_parse_unsafe_count() {
        assert_eq!(parse_unsafe_count("3/10"), 3);
        assert_eq!(parse_unsafe_count("0/50"), 0);
        assert_eq!(parse_unsafe_count("invalid"), 0);
    }
}
