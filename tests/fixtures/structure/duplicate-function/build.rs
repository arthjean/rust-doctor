//! A clone family living where Cargo puts it rather than where the crate ships.
//!
//! The two functions below are one family, and it is marked as a build script
//! rather than counted against the shipped codebase, exactly as a `println!`
//! in a build script is.

fn level_of(name: &str) -> u8 {
    let trimmed = name.trim();
    let level = match trimmed {
        "one" => 1,
        "two" => 2,
        "three" => 3,
        _ => 0,
    };
    level + 1
}

fn rank_of(label: &str) -> u8 {
    let cleaned = label.trim();
    let rank = match cleaned {
        "alpha" => 4,
        "beta" => 5,
        "gamma" => 6,
        _ => 9,
    };
    rank + 1
}

fn main() {
    let _ = (level_of("one"), rank_of("alpha"));
}
