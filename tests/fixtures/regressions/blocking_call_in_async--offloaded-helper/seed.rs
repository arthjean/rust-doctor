fn offload() {
    let _ = tokio::task::spawn_blocking(|| std::fs::read("data"));
}

pub async fn handler() {
    offload();
}
