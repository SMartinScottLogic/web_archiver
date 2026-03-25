use common::reqwest_ext::ResponseExt as _;

pub async fn fetch_url(url: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .user_agent("WeekLore/1.0")
        .build()
        .unwrap();
    let res = client.get(url).send().await?;
    //let res = reqwest::get(url).await?;
    Ok(res.text_limited(100_000).await?)
}
