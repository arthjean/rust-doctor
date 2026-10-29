mod support;

#[test]
fn doubles_the_fixture() {
    assert_eq!(seed_crate_level_allow_test_support_dead_code::double(support::fixture()), 42);
}
