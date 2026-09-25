//! Two warnings the catalog does not describe, in shipped code: rustc's
//! `unused_imports`, and `clippy::must_use_candidate`, which only fires
//! because this crate's `[lints.clippy]` table enables `pedantic`. The
//! catalogued `clippy::eq_op` next to them is the scored finding the notes
//! have to follow.

use std::collections::HashMap;

/// Doubles the value.
pub fn double(value: u32) -> u32 {
    value.wrapping_mul(2)
}

/// Compares the value with itself.
#[must_use]
pub fn same(value: u32) -> bool {
    value == value
}
