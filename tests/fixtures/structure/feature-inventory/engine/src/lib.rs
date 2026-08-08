//! The package that declares `engine-only` and reads it.

#[cfg(feature = "engine-only")]
pub fn gated() -> u8 {
    0
}

pub fn always() -> u8 {
    1
}
