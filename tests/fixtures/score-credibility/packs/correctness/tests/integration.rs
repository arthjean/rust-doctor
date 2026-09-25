//! The `eq_op` positive, repeated in an integration test target.
//!
//! The scan compiles Cargo's default targets, which leave integration tests
//! out, so this trigger reaches the report through no lint and weighs nothing.
//! The oracle records that as an empty `test_context`.

#[test]
fn integration_eq_op() {
    let value = 1;
    assert!(value == value);
}
