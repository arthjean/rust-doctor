pub async fn save(path: std::path::PathBuf, data: Vec<u8>) {
    let _ = smol::unblock(move || std::fs::write(path, data)).await;
}
