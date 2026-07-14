use std::{
    fs::{File, create_dir_all},
    io::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use common::types::FetchTask;
use email::{FromHeader, mimeheaders::MimeContentTypeHeader};
use mailparse::parse_content_disposition;
use reqwest::{
    Response,
    header::{CONTENT_DISPOSITION, CONTENT_TYPE},
};
use tokio::sync::mpsc::Sender;
use tracing::{debug, error, info};
use uuid::Uuid;

use crate::{extractor::FetchedPage, frontier::db::frontier::FrontierDbTrait, settings::CONFIG};

pub async fn worker_loop_single<DB>(task: FetchTask, tx: Sender<FetchedPage>, db: Arc<DB>)
where
    DB: FrontierDbTrait,
{
    let client = reqwest::Client::builder()
        .user_agent(&CONFIG.get().unwrap().user_agent)
        .build()
        .unwrap();

    let url = task.url.clone();
    debug!("Fetching page {} ...", &url);
    match fetch_page(&client, &url).await {
        Ok((Some((major, _minor)), _, body)) if major == "text" => {
            let fetched = FetchedPage {
                task,
                status_code: 200,
                content_type: None,
                fetch_time: CONFIG.get().unwrap().archive_time,
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
        Ok((content_type, filename, body)) => {
            let _ = save_content(task, &filename, &body, content_type, db)
                .inspect_err(|e| error!(?e, ?filename, "save content"));
        }
        Err(err) => {
            error!("Failed to fetch {}: {}", url, err);
        }
    }
}

fn save_content<DB>(
    task: FetchTask,
    filename: &Path,
    body: &[u8],
    content_type: Option<(String, String)>,
    db: Arc<DB>,
) -> Result<()>
where
    DB: FrontierDbTrait,
{
    debug!(
        binary_size = body.len(),
        ?filename,
        ?content_type,
        "other content_type"
    );
    create_dir_all(filename.parent().unwrap())?;
    let mut file = File::create(filename)?;
    file.write_all(body)?;
    info!(?filename, ?content_type, "media file");

    db.mark_complete_article(task.article_id)?;

    Ok(())
}

async fn fetch_page(
    client: &reqwest::Client,
    url: &str,
) -> Result<(Option<(String, String)>, PathBuf, Vec<u8>), reqwest::Error> {
    let resp = client.get(url).send().await?;

    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|content_type| content_type.to_str().ok())
        .and_then(|content_type| MimeContentTypeHeader::from_header(content_type.to_string()).ok())
        .map(|content_type| content_type.content_type);
    let filename = generate_filename(&resp);
    let bytes = resp.bytes().await?;

    Ok((content_type, filename, bytes.to_vec()))
}

fn generate_filename(response: &Response) -> PathBuf {
    let media_dir = PathBuf::from(&CONFIG.get().unwrap().archive_dir).join("media");
    if let Some(name) = response.url().path().split('/').next_back() {
        let filename = media_dir.join(name);
        if !filename.exists() {
            return filename;
        }
    }

    if let Some((_, name)) = response
        .headers()
        .get(CONTENT_DISPOSITION)
        .and_then(|content_disposition| {
            debug!(?content_disposition, "content_disposition");
            content_disposition.to_str().ok()
        })
        .map(parse_content_disposition)
        .map(|content_disposition| content_disposition.params)
        .and_then(|params| {
            params
                .clone()
                .into_iter()
                .find(|(k, _v)| k.eq_ignore_ascii_case("filename"))
        })
    {
        let filename = media_dir.join(name);
        if !filename.exists() {
            return filename;
        }
    }

    if let Some((_, name)) = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|content_type| {
            debug!(?content_type, "content_type");
            content_type.to_str().ok()
        })
        .and_then(|content_type| MimeContentTypeHeader::from_header(content_type.to_string()).ok())
        .map(|content_type| content_type.params)
        .and_then(|params| {
            params
                .clone()
                .into_iter()
                .find(|(k, _v)| k.eq_ignore_ascii_case("name"))
        })
    {
        let filename = media_dir.join(name);
        if !filename.exists() {
            return filename;
        }
    }

    loop {
        let uuid = Uuid::new_v4();
        let filename = media_dir.join(format!("{}.raw.child", uuid));
        if !filename.exists() {
            break filename;
        }
    }
}

#[cfg(test)]
mod tests {
    use common::types::Priority;

    use crate::{
        frontier::db::frontier::MockFrontierDbTrait, settings::test_setup::setup_test_config,
    };

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

        setup_test_config();

        // Use a known good URL for testing (httpbin.org is reliable for tests)
        let task = FetchTask {
            article_id: 0,
            url_id: 1,
            url: "https://httpbin.org/html".to_string(),
            depth: 0,
            priority: Priority::default(),
            discovered_from: None,
            use_playwright: false,
        };
        let db = MockFrontierDbTrait::new();
        let (tx, mut rx) = mpsc::channel(1);
        worker_loop_single(task, tx, Arc::new(db)).await;
        // Should receive a FetchedPage
        let fetched = rx.try_recv().unwrap();
        assert_eq!(fetched.status_code, 200);
        assert!(!fetched.body.is_empty());
    }
}
