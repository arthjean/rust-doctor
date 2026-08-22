//! Reached twice, and the two reaches disagree on which non-production context
//! this is: the bench target declares it directly, and `src/tests/mod.rs`
//! declares it from under a `#[cfg(test)]` gate. Two different non-production
//! contexts are not unanimous, so the unit abstains.

pub fn dual() -> usize {
    11
}
