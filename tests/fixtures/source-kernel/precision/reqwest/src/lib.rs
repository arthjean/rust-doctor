pub struct Client;
pub struct ClientBuilder;
pub struct OtherClient;

impl Client {
    pub fn builder() -> ClientBuilder {
        ClientBuilder
    }
}

impl OtherClient {
    pub fn builder() -> ClientBuilder {
        ClientBuilder
    }
}

impl ClientBuilder {
    pub fn tls_danger_accept_invalid_certs(self, _enabled: bool) -> Self {
        self
    }

    pub fn tls_danger_accept_invalid_hostnames(self, _enabled: bool) -> Self {
        self
    }

    pub fn danger_accept_invalid_certs(self, _enabled: bool) -> Self {
        self
    }

    pub fn danger_accept_invalid_hostnames(self, _enabled: bool) -> Self {
        self
    }

    pub fn http2_prior_knowledge(self, _enabled: bool) -> Self {
        self
    }
}

pub mod blocking {
    pub struct Client;

    impl Client {
        pub fn builder() -> super::ClientBuilder {
            super::ClientBuilder
        }
    }
}
