use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::rules::CustomRule;
use std::path::Path;
use syn::visit::Visit;

/// Flags `.collect::<Vec<_>>()` immediately followed by `.iter()` or `.into_iter()`.
pub struct CollectThenIterate;

impl CustomRule for CollectThenIterate {
    fn name(&self) -> &'static str {
        "collect-then-iterate"
    }
    fn category(&self) -> Category {
        Category::Performance
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &'static str {
        "Flags `.collect::<Vec<_>>()` immediately followed by `.iter()`. This allocates a temporary vector unnecessarily since the original iterator could be used directly."
    }
    fn fix_hint(&self) -> &'static str {
        "Remove the `.collect()` and chain the iterator operations directly."
    }
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        let mut visitor = CollectIterVisitor {
            path,
            diagnostics: Vec::new(),
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

struct CollectIterVisitor<'a> {
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
}

impl<'ast> Visit<'ast> for CollectIterVisitor<'_> {
    fn visit_expr_method_call(&mut self, i: &'ast syn::ExprMethodCall) {
        // Check if this is .iter() or .into_iter() called on a .collect() result
        let method_name = i.method.to_string();
        if (method_name == "iter" || method_name == "into_iter")
            && i.args.is_empty()
            && is_collect_call(&i.receiver)
        {
            let span = i.method.span();
            self.diagnostics.push(Diagnostic {
                file_path: self.path.to_path_buf(),
                rule: "collect-then-iterate".to_string(),
                category: Category::Performance,
                severity: Severity::Warning,
                message: ".collect() immediately followed by .iter() — unnecessary allocation"
                    .to_string(),
                help: Some(
                    "Remove the .collect() call and continue chaining iterator adaptors directly"
                        .to_string(),
                ),
                line: Some(span.start().line as u32),
                column: Some(span.start().column as u32 + 1),
                fix: None,
            });
        }
        syn::visit::visit_expr_method_call(self, i);
    }
}

fn is_collect_call(expr: &syn::Expr) -> bool {
    if let syn::Expr::MethodCall(call) = expr {
        call.method == "collect"
    } else {
        false
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
    fn test_collect_then_iter_detected() {
        let diags = check(
            &CollectThenIterate,
            r"
            fn main() {
                let v: Vec<i32> = (0..10).collect::<Vec<_>>().iter().map(|x| x + 1).collect();
            }
            ",
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "collect-then-iterate");
    }

    #[test]
    fn test_collect_without_iter_not_flagged() {
        let diags = check(
            &CollectThenIterate,
            r"
            fn main() {
                let v: Vec<i32> = (0..10).collect();
            }
            ",
        );
        assert!(diags.is_empty());
    }
}
