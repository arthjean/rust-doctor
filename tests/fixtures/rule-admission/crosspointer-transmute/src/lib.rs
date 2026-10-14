//! Trigger and silence of `clippy::crosspointer_transmute`.
//!
//! The positive reinterprets a pointer to a `usize` as a `usize`, the mistake
//! the lint names: the caller almost always meant the value behind the pointer
//! or its address, and the transmute silently yields the address. The negative
//! asks for the address by name.

/// The address of the value.
pub fn positive(value: &usize) -> usize {
    let pointer: *const usize = value;
    // SAFETY: a pointer and a usize have the same size, and every bit pattern
    // is a valid usize.
    unsafe { std::mem::transmute::<*const usize, usize>(pointer) }
}

/// The address of the value.
pub fn negative(value: &usize) -> usize {
    std::ptr::from_ref(value).addr()
}
