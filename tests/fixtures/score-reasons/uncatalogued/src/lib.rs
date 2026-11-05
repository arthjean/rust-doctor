





use std::collections::HashMap;


pub fn double(value: u32) -> u32 {
    value.wrapping_mul(2)
}


#[must_use]
pub fn same(value: u32) -> bool {
    value == value
}
