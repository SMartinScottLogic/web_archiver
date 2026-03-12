use crate::types::messages::{FetchTask, FetchedPage};
use anyhow::Result;
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, info};

pub async fn worker_loop_single(task: FetchTask, tx: Sender<FetchedPage>) {
    let client = reqwest::Client::builder()
        .user_agent("Week1Crawler/0.1")
        .build()
        .unwrap();

    let url = task.url.clone();
    debug!("Fetching page {} ...", &url);
    match fetch_page(&client, &url).await {
        Ok(body) => {
            let fetched = FetchedPage {
                task,
                status_code: 200,
                content_type: None,
                fetch_time: chrono::Utc::now().timestamp() as u64,
                body: std::sync::Arc::new(body),
            };
            debug!("Fetched page successfully: {}", url);

            info!(
                "Worker sending page to extractor: {} ({}/{})",
                fetched.task.url,
                tx.capacity(),
                tx.max_capacity()
            );
            if let Err(e) = tx.send(fetched).await {
                error!("Failed to send page to extractor: {}", e);
            }
        }
        Err(err) => {
            error!("Failed to fetch {}: {}", url, err);
        }
    }
}

async fn fetch_page(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, reqwest::Error> {
    let resp = client.get(url).send().await?;
    let bytes = resp.bytes().await?;
    Ok(bytes.to_vec())
}
