//! One member of the family, in a bench target.

fn accumulate(values: &[u32], limit: u32) -> u32 {
    let mut total = 0;
    for value in values {
        if *value > limit {
            total += *value;
        } else {
            total -= limit;
        }
    }
    total
}

fn main() {
    let _ = accumulate(&[1, 2, 3], 1);
}
