use crate::config::ResolvedConfig;
use crate::diagnostics::{
    AnalysisFailureReceipt, CheckState, CheckStatus, CompilerDiagnosticEvidence, Diagnostic,
};
use crate::process::{ProcessStop, ScanControl};
use globset::{Glob, GlobSet, GlobSetBuilder};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const ANALYSIS_FAILURE_RULE: &str = "rust-doctor::analysis-work-unit-failed";

pub type FileProgressCallback = std::sync::Arc<dyn Fn(FileProgressUpdate) + Send + Sync + 'static>;

const FILE_PROGRESS_TICK_INTERVAL: Duration = Duration::from_millis(50);
const FILE_PROGRESS_TEMPLATE: &str = "{spinner:.cyan} {msg}{prefix:.dim}";

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

    /// Run the pass while reporting started files, scanned files, total files,
    /// and workers.
    ///
    /// Passes without file-level progress keep the default implementation.
    fn run_with_progress(
        &self,
        project_root: &Path,
        _on_file_progress: &FileProgressCallback,
    ) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        self.run(project_root)
    }

    /// Required checks make the score non-authoritative when incomplete.
    fn required(&self) -> bool {
        true
    }

    /// Drain adapter-specific compiler evidence after `run` completes.
    fn take_compiler_evidence(&self) -> Vec<CompilerDiagnosticEvidence> {
        Vec::new()
    }
}

/// Result from a single analysis pass (internal).
struct PassResult {
    name: String,
    required: bool,
    result: Result<Vec<Diagnostic>, crate::error::PassError>,
    elapsed: std::time::Duration,
    stop: Option<ProcessStop>,
}

#[derive(Clone, Copy)]
pub struct FileProgressUpdate {
    pub started: usize,
    pub scanned: usize,
    pub total: usize,
    pub workers: usize,
}

#[derive(Clone, Copy)]
struct FileProgress {
    started: usize,
    scanned: usize,
    displayed: usize,
    total: usize,
    workers: usize,
}

impl FileProgress {
    fn from_update(update: FileProgressUpdate) -> Self {
        Self {
            started: update.started.min(update.total),
            scanned: update.scanned.min(update.total),
            displayed: update.scanned.min(update.total),
            total: update.total,
            workers: update.workers,
        }
    }

    fn observe(&mut self, update: FileProgressUpdate) -> bool {
        let previous_message_state = (self.displayed, self.total, self.workers);
        self.total = update.total;
        self.workers = update.workers;
        self.started = self.started.max(update.started).min(self.total);
        self.scanned = self.scanned.max(update.scanned).min(self.total);
        self.displayed = self.displayed.max(self.scanned).min(self.total);
        previous_message_state != (self.displayed, self.total, self.workers)
    }

    fn tick(&mut self) -> bool {
        let ceiling = self.started.min(self.total.saturating_sub(1));
        if self.displayed >= ceiling {
            return false;
        }
        self.displayed += 1;
        true
    }
}

/// Result from the scan orchestrator (diagnostics + metadata, no score).
/// Score calculation happens once in main after all workspace members are merged.
#[derive(Default)]
pub struct ScanPassResult {
    pub diagnostics: Vec<Diagnostic>,
    pub skipped_passes: Vec<String>,
    pub elapsed: std::time::Duration,
    pub pass_timings: Vec<(String, std::time::Duration)>,
    pub compiler_evidence: Vec<CompilerDiagnosticEvidence>,
    pub checks: Vec<CheckState>,
    pub analysis_failures: Vec<AnalysisFailureReceipt>,
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
    #[cfg(test)]
    pub fn run(
        &self,
        project_root: &Path,
        config: &ResolvedConfig,
        suppress_spinner: bool,
    ) -> ScanPassResult {
        self.run_controlled(
            project_root,
            config,
            suppress_spinner,
            &ScanControl::unlimited(),
        )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "pass outcomes, partial diagnostics, check states, and compiler evidence are reduced at one orchestration boundary"
    )]
    pub fn run_controlled(
        &self,
        project_root: &Path,
        config: &ResolvedConfig,
        suppress_spinner: bool,
        control: &ScanControl,
    ) -> ScanPassResult {
        let start = Instant::now();
        tracing::debug!(passes = self.passes.len(), "starting scan passes");

        let progress_enabled = !suppress_spinner && !self.passes.is_empty();
        let progress = if progress_enabled {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template(FILE_PROGRESS_TEMPLATE)
                    .unwrap_or_else(|_| ProgressStyle::default_spinner())
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            );
            pb.set_message("Scanning...");
            pb.enable_steady_tick(Duration::from_millis(100));
            pb
        } else {
            ProgressBar::hidden()
        };

        // Run passes in parallel using std::thread::scope
        let (results, file_progress) =
            self.run_passes_parallel(project_root, control, &progress, progress_enabled);

        if progress_enabled {
            let required_pass_failed = results
                .iter()
                .any(|result| result.required && (result.stop.is_some() || result.result.is_err()));
            finish_scan_progress(
                &progress,
                file_progress,
                start.elapsed(),
                !required_pass_failed,
            );
        }

        // Collect diagnostics and track failures
        let mut all_diagnostics = Vec::new();
        let mut skipped_passes = Vec::new();
        let mut pass_errors = Vec::new();
        let mut pass_timings = Vec::new();
        let mut checks = Vec::new();
        let mut analysis_failures = Vec::new();

        for result in results {
            pass_timings.push((result.name.clone(), result.elapsed));
            let stopped_status = result.stop.map(|stop| match stop {
                ProcessStop::TimedOut => CheckStatus::TimedOut,
                ProcessStop::Cancelled => CheckStatus::Cancelled,
            });
            match result.result {
                Ok(mut diagnostics) => {
                    analysis_failures.extend(
                        diagnostics
                            .iter()
                            .filter(|diagnostic| diagnostic.rule == ANALYSIS_FAILURE_RULE)
                            .map(|diagnostic| analysis_failure_receipt(&result.name, diagnostic)),
                    );
                    let work_failures: Vec<_> = diagnostics
                        .iter()
                        .filter(|diagnostic| diagnostic.rule == ANALYSIS_FAILURE_RULE)
                        .map(|diagnostic| diagnostic.message.clone())
                        .collect();
                    diagnostics.retain(|diagnostic| diagnostic.rule != ANALYSIS_FAILURE_RULE);
                    let compiler_failed =
                        result.name == "clippy" && diagnostics.iter().any(is_compiler_failure);
                    all_diagnostics.extend(diagnostics);
                    let work_units_failed = !work_failures.is_empty();
                    let status =
                        stopped_status.unwrap_or(if compiler_failed || work_units_failed {
                            CheckStatus::Failed
                        } else {
                            CheckStatus::Completed
                        });
                    let reason = match status {
                        CheckStatus::TimedOut => Some("scan deadline reached".to_string()),
                        CheckStatus::Cancelled => Some("scan cancellation requested".to_string()),
                        CheckStatus::Failed if compiler_failed => {
                            Some("project compilation failed".to_string())
                        }
                        CheckStatus::Failed if work_units_failed => {
                            Some(summarize_work_failures(&work_failures))
                        }
                        _ => None,
                    };
                    if let Some(reason) = &reason {
                        skipped_passes.push(format!(
                            "{}: {}: {reason}",
                            result.name,
                            check_status_name(status)
                        ));
                    }
                    checks.push(CheckState {
                        name: result.name,
                        required: result.required,
                        status,
                        reason,
                    });
                }
                Err(crate::error::PassError::Skipped { pass, reason }) => {
                    skipped_passes.push(format!("{pass}: skipped: {reason}"));
                    eprintln!("Info: {pass}: {reason}");
                    checks.push(CheckState {
                        name: result.name,
                        required: result.required,
                        status: CheckStatus::Skipped,
                        reason: Some(reason.clone()),
                    });
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
                Err(crate::error::PassError::TimedOut { pass, reason }) => {
                    skipped_passes.push(format!("{pass}: timed out: {reason}"));
                    checks.push(CheckState {
                        name: result.name,
                        required: result.required,
                        status: CheckStatus::TimedOut,
                        reason: Some(reason),
                    });
                }
                Err(crate::error::PassError::Cancelled { pass, reason }) => {
                    skipped_passes.push(format!("{pass}: cancelled: {reason}"));
                    checks.push(CheckState {
                        name: result.name,
                        required: result.required,
                        status: CheckStatus::Cancelled,
                        reason: Some(reason),
                    });
                }
                Err(e) => {
                    let error = format!("{}: failed: {e}", result.name);
                    skipped_passes.push(error.clone());
                    pass_errors.push(error);
                    checks.push(CheckState {
                        name: result.name,
                        required: result.required,
                        status: CheckStatus::Failed,
                        reason: Some(e.to_string()),
                    });
                }
            }
        }
        let compiler_evidence = self
            .passes
            .iter()
            .flat_map(|pass| pass.take_compiler_evidence())
            .collect();

        // If all passes failed, report it
        if checks
            .iter()
            .all(|check| check.status != CheckStatus::Completed)
            && !self.passes.is_empty()
        {
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

        tracing::debug!(
            diagnostics = filtered.len(),
            skipped = skipped_passes.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "scan passes complete"
        );

        ScanPassResult {
            diagnostics: filtered,
            skipped_passes,
            elapsed: start.elapsed(),
            pass_timings,
            compiler_evidence,
            checks,
            analysis_failures,
        }
    }

    /// Run all passes in parallel using std::thread::scope.
    #[expect(
        clippy::needless_collect,
        reason = "handles must be collected before joining"
    )]
    fn run_passes_parallel(
        &self,
        project_root: &Path,
        control: &ScanControl,
        progress: &ProgressBar,
        progress_enabled: bool,
    ) -> (Vec<PassResult>, Option<FileProgress>) {
        std::thread::scope(|s| {
            let pass_names: Vec<_> = self
                .passes
                .iter()
                .map(|pass| pass.name().to_string())
                .collect();
            let (progress_sender, progress_receiver) = std::sync::mpsc::channel();
            let handles: Vec<_> = self
                .passes
                .iter()
                .map(|pass| {
                    let name = pass.name().to_string();
                    let required = pass.required();
                    let control = control.clone();
                    let progress_sender = progress_sender.clone();
                    s.spawn(move || {
                        let start = Instant::now();
                        let _ = crate::process::take_process_stop();
                        let result = match control.stop_reason() {
                            Some(ProcessStop::TimedOut) => Err(crate::error::PassError::TimedOut {
                                pass: name.clone(),
                                reason: "scan deadline reached before launch".to_string(),
                            }),
                            Some(ProcessStop::Cancelled) => {
                                Err(crate::error::PassError::Cancelled {
                                    pass: name.clone(),
                                    reason: "scan cancellation requested before launch".to_string(),
                                })
                            }
                            None => crate::process::with_scan_control(control.clone(), || {
                                if progress_enabled {
                                    let file_progress_sender = progress_sender.clone();
                                    let on_file_progress: FileProgressCallback =
                                        std::sync::Arc::new(move |update| {
                                            let _ = file_progress_sender.send(update);
                                        });
                                    pass.run_with_progress(project_root, &on_file_progress)
                                } else {
                                    pass.run(project_root)
                                }
                            }),
                        };
                        let stop =
                            crate::process::take_process_stop().or_else(|| control.stop_reason());
                        let elapsed = start.elapsed();
                        (name, required, result, elapsed, stop)
                    })
                })
                .collect();
            drop(progress_sender);

            let file_progress = track_file_progress(&progress_receiver, progress);

            let results = handles
                .into_iter()
                .enumerate()
                .map(|(i, h)| {
                    if let Ok((name, required, result, elapsed, stop)) = h.join() {
                        tracing::debug!(pass = %name, elapsed_ms = elapsed.as_millis(), "pass complete");
                        PassResult {
                            name,
                            required,
                            result,
                            elapsed,
                            stop,
                        }
                    } else {
                        let name = pass_names
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| "<unknown>".to_string());
                        PassResult {
                            name: name.clone(),
                            required: true,
                            result: Err(crate::error::PassError::Panicked { pass: name }),
                            elapsed: Duration::ZERO,
                            stop: None,
                        }
                    }
                })
                .collect();
            (results, file_progress)
        })
    }
}

fn track_file_progress(
    progress_receiver: &std::sync::mpsc::Receiver<FileProgressUpdate>,
    progress: &ProgressBar,
) -> Option<FileProgress> {
    let mut file_progress: Option<FileProgress> = None;
    let mut next_progress_tick = Instant::now() + FILE_PROGRESS_TICK_INTERVAL;
    loop {
        let now = Instant::now();
        if now >= next_progress_tick {
            if let Some(current) = file_progress.as_mut()
                && current.tick()
            {
                render_file_progress(progress, *current);
            }
            next_progress_tick = now + FILE_PROGRESS_TICK_INTERVAL;
        }

        let wait = next_progress_tick.saturating_duration_since(Instant::now());
        match progress_receiver.recv_timeout(wait) {
            Ok(update) => {
                if let Some(current) = file_progress.as_mut() {
                    if current.observe(update) {
                        render_file_progress(progress, *current);
                    }
                } else {
                    let current = FileProgress::from_update(update);
                    if current.displayed > 0 {
                        render_file_progress(progress, current);
                    }
                    file_progress = Some(current);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    file_progress
}

fn render_file_progress(progress: &ProgressBar, files: FileProgress) {
    progress.set_prefix(worker_count_suffix(files.workers));
    progress.set_message(scan_files_message(files));
}

fn scan_files_message(progress: FileProgress) -> String {
    format!(
        "Scanning files ({}/{})...",
        progress.displayed, progress.total
    )
}

fn finish_scan_progress(
    progress: &ProgressBar,
    file_progress: Option<FileProgress>,
    elapsed: Duration,
    succeeded: bool,
) {
    let (symbol, color, message, prefix) = if succeeded {
        file_progress.map_or_else(
            || {
                (
                    "✔",
                    "green",
                    format!("Scan completed in {:.1}s", elapsed.as_secs_f64()),
                    String::new(),
                )
            },
            |files| {
                (
                    "✔",
                    "green",
                    format!(
                        "Scanned {} in {:.1}s",
                        file_count_label(files.scanned),
                        elapsed.as_secs_f64()
                    ),
                    worker_count_suffix(files.workers),
                )
            },
        )
    } else {
        (
            "✖",
            "red",
            format!("Scanning failed after {:.1}s", elapsed.as_secs_f64()),
            String::new(),
        )
    };
    progress.set_prefix(prefix);
    let template = format!("{{spinner:.{color}}} {{msg}}{{prefix:.dim}}");
    progress.set_style(
        ProgressStyle::default_spinner()
            .template(&template)
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_strings(&[symbol]),
    );
    progress.finish_with_message(message);
}

fn file_count_label(count: usize) -> String {
    let noun = if count == 1 { "file" } else { "files" };
    format!("{count} {noun}")
}

fn worker_count_suffix(workers: usize) -> String {
    if workers > 1 {
        format!(" [~{workers} workers]")
    } else {
        String::new()
    }
}

fn analysis_failure_receipt(check: &str, diagnostic: &Diagnostic) -> AnalysisFailureReceipt {
    let (kind, detail) = diagnostic
        .message
        .split_once(':')
        .unwrap_or(("analysis_failed", diagnostic.message.as_str()));
    let (rule, reason) = if kind == "rule_panicked" {
        detail
            .split_once(':')
            .map_or((None, detail), |(rule, reason)| {
                (Some(rule.to_string()), reason)
            })
    } else {
        (None, detail)
    };
    AnalysisFailureReceipt {
        check: check.to_string(),
        kind: kind.to_string(),
        path: diagnostic.file_path.to_string_lossy().replace('\\', "/"),
        rule,
        reason: reason.to_string(),
    }
}

fn is_compiler_failure(diagnostic: &Diagnostic) -> bool {
    diagnostic.severity == crate::diagnostics::Severity::Error
        && (matches!(diagnostic.rule.as_str(), "compiler-error" | "compiler-ice")
            || diagnostic.rule.strip_prefix('E').is_some_and(|code| {
                !code.is_empty() && code.bytes().all(|byte| byte.is_ascii_digit())
            }))
}

fn summarize_work_failures(failures: &[String]) -> String {
    let mut reason = failures
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    if failures.len() > 8 {
        use std::fmt::Write;
        let _ = write!(
            reason,
            "; and {} more failed work units",
            failures.len() - 8
        );
    }
    reason
}

const fn check_status_name(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Planned => "planned",
        CheckStatus::Running => "running",
        CheckStatus::Completed => "completed",
        CheckStatus::Skipped => "skipped",
        CheckStatus::Failed => "failed",
        CheckStatus::TimedOut => "timed out",
        CheckStatus::Cancelled => "cancelled",
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

    diagnostics
        .into_iter()
        .filter(|d| {
            // Filter by rule name
            if ignored_rules.contains(d.rule.as_str())
                && d.category != crate::diagnostics::Category::Security
            {
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

/// Build a GlobSet from a list of pattern strings.
///
/// Limits are validated once at the configuration boundary. This builder must
/// never truncate or skip a pattern because doing so would silently widen the
/// scan after configuration was accepted.
pub fn build_glob_set(patterns: &[String]) -> Result<GlobSet, globset::Error> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    builder.build()
}

/// Count the number of .rs source files under a directory.
/// Uses a lightweight counter instead of collecting into a Vec.
#[cfg(test)]
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
                if !name.starts_with('.') && name != "target" && name != "vendor" {
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
            // Generated Rust remains planned work. Policy controls its consumer
            // surfaces, but collection cannot silently erase a valid Cargo target.
            if !name.starts_with('.') && name != "target" && name != "vendor" {
                collect_rs_files_recursive(&path, files);
            }
        } else if meta.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
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
            category_config: std::collections::HashMap::new(),
            tag_config: std::collections::HashMap::new(),
            path_overrides: vec![],
            enable_rules: vec![],
            score_fail_below: None,
            respect_inline_disables: true,
            max_parallelism: None,
            adapter_policy: crate::config::AdapterPolicy::default(),
            evaluation_profile: false,
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

    #[test]
    fn scan_progress_matches_react_doctor_file_phases() {
        let files = FileProgress::from_update(FileProgressUpdate {
            started: 12,
            scanned: 12,
            total: 86,
            workers: 8,
        });
        assert_eq!(scan_files_message(files), "Scanning files (12/86)...");
        assert_eq!(worker_count_suffix(files.workers), " [~8 workers]");
    }

    #[test]
    fn scan_progress_creeps_toward_started_files_and_snaps_to_scanned_files() {
        let mut files = FileProgress::from_update(FileProgressUpdate {
            started: 8,
            scanned: 0,
            total: 10,
            workers: 4,
        });

        assert!(files.tick());
        assert_eq!(files.displayed, 1);

        files.observe(FileProgressUpdate {
            started: 8,
            scanned: 5,
            total: 10,
            workers: 4,
        });
        assert_eq!(files.displayed, 5);

        files.observe(FileProgressUpdate {
            started: 10,
            scanned: 10,
            total: 10,
            workers: 4,
        });
        assert_eq!(files.displayed, 10);
        assert!(!files.tick());
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
    fn test_invalid_glob_is_rejected_without_truncation() {
        assert!(build_glob_set(&["[invalid".to_string()]).is_err());
        let patterns: Vec<_> = (0..101).map(|index| format!("path-{index}/**")).collect();
        let set = build_glob_set(&patterns).unwrap();
        assert!(set.is_match("path-100/file.rs"));
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

    struct WorkFailurePass;

    impl AnalysisPass for WorkFailurePass {
        fn name(&self) -> &'static str {
            "custom rules"
        }

        fn run(&self, _root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
            Ok(vec![Diagnostic {
                file_path: PathBuf::from("src/lib.rs"),
                rule: ANALYSIS_FAILURE_RULE.to_string(),
                category: Category::Correctness,
                severity: Severity::Info,
                message: "rule_panicked:ffi-cstring-lifetime:synthetic panic".to_string(),
                help: None,
                line: None,
                column: None,
                fix: None,
            }])
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
        assert_eq!(
            result.skipped_passes,
            vec!["failing: failed: failing: pass failed"]
        );
    }

    #[test]
    fn failed_rule_work_unit_is_structured_and_marks_the_check_failed() {
        let orchestrator = ScanOrchestrator::new(vec![Box::new(WorkFailurePass)]);
        let result = orchestrator.run(Path::new("."), &make_config(), true);

        assert_eq!(result.checks[0].status, CheckStatus::Failed);
        assert_eq!(
            result.analysis_failures,
            vec![AnalysisFailureReceipt {
                check: "custom rules".to_string(),
                kind: "rule_panicked".to_string(),
                path: "src/lib.rs".to_string(),
                rule: Some("ffi-cstring-lifetime".to_string()),
                reason: "synthetic panic".to_string(),
            }]
        );
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
