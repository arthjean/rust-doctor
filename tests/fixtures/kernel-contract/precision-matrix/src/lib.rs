#[path = "précision.rs"]
mod precision;

pub use precision::{direct_dbg, direct_todo, direct_unimplemented};

// negative-comment-dbg: dbg!(value)
// negative-comment-todo: todo!()
// negative-comment-unimplemented: unimplemented!()
pub fn negative_strings() -> (&'static str, &'static str, &'static str) {
    (
        "negative-string-dbg: dbg!(value)",
        "negative-string-todo: todo!()",
        "negative-string-unimplemented: unimplemented!()",
    )
}

#[allow(clippy::dbg_macro)]
pub fn negative_allowed_dbg(value: u8) -> u8 {
    dbg!(value)
}

#[allow(clippy::todo)]
pub fn negative_allowed_todo() -> u8 {
    todo!()
}

#[allow(clippy::unimplemented)]
pub fn negative_allowed_unimplemented() -> u8 {
    unimplemented!()
}

macro_rules! negative_dbg_neighbor {
    ($value:expr) => {
        $value
    };
}

macro_rules! negative_todo_neighbor {
    () => {
        7_u8
    };
}

macro_rules! negative_unimplemented_neighbor {
    () => {
        9_u8
    };
}

pub fn negative_neighbor_macros(value: u8) -> (u8, u8, u8) {
    (
        negative_dbg_neighbor!(value),
        negative_todo_neighbor!(),
        negative_unimplemented_neighbor!(),
    )
}

pub fn non_curated_diagnostic() -> u8 {
    return 42;
}
