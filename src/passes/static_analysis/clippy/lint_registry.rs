#![expect(
    clippy::redundant_pub_crate,
    reason = "registry metadata is intentionally visible to the sibling catalog module"
)]

use crate::diagnostics::{Category, Severity};

// ---------------------------------------------------------------------------
// Lint registry — data-driven mapping of clippy lints to categories/severities
// ---------------------------------------------------------------------------

/// A single entry in the lint-to-category mapping table.
pub(crate) struct LintEntry {
    /// Lint name without the `clippy::` prefix.
    pub(crate) name: &'static str,
    pub(crate) category: Category,
    /// Severity override — takes precedence over clippy's default.
    pub(crate) severity: Severity,
    /// Whether this lint belongs to clippy's `restriction` group (allow-by-default).
    /// Restriction lints are downgraded to Info in test code because they are opt-in
    /// style checks, not correctness issues.
    pub(crate) is_restriction: bool,
}

/// Registry of 75+ impactful clippy lints with explicit category and severity.
/// Lints NOT in this table inherit clippy's default severity and map to `Style`.
pub(crate) static LINT_REGISTRY: &[LintEntry] = &[
    // ── Error Handling (restriction group — allow-by-default in clippy) ─
    LintEntry {
        name: "unwrap_used",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "expect_used",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "panic",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "indexing_slicing",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "unwrap_in_result",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "panic_in_result_fn",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "exit",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "map_unwrap_or",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "option_if_let_else",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "question_mark",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "manual_ok_or",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "result_unit_err",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "result_large_err",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "let_underscore_must_use",
        category: Category::ErrorHandling,
        severity: Severity::Warning,
        is_restriction: false,
    },
    // ── Performance ─────────────────────────────────────────────────────
    LintEntry {
        name: "box_collection",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "clone_on_copy",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "redundant_clone",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "needless_collect",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "large_enum_variant",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "inefficient_to_string",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "unnecessary_to_owned",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "large_stack_arrays",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "large_futures",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "single_char_pattern",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "cmp_owned",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "cloned_instead_of_copied",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "suboptimal_flops",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "or_fun_call",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "trivially_copy_pass_by_ref",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "useless_vec",
        category: Category::Performance,
        severity: Severity::Warning,
        is_restriction: false,
    },
    // ── Security ────────────────────────────────────────────────────────
    LintEntry {
        name: "undocumented_unsafe_blocks",
        category: Category::Security,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "multiple_unsafe_ops_per_block",
        category: Category::Security,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "transmute_ptr_to_ref",
        category: Category::Security,
        severity: Severity::Error,
        is_restriction: false,
    },
    LintEntry {
        name: "cast_ptr_alignment",
        category: Category::Security,
        severity: Severity::Error,
        is_restriction: false,
    },
    LintEntry {
        name: "fn_to_numeric_cast",
        category: Category::Security,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "mem_forget",
        category: Category::Security,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "cast_possible_truncation",
        category: Category::Security,
        severity: Severity::Warning,
        is_restriction: false,
    },
    // ── Correctness ─────────────────────────────────────────────────────
    LintEntry {
        name: "almost_swapped",
        category: Category::Correctness,
        severity: Severity::Error,
        is_restriction: false,
    },
    LintEntry {
        name: "approx_constant",
        category: Category::Correctness,
        severity: Severity::Error,
        is_restriction: false,
    },
    LintEntry {
        name: "bad_bit_mask",
        category: Category::Correctness,
        severity: Severity::Error,
        is_restriction: false,
    },
    LintEntry {
        name: "absurd_extreme_comparisons",
        category: Category::Correctness,
        severity: Severity::Error,
        is_restriction: false,
    },
    LintEntry {
        name: "invalid_regex",
        category: Category::Correctness,
        severity: Severity::Error,
        is_restriction: false,
    },
    LintEntry {
        name: "wrong_self_convention",
        category: Category::Correctness,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "cast_sign_loss",
        category: Category::Correctness,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "cast_possible_wrap",
        category: Category::Correctness,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "cast_lossless",
        category: Category::Correctness,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "float_cmp",
        category: Category::Correctness,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "eq_op",
        category: Category::Correctness,
        severity: Severity::Error,
        is_restriction: false,
    },
    LintEntry {
        name: "match_overlapping_arm",
        category: Category::Correctness,
        severity: Severity::Warning,
        is_restriction: false,
    },
    // ── Cargo ───────────────────────────────────────────────────────────
    LintEntry {
        name: "wildcard_dependencies",
        category: Category::Cargo,
        severity: Severity::Error,
        is_restriction: false,
    },
    LintEntry {
        name: "multiple_crate_versions",
        category: Category::Cargo,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "cargo_common_metadata",
        category: Category::Cargo,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "negative_feature_names",
        category: Category::Cargo,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "redundant_feature_names",
        category: Category::Cargo,
        severity: Severity::Warning,
        is_restriction: false,
    },
    // ── Async ───────────────────────────────────────────────────────────
    LintEntry {
        name: "await_holding_lock",
        category: Category::Async,
        severity: Severity::Error,
        is_restriction: false,
    },
    LintEntry {
        name: "await_holding_refcell_ref",
        category: Category::Async,
        severity: Severity::Error,
        is_restriction: false,
    },
    LintEntry {
        name: "unused_async",
        category: Category::Async,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "redundant_async_block",
        category: Category::Async,
        severity: Severity::Warning,
        is_restriction: false,
    },
    // ── Architecture ────────────────────────────────────────────────────
    LintEntry {
        name: "struct_excessive_bools",
        category: Category::Architecture,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "fn_params_excessive_bools",
        category: Category::Architecture,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "too_many_lines",
        category: Category::Architecture,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "cognitive_complexity",
        category: Category::Architecture,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "type_complexity",
        category: Category::Architecture,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "too_many_arguments",
        category: Category::Architecture,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "module_name_repetitions",
        category: Category::Architecture,
        severity: Severity::Warning,
        is_restriction: false,
    },
    // ── Style (restriction-group lints) ─────────────────────────────────
    LintEntry {
        name: "dbg_macro",
        category: Category::Style,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "todo",
        category: Category::Style,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "unimplemented",
        category: Category::Style,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "unreachable",
        category: Category::Style,
        severity: Severity::Warning,
        is_restriction: true,
    },
    // ── Style (non-restriction) ─────────────────────────────────────────
    LintEntry {
        name: "wildcard_imports",
        category: Category::Style,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "missing_errors_doc",
        category: Category::Style,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "missing_panics_doc",
        category: Category::Style,
        severity: Severity::Warning,
        is_restriction: false,
    },
    LintEntry {
        name: "print_stdout",
        category: Category::Style,
        severity: Severity::Warning,
        is_restriction: true,
    },
    LintEntry {
        name: "print_stderr",
        category: Category::Style,
        severity: Severity::Warning,
        is_restriction: true,
    },
];

/// Look up a lint in the registry. Returns `(category, severity, is_restriction)` if found.
pub(super) fn lookup_lint(lint: &str) -> Option<(Category, Severity, bool)> {
    let name = lint.strip_prefix("clippy::").unwrap_or(lint);
    LINT_REGISTRY
        .iter()
        .find(|e| e.name == name)
        .map(|e| (e.category.clone(), e.severity, e.is_restriction))
}

/// Map a clippy lint name to a rust-doctor category. Falls back to `Style`.
pub(super) fn map_lint_category(lint: &str) -> Category {
    match lint {
        "compiler-error" | "compiler-ice" => Category::Correctness,
        _ => lookup_lint(lint).map_or(Category::Style, |(cat, _, _)| cat),
    }
}

/// Apply severity override from the registry if the lint is known.
/// Otherwise, keep clippy's original severity.
pub(super) fn resolve_severity(lint: &str, clippy_severity: Severity) -> Severity {
    match lint {
        "compiler-error" | "compiler-ice" => Severity::Error,
        _ => lookup_lint(lint).map_or(clippy_severity, |(_, sev, _)| sev),
    }
}

/// Returns `true` if the lint is in clippy's `restriction` group (allow-by-default).
pub(super) fn is_restriction_lint(lint: &str) -> bool {
    lookup_lint(lint).is_some_and(|(_, _, restriction)| restriction)
}

/// Return the list of all known lint names (for config validation).
#[cfg(test)]
pub fn known_lint_names() -> Vec<&'static str> {
    LINT_REGISTRY.iter().map(|e| e.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_known_lint_returns_correct_category() {
        let result = lookup_lint("unwrap_used");
        assert!(result.is_some());
        let (cat, sev, restriction) = result.unwrap();
        assert!(matches!(cat, Category::ErrorHandling));
        assert!(matches!(sev, Severity::Warning));
        assert!(restriction);
    }

    #[test]
    fn lookup_with_clippy_prefix_strips_it() {
        let result = lookup_lint("clippy::clone_on_copy");
        assert!(result.is_some());
        let (cat, _, _) = result.unwrap();
        assert!(matches!(cat, Category::Performance));
    }

    #[test]
    fn lookup_unknown_lint_returns_none() {
        assert!(lookup_lint("totally_fake_lint").is_none());
    }

    #[test]
    fn map_lint_category_known_lint() {
        assert!(matches!(
            map_lint_category("transmute_ptr_to_ref"),
            Category::Security
        ));
    }

    #[test]
    fn map_lint_category_unknown_falls_back_to_style() {
        assert!(matches!(
            map_lint_category("some_unknown_lint"),
            Category::Style
        ));
    }

    #[test]
    fn map_lint_category_compiler_error() {
        assert!(matches!(
            map_lint_category("compiler-error"),
            Category::Correctness
        ));
        assert!(matches!(
            map_lint_category("compiler-ice"),
            Category::Correctness
        ));
    }

    #[test]
    fn resolve_severity_known_lint_overrides() {
        // transmute_ptr_to_ref is Error in registry
        let sev = resolve_severity("transmute_ptr_to_ref", Severity::Warning);
        assert!(matches!(sev, Severity::Error));
    }

    #[test]
    fn resolve_severity_unknown_lint_keeps_original() {
        let sev = resolve_severity("unknown_lint", Severity::Info);
        assert!(matches!(sev, Severity::Info));
    }

    #[test]
    fn resolve_severity_compiler_error_always_error() {
        let sev = resolve_severity("compiler-error", Severity::Warning);
        assert!(matches!(sev, Severity::Error));
    }

    #[test]
    fn is_restriction_lint_identifies_restriction_lints() {
        assert!(is_restriction_lint("unwrap_used"));
        assert!(is_restriction_lint("dbg_macro"));
        assert!(!is_restriction_lint("clone_on_copy"));
    }

    #[test]
    fn is_restriction_lint_unknown_returns_false() {
        assert!(!is_restriction_lint("totally_fake"));
    }

    #[test]
    fn known_lint_names_returns_all_registry_entries() {
        let names = known_lint_names();
        assert_eq!(names.len(), LINT_REGISTRY.len());
        assert!(names.contains(&"unwrap_used"));
        assert!(names.contains(&"clone_on_copy"));
        assert!(names.contains(&"wildcard_dependencies"));
    }

    #[test]
    fn registry_has_no_duplicate_names() {
        let names = known_lint_names();
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(
            names.len(),
            unique.len(),
            "duplicate lint names in registry"
        );
    }
}
