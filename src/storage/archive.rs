use crate::frontier::db::frontier::FrontierDb;
use crate::types::messages::ExtractedPage;
use crate::util::hash_url;
use anyhow::Result;
use chrono::Datelike;
use std::fs::{File, create_dir_all};
use tokio::sync::mpsc::Receiver;
use tracing::{error, info};

pub async fn storage_loop(mut rx: Receiver<ExtractedPage>, db: FrontierDb) {
    while let Some(page) = rx.recv().await {
        match store_page(&page) {
            Ok(_) => {
                // Mark as complete in the DB
                if let Err(e) = db.mark_complete(page.task.url_id) {
                    error!("Failed to mark complete for {}: {}", page.task.url, e);
                }
            }
            Err(e) => {
                error!("Failed to store {}: {}", page.task.url, e);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::messages::{ExtractedPage, FetchTask, PageMetadata};
    use std::fs;

    #[test]
    fn test_store_page_creates_file() {
        let page = ExtractedPage {
            task: FetchTask {
                url_id: 1,
                url: "http://foo.com/test".to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: Some("content".to_string()),
            links: vec![],
            metadata: PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 0,
                title: Some("Test".to_string()),
                document_metadata: vec![],
            },
        };
        let result = store_page(&page);
        assert!(result.is_ok());
        // Clean up
        let domain = crate::util::extract_domain(&page.task.url).unwrap();
        let now = chrono::Utc::now();
        let path = format!("archive/{}/{:04}/{:02}", domain, now.year(), now.month());
        let filename = format!("{}/{}.json", path, crate::util::hash_url(&page.task.url));
        assert!(fs::metadata(&filename).is_ok());
        let _ = fs::remove_file(&filename);
    }

    #[tokio::test]
    async fn test_storage_loop_marks_complete() {
        use crate::frontier::db::frontier::FrontierDb;
        use rusqlite::Connection;
        use std::sync::{Arc, Mutex};
        use tokio::sync::mpsc;

        // Setup in-memory DB with minimal schema
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE urls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT UNIQUE NOT NULL,
                domain TEXT,
                discovered_at INTEGER
            );
            CREATE TABLE frontier (
                url_id INTEGER,
                priority INTEGER,
                depth INTEGER,
                discovered_from INTEGER,
                status TEXT,
                claimed_at INTEGER,
                FOREIGN KEY(url_id) REFERENCES urls(id),
                UNIQUE(url_id)
            );
        "#,
        )
        .unwrap();
        let db = FrontierDb {
            conn: Arc::new(Mutex::new(conn)),
        };

        // Insert a test URL and frontier row
        let url = "http://foo.com/test";
        let url_id = {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO urls (url, domain, discovered_at) VALUES (?1, ?2, 0)",
                rusqlite::params![url, "foo.com"],
            )
            .unwrap();
            conn.query_row(
                "SELECT id FROM urls WHERE url = ?1",
                rusqlite::params![url],
                |row| row.get(0),
            )
            .unwrap()
        };
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO frontier (url_id, priority, depth, discovered_from, status) VALUES (?1, 0, 0, NULL, 'pending')",
                rusqlite::params![url_id],
            ).unwrap();
        }

        // Setup channel and send ExtractedPage
        let (tx, rx) = mpsc::channel(1);
        let page = ExtractedPage {
            task: FetchTask {
                url_id,
                url: url.to_string(),
                depth: 0,
                priority: 0,
                discovered_from: None,
            },
            content_markdown: Some("content".to_string()),
            links: vec![],
            metadata: PageMetadata {
                status_code: 200,
                content_type: Some("text/html".to_string()),
                fetch_time: 0,
                title: Some("Test".to_string()),
                document_metadata: vec![],
            },
        };
        tx.send(page).await.unwrap();
        drop(tx); // Close channel

        storage_loop(rx, db.clone()).await;

        // Check that the status is now 'complete'
        let conn = db.conn.lock().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM frontier WHERE url_id = ?1",
                rusqlite::params![url_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "complete");
    }
}
