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
