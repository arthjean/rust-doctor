use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::rules::{CustomRule, has_cfg_test, has_test_attr};
use std::path::Path;
use syn::spanned::Spanned;
use syn::visit::Visit;

// ─── Rule 1: hardcoded-secrets ──────────────────────────────────────────────

/// Flags string literals assigned to variables whose names match secret patterns.
pub struct HardcodedSecrets;

/// Regex-like patterns for secret variable names (case-insensitive match).
const SECRET_PATTERNS: &[&str] = &[
    "api_key",
    "apikey",
    "api_secret",
    "secret",
    "secret_key",
    "token",
    "password",
    "passwd",
    "credential",
    "auth_token",
    "access_key",
    "private_key",
];

/// Suffixes that indicate the variable is NOT a secret (e.g. `token_url`).
const NON_SECRET_SUFFIXES: &[&str] = &[
    "_url",
    "_path",
    "_name",
    "_type",
    "_label",
    "_mode",
    "_format",
    "_version",
    "_prefix",
    "_suffix",
    "_header",
    "_key_name",
    "_field",
];

fn is_secret_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Check non-secret suffixes first
    for suffix in NON_SECRET_SUFFIXES {
        if lower.ends_with(suffix) {
            return false;
        }
    }
    SECRET_PATTERNS.iter().any(|pat| lower.contains(pat))
}

impl CustomRule for HardcodedSecrets {
    fn name(&self) -> &'static str {
        "hardcoded-secrets"
    }
    fn category(&self) -> Category {
        Category::Security
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &'static str {
        "Flags string literals assigned to variables named `api_key`, `password`, `token`, `secret`, etc. (length > 8 chars). Hardcoded secrets in source code can be extracted from compiled binaries or version control."
    }
    fn fix_hint(&self) -> &'static str {
        "Use environment variables, a secrets manager, or config files excluded from version control."
    }
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        let mut visitor = SecretVisitor {
            path,
            diagnostics: Vec::new(),
            in_test: false,
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

struct SecretVisitor<'a> {
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
    in_test: bool,
}

impl<'ast> Visit<'ast> for SecretVisitor<'_> {
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
            return; // Skip entire #[cfg(test)] module
        }
        syn::visit::visit_item_mod(self, i);
    }

    fn visit_local(&mut self, i: &'ast syn::Local) {
        if !self.in_test
            && let Some(init) = &i.init
            && let syn::Expr::Lit(expr_lit) = init.expr.as_ref()
            && let syn::Lit::Str(lit_str) = &expr_lit.lit
            && lit_str.value().len() > 8
            && let syn::Pat::Ident(pat_ident) = &i.pat
        {
            let var_name = pat_ident.ident.to_string();
            if is_secret_name(&var_name) {
                let span = pat_ident.ident.span();
                self.diagnostics.push(Diagnostic {
                    file_path: self.path.to_path_buf(),
                    rule: "hardcoded-secrets".to_string(),
                    category: Category::Security,
                    severity: Severity::Error,
                    message: format!("Potential hardcoded secret in variable `{var_name}`"),
                    help: Some(
                        "Use environment variables (std::env::var) or a secrets manager instead of hardcoding credentials"
                            .to_string(),
                    ),
                    line: Some(span.start().line as u32),
                    column: Some(span.start().column as u32 + 1),
                    fix: None,
                });
            }
        }
        syn::visit::visit_local(self, i);
    }

    fn visit_expr_assign(&mut self, i: &'ast syn::ExprAssign) {
        if !self.in_test
            && let syn::Expr::Lit(expr_lit) = i.right.as_ref()
            && let syn::Lit::Str(lit_str) = &expr_lit.lit
            && lit_str.value().len() > 8
            && let Some(name) = extract_field_name(&i.left)
            && is_secret_name(&name)
        {
            let span = i.left.span();
            self.diagnostics.push(Diagnostic {
                file_path: self.path.to_path_buf(),
                rule: "hardcoded-secrets".to_string(),
                category: Category::Security,
                severity: Severity::Error,
                message: format!("Potential hardcoded secret in `{name}`"),
                help: Some(
                    "Use environment variables (std::env::var) or a secrets manager instead of hardcoding credentials"
                        .to_string(),
                ),
                line: Some(span.start().line as u32),
                column: Some(span.start().column as u32 + 1),
                fix: None,
            });
        }
        syn::visit::visit_expr_assign(self, i);
    }

    fn visit_item_const(&mut self, i: &'ast syn::ItemConst) {
        if !self.in_test
            && let syn::Expr::Lit(expr_lit) = i.expr.as_ref()
            && let syn::Lit::Str(lit_str) = &expr_lit.lit
            && lit_str.value().len() > 8
        {
            let var_name = i.ident.to_string();
            if is_secret_name(&var_name) {
                let span = i.ident.span();
                self.diagnostics.push(Diagnostic {
                    file_path: self.path.to_path_buf(),
                    rule: "hardcoded-secrets".to_string(),
                    category: Category::Security,
                    severity: Severity::Error,
                    message: format!("Potential hardcoded secret in const `{var_name}`"),
                    help: Some(
                        "Use environment variables (std::env::var) or a secrets manager instead of hardcoding credentials"
                            .to_string(),
                    ),
                    line: Some(span.start().line as u32),
                    column: Some(span.start().column as u32 + 1),
                    fix: None,
                });
            }
        }
        syn::visit::visit_item_const(self, i);
    }

    fn visit_item_static(&mut self, i: &'ast syn::ItemStatic) {
        if !self.in_test
            && let syn::Expr::Lit(expr_lit) = i.expr.as_ref()
            && let syn::Lit::Str(lit_str) = &expr_lit.lit
            && lit_str.value().len() > 8
        {
            let var_name = i.ident.to_string();
            if is_secret_name(&var_name) {
                let span = i.ident.span();
                self.diagnostics.push(Diagnostic {
                    file_path: self.path.to_path_buf(),
                    rule: "hardcoded-secrets".to_string(),
                    category: Category::Security,
                    severity: Severity::Error,
                    message: format!("Potential hardcoded secret in static `{var_name}`"),
                    help: Some(
                        "Use environment variables (std::env::var) or a secrets manager instead of hardcoding credentials"
                            .to_string(),
                    ),
                    line: Some(span.start().line as u32),
                    column: Some(span.start().column as u32 + 1),
                    fix: None,
                });
            }
        }
        syn::visit::visit_item_static(self, i);
    }
}

fn extract_field_name(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        syn::Expr::Field(f) => {
            if let syn::Member::Named(ident) = &f.member {
                Some(ident.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

// ─── Rule 2: unsafe-block-audit ─────────────────────────────────────────────

/// Reports all `unsafe` blocks with location info.
pub struct UnsafeBlockAudit;

impl CustomRule for UnsafeBlockAudit {
    fn name(&self) -> &'static str {
        "unsafe-block-audit"
    }
    fn category(&self) -> Category {
        Category::Security
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &'static str {
        "Flags `unsafe {}` blocks and `unsafe fn` declarations. Unsafe code bypasses Rust's memory safety guarantees and must be carefully audited."
    }
    fn fix_hint(&self) -> &'static str {
        "Verify the safety invariants are documented and correct. Consider safe abstractions or crates like `zerocopy` to eliminate unsafe."
    }
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        // Respect #![forbid(unsafe_code)] — if present, skip scanning
        for attr in &syntax.attrs {
            if attr.path().is_ident("forbid")
                && let Ok(ident) = attr.parse_args::<syn::Ident>()
                && ident == "unsafe_code"
            {
                return vec![];
            }
        }

        let mut visitor = UnsafeVisitor {
            path,
            diagnostics: Vec::new(),
            in_test: false,
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

struct UnsafeVisitor<'a> {
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
    in_test: bool,
}

impl<'ast> Visit<'ast> for UnsafeVisitor<'_> {
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        let was_in_test = self.in_test;
        if has_test_attr(&i.attrs) {
            self.in_test = true;
        }
        if !self.in_test && i.sig.unsafety.is_some() {
            let span = i.sig.ident.span();
            self.diagnostics.push(Diagnostic {
                file_path: self.path.to_path_buf(),
                rule: "unsafe-block-audit".to_string(),
                category: Category::Security,
                severity: Severity::Warning,
                message: format!("unsafe fn `{}` — review for memory safety", i.sig.ident),
                help: Some(
                    "Document the safety contract in the function's doc comment".to_string(),
                ),
                line: Some(span.start().line as u32),
                column: Some(span.start().column as u32 + 1),
                fix: None,
            });
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

    fn visit_expr_unsafe(&mut self, i: &'ast syn::ExprUnsafe) {
        if self.in_test {
            syn::visit::visit_expr_unsafe(self, i);
            return;
        }
        let span = i.unsafe_token.span();
        self.diagnostics.push(Diagnostic {
            file_path: self.path.to_path_buf(),
            rule: "unsafe-block-audit".to_string(),
            category: Category::Security,
            severity: Severity::Warning,
            message: "unsafe block — review for memory safety".to_string(),
            help: Some("Document the safety invariant with a // SAFETY: comment".to_string()),
            line: Some(span.start().line as u32),
            column: Some(span.start().column as u32 + 1),
            fix: None,
        });
        syn::visit::visit_expr_unsafe(self, i);
    }
}

// ─── Rule 3: sql-injection-risk ─────────────────────────────────────────────

/// Flags `format!()` used as argument to `.query()`, `.execute()`, or `.raw()`.
pub struct SqlInjectionRisk;

const SQL_METHODS: &[&str] = &["query", "execute", "raw", "query_as", "execute_raw"];

impl CustomRule for SqlInjectionRisk {
    fn name(&self) -> &'static str {
        "sql-injection-risk"
    }
    fn category(&self) -> Category {
        Category::Security
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &'static str {
        "Flags `format!()` output passed to `.query()`, `.execute()`, or `.raw()` methods. String interpolation in SQL queries enables SQL injection attacks."
    }
    fn fix_hint(&self) -> &'static str {
        "Use parameterized queries (`$1`, `?`) provided by your database library (sqlx, diesel, sea-orm)."
    }
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        let mut visitor = SqlVisitor {
            path,
            diagnostics: Vec::new(),
            in_test: false,
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

struct SqlVisitor<'a> {
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
    in_test: bool,
}

impl<'ast> Visit<'ast> for SqlVisitor<'_> {
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
            return; // Skip entire #[cfg(test)] module
        }
        syn::visit::visit_item_mod(self, i);
    }

    fn visit_expr_method_call(&mut self, i: &'ast syn::ExprMethodCall) {
        let method_name = i.method.to_string();
        if !self.in_test && SQL_METHODS.contains(&method_name.as_str()) {
            // Check if any argument is a format! or format_args! macro
            for arg in &i.args {
                if is_format_macro(arg) {
                    let span = i.method.span();
                    self.diagnostics.push(Diagnostic {
                        file_path: self.path.to_path_buf(),
                        rule: "sql-injection-risk".to_string(),
                        category: Category::Security,
                        severity: Severity::Error,
                        message: format!(
                            "format!() used in .{method_name}() — potential SQL injection"
                        ),
                        help: Some(
                            "Use parameterized queries with bind parameters ($1, ?) instead of string formatting"
                                .to_string(),
                        ),
                        line: Some(span.start().line as u32),
                        column: Some(span.start().column as u32 + 1),
                        fix: None,
                    });
                }
            }
        }
        syn::visit::visit_expr_method_call(self, i);
    }
}

fn is_format_macro(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Macro(m) => {
            let name = m.mac.path.segments.last().map(|s| s.ident.to_string());
            matches!(name.as_deref(), Some("format" | "format_args"))
        }
        // Also check for &format!(...) pattern
        syn::Expr::Reference(r) => is_format_macro(&r.expr),
        _ => false,
    }
}

// ─── Convenience ────────────────────────────────────────────────────────────

/// Returns all security rules.
pub fn all_rules() -> Vec<Box<dyn CustomRule>> {
    vec![
        Box::new(HardcodedSecrets),
        Box::new(UnsafeBlockAudit),
        Box::new(SqlInjectionRisk),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(rule: &dyn CustomRule, code: &str) -> Vec<Diagnostic> {
        let syntax = syn::parse_file(code).expect("test code should parse");
        rule.check_file(&syntax, Path::new("test.rs"))
    }

    // --- hardcoded-secrets ---

    #[test]
    fn test_hardcoded_secret_detected() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            fn main() {
                let api_key = "sk-1234567890abcdef";
            }
            "#,
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "hardcoded-secrets");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn test_hardcoded_password_detected() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            fn main() {
                let password = "super_secret_password_123";
            }
            "#,
        );
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn test_short_value_not_flagged() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            fn main() {
                let api_key = "short";
            }
            "#,
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_non_secret_variable_not_flagged() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            fn main() {
                let username = "some_long_username_value";
            }
            "#,
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_allowlist_suffix_not_flagged() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            fn main() {
                let token_url = "https://auth.example.com/oauth/token";
            }
            "#,
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_secret_name_detection() {
        assert!(is_secret_name("api_key"));
        assert!(is_secret_name("API_KEY"));
        assert!(is_secret_name("my_secret"));
        assert!(is_secret_name("auth_token"));
        assert!(is_secret_name("PASSWORD"));
        assert!(!is_secret_name("username"));
        assert!(!is_secret_name("token_url"));
        assert!(!is_secret_name("secret_path"));
        assert!(!is_secret_name("api_key_name"));
    }

    #[test]
    fn test_hardcoded_secret_in_const() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            const API_KEY: &str = "sk-1234567890abcdef";
            "#,
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("const"));
    }

    #[test]
    fn test_hardcoded_secret_in_static() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            static TOKEN: &str = "bearer-long-secret-token-value";
            "#,
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("static"));
    }

    // --- unsafe-block-audit ---

    #[test]
    fn test_unsafe_block_detected() {
        let diags = check(
            &UnsafeBlockAudit,
            r"
            fn main() {
                unsafe {
                    std::ptr::null::<i32>().read();
                }
            }
            ",
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "unsafe-block-audit");
        assert!(diags[0].message.contains("unsafe block"));
    }

    #[test]
    fn test_unsafe_fn_detected() {
        let diags = check(
            &UnsafeBlockAudit,
            r"
            unsafe fn dangerous() {
                // ...
            }
            ",
        );
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("unsafe fn"));
    }

    #[test]
    fn test_forbid_unsafe_code_skips_scan() {
        let diags = check(
            &UnsafeBlockAudit,
            r"
            #![forbid(unsafe_code)]
            fn main() {}
            ",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_no_unsafe_no_findings() {
        let diags = check(
            &UnsafeBlockAudit,
            r"
            fn main() {
                let x = 42;
            }
            ",
        );
        assert!(diags.is_empty());
    }

    // --- sql-injection-risk ---

    #[test]
    fn test_sql_injection_format_in_query() {
        let diags = check(
            &SqlInjectionRisk,
            r#"
            fn main() {
                let user_id = "1";
                db.query(format!("SELECT * FROM users WHERE id = {}", user_id));
            }
            "#,
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "sql-injection-risk");
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn test_sql_injection_format_in_execute() {
        let diags = check(
            &SqlInjectionRisk,
            r#"
            fn main() {
                db.execute(format!("DELETE FROM users WHERE id = {}", id));
            }
            "#,
        );
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn test_sql_injection_ref_format() {
        let diags = check(
            &SqlInjectionRisk,
            r#"
            fn main() {
                db.query(&format!("SELECT * FROM users WHERE id = {}", id));
            }
            "#,
        );
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn test_sql_no_format_not_flagged() {
        let diags = check(
            &SqlInjectionRisk,
            r#"
            fn main() {
                db.query("SELECT * FROM users WHERE id = $1");
            }
            "#,
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_format_outside_sql_not_flagged() {
        let diags = check(
            &SqlInjectionRisk,
            r#"
            fn main() {
                let msg = format!("Hello {}", name);
                println!("{}", msg);
            }
            "#,
        );
        assert!(diags.is_empty());
    }

    // --- test-code skipping ---

    #[test]
    fn test_hardcoded_secret_in_test_fn_skipped() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            #[test]
            fn test_auth() {
                let api_key = "sk-1234567890abcdef";
            }
            "#,
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_hardcoded_secret_in_cfg_test_module_skipped() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            #[cfg(test)]
            mod tests {
                fn helper() {
                    let password = "super_secret_password_123";
                }
            }
            "#,
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_unsafe_in_cfg_test_module_skipped() {
        let diags = check(
            &UnsafeBlockAudit,
            r"
            #[cfg(test)]
            mod tests {
                fn helper() {
                    unsafe { std::ptr::null::<i32>().read(); }
                }
            }
            ",
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn test_sql_injection_in_test_fn_skipped() {
        let diags = check(
            &SqlInjectionRisk,
            r#"
            #[test]
            fn test_query() {
                db.query(format!("SELECT * FROM users WHERE id = {}", user_id));
            }
            "#,
        );
        assert!(diags.is_empty());
    }

    // --- all_rules ---

    #[test]
    fn test_all_rules_returns_3() {
        assert_eq!(all_rules().len(), 3);
    }
}
