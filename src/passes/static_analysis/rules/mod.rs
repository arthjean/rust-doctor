pub mod async_rules;
pub mod complexity;
pub mod error_handling;
pub mod framework;
pub mod performance;
pub mod security;

use crate::cache::{self, ScanCache};
use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::scanner::{self, AnalysisPass};
use globset::GlobSet;
use rayon::prelude::*;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

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
    pub fn scan_with_config(
        &self,
        project_root: &Path,
        ignore_files: &[String],
        ignore_rules: &[String],
        enable_rules: &[String],
    ) -> Vec<Diagnostic> {
        if self.rules.is_empty() {
            return vec![];
        }

        // Collect .rs files
        let src_dir = project_root.join("src");
        if !src_dir.is_dir() {
            return vec![];
        }

        let files = scanner::collect_rs_files(&src_dir);
        if files.is_empty() {
            return vec![];
        }

        // Build ignore glob set
        let ignore_set = build_ignore_set(ignore_files);

        // Compute config hash including active rule names so cache
        // invalidates when rules are added or removed.
        let active_rule_names: Vec<&str> = self.rules.iter().map(|r| r.name()).collect();
        let config_hash = cache::compute_config_hash(
            ignore_rules,
            ignore_files,
            enable_rules,
            &active_rule_names,
        );
        let mut scan_cache = ScanCache::load(project_root, &config_hash)
            .unwrap_or_else(|| ScanCache::new(config_hash.clone()));

        // Read all files into memory, filtering ignored paths
        let file_contents: Vec<(std::path::PathBuf, String)> = files
            .into_iter()
            .filter_map(|file_path| {
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
            .map(|&(file_path, content, ref hash)| {
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

                (rel_path.to_path_buf(), hash.clone(), diagnostics)
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
        }
    }
}

impl AnalysisPass for RuleEnginePass {
    fn name(&self) -> &'static str {
        "custom rules"
    }

    fn produces_category(&self, filter: crate::cli::CategoryFilter) -> bool {
        self.engine
            .rules
            .iter()
            .any(|rule| filter.matches(&rule.category()))
    }

    fn run(&self, project_root: &Path) -> Result<Vec<Diagnostic>, crate::error::PassError> {
        Ok(self.engine.scan_with_config(
            project_root,
            &self.ignore_files,
            &self.ignore_rules,
            &self.enable_rules,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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

    #[test]
    fn test_rule_engine_with_no_rules() {
        let engine = RuleEngine::new(vec![]);
        let dir = make_temp_project(&[("main.rs", "fn main() {}")]);
        let diags = engine.scan_with_config(dir.path(), &[], &[], &[]);
        assert!(diags.is_empty());
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
