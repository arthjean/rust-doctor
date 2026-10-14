use std::time::Duration;

pub async fn async_equivalents() -> std::io::Result<Vec<u8>> {
    tokio::time::sleep(Duration::from_millis(1)).await;
    tokio::fs::read("Cargo.toml").await
}

fn synchronous_helper() -> std::io::Result<Vec<u8>> {
    std::fs::read("Cargo.toml")
}

pub async fn calls_a_synchronous_helper() -> std::io::Result<Vec<u8>> {
    synchronous_helper()
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

pub async fn nested_synchronous_function() {
    fn inner() {
        std::thread::sleep(Duration::from_millis(1));
    }
    inner();
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
