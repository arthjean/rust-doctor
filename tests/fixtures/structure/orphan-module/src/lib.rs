//! Trigger and silence of `rust_doctor::structure::orphan_module_file`.
//!
//! One file of this crate is reached by nothing: `src/orphan.rs` carries no
//! `mod` declaration anywhere, so Cargo never compiles it and no compiler
//! message will ever mention it. Everything else here is a way of reaching a
//! file that the rule must recognize, which is what proves it was checked for
//! over-reach: an ordinary `mod`, a `#[path]` attribute, a `mod` gated on
//! another platform, an `include!`, a build script, and an integration test
//! root. The generated file next to the orphan is unreached too, and stays
//! silent because no structural detector reads generated code.

pub mod reached;

#[path = "renamed/other.rs"]
pub mod aliased;

/// Compiled on Windows only, and reached everywhere: the pass reads module
/// declarations, it does not evaluate them.
#[cfg(windows)]
pub mod platform;

include!("table.rs");

pub fn size() -> usize {
    TABLE.len()
}
