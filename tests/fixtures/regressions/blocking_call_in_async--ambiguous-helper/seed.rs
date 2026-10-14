// rule: rust_doctor::source::blocking_call_in_async
// weakness: helper-call
// source: US-017, two items of one name
// verdict: quiet

#[cfg(unix)]
fn load() -> Vec<u8> {
    std::fs::read("data").unwrap_or_default()
}

#[cfg(not(unix))]
fn load() -> Vec<u8> {
    std::fs::read("data").unwrap_or_default()
}

pub async fn handler() -> Vec<u8> {
    load()
}
