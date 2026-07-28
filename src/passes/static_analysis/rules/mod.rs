pub mod async_rules;
pub mod complexity;
pub mod context;
pub mod error_handling;
pub mod framework;
pub mod framework_packs;
pub mod performance;
pub mod reliability;
pub mod security;
pub mod tranche;

pub use context::{
    AbstentionReceipt, ContextRequirement, CrateRole, PackageContext, PackageIdentity, RuleContext,
    SourceOrigin,
};

use crate::cache::{self, ScanCache};
use crate::catalog::{Confidence, NumericRange};
use crate::diagnostics::{Category, Diagnostic, Severity, SourceSurface};
use crate::discovery::CargoTargetContext;
use crate::scanner::{self, AnalysisPass};
use globset::GlobSet;
use rayon::prelude::*;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

const MAX_RS_FILE_SIZE: u64 = 10 * 1024 * 1024;

// ─── Shared helpers for test-code detection ─────────────────────────────────

/// Check if an attribute list contains `#[test]`.
pub fn has_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("test"))
}

/// Check if an attribute list contains `#[cfg(test)]`.
pub fn has_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("cfg") {
            return false;
        }
        attr.parse_args::<syn::Ident>()
            .is_ok_and(|ident| ident == "test")
    })
}

/// True if `attrs` put the item in test-only context — either `#[test]` or a
/// bare `#[cfg(test)]` on the function itself. Shared by every `in_test` toggle
/// (`UnwrapVisitor`, `PanicVisitor`, `SecretVisitor`) so they detect test code
/// identically — `#[cfg(test)]` modules are skipped separately via
/// [`has_cfg_test`] at the module level.
pub fn is_test_context(attrs: &[syn::Attribute]) -> bool {
    has_test_attr(attrs) || has_cfg_test(attrs)
}

/// Trait for custom AST-based rules that clippy doesn't cover.
///
/// Rules must be `Send + Sync` for parallel file processing.
/// Metadata methods (`description`, `fix_hint`) co-locate documentation
/// with the implementation, so adding a new rule only requires changes
/// in one place.
pub trait CustomRule: Send + Sync {
    /// Unique rule identifier (e.g. "unwrap-in-production").
    fn name(&self) -> &'static str;

    /// Category this rule belongs to.
    fn category(&self) -> Category;

    /// Default severity for findings from this rule.
    fn severity(&self) -> Severity;

    /// Human-readable description of what this rule detects.
    fn description(&self) -> &'static str;

    /// Actionable fix guidance for violations found by this rule.
    fn fix_hint(&self) -> &'static str;

    /// Whether this rule is enabled by default. Rules returning `false` are
    /// opt-in only — they run only when not listed in `ignore.rules` AND the
    /// scanner explicitly includes opt-in rules (e.g., via `--strict` or config).
    fn default_enabled(&self) -> bool {
        true
    }

    /// Confidence of this rule's analyzer evidence.
    fn confidence(&self) -> Confidence {
        Confidence::Medium
    }

    /// Framework capabilities required before the rule is applicable.
    fn applicable_frameworks(&self) -> &'static [&'static str] {
        &[]
    }

    /// Supported dependency versions for each applicable framework.
    fn framework_version_requirements(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    /// Cargo features required for framework-specific evidence.
    fn required_framework_features(&self) -> &'static [(&'static str, &'static [&'static str])] {
        &[]
    }

    /// Accepted inclusive range for a configurable numeric threshold.
    fn supported_threshold(&self) -> Option<NumericRange> {
        None
    }

    /// Apply a validated threshold before analysis starts.
    fn set_threshold(&mut self, _threshold: u32) {}

    /// Context this rule cannot decide without.
    ///
    /// The engine checks every requirement before traversal. An unavailable or
    /// ambiguous requirement produces an abstention receipt instead of a
    /// diagnostic, so a rule never emits on evidence it does not have (US-006).
    fn required_context(&self) -> &'static [ContextRequirement] {
        &[]
    }

    /// Crate roles this rule declares unsupported. Proc-macro crates compile to
    /// a compiler plugin and classify as libraries on the wire, so a rule that
    /// does not hold there declares it here.
    fn unsupported_crate_roles(&self) -> &'static [CrateRole] {
        &[]
    }

    /// Check a parsed Rust file and return diagnostics.
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic>;

    /// Check a parsed Rust file with Cargo-owned source context. Rules whose
    /// behavior does not vary by target can keep implementing `check_file`.
    fn check_file_with_context(
        &self,
        syntax: &syn::File,
        path: &Path,
        _context: RuleContext<'_>,
    ) -> Vec<Diagnostic> {
        self.check_file(syntax, path)
    }

    /// Helper to construct a `Diagnostic` using this rule's metadata.
    fn diagnostic(
        &self,
        path: &Path,
        message: String,
        help: Option<String>,
        line: Option<u32>,
        column: Option<u32>,
    ) -> Diagnostic {
        Diagnostic {
            file_path: path.to_path_buf(),
            rule: self.name().to_string(),
            category: self.category(),
            severity: self.severity(),
            message,
            help,
            line,
            column,
            fix: None,
        }
    }
}

/// Source surfaces a rule is excluded from before traversal when its catalog
/// contract does not list them. `Unknown` is handled separately as missing
/// evidence: a bounded catalog contract cannot prove that an unclassified file
/// belongs to one of its supported surfaces.
const EXCLUDABLE_SURFACES: &[SourceSurface] = &[
    SourceSurface::Test,
    SourceSurface::Bench,
    SourceSurface::Example,
    SourceSurface::BuildScript,
    SourceSurface::Generated,
    SourceSurface::MacroExpansion,
];

/// Applicability contract for one rule, resolved once per scan.
struct RuleGate {
    /// Source surfaces the catalog declares supported. Empty means unrestricted.
    supported_contexts: Vec<SourceSurface>,
    required_context: &'static [ContextRequirement],
    unsupported_crate_roles: &'static [CrateRole],
}

impl RuleGate {
    fn for_rule(rule: &dyn CustomRule) -> Self {
        let supported_contexts = crate::catalog::built_in_catalog()
            .ok()
            .and_then(|catalog| catalog.exact(rule.name()))
            .map(|descriptor| descriptor.trust.supported_contexts.clone())
            .unwrap_or_default();
        Self {
            supported_contexts,
            required_context: rule.required_context(),
            unsupported_crate_roles: rule.unsupported_crate_roles(),
        }
    }

    /// Stable abstention reason when this rule must not decide on `context`.
    fn abstention_reason(&self, context: RuleContext<'_>) -> Option<String> {
        if self
            .unsupported_crate_roles
            .contains(&context.package.crate_role)
        {
            return Some(format!(
                "unsupported-crate-role:{}",
                context.package.crate_role.as_str()
            ));
        }
        // A rule with no declared context contract is unrestricted: only a rule
        // that says where it holds can be excluded from anywhere.
        if self.supported_contexts.is_empty() {
            return context
                .first_missing(self.required_context)
                .map(|missing| format!("missing-context:{missing}"));
        }
        if context.source_surface == SourceSurface::Unknown {
            return Some("missing-context:source-surface".to_string());
        }
        if context.origin != SourceOrigin::Authored
            && !self.supported_contexts.contains(&context.source_surface)
        {
            return Some(format!("unsupported-origin:{}", context.origin.as_str()));
        }
        if EXCLUDABLE_SURFACES.contains(&context.source_surface)
            && !self.supported_contexts.contains(&context.source_surface)
        {
            return Some(format!(
                "unsupported-source-surface:{}",
                surface_name(context.source_surface)
            ));
        }
        context
            .first_missing(self.required_context)
            .map(|missing| format!("missing-context:{missing}"))
    }
}

const fn surface_name(surface: SourceSurface) -> &'static str {
    match surface {
        SourceSurface::Library => "library",
        SourceSurface::Binary => "binary",
        SourceSurface::Test => "test",
        SourceSurface::Bench => "bench",
        SourceSurface::Example => "example",
        SourceSurface::BuildScript => "build-script",
        SourceSurface::Generated => "generated",
        SourceSurface::MacroExpansion => "macro-expansion",
        SourceSurface::Unknown => "unknown",
    }
}

/// The rule engine: runs custom rules against all `.rs` files in parallel.
pub struct RuleEngine {
    rules: Vec<Box<dyn CustomRule>>,
    /// Per-scan abstention events, aggregated into receipts by the pass.
    abstentions: Mutex<Vec<(String, String)>>,
}

impl RuleEngine {
    /// Create a new rule engine with the given rules.
    pub fn new(rules: Vec<Box<dyn CustomRule>>) -> Self {
        Self {
            rules,
            abstentions: Mutex::new(Vec::new()),
        }
    }

    /// Drain the abstention receipts recorded by the last scan.
    fn take_abstentions(&self) -> Vec<AbstentionReceipt> {
        let events = self
            .abstentions
            .lock()
            .map_or_else(|_| Vec::new(), |mut events| std::mem::take(&mut *events));
        context::aggregate_abstentions(events)
    }

    /// Scan with full config context for cache key computation.
    ///
    /// This is the main implementation; [`scan`](Self::scan) delegates here
    /// with empty config slices for backward compatibility.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "kept as the full-scan compatibility entry point")
    )]
    pub fn scan_with_config(
        &self,
        project_root: &Path,
        ignore_files: &[String],
        ignore_rules: &[String],
        enable_rules: &[String],
    ) -> Vec<Diagnostic> {
        self.scan_selected_with_context(
            project_root,
            None,
            ignore_files,
            ignore_rules,
            enable_rules,
            &[],
            &context::UNRESOLVED_PACKAGE,
            false,
            None,
        )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "file selection, cache partitioning, panic isolation, and parallel rule execution form one rule-engine transaction"
    )]
    #[expect(
        clippy::too_many_arguments,
        reason = "the optional progress sink belongs to the existing rule-engine transaction context"
    )]
    fn scan_selected_with_context(
        &self,
        project_root: &Path,
        selected_files: Option<&[PathBuf]>,
        ignore_files: &[String],
        ignore_rules: &[String],
        enable_rules: &[String],
        cargo_targets: &[CargoTargetContext],
        package: &PackageContext,
        skip_fully_abstained_files: bool,
        on_file_progress: Option<&scanner::FileProgressCallback>,
    ) -> Vec<Diagnostic> {
        if self.rules.is_empty() {
            return vec![];
        }
        let gates: Vec<RuleGate> = self
            .rules
            .iter()
            .map(|rule| RuleGate::for_rule(rule.as_ref()))
            .collect();

        let mut files = selected_files.map_or_else(
            || scanner::collect_rs_files(project_root),
            |selected| {
                selected
                    .iter()
                    .map(|path| {
                        if path.is_absolute() {
                            path.clone()
                        } else {
                            project_root.join(path)
                        }
                    })
                    .filter(|path| path.starts_with(project_root) && path.is_file())
                    .collect()
            },
        );
        files.sort();
        files.dedup();
        if files.is_empty() {
            return vec![];
        }

        // Build ignore glob set
        let ignore_set = build_ignore_set(ignore_files);

        // Compute config hash including active rule names so cache
        // invalidates when rules are added or removed.
        let active_rule_fingerprints: Vec<String> = self
            .rules
            .iter()
            .map(|rule| {
                format!(
                    "{}:{:?}:{:?}:{:?}:{:?}:{:?}",
                    rule.name(),
                    rule.category(),
                    rule.severity(),
                    rule.default_enabled(),
                    rule.confidence(),
                    rule.supported_threshold()
                )
            })
            .chain(cargo_targets.iter().map(|target| {
                format!(
                    "cargo-target:{}:{}:{:?}:{}",
                    target.name,
                    target.src_path.display(),
                    target.source_surface,
                    target.is_proc_macro
                )
            }))
            // Package context selects which rules run, so it belongs in the
            // cache key: two members with different features or editions must
            // not share cached diagnostics.
            .chain(std::iter::once(format!(
                "package:{}:{}:{}:{}:{}:{}:{}",
                package.package_id,
                package.crate_role.as_str(),
                package.edition.as_deref().unwrap_or("-"),
                package.declared_msrv.as_deref().unwrap_or("-"),
                package.enabled_features.join(","),
                package.frameworks.join(","),
                package.metadata_complete
            )))
            .chain(std::iter::once(format!(
                "skip-fully-abstained-files:{skip_fully_abstained_files}"
            )))
            .collect();
        let config_hash = cache::compute_config_hash(
            project_root,
            ignore_rules,
            ignore_files,
            enable_rules,
            &active_rule_fingerprints,
        );
        let mut scan_cache = ScanCache::load(project_root, &config_hash)
            .unwrap_or_else(|| ScanCache::new(config_hash.clone()));

        // Resolve applicability before touching file contents. A peripheral
        // fixture may be invalid Rust or non-UTF-8 by design; if every rule
        // abstains on its source surface, it is not required analysis.
        let control = crate::process::current_scan_control();
        let mut all_diagnostics = Vec::new();
        let mut abstention_events = Vec::new();
        let file_contents: Vec<(std::path::PathBuf, String)> = files
            .into_iter()
            .filter_map(|file_path| {
                if control.is_stopped() {
                    return None;
                }
                let rel_path = file_path.strip_prefix(project_root).unwrap_or(&file_path);
                if let Ok(ref set) = ignore_set
                    && set.is_match(rel_path)
                {
                    return None;
                }
                let source_context = RuleContext::for_path(&file_path, cargo_targets, package);
                let mut applicable = false;
                for (rule, gate) in self.rules.iter().zip(&gates) {
                    if let Some(reason) = gate.abstention_reason(source_context) {
                        abstention_events.push((rule.name().to_string(), reason));
                    } else {
                        applicable = true;
                    }
                }
                if !applicable && skip_fully_abstained_files {
                    return None;
                }
                match std::fs::metadata(&file_path) {
                    Ok(metadata) if metadata.len() > MAX_RS_FILE_SIZE => {
                        all_diagnostics.push(analysis_failure(
                            rel_path,
                            format!("oversized_file:{}", metadata.len()),
                        ));
                        return None;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        all_diagnostics.push(analysis_failure(
                            rel_path,
                            format!("metadata_failed:{error}"),
                        ));
                        return None;
                    }
                }
                match std::fs::read_to_string(&file_path) {
                    Ok(content) => Some((file_path, content)),
                    Err(error) => {
                        eprintln!("Warning: could not read '{}': {error}", file_path.display());
                        all_diagnostics
                            .push(analysis_failure(rel_path, format!("read_failed:{error}")));
                        None
                    }
                }
            })
            .collect();
        if let Ok(mut recorded) = self.abstentions.lock() {
            recorded.extend(abstention_events);
        }
        let total_file_count = file_contents.len();
        let worker_count = rayon::current_num_threads();

        // Partition into fresh (cache hit) and stale (need scanning) files,
        // keeping the pre-computed hash for stale files to avoid double hashing.
        let mut fresh_files = Vec::new();
        let mut stale_files = Vec::new();
        for (file_path, content) in &file_contents {
            let rel_path = file_path.strip_prefix(project_root).unwrap_or(file_path);
            let (is_fresh, hash) = scan_cache.is_fresh_with_hash(rel_path, content);
            if is_fresh {
                fresh_files.push((file_path, content));
            } else {
                stale_files.push((file_path, content, hash));
            }
        }

        // Collect cached diagnostics for fresh files
        all_diagnostics.extend(fresh_files.iter().flat_map(|(file_path, _content)| {
            let rel_path = file_path.strip_prefix(project_root).unwrap_or(file_path);
            scan_cache
                .get_cached_diagnostics(rel_path)
                .unwrap_or(&[])
                .to_vec()
        }));
        let started_file_count = AtomicUsize::new(fresh_files.len());
        let scanned_file_count = AtomicUsize::new(fresh_files.len());
        if !fresh_files.is_empty()
            && let Some(report_progress) = on_file_progress
        {
            report_progress(scanner::FileProgressUpdate {
                started: fresh_files.len(),
                scanned: fresh_files.len(),
                total: total_file_count,
                workers: worker_count,
            });
        }

        // Process stale files in parallel with rayon
        let stale_results: Vec<(std::path::PathBuf, String, Vec<Diagnostic>, bool)> = stale_files
            .par_iter()
            .filter_map(|&(file_path, content, ref hash)| {
                if control.is_stopped() {
                    return None;
                }
                let rel_path = file_path.strip_prefix(project_root).unwrap_or(file_path);
                let started = started_file_count.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(report_progress) = on_file_progress {
                    report_progress(scanner::FileProgressUpdate {
                        started,
                        scanned: scanned_file_count.load(Ordering::Relaxed),
                        total: total_file_count,
                        workers: worker_count,
                    });
                }

                let (diagnostics, cacheable) = match syn::parse_file(content) {
                    Ok(syntax) => {
                        let mut diagnostics = Vec::new();
                        let mut cacheable = true;
                        let context = RuleContext::for_path(file_path, cargo_targets, package);
                        for (rule, gate) in self.rules.iter().zip(&gates) {
                            // Excluded before traversal: an abstaining rule never
                            // walks the AST, so it cannot emit on evidence it
                            // declared it does not have.
                            if gate.abstention_reason(context).is_some() {
                                continue;
                            }
                            let (mut findings, failure) = run_rule_safely_with_status(
                                rule.as_ref(),
                                &syntax,
                                rel_path,
                                context,
                            );
                            diagnostics.append(&mut findings);
                            if let Some(failure) = failure {
                                diagnostics.push(analysis_failure(
                                    rel_path,
                                    format!("rule_panicked:{}:{failure}", rule.name()),
                                ));
                                cacheable = false;
                            }
                        }
                        (diagnostics, cacheable)
                    }
                    Err(error) => {
                        eprintln!("Warning: parse error in '{}': {error}", rel_path.display());
                        (
                            vec![analysis_failure(rel_path, format!("parse_failed:{error}"))],
                            false,
                        )
                    }
                };

                let scanned = scanned_file_count.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(report_progress) = on_file_progress {
                    report_progress(scanner::FileProgressUpdate {
                        started: started_file_count.load(Ordering::Relaxed),
                        scanned,
                        total: total_file_count,
                        workers: worker_count,
                    });
                }
                Some((rel_path.to_path_buf(), hash.clone(), diagnostics, cacheable))
            })
            .collect();

        // Update the cache with newly scanned results using pre-computed hashes
        for (rel_path, hash, diagnostics, cacheable) in stale_results {
            all_diagnostics.extend_from_slice(&diagnostics);
            if cacheable {
                scan_cache.update_with_hash(&rel_path, hash, diagnostics);
            }
        }

        // Persist the updated cache (best-effort)
        scan_cache.save(project_root);

        all_diagnostics
    }
}

/// Run a single rule with panic isolation.
#[cfg(feature = "lsp")]
fn run_rule_safely(
    rule: &dyn CustomRule,
    syntax: &syn::File,
    path: &Path,
    context: RuleContext<'_>,
) -> Vec<Diagnostic> {
    run_rule_safely_with_status(rule, syntax, path, context).0
}

fn run_rule_safely_with_status(
    rule: &dyn CustomRule,
    syntax: &syn::File,
    path: &Path,
    context: RuleContext<'_>,
) -> (Vec<Diagnostic>, Option<String>) {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        rule.check_file_with_context(syntax, path, context)
    }));

    match result {
        Ok(diagnostics) => (diagnostics, None),
        Err(payload) => {
            let msg = payload.downcast_ref::<&'static str>().map_or_else(
                || {
                    payload
                        .downcast_ref::<String>()
                        .map_or_else(|| "<non-string panic>".to_string(), String::clone)
                },
                |s| (*s).to_string(),
            );
            eprintln!(
                "Warning: rule '{}' panicked on '{}': {msg}",
                rule.name(),
                path.display()
            );
            (vec![], Some(msg))
        }
    }
}

fn analysis_failure(path: &Path, message: String) -> Diagnostic {
    Diagnostic {
        file_path: path.to_path_buf(),
        rule: scanner::ANALYSIS_FAILURE_RULE.to_string(),
        category: Category::Correctness,
        severity: Severity::Info,
        message,
        help: None,
        line: None,
        column: None,
        fix: None,
    }
}

/// Return all custom rules across all categories.
/// Used to derive the rule registry and documentation at startup.
pub fn all_custom_rules() -> Vec<Box<dyn CustomRule>> {
    error_handling::all_rules()
        .into_iter()
        .chain(performance::all_rules())
        .chain(reliability::all_rules())
        .chain(complexity::all_rules())
        .chain(security::all_rules())
        .chain(tranche::all_rules())
        .chain(async_rules::all_rules())
        .chain(framework::all_rules())
        .chain(framework_packs::all_rules())
        .collect()
}

/// Stable limitation IDs declared by the custom-rule catalog and mirrored by
/// conformance fixtures.
pub fn documented_limitations(rule: &str) -> &'static [&'static str] {
    match rule {
        "unwrap-in-production"
        | "panic-in-library"
        | "high-cyclomatic-complexity"
        | "excessive-clone"
        | "string-from-literal"
        | "hardcoded-secrets" => &["test-function-exception", "cfg-test-exception"],
        "box-dyn-error-in-public-api" | "result-unit-error" => &["test-function-exception"],
        "unsafe-block-audit" => &["forbid-unsafe-code-short-circuit"],
        "sql-injection-risk" => &["immediate-format-only"],
        "blocking-in-async" => &["method-call-types-unknown"],
        "tokio-main-missing" => &["main-file-only"],
        "tokio-spawn-without-move" => &["async-block-syntax-only"],
        "axum-handler-not-async" => &["top-level-functions-only"],
        "actix-blocking-handler" => &["web-prefix-only"],
        _ => &[],
    }
}

/// Analyze one in-memory editor document without touching the project cache or
/// launching package-global adapters. Policy and framework gates are resolved
/// before each custom rule executes.
#[cfg(feature = "lsp")]
pub fn analyze_editor_source(
    source: &str,
    path: &Path,
    config: &crate::config::ResolvedConfig,
    capabilities: &[crate::discovery::FrameworkCapability],
    cargo_targets: &[CargoTargetContext],
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<Vec<Diagnostic>, syn::Error> {
    use std::sync::atomic::Ordering;

    let syntax = syn::parse_file(source)?;
    let context = RuleContext::for_path(path, cargo_targets, &context::UNRESOLVED_PACKAGE);
    let catalog = crate::catalog::built_in_catalog().ok();
    let mut diagnostics = Vec::new();
    for mut rule in all_custom_rules() {
        if cancelled.load(Ordering::Acquire) {
            break;
        }
        let Some(descriptor) = catalog.and_then(|catalog| catalog.exact(rule.name())) else {
            continue;
        };
        if !descriptor.applicable_frameworks.is_empty()
            && framework_packs::capability_decision(rule.as_ref(), capabilities).is_err()
        {
            continue;
        }
        if RuleGate::for_rule(rule.as_ref())
            .abstention_reason(context)
            .is_some()
        {
            continue;
        }
        let policy = config.rule_policy(descriptor, Some(path));
        let Some(severity) = policy.severity else {
            continue;
        };
        if let Some(threshold) = policy.threshold {
            rule.set_threshold(threshold);
        }
        let mut findings = run_rule_safely(rule.as_ref(), &syntax, path, context);
        for finding in &mut findings {
            finding.severity = severity;
        }
        diagnostics.extend(findings);
    }
    Ok(diagnostics)
}

/// Names of every syn-based custom rule — the "heuristic" set, computed once.
static HEURISTIC_RULE_NAMES: std::sync::LazyLock<std::collections::HashSet<&'static str>> =
    std::sync::LazyLock::new(|| all_custom_rules().iter().map(|r| r.name()).collect());

/// Returns `true` if `rule` is a syn-only custom rule — a heuristic with no
/// type information or macro expansion — as opposed to a type-aware clippy lint
/// or an external-tool finding (cargo-audit, cargo-deny, …).
///
/// Used to mark diagnostics so users can calibrate their confidence: a clippy
/// lint resolved against the `TyCtxt` is more authoritative than a name-based
/// AST heuristic (US-013).
#[must_use]
pub fn is_heuristic_rule(rule: &str) -> bool {
    HEURISTIC_RULE_NAMES.contains(rule)
}

/// Check if the tail of `actual` matches `pattern` exactly.
/// Used by async and framework rules to match blocking-call path segments.
#[inline]
pub fn segments_match(actual: &[&str], pattern: &[&str]) -> bool {
    actual.len() >= pattern.len() && actual.ends_with(pattern)
}

fn build_ignore_set(patterns: &[String]) -> Result<GlobSet, globset::Error> {
    scanner::build_glob_set(patterns)
}

/// Analysis pass that wraps the rule engine for the scan orchestrator.
pub struct RuleEnginePass {
    engine: RuleEngine,
    ignore_files: Vec<String>,
    ignore_rules: Vec<String>,
    enable_rules: Vec<String>,
    selected_files: Option<Vec<PathBuf>>,
    cargo_targets: Vec<CargoTargetContext>,
    package: PackageContext,
    skip_fully_abstained_files: bool,
}

impl RuleEnginePass {
    /// Create a new pass with full config context for incremental caching.
    pub fn with_config(
        rules: Vec<Box<dyn CustomRule>>,
        ignore_files: Vec<String>,
        ignore_rules: Vec<String>,
        enable_rules: Vec<String>,
    ) -> Self {
        Self {
            engine: RuleEngine::new(rules),
            ignore_files,
            ignore_rules,
            enable_rules,
            selected_files: None,
            cargo_targets: Vec::new(),
            package: PackageContext::unresolved(),
            skip_fully_abstained_files: false,
        }
    }

    pub fn with_selected_files(mut self, selected_files: Vec<PathBuf>) -> Self {
        self.selected_files = Some(selected_files);
        self
    }

    pub fn with_cargo_targets(mut self, cargo_targets: Vec<CargoTargetContext>) -> Self {
        self.cargo_targets = cargo_targets;
        self
    }

    /// Attach the Cargo-resolved context of the package this pass analyzes.
    pub fn with_package_context(mut self, package: PackageContext) -> Self {
        self.package = package;
        self
    }

    /// In corpus evaluation, files outside every rule's declared surfaces are
    /// evidence of abstention rather than required parser inputs.
    pub const fn skip_fully_abstained_files(mut self, enabled: bool) -> Self {
        self.skip_fully_abstained_files = enabled;
        self
    }

    fn run_engine(
        &self,
        project_root: &Path,
        on_file_progress: Option<&scanner::FileProgressCallback>,
    ) -> Vec<Diagnostic> {
        self.engine.scan_selected_with_context(
            project_root,
            self.selected_files.as_deref(),
            &self.ignore_files,
            &self.ignore_rules,
            &self.enable_rules,
            &self.cargo_targets,
            &self.package,
            self.skip_fully_abstained_files,
            on_file_progress,
        )
    }
}

impl AnalysisPass for RuleEnginePass {
    fn name(&self) -> &'static str {
        "custom rules"
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        Ok(self.run_engine(project_root, None))
    }

    fn run_with_progress(
        &self,
        project_root: &Path,
        on_file_progress: &scanner::FileProgressCallback,
    ) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        Ok(self.run_engine(project_root, Some(on_file_progress)))
    }

    fn take_abstentions(&self) -> Vec<AbstentionReceipt> {
        self.engine.take_abstentions()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    // --- Test rule implementations ---

    struct CountFnRule;

    impl CustomRule for CountFnRule {
        fn name(&self) -> &'static str {
            "count-fn"
        }
        fn category(&self) -> Category {
            Category::Architecture
        }
        fn severity(&self) -> Severity {
            Severity::Warning
        }
        fn description(&self) -> &'static str {
            "test rule"
        }
        fn fix_hint(&self) -> &'static str {
            "test fix"
        }
        fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
            let fn_count = syntax
                .items
                .iter()
                .filter(|item| matches!(item, syn::Item::Fn(_)))
                .count();
            if fn_count > 10 {
                vec![Diagnostic {
                    file_path: path.to_path_buf(),
                    rule: self.name().to_string(),
                    category: self.category(),
                    severity: self.severity(),
                    message: format!("File has {fn_count} functions (threshold: 10)"),
                    help: None,
                    line: None,
                    column: None,
                    fix: None,
                }]
            } else {
                vec![]
            }
        }
    }

    struct PanickingRule;

    impl CustomRule for PanickingRule {
        fn name(&self) -> &'static str {
            "panicking-rule"
        }
        fn category(&self) -> Category {
            Category::Correctness
        }
        fn severity(&self) -> Severity {
            Severity::Error
        }
        fn description(&self) -> &'static str {
            "test rule"
        }
        fn fix_hint(&self) -> &'static str {
            "test fix"
        }
        fn check_file(&self, _syntax: &syn::File, _path: &Path) -> Vec<Diagnostic> {
            panic!("intentional test panic");
        }
    }

    struct AlwaysWarnsRule;

    impl CustomRule for AlwaysWarnsRule {
        fn name(&self) -> &'static str {
            "always-warns"
        }
        fn category(&self) -> Category {
            Category::Style
        }
        fn severity(&self) -> Severity {
            Severity::Warning
        }
        fn description(&self) -> &'static str {
            "test rule"
        }
        fn fix_hint(&self) -> &'static str {
            "test fix"
        }
        fn check_file(&self, _syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
            vec![Diagnostic {
                file_path: path.to_path_buf(),
                rule: self.name().to_string(),
                category: self.category(),
                severity: self.severity(),
                message: "Test warning".to_string(),
                help: None,
                line: None,
                column: None,
                fix: None,
            }]
        }
    }

    // --- Tests ---

    fn make_temp_project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        for (filename, content) in files {
            let path = src_dir.join(filename);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            let mut f = std::fs::File::create(&path).unwrap();
            write!(f, "{content}").unwrap();
        }
        dir
    }

    #[derive(Debug)]
    struct ConformanceCase {
        kind: String,
        path: PathBuf,
        source: String,
    }

    #[derive(Debug)]
    struct ConformanceRule {
        id: String,
        cases: Vec<ConformanceCase>,
    }

    fn conformance_manifest() -> Vec<ConformanceRule> {
        let input = include_str!("../../../../tests/fixtures/rules/conformance.txt");
        let mut rules = Vec::new();
        let mut current_rule: Option<ConformanceRule> = None;
        let mut current_case: Option<ConformanceCase> = None;

        let finish_case = |rule: &mut Option<ConformanceRule>,
                           case: &mut Option<ConformanceCase>| {
            if let Some(case) = case.take() {
                rule.as_mut()
                    .expect("a case must follow a rule header")
                    .cases
                    .push(case);
            }
        };

        for line in input.lines() {
            if let Some(id) = line.strip_prefix("=== ") {
                finish_case(&mut current_rule, &mut current_case);
                if let Some(rule) = current_rule.take() {
                    rules.push(rule);
                }
                current_rule = Some(ConformanceRule {
                    id: id.trim().to_string(),
                    cases: Vec::new(),
                });
                continue;
            }
            if let Some(header) = line.strip_prefix("--- ") {
                finish_case(&mut current_rule, &mut current_case);
                let (kind, path) = header
                    .split_once(' ')
                    .expect("fixture header must contain a kind and path");
                current_case = Some(ConformanceCase {
                    kind: kind.to_string(),
                    path: PathBuf::from(path),
                    source: String::new(),
                });
                continue;
            }
            if let Some(case) = current_case.as_mut() {
                case.source.push_str(line);
                case.source.push('\n');
            }
        }
        finish_case(&mut current_rule, &mut current_case);
        if let Some(rule) = current_rule {
            rules.push(rule);
        }
        rules
    }

    fn rule_panics(rule: &dyn CustomRule, input: &[u8]) -> bool {
        panic::catch_unwind(AssertUnwindSafe(|| {
            let Ok(source) = std::str::from_utf8(input) else {
                return;
            };
            let Ok(syntax) = syn::parse_file(source) else {
                return;
            };
            let _ = rule.check_file(&syntax, Path::new("mutation.rs"));
        }))
        .is_err()
    }

    fn minimize_panicking_input(rule: &dyn CustomRule, mut input: Vec<u8>) -> Vec<u8> {
        let mut chunk = input.len().div_ceil(2);
        while chunk > 0 {
            let mut removed = false;
            let mut start = 0;
            while start < input.len() {
                let end = (start + chunk).min(input.len());
                let mut candidate = input.clone();
                candidate.drain(start..end);
                if rule_panics(rule, &candidate) {
                    input = candidate;
                    removed = true;
                    break;
                }
                start += chunk;
            }
            if !removed {
                chunk /= 2;
            }
        }
        input
    }

    fn xorshift(mut value: u64) -> u64 {
        value ^= value << 13;
        value ^= value >> 7;
        value ^ (value << 17)
    }

    fn mutate_source(seed: u64, source: &str) -> Vec<u8> {
        let mut bytes = source.as_bytes().to_vec();
        if bytes.is_empty() {
            bytes.extend_from_slice(b"fn empty() {}\n");
        }
        let random = xorshift(seed);
        let index = (random as usize) % bytes.len();
        match random % 7 {
            0 => bytes.insert(index, b' '),
            1 => {
                bytes.remove(index);
            }
            2 => bytes.truncate(index),
            3 => {
                let end = (index + 8).min(bytes.len());
                let duplicate = bytes[index..end].to_vec();
                bytes.splice(index..index, duplicate);
            }
            4 => {
                bytes.splice(0..0, b"#[unknown_tool::attribute]\n".iter().copied());
            }
            5 => bytes[index] ^= 0x80,
            _ => bytes.extend_from_slice(b"\nconst _: () = ();\n"),
        }
        bytes
    }

    fn stable_seed(rule: &str, iteration: u64) -> u64 {
        rule.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x1000_0000_01b3)
        }) ^ iteration.wrapping_mul(0x9e37_79b9_7f4a_7c15)
    }

    fn adversarial_input(iteration: usize, source: &str, seed: u64) -> Vec<u8> {
        match iteration {
            0 => b"fn truncated( {".to_vec(),
            1 => vec![b'f', b'n', b' ', 0xf0, 0x9f, 0x92],
            2 => {
                let mut deep = String::from("fn deep() {");
                deep.push_str(&"{".repeat(64));
                deep.push_str("let _value = 1;");
                deep.push_str(&"}".repeat(64));
                deep.push('}');
                deep.into_bytes()
            }
            3 => b"#[unknown(attribute)] fn attributed() {}".to_vec(),
            _ => mutate_source(seed, source),
        }
    }

    fn fires(rule: &dyn CustomRule, source: &str, path: &Path) -> bool {
        let syntax = syn::parse_file(source).expect("metamorphic input must remain parseable");
        rule.check_file(&syntax, path)
            .iter()
            .any(|diagnostic| diagnostic.rule == rule.name())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the mutation oracle keeps each reproducibility boundary in execution order"
    )]
    fn run_mutations_for_rule(rule: &dyn CustomRule, fixture: &ConformanceRule) {
        const MUTATIONS_PER_RULE: usize = 1_000;
        const MIN_DISTINCT_PARSEABLE: usize = 100;
        let positive = fixture
            .cases
            .iter()
            .find(|case| case.kind == "positive")
            .expect("conformance requires a positive liveness fixture");
        let negative = fixture
            .cases
            .iter()
            .find(|case| case.kind == "negative")
            .expect("conformance requires a negative liveness fixture");
        write_mutation_reproduction(
            stable_seed(rule.name(), u64::MAX),
            positive.source.as_bytes(),
        );
        assert!(
            fires(rule, &positive.source, &positive.path),
            "{} failed its positive liveness oracle",
            rule.name()
        );
        write_mutation_reproduction(
            stable_seed(rule.name(), u64::MAX - 1),
            negative.source.as_bytes(),
        );
        assert!(
            !fires(rule, &negative.source, &negative.path),
            "{} failed its negative liveness oracle",
            rule.name()
        );
        for (iteration, case) in fixture
            .cases
            .iter()
            .filter(|case| matches!(case.kind.as_str(), "positive" | "negative"))
            .enumerate()
        {
            let seed = stable_seed(rule.name(), iteration as u64);
            let source = format!(
                "// rust-doctor-metamorphic-seed:{seed:016x}\n{}",
                case.source
            );
            let expected = case.kind == "positive";
            write_mutation_reproduction(seed, source.as_bytes());
            assert_eq!(
                fires(rule, &source, &case.path),
                expected,
                "metamorphic oracle failed: rule={} seed={seed:#018x} minimized_input={source:?}",
                rule.name()
            );
        }

        let mut parseable = HashSet::new();
        let mut fired = 0usize;
        let mut did_not_fire = 0usize;
        let mut parse_skips = 0usize;
        for iteration in 0..MUTATIONS_PER_RULE {
            let case = &fixture.cases[iteration % fixture.cases.len()];
            let seed = stable_seed(rule.name(), iteration as u64);
            let input = adversarial_input(iteration, &case.source, seed);
            write_mutation_reproduction(seed, &input);
            if rule_panics(rule, &input) {
                let minimized = minimize_panicking_input(rule, input);
                panic!(
                    "mutation panic: rule={} seed={seed:#018x} minimized_input={:?}",
                    rule.name(),
                    String::from_utf8_lossy(&minimized)
                );
            }
            let Ok(source) = std::str::from_utf8(&input) else {
                parse_skips += 1;
                continue;
            };
            let Ok(syntax) = syn::parse_file(source) else {
                parse_skips += 1;
                continue;
            };
            parseable.insert(hex_input(source));
            if rule
                .check_file(&syntax, &case.path)
                .iter()
                .any(|diagnostic| diagnostic.rule == rule.name())
            {
                fired += 1;
            } else {
                did_not_fire += 1;
            }
        }
        assert!(
            parseable.len() >= MIN_DISTINCT_PARSEABLE,
            "{} produced only {} distinct parseable mutations (parse_skips={parse_skips})",
            rule.name(),
            parseable.len()
        );
        assert!(
            parse_skips > 0,
            "{} did not exercise the parse-skip boundary",
            rule.name()
        );
        assert!(
            fired > 0 && did_not_fire > 0,
            "{} mutation oracle was non-live or fired universally: fired={fired}, quiet={did_not_fire}",
            rule.name()
        );
    }

    fn hex_input(source: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut digest = Sha256::new();
        digest.update(source.as_bytes());
        format!("{:x}", digest.finalize())
    }

    #[derive(Debug, PartialEq, Eq)]
    enum WorkerOutcome {
        Success,
        CaughtPanic(String),
        Abort(String),
        Timeout(String),
    }

    fn mutation_worker(rule: Option<&str>, mode: Option<&str>, timeout: Duration) -> WorkerOutcome {
        let reproduction_dir = tempfile::tempdir().expect("create mutation reproduction directory");
        let reproduction_path = reproduction_dir.path().join("active-mutation.txt");
        std::fs::write(
            &reproduction_path,
            "seed=0x0000000000000000\ninput=\"synthetic worker boundary\"\n",
        )
        .expect("initialize mutation reproduction");
        let mut command = Command::new(std::env::current_exe().expect("current test executable"));
        command
            .arg("mutation_worker_child")
            .args(["--ignored", "--nocapture"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("RUST_DOCTOR_MUTATION_REPRO", &reproduction_path);
        if let Some(rule) = rule {
            command.env("RUST_DOCTOR_MUTATION_RULE", rule);
        }
        if let Some(mode) = mode {
            command.env("RUST_DOCTOR_MUTATION_MODE", mode);
        }
        let mut child = command.spawn().expect("spawn mutation worker");
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().expect("poll mutation worker") {
                let output = child.wait_with_output().expect("collect mutation worker");
                if status.success() {
                    return WorkerOutcome::Success;
                }
                let stderr = String::from_utf8_lossy(&output.stderr);
                return if stderr.contains("mutation panic:") {
                    WorkerOutcome::CaughtPanic(format!(
                        "{stderr}\n{}",
                        read_mutation_reproduction(&reproduction_path)
                    ))
                } else {
                    WorkerOutcome::Abort(read_mutation_reproduction(&reproduction_path))
                };
            }
            if started.elapsed() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                return WorkerOutcome::Timeout(read_mutation_reproduction(&reproduction_path));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn write_mutation_reproduction(seed: u64, input: &[u8]) {
        let Ok(path) = std::env::var("RUST_DOCTOR_MUTATION_REPRO") else {
            return;
        };
        std::fs::write(
            path,
            format!(
                "seed={seed:#018x}\ninput={:?}\n",
                String::from_utf8_lossy(input)
            ),
        )
        .expect("persist active mutation reproduction");
    }

    fn read_mutation_reproduction(path: &Path) -> String {
        std::fs::read_to_string(path)
            .unwrap_or_else(|error| format!("reproduction unavailable: {error}"))
    }

    #[test]
    #[ignore = "subprocess entry point for the parent mutation harness"]
    fn mutation_worker_child() {
        match std::env::var("RUST_DOCTOR_MUTATION_MODE").as_deref() {
            Ok("abort") => std::process::abort(),
            Ok("panic") => panic!("mutation panic: synthetic worker boundary"),
            Ok("timeout") => thread::sleep(Duration::from_mins(1)),
            _ => {}
        }
        let selected =
            std::env::var("RUST_DOCTOR_MUTATION_RULE").expect("mutation worker requires a rule ID");
        let fixtures: HashMap<_, _> = conformance_manifest()
            .into_iter()
            .map(|fixture| (fixture.id.clone(), fixture))
            .collect();
        let rule = all_custom_rules()
            .into_iter()
            .find(|rule| rule.name() == selected)
            .expect("selected mutation rule must exist");
        run_mutations_for_rule(rule.as_ref(), &fixtures[&selected]);
    }

    #[test]
    fn conformance_manifest_covers_every_custom_rule() {
        let manifest = conformance_manifest();
        let mut ids = HashSet::new();
        for fixture in &manifest {
            assert!(ids.insert(fixture.id.as_str()), "duplicate {}", fixture.id);
            let positive = fixture
                .cases
                .iter()
                .filter(|case| case.kind == "positive")
                .count();
            let negative = fixture
                .cases
                .iter()
                .filter(|case| case.kind == "negative")
                .count();
            assert!(
                positive >= 2,
                "{} has {positive} positive fixtures",
                fixture.id
            );
            assert!(
                negative >= 4,
                "{} has {negative} negative fixtures",
                fixture.id
            );
        }

        let manifest_by_id: HashMap<_, _> = manifest
            .iter()
            .map(|fixture| (fixture.id.as_str(), fixture))
            .collect();
        let custom_rules = all_custom_rules();
        assert_eq!(manifest_by_id.len(), custom_rules.len());
        for rule in custom_rules {
            let fixture = manifest_by_id
                .get(rule.name())
                .unwrap_or_else(|| panic!("missing conformance manifest for {}", rule.name()));
            if rule.default_enabled() {
                assert!(
                    fixture.cases.iter().any(|case| case.kind == "positive")
                        && fixture.cases.iter().any(|case| case.kind == "negative"),
                    "default-enabled rule {} has incomplete conformance",
                    rule.name()
                );
            }
            let fixture_limitations: HashSet<_> = fixture
                .cases
                .iter()
                .filter_map(|case| case.kind.strip_prefix("limitation:"))
                .map(|limitation| format!("{}:{limitation}", rule.name()))
                .collect();
            let descriptor_limitations: HashSet<_> = crate::catalog::built_in_catalog()
                .unwrap()
                .exact(rule.name())
                .unwrap()
                .limitation_ids
                .iter()
                .cloned()
                .collect();
            assert_eq!(
                fixture_limitations,
                descriptor_limitations,
                "{} limitation fixtures and catalog IDs differ",
                rule.name()
            );
        }
    }

    #[test]
    fn conformance_fixtures_match_rule_behavior() {
        let rules: HashMap<_, _> = all_custom_rules()
            .into_iter()
            .map(|rule| (rule.name(), rule))
            .collect();
        for fixture in conformance_manifest() {
            let rule = rules.get(fixture.id.as_str()).unwrap();
            for case in fixture.cases {
                let syntax = syn::parse_file(&case.source).unwrap_or_else(|error| {
                    panic!(
                        "invalid fixture {} {}: {error}\n{}",
                        fixture.id, case.kind, case.source
                    )
                });
                let result =
                    panic::catch_unwind(AssertUnwindSafe(|| rule.check_file(&syntax, &case.path)));
                let diagnostics = result.unwrap_or_else(|_| {
                    panic!(
                        "rule {} panicked for {} fixture at {}",
                        fixture.id,
                        case.kind,
                        case.path.display()
                    )
                });
                if case.kind == "positive" {
                    assert!(
                        diagnostics.iter().any(|value| value.rule == fixture.id),
                        "{} did not fire for {}\n{}",
                        fixture.id,
                        case.path.display(),
                        case.source
                    );
                } else {
                    assert!(
                        diagnostics.iter().all(|value| value.rule != fixture.id),
                        "{} fired for {} fixture {}\n{}",
                        fixture.id,
                        case.kind,
                        case.path.display(),
                        case.source
                    );
                }
            }
        }
    }

    #[test]
    fn every_clippy_mapping_matches_its_catalog_descriptor() {
        let catalog = crate::catalog::built_in_catalog().unwrap();
        let mut seen = HashSet::new();
        for lint in crate::clippy::LINT_REGISTRY {
            assert!(
                seen.insert(lint.name),
                "duplicate Clippy mapping {}",
                lint.name
            );
            let canonical = format!("clippy::{}", lint.name);
            let descriptor = catalog.exact(&canonical).unwrap();
            assert_eq!(descriptor.category, lint.category, "{canonical}");
            assert_eq!(descriptor.default_severity, lint.severity, "{canonical}");
            assert_eq!(
                catalog.exact(lint.name).unwrap().canonical_id,
                canonical,
                "alias {}",
                lint.name
            );
        }
        assert_eq!(seen.len(), crate::clippy::LINT_REGISTRY.len());
    }

    #[test]
    fn seeded_mutations_are_subprocess_isolated_and_semantically_bounded() {
        for rule in all_custom_rules() {
            let outcome = mutation_worker(Some(rule.name()), None, Duration::from_secs(10));
            assert!(
                matches!(outcome, WorkerOutcome::Success),
                "mutation worker failed for {}: {outcome:?}",
                rule.name()
            );
        }
    }

    #[test]
    fn mutation_worker_distinguishes_panic_abort_and_timeout() {
        let panic = mutation_worker(None, Some("panic"), Duration::from_secs(2));
        assert!(
            matches!(panic, WorkerOutcome::CaughtPanic(message) if message.contains("mutation panic:")
                && message.contains("seed=") && message.contains("input="))
        );
        let abort = mutation_worker(None, Some("abort"), Duration::from_secs(2));
        assert!(
            matches!(abort, WorkerOutcome::Abort(reproduction) if reproduction.contains("seed=") && reproduction.contains("input="))
        );
        let timeout = mutation_worker(None, Some("timeout"), Duration::from_millis(100));
        assert!(
            matches!(timeout, WorkerOutcome::Timeout(reproduction) if reproduction.contains("seed=") && reproduction.contains("input="))
        );
    }

    #[test]
    fn test_rule_engine_with_no_rules() {
        let engine = RuleEngine::new(vec![]);
        let dir = make_temp_project(&[("main.rs", "fn main() {}")]);
        let diags = engine.scan_with_config(dir.path(), &[], &[], &[]);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_is_heuristic_rule() {
        // Every syn-based custom rule is heuristic.
        for rule in all_custom_rules() {
            assert!(
                is_heuristic_rule(rule.name()),
                "{} should be heuristic",
                rule.name()
            );
        }
        // Clippy lints and external-tool findings are not.
        assert!(!is_heuristic_rule("clippy::unwrap_used"));
        assert!(!is_heuristic_rule("unused-dependency"));
        assert!(!is_heuristic_rule("RUSTSEC-2021-0001"));
        assert!(!is_heuristic_rule("nonexistent-rule"));
    }

    #[test]
    fn test_rule_engine_no_src_dir() {
        let dir = tempfile::tempdir().unwrap();
        let engine = RuleEngine::new(vec![Box::new(AlwaysWarnsRule)]);
        let diags = engine.scan_with_config(dir.path(), &[], &[], &[]);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_rule_engine_runs_rules_on_files() {
        let dir = make_temp_project(&[("main.rs", "fn main() {}")]);
        let engine = RuleEngine::new(vec![Box::new(AlwaysWarnsRule)]);
        let diags = engine.scan_with_config(dir.path(), &[], &[], &[]);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "always-warns");
    }

    #[test]
    fn test_rule_engine_multiple_files() {
        let dir = make_temp_project(&[
            ("main.rs", "fn main() {}"),
            ("lib.rs", "pub fn hello() {}"),
            ("utils.rs", "pub fn util() {}"),
        ]);
        let engine = RuleEngine::new(vec![Box::new(AlwaysWarnsRule)]);
        let diags = engine.scan_with_config(dir.path(), &[], &[], &[]);
        assert_eq!(diags.len(), 3);
    }

    #[test]
    fn rule_engine_reports_real_file_progress() {
        let dir = make_temp_project(&[
            ("main.rs", "fn main() {}"),
            ("lib.rs", "pub fn hello() {}"),
            ("utils.rs", "pub fn util() {}"),
        ]);
        let pass =
            RuleEnginePass::with_config(vec![Box::new(AlwaysWarnsRule)], vec![], vec![], vec![]);
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_events = std::sync::Arc::clone(&events);
        let on_file_progress: scanner::FileProgressCallback =
            std::sync::Arc::new(move |progress| {
                captured_events.lock().unwrap().push((
                    progress.started,
                    progress.scanned,
                    progress.total,
                    progress.workers,
                ));
            });

        let diagnostics = pass
            .run_with_progress(dir.path(), &on_file_progress)
            .unwrap();
        let events = events.lock().unwrap();

        assert_eq!(diagnostics.len(), 3);
        assert!(
            events
                .first()
                .is_some_and(|event| { event.0 > 0 && event.1 == 0 && event.2 == 3 })
        );
        assert!(
            events
                .iter()
                .any(|event| (event.0, event.1, event.2) == (3, 3, 3))
        );
        assert!(events.iter().all(|event| event.0 >= event.1));
        assert!(events.iter().all(|event| event.3 > 0));
        drop(events);
    }

    #[test]
    fn full_rule_engine_scans_every_rust_source_surface() {
        let dir = tempfile::tempdir().unwrap();
        for path in [
            "src/lib.rs",
            "tests/integration.rs",
            "benches/health.rs",
            "examples/demo.rs",
            "build.rs",
            "custom/entry.rs",
            "generated/output.rs",
        ] {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "fn surface() {}\n").unwrap();
        }
        let engine = RuleEngine::new(vec![Box::new(AlwaysWarnsRule)]);
        let diagnostics = engine.scan_with_config(dir.path(), &[], &[], &[]);
        let paths: HashSet<_> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.file_path.as_path())
            .collect();
        assert_eq!(paths.len(), 7);
        assert!(paths.contains(Path::new("generated/output.rs")));
        assert!(paths.contains(Path::new("tests/integration.rs")));
        assert!(paths.contains(Path::new("build.rs")));
    }

    #[test]
    fn test_rule_engine_catches_panics() {
        let dir = make_temp_project(&[("main.rs", "fn main() {}")]);
        let engine = RuleEngine::new(vec![Box::new(PanickingRule), Box::new(AlwaysWarnsRule)]);
        let diags = engine.scan_with_config(dir.path(), &[], &[], &[]);
        assert_eq!(diags.len(), 2);
        assert!(
            diags
                .iter()
                .any(|diagnostic| diagnostic.rule == "always-warns")
        );
        assert!(
            diags
                .iter()
                .any(|diagnostic| diagnostic.rule == scanner::ANALYSIS_FAILURE_RULE)
        );
    }

    #[test]
    fn cargo_target_context_reaches_rules_before_execution() {
        struct LibraryOnlyRule;
        impl CustomRule for LibraryOnlyRule {
            fn name(&self) -> &'static str {
                "library-only"
            }
            fn category(&self) -> Category {
                Category::Correctness
            }
            fn severity(&self) -> Severity {
                Severity::Warning
            }
            fn description(&self) -> &'static str {
                "library only"
            }
            fn fix_hint(&self) -> &'static str {
                "none"
            }
            fn check_file(&self, _: &syn::File, _: &Path) -> Vec<Diagnostic> {
                Vec::new()
            }
            fn check_file_with_context(
                &self,
                _: &syn::File,
                path: &Path,
                context: RuleContext,
            ) -> Vec<Diagnostic> {
                (context.source_surface == SourceSurface::Library)
                    .then(|| self.diagnostic(path, "library".to_string(), None, None, None))
                    .into_iter()
                    .collect()
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("custom/api.rs");
        let binary = dir.path().join("custom/daemon.rs");
        std::fs::create_dir_all(library.parent().unwrap()).unwrap();
        std::fs::write(&library, "pub fn api() {}\n").unwrap();
        std::fs::write(&binary, "fn main() {}\n").unwrap();
        let pass =
            RuleEnginePass::with_config(vec![Box::new(LibraryOnlyRule)], vec![], vec![], vec![])
                .with_cargo_targets(vec![
                    CargoTargetContext {
                        name: "api".to_string(),
                        src_path: library,
                        source_surface: SourceSurface::Library,
                        is_proc_macro: false,
                    },
                    CargoTargetContext {
                        name: "daemon".to_string(),
                        src_path: binary,
                        source_surface: SourceSurface::Binary,
                        is_proc_macro: false,
                    },
                ]);

        let diagnostics = pass.run(dir.path()).unwrap();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "library");
    }

    // --- US-006: context, exclusion, and abstention ---

    /// A rule that declares a context contract and records every context it was
    /// actually handed, so the tests can assert on member-specific evidence.
    struct ContextProbeRule {
        seen: std::sync::Mutex<Vec<(String, String, bool)>>,
        required: &'static [ContextRequirement],
    }

    impl ContextProbeRule {
        fn new(required: &'static [ContextRequirement]) -> Self {
            Self {
                seen: std::sync::Mutex::new(Vec::new()),
                required,
            }
        }
    }

    impl CustomRule for ContextProbeRule {
        fn name(&self) -> &'static str {
            // Borrow a real catalog identity so the gate resolves a real
            // supported-context contract (library and binary only).
            "panic-in-library"
        }
        fn category(&self) -> Category {
            Category::ErrorHandling
        }
        fn severity(&self) -> Severity {
            Severity::Warning
        }
        fn description(&self) -> &'static str {
            "context probe"
        }
        fn fix_hint(&self) -> &'static str {
            "none"
        }
        fn required_context(&self) -> &'static [ContextRequirement] {
            self.required
        }
        fn check_file(&self, _: &syn::File, _: &Path) -> Vec<Diagnostic> {
            Vec::new()
        }
        fn check_file_with_context(
            &self,
            _: &syn::File,
            path: &Path,
            context: RuleContext<'_>,
        ) -> Vec<Diagnostic> {
            if let Ok(mut seen) = self.seen.lock() {
                seen.push((
                    context.package.package_id.clone(),
                    context.package.crate_role.as_str().to_string(),
                    context.package.metadata_complete,
                ));
            }
            vec![self.diagnostic(path, "probed".to_string(), None, None, None)]
        }
    }

    fn probe_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for path in ["src/lib.rs", "tests/integration.rs", "benches/health.rs"] {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "fn surface() {}\n").unwrap();
        }
        dir
    }

    fn probe_targets(root: &Path) -> Vec<CargoTargetContext> {
        vec![
            CargoTargetContext {
                name: "probe".to_string(),
                src_path: root.join("src/lib.rs"),
                source_surface: SourceSurface::Library,
                is_proc_macro: false,
            },
            CargoTargetContext {
                name: "integration".to_string(),
                src_path: root.join("tests/integration.rs"),
                source_surface: SourceSurface::Test,
                is_proc_macro: false,
            },
            CargoTargetContext {
                name: "health".to_string(),
                src_path: root.join("benches/health.rs"),
                source_surface: SourceSurface::Bench,
                is_proc_macro: false,
            },
        ]
    }

    fn resolved_package() -> PackageContext {
        PackageContext::new(
            PackageIdentity {
                package_id: "probe 0.1.0".to_string(),
                edition: Some("2024".to_string()),
                declared_msrv: Some("1.97".to_string()),
                enabled_features: vec!["default".to_string()],
                frameworks: vec!["tokio".to_string()],
                dependency_capabilities: vec!["tokio@1.40.0".to_string()],
                cfg_profile: Some("x86_64-unknown-linux-gnu".to_string()),
            },
            &[CargoTargetContext {
                name: "probe".to_string(),
                src_path: PathBuf::from("src/lib.rs"),
                source_surface: SourceSurface::Library,
                is_proc_macro: false,
            }],
        )
    }

    #[test]
    fn unsupported_source_surfaces_are_excluded_before_traversal() {
        let dir = probe_project();
        let targets = probe_targets(dir.path());
        let pass = RuleEnginePass::with_config(
            vec![Box::new(ContextProbeRule::new(&[]))],
            vec![],
            vec![],
            vec![],
        )
        .with_cargo_targets(targets)
        .with_package_context(resolved_package());

        let diagnostics = pass.run(dir.path()).unwrap();
        // Only the library file survives: the catalog contract for
        // `panic-in-library` lists library, never test or bench.
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].file_path.ends_with("lib.rs"));

        let receipts = pass.take_abstentions();
        let reasons: HashSet<_> = receipts
            .iter()
            .map(|receipt| receipt.reason.as_str())
            .collect();
        assert!(reasons.contains("unsupported-source-surface:test"));
        assert!(reasons.contains("unsupported-source-surface:bench"));
        assert!(
            receipts
                .iter()
                .all(|receipt| receipt.rule == "panic-in-library")
        );
    }

    #[test]
    fn unknown_and_test_fixtures_abstain_before_file_parsing() {
        let dir = probe_project();
        let unknown = dir.path().join("fixtures/broken.rs");
        let test = dir.path().join("tests/integration.rs");
        std::fs::create_dir_all(unknown.parent().unwrap()).unwrap();
        std::fs::write(&unknown, b"\xff\xfe not UTF-8").unwrap();
        std::fs::write(&test, "this is deliberately invalid Rust {{{").unwrap();
        let pass = RuleEnginePass::with_config(
            vec![Box::new(ContextProbeRule::new(&[]))],
            vec![],
            vec![],
            vec![],
        )
        .with_cargo_targets(probe_targets(dir.path()))
        .with_package_context(resolved_package())
        .skip_fully_abstained_files(true);

        let diagnostics = pass.run(dir.path()).unwrap();
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].file_path.ends_with("lib.rs"));
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule != scanner::ANALYSIS_FAILURE_RULE)
        );

        let receipts = pass.take_abstentions();
        assert!(receipts.iter().any(|receipt| {
            receipt.reason == "missing-context:source-surface" && receipt.count == 1
        }));
        assert!(receipts.iter().any(|receipt| {
            receipt.reason == "unsupported-source-surface:test" && receipt.count == 1
        }));
    }

    #[test]
    fn missing_required_context_abstains_instead_of_emitting() {
        let dir = make_temp_project(&[("lib.rs", "fn thing() {}")]);
        let pass = RuleEnginePass::with_config(
            vec![Box::new(ContextProbeRule::new(&[
                ContextRequirement::PackageMetadata,
                ContextRequirement::DeclaredMsrv,
            ]))],
            vec![],
            vec![],
            vec![],
        );

        let diagnostics = pass.run(dir.path()).unwrap();
        assert!(
            diagnostics.is_empty(),
            "a rule without its required context must not emit"
        );
        let receipts = pass.take_abstentions();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].reason, "missing-context:package-metadata");
        assert_eq!(receipts[0].count, 1);
    }

    #[test]
    fn each_workspace_member_receives_its_own_context() {
        let first = make_temp_project(&[("lib.rs", "fn a() {}")]);
        let second = make_temp_project(&[("lib.rs", "fn b() {}")]);

        let run = |dir: &tempfile::TempDir, package_id: &str, features: Vec<String>| {
            let probe = std::sync::Arc::new(ContextProbeRule::new(&[]));
            struct Shared(std::sync::Arc<ContextProbeRule>);
            impl CustomRule for Shared {
                fn name(&self) -> &'static str {
                    self.0.name()
                }
                fn category(&self) -> Category {
                    self.0.category()
                }
                fn severity(&self) -> Severity {
                    self.0.severity()
                }
                fn description(&self) -> &'static str {
                    self.0.description()
                }
                fn fix_hint(&self) -> &'static str {
                    self.0.fix_hint()
                }
                fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
                    self.0.check_file(syntax, path)
                }
                fn check_file_with_context(
                    &self,
                    syntax: &syn::File,
                    path: &Path,
                    context: RuleContext<'_>,
                ) -> Vec<Diagnostic> {
                    self.0.check_file_with_context(syntax, path, context)
                }
            }
            let pass = RuleEnginePass::with_config(
                vec![Box::new(Shared(std::sync::Arc::clone(&probe)))],
                vec![],
                vec![],
                vec![],
            )
            .with_package_context(PackageContext::new(
                PackageIdentity {
                    package_id: package_id.to_string(),
                    edition: Some("2024".to_string()),
                    declared_msrv: Some("1.97".to_string()),
                    enabled_features: features,
                    frameworks: Vec::new(),
                    dependency_capabilities: Vec::new(),
                    cfg_profile: Some("x86_64-unknown-linux-gnu".to_string()),
                },
                &[CargoTargetContext {
                    name: "member".to_string(),
                    src_path: dir.path().join("src/lib.rs"),
                    source_surface: SourceSurface::Library,
                    is_proc_macro: false,
                }],
            ));
            let _ = pass.run(dir.path()).unwrap();
            probe.seen.lock().unwrap().clone()
        };

        let alpha = run(&first, "alpha 0.1.0", vec!["serde".to_string()]);
        let beta = run(&second, "beta 0.2.0", vec!["tokio".to_string()]);
        assert_eq!(alpha.len(), 1);
        assert_eq!(beta.len(), 1);
        assert_eq!(alpha[0].0, "alpha 0.1.0");
        assert_eq!(beta[0].0, "beta 0.2.0");
        assert!(alpha[0].2 && beta[0].2);
    }

    #[test]
    fn a_panicking_rule_stays_isolated_with_the_expanded_context() {
        let dir = make_temp_project(&[("main.rs", "fn main() {}")]);
        let pass = RuleEnginePass::with_config(
            vec![Box::new(PanickingRule), Box::new(AlwaysWarnsRule)],
            vec![],
            vec![],
            vec![],
        )
        .with_package_context(resolved_package());

        let diagnostics = pass.run(dir.path()).unwrap();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == "always-warns")
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == scanner::ANALYSIS_FAILURE_RULE)
        );
    }

    #[test]
    fn test_rule_engine_handles_parse_errors() {
        let dir = make_temp_project(&[("main.rs", "this is not valid rust {{{{")]);
        let engine = RuleEngine::new(vec![Box::new(AlwaysWarnsRule)]);
        let diags = engine.scan_with_config(dir.path(), &[], &[], &[]);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, scanner::ANALYSIS_FAILURE_RULE);
        assert!(diags[0].message.starts_with("parse_failed:"));
    }

    #[test]
    fn test_rule_engine_skips_ignored_files() {
        let dir = make_temp_project(&[
            ("main.rs", "fn main() {}"),
            ("generated/output.rs", "pub fn gen() {}"),
        ]);
        let engine = RuleEngine::new(vec![Box::new(AlwaysWarnsRule)]);
        let ignore = vec!["src/generated/**".to_string()];
        let diags = engine.scan_with_config(dir.path(), &ignore, &[], &[]);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].file_path.to_string_lossy().contains("main.rs"));
    }

    #[test]
    fn test_rule_engine_on_self() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let engine = RuleEngine::new(vec![Box::new(CountFnRule)]);
        let _diags = engine.scan_with_config(manifest_dir, &[], &[], &[]);
        // CountFnRule only fires if a file has >10 functions, so may or may not find issues
    }

    #[test]
    fn test_collect_rs_files() {
        let dir = make_temp_project(&[("main.rs", ""), ("lib.rs", ""), ("sub/mod.rs", "")]);
        let files = scanner::collect_rs_files(&dir.path().join("src"));
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn test_rule_engine_pass() {
        let dir = make_temp_project(&[("main.rs", "fn main() {}")]);
        let pass =
            RuleEnginePass::with_config(vec![Box::new(AlwaysWarnsRule)], vec![], vec![], vec![]);
        let result = pass.run(dir.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 1);
    }
}
