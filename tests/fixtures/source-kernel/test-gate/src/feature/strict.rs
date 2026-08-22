//! Declared under `#[cfg(all(test, feature = "strict"))]`, which is a gate.

pub fn strict() -> usize {
    6
}
