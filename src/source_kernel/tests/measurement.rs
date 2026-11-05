use super::{metadata, scan};

#[test]
fn every_line_of_a_production_file_is_counted_except_its_inline_tests() {
    let scanned = scan(&metadata("line-count"));

    assert_eq!(scanned.production_lines, 16);
    assert!(scanned.complete);
    assert!(scanned.errors.is_empty(), "{:?}", scanned.errors);
}

#[test]
fn a_test_target_contributes_no_line_to_the_denominator() {
    let scanned = scan(&metadata("line-count"));

    assert_ne!(
        scanned.production_lines, 22,
        "the 6 lines of the test target were counted as production"
    );
    assert_eq!(scanned.production_lines, 16);
}

#[test]
fn an_undecodable_file_contributes_no_line_and_makes_the_count_a_floor() {
    let scanned = scan(&metadata("line-count-undecodable"));

    assert_eq!(scanned.production_lines, 3);
    assert!(!scanned.complete);
    assert_eq!(scanned.errors.len(), 1);
    assert_eq!(scanned.errors[0].code, "read-failed");
}

#[test]
fn two_walks_of_one_workspace_measure_the_same_number_of_lines() {
    let metadata = metadata("line-count");

    assert_eq!(
        scan(&metadata).production_lines,
        scan(&metadata).production_lines
    );
}
