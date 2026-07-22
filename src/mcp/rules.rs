use crate::catalog::{AnalyzerKind, built_in_catalog};

#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct RuleDoc {
    pub(super) name: &'static str,
    pub(super) category: String,
    pub(super) severity: String,
    pub(super) description: &'static str,
    pub(super) fix: &'static str,
}

/// Compatibility view derived from the canonical catalog, never maintained separately.
pub(super) fn rule_docs() -> &'static [RuleDoc] {
    static DOCS: std::sync::OnceLock<Vec<RuleDoc>> = std::sync::OnceLock::new();
    DOCS.get_or_init(|| {
        let Ok(catalog) = built_in_catalog() else {
            return Vec::new();
        };
        catalog
            .descriptors()
            .iter()
            .filter(|descriptor| descriptor.analyzer_kind == AnalyzerKind::SynAst)
            .map(|descriptor| RuleDoc {
                name: descriptor.canonical_id.as_str(),
                category: descriptor.category.to_string(),
                severity: descriptor.default_severity.to_string(),
                description: descriptor.description.as_str(),
                fix: descriptor.fix_guidance.as_str(),
            })
            .collect()
    })
}

/// Documented structural false-positive caveats for specific heuristic rules.
///
/// These syn-only rules are correct in spirit but, lacking type information,
/// have known blind spots worth surfacing so users calibrate confidence.
pub(super) fn rule_limitation(rule: &str) -> Option<&'static str> {
    match rule {
        "unwrap-in-production" => Some(
            "Matches `.unwrap()`/`.expect()` syntactically; it cannot tell a \
             provably-infallible unwrap from a risky one.",
        ),
        "large-enum-variant" => Some(
            "Counts a variant's fields, not its byte size; a few wide-type fields \
             can outweigh many small ones, and vice versa.",
        ),
        "blocking-in-async" => Some(
            "Flags known blocking calls by name inside async fns; it cannot follow \
             calls into other functions or resolve aliased imports.",
        ),
        "sql-injection-risk" => Some(
            "Flags string-built queries heuristically; it cannot confirm the \
             interpolated value is actually untrusted input.",
        ),
        _ => None,
    }
}

pub(super) fn get_rule_explanation(rule: &str) -> String {
    // Look up in the data-driven registry first
    let Ok(catalog) = built_in_catalog() else {
        return "Rule catalog is unavailable because its invariant validation failed.".to_string();
    };
    if let Some(descriptor) = catalog.exact(rule) {
        let analysis = match descriptor.analyzer_kind {
            AnalyzerKind::SynAst => "Heuristic (syn AST only)",
            AnalyzerKind::Clippy => "Type-aware (Clippy lint)",
            _ => "External analyzer",
        };
        let mut out = format!(
            "## {}\n\n**Provider:** {} | **Category:** {} | **Severity:** {} | **Analysis:** {} | **Confidence:** {:?}\n\n{}\n\n**Fix:** {}\n\nDocumentation: {}",
            descriptor.canonical_id,
            descriptor.provider,
            descriptor.category,
            descriptor.default_severity,
            analysis,
            descriptor.confidence,
            descriptor.description,
            descriptor.fix_guidance,
            descriptor.documentation_url,
        );
        if let Some(limitation) = rule_limitation(rule) {
            use std::fmt::Write;
            let _ = write!(out, "\n\n**Known limitation:** {limitation}");
        }
        return out;
    }

    format!("Unknown rule: `{rule}`\n\nUse the `list_rules` tool to see all available rules.")
}

pub(super) fn get_all_rules_listing() -> String {
    let Ok(catalog) = built_in_catalog() else {
        return "Rule catalog is unavailable because its invariant validation failed.".to_string();
    };
    let mut text = String::from(
        "# rust-doctor Rules\n\nCustom rules are heuristic; Clippy lints are type-aware.\n\n",
    );

    use std::fmt::Write;
    let custom: Vec<_> = catalog
        .descriptors()
        .iter()
        .filter(|descriptor| descriptor.analyzer_kind == AnalyzerKind::SynAst)
        .collect();
    let clippy: Vec<_> = catalog
        .descriptors()
        .iter()
        .filter(|descriptor| descriptor.analyzer_kind == AnalyzerKind::Clippy)
        .collect();
    let external: Vec<_> = catalog
        .descriptors()
        .iter()
        .filter(|descriptor| {
            !matches!(
                descriptor.analyzer_kind,
                AnalyzerKind::SynAst | AnalyzerKind::Clippy
            )
        })
        .collect();

    let _ = writeln!(text, "## Custom Rules ({})\n", custom.len());
    let mut current_category = String::new();
    for descriptor in &custom {
        let category = descriptor.category.to_string();
        if category != current_category {
            let _ = writeln!(text, "### {category}");
            current_category = category;
        }
        let caveat = if rule_limitation(&descriptor.canonical_id).is_some() {
            " (known limitation)"
        } else {
            ""
        };
        let _ = writeln!(
            text,
            "- `{}` ({}, {}){caveat}: {}",
            descriptor.canonical_id,
            descriptor.category,
            descriptor.default_severity,
            descriptor.description
        );
    }

    text.push_str("\n### Known heuristic limitations\n\n");
    for descriptor in &custom {
        if let Some(limitation) = rule_limitation(&descriptor.canonical_id) {
            let _ = writeln!(text, "- `{}`: {limitation}", descriptor.canonical_id);
        }
    }

    let _ = writeln!(text, "\n## Clippy Lints ({})\n", clippy.len());
    for descriptor in clippy {
        let _ = writeln!(
            text,
            "- `{}` ({}, {})",
            descriptor.canonical_id, descriptor.category, descriptor.default_severity
        );
    }

    let _ = writeln!(
        text,
        "\n## External Tools and Project Rules ({})\n",
        external.len()
    );
    for descriptor in external {
        let _ = writeln!(
            text,
            "- `{}` ({})",
            descriptor.canonical_id, descriptor.provider
        );
    }

    text
}
