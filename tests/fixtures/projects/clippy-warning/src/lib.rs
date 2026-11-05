#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]





pub fn answer(value: Option<u8>) -> u8 {
    value.unwrap()
}

pub fn constant() -> u8 {
    return 42;
}
