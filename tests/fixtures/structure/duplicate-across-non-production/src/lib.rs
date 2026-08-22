//! A crate whose only clone family lives entirely outside what it ships.
//!
//! Nothing here is duplicated: the family this fixture exists for has one
//! member in the bench target and one in the integration test, so a pass that
//! marks a family only when its members agree on a single context would leave
//! it unmarked and charge the shipped codebase for code the crate never ships.

/// The one thing the crate ships, and it is nobody's copy.
pub fn shipped(values: &[u32]) -> u32 {
    values.iter().copied().max().unwrap_or_default()
}
