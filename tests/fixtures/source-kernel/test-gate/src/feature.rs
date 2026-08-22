//! Every declaration form the gate grammar has to tell apart.

#[cfg(test)]
mod tests;

// Compiled out of a test build: the opposite claim, never a gate.
#[cfg(not(test))]
pub mod production;

// A feature named after tests is a string, never the `test` predicate.
#[cfg(feature = "test-util")]
pub mod util;

// Every arm of an `all` has to hold, so this is a gate.
#[cfg(all(test, feature = "strict"))]
mod strict;

// Another arm can hold on its own, so the module survives outside a test
// build: the conservative reading refuses it.
#[cfg(any(test, feature = "strict"))]
pub mod loose;

pub fn feature() -> usize {
    2
}
