use std::env;

pub fn database_url() -> String {
    env::var("DATABASE_URL").unwrap_or("postgres://localhost/dev".into())
}

pub fn placeholder() -> String {
    env::var("APP_SECRET").unwrap_or("changeme-now".into())
}

pub fn header_name() -> String {
    env::var("API_KEY_HEADER").unwrap_or("x-api-key-header".into())
}

pub fn required() -> Result<String, env::VarError> {
    env::var("JWT_SECRET")
}

#[cfg(test)]
mod tests {
    fn fixture_secret() -> String {
        std::env::var("JWT_SECRET").unwrap_or("test-signing-key".into())
    }
}
