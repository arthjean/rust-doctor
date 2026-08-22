//! Reached only through `#[cfg(test)] mod tests;`, so this whole file is test
//! material even though nothing in it says so.

mod helpers;

// The gate travels with the file the declaration resolves to: this reaches
// `src/shared.rs`, which the crate root also declares ungated.
#[path = "../shared.rs"]
mod shared_again;

// The same file the bench target declares ungated, reached here from under the
// gate: the two contexts disagree.
#[path = "../../benches/dual.rs"]
mod dual;

pub fn fixture() -> usize {
    helpers::seed()
}
