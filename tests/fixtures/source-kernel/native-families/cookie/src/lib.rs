//! The surface of cookie the fixture calls, with none of its behavior.

pub struct Cookie;

impl Cookie {
    pub fn new(_name: &str, _value: &str) -> Self {
        Self
    }

    pub fn build(_pair: (&str, &str)) -> CookieBuilder {
        CookieBuilder
    }

    pub fn set_http_only(&mut self, _value: bool) {}
}

pub struct CookieBuilder;

impl CookieBuilder {
    pub fn http_only(self, _value: bool) -> Self {
        self
    }

    pub fn build(self) -> Cookie {
        Cookie
    }
}
