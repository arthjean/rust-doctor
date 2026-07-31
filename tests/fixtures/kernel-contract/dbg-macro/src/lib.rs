use std::dbg as aliased_dbg;

pub fn direct(value: u8) -> u8 {
    dbg!(value)
}

pub fn qualified(value: u8) -> u8 {
    std::dbg!(value)
}

pub fn aliased(value: u8) -> u8 {
    aliased_dbg!(value)
}

#[allow(clippy::dbg_macro)]
pub fn allowed(value: u8) -> u8 {
    dbg!(value)
}
