//! Trigger and silence of `clippy::lint_groups_priority`.
//!
//! The lint reads the `[lints]` tables of `Cargo.toml`, not this file: the
//! `rust` table there is the reported form, and the `clippy` table below it is
//! the same shape with the group given a lower priority, which it leaves alone.

/// Returns the value unchanged.
pub fn identity(value: u8) -> u8 {
    value
}
