//! The variant tables of the three web hardening rules.

use super::super::{Case, Semantic, Variants};
use crate::policy::{SOURCE_DISABLED_JWT_VALIDATION, SOURCE_INSECURE_AUTH_COOKIE, SOURCE_PERMISSIVE_CORS};

pub(in crate::source_kernel) static CORS_VARIANTS: Variants = Variants {
    rule: SOURCE_PERMISSIVE_CORS.id,
    positive: Case {
        attributes: "",
        asynchronous: false,
        import: "tower_http::cors::CorsLayer",
        parameters: "",
        setup: "let $credentials = true;",
        receiver: "{Item}::new()",
        chain: ".allow_origin(tower_http::cors::AllowOrigin::mirror_request()).allow_credentials($credentials)",
    },
    negative: Case {
        attributes: "",
        asynchronous: false,
        import: "tower_http::cors::CorsLayer",
        parameters: "",
        setup: "let $credentials = false;",
        receiver: "{Item}::new()",
        chain: ".allow_origin(tower_http::cors::AllowOrigin::mirror_request()).allow_credentials($credentials)",
    },
    interpolates_parameters: false,
    known_misses: &[],
    semantic: &[
        Semantic {
            description: "tower-http's very permissive layer",
            source: "fn layer() -> tower_http::cors::CorsLayer { tower_http::cors::CorsLayer::very_permissive() }",
            fires: true,
            carve_out: "",
        },
        Semantic {
            description: "actix-cors's permissive policy",
            source: "use actix_cors::Cors;\nfn cors() -> Cors { Cors::permissive() }",
            fires: true,
            carve_out: "",
        },
        Semantic {
            description: "credentials allowed on a layer stored with a wildcard origin",
            source: "use tower_http::cors::{Any, CorsLayer};\nfn layer() -> CorsLayer { let cors = CorsLayer::new().allow_origin(Any); cors.allow_credentials(true) }",
            fires: true,
            carve_out: "",
        },
        Semantic {
            description: "tower-http's permissive layer, which sends no credentials",
            source: "fn layer() -> tower_http::cors::CorsLayer { tower_http::cors::CorsLayer::permissive() }",
            fires: false,
            carve_out: "a wildcard origin without credentials exposes only what an anonymous request reads",
        },
        Semantic {
            description: "credentials allowed for a listed origin",
            source: "use tower_http::cors::CorsLayer;\nfn layer(origin: http::HeaderValue) -> CorsLayer { CorsLayer::new().allow_origin(origin).allow_credentials(true) }",
            fires: false,
            carve_out: "the origin is a listed one",
        },
    ],
};

pub(in crate::source_kernel) static COOKIE_VARIANTS: Variants = Variants {
    rule: SOURCE_INSECURE_AUTH_COOKIE.id,
    positive: Case {
        attributes: "",
        asynchronous: false,
        import: "cookie::Cookie",
        parameters: "",
        setup: "let $readable = false;",
        receiver: "{Item}::build((\"session_id\", \"value\"))",
        chain: ".http_only($readable)",
    },
    negative: Case {
        attributes: "",
        asynchronous: false,
        import: "cookie::Cookie",
        parameters: "",
        setup: "let $readable = false;",
        receiver: "{Item}::build((\"theme\", \"value\"))",
        chain: ".http_only($readable)",
    },
    interpolates_parameters: false,
    known_misses: &[],
    semantic: &[
        Semantic {
            description: "a stored auth cookie made readable",
            source: "use cookie::Cookie;\nfn cookie(value: String) { let mut jar = Cookie::new(\"auth\", value); jar.set_http_only(false); }",
            fires: true,
            carve_out: "",
        },
        Semantic {
            description: "a bare token cookie made readable",
            source: "fn cookie() { let _ = cookie::Cookie::build((\"token\", \"value\")).http_only(false); }",
            fires: false,
            carve_out: "a bare token names a CSRF cookie as often as a session one",
        },
        Semantic {
            description: "a CSRF cookie made readable",
            source: "fn cookie() { let _ = cookie::Cookie::build((\"csrf_session\", \"value\")).http_only(false); }",
            fires: false,
            carve_out: "a CSRF cookie has to be read by the page that sends it back",
        },
    ],
};

pub(in crate::source_kernel) static JWT_VARIANTS: Variants = Variants {
    rule: SOURCE_DISABLED_JWT_VALIDATION.id,
    positive: Case {
        attributes: "",
        asynchronous: false,
        import: "jsonwebtoken::Validation",
        parameters: "",
        setup: "",
        receiver: "{Item}::default()",
        chain: ".insecure_disable_signature_validation()",
    },
    negative: Case {
        attributes: "",
        asynchronous: false,
        import: "jsonwebtoken::Validation",
        parameters: "",
        setup: "",
        receiver: "{Item}::default()",
        chain: ".set_required_spec_claims(&[\"exp\"])",
    },
    interpolates_parameters: false,
    known_misses: &[],
    semantic: &[
        Semantic {
            description: "jsonwebtoken 10's insecure_decode",
            source: "fn claims(token: &str) { let _ = jsonwebtoken::dangerous::insecure_decode::<u8>(token); }",
            fires: true,
            carve_out: "",
        },
        Semantic {
            description: "a stored validation with its signature check turned off",
            source: "use jsonwebtoken::{Algorithm, Validation};\nfn validation() -> Validation { let mut validation = Validation::new(Algorithm::HS256); validation.insecure_disable_signature_validation(); validation }",
            fires: true,
            carve_out: "",
        },
        Semantic {
            description: "a signature check turned off in a test",
            source: "#[test]\nfn decodes() { let _ = jsonwebtoken::dangerous::insecure_decode::<u8>(\"t\"); }",
            fires: false,
            carve_out: "a test decodes its own unsigned fixture",
        },
    ],
};
