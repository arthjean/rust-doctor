use crate::catalog::built_in_catalog;
use crate::config::{ResolvedConfig, VisibilitySurface};
use crate::diagnostics::{
    Category, CheckState, CheckStatus, CompilerDiagnosticEvidence, Diagnostic, PackageExecution,
    ScanExecution, ScanResult, Severity, SuppressionCounts,
};
use crate::discovery::ProjectInfo;
use crate::process::{ProcessStop, ScanControl};
use crate::{
    audit, clippy, config, coverage, deny, diff, geiger, machete, msrv, output, rules, scanner,
    semver_checks, suppression, workspace,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
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
    let cancel = Arc::new(AtomicBool::new(false));
    let control = ScanControl::new(cancel, None);
    let scope = scope_from_resolved_config(project_info, resolved)?;
    scan_project_scoped(
        project_info,
        resolved,
        offline,
        project_filter,
        suppress_spinner,
        &scope,
        &control,
    )
}

/// Cancellable variant of [`scan_project`].
///
/// The `cancel` flag is polled between scan-root batches and propagated to the
/// subprocess passes. When it is set (e.g. by the MCP 5-minute timeout), the scan
/// stops launching new passes and any in-flight `cargo` subprocess tree is killed
/// instead of being detached to run in the background (US-007 / US-008).
pub fn scan_project_cancellable(
    project_info: &ProjectInfo,
    resolved: &ResolvedConfig,
    offline: bool,
    project_filter: &[String],
    suppress_spinner: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<ScanResult, crate::error::ScanError> {
    let control = ScanControl::new(Arc::clone(cancel), None);
    let scope = scope_from_resolved_config(project_info, resolved)?;
    scan_project_scoped(
        project_info,
        resolved,
        offline,
        project_filter,
        suppress_spinner,
        &scope,
        &control,
    )
}

fn scope_from_resolved_config(
    project_info: &ProjectInfo,
    resolved: &ResolvedConfig,
) -> Result<diff::ScopePlan, crate::error::ScanError> {
    let Some(base) = resolved.diff.as_ref() else {
        return Ok(diff::ScopePlan::full());
    };
    let request = diff::ScopeRequest {
        reporting_scope: diff::ReportingScope::Changed,
        base: (base != "auto").then(|| base.clone()),
        files: Vec::new(),
        include_untracked: false,
    };
    diff::resolve_scope(&project_info.root_dir, &request, &resolved.ignore_files)
        .map_err(Into::into)
}

pub(crate) fn scan_project_scoped(
    project_info: &ProjectInfo,
    resolved: &ResolvedConfig,
    offline: bool,
    project_filter: &[String],
    suppress_spinner: bool,
    scope: &diff::ScopePlan,
    control: &ScanControl,
) -> Result<ScanResult, crate::error::ScanError> {
    scan_project_scoped_for_categories(
        project_info,
        resolved,
        offline,
        project_filter,
        suppress_spinner,
        scope,
        control,
        &[],
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "category scope extends the existing internal scan boundary without changing the public scan API"
)]
pub(crate) fn scan_project_scoped_for_categories(
    project_info: &ProjectInfo,
    resolved: &ResolvedConfig,
    offline: bool,
    project_filter: &[String],
    suppress_spinner: bool,
    scope: &diff::ScopePlan,
    control: &ScanControl,
    selected_categories: &[Category],
) -> Result<ScanResult, crate::error::ScanError> {
    tracing::info!(project = %project_info.name, "starting scan");

    // Step 1: verify the canonical catalog and reject unresolved policy typos.
    validate_config(resolved)?;

    // Step 2: Resolve workspace members or single project root
    let scan_roots = resolve_scan_roots(project_info, resolved, project_filter, scope)?;
    log_project_info(project_info, resolved);

    if scope.has_selected_files() && !scope.has_applicable_work() {
        return Ok(empty_scoped_result(
            project_info,
            scope,
            resolved,
            selected_categories,
        ));
    }

    // Step 4: Run all analysis passes
    // Three levels of parallelism, all OS-thread / rayon layered so rayon workers
    // never block on a join: bounded OS threads over scan roots, std::thread::scope
    // over passes within a root, rayon par_iter over files in the rule engine.
    // See the invariant comment in `run_passes` for why root-level rayon is banned.
    let mut passes_output = run_passes(
        project_info,
        resolved,
        &scan_roots,
        scope,
        offline,
        suppress_spinner,
        control,
        selected_categories,
    );

    // Step 5: Deduplicate — same rule+file+line from overlapping workspace scans = one diagnostic
    dedup_diagnostics(&mut passes_output.diagnostics);
    let line_score_candidates = (scope.reporting_scope == diff::ReportingScope::Lines)
        .then(|| passes_output.diagnostics.clone());

    passes_output.diagnostics = diff::filter_diagnostics(
        passes_output.diagnostics,
        &passes_output.compiler_evidence,
        &project_info.root_dir,
        scope,
    );

    // Step 7: Apply project policy, then inline suppressions.
    let (mut configured_diagnostics, mut suppressed_security, mut suppression_counts) =
        apply_rule_configuration(passes_output.diagnostics, resolved);
    dedup_diagnostics(&mut configured_diagnostics);
    let (mut all_diagnostics, inline_suppressions) =
        apply_suppressions(configured_diagnostics, project_info, resolved);
    suppression_counts.inline = suppression_counts
        .inline
        .saturating_add(inline_suppressions);
    retain_selected_categories(&mut all_diagnostics, selected_categories);
    retain_selected_categories(&mut suppressed_security, selected_categories);
    passes_output.compiler_evidence.retain(|evidence| {
        all_diagnostics
            .iter()
            .any(|diagnostic| evidence.matches(diagnostic))
    });

    // Step 8: Calculate score and build the final result
    let mut result = build_result(
        all_diagnostics,
        passes_output.source_file_count,
        passes_output.skipped_passes,
        passes_output.elapsed,
        passes_output.pass_timings,
        suppressed_security,
        suppression_counts,
        passes_output.planned_files,
        passes_output.analyzed_files,
        passes_output.compiler_evidence,
        passes_output.checks,
        passes_output.package_executions,
        &project_info.root_dir,
        scope,
        resolved,
        selected_categories,
    );
    result.execution.analysis_failures = passes_output.analysis_failures;
    if let Some(candidates) = line_score_candidates {
        let (mut configured, _, _) = apply_rule_configuration(candidates, resolved);
        dedup_diagnostics(&mut configured);
        let mut configured = if resolved.respect_inline_disables {
            suppression::apply_inline_suppressions(configured, &project_info.root_dir).0
        } else {
            configured
        };
        retain_selected_categories(&mut configured, selected_categories);
        let score_diagnostics = score_visible_diagnostics(&configured, resolved);
        let (health_score, label, dimensions) = recalculate_package_scores_and_select_headline(
            &mut result.execution.packages,
            &score_diagnostics,
            &project_info.root_dir,
            selected_categories,
        );
        result.score = health_score;
        result.score_label = label;
        result.dimension_scores = dimensions;
    }

    tracing::info!(
        score = result.score,
        diagnostics = result.diagnostics.len(),
        "scan complete"
    );

    Ok(result)
}

// ---------------------------------------------------------------------------
// Pipeline stages
// ---------------------------------------------------------------------------

fn validate_config(resolved: &ResolvedConfig) -> Result<(), crate::error::ScanError> {
    config::validate_resolved_config(resolved).map_err(crate::error::ScanError::InvalidPolicy)
}

fn resolve_scan_roots(
    project_info: &ProjectInfo,
    resolved: &ResolvedConfig,
    project_filter: &[String],
    scope: &diff::ScopePlan,
) -> Result<Vec<PathBuf>, crate::error::ScanError> {
    if project_info.is_workspace {
        let members = workspace::resolve_members(
            &project_info.workspace_members,
            &project_info.root_dir,
            &project_info.default_member_ids,
            project_filter,
        )?;
        let members = if scope.has_selected_files() {
            workspace::affected_members(members, &project_info.root_dir, &scope.execution_paths())
        } else {
            members
        };
        if resolved.verbose {
            eprintln!(
                "Workspace: scanning {} of {} members",
                members.len(),
                project_info.member_count
            );
        }
        Ok(members.iter().map(|m| m.root_dir.clone()).collect())
    } else {
        if project_filter
            .iter()
            .any(|selector| selector != "*" && selector != &project_info.name && selector != ".")
        {
            return Err(crate::error::WorkspaceError::UnknownMember {
                name: project_filter.join(","),
                available: format!("{}, .", project_info.name),
            }
            .into());
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

/// Construct the set of analysis passes based on project info and config.
/// Lint passes (clippy + custom rules) are included when lint=true. The
/// deterministic corpus profile runs only Rust Doctor's custom rules.
/// Package dependency passes run per affected member. Workspace-global passes
/// run once after required package analysis.
#[expect(
    clippy::too_many_lines,
    reason = "pass construction keeps category, framework, policy, and tool applicability decisions at one planning boundary"
)]
fn build_passes(
    project_info: &ProjectInfo,
    scan_root: &Path,
    resolved: &ResolvedConfig,
    offline: bool,
    selected_files: Option<Vec<PathBuf>>,
    selected_categories: &[Category],
) -> Vec<Box<dyn scanner::AnalysisPass>> {
    let member = workspace::member_for_root(&project_info.workspace_members, scan_root);
    let frameworks = member.map_or(project_info.frameworks.as_slice(), |member| {
        member.frameworks.as_slice()
    });
    let framework_capabilities = member
        .map_or(project_info.framework_capabilities.as_slice(), |member| {
            member.framework_capabilities.as_slice()
        });
    let cargo_targets = member.map_or(project_info.cargo_targets.as_slice(), |member| {
        member.cargo_targets.as_slice()
    });
    let has_async_runtime = frameworks.iter().any(|f| {
        matches!(
            f,
            crate::discovery::Framework::Tokio
                | crate::discovery::Framework::AsyncStd
                | crate::discovery::Framework::Smol
        )
    });
    let framework_names: Vec<String> = frameworks
        .iter()
        .map(std::string::ToString::to_string)
        .collect();

    let mut passes: Vec<Box<dyn scanner::AnalysisPass>> = Vec::new();

    if resolved.adapter_policy.compiler_lint && !resolved.evaluation_profile {
        passes.push(Box::new(clippy::ClippyPass::default()));
    }
    if resolved.adapter_policy.custom_ast {
        let mut custom_rules: Vec<Box<dyn rules::CustomRule>> = if resolved.evaluation_profile {
            rules::all_custom_rules()
        } else {
            rules::error_handling::all_rules()
                .into_iter()
                .chain(rules::performance::all_rules())
                .chain(rules::reliability::all_rules())
                .chain(rules::complexity::all_rules())
                .chain(rules::security::all_rules())
                .chain(rules::tranche::all_rules())
                .chain(if has_async_runtime {
                    rules::async_rules::all_rules()
                } else {
                    vec![]
                })
                .chain(rules::framework::rules_for_frameworks(frameworks))
                .chain(rules::framework_packs::rules_for_capabilities(
                    framework_capabilities,
                    resolved.verbose,
                ))
                .collect()
        };
        if let Ok(catalog) = built_in_catalog() {
            custom_rules.retain_mut(|rule| {
                let Some(descriptor) = catalog.exact(rule.name()) else {
                    return false;
                };
                if !resolved.evaluation_profile
                    && !descriptor.applicable_frameworks.is_empty()
                    && !descriptor
                        .applicable_frameworks
                        .iter()
                        .any(|required| framework_names.contains(required))
                {
                    return false;
                }
                let policy = resolved.rule_policy(descriptor, None);
                if let Some(threshold) = policy.threshold {
                    rule.set_threshold(threshold);
                }
                let path_can_enable = resolved.path_overrides.iter().any(|override_| {
                    override_
                        .severity
                        .is_some_and(|level| level != config::RuleLevel::Off)
                });
                policy.severity.is_some()
                    || descriptor.category == Category::Security
                    || path_can_enable
            });
        }
        custom_rules.retain(|rule| category_requested(selected_categories, &rule.category()));
        let mut cache_policy = resolved.enable_rules.clone();
        cache_policy.extend(rule_cache_keys(resolved));
        if selected_categories.is_empty() || !custom_rules.is_empty() {
            let mut rule_pass = rules::RuleEnginePass::with_config(
                custom_rules,
                resolved.ignore_files.clone(),
                resolved.ignore_rules.clone(),
                cache_policy,
            )
            .with_cargo_targets(cargo_targets.to_vec());
            if let Some(files) = selected_files {
                rule_pass = rule_pass.with_selected_files(files);
            }
            passes.push(Box::new(rule_pass));
        }
    }

    if resolved.adapter_policy.supply_chain {
        if !project_info.is_workspace {
            passes.extend(build_workspace_global_passes(offline, selected_categories));
        }
        if category_requested(selected_categories, &Category::Security) {
            passes.push(optional_pass(geiger::GeigerPass));
        }
    }
    if resolved.adapter_policy.quality {
        if category_requested(selected_categories, &Category::Cargo) {
            passes.push(optional_pass(semver_checks::SemVerPass));
        }
        if category_requested(selected_categories, &Category::Correctness) {
            passes.push(optional_pass(coverage::CoveragePass));
        }
    }

    // MSRV validation runs for full scans and Cargo category scans.
    if category_requested(selected_categories, &Category::Cargo) {
        passes.push(Box::new(msrv::MsrvPass {
            rust_version: member
                .and_then(|member| member.rust_version.clone())
                .or_else(|| project_info.rust_version.clone()),
        }));
    }

    passes
}

struct OptionalPass<T>(T);

impl<T: scanner::AnalysisPass> scanner::AnalysisPass for OptionalPass<T> {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        self.0.run(project_root)
    }

    fn required(&self) -> bool {
        false
    }

    fn take_compiler_evidence(&self) -> Vec<CompilerDiagnosticEvidence> {
        self.0.take_compiler_evidence()
    }
}

fn optional_pass<T: scanner::AnalysisPass + 'static>(pass: T) -> Box<dyn scanner::AnalysisPass> {
    Box::new(OptionalPass(pass))
}

fn build_workspace_global_passes(
    offline: bool,
    selected_categories: &[Category],
) -> Vec<Box<dyn scanner::AnalysisPass>> {
    let mut passes: Vec<Box<dyn scanner::AnalysisPass>> = Vec::new();
    let dependencies_requested = category_requested(selected_categories, &Category::Dependencies);
    if dependencies_requested || category_requested(selected_categories, &Category::Cargo) {
        passes.push(optional_pass(deny::DenyPass { offline }));
    }
    if dependencies_requested {
        if !deny::is_cargo_deny_available() {
            passes.push(optional_pass(audit::AuditPass { offline }));
        }
        passes.push(optional_pass(machete::MachetePass));
    }
    passes
}

/// Aggregated output from running all analysis passes across scan roots.
struct PassesOutput {
    diagnostics: Vec<Diagnostic>,
    source_file_count: usize,
    skipped_passes: Vec<String>,
    elapsed: Duration,
    pass_timings: Vec<(String, Duration)>,
    planned_files: Vec<PathBuf>,
    analyzed_files: Vec<PathBuf>,
    compiler_evidence: Vec<CompilerDiagnosticEvidence>,
    checks: Vec<CheckState>,
    package_executions: Vec<PackageExecution>,
    analysis_failures: Vec<crate::diagnostics::AnalysisFailureReceipt>,
}

#[expect(
    clippy::too_many_lines,
    reason = "package batching, workspace checks, and completeness accounting share one orchestration boundary"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "pass execution receives the resolved category scope alongside the existing orchestration context"
)]
fn run_passes(
    project_info: &ProjectInfo,
    resolved: &ResolvedConfig,
    scan_roots: &[PathBuf],
    scope: &diff::ScopePlan,
    offline: bool,
    suppress_spinner: bool,
    control: &ScanControl,
    selected_categories: &[Category],
) -> PassesOutput {
    let is_multi_root = scan_roots.len() > 1;
    let mut all_diagnostics = Vec::new();
    let mut all_skipped_passes = Vec::new();
    let mut total_elapsed = Duration::ZERO;
    let mut all_pass_timings = Vec::new();
    let mut all_compiler_evidence = Vec::new();
    let mut all_checks = Vec::new();
    let mut package_executions = Vec::new();
    let mut all_analysis_failures = Vec::new();
    let ignore_set = scanner::build_glob_set(&resolved.ignore_files).ok();
    let scan_work: Vec<(PathBuf, Vec<PathBuf>)> = scan_roots
        .iter()
        .map(|root| {
            let mut files = if scope.has_selected_files() {
                scope
                    .rust_files
                    .iter()
                    .map(|path| project_info.root_dir.join(path))
                    .filter(|path| path.starts_with(root) && path.is_file())
                    .collect()
            } else {
                scanner::collect_rs_files(root)
            };
            files.retain(|path| belongs_to_package(path, root));
            if let Some(ignore_set) = &ignore_set {
                files.retain(|path| {
                    let relative = path.strip_prefix(root).unwrap_or(path);
                    !ignore_set.is_match(relative)
                });
            }
            (root.clone(), files)
        })
        .collect();
    let mut planned_files: Vec<PathBuf> = scan_work
        .iter()
        .flat_map(|(_, files)| files.iter().cloned())
        .collect();
    planned_files.sort();
    planned_files.dedup();
    let planned_file_stamps: std::collections::BTreeMap<_, _> = planned_files
        .iter()
        .filter_map(|path| file_stamp(path).map(|stamp| (path.clone(), stamp)))
        .collect();
    let mut analyzed_files = Vec::new();

    // INVARIANT: parallelize scan roots with OS threads (`std::thread::scope`),
    // bounded to `available_parallelism` per batch — NEVER with rayon
    // (`par_iter`/`rayon::scope`). Each root runs its passes via an inner
    // `std::thread::scope` (`ScanOrchestrator::run_passes_parallel`), and the rule
    // engine fans out file work with an inner rayon `par_iter` (`rules/mod.rs`).
    // A rayon iterator at THIS level would park a rayon worker on the inner
    // `thread::scope.join()` whose rule engine awaits inner rayon work on the same
    // global pool; with workspace members ≥ cores and a cold cache, every worker
    // parks on `join` and the inner `par_iter` starves → permanent hang (EP-001).
    // OS threads sidestep this: rayon pool workers are never the threads that block
    // on a join, so the pool always makes progress regardless of the member/core
    // ratio. Root-level parallelism is KEPT (it overlaps each root's blocking
    // `cargo clippy` wait — sequential roots measured ~20% slower on a cold
    // multi-member workspace) but bounded, so a huge workspace cannot spawn
    // unbounded threads or `cargo` subprocesses.
    // DO NOT reintroduce rayon above a `thread::scope` that itself contains rayon.
    let available = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let max_parallel = resolved
        .max_parallelism
        .unwrap_or(available)
        .min(scan_work.len().max(1));
    for batch in scan_work.chunks(max_parallel) {
        if control.is_stopped() {
            break;
        }
        let batch_results = std::thread::scope(|s| {
            #[expect(
                clippy::needless_collect,
                reason = "all roots must be spawned before any is joined, else they run serially"
            )]
            let handles: Vec<_> = batch
                .iter()
                .map(|(scan_root, root_files)| {
                    let selected_files = Some(root_files.clone());
                    s.spawn(move || {
                        if control.is_stopped() {
                            return (false, scanner::ScanPassResult::default());
                        }
                        let passes = build_passes(
                            project_info,
                            scan_root,
                            resolved,
                            offline,
                            selected_files,
                            selected_categories,
                        );
                        let orchestrator = scanner::ScanOrchestrator::new(passes);
                        let pass_result = orchestrator.run_controlled(
                            scan_root,
                            resolved,
                            suppress_spinner || is_multi_root,
                            control,
                        );
                        (true, pass_result)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| {
                        // Pass panics are already caught inside the orchestrator
                        // (PassError::Panicked), so a join failure here is a rare
                        // root-level panic. Keep the other roots' results instead of
                        // aborting the whole scan (US-001 AC5).
                        eprintln!(
                            "Warning: a scan root worker panicked; its diagnostics are omitted"
                        );
                        (false, scanner::ScanPassResult::default())
                    })
                })
                .collect::<Vec<_>>()
        });

        // Roots within a batch run in parallel (wall-clock ≈ max); batches run
        // sequentially (wall-clock ≈ sum of per-batch maxima).
        let mut batch_elapsed = Duration::ZERO;
        for ((scan_root, root_files), (root_started, mut pass_result)) in
            batch.iter().zip(batch_results)
        {
            rebase_analysis_failure_paths(
                &mut pass_result.analysis_failures,
                scan_root,
                &project_info.root_dir,
            );
            rebase_pass_paths(
                &mut pass_result.diagnostics,
                &mut pass_result.compiler_evidence,
                scan_root,
                &project_info.root_dir,
            );
            let required_complete = root_started
                && pass_result
                    .checks
                    .iter()
                    .filter(|check| check.required)
                    .all(|check| check.status == CheckStatus::Completed);
            let root_analyzed_files = if required_complete {
                root_files
                    .iter()
                    .filter(|path| {
                        planned_file_stamps
                            .get(*path)
                            .is_some_and(|planned| file_stamp(path).as_ref() == Some(planned))
                    })
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            analyzed_files.extend(root_analyzed_files.iter().cloned());
            let package_id = package_id_for_root(project_info, scan_root);
            let mut global_checks = pass_result.checks.clone();
            if scan_roots.len() > 1 {
                for check in &mut global_checks {
                    check.name = format!("{package_id}:{}", check.name);
                }
            }
            all_checks.extend(global_checks);
            package_executions.push(PackageExecution {
                cargo_package_id: package_id,
                package_root: scan_root.clone(),
                planned_files: root_files.clone(),
                analyzed_files: root_analyzed_files,
                checks: pass_result.checks,
                elapsed: pass_result.elapsed,
                score: None,
            });
            all_diagnostics.extend(pass_result.diagnostics);
            all_skipped_passes.extend(pass_result.skipped_passes);
            batch_elapsed = batch_elapsed.max(pass_result.elapsed);
            all_pass_timings.extend(pass_result.pass_timings);
            all_compiler_evidence.extend(pass_result.compiler_evidence);
            all_analysis_failures.extend(pass_result.analysis_failures);
        }
        total_elapsed += batch_elapsed;
    }

    let completed_roots: std::collections::BTreeSet<_> = package_executions
        .iter()
        .map(|execution| execution.package_root.clone())
        .collect();
    if let Some(stop) = control.stop_reason() {
        for (root, root_files) in scan_work
            .iter()
            .filter(|(root, _)| !completed_roots.contains(root))
        {
            let status = match stop {
                ProcessStop::TimedOut => CheckStatus::TimedOut,
                ProcessStop::Cancelled => CheckStatus::Cancelled,
            };
            let reason = match stop {
                ProcessStop::TimedOut => "scan deadline reached before package launch",
                ProcessStop::Cancelled => "scan cancellation requested before package launch",
            }
            .to_string();
            let check = CheckState {
                name: "package scan".to_string(),
                required: true,
                status,
                reason: Some(reason.clone()),
            };
            let package_id = package_id_for_root(project_info, root);
            let mut global_check = check.clone();
            global_check.name = format!("{package_id}:package scan");
            all_checks.push(global_check);
            all_skipped_passes.push(format!("{package_id}: {reason}"));
            package_executions.push(PackageExecution {
                cargo_package_id: package_id,
                package_root: root.clone(),
                planned_files: root_files.clone(),
                analyzed_files: Vec::new(),
                checks: vec![check],
                elapsed: Duration::ZERO,
                score: None,
            });
        }
    }

    if project_info.is_workspace
        && resolved.adapter_policy.supply_chain
        && !scan_roots.is_empty()
        && (category_requested(selected_categories, &Category::Dependencies)
            || category_requested(selected_categories, &Category::Cargo))
    {
        let passes = build_workspace_global_passes(offline, selected_categories);
        let workspace_result = if let Some(stop) = control.stop_reason() {
            let status = match stop {
                ProcessStop::TimedOut => CheckStatus::TimedOut,
                ProcessStop::Cancelled => CheckStatus::Cancelled,
            };
            let reason = match stop {
                ProcessStop::TimedOut => "scan deadline reached before workspace checks",
                ProcessStop::Cancelled => "scan cancellation requested before workspace checks",
            }
            .to_string();
            scanner::ScanPassResult {
                skipped_passes: passes
                    .iter()
                    .map(|pass| format!("{}: {reason}", pass.name()))
                    .collect(),
                checks: passes
                    .iter()
                    .map(|pass| CheckState {
                        name: pass.name().to_string(),
                        required: false,
                        status,
                        reason: Some(reason.clone()),
                    })
                    .collect(),
                ..scanner::ScanPassResult::default()
            }
        } else {
            scanner::ScanOrchestrator::new(passes).run_controlled(
                &project_info.root_dir,
                resolved,
                suppress_spinner || is_multi_root,
                control,
            )
        };
        let mut workspace_checks = workspace_result.checks.clone();
        for check in &mut workspace_checks {
            check.name = format!("workspace:{}", check.name);
        }
        for package in &mut package_executions {
            package.checks.extend(workspace_checks.clone());
        }
        all_checks.extend(workspace_checks);
        all_diagnostics.extend(workspace_result.diagnostics);
        all_skipped_passes.extend(workspace_result.skipped_passes);
        total_elapsed += workspace_result.elapsed;
        all_pass_timings.extend(workspace_result.pass_timings);
        all_compiler_evidence.extend(workspace_result.compiler_evidence);
        all_analysis_failures.extend(workspace_result.analysis_failures);
    }

    all_checks.sort_by(|left, right| left.name.cmp(&right.name));

    PassesOutput {
        diagnostics: all_diagnostics,
        source_file_count: analyzed_files.len(),
        skipped_passes: all_skipped_passes,
        elapsed: total_elapsed,
        pass_timings: all_pass_timings,
        planned_files,
        analyzed_files,
        compiler_evidence: all_compiler_evidence,
        checks: all_checks,
        package_executions,
        analysis_failures: all_analysis_failures,
    }
}

fn rebase_analysis_failure_paths(
    failures: &mut [crate::diagnostics::AnalysisFailureReceipt],
    scan_root: &Path,
    workspace_root: &Path,
) {
    for failure in failures {
        let path = scan_root.join(&failure.path);
        failure.path = path
            .strip_prefix(workspace_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
    }
}

#[derive(PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    metadata.is_file().then(|| FileStamp {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn belongs_to_package(path: &Path, package_root: &Path) -> bool {
    let mut current = path.parent();
    while let Some(directory) = current {
        if directory == package_root {
            return true;
        }
        if !directory.starts_with(package_root) || directory.join("Cargo.toml").is_file() {
            return false;
        }
        current = directory.parent();
    }
    false
}

fn package_id_for_root(project_info: &ProjectInfo, root: &Path) -> String {
    workspace::member_for_root(&project_info.workspace_members, root).map_or_else(
        || project_info.package_id.clone(),
        |member| member.package_id.clone(),
    )
}

fn rebase_pass_paths(
    diagnostics: &mut [Diagnostic],
    evidence: &mut [CompilerDiagnosticEvidence],
    package_root: &Path,
    workspace_root: &Path,
) {
    for diagnostic in diagnostics {
        diagnostic.file_path = rebase_path(&diagnostic.file_path, package_root, workspace_root);
    }
    for item in evidence {
        item.file_path = rebase_path(&item.file_path, package_root, workspace_root);
        if let Some(span) = &mut item.primary_span {
            span.file_path = rebase_path(&span.file_path, package_root, workspace_root);
        }
        for (_, span) in &mut item.related_locations {
            span.file_path = rebase_path(&span.file_path, package_root, workspace_root);
        }
        if let Some(expansion) = &mut item.macro_expansion {
            expansion.call_site.file_path =
                rebase_path(&expansion.call_site.file_path, package_root, workspace_root);
        }
        for fix in &mut item.fixes {
            fix.span.file_path = rebase_path(&fix.span.file_path, package_root, workspace_root);
        }
    }
}

fn rebase_path(path: &Path, package_root: &Path, workspace_root: &Path) -> PathBuf {
    if path == Path::new("<unknown>") {
        return path.to_path_buf();
    }
    if path.is_absolute() {
        return path
            .strip_prefix(workspace_root)
            .unwrap_or(path)
            .to_path_buf();
    }
    if path == Path::new("Cargo.lock") && workspace_root.join(path).is_file() {
        return path.to_path_buf();
    }
    package_root
        .join(path)
        .strip_prefix(workspace_root)
        .unwrap_or(path)
        .to_path_buf()
}

/// Deduplicate diagnostics from overlapping workspace scans.
fn dedup_diagnostics(diagnostics: &mut Vec<Diagnostic>) {
    diagnostics.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.rule.cmp(&b.rule))
            .then(a.line.cmp(&b.line))
            .then(a.column.cmp(&b.column))
            .then_with(|| {
                a.message
                    .split_whitespace()
                    .cmp(b.message.split_whitespace())
            })
            .then(a.message.cmp(&b.message))
    });
    diagnostics.dedup_by(|a, b| {
        a.file_path == b.file_path
            && a.rule == b.rule
            && a.line == b.line
            && a.column == b.column
            && a.message
                .split_whitespace()
                .eq(b.message.split_whitespace())
    });
}

fn apply_suppressions(
    diagnostics: Vec<Diagnostic>,
    project_info: &ProjectInfo,
    resolved: &ResolvedConfig,
) -> (Vec<Diagnostic>, usize) {
    if !resolved.respect_inline_disables {
        if resolved.verbose {
            eprintln!("Inline rust-doctor suppression directives are ignored for this scan");
        }
        return (diagnostics, 0);
    }
    let (diagnostics, suppressed_count) =
        suppression::apply_inline_suppressions(diagnostics, &project_info.root_dir);
    if resolved.verbose && suppressed_count > 0 {
        eprintln!("Suppressed {suppressed_count} diagnostic(s) via inline comments");
    }
    (diagnostics, suppressed_count)
}

fn apply_rule_configuration(
    diagnostics: Vec<Diagnostic>,
    resolved: &ResolvedConfig,
) -> (Vec<Diagnostic>, Vec<Diagnostic>, SuppressionCounts) {
    let Ok(catalog) = built_in_catalog() else {
        return (diagnostics, Vec::new(), SuppressionCounts::default());
    };
    let mut included = Vec::with_capacity(diagnostics.len());
    let mut suppressed_security = Vec::new();
    let mut suppression_counts = SuppressionCounts::default();

    for mut diagnostic in diagnostics {
        let resolved_descriptor =
            catalog.resolve(&diagnostic.rule, &diagnostic.category, diagnostic.severity);
        let descriptor = resolved_descriptor.as_descriptor();
        let policy = resolved.rule_policy(descriptor, Some(&diagnostic.file_path));
        diagnostic.rule.clone_from(&descriptor.canonical_id);
        diagnostic.category.clone_from(&descriptor.category);
        if let Some(severity) = policy.severity {
            diagnostic.severity = severity;
            included.push(diagnostic);
        } else {
            match resolved.suppression_source(descriptor, Some(&diagnostic.file_path)) {
                Some(config::PolicySuppressionSource::Rule) => {
                    suppression_counts.rule = suppression_counts.rule.saturating_add(1);
                }
                Some(config::PolicySuppressionSource::Category) => {
                    suppression_counts.category = suppression_counts.category.saturating_add(1);
                }
                Some(config::PolicySuppressionSource::Tag) => {
                    suppression_counts.tag = suppression_counts.tag.saturating_add(1);
                }
                Some(config::PolicySuppressionSource::Path) => {
                    suppression_counts.path = suppression_counts.path.saturating_add(1);
                }
                None => {}
            }
            if descriptor.category == Category::Security {
                suppression_counts.security_policy =
                    suppression_counts.security_policy.saturating_add(1);
                suppressed_security.push(diagnostic);
            }
        }
    }

    if resolved.verbose && !suppressed_security.is_empty() {
        eprintln!(
            "Audit: {} security diagnostic(s) suppressed by project policy",
            suppressed_security.len()
        );
    }
    (included, suppressed_security, suppression_counts)
}

fn category_requested(selected_categories: &[Category], category: &Category) -> bool {
    selected_categories.is_empty() || selected_categories.contains(category)
}

fn retain_selected_categories(diagnostics: &mut Vec<Diagnostic>, selected_categories: &[Category]) {
    if !selected_categories.is_empty() {
        diagnostics.retain(|diagnostic| selected_categories.contains(&diagnostic.category));
    }
}

fn rule_cache_keys(resolved: &ResolvedConfig) -> Vec<String> {
    let mut keys = Vec::new();
    for (rule, policy) in &resolved.rules_config {
        keys.push(format!(
            "rule:{rule}:{:?}:{:?}:{:?}",
            policy.severity, policy.threshold, policy.surfaces
        ));
    }
    for (category, policy) in &resolved.category_config {
        keys.push(format!(
            "category:{category}:{:?}:{:?}",
            policy.severity, policy.surfaces
        ));
    }
    for (tag, policy) in &resolved.tag_config {
        keys.push(format!(
            "tag:{tag}:{:?}:{:?}",
            policy.severity, policy.surfaces
        ));
    }
    for path in &resolved.path_overrides {
        keys.push(format!(
            "path:{}:{:?}:{:?}",
            path.pattern, path.severity, path.surfaces
        ));
    }
    keys.sort();
    keys
}

fn score_visible_diagnostics(
    diagnostics: &[Diagnostic],
    resolved: &ResolvedConfig,
) -> Vec<Diagnostic> {
    built_in_catalog().map_or_else(
        |_| diagnostics.to_vec(),
        |catalog| {
            diagnostics
                .iter()
                .filter(|diagnostic| {
                    let descriptor = catalog.resolve(
                        &diagnostic.rule,
                        &diagnostic.category,
                        diagnostic.severity,
                    );
                    resolved
                        .rule_policy(descriptor.as_descriptor(), Some(&diagnostic.file_path))
                        .visible_on(VisibilitySurface::Score)
                })
                .cloned()
                .collect()
        },
    )
}

pub(crate) fn recalculate_package_scores_and_select_headline(
    packages: &mut [PackageExecution],
    diagnostics: &[Diagnostic],
    workspace_root: &Path,
    selected_categories: &[Category],
) -> (
    u32,
    crate::diagnostics::ScoreLabel,
    crate::diagnostics::DimensionScores,
) {
    let aggregate_score = output::calculate_score_for_categories(diagnostics, selected_categories);
    let owners = diagnostic_package_owners(diagnostics, packages, workspace_root);
    assign_package_scores_with_owners(packages, diagnostics, &owners, selected_categories);
    if packages.len() <= 1 {
        return aggregate_score;
    }

    let mut worst_package = None;
    for (index, package) in packages.iter().enumerate() {
        let Some(score) = package.score else {
            continue;
        };
        if worst_package.is_none_or(|(_, worst_score)| score < worst_score) {
            worst_package = Some((index, score));
        }
    }
    let Some((worst_index, _)) = worst_package else {
        return aggregate_score;
    };
    let worst_diagnostics = diagnostics_for_package(diagnostics, &owners, worst_index);
    output::calculate_score_for_categories(&worst_diagnostics, selected_categories)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the scan accumulator is unpacked once at the internal ScanResult construction boundary"
)]
fn build_result(
    diagnostics: Vec<Diagnostic>,
    source_file_count: usize,
    mut skipped_passes: Vec<String>,
    elapsed: Duration,
    pass_timings: Vec<(String, Duration)>,
    suppressed_security: Vec<Diagnostic>,
    suppression_counts: SuppressionCounts,
    planned_files: Vec<PathBuf>,
    analyzed_files: Vec<PathBuf>,
    compiler_evidence: Vec<CompilerDiagnosticEvidence>,
    mut checks: Vec<CheckState>,
    mut package_executions: Vec<PackageExecution>,
    workspace_root: &Path,
    scope: &diff::ScopePlan,
    resolved: &ResolvedConfig,
    selected_categories: &[Category],
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
    let score_diagnostics = score_visible_diagnostics(&diagnostics, resolved);
    let (health_score, score_label, dimension_scores) =
        recalculate_package_scores_and_select_headline(
            &mut package_executions,
            &score_diagnostics,
            workspace_root,
            selected_categories,
        );

    if let Some(reason) = &scope.degradation_reason {
        let check = CheckState {
            name: "scope:lines".to_string(),
            required: false,
            status: CheckStatus::Skipped,
            reason: Some(reason.clone()),
        };
        checks.push(check.clone());
        for package in &mut package_executions {
            package.checks.push(check.clone());
            package
                .checks
                .sort_by(|left, right| left.name.cmp(&right.name));
        }
        skipped_passes.push(reason.clone());
    }
    skipped_passes.sort();
    skipped_passes.dedup();
    checks.sort_by(|left, right| left.name.cmp(&right.name));

    ScanResult {
        diagnostics,
        score: health_score,
        score_label,
        dimension_scores,
        source_file_count,
        elapsed,
        skipped_passes,
        error_count,
        warning_count,
        info_count,
        pass_timings,
        suppressed_security,
        planned_files,
        analyzed_files,
        compiler_evidence,
        execution: ScanExecution {
            execution_scope: execution_scope_name(scope).to_string(),
            reporting_scope: scope.reporting_scope.name().to_string(),
            checks,
            packages: package_executions,
            baseline: None,
            suppression_counts,
            analysis_failures: Vec::new(),
        },
    }
}

fn diagnostic_package_owners(
    diagnostics: &[Diagnostic],
    packages: &[PackageExecution],
    workspace_root: &Path,
) -> Vec<Option<usize>> {
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic_package_owner(diagnostic, packages, workspace_root))
        .collect()
}

fn assign_package_scores_with_owners(
    packages: &mut [PackageExecution],
    diagnostics: &[Diagnostic],
    owners: &[Option<usize>],
    selected_categories: &[Category],
) {
    for (package_index, package) in packages.iter_mut().enumerate() {
        if !crate::completeness::package_score_is_authoritative(package) {
            package.score = None;
            continue;
        }
        let package_diagnostics = diagnostics_for_package(diagnostics, owners, package_index);
        package.score = Some(
            output::calculate_score_for_categories(&package_diagnostics, selected_categories).0,
        );
    }
}

fn diagnostics_for_package(
    diagnostics: &[Diagnostic],
    owners: &[Option<usize>],
    package_index: usize,
) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .zip(owners)
        .filter(|(diagnostic, owner)| {
            is_workspace_global_diagnostic(diagnostic) || **owner == Some(package_index)
        })
        .map(|(diagnostic, _)| diagnostic.clone())
        .collect()
}

fn is_workspace_global_diagnostic(diagnostic: &Diagnostic) -> bool {
    matches!(
        diagnostic
            .file_path
            .to_string_lossy()
            .replace('\\', "/")
            .as_str(),
        "Cargo.toml" | "Cargo.lock" | "rust-doctor.toml" | "rust-toolchain" | "rust-toolchain.toml"
    )
}

fn diagnostic_package_owner(
    diagnostic: &Diagnostic,
    packages: &[PackageExecution],
    workspace_root: &Path,
) -> Option<usize> {
    let absolute = if diagnostic.file_path.is_absolute() {
        diagnostic.file_path.clone()
    } else {
        workspace_root.join(&diagnostic.file_path)
    };
    packages
        .iter()
        .enumerate()
        .filter(|(_, package)| absolute.starts_with(&package.package_root))
        .max_by_key(|(_, package)| package.package_root.components().count())
        .map(|(index, _)| index)
}

const fn execution_scope_name(scope: &diff::ScopePlan) -> &'static str {
    match scope.reporting_scope {
        diff::ReportingScope::Full => "full_packages",
        diff::ReportingScope::Files
        | diff::ReportingScope::Changed
        | diff::ReportingScope::Lines => "affected_packages",
        diff::ReportingScope::Staged | diff::ReportingScope::Baseline => "isolated_snapshot",
    }
}

fn empty_scoped_result(
    project_info: &ProjectInfo,
    scope: &diff::ScopePlan,
    resolved: &ResolvedConfig,
    selected_categories: &[Category],
) -> ScanResult {
    tracing::info!(project = %project_info.name, "scope contains no eligible Rust files");
    build_result(
        Vec::new(),
        0,
        Vec::new(),
        Duration::ZERO,
        Vec::new(),
        Vec::new(),
        SuppressionCounts::default(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        &project_info.root_dir,
        scope,
        resolved,
        selected_categories,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Category;
    use crate::discovery::WorkspaceMember;
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
        let config = config::resolve_config_defaults(None);
        let result = build_result(
            diags,
            10,
            vec![],
            Duration::from_secs(1),
            vec![],
            vec![],
            SuppressionCounts::default(),
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            Path::new("/workspace"),
            &diff::ScopePlan::full(),
            &config,
            &[],
        );
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
        let config = config::resolve_config_defaults(None);
        let result = build_result(
            vec![],
            0,
            skipped,
            Duration::ZERO,
            vec![],
            vec![],
            SuppressionCounts::default(),
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            Path::new("/workspace"),
            &diff::ScopePlan::full(),
            &config,
            &[],
        );
        assert_eq!(result.skipped_passes.len(), 2);
        assert_eq!(result.skipped_passes[0], "cargo-audit"); // sorted
        assert_eq!(result.skipped_passes[1], "cargo-deny");
    }

    #[test]
    fn build_result_empty_diagnostics_gives_perfect_score() {
        let config = config::resolve_config_defaults(None);
        let result = build_result(
            vec![],
            5,
            vec![],
            Duration::from_millis(100),
            vec![],
            vec![],
            SuppressionCounts::default(),
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            Path::new("/workspace"),
            &diff::ScopePlan::full(),
            &config,
            &[],
        );
        assert_eq!(result.score, 100);
        assert_eq!(result.error_count, 0);
        assert_eq!(result.warning_count, 0);
        assert_eq!(result.info_count, 0);
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "the regression keeps three package completeness states and their score inputs visible in one scenario"
    )]
    fn incomplete_package_with_worst_raw_score_cannot_become_headline() {
        let workspace_root = Path::new("/workspace");
        let first_root = workspace_root.join("first");
        let worst_root = workspace_root.join("worst");
        let incomplete_root = workspace_root.join("incomplete");
        let completed_check = || CheckState {
            name: "custom rules".to_string(),
            required: true,
            status: CheckStatus::Completed,
            reason: None,
        };
        let packages = vec![
            PackageExecution {
                cargo_package_id: "first".to_string(),
                package_root: first_root.clone(),
                planned_files: vec![first_root.join("src/lib.rs")],
                analyzed_files: vec![first_root.join("src/lib.rs")],
                checks: vec![completed_check()],
                elapsed: Duration::ZERO,
                score: None,
            },
            PackageExecution {
                cargo_package_id: "worst".to_string(),
                package_root: worst_root.clone(),
                planned_files: vec![worst_root.join("src/lib.rs")],
                analyzed_files: vec![worst_root.join("src/lib.rs")],
                checks: vec![completed_check()],
                elapsed: Duration::ZERO,
                score: None,
            },
            PackageExecution {
                cargo_package_id: "incomplete".to_string(),
                package_root: incomplete_root.clone(),
                planned_files: vec![
                    incomplete_root.join("src/lib.rs"),
                    incomplete_root.join("src/other.rs"),
                ],
                analyzed_files: vec![incomplete_root.join("src/lib.rs")],
                checks: vec![completed_check()],
                elapsed: Duration::ZERO,
                score: None,
            },
        ];
        let mut diagnostics = Vec::new();
        for index in 0..4 {
            let mut diagnostic = make_diagnostic(
                &format!("first-performance-{index}"),
                Severity::Error,
                Some(index + 1),
            );
            diagnostic.file_path = PathBuf::from("first/src/lib.rs");
            diagnostic.category = Category::Performance;
            diagnostics.push(diagnostic);
        }
        let mut worst_diagnostics = Vec::new();
        for index in 0..8 {
            let mut diagnostic = make_diagnostic(
                &format!("worst-reliability-{index}"),
                Severity::Error,
                Some(index + 1),
            );
            diagnostic.file_path = PathBuf::from("worst/src/lib.rs");
            worst_diagnostics.push(diagnostic.clone());
            diagnostics.push(diagnostic);
        }
        let mut incomplete_diagnostics = Vec::new();
        for index in 0..20 {
            let mut diagnostic = make_diagnostic(
                &format!("incomplete-reliability-{index}"),
                Severity::Error,
                Some(index + 1),
            );
            diagnostic.file_path = PathBuf::from("incomplete/src/lib.rs");
            incomplete_diagnostics.push(diagnostic.clone());
            diagnostics.push(diagnostic);
        }
        let expected = output::calculate_score_for_categories(&worst_diagnostics, &[]);
        let incomplete_raw = output::calculate_score_for_categories(&incomplete_diagnostics, &[]);
        let combined = output::calculate_score_for_categories(&diagnostics, &[]);
        assert_ne!(combined.0, expected.0);
        assert!(incomplete_raw.0 < expected.0);

        let config = config::resolve_config_defaults(None);
        let result = build_result(
            diagnostics,
            2,
            Vec::new(),
            Duration::ZERO,
            Vec::new(),
            Vec::new(),
            SuppressionCounts::default(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            packages,
            workspace_root,
            &diff::ScopePlan::full(),
            &config,
            &[],
        );

        assert_eq!(result.score, expected.0);
        assert_eq!(result.score_label, expected.1);
        assert_eq!(result.dimension_scores.reliability, expected.2.reliability);
        assert_eq!(result.dimension_scores.performance, expected.2.performance);
        assert_eq!(result.execution.packages[0].score, Some(99));
        assert_eq!(result.execution.packages[1].score, Some(expected.0));
        assert_eq!(result.execution.packages[2].score, None);
        assert!(!crate::completeness::package_score_is_authoritative(
            &result.execution.packages[2]
        ));
        assert!(crate::completeness::score_is_reportable(&result));
    }

    #[test]
    fn run_passes_preserves_selected_root_order() {
        let directory = tempfile::tempdir().unwrap();
        let first_root = directory.path().join("z-selected-first");
        let second_root = directory.path().join("a-selected-second");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&second_root).unwrap();
        let members = vec![
            WorkspaceMember {
                name: "first".to_string(),
                root_dir: first_root.clone(),
                package_id: "first 0.1.0".to_string(),
                targets: Vec::new(),
                cargo_targets: Vec::new(),
                frameworks: Vec::new(),
                framework_capabilities: Vec::new(),
                rust_version: None,
            },
            WorkspaceMember {
                name: "second".to_string(),
                root_dir: second_root.clone(),
                package_id: "second 0.1.0".to_string(),
                targets: Vec::new(),
                cargo_targets: Vec::new(),
                frameworks: Vec::new(),
                framework_capabilities: Vec::new(),
                rust_version: None,
            },
        ];
        let project = ProjectInfo {
            root_dir: directory.path().to_path_buf(),
            name: "workspace".to_string(),
            version: "0.1.0".to_string(),
            package_id: "workspace 0.1.0".to_string(),
            targets: Vec::new(),
            cargo_targets: Vec::new(),
            edition: "2024".to_string(),
            frameworks: Vec::new(),
            framework_capabilities: Vec::new(),
            is_workspace: true,
            member_count: members.len(),
            has_build_script: false,
            rust_version: None,
            is_no_std: false,
            package_metadata: serde_json::json!({}),
            workspace_members: members,
            default_member_ids: Vec::new(),
        };
        let mut resolved = config::resolve_config_defaults(None);
        resolved.adapter_policy = config::AdapterPolicy::none();
        resolved.max_parallelism = Some(2);
        let roots = vec![first_root.clone(), second_root.clone()];

        let output = run_passes(
            &project,
            &resolved,
            &roots,
            &diff::ScopePlan::full(),
            true,
            true,
            &ScanControl::unlimited(),
            &[Category::Performance],
        );

        let actual_roots: Vec<_> = output
            .package_executions
            .iter()
            .map(|package| package.package_root.clone())
            .collect();
        assert_eq!(actual_roots, vec![first_root, second_root]);
    }

    #[test]
    fn typed_policy_changes_severity_and_audits_security_suppression() {
        let file_config: config::FileConfig = toml::from_str(
            r#"
            [rules.unwrap-in-production]
            severity = "error"

            [rules.hardcoded-secrets]
            severity = "off"
            "#,
        )
        .unwrap();
        let resolved = config::resolve_config_defaults(Some(&file_config));
        let mut security = make_diagnostic("hardcoded-secrets", Severity::Error, Some(2));
        security.category = Category::Security;
        let (included, suppressed, counts) = apply_rule_configuration(
            vec![
                make_diagnostic("unwrap-in-production", Severity::Warning, Some(1)),
                security,
            ],
            &resolved,
        );
        assert_eq!(included.len(), 1);
        assert_eq!(included[0].severity, Severity::Error);
        assert_eq!(suppressed.len(), 1);
        assert_eq!(suppressed[0].rule, "hardcoded-secrets");
        assert_eq!(counts.rule, 1);
        assert_eq!(counts.security_policy, 1);
    }

    #[test]
    fn path_override_has_final_precedence() {
        let file_config: config::FileConfig = toml::from_str(
            r#"
            [rules.unwrap-in-production]
            severity = "error"

            [[path_overrides]]
            pattern = "tests/**"
            severity = "off"
            "#,
        )
        .unwrap();
        let resolved = config::resolve_config_defaults(Some(&file_config));
        let mut diagnostic = make_diagnostic("unwrap-in-production", Severity::Warning, Some(1));
        diagnostic.file_path = PathBuf::from("tests/integration.rs");
        let (included, _, counts) = apply_rule_configuration(vec![diagnostic], &resolved);
        assert!(included.is_empty());
        assert_eq!(counts.path, 1);
    }

    #[test]
    fn category_selection_is_exact_and_defaults_to_all() {
        assert!(category_requested(&[], &Category::Security));
        assert!(category_requested(
            &[Category::Security, Category::Performance],
            &Category::Security
        ));
        assert!(!category_requested(
            &[Category::Performance],
            &Category::Security
        ));
    }

    #[test]
    fn workspace_passes_follow_selected_categories() {
        assert!(build_workspace_global_passes(false, &[Category::Performance]).is_empty());
        let cargo_passes = build_workspace_global_passes(false, &[Category::Cargo]);
        assert_eq!(cargo_passes.len(), 1);
        assert_eq!(cargo_passes[0].name(), "dependencies (cargo-deny)");
        let dependency_passes = build_workspace_global_passes(false, &[Category::Dependencies]);
        assert!(
            dependency_passes
                .iter()
                .any(|pass| pass.name() == "dependencies (cargo-machete)")
        );
    }

    #[test]
    fn evaluation_profile_excludes_environment_dependent_adapters() {
        let mut resolved = config::resolve_config_defaults(None);
        resolved.evaluation_profile = true;
        resolved.dependencies = false;
        resolved.adapter_policy = config::AdapterPolicy {
            custom_ast: true,
            ..config::AdapterPolicy::none()
        };
        let (_, project, _) = crate::discovery::bootstrap_project(Path::new("."), true).unwrap();
        let passes = build_passes(&project, Path::new("."), &resolved, true, None, &[]);
        assert!(passes.iter().any(|pass| pass.name() == "custom rules"));
        assert!(!passes.iter().any(|pass| pass.name() == "clippy"));
        assert!(
            passes
                .iter()
                .all(|pass| !pass.name().starts_with("dependencies"))
        );
    }
}
