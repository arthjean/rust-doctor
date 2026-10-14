use tower_http::cors::{AllowOrigin, Any, CorsLayer};

pub fn permissive_without_credentials() -> CorsLayer {
    CorsLayer::permissive()
}

pub fn listed_origin() -> CorsLayer {
    CorsLayer::new().allow_origin(AllowOrigin::exact("https://app.example")).allow_credentials(true)
}

pub fn any_origin_without_credentials() -> CorsLayer {
    CorsLayer::new().allow_origin(Any)
}

pub fn readable_theme() -> cookie::Cookie {
    cookie::Cookie::build(("theme", "dark")).http_only(false).build()
}

pub fn readable_csrf() -> cookie::Cookie {
    cookie::Cookie::build(("csrf_token", "value")).http_only(false).build()
}

pub fn verified() -> jsonwebtoken::Validation {
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.set_audience(&["api"]);
    validation
}

#[cfg(test)]
mod tests {
    fn unsigned_fixture() {
        let _ = jsonwebtoken::dangerous::insecure_decode("token");
    }
}
