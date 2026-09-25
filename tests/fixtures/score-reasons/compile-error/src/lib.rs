//! A compile error: an error the catalog does not describe is the compilation
//! failing, never a compiler note.

/// Returns a string where a number is declared.
pub fn broken() -> u32 {
    "not a number"
}
