















pub mod reached;

#[allow(dead_code)]
pub struct First {
    unread: u8,
}

#[allow(dead_code)]
pub struct Second {
    unread: u8,
}

#[allow(unused_variables)]
pub fn alone(ignored: u8) {}

#[allow(dead_code, reason = "the field belongs to the published shape")]
pub struct Reasoned {
    unread: u8,
}

#[expect(dead_code)]
pub struct Expected {
    unread: u8,
}

#[cfg_attr(test, allow(dead_code))]
pub struct Conditional {
    pub read_by_the_caller: u8,
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::First;

    #[test]
    fn the_module_exists_to_carry_its_exemption() {}
}
