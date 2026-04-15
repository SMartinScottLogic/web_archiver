use anyhow::Result;
use common::types::FetchTask;
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, info};

use crate::extractor::FetchedPage;

pub async fn worker_loop_single(
    task: FetchTask,
    archive_time: i64,
    user_agent: &str,
    tx: Sender<FetchedPage>,
) {
    let client = reqwest::Client::builder()
        .user_agent(user_agent)
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
                fetch_time: archive_time,
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

#[cfg(test)]
mod tests {
    use common::types::Priority;

    use super::*;

    #[tokio::test]
    async fn test_fetch_page_invalid_url() {
        let client = reqwest::Client::new();
        let result = fetch_page(&client, "https://invalid.example.com").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_worker_loop_single_sends_fetched() {
        use common::types::FetchTask;
        use tokio::sync::mpsc;

        // Use a known good URL for testing (httpbin.org is reliable for tests)
        let task = FetchTask {
            article_id: 0,
            url_id: 1,
            url: "https://httpbin.org/html".to_string(),
            depth: 0,
            priority: Priority::default(),
            discovered_from: None,
        };
        let (tx, mut rx) = mpsc::channel(1);
        worker_loop_single(task, 0, "test", tx).await;
        // Should receive a FetchedPage
        let fetched = rx.try_recv().unwrap();
        assert_eq!(fetched.status_code, 200);
        assert!(!fetched.body.is_empty());
    }
}
