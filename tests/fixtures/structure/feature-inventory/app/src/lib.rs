//! Trigger and silence of `rust_doctor::structure::unreferenced_feature`.
//!
//! Two disagreements between this manifest and this code. The `unused` feature
//! is declared and read by nothing, and the two gates below name features this
//! package does not declare: one that no manifest of the workspace declares,
//! and one the neighbouring package declares, which is the point of resolving
//! features per package rather than over the union of the workspace.
//!
//! What must stay silent sits next to it: `default`, which Cargo reads, `read`,
//! which the gate below names through an `all(...)`, and `alias`, which
//! activates `read` whether or not a gate ever names it.

#[cfg(all(unix, feature = "read"))]
pub fn read() -> u8 {
    0
}

#[cfg(feature = "absent")]
pub fn absent() -> u8 {
    1
}

#[cfg(feature = "engine-only")]
pub fn borrowed() -> u8 {
    2
}
