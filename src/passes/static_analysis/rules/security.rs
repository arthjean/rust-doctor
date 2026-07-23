use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::rules::{CustomRule, has_cfg_test, is_test_context};
use std::path::Path;
use syn::spanned::Spanned;
use syn::visit::Visit;

// ─── Rule 1: hardcoded-secrets ──────────────────────────────────────────────

/// Flags string literals assigned to variables whose names match secret patterns.
pub struct HardcodedSecrets;

/// Name segments that signal a secret on their own (whole-word match).
const STANDALONE_SECRET_WORDS: &[&str] = &[
    "secret",
    "password",
    "passwd",
    "credential",
    "credentials",
    "apikey",
    "passphrase",
];

/// Two-word credential phrases (consecutive name segments). These pair an
/// otherwise-generic word (`key`/`token`/`secret`) with a credential qualifier,
/// so `auth_token`/`api_key` are secrets while `next_token`/`token_index` are not.
const CREDENTIAL_PHRASES: &[(&str, &str)] = &[
    ("api", "key"),
    ("api", "secret"),
    ("access", "key"),
    ("access", "token"),
    ("secret", "key"),
    ("private", "key"),
    ("private", "token"),
    ("auth", "token"),
    ("refresh", "token"),
    ("bearer", "token"),
    ("session", "token"),
    ("client", "secret"),
    ("encryption", "key"),
    ("signing", "key"),
];

/// Single-segment names that are secret even bare (legacy: a lone `token`).
const BARE_SECRET_WORDS: &[&str] = &["token"];

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

/// Fragments that mark a value (or adjacent name) as a placeholder, not a real
/// secret. Matched case-insensitively as substrings.
const PLACEHOLDER_FRAGMENTS: &[&str] = &[
    "example",
    "placeholder",
    "dummy",
    "changeme",
    "your_",
    "your-",
    "yourkey",
    "redacted",
    "replace",
    "sample",
    "fake",
    "todo",
    "insert",
    "lorem",
    "xxxx",
    "${",
    "{{",
    "}}",
    "<",
    ">",
    "...",
];

/// Split a variable name into lowercased word segments (snake_case + camelCase).
fn split_into_words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    // Reused across segments — `mem::take` below hands its buffer to `words`, so
    // this never holds a stale word between iterations.
    let mut current = String::new();
    for part in name.split('_') {
        if part.is_empty() {
            continue;
        }
        current.clear();
        let mut prev_lower = false;
        for ch in part.chars() {
            if ch.is_uppercase() && prev_lower && !current.is_empty() {
                words.push(std::mem::take(&mut current).to_lowercase());
            }
            current.push(ch);
            prev_lower = ch.is_lowercase();
        }
        if !current.is_empty() {
            words.push(current.to_lowercase());
        }
    }
    words
}

/// Returns true if the variable name suggests it holds a secret. Matches on word
/// boundaries (not substrings, `security.rs` pre-US-005) so `next_token` and
/// `token_index` are NOT secrets while `auth_token`/`api_key` are.
fn is_secret_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    if NON_SECRET_SUFFIXES
        .iter()
        .any(|suffix| lower.ends_with(suffix))
    {
        return false;
    }
    let words = split_into_words(name);
    if words.is_empty() {
        return false;
    }
    if words
        .iter()
        .any(|w| STANDALONE_SECRET_WORDS.contains(&w.as_str()))
    {
        return true;
    }
    if words.windows(2).any(|w| match w {
        [a, b] => CREDENTIAL_PHRASES.contains(&(a.as_str(), b.as_str())),
        _ => false,
    }) {
        return true;
    }
    matches!(words.as_slice(), [w] if BARE_SECRET_WORDS.contains(&w.as_str()))
}

/// Shannon entropy of a byte string in bits per character: ~0 for a uniform
/// string, approaching log2(alphabet) for random data. Home-grown — no crate.
fn shannon_entropy(value: &str) -> f64 {
    if value.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    for &byte in value.as_bytes() {
        if let Some(slot) = counts.get_mut(byte as usize) {
            *slot += 1;
        }
    }
    let len = value.len() as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = f64::from(c) / len;
            -p * p.log2()
        })
        .sum()
}

/// True if a single character dominates the value (> 30% of its length).
fn is_repetitive(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let mut counts = [0u32; 256];
    for &byte in value.as_bytes() {
        if let Some(slot) = counts.get_mut(byte as usize) {
            *slot += 1;
        }
    }
    let max = f64::from(counts.iter().copied().max().unwrap_or(0));
    max / (value.len() as f64) > 0.30
}

/// Trivial literals that are never secrets.
fn is_trivial_value(value: &str) -> bool {
    matches!(
        value.trim(),
        "" | "true" | "false" | "null" | "none" | "0" | "1"
    )
}

/// True if `s` contains any placeholder fragment (case-insensitive).
fn is_placeholder(s: &str) -> bool {
    let lower = s.to_lowercase();
    PLACEHOLDER_FRAGMENTS
        .iter()
        .any(|fragment| lower.contains(fragment))
}

/// True if the value is all hex digits (so the lower hex entropy threshold applies).
fn is_hex_like(value: &str) -> bool {
    value.len() >= 16 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// True iff `s` is non-empty and every byte is ASCII alphanumeric.
fn is_ascii_alnum(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// AWS access key id: `AKIA` + 16 uppercase/digit chars.
fn is_aws_access_key(value: &str) -> bool {
    value.strip_prefix("AKIA").is_some_and(|rest| {
        rest.len() == 16
            && rest
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    })
}

/// GitHub token: `gh[pousr]_` + 36+ alnum chars. Built from `strip_prefix` only,
/// so it stays char-boundary safe on arbitrary UTF-8 (no byte indexing).
fn is_github_token(value: &str) -> bool {
    value.len() >= 40
        && value
            .strip_prefix("gh")
            .and_then(|rest| rest.strip_prefix(['p', 'o', 'u', 's', 'r']))
            .and_then(|rest| rest.strip_prefix('_'))
            .is_some_and(is_ascii_alnum)
}

/// Stripe-style live/test keys: `sk_live_`, `pk_live_`, … + 16+ alnum chars.
fn is_stripe_key(value: &str) -> bool {
    ["sk_live_", "pk_live_", "rk_live_", "sk_test_", "pk_test_"]
        .iter()
        .any(|prefix| {
            value
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.len() >= 16 && is_ascii_alnum(rest))
        })
}

/// Google API key: `AIza` + 35 chars.
fn is_google_api_key(value: &str) -> bool {
    value
        .strip_prefix("AIza")
        .is_some_and(|rest| rest.len() == 35)
}

/// JWT: three dot-separated segments, the first starting with `eyJ`.
fn is_jwt(value: &str) -> bool {
    value.starts_with("eyJ") && value.split('.').count() == 3
}

/// Slack token: `xox[baprs]-…`.
fn is_slack_token(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.starts_with(b"xox") && bytes.get(3) == Some(&b'-')
}

/// Matches well-known secret token shapes without pulling in the `regex` crate.
/// Each shape is its own helper so this dispatcher stays simple — the previous
/// single-function form tripped our own `high-cyclomatic-complexity` rule.
fn matches_known_secret_shape(value: &str) -> bool {
    is_aws_access_key(value)
        || is_github_token(value)
        || is_stripe_key(value)
        || is_google_api_key(value)
        || is_jwt(value)
        || is_slack_token(value)
        || value.contains("PRIVATE KEY-----")
}

/// Decide whether a literal value actually resembles a secret (US-004 gate):
/// long enough, not trivial/placeholder/repetitive, and either matching a known
/// key shape OR exceeding the Shannon-entropy threshold (hex 2.5 / base64 3.5).
fn value_resembles_secret(value: &str, name: &str) -> bool {
    if value.len() <= 8 || is_trivial_value(value) {
        return false;
    }
    if is_placeholder(value) || is_placeholder(name) {
        return false;
    }
    if is_repetitive(value) {
        return false;
    }
    if matches_known_secret_shape(value) {
        return true;
    }
    let threshold = if is_hex_like(value) { 2.5 } else { 3.5 };
    shannon_entropy(value) >= threshold
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
        "Flags string literals assigned to secret-named variables (`api_key`, `password`, `token`, `secret`, …) when the value also looks like a real secret — matching a known key shape (AWS, GitHub, Stripe, JWT) or exceeding a Shannon-entropy threshold. Test code and placeholder values are ignored. Hardcoded secrets can be extracted from compiled binaries or version control."
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

impl SecretVisitor<'_> {
    /// Emit a diagnostic if (and only if) we are outside test code, the name
    /// looks like a secret, and the value resembles a real secret. Centralizing
    /// the gate keeps all four assignment sites consistent (US-003/004/005).
    fn maybe_flag(&mut self, name: &str, value: &str, span: proc_macro2::Span, descriptor: &str) {
        if self.in_test || !is_secret_name(name) || !value_resembles_secret(value, name) {
            return;
        }
        self.diagnostics.push(Diagnostic {
            file_path: self.path.to_path_buf(),
            rule: "hardcoded-secrets".to_string(),
            category: Category::Security,
            severity: Severity::Error,
            message: format!("Potential hardcoded secret in {descriptor}`{name}`"),
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

impl<'ast> Visit<'ast> for SecretVisitor<'_> {
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

    fn visit_local(&mut self, i: &'ast syn::Local) {
        if let Some(init) = &i.init
            && let syn::Expr::Lit(expr_lit) = init.expr.as_ref()
            && let syn::Lit::Str(lit_str) = &expr_lit.lit
            && let syn::Pat::Ident(pat_ident) = &i.pat
        {
            self.maybe_flag(
                &pat_ident.ident.to_string(),
                &lit_str.value(),
                pat_ident.ident.span(),
                "variable ",
            );
        }
        syn::visit::visit_local(self, i);
    }

    fn visit_expr_assign(&mut self, i: &'ast syn::ExprAssign) {
        if let syn::Expr::Lit(expr_lit) = i.right.as_ref()
            && let syn::Lit::Str(lit_str) = &expr_lit.lit
            && let Some(name) = extract_field_name(&i.left)
        {
            self.maybe_flag(&name, &lit_str.value(), i.left.span(), "");
        }
        syn::visit::visit_expr_assign(self, i);
    }

    fn visit_item_const(&mut self, i: &'ast syn::ItemConst) {
        if let syn::Expr::Lit(expr_lit) = i.expr.as_ref()
            && let syn::Lit::Str(lit_str) = &expr_lit.lit
        {
            self.maybe_flag(
                &i.ident.to_string(),
                &lit_str.value(),
                i.ident.span(),
                "const ",
            );
        }
        syn::visit::visit_item_const(self, i);
    }

    fn visit_item_static(&mut self, i: &'ast syn::ItemStatic) {
        if let syn::Expr::Lit(expr_lit) = i.expr.as_ref()
            && let syn::Lit::Str(lit_str) = &expr_lit.lit
        {
            self.maybe_flag(
                &i.ident.to_string(),
                &lit_str.value(),
                i.ident.span(),
                "static ",
            );
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
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

struct UnsafeVisitor<'a> {
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
}

impl<'ast> Visit<'ast> for UnsafeVisitor<'_> {
    fn visit_expr_unsafe(&mut self, i: &'ast syn::ExprUnsafe) {
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

    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        if i.sig.unsafety.is_some() {
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
        };
        visitor.visit_file(syntax);
        visitor.diagnostics
    }
}

struct SqlVisitor<'a> {
    path: &'a Path,
    diagnostics: Vec<Diagnostic>,
}

impl<'ast> Visit<'ast> for SqlVisitor<'_> {
    fn visit_expr_method_call(&mut self, i: &'ast syn::ExprMethodCall) {
        let method_name = i.method.to_string();
        if SQL_METHODS.contains(&method_name.as_str()) {
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
    fn test_secret_name_word_boundary() {
        // US-005: composed names with a non-credential `token` segment are NOT secret.
        assert!(!is_secret_name("next_token"));
        assert!(!is_secret_name("token_index"));
        assert!(!is_secret_name("token_count"));
        assert!(!is_secret_name("token_type"));
        assert!(!is_secret_name("nextToken"));
        // Real credential names still match (no false negatives).
        assert!(is_secret_name("api_key"));
        assert!(is_secret_name("auth_token"));
        assert!(is_secret_name("access_token"));
        assert!(is_secret_name("secret"));
        assert!(is_secret_name("password"));
        assert!(is_secret_name("private_key"));
        assert!(is_secret_name("apiKey"));
    }

    // --- US-003: test-context skip ---

    #[test]
    fn test_secret_in_test_fn_not_flagged() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            #[test]
            fn t() {
                let api_key = "Xk9mP2qR7wL5nZ8vB3cF6jH1dG4sT0yU";
            }
            "#,
        );
        assert!(diags.is_empty(), "secret in #[test] fn must be skipped");
    }

    #[test]
    fn test_secret_in_cfg_test_mod_not_flagged() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            #[cfg(test)]
            mod tests {
                fn helper() {
                    let api_key = "Xk9mP2qR7wL5nZ8vB3cF6jH1dG4sT0yU";
                }
            }
            "#,
        );
        assert!(
            diags.is_empty(),
            "secret in #[cfg(test)] mod must be skipped"
        );
    }

    #[test]
    fn test_secret_in_test_and_production_only_prod_flagged() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            fn prod() {
                let api_key = "Xk9mP2qR7wL5nZ8vB3cF6jH1dG4sT0yU";
            }
            #[cfg(test)]
            mod tests {
                fn helper() {
                    let api_key = "Ab7nQ4wE9rT2yU6iO0pL3kJ5hG8fD1sZ";
                }
            }
            "#,
        );
        assert_eq!(
            diags.len(),
            1,
            "only the production secret should be flagged"
        );
    }

    #[test]
    fn test_secret_in_bare_cfg_test_fn_not_flagged() {
        // US-003: a `#[cfg(test)]` function at file scope (not inside a
        // `#[cfg(test)]` mod, not `#[test]`) is still test code and must skip.
        let diags = check(
            &HardcodedSecrets,
            r#"
            #[cfg(test)]
            fn fixture() {
                let api_key = "Xk9mP2qR7wL5nZ8vB3cF6jH1dG4sT0yU";
            }
            "#,
        );
        assert!(
            diags.is_empty(),
            "secret in a bare #[cfg(test)] fn must be skipped"
        );
    }

    // --- US-004: value entropy + shape gate ---

    #[test]
    fn test_shannon_entropy_low_vs_high() {
        assert!(shannon_entropy("aaaaaaaaaaaaaaaa") < 0.5);
        assert!(shannon_entropy("Xk9mP2qR7wL5nZ8vB3cF6jH1dG4sT0yU") > 4.0);
    }

    #[test]
    fn test_known_secret_shapes() {
        assert!(matches_known_secret_shape("AKIA1234567890ABCDEF"));
        assert!(matches_known_secret_shape(
            "ghp_0123456789abcdefABCDEF0123456789abcd"
        ));
        assert!(matches_known_secret_shape("sk_live_0123456789abcdefABCDEF"));
        assert!(matches_known_secret_shape(
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.c2lnbmF0dXJlX3ZhbHVl"
        ));
        assert!(!matches_known_secret_shape("just_a_normal_string_value"));
    }

    #[test]
    fn test_shape_matching_utf8_safe() {
        // Arbitrary multibyte UTF-8 (literal values are attacker-controlled) must
        // never panic the byte-offset logic — only `strip_prefix`/iteration, no
        // raw `value[n..]` slicing. These all return cleanly (false).
        assert!(!matches_known_secret_shape(
            "gh\u{e9}_multibyte_after_prefix_xx"
        ));
        assert!(!matches_known_secret_shape("héllo"));
        assert!(!matches_known_secret_shape("ключ"));
        assert!(!matches_known_secret_shape("🔑🔑🔑🔑🔑🔑🔑🔑"));
        assert!(!matches_known_secret_shape("AKIA🔑"));
        // A genuine GitHub-shaped token is still recognised.
        assert!(matches_known_secret_shape(
            "ghp_0123456789abcdefABCDEF0123456789abcd"
        ));
    }

    #[test]
    fn test_placeholder_value_not_flagged() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            fn main() {
                let api_key = "your_api_key_here_please_change";
            }
            "#,
        );
        assert!(diags.is_empty(), "placeholder value must not be flagged");
    }

    #[test]
    fn test_placeholder_name_not_flagged() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            fn main() {
                let example_api_key = "Xk9mP2qR7wL5nZ8vB3cF6jH1dG4sT0yU";
            }
            "#,
        );
        assert!(
            diags.is_empty(),
            "placeholder-named var must not be flagged"
        );
    }

    #[test]
    fn test_repetitive_and_trivial_values_not_flagged() {
        let diags = check(
            &HardcodedSecrets,
            r#"
            fn main() {
                let api_key = "aaaaaaaaaaaaaaaaaaaa";
                let auth_token = "xxxxxxxxxxxxxxxxxxxx";
            }
            "#,
        );
        assert!(diags.is_empty(), "repetitive values must not be flagged");
        assert!(is_trivial_value("true"));
        assert!(is_trivial_value("null"));
        assert!(!is_trivial_value("Xk9mP2qR7wL5nZ8vB3cF6jH1"));
    }

    #[test]
    fn test_fp_corpus_zero_false_positives() {
        // Non-secret names with long, anodyne values: 0 false positives.
        let diags = check(
            &HardcodedSecrets,
            r#"
            fn main() {
                let commit_hash = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
                let request_id = "550e8400-e29b-41d4-a716-446655440000";
                let description = "lorem ipsum dolor sit amet consectetur adipiscing elit";
                let base_url = "https://api.example.com/v1/resources/list";
            }
            "#,
        );
        assert!(
            diags.is_empty(),
            "no false positives expected, got: {diags:?}"
        );
    }

    #[test]
    fn test_recall_corpus_well_formed_secrets() {
        // Production secret-named vars with well-formed values: recall ≥ 90%.
        let diags = check(
            &HardcodedSecrets,
            r#"
            fn main() {
                let api_key = "AKIA1234567890ABCDEF";
                let access_token = "ghp_0123456789abcdefABCDEF0123456789abcd";
                let api_secret = "sk_live_0123456789abcdefABCDEF";
                let auth_token = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.c2lnbmF0dXJlX3ZhbA";
                let secret_key = "Xk9mP2qR7wL5nZ8vB3cF6jH1dG4sT0yU";
            }
            "#,
        );
        assert_eq!(
            diags.len(),
            5,
            "all five well-formed secrets should be flagged"
        );
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

    // --- all_rules ---

    #[test]
    fn test_all_rules_returns_3() {
        assert_eq!(all_rules().len(), 3);
    }
}
