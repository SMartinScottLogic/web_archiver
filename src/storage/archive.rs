use crate::types::messages::ExtractedPage;
use crate::util::hash_url;
use anyhow::Result;
use chrono::Datelike;
use std::fs::{File, create_dir_all};
use tokio::sync::mpsc::Receiver;
use tracing::{error, info};

pub async fn storage_loop(mut rx: Receiver<ExtractedPage>) {
    while let Some(page) = rx.recv().await {
        if let Err(e) = store_page(&page) {
            error!("Failed to store {}: {}", page.task.url, e);
        }
    }
}

fn store_page(page: &ExtractedPage) -> Result<()> {
    let domain = match crate::util::extract_domain(&page.task.url) {
        Some(d) => d,
        None => "unknown".to_string(),
    };

    // archive/domain/yyyy/mm/hash.json
    let now = chrono::Utc::now();
    let path = format!("archive/{}/{:04}/{:02}", domain, now.year(), now.month());
    create_dir_all(&path)?;

    let filename = format!("{}/{}.json", path, hash_url(&page.task.url));
    let file = File::create(&filename)?;
    serde_json::to_writer_pretty(file, &page)?;

    info!("Stored page: {} -> {}", page.task.url, filename);

    Ok(())
}
