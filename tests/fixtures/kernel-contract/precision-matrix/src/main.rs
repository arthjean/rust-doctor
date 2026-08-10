fn qualified_dbg(value: u8) -> u8 {
    std::dbg!(value)
}

fn qualified_todo() -> u8 {
    std::todo!("qualified")
}

fn qualified_unimplemented() -> u8 {
    std::unimplemented!("qualified")
}

fn main() {
    let first = qualified_dbg as fn(u8) -> u8;
    let second = qualified_todo as fn() -> u8;
    let _ = (first, second, qualified_unimplemented as fn() -> u8);
}
