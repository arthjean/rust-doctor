//! `src/feature/tests.rs`, the other spelling of the same gated declaration.

mod nested;

pub fn reached() -> usize {
    nested::deep()
}
