pub async fn unchecked(url: &str) -> Result<u8, reqwest::Error> {
    let client = reqwest::Client::new();
    client.get(url).send().await?.json::<u8>().await
}

pub async fn unchecked_binding(url: &str) -> Result<String, reqwest::Error> {
    let response = reqwest::Client::new().get(url).send().await?;
    response.text().await
}
