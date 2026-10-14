//! The surface of jsonwebtoken the fixture calls, with none of its behavior.

pub enum Algorithm {
    HS256,
}

pub struct Validation;

impl Validation {
    pub fn new(_algorithm: Algorithm) -> Self {
        Self
    }

    pub fn insecure_disable_signature_validation(&mut self) {}

    pub fn set_audience(&mut self, _audience: &[&str]) {}
}

pub mod dangerous {
    pub fn insecure_decode(_token: &str) -> Result<(), ()> {
        Ok(())
    }
}
