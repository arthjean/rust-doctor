use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::rules::CustomRule;
use std::path::Path;
use syn::spanned::Spanned;
use syn::visit::Visit;

/// Flags `Vec::new()` inside loop bodies without pre-allocation hint.
pub struct UnnecessaryAllocation;

impl CustomRule for UnnecessaryAllocation {
    fn name(&self) -> &'static str {
        "unnecessary-allocation"
    }
    fn category(&self) -> Category {
        Category::Performance
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &'static str {
        "Flags `Vec::new()` or `String::new()` inside loops. Each iteration allocates a new buffer, which is expensive."
    }
    fn fix_hint(&self) -> &'static str {
        "Move the allocation outside the loop and use `.clear()` to reuse it."
    }
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        let mut visitor = AllocVisitor {
            path,
            diagnostics: Vec::new(),
            in_loop: false,
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

struct AllocVisitor<'a> {
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
    in_loop: bool,
}

impl<'ast> Visit<'ast> for AllocVisitor<'_> {
    fn visit_expr_for_loop(&mut self, i: &'ast syn::ExprForLoop) {
        let was_in_loop = self.in_loop;
        self.in_loop = true;
        syn::visit::visit_expr_for_loop(self, i);
        self.in_loop = was_in_loop;
    }

    fn visit_expr_while(&mut self, i: &'ast syn::ExprWhile) {
        let was_in_loop = self.in_loop;
        self.in_loop = true;
        syn::visit::visit_expr_while(self, i);
        self.in_loop = was_in_loop;
    }

    fn visit_expr_loop(&mut self, i: &'ast syn::ExprLoop) {
        let was_in_loop = self.in_loop;
        self.in_loop = true;
        syn::visit::visit_expr_loop(self, i);
        self.in_loop = was_in_loop;
    }

    fn visit_expr_call(&mut self, i: &'ast syn::ExprCall) {
        if self.in_loop
            && let syn::Expr::Path(func_path) = i.func.as_ref()
        {
            let segments: Vec<String> = func_path
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            if (segments == ["Vec", "new"] || segments == ["String", "new"]) && i.args.is_empty() {
                let span = func_path.path.span();
                self.diagnostics.push(Diagnostic {
                    file_path: self.path.to_path_buf(),
                    rule: "unnecessary-allocation".to_string(),
                    category: Category::Performance,
                    severity: Severity::Warning,
                    message: format!(
                        "{}::new() inside a loop — allocates on every iteration",
                        segments.join("::")
                    ),
                    help: Some(
                        "Move the allocation outside the loop and use .clear() to reuse, or use Vec::with_capacity()"
                            .to_string(),
                    ),
                    line: Some(span.start().line as u32),
                    column: Some(span.start().column as u32 + 1),
                    fix: None,
                });
            }
        }
        syn::visit::visit_expr_call(self, i);
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
    fn test_vec_new_in_loop_detected() {
        let diags = check(
            &UnnecessaryAllocation,
            r"
            fn main() {
                for _ in 0..10 {
                    let v = Vec::new();
                }
            }
            ",
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "unnecessary-allocation");
    }

    #[test]
    fn test_vec_new_outside_loop_not_flagged() {
        let diags = check(
            &UnnecessaryAllocation,
            r"
            fn main() {
                let v = Vec::new();
            }
            ",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_string_new_in_while_loop_detected() {
        let diags = check(
            &UnnecessaryAllocation,
            r"
            fn main() {
                while true {
                    let s = String::new();
                }
            }
            ",
        );
        assert_eq!(diags.len(), 1);
    }
}
