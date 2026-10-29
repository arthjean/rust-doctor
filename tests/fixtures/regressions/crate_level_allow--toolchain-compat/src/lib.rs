//! Builds on two toolchains: the older one does not know the lint, and
//! `unknown_lints` keeps it from warning about the allow itself.
#![allow(mismatched_lifetime_syntaxes, unknown_lints)]

pub fn first(text: &str) -> &str {
    text.split(',').next().unwrap_or_default()
}
