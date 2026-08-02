#![allow(dead_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod unchanged;

pub fn clean() -> usize {
    "projection".len()
}
