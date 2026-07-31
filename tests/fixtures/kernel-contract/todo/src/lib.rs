use std::todo as aliased_todo;

pub fn direct() -> u8 {
    todo!()
}

pub fn qualified() -> u8 {
    std::todo!()
}

pub fn aliased() -> u8 {
    aliased_todo!()
}

#[allow(clippy::todo)]
pub fn allowed() -> u8 {
    todo!()
}
