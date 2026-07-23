#![expect(
    clippy::redundant_pub_crate,
    reason = "the private lint registry is re-exported to the sibling catalog module"
)]

mod lint_registry;

#[cfg(test)]
pub use lint_registry::known_lint_names;
pub(crate) use lint_registry::{LINT_REGISTRY, LintEntry};
use lint_registry::{is_restriction_lint, map_lint_category, resolve_severity};

use crate::catalog::AdapterProvenance;
use crate::diagnostics::{
    Category, CompilerDiagnosticEvidence, CompilerFixEvidence, CompilerMacroEvidence,
    CompilerSpanEvidence, Diagnostic, FixApplicability, Severity, SourcePosition, SourceRange,
};
use crate::scanner::AnalysisPass;
use cargo_metadata::Message;
use cargo_metadata::diagnostic::{Applicability, DiagnosticLevel, DiagnosticSpan};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Note: clippy uses a streaming parser (Message::parse_stream) so it cannot use
// the process::run_with_timeout helper which reads all stdout into a String.
// The watchdog pattern is kept inline here for that reason.

/// Timeout for clippy subprocess in seconds.
const CLIPPY_TIMEOUT_SECS: u64 = 120;

/// Restriction-group lints that must be explicitly enabled via `-W` flags
/// since they are not covered by `clippy::all`, `pedantic`, `nursery`, or `cargo`.
const RESTRICTION_LINTS: &[&str] = &[
    "clippy::unwrap_used",
    "clippy::expect_used",
    "clippy::panic",
    "clippy::indexing_slicing",
    "clippy::unwrap_in_result",
    "clippy::panic_in_result_fn",
    "clippy::exit",
    "clippy::undocumented_unsafe_blocks",
    "clippy::multiple_unsafe_ops_per_block",
    "clippy::mem_forget",
    "clippy::cognitive_complexity",
    "clippy::dbg_macro",
    "clippy::print_stdout",
    "clippy::print_stderr",
    "clippy::unimplemented",
    "clippy::unreachable",
];

/// Returns `true` if the file path looks like test code.
/// Matches: `tests/`, `test_`, `_test.rs`, and paths containing `/tests/`.
fn is_test_file(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/tests/") || s.starts_with("tests/")
}

/// Returns `true` if `line` (1-based) falls within a `#[cfg(test)]` module.
/// Uses a simple heuristic: finds the first `#[cfg(test)]` line in the file
/// and considers everything at or below it as test code.
fn is_line_in_test_module(content: &str, line: u32) -> bool {
    for (i, text) in content.lines().enumerate() {
        let trimmed = text.trim();
        if trimmed == "#[cfg(test)]" || trimmed.starts_with("#[cfg(test)]") {
            // Everything from this line onward is test code
            return line >= (i + 1) as u32;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Clippy pass implementation
// ---------------------------------------------------------------------------

/// Clippy analysis pass — runs `cargo clippy --message-format=json` and
/// converts the output to rust-doctor diagnostics.
#[derive(Default)]
pub struct ClippyPass {
    compiler_evidence: Mutex<Vec<CompilerDiagnosticEvidence>>,
}

impl AnalysisPass for ClippyPass {
    fn name(&self) -> &'static str {
        "clippy"
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        if !is_clippy_available() {
            return Err(crate::error::PassError::Skipped {
                pass: "clippy".to_string(),
                reason: "clippy is not installed — lint analysis disabled. \
                         Install with: rustup component add clippy"
                    .to_string(),
            });
        }
        let (diagnostics, evidence) =
            run_clippy(project_root).map_err(|message| crate::error::PassError::Failed {
                pass: "clippy".to_string(),
                message,
            })?;
        if let Ok(mut stored) = self.compiler_evidence.lock() {
            *stored = evidence;
        }
        Ok(diagnostics)
    }

    fn take_compiler_evidence(&self) -> Vec<CompilerDiagnosticEvidence> {
        self.compiler_evidence.lock().map_or_else(
            |_| Vec::new(),
            |mut evidence| std::mem::take(&mut *evidence),
        )
    }
}

fn is_clippy_available() -> bool {
    crate::process::is_cargo_subcommand_available("clippy")
}

/// Build the full list of `-W` flags for clippy, including group-level
/// flags and individual restriction-group lints.
fn build_clippy_warn_flags() -> Vec<String> {
    let mut flags = Vec::new();

    // Group-level flags (override #[allow] directives)
    for group in [
        "clippy::all",
        "clippy::pedantic",
        "clippy::nursery",
        "clippy::cargo",
    ] {
        flags.push("-W".to_string());
        flags.push(group.to_string());
    }

    // Individual restriction-group lints
    for lint in RESTRICTION_LINTS {
        flags.push("-W".to_string());
        flags.push((*lint).to_string());
    }

    flags
}

/// Clippy config content that allows restriction lints in test code.
const CLIPPY_TEST_ALLOW_CONFIG: &str = "\
allow-unwrap-in-tests = true\n\
allow-expect-in-tests = true\n\
allow-indexing-slicing-in-tests = true\n\
allow-panic-in-tests = true\n\
allow-print-in-tests = true\n\
allow-dbg-in-tests = true\n\
allow-useless-vec-in-tests = true\n";

/// A guard that creates a temporary `clippy.toml` on construction
/// and removes it on drop, unless the project already had one.
struct ClippyConfigGuard {
    path: Option<PathBuf>,
}

impl ClippyConfigGuard {
    /// Write a temporary `clippy.toml` into `dir`. Returns `None` if one already exists.
    fn new(dir: &Path) -> Self {
        if dir.join("clippy.toml").exists() || dir.join(".clippy.toml").exists() {
            return Self { path: None };
        }
        let config_path = dir.join("clippy.toml");
        if std::fs::write(&config_path, CLIPPY_TEST_ALLOW_CONFIG).is_ok() {
            Self {
                path: Some(config_path),
            }
        } else {
            Self { path: None }
        }
    }
}

impl Drop for ClippyConfigGuard {
    fn drop(&mut self) {
        if let Some(ref path) = self.path {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Process a single clippy compiler message into a `Diagnostic`, if applicable.
fn process_compiler_message(
    diag: &mut cargo_metadata::diagnostic::Diagnostic,
) -> Option<(Diagnostic, CompilerDiagnosticEvidence)> {
    // Filter: only process error and warning level messages
    let clippy_severity = match &diag.level {
        DiagnosticLevel::Error | DiagnosticLevel::Ice => Severity::Error,
        DiagnosticLevel::Warning => Severity::Warning,
        _ => return None,
    };
    let is_ice = diag.level == DiagnosticLevel::Ice;

    // Extract code (lint name) — take() avoids cloning
    let rule = match diag.code.take() {
        Some(code) => code.code,
        None if clippy_severity == Severity::Error => if is_ice {
            "compiler-ice"
        } else {
            "compiler-error"
        }
        .to_string(),
        None => return None,
    };

    // Extract primary span
    let primary_span = diag.spans.iter().find(|s| s.is_primary);
    let (file_path, line, column) = primary_span.map_or_else(
        || (PathBuf::from("<unknown>"), None, None),
        |span| {
            (
                PathBuf::from(&span.file_name),
                Some(span.line_start as u32),
                Some(span.column_start as u32),
            )
        },
    );

    // Apply registry: category and severity override
    let category = map_lint_category(&rule);
    let severity = resolve_severity(&rule, clippy_severity);
    let evidence = compiler_evidence(diag, &rule, &file_path, line, column);

    // Extract help: prefer children help message, fall back to rendered
    // Move fields via std::mem::take to avoid cloning
    let rendered = diag.rendered.take();
    let help = std::mem::take(&mut diag.children)
        .into_iter()
        .find(|c| c.level == DiagnosticLevel::Help)
        .map(|c| c.message)
        .or(rendered);

    let diagnostic = Diagnostic {
        file_path,
        rule,
        category,
        severity,
        message: std::mem::take(&mut diag.message),
        help,
        line,
        column,
        fix: None,
    };
    Some((diagnostic, evidence))
}

fn compiler_evidence(
    diagnostic: &cargo_metadata::diagnostic::Diagnostic,
    rule: &str,
    file_path: &Path,
    line: Option<u32>,
    column: Option<u32>,
) -> CompilerDiagnosticEvidence {
    let primary_span = diagnostic
        .spans
        .iter()
        .find(|span| span.is_primary)
        .map(span_evidence);
    let mut related_locations: Vec<_> = diagnostic
        .spans
        .iter()
        .filter(|span| !span.is_primary)
        .map(|span| {
            (
                span.label
                    .clone()
                    .unwrap_or_else(|| "related compiler span".to_string()),
                span_evidence(span),
            )
        })
        .collect();
    for child in &diagnostic.children {
        related_locations.extend(child.spans.iter().map(|span| {
            (
                span.label.clone().unwrap_or_else(|| child.message.clone()),
                span_evidence(span),
            )
        }));
    }

    let macro_expansion = diagnostic
        .spans
        .iter()
        .find(|span| span.is_primary)
        .and_then(|span| span.expansion.as_ref())
        .map(|expansion| CompilerMacroEvidence {
            macro_name: expansion.macro_decl_name.clone(),
            call_site: span_evidence(&expansion.span),
        });
    let mut suggestion_spans: Vec<&DiagnosticSpan> = diagnostic
        .spans
        .iter()
        .chain(
            diagnostic
                .children
                .iter()
                .flat_map(|child| child.spans.iter()),
        )
        .filter(|span| span.suggested_replacement.is_some())
        .collect();
    suggestion_spans.sort_by(|left, right| {
        left.file_name
            .cmp(&right.file_name)
            .then(left.byte_start.cmp(&right.byte_start))
    });
    let group_id = (suggestion_spans.len() > 1).then(|| {
        let normalized_path = file_path.to_string_lossy().replace('\\', "/");
        format!(
            "rustc:{rule}:{}:{}:{}",
            normalized_path,
            line.unwrap_or(0),
            column.unwrap_or(0)
        )
    });
    let fixes = suggestion_spans
        .into_iter()
        .filter_map(|span| {
            Some(CompilerFixEvidence {
                group_id: group_id.clone(),
                applicability: map_applicability(span.suggestion_applicability.as_ref()),
                replacement: span.suggested_replacement.clone()?,
                span: span_evidence(span),
            })
        })
        .collect();

    CompilerDiagnosticEvidence {
        provenance: if rule.starts_with("clippy::") {
            AdapterProvenance::Clippy
        } else {
            AdapterProvenance::Rustc
        },
        rule: rule.to_string(),
        message: diagnostic.message.clone(),
        file_path: file_path.to_path_buf(),
        line,
        column,
        original_level: diagnostic_level_name(diagnostic.level).to_string(),
        primary_span,
        related_locations,
        macro_expansion,
        fixes,
    }
}

fn span_evidence(span: &DiagnosticSpan) -> CompilerSpanEvidence {
    CompilerSpanEvidence {
        file_path: PathBuf::from(&span.file_name),
        range: SourceRange {
            start: SourcePosition {
                line: u32::try_from(span.line_start).unwrap_or(u32::MAX),
                column: u32::try_from(span.column_start).unwrap_or(u32::MAX),
                byte_offset: Some(span.byte_start),
            },
            end: SourcePosition {
                line: u32::try_from(span.line_end).unwrap_or(u32::MAX),
                column: u32::try_from(span.column_end).unwrap_or(u32::MAX),
                byte_offset: Some(span.byte_end),
            },
        },
    }
}

const fn map_applicability(applicability: Option<&Applicability>) -> FixApplicability {
    match applicability {
        Some(Applicability::MachineApplicable) => FixApplicability::MachineApplicable,
        Some(Applicability::HasPlaceholders) => FixApplicability::HasPlaceholders,
        Some(Applicability::MaybeIncorrect) => FixApplicability::MaybeIncorrect,
        Some(_) | None => FixApplicability::Unspecified,
    }
}

const fn diagnostic_level_name(level: DiagnosticLevel) -> &'static str {
    match level {
        DiagnosticLevel::Ice => "error: internal compiler error",
        DiagnosticLevel::Error => "error",
        DiagnosticLevel::Warning => "warning",
        DiagnosticLevel::FailureNote => "failure-note",
        DiagnosticLevel::Note => "note",
        DiagnosticLevel::Help => "help",
        _ => "unknown",
    }
}

fn unknown_level_diagnostic(
    value: &serde_json::Value,
) -> Option<(Diagnostic, CompilerDiagnosticEvidence)> {
    if value.get("reason")?.as_str()? != "compiler-message" {
        return None;
    }
    let message = value.get("message")?;
    let level = message.get("level")?.as_str()?;
    if matches!(
        level,
        "error: internal compiler error" | "error" | "warning" | "failure-note" | "note" | "help"
    ) {
        return None;
    }
    let text = message.get("message")?.as_str()?.to_string();
    let spans: Vec<DiagnosticSpan> = serde_json::from_value(message.get("spans")?.clone()).ok()?;
    let primary = spans.iter().find(|span| span.is_primary);
    let file_path = primary.map_or_else(
        || PathBuf::from("<unknown>"),
        |span| PathBuf::from(&span.file_name),
    );
    let line = primary.and_then(|span| u32::try_from(span.line_start).ok());
    let column = primary.and_then(|span| u32::try_from(span.column_start).ok());
    let diagnostic = Diagnostic {
        file_path: file_path.clone(),
        rule: "unknown-rustc-level".to_string(),
        category: Category::Correctness,
        severity: Severity::Info,
        message: text.clone(),
        help: Some(format!(
            "rustc emitted the unknown diagnostic level `{level}`"
        )),
        line,
        column,
        fix: None,
    };
    let evidence = CompilerDiagnosticEvidence {
        provenance: AdapterProvenance::Rustc,
        rule: diagnostic.rule.clone(),
        message: text,
        file_path,
        line,
        column,
        original_level: format!("unknown:{level}"),
        primary_span: primary.map(span_evidence),
        related_locations: spans
            .iter()
            .filter(|span| !span.is_primary)
            .map(|span| {
                (
                    span.label
                        .clone()
                        .unwrap_or_else(|| "related compiler span".to_string()),
                    span_evidence(span),
                )
            })
            .collect(),
        macro_expansion: primary
            .and_then(|span| span.expansion.as_ref())
            .map(|expansion| CompilerMacroEvidence {
                macro_name: expansion.macro_decl_name.clone(),
                call_site: span_evidence(&expansion.span),
            }),
        fixes: spans
            .iter()
            .filter_map(|span| {
                Some(CompilerFixEvidence {
                    group_id: None,
                    applicability: map_applicability(span.suggestion_applicability.as_ref()),
                    replacement: span.suggested_replacement.clone()?,
                    span: span_evidence(span),
                })
            })
            .collect(),
    };
    Some((diagnostic, evidence))
}

/// Build a fallback compiler-error diagnostic from stderr when the build
/// failed but no JSON error diagnostics were produced.
fn build_stderr_fallback(stderr: std::process::ChildStderr) -> Option<Diagnostic> {
    use std::io::Read;

    const MAX_STDERR_BYTES: u64 = 4 * 1024; // 4 KB
    let mut stderr_output = String::new();
    let _ = stderr
        .take(MAX_STDERR_BYTES)
        .read_to_string(&mut stderr_output);

    if stderr_output.is_empty() {
        return None;
    }

    let first_error = stderr_output
        .lines()
        .find(|l| l.starts_with("error"))
        .unwrap_or("project failed to compile");

    // Truncate to 200 chars to avoid leaking verbose internal details
    let truncated: String = if first_error.chars().count() > 200 {
        let mut s: String = first_error.chars().take(200).collect();
        s.push('\u{2026}');
        s
    } else {
        first_error.to_string()
    };

    Some(Diagnostic {
        file_path: PathBuf::from("Cargo.toml"),
        rule: "compiler-error".to_string(),
        category: Category::Correctness,
        severity: Severity::Error,
        message: truncated,
        help: Some("Run `cargo build` to see the full error output".to_string()),
        line: None,
        column: None,
        fix: None,
    })
}

/// Remove restriction-group lints originating from test code and
/// print_stdout/print_stderr lints from binary crates.
fn filter_test_and_binary_lints(diagnostics: &mut Vec<Diagnostic>, project_root: &Path) {
    // Cache file contents to avoid re-reading the same file for every diagnostic
    let mut file_cache: std::collections::HashMap<PathBuf, String> =
        std::collections::HashMap::new();

    // Drop restriction-group lints from test code
    diagnostics.retain(|d| {
        if !is_restriction_lint(&d.rule) {
            return true;
        }
        if is_test_file(&d.file_path) {
            return false;
        }
        // For source files, check if line is in a #[cfg(test)] region
        if let Some(line) = d.line {
            let abs_path = if d.file_path.is_absolute() {
                d.file_path.clone()
            } else {
                project_root.join(&d.file_path)
            };
            let content = file_cache
                .entry(abs_path.clone())
                .or_insert_with(|| std::fs::read_to_string(&abs_path).unwrap_or_default());
            if is_line_in_test_module(content, line) {
                return false;
            }
        }
        true
    });

    // Drop print_stdout/print_stderr for binary crates
    if project_root.join("src/main.rs").exists() {
        diagnostics.retain(|d| {
            !matches!(
                d.rule.as_str(),
                "clippy::print_stdout" | "clippy::print_stderr"
            )
        });
    }
}

/// Run cargo clippy and parse JSON output into diagnostics.
fn run_clippy(
    project_root: &Path,
) -> Result<(Vec<Diagnostic>, Vec<CompilerDiagnosticEvidence>), String> {
    let manifest_path = project_root.join("Cargo.toml");

    let warn_flags = build_clippy_warn_flags();

    // Write a temporary clippy.toml that allows restriction lints in test code.
    // The guard removes it when dropped (even on early return via `?`).
    let _clippy_config_guard = ClippyConfigGuard::new(project_root);

    let mut cmd = Command::new("cargo");
    // Use a dedicated target dir so analysis never contends with or clobbers the
    // project's primary `target/` (mirrors rust-analyzer's `target/rust-analyzer`).
    cmd.env("CARGO_TARGET_DIR", project_root.join("target/rust-doctor"));
    cmd.args([
        "clippy",
        "--message-format=json",
        "--all-targets",
        "--all-features",
        "--manifest-path",
    ])
    .arg(&manifest_path)
    .arg("--");

    for flag in &warn_flags {
        cmd.arg(flag);
    }

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    // Spawn as a process-group leader so the watchdog can kill the whole tree
    // (cargo + its rustc children), not just the direct child (US-008).
    let mut child = crate::process::spawn_in_group(&mut cmd)
        .map_err(|e| format!("failed to spawn cargo clippy: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or("failed to capture clippy stdout")?;
    let stderr = child.stderr.take();

    let child = Arc::new(Mutex::new(child));
    let watchdog = crate::process::ProcessWatchdog::start(
        Arc::clone(&child),
        Duration::from_secs(CLIPPY_TIMEOUT_SECS),
        crate::process::current_scan_control(),
    );

    // Parse JSON messages from clippy stdout
    let reader = BufReader::new(stdout);
    let mut diagnostics = Vec::new();
    let mut compiler_evidence = Vec::new();
    let mut build_succeeded = true;

    for line in reader.lines() {
        let Ok(line) = line else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Ok(message) = serde_json::from_value::<Message>(value.clone()) else {
            if let Some((diagnostic, evidence)) = unknown_level_diagnostic(&value) {
                diagnostics.push(diagnostic);
                compiler_evidence.push(evidence);
            }
            continue;
        };
        match message {
            Message::CompilerMessage(compiler_msg) => {
                let mut diag = compiler_msg.message;
                if let Some((diagnostic, evidence)) = process_compiler_message(&mut diag) {
                    diagnostics.push(diagnostic);
                    compiler_evidence.push(evidence);
                }
            }
            Message::BuildFinished(finished) => {
                build_succeeded = finished.success;
            }
            _ => {}
        }
    }

    let stop = watchdog.finish();
    crate::process::record_process_stop(stop);

    // Reap the child process
    if let Ok(mut c) = child.lock() {
        let _ = c.wait();
    }

    // Check if we timed out
    match stop {
        Some(crate::process::ProcessStop::TimedOut) => {
            eprintln!(
                "Warning: clippy timed out after {CLIPPY_TIMEOUT_SECS}s; reporting partial results"
            );
        }
        Some(crate::process::ProcessStop::Cancelled) => {
            eprintln!("Warning: clippy was cancelled; reporting partial results");
        }
        None => {}
    }

    // If the build failed and we got no error diagnostics from JSON,
    // capture stderr as a compiler-error diagnostic
    if !build_succeeded
        && !diagnostics.iter().any(|d| d.severity == Severity::Error)
        && let Some(stderr) = stderr
        && let Some(fallback) = build_stderr_fallback(stderr)
    {
        diagnostics.push(fallback);
    }

    filter_test_and_binary_lints(&mut diagnostics, project_root);

    Ok((diagnostics, compiler_evidence))
}

#[cfg(test)]
mod tests {
    use super::lint_registry::{LINT_REGISTRY, lookup_lint};
    use super::*;

    // --- Registry tests ---

    #[test]
    fn test_registry_has_50_plus_entries() {
        assert!(
            LINT_REGISTRY.len() >= 50,
            "Registry has {} entries, expected 50+",
            LINT_REGISTRY.len()
        );
    }

    #[test]
    fn test_registry_no_duplicate_names() {
        let names: Vec<&str> = LINT_REGISTRY.iter().map(|e| e.name).collect();
        let mut seen = std::collections::HashSet::new();
        for name in &names {
            assert!(seen.insert(name), "Duplicate lint name in registry: {name}");
        }
    }

    // --- Lookup tests ---

    #[test]
    fn test_lookup_known_lint() {
        let result = lookup_lint("clippy::unwrap_used");
        assert!(result.is_some());
        let (cat, sev, restriction) = result.unwrap();
        assert_eq!(cat, Category::ErrorHandling);
        assert_eq!(sev, Severity::Warning);
        assert!(restriction, "unwrap_used should be marked as restriction");
    }

    #[test]
    fn test_lookup_without_prefix() {
        let result = lookup_lint("unwrap_used");
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, Category::ErrorHandling);
    }

    #[test]
    fn test_lookup_unknown_lint() {
        assert!(lookup_lint("clippy::some_unknown_lint").is_none());
    }

    // --- Category mapping tests ---

    #[test]
    fn test_map_error_handling() {
        assert_eq!(
            map_lint_category("clippy::unwrap_used"),
            Category::ErrorHandling
        );
        assert_eq!(
            map_lint_category("clippy::expect_used"),
            Category::ErrorHandling
        );
        assert_eq!(map_lint_category("clippy::panic"), Category::ErrorHandling);
    }

    #[test]
    fn test_map_performance() {
        assert_eq!(
            map_lint_category("clippy::clone_on_copy"),
            Category::Performance
        );
        assert_eq!(
            map_lint_category("clippy::needless_collect"),
            Category::Performance
        );
    }

    #[test]
    fn test_map_security() {
        assert_eq!(
            map_lint_category("clippy::transmute_ptr_to_ref"),
            Category::Security
        );
        assert_eq!(
            map_lint_category("clippy::undocumented_unsafe_blocks"),
            Category::Security
        );
    }

    #[test]
    fn test_map_correctness() {
        assert_eq!(
            map_lint_category("clippy::float_cmp"),
            Category::Correctness
        );
        assert_eq!(
            map_lint_category("clippy::almost_swapped"),
            Category::Correctness
        );
        assert_eq!(map_lint_category("compiler-error"), Category::Correctness);
        assert_eq!(map_lint_category("compiler-ice"), Category::Correctness);
    }

    #[test]
    fn test_map_cargo() {
        assert_eq!(
            map_lint_category("clippy::wildcard_dependencies"),
            Category::Cargo
        );
    }

    #[test]
    fn test_map_async() {
        assert_eq!(
            map_lint_category("clippy::await_holding_lock"),
            Category::Async
        );
        assert_eq!(map_lint_category("clippy::unused_async"), Category::Async);
    }

    #[test]
    fn test_map_architecture() {
        assert_eq!(
            map_lint_category("clippy::cognitive_complexity"),
            Category::Architecture
        );
        assert_eq!(
            map_lint_category("clippy::too_many_arguments"),
            Category::Architecture
        );
    }

    #[test]
    fn test_map_style() {
        assert_eq!(map_lint_category("clippy::dbg_macro"), Category::Style);
        assert_eq!(map_lint_category("clippy::todo"), Category::Style);
    }

    #[test]
    fn test_map_unknown_falls_to_style() {
        assert_eq!(
            map_lint_category("clippy::some_unknown_lint"),
            Category::Style
        );
    }

    // --- Severity override tests ---

    #[test]
    fn test_severity_restriction_lints_are_warning() {
        // Restriction-group lints should be Warning, not Error (aligned with clippy)
        let sev = resolve_severity("clippy::unwrap_used", Severity::Warning);
        assert_eq!(sev, Severity::Warning);
        let sev = resolve_severity("clippy::expect_used", Severity::Warning);
        assert_eq!(sev, Severity::Warning);
        let sev = resolve_severity("clippy::panic", Severity::Warning);
        assert_eq!(sev, Severity::Warning);
    }

    #[test]
    fn test_severity_override_keeps_registered_warning() {
        // clone_on_copy is registered as Warning
        let sev = resolve_severity("clippy::clone_on_copy", Severity::Warning);
        assert_eq!(sev, Severity::Warning);
    }

    #[test]
    fn test_severity_unknown_lint_keeps_clippy_default() {
        let sev = resolve_severity("clippy::some_unknown_lint", Severity::Warning);
        assert_eq!(sev, Severity::Warning);
    }

    #[test]
    fn test_severity_compiler_error_always_error() {
        assert_eq!(
            resolve_severity("compiler-error", Severity::Warning),
            Severity::Error
        );
        assert_eq!(
            resolve_severity("compiler-ice", Severity::Warning),
            Severity::Error
        );
    }

    // --- Known lint names ---

    #[test]
    fn test_known_lint_names_count() {
        let names = known_lint_names();
        assert!(names.len() >= 50);
        assert!(names.contains(&"unwrap_used"));
        assert!(names.contains(&"await_holding_lock"));
    }

    // --- Restriction flags ---

    #[test]
    fn test_build_clippy_warn_flags_contains_groups() {
        let flags = build_clippy_warn_flags();
        assert!(flags.contains(&"clippy::all".to_string()));
        assert!(flags.contains(&"clippy::pedantic".to_string()));
        assert!(flags.contains(&"clippy::nursery".to_string()));
        assert!(flags.contains(&"clippy::cargo".to_string()));
    }

    #[test]
    fn test_build_clippy_warn_flags_contains_restriction_lints() {
        let flags = build_clippy_warn_flags();
        assert!(flags.contains(&"clippy::unwrap_used".to_string()));
        assert!(flags.contains(&"clippy::expect_used".to_string()));
        assert!(flags.contains(&"clippy::dbg_macro".to_string()));
    }

    // --- Restriction lint detection ---

    #[test]
    fn test_is_restriction_lint() {
        assert!(is_restriction_lint("clippy::unwrap_used"));
        assert!(is_restriction_lint("clippy::expect_used"));
        assert!(is_restriction_lint("clippy::panic"));
        assert!(is_restriction_lint("clippy::indexing_slicing"));
        assert!(is_restriction_lint("clippy::print_stdout"));
        assert!(is_restriction_lint("clippy::dbg_macro"));
        assert!(!is_restriction_lint("clippy::clone_on_copy"));
        assert!(!is_restriction_lint("clippy::almost_swapped"));
        assert!(!is_restriction_lint("clippy::some_unknown_lint"));
    }

    #[test]
    fn test_is_test_file() {
        assert!(is_test_file(Path::new("tests/integration.rs")));
        assert!(is_test_file(Path::new("/home/user/project/tests/foo.rs")));
        assert!(!is_test_file(Path::new("src/main.rs")));
        assert!(!is_test_file(Path::new("src/rules/mod.rs")));
    }

    // --- Integration ---

    #[test]
    fn test_clippy_is_available() {
        assert!(is_clippy_available());
    }

    #[test]
    fn test_run_clippy_on_self() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let result = run_clippy(manifest_dir);
        assert!(result.is_ok(), "clippy failed: {:?}", result.err());
        // Verify that diagnostics from registered lints get severity overrides
        let (diags, _) = result.unwrap();
        for d in &diags {
            if let Some((_, expected_sev, _)) = lookup_lint(&d.rule) {
                assert_eq!(
                    d.severity, expected_sev,
                    "Lint {} should have severity {:?} but got {:?}",
                    d.rule, expected_sev, d.severity
                );
            }
        }
        // Verify no restriction lints from test files survived filtering
        for d in &diags {
            if is_test_file(&d.file_path) {
                assert!(
                    !is_restriction_lint(&d.rule),
                    "Restriction lint {} should have been filtered from test file {:?}",
                    d.rule,
                    d.file_path
                );
            }
        }
    }

    #[test]
    fn unknown_rustc_level_is_retained_explicitly() {
        let value = serde_json::json!({
            "reason": "compiler-message",
            "message": {
                "message": "future compiler signal",
                "code": null,
                "level": "future-level",
                "spans": [],
                "children": [],
                "rendered": null,
                "future_additive_field": true
            }
        });
        let (diagnostic, evidence) = unknown_level_diagnostic(&value).unwrap();
        assert_eq!(diagnostic.rule, "unknown-rustc-level");
        assert_eq!(diagnostic.severity, Severity::Info);
        assert_eq!(evidence.original_level, "unknown:future-level");
    }
}
