














fn parsed(value: &str) -> Option<u8> {
    value.parse().ok()
}

#[test]
fn exemption_unwrap_used() {
    let value = parsed("1").unwrap();
    assert_eq!(value, 1);
}

#[test]
fn exemption_expect_used() {
    let value = parsed("1").expect("the fixture provides a value");
    assert_eq!(value, 1);
}

#[test]
fn exemption_panic() {
    if parsed("1").is_none() {
        panic!("never reached");
    }
}

#[test]
fn exemption_print_stdout() {
    println!("exemption");
}
