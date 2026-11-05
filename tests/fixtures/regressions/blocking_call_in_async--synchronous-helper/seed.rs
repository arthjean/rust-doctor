fn load_config() -> String {
    std::fs::read_to_string("app.toml").unwrap_or_default()
}

pub async fn handler() -> String {
    load_config()
}
