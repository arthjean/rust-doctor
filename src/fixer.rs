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
