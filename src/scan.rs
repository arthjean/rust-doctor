use crate::config::ResolvedConfig;
use crate::diagnostics::{Diagnostic, ScanResult, Severity};
use crate::discovery::ProjectInfo;
use crate::{
    audit, clippy, config, coverage, deny, diff, geiger, machete, msrv, output, rules, scanner,
    semver_checks, suppression, workspace,
};
use rayon::prelude::*;
use std::path::PathBuf;
use std::time::Duration;

/// Derive custom rule names from the rule registry at runtime.
/// Includes the external "unused-dependency" rule which is not AST-based.
pub fn custom_rule_names() -> Vec<String> {
    let mut names: Vec<String> = rules::all_custom_rules()
        .iter()
        .map(|r| r.name().to_string())
        .collect();
    names.push("unused-dependency".to_string());
    names
}

/// Run a complete scan on a discovered Rust project.
///
/// This is the core scanning pipeline used by both the CLI and MCP server.
/// The caller is responsible for project discovery and config resolution.
///
/// Pipeline: validate → resolve roots → run passes (parallel) → dedup → diff filter → suppress → score
pub fn scan_project(
    project_info: &ProjectInfo,
    resolved: &ResolvedConfig,
    offline: bool,
    project_filter: &[String],
    suppress_spinner: bool,
) -> Result<ScanResult, crate::error::ScanError> {
    // Step 1: Verify ignored rules are known — warns on typos in config
    validate_config(resolved);

    // Step 2: Resolve workspace members or single project root
    let scan_roots = resolve_scan_roots(project_info, resolved, project_filter)?;
    log_project_info(project_info, resolved);

    // Step 3: Parse git diff if --diff was specified (narrows scope to changed files)
    let diff_context = resolve_diff_context(project_info, resolved);

    // Step 4: Run all analysis passes in parallel
    // Two levels of parallelism: rayon for scan roots, std::thread::scope for passes within a root
    let (mut all_diagnostics, total_source_files, all_skipped_passes, total_elapsed) = run_passes(
        project_info,
        resolved,
        &scan_roots,
        diff_context.as_ref(),
        offline,
        suppress_spinner,
    );

    // Step 5: Deduplicate — same rule+file+line from overlapping workspace scans = one diagnostic
    dedup_diagnostics(&mut all_diagnostics);

    // Step 6: In diff mode, drop diagnostics outside changed files
    if let Some(ref ctx) = diff_context {
        all_diagnostics = diff::filter_to_changed_files(all_diagnostics, &ctx.changed_files);
    }

    // Step 7: Apply inline suppressions (// rust-doctor-disable-next-line <rule>)
    let all_diagnostics = apply_suppressions(all_diagnostics, project_info, resolved);

    // Step 8: Calculate score and build the final result
    Ok(build_result(
        all_diagnostics,
        total_source_files,
        all_skipped_passes,
        total_elapsed,
        resolved.category_filter,
    ))
}

// ---------------------------------------------------------------------------
// Pipeline stages
// ---------------------------------------------------------------------------

fn validate_config(resolved: &ResolvedConfig) {
    let mut known_rules: Vec<&str> = clippy::known_lint_names();
    let custom_names = custom_rule_names();
    known_rules.extend(custom_names.iter().map(String::as_str));
    config::validate_ignored_rules(&resolved.ignore_rules, &known_rules);
}

fn resolve_scan_roots(
    project_info: &ProjectInfo,
    resolved: &ResolvedConfig,
    project_filter: &[String],
) -> Result<Vec<PathBuf>, crate::error::ScanError> {
    if project_info.is_workspace {
        let members = workspace::resolve_members(&project_info.workspace_members, project_filter)?;
        if resolved.verbose {
            eprintln!(
                "Workspace: scanning {} of {} members",
                members.len(),
                project_info.member_count
            );
        }
        Ok(members.iter().map(|m| m.root_dir.clone()).collect())
    } else {
        if !project_filter.is_empty() {
            eprintln!("Warning: --project is only applicable to Cargo workspaces; ignoring");
        }
        Ok(vec![project_info.root_dir.clone()])
    }
}

fn log_project_info(project_info: &ProjectInfo, resolved: &ResolvedConfig) {
    if !resolved.verbose {
        return;
    }
    eprintln!(
        "Project: {} v{} (edition {})",
        project_info.name, project_info.version, project_info.edition
    );
    if !project_info.frameworks.is_empty() {
        let fw_list: Vec<String> = project_info
            .frameworks
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        eprintln!("Frameworks: {}", fw_list.join(", "));
    }
    if project_info.is_no_std {
        eprintln!("Mode: no_std");
    }
    if project_info.has_build_script {
        eprintln!("Build script: yes");
    }
    if let Some(ref rv) = project_info.rust_version {
        eprintln!("MSRV: {rv}");
    }
}

fn resolve_diff_context(
    project_info: &ProjectInfo,
    resolved: &ResolvedConfig,
) -> Option<diff::DiffContext> {
    let ctx = resolved.diff.as_ref().and_then(|base_hint| {
        match diff::resolve_diff(&project_info.root_dir, base_hint) {
            Ok(ctx) => Some(ctx),
            Err(e) => {
                eprintln!("Warning: {e}");
                None
            }
        }
    });

    if let Some(ref ctx) = ctx {
        eprintln!(
            "Diff mode: scanning {} changed file(s) vs {}",
            ctx.changed_files.len(),
            ctx.base,
        );
    }

    ctx
}

/// Construct the set of analysis passes based on project info and config.
/// Lint passes (clippy + custom rules) are always included when lint=true.
/// Dependency passes (audit, deny, geiger, machete, semver, coverage) run
/// only when dependencies=true and NOT in diff mode (they scan the whole project).
fn build_passes(
    project_info: &ProjectInfo,
    resolved: &ResolvedConfig,
    is_diff_mode: bool,
    offline: bool,
) -> Vec<Box<dyn scanner::AnalysisPass>> {
    let has_async_runtime = project_info.frameworks.iter().any(|f| {
        matches!(
            f,
            crate::discovery::Framework::Tokio
                | crate::discovery::Framework::AsyncStd
                | crate::discovery::Framework::Smol
        )
    });

    let mut passes: Vec<Box<dyn scanner::AnalysisPass>> = Vec::new();

    if resolved.lint {
        passes.push(Box::new(clippy::ClippyPass));
        let filtered_rules: Vec<Box<dyn rules::CustomRule>> = rules::error_handling::all_rules()
            .into_iter()
            .chain(rules::performance::all_rules())
            .chain(rules::complexity::all_rules())
            .chain(rules::security::all_rules())
            .chain(if has_async_runtime {
                rules::async_rules::all_rules()
            } else {
                vec![]
            })
            .chain(rules::framework::rules_for_frameworks(
                &project_info.frameworks,
            ))
            .filter(|rule| {
                let is_enabled = rule.default_enabled()
                    || resolved.enable_rules.contains(&rule.name().to_string());
                let matches_filter = resolved
                    .category_filter
                    .is_none_or(|cf| cf.matches(&rule.category()));
                is_enabled && matches_filter
            })
            .collect();

        if !filtered_rules.is_empty() {
            passes.push(Box::new(rules::RuleEnginePass::with_config(
                filtered_rules,
                resolved.ignore_files.clone(),
                resolved.ignore_rules.clone(),
                resolved.enable_rules.clone(),
            )));
        }
    }

    if resolved.dependencies && !is_diff_mode {
        // Prefer cargo-deny (advisory + license + ban + source checks).
        // Fall back to cargo-audit for advisory-only checks when cargo-deny
        // is not installed.
        let deny_pass = deny::DenyPass { offline };
        passes.push(Box::new(deny_pass));
        if !deny::is_cargo_deny_available() {
            passes.push(Box::new(audit::AuditPass { offline }));
        }
        passes.push(Box::new(machete::MachetePass));
        passes.push(Box::new(geiger::GeigerPass));
        passes.push(Box::new(semver_checks::SemVerPass));
        passes.push(Box::new(coverage::CoveragePass));
    }

    // MSRV validation always runs (not gated by config flags).
    passes.push(Box::new(msrv::MsrvPass {
        rust_version: project_info.rust_version.clone(),
    }));

    if let Some(cf) = resolved.category_filter {
        passes.retain(|pass| pass.produces_category(cf));
    }

    passes
}

fn run_passes(
    project_info: &ProjectInfo,
    resolved: &ResolvedConfig,
    scan_roots: &[PathBuf],
    diff_context: Option<&diff::DiffContext>,
    offline: bool,
    suppress_spinner: bool,
) -> (Vec<Diagnostic>, usize, Vec<String>, Duration) {
    let is_diff_mode = diff_context.is_some();
    let mut all_diagnostics = Vec::new();
    let mut total_source_files = 0;
    let mut all_skipped_passes = Vec::new();
    let mut total_elapsed = Duration::ZERO;

    // In diff mode, count changed files once (not per scan root)
    if let Some(ctx) = diff_context {
        total_source_files = ctx.changed_files.len();
    }

    let results: Vec<_> = scan_roots
        .par_iter()
        .map(|scan_root| {
            let source_files = if diff_context.is_none() {
                scanner::count_source_files(scan_root)
            } else {
                0
            };
            let passes = build_passes(project_info, resolved, is_diff_mode, offline);
            let orchestrator = scanner::ScanOrchestrator::new(passes);
            let pass_result = orchestrator.run(scan_root, resolved, suppress_spinner);
            (source_files, pass_result)
        })
        .collect();

    for (source_files, pass_result) in results {
        total_source_files += source_files;
        all_diagnostics.extend(pass_result.diagnostics);
        all_skipped_passes.extend(pass_result.skipped_passes);
        total_elapsed = total_elapsed.max(pass_result.elapsed); // max, not sum, since parallel
    }

    (
        all_diagnostics,
        total_source_files,
        all_skipped_passes,
        total_elapsed,
    )
}

/// Deduplicate diagnostics from overlapping workspace scans.
fn dedup_diagnostics(diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.rule.cmp(&b.rule))
            .then(a.line.cmp(&b.line))
            .then(a.column.cmp(&b.column))
            .then(a.message.cmp(&b.message))
    });
    diagnostics.dedup_by(|a, b| {
        a.file_path == b.file_path
            && a.rule == b.rule
            && a.line == b.line
            && a.column == b.column
            && a.message == b.message
    });
}

fn apply_suppressions(
    diagnostics: Vec<Diagnostic>,
    project_info: &ProjectInfo,
    resolved: &ResolvedConfig,
) -> Vec<Diagnostic> {
    let (diagnostics, suppressed_count) =
        suppression::apply_inline_suppressions(diagnostics, &project_info.root_dir);
    if resolved.verbose && suppressed_count > 0 {
        eprintln!("Suppressed {suppressed_count} diagnostic(s) via inline comments");
    }
    diagnostics
}

fn build_result(
    diagnostics: Vec<Diagnostic>,
    source_file_count: usize,
    mut skipped_passes: Vec<String>,
    elapsed: Duration,
    category_filter: Option<crate::cli::CategoryFilter>,
) -> ScanResult {
    let error_count = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warning_count = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .count();
    let info_count = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Info)
        .count();
    let (score, score_label, dimension_scores) =
        output::calculate_score(&diagnostics, category_filter);

    skipped_passes.sort();
    skipped_passes.dedup();

    ScanResult {
        diagnostics,
        score,
        score_label,
        dimension_scores,
        source_file_count,
        elapsed,
        skipped_passes,
        error_count,
        warning_count,
        info_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Category;
    use std::path::PathBuf;

    fn make_diagnostic(rule: &str, severity: Severity, line: Option<u32>) -> Diagnostic {
        Diagnostic {
            file_path: PathBuf::from("src/main.rs"),
            rule: rule.to_string(),
            category: Category::Correctness,
            severity,
            message: format!("test diagnostic for {rule}"),
            help: None,
            line,
            column: None,
            fix: None,
        }
    }

    #[test]
    fn custom_rule_names_includes_all_rules() {
        let names = custom_rule_names();
        // Must include custom AST rules + the special "unused-dependency" rule
        assert!(names.contains(&"unwrap-in-production".to_string()));
        assert!(names.contains(&"high-cyclomatic-complexity".to_string()));
        assert!(names.contains(&"hardcoded-secrets".to_string()));
        assert!(names.contains(&"unused-dependency".to_string()));
        // At least 15+ rules (5 error_handling + 5 performance + 1 complexity + 3 security + ...)
        assert!(
            names.len() >= 15,
            "Expected >= 15 rules, got {}",
            names.len()
        );
    }

    #[test]
    fn dedup_removes_duplicate_diagnostics() {
        let mut diags = vec![
            make_diagnostic("rule-a", Severity::Warning, Some(10)),
            make_diagnostic("rule-a", Severity::Warning, Some(10)), // duplicate
            make_diagnostic("rule-b", Severity::Error, Some(20)),
        ];
        dedup_diagnostics(&mut diags);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].rule, "rule-a");
        assert_eq!(diags[1].rule, "rule-b");
    }

    #[test]
    fn dedup_keeps_different_lines() {
        let mut diags = vec![
            make_diagnostic("rule-a", Severity::Warning, Some(10)),
            make_diagnostic("rule-a", Severity::Warning, Some(20)), // same rule, different line
        ];
        dedup_diagnostics(&mut diags);
        assert_eq!(diags.len(), 2);
    }

    #[test]
    fn dedup_handles_empty() {
        let mut diags: Vec<Diagnostic> = vec![];
        dedup_diagnostics(&mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn build_result_counts_severities() {
        let diags = vec![
            make_diagnostic("err1", Severity::Error, Some(1)),
            make_diagnostic("err2", Severity::Error, Some(2)),
            make_diagnostic("warn1", Severity::Warning, Some(3)),
            make_diagnostic("info1", Severity::Info, Some(4)),
        ];
        let result = build_result(diags, 10, vec![], Duration::from_secs(1), None);
        assert_eq!(result.error_count, 2);
        assert_eq!(result.warning_count, 1);
        assert_eq!(result.info_count, 1);
        assert_eq!(result.source_file_count, 10);
    }

    #[test]
    fn build_result_deduplicates_skipped_passes() {
        let skipped = vec![
            "cargo-deny".to_string(),
            "cargo-audit".to_string(),
            "cargo-deny".to_string(), // duplicate
        ];
        let result = build_result(vec![], 0, skipped, Duration::ZERO, None);
        assert_eq!(result.skipped_passes.len(), 2);
        assert_eq!(result.skipped_passes[0], "cargo-audit"); // sorted
        assert_eq!(result.skipped_passes[1], "cargo-deny");
    }

    #[test]
    fn build_result_empty_diagnostics_gives_perfect_score() {
        let result = build_result(vec![], 5, vec![], Duration::from_millis(100), None);
        assert_eq!(result.score, 100);
        assert_eq!(result.error_count, 0);
        assert_eq!(result.warning_count, 0);
        assert_eq!(result.info_count, 0);
    }

    #[test]
    fn test_build_passes_category_filtering() {
        use crate::cli::{CategoryFilter, FailOn};
        use std::collections::HashMap;

        let project_info = ProjectInfo {
            root_dir: PathBuf::from("."),
            name: "test-project".to_string(),
            version: "0.1.0".to_string(),
            edition: "2021".to_string(),
            frameworks: vec![],
            is_workspace: false,
            member_count: 1,
            has_build_script: false,
            rust_version: Some("1.80.0".to_string()),
            is_no_std: false,
            package_metadata: serde_json::Value::Null,
            workspace_members: vec![],
        };

        // Case 1: Filter = None (should include all configured passes: clippy, rules engine, dependencies, msrv)
        let resolved = ResolvedConfig {
            ignore_rules: vec![],
            ignore_files: vec![],
            lint: true,
            dependencies: true,
            verbose: false,
            diff: None,
            fail_on: FailOn::None,
            rules_config: HashMap::new(),
            enable_rules: vec![],
            score_fail_below: None,
            category_filter: None,
        };
        let passes = build_passes(&project_info, &resolved, false, false);
        assert!(!passes.is_empty());
        let names: Vec<&str> = passes.iter().map(|p| p.name()).collect();
        assert!(names.contains(&"clippy"));
        assert!(names.contains(&"custom rules"));
        assert!(names.contains(&"msrv"));
        assert!(names.contains(&"coverage"));

        // Case 2: Filter = Performance (should include clippy, rule engine only if it has performance rules, no security/dependencies/msrv passes)
        let resolved_perf = ResolvedConfig {
            ignore_rules: vec![],
            ignore_files: vec![],
            lint: true,
            dependencies: true,
            verbose: false,
            diff: None,
            fail_on: FailOn::None,
            rules_config: HashMap::new(),
            enable_rules: vec![],
            score_fail_below: None,
            category_filter: Some(CategoryFilter::Performance),
        };
        let passes_perf = build_passes(&project_info, &resolved_perf, false, false);
        let names_perf: Vec<&str> = passes_perf.iter().map(|p| p.name()).collect();
        assert!(names_perf.contains(&"clippy"));
        assert!(names_perf.contains(&"custom rules"));
        assert!(!names_perf.contains(&"msrv"));
        assert!(!names_perf.contains(&"coverage"));
        assert!(!names_perf.contains(&"cargo-deny"));

        // Case 3: Filter = Correctness (should include coverage, clippy, custom rules)
        let resolved_correctness = ResolvedConfig {
            ignore_rules: vec![],
            ignore_files: vec![],
            lint: true,
            dependencies: true,
            verbose: false,
            diff: None,
            fail_on: FailOn::None,
            rules_config: HashMap::new(),
            enable_rules: vec![],
            score_fail_below: None,
            category_filter: Some(CategoryFilter::Correctness),
        };
        let passes_corr = build_passes(&project_info, &resolved_correctness, false, false);
        let names_corr: Vec<&str> = passes_corr.iter().map(|p| p.name()).collect();
        assert!(names_corr.contains(&"clippy"));
        assert!(names_corr.contains(&"coverage"));
        assert!(!names_corr.contains(&"msrv"));
    }
}
