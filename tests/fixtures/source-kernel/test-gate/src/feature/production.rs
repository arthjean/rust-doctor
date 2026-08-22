//! Declared under `#[cfg(not(test))]`, which is shipped code. The body is the
//! one `src/shared.rs` carries.

pub fn produced() -> usize {
    let mut count = 1;
    while count < 40 {
        count = count * 2 + 3;
    }
    let scaled = count / 4;
    scaled - 1
}
