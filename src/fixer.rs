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

    for (file_path, fixes) in &fixes_by_file {
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
        sorted_fixes.sort_by_key(|b| std::cmp::Reverse(b.line));

        for fix in sorted_fixes {
            let line_idx = (fix.line as usize).saturating_sub(1);
            if let Some(line) = new_lines.get_mut(line_idx) {
                if line.contains(&fix.old_text) {
                    let replaced = line.replacen(&fix.old_text, &fix.new_text, 1);
                    *line = replaced;
                    applied_in_file += 1;
                }
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
}
