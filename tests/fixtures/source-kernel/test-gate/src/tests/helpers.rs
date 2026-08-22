//! Declared by a file that is itself gated: the gate is inherited, not restated.
//!
//! Its body is duplicated in `src/feature/tests/nested.rs`, which is gated too,
//! so the family the structural pass reports has every member on test material
//! and carries a test context.

pub fn seed() -> usize {
    let mut total = 0;
    for step in 0..10 {
        total += step * 3;
        total -= step;
    }
    let doubled = total * 2;
    doubled + 7
}
