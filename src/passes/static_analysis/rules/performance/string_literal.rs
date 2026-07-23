use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::rules::{CustomRule, has_cfg_test, has_test_attr};
use std::path::Path;
use syn::spanned::Spanned;
use syn::visit::Visit;

/// Flags `String::from("literal")` and `"literal".to_string()` patterns.
pub struct StringFromLiteral;

impl CustomRule for StringFromLiteral {
    fn name(&self) -> &'static str {
        "string-from-literal"
    }
    fn category(&self) -> Category {
        Category::Performance
    }
    fn severity(&self) -> Severity {
        Severity::Info
    }
    fn default_enabled(&self) -> bool {
        false
    }
    fn description(&self) -> &'static str {
        "Flags `String::from(\"literal\")` and `\"literal\".to_string()`. These allocate on the heap when a `&str` reference might suffice."
    }
    fn fix_hint(&self) -> &'static str {
        "Use `&str` for function parameters and constants. Owned `String` is correct for struct fields, HashMap keys, error messages, and APIs requiring ownership."
    }
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        let mut visitor = StringLiteralVisitor {
            path,
            diagnostics: Vec::new(),
            in_test: false,
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

struct StringLiteralVisitor<'a> {
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
    in_test: bool,
}

impl<'ast> Visit<'ast> for StringLiteralVisitor<'_> {
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

    fn visit_expr_method_call(&mut self, i: &'ast syn::ExprMethodCall) {
        // "literal".to_string()
        if !self.in_test
            && i.method == "to_string"
            && i.args.is_empty()
            && matches!(i.receiver.as_ref(), syn::Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Str(_)))
        {
            let span = i.method.span();
            self.diagnostics.push(Diagnostic {
                file_path: self.path.to_path_buf(),
                rule: "string-from-literal".to_string(),
                category: Category::Performance,
                severity: Severity::Info,
                message:
                    r#""literal".to_string() allocates — consider &str if ownership is not needed"#
                        .to_string(),
                help: Some(
                    "Acceptable for struct fields, HashMap keys, and APIs requiring String. \
                     Use &str for function parameters and constants."
                        .to_string(),
                ),
                line: Some(span.start().line as u32),
                column: Some(span.start().column as u32 + 1),
                fix: None,
            });
        }
        syn::visit::visit_expr_method_call(self, i);
    }

    fn visit_expr_call(&mut self, i: &'ast syn::ExprCall) {
        // String::from("literal")
        if !self.in_test
            && let syn::Expr::Path(func_path) = i.func.as_ref()
        {
            let segments: Vec<String> = func_path
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            if segments == ["String", "from"]
                && i.args.len() == 1
                && i.args.first().is_some_and(
                    |arg| matches!(arg, syn::Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Str(_))),
                )
            {
                let span = func_path.path.span();
                self.diagnostics.push(Diagnostic {
                    file_path: self.path.to_path_buf(),
                    rule: "string-from-literal".to_string(),
                    category: Category::Performance,
                    severity: Severity::Info,
                    message: r#"String::from("literal") allocates — consider &str if ownership is not needed"#.to_string(),
                    help: Some(
                        "Acceptable for struct fields, HashMap keys, and APIs requiring String. \
                         Use &str for function parameters and constants."
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
    fn test_string_from_literal_detected() {
        let diags = check(
            &StringFromLiteral,
            r#"
            fn main() {
                let s = String::from("hello");
            }
            "#,
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "string-from-literal");
    }

    #[test]
    fn test_to_string_on_literal_detected() {
        let diags = check(
            &StringFromLiteral,
            r#"
            fn main() {
                let s = "hello".to_string();
            }
            "#,
        );
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn test_string_from_variable_not_flagged() {
        let diags = check(
            &StringFromLiteral,
            r#"
            fn main() {
                let x = "hello";
                let s = String::from(x);
            }
            "#,
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_to_string_on_variable_not_flagged() {
        let diags = check(
            &StringFromLiteral,
            r"
            fn main() {
                let x = 42;
                let s = x.to_string();
            }
            ",
        );
        assert!(diags.is_empty());
    }
}
