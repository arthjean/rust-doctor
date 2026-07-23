use crate::catalog::NumericRange;
use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::rules::{CustomRule, has_cfg_test, has_test_attr};
use std::path::Path;
use syn::visit::Visit;

/// Flags excessive `.clone()` calls as a review prompt.
/// Without type information, this rule cannot distinguish Copy-type clones
/// from necessary non-Copy clones. It uses heuristics to reduce false positives:
///
/// - Ignores clones inside test functions and `#[cfg(test)]` modules
/// - Only reports when a file has 3+ clone calls (isolated clones are usually intentional)
/// - Uses `clippy::clone_on_copy` (in the clippy pass) for precise Copy-type detection
///
/// Suppress with `// rust-doctor-disable-next-line excessive-clone` for reviewed clones.
pub struct ExcessiveClone {
    threshold: usize,
}

/// Minimum number of clone calls in a file before reporting.
const CLONE_THRESHOLD: usize = 3;

impl Default for ExcessiveClone {
    fn default() -> Self {
        Self {
            threshold: CLONE_THRESHOLD,
        }
    }
}

impl CustomRule for ExcessiveClone {
    fn name(&self) -> &'static str {
        "excessive-clone"
    }
    fn category(&self) -> Category {
        Category::Performance
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &'static str {
        "Flags `.clone()` calls that may indicate unnecessary heap allocations. Each clone copies the entire value, which is expensive for `String`, `Vec`, and other heap-allocated types."
    }
    fn fix_hint(&self) -> &'static str {
        "Use references (`&T`) or `Cow<T>` instead of cloning. Consider restructuring ownership to avoid the clone."
    }
    fn supported_threshold(&self) -> Option<NumericRange> {
        Some(NumericRange {
            min: 1,
            max: 100,
            default: CLONE_THRESHOLD as u32,
        })
    }
    fn set_threshold(&mut self, threshold: u32) {
        self.threshold = threshold as usize;
    }
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        let mut visitor = CloneVisitor {
            path,
            diagnostics: Vec::new(),
            in_test: false,
            loop_depth: 0,
        };
        visitor.visit_file(syntax);
        // Only report if the file has enough clones to suggest a pattern issue
        if visitor.diagnostics.len() >= self.threshold {
            visitor.diagnostics
        } else {
            Vec::new()
        }
    }
}

struct CloneVisitor<'a> {
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
    in_test: bool,
    /// Nesting depth of loops (for/while/loop). Clones inside loops are hot-path.
    loop_depth: u32,
}

impl<'ast> Visit<'ast> for CloneVisitor<'_> {
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let was_in_test = self.in_test;
        if has_test_attr(&i.attrs) {
            self.in_test = true;
        }
        syn::visit::visit_item_fn(self, i);
        self.in_test = was_in_test;
    }

    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        if has_cfg_test(&i.attrs) {
            return; // Skip entire #[cfg(test)] modules
        }
        syn::visit::visit_item_mod(self, i);
    }

    fn visit_expr_for_loop(&mut self, i: &'ast syn::ExprForLoop) {
        self.loop_depth += 1;
        syn::visit::visit_expr_for_loop(self, i);
        self.loop_depth -= 1;
    }

    fn visit_expr_while(&mut self, i: &'ast syn::ExprWhile) {
        self.loop_depth += 1;
        syn::visit::visit_expr_while(self, i);
        self.loop_depth -= 1;
    }

    fn visit_expr_loop(&mut self, i: &'ast syn::ExprLoop) {
        self.loop_depth += 1;
        syn::visit::visit_expr_loop(self, i);
        self.loop_depth -= 1;
    }

    fn visit_expr_method_call(&mut self, i: &'ast syn::ExprMethodCall) {
        if !self.in_test && i.method == "clone" && i.args.is_empty() {
            let span = i.method.span();
            let in_loop = self.loop_depth > 0;
            let (severity, message) = if in_loop {
                (
                    Severity::Warning,
                    "`.clone()` inside a loop — may cause repeated heap allocations".to_string(),
                )
            } else {
                (
                    Severity::Info,
                    "`.clone()` in non-loop context (cold path)".to_string(),
                )
            };
            self.diagnostics.push(Diagnostic {
                file_path: self.path.to_path_buf(),
                rule: "excessive-clone".to_string(),
                category: Category::Performance,
                severity,
                message,
                help: Some(
                    "If the type implements Copy, remove .clone(). Otherwise, consider borrowing or using Cow<T>".to_string(),
                ),
                line: Some(span.start().line as u32),
                column: Some(span.start().column as u32 + 1),
                fix: None,
            });
        }
        syn::visit::visit_expr_method_call(self, i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(rule: &dyn CustomRule, code: &str) -> Vec<Diagnostic> {
        let syntax = syn::parse_file(code).expect("test code should parse");
        rule.check_file(&syntax, Path::new("test.rs"))
    }

    #[test]
    fn test_clone_detected_above_threshold() {
        // 3+ clones in production code should trigger the rule
        let diags = check(
            &ExcessiveClone::default(),
            r"
            fn main() {
                let x = vec![1, 2, 3];
                let a = x.clone();
                let b = x.clone();
                let c = x.clone();
            }
            ",
        );
        assert_eq!(diags.len(), 3);
        assert_eq!(diags[0].rule, "excessive-clone");
    }

    #[test]
    fn test_clone_below_threshold_not_reported() {
        // 1-2 clones are considered intentional and not reported
        let diags = check(
            &ExcessiveClone::default(),
            r"
            fn main() {
                let x = vec![1, 2, 3];
                let y = x.clone();
            }
            ",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_clone_in_test_code_ignored() {
        let diags = check(
            &ExcessiveClone::default(),
            r"
            #[test]
            fn test_something() {
                let x = vec![1, 2, 3];
                let a = x.clone();
                let b = x.clone();
                let c = x.clone();
                let d = x.clone();
            }
            ",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_clone_in_cfg_test_module_ignored() {
        let diags = check(
            &ExcessiveClone::default(),
            r"
            #[cfg(test)]
            mod tests {
                fn helper() {
                    let x = vec![1];
                    let a = x.clone();
                    let b = x.clone();
                    let c = x.clone();
                }
            }
            ",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_clone_no_false_positive_on_other_methods() {
        let diags = check(
            &ExcessiveClone::default(),
            r"
            fn main() {
                let x = vec![1, 2, 3];
                let y = x.len();
            }
            ",
        );
        assert!(diags.is_empty());
    }
}
