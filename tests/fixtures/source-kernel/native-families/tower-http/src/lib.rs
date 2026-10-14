//! The surface of tower-http the fixture calls, with none of its behavior.

pub mod cors {
    pub struct Any;

    pub struct AllowOrigin;

    impl AllowOrigin {
        pub fn mirror_request() -> Self {
            Self
        }

        pub fn exact(_origin: &str) -> Self {
            Self
        }
    }

    pub struct CorsLayer;

    impl CorsLayer {
        pub fn new() -> Self {
            Self
        }

        pub fn permissive() -> Self {
            Self
        }

        pub fn very_permissive() -> Self {
            Self
        }

        pub fn allow_origin<T>(self, _origin: T) -> Self {
            self
        }

        pub fn allow_credentials(self, _allow: bool) -> Self {
            self
        }
    }
}
