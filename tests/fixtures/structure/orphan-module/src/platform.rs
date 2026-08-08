//! Reached only through a `mod` declaration gated on another platform.

pub fn only_on_windows() -> u8 {
    3
}
