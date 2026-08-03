//! Comportement d'exemption du pack dans un fichier de test au sens Cargo.
//!
//! L'exemption n'est pas une propriété du lint: elle est gouvernée par le
//! `clippy.toml` du workspace scanné. Celui de cette fixture pose
//! `allow-unwrap-in-tests` et `allow-expect-in-tests` à `true`,
//! `allow-panic-in-tests` et `allow-print-in-tests` à `false`, donc `unwrap` et
//! `expect` restent muets ici pendant que `panic` et `println` sont signalés.
//! L'oracle fige ce verdict et nomme l'option qui le produit.
//!
//! Les usages sont écrits hors de toute macro: Clippy ne signale pas ce qui
//! provient d'une expansion, donc un `assert_eq!(value.unwrap(), ..)` mesurerait
//! l'expansion et non l'exemption.

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
