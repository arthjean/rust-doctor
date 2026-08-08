//! Compiled by the neighbouring package through its `#[path]` declaration, and
//! by no target of the package this file sits in. Cargo compiles it, so it is
//! not an orphan.

pub fn value() -> u8 {
    7
}
