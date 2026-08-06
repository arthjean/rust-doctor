//! What a test file in the Cargo sense would produce, and no longer does.
//!
//! The exemption is not a property of the lint: it is governed by the
//! `clippy.toml` of the scanned workspace. The one of this fixture sets
//! `allow-unwrap-in-tests` and `allow-expect-in-tests` to `true`,
//! `allow-panic-in-tests` and `allow-print-in-tests` to `false`, so `unwrap`
//! and `expect` would stay silent here while `panic` and `println` would be
//! reported. Both verdicts are represented on purpose: the scan compiles
//! Cargo's default targets, so this file reaches the report through neither of
//! them, and the oracle freezes that emptiness.
//!
//! The usages are written outside any macro: Clippy does not report what comes
//! from an expansion, so an `assert_eq!(value.unwrap(), ..)` would measure the
//! expansion and not the exemption.

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
