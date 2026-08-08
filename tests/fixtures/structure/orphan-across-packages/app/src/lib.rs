//! The member that reaches into its neighbour.
//!
//! `#[path]` crosses the package boundary here, which Cargo allows and which
//! makes `engine/src/shared.rs` compiled code even though no module declaration
//! of the package holding it names it.

#[path = "../../engine/src/shared.rs"]
pub mod shared;

pub fn total() -> u8 {
    shared::value()
}
