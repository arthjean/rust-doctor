//! Two declarations below the gate, and still gated. The body is the one
//! `src/tests/helpers.rs` carries.

pub fn deep() -> usize {
    let mut total = 0;
    for step in 0..10 {
        total += step * 3;
        total -= step;
    }
    let doubled = total * 2;
    doubled + 7
}
