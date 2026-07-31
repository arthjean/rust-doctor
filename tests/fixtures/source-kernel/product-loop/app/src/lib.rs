pub fn hazards(user: &str) {
    let _ = reqwest::Client::builder().danger_accept_invalid_certs(true);
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("echo {user}"));
}

pub fn stable_non_source_finding() {
    todo!()
}

pub fn sentinels() -> (&'static str, &'static str, &'static str, &'static str, &'static str) {
    (
        "RD_SOURCE_PAYLOAD_8f9c",
        "https://private.invalid/RD_SOURCE_URL_8f9c",
        "credential=RD_SOURCE_CREDENTIAL_8f9c",
        "/private/RD_SOURCE_PATH_8f9c",
        "\u{1b}[31mRD_SOURCE_ANSI_8f9c",
    )
}
