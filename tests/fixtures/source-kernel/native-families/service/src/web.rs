use tower_http::cors::{AllowOrigin, CorsLayer};

pub fn credentialed_mirror() -> CorsLayer {
    CorsLayer::new().allow_origin(AllowOrigin::mirror_request()).allow_credentials(true)
}

pub fn very_permissive() -> CorsLayer {
    CorsLayer::very_permissive()
}

pub fn actix_permissive() -> actix_cors::Cors {
    actix_cors::Cors::permissive()
}

pub fn readable_session() -> cookie::Cookie {
    cookie::Cookie::build(("session_id", "value")).http_only(false).build()
}

pub fn disabled_signature() -> jsonwebtoken::Validation {
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.insecure_disable_signature_validation();
    validation
}

pub fn unverified(token: &str) -> Result<(), ()> {
    jsonwebtoken::dangerous::insecure_decode(token)
}
