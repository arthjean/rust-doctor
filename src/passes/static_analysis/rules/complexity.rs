use crate::catalog::NumericRange;
use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::rules::{CustomRule, has_cfg_test, has_test_attr};
use std::path::Path;
use syn::visit::Visit;

// ─── Rule: high-cyclomatic-complexity ────────────────────────────────────────

/// Flags functions with high cyclomatic complexity (> threshold).
/// Cyclomatic complexity measures the number of independent paths through a function.
/// High complexity makes code harder to test, understand, and maintain.
pub struct HighCyclomaticComplexity {
    threshold: u32,
}

/// Default complexity threshold above which a function is flagged.
const COMPLEXITY_THRESHOLD: u32 = 15;

impl Default for HighCyclomaticComplexity {
    fn default() -> Self {
        Self {
            threshold: COMPLEXITY_THRESHOLD,
        }
    }
}

impl CustomRule for HighCyclomaticComplexity {
    fn name(&self) -> &'static str {
        "high-cyclomatic-complexity"
    }
    fn category(&self) -> Category {
        Category::Architecture
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &'static str {
        "Function has high cyclomatic complexity — consider refactoring into smaller functions"
    }
    fn fix_hint(&self) -> &'static str {
        "Extract complex branches into helper functions, use early returns, simplify match arms"
    }
    fn supported_threshold(&self) -> Option<NumericRange> {
        Some(NumericRange {
            min: 5,
            max: 100,
            default: COMPLEXITY_THRESHOLD,
        })
    }
    fn set_threshold(&mut self, threshold: u32) {
        self.threshold = threshold;
    }
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        let mut visitor = ComplexityVisitor {
            path,
            diagnostics: Vec::new(),
            in_test: false,
            threshold: self.threshold,
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

struct ComplexityVisitor<'a> {
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
    in_test: bool,
    threshold: u32,
}

impl ComplexityVisitor<'_> {
    fn check_function_complexity(
        &mut self,
        fn_name: &str,
        block: &syn::Block,
        span: proc_macro2::Span,
    ) {
        if self.in_test {
            return;
        }
        let mut counter = ComplexityCounter { complexity: 1 };
        counter.visit_block(block);
        if counter.complexity > self.threshold {
            self.diagnostics.push(Diagnostic {
                file_path: self.path.to_path_buf(),
                rule: "high-cyclomatic-complexity".to_string(),
                category: Category::Architecture,
                severity: Severity::Warning,
                message: format!(
                    "Function `{}` has cyclomatic complexity of {} (threshold: {})",
                    fn_name, counter.complexity, self.threshold
                ),
                help: Some(
                    "Extract complex branches into helper functions, use early returns, simplify match arms"
                        .to_string(),
                ),
                line: Some(span.start().line as u32),
                column: Some(span.start().column as u32 + 1),
                fix: None,
            });
        }
    }
}

impl<'ast> Visit<'ast> for ComplexityVisitor<'_> {
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let was_in_test = self.in_test;
        if has_test_attr(&i.attrs) {
            self.in_test = true;
        }
        let span = i.sig.ident.span();
        let fn_name = i.sig.ident.to_string();
        self.check_function_complexity(&fn_name, &i.block, span);
        // Continue visiting nested items (but not the block again — we already counted it)
        for item in &i.block.stmts {
            if let syn::Stmt::Item(item) = item {
                syn::visit::visit_item(self, item);
            }
        }
        self.in_test = was_in_test;
    }

    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        let was_in_test = self.in_test;
        if has_test_attr(&i.attrs) {
            self.in_test = true;
        }
        let span = i.sig.ident.span();
        let fn_name = i.sig.ident.to_string();
        self.check_function_complexity(&fn_name, &i.block, span);
        // Continue visiting nested items
        for item in &i.block.stmts {
            if let syn::Stmt::Item(item) = item {
                syn::visit::visit_item(self, item);
            }
        }
        self.in_test = was_in_test;
    }

    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        if has_cfg_test(&i.attrs) {
            return;
        }
        syn::visit::visit_item_mod(self, i);
    }
}

/// Inner counter that walks a single function body and tallies complexity increments.
struct ComplexityCounter {
    complexity: u32,
}

impl<'ast> Visit<'ast> for ComplexityCounter {
    fn visit_expr_if(&mut self, i: &'ast syn::ExprIf) {
        self.complexity += 1;
        syn::visit::visit_expr_if(self, i);
    }

    fn visit_expr_match(&mut self, i: &'ast syn::ExprMatch) {
        // Each arm adds a path; subtract 1 because one arm is the base path
        let arms = i.arms.len() as u32;
        if arms > 1 {
            self.complexity += arms - 1;
        }
        syn::visit::visit_expr_match(self, i);
    }

    fn visit_expr_while(&mut self, i: &'ast syn::ExprWhile) {
        self.complexity += 1;
        syn::visit::visit_expr_while(self, i);
    }

    fn visit_expr_for_loop(&mut self, i: &'ast syn::ExprForLoop) {
        self.complexity += 1;
        syn::visit::visit_expr_for_loop(self, i);
    }

    fn visit_expr_loop(&mut self, i: &'ast syn::ExprLoop) {
        self.complexity += 1;
        syn::visit::visit_expr_loop(self, i);
    }

    fn visit_expr_binary(&mut self, i: &'ast syn::ExprBinary) {
        match i.op {
            syn::BinOp::And(_) | syn::BinOp::Or(_) => {
                self.complexity += 1;
            }
            _ => {}
        }
        syn::visit::visit_expr_binary(self, i);
    }

    fn visit_expr_try(&mut self, i: &'ast syn::ExprTry) {
        self.complexity += 1;
        syn::visit::visit_expr_try(self, i);
    }

    // Don't recurse into nested closures or async blocks — they are separate "functions"
    fn visit_expr_closure(&mut self, _i: &'ast syn::ExprClosure) {
        // Skip: closures have their own complexity
    }
}

// ─── Convenience ────────────────────────────────────────────────────────────

/// Returns the complexity rule.
pub fn all_rules() -> Vec<Box<dyn CustomRule>> {
    vec![Box::new(HighCyclomaticComplexity::default())]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(rule: &dyn CustomRule, code: &str) -> Vec<Diagnostic> {
        let syntax = syn::parse_file(code).expect("test code should parse");
        rule.check_file(&syntax, Path::new("test.rs"))
    }

    #[test]
    fn test_simple_function_no_complexity_diagnostic() {
        let diags = check(
            &HighCyclomaticComplexity::default(),
            r"
            fn simple() -> i32 {
                let x = 1 + 2;
                x * 3
            }
            ",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_complex_function_triggers_diagnostic() {
        // Build a function with complexity > 15:
        // base=1, 10 ifs=+10, 1 while=+1, 1 for=+1, 1 loop=+1, 2 &&=+2 = 16
        let diags = check(
            &HighCyclomaticComplexity::default(),
            r"
            fn very_complex(x: i32, y: bool, z: bool) {
                if x > 0 { }
                if x > 1 { }
                if x > 2 { }
                if x > 3 { }
                if x > 4 { }
                if x > 5 { }
                if x > 6 { }
                if x > 7 { }
                if x > 8 { }
                if x > 9 { }
                while x > 0 { }
                for _ in 0..10 { }
                loop { break; }
                if y && z { }
            }
            ",
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "high-cyclomatic-complexity");
        assert!(diags[0].message.contains("very_complex"));
        assert!(diags[0].message.contains("16"));
    }

    #[test]
    fn test_complex_test_function_skipped() {
        let diags = check(
            &HighCyclomaticComplexity::default(),
            r"
            #[test]
            fn test_complex(x: i32, y: bool, z: bool) {
                if x > 0 { }
                if x > 1 { }
                if x > 2 { }
                if x > 3 { }
                if x > 4 { }
                if x > 5 { }
                if x > 6 { }
                if x > 7 { }
                if x > 8 { }
                if x > 9 { }
                while x > 0 { }
                for _ in 0..10 { }
                loop { break; }
                if y && z { }
            }
            ",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_complexity_with_match_and_try() {
        // base=1, 8 ifs=+8, match with 5 arms=+4, 3 ?=+3 = 16
        let diags = check(
            &HighCyclomaticComplexity::default(),
            r#"
            fn complex_match(x: i32) -> Result<(), Box<dyn std::error::Error>> {
                if x > 0 { }
                if x > 1 { }
                if x > 2 { }
                if x > 3 { }
                if x > 4 { }
                if x > 5 { }
                if x > 6 { }
                if x > 7 { }
                match x {
                    0 => {},
                    1 => {},
                    2 => {},
                    3 => {},
                    _ => {},
                }
                let _a = "foo".parse::<i32>()?;
                let _b = "bar".parse::<i32>()?;
                let _c = "baz".parse::<i32>()?;
                Ok(())
            }
            "#,
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("16"));
    }

    #[test]
    fn test_all_rules_returns_1() {
        assert_eq!(all_rules().len(), 1);
    }

    #[test]
    fn configured_threshold_changes_detection() {
        let mut rule = HighCyclomaticComplexity::default();
        rule.set_threshold(5);
        let diagnostics = check(
            &rule,
            r"
            fn calibrated(x: i32) {
                if x > 0 {}
                if x > 1 {}
                if x > 2 {}
                if x > 3 {}
                if x > 4 {}
            }
            ",
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("threshold: 5"));
    }
}
