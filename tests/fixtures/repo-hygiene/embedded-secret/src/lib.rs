pub const ANALYTICS_KEY: Option<&str> = option_env!("ANALYTICS_KEY");

pub fn key() -> &'static str {
    ANALYTICS_KEY.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_test_token_is_optional() {
        let _ = option_env!("TEST_ONLY_TOKEN");
    }
}
