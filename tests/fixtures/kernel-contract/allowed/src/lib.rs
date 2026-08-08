#![allow(clippy::todo, reason = "the fixture pairs a reported form with a silenced one")]

pub fn crate_allowed_todo() -> u8 {
    todo!()
}

#[allow(clippy::dbg_macro, reason = "the fixture pairs a reported form with a silenced one")]
pub fn item_allowed_dbg(value: u8) -> u8 {
    dbg!(value)
}

#[allow(clippy::unimplemented, reason = "the fixture pairs a reported form with a silenced one")]
pub fn item_allowed_unimplemented() -> u8 {
    unimplemented!()
}
