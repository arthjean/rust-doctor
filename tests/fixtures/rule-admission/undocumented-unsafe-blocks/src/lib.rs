//! Trigger and silence of `clippy::undocumented_unsafe_blocks`.
//!
//! Both functions state the same contract and run the same block: only the
//! `// SAFETY:` comment in front of the second block, tying the read to that
//! contract, separates the reported one from the quiet one.

/// Reads the byte the pointer designates.
///
/// # Safety
///
/// `pointer` must be non-null, aligned, and valid for a read of one byte.
pub unsafe fn positive(pointer: *const u8) -> u8 {
    unsafe { *pointer }
}

/// Reads the byte the pointer designates.
///
/// # Safety
///
/// `pointer` must be non-null, aligned, and valid for a read of one byte.
pub unsafe fn negative(pointer: *const u8) -> u8 {
    // SAFETY: the caller upholds the contract stated above.
    unsafe { *pointer }
}
