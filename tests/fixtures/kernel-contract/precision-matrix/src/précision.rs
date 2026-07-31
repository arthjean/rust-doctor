pub fn direct_dbg(value: u8) -> u8 {
    dbg!(value)
}

pub fn direct_todo() -> u8 {
    todo!("précision")
}

pub fn direct_unimplemented() -> u8 {
    unimplemented!("précision")
}
