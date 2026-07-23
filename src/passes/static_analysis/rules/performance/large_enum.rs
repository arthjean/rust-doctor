use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::rules::CustomRule;
use std::path::Path;

/// Flags enums where the largest variant has >3x more fields than the smallest.
pub struct LargeEnumVariant;

impl CustomRule for LargeEnumVariant {
    fn name(&self) -> &'static str {
        "large-enum-variant"
    }
    fn category(&self) -> Category {
        Category::Performance
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn description(&self) -> &'static str {
        "Flags enums where variants have significantly different sizes (>3x field count disparity). The enum's size equals its largest variant, wasting memory for smaller variants."
    }
    fn fix_hint(&self) -> &'static str {
        "Box the large variant's data: `LargeVariant(Box<LargeData>)`."
    }
    fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for item in &syntax.items {
            if let syn::Item::Enum(e) = item {
                let field_counts: Vec<usize> = e
                    .variants
                    .iter()
                    .map(|v| match &v.fields {
                        syn::Fields::Named(f) => f.named.len(),
                        syn::Fields::Unnamed(f) => f.unnamed.len(),
                        syn::Fields::Unit => 0,
                    })
                    .collect();

                if field_counts.len() < 2 {
                    continue;
                }

                let min = field_counts.iter().copied().min().unwrap_or(0);
                let max = field_counts.iter().copied().max().unwrap_or(0);

                // Only flag if the largest variant has >3x the fields of the smallest non-zero,
                // or if the largest has >5 fields and the smallest is 0
                let threshold_exceeded = if min > 0 { max > min * 3 } else { max > 5 };

                if threshold_exceeded {
                    let span = e.ident.span();
                    diagnostics.push(Diagnostic {
                        file_path: path.to_path_buf(),
                        rule: "large-enum-variant".to_string(),
                        category: Category::Performance,
                        severity: Severity::Warning,
                        message: format!(
                            "Enum `{}` has variant size disparity (min {} fields, max {} fields)",
                            e.ident, min, max
                        ),
                        help: Some(
                            "Consider boxing the large variant's fields to reduce enum size"
                                .to_string(),
                        ),
                        line: Some(span.start().line as u32),
                        column: Some(span.start().column as u32 + 1),
                        fix: None,
                    });
                }
            }
        }

        diagnostics
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
    fn test_large_enum_variant_detected() {
        let diags = check(
            &LargeEnumVariant,
            r"
            enum Message {
                Quit,
                Data {
                    a: i32, b: i32, c: i32, d: i32,
                    e: i32, f: i32, g: i32, h: i32,
                },
            }
            ",
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "large-enum-variant");
    }

    #[test]
    fn test_balanced_enum_not_flagged() {
        let diags = check(
            &LargeEnumVariant,
            r"
            enum Color {
                Red(u8),
                Green(u8),
                Blue(u8),
            }
            ",
        );
        assert!(diags.is_empty());
    }
}
