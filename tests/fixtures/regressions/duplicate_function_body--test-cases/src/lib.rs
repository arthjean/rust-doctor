//! Two test cases written to look alike: each drives the same scaffold with
//! its own inputs, which is how a suite is meant to read.

#[test]
fn sums_above_the_limit() {
    let input = [1_u32, 5, 9];
    let mut total = 0;
    for value in input {
        if value > 4 {
            total += value;
        } else {
            total += 1;
        }
    }
    assert_eq!(total, 15);
}

#[test]
fn counts_below_the_limit() {
    let input = [2_u32, 3, 8];
    let mut total = 0;
    for value in input {
        if value > 7 {
            total += value;
        } else {
            total += 1;
        }
    }
    assert_eq!(total, 10);
}
