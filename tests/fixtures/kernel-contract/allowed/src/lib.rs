#![allow(clippy::todo)]

pub fn crate_allowed_todo() -> u8 {
    todo!()
}

#[allow(clippy::dbg_macro)]
pub fn item_allowed_dbg(value: u8) -> u8 {
    dbg!(value)
}

#[allow(clippy::unimplemented)]
pub fn item_allowed_unimplemented() -> u8 {
    unimplemented!()
}
