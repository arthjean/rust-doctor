use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::rules::{CustomRule, has_cfg_test, has_test_attr, is_test_context};
use std::path::Path;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Item, ReturnType, Type};

// ─── Rule 1: unwrap-in-production ───────────────────────────────────────────

/// Flags `.unwrap()` and `.expect()` calls outside of test code.
pub struct UnwrapInProduction;

impl CustomRule for UnwrapInProduction {
    fn name(&self) -> &'static str {
        "unwrap-in-production"
    }
    fn category(&self) -> Category {
        Category::ErrorHandling
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &'static str {
        "Flags `.unwrap()` and `.expect()` calls outside of test code. These calls panic at runtime if the value is `None` or `Err`, crashing your application."
    }
    fn fix_hint(&self) -> &'static str {
        "Use the `?` operator to propagate errors, or handle them with `match`, `if let`, `.unwrap_or()`, or `.unwrap_or_else()`."
    }
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        let mut visitor = UnwrapVisitor {
            path,
            diagnostics: Vec::new(),
            in_test: false,
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

struct UnwrapVisitor<'a> {
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
    in_test: bool,
}

impl<'ast> Visit<'ast> for UnwrapVisitor<'_> {
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let was_in_test = self.in_test;
        if is_test_context(&i.attrs) {
            self.in_test = true;
        }
        syn::visit::visit_item_fn(self, i);
        self.in_test = was_in_test;
    }

    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        if has_cfg_test(&i.attrs) {
            return; // Skip entire #[cfg(test)] module
        }
        syn::visit::visit_item_mod(self, i);
    }

    fn visit_expr_method_call(&mut self, i: &'ast syn::ExprMethodCall) {
        if !self.in_test {
            let method = i.method.to_string();
            if method == "unwrap" || method == "expect" {
                let span = i.method.span();
                self.diagnostics.push(Diagnostic {
                    file_path: self.path.to_path_buf(),
                    rule: "unwrap-in-production".to_string(),
                    category: Category::ErrorHandling,
                    severity: Severity::Warning,
                    message: format!("Use of .{method}() in production code"),
                    help: Some(
                        "Use `?` operator, `unwrap_or`, `unwrap_or_else`, or handle the error explicitly"
                            .to_string(),
                    ),
                    line: Some(span.start().line as u32),
                    column: Some(span.start().column as u32 + 1),
                    fix: None,
                });
            }
        }
        syn::visit::visit_expr_method_call(self, i);
    }
}

// ─── Rule 2: panic-in-library ───────────────────────────────────────────────

/// Flags `panic!()`, `todo!()`, `unimplemented!()` in library crates.
/// Only fires if the file appears to be a library (lib.rs or no main fn).
pub struct PanicInLibrary;

impl CustomRule for PanicInLibrary {
    fn name(&self) -> &'static str {
        "panic-in-library"
    }
    fn category(&self) -> Category {
        Category::ErrorHandling
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &'static str {
        "Flags `panic!()`, `todo!()`, and `unimplemented!()` macros in library code. Libraries should return errors rather than panicking, since callers cannot recover from a panic across crate boundaries."
    }
    fn fix_hint(&self) -> &'static str {
        "Return `Result<T, E>` or `Option<T>` instead of panicking."
    }
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        // Only check library files (not main.rs or bin/*.rs)
        let filename = path.file_name().unwrap_or_default().to_string_lossy();
        if filename == "main.rs" || path.components().any(|c| c.as_os_str() == "bin") {
            return vec![];
        }

        let mut visitor = PanicVisitor {
            path,
            diagnostics: Vec::new(),
            in_test: false,
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

struct PanicVisitor<'a> {
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
    in_test: bool,
}

impl<'ast> Visit<'ast> for PanicVisitor<'_> {
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let was_in_test = self.in_test;
        if is_test_context(&i.attrs) {
            self.in_test = true;
        }
        syn::visit::visit_item_fn(self, i);
        self.in_test = was_in_test;
    }

    fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
        if has_cfg_test(&i.attrs) {
            return;
        }
        syn::visit::visit_item_mod(self, i);
    }

    fn visit_macro(&mut self, i: &'ast syn::Macro) {
        if !self.in_test {
            let macro_name = i.path.segments.last().map(|s| s.ident.to_string());
            if let Some(name) = macro_name
                && matches!(name.as_str(), "panic" | "todo" | "unimplemented")
            {
                let span = i.path.span();
                self.diagnostics.push(Diagnostic {
                    file_path: self.path.to_path_buf(),
                    rule: "panic-in-library".to_string(),
                    category: Category::ErrorHandling,
                    severity: Severity::Error,
                    message: format!("{name}!() in library code can crash callers"),
                    help: Some("Return a Result or Option instead of panicking".to_string()),
                    line: Some(span.start().line as u32),
                    column: Some(span.start().column as u32 + 1),
                    fix: None,
                });
            }
        }
        syn::visit::visit_macro(self, i);
    }
}

// ─── Rule 3: box-dyn-error-in-public-api ────────────────────────────────────

/// Flags public functions returning `Box<dyn Error>`.
pub struct BoxDynErrorInPublicApi;

impl CustomRule for BoxDynErrorInPublicApi {
    fn name(&self) -> &'static str {
        "box-dyn-error-in-public-api"
    }
    fn category(&self) -> Category {
        Category::ErrorHandling
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &'static str {
        "Flags `pub fn` returning `Result<_, Box<dyn Error>>`. This erases error type information, making it impossible for callers to match on specific error variants."
    }
    fn fix_hint(&self) -> &'static str {
        "Define a custom error enum with `thiserror` or return a concrete error type."
    }
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for item in &syntax.items {
            match item {
                Item::Fn(func) if is_pub(&func.vis) && !has_test_attr(&func.attrs) => {
                    check_return_type_box_dyn_error(
                        &func.sig.output,
                        &func.sig.ident,
                        path,
                        &mut diagnostics,
                    );
                }
                Item::Impl(imp) => {
                    for impl_item in &imp.items {
                        if let syn::ImplItem::Fn(method) = impl_item
                            && is_pub(&method.vis)
                            && !has_test_attr(&method.attrs)
                        {
                            check_return_type_box_dyn_error(
                                &method.sig.output,
                                &method.sig.ident,
                                path,
                                &mut diagnostics,
                            );
                        }
                    }
                }
                _ => {}
            }
        }

        diagnostics
    }
}

const fn is_pub(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

fn check_return_type_box_dyn_error(
    output: &ReturnType,
    ident: &syn::Ident,
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let ReturnType::Type(_, ty) = output else {
        return;
    };

    if return_type_contains_box_dyn_error(ty) {
        let span = ident.span();
        diagnostics.push(Diagnostic {
            file_path: path.to_path_buf(),
            rule: "box-dyn-error-in-public-api".to_string(),
            category: Category::ErrorHandling,
            severity: Severity::Warning,
            message: format!(
                "Public function `{ident}` returns `Box<dyn Error>` — callers cannot match on error variants"
            ),
            help: Some(
                "Define a custom error enum with `thiserror` or use `anyhow::Error` for applications"
                    .to_string(),
            ),
            line: Some(span.start().line as u32),
            column: Some(span.start().column as u32 + 1),
            fix: None,
        });
    }
}

/// Check if a type is or contains `Box<dyn Error>` / `Box<dyn std::error::Error>`.
fn return_type_contains_box_dyn_error(ty: &Type) -> bool {
    match ty {
        Type::Path(type_path) => {
            // Check for Result<T, Box<dyn Error>>
            let last_seg = type_path.path.segments.last();
            if let Some(seg) = last_seg {
                if seg.ident == "Result"
                    && let syn::PathArguments::AngleBracketed(args) = &seg.arguments
                    && let Some(syn::GenericArgument::Type(err_ty)) = args.args.iter().nth(1)
                {
                    return is_box_dyn_error(err_ty);
                }
                // Direct Box<dyn Error> return
                if is_box_dyn_error(ty) {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

fn is_box_dyn_error(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    let Some(seg) = type_path.path.segments.last() else {
        return false;
    };
    if seg.ident != "Box" {
        return false;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return false;
    };
    args.args.iter().any(|arg| {
        if let syn::GenericArgument::Type(Type::TraitObject(trait_obj)) = arg {
            trait_obj.bounds.iter().any(|bound| {
                if let syn::TypeParamBound::Trait(trait_bound) = bound {
                    let path_str = trait_bound
                        .path
                        .segments
                        .iter()
                        .map(|s| s.ident.to_string())
                        .collect::<Vec<_>>()
                        .join("::");
                    path_str == "Error"
                        || path_str == "std::error::Error"
                        || path_str == "error::Error"
                } else {
                    false
                }
            })
        } else {
            false
        }
    })
}

// ─── Rule 4: result-unit-error ──────────────────────────────────────────────

/// Flags public functions returning `Result<T, ()>`.
pub struct ResultUnitError;

impl CustomRule for ResultUnitError {
    fn name(&self) -> &'static str {
        "result-unit-error"
    }
    fn category(&self) -> Category {
        Category::ErrorHandling
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &'static str {
        "Flags `pub fn` returning `Result<_, ()>`. A unit error carries no information about what went wrong."
    }
    fn fix_hint(&self) -> &'static str {
        "Use a meaningful error type that describes the failure."
    }
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for item in &syntax.items {
            match item {
                Item::Fn(func) if is_pub(&func.vis) && !has_test_attr(&func.attrs) => {
                    check_result_unit_error(
                        &func.sig.output,
                        &func.sig.ident,
                        path,
                        &mut diagnostics,
                    );
                }
                Item::Impl(imp) => {
                    for impl_item in &imp.items {
                        if let syn::ImplItem::Fn(method) = impl_item
                            && is_pub(&method.vis)
                            && !has_test_attr(&method.attrs)
                        {
                            check_result_unit_error(
                                &method.sig.output,
                                &method.sig.ident,
                                path,
                                &mut diagnostics,
                            );
                        }
                    }
                }
                _ => {}
            }
        }

        diagnostics
    }
}

fn check_result_unit_error(
    output: &ReturnType,
    ident: &syn::Ident,
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let ReturnType::Type(_, ty) = output else {
        return;
    };
    let Type::Path(type_path) = ty.as_ref() else {
        return;
    };
    let Some(seg) = type_path.path.segments.last() else {
        return;
    };
    if seg.ident != "Result" {
        return;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return;
    };
    // Check if second generic arg is `()`
    if let Some(syn::GenericArgument::Type(Type::Tuple(tuple))) = args.args.iter().nth(1)
        && tuple.elems.is_empty()
    {
        let span = ident.span();
        diagnostics.push(Diagnostic {
            file_path: path.to_path_buf(),
            rule: "result-unit-error".to_string(),
            category: Category::ErrorHandling,
            severity: Severity::Warning,
            message: format!(
                "Public function `{ident}` returns `Result<_, ()>` — callers have no error context"
            ),
            help: Some(
                "Define a meaningful error type or use `anyhow::Error` for ad-hoc errors"
                    .to_string(),
            ),
            line: Some(span.start().line as u32),
            column: Some(span.start().column as u32 + 1),
            fix: None,
        });
    }
}

// ─── Convenience: create all error handling rules ───────────────────────────

/// Returns all error handling rules.
pub fn all_rules() -> Vec<Box<dyn CustomRule>> {
    vec![
        Box::new(UnwrapInProduction),
        Box::new(PanicInLibrary),
        Box::new(BoxDynErrorInPublicApi),
        Box::new(ResultUnitError),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(rule: &dyn CustomRule, code: &str) -> Vec<Diagnostic> {
        let syntax = syn::parse_file(code).expect("test code should parse");
        rule.check_file(&syntax, Path::new("test.rs"))
    }

    fn parse_and_check_lib(rule: &dyn CustomRule, code: &str) -> Vec<Diagnostic> {
        let syntax = syn::parse_file(code).expect("test code should parse");
        rule.check_file(&syntax, Path::new("src/lib.rs"))
    }

    // --- unwrap-in-production ---

    #[test]
    fn test_unwrap_detected() {
        let diags = parse_and_check(
            &UnwrapInProduction,
            r"
            fn main() {
                let x: Option<i32> = Some(1);
                x.unwrap();
            }
            ",
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "unwrap-in-production");
        assert!(diags[0].message.contains(".unwrap()"));
    }

    #[test]
    fn test_expect_detected() {
        let diags = parse_and_check(
            &UnwrapInProduction,
            r#"
            fn main() {
                let x: Result<i32, &str> = Ok(1);
                x.expect("should work");
            }
            "#,
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains(".expect()"));
    }

    #[test]
    fn test_unwrap_in_test_function_skipped() {
        let diags = parse_and_check(
            &UnwrapInProduction,
            r"
            #[test]
            fn my_test() {
                let x: Option<i32> = Some(1);
                x.unwrap();
            }
            ",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_unwrap_in_cfg_test_module_skipped() {
        let diags = parse_and_check(
            &UnwrapInProduction,
            r"
            #[cfg(test)]
            mod tests {
                fn helper() {
                    let x: Option<i32> = Some(1);
                    x.unwrap();
                }
            }
            ",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_unwrap_no_false_positive_on_other_methods() {
        let diags = parse_and_check(
            &UnwrapInProduction,
            r"
            fn main() {
                let x = vec![1, 2, 3];
                x.len();
                x.is_empty();
            }
            ",
        );
        assert!(diags.is_empty());
    }

    // --- panic-in-library ---

    #[test]
    fn test_panic_in_library_detected() {
        let diags = parse_and_check_lib(
            &PanicInLibrary,
            r#"
            pub fn do_stuff() {
                panic!("something went wrong");
            }
            "#,
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "panic-in-library");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn test_todo_in_library_detected() {
        let diags = parse_and_check_lib(
            &PanicInLibrary,
            r"
            pub fn not_ready() {
                todo!();
            }
            ",
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("todo!()"));
    }

    #[test]
    fn test_panic_in_main_rs_skipped() {
        let rule = PanicInLibrary;
        let syntax = syn::parse_file("fn main() { panic!(\"oops\"); }").unwrap();
        let diags = rule.check_file(&syntax, Path::new("src/main.rs"));
        assert!(diags.is_empty());
    }

    #[test]
    fn test_panic_in_test_function_skipped() {
        let diags = parse_and_check_lib(
            &PanicInLibrary,
            r#"
            #[test]
            fn my_test() {
                panic!("expected");
            }
            "#,
        );
        assert!(diags.is_empty());
    }

    // --- box-dyn-error-in-public-api ---

    #[test]
    fn test_box_dyn_error_detected() {
        let diags = parse_and_check(
            &BoxDynErrorInPublicApi,
            r"
            use std::error::Error;
            pub fn do_thing() -> Result<(), Box<dyn Error>> {
                Ok(())
            }
            ",
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "box-dyn-error-in-public-api");
    }

    #[test]
    fn test_box_dyn_error_private_fn_skipped() {
        let diags = parse_and_check(
            &BoxDynErrorInPublicApi,
            r"
            use std::error::Error;
            fn do_thing() -> Result<(), Box<dyn Error>> {
                Ok(())
            }
            ",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_box_dyn_error_custom_error_ok() {
        let diags = parse_and_check(
            &BoxDynErrorInPublicApi,
            r"
            pub fn do_thing() -> Result<(), MyError> {
                Ok(())
            }
            ",
        );
        assert!(diags.is_empty());
    }

    // --- result-unit-error ---

    #[test]
    fn test_result_unit_error_detected() {
        let diags = parse_and_check(
            &ResultUnitError,
            r"
            pub fn do_thing() -> Result<i32, ()> {
                Ok(42)
            }
            ",
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "result-unit-error");
    }

    #[test]
    fn test_result_unit_error_private_fn_skipped() {
        let diags = parse_and_check(
            &ResultUnitError,
            r"
            fn do_thing() -> Result<i32, ()> {
                Ok(42)
            }
            ",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_result_proper_error_ok() {
        let diags = parse_and_check(
            &ResultUnitError,
            r"
            pub fn do_thing() -> Result<i32, String> {
                Ok(42)
            }
            ",
        );
        assert!(diags.is_empty());
    }

    // --- all_rules ---

    #[test]
    fn test_all_rules_returns_4() {
        assert_eq!(all_rules().len(), 4);
    }
}
