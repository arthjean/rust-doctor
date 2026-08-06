#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

// Two forms, one verdict each. `unwrap_used` is catalogued, so it reaches the
// report with its category, its tier and its help. `needless_return` is not,
// and `-A clippy::all` silences it at the source: it never reaches the report
// at all, where it used to arrive stripped of everything the catalog publishes.
pub fn answer(value: Option<u8>) -> u8 {
    value.unwrap()
}

pub fn constant() -> u8 {
    return 42;
}
