//! An integration test root is a Cargo target, and never an orphan.

#[test]
fn the_crate_publishes_its_table() {
    assert_eq!(structure_orphan_module::size(), 3);
}
