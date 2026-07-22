pub mod async_rules;
pub mod complexity;
pub mod error_handling;
pub mod framework;
pub mod performance;
pub mod security;

use crate::cache::{self, ScanCache};
use crate::catalog::{Confidence, NumericRange};
use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::scanner::{self, AnalysisPass};
use globset::GlobSet;
use rayon::prelude::*;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

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
#[expect(
    dead_code,
    reason = "helper methods used by implementors in sub-modules"
)]
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

    /// Accepted inclusive range for a configurable numeric threshold.
    fn supported_threshold(&self) -> Option<NumericRange> {
        None
    }

    /// Apply a validated threshold before analysis starts.
    fn set_threshold(&mut self, _threshold: u32) {}

    /// Check a parsed Rust file and return diagnostics.
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic>;

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

/// The rule engine: runs custom rules against all `.rs` files in parallel.
pub struct RuleEngine {
    rules: Vec<Box<dyn CustomRule>>,
}

impl RuleEngine {
    /// Create a new rule engine with the given rules.
    pub fn new(rules: Vec<Box<dyn CustomRule>>) -> Self {
        Self { rules }
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
        self.scan_selected_with_config(project_root, None, ignore_files, ignore_rules, enable_rules)
    }

    /// Scan only the supplied files when scope resolution narrowed local AST
    /// work. Compiler-aware passes are narrowed independently at package level.
    #[expect(
        clippy::too_many_lines,
        reason = "file selection, cache partitioning, panic isolation, and parallel rule execution form one rule-engine transaction"
    )]
    pub fn scan_selected_with_config(
        &self,
        project_root: &Path,
        selected_files: Option<&[PathBuf]>,
        ignore_files: &[String],
        ignore_rules: &[String],
        enable_rules: &[String],
    ) -> Vec<Diagnostic> {
        if self.rules.is_empty() {
            return vec![];
        }

        let mut files = selected_files.map_or_else(
            || {
                let src_dir = project_root.join("src");
                if src_dir.is_dir() {
                    scanner::collect_rs_files(&src_dir)
                } else {
                    Vec::new()
                }
            },
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

        // Read all files into memory, filtering ignored paths
        let control = crate::process::current_scan_control();
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
                match std::fs::read_to_string(&file_path) {
                    Ok(content) => Some((file_path, content)),
                    Err(e) => {
                        eprintln!("Warning: could not read '{}': {e}", file_path.display());
                        None
                    }
                }
            })
            .collect();

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
        let mut all_diagnostics: Vec<Diagnostic> = fresh_files
            .iter()
            .flat_map(|(file_path, _content)| {
                let rel_path = file_path.strip_prefix(project_root).unwrap_or(file_path);
                scan_cache
                    .get_cached_diagnostics(rel_path)
                    .unwrap_or(&[])
                    .to_vec()
            })
            .collect();

        // Process stale files in parallel with rayon
        let stale_results: Vec<(std::path::PathBuf, String, Vec<Diagnostic>)> = stale_files
            .par_iter()
            .filter_map(|&(file_path, content, ref hash)| {
                if control.is_stopped() {
                    return None;
                }
                let rel_path = file_path.strip_prefix(project_root).unwrap_or(file_path);

                let diagnostics = match syn::parse_file(content) {
                    Ok(syntax) => self
                        .rules
                        .iter()
                        .flat_map(|rule| run_rule_safely(rule.as_ref(), &syntax, rel_path))
                        .collect::<Vec<_>>(),
                    Err(e) => {
                        eprintln!("Warning: parse error in '{}': {e}", rel_path.display());
                        vec![]
                    }
                };

                Some((rel_path.to_path_buf(), hash.clone(), diagnostics))
            })
            .collect();

        // Update the cache with newly scanned results using pre-computed hashes
        for (rel_path, hash, diagnostics) in stale_results {
            all_diagnostics.extend_from_slice(&diagnostics);
            scan_cache.update_with_hash(&rel_path, hash, diagnostics);
        }

        // Persist the updated cache (best-effort)
        scan_cache.save(project_root);

        all_diagnostics
    }
}

/// Run a single rule with panic isolation.
fn run_rule_safely(rule: &dyn CustomRule, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| rule.check_file(syntax, path)));

    match result {
        Ok(diagnostics) => diagnostics,
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
            vec![]
        }
    }
}

/// Return all custom rules across all categories.
/// Used to derive the rule registry and documentation at startup.
pub fn all_custom_rules() -> Vec<Box<dyn CustomRule>> {
    error_handling::all_rules()
        .into_iter()
        .chain(performance::all_rules())
        .chain(complexity::all_rules())
        .chain(security::all_rules())
        .chain(async_rules::all_rules())
        .chain(framework::all_rules())
        .collect()
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
        }
    }

    pub fn with_selected_files(mut self, selected_files: Vec<PathBuf>) -> Self {
        self.selected_files = Some(selected_files);
        self
    }
}

impl AnalysisPass for RuleEnginePass {
    fn name(&self) -> &'static str {
        "custom rules"
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        Ok(self.engine.scan_selected_with_config(
            project_root,
            self.selected_files.as_deref(),
            &self.ignore_files,
            &self.ignore_rules,
            &self.enable_rules,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::io::Write;
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
    fn seeded_mutations_never_escape_rule_panic_isolation() {
        const MUTATIONS_PER_RULE: usize = 1_000;
        let started = Instant::now();
        let budget = std::env::var("RUST_DOCTOR_MUTATION_BUDGET_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map_or(Duration::from_secs(30), Duration::from_secs);
        let fixtures: HashMap<_, _> = conformance_manifest()
            .into_iter()
            .map(|fixture| (fixture.id.clone(), fixture))
            .collect();

        for rule in all_custom_rules() {
            let fixture = fixtures.get(rule.name()).unwrap();
            for iteration in 0..MUTATIONS_PER_RULE {
                let source = &fixture.cases[iteration % fixture.cases.len()].source;
                let seed = stable_seed(rule.name(), iteration as u64);
                let input = adversarial_input(iteration, source, seed);
                if rule_panics(rule.as_ref(), &input) {
                    let minimized = minimize_panicking_input(rule.as_ref(), input);
                    panic!(
                        "mutation panic: rule={} seed={seed:#018x} minimized_input={:?}",
                        rule.name(),
                        String::from_utf8_lossy(&minimized)
                    );
                }
            }
        }
        assert!(
            started.elapsed() <= budget,
            "mutation harness exceeded budget of {} seconds",
            budget.as_secs()
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
    fn test_rule_engine_catches_panics() {
        let dir = make_temp_project(&[("main.rs", "fn main() {}")]);
        let engine = RuleEngine::new(vec![Box::new(PanickingRule), Box::new(AlwaysWarnsRule)]);
        let diags = engine.scan_with_config(dir.path(), &[], &[], &[]);
        // PanickingRule panicked and was caught; AlwaysWarnsRule still ran
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "always-warns");
    }

    #[test]
    fn test_rule_engine_handles_parse_errors() {
        let dir = make_temp_project(&[("main.rs", "this is not valid rust {{{{")]);
        let engine = RuleEngine::new(vec![Box::new(AlwaysWarnsRule)]);
        let diags = engine.scan_with_config(dir.path(), &[], &[], &[]);
        // File couldn't be parsed, so no diagnostics
        assert!(diags.is_empty());
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
