use std::env;

pub fn signing_key() -> String {
    env::var("JWT_SECRET").unwrap_or("dev-signing-key".to_owned())
}

pub fn github_token() -> String {
    std::env::var("GITHUB_TOKEN").unwrap_or_default()
}
