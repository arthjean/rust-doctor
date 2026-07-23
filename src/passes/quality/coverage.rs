use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::scanner::AnalysisPass;
use std::path::{Path, PathBuf};

/// Coverage report analysis pass — reads existing LCOV coverage reports.
///
/// This pass does NOT run tests or generate coverage data. It only reads
/// pre-existing coverage reports from common locations and emits diagnostics
/// for low or missing coverage.
pub struct CoveragePass;

/// Locations to search for LCOV coverage reports, in priority order.
const LCOV_SEARCH_PATHS: &[&str] = &[
    "target/coverage/lcov.info",
    "lcov.info",
    "coverage/lcov.info",
    "target/llvm-cov/lcov.info",
];

impl AnalysisPass for CoveragePass {
    fn name(&self) -> &'static str {
        "coverage"
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        let lcov_path = find_lcov_report(project_root);

        let Some(lcov_path) = lcov_path else {
            return Err(crate::error::PassError::Skipped {
                pass: self.name().to_string(),
                reason: "No coverage report found. Generate one with: \
                         cargo llvm-cov --lcov --output-path target/coverage/lcov.info"
                    .to_string(),
            });
        };

        let content =
            std::fs::read_to_string(&lcov_path).map_err(|e| crate::error::PassError::Failed {
                pass: self.name().to_string(),
                message: format!(
                    "failed to read coverage report '{}': {e}",
                    lcov_path.display()
                ),
            })?;

        let file_records = parse_lcov(&content);

        if file_records.is_empty() {
            return Err(crate::error::PassError::Skipped {
                pass: self.name().to_string(),
                reason: "Coverage report is empty or contains no file records".to_string(),
            });
        }

        let mut diagnostics = Vec::new();

        // Compute overall coverage
        let total_lines_hit: u64 = file_records.iter().map(|r| r.lines_hit).sum();
        let total_lines: u64 = file_records.iter().map(|r| r.lines_total).sum();

        if total_lines > 0 {
            let coverage_pct = total_lines_hit as f64 / total_lines as f64 * 100.0;

            if coverage_pct < 50.0 {
                diagnostics.push(Diagnostic {
                    file_path: PathBuf::from("Cargo.toml"),
                    rule: "low-coverage".to_string(),
                    category: Category::Correctness,
                    severity: Severity::Warning,
                    message: format!(
                        "Overall test coverage is {coverage_pct:.1}% ({total_lines_hit}/{total_lines} lines) — below 50% threshold"
                    ),
                    help: Some(
                        "Increase test coverage. Generate a report with: \
                         cargo llvm-cov --lcov --output-path target/coverage/lcov.info"
                            .to_string(),
                    ),
                    line: None,
                    column: None,
                    fix: None,
                });
            }
        }

        // Emit info diagnostics for completely uncovered files.
        // Skip binary entry points (main.rs, bin/*.rs) — they are structurally
        // unreachable by `cargo test` and would always report 0% coverage.
        for record in &file_records {
            if record.lines_hit == 0
                && record.lines_total > 0
                && !is_binary_entry_point(&record.source_file)
            {
                diagnostics.push(Diagnostic {
                    file_path: PathBuf::from(&record.source_file),
                    rule: "uncovered-file".to_string(),
                    category: Category::Correctness,
                    severity: Severity::Info,
                    message: format!(
                        "File has 0% test coverage ({} lines not covered)",
                        record.lines_total
                    ),
                    help: Some("Add tests covering this file".to_string()),
                    line: None,
                    column: None,
                    fix: None,
                });
            }
        }

        Ok(diagnostics)
    }
}

/// Returns `true` if the path is a binary entry point that `cargo test` cannot cover.
fn is_binary_entry_point(path: &str) -> bool {
    let p = std::path::Path::new(path);
    let file_name = p.file_name().and_then(|f| f.to_str()).unwrap_or("");
    if file_name == "main.rs" {
        return true;
    }
    // Also skip files under src/bin/ or bin/
    p.components().any(|c| c.as_os_str() == "bin")
}

/// A parsed record for a single source file in an LCOV report.
#[derive(Debug, PartialEq)]
struct LcovFileRecord {
    source_file: String,
    lines_hit: u64,
    lines_total: u64,
}

/// Find the first existing LCOV report in the standard search locations.
fn find_lcov_report(project_root: &Path) -> Option<PathBuf> {
    for relative in LCOV_SEARCH_PATHS {
        let candidate = project_root.join(relative);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Parse LCOV format into per-file coverage records.
///
/// LCOV is a line-based format where each source file block looks like:
/// ```text
/// SF:/path/to/file.rs
/// ...
/// LH:42
/// LF:100
/// end_of_record
/// ```
fn parse_lcov(content: &str) -> Vec<LcovFileRecord> {
    let mut records = Vec::new();
    let mut current_file: Option<String> = None;
    let mut lines_hit: u64 = 0;
    let mut lines_total: u64 = 0;

    for line in content.lines() {
        let line = line.trim();

        if let Some(path) = line.strip_prefix("SF:") {
            // Start a new file record
            current_file = Some(path.to_string());
            lines_hit = 0;
            lines_total = 0;
        } else if let Some(val) = line.strip_prefix("LH:") {
            if let Ok(n) = val.trim().parse::<u64>() {
                lines_hit = n;
            }
        } else if let Some(val) = line.strip_prefix("LF:") {
            if let Ok(n) = val.trim().parse::<u64>() {
                lines_total = n;
            }
        } else if line == "end_of_record" {
            if let Some(sf) = current_file.take() {
                records.push(LcovFileRecord {
                    source_file: sf,
                    lines_hit,
                    lines_total,
                });
            }
            lines_hit = 0;
            lines_total = 0;
        }
    }

    records
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── LCOV parsing ────────────────────────────────────────────────────

    #[test]
    fn test_parse_valid_lcov_multiple_files() {
        let lcov = "\
SF:src/main.rs
FN:1,main
FNDA:1,main
FNF:1
FNH:1
DA:1,1
DA:2,1
DA:3,0
LH:2
LF:3
end_of_record
SF:src/lib.rs
DA:1,1
DA:2,1
DA:3,1
DA:4,1
LH:4
LF:4
end_of_record
SF:src/utils.rs
DA:1,0
DA:2,0
LH:0
LF:2
end_of_record
";
        let records = parse_lcov(lcov);
        assert_eq!(records.len(), 3);

        assert_eq!(records[0].source_file, "src/main.rs");
        assert_eq!(records[0].lines_hit, 2);
        assert_eq!(records[0].lines_total, 3);

        assert_eq!(records[1].source_file, "src/lib.rs");
        assert_eq!(records[1].lines_hit, 4);
        assert_eq!(records[1].lines_total, 4);

        assert_eq!(records[2].source_file, "src/utils.rs");
        assert_eq!(records[2].lines_hit, 0);
        assert_eq!(records[2].lines_total, 2);
    }

    #[test]
    fn test_parse_empty_content_returns_no_records() {
        let records = parse_lcov("");
        assert!(records.is_empty());

        let records = parse_lcov("   \n\n  ");
        assert!(records.is_empty());
    }

    // ── Pass behaviour: missing report → Skipped ────────────────────────

    #[test]
    fn test_missing_report_returns_skipped() {
        let pass = CoveragePass;
        let tmp = tempfile::tempdir().unwrap();

        let result = pass.run(tmp.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::error::PassError::Skipped { pass, reason } => {
                assert_eq!(pass, "coverage");
                assert!(
                    reason.contains("No coverage report found"),
                    "unexpected reason: {reason}"
                );
            }
            other => panic!("expected Skipped, got: {other:?}"),
        }
    }

    // ── Low coverage triggers warning ──────────────────────────────────

    #[test]
    fn test_low_coverage_emits_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let coverage_dir = tmp.path().join("target/coverage");
        std::fs::create_dir_all(&coverage_dir).unwrap();
        std::fs::write(
            coverage_dir.join("lcov.info"),
            "\
SF:src/main.rs
LH:10
LF:100
end_of_record
",
        )
        .unwrap();

        let pass = CoveragePass;
        let diagnostics = pass.run(tmp.path()).unwrap();

        let warnings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.rule == "low-coverage")
            .collect();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].severity, Severity::Warning);
        assert!(
            warnings[0].message.contains("10.0%"),
            "message should contain the percentage: {}",
            warnings[0].message
        );
    }

    // ── Uncovered file triggers info diagnostic ────────────────────────

    #[test]
    fn test_zero_coverage_file_emits_info() {
        let tmp = tempfile::tempdir().unwrap();
        let coverage_dir = tmp.path().join("target/coverage");
        std::fs::create_dir_all(&coverage_dir).unwrap();
        std::fs::write(
            coverage_dir.join("lcov.info"),
            "\
SF:src/covered.rs
LH:50
LF:100
end_of_record
SF:src/empty.rs
LH:0
LF:30
end_of_record
",
        )
        .unwrap();

        let pass = CoveragePass;
        let diagnostics = pass.run(tmp.path()).unwrap();

        let uncovered: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.rule == "uncovered-file")
            .collect();
        assert_eq!(uncovered.len(), 1);
        assert_eq!(uncovered[0].severity, Severity::Info);
        assert_eq!(uncovered[0].file_path, PathBuf::from("src/empty.rs"));
        assert!(
            uncovered[0].message.contains("0%"),
            "message should mention 0%: {}",
            uncovered[0].message
        );
    }

    // ── Good coverage emits no diagnostics ─────────────────────────────

    #[test]
    fn test_good_coverage_no_diagnostics() {
        let tmp = tempfile::tempdir().unwrap();
        let coverage_dir = tmp.path().join("target/coverage");
        std::fs::create_dir_all(&coverage_dir).unwrap();
        std::fs::write(
            coverage_dir.join("lcov.info"),
            "\
SF:src/main.rs
LH:80
LF:100
end_of_record
SF:src/lib.rs
LH:60
LF:100
end_of_record
",
        )
        .unwrap();

        let pass = CoveragePass;
        let diagnostics = pass.run(tmp.path()).unwrap();
        assert!(
            diagnostics.is_empty(),
            "expected no diagnostics for 70% coverage, got: {diagnostics:?}"
        );
    }

    // ── Search path priority ───────────────────────────────────────────

    #[test]
    fn test_find_lcov_report_priority() {
        let tmp = tempfile::tempdir().unwrap();

        // No report exists yet
        assert!(find_lcov_report(tmp.path()).is_none());

        // Create the lowest-priority location
        let llvm_dir = tmp.path().join("target/llvm-cov");
        std::fs::create_dir_all(&llvm_dir).unwrap();
        std::fs::write(llvm_dir.join("lcov.info"), "SF:a\nend_of_record\n").unwrap();
        let found = find_lcov_report(tmp.path()).unwrap();
        assert!(found.ends_with("target/llvm-cov/lcov.info"));

        // Create the highest-priority location — it should win
        let primary_dir = tmp.path().join("target/coverage");
        std::fs::create_dir_all(&primary_dir).unwrap();
        std::fs::write(primary_dir.join("lcov.info"), "SF:b\nend_of_record\n").unwrap();
        let found = find_lcov_report(tmp.path()).unwrap();
        assert!(found.ends_with("target/coverage/lcov.info"));
    }
}
