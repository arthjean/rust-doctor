use std::unimplemented as aliased_unimplemented;

pub fn direct() -> u8 {
    unimplemented!()
}

pub fn qualified() -> u8 {
    std::unimplemented!()
}

pub fn aliased() -> u8 {
    aliased_unimplemented!()
}

#[allow(clippy::unimplemented, reason = "the fixture pairs a reported form with a silenced one")]
pub fn allowed() -> u8 {
    unimplemented!()
}
