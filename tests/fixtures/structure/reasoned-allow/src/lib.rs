//! Silence of `rust_doctor::structure::unreasoned_allow_attribute`.
//!
//! Every exemption here states why it exists, so the census has nothing to
//! report and no `related` key reaches the published diagnostic set.

#![allow(dead_code, reason = "the crate exists to be scanned, not linked")]

pub struct Documented {
    unread: u8,
}
