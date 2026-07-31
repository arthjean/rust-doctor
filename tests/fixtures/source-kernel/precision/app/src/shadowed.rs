mod http_client {
    pub struct Client;
    pub struct Builder;

    impl Client {
        pub fn builder() -> Builder {
            Builder
        }
    }

    impl Builder {
        pub fn tls_danger_accept_invalid_certs(self, _enabled: bool) -> Self {
            self
        }
    }
}

pub fn local_builder() {
    let _ = http_client::Client::builder().tls_danger_accept_invalid_certs(true);
}
