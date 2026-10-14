pub async fn checked(url: &str) -> Result<u8, reqwest::Error> {
    let client = reqwest::Client::new();
    client.get(url).send().await?.error_for_status()?.json::<u8>().await
}

pub async fn status_read_first(url: &str) -> Option<String> {
    let response = reqwest::Client::new().get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.text().await.ok()
}

fn log(_response: &reqwest::Response) {}

pub async fn handed_over(url: &str) -> Option<String> {
    let response = reqwest::Client::new().get(url).send().await.ok()?;
    log(&response);
    response.text().await.ok()
}

pub async fn client_from_elsewhere(client: &reqwest::Client, url: &str) -> Result<String, reqwest::Error> {
    client.get(url).send().await?.text().await
}
