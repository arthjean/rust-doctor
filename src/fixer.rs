//! Apply machine-applicable fixes from diagnostics to source files.

use crate::diagnostics::{CodeFix, Diagnostic};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Apply all available fixes from diagnostics to the source files on disk.
/// Returns the number of fixes applied.
pub fn apply_fixes(diagnostics: &[Diagnostic], project_root: &Path) -> usize {
    // Group fixes by file
    let mut fixes_by_file: HashMap<PathBuf, Vec<&CodeFix>> = HashMap::new();
    for d in diagnostics {
        if let Some(ref fix) = d.fix {
            let abs_path = if d.file_path.is_absolute() {
                d.file_path.clone()
            } else {
                project_root.join(&d.file_path)
            };
            fixes_by_file.entry(abs_path).or_default().push(fix);
        }
    }

    let mut total_applied = 0;
    let project_root_canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());

    for (file_path, fixes) in &fixes_by_file {
        // Security: ensure the fix target stays under the project root
        if let Ok(canonical) = file_path.canonicalize()
            && !canonical.starts_with(&project_root_canonical)
        {
            eprintln!(
                "Warning: fix path escapes project root, skipping: {}",
                file_path.display()
            );
            continue;
        }

        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "Warning: cannot read '{}' for fix: {e}",
                    file_path.display()
                );
                continue;
            }
        };

        let lines: Vec<&str> = content.lines().collect();
        let mut new_lines: Vec<String> = lines.iter().map(|l| (*l).to_string()).collect();
        let mut applied_in_file = 0;

        // Sort fixes by line number (descending) to avoid offset shifts
        let mut sorted_fixes: Vec<&&CodeFix> = fixes.iter().collect();
        sorted_fixes.sort_by_key(|f| std::cmp::Reverse(f.line));

        for fix in sorted_fixes {
            let line_idx = (fix.line as usize).saturating_sub(1);
            if let Some(line) = new_lines.get_mut(line_idx)
                && line.contains(&fix.old_text)
            {
                let replaced = line.replacen(&fix.old_text, &fix.new_text, 1);
                *line = replaced;
                applied_in_file += 1;
            }
        }

        if applied_in_file > 0 {
            // Preserve trailing newline
            let mut output = new_lines.join("\n");
            if content.ends_with('\n') {
                output.push('\n');
            }
            if let Err(e) = std::fs::write(file_path, output) {
                eprintln!(
                    "Warning: cannot write fixes to '{}': {e}",
                    file_path.display()
                );
            } else {
                total_applied += applied_in_file;
                eprintln!(
                    "Fixed {} issue(s) in {}",
                    applied_in_file,
                    file_path.display()
                );
            }
        }
    }

    total_applied
}

// ---------------------------------------------------------------------------
// Canonical fix planning (US-016)
// ---------------------------------------------------------------------------

use crate::diagnostics::{CanonicalDiagnostic, FixEligibility, ReportV1};

/// One planned edit: an exact span, its replacement, and the file state it was
/// computed against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFix {
    pub path: String,
    pub start: u32,
    pub end: u32,
    pub replacement: String,
    pub precondition_hash: String,
    pub rule: String,
}

/// Edits that share one root cause and can be validated as a unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixGroup {
    pub root_cause_key: String,
    pub fixes: Vec<PlannedFix>,
}

/// A diagnostic whose remediation stays advice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceNote {
    pub rule: String,
    pub site_id: String,
    /// Why no automatic edit is offered. Never a generic invented fix.
    pub reason: String,
}

/// The decision: what may be edited, and what may only be explained.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixPlan {
    pub groups: Vec<FixGroup>,
    pub guidance_only: Vec<GuidanceNote>,
}

/// Result of applying one group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupOutcome {
    /// Edits applied and the file still parses as Rust.
    Validated { applied: usize },
    /// Applying or validating failed; the group was rolled back.
    Failed { reason: String },
    /// An earlier group failed, so this one was never attempted and must not be
    /// reported as validated (US-016 AC-7).
    NotAttempted,
}

/// Per-group outcome of an apply run, in plan order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixOutcome {
    pub groups: Vec<(String, GroupOutcome)>,
}

impl FixOutcome {
    #[must_use]
    pub fn applied(&self) -> usize {
        self.groups
            .iter()
            .map(|(_, outcome)| match outcome {
                GroupOutcome::Validated { applied } => *applied,
                GroupOutcome::Failed { .. } | GroupOutcome::NotAttempted => 0,
            })
            .sum()
    }

    #[must_use]
    pub fn failure(&self) -> Option<(&str, &str)> {
        self.groups.iter().find_map(|(key, outcome)| match outcome {
            GroupOutcome::Failed { reason } => Some((key.as_str(), reason.as_str())),
            GroupOutcome::Validated { .. } | GroupOutcome::NotAttempted => None,
        })
    }
}

/// Build the fix plan for a canonical report.
///
/// Eligibility is already decided per fix in Report V1. This adds the two
/// checks that need the whole set: overlapping spans inside one file, and edits
/// that a single root-cause group would spread across several files
/// (US-016 AC-3, AC-4).
#[must_use]
pub fn plan_fixes(report: &ReportV1) -> FixPlan {
    let mut plan = FixPlan::default();
    let mut order: Vec<String> = Vec::new();
    let mut candidates: HashMap<String, Vec<PlannedFix>> = HashMap::new();

    // `report.diagnostics` is already in canonical order, so first appearance
    // decides group order and the plan needs no second ranking.
    for diagnostic in &report.diagnostics {
        let Some(key) = diagnostic.root_cause_key.clone() else {
            note_guidance(&mut plan, diagnostic, "the rule has no canonical mapping");
            continue;
        };
        let mut eligible = Vec::new();
        for fix in &diagnostic.fixes {
            match planned_fix(diagnostic, fix) {
                Ok(planned) => eligible.push(planned),
                Err(reason) => note_guidance(&mut plan, diagnostic, &reason),
            }
        }
        if eligible.is_empty() {
            if diagnostic.fixes.is_empty() {
                note_guidance(
                    &mut plan,
                    diagnostic,
                    "no analyzer supplied a machine-applicable edit",
                );
            }
            continue;
        }
        if !candidates.contains_key(&key) {
            order.push(key.clone());
        }
        candidates.entry(key).or_default().extend(eligible);
    }

    for key in order {
        let Some(mut fixes) = candidates.remove(&key) else {
            continue;
        };
        // Deterministic order: file, then span, then replacement.
        fixes.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then(left.start.cmp(&right.start))
                .then(left.end.cmp(&right.end))
                .then(left.replacement.cmp(&right.replacement))
        });
        fixes.dedup();
        if let Some(reason) = group_hazard(&fixes) {
            for fix in &fixes {
                plan.guidance_only.push(GuidanceNote {
                    rule: fix.rule.clone(),
                    site_id: String::new(),
                    reason: reason.to_string(),
                });
            }
            continue;
        }
        plan.groups.push(FixGroup {
            root_cause_key: key,
            fixes,
        });
    }
    plan.guidance_only.sort_by(|left, right| {
        left.rule
            .cmp(&right.rule)
            .then(left.site_id.cmp(&right.site_id))
            .then(left.reason.cmp(&right.reason))
    });
    plan.guidance_only.dedup();
    plan
}

fn note_guidance(plan: &mut FixPlan, diagnostic: &CanonicalDiagnostic, reason: &str) {
    plan.guidance_only.push(GuidanceNote {
        rule: diagnostic.rule.clone(),
        site_id: diagnostic.site_id.clone(),
        reason: reason.to_string(),
    });
}

fn planned_fix(
    diagnostic: &CanonicalDiagnostic,
    fix: &crate::diagnostics::CanonicalFix,
) -> Result<PlannedFix, String> {
    if fix.eligibility != FixEligibility::MachineApplicable {
        return Err(fix
            .ineligible_reason
            .clone()
            .unwrap_or_else(|| "the fix is guidance only".to_string()));
    }
    let crate::diagnostics::DiagnosticLocation::Source { path, range } = &fix.location else {
        return Err("a project-level fix has no span".to_string());
    };
    let (Some(start), Some(end)) = (range.start.byte_offset, range.end.byte_offset) else {
        return Err("the exact byte span is unavailable".to_string());
    };
    let precondition_hash = fix
        .precondition_hash
        .clone()
        .ok_or_else(|| "no precondition hash was recorded".to_string())?;
    Ok(PlannedFix {
        path: path.clone(),
        start,
        end,
        replacement: fix.replacement.clone(),
        precondition_hash,
        rule: diagnostic.rule.clone(),
    })
}

/// Hazards that only appear once a group's edits are seen together.
fn group_hazard(fixes: &[PlannedFix]) -> Option<&'static str> {
    if fixes
        .iter()
        .map(|fix| fix.path.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        > 1
    {
        return Some("one root cause spans several files; the edit is not span-local");
    }
    fixes
        .windows(2)
        .any(|pair| pair[1].start < pair[0].end)
        .then_some("two suggested edits overlap in the same file")
}

/// Apply a plan group by group, validating each before attempting the next.
///
/// A group is applied atomically: its edits are written only after the patched
/// text still parses as Rust. When a group fails, every later group is reported
/// as `NotAttempted` rather than presented as validated (US-016 AC-7).
#[must_use]
pub fn apply_plan(plan: &FixPlan, project_root: &Path) -> FixOutcome {
    let mut outcome = FixOutcome::default();
    let mut halted = false;
    for group in &plan.groups {
        if halted {
            outcome
                .groups
                .push((group.root_cause_key.clone(), GroupOutcome::NotAttempted));
            continue;
        }
        match apply_group(group, project_root) {
            Ok(applied) => outcome.groups.push((
                group.root_cause_key.clone(),
                GroupOutcome::Validated { applied },
            )),
            Err(reason) => {
                halted = true;
                outcome.groups.push((
                    group.root_cause_key.clone(),
                    GroupOutcome::Failed { reason },
                ));
            }
        }
    }
    outcome
}

fn apply_group(group: &FixGroup, project_root: &Path) -> Result<usize, String> {
    let Some(first) = group.fixes.first() else {
        return Ok(0);
    };
    let absolute = project_root.join(&first.path);
    let canonical_root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let canonical_target = absolute
        .canonicalize()
        .map_err(|error| format!("target file is unavailable: {error}"))?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err("fix path escapes the project root".to_string());
    }
    let source = std::fs::read(&canonical_target)
        .map_err(|error| format!("target file could not be read: {error}"))?;
    if crate::diagnostics::sha256_hex_bytes(&source) != first.precondition_hash {
        return Err("the file changed since the scan; the fix is stale".to_string());
    }
    let text =
        String::from_utf8(source).map_err(|_| "target file is not valid UTF-8".to_string())?;

    // Apply from the end so earlier byte offsets stay valid.
    let mut patched = text;
    for fix in group.fixes.iter().rev() {
        let (start, end) = (fix.start as usize, fix.end as usize);
        if end > patched.len() || !patched.is_char_boundary(start) || !patched.is_char_boundary(end)
        {
            return Err("the span does not fall on a character boundary".to_string());
        }
        patched.replace_range(start..end, &fix.replacement);
    }
    syn::parse_file(&patched)
        .map_err(|error| format!("the patched file no longer parses: {error}"))?;
    std::fs::write(&canonical_target, patched)
        .map_err(|error| format!("the patched file could not be written: {error}"))?;
    Ok(group.fixes.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Category, Severity};
    use std::io::Write;

    #[test]
    fn test_apply_fix_replaces_text() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "fn main() {{").unwrap();
        writeln!(f, "    let s = \"hello\".to_string();").unwrap();
        writeln!(f, "}}").unwrap();

        let diags = vec![Diagnostic {
            file_path: file_path.clone(),
            rule: "test-rule".to_string(),
            category: Category::Performance,
            severity: Severity::Info,
            message: "test".to_string(),
            help: None,
            line: Some(2),
            column: None,
            fix: Some(CodeFix {
                old_text: "\"hello\".to_string()".to_string(),
                new_text: "String::from(\"hello\")".to_string(),
                line: 2,
            }),
        }];

        let applied = apply_fixes(&diags, dir.path());
        assert_eq!(applied, 1);

        let result = std::fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("String::from(\"hello\")"));
        assert!(!result.contains(".to_string()"));
    }

    #[test]
    fn test_no_fixes_returns_zero() {
        let diags = vec![Diagnostic {
            file_path: PathBuf::from("nonexistent.rs"),
            rule: "test".to_string(),
            category: Category::Style,
            severity: Severity::Info,
            message: "test".to_string(),
            help: None,
            line: Some(1),
            column: None,
            fix: None, // No fix
        }];

        let applied = apply_fixes(&diags, Path::new("."));
        assert_eq!(applied, 0);
    }

    #[test]
    fn test_multi_fix_in_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("multi.rs");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "fn main() {{").unwrap();
        writeln!(f, "    let a = \"hello\".to_string();").unwrap();
        writeln!(f, "    let b = \"world\".to_string();").unwrap();
        writeln!(f, "    let c = \"foo\".to_string();").unwrap();
        writeln!(f, "}}").unwrap();

        let diags = vec![
            Diagnostic {
                file_path: file_path.clone(),
                rule: "test-rule".to_string(),
                category: Category::Performance,
                severity: Severity::Info,
                message: "test".to_string(),
                help: None,
                line: Some(2),
                column: None,
                fix: Some(CodeFix {
                    old_text: "\"hello\".to_string()".to_string(),
                    new_text: "String::from(\"hello\")".to_string(),
                    line: 2,
                }),
            },
            Diagnostic {
                file_path: file_path.clone(),
                rule: "test-rule".to_string(),
                category: Category::Performance,
                severity: Severity::Info,
                message: "test".to_string(),
                help: None,
                line: Some(4),
                column: None,
                fix: Some(CodeFix {
                    old_text: "\"foo\".to_string()".to_string(),
                    new_text: "String::from(\"foo\")".to_string(),
                    line: 4,
                }),
            },
        ];

        let applied = apply_fixes(&diags, dir.path());
        assert_eq!(applied, 2);

        let result = std::fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("String::from(\"hello\")"));
        assert!(result.contains("String::from(\"foo\")"));
        // Line 3 should be unchanged
        assert!(result.contains("\"world\".to_string()"));
    }

    #[test]
    fn test_fixes_on_adjacent_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("adjacent.rs");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "fn main() {{").unwrap();
        writeln!(f, "    let a = \"one\".to_string();").unwrap();
        writeln!(f, "    let b = \"two\".to_string();").unwrap();
        writeln!(f, "}}").unwrap();

        let diags = vec![
            Diagnostic {
                file_path: file_path.clone(),
                rule: "test-rule".to_string(),
                category: Category::Performance,
                severity: Severity::Info,
                message: "test".to_string(),
                help: None,
                line: Some(2),
                column: None,
                fix: Some(CodeFix {
                    old_text: "\"one\".to_string()".to_string(),
                    new_text: "String::from(\"one\")".to_string(),
                    line: 2,
                }),
            },
            Diagnostic {
                file_path: file_path.clone(),
                rule: "test-rule".to_string(),
                category: Category::Performance,
                severity: Severity::Info,
                message: "test".to_string(),
                help: None,
                line: Some(3),
                column: None,
                fix: Some(CodeFix {
                    old_text: "\"two\".to_string()".to_string(),
                    new_text: "String::from(\"two\")".to_string(),
                    line: 3,
                }),
            },
        ];

        let applied = apply_fixes(&diags, dir.path());
        assert_eq!(applied, 2);

        let result = std::fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("String::from(\"one\")"));
        assert!(result.contains("String::from(\"two\")"));
        assert!(!result.contains(".to_string()"));
    }

    #[test]
    fn test_fix_on_last_line() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("lastline.rs");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "fn main() {{").unwrap();
        writeln!(f, "    println!(\"done\");").unwrap();
        writeln!(f, "}}").unwrap();

        let diags = vec![Diagnostic {
            file_path: file_path.clone(),
            rule: "test-rule".to_string(),
            category: Category::Style,
            severity: Severity::Info,
            message: "test".to_string(),
            help: None,
            line: Some(3),
            column: None,
            fix: Some(CodeFix {
                old_text: "}".to_string(),
                new_text: "} // end".to_string(),
                line: 3,
            }),
        }];

        let applied = apply_fixes(&diags, dir.path());
        assert_eq!(applied, 1);

        let result = std::fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("} // end"));
    }

    #[test]
    fn test_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("empty.rs");
        std::fs::File::create(&file_path).unwrap();

        let diags = vec![Diagnostic {
            file_path,
            rule: "test-rule".to_string(),
            category: Category::Style,
            severity: Severity::Info,
            message: "test".to_string(),
            help: None,
            line: Some(1),
            column: None,
            fix: Some(CodeFix {
                old_text: "old".to_string(),
                new_text: "new".to_string(),
                line: 1,
            }),
        }];

        let applied = apply_fixes(&diags, dir.path());
        assert_eq!(applied, 0);
    }

    #[test]
    fn test_fix_targeting_nonexistent_line() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("short.rs");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "line one").unwrap();
        writeln!(f, "line two").unwrap();
        writeln!(f, "line three").unwrap();

        let diags = vec![Diagnostic {
            file_path: file_path.clone(),
            rule: "test-rule".to_string(),
            category: Category::Style,
            severity: Severity::Info,
            message: "test".to_string(),
            help: None,
            line: Some(10),
            column: None,
            fix: Some(CodeFix {
                old_text: "anything".to_string(),
                new_text: "replaced".to_string(),
                line: 10,
            }),
        }];

        let applied = apply_fixes(&diags, dir.path());
        assert_eq!(applied, 0);

        // File should be unchanged
        let result = std::fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("line one"));
        assert!(result.contains("line two"));
        assert!(result.contains("line three"));
    }

    #[test]
    fn test_fixes_with_overlapping_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("overlap.rs");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "fn main() {{").unwrap();
        writeln!(f, "    let x = foo(bar());").unwrap();
        writeln!(f, "}}").unwrap();

        // Two fixes on the same line, targeting different text
        let diags = vec![
            Diagnostic {
                file_path: file_path.clone(),
                rule: "rule-a".to_string(),
                category: Category::Performance,
                severity: Severity::Info,
                message: "test".to_string(),
                help: None,
                line: Some(2),
                column: None,
                fix: Some(CodeFix {
                    old_text: "foo".to_string(),
                    new_text: "baz".to_string(),
                    line: 2,
                }),
            },
            Diagnostic {
                file_path: file_path.clone(),
                rule: "rule-b".to_string(),
                category: Category::Performance,
                severity: Severity::Info,
                message: "test".to_string(),
                help: None,
                line: Some(2),
                column: None,
                fix: Some(CodeFix {
                    old_text: "bar".to_string(),
                    new_text: "qux".to_string(),
                    line: 2,
                }),
            },
        ];

        let applied = apply_fixes(&diags, dir.path());
        assert_eq!(applied, 2);

        let result = std::fs::read_to_string(&file_path).unwrap();
        // Both replacements should have been applied to line 2
        assert!(result.contains("baz(qux())"));
        assert!(!result.contains("foo"));
        assert!(!result.contains("bar"));
    }

    #[test]
    fn test_relative_path_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let sub_dir = dir.path().join("src");
        std::fs::create_dir_all(&sub_dir).unwrap();
        let file_path = sub_dir.join("main.rs");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "fn main() {{").unwrap();
        writeln!(f, "    let x = \"old\".to_string();").unwrap();
        writeln!(f, "}}").unwrap();

        // Use a relative path in the diagnostic
        let diags = vec![Diagnostic {
            file_path: PathBuf::from("src/main.rs"),
            rule: "test-rule".to_string(),
            category: Category::Performance,
            severity: Severity::Info,
            message: "test".to_string(),
            help: None,
            line: Some(2),
            column: None,
            fix: Some(CodeFix {
                old_text: "\"old\".to_string()".to_string(),
                new_text: "String::from(\"new\")".to_string(),
                line: 2,
            }),
        }];

        let applied = apply_fixes(&diags, dir.path());
        assert_eq!(applied, 1);

        let result = std::fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("String::from(\"new\")"));
        assert!(!result.contains("\"old\".to_string()"));
    }

    #[test]
    fn test_fix_preserves_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("trailing.rs");
        // Write content WITH a trailing newline
        std::fs::write(&file_path, "fn main() {\n    old_func();\n}\n").unwrap();

        let diags = vec![Diagnostic {
            file_path: file_path.clone(),
            rule: "test-rule".to_string(),
            category: Category::Style,
            severity: Severity::Info,
            message: "test".to_string(),
            help: None,
            line: Some(2),
            column: None,
            fix: Some(CodeFix {
                old_text: "old_func()".to_string(),
                new_text: "new_func()".to_string(),
                line: 2,
            }),
        }];

        let applied = apply_fixes(&diags, dir.path());
        assert_eq!(applied, 1);

        let result = std::fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("new_func()"));
        assert!(result.ends_with('\n'), "File should still end with newline");
    }

    #[test]
    fn test_fix_on_file_without_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("no_trailing.rs");
        // Write content WITHOUT a trailing newline
        std::fs::write(&file_path, "fn main() {\n    old_func();\n}").unwrap();

        let diags = vec![Diagnostic {
            file_path: file_path.clone(),
            rule: "test-rule".to_string(),
            category: Category::Style,
            severity: Severity::Info,
            message: "test".to_string(),
            help: None,
            line: Some(2),
            column: None,
            fix: Some(CodeFix {
                old_text: "old_func()".to_string(),
                new_text: "new_func()".to_string(),
                line: 2,
            }),
        }];

        let applied = apply_fixes(&diags, dir.path());
        assert_eq!(applied, 1);

        let result = std::fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("new_func()"));
        assert!(!result.ends_with('\n'), "File should NOT end with newline");
    }
}

#[cfg(test)]
mod fix_eligibility_tests {
    use super::*;
    use crate::diagnostics::{
        CanonicalFix, Category, DiagnosticLocation, FixApplicability, SourcePosition, SourceRange,
    };

    fn canonical(rule: &str, category: Category, fixes: Vec<CanonicalFix>) -> CanonicalDiagnostic {
        CanonicalDiagnostic {
            provider: "rust-doctor".to_string(),
            rule: rule.to_string(),
            title: rule.to_string(),
            category,
            severity: crate::diagnostics::Severity::Warning,
            message: format!("{rule} fired"),
            help: None,
            url: String::new(),
            tags: Vec::new(),
            analysis_kind: "clippy".to_string(),
            confidence: "high".to_string(),
            original_level: "warning".to_string(),
            ownership: crate::diagnostics::DiagnosticOwnership::Workspace,
            source_surface: crate::diagnostics::SourceSurface::Library,
            location: DiagnosticLocation::Project,
            related_locations: Vec::new(),
            macro_expansion: None,
            fixes,
            visible_on: Vec::new(),
            site_id: format!("site-{rule}"),
            baseline_key: format!("key-{rule}"),
            namespace_fallback: false,
            advisory: false,
            priority: Some("p2".to_string()),
            trust_tier: "compiler-proven".to_string(),
            score_eligible: true,
            score_impact: crate::diagnostics::ScoreImpact::Scored,
            aggregation_policy: "bounded-occurrence".to_string(),
            root_cause_key: Some(format!("rule:{rule}")),
            evidence_summary: String::new(),
            limitations: Vec::new(),
            fix_recipe: None,
            suppressed: false,
        }
    }

    fn fix(path: &str, start: u32, end: u32, replacement: &str, hash: &str) -> CanonicalFix {
        CanonicalFix {
            group_id: None,
            applicability: FixApplicability::MachineApplicable,
            replacement: replacement.to_string(),
            location: DiagnosticLocation::Source {
                path: path.to_string(),
                range: SourceRange {
                    start: SourcePosition {
                        line: 1,
                        column: 1,
                        byte_offset: Some(start),
                    },
                    end: SourcePosition {
                        line: 1,
                        column: 1,
                        byte_offset: Some(end),
                    },
                },
            },
            eligibility: FixEligibility::MachineApplicable,
            precondition_hash: Some(hash.to_string()),
            ineligible_reason: None,
        }
    }

    fn report(diagnostics: Vec<CanonicalDiagnostic>) -> ReportV1 {
        let mut report = ReportV1::failure(
            Path::new("/repo"),
            crate::diagnostics::ScanMode::Full,
            "test",
            "fixture",
        );
        report.diagnostics = diagnostics;
        report
    }

    #[test]
    fn a_fix_without_an_exact_span_or_hash_stays_guidance_only() {
        let mut incomplete = fix("src/lib.rs", 0, 4, "value", "hash");
        incomplete.precondition_hash = None;
        let plan = plan_fixes(&report(vec![canonical(
            "clippy::redundant_clone",
            Category::Performance,
            vec![incomplete],
        )]));
        assert!(plan.groups.is_empty());
        assert_eq!(plan.guidance_only.len(), 1);
        assert!(plan.guidance_only[0].reason.contains("precondition hash"));
    }

    #[test]
    fn overlapping_edits_in_one_file_are_guidance_only() {
        let plan = plan_fixes(&report(vec![canonical(
            "clippy::redundant_clone",
            Category::Performance,
            vec![
                fix("src/lib.rs", 0, 10, "a", "hash"),
                fix("src/lib.rs", 5, 15, "b", "hash"),
            ],
        )]));
        assert!(plan.groups.is_empty());
        assert!(
            plan.guidance_only
                .iter()
                .any(|note| note.reason.contains("overlap"))
        );
    }

    #[test]
    fn a_root_cause_spanning_several_files_is_guidance_only() {
        let plan = plan_fixes(&report(vec![canonical(
            "clippy::redundant_clone",
            Category::Performance,
            vec![
                fix("src/a.rs", 0, 4, "a", "hash"),
                fix("src/b.rs", 0, 4, "b", "hash"),
            ],
        )]));
        assert!(plan.groups.is_empty());
        assert!(
            plan.guidance_only
                .iter()
                .any(|note| note.reason.contains("several files"))
        );
    }

    #[test]
    fn a_diagnostic_without_a_fix_states_the_decision_boundary() {
        let plan = plan_fixes(&report(vec![canonical(
            "unwrap-in-production",
            Category::ErrorHandling,
            Vec::new(),
        )]));
        assert!(plan.groups.is_empty());
        assert_eq!(plan.guidance_only.len(), 1);
        assert!(
            plan.guidance_only[0]
                .reason
                .contains("no analyzer supplied a machine-applicable edit")
        );
    }

    #[test]
    fn a_stale_file_is_never_edited() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("src")).unwrap();
        let path = directory.path().join("src/lib.rs");
        std::fs::write(&path, "pub fn value() -> u8 { 1 }\n").unwrap();
        let plan = FixPlan {
            groups: vec![FixGroup {
                root_cause_key: "rule:demo".to_string(),
                fixes: vec![PlannedFix {
                    path: "src/lib.rs".to_string(),
                    start: 23,
                    end: 24,
                    replacement: "2".to_string(),
                    precondition_hash: "not-the-current-hash".to_string(),
                    rule: "demo".to_string(),
                }],
            }],
            guidance_only: Vec::new(),
        };
        let outcome = apply_plan(&plan, directory.path());
        assert_eq!(outcome.applied(), 0);
        assert!(outcome.failure().unwrap().1.contains("stale"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "pub fn value() -> u8 { 1 }\n"
        );
    }

    #[test]
    fn an_eligible_group_applies_parses_and_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("src")).unwrap();
        let path = directory.path().join("src/lib.rs");
        let original = "pub fn value() -> u8 { 1 }\n";
        std::fs::write(&path, original).unwrap();
        let hash = crate::diagnostics::sha256_hex_bytes(original.as_bytes());
        let plan = FixPlan {
            groups: vec![FixGroup {
                root_cause_key: "rule:demo".to_string(),
                fixes: vec![PlannedFix {
                    path: "src/lib.rs".to_string(),
                    start: 23,
                    end: 24,
                    replacement: "2".to_string(),
                    precondition_hash: hash,
                    rule: "demo".to_string(),
                }],
            }],
            guidance_only: Vec::new(),
        };
        let outcome = apply_plan(&plan, directory.path());
        assert_eq!(outcome.applied(), 1);
        let patched = std::fs::read_to_string(&path).unwrap();
        assert_eq!(patched, "pub fn value() -> u8 { 2 }\n");
        assert!(syn::parse_file(&patched).is_ok());

        // Replaying the same plan is refused: the precondition no longer holds.
        let replay = apply_plan(&plan, directory.path());
        assert_eq!(replay.applied(), 0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), patched);
    }

    #[test]
    fn a_failed_group_stops_later_groups_from_claiming_validation() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("src")).unwrap();
        std::fs::write(directory.path().join("src/lib.rs"), "pub fn a() {}\n").unwrap();
        let stale = FixGroup {
            root_cause_key: "rule:first".to_string(),
            fixes: vec![PlannedFix {
                path: "src/lib.rs".to_string(),
                start: 0,
                end: 3,
                replacement: "pub".to_string(),
                precondition_hash: "stale".to_string(),
                rule: "first".to_string(),
            }],
        };
        let later = FixGroup {
            root_cause_key: "rule:second".to_string(),
            fixes: stale.fixes.clone(),
        };
        let outcome = apply_plan(
            &FixPlan {
                groups: vec![stale, later],
                guidance_only: Vec::new(),
            },
            directory.path(),
        );
        assert!(matches!(outcome.groups[0].1, GroupOutcome::Failed { .. }));
        assert_eq!(outcome.groups[1].1, GroupOutcome::NotAttempted);
    }

    #[test]
    fn policy_sensitive_families_are_never_automatic() {
        for (rule, category) in [
            ("hardcoded-secrets", Category::Security),
            ("unsafe-block-audit", Category::Security),
            ("missing-msrv", Category::Cargo),
            ("unused-dependency", Category::Dependencies),
            ("box-dyn-error-in-public-api", Category::ErrorHandling),
        ] {
            let mut candidate = crate::diagnostics::CanonicalFix {
                group_id: None,
                applicability: FixApplicability::MachineApplicable,
                replacement: "x".to_string(),
                location: DiagnosticLocation::Source {
                    path: "src/lib.rs".to_string(),
                    range: SourceRange {
                        start: SourcePosition {
                            line: 1,
                            column: 1,
                            byte_offset: Some(0),
                        },
                        end: SourcePosition {
                            line: 1,
                            column: 1,
                            byte_offset: Some(1),
                        },
                    },
                },
                eligibility: FixEligibility::MachineApplicable,
                precondition_hash: None,
                ineligible_reason: None,
            };
            crate::diagnostics::decide_fix_eligibility(
                &mut candidate,
                Path::new("/repo"),
                &category,
                rule,
                false,
            );
            assert_eq!(
                candidate.eligibility,
                FixEligibility::GuidanceOnly,
                "{rule} became automatically applicable"
            );
            assert!(candidate.ineligible_reason.is_some());
        }
    }
}
