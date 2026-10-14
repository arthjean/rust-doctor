use std::time::Duration;

pub async fn async_equivalents() -> std::io::Result<Vec<u8>> {
    tokio::time::sleep(Duration::from_millis(1)).await;
    tokio::fs::read("Cargo.toml").await
}

pub fn synchronous() {
    std::thread::sleep(Duration::from_millis(1));
}

pub async fn offloaded() -> std::io::Result<Vec<u8>> {
    tokio::task::block_in_place(|| std::thread::sleep(Duration::from_millis(1)));
    tokio::task::spawn_blocking(|| std::fs::read("Cargo.toml"))
}

pub async fn in_a_thread() {
    let _ = std::thread::spawn(|| std::fs::read("Cargo.toml")).join();
}

pub async fn async_process() -> std::io::Result<()> {
    tokio::process::Command::new("true").output().await
}

pub async fn async_http() -> Result<(), ()> {
    reqwest::get("http://localhost").await
}

pub async fn unlisted() -> std::io::Result<std::fs::Metadata> {
    std::fs::metadata("Cargo.toml")
}

pub mod prelude {
    pub fn noop() {}
}

/// A glob of the crate's own modules may bind `std`, so the rule abstains.
pub async fn under_the_crates_own_prelude_glob() {
    use crate::blocking_negative::prelude::*;
    noop();
    std::thread::sleep(Duration::from_millis(1));
}

#[cfg(unix)]
fn platform_read() -> std::io::Result<Vec<u8>> {
    std::fs::read("Cargo.toml")
}

#[cfg(not(unix))]
fn platform_read() -> std::io::Result<Vec<u8>> {
    std::fs::read("Cargo.toml")
}

/// Two `fn` items of the name: the helper is ambiguous and not followed.
pub async fn calls_an_ambiguous_helper() -> std::io::Result<Vec<u8>> {
    platform_read()
}
