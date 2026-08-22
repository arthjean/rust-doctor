//! Reached both ways: ungated from the crate root and gated from `src/tests`.
//! Two traversals that disagree, so the context abstains and the file weighs.
//!
//! Its body is duplicated in `src/feature/production.rs`, which is shipped
//! code. That family straddles an abstaining file and a production one, so it
//! abstains in turn and is charged: the duplication genuinely involves code
//! that ships.

pub fn shared() -> usize {
    let mut count = 1;
    while count < 40 {
        count = count * 2 + 3;
    }
    let scaled = count / 4;
    scaled - 1
}
