//! What counts as a counted line.
//!
//! The score divides by this number, so what it includes is a published
//! contract rather than a consequence of how the walk happens to decode. The
//! two fixtures freeze it: `line-count` holds one of every line kind a
//! production file carries, and `line-count-undecodable` holds the one case
//! that costs the count its exactness.

use super::{metadata, scan};

/// `src/lib.rs` is 22 lines, the last 9 of them a `#[cfg(test)]` module, and
/// `src/helper.rs` is 3, so the workspace is 16 production lines. Every other
/// kind of line the two carry is in that number: a doc comment, a plain
/// comment, blank lines and a `mod` declaration. The inline test module is
/// not: the numerator already set its findings aside, and counting its lines
/// diluted every density. `src/helper.rs` ends without a trailing newline and
/// contributes 3, not 4: a final unterminated line is a line, and a terminated
/// one adds no phantom successor.
#[test]
fn every_line_of_a_production_file_is_counted_except_its_inline_tests() {
    let scanned = scan(&metadata("line-count"));

    assert_eq!(scanned.production_lines, 16);
    assert!(scanned.complete);
    assert!(scanned.errors.is_empty(), "{:?}", scanned.errors);
}

/// `tests/integration.rs` is 6 lines Cargo names a test target, and the count
/// never sees them: the denominator is the shipped code the score judges.
#[test]
fn a_test_target_contributes_no_line_to_the_denominator() {
    let scanned = scan(&metadata("line-count"));

    assert_ne!(
        scanned.production_lines, 22,
        "the 6 lines of the test target were counted as production"
    );
    assert_eq!(scanned.production_lines, 16);
}

/// A file that never decodes was still read, so it is not silently absent: it
/// contributes nothing and says the count is a floor. A bound is a budget on
/// work, and this one is a refusal, so it costs the measurement its
/// completeness rather than its meaning.
#[test]
fn an_undecodable_file_contributes_no_line_and_makes_the_count_a_floor() {
    let scanned = scan(&metadata("line-count-undecodable"));

    assert_eq!(scanned.production_lines, 3);
    assert!(!scanned.complete);
    assert_eq!(scanned.errors.len(), 1);
    assert_eq!(scanned.errors[0].code, "read-failed");
}

/// The same workspace measured twice is the same number. The walk sorts its
/// units, so nothing about the order the filesystem answered in reaches the
/// count.
#[test]
fn two_walks_of_one_workspace_measure_the_same_number_of_lines() {
    let metadata = metadata("line-count");

    assert_eq!(
        scan(&metadata).production_lines,
        scan(&metadata).production_lines
    );
}
