#[test]
fn tls_in_test_target_is_excluded() {
    let _ = http_client::Client::builder().tls_danger_accept_invalid_hostnames(true);
}
