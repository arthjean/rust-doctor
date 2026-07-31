mod reqwest {
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

pub fn tls_without_dependency() {
    let _ = reqwest::Client::builder().tls_danger_accept_invalid_certs(true);
}
