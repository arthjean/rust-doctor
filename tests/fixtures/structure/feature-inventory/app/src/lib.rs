











#[cfg(all(unix, feature = "read"))]
pub fn read() -> u8 {
    0
}

#[cfg(feature = "absent")]
pub fn absent() -> u8 {
    1
}

#[cfg(feature = "engine-only")]
pub fn borrowed() -> u8 {
    2
}
