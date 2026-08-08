//! A crate-level exemption in an integration test crate.
//!
//! It is the inner form of the attribute, `#![allow(...)]` rather than
//! `#[allow(...)]`, and it lives in a crate no Cargo target ships. The census
//! counts it and marks it, so it stays published without weighing on the score.

#![allow(dead_code)]

#[test]
fn the_crate_exists_to_carry_its_crate_level_exemption() {}
