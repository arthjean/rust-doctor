

#![allow(mismatched_lifetime_syntaxes, unknown_lints)]

pub fn first(text: &str) -> &str {
    text.split(',').next().unwrap_or_default()
}
