//! Declared under `#[cfg(any(test, feature = "strict"))]`, which is not a gate.

pub fn loose() -> usize {
    8
}
