#![expect(
    clippy::redundant_pub_crate,
    reason = "adapter parsers and contracts are consumed by the sibling conformance module through this private crate module"
)]

//! cargo-geiger integration — unsafe code budget across the dependency tree.

use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::passes::adapter::{self, AdapterContract, EvidenceSource};
use crate::process;
use crate::scanner::AnalysisPass;
use std::path::Path;
use std::process::{Command, Stdio};

const GEIGER_TIMEOUT_SECS: u64 = 120;
const MAX_OUTPUT_BYTES: u64 = 10 * 1024 * 1024; // 10 MB

/// cargo-geiger has no stable machine format, so its tree output is parsed as
/// text. That parser is pinned by the fixtures in this module's tests
/// (US-009 AC-4).
pub(crate) const CONTRACT: AdapterContract = AdapterContract {
    pass: "unsafe audit (cargo-geiger)",
    subcommand: "geiger",
    parser_contract_version: "geiger-tree-v1",
    evidence_source: EvidenceSource::TextFixtureBacked,
};

/// cargo-geiger analysis pass — audits unsafe code usage across the dependency tree.
pub struct GeigerPass;

impl AnalysisPass for GeigerPass {
    fn name(&self) -> &'static str {
        "unsafe audit (cargo-geiger)"
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        if !is_geiger_available() {
            return Err(CONTRACT.skipped(
                "unsafe dependency auditing disabled. Install with: cargo install cargo-geiger",
            ));
        }
        run_geiger(project_root)
    }
}

fn is_geiger_available() -> bool {
    process::is_cargo_subcommand_available("geiger")
}

fn run_geiger(project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
    let child = process::spawn_in_group(
        Command::new("cargo")
            .args(["geiger"])
            .current_dir(project_root)
            .env("CARGO_TARGET_DIR", project_root.join("target/rust-doctor"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )
    .map_err(|error| CONTRACT.failed(format!("failed to spawn cargo geiger: {error}")))?;

    let output = process::run_with_timeout(child, GEIGER_TIMEOUT_SECS, MAX_OUTPUT_BYTES)
        .map_err(|error| CONTRACT.failed(error))?;
    CONTRACT.require_complete_run(&output, &[0, 1])?;

    Ok(parse_geiger_ascii(
        &output.stdout,
        &CONTRACT.provenance(&CONTRACT.tool_version()),
    ))
}

/// How a dependency's unsafe code reaches the scanned crate.
///
/// cargo-geiger draws the dependency tree with box characters; the root's
/// direct dependencies sit at depth one. The distinction matters because direct
/// exposure is the project's own choice while transitive exposure is inherited
/// (US-009 AC-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnsafeExposure {
    Direct,
    Transitive,
}

impl UnsafeExposure {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Transitive => "transitive",
        }
    }
}

/// Cargo's tree renderer indents four columns per level: a continuation
/// (`│   ` or four spaces) for each ancestor, then `└── ` or `├── ` for the
/// crate itself. One level of indentation means a direct dependency.
fn exposure_for(tree_prefix: &str) -> UnsafeExposure {
    if tree_prefix.chars().count() <= 4 {
        UnsafeExposure::Direct
    } else {
        UnsafeExposure::Transitive
    }
}

/// Parse cargo-geiger ASCII output.
///
/// Each dependency line looks like:
/// `0/0  77/98  1/5  0/0  2/2  !  │ └── crate-name 1.2.3`
///
/// The columns are: Functions, Expressions, Impls, Traits, Methods.
/// Format per column: `unsafe_used/total`. The `!` means unsafe detected.
pub(crate) fn parse_geiger_ascii(output: &str, provenance: &str) -> Vec<Diagnostic> {
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
        let after_marker = trimmed.split('!').nth(1).unwrap_or("").trim();
        let crate_part = after_marker
            .trim_start_matches(|c: char| "│├└─ ".contains(c))
            .trim();

        if crate_part.is_empty() {
            continue;
        }
        let exposure = exposure_for(&after_marker[..after_marker.len() - crate_part.len()]);

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

        diagnostics.push(adapter::project_diagnostic(
            "unsafe-dependency",
            Category::Security,
            severity,
            &format!(
                "{} unsafe exposure: `{crate_part}` uses {unsafe_fns} unsafe functions and \
                 {unsafe_exprs} unsafe expressions",
                exposure.as_str()
            ),
            Some(&format!(
                "Unsafe usage is an exposure measurement, not a confirmed vulnerability. \
                 Review `{crate_part}` or consider alternatives.\n  via {provenance}"
            )),
            "Cargo.toml",
        ));
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
        let diags = parse_geiger_ascii("", "cargo-geiger test");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_parse_ascii_with_unsafe_dependency() {
        let output = "Functions  Expressions  Impls  Traits  Methods  Dependency\n\
                       3/10       20/100       0/0    0/0     0/0      !  └── some-crate 1.0.0\n";
        let diags = parse_geiger_ascii(output, "cargo-geiger test");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("some-crate 1.0.0"));
        assert!(diags[0].message.contains("3 unsafe functions"));
        assert!(diags[0].message.contains("20 unsafe expressions"));
        assert!(diags[0].message.starts_with("direct unsafe exposure"));
    }

    #[test]
    fn unsafe_exposure_is_classified_and_never_called_a_vulnerability() {
        let output = "Functions  Expressions  Impls  Traits  Methods  Dependency\n\
                       3/10  20/100  0/0  0/0  0/0  !  ├── direct-crate 1.0.0\n\
                       1/4   5/9     0/0  0/0  0/0  !  │   └── nested-crate 0.3.0\n";
        let diags = parse_geiger_ascii(output, "cargo-geiger 0.11.7 (parser geiger-tree-v1)");
        assert_eq!(diags.len(), 2);
        assert!(diags[0].message.starts_with("direct unsafe exposure"));
        assert!(diags[1].message.starts_with("transitive unsafe exposure"));
        for diagnostic in &diags {
            // cargo-geiger measures exposure, never a confirmed defect.
            assert_eq!(diagnostic.rule, "unsafe-dependency");
            assert!(!diagnostic.message.to_lowercase().contains("vulnerab"));
            let help = diagnostic.help.as_deref().unwrap();
            assert!(help.contains("not a confirmed vulnerability"));
            assert!(help.contains("geiger-tree-v1"));
        }
    }

    #[test]
    fn test_parse_ascii_safe_crate_no_diagnostic() {
        let output = "Functions  Expressions  Impls  Traits  Methods  Dependency\n\
                       0/50       0/200        0/0    0/0     0/0      :) └── safe-crate 0.1.0\n";
        let diags = parse_geiger_ascii(output, "cargo-geiger test");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_high_unsafe_count_is_warning() {
        let output = "Functions  Expressions  Impls  Traits  Methods  Dependency\n\
                       30/35      25/30        0/0    0/0     0/0      !  └── risky-crate 2.0.0\n";
        let diags = parse_geiger_ascii(output, "cargo-geiger test");
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
