#[test]
fn a_blocking_read_in_a_test_target() {
    let task = async { std::fs::read("Cargo.toml") };
    drop(task);
}
