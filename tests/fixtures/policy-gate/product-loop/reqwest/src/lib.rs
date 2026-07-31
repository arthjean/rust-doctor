pub struct Client;
pub struct ClientBuilder;

impl Client {
    pub fn builder() -> ClientBuilder {
        ClientBuilder
    }
}

impl ClientBuilder {
    pub fn danger_accept_invalid_certs(self, _enabled: bool) -> Self {
        self
    }
}
