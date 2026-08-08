#![allow(dead_code, reason = "the fixture surface exists to be scanned, not linked")]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod unchanged;

pub fn clean() -> usize {
    "projection".len()
}
