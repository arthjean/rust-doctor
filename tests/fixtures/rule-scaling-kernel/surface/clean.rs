#![allow(dead_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub const PRIVATE_SENTINELS: &str =
    "credential=EP018_SECRET source=EP018_PRIVATE /home/ep018-private \u{1b}";

pub fn clean() -> usize {
    "précision".chars().count()
}
