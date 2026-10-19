// rule: rust_doctor::source::unchecked_http_status
// weakness: api-variant
// source: EP-006 review, the status checked inside and_then
// verdict: quiet

pub fn page(url: &str) -> reqwest::Result<String> {
    reqwest::blocking::get(url)
        .and_then(reqwest::blocking::Response::error_for_status)?
        .text()
}
