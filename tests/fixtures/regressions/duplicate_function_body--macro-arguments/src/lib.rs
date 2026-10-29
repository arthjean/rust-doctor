//! The same scaffold checking two different things: the claim each makes
//! lives in its assertion, which a macro carries.

pub fn checked_above(input: &[u32], limit: u32) -> u32 {
    let mut total = 0;
    for value in input {
        if *value > limit {
            total += *value;
        }
    }
    assert!(total > limit, "the sum stays above the limit");
    total
}

pub fn checked_exact(input: &[u32], limit: u32) -> u32 {
    let mut total = 0;
    for value in input {
        if *value > limit {
            total += *value;
        }
    }
    assert_eq!(total, limit + 1);
    total
}
