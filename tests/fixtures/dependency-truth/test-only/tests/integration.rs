#[test]
fn integration_probe() {
    assert_eq!(probe_integration::value(), 1);
    assert_eq!(probe_both::value(), 1);
    assert_eq!(dev_only::value(), 1);
}
