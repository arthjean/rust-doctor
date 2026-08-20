//! A `tests/` target is not production, so none of its lines are counted.

#[test]
fn it_integrates() {
    assert_eq!(source_kernel_line_count::helped(), 2);
}
