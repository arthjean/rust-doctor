pub fn page(url: &str) -> reqwest::Result<String> {
    reqwest::blocking::get(url)
        .and_then(reqwest::blocking::Response::error_for_status)?
        .text()
}
