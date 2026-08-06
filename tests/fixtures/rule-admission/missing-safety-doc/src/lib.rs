//! Trigger and silence of `clippy::missing_safety_doc`.
//!
//! The lint reads the documentation of a `pub unsafe fn`, not its body: the two
//! functions below are byte for byte the same code, and only the `# Safety`
//! section separates the reported one from the quiet one.

/// Reads the byte the pointer designates.
pub unsafe fn positive(pointer: *const u8) -> u8 {
    unsafe { *pointer }
}

/// Reads the byte the pointer designates.
///
/// # Safety
///
/// `pointer` must be non-null, aligned, and valid for a read of one byte.
pub unsafe fn negative(pointer: *const u8) -> u8 {
    unsafe { *pointer }
}
