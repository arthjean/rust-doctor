use super::parse;

#[test]
fn parses() {
    assert_eq!(parse("ab"), 2);
}
