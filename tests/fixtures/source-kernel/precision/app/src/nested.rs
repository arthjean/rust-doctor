use http_client::Client as OtherClient;

mod child;

pub fn clients() {
    let _ = http_client::Client::builder().tls_danger_accept_invalid_certs(true);
    let _ = http_client::blocking::Client::builder()
        .danger_accept_invalid_hostnames(true);
}

pub fn imported_builder() {
    let _ = OtherClient::builder().tls_danger_accept_invalid_certs(true);
}
