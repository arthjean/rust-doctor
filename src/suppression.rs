use crate::diagnostics::Diagnostic;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const DISABLE_NEXT_LINE: &str = "rust-doctor-disable-next-line";
const DISABLE_LINE: &str = "rust-doctor-disable-line";

/// A suppression directive found in source code.
#[derive(Debug)]
struct Suppression {
    /// The line this suppression applies to (1-based).
    target_line: u32,
    /// Rule name to suppress, or None for all rules.
    rule: Option<String>,
}

/// Apply inline suppression comments to filter diagnostics.
///
/// Reads source files referenced by diagnostics, finds `rust-doctor-disable-*`
/// comments, and removes matching diagnostics. Returns the filtered list and
/// the count of suppressed diagnostics.
pub fn apply_inline_suppressions(
    diagnostics: Vec<Diagnostic>,
    project_root: &Path,
) -> (Vec<Diagnostic>, usize) {
    if diagnostics.is_empty() {
        return (diagnostics, 0);
    }

    // Collect unique file paths that have diagnostics with line numbers
    let file_set: std::collections::HashSet<PathBuf> = diagnostics
        .iter()
        .filter(|d| d.line.is_some())
        .map(|d| d.file_path.clone())
        .collect();
    let files_to_check: Vec<PathBuf> = file_set.into_iter().collect();

    // Parse suppression comments from each file
    let mut suppressions: HashMap<PathBuf, Vec<Suppression>> = HashMap::new();
    for file_path in files_to_check {
        // Try absolute path first, then relative to project root
        let abs_buf;
        let abs_path: &Path = if file_path.is_absolute() {
            &file_path
        } else {
            abs_buf = project_root.join(&file_path);
            &abs_buf
        };

        // Guard (fail-closed): resolve symlinks/`..` and verify the file is under
        // the project root BEFORE reading it. If the path can't be canonicalized
        // or escapes the root, skip it — never read an unresolved or out-of-tree
        // path. Reading the *canonical* path (not `abs_path`) closes the symlink
        // TOCTOU window between this check and the read.
        let (Ok(canonical), Ok(root_canonical)) =
            (abs_path.canonicalize(), project_root.canonicalize())
        else {
            continue;
        };
        if !canonical.starts_with(&root_canonical) {
            continue; // skip out-of-tree files
        }

        if let Ok(content) = fs::read_to_string(&canonical) {
            let file_suppressions = parse_suppressions(&content);
            if !file_suppressions.is_empty() {
                // Key by the normalized absolute path so lookups anchor to a
                // single file identity (US-011), independent of the abs/rel
                // path shape clippy vs the rule engine happen to emit.
                suppressions.insert(normalize_path(&file_path, project_root), file_suppressions);
            }
        }
    }

    if suppressions.is_empty() {
        return (diagnostics, 0);
    }

    // Filter diagnostics
    let original_count = diagnostics.len();
    let filtered: Vec<Diagnostic> = diagnostics
        .into_iter()
        .filter(|d| !is_suppressed(d, &suppressions, project_root))
        .collect();
    let suppressed_count = original_count - filtered.len();

    (filtered, suppressed_count)
}

/// Return a human-readable reason when an inline directive suppresses one finding.
///
/// The source read uses the same canonical in-root guard as normal suppression
/// application. Unresolved paths, symlink escapes, and unreadable sources yield
/// no evidence instead of allowing an out-of-project read.
pub fn inline_suppression_reason(diagnostic: &Diagnostic, project_root: &Path) -> Option<String> {
    let line = diagnostic.line?;
    let joined = if diagnostic.file_path.is_absolute() {
        diagnostic.file_path.clone()
    } else {
        project_root.join(&diagnostic.file_path)
    };
    let canonical = joined.canonicalize().ok()?;
    let root = project_root.canonicalize().ok()?;
    if !canonical.starts_with(&root) {
        return None;
    }
    let content = fs::read_to_string(canonical).ok()?;
    parse_suppressions(&content)
        .into_iter()
        .find(|candidate| {
            candidate.target_line == line
                && (candidate.rule.is_none()
                    || candidate.rule.as_deref() == Some(diagnostic.rule.as_str()))
        })
        .map(|candidate| {
            candidate.rule.map_or_else(
                || format!("inline suppression targets every rule on line {line}"),
                |rule| format!("inline suppression targets rule {rule} on line {line}"),
            )
        })
}

/// Parse suppression comments from file content.
fn parse_suppressions(content: &str) -> Vec<Suppression> {
    let mut suppressions = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        let line_num = (line_idx + 1) as u32;
        let trimmed = line.trim();

        // Check for // rust-doctor-disable-next-line [rule]
        if let Some(rest) = extract_comment_directive(trimmed, DISABLE_NEXT_LINE) {
            let rule = parse_rule_name(rest);
            suppressions.push(Suppression {
                target_line: line_num + 1, // applies to the NEXT line
                rule,
            });
        }

        // Check for // rust-doctor-disable-line [rule]
        if let Some(rest) = extract_comment_directive(trimmed, DISABLE_LINE) {
            let rule = parse_rule_name(rest);
            suppressions.push(Suppression {
                target_line: line_num, // applies to THIS line
                rule,
            });
        }

        // Also check for inline comments at end of line: `code // rust-doctor-disable-line`
        // We must avoid matching `//` inside string literals. Use a simple heuristic:
        // find the last `//` on the line that is NOT preceded by `:` (to skip URLs like https://)
        // and is outside string literals (approximate: count unescaped `"` before the `//`).
        if !trimmed.starts_with("//")
            && let Some(comment_start) = find_line_comment_start(line)
        {
            let comment = line[comment_start + 2..].trim();
            if let Some(rest) = comment.strip_prefix(DISABLE_LINE) {
                let rule = parse_rule_name(rest);
                suppressions.push(Suppression {
                    target_line: line_num,
                    rule,
                });
            }
        }
    }

    suppressions
}

/// Find the start of a line comment (`//`) that is NOT inside a string literal.
/// Returns the byte offset of the `//` or `None` if no valid comment is found.
fn find_line_comment_start(line: &str) -> Option<usize> {
    let mut in_string = false;
    let mut prev_backslash = false;
    let bytes = line.as_bytes();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if in_string {
            if b == b'\\' && !prev_backslash {
                prev_backslash = true;
                i += 1;
                continue;
            }
            if b == b'"' && !prev_backslash {
                in_string = false;
            }
            prev_backslash = false;
        } else if b == b'"' {
            in_string = true;
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Extract the rest of a comment after a directive prefix.
fn extract_comment_directive<'a>(line: &'a str, directive: &str) -> Option<&'a str> {
    // Match: // directive [rest]
    let stripped = line.strip_prefix("//")?;
    let stripped = stripped.trim_start();
    stripped.strip_prefix(directive).map(str::trim)
}

/// Parse an optional rule name from the rest of a directive.
fn parse_rule_name(rest: &str) -> Option<String> {
    let name = rest.trim();
    if name.is_empty() {
        None // No rule = suppress all
    } else {
        Some(name.to_string())
    }
}

/// Resolve a path to an absolute, normalized identity anchored at `project_root`.
///
/// Relative paths are joined onto `project_root`; the result is canonicalized
/// when the file exists (resolving `.`/`..`/symlinks), else returned lexically.
/// This gives every file one absolute identity, so suppression matching can be
/// an exact lookup instead of a path-suffix test. It kills the cross-member bug
/// (US-011) — a `// rust-doctor-disable-*` in `crateB/src/main.rs` can no longer
/// neutralize a diagnostic in `crateA/src/main.rs` — while still bridging the
/// absolute-vs-relative mismatch clippy and the rule engine emit for the SAME
/// file.
fn normalize_path(path: &Path, project_root: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    };
    joined.canonicalize().unwrap_or(joined)
}

/// Check if a diagnostic is suppressed by a suppression in its file.
///
/// Matching anchors both sides to their absolute path identity (see
/// [`normalize_path`]): no path-suffix matching, so homonymous files in
/// different workspace members never cross-suppress (US-011).
fn is_suppressed(
    diag: &Diagnostic,
    suppressions: &HashMap<PathBuf, Vec<Suppression>>,
    project_root: &Path,
) -> bool {
    let Some(line) = diag.line else {
        return false; // Diagnostics without line numbers can't be suppressed inline
    };

    let key = normalize_path(&diag.file_path, project_root);
    let Some(file_suppressions) = suppressions.get(&key) else {
        return false;
    };

    file_suppressions.iter().any(|s| {
        s.target_line == line && (s.rule.is_none() || s.rule.as_deref() == Some(diag.rule.as_str()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Category, Severity};

    fn make_diag(file: &str, rule: &str, line: u32) -> Diagnostic {
        Diagnostic {
            file_path: PathBuf::from(file),
            rule: rule.to_string(),
            category: Category::Style,
            severity: Severity::Warning,
            message: "test".to_string(),
            help: None,
            line: Some(line),
            column: None,
            fix: None,
        }
    }

    // --- parse_suppressions ---

    #[test]
    fn test_parse_disable_next_line_with_rule() {
        let content =
            "// rust-doctor-disable-next-line unwrap-in-production\nlet x = foo.unwrap();\n";
        let supps = parse_suppressions(content);
        assert_eq!(supps.len(), 1);
        assert_eq!(supps[0].target_line, 2);
        assert_eq!(supps[0].rule, Some("unwrap-in-production".to_string()));
    }

    #[test]
    fn test_parse_disable_next_line_no_rule() {
        let content = "// rust-doctor-disable-next-line\nlet x = foo.unwrap();\n";
        let supps = parse_suppressions(content);
        assert_eq!(supps.len(), 1);
        assert_eq!(supps[0].target_line, 2);
        assert_eq!(supps[0].rule, None);
    }

    #[test]
    fn test_parse_disable_line() {
        let content = "let x = foo.unwrap(); // rust-doctor-disable-line\n";
        let supps = parse_suppressions(content);
        assert_eq!(supps.len(), 1);
        assert_eq!(supps[0].target_line, 1);
        assert_eq!(supps[0].rule, None);
    }

    #[test]
    fn test_parse_disable_line_with_rule() {
        let content = "let x = foo.unwrap(); // rust-doctor-disable-line unwrap-in-production\n";
        let supps = parse_suppressions(content);
        assert_eq!(supps.len(), 1);
        assert_eq!(supps[0].rule, Some("unwrap-in-production".to_string()));
    }

    #[test]
    fn test_parse_standalone_disable_line_comment() {
        let content = "// rust-doctor-disable-line some-rule\n";
        let supps = parse_suppressions(content);
        assert_eq!(supps.len(), 1);
        assert_eq!(supps[0].target_line, 1);
        assert_eq!(supps[0].rule, Some("some-rule".to_string()));
    }

    #[test]
    fn test_parse_no_suppressions() {
        let content = "fn main() {\n    println!(\"hello\");\n}\n";
        let supps = parse_suppressions(content);
        assert!(supps.is_empty());
    }

    #[test]
    fn test_parse_multiple_suppressions() {
        let content = "// rust-doctor-disable-next-line rule-a\nline1\n// rust-doctor-disable-next-line rule-b\nline2\n";
        let supps = parse_suppressions(content);
        assert_eq!(supps.len(), 2);
    }

    // --- is_suppressed ---

    /// A non-existent root: `normalize_path` falls back to a lexical join, so
    /// keys and diagnostics resolve deterministically without touching disk.
    const TEST_ROOT: &str = "/rust-doctor-suppression-test-root";

    /// Build a suppression map keyed the same way the production code keys it
    /// (normalized absolute path), so unit tests exercise the real lookup path.
    fn supp_map(file: &str, suppressions: Vec<Suppression>) -> HashMap<PathBuf, Vec<Suppression>> {
        let mut map = HashMap::new();
        map.insert(
            normalize_path(Path::new(file), Path::new(TEST_ROOT)),
            suppressions,
        );
        map
    }

    #[test]
    fn test_suppressed_by_specific_rule() {
        let diag = make_diag("test.rs", "unwrap-in-production", 5);
        let suppressions = supp_map(
            "test.rs",
            vec![Suppression {
                target_line: 5,
                rule: Some("unwrap-in-production".to_string()),
            }],
        );
        assert!(is_suppressed(&diag, &suppressions, Path::new(TEST_ROOT)));
    }

    #[test]
    fn test_suppressed_by_wildcard() {
        let diag = make_diag("test.rs", "any-rule", 5);
        let suppressions = supp_map(
            "test.rs",
            vec![Suppression {
                target_line: 5,
                rule: None,
            }],
        );
        assert!(is_suppressed(&diag, &suppressions, Path::new(TEST_ROOT)));
    }

    #[test]
    fn test_not_suppressed_wrong_rule() {
        let diag = make_diag("test.rs", "rule-a", 5);
        let suppressions = supp_map(
            "test.rs",
            vec![Suppression {
                target_line: 5,
                rule: Some("rule-b".to_string()),
            }],
        );
        assert!(!is_suppressed(&diag, &suppressions, Path::new(TEST_ROOT)));
    }

    #[test]
    fn test_not_suppressed_wrong_line() {
        let diag = make_diag("test.rs", "rule-a", 5);
        let suppressions = supp_map(
            "test.rs",
            vec![Suppression {
                target_line: 10,
                rule: Some("rule-a".to_string()),
            }],
        );
        assert!(!is_suppressed(&diag, &suppressions, Path::new(TEST_ROOT)));
    }

    #[test]
    fn test_not_suppressed_no_line_number() {
        let mut diag = make_diag("test.rs", "rule-a", 5);
        diag.line = None;
        let suppressions = supp_map(
            "test.rs",
            vec![Suppression {
                target_line: 5,
                rule: None,
            }],
        );
        assert!(!is_suppressed(&diag, &suppressions, Path::new(TEST_ROOT)));
    }

    // --- US-011: cross-member suppression matching ---

    #[test]
    fn test_homonym_members_do_not_cross_suppress() {
        // A suppression keyed under `crateB/src/main.rs` must NOT neutralize a
        // diagnostic carrying the shorter member-relative `src/main.rs` path —
        // exactly the orientation the old bidirectional `k.ends_with(diag)`
        // suffix match got wrong.
        let diag = make_diag("src/main.rs", "rule-a", 5);
        let suppressions = supp_map(
            "crateB/src/main.rs",
            vec![Suppression {
                target_line: 5,
                rule: Some("rule-a".to_string()),
            }],
        );
        assert!(!is_suppressed(&diag, &suppressions, Path::new(TEST_ROOT)));
    }

    #[test]
    fn test_homonym_members_do_not_cross_suppress_mirror() {
        // The mirror orientation (`diag.ends_with(k)`) must also not match.
        let diag = make_diag("crateA/src/main.rs", "rule-a", 5);
        let suppressions = supp_map(
            "src/main.rs",
            vec![Suppression {
                target_line: 5,
                rule: Some("rule-a".to_string()),
            }],
        );
        assert!(!is_suppressed(&diag, &suppressions, Path::new(TEST_ROOT)));
    }

    #[test]
    fn test_abs_rel_mismatch_still_matches_same_file() {
        // Legitimate origin case: clippy emits an absolute path, the rule engine
        // a relative one, for the SAME file. Normalization bridges them so the
        // suppression still applies.
        let root = Path::new(TEST_ROOT);
        let abs = root.join("src/main.rs");
        let diag = make_diag("src/main.rs", "rule-a", 5);
        let mut suppressions = HashMap::new();
        suppressions.insert(
            normalize_path(&abs, root),
            vec![Suppression {
                target_line: 5,
                rule: Some("rule-a".to_string()),
            }],
        );
        assert!(is_suppressed(&diag, &suppressions, root));
    }

    // --- apply_inline_suppressions with real files ---

    #[test]
    fn test_apply_with_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(
            &file_path,
            "// rust-doctor-disable-next-line test-rule\nlet x = 1;\nlet y = 2;\n",
        )
        .unwrap();

        let diags = vec![
            Diagnostic {
                file_path: file_path.clone(),
                rule: "test-rule".to_string(),
                category: Category::Style,
                severity: Severity::Warning,
                message: "test".to_string(),
                help: None,
                line: Some(2),
                column: None,
                fix: None,
            },
            Diagnostic {
                file_path,
                rule: "other-rule".to_string(),
                category: Category::Style,
                severity: Severity::Warning,
                message: "test".to_string(),
                help: None,
                line: Some(3),
                column: None,
                fix: None,
            },
        ];

        let (filtered, suppressed) = apply_inline_suppressions(diags, dir.path());
        assert_eq!(suppressed, 1);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].rule, "other-rule");
    }

    #[test]
    fn inline_reason_retains_the_matching_rule() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        std::fs::write(
            &path,
            "// rust-doctor-disable-next-line test-rule\nlet value = 1;\n",
        )
        .unwrap();
        let diagnostic = make_diag(path.to_str().unwrap(), "test-rule", 2);
        let reason = inline_suppression_reason(&diagnostic, dir.path()).unwrap();
        assert!(reason.contains("test-rule"));
        assert!(reason.contains("line 2"));
    }

    #[test]
    fn test_apply_homonym_workspace_no_cross_suppression() {
        // Regression for US-011: two workspace members each own a `src/main.rs`.
        // A suppression comment lives only in `crateB`. The homonymous diagnostic
        // in `crateA` (carrying the shorter member-relative path the rule engine
        // emits) must survive. On pre-US-011 code the bidirectional `ends_with`
        // suppressed it too, so `suppressed` was 2 and this assertion failed.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let crate_b_src = root.join("crateB/src");
        std::fs::create_dir_all(&crate_b_src).unwrap();
        std::fs::write(
            crate_b_src.join("main.rs"),
            "// rust-doctor-disable-next-line test-rule\nfn b() {}\n",
        )
        .unwrap();

        // crateA's diagnostic uses the short, member-relative path; it must survive.
        let surviving_diag = make_diag("src/main.rs", "test-rule", 2);
        // crateB's own diagnostic — legitimately suppressed by its own comment.
        let suppressed_diag = make_diag("crateB/src/main.rs", "test-rule", 2);

        let (filtered, suppressed) =
            apply_inline_suppressions(vec![surviving_diag, suppressed_diag], root);

        assert_eq!(
            suppressed, 1,
            "only crateB's own diagnostic must be suppressed"
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].file_path, PathBuf::from("src/main.rs"));
    }

    #[test]
    fn test_out_of_root_file_not_read() {
        // Fail-closed scope guard: a diagnostic pointing at a file OUTSIDE the
        // project root must not have its suppressions read, so a `disable`
        // comment planted out-of-tree cannot neutralize an in-scope diagnostic.
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("evil.rs");
        std::fs::write(
            &outside_file,
            "// rust-doctor-disable-next-line test-rule\nfn x() {}\n",
        )
        .unwrap();

        let diag = Diagnostic {
            file_path: outside_file,
            rule: "test-rule".to_string(),
            category: Category::Style,
            severity: Severity::Warning,
            message: "test".to_string(),
            help: None,
            line: Some(2),
            column: None,
            fix: None,
        };

        let (filtered, suppressed) = apply_inline_suppressions(vec![diag], root.path());
        assert_eq!(
            suppressed, 0,
            "an out-of-root suppression comment must not apply"
        );
        assert_eq!(filtered.len(), 1);
    }

    // --- find_line_comment_start ---

    #[test]
    fn test_find_comment_in_normal_code() {
        assert_eq!(find_line_comment_start("let x = 1; // comment"), Some(11));
    }

    #[test]
    fn test_find_comment_ignores_string_literal() {
        // The // inside "https://example.com" should NOT be found as a comment
        assert_eq!(
            find_line_comment_start(r#"let url = "https://example.com"; // real comment"#),
            Some(33)
        );
    }

    #[test]
    fn test_find_comment_only_string_no_comment() {
        assert_eq!(
            find_line_comment_start(r#"let url = "https://example.com";"#),
            None
        );
    }

    #[test]
    fn test_find_comment_no_comment_at_all() {
        assert_eq!(find_line_comment_start("let x = 1;"), None);
    }

    #[test]
    fn test_suppression_not_triggered_by_string_literal() {
        // A string literal containing "// rust-doctor-disable-line" should NOT create a suppression
        let content = r#"let msg = "see // rust-doctor-disable-line for details";"#;
        let supps = parse_suppressions(content);
        assert!(supps.is_empty());
    }
}
