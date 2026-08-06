//! Trigger and silence of `clippy::too_many_arguments`.
//!
//! The lint counts the parameters of a signature against
//! `too-many-arguments-threshold`, which no configuration here changes, so the
//! default of seven applies: the reported function takes eight, the quiet one
//! takes seven, and the two bodies are the same sum.

pub fn positive(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8, g: u8, h: u8) -> u32 {
    u32::from(a) + u32::from(b) + u32::from(c) + u32::from(d)
        + u32::from(e) + u32::from(f) + u32::from(g) + u32::from(h)
}

pub fn negative(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8, g: u8) -> u32 {
    u32::from(a) + u32::from(b) + u32::from(c) + u32::from(d)
        + u32::from(e) + u32::from(f) + u32::from(g)
}
