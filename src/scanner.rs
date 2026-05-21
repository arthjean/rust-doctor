use crate::config::ResolvedConfig;
use crate::diagnostics::Diagnostic;
use globset::{Glob, GlobSet, GlobSetBuilder};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Trait for pluggable analysis passes.
///
/// Each pass is run in parallel and returns a list of diagnostics.
/// Passes must be `Send + Sync` for parallel execution.
pub trait AnalysisPass: Send + Sync {
    /// Human-readable name of this pass (e.g. "clippy", "custom rules", "dependencies").
    fn name(&self) -> &str;

    /// Run the analysis and return diagnostics.
    /// The `project_root` is the absolute path to the project being scanned.
    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError>;

    /// Returns whether this pass can produce diagnostics for the given category filter.
    /// Defaults to `true` for backward compatibility.
    fn produces_category(&self, _filter: crate::cli::CategoryFilter) -> bool {
        true
    }
}

/// Result from a single analysis pass (internal).
struct PassResult {
    name: String,
    result: Result<Vec<Diagnostic>, crate::error::PassError>,
}

/// Result from the scan orchestrator (diagnostics + metadata, no score).
/// Score calculation happens once in main after all workspace members are merged.
pub struct ScanPassResult {
    pub diagnostics: Vec<Diagnostic>,
    pub skipped_passes: Vec<String>,
    pub elapsed: std::time::Duration,
}

/// Orchestrates multiple analysis passes in parallel and merges results.
pub struct ScanOrchestrator {
    passes: Vec<Box<dyn AnalysisPass>>,
}

impl ScanOrchestrator {
    pub fn new(passes: Vec<Box<dyn AnalysisPass>>) -> Self {
        Self { passes }
    }

    /// Run all analysis passes in parallel and return filtered diagnostics,
    /// skipped passes, and the elapsed time.
    ///
    /// `suppress_spinner` should be true for `--score` or `--json` modes.
    pub fn run(
        &self,
        project_root: &Path,
        config: &ResolvedConfig,
        suppress_spinner: bool,
    ) -> ScanPassResult {
        let start = Instant::now();

        // Create spinner
        let spinner = if suppress_spinner {
            ProgressBar::hidden()
        } else {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.cyan} {msg} [{elapsed}]")
                    .unwrap_or_else(|_| ProgressStyle::default_spinner())
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "✓"]),
            );
            pb.set_message("Scanning...");
            pb.enable_steady_tick(Duration::from_millis(100));
            pb
        };

        // Run passes in parallel using std::thread::scope
        let results = self.run_passes_parallel(project_root);

        spinner.finish_and_clear();

        // Collect diagnostics and track failures
        let mut all_diagnostics = Vec::new();
        let mut skipped_passes = Vec::new();
        let mut pass_errors = Vec::new();

        for result in results {
            match result.result {
                Ok(diagnostics) => all_diagnostics.extend(diagnostics),
                Err(crate::error::PassError::Skipped { pass, reason }) => {
                    skipped_passes.push(format!("{pass} (not installed)"));
                    eprintln!("Info: {pass}: {reason}");
                    // Emit a visible diagnostic so MCP/JSON consumers see the skip
                    all_diagnostics.push(crate::diagnostics::Diagnostic {
                        file_path: std::path::PathBuf::from("Cargo.toml"),
                        rule: "skipped-pass".to_string(),
                        category: crate::diagnostics::Category::Cargo,
                        severity: crate::diagnostics::Severity::Info,
                        message: reason,
                        help: None,
                        line: None,
                        column: None,
                        fix: None,
                    });
                }
                Err(e) => {
                    skipped_passes.push(result.name.clone());
                    pass_errors.push(format!("{}: {}", result.name, e));
                }
            }
        }

        // If all passes failed, report it
        if skipped_passes.len() == self.passes.len() && !self.passes.is_empty() {
            eprintln!("No analysis could be completed:");
            for err in &pass_errors {
                eprintln!("  - {err}");
            }
        } else if !pass_errors.is_empty() {
            for err in &pass_errors {
                eprintln!("Warning: {err}");
            }
        }

        // Filter diagnostics by config
        let filtered = filter_diagnostics(all_diagnostics, config);

        ScanPassResult {
            diagnostics: filtered,
            skipped_passes,
            elapsed: start.elapsed(),
        }
    }

    /// Run all passes in parallel using std::thread::scope.
    #[expect(
        clippy::needless_collect,
        reason = "handles must be collected before joining"
    )]
    fn run_passes_parallel(&self, project_root: &Path) -> Vec<PassResult> {
        std::thread::scope(|s| {
            let handles: Vec<_> = self
                .passes
                .iter()
                .map(|pass| {
                    let name = pass.name().to_string();
                    s.spawn(move || (name, pass.run(project_root)))
                })
                .collect();

            let pass_names: Vec<_> = self.passes.iter().map(|p| p.name().to_string()).collect();

            handles
                .into_iter()
                .enumerate()
                .map(|(i, h)| {
                    if let Ok((name, result)) = h.join() {
                        PassResult { name, result }
                    } else {
                        let name = pass_names
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| "<unknown>".to_string());
                        PassResult {
                            name: name.clone(),
                            result: Err(crate::error::PassError::Panicked { pass: name }),
                        }
                    }
                })
                .collect()
        })
    }
}

/// Filter diagnostics based on config ignore rules and ignore file patterns.
pub fn filter_diagnostics(
    diagnostics: Vec<Diagnostic>,
    config: &ResolvedConfig,
) -> Vec<Diagnostic> {
    // Build ignore rule set
    let ignored_rules: HashSet<&str> = config
        .ignore_rules
        .iter()
        .map(std::string::String::as_str)
        .collect();

    // Build ignore file glob set
    let ignore_files_set = build_glob_set(&config.ignore_files);
    if let Err(ref e) = ignore_files_set {
        eprintln!("Warning: could not build file ignore set: {e}");
    }

    diagnostics
        .into_iter()
        .filter(|d| {
            // Filter by rule name
            if ignored_rules.contains(d.rule.as_str()) {
                return false;
            }
            // Filter by file pattern
            if let Ok(ref glob_set) = ignore_files_set
                && glob_set.is_match(&d.file_path)
            {
                return false;
            }
            true
        })
        .collect()
}

/// Maximum number of glob patterns allowed in config.
const MAX_GLOB_PATTERNS: usize = 100;
/// Maximum length of a single glob pattern.
const MAX_GLOB_PATTERN_LEN: usize = 256;

/// Build a GlobSet from a list of pattern strings.
/// Caps patterns at 100 and individual pattern length at 256 chars.
/// Returns an error if any pattern is invalid.
pub fn build_glob_set(patterns: &[String]) -> Result<GlobSet, globset::Error> {
    let mut builder = GlobSetBuilder::new();

    if patterns.len() > MAX_GLOB_PATTERNS {
        eprintln!(
            "Warning: too many glob patterns ({}, max {}); truncating",
            patterns.len(),
            MAX_GLOB_PATTERNS
        );
    }

    for pattern in patterns.iter().take(MAX_GLOB_PATTERNS) {
        if pattern.len() > MAX_GLOB_PATTERN_LEN {
            eprintln!(
                "Warning: glob pattern too long ({} chars, max {}); skipping",
                pattern.len(),
                MAX_GLOB_PATTERN_LEN
            );
            continue;
        }
        match Glob::new(pattern) {
            Ok(glob) => {
                builder.add(glob);
            }
            Err(e) => {
                eprintln!("Warning: invalid glob pattern '{pattern}': {e}");
            }
        }
    }
    builder.build()
}

/// Count the number of .rs source files under a directory.
/// Uses a lightweight counter instead of collecting into a Vec.
pub fn count_source_files(root: &Path) -> usize {
    fn count_recursive(dir: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        let mut count = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if !name.starts_with('.')
                    && name != "target"
                    && name != "vendor"
                    && name != "generated"
                {
                    count += count_recursive(&path);
                }
            } else if meta.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
                count += 1;
            }
        }
        count
    }
    count_recursive(root)
}

/// Collect all `.rs` files recursively under a directory.
/// Skips hidden dirs, target, vendor, and generated directories.
/// Skips symlinks to prevent infinite loops.
pub fn collect_rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rs_files_recursive(dir, &mut files);
    files
}

/// Maximum file size to parse (10 MB). Files larger than this are skipped
/// to prevent out-of-memory crashes from pathologically large `.rs` files.
const MAX_RS_FILE_SIZE: u64 = 10 * 1024 * 1024;

fn collect_rs_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Use symlink_metadata to avoid following symlinks (prevents loops)
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            // Skip hidden dirs, target, vendor, generated, and common non-source dirs
            if !name.starts_with('.') && name != "target" && name != "vendor" && name != "generated"
            {
                collect_rs_files_recursive(&path, files);
            }
        } else if meta.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            if meta.len() > MAX_RS_FILE_SIZE {
                eprintln!(
                    "Warning: skipping oversized file {} ({} bytes)",
                    path.display(),
                    meta.len()
                );
            } else {
                files.push(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::FailOn;
    use crate::config::ResolvedConfig;
    use crate::diagnostics::{Category, Severity};

    fn make_config() -> ResolvedConfig {
        ResolvedConfig {
            ignore_rules: vec![],
            ignore_files: vec![],
            lint: true,
            dependencies: true,
            verbose: false,
            diff: None,
            fail_on: FailOn::None,
            rules_config: std::collections::HashMap::new(),
            enable_rules: vec![],
            score_fail_below: None,
            category_filter: None,
        }
    }

    fn make_diagnostic(rule: &str, file: &str, severity: Severity) -> Diagnostic {
        Diagnostic {
            file_path: file.into(),
            rule: rule.to_string(),
            category: Category::ErrorHandling,
            severity,
            message: format!("Issue: {rule}"),
            help: None,
            line: Some(1),
            column: None,
            fix: None,
        }
    }

    // --- Filter tests ---

    #[test]
    fn test_filter_no_config() {
        let diags = vec![
            make_diagnostic("rule1", "src/main.rs", Severity::Error),
            make_diagnostic("rule2", "src/lib.rs", Severity::Warning),
        ];
        let config = make_config();
        let filtered = filter_diagnostics(diags, &config);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_filter_by_rule_name() {
        let diags = vec![
            make_diagnostic("rule1", "src/main.rs", Severity::Error),
            make_diagnostic("rule2", "src/lib.rs", Severity::Warning),
        ];
        let mut config = make_config();
        config.ignore_rules = vec!["rule1".to_string()];
        let filtered = filter_diagnostics(diags, &config);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].rule, "rule2");
    }

    #[test]
    fn test_filter_by_file_pattern() {
        let diags = vec![
            make_diagnostic("rule1", "src/main.rs", Severity::Error),
            make_diagnostic("rule2", "tests/test_foo.rs", Severity::Warning),
            make_diagnostic("rule3", "tests/integration/test_bar.rs", Severity::Warning),
        ];
        let mut config = make_config();
        config.ignore_files = vec!["tests/**".to_string()];
        let filtered = filter_diagnostics(diags, &config);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].file_path.to_str().unwrap(), "src/main.rs");
    }

    #[test]
    fn test_filter_by_both_rule_and_file() {
        let diags = vec![
            make_diagnostic("rule1", "src/main.rs", Severity::Error),
            make_diagnostic("rule2", "tests/test.rs", Severity::Warning),
            make_diagnostic("rule3", "src/lib.rs", Severity::Warning),
        ];
        let mut config = make_config();
        config.ignore_rules = vec!["rule3".to_string()];
        config.ignore_files = vec!["tests/**".to_string()];
        let filtered = filter_diagnostics(diags, &config);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].rule, "rule1");
    }

    #[test]
    fn test_filter_invalid_glob_continues() {
        let diags = vec![make_diagnostic("rule1", "src/main.rs", Severity::Error)];
        let mut config = make_config();
        config.ignore_files = vec!["[invalid".to_string()];
        // Should not panic — invalid globs are warned and skipped
        let filtered = filter_diagnostics(diags, &config);
        assert_eq!(filtered.len(), 1);
    }

    // --- Orchestrator tests ---

    struct SuccessPass {
        diags: Vec<Diagnostic>,
    }

    impl AnalysisPass for SuccessPass {
        fn name(&self) -> &'static str {
            "success"
        }
        fn run(&self, _root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
            Ok(self.diags.clone())
        }
    }

    struct FailingPass;

    impl AnalysisPass for FailingPass {
        fn name(&self) -> &'static str {
            "failing"
        }
        fn run(&self, _root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
            Err(crate::error::PassError::Failed {
                pass: "failing".to_string(),
                message: "pass failed".to_string(),
            })
        }
    }

    #[test]
    fn test_orchestrator_merges_results() {
        let pass1 = SuccessPass {
            diags: vec![make_diagnostic("r1", "a.rs", Severity::Error)],
        };
        let pass2 = SuccessPass {
            diags: vec![make_diagnostic("r2", "b.rs", Severity::Warning)],
        };
        let orch = ScanOrchestrator::new(vec![Box::new(pass1), Box::new(pass2)]);
        let config = make_config();
        let result = orch.run(Path::new("."), &config, true);
        assert_eq!(result.diagnostics.len(), 2);
        let errors = result
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        let warnings = result
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count();
        assert_eq!(errors, 1);
        assert_eq!(warnings, 1);
        assert!(result.skipped_passes.is_empty());
    }

    #[test]
    fn test_orchestrator_handles_failed_pass() {
        let pass1 = SuccessPass {
            diags: vec![make_diagnostic("r1", "a.rs", Severity::Error)],
        };
        let orch = ScanOrchestrator::new(vec![Box::new(pass1), Box::new(FailingPass)]);
        let config = make_config();
        let result = orch.run(Path::new("."), &config, true);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.skipped_passes, vec!["failing"]);
    }

    #[test]
    fn test_orchestrator_all_passes_fail() {
        let orch = ScanOrchestrator::new(vec![Box::new(FailingPass), Box::new(FailingPass)]);
        let config = make_config();
        let result = orch.run(Path::new("."), &config, true);
        assert!(result.diagnostics.is_empty());
        assert_eq!(result.skipped_passes.len(), 2);
    }

    #[test]
    fn test_orchestrator_no_passes() {
        let orch = ScanOrchestrator::new(vec![]);
        let config = make_config();
        let result = orch.run(Path::new("."), &config, true);
        assert!(result.diagnostics.is_empty());
        assert!(result.skipped_passes.is_empty());
    }

    #[test]
    fn test_orchestrator_applies_config_filter() {
        let pass = SuccessPass {
            diags: vec![
                make_diagnostic("rule-to-ignore", "src/main.rs", Severity::Warning),
                make_diagnostic("rule-to-keep", "src/main.rs", Severity::Error),
            ],
        };
        let orch = ScanOrchestrator::new(vec![Box::new(pass)]);
        let mut config = make_config();
        config.ignore_rules = vec!["rule-to-ignore".to_string()];
        let result = orch.run(Path::new("."), &config, true);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].rule, "rule-to-keep");
    }

    // --- Source file counting ---

    #[test]
    fn test_count_source_files_self() {
        let count = count_source_files(Path::new(env!("CARGO_MANIFEST_DIR")));
        // rust-doctor has at least 5 .rs files (main, cli, config, discovery, diagnostics, scanner)
        assert!(count >= 6, "Expected at least 6 .rs files, found {count}");
    }

    #[test]
    fn test_count_source_files_nonexistent() {
        let count = count_source_files(Path::new("/nonexistent/path"));
        assert_eq!(count, 0);
    }
}
