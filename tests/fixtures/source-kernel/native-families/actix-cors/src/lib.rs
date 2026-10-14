//! The surface of actix-cors the fixture calls, with none of its behavior.

pub struct Cors;

impl Cors {
    pub fn default() -> Self {
        Self
    }

    pub fn permissive() -> Self {
        Self
    }
}
