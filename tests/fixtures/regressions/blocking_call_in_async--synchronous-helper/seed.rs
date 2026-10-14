// rule: rust_doctor::source::blocking_call_in_async
// weakness: helper-call
// source: audit Z2-N03, fixed by US-017
// verdict: fires

fn load_config() -> String {
    std::fs::read_to_string("app.toml").unwrap_or_default()
}

pub async fn handler() -> String {
    load_config()
}
