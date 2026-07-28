//! Remediation plan generator — turns scan diagnostics into a structured,
//! prioritized action plan.
//!
//! The plan groups findings by priority (P0–P3), provides effort estimates,
//! and includes actionable fix descriptions for each item.

use crate::diagnostics::{Diagnostic, ScanResult, Severity};
use std::collections::HashMap;
use std::fmt::Write;

/// A single remediation item in the plan.
#[derive(Debug, Clone)]
pub struct RemediationItem {
    pub priority: Priority,
    pub rule: String,
    pub count: usize,
    pub severity: Severity,
    pub description: String,
    pub fix_action: String,
    pub files: Vec<String>,
}

/// Priority level for remediation items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Critical — must fix (security, correctness bugs)
    P0,
    /// High — should fix (error handling, reliability)
    P1,
    /// Medium — recommended (performance, maintainability)
    P2,
    /// Low — nice to have (style, info-level)
    P3,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::P0 => write!(f, "P0 Critical"),
            Self::P1 => write!(f, "P1 High"),
            Self::P2 => write!(f, "P2 Medium"),
            Self::P3 => write!(f, "P3 Low"),
        }
    }
}

/// Generate a remediation plan from scan results.
pub fn generate_plan(result: &ScanResult) -> Vec<RemediationItem> {
    // Group diagnostics by rule
    let mut by_rule: HashMap<&str, Vec<&Diagnostic>> = HashMap::new();
    for d in &result.diagnostics {
        by_rule.entry(d.rule.as_str()).or_default().push(d);
    }

    let mut items: Vec<RemediationItem> = by_rule
        .into_iter()
        .filter(|(rule, _)| *rule != "skipped-pass") // Skip informational pass notices
        .filter_map(|(rule, diags)| {
            let first = diags.first()?;
            let priority = classify_priority(first.severity, &first.category);
            let file_set: std::collections::HashSet<String> = diags
                .iter()
                .map(|d| d.file_path.to_string_lossy().into_owned())
                .collect();
            let files: Vec<String> = file_set.into_iter().collect();

            Some(RemediationItem {
                priority,
                rule: rule.to_string(),
                count: diags.len(),
                severity: first.severity,
                description: first.message.clone(),
                fix_action: first
                    .help
                    .clone()
                    .unwrap_or_else(|| "Review and fix manually".to_string()),
                files,
            })
        })
        .collect();

    items.sort_by(|a, b| a.priority.cmp(&b.priority).then(b.count.cmp(&a.count)));
    items
}

/// Classify a finding into a priority level.
const fn classify_priority(
    severity: Severity,
    category: &crate::diagnostics::Category,
) -> Priority {
    use crate::diagnostics::Category;

    match severity {
        Severity::Error => Priority::P0,
        Severity::Warning => match category {
            Category::Security => Priority::P0,
            Category::Correctness
            | Category::ErrorHandling
            | Category::Cargo
            | Category::Dependencies
            | Category::Async
            | Category::Framework => Priority::P1,
            Category::Performance | Category::Architecture => Priority::P2,
            Category::Style => Priority::P3,
        },
        Severity::Info => Priority::P3,
    }
}

/// Format the plan as a human-readable markdown string.
pub fn format_plan_markdown(items: &[RemediationItem], result: &ScanResult) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "# Remediation Plan");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "**Score: {}/100 ({})** | {} errors, {} warnings, {} info | {} files scanned",
        result.score,
        result.score_label,
        result.error_count,
        result.warning_count,
        result.info_count,
        result.source_file_count
    );
    let _ = writeln!(
        out,
        "**Dimensions:** Security {}, Reliability {}, Maintainability {}, Performance {}, Dependencies {}",
        result.dimension_scores.security,
        result.dimension_scores.reliability,
        result.dimension_scores.maintainability,
        result.dimension_scores.performance,
        result.dimension_scores.dependencies,
    );
    let _ = writeln!(out);

    if items.is_empty() {
        let _ = writeln!(out, "No actionable findings. The codebase is clean.");
        return out;
    }

    let _ = writeln!(out, "## Action Items ({} total)", items.len());
    let _ = writeln!(out);

    let mut current_priority = None;

    for (i, item) in items.iter().enumerate() {
        if current_priority != Some(item.priority) {
            current_priority = Some(item.priority);
            let _ = writeln!(out, "### {}", item.priority);
            let _ = writeln!(out);
        }

        let severity_icon = match item.severity {
            Severity::Error => "E",
            Severity::Warning => "W",
            Severity::Info => "I",
        };

        let _ = writeln!(
            out,
            "{}. **[{}] `{}`** ({} occurrence{})",
            i + 1,
            severity_icon,
            item.rule,
            item.count,
            if item.count > 1 { "s" } else { "" }
        );
        let _ = writeln!(out, "   {}", item.description);
        let _ = writeln!(out, "   **Fix:** {}", item.fix_action);

        if item.files.len() <= 5 {
            let _ = writeln!(out, "   **Files:** {}", item.files.join(", "));
        } else if let Some(first_three) = item.files.get(..3) {
            let _ = writeln!(
                out,
                "   **Files:** {} (+{} more)",
                first_three.join(", "),
                item.files.len() - 3
            );
        }
        let _ = writeln!(out);
    }

    // Summary stats
    let p0_count = items.iter().filter(|i| i.priority == Priority::P0).count();
    let p1_count = items.iter().filter(|i| i.priority == Priority::P1).count();
    let p2_count = items.iter().filter(|i| i.priority == Priority::P2).count();
    let p3_count = items.iter().filter(|i| i.priority == Priority::P3).count();

    let _ = writeln!(out, "---");
    let _ = writeln!(
        out,
        "**Summary:** {p0_count} P0, {p1_count} P1, {p2_count} P2, {p3_count} P3"
    );

    if p0_count > 0 {
        let _ = writeln!(
            out,
            "\nP0 items should be fixed immediately before merging."
        );
    }

    out
}

/// Format the plan as a concise terminal-friendly string (no markdown).
pub fn format_plan_terminal(items: &[RemediationItem]) -> String {
    let mut out = String::new();

    if items.is_empty() {
        let _ = writeln!(out, "  No actionable findings.");
        return out;
    }

    for (i, item) in items.iter().enumerate() {
        let icon = match item.priority {
            Priority::P0 => "!!!",
            Priority::P1 => " ! ",
            Priority::P2 => " - ",
            Priority::P3 => "   ",
        };

        let _ = writeln!(
            out,
            "  {icon} {}) {} — {} ({}x)",
            i + 1,
            item.rule,
            item.fix_action.chars().take(80).collect::<String>(),
            item.count,
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Category, DimensionScores, ScoreLabel};
    use std::path::PathBuf;
    use std::time::Duration;

    fn make_result(diagnostics: Vec<Diagnostic>) -> ScanResult {
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
        ScanResult {
            diagnostics,
            score: 85,
            score_label: ScoreLabel::Great,
            dimension_scores: DimensionScores {
                security: 100,
                reliability: 90,
                maintainability: 95,
                performance: 85,
                dependencies: 100,
            },
            source_file_count: 10,
            elapsed: Duration::from_millis(500),
            skipped_passes: vec![],
            error_count,
            warning_count,
            info_count,
            pass_timings: vec![],
            suppressed_security: vec![],
            planned_files: vec![],
            analyzed_files: vec![],
            compiler_evidence: vec![],
            execution: crate::diagnostics::ScanExecution::default(),
        }
    }

    fn make_diag(rule: &str, severity: Severity, category: Category, file: &str) -> Diagnostic {
        Diagnostic {
            file_path: PathBuf::from(file),
            rule: rule.to_string(),
            category,
            severity,
            message: format!("Issue: {rule}"),
            help: Some(format!("Fix {rule}")),
            line: Some(1),
            column: None,
            fix: None,
        }
    }

    #[test]
    fn test_empty_scan_empty_plan() {
        let result = make_result(vec![]);
        let items = generate_plan(&result);
        assert!(items.is_empty());
    }

    #[test]
    fn test_plan_groups_by_rule() {
        let result = make_result(vec![
            make_diag("rule-a", Severity::Warning, Category::Performance, "a.rs"),
            make_diag("rule-a", Severity::Warning, Category::Performance, "b.rs"),
            make_diag("rule-b", Severity::Error, Category::Security, "c.rs"),
        ]);
        let items = generate_plan(&result);
        assert_eq!(items.len(), 2);
        // P0 (security error) should come first
        assert_eq!(items[0].rule, "rule-b");
        assert_eq!(items[0].priority, Priority::P0);
        assert_eq!(items[1].rule, "rule-a");
        assert_eq!(items[1].count, 2);
    }

    #[test]
    fn test_plan_sorted_by_priority() {
        let result = make_result(vec![
            make_diag("info-rule", Severity::Info, Category::Style, "a.rs"),
            make_diag("error-rule", Severity::Error, Category::Correctness, "b.rs"),
            make_diag(
                "warn-rule",
                Severity::Warning,
                Category::Architecture,
                "c.rs",
            ),
        ]);
        let items = generate_plan(&result);
        assert_eq!(items[0].priority, Priority::P0);
        assert_eq!(items[1].priority, Priority::P2);
        assert_eq!(items[2].priority, Priority::P3);
    }

    #[test]
    fn test_skipped_pass_excluded_from_plan() {
        let result = make_result(vec![make_diag(
            "skipped-pass",
            Severity::Info,
            Category::Cargo,
            "Cargo.toml",
        )]);
        let items = generate_plan(&result);
        assert!(items.is_empty());
    }

    #[test]
    fn test_format_markdown_includes_score() {
        let result = make_result(vec![]);
        let items = generate_plan(&result);
        let md = format_plan_markdown(&items, &result);
        assert!(md.contains("85/100"));
        assert!(md.contains("No actionable findings"));
    }

    #[test]
    fn test_format_markdown_with_items() {
        let result = make_result(vec![make_diag(
            "unwrap-in-production",
            Severity::Warning,
            Category::ErrorHandling,
            "src/scanner.rs",
        )]);
        let items = generate_plan(&result);
        let md = format_plan_markdown(&items, &result);
        assert!(md.contains("unwrap-in-production"));
        assert!(md.contains("P1 High"));
        assert!(md.contains("src/scanner.rs"));
    }
}
